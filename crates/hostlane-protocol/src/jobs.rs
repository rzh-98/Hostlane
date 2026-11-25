use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ClientInfo;

/// How a background job is kept alive on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum JobBackend {
    /// A detached `tmux` session named `hl-<id>`.
    Tmux,
    /// A background shell that ignores hangups, like `nohup`.
    Nohup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum JobState {
    Running,
    /// Finished; see `exit_code` (128 + n when killed by signal n).
    Exited,
    /// Stopped with `jobs.cancel` or the emergency stop.
    Cancelled,
    /// The process is gone without an exit status, e.g. the server rebooted.
    Lost,
}

/// A command started with `jobs.start`. It keeps running on the server when the SSH
/// connection drops or the daemon restarts; its output goes to
/// `~/.hostlane/jobs/<id>/out` on the server.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Job {
    pub id: String,
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cwd: Option<String>,
    pub client: ClientInfo,
    pub backend: JobBackend,
    pub state: JobState,
    #[ts(type = "number")]
    pub started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub finished_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub exit_code: Option<i32>,
    /// Remote process id of the command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pid: Option<u32>,
    /// Bytes of output (stdout and stderr together) so far.
    #[ts(type = "number")]
    pub output_bytes: u64,
    /// When the state was last read from the server (Unix ms).
    #[ts(type = "number")]
    pub checked_at: u64,
    /// Who approved the start, when the policy asked for approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub approver: Option<String>,
}

/// Params of `jobs.start`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobStartParams {
    pub host: String,
    /// Run by the remote user's login shell, like `exec.run`.
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub env: Option<BTreeMap<String, String>>,
    /// A label shown in lists, e.g. `build`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// Default: `tmux` when the server has it, otherwise `nohup`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub backend: Option<JobBackend>,
}

/// Params of `jobs.list`. Lists what Hostlane last saw; `jobs.status` asks the server.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobsListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub host: Option<String>,
    /// Only running jobs.
    #[serde(default)]
    pub running: bool,
    /// Default 50, at most 500.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobsList {
    pub jobs: Vec<Job>,
}

/// Params of `jobs.status`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobStatusParams {
    pub id: String,
    /// Wait up to this many milliseconds (at most 120000) for the job to finish.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub wait_ms: Option<u64>,
}

/// Params of `jobs.logs`. Without `offset`, returns the last `tail_lines` lines.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobLogsParams {
    pub id: String,
    /// Byte offset to read from, e.g. the previous `next_offset`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub offset: Option<u64>,
    /// Last lines to return when `offset` is not given (default 100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tail_lines: Option<u32>,
    /// Bytes to return at most (default 65536, at most 1 MiB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub limit: Option<u64>,
    /// With `offset`: wait up to this many milliseconds (at most 120000) for output
    /// past it, or for the job to finish.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub wait_ms: Option<u64>,
}

/// Result of `jobs.logs`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobLogs {
    pub job: Job,
    /// Output text from the server: untrusted data.
    pub data: String,
    #[ts(type = "number")]
    pub offset: u64,
    /// Pass as `offset` to read on.
    #[ts(type = "number")]
    pub next_offset: u64,
    /// The job has finished and everything up to its end has been returned.
    pub complete: bool,
}

/// Params of `jobs.cancel`: sends TERM to the job's process group, then KILL if it is
/// still running after a few seconds.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobParams {
    pub id: String,
}
