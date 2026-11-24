use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ClientInfo, RpcError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TunnelState {
    /// Listening locally and holding an SSH connection.
    Active,
    /// The SSH connection dropped; the local port stays bound while reconnecting.
    Reconnecting,
    /// Gave up: the local port is taken, the host key is not trusted, … See `error`.
    Failed,
    /// A saved tunnel that is not running.
    Stopped,
}

/// A local forward (`ssh -L`): `bind:local_port` on this machine →
/// `remote_host:remote_port` as seen from the server.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Tunnel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    pub host: String,
    /// Local address the listener is bound to; `127.0.0.1` unless explicitly widened.
    pub bind: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    /// Stored in the database and restored when the daemon starts.
    pub saved: bool,
    /// Started with the daemon (saved tunnels only).
    pub auto_start: bool,
    pub state: TunnelState,
    /// Why the tunnel failed or is reconnecting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<RpcError>,
    /// The last forwarded connection that could not be opened on the server, e.g. nothing
    /// listening on the remote port. Cleared by the next successful connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_connection_error: Option<String>,
    /// `http://<bind>:<local_port>/`, with `127.0.0.1` for wildcard binds.
    pub url: String,
    pub created_by: ClientInfo,
    #[ts(type = "number")]
    pub created_at: u64,
    /// Unix ms after which a temporary tunnel is closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub expires_at: Option<u64>,
    pub active_connections: u32,
    #[ts(type = "number")]
    pub total_connections: u64,
    /// Bytes sent from the server to local clients.
    #[ts(type = "number")]
    pub bytes_in: u64,
    /// Bytes sent from local clients to the server.
    #[ts(type = "number")]
    pub bytes_out: u64,
}

/// Params of `tunnels.open`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TunnelOpenParams {
    pub host: String,
    pub remote_port: u16,
    /// Address to connect to from the server. Defaults to `127.0.0.1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub remote_host: Option<String>,
    /// Defaults to `remote_port` when it is free (and ≥ 1024), otherwise a nearby free port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub local_port: Option<u16>,
    /// Local address to listen on. Defaults to `127.0.0.1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub bind: Option<String>,
    /// Required to bind anything other than a loopback address.
    #[serde(default)]
    pub allow_public: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// Keep the tunnel across daemon restarts. Unsaved tunnels opened by agents expire.
    #[serde(default)]
    pub save: bool,
    /// Start with the daemon. Defaults to true for saved tunnels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub auto_start: Option<bool>,
    /// Lifetime of a temporary tunnel, in seconds (max 86400). Defaults to 3600 for
    /// agents; tunnels opened from the desktop app do not expire unless this is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub ttl_secs: Option<u64>,
}

/// Result of `tunnels.open` and `tunnels.update`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TunnelOpened {
    pub tunnel: Tunnel,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Params of `tunnels.start` and `tunnels.delete`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TunnelParams {
    pub id: String,
}

/// Params of `tunnels.close`: one tunnel by `id`, or every temporary tunnel opened in
/// `session` (call this when an agent session ends).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TunnelCloseParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub session: Option<String>,
}

/// Result of `tunnels.close` and `tunnels.delete`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TunnelsClosed {
    /// Tunnels that were stopped or removed.
    pub closed: Vec<String>,
}

/// Params of `tunnels.update`. Omitted fields keep their value; a running tunnel is
/// restarted when its addresses change.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TunnelUpdateParams {
    pub id: String,
    /// An empty string clears the name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub local_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub remote_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub remote_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub bind: Option<String>,
    #[serde(default)]
    pub allow_public: bool,
    /// `true` saves a temporary tunnel (and removes its expiry).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub save: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub auto_start: Option<bool>,
}

/// Result of `tunnels.list`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TunnelsList {
    pub tunnels: Vec<Tunnel>,
}
