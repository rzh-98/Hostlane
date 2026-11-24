//! Wire protocol between Hostlane clients (CLI, MCP, desktop) and the daemon.
//!
//! Transport: newline-delimited JSON-RPC 2.0 over a local socket that only the
//! current user can open. Every type that crosses the wire lives here; running
//! `cargo test -p hostlane-protocol` regenerates the TypeScript bindings in
//! `apps/desktop/src/shared/protocol/`.

mod audit;
mod error;
mod events;
mod exec;
mod hosts;
mod rpc;
mod status;

pub use audit::{AuditEntry, AuditList, AuditListParams, AuditOutcome};
pub use error::{ErrorKind, RpcError, RpcErrorData};
pub use events::{Event, Subscribed};
pub use exec::{
    ExecList, ExecListParams, ExecParams, ExecRecord, ExecResult, ExecStatus, OutputChunk,
    OutputReadParams, OutputStream,
};
pub use hosts::{
    Connection, ConnectionState, ConnectionsList, Host, HostEnv, HostKeyInfo, HostParams,
    HostSource, HostsList, TrustParams, TrustResult,
};
pub use rpc::{ClientInfo, Notification, Request, Response};
pub use status::DaemonStatus;

/// Bumped whenever a breaking change is made to the wire format.
pub const PROTOCOL_VERSION: u32 = 1;

/// Method names understood by the daemon.
pub mod method {
    pub const DAEMON_STATUS: &str = "daemon.status";
    pub const DAEMON_SHUTDOWN: &str = "daemon.shutdown";
    pub const HOSTS_LIST: &str = "hosts.list";
    pub const HOSTS_CONNECT: &str = "hosts.connect";
    pub const HOSTS_DISCONNECT: &str = "hosts.disconnect";
    pub const HOSTS_TRUST: &str = "hosts.trust";
    pub const CONNECTIONS_LIST: &str = "connections.list";
    pub const EXEC_RUN: &str = "exec.run";
    pub const EXEC_LIST: &str = "exec.list";
    pub const OUTPUT_READ: &str = "output.read";
    pub const AUDIT_LIST: &str = "audit.list";
    /// Keeps the connection open and streams `event` notifications.
    pub const EVENTS_SUBSCRIBE: &str = "events.subscribe";

    /// Notification method used for pushed events.
    pub const EVENT_NOTIFICATION: &str = "event";

    /// Every method the daemon currently implements.
    pub const ALL: &[&str] = &[
        DAEMON_STATUS,
        DAEMON_SHUTDOWN,
        HOSTS_LIST,
        HOSTS_CONNECT,
        HOSTS_DISCONNECT,
        HOSTS_TRUST,
        CONNECTIONS_LIST,
        EXEC_RUN,
        EXEC_LIST,
        OUTPUT_READ,
        AUDIT_LIST,
        EVENTS_SUBSCRIBE,
    ];
}
