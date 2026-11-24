//! Connection pool: one SSH connection per host, opened on first use, shared by
//! every call, closed after it has been idle for a while.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hostlane_protocol::{Connection, ConnectionState, ErrorKind, Event, HostKeyInfo, RpcError};
use russh::client::Msg;
use russh::{Channel, Disconnect};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::events::EventBus;
use crate::ssh::{self, Locations, SshHandle, Trust};
use crate::ssh_config::{Entry, Loaded};
use crate::util::now_ms;

/// Concurrent channels per connection; OpenSSH's default `MaxSessions` is 10.
const MAX_CHANNELS: usize = 8;
const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(30);
const PING_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_JUMPS: usize = 8;

pub struct Conn {
    pub handle: SshHandle,
    channels: Arc<Semaphore>,
    /// Keeps the jump host's connection open while this one uses it.
    _via: Option<Arc<Conn>>,
}

struct Slot {
    /// Held while connecting, so concurrent callers share one attempt.
    conn: tokio::sync::Mutex<Option<Arc<Conn>>>,
    info: Mutex<Connection>,
}

impl Slot {
    fn new(host: &str) -> Self {
        Self {
            conn: tokio::sync::Mutex::new(None),
            info: Mutex::new(Connection {
                host: host.to_string(),
                state: ConnectionState::Disconnected,
                connected_at: None,
                last_used_at: None,
                latency_ms: None,
                active_channels: 0,
                host_key: None,
                error: None,
            }),
        }
    }

    fn snapshot(&self) -> Connection {
        self.info.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

pub struct Pool {
    locations: Locations,
    events: EventBus,
    slots: Mutex<HashMap<String, Arc<Slot>>>,
}

/// A connection plus one of its channel slots; released on drop.
pub struct Lease {
    pub conn: Arc<Conn>,
    _permit: OwnedSemaphorePermit,
    slot: Arc<Slot>,
    events: EventBus,
}

impl Lease {
    pub async fn open_session(&self) -> Result<Channel<Msg>, RpcError> {
        self.conn
            .handle
            .channel_open_session()
            .await
            .map_err(|err| {
                RpcError::new(
                    ErrorKind::ConnectionLost,
                    format!("cannot open a channel: {err}"),
                )
            })
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let snapshot = {
            let mut info = self.slot.info.lock().unwrap_or_else(|e| e.into_inner());
            info.active_channels = info.active_channels.saturating_sub(1);
            info.last_used_at = Some(now_ms());
            info.clone()
        };
        self.events.publish(Event::Connection {
            connection: snapshot,
        });
    }
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
type Connected = Result<(Arc<Conn>, HostKeyInfo, bool), RpcError>;

/// Resolves the last hop of a `ProxyJump` value into an entry that is itself
/// reached through the earlier hops.
fn jump_entry(spec: &str, loaded: &Loaded) -> Entry {
    let hops: Vec<&str> = spec
        .split(',')
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .collect();
    let last = hops.last().copied().unwrap_or(spec);
    let bare = last.strip_prefix("ssh://").unwrap_or(last);
    let (user, rest) = match bare.rsplit_once('@') {
        Some((user, rest)) => (Some(user.to_string()), rest),
        None => (None, bare),
    };
    let (name, port) = match rest.rsplit_once(':') {
        Some((name, port)) if port.parse::<u16>().is_ok() => (name, port.parse().ok()),
        _ => (rest, None),
    };
    let mut entry = loaded
        .find(name)
        .cloned()
        .unwrap_or_else(|| loaded.resolve_name(name));
    let customised = user.is_some() || port.is_some() || hops.len() > 1;
    if let Some(user) = user {
        entry.host.user = Some(user);
    }
    if let Some(port) = port {
        entry.host.port = port;
    }
    if hops.len() > 1 {
        entry.host.proxy_jump = Some(hops[..hops.len() - 1].join(","));
    }
    if customised {
        entry.host.id = hops.join(",");
    }
    entry
}

impl Pool {
    pub fn new(locations: Locations, events: EventBus) -> Self {
        Self {
            locations,
            events,
            slots: Mutex::new(HashMap::new()),
        }
    }

    pub fn locations(&self) -> &Locations {
        &self.locations
    }

    fn slot(&self, host: &str) -> Arc<Slot> {
        self.slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(host.to_string())
            .or_insert_with(|| Arc::new(Slot::new(host)))
            .clone()
    }

    fn update(&self, slot: &Slot, change: impl FnOnce(&mut Connection)) {
        let snapshot = {
            let mut info = slot.info.lock().unwrap_or_else(|e| e.into_inner());
            change(&mut info);
            info.clone()
        };
        self.events.publish(Event::Connection {
            connection: snapshot,
        });
    }

    /// Returns the pooled connection for `entry`, connecting if needed.
    pub async fn connect(&self, entry: &Entry, loaded: &Loaded, trust: Trust) -> Connected {
        self.connect_at(entry, loaded, trust, 0).await
    }

    fn connect_at<'a>(
        &'a self,
        entry: &'a Entry,
        loaded: &'a Loaded,
        trust: Trust,
        depth: usize,
    ) -> BoxFuture<'a, Connected> {
        Box::pin(async move {
            let host = entry.host.id.clone();
            let slot = self.slot(&host);
            let mut guard = slot.conn.lock().await;
            if let Some(conn) = guard.as_ref()
                && !conn.handle.is_closed()
            {
                let host_key = slot.snapshot().host_key;
                if let Some(host_key) = host_key {
                    return Ok((conn.clone(), host_key, false));
                }
            }
            *guard = None;
            self.update(&slot, |info| {
                info.state = ConnectionState::Connecting;
                info.active_channels = 0;
            });

            let result = self.establish(entry, loaded, trust, depth).await;
            match result {
                Ok((established, via)) => {
                    let conn = Arc::new(Conn {
                        handle: established.handle,
                        channels: Arc::new(Semaphore::new(MAX_CHANNELS)),
                        _via: via,
                    });
                    *guard = Some(conn.clone());
                    let host_key = established.host_key.clone();
                    self.update(&slot, |info| {
                        let now = now_ms();
                        info.state = ConnectionState::Connected;
                        info.connected_at = Some(now);
                        info.last_used_at = Some(now);
                        info.latency_ms = Some(established.transport_ms);
                        info.host_key = Some(established.host_key);
                        info.error = None;
                    });
                    tracing::info!(host, "connected");
                    Ok((conn, host_key, established.learned))
                }
                Err(error) => {
                    self.update(&slot, |info| {
                        info.state = ConnectionState::Failed;
                        info.connected_at = None;
                        info.error = Some(error.clone());
                        if let Some(details) = &error.data.details
                            && let Ok(key) = serde_json::from_value(details.clone())
                        {
                            info.host_key = Some(key);
                        }
                    });
                    tracing::info!(host, error = %error, "connection failed");
                    Err(error)
                }
            }
        })
    }

    async fn establish(
        &self,
        entry: &Entry,
        loaded: &Loaded,
        trust: Trust,
        depth: usize,
    ) -> Result<(ssh::Established, Option<Arc<Conn>>), RpcError> {
        let Some(spec) = &entry.host.proxy_jump else {
            let established =
                ssh::connect::<tokio::net::TcpStream>(entry, &self.locations, trust, None).await?;
            return Ok((established, None));
        };
        if depth >= MAX_JUMPS {
            return Err(RpcError::new(
                ErrorKind::InvalidParams,
                format!(
                    "ProxyJump chain of `{}` is too long or loops",
                    entry.host.id
                ),
            ));
        }
        let hop = jump_entry(spec, loaded);
        let (via, _, _) = self
            .connect_at(&hop, loaded, Trust::Strict, depth + 1)
            .await
            .map_err(|err| {
                let message = format!("jump host `{}`: {}", hop.host.id, err.message);
                RpcError { message, ..err }
            })?;
        let channel = via
            .handle
            .channel_open_direct_tcpip(
                entry.host.hostname.clone(),
                entry.host.port.into(),
                "127.0.0.1",
                0,
            )
            .await
            .map_err(|err| {
                RpcError::new(
                    ErrorKind::HostUnreachable,
                    format!(
                        "jump host `{}` cannot reach {}:{}: {err}",
                        hop.host.id, entry.host.hostname, entry.host.port
                    ),
                )
            })?;
        let established =
            ssh::connect(entry, &self.locations, trust, Some(channel.into_stream())).await?;
        Ok((established, Some(via)))
    }

    /// Connects if needed and reserves a channel slot.
    pub async fn lease(&self, entry: &Entry, loaded: &Loaded) -> Result<Lease, RpcError> {
        let (conn, _, _) = self.connect(entry, loaded, Trust::Strict).await?;
        let permit = conn
            .channels
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| RpcError::new(ErrorKind::ConnectionLost, "connection closed"))?;
        let slot = self.slot(&entry.host.id);
        self.update(&slot, |info| {
            info.active_channels += 1;
            info.last_used_at = Some(now_ms());
        });
        Ok(Lease {
            conn,
            _permit: permit,
            slot,
            events: self.events.clone(),
        })
    }

    /// Closes the connection to `host`. Returns false if there was none.
    pub async fn disconnect(&self, host: &str) -> bool {
        let slot = self
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(host)
            .cloned();
        let Some(slot) = slot else {
            return false;
        };
        let conn = slot.conn.lock().await.take();
        let was_open = conn.as_ref().is_some_and(|c| !c.handle.is_closed());
        if let Some(conn) = conn {
            let _ = conn
                .handle
                .disconnect(Disconnect::ByApplication, "", "en")
                .await;
        }
        self.update(&slot, |info| {
            info.state = ConnectionState::Disconnected;
            info.connected_at = None;
            info.latency_ms = None;
            info.active_channels = 0;
        });
        was_open
    }

    pub fn list(&self) -> Vec<Connection> {
        let slots: Vec<(String, Arc<Slot>)> = self
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut list: Vec<Connection> = slots
            .iter()
            .map(|(host, slot)| {
                self.reap_closed(host, slot);
                slot.snapshot()
            })
            .collect();
        list.sort_by(|a, b| a.host.cmp(&b.host));
        list
    }

    /// Forgets the host's connection if the server has closed it.
    pub fn forget_if_closed(&self, host: &str) {
        let slot = self
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(host)
            .cloned();
        if let Some(slot) = slot {
            self.reap_closed(host, &slot);
        }
    }

    fn reap_closed(&self, host: &str, slot: &Slot) -> bool {
        let Ok(mut guard) = slot.conn.try_lock() else {
            return false;
        };
        if !guard.as_ref().is_some_and(|conn| conn.handle.is_closed()) {
            return false;
        }
        *guard = None;
        drop(guard);
        tracing::info!(host, "connection dropped");
        self.update(slot, |info| {
            info.state = ConnectionState::Disconnected;
            info.connected_at = None;
            info.latency_ms = None;
            info.active_channels = 0;
        });
        true
    }

    pub async fn disconnect_all(&self) {
        let hosts: Vec<String> = self
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        for host in hosts {
            self.disconnect(&host).await;
        }
    }

    /// Measures latency, notices dropped connections and closes idle ones. Runs forever.
    pub async fn maintain(self: Arc<Self>) {
        let mut interval = tokio::time::interval(MAINTENANCE_INTERVAL);
        interval.tick().await;
        loop {
            interval.tick().await;
            let slots: Vec<(String, Arc<Slot>)> = self
                .slots
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            for (host, slot) in slots {
                self.maintain_slot(&host, &slot).await;
            }
        }
    }

    async fn maintain_slot(&self, host: &str, slot: &Arc<Slot>) {
        if self.reap_closed(host, slot) {
            return;
        }
        let Ok(guard) = slot.conn.try_lock() else {
            return;
        };
        let Some(conn) = guard.clone() else {
            return;
        };
        let idle_since = slot.snapshot().last_used_at.unwrap_or_default();
        // The slot and `conn` hold two references; more means a lease or a jump client.
        let in_use = Arc::strong_count(&conn) > 2;
        if !in_use && now_ms().saturating_sub(idle_since) > IDLE_TIMEOUT.as_millis() as u64 {
            drop(guard);
            drop(conn);
            tracing::info!(host, "closing idle connection");
            self.disconnect(host).await;
            return;
        }
        drop(guard);
        let started = std::time::Instant::now();
        match tokio::time::timeout(PING_TIMEOUT, conn.handle.send_ping()).await {
            Ok(Ok(())) => {
                let latency = started.elapsed().as_millis() as u64;
                self.update(slot, |info| info.latency_ms = Some(latency));
            }
            _ => tracing::debug!(host, "ping failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn jump_specs_resolve_through_config() {
        let loaded = crate::ssh_config::parse(
            "Host bastion\n  HostName 1.2.3.4\n  User ops\nHost *\n  Port 2200\n",
            Path::new("/nonexistent"),
        );
        let hop = jump_entry("bastion", &loaded);
        assert_eq!(hop.host.id, "bastion");
        assert_eq!(hop.host.hostname, "1.2.3.4");

        let hop = jump_entry("first,admin@other:2222", &loaded);
        assert_eq!(hop.host.id, "first,admin@other:2222");
        assert_eq!(hop.host.hostname, "other");
        assert_eq!(hop.host.user.as_deref(), Some("admin"));
        assert_eq!(hop.host.port, 2222);
        assert_eq!(hop.host.proxy_jump.as_deref(), Some("first"));

        let hop = jump_entry("plain", &loaded);
        assert_eq!(hop.host.port, 2200);
    }
}
