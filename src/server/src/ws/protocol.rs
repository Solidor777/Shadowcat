//! WebSocket wire protocol: client/server message envelopes.
//!
//! JSON text frames, internally tagged on `type`. Generated to TypeScript via
//! ts-rs (CI-enforced sync). Binary encodings are rejected: they bypass the
//! type-generation pipeline and reduce debuggability.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::data::command::{Command, Operation};
use crate::data::search::SearchHit;

/// Client -> server frames.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// First frame after upgrade: names the world and the client's last known seq.
    Hello { world: Uuid, last_seq: Option<i64> },
    /// A proposed write: a client-chosen `intent_id` for correlation plus the
    /// ops to apply. The server authorizes/validates/sequences them through the
    /// one write path; success broadcasts an `Event`, failure returns `Reject`.
    Intent {
        intent_id: Uuid,
        ops: Vec<Operation>,
    },
    /// Explicit gap recovery from the client's sequence guard.
    ResyncRequest { from_seq: i64 },
    /// Time calibration ping carrying the client's send timestamp.
    TimePing { client_t0: i64 },
    /// Heartbeat reply.
    Pong,
    /// A full-text search request, correlated by `request_id`. `cursor` is the
    /// opaque page token returned by a prior `SearchResult`.
    Search {
        request_id: Uuid,
        query: String,
        limit: u32,
        cursor: Option<String>,
    },
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
    Forbidden,
    Internal,
}

/// Why an `Intent` was rejected. Mirrors the write-path `DataError` categories
/// the client can act on: re-auth, re-read+retry, or fix the payload.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    Forbidden,
    Conflict,
    Invalid,
}

/// Server -> client frames.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Sent right after a successful join. Carries the world's default capability
    /// grants, the connecting actor's world role, and the declarative capability
    /// requirements so the client can replicate access resolution for advisory
    /// UI gating (the server remains authoritative).
    Welcome {
        world: Uuid,
        current_seq: i64,
        server_time: i64,
        world_default_grants: crate::data::document::CapabilityGrants,
        actor_role: crate::data::document::WorldRole,
        capability_requirements: Vec<crate::data::document::CapabilityRequirement>,
    },
    /// A sequenced broadcast carrying the authoritative command. `intent_id` is
    /// the originator's correlation token; it is `None` on the shared broadcast
    /// (an originator confirms its own write by receiving this echo of its
    /// authored command). Per-intent `Some` correlation is added in M6.
    Event {
        command: Command,
        intent_id: Option<Uuid>,
    },
    /// An `Intent` the write path refused, addressed to its originator only.
    Reject {
        intent_id: Uuid,
        reason: RejectReason,
    },
    /// Opens a resync replay range.
    ResyncBegin {
        from_seq: i64,
        to_seq: i64,
        source: ResyncSource,
    },
    /// Closes a resync replay range; live delivery resumes after this.
    ResyncEnd { current_seq: i64 },
    /// Time calibration reply: echoes the client send time, adds the server time.
    TimePong { client_t0: i64, server_t: i64 },
    /// Heartbeat.
    Ping,
    /// A non-fatal or fatal error, by code.
    Error { code: WsErrorCode, message: String },
    /// Results for the `Search` with this `request_id`. Documents are already
    /// filtered for the recipient. `next_cursor` is `None` when exhausted.
    SearchResult {
        request_id: Uuid,
        hits: Vec<SearchHit>,
        next_cursor: Option<String>,
    },
    /// The `Search` with this `request_id` failed.
    SearchError { request_id: Uuid, message: String },
}

impl ServerMsg {
    /// seq of an `Event` frame, else `None`. Only `Event`s are buffered/resynced.
    pub fn event_seq(&self) -> Option<i64> {
        match self {
            ServerMsg::Event { command, .. } => Some(command.seq),
            _ => None,
        }
    }

    /// server-stamped ts of an `Event` frame, else `None`.
    pub fn event_ts(&self) -> Option<i64> {
        match self {
            ServerMsg::Event { command, .. } => Some(command.ts),
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
        let m = ClientMsg::Hello {
            world: Uuid::from_u128(7),
            last_seq: Some(3),
        };
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("\"type\":\"hello\""));
        let back: ClientMsg = serde_json::from_str(&s).unwrap();
        assert!(matches!(
            back,
            ClientMsg::Hello {
                last_seq: Some(3),
                ..
            }
        ));
    }

    #[test]
    fn server_event_and_resync_round_trip() {
        let begin = ServerMsg::ResyncBegin {
            from_seq: 2,
            to_seq: 5,
            source: ResyncSource::Buffer,
        };
        let s = serde_json::to_string(&begin).unwrap();
        assert!(s.contains("\"type\":\"resync_begin\""));
        assert!(s.contains("\"source\":\"buffer\""));
        let _back: ServerMsg = serde_json::from_str(&s).unwrap();
    }

    #[test]
    fn reject_round_trips_snake_case() {
        let m = ServerMsg::Reject {
            intent_id: Uuid::from_u128(3),
            reason: RejectReason::Conflict,
        };
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("\"type\":\"reject\""));
        assert!(s.contains("\"reason\":\"conflict\""));
        let _back: ServerMsg = serde_json::from_str(&s).unwrap();
    }

    #[test]
    fn error_code_serializes_snake_case() {
        let e = ServerMsg::Error {
            code: WsErrorCode::WorldNotFound,
            message: "x".into(),
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"code\":\"world_not_found\""));
    }

    #[test]
    fn search_frames_round_trip() {
        let req = ClientMsg::Search {
            request_id: Uuid::from_u128(1),
            query: "dragon".into(),
            limit: 20,
            cursor: None,
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"type\":\"search\""));
        let _back: ClientMsg = serde_json::from_str(&s).unwrap();

        let err = ServerMsg::SearchError {
            request_id: Uuid::from_u128(2),
            message: "x".into(),
        };
        let s = serde_json::to_string(&err).unwrap();
        assert!(s.contains("\"type\":\"search_error\""));
    }

    #[test]
    fn welcome_carries_caps_role_and_requirements() {
        use crate::data::document::{CapabilityGrants, WorldRole};
        let w = ServerMsg::Welcome {
            world: Uuid::from_u128(1),
            current_seq: 0,
            server_time: 0,
            world_default_grants: CapabilityGrants::default(),
            actor_role: WorldRole::Player,
            capability_requirements: Vec::new(),
        };
        let json = serde_json::to_value(&w).unwrap();
        assert_eq!(json["type"], "welcome");
        assert_eq!(json["actor_role"], "player");
        assert!(json.get("world_default_grants").is_some());
        assert!(json.get("capability_requirements").is_some());
    }
}
