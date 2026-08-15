//! WebSocket wire protocol: client/server message envelopes.
//!
//! JSON text frames, internally tagged on `type`. Generated to TypeScript via
//! ts-rs (CI-enforced sync). Binary encodings are rejected: they bypass the
//! type-generation pipeline and reduce debuggability.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::chat::{ActorOwnerRef, Audience};
use crate::data::command::{Command, Operation};
use crate::data::search::SearchHit;

/// Client -> server frames.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// First frame after upgrade: names the world and the client's last known seq.
    Hello {
        /// The world to join.
        world: Uuid,
        /// Highest seq the client has applied; `None` = cold start (full sync).
        last_seq: Option<i64>,
    },
    /// A proposed write: a client-chosen `intent_id` for correlation plus the
    /// ops to apply. The server authorizes/validates/sequences them through the
    /// one write path; success broadcasts an `Event`, failure returns `Reject`.
    Intent {
        /// Client-chosen correlation token echoed on `Event`/`Reject`.
        intent_id: Uuid,
        /// The proposed operations, applied all-or-nothing.
        ops: Vec<Operation>,
    },
    /// Explicit gap recovery from the client's sequence guard.
    ResyncRequest {
        /// The first seq to replay, INCLUSIVE — the next seq the client has
        /// not yet applied (both resync tiers deliver `seq >= from_seq`).
        from_seq: i64,
    },
    /// Time calibration ping carrying the client's send timestamp.
    TimePing {
        /// Client send timestamp, echoed back in `TimePong`.
        client_t0: i64,
    },
    /// Heartbeat reply.
    Pong,
    /// A full-text search request, correlated by `request_id`. `cursor` is the
    /// opaque page token returned by a prior `SearchResult`. When `subscribe` is
    /// true, the initial `SearchResult` is followed by `SearchUpdate`s on change
    /// (a live top-N subscription keyed by `request_id`).
    Search {
        /// Correlation token for the result/update/error frames.
        request_id: Uuid,
        /// Raw query text (sanitized server-side into an FTS MATCH).
        query: String,
        /// Maximum hits per page.
        limit: u32,
        /// Opaque page token from a prior `SearchResult`; `None` = first page.
        cursor: Option<String>,
        /// True = keep a live top-N subscription pushing `SearchUpdate`s.
        #[serde(default)]
        subscribe: bool,
    },
    /// Cancel a live search subscription (idempotent; unknown id ignored).
    Unsubscribe {
        /// The live search to cancel.
        request_id: Uuid,
    },
    /// Subscribe to a derived scene channel (`compute_derived` currently recognizes
    /// "vision", plus a debug-only "identity" channel); unknown channels yield SceneError.
    /// `as_user` (see-as-player) is **GM-only**: it views the channel as that user; the
    /// server rejects it for non-GMs and resolves the target's role server-side. Omitted/None =
    /// the connection's own view.
    SceneSubscribe {
        /// Correlation token for the derived pushes/errors.
        request_id: Uuid,
        /// Channel name (e.g. "vision").
        channel: String,
        /// GM-only see-as-player target; `None` = the connection's own view.
        #[serde(default)]
        #[ts(optional)]
        as_user: Option<Uuid>,
    },
    /// Cancel a derived subscription by request id.
    SceneUnsubscribe {
        /// The derived subscription to cancel.
        request_id: Uuid,
    },
    /// A transient location ping at scene coords. Relayed out-of-band to the world
    /// room with the sender stamped; never sequenced, logged, or a document (#3).
    /// Coordinates are not validated; the scene must exist in this world and grant the sender
    /// READ (silent drop otherwise); rate-limited per connection.
    ScenePing {
        /// Scene the ping lands on (must grant the sender READ).
        scene: Uuid,
        /// Scene-coordinate x.
        x: f64,
        /// Scene-coordinate y.
        y: f64,
    },
    /// A one-shot grid pathfinding request, correlated by `request_id`. `start`/`waypoints` are
    /// scene coords; `waypoints`' LAST element is the goal. The route is mask-bounded for non-GM
    /// requesters.
    ///
    /// `token`, when present, is the token the route is for: the server AUTHORIZES it (effectively
    /// owned by the requester AND parented to `scene`) and then DERIVES the footprint from its
    /// document, IGNORING `footprint_radius` — so a route preview and the authoritative gate cannot
    /// disagree about the mover's size. It is NOT a presence proof: scene presence remains the
    /// separate ownership scan in `handle_pathfind`, which naming a token neither replaces nor
    /// satisfies. When absent, `footprint_radius` (grid units, the client's `footprintRadius`) is
    /// honored and the result is an explicitly hypothetical preview carrying no
    /// preview-equals-execution guarantee.
    Pathfind {
        /// Correlation token for `PathResult`/`PathError`.
        request_id: Uuid,
        /// Scene to route on.
        scene: Uuid,
        /// Route origin, scene coords.
        start: (f64, f64),
        /// Intermediate points; the LAST element is the goal, scene coords.
        waypoints: Vec<(f64, f64)>,
        /// Mover radius in grid units — the client's `footprintRadius`/`resolveFootprintGeometry`
        /// convention (a hex scene's radius is the circumscribing radius of the authored hex
        /// count, never a square approximation). IGNORED when `token` is named.
        footprint_radius: f64,
        /// The token the route is for; authorized server-side and the source
        /// of the authoritative footprint (see the variant doc).
        #[serde(default)]
        #[ts(optional)]
        token: Option<Uuid>,
    },
    /// A server-authoritative move request: the client submits the previewed cell-center scene
    /// points (start … goal) for a token it controls. The server validates, executes the move,
    /// and broadcasts `MoveStream` out-of-band to the scene on success, or replies `MoveError`
    /// to the originator on failure. `path` carries the exact route preview so the server can
    /// reproduce the animation.
    MoveRequest {
        /// Correlation token for `MoveError` (success echoes via `MoveStream`).
        request_id: Uuid,
        /// Scene the token moves on.
        scene: Uuid,
        /// The token to move (must be effectively owned by the requester).
        token_id: Uuid,
        /// Ordered cell-center scene points: start … goal (inclusive). Type is `[f64; 2]` not a
        /// tuple so the TS binding emits `[number, number][]` (array literal, not tuple object).
        path: Vec<[f64; 2]>,
    },
    /// Author a chat message. The server sanitizes `content` and CONSTRUCTS the
    /// stored message doc (server-authoritative ingest). The sole message-
    /// authoring path — a client `Create` of a `message` doc is rejected.
    /// `request_id` correlates a rejection back to the sender via `ChatError`
    /// (success is confirmed by the broadcast `Event` echo, same as `Intent`).
    SendMessage {
        /// Correlation token for a `ChatError` rejection.
        request_id: Uuid,
        /// Target channel id.
        channel: String,
        /// Raw message text (sanitized server-side).
        content: String,
        /// Optional in-character attribution (authz-checked server-side).
        #[serde(default)]
        actor_owner: Option<ActorOwnerRef>,
        /// Visibility policy (public / gm-only / whisper).
        #[serde(default)]
        audience: Audience,
    },
    /// Edit an existing message the requester owns (or any, if GM). The server
    /// re-runs the sanitize+command pipeline; audience/channel are frozen.
    /// `request_id` correlates a rejection back to the sender via `ChatError`.
    EditMessage {
        /// Correlation token for a `ChatError` rejection.
        request_id: Uuid,
        /// The message to edit.
        message_id: Uuid,
        /// Replacement text (re-sanitized server-side).
        content: String,
    },
    /// Soft-delete a message the requester owns (or any, if GM): the doc stays
    /// in the sequenced log as a tombstone (content cleared, deleted_at set).
    /// `request_id` correlates a rejection back to the sender via `ChatError`.
    DeleteMessage {
        /// Correlation token for a `ChatError` rejection.
        request_id: Uuid,
        /// The message to tombstone.
        message_id: Uuid,
    },
}

/// Which tier served a resync.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum ResyncSource {
    /// Served from the room's in-memory ring buffer.
    Buffer,
    /// Served from the persisted `world_events` log.
    Log,
}

/// Error categories surfaced over the socket.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum WsErrorCode {
    /// The named world does not exist (or is invisible to the caller).
    WorldNotFound,
    /// The frame failed to parse/validate.
    BadMessage,
    /// The write path failed to apply a publish.
    PublishFailed,
    /// The caller lacks the required authority.
    Forbidden,
    /// Unexpected server-side failure (details logged, never echoed).
    Internal,
}

/// Why an `Intent` was rejected. Mirrors the write-path `DataError` categories
/// the client can act on: re-auth, re-read+retry, or fix the payload.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    /// Authorization refused — re-auth or give up.
    Forbidden,
    /// OCC pre-image mismatch or duplicate singleton — re-read and retry.
    Conflict,
    /// Anything else: structurally invalid payload, an absent target, or an
    /// internal failure — not retryable without changing the request.
    Invalid,
}

/// The kind of asset mutation an `AssetChanged` frame reports.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum AssetOp {
    /// The asset's bytes were replaced (version bumped; re-fetch).
    Replaced,
    /// The asset row was removed.
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
        /// The joined world.
        world: Uuid,
        /// The world's latest committed seq at join time.
        current_seq: i64,
        /// Server wall-clock at send, Unix epoch milliseconds.
        server_time: i64,
        /// The running server's semver (`CARGO_PKG_VERSION`). The client's
        /// load-time engine-compat gate checks each external module's
        /// `engines.shadowcat` range against this; delivered here (authenticated,
        /// per-session) rather than on public `/api/config` to avoid disclosing
        /// the exact build to unauthenticated callers.
        server_version: String,
        /// The world's default per-document capability grants.
        world_default_grants: crate::data::document::CapabilityGrants,
        /// The connecting user's role in this world.
        user_role: crate::data::document::WorldRole,
        /// Declarative path-prefix capability requirements (advisory mirror).
        capability_requirements: Vec<crate::data::document::CapabilityRequirement>,
        /// The world's UI contract declarations, so the client can validate its
        /// loaded module set against the world's declared topology.
        contract_declarations: Vec<crate::data::document::ContractDeclaration>,
        /// The world's structural schema declarations (tier-2), so the client
        /// can mirror expectations. Informational/parity only — tier-1 Zod
        /// validates client-side; this is NOT a client enforcement gate.
        schema_declarations: Vec<crate::data::document::SchemaDeclaration>,
    },
    /// A sequenced broadcast carrying the authoritative command. `intent_id` is
    /// the originator's correlation token; it is `None` on the shared broadcast
    /// (an originator confirms its own write by receiving this echo of its
    /// authored command), and `Some` when the write was made under an
    /// intent id, correlating this Event back to that specific intent.
    Event {
        /// The committed, per-recipient-filtered command.
        command: Command,
        /// Originator's correlation token; `None` on the shared broadcast.
        intent_id: Option<Uuid>,
    },
    /// An `Intent` the write path refused, addressed to its originator only.
    Reject {
        /// The refused intent's correlation token.
        intent_id: Uuid,
        /// Why it was refused.
        reason: RejectReason,
    },
    /// Opens a resync replay range.
    ResyncBegin {
        /// First seq delivered in the replay (inclusive; equals the client's
        /// requested `from_seq`).
        from_seq: i64,
        /// Last seq the replay will deliver.
        to_seq: i64,
        /// Which tier serves the replay.
        source: ResyncSource,
    },
    /// Closes a resync replay range; live delivery resumes after this.
    ResyncEnd {
        /// The authoritative seq after replay; live delivery resumes here.
        current_seq: i64,
    },
    /// Time calibration reply: echoes the client send time, adds the server time.
    TimePong {
        /// Echo of the ping's client send time.
        client_t0: i64,
        /// Server wall-clock at reply, Unix epoch milliseconds.
        server_t: i64,
    },
    /// Heartbeat.
    Ping,
    /// A non-fatal or fatal error, by code.
    Error {
        /// Machine-actionable category.
        code: WsErrorCode,
        /// Player-presentable text (never internal details).
        message: String,
    },
    /// Terminal eviction notice: the recipient's world or account is being
    /// deleted. `user: None` addresses every connection in the room (world
    /// deletion); `Some(id)` addresses only that user's connections (account
    /// deletion — broadcast to every room, non-targets skip it silently). The
    /// egress loop delivers this frame, sends a protocol Close, and terminates
    /// the connection; the client must treat it as terminal (no reconnect).
    Evicted {
        /// `None` = every connection in the room; `Some(id)` = that user only.
        user: Option<Uuid>,
    },
    /// Results for the `Search` with this `request_id`. Documents are already
    /// filtered for the recipient. `next_cursor` is `None` when exhausted.
    SearchResult {
        /// The originating search's correlation token.
        request_id: Uuid,
        /// Per-recipient-filtered hits, rank order.
        hits: Vec<SearchHit>,
        /// Opaque next-page token; `None` = exhausted.
        next_cursor: Option<String>,
    },
    /// The `Search` with this `request_id` failed.
    SearchError {
        /// The failed search's correlation token.
        request_id: Uuid,
        /// Player-presentable failure text.
        message: String,
    },
    /// A live subscription's refreshed top-N (full replace). Documents are
    /// already filtered for the recipient.
    SearchUpdate {
        /// The live subscription's correlation token.
        request_id: Uuid,
        /// The refreshed, per-recipient-filtered top-N (full replace).
        hits: Vec<SearchHit>,
    },
    /// A derived-state push: coalesced, per recipient, ordered after the
    /// document events it reflects via `computed_at_seq`. `payload` is opaque to
    /// the transport (#6).
    SceneDerived {
        /// The subscription's correlation token.
        request_id: Uuid,
        /// The channel this push belongs to.
        channel: String,
        /// The document seq this state was computed at (orders vs events).
        computed_at_seq: i64,
        /// Channel-defined derived state; opaque to the transport.
        #[ts(type = "unknown")]
        payload: serde_json::Value,
    },
    /// A derived subscription failed (e.g. unknown channel).
    SceneError {
        /// The failed subscription's correlation token.
        request_id: Uuid,
        /// Player-presentable failure text.
        message: String,
    },
    /// Out-of-band asset mutation notice. Carries no seq and is never buffered
    /// or resynced; holders re-resolve against the record's `version`.
    AssetChanged {
        /// The mutated asset's id.
        uuid: Uuid,
        /// What happened to it.
        op: AssetOp,
    },
    /// A relayed location ping: the sender's transient marker at scene coords.
    /// Out-of-band (no seq, never buffered/resynced), mirroring `AssetChanged`.
    ScenePing {
        /// Scene the ping landed on.
        scene: Uuid,
        /// Scene-coordinate x.
        x: f64,
        /// Scene-coordinate y.
        y: f64,
        /// Who pinged (senders receive their own echo).
        user: Uuid,
    },
    /// The route for the `Pathfind` with this `request_id`: ordered cell-center scene points
    /// (incl. start + goal) and the total cost in cells (client multiplies `grid.distance.perCell`).
    /// `arrested` is true when an arrest region truncated the route before the requested goal —
    /// the player-facing route never silently ends short without telling the client why.
    PathResult {
        /// The originating pathfind's correlation token.
        request_id: Uuid,
        /// Ordered cell-center scene points, start through goal inclusive.
        path: Vec<(f64, f64)>,
        /// Total route cost in cells (multiply by `grid.distance.perCell`).
        cost: f64,
        /// True when an arrest region truncated the route short of the goal.
        arrested: bool,
    },
    /// The `Pathfind` with this `request_id` failed (unreachable / invalid request / search exceeded).
    PathError {
        /// The failed pathfind's correlation token.
        request_id: Uuid,
        /// Player-presentable failure text.
        message: String,
    },
    /// A `MoveRequest` was rejected (token already moving, caller not owner, malformed path, etc.).
    /// Addressed to the originating connection only; never broadcast.
    MoveError {
        /// The refused move's correlation token.
        request_id: Uuid,
        /// Player-presentable failure text.
        message: String,
    },
    /// A `SendMessage`/`EditMessage`/`DeleteMessage` was rejected. One shared
    /// variant covers all three chat ops: they share a single error enum
    /// (`chat::SendMessageError`) and its player-presentable `Display`; the
    /// failed op is implicit in which request `request_id` belongs to.
    /// Addressed to the originating connection only; never broadcast. `message`
    /// is `SendMessageError`'s `Display` text — authorization/existence/internal
    /// classes are already collapsed to a generic string there (no leak).
    ChatError {
        /// The refused chat op's correlation token.
        request_id: Uuid,
        /// `SendMessageError`'s player-presentable `Display` text.
        message: String,
    },
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
        /// Total terrain-weighted movement cost accumulated over the executed move.
        /// Informational — no per-turn budget cap consumes it in v1.
        /// `Some(cost)` for the mover and a GM (trusted, full information); `None` for a
        /// clipped observer, mirroring `mover_vision`'s null-for-observers treatment — the
        /// authoritative cost may reflect secret-region (`gm_only`) terrain the observer's
        /// clipped `samples` don't show, and disclosing it would let an observer detect hidden
        /// terrain by comparing the visible portion of the move against the reported total.
        cost: Option<f64>,
        /// `true` when the move stopped before the requested goal — wall, mask,
        /// region-impassable, or region-arrest. The authoritative answer: a client cannot
        /// derive it from `stop` alone, because a region-arrest on the FINAL step ends the
        /// move AT the goal coordinate and so is indistinguishable from an untruncated move
        /// by geometry.
        /// `Some(flag)` for the mover and a GM (trusted, full information); `None` for a
        /// clipped observer, on the same grounds as `cost` — the observer's `samples` and
        /// `stop` are already clipped to what they witnessed, so a truthful `truncated` would
        /// disclose whether anything blocked the token BEYOND their vision, revealing the
        /// presence of a wall or a `gm_only` region they cannot see.
        truncated: Option<bool>,
    },
}

impl ServerMsg {
    /// seq of an `Event` frame, else `None`. Only `Event`s are buffered/resynced.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::protocol::ServerMsg;
    ///
    /// // Out-of-band frames carry no seq — egress skips gap/resync logic for them.
    /// assert_eq!(ServerMsg::Ping.event_seq(), None);
    /// ```
    pub fn event_seq(&self) -> Option<i64> {
        match self {
            ServerMsg::Event { command, .. } => Some(command.seq),
            _ => None,
        }
    }

    /// server-stamped ts of an `Event` frame, else `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::protocol::ServerMsg;
    ///
    /// assert_eq!(ServerMsg::Ping.event_ts(), None);
    /// ```
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
            token: None,
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"type\":\"pathfind\""), "got {s}");
        let back: ClientMsg = serde_json::from_str(&s).unwrap();
        assert!(matches!(back, ClientMsg::Pathfind { .. }));

        let ok = ServerMsg::PathResult {
            request_id: Uuid::from_u128(1),
            path: vec![(50.0, 50.0)],
            cost: 2.0,
            arrested: true,
        };
        let json = serde_json::to_string(&ok).unwrap();
        assert!(json.contains("\"type\":\"path_result\""));
        let back: ServerMsg = serde_json::from_str(&json).unwrap();
        match back {
            ServerMsg::PathResult { arrested, .. } => assert!(arrested),
            _ => panic!("expected PathResult"),
        }
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
            cost: Some(3.5),
            truncated: Some(true),
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
                cost,
                truncated,
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
                assert_eq!(cost, Some(3.5), "mover/GM path: cost is disclosed");
                assert_eq!(
                    truncated,
                    Some(true),
                    "mover/GM path: truncation is disclosed"
                );
            }
            _ => panic!("expected MoveStream"),
        }
        // None mover_vision + None cost path — verify both round-trip as None for a clipped observer.
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
            cost: None,
            truncated: None,
        };
        let wire2 = serde_json::to_string(&msg_no_vision).unwrap();
        let back2: ServerMsg = serde_json::from_str(&wire2).unwrap();
        match back2 {
            ServerMsg::MoveStream {
                mover_vision,
                cost,
                truncated,
                ..
            } => {
                assert_eq!(
                    mover_vision, None,
                    "observer path: mover_vision must round-trip as None"
                );
                assert_eq!(
                    cost, None,
                    "observer path: cost must round-trip as None (secrecy: must not disclose \
                     authoritative cost, which may reflect secret terrain, to an observer)"
                );
                assert_eq!(
                    truncated, None,
                    "observer path: truncated must round-trip as None (secrecy: a truthful flag \
                     answers whether anything stopped the token BEYOND the observer's clipped \
                     view, disclosing a wall or gm_only region they cannot see)"
                );
            }
            _ => panic!("expected MoveStream"),
        }
    }

    #[test]
    fn send_message_frame_parses() {
        let raw = r#"{"type":"send_message","request_id":"00000000-0000-0000-0000-0000000000aa","channel":"all","content":"hi","actor_owner":null}"#;
        let msg: ClientMsg = serde_json::from_str(raw).unwrap();
        match msg {
            ClientMsg::SendMessage {
                request_id,
                channel,
                content,
                actor_owner,
                audience,
            } => {
                assert_eq!(request_id, Uuid::from_u128(0xaa));
                assert_eq!(channel, "all");
                assert_eq!(content, "hi");
                assert!(actor_owner.is_none());
                assert_eq!(
                    audience,
                    crate::chat::Audience::Public,
                    "omitted audience defaults to Public"
                );
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn edit_and_delete_frames_carry_request_id() {
        let edit: ClientMsg = serde_json::from_str(
            r#"{"type":"edit_message","request_id":"00000000-0000-0000-0000-0000000000ab","message_id":"00000000-0000-0000-0000-000000000001","content":"fixed"}"#,
        )
        .unwrap();
        match edit {
            ClientMsg::EditMessage { request_id, .. } => {
                assert_eq!(request_id, Uuid::from_u128(0xab))
            }
            _ => panic!("wrong variant"),
        }
        let del: ClientMsg = serde_json::from_str(
            r#"{"type":"delete_message","request_id":"00000000-0000-0000-0000-0000000000ac","message_id":"00000000-0000-0000-0000-000000000001"}"#,
        )
        .unwrap();
        match del {
            ClientMsg::DeleteMessage { request_id, .. } => {
                assert_eq!(request_id, Uuid::from_u128(0xac))
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn chat_error_frame_round_trips_and_is_tagged() {
        let e = ServerMsg::ChatError {
            request_id: Uuid::from_u128(9),
            message: "You are not permitted to send this message.".into(),
        };
        // Correlated reply, not a sequenced Event: never buffered/resynced.
        assert_eq!(e.event_seq(), None);
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"type\":\"chat_error\""), "got {s}");
        let _back: ServerMsg = serde_json::from_str(&s).unwrap();
    }

    #[test]
    fn send_message_frame_parses_whisper_audience() {
        let raw = r#"{"type":"send_message","request_id":"00000000-0000-0000-0000-0000000000aa","channel":"all","content":"psst","actor_owner":null,"audience":{"kind":"whisper","recipients":["00000000-0000-0000-0000-000000000001"]}}"#;
        let msg: ClientMsg = serde_json::from_str(raw).unwrap();
        match msg {
            ClientMsg::SendMessage { audience, .. } => {
                assert_eq!(
                    audience,
                    crate::chat::Audience::Whisper {
                        recipients: vec![Uuid::from_u128(1)]
                    }
                );
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn send_message_frame_parses_gm_only_audience() {
        let raw = r#"{"type":"send_message","request_id":"00000000-0000-0000-0000-0000000000aa","channel":"gm","content":"for your eyes only","actor_owner":null,"audience":{"kind":"gm_only"}}"#;
        let msg: ClientMsg = serde_json::from_str(raw).unwrap();
        match msg {
            ClientMsg::SendMessage { audience, .. } => {
                assert_eq!(audience, crate::chat::Audience::GmOnly);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn welcome_carries_caps_role_and_requirements() {
        use crate::data::document::{CapabilityGrants, WorldRole};
        let w = ServerMsg::Welcome {
            world: Uuid::from_u128(1),
            current_seq: 0,
            server_time: 0,
            server_version: "0.0.0-test".to_string(),
            world_default_grants: CapabilityGrants::default(),
            user_role: WorldRole::Player,
            capability_requirements: Vec::new(),
            contract_declarations: Vec::new(),
            schema_declarations: Vec::new(),
        };
        let json = serde_json::to_value(&w).unwrap();
        assert_eq!(json["type"], "welcome");
        assert_eq!(json["user_role"], "player");
        assert!(json.get("world_default_grants").is_some());
        assert!(json.get("capability_requirements").is_some());
        assert!(json.get("contract_declarations").is_some());
        assert!(json.get("schema_declarations").is_some());
        assert_eq!(json["server_version"], "0.0.0-test");
    }
}
