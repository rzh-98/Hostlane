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
            data: RpcErrorData { kind, hint: None },
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.data.hint = Some(hint.into());
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
