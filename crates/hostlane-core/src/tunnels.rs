//! Local port forwarding (`ssh -L`). A running tunnel owns a local listener and keeps
//! the host's pooled SSH connection open (so the pool never closes it as idle); every
//! accepted connection becomes a `direct-tcpip` channel. Saved tunnels live in the
//! `tunnels` table and are restored when the daemon starts; temporary tunnels opened by
//! agents expire.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use hostlane_protocol::{
    AuditEntry, AuditOutcome, ClientInfo, ErrorKind, Event, RpcError, Tunnel, TunnelCloseParams,
    TunnelOpenParams, TunnelOpened, TunnelState, TunnelUpdateParams, TunnelsClosed, TunnelsList,
    method,
};
use russh::ChannelOpenFailure;
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, watch};
use tokio::task::{JoinHandle, JoinSet};

use crate::events::EventBus;
use crate::pool::{Conn, Pool};
use crate::ssh::Trust;
use crate::state::resolve_host;
use crate::store::{NewCall, Store};
use crate::util::{new_id, now_ms};

/// Lifetime of a temporary tunnel opened by an agent when it does not say.
const DEFAULT_TTL_SECS: u64 = 60 * 60;
const MAX_TTL_SECS: u64 = 24 * 60 * 60;
/// Tunnels opened by this client do not expire unless asked to.
const DESKTOP_CLIENT: &str = "desktop";
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
const HEALTH_INTERVAL: Duration = Duration::from_secs(2);
const STATS_INTERVAL: Duration = Duration::from_secs(2);
/// How long an accepted connection waits for the SSH connection to come back.
const WAIT_FOR_CONNECTION: Duration = Duration::from_secs(15);
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
/// Ports tried after `remote_port` when no local port is given.
const AUTO_PORT_RANGE: u16 = 20;
const SUGGEST_RANGE: u16 = 200;
const BUFFER: usize = 32 * 1024;

/// The stored part of a tunnel.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TunnelConfig {
    pub id: String,
    pub name: Option<String>,
    pub host: String,
    pub bind: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub auto_start: bool,
    pub created_by: ClientInfo,
    pub created_at: u64,
}

#[derive(Default)]
struct Stats {
    active: AtomicU32,
    total: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
}

type StatsSnapshot = (u32, u64, u64, u64);

impl Stats {
    fn read(&self) -> StatsSnapshot {
        (
            self.active.load(Ordering::Relaxed),
            self.total.load(Ordering::Relaxed),
            self.bytes_in.load(Ordering::Relaxed),
            self.bytes_out.load(Ordering::Relaxed),
        )
    }
}

struct Status {
    saved: bool,
    expires_at: Option<u64>,
    state: TunnelState,
    error: Option<RpcError>,
    last_connection_error: Option<String>,
    /// Counters as of the last `tunnel` event.
    published: StatsSnapshot,
}

struct Runtime {
    config: Mutex<TunnelConfig>,
    status: Mutex<Status>,
    stats: Stats,
    task: Mutex<Option<JoinHandle<()>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Runtime {
    fn new(config: TunnelConfig, saved: bool, expires_at: Option<u64>) -> Arc<Self> {
        Arc::new(Self {
            config: Mutex::new(config),
            status: Mutex::new(Status {
                saved,
                expires_at,
                state: TunnelState::Stopped,
                error: None,
                last_connection_error: None,
                published: StatsSnapshot::default(),
            }),
            stats: Stats::default(),
            task: Mutex::new(None),
        })
    }

    fn config(&self) -> TunnelConfig {
        lock(&self.config).clone()
    }

    fn id(&self) -> String {
        lock(&self.config).id.clone()
    }

    fn saved(&self) -> bool {
        lock(&self.status).saved
    }

    fn running(&self) -> bool {
        lock(&self.task)
            .as_ref()
            .is_some_and(|task| !task.is_finished())
    }

    /// Stops the listener and every forwarded connection; returns once the port is free.
    async fn stop(&self) {
        let task = lock(&self.task).take();
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
    }

    fn connection_failed(&self, message: String) {
        tracing::debug!(tunnel = self.id(), message, "forwarded connection failed");
        lock(&self.status).last_connection_error = Some(message);
    }

    fn snapshot(&self) -> Tunnel {
        let config = self.config();
        let status = lock(&self.status);
        let (active, total, bytes_in, bytes_out) = self.stats.read();
        Tunnel {
            url: local_url(&config.bind, config.local_port),
            id: config.id,
            name: config.name,
            host: config.host,
            bind: config.bind,
            local_port: config.local_port,
            remote_host: config.remote_host,
            remote_port: config.remote_port,
            saved: status.saved,
            auto_start: config.auto_start,
            state: status.state,
            error: status.error.clone(),
            last_connection_error: status.last_connection_error.clone(),
            created_by: config.created_by,
            created_at: config.created_at,
            expires_at: status.expires_at,
            active_connections: active,
            total_connections: total,
            bytes_in,
            bytes_out,
        }
    }
}

#[derive(Clone)]
struct Ctx {
    pool: Arc<Pool>,
    events: EventBus,
    store: Store,
    ssh_config_path: PathBuf,
}

impl Ctx {
    fn publish(&self, rt: &Runtime) {
        lock(&rt.status).published = rt.stats.read();
        self.events.publish(Event::Tunnel {
            tunnel: rt.snapshot(),
        });
    }

    fn set_state(&self, rt: &Runtime, state: TunnelState, error: Option<RpcError>) {
        {
            let mut status = lock(&rt.status);
            status.state = state;
            status.error = error;
        }
        self.publish(rt);
    }

    async fn connect(&self, host: &str) -> Result<Arc<Conn>, RpcError> {
        let (entry, loaded) = resolve_host(&self.ssh_config_path, host).await?;
        let (conn, _, _) = self.pool.connect(&entry, &loaded, Trust::Strict).await?;
        Ok(conn)
    }
}

pub(crate) struct Tunnels {
    ctx: Ctx,
    map: Mutex<HashMap<String, Arc<Runtime>>>,
    /// Serialises changes so two calls cannot claim the same local port.
    ops: tokio::sync::Mutex<()>,
}

impl Tunnels {
    pub fn new(pool: Arc<Pool>, events: EventBus, store: Store, ssh_config_path: PathBuf) -> Self {
        Self {
            ctx: Ctx {
                pool,
                events,
                store,
                ssh_config_path,
            },
            map: Mutex::new(HashMap::new()),
            ops: tokio::sync::Mutex::new(()),
        }
    }

    fn all(&self) -> Vec<Arc<Runtime>> {
        lock(&self.map).values().cloned().collect()
    }

    fn get(&self, id: &str) -> Result<Arc<Runtime>, RpcError> {
        lock(&self.map).get(id).cloned().ok_or_else(|| {
            RpcError::new(ErrorKind::TunnelNotFound, format!("no tunnel `{id}`"))
                .with_hint("list tunnels with tunnels.list (CLI: hostlane tunnels)")
        })
    }

    /// Local ports claimed by other tunnels, running or not.
    fn reserved(&self, except: &str) -> HashSet<u16> {
        self.all()
            .iter()
            .map(|rt| rt.config())
            .filter(|config| config.id != except)
            .map(|config| config.local_port)
            .collect()
    }

    pub fn list(&self) -> TunnelsList {
        let mut tunnels: Vec<Tunnel> = self.all().iter().map(|rt| rt.snapshot()).collect();
        tunnels.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        TunnelsList { tunnels }
    }

    pub async fn open(
        &self,
        client: &ClientInfo,
        params: TunnelOpenParams,
    ) -> Result<TunnelOpened, RpcError> {
        let remote_port = check_port("remote_port", params.remote_port)?;
        let ip = parse_bind(params.bind.as_deref(), params.allow_public)?;
        let remote_host = check_remote_host(params.remote_host.as_deref().unwrap_or("127.0.0.1"))?;
        if params.save && params.ttl_secs.is_some() {
            return Err(RpcError::new(
                ErrorKind::InvalidParams,
                "saved tunnels do not expire; pass either `save` or `ttl_secs`",
            ));
        }
        let ttl = match params.ttl_secs {
            Some(ttl) if ttl == 0 || ttl > MAX_TTL_SECS => {
                return Err(RpcError::new(
                    ErrorKind::InvalidParams,
                    format!("`ttl_secs` must be between 1 and {MAX_TTL_SECS}"),
                ));
            }
            Some(ttl) => Some(ttl),
            None if params.save || client.name == DESKTOP_CLIENT => None,
            None => Some(DEFAULT_TTL_SECS),
        };

        let _ops = self.ops.lock().await;
        let conn = self.ctx.connect(&params.host).await?;
        let mut warnings = check_remote(&conn, &remote_host, remote_port).await?;
        let listener = bind(ip, params.local_port, remote_port, self.reserved("")).await?;
        let local_port = listener
            .local_addr()
            .map_err(|err| RpcError::new(ErrorKind::Internal, err.to_string()))?
            .port();
        if !ip.is_loopback() {
            warnings.push(format!(
                "the tunnel listens on {ip}, so other machines that can reach this one can use it"
            ));
        }
        let config = TunnelConfig {
            id: new_id("tn"),
            name: clean_name(params.name),
            host: params.host,
            bind: ip.to_string(),
            local_port,
            remote_host,
            remote_port,
            auto_start: params.save && params.auto_start.unwrap_or(true),
            created_by: client.clone(),
            created_at: now_ms(),
        };
        if params.save {
            self.ctx.store.save_tunnel(config.clone()).await?;
        }
        let expires_at = ttl.map(|ttl| config.created_at + ttl * 1000);
        let rt = Runtime::new(config.clone(), params.save, expires_at);
        lock(&self.map).insert(config.id, rt.clone());
        self.launch(&rt, listener, Some(conn));
        tracing::info!(tunnel = rt.id(), local_port, "tunnel opened");
        Ok(TunnelOpened {
            tunnel: rt.snapshot(),
            warnings,
        })
    }

    pub async fn start(&self, id: &str) -> Result<TunnelOpened, RpcError> {
        let _ops = self.ops.lock().await;
        let rt = self.get(id)?;
        if rt.running() {
            return Ok(TunnelOpened {
                tunnel: rt.snapshot(),
                warnings: vec!["the tunnel is already running".into()],
            });
        }
        let config = rt.config();
        let started = async {
            let conn = self.ctx.connect(&config.host).await?;
            let warnings = check_remote(&conn, &config.remote_host, config.remote_port).await?;
            let listener = self.bind_config(&config).await?;
            Ok::<_, RpcError>((conn, warnings, listener))
        }
        .await;
        match started {
            Ok((conn, warnings, listener)) => {
                self.launch(&rt, listener, Some(conn));
                Ok(TunnelOpened {
                    tunnel: rt.snapshot(),
                    warnings,
                })
            }
            Err(err) => {
                self.ctx
                    .set_state(&rt, TunnelState::Failed, Some(err.clone()));
                Err(err)
            }
        }
    }

    pub async fn close(&self, params: TunnelCloseParams) -> Result<TunnelsClosed, RpcError> {
        let _ops = self.ops.lock().await;
        let targets = match (params.id, params.session) {
            (Some(id), _) => vec![self.get(&id)?],
            (None, Some(session)) => self
                .all()
                .into_iter()
                .filter(|rt| {
                    !rt.saved() && rt.config().created_by.session.as_deref() == Some(&session)
                })
                .collect(),
            (None, None) => {
                return Err(RpcError::new(
                    ErrorKind::InvalidParams,
                    "pass `id`, or `session` to close every temporary tunnel of an agent session",
                ));
            }
        };
        let mut closed = Vec::new();
        for rt in targets {
            closed.push(rt.id());
            self.close_one(&rt).await;
        }
        Ok(TunnelsClosed { closed })
    }

    /// Stops a tunnel; temporary tunnels are forgotten, saved ones stay as `stopped`.
    async fn close_one(&self, rt: &Arc<Runtime>) {
        rt.stop().await;
        if rt.saved() {
            self.ctx.set_state(rt, TunnelState::Stopped, None);
        } else {
            self.forget(rt);
        }
    }

    fn forget(&self, rt: &Runtime) {
        let id = rt.id();
        lock(&self.map).remove(&id);
        self.ctx.events.publish(Event::TunnelRemoved { id });
    }

    pub async fn delete(&self, id: &str) -> Result<TunnelsClosed, RpcError> {
        let _ops = self.ops.lock().await;
        let rt = self.get(id)?;
        rt.stop().await;
        if rt.saved() {
            self.ctx.store.delete_tunnel(id.to_string()).await?;
        }
        self.forget(&rt);
        Ok(TunnelsClosed {
            closed: vec![id.to_string()],
        })
    }

    pub async fn update(&self, params: TunnelUpdateParams) -> Result<TunnelOpened, RpcError> {
        let _ops = self.ops.lock().await;
        let rt = self.get(&params.id)?;
        let old = rt.config();
        let mut new = old.clone();
        if let Some(name) = params.name {
            new.name = clean_name(Some(name));
        }
        if let Some(host) = &params.remote_host {
            new.remote_host = check_remote_host(host)?;
        }
        if let Some(port) = params.remote_port {
            new.remote_port = check_port("remote_port", port)?;
        }
        if let Some(port) = params.local_port {
            new.local_port = check_port("local_port", port)?;
        }
        if let Some(bind) = &params.bind {
            new.bind = parse_bind(Some(bind), params.allow_public)?.to_string();
        }
        let was_saved = rt.saved();
        let saved = params.save.unwrap_or(was_saved);
        let auto_default = if was_saved { old.auto_start } else { true };
        new.auto_start = saved && params.auto_start.unwrap_or(auto_default);

        let mut warnings = Vec::new();
        let running = rt.running();
        let rebind = new.local_port != old.local_port || new.bind != old.bind;
        if running && rebind {
            rt.stop().await;
            match self.bind_config(&new).await {
                Ok(listener) => {
                    *lock(&rt.config) = new.clone();
                    let conn = self.ctx.connect(&new.host).await.ok();
                    self.launch(&rt, listener, conn);
                }
                Err(err) => {
                    match self.bind_config(&old).await {
                        Ok(listener) => {
                            let conn = self.ctx.connect(&old.host).await.ok();
                            self.launch(&rt, listener, conn);
                        }
                        Err(old_err) => {
                            self.ctx.set_state(&rt, TunnelState::Failed, Some(old_err));
                        }
                    }
                    return Err(err);
                }
            }
        } else if rebind {
            let reserved = self.reserved(&new.id);
            if reserved.contains(&new.local_port) {
                let ip = new.bind.parse().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
                let suggested = suggest(ip, new.local_port, &reserved);
                return Err(port_in_use(
                    new.local_port,
                    "used by another tunnel",
                    suggested,
                ));
            }
        }
        let remote_changed =
            new.remote_host != old.remote_host || new.remote_port != old.remote_port;
        if running
            && remote_changed
            && let Ok(conn) = self.ctx.connect(&new.host).await
        {
            warnings.extend(check_remote(&conn, &new.remote_host, new.remote_port).await?);
        }

        *lock(&rt.config) = new.clone();
        {
            let mut status = lock(&rt.status);
            status.saved = saved;
            if saved {
                status.expires_at = None;
            }
        }
        if saved {
            self.ctx.store.save_tunnel(new).await?;
        } else if was_saved {
            self.ctx.store.delete_tunnel(params.id).await?;
        }
        self.ctx.publish(&rt);
        Ok(TunnelOpened {
            tunnel: rt.snapshot(),
            warnings,
        })
    }

    /// Loads saved tunnels and starts the ones marked `auto_start`. Connection problems
    /// are retried in the background; a taken local port marks the tunnel failed.
    pub async fn restore(&self) {
        let configs = match self.ctx.store.tunnels().await {
            Ok(configs) => configs,
            Err(err) => {
                tracing::warn!(%err, "cannot load saved tunnels");
                return;
            }
        };
        let _ops = self.ops.lock().await;
        for config in configs {
            let rt = Runtime::new(config.clone(), true, None);
            lock(&self.map).insert(config.id.clone(), rt.clone());
            if !config.auto_start {
                continue;
            }
            match self.bind_config(&config).await {
                Ok(listener) => self.launch(&rt, listener, None),
                Err(err) => {
                    tracing::warn!(tunnel = config.id, %err, "cannot restore tunnel");
                    self.ctx.set_state(&rt, TunnelState::Failed, Some(err));
                }
            }
        }
    }

    /// Publishes traffic counters and closes expired tunnels. Runs forever.
    pub async fn maintain(self: Arc<Self>) {
        let mut interval = tokio::time::interval(STATS_INTERVAL);
        loop {
            interval.tick().await;
            let now = now_ms();
            for rt in self.all() {
                let (expired, published) = {
                    let status = lock(&rt.status);
                    (
                        status.expires_at.is_some_and(|at| at <= now),
                        status.published,
                    )
                };
                if expired {
                    self.expire(&rt).await;
                } else if rt.stats.read() != published {
                    self.ctx.publish(&rt);
                }
            }
        }
    }

    async fn expire(&self, rt: &Arc<Runtime>) {
        let _ops = self.ops.lock().await;
        let config = rt.config();
        if !lock(&self.map).contains_key(&config.id) {
            return;
        }
        self.close_one(rt).await;
        tracing::info!(tunnel = config.id, "closed expired tunnel");
        let now = now_ms();
        let call = NewCall {
            started_at: now,
            finished_at: now,
            client: ClientInfo::new("hostlane"),
            host: Some(config.host),
            method: method::TUNNELS_CLOSE.into(),
            params: json!({ "id": config.id, "reason": "expired" }),
            outcome: AuditOutcome::Ok,
            error: None,
            result: json!({ "closed": [config.id] }),
        };
        let entry = call.entry(0);
        match self.ctx.store.append_call(call).await {
            Ok(id) => self.ctx.events.publish(Event::AuditAppended {
                entry: AuditEntry { id, ..entry },
            }),
            Err(err) => tracing::error!(%err, "cannot write the audit log"),
        }
    }

    pub async fn stop_all(&self) {
        for rt in self.all() {
            rt.stop().await;
        }
    }

    async fn bind_config(&self, config: &TunnelConfig) -> Result<TcpListener, RpcError> {
        let ip = config.bind.parse().map_err(|_| {
            RpcError::new(
                ErrorKind::InvalidParams,
                format!("invalid bind address `{}`", config.bind),
            )
        })?;
        bind(
            ip,
            Some(config.local_port),
            config.remote_port,
            self.reserved(&config.id),
        )
        .await
    }

    fn launch(&self, rt: &Arc<Runtime>, listener: TcpListener, conn: Option<Arc<Conn>>) {
        let state = if conn.is_some() {
            TunnelState::Active
        } else {
            TunnelState::Reconnecting
        };
        self.ctx.set_state(rt, state, None);
        let task = tokio::spawn(supervise(self.ctx.clone(), rt.clone(), listener, conn));
        *lock(&rt.task) = Some(task);
    }
}

async fn supervise(ctx: Ctx, rt: Arc<Runtime>, listener: TcpListener, conn: Option<Arc<Conn>>) {
    let (sender, receiver) = watch::channel(conn);
    let broken = Arc::new(Notify::new());
    tokio::select! {
        () = accept_loop(&rt, listener, receiver, broken.clone()) => {}
        () = keep_connected(&ctx, &rt, sender, &broken) => {}
    }
}

/// Errors that retrying cannot fix.
fn is_fatal(err: &RpcError) -> bool {
    matches!(
        err.kind(),
        ErrorKind::HostKeyUnknown
            | ErrorKind::HostKeyChanged
            | ErrorKind::HostNotFound
            | ErrorKind::InvalidParams
    )
}

/// Holds the SSH connection and replaces it when it drops, backing off between
/// attempts; `broken` (a failed channel, or a local client arriving while
/// disconnected) cuts the wait short. Returns only when reconnecting cannot succeed.
async fn keep_connected(
    ctx: &Ctx,
    rt: &Runtime,
    sender: watch::Sender<Option<Arc<Conn>>>,
    broken: &Notify,
) {
    let host = rt.config().host;
    let mut backoff = BACKOFF_MIN;
    loop {
        let current = sender.borrow().clone();
        if let Some(conn) = current {
            backoff = BACKOFF_MIN;
            if lock(&rt.status).state != TunnelState::Active {
                ctx.set_state(rt, TunnelState::Active, None);
            }
            while !conn.handle.is_closed() {
                tokio::select! {
                    () = tokio::time::sleep(HEALTH_INTERVAL) => {}
                    () = broken.notified() => {}
                }
            }
            sender.send_replace(None);
            drop(conn);
            ctx.pool.forget_if_closed(&host);
            tracing::info!(tunnel = rt.id(), host, "tunnel lost its SSH connection");
            let lost = RpcError::new(ErrorKind::ConnectionLost, "the SSH connection dropped");
            ctx.set_state(rt, TunnelState::Reconnecting, Some(lost));
        }
        match ctx.connect(&host).await {
            Ok(conn) => {
                sender.send_replace(Some(conn));
            }
            Err(err) if is_fatal(&err) => {
                tracing::warn!(tunnel = rt.id(), %err, "tunnel failed");
                ctx.set_state(rt, TunnelState::Failed, Some(err));
                return;
            }
            Err(err) => {
                ctx.set_state(rt, TunnelState::Reconnecting, Some(err));
                tokio::select! {
                    () = tokio::time::sleep(backoff) => {}
                    () = broken.notified() => {}
                }
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
        }
    }
}

async fn accept_loop(
    rt: &Arc<Runtime>,
    listener: TcpListener,
    receiver: watch::Receiver<Option<Arc<Conn>>>,
    broken: Arc<Notify>,
) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    connections.spawn(forward(
                        rt.clone(),
                        stream,
                        peer,
                        receiver.clone(),
                        broken.clone(),
                    ));
                }
                Err(err) => {
                    tracing::debug!(%err, "accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}

async fn forward(
    rt: Arc<Runtime>,
    stream: TcpStream,
    peer: SocketAddr,
    mut receiver: watch::Receiver<Option<Arc<Conn>>>,
    broken: Arc<Notify>,
) {
    let _ = stream.set_nodelay(true);
    if receiver.borrow().is_none() {
        broken.notify_one();
    }
    let ready = tokio::time::timeout(WAIT_FOR_CONNECTION, receiver.wait_for(Option::is_some))
        .await
        .is_ok_and(|waited| waited.is_ok());
    let conn = if ready {
        receiver.borrow().clone()
    } else {
        None
    };
    let Some(conn) = conn else {
        rt.connection_failed("the SSH connection is not available".into());
        return;
    };
    let config = rt.config();
    let target = format!("{}:{}", config.remote_host, config.remote_port);
    let opened = tokio::time::timeout(
        OPEN_TIMEOUT,
        conn.handle.channel_open_direct_tcpip(
            config.remote_host,
            config.remote_port.into(),
            peer.ip().to_string(),
            peer.port().into(),
        ),
    )
    .await;
    let channel = match opened {
        Ok(Ok(channel)) => channel,
        Ok(Err(russh::Error::ChannelOpenFailure(ChannelOpenFailure::ConnectFailed))) => {
            rt.connection_failed(format!("{target} refused the connection on the server"));
            return;
        }
        Ok(Err(russh::Error::ChannelOpenFailure(
            ChannelOpenFailure::AdministrativelyProhibited,
        ))) => {
            rt.connection_failed("the SSH server does not allow TCP forwarding".into());
            return;
        }
        Ok(Err(err)) => {
            rt.connection_failed(format!("cannot open a channel to {target}: {err}"));
            broken.notify_one();
            return;
        }
        Err(_) => {
            rt.connection_failed(format!("opening a channel to {target} timed out"));
            broken.notify_one();
            return;
        }
    };
    rt.stats.active.fetch_add(1, Ordering::Relaxed);
    rt.stats.total.fetch_add(1, Ordering::Relaxed);
    lock(&rt.status).last_connection_error = None;
    pump(stream, channel.into_stream(), &rt.stats).await;
    rt.stats.active.fetch_sub(1, Ordering::Relaxed);
}

/// Copies both ways until both sides finish, or either fails.
async fn pump<L, R>(local: L, remote: R, stats: &Stats)
where
    L: AsyncRead + AsyncWrite,
    R: AsyncRead + AsyncWrite,
{
    let (mut local_read, mut local_write) = tokio::io::split(local);
    let (mut remote_read, mut remote_write) = tokio::io::split(remote);
    let up = copy_counting(&mut local_read, &mut remote_write, &stats.bytes_out);
    let down = copy_counting(&mut remote_read, &mut local_write, &stats.bytes_in);
    tokio::pin!(up, down);
    tokio::select! {
        result = &mut up => if result.is_ok() { let _ = down.await; },
        result = &mut down => if result.is_ok() { let _ = up.await; },
    }
}

async fn copy_counting<R, W>(
    reader: &mut R,
    writer: &mut W,
    counter: &AtomicU64,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = vec![0u8; BUFFER];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read]).await?;
        counter.fetch_add(read as u64, Ordering::Relaxed);
    }
    writer.shutdown().await
}

/// Opens and closes one channel to the target: refusal is only a warning (the service
/// may not be up yet), a server that forbids forwarding is an error.
async fn check_remote(conn: &Conn, host: &str, port: u16) -> Result<Vec<String>, RpcError> {
    let target = format!("{host}:{port}");
    let opened = tokio::time::timeout(
        OPEN_TIMEOUT,
        conn.handle
            .channel_open_direct_tcpip(host, port.into(), "127.0.0.1", 0),
    )
    .await;
    match opened {
        Ok(Ok(channel)) => {
            let _ = channel.close().await;
            Ok(Vec::new())
        }
        Ok(Err(russh::Error::ChannelOpenFailure(
            ChannelOpenFailure::AdministrativelyProhibited,
        ))) => Err(RpcError::new(
            ErrorKind::ForwardingDenied,
            "the SSH server does not allow TCP forwarding",
        )
        .with_hint("set AllowTcpForwarding yes in the server's sshd_config")),
        Ok(Err(russh::Error::ChannelOpenFailure(ChannelOpenFailure::ConnectFailed))) => {
            Ok(vec![format!(
                "nothing accepts connections on {target} on the server yet; the tunnel stays open and forwards once it does"
            )])
        }
        Ok(Err(err)) => Ok(vec![format!(
            "could not test {target} on the server: {err}"
        )]),
        Err(_) => Ok(vec![format!("testing {target} on the server timed out")]),
    }
}

fn check_port(field: &str, port: u16) -> Result<u16, RpcError> {
    if port == 0 {
        return Err(RpcError::new(
            ErrorKind::InvalidParams,
            format!("`{field}` must be between 1 and 65535"),
        ));
    }
    Ok(port)
}

/// The local address to listen on; anything but loopback needs `allow_public`.
fn parse_bind(bind: Option<&str>, allow_public: bool) -> Result<IpAddr, RpcError> {
    let text = bind.map(str::trim).unwrap_or("127.0.0.1");
    let text = if text == "localhost" {
        "127.0.0.1"
    } else {
        text
    };
    let ip: IpAddr = text
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .map_err(|_| {
            RpcError::new(
                ErrorKind::InvalidParams,
                format!("`bind` must be an IP address, got `{text}`"),
            )
        })?;
    if !ip.is_loopback() && !allow_public {
        return Err(RpcError::new(
            ErrorKind::InvalidParams,
            format!("binding {ip} would let other machines use the tunnel"),
        )
        .with_hint(
            "keep the default 127.0.0.1, or confirm with the user and pass allow_public: true (CLI: --allow-public)",
        ));
    }
    Ok(ip)
}

fn check_remote_host(host: &str) -> Result<String, RpcError> {
    let host = host.trim();
    if host.is_empty()
        || host.len() > 253
        || host.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(RpcError::new(
            ErrorKind::InvalidParams,
            format!("invalid remote_host `{host}`"),
        ));
    }
    Ok(host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string())
}

fn clean_name(name: Option<String>) -> Option<String> {
    let name = name?.trim().chars().take(100).collect::<String>();
    (!name.is_empty()).then_some(name)
}

/// The address a browser on this machine should open.
fn local_url(bind: &str, port: u16) -> String {
    match bind.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) if ip.is_unspecified() => format!("http://127.0.0.1:{port}/"),
        Ok(IpAddr::V6(ip)) if ip.is_unspecified() => format!("http://[::1]:{port}/"),
        Ok(IpAddr::V6(ip)) => format!("http://[{ip}]:{port}/"),
        _ => format!("http://{bind}:{port}/"),
    }
}

enum BindFailure {
    /// The port is taken; the reason completes "local port N is …".
    Taken(&'static str),
    Other(RpcError),
}

/// Whether something already accepts connections on the port. Binding alone misses a
/// wildcard listener when we bind a specific address on some systems.
fn occupied(ip: IpAddr, port: u16) -> bool {
    let target = match ip {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    std::net::TcpStream::connect_timeout(&SocketAddr::new(target, port), Duration::from_millis(200))
        .is_ok()
}

fn try_bind(
    ip: IpAddr,
    port: u16,
    reserved: &HashSet<u16>,
) -> Result<std::net::TcpListener, BindFailure> {
    if port != 0 {
        if reserved.contains(&port) {
            return Err(BindFailure::Taken("used by another tunnel"));
        }
        if occupied(ip, port) {
            return Err(BindFailure::Taken("already in use on this machine"));
        }
    }
    std::net::TcpListener::bind((ip, port)).map_err(|err| match err.kind() {
        std::io::ErrorKind::AddrInUse => BindFailure::Taken("already in use on this machine"),
        std::io::ErrorKind::PermissionDenied => BindFailure::Taken("reserved for administrators"),
        std::io::ErrorKind::AddrNotAvailable => BindFailure::Other(RpcError::new(
            ErrorKind::InvalidParams,
            format!("{ip} is not an address of this machine"),
        )),
        _ => BindFailure::Other(RpcError::new(
            ErrorKind::Internal,
            format!("cannot listen on {ip}:{port}: {err}"),
        )),
    })
}

fn suggest(ip: IpAddr, port: u16, reserved: &HashSet<u16>) -> Option<u16> {
    let start = port.max(1023);
    (start.saturating_add(1)..=start.saturating_add(SUGGEST_RANGE))
        .find(|candidate| try_bind(ip, *candidate, reserved).is_ok())
}

fn port_in_use(port: u16, reason: &str, suggested: impl Into<Option<u16>>) -> RpcError {
    let suggested = suggested.into().filter(|s| *s != port);
    let mut err = RpcError::new(
        ErrorKind::PortInUse,
        format!("local port {port} is {reason}"),
    );
    err = match suggested {
        Some(free) => err.with_hint(format!(
            "use local_port {free}, or leave local_port out to pick a free port"
        )),
        None => err.with_hint("leave local_port out to pick a free port"),
    };
    err.with_details(json!({ "port": port, "suggested_port": suggested }))
}

/// Binds the requested port, or `remote_port` (or a port just above it) when none is
/// requested, falling back to one the system picks.
fn bind_local(
    ip: IpAddr,
    requested: Option<u16>,
    remote_port: u16,
    reserved: &HashSet<u16>,
) -> Result<std::net::TcpListener, RpcError> {
    if let Some(port) = requested.filter(|p| *p != 0) {
        return try_bind(ip, port, reserved).map_err(|failure| match failure {
            BindFailure::Taken(reason) => port_in_use(port, reason, suggest(ip, port, reserved)),
            BindFailure::Other(err) => err,
        });
    }
    if remote_port >= 1024 {
        for port in remote_port..=remote_port.saturating_add(AUTO_PORT_RANGE) {
            match try_bind(ip, port, reserved) {
                Ok(listener) => return Ok(listener),
                Err(BindFailure::Taken(_)) => continue,
                Err(BindFailure::Other(err)) => return Err(err),
            }
        }
    }
    for _ in 0..16 {
        let listener = try_bind(ip, 0, reserved).map_err(|failure| match failure {
            BindFailure::Taken(reason) => port_in_use(0, reason, None),
            BindFailure::Other(err) => err,
        })?;
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        if !reserved.contains(&port) {
            return Ok(listener);
        }
    }
    Err(RpcError::new(ErrorKind::Internal, "no free local port"))
}

async fn bind(
    ip: IpAddr,
    requested: Option<u16>,
    remote_port: u16,
    reserved: HashSet<u16>,
) -> Result<TcpListener, RpcError> {
    let listener =
        tokio::task::spawn_blocking(move || bind_local(ip, requested, remote_port, &reserved))
            .await
            .map_err(|err| RpcError::new(ErrorKind::Internal, err.to_string()))??;
    listener
        .set_nonblocking(true)
        .and_then(|()| TcpListener::from_std(listener))
        .map_err(|err| RpcError::new(ErrorKind::Internal, err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    #[test]
    fn bind_defaults_to_loopback_and_guards_public() {
        assert_eq!(parse_bind(None, false).unwrap(), LOOPBACK);
        assert_eq!(parse_bind(Some("localhost"), false).unwrap(), LOOPBACK);
        assert_eq!(
            parse_bind(Some("[::1]"), false).unwrap(),
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        );
        let err = parse_bind(Some("0.0.0.0"), false).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidParams);
        assert!(err.data.hint.unwrap().contains("allow_public"));
        assert!(parse_bind(Some("0.0.0.0"), true).unwrap().is_unspecified());
        assert!(parse_bind(Some("example.com"), true).is_err());
    }

    #[test]
    fn urls_point_at_this_machine() {
        assert_eq!(local_url("127.0.0.1", 8080), "http://127.0.0.1:8080/");
        assert_eq!(local_url("0.0.0.0", 80), "http://127.0.0.1:80/");
        assert_eq!(local_url("::", 80), "http://[::1]:80/");
        assert_eq!(local_url("::1", 80), "http://[::1]:80/");
    }

    #[test]
    fn names_and_remote_hosts_are_cleaned() {
        assert_eq!(clean_name(Some("  web  ".into())), Some("web".into()));
        assert_eq!(clean_name(Some("   ".into())), None);
        assert_eq!(check_remote_host(" db.internal ").unwrap(), "db.internal");
        assert_eq!(check_remote_host("[::1]").unwrap(), "::1");
        assert!(check_remote_host("a b").is_err());
        assert!(check_remote_host("").is_err());
        assert!(check_port("remote_port", 0).is_err());
    }

    #[test]
    fn taken_ports_are_reported_with_a_suggestion() {
        let taken = std::net::TcpListener::bind((LOOPBACK, 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        let err = bind_local(LOOPBACK, Some(port), 80, &HashSet::new()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PortInUse);
        let details = err.data.details.unwrap();
        assert_eq!(details["port"], port);
        let suggested = details["suggested_port"].as_u64().unwrap() as u16;
        assert_ne!(suggested, port);
        assert!(bind_local(LOOPBACK, Some(suggested), 80, &HashSet::new()).is_ok());

        let free = std::net::TcpListener::bind((LOOPBACK, 0)).unwrap();
        let reserved_port = free.local_addr().unwrap().port();
        drop(free);
        let reserved = HashSet::from([reserved_port]);
        let err = bind_local(LOOPBACK, Some(reserved_port), 80, &reserved).unwrap_err();
        assert!(err.message.contains("another tunnel"));
    }

    #[test]
    fn automatic_ports_prefer_the_remote_port() {
        let taken = std::net::TcpListener::bind((LOOPBACK, 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        if (1024..u16::MAX - AUTO_PORT_RANGE).contains(&port) {
            let listener = bind_local(LOOPBACK, None, port, &HashSet::new()).unwrap();
            let chosen = listener.local_addr().unwrap().port();
            assert!(chosen > port && chosen <= port + AUTO_PORT_RANGE);
        }
        let listener = bind_local(LOOPBACK, None, 22, &HashSet::new()).unwrap();
        assert_ne!(listener.local_addr().unwrap().port(), 22);
    }

    #[tokio::test]
    async fn pump_copies_both_ways_and_counts_bytes() {
        let (local, mut client) = tokio::io::duplex(1024);
        let (remote, mut server) = tokio::io::duplex(1024);
        let stats = Arc::new(Stats::default());
        let counters = stats.clone();
        let pumping = tokio::spawn(async move { pump(local, remote, &counters).await });

        client.write_all(b"GET / HTTP/1.0\r\n\r\n").await.unwrap();
        let mut request = [0u8; 18];
        server.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"GET / HTTP/1.0\r\n\r\n");
        server
            .write_all(b"HTTP/1.0 200 OK\r\n\r\nhello")
            .await
            .unwrap();
        server.shutdown().await.unwrap();
        drop(server);
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert_eq!(response, b"HTTP/1.0 200 OK\r\n\r\nhello");
        client.shutdown().await.unwrap();
        drop(client);
        pumping.await.unwrap();

        let (_, _, bytes_in, bytes_out) = stats.read();
        assert_eq!(bytes_out, 18);
        assert_eq!(bytes_in, 24);
    }
}
