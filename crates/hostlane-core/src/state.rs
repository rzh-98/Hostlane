use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hostlane_protocol::{DaemonStatus, PROTOCOL_VERSION, RpcError};
use tokio::sync::Notify;

use crate::catalog::Catalog;
use crate::clients::Clients;
use crate::events::EventBus;
use crate::jobs::Jobs;
use crate::policy::Policy;
use crate::pool::Pool;
use crate::secrets::Secrets;
use crate::ssh::Locations;
use crate::ssh_config::{Entry, Loaded};
use crate::store::Store;
use crate::tunnels::Tunnels;
use crate::util::now_ms;

/// Stored command output and file backups are kept this long.
const OUTPUT_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Debug, Clone)]
pub struct DaemonOptions {
    pub socket_path: PathBuf,
    pub data_dir: PathBuf,
    pub ssh_config_path: PathBuf,
}

impl DaemonOptions {
    /// Paths from the environment and platform defaults (see [`crate::paths`]).
    pub fn from_env(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            data_dir: crate::paths::data_dir(),
            ssh_config_path: crate::paths::ssh_config_path(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("cannot create {0}: {1}")]
    Dir(String, std::io::Error),
    #[error("cannot open the database: {0}")]
    Database(#[from] rusqlite::Error),
}

/// Shared state of one running daemon.
pub struct Daemon {
    started: Instant,
    started_at: u64,
    pub(crate) socket_path: PathBuf,
    pub(crate) data_dir: PathBuf,
    pub(crate) catalog: Catalog,
    pub(crate) secrets: Arc<Secrets>,
    pub(crate) outputs_dir: PathBuf,
    pub(crate) backups_dir: PathBuf,
    pub(crate) store: Store,
    pub(crate) events: EventBus,
    pub(crate) pool: Arc<Pool>,
    pub(crate) tunnels: Arc<Tunnels>,
    pub(crate) policy: Arc<Policy>,
    pub(crate) jobs: Arc<Jobs>,
    pub(crate) clients: Clients,
    pub(crate) shutdown: Notify,
}

fn private_dir(path: &PathBuf) -> Result<(), OpenError> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|err| OpenError::Dir(path.display().to_string(), err))
}

impl Daemon {
    pub fn open(options: DaemonOptions) -> Result<Self, OpenError> {
        private_dir(&options.data_dir)?;
        let outputs_dir = options.data_dir.join("outputs");
        private_dir(&outputs_dir)?;
        let backups_dir = options.data_dir.join("backups");
        private_dir(&backups_dir)?;
        let store = Store::open(&options.data_dir.join("hostlane.db"))?;
        let events = EventBus::default();
        let ssh_dir = options
            .ssh_config_path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_default();
        let secrets = Arc::new(Secrets::from_env(&options.data_dir));
        let locations = Locations {
            ssh_dir,
            hostlane_known_hosts: options.data_dir.join("known_hosts"),
            secrets: secrets.clone(),
        };
        let catalog = Catalog::new(options.ssh_config_path.clone(), store.clone());
        let pool = Arc::new(Pool::new(locations, events.clone()));
        let tunnels = Arc::new(Tunnels::new(
            pool.clone(),
            events.clone(),
            store.clone(),
            catalog.clone(),
        ));
        let policy = Arc::new(Policy::new(store.clone(), events.clone()));
        let jobs = Arc::new(Jobs::new(
            pool.clone(),
            catalog.clone(),
            store.clone(),
            events.clone(),
        ));
        let clients = Clients::new(store.clone());
        Ok(Self {
            started: Instant::now(),
            started_at: now_ms(),
            socket_path: options.socket_path,
            data_dir: options.data_dir,
            catalog,
            secrets,
            outputs_dir,
            backups_dir,
            store,
            pool,
            tunnels,
            policy,
            jobs,
            clients,
            events,
            shutdown: Notify::new(),
        })
    }

    /// Recovers state left by a previous run and starts background maintenance.
    pub async fn start(&self) {
        match self.store.recover_interrupted(now_ms()).await {
            Ok(0) => {}
            Ok(count) => tracing::warn!(
                count,
                "marked commands from the previous run as interrupted"
            ),
            Err(err) => tracing::warn!(%err, "cannot recover previous commands"),
        }
        let outputs = self.outputs_dir.clone();
        let backups = self.backups_dir.clone();
        tokio::task::spawn_blocking(move || {
            crate::exec::prune_outputs(&outputs, OUTPUT_RETENTION);
            crate::exec::prune_outputs(&backups, OUTPUT_RETENTION);
        });
        tokio::spawn(self.pool.clone().maintain());
        self.policy.load().await;
        self.clients.load().await;
        tokio::spawn(self.jobs.clone().maintain());
        tokio::spawn(self.policy.clone().maintain());
        self.tunnels.restore(self.policy.is_stopped()).await;
        tokio::spawn(self.tunnels.clone().maintain());
    }

    pub fn status(&self) -> DaemonStatus {
        DaemonStatus {
            version: crate::VERSION.into(),
            protocol: PROTOCOL_VERSION,
            pid: std::process::id(),
            started_at: self.started_at,
            uptime_ms: self.started.elapsed().as_millis() as u64,
            socket_path: self.socket_path.display().to_string(),
            data_dir: self.data_dir.display().to_string(),
        }
    }

    /// Hosts from ssh_config and those added in Hostlane, with labels applied.
    pub(crate) async fn load_hosts(&self) -> Result<Loaded, RpcError> {
        self.catalog.load().await
    }

    /// Finds a host by id, returning it with the catalog it came from.
    pub(crate) async fn resolve_host(&self, id: &str) -> Result<(Entry, Loaded), RpcError> {
        self.catalog.resolve(id).await
    }

    /// Asks the accept loop to stop; the socket file is removed on the way out.
    pub fn request_shutdown(&self) {
        self.shutdown.notify_one();
    }

    /// Closes SSH connections before the process exits.
    pub async fn close(&self) {
        self.tunnels.stop_all().await;
        self.pool.disconnect_all().await;
    }
}
