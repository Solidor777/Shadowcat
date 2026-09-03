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

use crate::chat::{ActorOwnerRef, Audience, WireRecalcOp};
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
    /// A transient emote over a token. Relayed out-of-band to the world room with the
    /// sender stamped; never sequenced, logged, or a document (mirrors `ScenePing`).
    /// The token must be parented to `scene` and effectively owned by the sender
    /// (a GM is exempt from the ownership half) — silent drop otherwise; rate-limited
    /// per user on its own budget. `emote` must be 1..=16 bytes (1–4 emoji graphemes).
    Emote {
        /// Scene the token stands on.
        scene: Uuid,
        /// Token the emote plays over (must be effectively owned by the sender).
        token: Uuid,
        /// The emote glyph(s); 1..=16 bytes.
        emote: String,
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
    /// satisfies. When absent, `footprint_radius` (grid units) is honored and the result is an
    /// explicitly hypothetical preview carrying no preview-equals-execution guarantee.
    Pathfind {
        /// Correlation token for `PathResult`/`PathError`.
        request_id: Uuid,
        /// Scene to route on.
        scene: Uuid,
        /// Route origin, scene coords.
        start: (f64, f64),
        /// Intermediate points; the LAST element is the goal, scene coords.
        waypoints: Vec<(f64, f64)>,
        /// Mover radius in grid units, on `scene::footprint::resolve_footprint_cells`'s
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
    /// GM-only roll correction: locates the targeted `RollEmbed` by `roll_id`
    /// (never by array index) and re-derives it via the dice engine's
    /// `recalculate`, appending an auditable `recalc_history` entry. Same
    /// asymmetric reply protocol as `SendMessage`; a non-GM sender is
    /// rejected via a correlated `ChatError`.
    RecalcRoll {
        /// Correlation token for a `ChatError` rejection.
        request_id: Uuid,
        /// The message carrying the targeted roll.
        message_id: Uuid,
        /// The targeted roll's stable id (`Segment::RollEmbed::roll_id`).
        roll_id: Uuid,
        /// The targeted mutation(s) to apply.
        ops: Vec<WireRecalcOp>,
    },
    /// Activate a combat: pauses any other active combat on its scene in the same command;
    /// a combat with `turn == None` initializes (round 1, first turn), one with a turn resumes.
    /// A rejection replies `CombatError`, correlated by `request_id`; success is confirmed by
    /// the broadcast `Event` echo.
    CombatStart {
        /// Correlation token for `CombatError`.
        request_id: Uuid,
        /// The combat.
        combat_id: Uuid,
    },
    /// Deactivate a combat; nothing else runs. Same reply protocol as `CombatStart`.
    CombatPause {
        /// Correlation token for `CombatError`.
        request_id: Uuid,
        /// The combat.
        combat_id: Uuid,
    },
    /// Run end-of-combat effect cleanup, then delete the combat (children cascade). Same reply
    /// protocol as `CombatStart`.
    CombatEnd {
        /// Correlation token for `CombatError`.
        request_id: Uuid,
        /// The combat.
        combat_id: Uuid,
    },
    /// End the current turn (GM, or its owner under `OwnerMayEnd`). Same reply protocol as
    /// `CombatStart`.
    CombatAdvance {
        /// Correlation token for `CombatError`.
        request_id: Uuid,
        /// The combat.
        combat_id: Uuid,
    },
    /// Step back one turn record (GM). Same reply protocol as `CombatStart`.
    CombatRewind {
        /// Correlation token for `CombatError`.
        request_id: Uuid,
        /// The combat.
        combat_id: Uuid,
    },
    /// Roll initiative for the named combatants on `channel`; posts one message per roll. Same
    /// reply protocol as `CombatStart`.
    CombatRoll {
        /// Correlation token for `CombatError`.
        request_id: Uuid,
        /// The combat.
        combat_id: Uuid,
        /// Chat channel the results post to (dice settings resolve per channel).
        channel: String,
        /// The rolls.
        rolls: Vec<CombatRollEntry>,
    },
    /// Adjust one tracked resource; the server clamps to `[0, max]`. Same reply protocol as
    /// `CombatStart`.
    CombatResource {
        /// Correlation token for `CombatError`.
        request_id: Uuid,
        /// The combat.
        combat_id: Uuid,
        /// Target combatant.
        combatant_id: Uuid,
        /// Registry key.
        resource: String,
        /// Delta or set.
        op: ResourceOp,
    },
    /// Rebuild `order` from initiative without rolling (GM). Same reply protocol as
    /// `CombatStart`.
    CombatSort {
        /// Correlation token for `CombatError`.
        request_id: Uuid,
        /// The combat.
        combat_id: Uuid,
    },
}

/// One initiative roll within a `ClientMsg::CombatRoll` request.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct CombatRollEntry {
    /// The combatant to roll initiative for.
    pub combatant_id: Uuid,
    /// Dice notation for the roll (e.g. `1d20+3`), sent as a RAW template:
    /// dotted references (`1d20 + init`) resolve SERVER-side against this
    /// combatant's formula host (its token-embedded actor copy, else its
    /// linked actor) at execution — never pre-substituted by the client.
    /// Pre-substituted literals like `1d20 + 3[init]` remain valid (a
    /// labeled constant is already plain notation).
    pub notation: String,
}

/// How `ClientMsg::CombatResource` adjusts a tracked resource. The server clamps the
/// resulting value to `[0, max]` in both cases.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResourceOp {
    /// Add `amount` to the current value (negative to subtract).
    Delta {
        /// The signed adjustment.
        amount: f64,
    },
    /// Overwrite the current value outright.
    Set {
        /// The value to set.
        value: f64,
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
    /// A new asset row exists (version 1); listings are stale, no URL changes.
    Created,
    /// The asset's bytes were replaced (version bumped; re-fetch).
    Replaced,
    /// Name, folder or tags changed (version unchanged); listings are stale,
    /// no URL changes.
    Moved,
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

/// A single carried-light sample in a `MoveStream` timeline, paired with a `PosSample` by
/// `t_ms`: the mover's resolved carried emission (`SceneEcs::token_light_emission`) raycast at
/// that instant's position through the SAME `scene::emitters::light_polygon` the committed
/// illumination field uses. `pos` is the emitter position and `bright`/`dim` its reaches, all
/// in SCENE units — `dim` is also the per-recipient admission disc
/// (`ws::move_clip::admit_light_samples`); `color` is the packed `0xRRGGBB` tint;
/// `intensity`/`falloff` are the emission's photometric fields, carried so the per-recipient
/// clip can compose this glow into the illumination field at the sample's instant
/// (`SceneEcs::recipient_sight`) from the frame alone — a frame is self-describing, never
/// dependent on a registry entry that may have expired. Present only when the mover carries an
/// enabled emission in an environment-lit scene; per recipient at egress the timeline keeps
/// only the samples whose disc reaches that recipient's vision, and is nulled when none does.
/// Within an admitted timeline the polygons are NOT clipped to the recipient's line of sight —
/// the client intersects them with its own fog, and the glow geometry outside it is the
/// accepted disclosure bounded by the emission's own reach.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct LightSample {
    /// Elapsed time in milliseconds — matches the corresponding `PosSample.t_ms`.
    pub t_ms: f64,
    /// The emitter's scene-coordinate position (x, y) at this instant.
    pub pos: [f64; 2],
    /// Full-brightness reach from `pos`, scene units.
    pub bright: f64,
    /// Dim-light outer reach from `pos`, scene units — the admission disc radius.
    pub dim: f64,
    /// Peak illumination level within `bright`, `[0, 1]` (`LightEmission.intensity`).
    pub intensity: f64,
    /// Taper curve across `(bright, dim]` (`LightEmission.falloff`, linear when unauthored).
    pub falloff: crate::data::engine::FalloffCurve,
    /// Packed `0xRRGGBB` light color.
    pub color: u32,
    /// The light's `blocksLight`-occluded illumination polygon(s) at this instant (scene
    /// coords), each an ordered list of [x, y] vertices.
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
        /// The asset's authoritative version at the time of the mutation: the
        /// bumped version for `Replaced` (the value a receiver's cache-bust
        /// key must converge to), the version the row held immediately
        /// before removal for `Deleted`, `1` for `Created`, and the
        /// unchanged current version for `Moved` — a real ordering token in
        /// every case, letting a receiver compare it against any listing
        /// snapshot straddling the mutation.
        version: i64,
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
    /// A relayed emote: the sender's transient glyph over a token. Out-of-band
    /// (no seq, never buffered/resynced), mirroring `ScenePing`.
    Emote {
        /// Scene the token stands on.
        scene: Uuid,
        /// Token the emote plays over.
        token: Uuid,
        /// Who emoted (senders receive their own echo).
        user: Uuid,
        /// The emote glyph(s).
        emote: String,
    },
    /// The route for the `Pathfind` with this `request_id`: ordered cell-center scene points
    /// (incl. start + goal) and the total cost in cells (client multiplies `grid.distance.perCell`).
    /// `arrested` is true when an arrest region truncated the route before the requested goal,
    /// `truncated` when the mover's movement budget did — the player-facing route never silently
    /// ends short without telling the client why.
    PathResult {
        /// The originating pathfind's correlation token.
        request_id: Uuid,
        /// Ordered cell-center scene points, start through goal inclusive.
        path: Vec<(f64, f64)>,
        /// Total route cost in cells (multiply by `grid.distance.perCell`).
        cost: f64,
        /// True when an arrest region truncated the route short of the goal.
        arrested: bool,
        /// True when the mover's movement budget truncated the route short of
        /// the goal (Hard enforcement; reaches only the requester's own preview).
        truncated: bool,
        /// The named token's remaining movement budget in cells, present iff the requester can
        /// READ the combat's combatant for that token (`BudgetGate::enforced`) — regardless of
        /// enforcement mode, so a GM or a `Warn`/`None` mover still sees the number. `None` when
        /// the token names no combatant, the caller cannot read it, or no combat is running.
        budget_cells: Option<f64>,
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
    /// A combat intent (`CombatStart`/`CombatPause`/`CombatEnd`/`CombatAdvance`/`CombatRewind`/
    /// `CombatRoll`/`CombatResource`/`CombatSort`) was rejected. One wording for every refusal —
    /// never distinguishes hidden from absent from not-yours. Addressed to the originating
    /// connection only; never broadcast. Success is confirmed by a correlated `CombatResult`.
    CombatError {
        /// The refused combat intent's correlation token.
        request_id: Uuid,
        /// Player-presentable failure text.
        message: String,
    },
    /// A combat intent was accepted and committed as the sequenced `Event` at `seq`. Addressed to
    /// the originating connection only; never broadcast. The broadcast `Event` remains the state
    /// notification — this frame only correlates it, and may arrive before OR after that `Event`
    /// (`egress_loop` is `biased;` on this connection's own reply channel, ahead of the room
    /// broadcast channel).
    CombatResult {
        /// The confirmed combat intent's correlation token.
        request_id: Uuid,
        /// The committed command's sequence number — matches the broadcast `Event`'s `seq`.
        seq: i64,
    },
    /// Broadcast to the scene, then clipped per recipient at egress: the mover receives the full
    /// trajectory, `mover_vision` and `mover_light`; observers receive only the position samples
    /// their own vision admits, with `mover_vision` nulled and `mover_light` reduced to the
    /// samples whose light reaches them. A recipient reached by the glow alone gets a GLOW-ONLY
    /// frame — `samples` empty, `mover_light` present, `stop`/`duration_ms` at the last admitted
    /// light sample — and a recipient reached by neither receives nothing.
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
        /// INVARIANT: at least one of `samples`/`mover_light` is non-empty — the frame as
        /// broadcast always carries samples (the first at t_ms == 0.0, the starting
        /// cell-center); a per-recipient clip may empty them and keep only the admitted light
        /// (the glow-only frame), or suppress the frame outright when both would be empty.
        samples: Vec<PosSample>,
        /// Per-sample vision polygons for the mover only. `None` for observers, who receive
        /// server-clipped position samples and render against their existing authoritative fog;
        /// the client computes no vision. Sending mover vision to observers would leak geometry.
        mover_vision: Option<Vec<VisionSample>>,
        /// Per-sample carried-light timeline (`LightSample`): the mover's enabled emission
        /// raycast at each sample position, computed only in an environment-lit scene. Full
        /// for the mover and a plain GM; every other recipient keeps only the samples whose
        /// dim-reach disc intersects their own vision at that instant (the same per-instant
        /// vision the position clip reads), and receives `None` when no sample does.
        mover_light: Option<Vec<LightSample>>,
        /// Total terrain-weighted movement cost accumulated over the executed move. The
        /// movement-budget gate (`move_exec::MoveGateInputs::budget`) consumes this quantity; it
        /// equals the route preview's cost (`PathResult.cost`) for the same route.
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
mod protocol_tests;
