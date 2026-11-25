use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// An agent registered with `clients.create`. Its calls carry a token and appear in
/// the audit log under `name`, marked as verified.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RegisteredClient {
    pub name: String,
    #[ts(type = "number")]
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub last_used_at: Option<u64>,
}

/// Result of `clients.list`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ClientsList {
    pub clients: Vec<RegisteredClient>,
    /// Calls from agents (any name except `cli` and `desktop`) must carry a token.
    pub require_token: bool,
}

/// Params of `clients.create`, `clients.revoke`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ClientNameParams {
    pub name: String,
}

/// Result of `clients.create`. The token is shown only here; Hostlane keeps its hash.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ClientCreated {
    pub client: RegisteredClient,
    /// Give it to the agent as `HOSTLANE_TOKEN`.
    pub token: String,
}

/// Params of `clients.configure`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ClientsConfigureParams {
    pub require_token: bool,
}
