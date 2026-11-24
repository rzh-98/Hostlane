use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use hostlane_protocol::{DaemonStatus, PROTOCOL_VERSION};
use tokio::sync::Notify;

use crate::paths;

/// Shared state of one running daemon.
pub struct Daemon {
    started: Instant,
    started_at: u64,
    pub(crate) socket_path: PathBuf,
    pub(crate) ssh_config_path: PathBuf,
    pub(crate) shutdown: Notify,
}

impl Daemon {
    pub fn new(socket_path: PathBuf) -> Self {
        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or_default();
        Self {
            started: Instant::now(),
            started_at,
            socket_path,
            ssh_config_path: paths::ssh_config_path(),
            shutdown: Notify::new(),
        }
    }

    pub fn with_ssh_config(mut self, path: PathBuf) -> Self {
        self.ssh_config_path = path;
        self
    }

    pub fn status(&self) -> DaemonStatus {
        DaemonStatus {
            version: crate::VERSION.into(),
            protocol: PROTOCOL_VERSION,
            pid: std::process::id(),
            started_at: self.started_at,
            uptime_ms: self.started.elapsed().as_millis() as u64,
            socket_path: self.socket_path.display().to_string(),
            data_dir: paths::data_dir().display().to_string(),
        }
    }

    /// Asks the accept loop to stop; the socket file is removed on the way out.
    pub fn request_shutdown(&self) {
        self.shutdown.notify_one();
    }
}
