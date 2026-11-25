use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    Approval, AuditEntry, Connection, ExecRecord, FileChange, Job, OutputStream, PolicyStatus,
    Tunnel,
};

/// Pushed to `events.subscribe` connections as `{"method":"event","params":<Event>}`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum Event {
    ExecStarted {
        exec: ExecRecord,
    },
    /// Live output. Each command streams at most 1 MiB per stream; the rest is
    /// still stored and readable with `output.read`.
    ExecOutput {
        id: String,
        stream: OutputStream,
        data: String,
    },
    ExecFinished {
        exec: ExecRecord,
    },
    Connection {
        connection: Connection,
    },
    /// A file was written, patched, uploaded or reverted. `diff` is left out; read it
    /// with `files.change`.
    FileChanged {
        change: FileChange,
    },
    /// A call was added to the audit log.
    AuditAppended {
        entry: AuditEntry,
    },
    /// A tunnel was opened or changed state; also sent every few seconds while its
    /// traffic counters move.
    Tunnel {
        tunnel: Tunnel,
    },
    /// A tunnel was closed (temporary) or deleted (saved).
    TunnelRemoved {
        id: String,
    },
    /// A call needs a person's approval.
    ApprovalRequested {
        approval: Approval,
    },
    /// An approval was decided, expired, or its call ran or stopped waiting.
    ApprovalUpdated {
        approval: Approval,
    },
    /// The emergency stop was switched, or the number of pending approvals changed.
    Policy {
        status: PolicyStatus,
    },
    /// A background job started, finished, was cancelled, or its output grew.
    Job {
        job: Job,
    },
    /// This subscriber fell behind and missed events; reload state with the list methods.
    Lagged {
        #[ts(type = "number")]
        skipped: u64,
    },
}

/// Result of `events.subscribe`, sent before the first event.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Subscribed {
    pub subscribed: bool,
}
