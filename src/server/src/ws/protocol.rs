//! WebSocket wire protocol: client/server message envelopes.
//!
//! JSON text frames, internally tagged on `type`. Generated to TypeScript via
//! ts-rs (CI-enforced sync). The full enum lands incrementally; the ring buffer
//! only needs the `Event` variant plus the seq/ts accessors.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::data::command::Command;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// A sequenced broadcast carrying the authoritative command.
    Event { command: Command },
}

impl ServerMsg {
    /// seq of an `Event` frame, else `None`. Only `Event`s are buffered/resynced.
    pub fn event_seq(&self) -> Option<i64> {
        match self {
            ServerMsg::Event { command } => Some(command.seq),
        }
    }

    /// server-stamped ts of an `Event` frame, else `None`.
    pub fn event_ts(&self) -> Option<i64> {
        match self {
            ServerMsg::Event { command } => Some(command.ts),
        }
    }
}
