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
    /// The legal prefix of the requested path including `start` through `stop`.
    /// NOT read by the per-recipient egress clip (`clip_move_stream` trims `samples`, never
    /// `render_path`). Kept alive by this struct's own construction (distance/duration and
    /// `sample_path` input, computed inline above) and by a test assertion that the executor's
    /// stop matches the path's last coordinate.
    #[allow(dead_code)]
    pub render_path: Vec<(f64, f64)>,
    /// Animation duration in milliseconds (distance / cell / speed * 1000). Zero when stop == start.
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
    /// Buffered frames, ascending seq; every entry is a `ServerMsg::Event`.
    events: VecDeque<Arc<ServerMsg>>,
}

impl RingBuffer {
    /// An empty buffer.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::ws::room::RingBuffer;
    ///
    /// // An empty ring cannot serve any range — the caller falls to the log tier.
    /// assert!(RingBuffer::new().range_from(1).is_none());
    /// ```
    pub fn new() -> Self {
        Self {
            events: VecDeque::new(),
        }
    }

    /// Append an `Event` frame and prune by count then age.
    pub fn push(&mut self, msg: Arc<ServerMsg>) {
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
    pub fn range_from(&self, from_seq: i64) -> Option<Vec<Arc<ServerMsg>>> {
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
    tx: broadcast::Sender<Arc<ServerMsg>>,
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
}

impl Room {
    /// A room seeded at `seed_seq` with a hydrated scene read-model.
    ///
    /// # Examples
    ///
    /// ```text
    /// RoomRegistry::get_or_create hydrates and constructs rooms; never build one directly.
    /// ```
    fn new(world_id: Uuid, seed_seq: i64, scene: SceneEcs, broadcast_capacity: usize) -> Self {
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
        }
    }

    /// Read access to the derived scene ECS for the per-connection derived
    /// recompute. Writes happen only in `publish` under `publish_guard`.
    pub fn scene(&self) -> &RwLock<SceneEcs> {
        &self.scene
    }

    /// Subscribe to live frames; also returns the room's current seq so a joiner
    /// knows whether it needs to resync.
    pub fn subscribe(&self) -> (broadcast::Receiver<Arc<ServerMsg>>, i64) {
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
    /// also drops when there are no receivers. DELIVERY IS NOT GUARANTEED and an
    /// AssetChanged loss is NOT self-healing — see `Assets`'s `onReplace`.
    pub fn broadcast_aux(&self, msg: ServerMsg) {
        let _ = self.tx.send(std::sync::Arc::new(msg));
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
            let mut revealed_pending: Vec<(uuid::Uuid, CellSet, CellSet)> = Vec::new();
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
                                // scene read guard — defer exactly as the movement gate did.
                                revealed_pending.push((
                                    scene_id,
                                    [target].into_iter().collect(),
                                    mask,
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
            for (scene_id, move_cells, visible) in revealed_pending {
                let explored = match explored_cache.entry(scene_id) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        let set = match repo.get_explored(scene_id, ctx.user_id).await {
                            Ok(Some(blob)) => {
                                crate::scene::explored::ExploredSet::from_bytes(&blob)
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
        let cmd = repo
            .apply_intent(ctx, self.world_id, ops, ts, origin)
            .await?;
        // Hydrate the derived ECS from the committed command while still holding
        // publish_guard (enforced by the caller), so the ECS is consistent with cmd.seq
        // before the Event (and any derived recompute keyed to that seq) is observable.
        {
            let mut scene = self.scene.write().await;
            for op in &cmd.ops {
                scene.apply_op(op);
            }
            // Stamp the seq the ECS now reflects under the same lock, so a
            // derived reader sees a consistent (entities, seq) pair.
            scene.set_committed_seq(cmd.seq);
        }
        let msg = Arc::new(ServerMsg::Event {
            command: cmd.clone(),
            intent_id: None,
        });
        self.ring.lock().await.push(msg.clone());
        self.current_seq.store(cmd.seq, Ordering::Release);
        let _ = self.tx.send(msg); // Err only when there are no receivers
        self.stats.events_published.fetch_add(1, Ordering::Relaxed);
        Ok(cmd)
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
            let Some(fp) = scene.resolve_token_footprint(token) else {
                return Err(DataError::Forbidden);
            };
            footprint = fp;

            let settings = scene.resolve_scene(token_scene);
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
                Ok(Some(blob)) => crate::scene::explored::ExploredSet::from_bytes(&blob),
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
                token_scene,
                token,
                &path,
                restriction,
                &visible,
                cell,
                is_gm,
                footprint,
            )
            .map_err(|_| DataError::Forbidden)?;
            let speed_cells_per_sec = scene.resolved_animation_speed();

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
                (distance / cell) / speed_cells_per_sec * 1000.0
            };

            samples =
                crate::scene::move_stream::sample_path(&outcome.render_path, cell, duration_ms);

            // GM mover → None (no fog to sweep), regardless of restriction mode. Non-GM movers
            // get a per-sample vision polygon at each hypothetical position along the
            // trajectory, including in Unrestricted-mode scenes. The SAME full sight_walls set
            // is used as for static vision. Hoisting:
            // player_vision_inputs collects walls + static-token polygons ONCE per move; each
            // sample calls polygons_at (one moving-token raycast only, no repeated ECS scan).
            mover_vision = if ctx.world_role == crate::data::document::WorldRole::Gm {
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
                render_path: vec![start],
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
            render_path: outcome.render_path,
            duration_ms,
            samples,
            mover_vision,
            cost: outcome.cost,
        })
    }

    /// Resolve a resync range: hot ring tier when fully resident, else the cold
    /// `events_since` tier. Increments the matching telemetry counter.
    pub async fn resync_range(
        &self,
        repo: &dyn Repository,
        from_seq: i64,
    ) -> Result<(Vec<Arc<ServerMsg>>, ResyncSource), DataError> {
        if let Some(hot) = self.ring.lock().await.range_from(from_seq) {
            self.stats.resyncs_hot.fetch_add(1, Ordering::Relaxed);
            return Ok((hot, ResyncSource::Buffer));
        }
        let cmds = repo.events_since(self.world_id, from_seq - 1).await?;
        self.stats.resyncs_cold.fetch_add(1, Ordering::Relaxed);
        let frames = cmds
            .into_iter()
            .map(|c| {
                Arc::new(ServerMsg::Event {
                    command: c,
                    intent_id: None,
                })
            })
            .collect();
        Ok((frames, ResyncSource::Log))
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
}

impl RoomRegistry {
    /// A registry with the production broadcast capacity.
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
mod ring_tests {
    use super::*;
    use crate::data::command::Command;
    use uuid::Uuid;

    fn event(seq: i64, ts: i64) -> Arc<ServerMsg> {
        Arc::new(ServerMsg::Event {
            command: Command {
                seq,
                world_id: Uuid::from_u128(1),
                author: Uuid::from_u128(2),
                ts,
                ops: vec![],
            },
            intent_id: None,
        })
    }

    #[test]
    fn evicts_by_count() {
        let mut rb = RingBuffer::new();
        for s in 1..=(MAX_EVENTS as i64 + 10) {
            rb.push(event(s, 0));
        }
        // Only the newest MAX_EVENTS are retained; oldest resident is seq 11.
        let all = rb.range_from(11).unwrap();
        assert_eq!(all.len(), MAX_EVENTS);
        assert_eq!(all.first().unwrap().event_seq().unwrap(), 11);
        // Seq 1..=10 evicted: a from_seq below the resident floor is not serviceable.
        assert!(rb.range_from(1).is_none());
    }

    #[test]
    fn evicts_by_age_relative_to_newest() {
        let mut rb = RingBuffer::new();
        rb.push(event(1, 0));
        rb.push(event(2, 100));
        rb.push(event(3, MAX_AGE_MS + 1)); // pushes seq 1 (age > MAX) out
        assert!(
            rb.range_from(1).is_none(),
            "seq 1 evicted, range not fully resident"
        );
        let r = rb.range_from(2).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].event_seq().unwrap(), 2);
    }

    #[test]
    fn range_from_returns_suffix_when_resident() {
        let mut rb = RingBuffer::new();
        for s in 1..=5 {
            rb.push(event(s, 0));
        }
        let r = rb.range_from(3).unwrap();
        assert_eq!(
            r.iter().map(|m| m.event_seq().unwrap()).collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
    }

    #[test]
    fn range_from_none_when_requested_seq_evicted() {
        let mut rb = RingBuffer::new();
        for s in 1..=(MAX_EVENTS as i64 + 5) {
            rb.push(event(s, 0));
        }
        // oldest resident is 6; asking from 1 cannot be fully served from buffer.
        assert!(rb.range_from(1).is_none());
    }

    #[test]
    fn range_from_none_on_empty_buffer() {
        let rb = RingBuffer::new();
        assert!(rb.range_from(1).is_none());
    }
}

#[cfg(test)]
mod room_tests {
    use super::*;
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::membership::PermissionContext;
    use crate::data::sqlite::SqliteRepository;
    use std::sync::atomic::Ordering;
    use uuid::Uuid;

    // Dual-write fixture helpers (`ws_engine`/`token_engine`) live in `ws::test_support`,
    // shared with `ws::conn`'s test module.
    use crate::ws::test_support::{token_engine, ws_engine};

    async fn repo_with_world() -> (SqliteRepository, Uuid, PermissionContext) {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let author = repo
            .create_user("a", None, ServerRole::User, 0)
            .await
            .unwrap();
        let world = repo.create_world_owned("W", author, 0).await.unwrap();
        let ctx = PermissionContext {
            user_id: author,
            world_role: WorldRole::Gm,
        };
        (repo, world.id, ctx)
    }

    #[tokio::test]
    async fn begin_delete_tombstones_and_removes() {
        let (repo, world_id, _ctx) = repo_with_world().await;
        let reg = RoomRegistry::new();
        reg.get_or_create(&repo, world_id).await.unwrap().unwrap();

        let room = reg.begin_delete(world_id);
        assert!(room.is_some(), "live room returned for eviction broadcast");
        // The world row still exists — the refusal below is the tombstone's.
        assert!(reg.get_or_create(&repo, world_id).await.unwrap().is_none());

        reg.finish_delete(world_id);
        assert!(reg.get_or_create(&repo, world_id).await.unwrap().is_some());
    }

    /// Delegating repo whose `query_documents_by_types` — the LAST hydration
    /// read in `get_or_create` — performs a COMPLETE world deletion
    /// (begin_delete → delete_world → finish_delete) on its first call: a
    /// delete that starts and finishes entirely inside the hydration window,
    /// after the caller's tombstone check and `get_world` read but before its
    /// registry insert. At re-check time the tombstone is already lifted, so
    /// only a world-existence re-verify can refuse the ghost room.
    struct DeleteMidHydration<'a> {
        inner: &'a SqliteRepository,
        registry: &'a RoomRegistry,
        world: Uuid,
        fired: std::sync::atomic::AtomicBool,
    }

    #[async_trait::async_trait]
    impl Repository for DeleteMidHydration<'_> {
        async fn apply_command(
            &self,
            cmd: crate::data::command::UnsequencedCommand,
        ) -> Result<Command, DataError> {
            self.inner.apply_command(cmd).await
        }
        async fn apply_intent(
            &self,
            ctx: &crate::data::membership::PermissionContext,
            world_id: Uuid,
            ops: Vec<Operation>,
            ts: i64,
            origin: WriteOrigin,
        ) -> Result<Command, DataError> {
            self.inner
                .apply_intent(ctx, world_id, ops, ts, origin)
                .await
        }
        async fn get_document(&self, id: Uuid) -> Result<Option<Document>, DataError> {
            self.inner.get_document(id).await
        }
        async fn effective_owner_of(&self, doc: &Document) -> Result<Option<Uuid>, DataError> {
            self.inner.effective_owner_of(doc).await
        }
        async fn query_documents(
            &self,
            world_id: Uuid,
            doc_type: &str,
        ) -> Result<Vec<Document>, DataError> {
            self.inner.query_documents(world_id, doc_type).await
        }
        async fn query_documents_by_types(
            &self,
            world_id: Uuid,
            doc_types: &[&str],
        ) -> Result<Vec<Document>, DataError> {
            if !self.fired.swap(true, Ordering::SeqCst) {
                self.registry.begin_delete(self.world);
                self.inner.delete_world(self.world).await?;
                self.registry.finish_delete(self.world);
            }
            self.inner
                .query_documents_by_types(world_id, doc_types)
                .await
        }
        async fn query_children(&self, parent: Uuid) -> Result<Vec<Document>, DataError> {
            self.inner.query_children(parent).await
        }
        async fn query_scene_entities(&self, world: Uuid) -> Result<Vec<Document>, DataError> {
            self.inner.query_scene_entities(world).await
        }
        async fn documents_by_source(
            &self,
            pack: Option<&str>,
            source_id: Uuid,
        ) -> Result<Vec<Document>, DataError> {
            self.inner.documents_by_source(pack, source_id).await
        }
        async fn events_since(&self, world_id: Uuid, seq: i64) -> Result<Vec<Command>, DataError> {
            self.inner.events_since(world_id, seq).await
        }
        async fn get_world(
            &self,
            id: Uuid,
        ) -> Result<Option<crate::data::document::World>, DataError> {
            self.inner.get_world(id).await
        }
        async fn member_role(
            &self,
            world: Uuid,
            user: Uuid,
        ) -> Result<Option<WorldRole>, DataError> {
            self.inner.member_role(world, user).await
        }
        async fn member_id_by_username(
            &self,
            world: Uuid,
            username: &str,
        ) -> Result<Option<Uuid>, DataError> {
            self.inner.member_id_by_username(world, username).await
        }
        async fn world_cap_defaults(
            &self,
            world: Uuid,
        ) -> Result<crate::data::document::WorldCapDefaults, DataError> {
            self.inner.world_cap_defaults(world).await
        }
        async fn world_cap_requirements(
            &self,
            world: Uuid,
        ) -> Result<Vec<crate::data::document::CapabilityRequirement>, DataError> {
            self.inner.world_cap_requirements(world).await
        }
        async fn world_contract_declarations(
            &self,
            world: Uuid,
        ) -> Result<Vec<crate::data::document::ContractDeclaration>, DataError> {
            self.inner.world_contract_declarations(world).await
        }
        async fn world_schema_declarations(
            &self,
            world: Uuid,
        ) -> Result<Vec<crate::data::document::SchemaDeclaration>, DataError> {
            self.inner.world_schema_declarations(world).await
        }
        async fn world_enabled_modules(&self, world: Uuid) -> Result<Vec<String>, DataError> {
            self.inner.world_enabled_modules(world).await
        }
        async fn search(
            &self,
            ctx: &crate::data::membership::PermissionContext,
            world_id: Uuid,
            query: &str,
            limit: u32,
            cursor: Option<i64>,
        ) -> Result<crate::data::search::SearchPage, DataError> {
            self.inner.search(ctx, world_id, query, limit, cursor).await
        }
        async fn get_explored(
            &self,
            scene: Uuid,
            user: Uuid,
        ) -> Result<Option<Vec<u8>>, DataError> {
            self.inner.get_explored(scene, user).await
        }
    }

    #[tokio::test]
    async fn get_or_create_refuses_when_delete_completes_mid_hydration() {
        let (repo, world_id, _ctx) = repo_with_world().await;
        let reg = RoomRegistry::new();
        let wrapper = DeleteMidHydration {
            inner: &repo,
            registry: &reg,
            world: world_id,
            fired: std::sync::atomic::AtomicBool::new(false),
        };
        // The deletion completes (tombstone lifted, row gone) while hydration
        // is still in flight; the lifted flag alone proves nothing at re-check
        // time — only the world row's absence can.
        let room = reg.get_or_create(&wrapper, world_id).await.unwrap();
        assert!(room.is_none(), "ghost room registered for a deleted world");
        assert!(reg.get(world_id).is_none());
    }

    #[tokio::test]
    async fn evict_user_reaches_every_room() {
        let (repo, w1, _ctx) = repo_with_world().await;
        let author2 = repo
            .create_user("b", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w2 = repo.create_world_owned("W2", author2, 0).await.unwrap().id;

        let reg = RoomRegistry::new();
        let r1 = reg.get_or_create(&repo, w1).await.unwrap().unwrap();
        let r2 = reg.get_or_create(&repo, w2).await.unwrap().unwrap();
        let (mut rx1, _) = r1.subscribe();
        let (mut rx2, _) = r2.subscribe();

        let target = Uuid::new_v4();
        reg.evict_user(target);

        for rx in [&mut rx1, &mut rx2] {
            match rx.recv().await.unwrap().as_ref() {
                ServerMsg::Evicted { user } => assert_eq!(*user, Some(target)),
                other => panic!("expected Evicted, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn publish_hydrates_scene_ecs() {
        let (repo, world_id, ctx) = repo_with_world().await;
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        assert_eq!(room.scene().read().await.entity_count(), 0);

        // Publish a scene doc (a scene entity by doc_type, no parent FK needed).
        let mut scene =
            crate::data::document::tests::world_scoped_doc(world_id, Uuid::from_u128(20), "scene");
        scene.owner = Some(ctx.user_id);
        room.publish(
            &repo,
            &ctx,
            vec![Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
        assert_eq!(room.scene().read().await.entity_count(), 1);
    }

    /// A `/system/x` write on a token is game-system data — it must not be treated as a move
    /// by `Room::publish`'s movement gate (which reads `/engine` exclusively), and the
    /// write must not desync the ECS's committed `/engine` position. This is the integration-
    /// level counterpart of `scene::mod::tests::token_move_uses_post_image_resisting_forged_
    /// bypasses`'s `/system/x` decoy assertion (same naming-collision decoy: `/system/x` vs.
    /// `/engine/x`), proved end-to-end through `Room::publish` rather than the bare ECS method.
    #[tokio::test]
    async fn system_field_write_bypasses_the_move_gate_and_does_not_desync_the_engine_band() {
        let h = movement_scene_with_wall().await;

        // A `/system/x` + `/system/y` decoy pair targeting `(200,150)`. If the gate mistakenly
        // read these `/system/*` paths as `/engine/x,y` (the naming-collision decoy this test
        // targets), the resulting straight-line move from the committed start `(50,50)` to
        // `(200,150)` crosses `movement_scene_with_wall`'s horizontal wall (y=100, x∈[100,200])
        // at x=125 — well clear of both wall endpoints, no corner-touch ambiguity — and would be
        // rejected. The gate must not even see this write, since it targets `/system`, not
        // `/engine`.
        let write = Operation::Update {
            doc_id: h.token_id,
            changes: vec![
                FieldChange {
                    remove: false,
                    path: "/system/x".into(),
                    old: serde_json::Value::Null, // absent key reads as Null (no `system.x` default)
                    new: serde_json::json!(200.0),
                },
                FieldChange {
                    remove: false,
                    path: "/system/y".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!(150.0),
                },
            ],
        };
        h.room
            .publish(
                &h.repo,
                &h.player,
                vec![write],
                now_millis(),
                WriteOrigin::Client,
            )
            .await
            .expect("a /system write must not be rejected by the movement gate");

        // The engine-band position is untouched by the /system write.
        let pos = h.committed_pos(h.token_id).await;
        assert_eq!(
            pos, h.start,
            "/system write must not move the token's /engine position"
        );
    }

    /// Defense-in-depth: a single `Update`'s FieldChange list combining a wholesale `/engine`
    /// replace AND a leaf `/engine/x` change must produce the SAME post-image whether the
    /// refusal predicate's replay (`SceneEcs::token_move`) or the commit path's replay
    /// (`apply_intent`'s sequential `command::apply_field_change` application) computes it —
    /// in BOTH possible orderings of the two changes. Both replay implementations apply
    /// `changes` via `command::apply_field_change` in array order independently; this pins them
    /// against silently diverging (which would let the predicate judge one post-image while a
    /// different one actually lands). Actor is the GM: this is a real position CHANGE, which the
    /// movement gate refuses for a non-GM outright — the property under test (replay agreement)
    /// is orthogonal to who is writing, and the GM path exercises the identical
    /// `token_move`/commit replay.
    #[tokio::test]
    async fn mixed_wholesale_and_leaf_engine_changes_agree_between_gate_and_commit_in_both_orderings(
    ) {
        use crate::data::command::FieldChange;
        use serde_json::json;

        // The wholesale `old` pre-image must equal the ACTUAL stored `/engine` value, which
        // includes `TokenEngine`'s `#[serde(default)]` `null` fields (visual/actor_id/
        // overrides/face) beyond the `token_engine(50.0, 50.0)` fixture's x/y/w/h/rotation —
        // read it back rather than hand-constructing it, mirroring `mv_to`'s convention.
        async fn stored_engine(h: &MovementHandle) -> serde_json::Value {
            h.repo
                .get_document(h.token_id)
                .await
                .unwrap()
                .unwrap()
                .engine
                .unwrap()
        }
        let wholesale_new = json!({
            "x": 10.0, "y": 10.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
            "visual": null, "actor_id": null, "overrides": null, "face": null
        });

        // Ordering A: wholesale replace, then a leaf x-overwrite. Expected final: (20,10).
        {
            let h = movement_scene_with_wall().await;
            let start_engine = stored_engine(&h).await;
            let changes = vec![
                FieldChange {
                    remove: false,
                    path: "/engine".into(),
                    old: start_engine.clone(),
                    new: wholesale_new.clone(),
                },
                FieldChange {
                    remove: false,
                    path: "/engine/x".into(),
                    old: json!(50.0),
                    new: json!(20.0),
                },
            ];
            let gate_post = {
                let scene = h.room.scene().read().await;
                scene.token_move(h.token_id, &changes).unwrap().2
            };
            h.room
                .publish(
                    &h.repo,
                    &h.gm,
                    vec![Operation::Update {
                        doc_id: h.token_id,
                        changes,
                    }],
                    now_millis(),
                    WriteOrigin::Client,
                )
                .await
                .unwrap();
            let committed = h.committed_pos(h.token_id).await;
            assert_eq!(gate_post, (20.0, 10.0), "ordering A gate post-image");
            assert_eq!(committed, (20.0, 10.0), "ordering A committed post-image");
            assert_eq!(
                gate_post, committed,
                "ordering A: gate and commit post-images must agree"
            );
        }

        // Ordering B: leaf x-overwrite, then wholesale replace. Expected final: (10,10).
        {
            let h = movement_scene_with_wall().await;
            let start_engine = stored_engine(&h).await;
            let changes = vec![
                FieldChange {
                    remove: false,
                    path: "/engine/x".into(),
                    old: json!(50.0),
                    new: json!(20.0),
                },
                FieldChange {
                    remove: false,
                    path: "/engine".into(),
                    old: start_engine.clone(),
                    new: wholesale_new.clone(),
                },
            ];
            let gate_post = {
                let scene = h.room.scene().read().await;
                scene.token_move(h.token_id, &changes).unwrap().2
            };
            h.room
                .publish(
                    &h.repo,
                    &h.gm,
                    vec![Operation::Update {
                        doc_id: h.token_id,
                        changes,
                    }],
                    now_millis(),
                    WriteOrigin::Client,
                )
                .await
                .unwrap();
            let committed = h.committed_pos(h.token_id).await;
            assert_eq!(gate_post, (10.0, 10.0), "ordering B gate post-image");
            assert_eq!(committed, (10.0, 10.0), "ordering B committed post-image");
            assert_eq!(
                gate_post, committed,
                "ordering B: gate and commit post-images must agree"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Non-GM position writes refused; Create placement gated on the same mask
    // -----------------------------------------------------------------------

    /// Shared fixture handle for the non-GM-refusal/Create-gate tests: a room with a scene, an
    /// optional token, and both a GM and a player `PermissionContext`.
    struct PlaceHandle {
        room: Arc<Room>,
        repo: SqliteRepository,
        gm_ctx: PermissionContext,
        player_ctx: PermissionContext,
        token: Uuid,
        world: Uuid,
        scene: Uuid,
    }

    /// A scene with a player-owned token at (50,50). No movement-restriction machinery: the
    /// movement gate refuses a non-GM position CHANGE unconditionally, so these fixtures need
    /// no lighting.
    async fn room_with_player_owned_token() -> PlaceHandle {
        use crate::data::document::DocRole;

        let (repo, world_id, gm_ctx) = repo_with_world().await;
        let p = repo
            .create_user("player", None, ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world_id, p, WorldRole::Player)
            .await
            .unwrap();
        let player_ctx = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let scene_id = Uuid::from_u128(0xD901);
        let token_id = Uuid::from_u128(0xD902);

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm_ctx.user_id);
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut token = wdoc(world_id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.engine = Some(token_engine(50.0, 50.0));
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: token }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        PlaceHandle {
            room,
            repo,
            gm_ctx,
            player_ctx,
            token: token_id,
            world: world_id,
            scene: scene_id,
        }
    }

    /// A scene with a `blocksMove` wall crossing the GM's test move (50,50)->(250,50), and a
    /// token at (50,50). Demonstrates the "GM ignores walls" exemption narratively — the
    /// non-GM-refusal never reaches a GM regardless, so no gate actually consults this wall.
    async fn room_with_gm_and_blocking_wall() -> PlaceHandle {
        use serde_json::json;

        let (repo, world_id, gm_ctx) = repo_with_world().await;
        let p = repo
            .create_user("player", None, ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world_id, p, WorldRole::Player)
            .await
            .unwrap();
        let player_ctx = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let scene_id = Uuid::from_u128(0xD910);
        let token_id = Uuid::from_u128(0xD911);
        let wall_id = Uuid::from_u128(0xD912);

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm_ctx.user_id);
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut token = wdoc(world_id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(gm_ctx.user_id);
        token.engine = Some(token_engine(50.0, 50.0));
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: token }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // Vertical wall at x=150 spanning y∈[-50,50]: the horizontal step (50,50)->(250,50)
        // crosses it.
        let mut wall = wdoc(world_id, wall_id, "wall");
        wall.parent_id = Some(scene_id);
        wall.owner = Some(gm_ctx.user_id);
        wall.engine = Some(
            json!({ "seg": { "x1": 150.0, "y1": -50.0, "x2": 150.0, "y2": 50.0 }, "blocksMove": true }),
        );
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: wall }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        PlaceHandle {
            room,
            repo,
            gm_ctx,
            player_ctx,
            token: token_id,
            world: world_id,
            scene: scene_id,
        }
    }

    /// A scene lit only near (50,50) (movementRestriction="visible"), with `core:create` granted
    /// to Player so a player-authored `Create` reaches the placement gate at all. Asserts its own
    /// mask is non-empty so the two Create tests built on it fail for the right reason.
    async fn room_with_player_create_capability_and_lit_corner() -> PlaceHandle {
        use serde_json::json;

        let (repo, world_id, gm_ctx) = repo_with_world().await;
        let p = repo
            .create_user("player", None, ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world_id, p, WorldRole::Player)
            .await
            .unwrap();
        let player_ctx = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };

        let mut caps = crate::data::document::WorldCapDefaults::default();
        caps.role_caps
            .all
            .entry(WorldRole::Player)
            .or_default()
            .insert("core:create".into());
        repo.set_world_cap_defaults(world_id, &caps).await.unwrap();

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let scene_id = Uuid::from_u128(0xD920);
        let ws_id = Uuid::from_u128(0xD921);
        let light_id = Uuid::from_u128(0xD922);
        let vision_token_id = Uuid::from_u128(0xD923);

        let mut ws = wdoc(world_id, ws_id, "world-settings");
        ws.owner = Some(gm_ctx.user_id);
        ws.system = json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#000000", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        ws.engine = Some(ws_engine(ws.system.clone()));
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm_ctx.user_id);
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // brightRadius=1.0 cell (100 wu) around (50,50): lights cell (0,0) only.
        let mut light = wdoc(world_id, light_id, "light");
        light.parent_id = Some(scene_id);
        light.owner = Some(gm_ctx.user_id);
        light.system = json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 1.0, "dimRadius": 1.0, "enabled": true
        });
        light.engine = Some(light.system.clone());
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: light }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // A vision source: `visible_cells` requires an owned (or observer-tier) token in the
        // scene — without one, `sources` is empty and the mask is unconditionally empty
        // regardless of lighting.
        let mut vision_token = wdoc(world_id, vision_token_id, "token");
        vision_token.parent_id = Some(scene_id);
        vision_token.owner = Some(p);
        vision_token
            .permissions
            .users
            .insert(p, crate::data::document::DocRole::Owner);
        vision_token.engine = Some(token_engine(50.0, 50.0));
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: vision_token }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // Non-vacuity: an empty mask would make the "inside the mask" Create test pass for the
        // wrong reason (Forbidden regardless of placement).
        {
            let scene_ecs = room.scene().read().await;
            let mask = scene_ecs.visible_cells(player_ctx.user_id, scene_id, true);
            assert!(
                !mask.is_empty(),
                "fixture's lit corner must produce a non-empty visible mask"
            );
        }

        PlaceHandle {
            room,
            repo,
            gm_ctx,
            player_ctx,
            token: Uuid::nil(),
            world: world_id,
            scene: scene_id,
        }
    }

    /// Same scene/lighting as `room_with_player_create_capability_and_lit_corner`, without the
    /// `core:create` grant — exercises the GM's unconditional Create placement instead.
    async fn room_with_gm_and_lit_corner() -> PlaceHandle {
        use serde_json::json;

        let (repo, world_id, gm_ctx) = repo_with_world().await;
        let p = repo
            .create_user("player", None, ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world_id, p, WorldRole::Player)
            .await
            .unwrap();
        let player_ctx = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let scene_id = Uuid::from_u128(0xD930);
        let ws_id = Uuid::from_u128(0xD931);
        let light_id = Uuid::from_u128(0xD932);

        let mut ws = wdoc(world_id, ws_id, "world-settings");
        ws.owner = Some(gm_ctx.user_id);
        ws.system = json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#000000", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        ws.engine = Some(ws_engine(ws.system.clone()));
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm_ctx.user_id);
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut light = wdoc(world_id, light_id, "light");
        light.parent_id = Some(scene_id);
        light.owner = Some(gm_ctx.user_id);
        light.system = json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 1.0, "dimRadius": 1.0, "enabled": true
        });
        light.engine = Some(light.system.clone());
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: light }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        PlaceHandle {
            room,
            repo,
            gm_ctx,
            player_ctx,
            token: Uuid::nil(),
            world: world_id,
            scene: scene_id,
        }
    }

    /// Same as `room_with_player_create_capability_and_lit_corner`, but `movementRestriction:
    /// unrestricted` — Create is authorized everywhere regardless of the mask.
    async fn room_with_player_create_and_unrestricted_scene() -> PlaceHandle {
        use serde_json::json;

        let (repo, world_id, gm_ctx) = repo_with_world().await;
        let p = repo
            .create_user("player", None, ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world_id, p, WorldRole::Player)
            .await
            .unwrap();
        let player_ctx = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };

        let mut caps = crate::data::document::WorldCapDefaults::default();
        caps.role_caps
            .all
            .entry(WorldRole::Player)
            .or_default()
            .insert("core:create".into());
        repo.set_world_cap_defaults(world_id, &caps).await.unwrap();

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let scene_id = Uuid::from_u128(0xD940);
        let ws_id = Uuid::from_u128(0xD941);

        let mut ws = wdoc(world_id, ws_id, "world-settings");
        ws.owner = Some(gm_ctx.user_id);
        ws.system = json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": false, "lightMode": "environmentLight",
                "environment": { "color": "#000000", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "unrestricted",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        ws.engine = Some(ws_engine(ws.system.clone()));
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm_ctx.user_id);
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        PlaceHandle {
            room,
            repo,
            gm_ctx,
            player_ctx,
            token: Uuid::nil(),
            world: world_id,
            scene: scene_id,
        }
    }

    /// Same lighting/vision setup as `room_with_player_create_capability_and_lit_corner`, but
    /// `movementRestriction: revealed` — Create is authorized against `visible ∪ explored`,
    /// exercising the Create gate's Revealed branch (`revealed_pending`/`get_explored`), which
    /// is otherwise unreachable through `Operation::Create` in this test module.
    async fn room_with_player_create_capability_and_revealed_corner() -> PlaceHandle {
        use serde_json::json;

        let (repo, world_id, gm_ctx) = repo_with_world().await;
        let p = repo
            .create_user("player", None, ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world_id, p, WorldRole::Player)
            .await
            .unwrap();
        let player_ctx = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };

        let mut caps = crate::data::document::WorldCapDefaults::default();
        caps.role_caps
            .all
            .entry(WorldRole::Player)
            .or_default()
            .insert("core:create".into());
        repo.set_world_cap_defaults(world_id, &caps).await.unwrap();

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let scene_id = Uuid::from_u128(0xD950);
        let ws_id = Uuid::from_u128(0xD951);
        let light_id = Uuid::from_u128(0xD952);
        let vision_token_id = Uuid::from_u128(0xD953);

        let mut ws = wdoc(world_id, ws_id, "world-settings");
        ws.owner = Some(gm_ctx.user_id);
        ws.system = json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#000000", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "revealed",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        ws.engine = Some(ws_engine(ws.system.clone()));
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm_ctx.user_id);
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // brightRadius=1.0 cell (100 wu) around (50,50): lights cell (0,0) only.
        let mut light = wdoc(world_id, light_id, "light");
        light.parent_id = Some(scene_id);
        light.owner = Some(gm_ctx.user_id);
        light.system = json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 1.0, "dimRadius": 1.0, "enabled": true
        });
        light.engine = Some(light.system.clone());
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: light }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // A vision source: `visible_cells` requires an owned (or observer-tier) token in the
        // scene — without one, `sources` is empty and the mask is unconditionally empty
        // regardless of lighting.
        let mut vision_token = wdoc(world_id, vision_token_id, "token");
        vision_token.parent_id = Some(scene_id);
        vision_token.owner = Some(p);
        vision_token
            .permissions
            .users
            .insert(p, crate::data::document::DocRole::Owner);
        vision_token.engine = Some(token_engine(50.0, 50.0));
        room.publish(
            &repo,
            &gm_ctx,
            vec![Operation::Create { doc: vision_token }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // Non-vacuity: an empty visible mask leaves it ambiguous whether the Revealed branch's
        // explored union is doing any work in the tests built on this fixture.
        {
            let scene_ecs = room.scene().read().await;
            let mask = scene_ecs.visible_cells(player_ctx.user_id, scene_id, true);
            assert!(
                !mask.is_empty(),
                "fixture's lit corner must produce a non-empty visible mask"
            );
        }

        PlaceHandle {
            room,
            repo,
            gm_ctx,
            player_ctx,
            token: Uuid::nil(),
            world: world_id,
            scene: scene_id,
        }
    }

    /// A token `Document` at `(x, y)` in `world`, parented to `scene`, ready for
    /// `Operation::Create`. `permissions.default = Owner` carries the WRITE_FIELDS floor for
    /// WHICHEVER user creates it (player or GM) — this suite tests the placement mask, not
    /// per-user document ownership, so `core:create` (from the fixture's world-cap grant, or
    /// the GM's unconditional access) is the only authorization axis in play here.
    fn token_doc_at(world: Uuid, scene: Uuid, x: f64, y: f64) -> Document {
        use crate::data::document::DocRole;
        let mut doc =
            crate::data::document::tests::world_scoped_doc(world, Uuid::new_v4(), "token");
        doc.parent_id = Some(scene);
        doc.permissions.default = DocRole::Owner;
        doc.engine = Some(token_engine(x, y));
        doc
    }

    #[tokio::test]
    async fn non_gm_token_position_update_is_refused() {
        use serde_json::json;
        let h = room_with_player_owned_token().await;
        let ops = vec![Operation::Update {
            doc_id: h.token,
            changes: vec![
                FieldChange {
                    path: "/engine/x".into(),
                    old: json!(50.0),
                    new: json!(150.0),
                    remove: false,
                },
                FieldChange {
                    path: "/engine/y".into(),
                    old: json!(50.0),
                    new: json!(50.0),
                    remove: false,
                },
            ],
        }];
        let err = h
            .room
            .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
            .await
            .expect_err("a player may not write a token position");
        assert!(
            matches!(err, DataError::Forbidden),
            "refused as Forbidden, got {err:?}"
        );
    }

    #[tokio::test]
    async fn gm_token_position_update_still_succeeds_through_a_wall() {
        use serde_json::json;
        // A GM places a token where they like, walls included.
        let h = room_with_gm_and_blocking_wall().await;
        let ops = vec![Operation::Update {
            doc_id: h.token,
            changes: vec![FieldChange {
                path: "/engine/x".into(),
                old: json!(50.0),
                new: json!(250.0),
                remove: false,
            }],
        }];
        h.room
            .publish(&h.repo, &h.gm_ctx, ops, 0, WriteOrigin::Client)
            .await
            .expect("a GM position write is unconditional");
    }

    #[tokio::test]
    async fn non_gm_wholesale_engine_write_that_moves_a_token_is_refused() {
        // Post-image detection: `token_move` applies all changes in array order over the
        // committed /engine band, so replacing the whole band cannot smuggle a position change
        // past a per-path check.
        use serde_json::json;
        let h = room_with_player_owned_token().await;
        let ops = vec![Operation::Update {
            doc_id: h.token,
            changes: vec![FieldChange {
                path: "/engine".into(),
                old: json!({"x": 50.0, "y": 50.0}),
                new: json!({"x": 150.0, "y": 50.0}),
                remove: false,
            }],
        }];
        let err = h
            .room
            .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
            .await
            .expect_err("a wholesale engine write is caught");
        assert!(matches!(err, DataError::Forbidden));
    }

    #[tokio::test]
    async fn non_gm_non_position_token_update_still_succeeds() {
        // The refusal is scoped to POSITION CHANGE, not to token writes generally. `token_move`
        // returns Some for any token with readable x/y, so an `.is_some()` predicate would refuse
        // this — the pre/post comparison is what makes this test pass.
        use serde_json::json;
        let h = room_with_player_owned_token().await;
        let ops = vec![Operation::Update {
            doc_id: h.token,
            changes: vec![FieldChange {
                path: "/engine/rotation".into(),
                old: json!(0.0),
                new: json!(90.0),
                remove: false,
            }],
        }];
        h.room
            .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
            .await
            .expect("a player may still rotate a token they own");
    }

    #[tokio::test]
    async fn non_gm_engine_write_leaving_position_unchanged_succeeds() {
        // The boundary of the pre/post comparison: an /engine write that re-states the SAME x,y
        // is not a move and must be allowed.
        use serde_json::json;
        let h = room_with_player_owned_token().await;
        let ops = vec![Operation::Update {
            doc_id: h.token,
            changes: vec![FieldChange {
                path: "/engine/x".into(),
                old: json!(50.0),
                new: json!(50.0),
                remove: false,
            }],
        }];
        h.room
            .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
            .await
            .expect("a no-op position write is not a move");
    }

    #[tokio::test]
    async fn non_gm_token_create_outside_the_mask_is_refused() {
        let h = room_with_player_create_capability_and_lit_corner().await;
        let ops = vec![Operation::Create {
            doc: token_doc_at(h.world, h.scene, 500.0, 500.0),
        }];
        let err = h
            .room
            .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
            .await
            .expect_err("placement in fog is refused");
        assert!(matches!(err, DataError::Forbidden));
    }

    #[tokio::test]
    async fn non_gm_token_create_inside_the_mask_succeeds() {
        let h = room_with_player_create_capability_and_lit_corner().await;
        let ops = vec![Operation::Create {
            doc: token_doc_at(h.world, h.scene, 50.0, 50.0),
        }];
        h.room
            .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
            .await
            .expect("placement in a visible cell is allowed");
    }

    #[tokio::test]
    async fn non_gm_token_create_in_explored_but_unlit_cell_succeeds() {
        // Guards the Create gate's Revealed branch (the `revealed_pending` consumption
        // loop): a cell that is explored-but-not-currently-visible must still admit a player
        // Create, mirroring `execute_move_revealed_union_allows_explored_cell`'s movement-side
        // assertion of the same `visible ∪ explored` contract.
        let h = room_with_player_create_capability_and_revealed_corner().await;
        let cell = 100.0_f64;

        // Target (550,550) = cell (5,5): outside the fixture's lit corner (only (0,0) is lit).
        let mut seed = crate::scene::explored::ExploredSet::new();
        seed.mark_polygons(
            &[vec![
                0.0,
                0.0,
                6.0 * cell,
                0.0,
                6.0 * cell,
                6.0 * cell,
                0.0,
                6.0 * cell,
            ]],
            &crate::scene::grid_shape::SquareGrid {
                cell,
                rule: crate::scene::pathfinding::DiagonalRule::Chebyshev,
            },
            cell,
        );
        h.repo
            .set_explored(h.world, h.scene, h.player_ctx.user_id, &seed.to_bytes())
            .await
            .unwrap();

        let ops = vec![Operation::Create {
            doc: token_doc_at(h.world, h.scene, 550.0, 550.0),
        }];
        h.room
            .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
            .await
            .expect("placement in an explored-but-unlit cell is allowed under Revealed");
    }

    #[tokio::test]
    async fn non_gm_token_create_outside_visible_and_explored_is_refused() {
        let h = room_with_player_create_capability_and_revealed_corner().await;
        // No explored blob seeded: the target cell is neither visible nor explored.
        let ops = vec![Operation::Create {
            doc: token_doc_at(h.world, h.scene, 900.0, 900.0),
        }];
        let err = h
            .room
            .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
            .await
            .expect_err("placement outside visible ∪ explored is refused");
        assert!(matches!(err, DataError::Forbidden));
    }

    #[tokio::test]
    async fn gm_token_create_anywhere_succeeds() {
        let h = room_with_gm_and_lit_corner().await;
        let ops = vec![Operation::Create {
            doc: token_doc_at(h.world, h.scene, 500.0, 500.0),
        }];
        h.room
            .publish(&h.repo, &h.gm_ctx, ops, 0, WriteOrigin::Client)
            .await
            .expect("a GM places a token anywhere");
    }

    #[tokio::test]
    async fn unrestricted_scene_ungates_non_gm_token_create() {
        let h = room_with_player_create_and_unrestricted_scene().await;
        let ops = vec![Operation::Create {
            doc: token_doc_at(h.world, h.scene, 500.0, 500.0),
        }];
        h.room
            .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
            .await
            .expect("Unrestricted ungates placement, as it ungates movement");
    }

    #[tokio::test]
    async fn get_or_create_hydrates_config_and_actors_from_db() {
        use crate::data::document::DocRole;
        use serde_json::json;
        let (repo, world_id, gm) = repo_with_world().await;
        let p = repo
            .create_user("p", None, ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world_id, p, WorldRole::Player)
            .await
            .unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let (scene_id, token_id, light_id, ws_id) = (
            Uuid::from_u128(10),
            Uuid::from_u128(11),
            Uuid::from_u128(12),
            Uuid::from_u128(13),
        );

        // First registry: publish (→ DB) world-settings + scene + player-owned token + an enabled
        // light at the token cell. These writes go through apply_op on reg1's room, committing to
        // the DB. The second registry never sees any of these live publishes.
        let reg1 = RoomRegistry::new();
        let room1 = reg1.get_or_create(&repo, world_id).await.unwrap().unwrap();

        let mut ws = wdoc(world_id, ws_id, "world-settings");
        ws.owner = Some(gm.user_id);
        ws.system = json!({
            "scene": { "losRestriction": true, "fog": true, "lightingEnabled": true,
                       "lightMode": "environmentLight", "environment": {"color":"#0a0e1a","intensity":0.0},
                       "observerVision": false, "movementRestriction": "visible", "partialCellLeniency": true },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" } });
        ws.engine = Some(ws_engine(ws.system.clone()));
        room1
            .publish(
                &repo,
                &gm,
                vec![Operation::Create { doc: ws }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm.user_id);
        scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
        room1
            .publish(
                &repo,
                &gm,
                vec![Operation::Create { doc: scene }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();

        let mut token = wdoc(world_id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.engine = Some(token_engine(50.0, 50.0));
        room1
            .publish(
                &repo,
                &gm,
                vec![Operation::Create { doc: token }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();

        let mut light = wdoc(world_id, light_id, "light");
        light.parent_id = Some(scene_id);
        light.owner = Some(gm.user_id);
        light.system = json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true
        });
        light.engine = Some(light.system.clone());
        room1
            .publish(
                &repo,
                &gm,
                vec![Operation::Create { doc: light }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();

        // A FRESH registry never saw the live publishes: a non-empty mask here proves
        // get_or_create hydrated the config-docs + scene/token/light from the DB (NOT the
        // apply_op live path). If the four query_documents hydration calls are removed from
        // get_or_create, world_settings_doc() returns None and the player_lit_mask uses
        // fail-closed defaults with env_intensity 0.0 + no world-settings structural guard,
        // meaning resolve_scene has no world-settings layer — but the light is still a scene
        // entity so it IS hydrated via from_documents. What the hydration calls specifically
        // prove is that the world-settings doc is present on the cold-start room, confirming
        // the config-doc queries ran. The mask non-emptiness proves the full chain end-to-end
        // (world-settings resolved + scene entity light + player token all loaded from DB).
        let reg2 = RoomRegistry::new();
        let room2 = reg2.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let ecs = room2.scene().read().await;
        assert!(
            ecs.world_settings_doc().is_some(),
            "world-settings hydrated from DB by get_or_create"
        );
        let mask = ecs.player_lit_mask(p);
        assert!(
            mask.iter().any(|s| !s.cells.is_empty()),
            "player lit mask non-empty after cold-start hydration (config + token + light from DB)"
        );
    }

    #[tokio::test]
    async fn get_or_create_batched_query_handles_partial_doc_type_presence() {
        use serde_json::json;
        let (repo, world_id, gm) = repo_with_world().await;
        let wdoc = crate::data::document::tests::world_scoped_doc;

        let reg1 = RoomRegistry::new();
        let room1 = reg1.get_or_create(&repo, world_id).await.unwrap().unwrap();

        // Only actor + world-settings exist; light-gradation/vision-modes absent.
        let actor_id = Uuid::from_u128(30);
        let mut actor = wdoc(world_id, actor_id, "actor");
        actor.owner = Some(gm.user_id);
        actor.engine = Some(json!({
            "displayName": "Fixture Actor",
            "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": 1.0, "h": 1.0 },
            "shape": "square",
            "conditions": [],
            "prototype": true,
            "vision": [],
        }));
        room1
            .publish(
                &repo,
                &gm,
                vec![Operation::Create { doc: actor }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();

        let ws_id = Uuid::from_u128(31);
        let mut ws = wdoc(world_id, ws_id, "world-settings");
        ws.owner = Some(gm.user_id);
        ws.system = json!({
            "scene": { "losRestriction": true, "fog": true, "lightingEnabled": true,
                       "lightMode": "environmentLight", "environment": {"color":"#0a0e1a","intensity":0.0},
                       "observerVision": false, "movementRestriction": "visible", "partialCellLeniency": true },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" } });
        ws.engine = Some(ws_engine(ws.system.clone()));
        room1
            .publish(
                &repo,
                &gm,
                vec![Operation::Create { doc: ws }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();

        // A FRESH registry never saw the live publishes: get_or_create must hydrate
        // world-settings + actor from the DB even though light-gradation/vision-modes
        // are absent for this world — proving the batched query resolves each doc_type
        // independently rather than requiring all four to be present.
        let reg2 = RoomRegistry::new();
        let room2 = reg2.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let ecs = room2.scene().read().await;

        assert!(
            ecs.world_settings_doc().is_some(),
            "world-settings must hydrate independently"
        );
        assert!(
            ecs.actor(&actor_id).is_some(),
            "actor must hydrate independently"
        );
        assert!(
            ecs.gradation_doc().is_none(),
            "absent light-gradation must not error or block others"
        );
        assert!(
            ecs.vision_modes_doc().is_none(),
            "absent vision-modes must not error or block others"
        );
    }

    #[tokio::test]
    async fn publish_allocates_seq_buffers_and_broadcasts() {
        let (repo, world_id, ctx) = repo_with_world().await;
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let (mut rx, current) = room.subscribe();
        assert_eq!(current, 0);

        let cmd = room
            .publish(&repo, &ctx, vec![], 10, WriteOrigin::Client)
            .await
            .unwrap();
        assert_eq!(cmd.seq, 1);
        assert_eq!(room.current_seq(), 1);

        let got = rx.recv().await.unwrap();
        assert_eq!(got.event_seq(), Some(1));
        assert_eq!(room.stats.events_published.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn get_or_create_returns_none_for_missing_world() {
        let (repo, _world_id, _ctx) = repo_with_world().await;
        let reg = RoomRegistry::new();
        assert!(reg
            .get_or_create(&repo, Uuid::from_u128(999))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn resync_hot_then_cold_tiers() {
        let (repo, world_id, ctx) = repo_with_world().await;
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        for _ in 0..3 {
            room.publish(&repo, &ctx, vec![], 0, WriteOrigin::Client)
                .await
                .unwrap();
        } // seq 1,2,3

        // hot: from_seq 2 resident in buffer
        let (hot, src) = room.resync_range(&repo, 2).await.unwrap();
        assert_eq!(src, ResyncSource::Buffer);
        assert_eq!(
            hot.iter()
                .map(|m| m.event_seq().unwrap())
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
    }

    #[tokio::test]
    async fn publish_is_ordered_under_concurrency() {
        let (repo, world_id, ctx) = repo_with_world().await;
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let (mut rx, _) = room.subscribe();

        let repo = std::sync::Arc::new(repo);
        let mut handles = vec![];
        for _ in 0..50 {
            let room = room.clone();
            let repo = repo.clone();
            handles.push(tokio::spawn(async move {
                room.publish(repo.as_ref(), &ctx, vec![], 0, WriteOrigin::Client)
                    .await
                    .unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let mut seqs = vec![];
        for _ in 0..50 {
            seqs.push(rx.recv().await.unwrap().event_seq().unwrap());
        }
        let mut sorted = seqs.clone();
        sorted.sort();
        assert_eq!(
            seqs, sorted,
            "broadcast delivery order must equal seq order"
        );
        assert_eq!(seqs, (1..=50).collect::<Vec<_>>());
    }

    // -----------------------------------------------------------------------
    // Movement-restriction gate
    // -----------------------------------------------------------------------

    struct MovementHandle {
        room: Arc<Room>,
        repo: SqliteRepository,
        gm: PermissionContext,
        player: PermissionContext,
        world_id: Uuid,
        scene_id: Uuid,
        token_id: Uuid,
        /// Committed start position of the primary token (scene-unit coords).
        start: (f64, f64),
        /// A lit cell reachable from `start` without crossing any wall.
        lit_goal: (f64, f64),
        /// An adjacent (king-step) cell reachable from `start` (unrestricted/visible scenes).
        adj: (f64, f64),
        /// A cell adjacent to `adj`, used as the second leg in moving-lock tests.
        adj2: (f64, f64),
    }

    impl MovementHandle {
        /// Read the committed position of `token` from the authoritative ECS.
        async fn committed_pos(&self, token: Uuid) -> (f64, f64) {
            self.room
                .scene()
                .read()
                .await
                .token_position(token)
                .expect("token not found in ECS")
        }
    }

    /// Publish world-settings with `movementRestriction`, a scene (grid 100), a
    /// player-owned token at (50,50), and optionally a white point light at (50,50)
    /// with brightRadius=1.5, dimRadius=3.0. Env intensity=0 so only the placed
    /// light illuminates (cells beyond ~1.5 cell-radii are dark).
    async fn movement_scene(restriction: &str, with_light: bool) -> MovementHandle {
        movement_scene_with_speed(restriction, with_light, 6.0).await
    }

    /// `movement_scene`, with the world's animation speed (cells/sec) under test control.
    ///
    /// The per-token moving lock's end epoch is derived as `distance / speed`, and
    /// `Room::execute_move` checks it against its OWN internal `ws::time::now_millis()` — not the
    /// `now` argument — so a test cannot hold the lock open by pinning the clock. At the default 6
    /// cells/sec a one-cell move locks for only ~167 ms, which a loaded machine can outrun between
    /// two awaits. A test asserting lock-held behavior must therefore choose a speed slow enough
    /// that the window cannot close under any plausible scheduling delay.
    async fn movement_scene_with_speed(
        restriction: &str,
        with_light: bool,
        speed_cells_per_sec: f64,
    ) -> MovementHandle {
        use crate::data::document::DocRole;
        use serde_json::json;

        let (repo, world_id, gm) = repo_with_world().await;
        let p = repo
            .create_user("player", None, crate::auth::role::ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world_id, p, WorldRole::Player)
            .await
            .unwrap();
        let player = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let (scene_id, token_id, ws_id, light_id) = (
            Uuid::from_u128(0x5CE0),
            Uuid::from_u128(0x5CE1),
            Uuid::from_u128(0x5CE2),
            Uuid::from_u128(0x5CE3),
        );

        let mut ws = wdoc(world_id, ws_id, "world-settings");
        ws.owner = Some(gm.user_id);
        ws.system = json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#000000", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": restriction,
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": speed_cells_per_sec, "easing": "easeInOut" }
        });
        ws.engine = Some(ws_engine(ws.system.clone()));
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm.user_id);
        scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut token = wdoc(world_id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.engine = Some(token_engine(50.0, 50.0));
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: token }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        if with_light {
            // Bright boundary = 1.5 * 100 = 150 world units from (50,50).
            // Cell (0,0) center=(50,50): dist=0 → lit. Cell (20,20) center=(2050,2050): dark.
            let mut light = wdoc(world_id, light_id, "light");
            light.parent_id = Some(scene_id);
            light.owner = Some(gm.user_id);
            light.system = json!({
                "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
                "brightRadius": 1.5, "dimRadius": 3.0, "enabled": true
            });
            light.engine = Some(light.system.clone());
            room.publish(
                &repo,
                &gm,
                vec![Operation::Create { doc: light }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
        }

        MovementHandle {
            room,
            repo,
            gm,
            player,
            world_id,
            scene_id,
            token_id,
            // Token starts at (50,50) — center of cell (0,0) with grid size 100.
            start: (50.0, 50.0),
            // Cell (0,0) is illuminated by the light at (50,50); (0,0) center=(50,50) → lit.
            // For unrestricted/no-light scenes this field is still a reachable adjacent cell.
            lit_goal: (50.0, 150.0),
            // Adjacent cell: one king-step from (50,50).
            adj: (150.0, 50.0),
            // Two king-steps from start: used as the second leg in moving-lock tests.
            adj2: (250.0, 50.0),
        }
    }

    // -----------------------------------------------------------------------
    // commit_ops_locked direct test — gate-free authoritative write path
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn commit_ops_writes_and_broadcasts_without_gating() {
        let (repo, world_id, ctx) = repo_with_world().await;
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let (mut rx, current) = room.subscribe();
        assert_eq!(current, 0);

        // Build a real create op — mirrors publish_hydrates_scene_ecs exactly so this
        // test exercises the ECS apply_op write path and commits a real document row,
        // not just the seq-bump + broadcast path.
        let mut scene =
            crate::data::document::tests::world_scoped_doc(world_id, Uuid::from_u128(20), "scene");
        scene.owner = Some(ctx.user_id);
        let op = Operation::Create { doc: scene };

        // Acquire the guard here, mirroring the single-acquisition discipline: the caller
        // (publish or execute_move) holds the guard, then calls commit_ops_locked.
        // Invariant: commit_ops_locked MUST NOT re-acquire publish_guard (deadlock).
        let _guard = room.publish_guard.lock().await;
        let cmd = room
            .commit_ops_locked(&repo, &ctx, vec![op], 10, WriteOrigin::Client)
            .await
            .unwrap();
        drop(_guard);

        assert_eq!(cmd.seq, 1);
        assert_eq!(room.current_seq(), cmd.seq);
        assert_eq!(room.stats.events_published.load(Ordering::Relaxed), 1);
        assert!(matches!(
            &*rx.recv().await.unwrap(),
            ServerMsg::Event { .. }
        ));
        // Verify the create op landed: cmd carries the committed op and the ECS reflects it.
        assert!(
            !cmd.ops.is_empty(),
            "committed command must carry the create op"
        );
        assert_eq!(
            room.scene().read().await.entity_count(),
            1,
            "ECS must reflect the committed scene entity"
        );
    }

    // -----------------------------------------------------------------------
    // Room::execute_move — server-authoritative atomic move + moving lock
    // -----------------------------------------------------------------------

    /// Scene with token at (50,50), a wall that blocks the step from `corner` to
    /// `beyond_wall`, and movementRestriction="unrestricted" so only the wall gate fires.
    ///
    /// Geometry (grid size=100):
    ///   - start       = (50,50)  — token committed position (center of cell 0,0)
    ///   - corner      = (150,50) — one king-step right; clear (no wall on this path)
    ///   - beyond_wall = (150,150) — one king-step down from corner; a horizontal wall
    ///     at y=100 (x ∈ [100,200]) blocks the step corner→beyond_wall.
    ///
    /// Wall: x1=100,y1=100,x2=200,y2=100. Step (150,50)→(150,150): vertical at x=150
    /// crosses y=100 — blocked.
    async fn movement_scene_with_wall() -> MovementHandle {
        use crate::data::document::DocRole;
        use serde_json::json;

        let (repo, world_id, gm) = repo_with_world().await;
        let p = repo
            .create_user("player_wall", None, crate::auth::role::ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world_id, p, WorldRole::Player)
            .await
            .unwrap();
        let player = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let (scene_id, token_id, ws_id, wall_id) = (
            Uuid::from_u128(0xFA11_0001),
            Uuid::from_u128(0xFA11_0002),
            Uuid::from_u128(0xFA11_0003),
            Uuid::from_u128(0xFA11_0004),
        );

        // Unrestricted: only the wall gate applies, no lighting or mask required.
        let mut ws = wdoc(world_id, ws_id, "world-settings");
        ws.owner = Some(gm.user_id);
        ws.system = json!({
            "scene": {
                "losRestriction": false, "fog": false,
                "lightingEnabled": false, "lightMode": "environmentLight",
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": "unrestricted",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        ws.engine = Some(ws_engine(ws.system.clone()));
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm.user_id);
        scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut token = wdoc(world_id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.engine = Some(token_engine(50.0, 50.0));
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: token }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // Horizontal wall at y=100, x ∈ [100,200]. Blocks vertical step (150,50)→(150,150).
        let mut wall = wdoc(world_id, wall_id, "wall");
        wall.parent_id = Some(scene_id);
        wall.owner = Some(gm.user_id);
        wall.engine = Some(
            json!({ "seg": { "x1": 100, "y1": 100, "x2": 200, "y2": 100 }, "blocksMove": true }),
        );
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: wall }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        MovementHandle {
            room,
            repo,
            gm,
            player,
            world_id,
            scene_id,
            token_id,
            start: (50.0, 50.0),
            // clear one-step right; used as `lit_goal` and `adj` (corner)
            lit_goal: (150.0, 50.0),
            adj: (150.0, 50.0),
            // wall blocks the step adj→adj2 (beyond wall)
            adj2: (150.0, 150.0),
        }
    }

    /// Current epoch milliseconds for test timestamps.
    fn now_millis() -> i64 {
        crate::ws::time::now_millis()
    }

    #[tokio::test]
    async fn execute_move_commits_stop_and_returns_render_path() {
        // "visible" restriction with a light: start (50,50) and the adjacent cell (50,150)
        // are both within the bright radius (1.5 cells), so the player move is allowed.
        // The committed ECS position must equal the returned stop.
        let h = movement_scene("visible", /*with_light=*/ true).await;
        let res = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                h.scene_id,
                h.token_id,
                vec![h.start, h.lit_goal],
                now_millis(),
            )
            .await
            .unwrap();
        assert_eq!(res.render_path.last().copied(), Some(res.stop));
        // Committed ECS position must equal stop (atomic write invariant).
        assert_eq!(h.committed_pos(h.token_id).await, res.stop);
    }

    /// `movement_scene("visible", true)` — one player token in scene A, lit only near the
    /// origin — plus a SECOND scene B in the same world, for exercising a `MoveRequest` that
    /// names a scene the moved token does not live in.
    ///
    /// `b_unrestricted`: B carries a per-scene `movementRestriction: "unrestricted"` override,
    /// so gating against B skips the visibility mask entirely.
    /// `b_lit_token`: the player also owns a token in B under a wide light, so B's mask
    /// authorizes scene-local coordinates that are dark (and therefore unauthorized) in A.
    ///
    /// Returns the handle for scene A and B's id.
    async fn movement_scene_with_second_scene(
        b_unrestricted: bool,
        b_lit_token: bool,
    ) -> (MovementHandle, Uuid) {
        use crate::data::document::DocRole;
        use serde_json::json;

        let h = movement_scene("visible", true).await;
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let (scene_b, token_b, light_b) = (
            Uuid::from_u128(0x5CEB_0001),
            Uuid::from_u128(0x5CEB_0002),
            Uuid::from_u128(0x5CEB_0003),
        );

        let mut scene = wdoc(h.world_id, scene_b, "scene");
        scene.owner = Some(h.gm.user_id);
        scene.engine = Some(if b_unrestricted {
            json!({
                "grid": { "kind": "square", "size": 100 },
                "vision": { "movementRestriction": "unrestricted" }
            })
        } else {
            json!({ "grid": { "kind": "square", "size": 100 } })
        });
        h.room
            .publish(
                &h.repo,
                &h.gm,
                vec![Operation::Create { doc: scene }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();

        if b_lit_token {
            // Vision source in B so the player has an LOS polygon there at all.
            let mut token = wdoc(h.world_id, token_b, "token");
            token.parent_id = Some(scene_b);
            token.owner = Some(h.player.user_id);
            token
                .permissions
                .users
                .insert(h.player.user_id, DocRole::Owner);
            token.engine = Some(token_engine(250.0, 50.0));
            h.room
                .publish(
                    &h.repo,
                    &h.gm,
                    vec![Operation::Create { doc: token }],
                    0,
                    WriteOrigin::Client,
                )
                .await
                .unwrap();

            // Dim boundary = 6 cells = 600 units from (250,50): cells (0,0)..(8,0) are lit in B,
            // whereas A's light (bright 1.5 / dim 3.0 from (50,50)) leaves cell (4,0) dark.
            let mut light = wdoc(h.world_id, light_b, "light");
            light.parent_id = Some(scene_b);
            light.owner = Some(h.gm.user_id);
            light.system = json!({
                "x": 250.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
                "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true
            });
            light.engine = Some(light.system.clone());
            h.room
                .publish(
                    &h.repo,
                    &h.gm,
                    vec![Operation::Create { doc: light }],
                    0,
                    WriteOrigin::Client,
                )
                .await
                .unwrap();
        }

        (h, scene_b)
    }

    #[tokio::test]
    async fn execute_move_refuses_a_scene_id_the_token_does_not_live_in_unrestricted() {
        // Cross-scene gate substitution: the token lives in A (movementRestriction "visible",
        // lit only near the origin) but the request names B, which is "unrestricted". Gating
        // against B would skip the mask entirely and teleport the token 20 cells across A's fog.
        let (h, scene_b) = movement_scene_with_second_scene(true, false).await;
        let far_dark = (2050.0, 2050.0);
        let res = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                scene_b,
                h.token_id,
                vec![h.start, far_dark],
                now_millis(),
            )
            .await;
        assert!(
            matches!(res, Err(DataError::Forbidden)),
            "a MoveRequest naming a scene the token does not live in must be refused by the gate — not incidentally by the moving lock or a downstream write"
        );
        assert_eq!(
            h.committed_pos(h.token_id).await,
            h.start,
            "the refused move must not have committed a position"
        );
    }

    #[tokio::test]
    async fn execute_move_refuses_a_scene_id_the_token_does_not_live_in_visible() {
        // Same substitution with every scene "visible": B's mask (a wide light around the
        // player's own token in B) would authorize scene-local coordinates that are dark in A.
        let (h, scene_b) = movement_scene_with_second_scene(false, true).await;
        // Cell (4,0): inside B's dim radius, outside A's.
        let dark_in_a = (450.0, 50.0);

        let res = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                scene_b,
                h.token_id,
                vec![h.start, dark_in_a],
                now_millis(),
            )
            .await;
        assert!(
            matches!(res, Err(DataError::Forbidden)),
            "B's mask must never authorize movement of a token that lives in A, and the refusal must come from the gate — not incidentally from the moving lock"
        );
        assert_eq!(
            h.committed_pos(h.token_id).await,
            h.start,
            "the refused move must not have committed a position"
        );

        // Control (runs second: the refused request above committed nothing and took no moving
        // lock): the same request named against A truncates short of the destination, proving
        // A's own mask genuinely does not authorize it.
        let control = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                h.scene_id,
                h.token_id,
                vec![h.start, dark_in_a],
                now_millis(),
            )
            .await
            .expect("same-scene request is executed, then gated per cell");
        assert_ne!(
            control.stop, dark_in_a,
            "control: A's own mask must not authorize this destination"
        );
    }

    #[tokio::test]
    async fn both_movement_gates_refuse_a_token_whose_parent_scene_has_no_document() {
        // Anti-drift: `Room::publish` (drag) and `Room::execute_move` (MoveRequest) must agree on
        // which scenes are ADMISSIBLE AT ALL, not merely on which cells are visible — the same
        // parity axis as the shared `MAX_GATE_WALK_COORD` bound. A silent 100-unit cell-size
        // default in either gate would index the mask, the region field, and the traversal walk
        // in a grid no scene declared, and would do so in only one of the two gates.
        //
        // The world here is `unrestricted`, so neither gate can refuse for an unrelated
        // mask reason: with the default restored, `publish` reaches its `Unrestricted` continue
        // and `execute_move` walks the path unmasked, and both then fail — if at all — with
        // something other than `Forbidden`.
        use crate::data::command::FieldChange;
        use crate::data::document::DocRole;
        let h = movement_scene_with_wall().await;
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let dangling_id = Uuid::from_u128(0xDA46_3000);
        let ghost_scene = Uuid::from_u128(0xDA46_4000);
        let mut dangling = wdoc(h.world_id, dangling_id, "token");
        dangling.parent_id = Some(ghost_scene);
        dangling.owner = Some(h.player.user_id);
        dangling
            .permissions
            .users
            .insert(h.player.user_id, DocRole::Owner);
        dangling.engine = Some(token_engine(50.0, 50.0));
        // Injected straight into the derived read-model: storage's foreign key (and its
        // descendant-expanding delete) makes this state unreachable through `publish`, and
        // neither gate may depend on that storage guarantee.
        h.room
            .scene()
            .write()
            .await
            .apply_op(&Operation::Create { doc: dangling });

        let moved = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                ghost_scene,
                dangling_id,
                vec![(50.0, 50.0), (150.0, 50.0)],
                now_millis(),
            )
            .await;
        assert!(
            matches!(moved, Err(DataError::Forbidden)),
            "execute_move must refuse a token whose parent scene has no document"
        );

        let dragged = h
            .room
            .publish(
                &h.repo,
                &h.player,
                vec![Operation::Update {
                    doc_id: dangling_id,
                    changes: vec![
                        FieldChange {
                            remove: false,
                            path: "/engine/x".into(),
                            old: serde_json::json!(50.0),
                            new: serde_json::json!(150.0),
                        },
                        FieldChange {
                            remove: false,
                            path: "/engine/y".into(),
                            old: serde_json::json!(50.0),
                            new: serde_json::json!(50.0),
                        },
                    ],
                }],
                now_millis(),
                WriteOrigin::Client,
            )
            .await;
        assert!(
            matches!(dragged, Err(DataError::Forbidden)),
            "publish's drag gate must refuse the same input execute_move refuses"
        );
    }

    #[tokio::test]
    async fn execute_move_gate_inputs_come_from_the_tokens_own_scene() {
        // Pins the DERIVATION, independently of the rejection. Whatever the outcome shape, a
        // request naming an `unrestricted` scene the token does not live in must not move the
        // token, because the restriction, mask, walls, and regions the walk is gated against
        // come from the token's own scene. This holds both when the mismatch is refused outright
        // and when it is merely executed against the derived scene (a zero-progress stop), so
        // dropping the equality check leaves it green while dropping the derivation breaks it —
        // unlike the two tests above, which assert `is_err()` and therefore pin only the
        // redundant rejection.
        let (h, scene_b) = movement_scene_with_second_scene(true, false).await;
        let far_dark = (2050.0, 2050.0);
        let res = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                scene_b,
                h.token_id,
                vec![h.start, far_dark],
                now_millis(),
            )
            .await;
        if let Ok(exec) = &res {
            assert_eq!(
                exec.scene, h.scene_id,
                "the executed scene is the token's own, never the one the request named"
            );
        }
        // Wherever the token ended up, that cell must be one A's OWN mask authorizes — the
        // property that fails the instant any gate input is keyed on the requested scene, and
        // that holds equally whether the request was refused (stop == start) or walked under
        // A's gate. `far_dark` is outside A's mask, so a mask-skipped walk lands outside it.
        let (cx, cy) = h.committed_pos(h.token_id).await;
        let committed_cell = ((cx / 100.0).floor() as i32, (cy / 100.0).floor() as i32);
        let mask = h
            .room
            .scene()
            .read()
            .await
            .visible_cells(h.player.user_id, h.scene_id, true);
        assert!(
            mask.contains(&committed_cell),
            "committed cell {committed_cell:?} is not in scene A's visibility mask"
        );
    }

    #[tokio::test]
    async fn execute_move_still_executes_a_request_naming_the_tokens_own_scene() {
        // Guard cannot silently break play: the legitimate same-scene move still commits.
        let (h, _scene_b) = movement_scene_with_second_scene(true, true).await;
        let res = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                h.scene_id,
                h.token_id,
                vec![h.start, h.lit_goal],
                now_millis(),
            )
            .await
            .expect("same-scene move must still succeed");
        assert_eq!(res.stop, h.lit_goal);
        assert_eq!(
            res.scene, h.scene_id,
            "the executed scene is the token's own parent scene"
        );
        assert_eq!(h.committed_pos(h.token_id).await, h.lit_goal);
    }

    #[tokio::test]
    async fn execute_move_refuses_a_token_with_no_parent_scene() {
        // Fail closed: a token with no resolvable scene has no gate inputs of its own, so the
        // client's `scene_id` must never be used as a fallback.
        use crate::data::document::DocRole;
        let h = movement_scene("visible", true).await;
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let orphan_id = Uuid::from_u128(0x0FFA_1000);
        let mut orphan = wdoc(h.world_id, orphan_id, "token");
        orphan.parent_id = None;
        orphan.owner = Some(h.player.user_id);
        orphan
            .permissions
            .users
            .insert(h.player.user_id, DocRole::Owner);
        orphan.engine = Some(token_engine(50.0, 50.0));
        h.room
            .publish(
                &h.repo,
                &h.gm,
                vec![Operation::Create { doc: orphan }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();

        let res = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                h.scene_id,
                orphan_id,
                vec![(50.0, 50.0), (50.0, 150.0)],
                now_millis(),
            )
            .await;
        assert!(
            matches!(res, Err(DataError::Forbidden)),
            "a parentless token must be refused by the gate, not by a downstream write"
        );
    }

    #[tokio::test]
    async fn execute_move_refuses_a_token_whose_parent_scene_does_not_exist() {
        // Fail closed: a dangling `parent_id` resolves to no scene document, so no cell size,
        // restriction, mask, or wall set can be derived — the move is refused, never gated
        // against the client's `scene_id` or a default cell size.
        //
        // The state is injected straight into the derived read-model: storage enforces the
        // `parent_id` foreign key (and cascades on scene delete), so a dangling parent cannot be
        // reached through `publish`. The gate must not depend on that storage guarantee.
        // `DataError::Forbidden` (not a storage error) is asserted so the test cannot pass on the
        // downstream write failing instead of the gate refusing.
        use crate::data::document::DocRole;
        let h = movement_scene("visible", true).await;
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let dangling_id = Uuid::from_u128(0xDA46_1000);
        let ghost_scene = Uuid::from_u128(0xDA46_2000);
        let mut dangling = wdoc(h.world_id, dangling_id, "token");
        dangling.parent_id = Some(ghost_scene);
        dangling.owner = Some(h.player.user_id);
        dangling
            .permissions
            .users
            .insert(h.player.user_id, DocRole::Owner);
        dangling.engine = Some(token_engine(50.0, 50.0));
        h.room
            .scene()
            .write()
            .await
            .apply_op(&Operation::Create { doc: dangling });

        let res = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                ghost_scene,
                dangling_id,
                vec![(50.0, 50.0), (50.0, 150.0)],
                now_millis(),
            )
            .await;
        assert!(
            matches!(res, Err(DataError::Forbidden)),
            "a token whose parent scene does not exist must be refused by the gate"
        );
    }

    #[tokio::test]
    async fn client_update_with_posint_pre_image_after_execute_move_is_accepted() {
        // Reproduces the OCC PosInt/Float variant-mismatch bug end-to-end:
        // `execute_move` commits a whole-number-valued token position, which
        // stores as a serde_json `Float` (`json!(f64)` always serializes to the
        // Float variant, even for a whole number). A subsequent client-authored
        // `Update` -- like an ordinary `sendMoves` token drag -- echoes the
        // JS-side whole number back as a `PosInt` pre-image (`JSON.parse` cannot
        // preserve "this was a float" for a whole-number value). The OCC check
        // in `apply_intent` must accept this pre-image, not spuriously Conflict.
        use crate::data::command::FieldChange;

        let h = movement_scene("unrestricted", false).await;
        h.room
            .execute_move(
                &h.repo,
                &h.player,
                h.scene_id,
                h.token_id,
                vec![h.start, h.adj],
                now_millis(),
            )
            .await
            .unwrap();
        assert_eq!(h.committed_pos(h.token_id).await, h.adj);

        // Sanity: the stored /engine/x is the Float variant serialization.
        let stored = h.repo.get_document(h.token_id).await.unwrap().unwrap();
        let stored_x = stored.engine.unwrap()["x"].clone();
        assert_eq!(
            serde_json::to_string(&stored_x).unwrap(),
            "150.0",
            "execute_move must commit the whole-number position as a Float"
        );

        // Client echoes the JS whole number 150 as a PosInt pre-image, not a
        // Float, for the OCC comparison -- exactly what `sendMoves` does.
        let ops = vec![Operation::Update {
            doc_id: h.token_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/engine/x".into(),
                old: serde_json::Value::Number(serde_json::Number::from(150u64)),
                new: serde_json::json!(160.0),
            }],
        }];
        let result = h
            .repo
            .apply_intent(
                &h.player,
                h.world_id,
                ops,
                now_millis(),
                WriteOrigin::Client,
            )
            .await;
        assert!(
            result.is_ok(),
            "a PosInt pre-image numerically equal to the stored Float must be accepted, got: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn execute_move_rejects_a_moving_token() {
        // First execute_move succeeds and stamps the moving lock (end epoch in the future).
        // An immediate second call on the same token must be Forbidden while the lock is held.
        //
        // Speed 0.001 cells/sec (the floor `resolved_animation_speed` clamps to) makes the lock
        // window ~1.4e6 seconds for this one-cell move. The lock is checked against
        // `execute_move`'s own internal clock, not a test-supplied `now`, so the window must be
        // wide enough that no scheduling delay between the two awaits can close it — at the
        // default 6 cells/sec it is only ~167 ms, which a loaded machine outruns intermittently.
        let h = movement_scene_with_speed("unrestricted", false, 0.001).await;
        let _ = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                h.scene_id,
                h.token_id,
                vec![h.start, h.adj],
                now_millis(),
            )
            .await
            .unwrap();
        // Immediately request again — moving lock end is still in the future.
        let again = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                h.scene_id,
                h.token_id,
                vec![h.adj, h.adj2],
                now_millis(),
            )
            .await;
        assert!(
            matches!(again, Err(DataError::Forbidden)),
            "second execute_move on a moving token must be Forbidden"
        );
    }

    #[tokio::test]
    async fn non_gm_mover_gets_progressive_sweep_in_unrestricted_scene() {
        // A non-GM mover in an Unrestricted-mode scene must get a progressive vision
        // sweep gated on ROLE, not on the Unrestricted restriction mode itself.
        let h = movement_scene("unrestricted", false).await;
        let res = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                h.scene_id,
                h.token_id,
                vec![h.start, h.adj],
                now_millis(),
            )
            .await
            .unwrap();
        assert!(
            res.mover_vision.is_some(),
            "a non-GM mover in an Unrestricted scene must get a progressive vision sweep, not a static-fog snap"
        );
    }

    #[tokio::test]
    async fn gm_mover_still_gets_no_sweep_in_unrestricted_scene() {
        // GM movers must never get a sweep, regardless of restriction mode (unchanged).
        let h = movement_scene("unrestricted", false).await;
        let res = h
            .room
            .execute_move(
                &h.repo,
                &h.gm,
                h.scene_id,
                h.token_id,
                vec![h.start, h.adj],
                now_millis(),
            )
            .await
            .unwrap();
        assert!(
            res.mover_vision.is_none(),
            "GM movers must not get a sweep, regardless of restriction mode (unchanged behavior)"
        );
    }

    #[tokio::test]
    async fn execute_move_truncates_at_a_wall_atomically() {
        // Path: start → corner → beyond_wall. Wall blocks the second step; executor
        // truncates at corner and commits atomically at that stop.
        let h = movement_scene_with_wall().await;
        let corner = h.adj;
        let beyond_wall = h.adj2;
        let res = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                h.scene_id,
                h.token_id,
                vec![h.start, corner, beyond_wall],
                now_millis(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.stop, corner,
            "executor must stop at the last clear cell"
        );
        assert_eq!(
            h.committed_pos(h.token_id).await,
            corner,
            "committed position must equal the truncation stop"
        );
    }

    #[tokio::test]
    async fn execute_move_authoritative_field_arrests_a_region_the_players_route_never_saw() {
        use crate::data::document::Visibility;
        use serde_json::json;

        let (repo, world_id, gm) = repo_with_world().await;
        let p = repo
            .create_user("p", None, ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world_id, p, WorldRole::Player)
            .await
            .unwrap();
        let player = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let region_id = Uuid::from_u128(12);
        let ws_id = Uuid::from_u128(13);

        let mut ws = wdoc(world_id, ws_id, "world-settings");
        ws.owner = Some(gm.user_id);
        ws.system = json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": false, "lightMode": "environmentLight",
                "environment": { "color": "#000000", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "unrestricted",
                "partialCellLeniency": true,
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" },
        });
        ws.engine = Some(ws_engine(ws.system.clone()));
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm.user_id);
        scene.system = json!({ "grid": { "size": 100 } });
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: scene }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut token = wdoc(world_id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(player.user_id);
        token.engine = Some(json!({ "x": 0.0, "y": 0.0, "w": 100, "h": 100, "rotation": 0 }));
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: token }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut region = wdoc(world_id, region_id, "region");
        region.parent_id = Some(scene_id);
        region.owner = Some(gm.user_id);
        region.engine = Some(json!({
            "shape": { "kind": "rect", "points": [50.0, 0.0, 150.0, 100.0] },
            "behavior": "impassable", "cost": 1.0, "enabled": true,
        }));
        region
            .permissions
            .property_overrides
            .insert("/engine".into(), Visibility::GmOnly);
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: region }],
            3,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // The player's own pathfind field never sees this secret region — the route request
        // itself is out of scope here (the router is covered elsewhere); this test proves
        // execute_move enforces it regardless.
        let exec = room
            .execute_move(
                &repo,
                &player,
                scene_id,
                token_id,
                vec![(0.0, 0.0), (100.0, 0.0)],
                100,
            )
            .await
            .unwrap();
        assert_eq!(
            exec.stop,
            (0.0, 0.0),
            "authoritative field blocks the secret impassable region"
        );
    }

    #[tokio::test]
    async fn execute_move_revealed_union_allows_explored_cell() {
        // Guards the Revealed-union contract: visible_cells ∪ explored must be passed to
        // the pure executor, not visible_cells alone. A cell that is explored-but-not-
        // currently-visible must be reachable under Revealed restriction.
        //
        // "revealed" scene, light at (50,50) radius 1.5 cells. Target (550,550) = cell (5,5)
        // is outside the light radius (not in visible_cells). The explored set is seeded to
        // cover cells (0,0)–(5,5) so visible ∪ explored includes the entire path.
        let h = movement_scene("revealed", /*with_light=*/ true).await;
        let cell = 100.0_f64;

        let mut seed = crate::scene::explored::ExploredSet::new();
        seed.mark_polygons(
            &[vec![
                0.0,
                0.0,
                6.0 * cell,
                0.0,
                6.0 * cell,
                6.0 * cell,
                0.0,
                6.0 * cell,
            ]],
            &crate::scene::grid_shape::SquareGrid {
                cell,
                rule: crate::scene::pathfinding::DiagonalRule::Chebyshev,
            },
            cell,
        );
        h.repo
            .set_explored(h.world_id, h.scene_id, h.player.user_id, &seed.to_bytes())
            .await
            .unwrap();

        // Diagonal king-steps from (50,50) to (550,550) — 5 steps, all in the explored zone.
        let path: Vec<(f64, f64)> = (0..=5)
            .map(|i| (50.0 + i as f64 * 100.0, 50.0 + i as f64 * 100.0))
            .collect();

        let res = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                h.scene_id,
                h.token_id,
                path.clone(),
                now_millis(),
            )
            .await
            .unwrap();

        // If the union was correctly applied the token reaches the explored-but-dark goal.
        assert_eq!(
            res.stop,
            *path.last().unwrap(),
            "revealed union must allow move into explored-but-not-visible cell"
        );
        assert_eq!(h.committed_pos(h.token_id).await, res.stop);
    }

    /// Identical to `movement_scene`, but the scene doc's `engine.vision.movementModel` is
    /// explicitly `"continuous"`: proves `execute_move` gates an any-angle route
    /// from a scene genuinely marked continuous, not just incidentally sent a diagonal path.
    /// Functionally inert on the server today — `execute_move` has no `movementModel` branch,
    /// being engine-agnostic; this mirrors `movement_scene`'s body (this file's
    /// established per-scenario-helper convention) with one added JSON key.
    async fn movement_scene_continuous(restriction: &str, with_light: bool) -> MovementHandle {
        use serde_json::json;

        let (repo, world_id, gm) = repo_with_world().await;
        let p = repo
            .create_user(
                "player_continuous",
                None,
                crate::auth::role::ServerRole::User,
                0,
            )
            .await
            .unwrap();
        repo.add_member(world_id, p, WorldRole::Player)
            .await
            .unwrap();
        let player = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let (scene_id, token_id, ws_id, light_id) = (
            Uuid::from_u128(0xC047_0000),
            Uuid::from_u128(0xC047_0001),
            Uuid::from_u128(0xC047_0002),
            Uuid::from_u128(0xC047_0003),
        );

        let mut ws = wdoc(world_id, ws_id, "world-settings");
        ws.owner = Some(gm.user_id);
        ws.system = json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#000000", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": restriction,
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        ws.engine = Some(ws_engine(ws.system.clone()));
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // Only structural difference from `movement_scene`: declares `vision.movementModel` on
        // the scene doc. Inert server-side today — execute_move has no movementModel branch.
        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm.user_id);
        scene.system = json!({
            "grid": { "kind": "square", "size": 100 },
            "vision": { "movementModel": "continuous" }
        });
        scene.engine = Some(scene.system.clone());
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut token = wdoc(world_id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        // Required for the player to have write permission on the token's /engine/x,y fields
        // at commit time (mirrors every sibling helper — movement_scene et al.); `owner` alone
        // does not grant the per-doc write permission apply_intent checks.
        token
            .permissions
            .users
            .insert(p, crate::data::document::DocRole::Owner);
        token.engine = Some(token_engine(50.0, 50.0));
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: token }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        if with_light {
            // Bright boundary = 1.5 * 100 = 150 world units; dim boundary = 3.0 * 100 = 300.
            let mut light = wdoc(world_id, light_id, "light");
            light.parent_id = Some(scene_id);
            light.owner = Some(gm.user_id);
            light.system = json!({
                "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
                "brightRadius": 1.5, "dimRadius": 3.0, "enabled": true
            });
            light.engine = Some(light.system.clone());
            room.publish(
                &repo,
                &gm,
                vec![Operation::Create { doc: light }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
        }

        MovementHandle {
            room,
            repo,
            gm,
            player,
            world_id,
            scene_id,
            token_id,
            start: (50.0, 50.0),
            lit_goal: (50.0, 150.0),
            adj: (150.0, 50.0),
            adj2: (250.0, 50.0),
        }
    }

    #[tokio::test]
    async fn execute_move_continuous_any_angle_route_commits_atomically() {
        // Proves the unified sampled executor gates a genuinely any-angle
        // (non-grid-aligned) polyline exactly like a grid path — no movementModel branch
        // anywhere on this path. Goal (110,130) is a 3-4-5 triangle scaled ×20
        // from start (50,50): distance = sqrt(60²+80²) = 100 wu, safely inside the light's
        // 150 wu bright radius (50 wu margin) and not a grid cell-center (cell centers sit
        // at 50 + 100k on each axis).
        let h = movement_scene_continuous("visible", /*with_light=*/ true).await;
        let goal = (110.0, 130.0);
        let res = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                h.scene_id,
                h.token_id,
                vec![h.start, goal],
                now_millis(),
            )
            .await
            .unwrap();
        assert_eq!(res.stop, goal, "any-angle move commits at the exact goal");
        assert_eq!(h.committed_pos(h.token_id).await, res.stop);
    }

    #[tokio::test]
    async fn execute_move_continuous_truncates_before_entering_unseen_space() {
        // `execute_move`'s per-cell gate TRUNCATES a route at the last visible sample rather
        // than rejecting the whole request outright (`DataError::Forbidden` is reserved for
        // structural failures — unknown token / TooLong / Degenerate — and the moving-lock
        // check; a genuine cell-gate stop is `Ok` with a partial `stop`, exactly like the
        // sibling wall-truncation test `execute_move_truncates_at_a_wall_atomically`). This
        // proves the cell-sampled gate applies to any-angle paths, not just grid ones, and
        // still commits atomically at the truncation point rather than silently reaching a
        // goal in unseen territory.
        //
        // Goal (650,850) is a 3-4-5 triangle scaled ×200 from start (50,50): distance =
        // sqrt(600²+800²) = 1000 wu. `gate_walk` subdivides this into 8 dense ≤1-cell samples
        // (cheby = max(600,800) = 800 wu ⇒ k = ceil(800/100) = 8). Sample 1, (125,150), lands
        // in cell (1,1) — inside the ~100wu (1-cell) `VISION_BOUND_MARGIN` scan box around the
        // colocated token/light viewpoint (50,50), so it is visible. Sample 2, (200,250), lands
        // in cell (2,2), outside that scan box — not in the mask — so the walk truncates there,
        // leaving the token at sample 1's exact position.
        let h = movement_scene_continuous("visible", /*with_light=*/ true).await;
        let goal = (650.0, 850.0);
        let res = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                h.scene_id,
                h.token_id,
                vec![h.start, goal],
                now_millis(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.stop,
            (125.0, 150.0),
            "cell-gate truncates the route at the last visible sample, short of the goal"
        );
        assert_ne!(
            res.stop, goal,
            "must not silently reach a goal in unseen space"
        );
        assert_eq!(h.committed_pos(h.token_id).await, res.stop);
    }
}
