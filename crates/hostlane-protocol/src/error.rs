use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Machine-readable error category. Agents branch on this, not on `message`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[ts(export)]
pub enum ErrorKind {
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    Internal,
    NotImplemented,
    HostNotFound,
    HostUnreachable,
    ApprovalRequired,
    PolicyDenied,
    FileChanged,
    /// The server's host key is not in any known_hosts file yet; see `details`.
    HostKeyUnknown,
    /// The server presented a different key than the one on record. Never bypassed.
    HostKeyChanged,
    AuthFailed,
    /// The SSH connection dropped while a call was using it.
    ConnectionLost,
    OutputNotFound,
    /// The remote file or directory does not exist.
    FileNotFound,
    /// The SSH user may not read or write the remote path.
    PermissionDenied,
    /// An edit or diff hunk did not match the file; `details` says which one.
    PatchFailed,
    /// Another SFTP failure; `message` carries the server's reason.
    FileError,
}

impl ErrorKind {
    /// JSON-RPC numeric code. Standard codes for protocol errors, -32000.. for Hostlane errors.
    pub fn code(self) -> i64 {
        match self {
            Self::ParseError => -32700,
            Self::InvalidRequest => -32600,
            Self::MethodNotFound => -32601,
            Self::InvalidParams => -32602,
            Self::Internal => -32603,
            Self::NotImplemented => -32000,
            Self::HostNotFound => -32001,
            Self::HostUnreachable => -32002,
            Self::ApprovalRequired => -32003,
            Self::PolicyDenied => -32004,
            Self::FileChanged => -32005,
            Self::HostKeyUnknown => -32006,
            Self::HostKeyChanged => -32007,
            Self::AuthFailed => -32008,
            Self::ConnectionLost => -32009,
            Self::OutputNotFound => -32010,
            Self::FileNotFound => -32011,
            Self::PermissionDenied => -32012,
            Self::PatchFailed => -32013,
            Self::FileError => -32014,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RpcErrorData {
    pub kind: ErrorKind,
    /// What the caller can do next, written for an agent to act on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hint: Option<String>,
    /// Structured context for some kinds, e.g. a [`crate::HostKeyInfo`] for host key errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "Record<string, unknown>")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RpcError {
    #[ts(type = "number")]
    pub code: i64,
    pub message: String,
    pub data: RpcErrorData,
}

impl RpcError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            code: kind.code(),
            message: message.into(),
            data: RpcErrorData {
                kind,
                hint: None,
                details: None,
            },
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.data.hint = Some(hint.into());
        self
    }

    pub fn with_details(mut self, details: impl serde::Serialize) -> Self {
        self.data.details = serde_json::to_value(details).ok();
        self
    }

    pub fn kind(&self) -> ErrorKind {
        self.data.kind
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RpcError {}
