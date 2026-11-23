//! Per-world rooms, ring buffer, registry, and telemetry counters.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::{DashMap, DashSet};
use serde::Serialize;
use tokio::sync::{broadcast, Mutex, RwLock};
use ts_rs::TS;
use uuid::Uuid;

use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
use crate::data::document::Document;
use crate::data::engine as eng;
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::snapshot::StoredCommand;
use crate::data::DataError;
use crate::scene::SceneEcs;
use crate::ws::protocol::{ResyncSource, ServerMsg};
use crate::ws::PingRateLimiter;

mod portal;
pub(crate) use portal::{portal_choice_wire_message, PortalChoiceError};

/// The room-facing result of a server-authoritative token move. Production code reads only
/// `frame` (the wire `MoveStream`, already registered in the room's in-flight registry); the
/// other fields restate a subset of `frame`'s content for direct test assertion against the
/// executor's internal outcome and are compiled only for test builds.
pub(crate) struct MoveExecution {
    /// The scene the moved token actually lives in, derived from the ECS — NOT the scene the
    /// request named. Read only by test assertions against the executor's internal outcome
    /// (`frame.scene` carries the same value onto the wire); production code reads `frame` only.
    #[cfg(test)]
    pub scene: Uuid,
    /// The last successfully reached path coordinate (the committed position after the move).
    /// Read only by test assertions (`frame.stop` carries the same value onto the wire).
    #[cfg(test)]
    pub stop: (f64, f64),
    /// Animation duration in milliseconds: the travelled distance converted to grid steps through
    /// the scene shape's `GridShape::world_units_per_cell`, divided by the authored cells-per-second
    /// speed. Zero when `stop == start`. Read only by test assertions.
    #[cfg(test)]
    pub duration_ms: f64,
    /// The full unclipped wire frame, already registered in the room's in-flight registry;
    /// the caller broadcasts it via `broadcast_aux_shared`.
    pub frame: Arc<ServerMsg>,
}

/// An in-flight token move retained for the duration of its client-side animation.
/// Serves two consumers: the per-token moving lock (`end_ms`) and the egress clip, which
/// reads `frame.mover_vision` as the mover's vision TIMELINE so concurrent moves can be
/// clipped against the recipient's vision at each sample's instant (`ws::move_clip`).
/// INVARIANT: `frame` is the full in-process `ServerMsg::MoveStream` — never a clipped copy.
pub(crate) struct ActiveStream {
    /// The user whose move this is (`MoveStream.mover`).
    pub mover: Uuid,
    /// The scene the token lives in (`MoveStream.scene`).
    pub scene: Uuid,
    /// Server epoch-ms the animation ends; the entry is expired when `now >= end_ms`.
    pub end_ms: i64,
    /// The full unclipped frame.
    pub frame: Arc<ServerMsg>,
}

/// Grouped inputs to `Room::execute_move`, avoiding a >7-argument signature (`&self`, `repo`,
/// and `ctx` plus these five would otherwise total 8).
pub(crate) struct MoveRequestInputs {
    /// The scene the request names — checked for agreement against the token's own scene
    /// (`Room::execute_move`'s scene-derivation invariant) but never used to select gate inputs.
    pub scene_id: Uuid,
    /// The token being moved.
    pub token: Uuid,
    /// Ordered scene-coordinate waypoints, start through goal.
    pub path: Vec<(f64, f64)>,
    /// Server-authoritative commit timestamp (also `MoveStream.start_server_ms`).
    pub ts: i64,
    /// Correlates the resulting `MoveStream`/`MoveError` with the originating `MoveRequest`.
    pub request_id: Uuid,
}

/// Grouped inputs to `Room::toggle_door` (mirroring `MoveRequestInputs`).
pub(crate) struct ToggleDoorInputs {
    /// Correlates a rejection's `ServerMsg::DoorToggleError` with the originating
    /// `ClientMsg::ToggleDoor` (success is the broadcast `Event` echo).
    pub request_id: Uuid,
    /// The scene the request names — checked for agreement against the wall's own derived
    /// `parent_id` (`Room::toggle_door`'s derive-from-the-document rule), never used to
    /// select anything.
    pub scene_id: Uuid,
    /// The wall whose door state flips.
    pub wall_id: Uuid,
    /// The target state.
    pub to: eng::DoorState,
    /// Server-authoritative commit timestamp.
    pub ts: i64,
}

/// How far a token may stand from a door and still flip it, in CELLS (converted to world
/// units at the gate via `GridShape::world_units_per_cell`). On a unit square grid, a token
/// standing in a cell DIAGONALLY adjacent to the door's own cell has its center up to
/// √2 ≈ 1.41 cells from a wall segment sitting at the shared cell boundary; 1.5 is the
/// smallest round number clearing that worst case, so a token in any of the 8 cells
/// surrounding the door's cell can always reach it while a token two cells away (minimum
/// distance 2.0) never can.
pub(crate) const DOOR_REACH_CELLS: f64 = 1.5;

/// Why a `ClientMsg::ToggleDoor` was refused. `Display` yields ONE wording for every variant
/// ("door rejected") — the wire message for a NON-GM caller, because a distinguishable wording
/// would disclose something about the wall/token/scene the requester has no right to learn
/// (does this wall exist, is my token close enough, can I see it, is it secretly `gm_only`,
/// is it locked). A GM caller may instead receive the variant-specific diagnostic text
/// (`door_toggle_wire_message`), mirroring the role-conditioned disclosure discipline
/// `combat::CombatError` already establishes: the role decides disclosure, never the variant.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DoorToggleError {
    /// No scene-hydrated wall document by that id (including a doc of another type).
    #[error("door rejected")]
    UnknownWall,
    /// The request's `scene_id` disagrees with the wall's derived parent scene (or the wall's
    /// scene has no live scene document).
    #[error("door rejected")]
    SceneMismatch,
    /// The requester effectively controls no token parented to the wall's scene.
    #[error("door rejected")]
    NoOwnedToken,
    /// The nearest owned token is farther than `DOOR_REACH_CELLS` from the door's segment.
    #[error("door rejected")]
    OutOfReach,
    /// The door segment's cells fall outside the requester's visible-cell set.
    #[error("door rejected")]
    NotVisible,
    /// The requester holds no engine-tier visibility on the wall document itself (a `gm_only`
    /// wall) — geometric adjacency and sight do not imply this (the visibility raycast is
    /// deliberately permission-blind), which is exactly why the check exists.
    #[error("door rejected")]
    WallNotVisible,
    /// The flip would move onto or off `Locked` (or the wall has no door state at all) — a
    /// non-GM may only flip `Closed`↔`Open`.
    #[error("door rejected")]
    Locked,
    /// An authority input or the commit itself failed (repository/serialization).
    #[error("door rejected")]
    Data(#[from] DataError),
}

/// The error-to-wire conversion `ws::conn`'s `handle_toggle_door` applies: the caller's ROLE
/// decides disclosure, never the variant — a non-GM gets the uniform `Display` wording
/// (`DoorToggleError`'s own discipline), a GM gets the variant-specific diagnostic (the same
/// GM-only-reachable distinction `CombatRewind`'s wording precedent establishes). Takes the
/// role as an argument because the error value itself carries no role.
pub(crate) fn door_toggle_wire_message(
    err: &DoorToggleError,
    world_role: crate::data::document::WorldRole,
) -> String {
    if world_role != crate::data::document::WorldRole::Gm {
        return err.to_string();
    }
    match err {
        DoorToggleError::UnknownWall => "no such wall",
        DoorToggleError::SceneMismatch => "the wall is not on that scene",
        DoorToggleError::NoOwnedToken => "you control no token in that scene",
        DoorToggleError::OutOfReach => "no token you control is within reach of the door",
        DoorToggleError::NotVisible => "your token cannot see the door",
        DoorToggleError::WallNotVisible => "the wall is hidden from you",
        DoorToggleError::Locked => "the door is locked",
        DoorToggleError::Data(_) => "internal error",
    }
    .to_string()
}

/// The mover's turn-budget state in the token's scene's active combat, built by
/// `budget_gate_for_token` under the caller's scene read guard. Absent (`None` at the call
/// site) means no gate applies at all — no active combat on the scene, or the token names no
/// combatant in it.
pub(crate) struct BudgetGate {
    /// The combatant document to decrement on a successful move.
    combatant_id: Uuid,
    /// The `resource-registry` key `CombatEngine.movement.resource` names.
    resource: String,
    /// The registry's binding for `resource`, or `None` (no registry document,
    /// or no such key) — unresolvable.
    binding: Option<eng::ResourceBinding>,
    /// The combatant's stored `current` for `resource`; `None` = untouched,
    /// which reads as full (the evaluated `max` — the lazy-full rule).
    stored: Option<f64>,
    /// The combatant's formula-host document, cloned under the same guard
    /// (`SceneEcs::combatant_formula_host`).
    host: Option<crate::data::document::Document>,
    /// The scene's `grid.distance.per_cell`, or `None` when absent — `MoveReject::
    /// BudgetUnresolvable` under `Interpretation::PerCell` (there is no distance scale to
    /// convert the resource budget into cells).
    per_cell: Option<f64>,
    /// Whether this combatant holds `CombatEngine.turn`.
    is_turn_owner: bool,
    /// Whether the gate's REFUSALS and TRUNCATION apply to this caller at all: exactly whether
    /// the caller holds whole-document `cap::READ` on the combatant, resolved by
    /// `SceneEcs::combatant_for_token` through the SAME `effective_owner_via` +
    /// `resolve_access_world` pair `filter_command` uses at document egress — so a
    /// PER-DOCUMENT `permissions.users` grant or override moves this flag in lockstep with what
    /// the caller receives on the wire, both being reads of the same live document.
    ///
    /// The WORLD-level half of that resolution is NOT lockstep, so the parity claim above is
    /// scoped to per-document permissions and does not extend to world capability defaults:
    /// `Room::execute_move` reads `world_cap_defaults` fresh per move, while WS egress reads it
    /// once per connection under a documented takes-effect-on-next-reconnect policy. TWO shapes
    /// are reachable, in opposite directions.
    ///
    /// A world-level READ GRANT reaches this flag before it reaches that connection's egress: the
    /// gate binds a caller whose grant is genuine and already authoritative, and delivery follows
    /// on reconnect. Narrow — it admits nothing egress would not eventually deliver anyway.
    ///
    /// A world-level READ REVOCATION is the inverse, and fails OPEN.
    /// `http::routes::set_world_capability_defaults` REPLACES the whole `WorldCapDefaults` rather
    /// than extending it, so a caller whose `cap::READ` on the combatant came only from a world
    /// grant loses it from this fresh read immediately: `enforced` goes false, the gate stands
    /// down entirely, and that mover moves UNBUDGETED for as long as their still-connected
    /// session's cached defaults keep delivering the document. `resolve_access_world` being
    /// additive (it extends `caps`, never removes) does not bound this — the defaults VALUE it
    /// reads is what shrank. Accepted as a documented residual, not a fixed one: the window is a
    /// gameplay-budget laxity with no disclosure component (the mover reads that combatant
    /// throughout), and it closes on their next reconnect like every other world-defaults change.
    ///
    /// This second shape is specific to THIS gate. `combat::authorize` resolves the same world
    /// defaults fresh per intent and fails CLOSED on the same revocation — it briefly refuses a
    /// legitimate owner rather than admitting anything — so the two must not be reasoned about
    /// as one residual.
    ///
    /// `false` means the caller cannot read that document, so a `MoveReject::NotYourTurn`
    /// refusal or a budget truncation would disclose both the combatant's existence and its
    /// exact numeric budget through move behaviour alone — reachable without owning the hidden
    /// combatant's own token, since `SceneEcs::combatant_for_token`'s `actor_id` fallback
    /// matches ANY token instanced from the same actor. Such a caller moves exactly as if the
    /// token named no combatant. The resource decrement still records the spend: it writes only
    /// that combatant's own document, and `filter_command`'s `Operation::Update` arm drops the
    /// whole op for any recipient lacking `cap::READ` on it, so no value reaches them.
    enforced: bool,
    /// `CombatEngine.movement.interpretation`.
    interpretation: eng::Interpretation,
    /// `CombatEngine.movement.enforcement`.
    enforcement: eng::Enforcement,
}

impl BudgetGate {
    /// Whether the gate's refusals and truncation apply to this caller at
    /// all — see the field's own doc for the full disclosure rationale.
    /// `handle_pathfind` reads this through the accessor rather than the
    /// private field to decide whether a route preview may disclose
    /// `PathResult.budget_cells` for the named token.
    pub(crate) fn enforced(&self) -> bool {
        self.enforced
    }
}

/// `BudgetGate`, once resolved: the evaluated resource numbers (and, under
/// `Interpretation::PerCell`, the per-cell distance) are guaranteed present — every refusal
/// path has already returned before this is constructed. `cost_to_resource`
/// folds the interpretation into one multiplier so both the ceiling (`current /
/// cost_to_resource`) and the post-move decrement (`MoveOutcome.cost * cost_to_resource`) share
/// one conversion: the scene's `per_cell` distance under `PerCell`, or `1.0` under `Spaces`
/// (`MoveOutcome.cost` is already in the same units as the budget).
pub(crate) struct ResolvedBudget {
    /// The combatant document to decrement on a successful move.
    combatant_id: Uuid,
    /// The `resource-registry` key to decrement.
    resource: String,
    /// The combatant's current value for `resource` (stored, else the
    /// evaluated full), before this move's decrement.
    current: f64,
    /// The STORED value at gate time; `None` = the entry was absent, so the
    /// decrement's OCC pre-image is Null and its write materializes the entry.
    stored: Option<f64>,
    /// `MoveOutcome.cost` (cells) → resource units.
    cost_to_resource: f64,
}

impl ResolvedBudget {
    /// The mover's remaining movement budget in cells, through the shared
    /// `combat::budget::resource_cells` helper — the same number
    /// `handle_pathfind` discloses as `PathResult.budget_cells` for an
    /// enforced caller, computed from this resolution's own
    /// `current`/`cost_to_resource` rather than a second read of either.
    pub(crate) fn resource_cells(&self) -> f64 {
        crate::combat::budget::resource_cells(self.current, self.cost_to_resource)
    }
}

/// Builds the movement-budget gate inputs for `token` on `token_scene`'s active combat, under
/// the caller's scene read guard: the combat/combatant lookup, the registry binding for the
/// combat's movement resource, the combatant's stored entry and formula-host document, the
/// per-cell scale, and the caller's enforcement standing. `None` means no gate applies at all
/// — no active combat, no movement resource named, or the token names no combatant — and the
/// caller must treat that as "moves freely", never a refusal. Shared by `Room::execute_move`
/// and the route-preview clamp so the two cannot resolve the same budget differently.
pub(crate) fn budget_gate_for_token(
    scene: &crate::scene::SceneEcs,
    token_scene: Uuid,
    token: Uuid,
    ctx: &crate::data::membership::PermissionContext,
    world_defaults: &crate::data::document::WorldCapDefaults,
) -> Option<BudgetGate> {
    let (combat_id, ce) = scene.active_combat_for_scene(token_scene)?;
    let resource = ce.movement.resource.clone()?;
    let (combatant_id, c, access) =
        scene.combatant_for_token(combat_id, token, ctx, world_defaults)?;
    let binding = scene
        .resource_registry_engine()
        .and_then(|r| r.resources.get(&resource).map(|res| res.binding.clone()));
    Some(BudgetGate {
        combatant_id,
        binding,
        stored: c.resources.get(&resource).map(|r| r.current),
        host: scene.combatant_formula_host(&c.kind),
        resource,
        per_cell: scene.scene_per_cell(token_scene),
        is_turn_owner: ce.turn == Some(combatant_id),
        // See `BudgetGate::enforced`: whole-document `cap::READ` on the
        // combatant, resolved by the shared authority rather than re-derived
        // from any single permission field.
        enforced: access.has(crate::data::permission::cap::READ),
        interpretation: ce.movement.interpretation,
        enforcement: ce.movement.enforcement,
    })
}

/// The gate's decision for one `BudgetGate`.
pub(crate) enum BudgetResolution {
    /// Hard enforcement, an enforced non-GM caller, and the combatant does not
    /// hold the turn — refuse (`MoveReject::NotYourTurn` names the reason).
    NotYourTurn,
    /// The budget cannot be resolved for an ENFORCED caller — no registry
    /// binding for the resource, a `Mirror` binding (a spend cannot decrement
    /// a derived value, and the server never writes the `system` band), a
    /// formula-evaluation failure, or a missing `grid.distance` scale under
    /// `Interpretation::PerCell` — refuse (`MoveReject::BudgetUnresolvable`
    /// names the reason). An EXEMPT caller never reaches this variant.
    Unresolvable,
    /// The move proceeds: `budget_cells` truncates the walk when `Some`
    /// (enforced Hard callers only), and `decrement` records the spend when
    /// `Some` (any caller whose budget resolved — GM and Warn/None included).
    Resolved {
        /// Truncation ceiling in cells.
        budget_cells: Option<f64>,
        /// The post-move decrement, when the budget resolved.
        decrement: Option<ResolvedBudget>,
    },
}

/// Resolves one `BudgetGate` into the gate's decision. The budget itself
/// comes from `combat::budget::resolve_movement_budget` — the ONE resolution
/// the `"combat"` channel (`SceneEcs::resolved_combats`) reads too, deriving
/// the numbers through `combat::eval::resolved_resource` over the combatant's
/// formula host, so the gate, the channel and the clock cannot price a
/// resource differently. An absent stored entry reads as full; the decrement
/// then materializes it. Exemption (a GM, or `enforced: false` — see
/// `BudgetGate::enforced`) skips every refusal and the truncation but still
/// records the decrement when the budget resolves, and degrades an
/// UNRESOLVABLE budget to "move freely, no decrement" — the same outcome as a
/// token bound to no combatant at all.
pub(crate) fn resolve_budget(bg: &BudgetGate, is_gm: bool) -> BudgetResolution {
    let exempt = is_gm || !bg.enforced;
    if !exempt && !bg.is_turn_owner && matches!(bg.enforcement, eng::Enforcement::Hard) {
        return BudgetResolution::NotYourTurn;
    }
    let budget =
        crate::combat::budget::resolve_movement_budget(&crate::combat::budget::BudgetInputs {
            binding: bg.binding.as_ref(),
            stored: bg.stored,
            host: bg.host.as_ref(),
            interpretation: bg.interpretation,
            per_cell: bg.per_cell,
        });
    match budget {
        Some(mb) => BudgetResolution::Resolved {
            budget_cells: (!exempt && matches!(bg.enforcement, eng::Enforcement::Hard))
                .then(|| mb.cells()),
            decrement: Some(ResolvedBudget {
                combatant_id: bg.combatant_id,
                resource: bg.resource.clone(),
                current: mb.current,
                stored: bg.stored,
                cost_to_resource: mb.cost_to_resource,
            }),
        },
        None if exempt => BudgetResolution::Resolved {
            budget_cells: None,
            decrement: None,
        },
        None => BudgetResolution::Unresolvable,
    }
}

/// Grouped trailing inputs to `wire_move_stream`, avoiding a >7-argument signature.
struct WireMoveInputs<'a> {
    /// Final resting position (scene coords).
    stop: (f64, f64),
    /// Total wall-clock animation budget in milliseconds.
    duration_ms: f64,
    /// Ordered position samples along the route.
    samples: &'a [crate::scene::move_stream::PosSamplePt],
    /// Per-sample vision polygons for the mover; `None` for GM movers or a zero-progress move.
    mover_vision: Option<Vec<crate::scene::move_stream::VisionSamplePt>>,
    /// Per-sample carried-light polygons; `None` for a lightless mover, an all-bright scene,
    /// or a zero-progress move.
    mover_light: Option<Vec<crate::scene::move_stream::LightSamplePt>>,
    /// Total terrain-weighted movement cost accumulated over the executed move.
    cost: f64,
    /// `true` when the move stopped before the requested goal.
    truncated: bool,
}

/// Map an executed move to its wire frame. Polygon vertex counts are capped at
/// `MAX_VISION_POLYGON_VERTS` (fail-closed under-reveal: truncation never over-reveals).
fn wire_move_stream(
    request_id: Uuid,
    token_id: Uuid,
    mover: Uuid,
    start_ms: i64,
    scene: Uuid,
    inputs: WireMoveInputs<'_>,
) -> ServerMsg {
    use crate::scene::move_stream::MAX_VISION_POLYGON_VERTS;
    use crate::ws::protocol::{LightSample, VisionSample};

    // Map internal VisionSamplePt → wire VisionSample, capping polygon vertex count.
    // Fail-closed: truncation under-reveals (the mover sees less of the fog sweep) but
    // never over-reveals hidden geometry to the client.
    let mover_vision = inputs.mover_vision.map(|mvs| {
        mvs.into_iter()
            .map(|vs| VisionSample {
                t_ms: vs.t_ms,
                polygons: vs
                    .polygons
                    .into_iter()
                    .map(|poly| {
                        poly.into_iter()
                            .take(MAX_VISION_POLYGON_VERTS)
                            .map(|(x, y)| [x, y])
                            .collect()
                    })
                    .collect(),
            })
            .collect()
    });

    // Same vertex cap for the carried-light polygons: a truncated light polygon lights less,
    // never more, and the per-recipient admission reads the disc, not the polygon.
    let mover_light = inputs.mover_light.map(|mls| {
        mls.into_iter()
            .map(|ls| LightSample {
                t_ms: ls.t_ms,
                pos: [ls.pos.0, ls.pos.1],
                bright: ls.bright,
                dim: ls.dim,
                intensity: ls.intensity,
                falloff: crate::scene::emitters::wire_falloff(ls.falloff),
                color: ls.color,
                angle: ls.angle,
                rotation: ls.rotation,
                polygons: ls
                    .polygons
                    .into_iter()
                    .map(|poly| {
                        poly.into_iter()
                            .take(MAX_VISION_POLYGON_VERTS)
                            .map(|(x, y)| [x, y])
                            .collect()
                    })
                    .collect(),
            })
            .collect()
    });

    ServerMsg::MoveStream {
        request_id,
        token_id,
        mover,
        scene,
        start_server_ms: start_ms as f64,
        duration_ms: inputs.duration_ms,
        stop: [inputs.stop.0, inputs.stop.1],
        samples: inputs
            .samples
            .iter()
            .map(|s| crate::ws::protocol::PosSample {
                t_ms: s.t_ms,
                pos: [s.pos.0, s.pos.1],
            })
            .collect(),
        mover_vision,
        mover_light,
        // Broadcast in-process carries the full authoritative cost; `clip_move_stream`
        // nulls it per recipient at egress for a clipped observer (secrecy: see
        // `ServerMsg::MoveStream.cost` doc).
        cost: Some(inputs.cost),
        // Same trusted-only treatment as `cost`: full value in-process, nulled per
        // recipient at egress for a clipped observer.
        truncated: Some(inputs.truncated),
    }
}

/// Ring-buffer event cap (hot-resync depth).
const MAX_EVENTS: usize = 1024;
/// Ring-buffer age cap, ms, relative to the newest buffered event.
const MAX_AGE_MS: i64 = 5 * 60 * 1000;
/// Tokio broadcast channel capacity; a receiver farther behind than this lags
/// out and resyncs from the ring/log tiers.
const BROADCAST_CAPACITY: usize = 256;

/// Recent `Event` frames for hot resync, bounded by count and age. Age is
/// measured relative to the newest buffered event's `ts`.
///
/// # Examples
///
/// ```
/// use shadowcat::ws::room::RingBuffer;
///
/// // `push`/`range_from` are crate-internal (`Room` is the sole owner, behind
/// // `Mutex<RingBuffer>`); externally only construction is observable.
/// let _ring = RingBuffer::default();
/// ```
pub struct RingBuffer {
    /// Buffered frames, ascending seq; every entry is a `RoomEvent::Event`.
    events: VecDeque<RoomEvent>,
}

impl RingBuffer {
    /// An empty buffer.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::room::RingBuffer;
    ///
    /// // `push`/`range_from` are crate-internal; externally only construction
    /// // is observable (see `Room`, the sole owner, for the buffer in use).
    /// let _ring = RingBuffer::new();
    /// ```
    pub fn new() -> Self {
        Self {
            events: VecDeque::new(),
        }
    }

    /// Append an `Event` frame and prune by count then age.
    pub(crate) fn push(&mut self, msg: RoomEvent) {
        debug_assert!(msg.event_seq().is_some(), "only Event frames are buffered");
        self.events.push_back(msg);
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
        if let Some(newest) = self.events.back().and_then(|m| m.event_ts()) {
            while let Some(oldest) = self.events.front().and_then(|m| m.event_ts()) {
                if newest - oldest > MAX_AGE_MS {
                    self.events.pop_front();
                } else {
                    break;
                }
            }
        }
    }

    /// Events with `seq >= from_seq`, but only when the whole requested range is
    /// still resident (oldest buffered seq <= from_seq). Otherwise `None` so the
    /// caller falls back to the durable `events_since` cold tier. An empty buffer
    /// returns `None` (cannot prove residency).
    pub(crate) fn range_from(&self, from_seq: i64) -> Option<Vec<RoomEvent>> {
        match self.events.front().and_then(|m| m.event_seq()) {
            Some(oldest) if oldest <= from_seq => Some(
                self.events
                    .iter()
                    .filter(|m| m.event_seq().map(|s| s >= from_seq).unwrap_or(false))
                    .cloned()
                    .collect(),
            ),
            _ => None,
        }
    }
}

impl Default for RingBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Internal broadcast/ring element `Room` fans out on `Room.tx` and buffers in `RingBuffer`.
/// Never serialized to the wire — the client-facing `ServerMsg` (including its own `Event`
/// variant) is untouched by this type's existence. Distinguishes a `StoredCommand`-carrying
/// broadcast (the only case needing the commit-time redaction snapshot) from every OTHER
/// `ServerMsg` variant `Room` broadcasts (pings, presence, `MoveStream`, ...), which pass
/// through unchanged.
#[derive(Debug, Clone)]
pub(crate) enum RoomEvent {
    /// A committed command awaiting per-recipient redaction and reduction to a plain wire
    /// `ServerMsg::Event` at send time.
    Event(Arc<StoredCommand>),
    /// Any other broadcast `ServerMsg`, forwarded unchanged.
    Other(Arc<ServerMsg>),
}

impl RoomEvent {
    /// seq of an `Event` variant, else `None`. Mirrors `ServerMsg::event_seq`.
    pub(crate) fn event_seq(&self) -> Option<i64> {
        match self {
            RoomEvent::Event(stored) => Some(stored.command.seq),
            RoomEvent::Other(msg) => msg.event_seq(),
        }
    }

    /// server-stamped ts of an `Event` variant, else `None`. Mirrors `ServerMsg::event_ts`.
    pub(crate) fn event_ts(&self) -> Option<i64> {
        match self {
            RoomEvent::Event(stored) => Some(stored.command.ts),
            RoomEvent::Other(msg) => msg.event_ts(),
        }
    }
}

/// Per-room telemetry counters (lock-free).
///
/// # Examples
///
/// ```
/// use shadowcat::ws::room::RoomStats;
/// use std::sync::atomic::Ordering;
///
/// let stats = RoomStats::default();
/// assert_eq!(stats.connections.load(Ordering::Relaxed), 0);
/// stats.connections.fetch_add(1, Ordering::Relaxed);
/// assert_eq!(stats.connections.load(Ordering::Relaxed), 1);
/// ```
#[derive(Default)]
pub struct RoomStats {
    /// Live connection count.
    pub connections: AtomicI64,
    /// Sequenced events published since room creation.
    pub events_published: AtomicU64,
    /// Client-reported sequence gaps.
    pub gaps_detected: AtomicU64,
    /// Resyncs served from the ring buffer.
    pub resyncs_hot: AtomicU64,
    /// Resyncs served from the persisted log.
    pub resyncs_cold: AtomicU64,
    /// Frames dropped on lagging receivers (they resync afterward).
    pub lagged_drops: AtomicU64,
}

/// Serializable snapshot of a room's telemetry for the admin debug endpoint.
///
/// # Examples
///
/// ```
/// use shadowcat::ws::room::RoomStatsSnapshot;
///
/// let snap = RoomStatsSnapshot {
///     world_id: uuid::Uuid::nil(),
///     connections: 1,
///     current_seq: 5,
///     events_published: 5,
///     gaps_detected: 0,
///     resyncs_hot: 0,
///     resyncs_cold: 0,
///     lagged_drops: 0,
/// };
/// assert_eq!(snap.current_seq, 5);
/// ```
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct RoomStatsSnapshot {
    /// The room's world.
    pub world_id: Uuid,
    /// Live connection count at snapshot time.
    pub connections: i64,
    /// The room's committed seq at snapshot time.
    pub current_seq: i64,
    /// Sequenced events published since room creation.
    pub events_published: u64,
    /// Client-reported sequence gaps.
    pub gaps_detected: u64,
    /// Resyncs served from the ring buffer.
    pub resyncs_hot: u64,
    /// Resyncs served from the persisted log.
    pub resyncs_cold: u64,
    /// Frames dropped on lagging receivers.
    pub lagged_drops: u64,
}

/// A per-world fan-out room. The `broadcast` channel is intentionally lossy —
/// a lagging receiver gets `Lagged(n)` and resyncs from the ring/log tiers.
///
/// # Examples
///
/// ```
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::data::repository::Repository;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::ws::room::RoomRegistry;
///
/// # #[tokio::main]
/// # async fn main() {
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
/// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
///
/// // `Room::new` is private; `RoomRegistry::get_or_create` is the sole public
/// // constructor, hydrating the room from persisted world/scene state.
/// let room = RoomRegistry::new()
///     .get_or_create(&repo, world.id)
///     .await
///     .unwrap()
///     .unwrap();
/// assert_eq!(room.world_id, world.id);
/// # }
/// ```
pub struct Room {
    /// The world this room fans out for.
    pub world_id: Uuid,
    /// The lossy broadcast sender every connection subscribes to.
    tx: broadcast::Sender<RoomEvent>,
    /// Hot-resync tier (recent events, count/age bounded).
    ring: Mutex<RingBuffer>,
    /// Serializes publishes so seq order equals broadcast order.
    publish_guard: Mutex<()>,
    /// The room's committed sequence watermark.
    current_seq: AtomicI64,
    /// The derived, in-memory scene read-model (vision/movement source).
    scene: RwLock<SceneEcs>,
    /// Telemetry counters.
    pub stats: RoomStats,
    /// Per-token in-flight registry doubling as the moving lock: token → `ActiveStream`.
    /// Expired when `now_millis() >= end_ms` (lazy expiry, no timer); expired/absent entries
    /// are treated as available. Updated by `execute_move` after a successful commit. Also
    /// serves `mover_streams`/`concurrent_streams`, which read `ActiveStream.frame` as the
    /// mover's vision timeline for the egress clip.
    moving: Mutex<HashMap<Uuid, ActiveStream>>,
    /// Per-user resync floor: user_id → this room's `current_seq` at their most recent
    /// cold-start `ClientMsg::Hello { last_seq: None }`. When `resync_floor_enforced`,
    /// an explicit `ResyncRequest.from_seq` is clamped to never go below `floor + 1` for
    /// that user — this is what bounds "any member can request the entire world history
    /// unvalidated". In-memory only: a server restart or room eviction resets it, which
    /// only ever WIDENS the bound back toward the fully-open behavior (never narrows it
    /// below a value a client legitimately established), and self-heals as clients send a
    /// fresh `Hello` on reconnect. Never persisted to the database — deliberately out of
    /// scope: the floor only needs to survive for the lifetime of the room, exactly like
    /// `moving`.
    session_floors: Mutex<HashMap<Uuid, i64>>,
    /// Per-user live connection count in this room, driving the 0<->1 presence-transition
    /// broadcast (`ServerMsg::Presence`). Derived, in-memory only, cleared with the room: there
    /// is no snapshot frame, so a client learns a user's presence only from transitions seen
    /// since its own connect. A `std::sync::Mutex` because the guard is never held across an
    /// `.await` and `presence_connect`/`presence_disconnect` are synchronous. Parallels
    /// `RoomStats::connections` without replacing it: `reap_if_empty` still reads that
    /// aggregate.
    presence: std::sync::Mutex<HashMap<Uuid, u32>>,
    /// Whether an explicit `ClientMsg::ResyncRequest` is clamped against `resync_floor`
    /// (`ws::conn`'s `Egress::Resync` handler reads this via `Room::resync_floor_enforced`
    /// — the internal `Lagged`-driven auto-resync deliberately does NOT consult it, since
    /// that path replays from a connection's own live-tracked watermark, not an untrusted
    /// client-supplied `from_seq`). `true` for every production `RoomRegistry` constructor:
    /// the client unconditionally sends a cold-start `Hello { last_seq: None }` as the first
    /// frame on every socket open (`WsClient`'s `open()`), so every connection this room ever
    /// sees establishes its floor before it could plausibly send a `ResyncRequest`.
    resync_floor_enforced_flag: bool,
    /// Per-user VFX one-shot budget for trigger-fired plays (`fire_region_triggers`'s Teleport
    /// arm) — the SAME `Arc<PingRateLimiter>` instance `RoomRegistry::vfx_rate` hands to
    /// `WsState::vfx_rate`, so a portal's carried VFX draws from the identical bucket the raw
    /// `ClientMsg::PlayVfx` frame and `/fx` already share, rather than a parallel unthrottled
    /// path (never-fork: one validation+authz/budget source per one-shot family).
    vfx_rate: Arc<PingRateLimiter>,
    /// Ephemeral per-combat once-per-turn/round region-trigger dedup —
    /// `fire_turn_round_triggers`'s seen-set, the SAME seen-set shape
    /// `combat::transition`'s per-sweep recovery dedup keeps, living
    /// room-side because the (combatant, region, trigger) identity it keys
    /// on only exists at the firing pass (the pure transition never sees
    /// regions). Never persisted: a restart clears it, which can only
    /// re-permit a fire — the same accepted ephemeral-table residual class
    /// as `SceneEcs::region_membership` and `Room::moving`. `Mutex`, never
    /// locked across an `.await`... the firing pass holds `publish_guard`,
    /// which serializes every updater anyway.
    turn_round_dedup: Mutex<HashMap<Uuid, TurnRoundDedup>>,
    /// Pending portal offers, one per token (`ws::room::portal`): token id → its live offer. Held
    /// in memory only, because an offer is a moment: a restart drops it and `OpenPortal` recovers
    /// it. A `std::sync::Mutex` never held across an `.await`; every writer also holds
    /// `publish_guard`, which makes an answer's validate-then-consume atomic.
    portal_offers: std::sync::Mutex<HashMap<Uuid, portal::PortalOffer>>,
}

/// One firing report: the token that entered, the scene it happened in, the cells it
/// entered (`MoveOutcome::entered_cells` for a move, the new footprint cells for a
/// placement), and the arrest stop position when the walk was region-arrested.
/// `TriggerEvent::Exit` fires from the membership diff, not a per-cell crossing:
/// `fire_region_triggers` replaces `SceneEcs::region_membership`'s stored set with the
/// regions intersecting `occupancy` and fires `Exit` on whatever the stored set loses.
pub(crate) struct TriggerReport {
    /// The scene the entry happened in (derived from the token, never the request).
    pub scene: Uuid,
    /// The entering token.
    pub token: Uuid,
    /// The entered cells: a move's deduped transition sequence, or a placement's
    /// footprint cells at the new position.
    pub entered: Vec<(i32, i32)>,
    /// The arrest stop position, `Some` only when the walk was region-arrested.
    pub arrest_stop: Option<(f64, f64)>,
    /// The path-end OCCUPANCY basis for the `Exit` membership diff: the cells whose
    /// intersection with a region's cell set makes the token "inside" it after this
    /// report. Each call site supplies the SAME cell basis its own Enter detection
    /// uses — a move's stop cell (the move executor's region gate and `entered` are
    /// center-cell keyed) or a placement's full footprint — so an `Enter` and its
    /// later `Exit` for one report kind can never drift onto different bases (a
    /// footprint bleed past the center cell would otherwise register membership a
    /// center-keyed `Enter` never fired, and vice versa).
    pub occupancy: Vec<(i32, i32)>,
    /// When `Some`, the only regions whose `Enter`/`Arrest` triggers may fire from `entered`;
    /// `Exit` is never filtered. A carrier-move report sets it to the attached regions the
    /// token has not yet entered, so a stationary token's `entered` centre cell cannot
    /// re-fire the Enter triggers of an unrelated region it already occupies.
    pub enter_only: Option<Vec<Uuid>>,
}

/// One fired (event, round, combatant, region, trigger-index) tuple — the
/// once-per-turn/round dedup identity for `Room::fire_turn_round_triggers`.
/// Scoped to (combatant, region, trigger identity) — the same axes the
/// recovery dedup's own sets key on — plus the boundary instance (`event`,
/// `round`) so a genuinely NEW turn or round can never be suppressed by an
/// earlier one.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct TurnRoundFireKey {
    /// The boundary event that fired (`TriggerEvent::TurnStart` & kin).
    event: eng::TriggerEvent,
    /// The round the firing boundary belongs to — the boundary instance:
    /// `(round, combatant)` is one turn for turn-scoped events; for
    /// round-scoped events it is the new round `round_wrap` committed.
    round: u32,
    /// The combatant whose token fired. Round-scoped boundaries expand to
    /// one key per covered combatant, so two member tokens of one region
    /// each fire once per round boundary.
    combatant: Uuid,
    /// The region carrying the fired trigger.
    region: Uuid,
    /// The trigger's index in the region's authored `triggers` list
    /// (`regions::fired_turn_round_triggers` returns it) — the trigger
    /// identity content-equality cannot supply: two identical authored
    /// triggers on one region are two distinct identities (a `resource_delta`
    /// authored twice applies twice).
    trigger: usize,
}

/// Per-combat once-per-turn/round dedup state: the keys already fired within
/// the current round window, and the highest round the firing pass has
/// observed for the combat. A fresh instance sees every fire.
#[derive(Default)]
struct TurnRoundDedup {
    /// Highest round observed for the combat; keys older than the previous
    /// round are pruned against it on each pass (`k.round + 1 < last_round`
    /// drops). A key's lifetime is therefore one boundary-to-boundary window
    /// — the same rotation `combat::transition`'s per-sweep recovery dedup
    /// sets keep, retained across commands because the double-fire this set
    /// must catch is a CROSS-command shape: an auto-resolved entry the
    /// `settle_turn` budget parks on has already run its `turn_end`, and the
    /// next `CombatAdvance` ends that SAME (round, combatant) turn again.
    /// A committed `CombatRewind` CLEARS the whole entry
    /// (`Room::commit_combat_and_fire`'s `rewind` arm): the rewind re-runs
    /// recovery when the clock re-crosses those boundaries, so the triggers
    /// riding them re-arm in full — pruning alone would only re-arm rounds
    /// outside the retained window, silently suppressing an ordinary
    /// one-round rewind's re-crossing.
    last_round: u32,
    /// The fired keys inside the current/previous round window.
    keys: std::collections::BTreeSet<TurnRoundFireKey>,
}

/// Combats retained in `Room::turn_round_dedup` before the whole map is
/// cleared wholesale — over-invalidation is the safe direction (a cleared
/// set can only re-permit a legitimate-looking fire, never suppress a first
/// one), the same discipline `MAX_LIGHTING_INPUTS_CACHE_ENTRIES` keeps.
/// Entries otherwise live for the room's lifetime: one small set per combat
/// that ever crossed a boundary.
const MAX_TURN_ROUND_DEDUP_COMBATS: usize = 64;

/// The token a teleport moves: what `Room::apply_teleport_to` reads of it. Bundled to keep
/// that function under the >7-argument signature limit.
struct TeleportSubject<'a> {
    /// The token document id.
    token: Uuid,
    /// The token's current scene (`None` when it sits on no scene).
    scene: Option<Uuid>,
    /// The token's parsed engine body (position/elevation); `None` when it does not parse.
    token_eng: Option<&'a eng::TokenEngine>,
    /// The token's effective owner, whose `vfx_rate` bucket a carried VFX charges; `None`
    /// admits uncharged.
    effective_owner: Option<Uuid>,
}

/// The one hop a fire applies: a fixed destination, or a choice that registers an offer.
enum HopEffect<'a> {
    /// A `TriggerEffect::Teleport` target, applied at once.
    Fixed(&'a eng::PortalTarget),
    /// A `TriggerEffect::PortalChoice`, offered to the token's owner and the GMs.
    Choice(portal::PortalSource<'a>),
}

/// The firing-plan input bundle for `Room::apply_region_trigger_effects` —
/// what fires, the rows the secrecy forcing searches, the combat a
/// `ResourceDelta` resolves against, and the one-hop teleport guard. Bundled
/// to keep the application funnel under the >7-argument signature limit
/// (`MoveRequestInputs`'s own discipline).
struct TriggerEffectPlan<'a> {
    /// The (region id, trigger) pairs to apply, in fire order.
    fired: Vec<(Uuid, eng::RegionTrigger)>,
    /// The identity rows a `ChatNotice`'s audience forcing searches (an
    /// unresolvable row fails closed to forced-GM-only).
    secrecy_rows: &'a [crate::scene::regions::TriggerRegion],
    /// The combat a `ResourceDelta` resolves against: `Some` for the
    /// turn/round fire site (the FIRING combat — correct even for a token
    /// teleported off its combat's scene), `None` for the movement sites
    /// (the token's scene's ACTIVE combat, the rule the movement sites have
    /// always used).
    combat_hint: Option<Uuid>,
    /// Whether a `Teleport` effect may apply on this pass (the one-hop
    /// anti-loop guard; `false` only on a teleport destination's own
    /// re-fire).
    allow_teleport: bool,
}

/// The post-commit tail of one combat intent — everything
/// `Room::commit_combat_and_fire` does AFTER the transition's own commit.
/// Bundled to keep the method under the >7-argument signature limit
/// (`MoveRequestInputs`'s own discipline).
pub(crate) struct CombatTriggerTail<'a> {
    /// The snapshot the transition resolved from (combat id, combatant→token
    /// kinds) — the firing pass's lookup table.
    pub snap: &'a crate::combat::CombatSnapshot,
    /// The transition's turn/round boundary record (empty for every intent
    /// but a boundary-crossing `start`/`advance`).
    pub plan: Vec<crate::combat::transition::TurnRoundBoundary>,
    /// Whether the committed intent was a `CombatRewind`: its commit re-arms
    /// the combat's `turn_round_dedup` entry (recovery re-runs on the
    /// re-crossed boundaries, so the triggers riding them re-fire too).
    pub rewind: bool,
}

impl Room {
    /// A room seeded at `seed_seq` with a hydrated scene read-model.
    ///
    /// # Examples
    ///
    /// ```text
    /// RoomRegistry::get_or_create hydrates and constructs rooms; never build one directly.
    /// ```
    fn new(
        world_id: Uuid,
        seed_seq: i64,
        scene: SceneEcs,
        broadcast_capacity: usize,
        resync_floor_enforced: bool,
        vfx_rate: Arc<PingRateLimiter>,
    ) -> Self {
        let (tx, _rx) = broadcast::channel(broadcast_capacity);
        Self {
            world_id,
            tx,
            ring: Mutex::new(RingBuffer::new()),
            publish_guard: Mutex::new(()),
            current_seq: AtomicI64::new(seed_seq),
            scene: RwLock::new(scene),
            stats: RoomStats::default(),
            moving: Mutex::new(HashMap::new()),
            session_floors: Mutex::new(HashMap::new()),
            presence: std::sync::Mutex::new(HashMap::new()),
            resync_floor_enforced_flag: resync_floor_enforced,
            vfx_rate,
            turn_round_dedup: Mutex::new(HashMap::new()),
            portal_offers: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Read access to the derived scene ECS for the per-connection derived
    /// recompute. Writes happen only in `publish` under `publish_guard`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::ws::room::RoomRegistry;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    /// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
    /// let room = RoomRegistry::new()
    ///     .get_or_create(&repo, world.id)
    ///     .await
    ///     .unwrap()
    ///     .unwrap();
    ///
    /// // A freshly hydrated world has no scene documents yet.
    /// assert!(room.scene().read().await.scene_grid_sizes().is_empty());
    /// # }
    /// ```
    pub fn scene(&self) -> &RwLock<SceneEcs> {
        &self.scene
    }

    /// Subscribe to live frames; also returns the room's current seq so a joiner
    /// knows whether it needs to resync.
    pub(crate) fn subscribe(&self) -> (broadcast::Receiver<RoomEvent>, i64) {
        (
            self.tx.subscribe(),
            self.current_seq.load(Ordering::Acquire),
        )
    }

    /// The room's committed sequence watermark.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::ws::room::RoomRegistry;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    /// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
    /// let room = RoomRegistry::new()
    ///     .get_or_create(&repo, world.id)
    ///     .await
    ///     .unwrap()
    ///     .unwrap();
    ///
    /// // Seeded from the freshly created world's own seq.
    /// assert_eq!(room.current_seq(), 0);
    /// # }
    /// ```
    pub fn current_seq(&self) -> i64 {
        self.current_seq.load(Ordering::Acquire)
    }

    /// Broadcast a non-sequenced, out-of-band frame (e.g. AssetChanged). Unlike
    /// `publish`, it does NOT push to the ring or bump `current_seq`, so a
    /// lagging receiver that resyncs from the ring/log never replays it, and it
    /// also drops when there are no receivers. DELIVERY IS NOT GUARANTEED — a
    /// dropped `AssetChanged` frame is reconciled opportunistically instead: the
    /// frame carries the asset's authoritative `version`, and `AssetResolver.reconcile`
    /// re-syncs any uuid still stale the next time a listing (e.g. `Assets`'s
    /// own `reload`) fetches the true value.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::ws::protocol::ServerMsg;
    /// use shadowcat::ws::room::RoomRegistry;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    /// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
    /// let room = RoomRegistry::new()
    ///     .get_or_create(&repo, world.id)
    ///     .await
    ///     .unwrap()
    ///     .unwrap();
    ///
    /// // Out-of-band: never bumps `current_seq`, unlike `publish`.
    /// room.broadcast_aux(ServerMsg::Ping);
    /// assert_eq!(room.current_seq(), 0);
    /// # }
    /// ```
    pub fn broadcast_aux(&self, msg: ServerMsg) {
        self.broadcast_aux_shared(std::sync::Arc::new(msg));
    }

    /// Broadcast an already-shared out-of-band frame (see `broadcast_aux`).
    pub(crate) fn broadcast_aux_shared(&self, msg: Arc<ServerMsg>) {
        let _ = self.tx.send(RoomEvent::Other(msg));
    }

    /// The one authoritative write path: authorize/validate/sequence `ops`
    /// through `apply_intent`, append to the ring, and broadcast — serialized
    /// per world by `publish_guard` so broadcast order equals seq order. The
    /// broadcast `Event` carries `intent_id: None`; an originator confirms its
    /// own write by receiving this echo. A rejected intent returns its
    /// `DataError` without consuming a seq or broadcasting. `origin` is
    /// forwarded to `apply_intent` to gate the message-Update exemption; every
    /// caller other than the server's own edit/delete revision path passes
    /// `WriteOrigin::Client`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::command::{Operation, WriteOrigin};
    /// use shadowcat::data::document::{Document, PermissionSet, Scope, WorldRole};
    /// use shadowcat::data::membership::PermissionContext;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::ws::room::RoomRegistry;
    /// use uuid::Uuid;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    /// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
    /// let ctx = PermissionContext {
    ///     user_id: gm,
    ///     world_role: WorldRole::Gm,
    /// };
    /// let room = RoomRegistry::new()
    ///     .get_or_create(&repo, world.id)
    ///     .await
    ///     .unwrap()
    ///     .unwrap();
    ///
    /// let doc = Document {
    ///     id: Uuid::new_v4(),
    ///     scope: Scope::World { world_id: world.id },
    ///     // A client-only doc_type (opaque `system` band, no `engine` body required).
    ///     doc_type: "item".into(),
    ///     schema_version: 1,
    ///     name: Some("example".into()),
    ///     source: None,
    ///     base: None,
    ///     owner: None,
    ///     permissions: PermissionSet::default(),
    ///     embedded: Default::default(),
    ///     parent_id: None,
    ///     engine: None,
    ///     system: serde_json::json!({}),
    ///     created_at: 0,
    ///     updated_at: 0,
    /// };
    /// let cmd = room
    ///     .publish(&repo, &ctx, vec![Operation::Create { doc }], 0, WriteOrigin::Client)
    ///     .await
    ///     .unwrap();
    /// assert_eq!(cmd.seq, 1);
    /// assert_eq!(room.current_seq(), 1);
    /// # }
    /// ```
    pub async fn publish(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        ops: Vec<Operation>,
        ts: i64,
        origin: WriteOrigin,
    ) -> Result<Command, DataError> {
        let _guard = self.publish_guard.lock().await;
        // A non-GM may not CHANGE a token's position. Gated movement is request-only and
        // server-executed (`ClientMsg::MoveRequest` → `execute_move`), the only path that can
        // gate each step, arrest a token partway, and stream the authoritative trajectory. A
        // client-authored position write can do none of those, so it is refused rather than
        // validated — strictly stricter than the traversal gate this replaces, leaving
        // `execute_move` the SOLE implementation of the per-cell traversal decision.
        //
        // `token_move` yields (scene, committed_start, post_image_end) over the whole /engine
        // band with all changes applied in array order, so a wholesale `/engine` write or
        // duplicate `/engine/x` entries cannot present a safe target while committing a moved
        // one. It does NOT itself test whether the position changed, so the comparison is here:
        // a write that re-states the same coordinates is not a move. Bitwise inequality, not an
        // epsilon window — an epsilon would grant a free sub-threshold teleport per op.
        //
        // GMs are exempt: a GM places a token where they choose, walls included.
        if ctx.world_role != crate::data::document::WorldRole::Gm {
            // Pending Revealed-mode checks deferred past the ECS read borrow: (scene_id,
            // cells, visible_set). Revealed mode requires an async get_explored call which
            // cannot occur while holding the scene read lock.
            type CellSet = std::collections::BTreeSet<(i32, i32)>;
            let mut revealed_pending: Vec<(
                uuid::Uuid,
                String,
                CellSet,
                CellSet,
                crate::scene::GridKind,
                uuid::Uuid,
            )> = Vec::new();
            // The Create placement gate's mask (`visible_cells_cached`) resolves observer-vision
            // source admission through `resolve_access_world`, which reads the world's capability
            // grants — an await, which must not run under the scene read guard below. Fetched
            // once, ahead of the guard, and only when this batch contains a token Create (which
            // reads the mask) or an Update (whose token rotation write resolves the caller's
            // standing on the token's combatant); every other batch skips the fetch.
            let needs_world_defaults = ops.iter().any(|op| match op {
                Operation::Create { doc } => doc.doc_type == "token" && doc.parent_id.is_some(),
                Operation::Update { .. } => true,
                _ => false,
            });
            let world_defaults = if needs_world_defaults {
                Some(repo.world_cap_defaults(self.world_id).await?)
            } else {
                None
            };
            {
                let scene = self.scene.read().await;
                // Memoize the visible mask per (scene, leniency, mover level) within this publish
                // so a batch of same-floor Creates in the same scene does not recompute the mask
                // per token — level is part of the key because the mask is now level-scoped to
                // each placed token's own floor.
                let mut visible_cache: std::collections::HashMap<
                    (uuid::Uuid, bool, String),
                    std::collections::BTreeSet<(i32, i32)>,
                > = std::collections::HashMap::new();
                for op in &ops {
                    if let Operation::Update { doc_id, changes } = op {
                        if let Some((_, a0, a1)) = scene.token_move(*doc_id, changes) {
                            if a0 != a1 {
                                return Err(DataError::Forbidden);
                            }
                        }
                        // A non-GM rotation write is refused by the one rotation predicate
                        // (`rotation_refusal`): the token's `lock_rotation`, then the scene's
                        // turn rule. The auto-rotate `FieldChange` of `execute_move` never
                        // passes through here (it commits through `commit_ops_locked`).
                        if let Some(w) = scene.token_rotation_write(*doc_id, changes) {
                            if w.rotation_changed {
                                let rule = scene.resolve_scene(w.scene).facing_turn_rule;
                                let standing =
                                    if rule == crate::data::engine::FacingTurnRule::OwnTurn {
                                        // Fail closed: the defaults were fetched for every batch
                                        // holding an Update, so `None` cannot occur.
                                        let Some(wd) = world_defaults.as_ref() else {
                                            return Err(DataError::Forbidden);
                                        };
                                        scene.token_turn_standing(w.scene, *doc_id, ctx, wd)
                                    } else {
                                        crate::scene::rotation_gate::TurnStanding::Unconstrained
                                    };
                                if crate::scene::rotation_gate::rotation_refusal(
                                    w.locked, rule, standing,
                                )
                                .is_some()
                                {
                                    return Err(DataError::Forbidden);
                                }
                            }
                        }
                    }
                    if let Operation::Create { doc } = op {
                        // A created token's position is authorized against the SAME mask
                        // accessor the movement gate used. Placement was ungated on the
                        // reasoning that `core:create` is privileged, but a world can grant
                        // it to Player via `WorldCapDefaults::role_has`, and placing a token in an
                        // unseen cell reveals that area through the new token's own vision —
                        // a strictly larger capability than the movement refused above.
                        // Center-cell only: a placement is a point, not a traversal.
                        if doc.doc_type != "token" {
                            continue;
                        }
                        let Some(scene_id) = doc.parent_id else {
                            continue;
                        };
                        let Some(eng) = doc.engine.as_ref().and_then(|v| {
                            serde_json::from_value::<crate::data::engine::TokenEngine>(v.clone())
                                .ok()
                        }) else {
                            return Err(DataError::Forbidden); // unparseable engine ⇒ fail closed
                        };
                        if !eng.x.is_finite() || !eng.y.is_finite() {
                            return Err(DataError::Forbidden);
                        }
                        // Scene-existence refusal (parity axis 6): an absent entry means no
                        // scene document, so no authored cell size exists to index the mask
                        // against.
                        let Some(cell) = scene.scene_grid_sizes().get(&scene_id).copied() else {
                            return Err(DataError::Forbidden);
                        };
                        let settings = scene.resolve_scene(scene_id);
                        let lenient = settings.partial_cell_leniency;
                        let target = scene
                            .resolve_grid_shape(scene_id, cell)
                            .cell_of((eng.x, eng.y));
                        // The placed token's OWN elevation decides which level's mask/explored
                        // memory the placement gate consults — a mask/explored source on another
                        // floor of the same scene must not admit or reveal a cell on this one.
                        let placed_elevation =
                            crate::scene::elevation::elevation_or_ground(eng.elevation);
                        let level = crate::scene::elevation::level_of(
                            &scene.scene_levels(scene_id),
                            placed_elevation,
                        )
                        .map(|l| l.id.clone())
                        .unwrap_or_default();
                        // Guaranteed `Some`: reaching this point means this op is a token Create
                        // with a parent, which is exactly what `needs_world_defaults` scanned for.
                        // Fail closed rather than defaulting an authority input.
                        let Some(wd) = world_defaults.as_ref() else {
                            return Err(DataError::Forbidden);
                        };
                        match settings.movement_restriction {
                            crate::scene::MovementRestriction::Unrestricted => {}
                            crate::scene::MovementRestriction::Visible => {
                                let mask = visible_cache
                                    .entry((scene_id, lenient, level.clone()))
                                    .or_insert_with(|| {
                                        scene.visible_cells_cached(
                                            ctx.user_id,
                                            ctx.world_role,
                                            wd,
                                            scene_id,
                                            lenient,
                                            placed_elevation,
                                        )
                                    });
                                if !mask.contains(&target) {
                                    return Err(DataError::Forbidden);
                                }
                            }
                            crate::scene::MovementRestriction::Revealed => {
                                let mask = visible_cache
                                    .entry((scene_id, lenient, level.clone()))
                                    .or_insert_with(|| {
                                        scene.visible_cells_cached(
                                            ctx.user_id,
                                            ctx.world_role,
                                            wd,
                                            scene_id,
                                            lenient,
                                            placed_elevation,
                                        )
                                    })
                                    .clone();
                                // Explored needs an async fetch, which must not run under the
                                // scene read guard — defer exactly as the movement gate did. The
                                // grid kind is captured here, under the same guard `settings` was
                                // resolved in, since decoding runs after the guard is dropped.
                                // The placed token's OWN level decides which level's explored
                                // memory unions in: explored is keyed per level, and the cell
                                // being admitted sits on the new token's floor.
                                revealed_pending.push((
                                    scene_id,
                                    level,
                                    [target].into_iter().collect(),
                                    mask,
                                    settings.grid_kind,
                                    crate::scene::explored::explored_fog_key(
                                        settings.fog_pooling,
                                        ctx.user_id,
                                    ),
                                ));
                            }
                        }
                    }
                }
            } // scene read guard dropped here — safe to await

            // Memoize the explored blob per (scene, level): a batch of Revealed moves in the same
            // scene (e.g. multi-waypoint) must not issue N DB round-trips. Pattern mirrors
            // visible_cache above. Fail closed: error or missing blob → empty set (visible-only).
            let mut explored_cache: std::collections::HashMap<
                (uuid::Uuid, String),
                crate::scene::explored::ExploredSet,
            > = std::collections::HashMap::new();
            for (scene_id, level, move_cells, visible, grid_kind, explored_key) in revealed_pending
            {
                let explored = match explored_cache.entry((scene_id, level.clone())) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        let set = match repo.get_explored(scene_id, &level, explored_key).await {
                            Ok(Some(blob)) => {
                                crate::scene::explored::ExploredSet::from_bytes(&blob, grid_kind)
                            }
                            _ => crate::scene::explored::ExploredSet::new(),
                        };
                        e.insert(set)
                    }
                };
                // Invariant: `visible` may be corner-sampled (lenient) while `explored` is
                // center-sampled by construction (`ExploredSet::mark_cells` holds the lit mask's own cells). The asymmetry only ever ENLARGES
                // `visible ∪ explored`, so it is fail-safe — it never over-permits beyond cells
                // the player currently sees or has genuinely explored.
                if !move_cells
                    .iter()
                    .all(|c| visible.contains(c) || explored.contains(*c))
                {
                    return Err(DataError::Forbidden);
                }
            }
        }
        // Placement/teleport trigger candidates: a token Create always candidates; a token
        // Update only when its applied post-image position differs from the committed one —
        // the SAME comparison the move gate above makes, evaluated through `token_move` so a
        // wholesale `/engine` write is seen on its post-image. Non-GM position writes never
        // reach here (refused above); GM teleports do. Effects fire AFTER the commit, as
        // their own server-authored commit (`fire_placement_triggers`).
        let mut placement_tokens: Vec<Uuid> = Vec::new();
        {
            let scene = self.scene.read().await;
            for op in &ops {
                match op {
                    Operation::Create { doc }
                        if doc.doc_type == "token" && doc.parent_id.is_some() =>
                    {
                        placement_tokens.push(doc.id);
                    }
                    Operation::Update { doc_id, changes } => {
                        if let Some((_, a0, a1)) = scene.token_move(*doc_id, changes) {
                            if a0 != a1 {
                                placement_tokens.push(*doc_id);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // Active-scene change detector: a `world-settings` Update touching
        // `/engine/activeScene` swaps the world's ambience playlist server-side
        // (`audio::transport::on_active_scene`). Same shape as `placement_tokens`: detect
        // BEFORE commit from the ops/pre-image, act AFTER commit from the now-updated ECS.
        let active_scene_write: Option<uuid::Uuid> = {
            let scene = self.scene.read().await;
            let ws_doc_id = scene.world_settings_doc().map(|d| d.id);
            ops.iter().find_map(|op| match op {
                Operation::Update { doc_id, changes }
                    if Some(*doc_id) == ws_doc_id
                        && changes.iter().any(|c| c.path == "/engine/activeScene") =>
                {
                    Some(*doc_id)
                }
                _ => None,
            })
        };
        let old_active_scene = if active_scene_write.is_some() {
            self.scene
                .read()
                .await
                .world_settings_doc()
                .map(|d| {
                    crate::data::engine::engine_of::<crate::data::engine::WorldSettingsEngine>(d)
                        .active_scene
                })
                .unwrap_or(None)
        } else {
            None
        };
        let session_before = self.session_before_write(&ops).await;
        let command = self.commit_ops_locked(repo, ctx, ops, ts, origin).await?;
        if let Some(before) = session_before {
            let after = self.session_state().await;
            crate::audio::transport::on_session_pause(
                repo,
                ctx,
                self,
                self.world_id,
                &before,
                &after,
                ts,
            )
            .await;
        }
        if let Some(ws_doc_id) = active_scene_write {
            let new_active_scene = self
                .scene
                .read()
                .await
                .world_settings_doc()
                .filter(|d| d.id == ws_doc_id)
                .map(|d| {
                    crate::data::engine::engine_of::<crate::data::engine::WorldSettingsEngine>(d)
                        .active_scene
                })
                .unwrap_or(None);
            if new_active_scene != old_active_scene {
                crate::audio::transport::on_active_scene(
                    repo,
                    ctx,
                    self,
                    self.world_id,
                    old_active_scene,
                    new_active_scene,
                    ts,
                )
                .await;
            }
        }
        if !placement_tokens.is_empty() {
            self.fire_placement_triggers(repo, ctx, placement_tokens, ts)
                .await;
        }
        Ok(command)
    }

    /// Gate-free authoritative write tail: apply_intent → ECS-hydrate → ring/seq →
    /// broadcast Event → stats. No move gate runs here; `publish` runs the gate and
    /// delegates here; the server-authoritative move executor calls here directly.
    ///
    /// PRECONDITION (load-bearing): caller MUST hold `self.publish_guard` for the full
    /// duration of this call. tokio Mutex is not reentrant — re-acquiring inside would
    /// deadlock. Single-acquisition per logical write ensures broadcast order equals seq order.
    ///
    /// Implicit coupling: every caller acquires `publish_guard` once, optionally runs a gate,
    /// then calls this method — no callers may skip the guard or hold it across unrelated awaits.
    pub(crate) async fn commit_ops_locked(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        ops: Vec<Operation>,
        ts: i64,
        origin: WriteOrigin,
    ) -> Result<Command, DataError> {
        let stored = match repo.apply_intent(ctx, self.world_id, ops, ts, origin).await {
            Ok(stored) => stored,
            Err(e) => {
                // The ONE auto-disable funnel: every guarded write path reaches
                // this error arm, so the `VALIDATOR_FAULT_LIMIT` check exists
                // here and nowhere else (never beside a single ingress call
                // site). The caller's `publish_guard` is held, so the disable
                // runs through the `_locked` form inline — no re-acquisition,
                // no deadlock. Room-less paths (`import_world`, `create_world`)
                // never reach here: they record fault streaks only and never
                // auto-disable, a deliberate policy — a bulk import must not
                // flip a world's settings as a side effect of being read in.
                if let DataError::Validator(fault) = &e {
                    if fault.consecutive >= crate::sandbox::VALIDATOR_FAULT_LIMIT {
                        self.disable_faulting_validator_locked(repo, &fault.module)
                            .await;
                    }
                }
                return Err(e);
            }
        };
        // Hydrate the derived ECS from the committed command while still holding
        // publish_guard (enforced by the caller), so the ECS is consistent with the seq
        // before the Event (and any derived recompute keyed to that seq) is observable.
        {
            let mut scene = self.scene.write().await;
            for op in &stored.command.ops {
                scene.apply_op(op);
            }
            // Stamp the seq the ECS now reflects under the same lock, so a
            // derived reader sees a consistent (entities, seq) pair.
            scene.set_committed_seq(stored.command.seq);
        }
        let stored = Arc::new(stored);
        let ev = RoomEvent::Event(stored.clone());
        self.ring.lock().await.push(ev.clone());
        self.current_seq
            .store(stored.command.seq, Ordering::Release);
        let _ = self.tx.send(ev); // Err only when there are no receivers
        self.stats.events_published.fetch_add(1, Ordering::Relaxed);
        Ok(stored.command.clone())
    }

    /// Commits a server-authored combat-clock command: acquires `publish_guard` FRESH here (the
    /// `CombatSnapshot` backing `ops` was loaded by the caller OUTSIDE any guard), then delegates
    /// to `commit_ops_locked` under `WriteOrigin::CombatTransition`. Because every op the pure
    /// `combat::transition` functions
    /// produce carries an OCC pre-image (`FieldChange.old`), a write racing a concurrent change to
    /// the same document surfaces as a clean `DataError::Conflict` here rather than a lost or
    /// corrupted update — the snapshot-outside/guard-only-at-commit split is what makes that hold.
    ///
    /// The combat-INTENT pipeline (`combat::run_intent`) does NOT use this — it commits through
    /// `commit_combat_and_fire`, whose single guard acquisition spans the transition commit AND
    /// the turn/round region-trigger fire, so no queued write can interleave between them. This
    /// self-contained acquire-commit-release form remains as the TEST SEAM for direct callers
    /// that carry no boundary record to fire (the `sight_walls`/`light_walls` test-seam
    /// precedent), hence `#[cfg(test)]`.
    #[cfg(test)]
    pub(crate) async fn commit_combat(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        ops: Vec<Operation>,
        ts: i64,
    ) -> Result<Command, DataError> {
        let _guard = self.publish_guard.lock().await;
        self.commit_ops_locked(repo, ctx, ops, ts, WriteOrigin::CombatTransition)
            .await
    }

    /// THE one commit+fire tail for a combat intent (`combat::run_intent`'s
    /// only commit path): commits the transition's ops under
    /// `WriteOrigin::CombatTransition`, then fires its turn/round
    /// region-trigger plan — under ONE `publish_guard` acquisition spanning
    /// both, so a queued write from another connection can never interleave
    /// between the clock commit and the fire (mutating a boundary token's
    /// region membership, shifting a region's authored trigger list, or
    /// deleting the token mid-pass). The split COMMIT (`Trigger`-origin
    /// effect batch after the `CombatTransition`-origin clock batch) is
    /// unchanged — the guard, not the batch, is what spans.
    ///
    /// `tail.rewind` clears the combat's `turn_round_dedup` entry: a
    /// committed `CombatRewind` re-runs recovery when the clock re-crosses
    /// those boundaries, so the region triggers riding them must re-arm too
    /// (absence is the seen-set's ground state; the parked-turn suppression
    /// it serves rebuilds from the next advance's own inserts).
    pub(crate) async fn commit_combat_and_fire(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        ops: Vec<Operation>,
        ts: i64,
        tail: CombatTriggerTail<'_>,
    ) -> Result<Command, DataError> {
        let _guard = self.publish_guard.lock().await;
        let cmd = self
            .commit_ops_locked(repo, ctx, ops, ts, WriteOrigin::CombatTransition)
            .await?;
        if tail.rewind {
            self.turn_round_dedup
                .lock()
                .await
                .remove(&tail.snap.combat.id);
        }
        self.fire_turn_round_triggers(repo, ctx, tail.snap, tail.plan, ts)
            .await;
        Ok(cmd)
    }

    /// The audio-transport counterpart of `commit_combat`'s guard discipline: the
    /// read→apply→commit section (`audio::transport::handle_transport_locked`) runs INSIDE a
    /// freshly-acquired `publish_guard`, so two transports can never interleave a stale
    /// `audio-state` read with the other's commit. `on_active_scene` must NOT route through
    /// here: `publish` already holds the guard and a tokio Mutex is non-reentrant.
    pub(crate) async fn commit_audio_transport(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        op: crate::ws::protocol::AudioOp,
        ts: i64,
    ) -> Result<(), crate::audio::transport::TransportError> {
        let _guard = self.publish_guard.lock().await;
        crate::audio::transport::handle_transport_locked(repo, ctx, self, self.world_id, op, ts)
            .await
    }

    /// `Box::pin` wrapper around `commit_ops_locked`, existing solely so
    /// `disable_faulting_validator_locked`'s notice commit can call back into it
    /// without tripping the async-recursion check (see that call site's comment).
    fn commit_ops_locked_boxed<'a>(
        &'a self,
        repo: &'a dyn Repository,
        ctx: &'a PermissionContext,
        ops: Vec<Operation>,
        ts: i64,
        origin: WriteOrigin,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Command, DataError>> + Send + 'a>>
    {
        Box::pin(self.commit_ops_locked(repo, ctx, ops, ts, origin))
    }

    /// The guard-held form of the validator auto-disable, called by
    /// `commit_ops_locked`'s error arm — the ONE funnel every guarded write path
    /// (intent ingress, HTTP writes, chat sends, merge intents, combat transitions,
    /// config reseeds) shares, so the auto-disable check exists at exactly one site.
    /// PRECONDITION (load-bearing): the caller MUST hold `publish_guard` (the notice
    /// commit goes through `commit_ops_locked`, whose own precondition that is).
    /// Idempotent: a module whose flag is already `false` (or no longer enabled at
    /// all) is a no-op beyond the streak reset — a streak of 6, 7, ... must not
    /// duplicate the GM notice the 5th fault already posted. A world with no GM
    /// member gets no notice (`seed_author`'s own rule), but is still disabled and
    /// reset.
    async fn disable_faulting_validator_locked(&self, repo: &dyn Repository, module: &str) {
        match repo.world_enabled_modules(self.world_id).await {
            Ok(mut entries) => {
                let Some(entry) = entries.iter_mut().find(|e| e.id == module) else {
                    // No longer enabled at all — nothing to disable, nothing to notice.
                    repo.reset_validator_fault_streak(self.world_id, module)
                        .await;
                    return;
                };
                if !entry.validators_enabled {
                    repo.reset_validator_fault_streak(self.world_id, module)
                        .await;
                    return;
                }
                entry.validators_enabled = false;
                if let Err(e) = repo
                    .set_world_enabled_modules(self.world_id, &entries)
                    .await
                {
                    tracing::warn!(world = %self.world_id, module, error = %e, "validator auto-disable write failed");
                }
            }
            Err(e) => {
                tracing::warn!(world = %self.world_id, module, error = %e, "validator auto-disable could not read the enabled set");
            }
        }
        match crate::data::world_seed::seed_author(repo, self.world_id).await {
            Some(seed_ctx) => {
                let doc = crate::chat::build_message_doc(
                    self.world_id,
                    seed_ctx.user_id,
                    crate::chat::MessageDraft {
                        channel: "sandbox".to_string(),
                        actor_owner: None,
                        audience: crate::chat::Audience::GmOnly,
                        kind: crate::chat::MessageKind::System,
                        content: vec![crate::chat::Segment::Text {
                            text: format!(
                                "Sandboxed validator '{module}' faulted {} times in a row and has been disabled for this world.",
                                crate::sandbox::VALIDATOR_FAULT_LIMIT
                            ),
                        }],
                        source: None,
                        system: None,
                    },
                    crate::ws::time::now_millis(),
                );
                if let Err(e) = self
                    // Boxed: `commit_ops_locked`'s error arm can reach this notice
                    // commit, which is itself a `commit_ops_locked` call — async
                    // recursion requires `Box::pin` (bounded in practice: each
                    // level disables a distinct module).
                    .commit_ops_locked_boxed(
                        repo,
                        &seed_ctx,
                        vec![Operation::Create { doc }],
                        crate::ws::time::now_millis(),
                        WriteOrigin::ConfigSeed,
                    )
                    .await
                {
                    tracing::warn!(world = %self.world_id, module, error = %e, "validator auto-disable notice failed");
                }
            }
            None => {
                tracing::warn!(world = %self.world_id, module, "validator auto-disable: no GM member to attribute the notice to; notice skipped");
            }
        }
        repo.reset_validator_fault_streak(self.world_id, module)
            .await;
    }

    /// Server-authoritative token move: resolves gate inputs off the ECS read lock, calls the
    /// pure path executor, atomically commits the token to its stop location, and enforces a
    /// per-token `moving` lock so a client cannot re-dispatch while the animation is in flight.
    ///
    /// # Scene derivation (load-bearing)
    ///
    /// The scene every gate input is keyed on — restriction, cell size, visibility mask,
    /// explored blob, walls, and regions — is DERIVED from the token's own `parent_id`
    /// (`SceneEcs::token_move`), never taken from the caller's `scene_id`. Gating a token that
    /// lives in scene A against scene B's walls, mask, and regions is a full bypass of the
    /// movement gate for a token the requester legitimately owns. `Room::publish` takes only
    /// the start and end positions from `SceneEcs::token_move` and refuses a non-GM `Update`
    /// when they differ. `scene_id` is additionally required to agree with the derived scene
    /// (defense in depth — it selects nothing).
    ///
    /// # Critical-section invariant (load-bearing)
    ///
    /// `publish_guard` is held across the ENTIRE validate→commit body: gate-input resolution,
    /// `get_explored` await, the pure executor call, the moving-lock check/set, and
    /// `commit_ops_locked`. This makes the gate decision, the moving-lock check/set, and the
    /// position write one atomic critical section serialized with `publish` — mirrors `publish`'s
    /// discipline exactly. Scene read locks remain scoped and are never held across the
    /// `get_explored` await (no lock across await — the `publish_guard` Mutex is safe to hold
    /// across awaits; the scene RwLock is not).
    ///
    /// # Lock ordering (load-bearing — do NOT reorder)
    ///
    /// 1. Acquire `self.publish_guard` (held for the full body below).
    /// 2. Await `repo.world_cap_defaults(...)` — an input to the budget gate's `cap::READ`
    ///    resolution, which runs under the read guard taken next and so cannot await for it.
    /// 3. Take `self.scene.read()` inside the guard to resolve restriction/cell/visible_cells/
    ///    start AND the combat movement-budget gate (`SceneEcs::active_combat_for_scene` +
    ///    `SceneEcs::combatant_for_token`), all under the same read.
    /// 4. DROP the read guard before any await (no lock across await — mirrors `publish`).
    /// 5. Await `repo.get_explored(...)` for Revealed union (only after the read guard is dropped).
    /// 6. Call the pure `move_exec::execute_move` (lock-free).
    /// 7. Call `commit_ops_locked` — non-reentrant Mutex, guard already held, MUST NOT re-acquire.
    ///    Single acquisition per logical write ensures broadcast order equals seq order.
    ///
    /// # Revealed-union contract
    ///
    /// For `MovementRestriction::Revealed` the `visible` set passed to the executor MUST be
    /// `visible_cells(user, scene, lenient) ∪ explored` — the same union `publish` tests with
    /// `visible.contains(c) || explored.contains(c)`. Passing `visible_cells` alone would over-
    /// restrict, disagreeing with the `publish` gate and breaking Revealed-mode movement.
    ///
    /// # Moving lock
    ///
    /// `moving` maps token → `ActiveStream` (mover, scene, move-end epoch-ms, and the full
    /// in-flight `MoveStream` frame). An absent or expired entry (now >= end) allows the move.
    /// After a successful commit the entry is updated. Lazy expiry — no cleanup timer; a fresh
    /// server reload has no in-memory lock, consistent with the atomic-state invariant (the lock
    /// is a liveness hint, not durable state).
    pub(crate) async fn execute_move(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        req: MoveRequestInputs,
    ) -> Result<MoveExecution, DataError> {
        use crate::scene::{move_exec, MovementRestriction};
        let MoveRequestInputs {
            scene_id,
            token,
            path,
            ts,
            request_id,
        } = req;

        // Trusted server clock captured before the guard so the moving-lock end epoch is
        // consistent for both the check and the post-commit insert.
        let now = crate::ws::time::now_millis();

        // --- Acquire publish_guard at the top — held for the full validate→commit body ---
        // Mirrors `publish`: the guard serializes all gate decisions, the moving-lock
        // check/set, and the commit against concurrent publishes and execute_move calls.
        // Safe to hold across awaits (tokio Mutex); scene read locks remain scoped below.
        let _guard = self.publish_guard.lock().await;

        // --- Moving-lock check (lazy expiry: absent or expired entries are allowed) ---
        // Serialized by publish_guard: no concurrent execute_move for this room can race
        // the check-and-set. Coupling: this lock is intentionally in-memory only. A server
        // restart clears it, consistent with the fact that move state is derived (not durable).
        // The lock prevents a client from queuing multiple moves before the first animation completes.
        {
            let moving = self.moving.lock().await;
            if let Some(st) = moving.get(&token) {
                if now < st.end_ms {
                    return Err(DataError::Forbidden);
                }
            }
        }

        // --- World capability defaults, resolved before the scene read guard ---
        // An input to the combat gate's whole-document `cap::READ` resolution
        // (`SceneEcs::combatant_for_token` → `ctx_access` → `resolve_access_world`) and to the
        // visibility mask's observer-vision source admission (`visible_cells_cached` →
        // `gather_vision_sources_in_scene` → `user_access`), both of which run
        // UNDER that guard — and a scene read guard is never held across an await, so the
        // settings read has to happen ahead of it. Fetched unconditionally rather than only when
        // a combat turns out to be running: whether one is running is itself only knowable under
        // the guard. Propagates its error rather than defaulting: an unresolvable authority input
        // fails closed (the move is refused) instead of being guessed at.
        let world_defaults = repo.world_cap_defaults(self.world_id).await?;

        // --- Resolve gate inputs under the ECS read lock ---
        // restriction, cell, visible, and the derived footprint are all resolved while holding
        // the read lock and DROPPED before any await (no lock-across-await; mirrors `publish`).
        let restriction;
        let cell;
        let start;
        let token_scene;
        let visible_cells;
        let is_revealed;
        let is_gm;
        let footprint;
        let grid_kind;
        // The `explored_fog` row the Revealed union reads: the mover's own row under
        // `FogPooling::Individual`, the shared sentinel row under `Shared` — the same key
        // `ws::conn::enrich_vision_explored` accumulates into.
        let explored_key: Uuid;
        // The mover's resolved level id (`""` = ground/a level-less scene), captured under the
        // first guard: explored memory is keyed per level, so the Revealed union reads the
        // mover's OWN floor's memory — a floor the token is not on grants nothing.
        let mover_level: String;
        // The mover's resolved locomotion traits (terrain exemption), resolved in the SAME
        // first guard block as `footprint` and threaded into `MoveGateInputs` — the executor
        // never re-derives them (that struct's caller-resolves invariant). The tags belong to
        // the token/actor, so a GM moving a flying token is priced as flying too: the GM
        // exemption covers the gameplay GATES, never the cost accounting.
        let move_traits: crate::scene::pathfinding::MoveTraits;
        // The per-turn movement-budget gate, resolved off this same read guard (the combat lookup
        // below runs under the same lock as restriction/cell/visible_cells/start). `None` means
        // no active combat on the token's scene, or the token names no combatant in it — either
        // way the caller treats that as "moves freely", never a refusal.
        let budget_gate: Option<BudgetGate>;
        // The stored rotation this move would replace, or `None` when the move must not write
        // one: the resolved scene has `auto_rotate_tokens` off, or the token has
        // `lock_rotation` set. Captured under the first guard so the rotation `FieldChange`'s
        // OCC pre-image is as fresh as the position pre-image.
        let auto_rotate_from: Option<f64>;
        {
            let scene = self.scene.read().await;

            // Resolve the token's OWN scene and its committed position (the `/engine/x,y` band —
            // the sole position source; `/system` never carries position) from one authoritative
            // ECS read. `token_move(token, &[])` is the committed pre-image with no changes
            // applied, so the scene comes from the committed document and never from the client.
            // Fail-closed `None`: not a token entity, no `parent_id` (a parentless doc is never
            // hydrated as a scene entity at all), or no `/engine/x,y`.
            let (owner_scene, committed, _) =
                scene.token_move(token, &[]).ok_or(DataError::Forbidden)?;
            token_scene = owner_scene;
            start = committed;
            // Every gate input below is keyed on `token_scene`, so the request's own `scene_id`
            // selects nothing. Defense in depth: a request that disagrees is refused rather than
            // silently executed against a scene the client did not name.
            if scene_id != token_scene {
                return Err(DataError::Forbidden);
            }

            // An out-of-range footprint refuses the move outright, never clamps — clamping
            // would gate a wider token as a narrower disc, a geometric fail-open.
            let Some(fp) = scene.resolve_token_footprint(token, token_scene) else {
                return Err(DataError::Forbidden);
            };
            footprint = fp;
            let mover_tags = scene.token_movement_tags(token);
            move_traits = crate::scene::pathfinding::MoveTraits {
                ignore_terrain: crate::scene::movement_tags::ignores_terrain_cost(&mover_tags),
                tags: mover_tags,
            };

            let settings = scene.resolve_scene(token_scene);
            auto_rotate_from = scene
                .token_rotation_state(token)
                .filter(|&(_, locked)| {
                    settings.auto_rotate_tokens
                        && crate::scene::rotation_gate::lock_refusal(locked).is_none()
                })
                .map(|(rotation, _)| rotation);
            // Captured under this same read guard for the same reason `cell` is: the explored
            // decode below runs after the guard is dropped. Also feeds `visible_cells_cached`
            // below, so the movement gate's mask is level-scoped to this SAME mover floor.
            let mover_elevation = scene.token_mover_elevation(token);
            grid_kind = settings.grid_kind;
            explored_key =
                crate::scene::explored::explored_fog_key(settings.fog_pooling, ctx.user_id);
            mover_level = crate::scene::elevation::level_of(
                &scene.scene_levels(token_scene),
                mover_elevation,
            )
            .map(|l| l.id.clone())
            .unwrap_or_default();
            // Fail-closed on a `parent_id` with no scene document: `scene_grid_sizes` carries an
            // entry (defaulting to 100) for every live scene, so an absent entry means the scene
            // itself is gone — no authored cell size exists to index the visibility mask, the
            // region field, or the traversal walk against.
            cell = *scene
                .scene_grid_sizes()
                .get(&token_scene)
                .ok_or(DataError::Forbidden)?;

            // GMs are exempt from every gameplay gate here — walls, mask, impassable and arrest —
            // matching `publish`'s own GM "ignore walls" position write. Resource guards
            // (`gate_walk`'s coordinate/sample bounds, the scene-existence refusal) stay
            // unconditional for a GM.
            is_gm = ctx.world_role == crate::data::document::WorldRole::Gm;
            restriction = if is_gm {
                MovementRestriction::Unrestricted
            } else {
                settings.movement_restriction
            };

            // Combat lookup (under this same read guard): the scene's active combat, if
            // any, and this token's combatant in it, if any. Either miss means no gate applies —
            // there is no active combat on the token's own scene, or the token is not fighting in
            // it. `MoveGateInputs.budget` and the turn/resource checks below are resolved from
            // this once the guard is dropped.
            budget_gate = budget_gate_for_token(&scene, token_scene, token, ctx, &world_defaults);

            let lenient = settings.partial_cell_leniency;
            is_revealed = matches!(restriction, MovementRestriction::Revealed);

            // Pre-compute the visible set off the read lock. For Revealed, this is only the
            // `visible_cells` half; the explored half is fetched after the guard is dropped
            // (explored fetch is async — holding the read lock across it would violate the
            // no-lock-across-await rule).
            visible_cells = if matches!(restriction, MovementRestriction::Unrestricted) {
                std::collections::BTreeSet::new()
            } else {
                scene.visible_cells_cached(
                    ctx.user_id,
                    ctx.world_role,
                    &world_defaults,
                    token_scene,
                    lenient,
                    mover_elevation,
                )
            };
        } // scene read guard dropped here — safe to await (publish_guard still held)

        // --- Combat movement-budget gate: turn ownership + resource resolution ---
        // Resolved from `budget_gate` (computed above, off the scene read guard already
        // dropped). `None` at this point means no gate applies at all — no active combat on the
        // token's scene, or the token names no combatant in it — and both `move_budget_cells`
        // and `resolved_budget` stay `None`.
        let mut move_budget_cells: Option<f64> = None;
        let mut resolved_budget: Option<ResolvedBudget> = None;
        if let Some(bg) = &budget_gate {
            match resolve_budget(bg, is_gm) {
                BudgetResolution::NotYourTurn => {
                    tracing::debug!(
                        combatant = %bg.combatant_id, token = %token, user = %ctx.user_id,
                        reject = ?move_exec::MoveReject::NotYourTurn,
                        "move rejected: not the current turn owner under Hard enforcement"
                    );
                    return Err(DataError::Forbidden);
                }
                BudgetResolution::Unresolvable => {
                    tracing::debug!(
                        combatant = %bg.combatant_id, token = %token, resource = %bg.resource,
                        reject = ?move_exec::MoveReject::BudgetUnresolvable,
                        "move rejected: the movement budget cannot be resolved"
                    );
                    return Err(DataError::Forbidden);
                }
                BudgetResolution::Resolved {
                    budget_cells,
                    decrement,
                } => {
                    move_budget_cells = budget_cells;
                    resolved_budget = decrement;
                }
            }
        }

        // --- Revealed union: fetch explored AFTER dropping the scene read guard ---
        // INVARIANT: for Revealed the `visible` set passed to execute_move MUST be
        // visible_cells ∪ explored. Fail-closed: error or missing blob → empty explored set
        // (falls back to visible-only, which is stricter but safe).
        let visible = if is_revealed {
            let mut union = visible_cells;
            let explored = match repo
                .get_explored(token_scene, &mover_level, explored_key)
                .await
            {
                Ok(Some(blob)) => crate::scene::explored::ExploredSet::from_bytes(&blob, grid_kind),
                _ => crate::scene::explored::ExploredSet::new(),
            };
            // Union: insert every explored cell into the visible set.
            for (ci, cj) in explored.iter() {
                union.insert((ci, cj));
            }
            union
        } else {
            visible_cells
        };

        // --- Pure path executor, animation speed, samples, and mover vision ---
        // Re-acquire the read lock now that the explored await is complete. All synchronous
        // work — executor, animation speed, distance/duration, samples, and mover_vision
        // raycasts — runs here before dropping the lock so no lock-across-await occurs.
        // publish_guard remains held for the full body (commit_ops_locked depends on it).
        // Maps MoveReject → DataError::Forbidden (all reject reasons indicate the request
        // is invalid: unknown token, too-long path, bad start, non-adjacent step).
        let outcome;
        let stop_cell;
        let duration_ms;
        let samples;
        let mover_vision: Option<Vec<crate::scene::move_stream::VisionSamplePt>>;
        let mover_light: Option<Vec<crate::scene::move_stream::LightSamplePt>>;
        {
            let scene = self.scene.read().await;
            outcome = move_exec::execute_move(
                &scene,
                move_exec::MoveGateInputs {
                    scene: token_scene,
                    restriction,
                    visible: &visible,
                    cell,
                    // The combat's per-turn budget ceiling, or unlimited — resolved above from
                    // `budget_gate` (never `Some` for a caller the gate exempts: a GM, or one
                    // the combatant is hidden from — see `BudgetGate::enforced`).
                    budget: move_budget_cells,
                    traits: move_traits,
                    // The mover's floor, resolved off the token's stored elevation under this
                    // same read guard — never the client's claim (mirrors `footprint`/`traits`).
                    mover_elevation: scene.token_mover_elevation(token),
                },
                token,
                &path,
                is_gm,
                footprint,
            )
            .map_err(|_| DataError::Forbidden)?;
            // The move report's occupancy basis for the Exit membership diff: the stop's
            // center cell — the SAME center-cell basis the executor's region gate and the
            // report's `entered` transitions key on (`TriggerReport::occupancy`).
            stop_cell = scene
                .resolve_grid_shape(token_scene, cell)
                .cell_of(outcome.stop);
            let speed_cells_per_sec = scene.resolved_animation_speed();
            // Animation speed is authored in cells/sec, so the travelled distance converts
            // through the scene shape's per-cell world distance, not its indexing scale.
            let world_per_cell = scene
                .resolve_grid_shape(token_scene, cell)
                .world_units_per_cell();

            // Distance and duration computed here so samples and mover_vision can be built
            // under the same lock — all synchronous, no lock-across-await hazard.
            let distance: f64 = outcome
                .render_path
                .windows(2)
                .map(|w| {
                    let dx = w[1].0 - w[0].0;
                    let dy = w[1].1 - w[0].1;
                    (dx * dx + dy * dy).sqrt()
                })
                .sum();
            duration_ms = if distance < 1e-9 {
                0.0
            } else {
                (distance / world_per_cell) / speed_cells_per_sec * 1000.0
            };

            samples =
                crate::scene::move_stream::sample_path(&outcome.render_path, cell, duration_ms);

            // GM mover → None (no fog to sweep), regardless of restriction mode. Non-GM movers
            // get a per-sample vision polygon at each hypothetical position along the
            // trajectory, including in Unrestricted-mode scenes. Hoisting: `sight_sources`
            // resolves the mover's sources and their committed polygons ONCE per move (the
            // SAME sources + `source_los_poly` the committed `vision` polygons and the egress
            // clip read); each sample re-raycasts only the moving token (`SightSources::los_at`).
            mover_vision = if is_gm {
                None
            } else {
                let sight =
                    scene.sight_sources(ctx.user_id, ctx.world_role, &world_defaults, token_scene);
                Some(
                    samples
                        .iter()
                        .map(|s| crate::scene::move_stream::VisionSamplePt {
                            t_ms: s.t_ms,
                            polygons: sight
                                .los_at(&[(token, s.pos)])
                                .into_iter()
                                .map(|(_, p)| p.into_owned())
                                .collect(),
                        })
                        .collect(),
                )
            };
            // Carried light: computed only when the mover carries an enabled emission in an
            // environment-lit scene ("cost only on request") — one raycast per sample against
            // the light walls at the token's elevation, hoisted once per move. Not gated on
            // the mover's role: a GM walking a torch-bearer lights the corridor for the
            // players watching it exactly as a player's own move does.
            mover_light = scene
                .mover_light_inputs(token_scene, token, cell)
                .map(|li| {
                    samples
                        .iter()
                        .map(|s| li.sample_at(s.t_ms, s.pos))
                        .collect()
                });
        } // scene read lock dropped — commit_ops_locked awaits safely under publish_guard

        // Zero-progress move (stop == start): return immediately without writing.
        // Invariant: render_path always contains at least `start` (path.len() >= 2 was
        // validated by execute_move), so this only fires when the very first step was blocked.
        // Skipping the fire below also skips the `SceneEcs::region_membership` update —
        // the token's occupancy provably did not change, which is correct, but it means a
        // stale entry (e.g. a fresh post-restart table) is NOT healed by a zero-progress
        // move: healing defers to the first move with real progress.
        if (outcome.stop.0 - start.0).abs() < 1e-9 && (outcome.stop.1 - start.1).abs() < 1e-9 {
            let zero_samples = vec![crate::scene::move_stream::PosSamplePt {
                t_ms: 0.0,
                pos: start,
            }];
            let frame = Arc::new(wire_move_stream(
                request_id,
                token,
                ctx.user_id,
                ts,
                token_scene,
                WireMoveInputs {
                    stop: start,
                    duration_ms: 0.0,
                    samples: &zero_samples,
                    mover_vision: None,
                    mover_light: None,
                    cost: 0.0,
                    // Not hardcoded false: this branch is reached only when the very first
                    // step was blocked, so the outcome is truncated. Reading it keeps the wire
                    // signal derived from the executor rather than restated here.
                    truncated: outcome.truncated,
                },
            ));
            // NOT registered in `moving`: a zero-duration move never held the lock, and
            // there is no in-flight animation to re-clip against.
            return Ok(MoveExecution {
                #[cfg(test)]
                scene: token_scene,
                #[cfg(test)]
                stop: start,
                #[cfg(test)]
                duration_ms: 0.0,
                frame,
            });
        }

        // --- Atomic commit (publish_guard already held — single acquisition, no re-entry) ---
        // PRECONDITION: commit_ops_locked requires the caller to hold publish_guard for its
        // full duration. The guard was acquired at the top of this function and is still held
        // here — no re-acquisition needed or allowed (tokio Mutex is non-reentrant; re-acquiring
        // would deadlock). The position ops write ONLY `/engine/x,y` — position lives
        // exclusively in the engine band; `/system` is game-system data and is never touched by
        // movement. `old` is keyed on the authoritative ECS-read `start` (the committed start
        // `SceneEcs::token_move` returns, itself `/engine/x,y`) so the optimistic-concurrency
        // check in `apply_intent` passes as defense-in-depth.
        let mut position_changes = vec![
            FieldChange {
                remove: false,
                path: "/engine/x".into(),
                old: serde_json::json!(start.0),
                new: serde_json::json!(outcome.stop.0),
            },
            FieldChange {
                remove: false,
                path: "/engine/y".into(),
                old: serde_json::json!(start.1),
                new: serde_json::json!(outcome.stop.1),
            },
        ];
        // Auto-rotate: the facing rides in the SAME `Update` as the position so every viewer
        // sees both change together under one commit. Only reachable past the zero-progress
        // return above, so `start != stop` and the bearing is a real direction. A GM move
        // rotates too: the GM exemption covers gameplay gates, not a cosmetic write.
        if let Some(current_rotation) = auto_rotate_from {
            position_changes.push(FieldChange {
                remove: false,
                path: "/engine/rotation".into(),
                old: serde_json::json!(current_rotation),
                new: serde_json::json!(move_exec::bearing_degrees(
                    start,
                    outcome.stop,
                    current_rotation
                )),
            });
        }
        let ops = vec![Operation::Update {
            doc_id: token,
            changes: position_changes,
        }];

        // Position write commits ALONE under `WriteOrigin::Client`, UNCONDITIONALLY — never
        // bundled with the combat decrement below. `apply_intent`'s ownership/capability check
        // (`!origin.skips_capability_gates() && !access.has(need)`) is skipped for every
        // op in a batch whenever ANY op in it carries `CombatTransition`; bundling the two under
        // one origin let a `CombatTransition`-tagged decrement silently waive the ownership check
        // on the token-position write in the same batch, so any authenticated non-GM could move
        // any other player's token by naming their `token_id` once combat is active. Splitting
        // the commit is the fix: the position write's origin can never be anything but `Client`.
        self.commit_ops_locked(repo, ctx, ops, ts, WriteOrigin::Client)
            .await?;

        // Combat resource decrement: a SEPARATE commit against a DIFFERENT document (the
        // combatant), issued only after the position commit above has succeeded. Floored at zero
        // (`max(0.0)`); skipped entirely when the walked distance spent nothing (a zero-progress/
        // zero-cost move, or a combatant this gate never applied to). The pre-image (`old`) is
        // read from the SAME scene-read-guard snapshot the turn/resource validation used, before
        // any `.await` — the position commit touches only the token document, never the
        // combatant, so it cannot have staled this pre-image by the time this second commit
        // fires. A conflict here (e.g. a genuine concurrent write to the combatant) surfaces as
        // `DataError::Conflict` and is NOT rolled back into the already-committed position move:
        // the position write is authoritative once it lands, and failing the whole call over a
        // decrement conflict would leave the client's token stuck out of sync with what was
        // actually written.
        if let Some(rb) = &resolved_budget {
            let spent = outcome.cost * rb.cost_to_resource;
            if spent != 0.0 {
                let new_current = (rb.current - spent).max(0.0);
                let decrement_ops = vec![Operation::Update {
                    doc_id: rb.combatant_id,
                    changes: vec![FieldChange {
                        remove: false,
                        path: format!("/engine/resources/{}/current", rb.resource),
                        // `None` = the entry was absent at gate time: the OCC
                        // pre-image is Null and this write materializes it.
                        old: match rb.stored {
                            Some(s) => serde_json::json!(s),
                            None => serde_json::Value::Null,
                        },
                        new: serde_json::json!(new_current),
                    }],
                }];
                if let Err(err) = self
                    .commit_ops_locked(repo, ctx, decrement_ops, ts, WriteOrigin::CombatTransition)
                    .await
                {
                    tracing::debug!(
                        combatant = %rb.combatant_id, resource = %rb.resource, ?err,
                        "movement-budget decrement commit failed after the position move already \
                         committed; the move stands, the resource spend was not recorded"
                    );
                }
            }
        }

        // Region triggers, fired AFTER the position commit AND the combat decrement: a
        // `ResourceDelta` trigger can write the very combatant/resource the decrement just
        // wrote, so effects run last and read freshly committed state (the decrement's own
        // pre-image was captured pre-commit and must not be raced by an effect write). A
        // SEPARATE commit under `WriteOrigin::Trigger`, never batched with the
        // client-origin position write — the same split discipline the position commit above
        // states. The zero-progress early return above never reaches here, and it has no
        // entered cells to report in any case — and its skipped membership update means a
        // zero-progress move also does NOT heal a stale `SceneEcs::region_membership`
        // entry (e.g. post-restart): healing defers to the first move with real progress.
        self.fire_region_triggers(
            repo,
            ctx,
            TriggerReport {
                scene: token_scene,
                token,
                entered: outcome.entered_cells.clone(),
                arrest_stop: outcome.arrested.then_some(outcome.stop),
                occupancy: vec![stop_cell],
                enter_only: None,
            },
            ts,
            true,
        )
        .await;
        self.fire_attached_region_triggers(repo, ctx, token, ts, true)
            .await;

        // Build the wire frame before registering it.
        let frame = Arc::new(wire_move_stream(
            request_id,
            token,
            ctx.user_id,
            ts,
            token_scene,
            WireMoveInputs {
                stop: outcome.stop,
                duration_ms,
                samples: &samples,
                mover_vision,
                mover_light,
                cost: outcome.cost,
                truncated: outcome.truncated,
            },
        ));

        // --- Update the moving lock after a successful commit (still inside publish_guard) ---
        // Serialized by publish_guard: the check above and this insert form one atomic
        // check-and-set with no window for a concurrent execute_move to slip through.
        // Uses server-owned `now` (captured at entry), never the caller-supplied `ts`
        // (which is only used for the committed event timestamp and is not trusted for timing).
        // Lazy expiry: prune expired entries before inserting so the map stays bounded in
        // long sessions (tokens that moved once and never moved again do not leak permanently).
        // Sub-ms floor: a non-zero-progress move whose duration rounds to 0 ms would let the
        // next request pass immediately; ceil().max(1) guarantees end > now for any real move.
        {
            let mut moving = self.moving.lock().await;
            moving.retain(|_, st| now < st.end_ms);
            moving.insert(
                token,
                ActiveStream {
                    mover: ctx.user_id,
                    scene: token_scene,
                    end_ms: now + (duration_ms.ceil() as i64).max(1),
                    frame: frame.clone(),
                },
            );
        }

        Ok(MoveExecution {
            #[cfg(test)]
            scene: token_scene,
            #[cfg(test)]
            stop: outcome.stop,
            #[cfg(test)]
            duration_ms,
            frame,
        })
    }

    /// Flip a wall's door state (`ClientMsg::ToggleDoor`): the ONE positional
    /// authorization + commit path for a non-GM door interaction.
    ///
    /// # Authorization (load-bearing)
    ///
    /// A GM bypasses the positional clauses entirely — matching the documented GM exemption
    /// from every other gameplay gate in this subsystem (`execute_move`'s walls/mask/
    /// impassable/arrest/footprint) — and may additionally set/clear `Locked`. A non-GM is
    /// admitted only when EVERY one of these holds:
    ///
    /// 1. the requester effectively controls at least one token parented to the wall's scene
    ///    (`SceneEcs::token_scene_and_effective_owner` — the SAME effective-ownership rule
    ///    `token_emote_permitted`/`handle_pathfind`'s token gate use, never a forked check);
    /// 2. that token's position is within `DOOR_REACH_CELLS` (converted to world units via
    ///    `GridShape::world_units_per_cell`) of the door segment (`vision::point_segment_distance`,
    ///    the same primitive the footprint-clearance wall check uses);
    /// 3. the token's current visible-cell set (`SceneEcs::visible_cells_cached`, the SAME mask
    ///    the movement gate reads) intersects the door segment's own supercover cells
    ///    (`GridShape::line_traversal`, the same primitive `move_exec`'s wall-crossing test
    ///    uses — never a single-midpoint approximation that could misjudge a wall spanning a
    ///    cell boundary);
    /// 4. the requester holds engine-tier visibility on the WALL DOCUMENT ITSELF
    ///    (`engine_tier_visible(&wall_doc, Some(user))` — the same per-requester authority
    ///    `move_wall_entries`/`region_field` apply). This clause is NOT implied by the visible-cells clause:
    ///    `visible_cells_cached`'s raycast is DELIBERATELY permission-blind (a `gm_only` wall
    ///    still occludes sight for someone who may not know it exists), so a token can be
    ///    geometrically adjacent to and "in sight of" a hidden door's cells while the requester
    ///    has no right to know that door exists. Without this clause a non-GM could toggle —
    ///    and thereby discover the existence and state of — a door the GM authored as hidden.
    /// 5. the wall's CURRENT stored `door` is `Closed`/`Open` (never `Locked`, never absent)
    ///    and `to` is `Closed`/`Open` (never `Locked`) — the "never onto or off `Locked`"
    ///    constraint, enforced here on the handler's own read of the current document.
    ///
    /// `scene_id` is NOT trusted and selects nothing: the authoritative scene is DERIVED from
    /// the wall document's own `parent_id`, and a `scene_id` that disagrees is refused —
    /// `execute_move`'s derive-from-the-document invariant applied to a wall instead of a token.
    ///
    /// # Critical section
    ///
    /// `publish_guard` is held across the whole validate→commit body (mirrors `execute_move`):
    /// the state read, the gate decision, and the commit serialize against every other write,
    /// so two simultaneous toggles resolve sequentially (the second reads the first's committed
    /// state) instead of double-writing from a shared stale pre-image. The scene read guard is
    /// scoped and dropped before the commit (a read guard is never held across an await).
    ///
    /// # Commit shape
    ///
    /// One `Operation::Update` of `/engine/door` via `commit_ops_locked` under
    /// `WriteOrigin::DoorToggle` — the write reaches `apply_intent` already vetted, so
    /// `authorize_update_change` needs NO exemption of any kind (a positional authorization
    /// cannot be expressed as a `FieldChange` classifier: it sees a document id and old/new
    /// values, never where the actor stands or what they can see). The committed Event echo is
    /// the success confirmation; only a rejection produces a dedicated frame
    /// (`ServerMsg::DoorToggleError`, built by `ws::conn`'s handler wrapper).
    pub(crate) async fn toggle_door(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        req: ToggleDoorInputs,
    ) -> Result<(), DoorToggleError> {
        let ToggleDoorInputs {
            scene_id,
            wall_id,
            to,
            ts,
            ..
        } = req;
        let is_gm = ctx.world_role == crate::data::document::WorldRole::Gm;

        let _guard = self.publish_guard.lock().await;

        // The world capability defaults feed the visible-cells clause's `visible_cells_cached` (its
        // observer-vision source admission reads them under the scene read guard, and a read
        // guard is never held across an await, so the fetch happens first) — the same call
        // `execute_move` makes for its own mask. A GM skips the positional clauses and never
        // needs it. Fails closed: an unresolvable authority input refuses the toggle rather
        // than guessing at it.
        let world_defaults = if is_gm {
            None
        } else {
            Some(repo.world_cap_defaults(self.world_id).await?)
        };

        // The OCC pre-image for the committed write, captured under the same read guard as
        // the gate inputs so both read one consistent ECS snapshot.
        let old_door;
        {
            let ecs = self.scene.read().await;

            // The wall must exist as a scene-hydrated wall document; its scene is
            // DERIVED from its own `parent_id`, and the request's `scene_id` must agree.
            let wall_doc = ecs
                .scene_entity_doc(wall_id)
                .filter(|d| d.doc_type == "wall")
                .ok_or(DoorToggleError::UnknownWall)?;
            let wall_scene = wall_doc.parent_id.ok_or(DoorToggleError::UnknownWall)?;
            if wall_scene != scene_id {
                return Err(DoorToggleError::SceneMismatch);
            }
            let wall = ecs
                .engine_as_cached::<eng::WallEngine>(wall_id, &wall_doc)
                .ok_or(DoorToggleError::UnknownWall)?;

            // Fail-closed on a missing scene document (an absent `scene_grid_sizes` entry
            // means the scene is gone) — never a synthesized grid.
            let cell = *ecs
                .scene_grid_sizes()
                .get(&wall_scene)
                .ok_or(DoorToggleError::SceneMismatch)?;
            let shape = ecs.resolve_grid_shape(wall_scene, cell);

            if !is_gm {
                let user = ctx.user_id;
                // Fetched above for every non-GM, so this is always `Some` here; a miss fails
                // closed rather than guessing at the authority input.
                let Some(defaults) = world_defaults.as_ref() else {
                    return Err(DoorToggleError::Data(DataError::OpFailed(
                        "world defaults unresolved for a non-GM door toggle".into(),
                    )));
                };
                let seg_a = (wall.seg.x1, wall.seg.y1);
                let seg_b = (wall.seg.x2, wall.seg.y2);

                // The ownership and reach clauses: the requester's nearest effectively-owned token in the scene,
                // and its reach. The scan pattern is the same one `compute_audibility`'s
                // emitter loop shaped: enumerate the scene's tokens
                // (`SceneEcs::scene_token_ids`), filter to the requester's own via the shared
                // effective-ownership rule, take the nearest to the segment.
                let mut nearest: Option<(Uuid, f64)> = None;
                for token in ecs.scene_token_ids(wall_scene) {
                    if ecs.token_scene_and_effective_owner(token) != Some((wall_scene, Some(user)))
                    {
                        continue;
                    }
                    let Some(pos) = ecs.token_position(token) else {
                        continue;
                    };
                    let d = crate::scene::vision::point_segment_distance(pos, seg_a, seg_b);
                    if nearest.is_none_or(|(_, best)| d < best) {
                        nearest = Some((token, d));
                    }
                }
                let Some((mover, dist)) = nearest else {
                    return Err(DoorToggleError::NoOwnedToken);
                };
                if dist > DOOR_REACH_CELLS * shape.world_units_per_cell() {
                    return Err(DoorToggleError::OutOfReach);
                }

                // The visible-cells clause: the door segment's own supercover cells must intersect the
                // requester's visible-cell set — the SAME mask the movement gate reads,
                // level-scoped to the nearest token's floor exactly as a move of that token
                // would be.
                let lenient = ecs.resolve_scene(wall_scene).partial_cell_leniency;
                let visible = ecs.visible_cells_cached(
                    user,
                    ctx.world_role,
                    defaults,
                    wall_scene,
                    lenient,
                    ecs.token_mover_elevation(mover),
                );
                let door_in_sight = shape
                    .line_traversal(seg_a, seg_b, cell)
                    .is_some_and(|cells| cells.iter().any(|c| visible.contains(c)));
                if !door_in_sight {
                    return Err(DoorToggleError::NotVisible);
                }

                // The wall-document-visibility clause: engine-tier visibility on the wall DOCUMENT — the one clause that
                // reads the wall's own declared visibility tier rather than a geometric
                // consequence of it (see the method doc for why the visible-cells clause cannot imply it).
                if !crate::scene::engine_tier_visible(&wall_doc, Some(user)) {
                    return Err(DoorToggleError::WallNotVisible);
                }

                // The state-machine clause: flip `Closed`↔`Open` only, never onto or off `Locked`, and a
                // doorless wall has nothing to flip.
                let flippable = matches!(
                    wall.door,
                    Some(eng::DoorState::Closed) | Some(eng::DoorState::Open)
                );
                let admitted_target = matches!(to, eng::DoorState::Closed | eng::DoorState::Open);
                if !flippable || !admitted_target {
                    return Err(DoorToggleError::Locked);
                }
            }

            old_door = wall_doc
                .engine
                .as_ref()
                .and_then(|e| e.get("door").cloned())
                .unwrap_or(serde_json::Value::Null);
        } // scene read guard dropped — commit_ops_locked takes the write guard itself

        let new_door = serde_json::to_value(to).map_err(DataError::from)?;
        self.commit_ops_locked(
            repo,
            ctx,
            vec![Operation::Update {
                doc_id: wall_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/engine/door".into(),
                    old: old_door,
                    new: new_door,
                }],
            }],
            ts,
            WriteOrigin::DoorToggle,
        )
        .await?;
        Ok(())
    }

    /// Fire the region triggers a token report sets off: every enabled trigger-bearing region
    /// covering an `entered` cell fires its `Enter` effects, and when `arrest_stop` is `Some`
    /// (the executor reported a region-arrest), regions covering the arrest cell fire their
    /// `Arrest` effects. `Exit` effects fire from the membership diff: the regions
    /// intersecting the report's `occupancy` basis (its own Enter basis — a move's stop
    /// cell, a placement's footprint) replace `SceneEcs::region_membership`'s stored set
    /// UNCONDITIONALLY, and any region the stored set loses fires its `Exit` triggers —
    /// including, via `SceneEcs::trigger_region_row`'s unfiltered by-id rows, a region the
    /// report's scene/elevation table no longer selects (a cross-scene portal or an
    /// elevation-changing teleport). This is the membership-diff's ONLY writer, so the
    /// stored set can never skew from the firing decision. Effects themselves apply through
    /// `apply_region_trigger_effects`, THE one application path every fire site funnels
    /// into — `execute_move` (entered-cell sequence from `MoveOutcome`),
    /// `fire_placement_triggers` (footprint cells of the new position), and
    /// `fire_turn_round_triggers` (the combat boundary plan) — so the sites
    /// cannot drift on what an effect means.
    ///
    /// Effects are server-authored and commit as ONE batch via `commit_ops_locked` under
    /// `WriteOrigin::Trigger` — never batched with the client-origin write that
    /// triggered them (the split discipline `execute_move`'s position commit states). A commit
    /// failure is logged and swallowed rather than propagated: the triggering write already
    /// stands, and failing the caller over a lost effect would desync it from what was
    /// actually written (the movement decrement's own precedent).
    ///
    /// PRECONDITION: caller holds `publish_guard` (`commit_ops_locked` requires it).
    ///
    /// # Effect semantics
    ///
    /// - `ConditionAdd`/`ConditionRemove`: applied to the token's actor host — the embedded
    ///   actor copy first, else the linked actor (`combat::eval::formula_host`'s
    ///   precedence) — folded onto the host's current `conditions` array and written as ONE
    ///   OCC `Operation::Update` (`combat::ops::set_engine` supplies the pre-image).
    ///   Idempotent: adding a present condition or removing an absent one is a no-op.
    /// - `ResourceDelta`: combat-scoped. The token's combatant in the scene's ACTIVE combat
    ///   is resolved (`token_id` match first, else the resolved actor —
    ///   `SceneEcs::combatant_for_token`'s precedence), each `amount` is evaluated via
    ///   `combat::eval::eval_formula` against the combatant's formula host, and amounts
    ///   naming the same resource are summed into ONE `ResourceOp::Delta` per key (net-delta
    ///   per report, clamped once by `combat::transition::resource`). No active combat, no
    ///   combatant, a `Mirror` binding, an unknown key, or an amount that fails to evaluate
    ///   is a no-op surfaced in ONE deduplicated GM-only chat notice
    ///   (`combat::transition::eval_notice`'s shape).
    /// - `ChatNotice`: posted as a `MessageKind::System` message authored by the mover. The
    ///   authored audience is FORCED to `GmOnly` when the region's `/engine` band is not
    ///   visible to every world member — a wider notice would name or imply a region some
    ///   recipients cannot see. `Owner` (effective owner + every GM) has no
    ///   `chat::Audience` shape of its own, so it builds the `GmOnly` permission shape and
    ///   grants the token's effective owner a read on top.
    /// - `Teleport`: the token moves to the target — same-scene (`target.scene: None`) as a
    ///   server-authored `Operation::Update` of `/engine/x`,`/engine/y` (+ `/engine/elevation`
    ///   when the target names one); cross-scene as the server-authored `Operation::Move`
    ///   reparenting PLUS the position Update, in the SAME committed batch (one seq). At most
    ///   ONE teleport applies per token per fire (a region's own trigger list could carry
    ///   several; only the first wins). The target scene's existence is checked at FIRE time,
    ///   never at ingress (`validate_engine_tree` is pure): a missing target scene is a GM-only
    ///   notice, no move. After a teleport the destination cells fire `Enter` effects EXCEPT
    ///   another `Teleport` (one hop per move; a chained portal is refused with a GM-only
    ///   notice) — enforced by the recursive re-fire passing `allow_teleport: false`. A token
    ///   teleported OFF its combat's scene keeps its combatant record and turn but moves
    ///   unbudgeted there (a portal is a legitimate escape), made visible by a GM-only notice
    ///   when an active combat runs on the source scene.
    /// - `PortalChoice` (on `Enter`): shares the one-hop slot with `Teleport` (the first of the
    ///   two in the fired list wins). With `allow_teleport` it moves nothing: it registers a
    ///   per-token offer (`offer_portal`) and broadcasts `ServerMsg::PortalOffer` to the token's
    ///   owner and the GMs. The owner's answer (`handle_choose_portal`) runs the chosen target
    ///   back through this funnel as a synthetic `Teleport`, so it shares `apply_teleport_to`.
    ///
    /// `allow_teleport` is a FUNCTION parameter rather than a `TriggerReport` field by
    /// deliberate choice: the report describes the entry EVENT (scene, token, cells, arrest),
    /// while the flag is a property of the firing PASS (the one-hop anti-loop), so it travels
    /// beside `ts`, not inside the event description.
    pub(crate) async fn fire_region_triggers(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        report: TriggerReport,
        ts: i64,
        allow_teleport: bool,
    ) {
        let TriggerReport {
            scene,
            token,
            entered,
            arrest_stop,
            occupancy,
            enter_only,
        } = report;
        let entered: &[(i32, i32)] = &entered;
        use crate::scene::regions;

        // The entering token's document, loaded AFTER the triggering write committed, so the
        // actor join and every condition pre-image read committed state.
        let token_doc = match repo.get_document(token).await {
            Ok(Some(doc)) => doc,
            Ok(None) => {
                tracing::debug!(%token, "region triggers: the entering token is gone; no effects fire");
                return;
            }
            Err(err) => {
                tracing::debug!(%token, ?err, "region triggers: the entering token could not be loaded; no effects fire");
                return;
            }
        };

        // The token's actor join, resolved once: `TokenEngine.actor_id` else the embedded
        // copy's id (`SceneEcs::combatant_for_token`'s rule). The CONDITIONS host uses the
        // `formula_host` precedence instead (embedded copy first, else the linked actor) —
        // the two answers differ only for a token carrying both, and each consumer's own
        // precedent is kept.
        let token_eng: Option<eng::TokenEngine> = token_doc
            .engine
            .clone()
            .and_then(|v| serde_json::from_value(v).ok());

        // The authoritative identity table (the server springs secret regions), the arrest
        // cell, and the Exit-firing inputs — one WRITE guard
        // (the membership diff updates `SceneEcs::region_membership`), no lock across an
        // await. `trigger_regions` is filtered to the entering token's floor: a region
        // banded to another level never fires on this token (`elevation::band_contains`).
        let (regions, arrest_cell, exited, stale_rows) = {
            let mut ecs = self.scene.write().await;
            let regions = ecs
                .trigger_regions(
                    scene,
                    crate::scene::elevation::elevation_or_ground(
                        token_eng.as_ref().and_then(|t| t.elevation),
                    ),
                    Some(token),
                )
                .unwrap_or_default();
            let arrest_cell = match arrest_stop {
                Some(pos) => ecs
                    .scene_grid_sizes()
                    .get(&scene)
                    .map(|&cell| ecs.resolve_grid_shape(scene, cell).cell_of(pos)),
                None => None,
            };
            // Exit membership diff: the regions intersecting the report's OCCUPANCY basis
            // (its own Enter basis — a move's stop cell, a placement's footprint) replace
            // the stored set UNCONDITIONALLY, and whatever the stored set loses is this
            // report's exited list. Unconditional even when nothing fires: a stored entry
            // is dropped on the move out of its region whether or not the region carries
            // an `Exit` trigger, so a later edit ADDING one can never fire an exit for a
            // region the token had already left.
            let exited = {
                let now: std::collections::BTreeSet<Uuid> = regions
                    .iter()
                    .filter(|r| occupancy.iter().any(|c| r.cells.contains(c)))
                    .map(|r| r.region_id)
                    .collect();
                ecs.diff_region_membership(token, now)
            };
            // Stale rows: exited ids the scene/elevation-filtered table no longer selects
            // (a cross-scene portal or an elevation-changing teleport carried the token
            // out of a region its stored membership still names). Their rows resolve by
            // id WITHOUT the band filter (`SceneEcs::trigger_region_row`), so the exit
            // still fires; fired below with an empty `entered` so no Enter/Arrest can
            // fire from them. Partitioned from `regions` by construction, so no trigger
            // can fire twice on one report.
            let stale_rows: Vec<regions::TriggerRegion> = exited
                .iter()
                .filter(|id| !regions.iter().any(|r| r.region_id == **id))
                .filter_map(|&id| ecs.trigger_region_row(id))
                .collect();
            (regions, arrest_cell, exited, stale_rows)
        };
        let mut fired = regions::fired_triggers(&regions, entered, arrest_cell, &exited);
        if let Some(only) = &enter_only {
            // Enter/Arrest narrow to the named regions; an Exit pair (its region id is in
            // `exited`) always stays.
            fired.retain(|(id, _)| only.contains(id) || exited.contains(id));
        }
        fired.extend(regions::fired_triggers(&stale_rows, &[], None, &exited));
        // The rows a notice's secrecy forcing searches: the scene table AND the stale
        // exit rows (an Exit-fired notice rides the identical rule).
        let mut secrecy_rows = regions;
        secrecy_rows.extend(stale_rows);
        self.apply_region_trigger_effects(
            repo,
            ctx,
            token_doc,
            TriggerEffectPlan {
                fired,
                secrecy_rows: &secrecy_rows,
                combat_hint: None,
                allow_teleport,
            },
            ts,
        )
        .await;
    }

    /// The effect-application half of region-trigger firing — THE one path
    /// every fire site shares, so a move entry, a placement/teleport, and a
    /// combat turn/round boundary can never drift on what an effect MEANS:
    /// `fire_region_triggers` (movement/placement reports) and
    /// `fire_turn_round_triggers` (the combat boundary plan) both end here.
    /// The caller computes the firing set; this function never re-decides
    /// WHAT fires, only applies it: condition/resource/notice/teleport
    /// effects against `plan.fired`, then ONE batch commit via
    /// `commit_ops_locked` under `WriteOrigin::Trigger` — never batched with
    /// the write whose firing set this was (the `execute_move` split
    /// discipline). A commit failure is logged and swallowed rather than
    /// propagated: the triggering write already stands, and failing the
    /// caller over a lost effect would desync it from what was actually
    /// written (the movement decrement's own precedent).
    ///
    /// `token_doc` is the entering token's committed document (each caller
    /// loads it AFTER the triggering write committed, so the actor join and
    /// every pre-image read committed state); the token's scene is derived
    /// from its `parent_id` — the same value every caller's own derivation
    /// already produces, stated once here instead.
    ///
    /// PRECONDITION: caller holds `publish_guard` (`commit_ops_locked` requires it).
    async fn apply_region_trigger_effects(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        token_doc: Document,
        plan: TriggerEffectPlan<'_>,
        ts: i64,
    ) {
        use crate::chat::{build_message_doc, Audience, MessageDraft, MessageKind, Segment};
        use crate::combat::transition::{resource as resource_transition, ResourceOp};
        use crate::combat::CombatError;

        let TriggerEffectPlan {
            fired,
            secrecy_rows,
            combat_hint,
            allow_teleport,
        } = plan;
        if fired.is_empty() {
            return;
        }
        let token = token_doc.id;
        // The token's scene, derived from its committed document (a caller's
        // own scene derivation — `execute_move`'s `token_move`, a placement's
        // footprint — resolves to this same value post-commit).
        let scene = token_doc.parent_id;
        // The token's own engine body (position/elevation for a Teleport,
        // `actor_id` for the resource combatant join) — parsed here rather
        // than handed in, so both fire sites share one derivation.
        let token_eng: Option<eng::TokenEngine> = token_doc
            .engine
            .clone()
            .and_then(|v| serde_json::from_value(v).ok());
        let effective_owner = { self.scene.read().await.token_effective_owner(&token_doc) };

        let mut ops: Vec<Operation> = Vec::new();
        let mut failures: Vec<String> = Vec::new();

        let embedded_actor = token_doc
            .embedded
            .get("actor")
            .and_then(|v| v.first())
            .cloned();
        let resolved_actor = token_eng
            .as_ref()
            .and_then(|t| t.actor_id)
            .or_else(|| embedded_actor.as_ref().map(|a| a.id));

        // --- Condition effects, folded onto the host's current array into ONE update ---
        let needs_conditions = fired.iter().any(|(_, t)| {
            matches!(
                t.effect,
                eng::TriggerEffect::ConditionAdd { .. }
                    | eng::TriggerEffect::ConditionRemove { .. }
            )
        });
        if needs_conditions {
            let host: Option<(Document, &str)> = if embedded_actor.is_some() {
                Some((token_doc.clone(), "/embedded/actor/0/engine/conditions"))
            } else if let Some(actor_id) = resolved_actor {
                match repo.get_document(actor_id).await {
                    Ok(Some(actor)) => Some((actor, "/engine/conditions")),
                    _ => {
                        failures.push("condition host actor could not be loaded".to_string());
                        None
                    }
                }
            } else {
                failures.push("the token has no actor host to carry a condition".to_string());
                None
            };
            if let Some((host_doc, pointer)) = host {
                let mut conditions: Vec<String> = serde_json::to_value(&host_doc)
                    .ok()
                    .and_then(|v| v.pointer(pointer).cloned())
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                let mut dirty = false;
                for (_, trigger) in &fired {
                    match &trigger.effect {
                        eng::TriggerEffect::ConditionAdd { condition }
                            if !conditions.contains(condition) =>
                        {
                            conditions.push(condition.clone());
                            dirty = true;
                        }
                        eng::TriggerEffect::ConditionRemove { condition } => {
                            let before = conditions.len();
                            conditions.retain(|c| c != condition);
                            dirty |= conditions.len() != before;
                        }
                        _ => {}
                    }
                }
                if dirty {
                    match crate::combat::ops::set_engine(
                        &host_doc,
                        pointer,
                        serde_json::json!(conditions),
                    ) {
                        Ok(change) => ops.push(Operation::Update {
                            doc_id: host_doc.id,
                            changes: vec![change],
                        }),
                        Err(_) => {
                            failures.push("condition write could not be built".to_string());
                        }
                    }
                }
            }
        }

        // --- Resource effects, net-summed per key against the token's combatant ---
        let needs_resources = fired
            .iter()
            .any(|(_, t)| matches!(t.effect, eng::TriggerEffect::ResourceDelta { .. }));
        if needs_resources {
            // The combat the delta resolves against: the FIRING combat when
            // the turn/round site named one (`combat_hint` — a boundary's
            // combatant is resolved there even when the token was teleported
            // off that combat's scene), else the token's scene's active
            // combat (the movement sites' rule).
            let combat_id = match combat_hint {
                Some(id) => Some(id),
                None => match scene {
                    Some(scene) => {
                        let active = { self.scene.read().await.active_combat_for_scene(scene) };
                        active.map(|(id, _)| id)
                    }
                    None => None,
                },
            };
            match combat_id {
                None => failures.push("no active combat on the scene".to_string()),
                Some(combat_id) => {
                    match crate::combat::load_snapshot(repo, self.world_id, combat_id).await {
                        Err(_) => {
                            failures
                                .push("the scene's active combat could not be loaded".to_string());
                        }
                        Ok(snap) => {
                            let combatant = snap
                                .combatants
                                .iter()
                                .find(|c| {
                                    matches!(&c.engine.kind, eng::CombatantKind::Actor { token_id: Some(t), .. } if *t == token)
                                })
                                .or_else(|| {
                                    snap.combatants.iter().find(|c| {
                                        matches!(&c.engine.kind, eng::CombatantKind::Actor { actor_id: Some(a), .. } if Some(*a) == resolved_actor)
                                    })
                                });
                            match combatant {
                                None => failures.push(
                                    "the token names no combatant in the active combat".to_string(),
                                ),
                                Some(combatant) => {
                                    let host = crate::combat::eval::formula_host(
                                        &snap.hosts,
                                        &combatant.engine.kind,
                                    );
                                    let mut sums: std::collections::BTreeMap<String, f64> =
                                        std::collections::BTreeMap::new();
                                    for (_, trigger) in &fired {
                                        if let eng::TriggerEffect::ResourceDelta {
                                            resource,
                                            amount,
                                        } = &trigger.effect
                                        {
                                            match crate::combat::eval::eval_formula(amount, host) {
                                                Ok(value) => {
                                                    *sums.entry(resource.clone()).or_insert(0.0) +=
                                                        value;
                                                }
                                                Err(err) => failures.push(format!(
                                                    "resource '{resource}' amount failed to evaluate: {}",
                                                    err.detail
                                                )),
                                            }
                                        }
                                    }
                                    // Every key's FieldChange folds into ONE Update on the
                                    // combatant: the store checks EACH op's pre-image against
                                    // PRE-BATCH state and applies ops in order (two Updates to
                                    // one document are admitted, never chained), so a second
                                    // per-key Update carrying the pre-fire value as its `old`
                                    // would silently overwrite the first's write instead of
                                    // composing with it.
                                    let mut changes = Vec::new();
                                    for (key, sum) in sums {
                                        match resource_transition(
                                            &snap,
                                            combatant.doc.id,
                                            &key,
                                            ResourceOp::Delta { amount: sum },
                                        ) {
                                            Ok(mut resource_ops) => {
                                                if let Some(Operation::Update {
                                                    changes: mut cs,
                                                    ..
                                                }) = resource_ops.pop()
                                                {
                                                    changes.append(&mut cs);
                                                }
                                            }
                                            Err(CombatError::Forbidden) => failures.push(format!(
                                                "resource '{key}' is mirror-bound; its value lives on the actor"
                                            )),
                                            Err(CombatError::NotFound) => failures.push(format!(
                                                "resource '{key}' is not in the world resource registry"
                                            )),
                                            Err(_) => failures.push(format!(
                                                "resource '{key}' adjustment failed"
                                            )),
                                        }
                                    }
                                    if !changes.is_empty() {
                                        ops.push(Operation::Update {
                                            doc_id: combatant.doc.id,
                                            changes,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // --- Teleport: at most ONE per token per fire (a region's own trigger list could in
        // principle carry several teleports; only the FIRST wins — a second is silently ignored
        // rather than double-moving the token, mirroring "one hop per move"). A `PortalChoice`
        // is a kind of teleport: it shares the one-hop guard, so a fire that may not teleport
        // refuses it with the same chained-portal failure. A fixed teleport applies now; a
        // choice only registers an offer (`Room::offer_portal`), and its hop applies later,
        // through this same funnel, when the offer is answered. ---
        let mut teleported: Option<Uuid> = None;
        let hop = fired.iter().find_map(|(region_id, t)| match &t.effect {
            eng::TriggerEffect::Teleport { target } => Some(HopEffect::Fixed(target)),
            eng::TriggerEffect::PortalChoice { prompt, options } if portal::offers_portal(t) => {
                // Secrecy: an unresolvable row fails closed to GM-only, as for a notice.
                let visible_to_all = secrecy_rows
                    .iter()
                    .find(|r| &r.region_id == region_id)
                    .map(|r| r.visible_to_all)
                    .unwrap_or(false);
                Some(HopEffect::Choice(portal::PortalSource {
                    region_id: *region_id,
                    visible_to_all,
                    prompt: prompt.as_ref(),
                    options,
                }))
            }
            _ => None,
        });
        if allow_teleport {
            match hop {
                Some(HopEffect::Fixed(target)) => {
                    let subject = TeleportSubject {
                        token,
                        scene,
                        token_eng: token_eng.as_ref(),
                        effective_owner,
                    };
                    match self
                        .apply_teleport_to(repo, ctx, &subject, target, ts, &mut ops)
                        .await
                    {
                        Ok(dest_scene) => teleported = Some(dest_scene),
                        Err(failure) => failures.push(failure),
                    }
                }
                Some(HopEffect::Choice(source)) => {
                    self.offer_portal(repo, &token_doc, source, ts).await;
                }
                None => {}
            }
        } else if fired.iter().any(|(_, t)| {
            matches!(
                t.effect,
                eng::TriggerEffect::Teleport { .. } | eng::TriggerEffect::PortalChoice { .. }
            )
        }) {
            failures.push("chained portal refused (one hop per move)".to_string());
        }

        // --- Scrolling-text overlays, in fired order. Out-of-band aux broadcast (no document,
        // no seq) mirroring `Emote`'s own shape — the client resolves the anchor position from
        // `token` via `EmoteView`, exactly as a player-fired emote does. `gm_only` mirrors
        // `ChatNotice`'s own secrecy forcing: a region whose `/engine` band is not visible to
        // every world member is delivered only to GM connections (`egress_loop`'s per-connection
        // filter), never broadcast to the whole room. ---
        for (region_id, trigger) in &fired {
            if let eng::TriggerEffect::ScrollingText { text } = &trigger.effect {
                let Some(scene_id) = scene else { continue };
                let visible_to_all = secrecy_rows
                    .iter()
                    .find(|r| &r.region_id == region_id)
                    .map(|r| r.visible_to_all)
                    .unwrap_or(false);
                self.broadcast_aux(ServerMsg::ScrollingText {
                    scene: scene_id,
                    token,
                    text: text.clone(),
                    gm_only: !visible_to_all,
                });
            }
        }

        // --- Chat notices, in fired order ---
        for (region_id, trigger) in &fired {
            if let eng::TriggerEffect::ChatNotice { text, audience } = &trigger.effect {
                // Secrecy: a region whose `/engine` band is not visible to every world member
                // forces GM-only whatever the authored audience says — a wider notice would
                // name or imply a region some recipients cannot see. The row is found by id
                // across `secrecy_rows` (the caller's full row set — the movement site's
                // scene table AND its stale exit rows, so an Exit-fired notice rides the
                // identical rule); an unresolvable row fails closed to forced.
                let visible_to_all = secrecy_rows
                    .iter()
                    .find(|r| &r.region_id == region_id)
                    .map(|r| r.visible_to_all)
                    .unwrap_or(false);
                let forced = !visible_to_all;
                let wire_audience = if forced {
                    Audience::GmOnly
                } else {
                    match audience {
                        eng::NoticeAudience::Public => Audience::Public,
                        eng::NoticeAudience::GmOnly | eng::NoticeAudience::Owner => {
                            Audience::GmOnly
                        }
                    }
                };
                let mut doc = build_message_doc(
                    self.world_id,
                    ctx.user_id,
                    MessageDraft {
                        channel: "region".to_string(),
                        actor_owner: None,
                        audience: wire_audience,
                        kind: MessageKind::System,
                        content: vec![Segment::Text { text: text.clone() }],
                        source: None,
                        system: None,
                    },
                    ts,
                );
                // `Owner` = effective owner + every GM: the GM-only shape plus an explicit
                // owner grant. Never a downgrade of the author (`build_message_doc`'s own
                // users-last invariant), and never under a secrecy force — a secret region's
                // notice reaches no non-GM through any door.
                if !forced && matches!(audience, eng::NoticeAudience::Owner) {
                    if let Some(owner) = effective_owner {
                        if owner != ctx.user_id {
                            doc.permissions
                                .users
                                .insert(owner, crate::data::document::DocRole::Observer);
                        }
                    }
                }
                ops.push(Operation::Create { doc });
            }
        }

        // --- ONE deduplicated GM-only notice for every skipped effect (`eval_notice`'s shape) ---
        if !failures.is_empty() {
            let mut seen = std::collections::HashSet::new();
            let lines: Vec<String> = failures
                .into_iter()
                .filter(|f| seen.insert(f.clone()))
                .collect();
            let doc = build_message_doc(
                self.world_id,
                ctx.user_id,
                MessageDraft {
                    channel: "region".to_string(),
                    actor_owner: None,
                    audience: Audience::GmOnly,
                    kind: MessageKind::System,
                    content: vec![Segment::Text {
                        text: format!("Region trigger did not apply: {}", lines.join("; ")),
                    }],
                    source: None,
                    system: None,
                },
                ts,
            );
            ops.push(Operation::Create { doc });
        }

        if ops.is_empty() {
            return;
        }
        match self
            .commit_ops_locked(repo, ctx, ops, ts, WriteOrigin::Trigger)
            .await
        {
            Err(err) => {
                tracing::debug!(
                    ?scene, %token, ?err,
                    "region-trigger commit failed after the triggering write already committed; \
                     the write stands, the effects were not applied"
                );
            }
            Ok(_) => {
                // A teleport that committed re-fires `Enter` on the destination cells — EXCEPT
                // another Teleport (`allow_teleport: false`): one hop per move, so a portal
                // chain can never loop. The destination's other effects (conditions, resources,
                // notices) fire exactly as if the token had been placed there.
                if teleported.is_some() {
                    let new_footprint = {
                        let ecs = self.scene.read().await;
                        // The ONE footprint-cell computation every fire site shares
                        // (`SceneEcs::token_footprint_cells`), scene derived from the
                        // token's committed document. `None` (the token or its scene
                        // unreadable, or a refused footprint) skips the re-fire silently,
                        // mirroring placement's own `continue`.
                        ecs.token_footprint_cells(token)
                    };
                    if let Some((scene, entered)) = new_footprint {
                        Box::pin(self.fire_region_triggers(
                            repo,
                            ctx,
                            TriggerReport {
                                scene,
                                token,
                                // The destination re-fire is placement-shaped: its
                                // occupancy basis is the destination footprint, the same
                                // cells its Enter detection uses.
                                occupancy: entered.clone(),
                                entered,
                                arrest_stop: None,
                                enter_only: None,
                            },
                            ts,
                            false,
                        ))
                        .await;
                        // The teleported token may carry following regions: the zone arrived
                        // with it. The attached pass is itself one-hop (`allow_teleport: false`).
                        Box::pin(self.fire_attached_region_triggers(repo, ctx, token, ts, false))
                            .await;
                    }
                }
            }
        }
    }

    /// The ONE teleport implementation: moves the token described by `subject` to `target`
    /// and returns the destination scene id, or the GM-notice failure line that refused it.
    /// Appends the committing ops (a cross-scene `Move`, the active-combat notice for a token
    /// leaving a combat's scene, the position `Update`) to `ops` and broadcasts the target's
    /// `vfx` at both ends of the hop, charged once to the token owner's `vfx_rate` bucket.
    /// Both a fixed `TriggerEffect::Teleport` and a chosen `PortalOption` apply through it, so
    /// the two can never produce different ops for the same target.
    ///
    /// The destination scene's existence and same-world membership are checked here (fire
    /// time), never at ingress (`validate_engine_tree` is pure — no repository access). A
    /// wrong-world scene would otherwise pass `doc_type == "scene"` and fail deep inside
    /// `commit_ops_locked`'s `check_command_scope`, dropping the whole batch with no GM
    /// notice; returning the failure here lets the batch's other effects still commit.
    ///
    /// PRECONDITION: the caller commits `ops` under `publish_guard`.
    async fn apply_teleport_to(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        subject: &TeleportSubject<'_>,
        target: &eng::PortalTarget,
        ts: i64,
        ops: &mut Vec<Operation>,
    ) -> Result<Uuid, String> {
        use crate::chat::{build_message_doc, Audience, MessageDraft, MessageKind, Segment};

        let TeleportSubject {
            token,
            scene,
            token_eng,
            effective_owner,
        } = *subject;
        // The destination scene: the target's own when it names one, else the token's current
        // scene (a same-scene hop). A token on NO scene (reachable only from the turn/round
        // fire site, whose token may sit outside every scene) can hop only to a named target.
        let dest_scene = target.scene.or(scene);
        let scene_exists = match dest_scene {
            Some(dest) => match repo.get_document(dest).await {
                Ok(Some(doc)) => {
                    doc.doc_type == "scene"
                        && crate::data::document::world_of(&doc) == Some(self.world_id)
                }
                _ => false,
            },
            None => false,
        };
        let Some(dest_scene) = dest_scene.filter(|_| scene_exists) else {
            return Err(match dest_scene {
                Some(dest) => format!("teleport target scene {dest} does not exist"),
                None => "the token is on no scene to teleport from".to_string(),
            });
        };
        let Some(t) = token_eng else {
            return Err("token has no engine body to teleport".to_string());
        };
        let mut update_changes = Vec::new();
        update_changes.push(crate::data::command::FieldChange {
            path: "/engine/x".to_string(),
            old: serde_json::json!(t.x),
            new: serde_json::json!(target.x),
            remove: false,
        });
        update_changes.push(crate::data::command::FieldChange {
            path: "/engine/y".to_string(),
            old: serde_json::json!(t.y),
            new: serde_json::json!(target.y),
            remove: false,
        });
        if let Some(new_elev) = target.elevation {
            update_changes.push(crate::data::command::FieldChange {
                path: "/engine/elevation".to_string(),
                old: serde_json::json!(t.elevation),
                new: serde_json::json!(new_elev),
                remove: false,
            });
        }
        if Some(dest_scene) != scene {
            ops.push(Operation::Move {
                doc_id: token,
                parent_id: Some(dest_scene),
                old_parent_id: scene,
            });
            // Active-combat visibility: the budget decrement for the walk that
            // entered the portal already happened; this notice only makes the
            // scene-change visible.
            if let Some(source_scene) = scene {
                if self
                    .scene
                    .read()
                    .await
                    .active_combat_for_scene(source_scene)
                    .is_some()
                {
                    let notice = build_message_doc(
                        self.world_id,
                        ctx.user_id,
                        MessageDraft {
                            channel: "region".to_string(),
                            actor_owner: None,
                            audience: Audience::GmOnly,
                            kind: MessageKind::System,
                            content: vec![Segment::Text {
                                text: format!(
                                    "Token {token} teleported off scene {source_scene} while an active combat is running there"
                                ),
                            }],
                            source: None,
                            system: None,
                        },
                        ts,
                    );
                    ops.push(Operation::Create { doc: notice });
                }
            }
        }
        ops.push(Operation::Update {
            doc_id: token,
            changes: update_changes,
        });
        // A carried `vfx` asset plays at BOTH ends of the hop — the source position
        // on the ORIGIN scene (where the token just vanished from) and the
        // destination position on `dest_scene` (where it now stands) — mirroring
        // `chat::fx`'s `ServerMsg::Vfx` broadcast shape (`ScenePing`'s precedent:
        // out-of-band, no seq, one fresh id per broadcast). Charged against
        // `self.vfx_rate` BEFORE either broadcast — the SAME `Arc<PingRateLimiter>`
        // `RoomRegistry::vfx_rate` hands to `WsState::vfx_rate`, which the raw
        // `ClientMsg::PlayVfx` frame and `/fx` already share — so repeatedly walking
        // into and out of a portal's trigger region cannot emit unlimited,
        // unthrottled broadcasts the way an unthrottled third VFX-broadcast site
        // would (never-fork: one budget source per one-shot family). Keyed on the
        // token's effective owner: a GM-owned/NPC-moved token
        // (`effective_owner: None`) has no attributable player to charge and is
        // admitted uncharged, matching this codebase's broader pattern of GM actions
        // being less restricted than player ones; a player-owned token charges that
        // player's bucket exactly once per fire (one check covers both ends of the
        // hop, not two).
        if let Some(asset) = &target.vfx {
            let vfx_admitted = match effective_owner {
                Some(owner) => self.vfx_rate.check(owner, ts, 30),
                None => true,
            };
            if vfx_admitted {
                if let Some(source_scene) = scene {
                    self.broadcast_aux(ServerMsg::Vfx {
                        scene: source_scene,
                        user: ctx.user_id,
                        asset: asset.clone(),
                        x: t.x,
                        y: t.y,
                        scale: None,
                        rotation: None,
                        duration_ms: None,
                        sound: None,
                        elevation: t.elevation,
                        id: Uuid::new_v4(),
                    });
                }
                self.broadcast_aux(ServerMsg::Vfx {
                    scene: dest_scene,
                    user: ctx.user_id,
                    asset: asset.clone(),
                    x: target.x,
                    y: target.y,
                    scale: None,
                    rotation: None,
                    duration_ms: None,
                    sound: None,
                    elevation: target.elevation,
                    id: Uuid::new_v4(),
                });
            }
        }
        Ok(dest_scene)
    }

    /// The placement/teleport fire site: token Creates and genuine position Updates in a
    /// committed batch fire `Enter` effects on each token's NEW footprint
    /// (`fire_region_triggers` applies them). Only token ops are candidates — a region-doc
    /// edit never reaches here, so editing a region never re-fires its triggers.
    ///
    /// `tokens` are the candidates `publish` resolved PRE-commit through
    /// `SceneEcs::token_move`: a Create always candidates; an Update only when its applied
    /// post-image position differs from the committed one (the same comparison the move gate
    /// makes, so a wholesale `/engine` write is evaluated on its post-image too). Footprint
    /// cells derive through the SAME machinery the move gate uses
    /// (`SceneEcs::resolve_token_footprint` + `GridShape::footprint_cells`), never a second
    /// footprint formula; a token whose footprint refuses (degenerate size) fires nothing —
    /// the gate's own fail-closed refusal.
    ///
    /// PRECONDITION: caller holds `publish_guard`.
    pub(crate) async fn fire_placement_triggers(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        tokens: Vec<Uuid>,
        ts: i64,
    ) {
        // One report per candidate token, resolved post-commit under one read guard.
        let mut reports: Vec<TriggerReport> = Vec::new();
        {
            let ecs = self.scene.read().await;
            let mut seen = std::collections::HashSet::new();
            for token in tokens {
                if !seen.insert(token) {
                    continue;
                }
                // The ONE footprint-cell computation every fire site shares
                // (`SceneEcs::token_footprint_cells`), scene derived from the token.
                let Some((scene, cells)) = ecs.token_footprint_cells(token) else {
                    continue;
                };
                reports.push(TriggerReport {
                    scene,
                    token,
                    // A placement's occupancy basis IS its footprint — the same cells its
                    // Enter detection uses (`TriggerReport::occupancy`).
                    occupancy: cells.clone(),
                    entered: cells,
                    arrest_stop: None,
                    enter_only: None,
                });
            }
        }
        let carriers: Vec<Uuid> = reports.iter().map(|r| r.token).collect();
        for report in reports {
            self.fire_region_triggers(repo, ctx, report, ts, true).await;
        }
        for carrier in carriers {
            self.fire_attached_region_triggers(repo, ctx, carrier, ts, true)
                .await;
        }
    }

    /// The carrier-move fire site: after `carrier` committed a new position, fires the
    /// triggers of the regions that follow it on every OTHER token of its scene, tokens that
    /// did not move but that the zone moved onto or off of. Each occupant
    /// (`SceneEcs::attached_occupants`) gets one `TriggerReport` whose occupancy is its centre
    /// cell, so `fire_region_triggers`'s unconditional membership diff yields `Exit` for
    /// regions the zone left; `Enter` fires only for attached regions the token's stored
    /// membership does not name yet (`TriggerReport::enter_only`), so a second carrier step
    /// with the token still inside fires nothing. A carrier with no attached region is a
    /// single index lookup.
    ///
    /// Called after the carrier's own report at every position-commit site:
    /// `execute_move`, `fire_placement_triggers` (placements, GM teleports and drags) and
    /// the post-teleport re-fire in `fire_region_triggers`. `allow_teleport` is `false` only
    /// from that last site, so teleport effects cannot chain across carriers.
    ///
    /// PRECONDITION: caller holds `publish_guard`.
    pub(crate) async fn fire_attached_region_triggers(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        carrier: Uuid,
        ts: i64,
        allow_teleport: bool,
    ) {
        let occupants = self.scene.read().await.attached_occupants(carrier);
        for occ in occupants {
            let entered = if occ.enter_regions.is_empty() {
                Vec::new()
            } else {
                vec![occ.cell]
            };
            self.fire_region_triggers(
                repo,
                ctx,
                TriggerReport {
                    scene: occ.scene,
                    token: occ.token,
                    entered,
                    arrest_stop: None,
                    occupancy: vec![occ.cell],
                    enter_only: Some(occ.enter_regions),
                },
                ts,
                allow_teleport,
            )
            .await;
        }
    }

    /// The turn/round-scoped region-trigger fire site: fires the turn/round
    /// boundaries a COMMITTED `CombatStart`/`CombatAdvance` transition
    /// crossed (`combat::transition::TurnRoundBoundary` — recorded by the
    /// pure transition, applied here) against the trigger-bearing regions
    /// each boundary combatant's token is a recorded member of
    /// (`SceneEcs::region_membership` — READ here, never written: a clock
    /// boundary does not move tokens, so membership changes only through the
    /// movement/placement funnel). Turn-scoped entries fire for the one
    /// combatant whose turn it is; round-scoped entries expand across
    /// `covered` (the order at crossing time), one fire per member token per
    /// round boundary — `round_wrap`'s own structure. Only a combatant whose
    /// kind names a token (`CombatantKind::Actor { token_id: Some, .. }`)
    /// fires: an unbound combatant has no scene position, so no membership
    /// can prove its occupancy. (The budget gate's `actor_id` fallback
    /// resolves token→combatant; the inverse combatant→token is one-to-many
    /// and would pick one token's position arbitrarily — a forked decision
    /// this rule deliberately avoids.)
    ///
    /// Effects apply through `apply_region_trigger_effects`, the SAME funnel
    /// the movement sites use, committing under `WriteOrigin::Trigger` as a
    /// SECOND batch after the transition's own commit — the split
    /// is structural, not stylistic: the effect ops' OCC pre-images must be
    /// read from committed post-transition state (the transition's own
    /// recovery may have just written the same combatant document).
    /// `apply_intent` admits MULTIPLE `Update`s to one document in a batch
    /// and checks each op's pre-image against PRE-BATCH state, applying in
    /// order — so a folded-in effect op computed from the pre-transition
    /// snapshot would not be rejected; it would silently overwrite the
    /// transition's own write to the same path (the last applied wins).
    /// `combat_hint` resolves a `ResourceDelta` against the FIRING combat
    /// even when the token was teleported off its scene.
    ///
    /// Once-per-turn/round dedup (`Room::turn_round_dedup`, key
    /// `TurnRoundFireKey`) runs at PLAN time: the set records that the clock
    /// crossed the boundary, so an effect commit failure (swallowed inside
    /// the application funnel) is lost exactly as the movement sites'
    /// swallowed failures are — one failure philosophy.
    ///
    /// PRECONDITION: caller holds `publish_guard` (`commit_ops_locked`
    /// requires it, and the ONE acquisition must span the transition commit
    /// and this fire — see `commit_combat_and_fire`, this function's only
    /// caller, for the interleave that rule closes).
    pub(crate) async fn fire_turn_round_triggers(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        snap: &crate::combat::CombatSnapshot,
        plan: Vec<crate::combat::transition::TurnRoundBoundary>,
        ts: i64,
    ) {
        use crate::scene::regions;

        if plan.is_empty() {
            return;
        }

        // Resolve each boundary to the tokens it fires for, preserving plan
        // (chronological) order for application. `by_token` values are
        // (combatant, event, round) triples.
        let mut token_order: Vec<Uuid> = Vec::new();
        let mut by_token: HashMap<Uuid, Vec<(Uuid, eng::TriggerEvent, u32)>> = HashMap::new();
        for boundary in &plan {
            let combatants: &[Uuid] = match &boundary.combatant {
                Some(c) => std::slice::from_ref(c),
                None => &boundary.covered,
            };
            for &combatant in combatants {
                let Some(token) = snap
                    .combatants
                    .iter()
                    .find(|c| c.doc.id == combatant)
                    .and_then(|c| match c.engine.kind {
                        eng::CombatantKind::Actor { token_id, .. } => token_id,
                        _ => None,
                    })
                else {
                    continue;
                };
                by_token
                    .entry(token)
                    .or_insert_with(|| {
                        token_order.push(token);
                        Vec::new()
                    })
                    .push((combatant, boundary.event, boundary.round));
            }
        }
        if token_order.is_empty() {
            return;
        }

        // The dedup window update, once per pass: fold this plan's highest
        // round in, then prune keys older than the previous round (the
        // rotation `TurnRoundDedup::last_round` documents).
        let plan_max_round = plan.iter().map(|b| b.round).max().unwrap_or(0);
        {
            let mut dedup = self.turn_round_dedup.lock().await;
            if dedup.len() >= MAX_TURN_ROUND_DEDUP_COMBATS && !dedup.contains_key(&snap.combat.id) {
                dedup.clear();
            }
            let entry = dedup.entry(snap.combat.id).or_default();
            entry.last_round = entry.last_round.max(plan_max_round);
            let last_round = entry.last_round;
            entry.keys.retain(|k| k.round + 1 >= last_round);
        }

        for token in token_order {
            let boundaries = by_token.remove(&token).unwrap_or_default();
            // Membership + identity rows under ONE read guard (never nested
            // with the dedup lock): membership is the occupancy proof; rows
            // resolve by id WITHOUT the per-scene/elevation filter — the
            // `trigger_region_row` rule Exit firing already uses, for the
            // same reason (re-filtering would drop a region the token was
            // legitimately recorded inside).
            let (member, rows) = {
                let ecs = self.scene.read().await;
                let member: std::collections::BTreeSet<Uuid> =
                    ecs.region_membership_of(token).cloned().unwrap_or_default();
                let rows: Vec<regions::TriggerRegion> = member
                    .iter()
                    .filter_map(|&id| ecs.trigger_region_row(id))
                    .collect();
                (member, rows)
            };
            if member.is_empty() {
                continue;
            }
            // The candidate fires in plan order, then the seen-set filter.
            let mut candidates: Vec<(TurnRoundFireKey, (Uuid, eng::RegionTrigger))> = Vec::new();
            for (combatant, event, round) in boundaries {
                for (region, index, trigger) in
                    regions::fired_turn_round_triggers(&rows, &member, event)
                {
                    candidates.push((
                        TurnRoundFireKey {
                            event,
                            round,
                            combatant,
                            region,
                            trigger: index,
                        },
                        (region, trigger),
                    ));
                }
            }
            let fired: Vec<(Uuid, eng::RegionTrigger)> = {
                let mut dedup = self.turn_round_dedup.lock().await;
                let entry = dedup.entry(snap.combat.id).or_default();
                candidates
                    .into_iter()
                    .filter(|(key, _)| entry.keys.insert(*key))
                    .map(|(_, pair)| pair)
                    .collect()
            };
            if fired.is_empty() {
                continue;
            }
            // The token's committed document, loaded after the transition's
            // commit — the application funnel's own discipline.
            let token_doc = match repo.get_document(token).await {
                Ok(Some(doc)) => doc,
                Ok(None) => {
                    tracing::debug!(%token, "turn/round region triggers: the token is gone; no effects fire");
                    continue;
                }
                Err(err) => {
                    tracing::debug!(%token, ?err, "turn/round region triggers: the token could not be loaded; no effects fire");
                    continue;
                }
            };
            self.apply_region_trigger_effects(
                repo,
                ctx,
                token_doc,
                TriggerEffectPlan {
                    fired,
                    secrecy_rows: &rows,
                    combat_hint: Some(snap.combat.id),
                    allow_teleport: true,
                },
                ts,
            )
            .await;
        }
    }

    /// Every unexpired in-flight frame in `scene`, keyed by its moving token — the timelines
    /// (the movers' viewpoints and carried lights per instant) the egress clip composes a
    /// recipient's sight from (`ws::move_clip::ClipInputs`). Mutates `moving`:
    /// opportunistically prunes entries expired as of `now` before reading (see the pruning
    /// comment in the body).
    pub(crate) async fn scene_streams(&self, scene: Uuid, now: i64) -> Vec<(Uuid, Arc<ServerMsg>)> {
        let mut moving = self.moving.lock().await;
        // Opportunistic prune alongside the read: reclaims an expired entry as soon as any
        // further move triggers a read here, rather than waiting for that entry's OWN next
        // move to hit `execute_move`'s post-commit `retain`. Does not bound registry size on
        // its own — a room with no further moves at all is never read here and its last
        // frames stay resident regardless (mutable only because this call mutates `moving`,
        // not because callers here need a snapshot).
        moving.retain(|_, st| now < st.end_ms);
        moving
            .iter()
            .filter(|(_, st)| st.scene == scene)
            .map(|(token, st)| (*token, st.frame.clone()))
            .collect()
    }

    /// Unexpired in-flight frames in `scene` moved by anyone other than `exclude_mover` —
    /// re-clipped and re-emitted to a recipient whose own move just started. Excludes by
    /// MOVER so a recipient's other in-flight token is never re-sent to them. Mutates `moving`:
    /// opportunistically prunes entries expired as of `now` before reading (see the pruning
    /// comment in the body).
    pub(crate) async fn concurrent_streams(
        &self,
        scene: Uuid,
        exclude_mover: Uuid,
        now: i64,
    ) -> Vec<Arc<ServerMsg>> {
        let mut moving = self.moving.lock().await;
        // Opportunistic prune alongside the read: reclaims an expired entry as soon as any
        // further move triggers a read here, rather than waiting for that entry's OWN next
        // move to hit `execute_move`'s post-commit `retain`. Does not bound registry size on
        // its own — a room with no further moves at all is never read here and its last
        // frames stay resident regardless (mutable only because this call mutates `moving`,
        // not because callers here need a snapshot).
        moving.retain(|_, st| now < st.end_ms);
        moving
            .values()
            .filter(|st| st.mover != exclude_mover && st.scene == scene)
            .map(|st| st.frame.clone())
            .collect()
    }

    /// Test-only direct registration (bypasses `execute_move`'s gate) for clip/egress tests.
    #[cfg(test)]
    pub(crate) async fn register_stream_for_test(&self, token: Uuid, stream: ActiveStream) {
        self.moving.lock().await.insert(token, stream);
    }

    /// Resolve a resync range: hot ring tier when fully resident, else the cold
    /// `events_since` tier. Increments the matching telemetry counter.
    pub(crate) async fn resync_range(
        &self,
        repo: &dyn Repository,
        from_seq: i64,
    ) -> Result<(Vec<RoomEvent>, ResyncSource), DataError> {
        if let Some(hot) = self.ring.lock().await.range_from(from_seq) {
            self.stats.resyncs_hot.fetch_add(1, Ordering::Relaxed);
            return Ok((hot, ResyncSource::Buffer));
        }
        let cmds = repo.events_since(self.world_id, from_seq - 1).await?;
        self.stats.resyncs_cold.fetch_add(1, Ordering::Relaxed);
        let frames = cmds
            .into_iter()
            .map(|stored| RoomEvent::Event(Arc::new(stored)))
            .collect();
        Ok((frames, ResyncSource::Log))
    }

    /// Records `user_id`'s resync floor at this room's CURRENT `current_seq` — called on a
    /// cold-start `ClientMsg::Hello { last_seq: None }`. Idempotent-safe to call repeatedly:
    /// each call simply advances the floor to whatever `current_seq` is at that moment, which
    /// can only move forward over time (a later cold start never legitimately needs an
    /// EARLIER floor than one already established).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::ws::room::RoomRegistry;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    /// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
    /// let room = RoomRegistry::new()
    ///     .get_or_create(&repo, world.id)
    ///     .await
    ///     .unwrap()
    ///     .unwrap();
    ///
    /// room.establish_resync_floor(gm).await;
    /// // Inclusive floor = current_seq + 1, matching `ResyncRequest.from_seq`.
    /// assert_eq!(room.resync_floor(gm).await, room.current_seq() + 1);
    /// # }
    /// ```
    pub async fn establish_resync_floor(&self, user_id: Uuid) {
        let seq = self.current_seq();
        self.session_floors.lock().await.insert(user_id, seq);
    }

    /// The lowest `from_seq` `user_id` may currently resync from (INCLUSIVE — matches
    /// `ClientMsg::ResyncRequest.from_seq`'s own inclusive semantics). A `user_id` with no
    /// recorded floor (never sent a cold-start `Hello` this room's lifetime) fails closed to
    /// `current_seq() + 1` — an EMPTY resync, not an unbounded one.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::ws::room::RoomRegistry;
    /// use uuid::Uuid;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    /// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
    /// let room = RoomRegistry::new()
    ///     .get_or_create(&repo, world.id)
    ///     .await
    ///     .unwrap()
    ///     .unwrap();
    ///
    /// // No cold-start `Hello` recorded for this user yet: fails closed to an
    /// // empty resync rather than an unbounded one.
    /// assert_eq!(room.resync_floor(Uuid::new_v4()).await, room.current_seq() + 1);
    /// # }
    /// ```
    pub async fn resync_floor(&self, user_id: Uuid) -> i64 {
        match self.session_floors.lock().await.get(&user_id) {
            Some(&floor) => floor + 1,
            None => self.current_seq() + 1,
        }
    }

    /// Records one more live connection for `user`; `true` iff it is the user's first in this
    /// room (the 0->1 transition). The caller broadcasts `ServerMsg::Presence { online: true }`
    /// only on `true`, so a second tab never re-announces an already-online user.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::room::RoomRegistry;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::auth::role::ServerRole;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    /// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
    /// let room = RoomRegistry::new()
    ///     .get_or_create(&repo, world.id)
    ///     .await
    ///     .unwrap()
    ///     .unwrap();
    ///
    /// assert!(room.presence_connect(gm));
    /// assert!(!room.presence_connect(gm));
    /// # }
    /// ```
    pub fn presence_connect(&self, user: Uuid) -> bool {
        let mut counts = self.presence.lock().unwrap_or_else(|e| e.into_inner());
        let count = counts.entry(user).or_insert(0);
        *count += 1;
        *count == 1
    }

    /// Drops one live connection for `user`; `true` iff the count reached zero (the 1->0
    /// transition). A user with no recorded connection is a no-op returning `false`; the count
    /// never underflows and a zero count leaves no map entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::room::RoomRegistry;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::auth::role::ServerRole;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    /// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
    /// let room = RoomRegistry::new()
    ///     .get_or_create(&repo, world.id)
    ///     .await
    ///     .unwrap()
    ///     .unwrap();
    ///
    /// room.presence_connect(gm);
    /// room.presence_connect(gm);
    /// assert!(!room.presence_disconnect(gm));
    /// assert!(room.presence_disconnect(gm));
    /// assert!(!room.presence_disconnect(gm));
    /// # }
    /// ```
    pub fn presence_disconnect(&self, user: Uuid) -> bool {
        let mut counts = self.presence.lock().unwrap_or_else(|e| e.into_inner());
        match counts.get_mut(&user) {
            Some(count) if *count > 1 => {
                *count -= 1;
                false
            }
            Some(_) => {
                counts.remove(&user);
                true
            }
            None => false,
        }
    }

    /// Whether an explicit `ClientMsg::ResyncRequest` should be clamped against
    /// `resync_floor`. See the `resync_floor_enforced_flag` field doc for why this is `true`
    /// for every production constructor.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::ws::room::RoomRegistry;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    /// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
    /// let room = RoomRegistry::new()
    ///     .get_or_create(&repo, world.id)
    ///     .await
    ///     .unwrap()
    ///     .unwrap();
    ///
    /// // Always true for a production `RoomRegistry` constructor.
    /// assert!(room.resync_floor_enforced());
    /// # }
    /// ```
    pub fn resync_floor_enforced(&self) -> bool {
        self.resync_floor_enforced_flag
    }

    /// One consistent-enough telemetry snapshot (relaxed loads; counters may
    /// skew by in-flight increments).
    ///
    /// # Examples
    ///
    /// ```text
    /// let stats = room.snapshot(); // admin debug endpoint payload
    /// ```
    fn snapshot(&self) -> RoomStatsSnapshot {
        RoomStatsSnapshot {
            world_id: self.world_id,
            connections: self.stats.connections.load(Ordering::Acquire),
            current_seq: self.current_seq(),
            events_published: self.stats.events_published.load(Ordering::Relaxed),
            gaps_detected: self.stats.gaps_detected.load(Ordering::Relaxed),
            resyncs_hot: self.stats.resyncs_hot.load(Ordering::Relaxed),
            resyncs_cold: self.stats.resyncs_cold.load(Ordering::Relaxed),
            lagged_drops: self.stats.lagged_drops.load(Ordering::Relaxed),
        }
    }
}

/// World -> room map. The stable abstraction boundary: the broadcast internals
/// can later be swapped for an actor or an external broker without touching
/// callers or connections.
///
/// # Examples
///
/// ```
/// use shadowcat::ws::room::RoomRegistry;
///
/// let registry = RoomRegistry::default();
/// assert!(registry.get(uuid::Uuid::nil()).is_none()); // no room until a join hydrates one
/// ```
pub struct RoomRegistry {
    /// Live rooms by world id.
    rooms: DashMap<Uuid, Arc<Room>>,
    /// Worlds mid-deletion. `get_or_create` refuses these so an evicted
    /// client's reconnect (or a racing HTTP document write) cannot re-hydrate
    /// a room between the eviction broadcast and the DB commit that removes
    /// the world row. Lifted by `finish_delete` on success AND failure.
    deleting: DashSet<Uuid>,
    /// Broadcast ring capacity for rooms created by this registry. Production uses
    /// `BROADCAST_CAPACITY`; test harnesses shrink it to force the lag path.
    broadcast_capacity: usize,
    /// Whether rooms created by this registry enforce the resync floor against an
    /// explicit `ResyncRequest` (`Room::resync_floor_enforced`). `true` for every
    /// production constructor: the client unconditionally sends a cold-start `Hello`
    /// as the first frame on every socket open, so enforcement is always safe.
    resync_floor_enforced: bool,
    /// Per-user VFX one-shot budget shared by every room this registry creates. `WsState::new`
    /// clones this SAME instance into `WsState.vfx_rate` (via `RoomRegistry::vfx_rate`), so the
    /// raw `ClientMsg::PlayVfx` frame, `/fx`, and any `Room`-internal trigger-fired VFX draw from
    /// one bucket rather than three independent ones.
    vfx_rate: Arc<PingRateLimiter>,
}

impl RoomRegistry {
    /// A registry with the production broadcast capacity, whose rooms enforce the
    /// resync floor against an explicit `ResyncRequest` (see `Room`'s
    /// `resync_floor_enforced_flag` doc for why this is safe unconditionally).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::room::RoomRegistry;
    ///
    /// let reg = RoomRegistry::new();
    /// assert!(reg.get(uuid::Uuid::nil()).is_none()); // no room until a join hydrates one
    /// ```
    pub fn new() -> Self {
        Self {
            rooms: DashMap::new(),
            deleting: DashSet::new(),
            broadcast_capacity: BROADCAST_CAPACITY,
            resync_floor_enforced: true,
            vfx_rate: Arc::new(PingRateLimiter::new()),
        }
    }

    /// A registry whose rooms use a custom broadcast ring capacity. Test-only: a
    /// tiny capacity lets a non-reading client deterministically overflow the ring
    /// and exercise the `Lagged` → resync path.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::room::RoomRegistry;
    ///
    /// let registry = RoomRegistry::with_capacity(4);
    /// assert!(registry.get(uuid::Uuid::nil()).is_none());
    /// ```
    pub fn with_capacity(broadcast_capacity: usize) -> Self {
        Self {
            rooms: DashMap::new(),
            deleting: DashSet::new(),
            broadcast_capacity,
            resync_floor_enforced: true,
            vfx_rate: Arc::new(PingRateLimiter::new()),
        }
    }

    /// The registry's shared VFX-budget limiter — the same instance every `Room` it creates
    /// charges against for a trigger-fired one-shot. `WsState::new` clones this into
    /// `WsState.vfx_rate` so the raw `PlayVfx` frame and `/fx` share the identical bucket.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::room::RoomRegistry;
    ///
    /// let reg = RoomRegistry::new();
    /// assert!(reg.vfx_rate().check(uuid::Uuid::nil(), 0, 30));
    /// ```
    pub fn vfx_rate(&self) -> Arc<PingRateLimiter> {
        self.vfx_rate.clone()
    }

    /// Get the room for an existing world, creating it (seeded from the world's
    /// current seq) on first join. `None` when the world does not exist or is
    /// mid-deletion (tombstoned).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::ws::room::RoomRegistry;
    /// use uuid::Uuid;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    /// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
    /// let registry = RoomRegistry::new();
    ///
    /// assert!(registry.get_or_create(&repo, world.id).await.unwrap().is_some());
    /// // An absent world hydrates nothing.
    /// assert!(registry.get_or_create(&repo, Uuid::new_v4()).await.unwrap().is_none());
    /// # }
    /// ```
    pub async fn get_or_create(
        &self,
        repo: &dyn Repository,
        world_id: Uuid,
    ) -> Result<Option<Arc<Room>>, DataError> {
        if self.deleting.contains(&world_id) {
            // Mid-deletion: refuse exactly like an absent world row.
            return Ok(None);
        }
        if let Some(r) = self.rooms.get(&world_id) {
            return Ok(Some(r.clone()));
        }
        let Some(world) = repo.get_world(world_id).await? else {
            return Ok(None);
        };
        // Hydrate the derived ECS from persisted scene entities using the
        // same definition as the live path (`is_scene_entity`), so the loader and
        // the predicate cannot drift. Stamp it with the world's current seq.
        let docs = repo.query_scene_entities(world_id).await?;
        let mut scene_ecs = SceneEcs::from_documents(docs, world.seq);
        // Hydrate the lighting-aware vision inputs that are NOT scene entities — the three
        // world config singletons + actors — so the mask computation is pure/synchronous under the
        // scene read-lock. Kept live thereafter by `apply_op`.
        //
        // Safety of the race window between these queries and the `entry()` insert below:
        // a concurrent publish that lands AFTER any of these queries but BEFORE the entry insert
        // is safe — `apply_op` keeps the side-tables current once the room is live, so the
        // built-but-discarded `scene_ecs` from a racing first-joiner is harmless (the winner's
        // `or_insert_with` closure reflects the DB state it queried; the loser's closure is
        // simply never called). There is no window where the live room's side-tables are stale.
        //
        let docs = repo
            .query_documents_by_types(
                world_id,
                &[
                    "world-settings",
                    "light-gradation",
                    "vision-modes",
                    "actor",
                    "system-defaults",
                    "resource-registry",
                    "combat",
                    "faction-registry",
                ],
            )
            .await?;
        let world_settings = docs
            .iter()
            .find(|d| d.doc_type == "world-settings")
            .cloned();
        let gradation = docs
            .iter()
            .find(|d| d.doc_type == "light-gradation")
            .cloned();
        let vision_modes = docs.iter().find(|d| d.doc_type == "vision-modes").cloned();
        let system_defaults = docs
            .iter()
            .find(|d| d.doc_type == "system-defaults")
            .cloned();
        let resource_registry = docs
            .iter()
            .find(|d| d.doc_type == "resource-registry")
            .cloned();
        let faction_registry = docs
            .iter()
            .find(|d| d.doc_type == "faction-registry")
            .cloned();
        let actors: Vec<Document> = docs
            .iter()
            .filter(|d| d.doc_type == "actor")
            .cloned()
            .collect();
        let combats: Vec<Document> = docs
            .into_iter()
            .filter(|d| d.doc_type == "combat")
            .collect();
        scene_ecs.set_world_config(
            world_settings,
            gradation,
            vision_modes,
            system_defaults,
            resource_registry,
            faction_registry,
        );
        scene_ecs.set_actors(actors);
        scene_ecs.set_combats(combats);
        let room = self
            .rooms
            .entry(world_id)
            .or_insert_with(|| {
                Arc::new(Room::new(
                    world_id,
                    world.seq,
                    scene_ecs,
                    self.broadcast_capacity,
                    self.resync_floor_enforced,
                    self.vfx_rate.clone(),
                ))
            })
            .clone();
        // TOCTOU closure, two layers. First, a `begin_delete` still in flight
        // between the tombstone check above and this insert has already
        // removed-and-evicted whatever room it saw — which may not include the
        // one just inserted; the tombstone re-check catches it. Second, a delete
        // that COMPLETED entirely inside that window (begin → commit →
        // finish_delete) has already lifted the tombstone, so the flag proves
        // nothing — re-verify the world ROW. That read is serialized after any
        // committed delete on the single-writer pool, so a vanished row is
        // always observed here. Cold-create cost only; the fast path returned
        // above. Residual: a delete that STARTS after this point removes the
        // just-inserted room via `begin_delete`'s own `rooms.remove` and
        // broadcasts the eviction on it.
        if self.deleting.contains(&world_id) || repo.get_world(world_id).await?.is_none() {
            self.rooms.remove(&world_id);
            return Ok(None);
        }
        Ok(Some(room))
    }

    /// The live room for `world_id`, or `None` if nobody has joined it.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::ws::room::RoomRegistry;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    /// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
    /// let registry = RoomRegistry::new();
    ///
    /// assert!(registry.get(world.id).is_none()); // nobody has joined yet
    /// registry.get_or_create(&repo, world.id).await.unwrap();
    /// assert!(registry.get(world.id).is_some());
    /// # }
    /// ```
    pub fn get(&self, world_id: Uuid) -> Option<Arc<Room>> {
        self.rooms.get(&world_id).map(|r| r.clone())
    }

    /// Telemetry snapshots for every live room (admin debug endpoint).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::ws::room::RoomRegistry;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    /// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
    /// let registry = RoomRegistry::new();
    /// registry.get_or_create(&repo, world.id).await.unwrap();
    ///
    /// let snaps = registry.snapshot();
    /// assert_eq!(snaps.len(), 1);
    /// assert_eq!(snaps[0].world_id, world.id);
    /// # }
    /// ```
    pub fn snapshot(&self) -> Vec<RoomStatsSnapshot> {
        self.rooms.iter().map(|r| r.snapshot()).collect()
    }

    /// Best-effort removal of a room whose last subscriber has left. A racing
    /// re-join re-creates the room seeded from the world's current seq, so a
    /// reaped buffer only forces the rejoining client onto the cold tier.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::ws::room::RoomRegistry;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    /// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
    /// let registry = RoomRegistry::new();
    /// registry.get_or_create(&repo, world.id).await.unwrap();
    ///
    /// // `RoomStats::connections` is 0 by default (no live connection tracked here).
    /// registry.reap_if_empty(world.id);
    /// assert!(registry.get(world.id).is_none());
    /// # }
    /// ```
    pub fn reap_if_empty(&self, world_id: Uuid) {
        self.rooms.remove_if(&world_id, |_, r| {
            r.stats.connections.load(Ordering::Acquire) <= 0
        });
    }

    /// Begin a world deletion: tombstone the world (blocking room re-creation)
    /// and unconditionally remove its live room, returning it so the caller can
    /// broadcast the eviction frame. Every cache the world holds (navmesh,
    /// engine, visible-cells, hecs world, ring) is Room-owned, so dropping the
    /// last Arc frees them all. Pair with `finish_delete` on ALL exit paths.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::ws::room::RoomRegistry;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    /// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
    /// let registry = RoomRegistry::new();
    /// registry.get_or_create(&repo, world.id).await.unwrap();
    ///
    /// let room = registry.begin_delete(world.id);
    /// assert!(room.is_some(), "the live room, for the caller's eviction broadcast");
    /// // Tombstoned: re-creation is refused even though the world row still exists.
    /// assert!(registry.get_or_create(&repo, world.id).await.unwrap().is_none());
    /// # }
    /// ```
    pub fn begin_delete(&self, world_id: Uuid) -> Option<Arc<Room>> {
        self.deleting.insert(world_id);
        self.rooms.remove(&world_id).map(|(_, room)| room)
    }

    /// End a world deletion (success or failure), lifting the tombstone. After
    /// a committed delete, re-creation is refused by the missing world row;
    /// after a failure the world is live again and re-creation is legitimate.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::ws::room::RoomRegistry;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    /// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
    /// let registry = RoomRegistry::new();
    /// registry.get_or_create(&repo, world.id).await.unwrap();
    ///
    /// registry.begin_delete(world.id); // failed delete: the world row survives
    /// registry.finish_delete(world.id);
    /// assert!(registry.get_or_create(&repo, world.id).await.unwrap().is_some());
    /// # }
    /// ```
    pub fn finish_delete(&self, world_id: Uuid) {
        self.deleting.remove(&world_id);
    }

    /// Address every connection of `user` across all live rooms with a terminal
    /// eviction frame (account deletion). Rooms without that user's connections
    /// skip the frame in their egress loops.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::ws::room::RoomRegistry;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    /// let world = repo.create_world_owned("w", gm, 0).await.unwrap();
    /// let registry = RoomRegistry::new();
    /// let room = registry
    ///     .get_or_create(&repo, world.id)
    ///     .await
    ///     .unwrap()
    ///     .unwrap();
    ///
    /// // Out-of-band, like `broadcast_aux`: never bumps `current_seq`.
    /// registry.evict_user(gm);
    /// assert_eq!(room.current_seq(), 0);
    /// # }
    /// ```
    pub fn evict_user(&self, user: Uuid) {
        for entry in self.rooms.iter() {
            entry
                .value()
                .broadcast_aux(ServerMsg::Evicted { user: Some(user) });
        }
    }
}

impl Default for RoomRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod ring_tests;

#[cfg(test)]
mod tests;
