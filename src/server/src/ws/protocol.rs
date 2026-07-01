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
    /// `as_user` (M9c-2 see-as-player) is **GM-only**: it views the channel as that user; the
    /// server rejects it for non-GMs and resolves the target's role server-side. Omitted/None =
    /// the connection's own view.
    SceneSubscribe {
        request_id: Uuid,
        channel: String,
        #[serde(default)]
        #[ts(optional)]
        as_user: Option<Uuid>,
    },
    /// Cancel a derived subscription by request id.
    SceneUnsubscribe { request_id: Uuid },
    /// A transient location ping at scene coords. Relayed out-of-band to the world
    /// room with the sender stamped; never sequenced, logged, or a document (#3).
    /// Coordinates are not validated (#6); rate-limited per connection.
    ScenePing { scene: Uuid, x: f64, y: f64 },
    /// A one-shot grid pathfinding request, correlated by `request_id`. `start`/`waypoints` are
    /// scene coords; `waypoints`' LAST element is the goal. `footprint_radius` is in grid units
    /// (cells; the client's `footprintRadius`). The route is mask-bounded for non-GM requesters.
    Pathfind {
        request_id: Uuid,
        scene: Uuid,
        start: (f64, f64),
        waypoints: Vec<(f64, f64)>,
        footprint_radius: f64,
    },
    /// A server-authoritative move request: the client submits the previewed cell-center scene
    /// points (start … goal) for a token it controls. The server validates, executes the move,
    /// and broadcasts `MoveStream` out-of-band to the scene on success, or replies `MoveError`
    /// to the originator on failure. `path` carries the exact route preview so the server can
    /// reproduce the animation.
    MoveRequest {
        request_id: Uuid,
        scene: Uuid,
        token_id: Uuid,
        /// Ordered cell-center scene points: start … goal (inclusive). Type is `[f64; 2]` not a
        /// tuple so the TS binding emits `[number, number][]` (array literal, not tuple object).
        path: Vec<[f64; 2]>,
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

/// The kind of asset mutation an `AssetChanged` frame reports.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum AssetOp {
    Replaced,
    Deleted,
}

/// A single position sample in a `MoveStream` timeline.
/// `t_ms` is elapsed milliseconds from `start_server_ms`; `pos` is the scene-coord
/// cell-center at that instant. INVARIANT: `t_ms >= 0`; samples are ordered by ascending `t_ms`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct PosSample {
    /// Elapsed time in milliseconds from `MoveStream.start_server_ms`.
    pub t_ms: f64,
    /// Scene-coordinate position (x, y) at this sample instant.
    pub pos: [f64; 2],
}

/// A single vision-polygon sample in a `MoveStream` timeline, paired with a `PosSample` by `t_ms`.
/// Ordered `[x,y]` vertices of a visible region at this instant; multiple polygons cover
/// non-contiguous visible regions. Not necessarily convex. Sent only for the mover.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct VisionSample {
    /// Elapsed time in milliseconds — matches the corresponding `PosSample.t_ms`.
    pub t_ms: f64,
    /// Visibility polygons (scene coords) visible at this instant. Each polygon is
    /// an ordered list of [x, y] vertices; multiple polygons cover non-contiguous visible areas.
    pub polygons: Vec<Vec<[f64; 2]>>,
}

/// Server -> client frames.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Sent right after a successful join. Carries the world's default capability
    /// grants, the connecting user's world role, and the declarative capability
    /// requirements so the client can replicate access resolution for advisory
    /// UI gating (the server remains authoritative).
    Welcome {
        world: Uuid,
        current_seq: i64,
        server_time: i64,
        world_default_grants: crate::data::document::CapabilityGrants,
        user_role: crate::data::document::WorldRole,
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
    /// The route for the `Pathfind` with this `request_id`: ordered cell-center scene points
    /// (incl. start + goal) and the total cost in cells (client multiplies `grid.distance.perCell`).
    PathResult {
        request_id: Uuid,
        path: Vec<(f64, f64)>,
        cost: f64,
    },
    /// The `Pathfind` with this `request_id` failed (unreachable / invalid request / search exceeded).
    PathError { request_id: Uuid, message: String },
    /// A `MoveRequest` was rejected (token already moving, caller not owner, malformed path, etc.).
    /// Addressed to the originating connection only; never broadcast.
    MoveError { request_id: Uuid, message: String },
    /// Broadcast to the scene, then clipped per recipient at egress: the mover receives the full
    /// trajectory and `mover_vision`; observers receive only the position samples their own vision
    /// admits, with `mover_vision` nulled; a fully-occluded recipient receives nothing.
    MoveStream {
        /// Correlates with the originating `MoveRequest`.
        request_id: Uuid,
        /// The token being moved.
        token_id: Uuid,
        /// The user who owns the move (mover's user id).
        mover: Uuid,
        /// The scene in which the move occurs.
        scene: Uuid,
        /// Authoritative server wall-clock time (ms) at which the animation starts.
        /// INVARIANT: must be set before send so all clients sync to the same origin.
        start_server_ms: f64,
        /// Total wall-clock animation budget in milliseconds.
        duration_ms: f64,
        /// Final resting position (scene coords) after the move completes.
        stop: [f64; 2],
        /// Ordered position samples along the route (t=0 is start, t=duration_ms is stop).
        /// INVARIANT: non-empty; first sample t_ms == 0.0 is the starting cell-center.
        samples: Vec<PosSample>,
        /// Per-sample vision polygons for the mover only. `None` for observers, who receive
        /// server-clipped position samples and render against their existing authoritative fog;
        /// the client computes no vision. Sending mover vision to observers would leak geometry.
        mover_vision: Option<Vec<VisionSample>>,
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
    fn pathfind_frames_round_trip() {
        let req = ClientMsg::Pathfind {
            request_id: Uuid::from_u128(1),
            scene: Uuid::from_u128(2),
            start: (50.0, 50.0),
            waypoints: vec![(150.0, 50.0), (250.0, 50.0)],
            footprint_radius: 0.5,
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"type\":\"pathfind\""), "got {s}");
        let back: ClientMsg = serde_json::from_str(&s).unwrap();
        assert!(matches!(back, ClientMsg::Pathfind { .. }));

        let ok = ServerMsg::PathResult {
            request_id: Uuid::from_u128(1),
            path: vec![(50.0, 50.0)],
            cost: 2.0,
        };
        assert!(serde_json::to_string(&ok)
            .unwrap()
            .contains("\"type\":\"path_result\""));
        let err = ServerMsg::PathError {
            request_id: Uuid::from_u128(1),
            message: "unreachable".into(),
        };
        assert!(serde_json::to_string(&err)
            .unwrap()
            .contains("\"type\":\"path_error\""));
    }

    #[test]
    fn scene_frames_round_trip() {
        let sub = ClientMsg::SceneSubscribe {
            request_id: Uuid::from_u128(1),
            channel: "identity".into(),
            as_user: None,
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
    fn move_request_round_trip() {
        let req = ClientMsg::MoveRequest {
            request_id: Uuid::from_u128(1),
            scene: Uuid::from_u128(2),
            token_id: Uuid::from_u128(3),
            path: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]],
        };
        let wire = serde_json::to_string(&req).unwrap();
        assert!(wire.contains("\"type\":\"move_request\""), "got {wire}");
        let back: ClientMsg = serde_json::from_str(&wire).unwrap();
        assert!(matches!(back, ClientMsg::MoveRequest { .. }));

        // Server replies with MoveError (rejection path) or MoveStream (success path); no MoveExecuted.
        let err = ServerMsg::MoveError {
            request_id: Uuid::from_u128(1),
            message: "token is moving".into(),
        };
        assert!(serde_json::to_string(&err).unwrap().contains("move_error"));
    }

    #[test]
    fn move_stream_round_trips_and_is_tagged() {
        let in_samples = vec![PosSample {
            t_ms: 0.0,
            pos: [0.0, 0.0],
        }];
        let in_vision = Some(vec![VisionSample {
            t_ms: 0.0,
            polygons: vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]],
        }]);
        let msg = ServerMsg::MoveStream {
            request_id: Uuid::from_u128(1),
            token_id: Uuid::from_u128(2),
            mover: Uuid::from_u128(3),
            scene: Uuid::from_u128(4),
            start_server_ms: 1000.0,
            duration_ms: 500.0,
            stop: [100.0, 200.0],
            samples: in_samples.clone(),
            mover_vision: in_vision.clone(),
        };
        let wire = serde_json::to_string(&msg).unwrap();
        // Tag must be snake_case.
        assert!(wire.contains("\"type\":\"move_stream\""), "got {wire}");
        // Deserializes back; each field survives the round-trip.
        let back: ServerMsg = serde_json::from_str(&wire).unwrap();
        match back {
            ServerMsg::MoveStream {
                request_id,
                token_id,
                mover,
                scene,
                start_server_ms,
                duration_ms,
                stop,
                samples,
                mover_vision,
            } => {
                assert_eq!(request_id, Uuid::from_u128(1));
                assert_eq!(token_id, Uuid::from_u128(2));
                assert_eq!(mover, Uuid::from_u128(3));
                assert_eq!(scene, Uuid::from_u128(4));
                assert_eq!(start_server_ms, 1000.0);
                assert_eq!(duration_ms, 500.0);
                assert_eq!(stop, [100.0, 200.0]);
                assert_eq!(samples, in_samples);
                assert_eq!(mover_vision, in_vision);
            }
            _ => panic!("expected MoveStream"),
        }
        // None mover_vision path — verify mover_vision round-trips as None.
        let in_samples2 = vec![PosSample {
            t_ms: 0.0,
            pos: [0.0, 0.0],
        }];
        let msg_no_vision = ServerMsg::MoveStream {
            request_id: Uuid::from_u128(1),
            token_id: Uuid::from_u128(2),
            mover: Uuid::from_u128(3),
            scene: Uuid::from_u128(4),
            start_server_ms: 1000.0,
            duration_ms: 500.0,
            stop: [100.0, 200.0],
            samples: in_samples2,
            mover_vision: None,
        };
        let wire2 = serde_json::to_string(&msg_no_vision).unwrap();
        let back2: ServerMsg = serde_json::from_str(&wire2).unwrap();
        match back2 {
            ServerMsg::MoveStream { mover_vision, .. } => {
                assert_eq!(
                    mover_vision, None,
                    "observer path: mover_vision must round-trip as None"
                );
            }
            _ => panic!("expected MoveStream"),
        }
    }

    #[test]
    fn welcome_carries_caps_role_and_requirements() {
        use crate::data::document::{CapabilityGrants, WorldRole};
        let w = ServerMsg::Welcome {
            world: Uuid::from_u128(1),
            current_seq: 0,
            server_time: 0,
            world_default_grants: CapabilityGrants::default(),
            user_role: WorldRole::Player,
            capability_requirements: Vec::new(),
            contract_declarations: Vec::new(),
        };
        let json = serde_json::to_value(&w).unwrap();
        assert_eq!(json["type"], "welcome");
        assert_eq!(json["user_role"], "player");
        assert!(json.get("world_default_grants").is_some());
        assert!(json.get("capability_requirements").is_some());
        assert!(json.get("contract_declarations").is_some());
    }
}
