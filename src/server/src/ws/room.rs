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
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::snapshot::StoredCommand;
use crate::data::DataError;
use crate::scene::SceneEcs;
use crate::ws::protocol::{ResyncSource, ServerMsg};

/// The room-facing result of a server-authoritative token move: the stop cell, the legal
/// prefix of the path that was walked (render animation input), the animation duration,
/// the time-tagged position samples, and the per-sample vision trajectory for the mover.
pub(crate) struct MoveExecution {
    /// The scene the moved token actually lives in, derived from the ECS — NOT the scene the
    /// request named. The `MoveStream` frame is stamped with this so its `scene` (which the
    /// per-recipient egress clip and the client's viewed-scene filter both key on) can never
    /// describe a scene other than the one the position was committed to.
    pub scene: Uuid,
    /// The last successfully reached path coordinate (the committed position after the move).
    pub stop: (f64, f64),
    /// Animation duration in milliseconds: the travelled distance converted to grid steps through
    /// the scene shape's `GridShape::world_units_per_cell`, divided by the authored cells-per-second
    /// speed. Zero when `stop == start`.
    pub duration_ms: f64,
    /// Time-tagged position samples for `MoveStream` broadcast playback.
    /// Non-empty; the first sample has `t_ms == 0.0` at the starting position.
    pub samples: Vec<crate::scene::move_stream::PosSamplePt>,
    /// Per-sample vision polygons for the mover (fog-sweep trajectory). `None` for GM movers
    /// (`Unrestricted` — no fog to sweep) and for a zero-progress move (`stop == start`, no
    /// animation regardless of role). Index-aligned with `samples` when `Some`.
    pub mover_vision: Option<Vec<crate::scene::move_stream::VisionSamplePt>>,
    /// Total terrain-weighted cost accumulated over the walked prefix, from
    /// `move_exec::MoveOutcome::cost`. Threaded onto the `MoveStream` wire frame downstream.
    pub cost: f64,
    /// `true` when the move stopped before the requested goal, from
    /// `move_exec::MoveOutcome::truncated`. Threaded onto the `MoveStream` wire frame
    /// downstream, where it is trusted-only (`None` for a clipped observer) for the same
    /// reason as `cost`.
    pub truncated: bool,
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
pub struct RingBuffer {
    /// Buffered frames, ascending seq; every entry is a `RoomEvent::Event`.
    events: VecDeque<RoomEvent>,
}

impl RingBuffer {
    /// An empty buffer.
    ///
    /// # Examples
    ///
    /// ```text
    /// // An empty ring cannot serve any range — the caller falls to the log tier.
    /// assert!(RingBuffer::new().range_from(1).is_none());
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
    /// Per-token moving lock: token → move-end epoch-ms. An entry is expired when
    /// `now_millis() >= end`; expired/absent entries are treated as available (lazy expiry,
    /// no timer). Updated by `execute_move` after a successful commit.
    moving: Mutex<HashMap<Uuid, i64>>,
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
    /// Whether an explicit `ClientMsg::ResyncRequest` is clamped against `resync_floor`
    /// (`ws::conn`'s `Egress::Resync` handler reads this via `Room::resync_floor_enforced`
    /// — the internal `Lagged`-driven auto-resync deliberately does NOT consult it, since
    /// that path replays from a connection's own live-tracked watermark, not an untrusted
    /// client-supplied `from_seq`). `true` for every production `RoomRegistry` constructor:
    /// the client unconditionally sends a cold-start `Hello { last_seq: None }` as the first
    /// frame on every socket open (`WsClient`'s `open()`), so every connection this room ever
    /// sees establishes its floor before it could plausibly send a `ResyncRequest`.
    resync_floor_enforced_flag: bool,
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
            resync_floor_enforced_flag: resync_floor_enforced,
        }
    }

    /// Read access to the derived scene ECS for the per-connection derived
    /// recompute. Writes happen only in `publish` under `publish_guard`.
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
    /// ```text
    /// let seq = room.current_seq(); // compare against a client's last_seq
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
    pub fn broadcast_aux(&self, msg: ServerMsg) {
        let _ = self.tx.send(RoomEvent::Other(std::sync::Arc::new(msg)));
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
            let mut revealed_pending: Vec<(uuid::Uuid, CellSet, CellSet, crate::scene::GridKind)> =
                Vec::new();
            {
                let scene = self.scene.read().await;
                // Memoize the visible mask per (scene, leniency) within this publish so a
                // batch of Creates in the same scene does not recompute the mask per token.
                let mut visible_cache: std::collections::HashMap<
                    (uuid::Uuid, bool),
                    std::collections::BTreeSet<(i32, i32)>,
                > = std::collections::HashMap::new();
                for op in &ops {
                    if let Operation::Update { doc_id, changes } = op {
                        if let Some((_, a0, a1)) = scene.token_move(*doc_id, changes) {
                            if a0 != a1 {
                                return Err(DataError::Forbidden);
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
                        match settings.movement_restriction {
                            crate::scene::MovementRestriction::Unrestricted => {}
                            crate::scene::MovementRestriction::Visible => {
                                let mask =
                                    visible_cache.entry((scene_id, lenient)).or_insert_with(|| {
                                        scene.visible_cells_cached(ctx.user_id, scene_id, lenient)
                                    });
                                if !mask.contains(&target) {
                                    return Err(DataError::Forbidden);
                                }
                            }
                            crate::scene::MovementRestriction::Revealed => {
                                let mask = visible_cache
                                    .entry((scene_id, lenient))
                                    .or_insert_with(|| {
                                        scene.visible_cells_cached(ctx.user_id, scene_id, lenient)
                                    })
                                    .clone();
                                // Explored needs an async fetch, which must not run under the
                                // scene read guard — defer exactly as the movement gate did. The
                                // grid kind is captured here, under the same guard `settings` was
                                // resolved in, since decoding runs after the guard is dropped.
                                revealed_pending.push((
                                    scene_id,
                                    [target].into_iter().collect(),
                                    mask,
                                    settings.grid_kind,
                                ));
                            }
                        }
                    }
                }
            } // scene read guard dropped here — safe to await

            // Memoize the explored blob per scene: a batch of Revealed moves in the same
            // scene (e.g. multi-waypoint) must not issue N DB round-trips. Pattern mirrors
            // visible_cache above. Fail closed: error or missing blob → empty set (visible-only).
            let mut explored_cache: std::collections::HashMap<
                uuid::Uuid,
                crate::scene::explored::ExploredSet,
            > = std::collections::HashMap::new();
            for (scene_id, move_cells, visible, grid_kind) in revealed_pending {
                let explored = match explored_cache.entry(scene_id) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        let set = match repo.get_explored(scene_id, ctx.user_id).await {
                            Ok(Some(blob)) => {
                                crate::scene::explored::ExploredSet::from_bytes(&blob, grid_kind)
                            }
                            _ => crate::scene::explored::ExploredSet::new(),
                        };
                        e.insert(set)
                    }
                };
                // Invariant: `visible` may be corner-sampled (lenient) while `explored` is
                // center-sampled by construction (`ExploredSet::mark_polygons`). The asymmetry only ever ENLARGES
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
        return self.commit_ops_locked(repo, ctx, ops, ts, origin).await;
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
        let stored = repo
            .apply_intent(ctx, self.world_id, ops, ts, origin)
            .await?;
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
    /// movement gate for a token the requester legitimately owns. `Room::publish`'s drag path
    /// has always derived the scene this way; this mirrors it. `scene_id` is additionally
    /// required to agree with the derived scene (defense in depth — it selects nothing).
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
    /// 2. Take `self.scene.read()` inside the guard to resolve restriction/cell/visible_cells/start.
    /// 3. DROP the read guard before any await (no lock across await — mirrors `publish`).
    /// 4. Await `repo.get_explored(...)` for Revealed union (only after the read guard is dropped).
    /// 5. Call the pure `move_exec::execute_move` (lock-free).
    /// 6. Call `commit_ops_locked` — non-reentrant Mutex, guard already held, MUST NOT re-acquire.
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
    /// `moving` maps token → move-end epoch-ms. An absent or expired entry (now >= end) allows
    /// the move. After a successful commit the entry is updated to `now + duration_ms`. Lazy
    /// expiry — no cleanup timer; a fresh server reload has no in-memory lock, consistent with
    /// the atomic-state invariant (the lock is a liveness hint, not durable state).
    pub(crate) async fn execute_move(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        scene_id: Uuid,
        token: Uuid,
        path: Vec<(f64, f64)>,
        ts: i64,
    ) -> Result<MoveExecution, DataError> {
        use crate::scene::{move_exec, MovementRestriction};

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
            if let Some(&end) = moving.get(&token) {
                if now < end {
                    return Err(DataError::Forbidden);
                }
            }
        }

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
        {
            let scene = self.scene.read().await;

            // Resolve the token's OWN scene and its committed position (the `/engine/x,y` band —
            // the sole position source; `/system` never carries position) from one authoritative
            // ECS read. `token_move(token, &[])` is the committed pre-image with no changes
            // applied, which is exactly how `Room::publish` derives the scene of a dragged token —
            // the precedent this mirrors, and why the drag path never had to trust a client scene.
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

            let settings = scene.resolve_scene(token_scene);
            // Captured under this same read guard for the same reason `cell` is: the explored
            // decode below runs after the guard is dropped.
            grid_kind = settings.grid_kind;
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

            let lenient = settings.partial_cell_leniency;
            is_revealed = matches!(restriction, MovementRestriction::Revealed);

            // Pre-compute the visible set off the read lock. For Revealed, this is only the
            // `visible_cells` half; the explored half is fetched after the guard is dropped
            // (explored fetch is async — holding the read lock across it would violate the
            // no-lock-across-await rule).
            visible_cells = if matches!(restriction, MovementRestriction::Unrestricted) {
                std::collections::BTreeSet::new()
            } else {
                scene.visible_cells_cached(ctx.user_id, token_scene, lenient)
            };
        } // scene read guard dropped here — safe to await (publish_guard still held)

        // --- Revealed union: fetch explored AFTER dropping the scene read guard ---
        // INVARIANT: for Revealed the `visible` set passed to execute_move MUST be
        // visible_cells ∪ explored. Fail-closed: error or missing blob → empty explored set
        // (falls back to visible-only, which is stricter but safe).
        let visible = if is_revealed {
            let mut union = visible_cells;
            let explored = match repo.get_explored(token_scene, ctx.user_id).await {
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
        let duration_ms;
        let samples;
        let mover_vision: Option<Vec<crate::scene::move_stream::VisionSamplePt>>;
        {
            let scene = self.scene.read().await;
            outcome = move_exec::execute_move(
                &scene,
                move_exec::MoveGateInputs {
                    scene: token_scene,
                    restriction,
                    visible: &visible,
                    cell,
                },
                token,
                &path,
                is_gm,
                footprint,
            )
            .map_err(|_| DataError::Forbidden)?;
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
            // trajectory, including in Unrestricted-mode scenes. The SAME full sight_walls set
            // is used as for static vision. Hoisting:
            // player_vision_inputs collects walls + static-token polygons ONCE per move; each
            // sample calls polygons_at (one moving-token raycast only, no repeated ECS scan).
            mover_vision = if is_gm {
                None
            } else {
                let vision_inputs = scene.player_vision_inputs(ctx.user_id, token_scene, token);
                Some(
                    samples
                        .iter()
                        .map(|s| crate::scene::move_stream::VisionSamplePt {
                            t_ms: s.t_ms,
                            polygons: vision_inputs.polygons_at(s.pos),
                        })
                        .collect(),
                )
            };
        } // scene read lock dropped — commit_ops_locked awaits safely under publish_guard

        // Zero-progress move (stop == start): return immediately without writing.
        // Invariant: render_path always contains at least `start` (path.len() >= 2 was
        // validated by execute_move), so this only fires when the very first step was blocked.
        if (outcome.stop.0 - start.0).abs() < 1e-9 && (outcome.stop.1 - start.1).abs() < 1e-9 {
            return Ok(MoveExecution {
                scene: token_scene,
                stop: start,
                duration_ms: 0.0,
                samples: vec![crate::scene::move_stream::PosSamplePt {
                    t_ms: 0.0,
                    pos: start,
                }],
                // None regardless of world role: zero-progress has no animation or fog sweep.
                // Deliberate exception to the convention that None signals a GM (Unrestricted)
                // mover — here None signals stop == start, not an Unrestricted restriction.
                mover_vision: None,
                cost: 0.0,
                // Not hardcoded false: this branch is reached only when the very first step
                // was blocked, so the outcome is truncated. Reading it keeps the wire signal
                // derived from the executor rather than restated here.
                truncated: outcome.truncated,
            });
        }

        // --- Atomic commit (publish_guard already held — single acquisition, no re-entry) ---
        // PRECONDITION: commit_ops_locked requires the caller to hold publish_guard for its
        // full duration. The guard was acquired at the top of this function and is still held
        // here — no re-acquisition needed or allowed (tokio Mutex is non-reentrant; re-acquiring
        // would deadlock). The position ops write ONLY `/engine/x,y` — position lives
        // exclusively in the engine band since this task; `/system` is game-system data and is
        // never touched by movement. `old` is keyed on the authoritative ECS-read `start`
        // (`SceneEcs::token_position`, itself `/engine/x,y`) so the optimistic-concurrency check
        // in `apply_intent` passes as defense-in-depth.
        let pos_ops = vec![Operation::Update {
            doc_id: token,
            changes: vec![
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
            ],
        }];

        self.commit_ops_locked(repo, ctx, pos_ops, ts, WriteOrigin::Client)
            .await?;

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
            moving.retain(|_, &mut end| now < end);
            moving.insert(token, now + (duration_ms.ceil() as i64).max(1));
        }

        Ok(MoveExecution {
            scene: token_scene,
            stop: outcome.stop,
            duration_ms,
            samples,
            mover_vision,
            cost: outcome.cost,
            truncated: outcome.truncated,
        })
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
    pub async fn establish_resync_floor(&self, user_id: Uuid) {
        let seq = self.current_seq();
        self.session_floors.lock().await.insert(user_id, seq);
    }

    /// The lowest `from_seq` `user_id` may currently resync from (INCLUSIVE — matches
    /// `ClientMsg::ResyncRequest.from_seq`'s own inclusive semantics). A `user_id` with no
    /// recorded floor (never sent a cold-start `Hello` this room's lifetime) fails closed to
    /// `current_seq() + 1` — an EMPTY resync, not an unbounded one.
    pub async fn resync_floor(&self, user_id: Uuid) -> i64 {
        match self.session_floors.lock().await.get(&user_id) {
            Some(&floor) => floor + 1,
            None => self.current_seq() + 1,
        }
    }

    /// Whether an explicit `ClientMsg::ResyncRequest` should be clamped against
    /// `resync_floor`. See the `resync_floor_enforced_flag` field doc for why this is `true`
    /// for every production constructor.
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
        }
    }

    /// A registry whose rooms use a custom broadcast ring capacity. Test-only: a
    /// tiny capacity lets a non-reading client deterministically overflow the ring
    /// and exercise the `Lagged` → resync path.
    pub fn with_capacity(broadcast_capacity: usize) -> Self {
        Self {
            rooms: DashMap::new(),
            deleting: DashSet::new(),
            broadcast_capacity,
            resync_floor_enforced: true,
        }
    }

    /// Get the room for an existing world, creating it (seeded from the world's
    /// current seq) on first join. `None` when the world does not exist or is
    /// mid-deletion (tombstoned).
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
                &["world-settings", "light-gradation", "vision-modes", "actor"],
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
        let actors: Vec<Document> = docs.into_iter().filter(|d| d.doc_type == "actor").collect();
        scene_ecs.set_world_config(world_settings, gradation, vision_modes);
        scene_ecs.set_actors(actors);
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
                ))
            })
            .clone();
        // TOCTOU closure, two layers. (1) A `begin_delete` still in flight
        // between the tombstone check above and this insert has already
        // removed-and-evicted whatever room it saw — which may not include the
        // one just inserted; the tombstone re-check catches it. (2) A delete
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
    pub fn get(&self, world_id: Uuid) -> Option<Arc<Room>> {
        self.rooms.get(&world_id).map(|r| r.clone())
    }

    /// Telemetry snapshots for every live room (admin debug endpoint).
    pub fn snapshot(&self) -> Vec<RoomStatsSnapshot> {
        self.rooms.iter().map(|r| r.snapshot()).collect()
    }

    /// Best-effort removal of a room whose last subscriber has left. A racing
    /// re-join re-creates the room seeded from the world's current seq, so a
    /// reaped buffer only forces the rejoining client onto the cold tier.
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
    pub fn begin_delete(&self, world_id: Uuid) -> Option<Arc<Room>> {
        self.deleting.insert(world_id);
        self.rooms.remove(&world_id).map(|(_, room)| room)
    }

    /// End a world deletion (success or failure), lifting the tombstone. After
    /// a committed delete, re-creation is refused by the missing world row;
    /// after a failure the world is live again and re-creation is legitimate.
    pub fn finish_delete(&self, world_id: Uuid) {
        self.deleting.remove(&world_id);
    }

    /// Address every connection of `user` across all live rooms with a terminal
    /// eviction frame (account deletion). Rooms without that user's connections
    /// skip the frame in their egress loops.
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
mod room_tests;
