use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{AuditEntry, Connection, ExecRecord, FileChange, OutputStream};

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
