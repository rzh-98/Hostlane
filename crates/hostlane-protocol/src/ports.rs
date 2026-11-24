use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Params of `ports.list`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PortsListParams {
    pub host: String,
    /// Probe each port for HTTP/HTTPS through the SSH connection. Defaults to true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub probe: Option<bool>,
}

/// Who can reach a listening socket, judged from the addresses it is bound to.
/// Firewalls are not taken into account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BindScope {
    /// Only the server itself (127.0.0.1, ::1).
    Loopback,
    /// A private or link-local address (10/8, 172.16/12, 192.168/16, 100.64/10, fc00::/7, …).
    Private,
    /// A specific public address.
    Public,
    /// Every interface (0.0.0.0, ::, *).
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProcessInfo {
    #[ts(type = "number")]
    pub pid: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub user: Option<String>,
    /// Full command line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub command: Option<String>,
    /// Unix ms, derived from the elapsed time `ps` reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub started_at: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ServiceKind {
    Systemd,
    Docker,
}

/// The systemd unit or Docker container behind a port.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ServiceInfo {
    pub kind: ServiceKind,
    /// Unit name (`nginx.service`) or container name.
    pub name: String,
    /// A systemd user unit (`systemctl --user`).
    #[serde(default)]
    pub user_unit: bool,
    /// Short (12 character) container id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub container_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub image: Option<String>,
    /// Port inside the container this host port is published from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub container_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub compose_project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub compose_service: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TlsInfo {
    /// Common name of the certificate subject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub issuer: Option<String>,
    /// DNS names the certificate is valid for.
    pub names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub not_before: Option<u64>,
    /// Expiry, Unix ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub not_after: Option<u64>,
    /// Negative once expired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub days_left: Option<i64>,
    pub self_signed: bool,
    /// E.g. `TLSv1.3`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
}

/// What an HTTP request to the port returned. Requests go from the server to
/// itself through the SSH connection, so nothing is needed on the server.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HttpProbe {
    /// `http` or `https`.
    pub scheme: String,
    pub status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title: Option<String>,
    /// The `Server` header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub content_type: Option<String>,
    /// The `Location` header of a redirect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub location: Option<String>,
    #[ts(type = "number")]
    pub latency_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tls: Option<TlsInfo>,
}

/// A user's note on a port.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PortNote {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<String>,
    #[ts(type = "number")]
    pub updated_at: u64,
}

/// One TCP port on a server: every socket listening on it, the process and service
/// behind it, what it answers and the user's note.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ListeningPort {
    pub port: u16,
    /// False for a noted port that nothing listens on right now.
    pub listening: bool,
    /// Bound addresses, e.g. `0.0.0.0` and `::`.
    pub addresses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub bind: Option<BindScope>,
    /// Every process holding a listening socket (e.g. nginx master and workers).
    /// Empty when they belong to another user and the SSH user is not root.
    #[ts(type = "Array<number>")]
    pub pids: Vec<u32>,
    /// The main process: the one whose parent is not in `pids`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub process: Option<ProcessInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub service: Option<ServiceInfo>,
    /// Protocol detected by probing: `http`, `https`, or a server-first protocol such as `ssh`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub http: Option<HttpProbe>,
    /// Why probing failed or was skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub probe_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<PortNote>,
}

/// Result of `ports.list`, ordered by port. `ports.inspect` returns the same with
/// exactly one entry, the requested port (`listening: false` when nothing is there).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PortsList {
    pub host: String,
    /// What listed the sockets: `ss`, `netstat`, `proc` (/proc/net/tcp) or `netstat+lsof`.
    pub tool: String,
    pub ports: Vec<ListeningPort>,
    pub warnings: Vec<String>,
    #[ts(type = "number")]
    pub scanned_at: u64,
}

/// Params of `ports.inspect` and `ports.check`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PortParams {
    pub host: String,
    pub port: u16,
    /// `ports.inspect` only: host name for the HTTP `Host` header and TLS SNI,
    /// e.g. the site's domain when several sites share the port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub server_name: Option<String>,
}

/// Result of `ports.check`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PortCheck {
    pub host: String,
    pub port: u16,
    pub in_use: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub listener: Option<ListeningPort>,
    /// The nearest free port above `port`, when it is in use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub suggestion: Option<u16>,
    pub warnings: Vec<String>,
}

/// Params of `ports.find_free`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FindFreeParams {
    pub host: String,
    /// Defaults to 8000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from: Option<u16>,
    /// Inclusive. Defaults to 9999.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub to: Option<u16>,
    /// Defaults to 1, at most 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub count: Option<u32>,
}

/// Result of `ports.find_free`. Ports with a note are skipped as reserved.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FreePorts {
    pub host: String,
    pub ports: Vec<u16>,
    pub warnings: Vec<String>,
}

/// Params of `ports.annotate`. Empty or missing fields clear them; clearing all
/// three removes the note.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PortAnnotateParams {
    pub host: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<String>,
}

/// Result of `ports.annotate`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PortAnnotated {
    pub host: String,
    pub port: u16,
    /// Missing when the note was removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<PortNote>,
}
