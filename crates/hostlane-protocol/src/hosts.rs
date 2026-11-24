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
