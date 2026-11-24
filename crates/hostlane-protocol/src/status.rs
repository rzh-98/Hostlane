use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Result of `daemon.status`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DaemonStatus {
    pub version: String,
    pub protocol: u32,
    pub pid: u32,
    /// Unix time in milliseconds.
    #[ts(type = "number")]
    pub started_at: u64,
    #[ts(type = "number")]
    pub uptime_ms: u64,
    pub socket_path: String,
    pub data_dir: String,
}
