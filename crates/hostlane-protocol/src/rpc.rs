use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::RpcError;

const JSONRPC_VERSION: &str = "2.0";

/// Who is calling. Sent as a top-level `client` member of each request (a Hostlane
/// extension to JSON-RPC) and recorded in the audit log and session monitor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ClientInfo {
    /// e.g. `cli`, `desktop`, or the value of `HOSTLANE_CLIENT`.
    pub name: String,
    /// Groups calls from one agent session (`HOSTLANE_SESSION`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub session: Option<String>,
}

impl ClientInfo {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            session: None,
        }
    }
}

impl Default for ClientInfo {
    fn default() -> Self {
        Self::new("unknown")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<ClientInfo>,
}

impl Request {
    pub fn new(id: impl Into<Value>, method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            id: id.into(),
            method: method.into(),
            params,
            client: None,
        }
    }

    pub fn with_client(mut self, client: ClientInfo) -> Self {
        self.client = Some(client);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, error: RpcError) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// Server-to-client message without an id; used by `events.subscribe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
}

impl Notification {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            method: method.into(),
            params,
        }
    }
}
