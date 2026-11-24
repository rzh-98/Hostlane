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
mod files;
mod hosts;
mod ports;
mod rpc;
mod status;
mod tunnels;

pub use audit::{AuditEntry, AuditList, AuditListParams, AuditOutcome};
pub use error::{ErrorKind, RpcError, RpcErrorData};
pub use events::{Event, Subscribed};
pub use exec::{
    ExecList, ExecListParams, ExecParams, ExecRecord, ExecResult, ExecStatus, OutputChunk,
    OutputReadParams, OutputStream,
};
pub use files::{
    FileChange, FileChangeParams, FileChangeResult, FileContent, FileDownloadParams,
    FileDownloadResult, FileEdit, FileEntry, FileKind, FileList, FileListParams, FileOperation,
    FilePatchParams, FileReadParams, FileSearchParams, FileSearchResult, FileUploadParams,
    FileWriteParams, SearchMatch,
};
pub use hosts::{
    Connection, ConnectionState, ConnectionsList, Host, HostEnv, HostKeyInfo, HostParams,
    HostSource, HostsList, TrustParams, TrustResult,
};
pub use ports::{
    BindScope, FindFreeParams, FreePorts, HttpProbe, ListeningPort, PortAnnotateParams,
    PortAnnotated, PortCheck, PortNote, PortParams, PortsList, PortsListParams, ProcessInfo,
    ServiceInfo, ServiceKind, TlsInfo,
};
pub use rpc::{ClientInfo, Notification, Request, Response};
pub use status::DaemonStatus;
pub use tunnels::{
    Tunnel, TunnelCloseParams, TunnelOpenParams, TunnelOpened, TunnelParams, TunnelState,
    TunnelUpdateParams, TunnelsClosed, TunnelsList,
};

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
    pub const FILES_READ: &str = "files.read";
    pub const FILES_LIST: &str = "files.list";
    pub const FILES_SEARCH: &str = "files.search";
    pub const FILES_WRITE: &str = "files.write";
    pub const FILES_PATCH: &str = "files.patch";
    pub const FILES_UPLOAD: &str = "files.upload";
    pub const FILES_DOWNLOAD: &str = "files.download";
    pub const FILES_CHANGE: &str = "files.change";
    pub const FILES_REVERT: &str = "files.revert";
    pub const PORTS_LIST: &str = "ports.list";
    pub const PORTS_INSPECT: &str = "ports.inspect";
    pub const PORTS_CHECK: &str = "ports.check";
    pub const PORTS_FIND_FREE: &str = "ports.find_free";
    pub const PORTS_ANNOTATE: &str = "ports.annotate";
    pub const TUNNELS_LIST: &str = "tunnels.list";
    pub const TUNNELS_OPEN: &str = "tunnels.open";
    pub const TUNNELS_CLOSE: &str = "tunnels.close";
    pub const TUNNELS_START: &str = "tunnels.start";
    pub const TUNNELS_UPDATE: &str = "tunnels.update";
    pub const TUNNELS_DELETE: &str = "tunnels.delete";
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
        FILES_READ,
        FILES_LIST,
        FILES_SEARCH,
        FILES_WRITE,
        FILES_PATCH,
        FILES_UPLOAD,
        FILES_DOWNLOAD,
        FILES_CHANGE,
        FILES_REVERT,
        PORTS_LIST,
        PORTS_INSPECT,
        PORTS_CHECK,
        PORTS_FIND_FREE,
        PORTS_ANNOTATE,
        TUNNELS_LIST,
        TUNNELS_OPEN,
        TUNNELS_CLOSE,
        TUNNELS_START,
        TUNNELS_UPDATE,
        TUNNELS_DELETE,
        AUDIT_LIST,
        EVENTS_SUBSCRIBE,
    ];
}
