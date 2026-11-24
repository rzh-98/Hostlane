use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ExecResult, ServiceKind};

/// Params of `services.logs`: the last lines of a systemd unit's journal or a
/// container's log, stdout and stderr together.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ServiceLogsParams {
    pub host: String,
    pub kind: ServiceKind,
    /// Unit name (`nginx`, `nginx.service`) or container name or id.
    pub name: String,
    /// A systemd user unit (`journalctl --user`).
    #[serde(default)]
    pub user_unit: bool,
    /// Number of lines, default 200, at most 10000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub lines: Option<u32>,
    /// Only newer entries, e.g. `10m`, `2h`, `1d` (`journalctl --since`, `docker logs --since`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub since: Option<String>,
    /// Run with `sudo -n`. Defaults to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sudo: Option<bool>,
}

/// Params of `services.start`, `services.stop` and `services.restart`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ServiceControlParams {
    pub host: String,
    pub kind: ServiceKind,
    pub name: String,
    #[serde(default)]
    pub user_unit: bool,
    /// Run with `sudo -n`. Defaults to true for system units when the SSH user is not
    /// root, false otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sudo: Option<bool>,
}

/// Params of `processes.kill`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProcessKillParams {
    pub host: String,
    pub pid: u32,
    /// `TERM` (default), `INT`, `HUP`, `KILL`, `QUIT`, `USR1` or `USR2`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub signal: Option<String>,
    /// Run with `sudo -n`. Defaults to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sudo: Option<bool>,
}

/// Result of `services.*` and `processes.kill`: the command Hostlane ran and its result.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ServiceCommandResult {
    pub command: String,
    #[serde(flatten)]
    pub result: ExecResult,
    /// Set when the command failed in a recognizable way, e.g. sudo needs a password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hint: Option<String>,
}
