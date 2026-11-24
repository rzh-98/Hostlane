use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::{ClientInfo, ErrorKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AuditOutcome {
    Ok,
    Error,
    /// The daemon stopped before the call finished.
    Interrupted,
}

/// One audited call. The log is append-only.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AuditEntry {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub started_at: u64,
    #[ts(type = "number")]
    pub finished_at: u64,
    pub client: ClientInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub host: Option<String>,
    pub method: String,
    /// Call params; `stdin` is replaced by its size.
    #[ts(type = "unknown")]
    pub params: Value,
    pub outcome: AuditOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_kind: Option<ErrorKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_message: Option<String>,
    /// Summary of the result, e.g. exit code and output id for `exec.run`.
    #[ts(type = "unknown")]
    pub result: Value,
    /// Who approved the call (M5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub approver: Option<String>,
}

/// Params of `audit.list`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AuditListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub host: Option<String>,
    /// Client name, e.g. `cli`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub client: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub method: Option<String>,
    /// Unix ms, inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub since: Option<u64>,
    /// Unix ms, exclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub until: Option<u64>,
    /// Only entries with a smaller id, for paging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub before_id: Option<i64>,
    /// Defaults to 100, at most 1000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub limit: Option<u32>,
}

/// Result of `audit.list`, newest first.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AuditList {
    pub entries: Vec<AuditEntry>,
    pub has_more: bool,
}
