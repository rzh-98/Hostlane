use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HostEnv {
    Dev,
    Staging,
    Prod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HostSource {
    /// Imported from `~/.ssh/config` (read-only; edit the file to change it).
    SshConfig,
    /// Added in Hostlane.
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Host {
    /// Stable id used by every other method, e.g. `exec --host <id>`.
    pub id: String,
    /// Name shown to people; the `Host` alias for ssh_config entries.
    pub alias: String,
    pub hostname: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub user: Option<String>,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub proxy_jump: Option<String>,
    pub identity_files: Vec<String>,
    pub source: HostSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub env: Option<HostEnv>,
    pub tags: Vec<String>,
}

/// Result of `hosts.list`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HostsList {
    pub hosts: Vec<Host>,
    /// Problems found while reading host sources; the list is still usable.
    pub warnings: Vec<String>,
}

/// Params of `hosts.connect` and `hosts.disconnect`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HostParams {
    pub host: String,
}

/// Params of `hosts.trust`. The fingerprint must match the key the server presents now.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TrustParams {
    pub host: String,
    /// `SHA256:…`, as shown in a `HOST_KEY_UNKNOWN` error.
    pub fingerprint: String,
}

/// A server host key, attached to host key errors and trust results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HostKeyInfo {
    pub host: String,
    /// Name the key is recorded under in known_hosts, e.g. `[10.0.0.5]:2222`.
    pub known_hosts_name: String,
    /// e.g. `ssh-ed25519`.
    pub key_type: String,
    /// `SHA256:…`.
    pub fingerprint: String,
    /// For `HOST_KEY_CHANGED`: where the old key is recorded, as `path:line`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub recorded_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Disconnected,
    Failed,
}

/// Pool state for one host.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Connection {
    pub host: String,
    pub state: ConnectionState,
    /// Unix time in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub connected_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub last_used_at: Option<u64>,
    /// Round trip of the latest SSH ping, or of the TCP handshake before the first ping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub latency_ms: Option<u64>,
    pub active_channels: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub host_key: Option<HostKeyInfo>,
    /// Last connection error, kept until the next successful connect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<crate::RpcError>,
}

/// Result of `connections.list`: hosts that have been used since the daemon started.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ConnectionsList {
    pub connections: Vec<Connection>,
}

/// Result of `hosts.trust`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TrustResult {
    pub host_key: HostKeyInfo,
    /// False when the key was already trusted.
    pub added: bool,
    /// File the key was written to.
    pub known_hosts_path: String,
}
