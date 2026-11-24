use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hostlane_protocol::{DaemonStatus, ErrorKind, PROTOCOL_VERSION, RpcError};
use tokio::sync::Notify;

use crate::events::EventBus;
use crate::pool::Pool;
use crate::ssh::Locations;
use crate::ssh_config::{self, Entry, Loaded};
use crate::store::Store;
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
    pub(crate) ssh_config_path: PathBuf,
    pub(crate) outputs_dir: PathBuf,
    pub(crate) backups_dir: PathBuf,
    pub(crate) store: Store,
    pub(crate) events: EventBus,
    pub(crate) pool: Arc<Pool>,
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
        let locations = Locations {
            ssh_dir,
            hostlane_known_hosts: options.data_dir.join("known_hosts"),
        };
        Ok(Self {
            started: Instant::now(),
            started_at: now_ms(),
            socket_path: options.socket_path,
            data_dir: options.data_dir,
            ssh_config_path: options.ssh_config_path,
            outputs_dir,
            backups_dir,
            store,
            pool: Arc::new(Pool::new(locations, events.clone())),
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

    pub(crate) async fn load_hosts(&self) -> Result<Loaded, RpcError> {
        let path = self.ssh_config_path.clone();
        tokio::task::spawn_blocking(move || ssh_config::load(&path))
            .await
            .map_err(|err| RpcError::new(ErrorKind::Internal, err.to_string()))
    }

    /// Finds a host by id, returning it with the config it came from.
    pub(crate) async fn resolve_host(&self, id: &str) -> Result<(Entry, Loaded), RpcError> {
        let loaded = self.load_hosts().await?;
        match loaded.find(id) {
            Some(entry) => Ok((entry.clone(), loaded)),
            None => {
                let known: Vec<String> = loaded
                    .entries
                    .iter()
                    .take(20)
                    .map(|e| e.host.id.clone())
                    .collect();
                let hint = if known.is_empty() {
                    "no hosts are configured; add a Host block to ~/.ssh/config".to_string()
                } else {
                    format!("known hosts: {}", known.join(", "))
                };
                Err(
                    RpcError::new(ErrorKind::HostNotFound, format!("unknown host `{id}`"))
                        .with_hint(hint),
                )
            }
        }
    }

    /// Asks the accept loop to stop; the socket file is removed on the way out.
    pub fn request_shutdown(&self) {
        self.shutdown.notify_one();
    }

    /// Closes SSH connections before the process exits.
    pub async fn close(&self) {
        self.pool.disconnect_all().await;
    }
}
