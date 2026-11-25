//! Wire protocol between Hostlane clients (CLI, MCP, desktop) and the daemon.
//!
//! Transport: newline-delimited JSON-RPC 2.0 over a local socket that only the
//! current user can open. Every type that crosses the wire lives here; running
//! `cargo test -p hostlane-protocol` regenerates the TypeScript bindings in
//! `apps/desktop/src/shared/protocol/`.

mod audit;
mod clients;
mod error;
mod events;
mod exec;
mod files;
mod hosts;
mod jobs;
mod policy;
mod ports;
mod rpc;
mod secrets;
mod services;
mod status;
mod tunnels;
mod untrusted;

pub use audit::{AuditEntry, AuditList, AuditListParams, AuditOutcome};
pub use clients::{
    ClientCreated, ClientNameParams, ClientsConfigureParams, ClientsList, RegisteredClient,
};
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
    AuthFailure, Connection, ConnectionState, ConnectionsList, Host, HostAddParams, HostEnv,
    HostKeyInfo, HostParams, HostRemoved, HostSource, HostsList, TrustParams, TrustResult,
};
pub use jobs::{
    Job, JobBackend, JobLogs, JobLogsParams, JobParams, JobStartParams, JobState, JobStatusParams,
    JobsList, JobsListParams,
};
pub use policy::{
    Approval, ApprovalResolveParams, ApprovalScope, ApprovalState, ApprovalsList,
    ApprovalsListParams, Decision, HostLabelParams, IdParams, PolicyCheck, PolicyCheckParams,
    PolicyRule, PolicyRuleAddParams, PolicyRules, PolicyStatus, PolicyStopParams, PolicyStopped,
    Risk, RuleConditions,
};
pub use ports::{
    BindScope, FindFreeParams, FreePorts, HttpProbe, ListeningPort, PortAnnotateParams,
    PortAnnotated, PortCheck, PortNote, PortParams, PortsList, PortsListParams, ProcessInfo,
    ServiceInfo, ServiceKind, TlsInfo,
};
pub use rpc::{ClientInfo, Notification, Request, Response};
pub use secrets::{
    SavedSecret, SecretChanged, SecretDeleteParams, SecretKind, SecretSetParams, SecretsList,
};
pub use services::{
    ProcessKillParams, ServiceCommandResult, ServiceControlParams, ServiceLogsParams,
};
pub use status::DaemonStatus;
pub use tunnels::{
    Tunnel, TunnelCloseParams, TunnelOpenParams, TunnelOpened, TunnelParams, TunnelState,
    TunnelUpdateParams, TunnelsClosed, TunnelsList,
};
pub use untrusted::{UNTRUSTED_MEMBER, UNTRUSTED_NOTE, untrusted_fields};

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
    pub const HOSTS_LABEL: &str = "hosts.label";
    pub const HOSTS_ADD: &str = "hosts.add";
    pub const HOSTS_UPDATE: &str = "hosts.update";
    pub const HOSTS_REMOVE: &str = "hosts.remove";
    pub const SECRETS_LIST: &str = "secrets.list";
    pub const SECRETS_SET: &str = "secrets.set";
    pub const SECRETS_DELETE: &str = "secrets.delete";
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
    pub const SERVICES_LOGS: &str = "services.logs";
    pub const SERVICES_START: &str = "services.start";
    pub const SERVICES_STOP: &str = "services.stop";
    pub const SERVICES_RESTART: &str = "services.restart";
    pub const PROCESSES_KILL: &str = "processes.kill";
    pub const JOBS_START: &str = "jobs.start";
    pub const JOBS_LIST: &str = "jobs.list";
    pub const JOBS_STATUS: &str = "jobs.status";
    pub const JOBS_LOGS: &str = "jobs.logs";
    pub const JOBS_CANCEL: &str = "jobs.cancel";
    pub const CLIENTS_LIST: &str = "clients.list";
    pub const CLIENTS_CREATE: &str = "clients.create";
    pub const CLIENTS_REVOKE: &str = "clients.revoke";
    pub const CLIENTS_CONFIGURE: &str = "clients.configure";
    pub const APPROVALS_LIST: &str = "approvals.list";
    pub const APPROVALS_GET: &str = "approvals.get";
    pub const APPROVALS_RESOLVE: &str = "approvals.resolve";
    pub const POLICY_STATUS: &str = "policy.status";
    pub const POLICY_STOP: &str = "policy.stop";
    pub const POLICY_RESUME: &str = "policy.resume";
    pub const POLICY_CHECK: &str = "policy.check";
    pub const POLICY_RULES_LIST: &str = "policy.rules.list";
    pub const POLICY_RULES_ADD: &str = "policy.rules.add";
    pub const POLICY_RULES_DELETE: &str = "policy.rules.delete";
    pub const AUDIT_LIST: &str = "audit.list";
    /// Keeps the connection open and streams `event` notifications.
    pub const EVENTS_SUBSCRIBE: &str = "events.subscribe";

    /// Notification method used for pushed events.
    pub const EVENT_NOTIFICATION: &str = "event";

    /// Methods checked by the policy before they run. Their params also accept
    /// `ticket_id` (retry after approval), `async` (return `APPROVAL_REQUIRED` instead
    /// of waiting) and `approval_timeout_secs` (how long to wait, default 120).
    pub const GATED: &[&str] = &[
        HOSTS_CONNECT,
        HOSTS_DISCONNECT,
        HOSTS_TRUST,
        EXEC_RUN,
        FILES_READ,
        FILES_LIST,
        FILES_SEARCH,
        FILES_WRITE,
        FILES_PATCH,
        FILES_UPLOAD,
        FILES_DOWNLOAD,
        FILES_REVERT,
        PORTS_LIST,
        PORTS_INSPECT,
        PORTS_CHECK,
        PORTS_FIND_FREE,
        PORTS_ANNOTATE,
        TUNNELS_OPEN,
        TUNNELS_CLOSE,
        TUNNELS_START,
        TUNNELS_UPDATE,
        TUNNELS_DELETE,
        SERVICES_LOGS,
        SERVICES_START,
        SERVICES_STOP,
        SERVICES_RESTART,
        PROCESSES_KILL,
        JOBS_START,
        JOBS_LOGS,
        JOBS_CANCEL,
    ];

    /// Every method the daemon currently implements.
    pub const ALL: &[&str] = &[
        DAEMON_STATUS,
        DAEMON_SHUTDOWN,
        HOSTS_LIST,
        HOSTS_CONNECT,
        HOSTS_DISCONNECT,
        HOSTS_TRUST,
        HOSTS_LABEL,
        HOSTS_ADD,
        HOSTS_UPDATE,
        HOSTS_REMOVE,
        SECRETS_LIST,
        SECRETS_SET,
        SECRETS_DELETE,
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
        SERVICES_LOGS,
        SERVICES_START,
        SERVICES_STOP,
        SERVICES_RESTART,
        PROCESSES_KILL,
        JOBS_START,
        JOBS_LIST,
        JOBS_STATUS,
        JOBS_LOGS,
        JOBS_CANCEL,
        CLIENTS_LIST,
        CLIENTS_CREATE,
        CLIENTS_REVOKE,
        CLIENTS_CONFIGURE,
        APPROVALS_LIST,
        APPROVALS_GET,
        APPROVALS_RESOLVE,
        POLICY_STATUS,
        POLICY_STOP,
        POLICY_RESUME,
        POLICY_CHECK,
        POLICY_RULES_LIST,
        POLICY_RULES_ADD,
        POLICY_RULES_DELETE,
        AUDIT_LIST,
        EVENTS_SUBSCRIBE,
    ];
}
