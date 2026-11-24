use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ClientInfo;

/// Params of `exec.run`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExecParams {
    pub host: String,
    /// Run by the remote user's login shell, like `ssh host '<command>'`.
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub env: Option<BTreeMap<String, String>>,
    /// Sent to the command's standard input, then closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stdin: Option<String>,
    /// Defaults to 120 000 (2 minutes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub timeout_ms: Option<u64>,
    /// Per stream; longer output keeps its head and tail. Defaults to 16 384.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub max_output_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ExecStatus {
    Running,
    /// Finished with an exit code (see `exit_code`) or a signal (see `signal`).
    Exited,
    TimedOut,
    /// Could not start or lost the connection; see `error`.
    Failed,
    /// The daemon stopped while the command was running.
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

/// One command run by `exec.run`, as shown in the session monitor.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExecRecord {
    /// Also the `output_id` for `output.read`.
    pub id: String,
    pub host: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cwd: Option<String>,
    pub client: ClientInfo,
    pub status: ExecStatus,
    #[ts(type = "number")]
    pub started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub finished_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub signal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    #[ts(type = "number")]
    pub stdout_bytes: u64,
    #[ts(type = "number")]
    pub stderr_bytes: u64,
}

/// Result of `exec.run`. A non-zero exit code or a timeout is a result, not an error.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExecResult {
    pub host: String,
    pub status: ExecStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub signal: Option<String>,
    #[ts(type = "number")]
    pub duration_ms: u64,
    pub stdout: String,
    pub stderr: String,
    /// Full sizes; larger than `stdout` / `stderr` when `truncated`.
    #[ts(type = "number")]
    pub stdout_bytes: u64,
    #[ts(type = "number")]
    pub stderr_bytes: u64,
    /// The middle of a stream was cut; read it with `output.read`.
    pub truncated: bool,
    pub output_id: String,
}

/// Params of `exec.list`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExecListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub host: Option<String>,
    /// Only records that started before this time (Unix ms), for paging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub before: Option<u64>,
    /// Defaults to 50, at most 500.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub limit: Option<u32>,
}

/// Result of `exec.list`, newest first.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExecList {
    pub execs: Vec<ExecRecord>,
}

/// Params of `output.read`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OutputReadParams {
    pub output_id: String,
    pub stream: OutputStream,
    /// Byte offset; negative counts from the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub offset: Option<i64>,
    /// Defaults to 65 536, at most 1 MiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub limit: Option<u64>,
}

/// Result of `output.read`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OutputChunk {
    pub data: String,
    #[ts(type = "number")]
    pub offset: u64,
    /// Offset to pass next; equals `total_bytes` at the end.
    #[ts(type = "number")]
    pub next_offset: u64,
    /// Bytes stored for this stream.
    #[ts(type = "number")]
    pub total_bytes: u64,
    /// The stream was longer than the storage cap; the rest was dropped.
    pub capped: bool,
}
