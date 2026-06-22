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
    /// opaque page token returned by a prior `SearchResult`. When `subscribe` is
    /// true, the initial `SearchResult` is followed by `SearchUpdate`s on change
    /// (a live top-N subscription keyed by `request_id`).
    Search {
        request_id: Uuid,
        query: String,
        limit: u32,
        cursor: Option<String>,
        #[serde(default)]
        subscribe: bool,
    },
    /// Cancel a live search subscription (idempotent; unknown id ignored).
    Unsubscribe { request_id: Uuid },
    /// Subscribe to a derived scene channel (e.g. M9 "vision"). M8a recognizes
    /// only the debug "identity" channel; unknown channels yield SceneError.
    SceneSubscribe { request_id: Uuid, channel: String },
    /// Cancel a derived subscription by request id.
    SceneUnsubscribe { request_id: Uuid },
    /// A transient location ping at scene coords. Relayed out-of-band to the world
    /// room with the sender stamped; never sequenced, logged, or a document (#3).
    /// Coordinates are not validated (#6); rate-limited per connection.
    ScenePing { scene: Uuid, x: f64, y: f64 },
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

/// The kind of asset mutation an `AssetChanged` frame reports.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum AssetOp {
    Replaced,
    Deleted,
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
        /// The world's UI contract declarations, so the client can validate its
        /// loaded module set against the world's declared topology.
        contract_declarations: Vec<crate::data::document::ContractDeclaration>,
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
    /// A live subscription's refreshed top-N (full replace). Documents are
    /// already filtered for the recipient.
    SearchUpdate {
        request_id: Uuid,
        hits: Vec<SearchHit>,
    },
    /// A derived-state push: coalesced, per recipient, ordered after the
    /// document events it reflects via `computed_at_seq`. `payload` is opaque to
    /// the transport (#6).
    SceneDerived {
        request_id: Uuid,
        channel: String,
        computed_at_seq: i64,
        #[ts(type = "unknown")]
        payload: serde_json::Value,
    },
    /// A derived subscription failed (e.g. unknown channel).
    SceneError { request_id: Uuid, message: String },
    /// Out-of-band asset mutation notice. Carries no seq and is never buffered
    /// or resynced; holders re-resolve against the record's `version`.
    AssetChanged { uuid: Uuid, op: AssetOp },
    /// A relayed location ping: the sender's transient marker at scene coords.
    /// Out-of-band (no seq, never buffered/resynced), mirroring `AssetChanged`.
    ScenePing {
        scene: Uuid,
        x: f64,
        y: f64,
        user: Uuid,
    },
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
    fn asset_changed_is_out_of_band_and_serializes_snake_case() {
        let m = ServerMsg::AssetChanged {
            uuid: Uuid::from_u128(7),
            op: AssetOp::Replaced,
        };
        // Out-of-band: no event seq, so egress sends it without gap/resync logic.
        assert_eq!(m.event_seq(), None);
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("\"type\":\"asset_changed\""), "got {s}");
        assert!(s.contains("\"op\":\"replaced\""), "got {s}");
    }

    #[test]
    fn scene_ping_round_trips_and_is_out_of_band() {
        let c = ClientMsg::ScenePing {
            scene: Uuid::from_u128(1),
            x: 10.0,
            y: 20.0,
        };
        let s = serde_json::to_string(&c).unwrap();
        assert!(s.contains("\"type\":\"scene_ping\""), "got {s}");
        let _back: ClientMsg = serde_json::from_str(&s).unwrap();

        let sv = ServerMsg::ScenePing {
            scene: Uuid::from_u128(1),
            x: 10.0,
            y: 20.0,
            user: Uuid::from_u128(2),
        };
        // Out-of-band: never buffered/resynced.
        assert_eq!(sv.event_seq(), None);
        let j = serde_json::to_value(&sv).unwrap();
        assert_eq!(j["type"], "scene_ping");
        assert_eq!(j["x"], 10.0);
        assert!(j.get("user").is_some());
    }

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
            subscribe: false,
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
    fn subscribe_defaults_false_and_live_frames_round_trip() {
        // A one-shot Search frame (no `subscribe`) still deserializes (default false).
        let oneshot: ClientMsg = serde_json::from_str(
            r#"{"type":"search","request_id":"00000000-0000-0000-0000-000000000001","query":"x","limit":20,"cursor":null}"#,
        )
        .unwrap();
        match oneshot {
            ClientMsg::Search { subscribe, .. } => assert!(!subscribe),
            _ => panic!("expected Search"),
        }
        let unsub = ClientMsg::Unsubscribe {
            request_id: Uuid::from_u128(1),
        };
        assert!(serde_json::to_string(&unsub)
            .unwrap()
            .contains("\"type\":\"unsubscribe\""));
        let upd = ServerMsg::SearchUpdate {
            request_id: Uuid::from_u128(2),
            hits: Vec::new(),
        };
        assert!(serde_json::to_string(&upd)
            .unwrap()
            .contains("\"type\":\"search_update\""));
    }

    #[test]
    fn scene_frames_round_trip() {
        let sub = ClientMsg::SceneSubscribe {
            request_id: Uuid::from_u128(1),
            channel: "identity".into(),
        };
        let j = serde_json::to_value(&sub).unwrap();
        assert_eq!(j["type"], "scene_subscribe");
        assert_eq!(j["channel"], "identity");

        let d = ServerMsg::SceneDerived {
            request_id: Uuid::from_u128(1),
            channel: "identity".into(),
            computed_at_seq: 7,
            payload: serde_json::json!({ "entity_count": 3 }),
        };
        let j = serde_json::to_value(&d).unwrap();
        assert_eq!(j["type"], "scene_derived");
        assert_eq!(j["computed_at_seq"], 7);
        assert_eq!(j["payload"]["entity_count"], 3);
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
            contract_declarations: Vec::new(),
        };
        let json = serde_json::to_value(&w).unwrap();
        assert_eq!(json["type"], "welcome");
        assert_eq!(json["actor_role"], "player");
        assert!(json.get("world_default_grants").is_some());
        assert!(json.get("capability_requirements").is_some());
        assert!(json.get("contract_declarations").is_some());
    }
}
