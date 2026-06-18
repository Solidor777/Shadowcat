//! WebSocket wire protocol: client/server message envelopes.
//!
//! JSON text frames, internally tagged on `type`. Generated to TypeScript via
//! ts-rs (CI-enforced sync). Binary encodings are rejected: they bypass the
//! type-generation pipeline and reduce debuggability.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::data::command::Command;

/// Client -> server frames.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// First frame after upgrade: names the world and the client's last known seq.
    Hello { world: Uuid, last_seq: Option<i64> },
    /// M4 driver: request an empty-ops command be sequenced and broadcast.
    EmitTest { nonce: u64 },
    /// Explicit gap recovery from the client's sequence guard.
    ResyncRequest { from_seq: i64 },
    /// Time calibration ping carrying the client's send timestamp.
    TimePing { client_t0: i64 },
    /// Heartbeat reply.
    Pong,
}

/// Which tier served a resync.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum ResyncSource {
    Buffer,
    Log,
}

/// Error categories surfaced over the socket.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum WsErrorCode {
    WorldNotFound,
    BadMessage,
    PublishFailed,
    Internal,
}

/// Server -> client frames.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Sent right after a successful join.
    Welcome { world: Uuid, current_seq: i64, server_time: i64 },
    /// A sequenced broadcast carrying the authoritative command.
    Event { command: Command },
    /// Opens a resync replay range.
    ResyncBegin { from_seq: i64, to_seq: i64, source: ResyncSource },
    /// Closes a resync replay range; live delivery resumes after this.
    ResyncEnd { current_seq: i64 },
    /// Time calibration reply: echoes the client send time, adds the server time.
    TimePong { client_t0: i64, server_t: i64 },
    /// Heartbeat.
    Ping,
    /// A non-fatal or fatal error, by code.
    Error { code: WsErrorCode, message: String },
}

impl ServerMsg {
    /// seq of an `Event` frame, else `None`. Only `Event`s are buffered/resynced.
    pub fn event_seq(&self) -> Option<i64> {
        match self {
            ServerMsg::Event { command } => Some(command.seq),
            _ => None,
        }
    }

    /// server-stamped ts of an `Event` frame, else `None`.
    pub fn event_ts(&self) -> Option<i64> {
        match self {
            ServerMsg::Event { command } => Some(command.ts),
            _ => None,
        }
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn client_hello_round_trips_and_is_tagged() {
        let m = ClientMsg::Hello { world: Uuid::from_u128(7), last_seq: Some(3) };
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("\"type\":\"hello\""));
        let back: ClientMsg = serde_json::from_str(&s).unwrap();
        assert!(matches!(back, ClientMsg::Hello { last_seq: Some(3), .. }));
    }

    #[test]
    fn server_event_and_resync_round_trip() {
        let begin = ServerMsg::ResyncBegin { from_seq: 2, to_seq: 5, source: ResyncSource::Buffer };
        let s = serde_json::to_string(&begin).unwrap();
        assert!(s.contains("\"type\":\"resync_begin\""));
        assert!(s.contains("\"source\":\"buffer\""));
        let _back: ServerMsg = serde_json::from_str(&s).unwrap();
    }

    #[test]
    fn error_code_serializes_snake_case() {
        let e = ServerMsg::Error { code: WsErrorCode::WorldNotFound, message: "x".into() };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"code\":\"world_not_found\""));
    }
}
