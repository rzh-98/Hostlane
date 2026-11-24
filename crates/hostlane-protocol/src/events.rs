use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Connection, ExecRecord, OutputStream};

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
