//! Wire protocol between Hostlane clients (CLI, MCP, desktop) and the daemon.
//!
//! Transport: newline-delimited JSON-RPC 2.0 over a local socket that only the
//! current user can open. Every type that crosses the wire lives here; running
//! `cargo test -p hostlane-protocol` regenerates the TypeScript bindings in
//! `apps/desktop/src/shared/protocol/`.

mod error;
mod hosts;
mod rpc;
mod status;

pub use error::{ErrorKind, RpcError, RpcErrorData};
pub use hosts::{Host, HostEnv, HostSource, HostsList};
pub use rpc::{Request, Response};
pub use status::DaemonStatus;

/// Bumped whenever a breaking change is made to the wire format.
pub const PROTOCOL_VERSION: u32 = 1;

/// Method names understood by the daemon.
pub mod method {
    pub const DAEMON_STATUS: &str = "daemon.status";
    pub const DAEMON_SHUTDOWN: &str = "daemon.shutdown";
    pub const HOSTS_LIST: &str = "hosts.list";

    /// Every method the daemon currently implements.
    pub const ALL: &[&str] = &[DAEMON_STATUS, DAEMON_SHUTDOWN, HOSTS_LIST];
}
