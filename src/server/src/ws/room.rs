//! Per-world rooms, ring buffer, registry, and telemetry counters.

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
    #[allow(dead_code)]
    pub cost: f64,
}

const MAX_EVENTS: usize = 1024;
const MAX_AGE_MS: i64 = 5 * 60 * 1000;
const BROADCAST_CAPACITY: usize = 256;

/// Recent `Event` frames for hot resync, bounded by count and age. Age is
/// measured relative to the newest buffered event's `ts`.
pub struct RingBuffer {
    events: VecDeque<Arc<ServerMsg>>, // ascending seq; each is ServerMsg::Event
}

impl RingBuffer {
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
    pub connections: AtomicI64,
    pub events_published: AtomicU64,
    pub gaps_detected: AtomicU64,
    pub resyncs_hot: AtomicU64,
    pub resyncs_cold: AtomicU64,
    pub lagged_drops: AtomicU64,
}

/// Serializable snapshot of a room's telemetry for the admin debug endpoint.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct RoomStatsSnapshot {
    pub world_id: Uuid,
    pub connections: i64,
    pub current_seq: i64,
    pub events_published: u64,
    pub gaps_detected: u64,
    pub resyncs_hot: u64,
    pub resyncs_cold: u64,
    pub lagged_drops: u64,
}

/// A per-world fan-out room. The `broadcast` channel is intentionally lossy —
/// a lagging receiver gets `Lagged(n)` and resyncs from the ring/log tiers.
pub struct Room {
    pub world_id: Uuid,
    tx: broadcast::Sender<Arc<ServerMsg>>,
    ring: Mutex<RingBuffer>,
    publish_guard: Mutex<()>,
    current_seq: AtomicI64,
    scene: RwLock<SceneEcs>,
    pub stats: RoomStats,
    /// Per-token moving lock: token → move-end epoch-ms. An entry is expired when
    /// `now_millis() >= end`; expired/absent entries are treated as available (lazy expiry,
    /// no timer). Updated by `execute_move` after a successful commit.
    moving: Mutex<HashMap<Uuid, i64>>,
}

impl Room {
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

    pub fn current_seq(&self) -> i64 {
        self.current_seq.load(Ordering::Acquire)
    }

    /// Broadcast a non-sequenced, out-of-band frame (e.g. AssetChanged). Unlike
    /// `publish`, it does NOT push to the ring or bump `current_seq`, so a
    /// lagging receiver that resyncs from the ring/log never replays it — by
    /// design, since the frame's source of truth (the asset `version`) is
    /// re-read on any access. Best-effort: drops if there are no receivers.
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
        // M9a: server-authoritative movement collision (engine-owned geometry — the second
        // ARCHITECTURE #6 exception). A non-GM token move whose path crosses a `blocksMove`
        // wall is rejected BEFORE the write, so it consumes no seq and the client rolls back.
        // GM moves ignore walls (the override, M9 §5). The move start is the authoritative
        // ECS position, never the client's claimed pre-image.
        if ctx.world_role != crate::data::document::WorldRole::Gm {
            // Pending Revealed-mode checks deferred past the ECS read borrow: (scene_id,
            // move_cells, visible_set). Revealed mode requires an async get_explored call
            // which cannot occur while holding the scene read lock.
            type CellSet = std::collections::BTreeSet<(i32, i32)>;
            let mut revealed_pending: Vec<(uuid::Uuid, CellSet, CellSet)> = Vec::new();
            {
                let scene = self.scene.read().await;
                // Memoize the visible mask per (scene, leniency) within this publish so a
                // batch of moves in the same scene does not recompute the mask per token.
                let mut visible_cache: std::collections::HashMap<
                    (uuid::Uuid, bool),
                    std::collections::BTreeSet<(i32, i32)>,
                > = std::collections::HashMap::new();
                // By design: this movement gate only inspects Operation::Update (a token
                // move). Operation::Create (initial token placement) is intentionally
                // ungated — the create capability is already a privileged grant (GM or a
                // place-token tool), and unrestricted initial placement is normal
                // authoring behavior. This is not a movement-restriction bypass: it is
                // the placement path, not the move path. (This scoping is intentional —
                // see docs/design/ARCHITECTURE.md invariant 6.)
                for op in &ops {
                    if let Operation::Update { doc_id, changes } = op {
                        // Validate the POST-IMAGE position over the committed `/engine` band
                        // (the engine band with all changes applied), so a wholesale `/engine`
                        // write or duplicate `/engine/x` changes can't present a safe target
                        // while committing an unsafe one.
                        if let Some((scene_id, a0, a1)) = scene.token_move(*doc_id, changes) {
                            // Coordinate-magnitude admissibility, checked before ANY geometry
                            // work. Coupling: this is the SAME `MAX_GATE_WALK_COORD` bound
                            // `move_exec::gate_walk` applies to every path it walks, reused (not
                            // duplicated) so the two movement gates agree on which inputs are
                            // admissible, not merely on which cells are visible. Checked for every
                            // restriction mode — including `Unrestricted`, which `gate_walk` also
                            // bounds — so the agreement holds in all modes. SCOPE: this whole block
                            // is non-GM only (mirroring `execute_move`'s own scoping), but
                            // `TokenEngine::validate` bounds every document write unconditionally —
                            // GM included — at ingress, so no live write (drag or `Create`) can
                            // commit an over-bound coordinate any more. The `a0` test below is
                            // defense-in-depth against a position that predates that ingress gate
                            // (e.g. legacy data); it fails closed regardless of how such a position
                            // came to exist. Beyond the bound the
                            // downstream primitives lose their guarantees (`gate_walk`'s
                            // magnitude-scaled identity tolerance, `HexGrid::line_traversal`'s
                            // `VERTEX_PROBE` offset, which scales with `self.size`), so an over-magnitude endpoint fails
                            // closed exactly as a `line_traversal` `None` does.
                            // Non-finite is rejected first, mirroring `gate_walk`'s own ordering:
                            // `NaN.abs() > bound` is false, so a magnitude-only test admits NaN,
                            // and `Unrestricted` `continue`s before any downstream finiteness
                            // check could catch it. No reachable input produces NaN today
                            // (`token_move` sources both coords via `Value::as_f64` and serde_json
                            // parses no NaN literal), so this is what makes the admissibility
                            // agreement above hold literally rather than only for reachable input.
                            let bound = crate::scene::move_exec::MAX_GATE_WALK_COORD;
                            if !a0.0.is_finite()
                                || !a0.1.is_finite()
                                || !a1.0.is_finite()
                                || !a1.1.is_finite()
                                || a0.0.abs() > bound
                                || a0.1.abs() > bound
                                || a1.0.abs() > bound
                                || a1.1.abs() > bound
                            {
                                return Err(DataError::Forbidden);
                            }
                            // M9a wall gate (unchanged): a wall crossing short-circuits before
                            // any mask work.
                            if scene.blocks_move(scene_id, a0, a1) {
                                return Err(DataError::Forbidden);
                            }
                            // Scene-existence admissibility, checked before the restriction
                            // dispatch so it holds in EVERY mode — including `Unrestricted`,
                            // which `continue`s below. Coupling: `Room::execute_move` refuses
                            // the same input, so the two movement gates agree on which scenes
                            // are admissible at all, not merely on which cells are visible
                            // (the same never-fork parity axis as the coordinate bound above).
                            // `scene_grid_sizes` carries an entry — already defaulted to 100 —
                            // for every live scene, so an absent entry means the token's parent
                            // scene has no document: no authored cell size exists to index the
                            // visibility mask, the region field, or the traversal walk against.
                            // SCOPE (the same caveat the coordinate bound above carries): this
                            // whole block is non-GM only while `execute_move` refuses a GM too,
                            // so the agreement between the two gates is over non-GM input. A GM
                            // drag never reaches this check and never reads `cell` at all, so
                            // nothing is silently defaulted on that path.
                            let Some(cell) = scene.scene_grid_sizes().get(&scene_id).copied()
                            else {
                                return Err(DataError::Forbidden);
                            };
                            // M10e-4 movement-restriction gate.
                            let settings = scene.resolve_scene(scene_id);
                            if matches!(
                                settings.movement_restriction,
                                crate::scene::MovementRestriction::Unrestricted
                            ) {
                                continue;
                            }
                            // Every cell the move segment crosses, via the scene's own
                            // resolved grid shape (a supercover on both kinds: square cell-walk,
                            // hex psi-crossing)
                            // — the same primitive `move_exec::execute_move` gates against, so
                            // this agrees with the executor on hex scenes too, not just square.
                            // None ⇒ over-cap or degenerate grid → fail closed (DoS guard, spec §8).
                            let grid = scene.resolve_grid_shape(scene_id, cell);
                            let Some(move_cells) = grid.line_traversal(a0, a1, cell) else {
                                return Err(DataError::Forbidden);
                            };
                            let lenient = settings.partial_cell_leniency;
                            let visible = visible_cache
                                .entry((scene_id, lenient))
                                .or_insert_with(|| {
                                    scene.visible_cells_cached(ctx.user_id, scene_id, lenient)
                                })
                                .clone();
                            match settings.movement_restriction {
                                crate::scene::MovementRestriction::Visible => {
                                    if !move_cells.iter().all(|c| visible.contains(c)) {
                                        return Err(DataError::Forbidden);
                                    }
                                }
                                crate::scene::MovementRestriction::Revealed => {
                                    // explored ∪ visible — explored is async; defer past
                                    // the read guard so no lock is held across an await.
                                    revealed_pending.push((scene_id, move_cells, visible));
                                }
                                crate::scene::MovementRestriction::Unrestricted => {}
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
                // center-sampled by construction (explored.rs). The asymmetry only ever ENLARGES
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
    /// # Revealed-union contract (spec §13)
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
        // All three inputs (restriction, cell, visible) are resolved while holding the read
        // lock and DROPPED before any await (no lock-across-await; mirrors `publish`).
        let restriction;
        let cell;
        let start;
        let token_scene;
        let visible_cells;
        let is_revealed;
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

            let settings = scene.resolve_scene(token_scene);
            // Fail-closed on a `parent_id` with no scene document: `scene_grid_sizes` carries an
            // entry (defaulting to 100) for every live scene, so an absent entry means the scene
            // itself is gone — no authored cell size exists to index the visibility mask, the
            // region field, or the traversal walk against.
            cell = *scene
                .scene_grid_sizes()
                .get(&token_scene)
                .ok_or(DataError::Forbidden)?;

            // GMs use Unrestricted (mask-skipped), but `execute_move` still honors walls for
            // GMs (step-1 `blocks_move` is unconditional). This intentionally diverges from
            // `publish`'s legacy GM wall-bypass, which is to be retired. Do NOT re-grant GM
            // wall-bypass here: the M1 server-authoritative model requires wall enforcement
            // for all movers including GMs when moves are executed through this path.
            restriction = if ctx.world_role == crate::data::document::WorldRole::Gm {
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
        // INVARIANT (spec §13): for Revealed the `visible` set passed to execute_move MUST be
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
            // is used as for static vision (M9b full-wall-set invariant). Hoisting:
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
        // Hydrate the derived ECS from persisted scene entities (#5) using the
        // same definition as the live path (`is_scene_entity`), so the loader and
        // the predicate cannot drift. Stamp it with the world's current seq.
        let docs = repo.query_scene_entities(world_id).await?;
        let mut scene_ecs = SceneEcs::from_documents(docs, world.seq);
        // M10e-2: hydrate the lighting-aware vision inputs that are NOT scene entities — the three
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

    pub fn get(&self, world_id: Uuid) -> Option<Arc<Room>> {
        self.rooms.get(&world_id).map(|r| r.clone())
    }

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

    #[tokio::test]
    async fn movement_blocked_for_player_crossing_wall_but_gm_bypasses() {
        use crate::data::command::FieldChange;
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
        let player = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let wall_id = Uuid::from_u128(12);
        let ws_id = Uuid::from_u128(13);

        // World-settings with movementRestriction="unrestricted" so this test isolates the
        // M9a wall-collision gate without the M10e-4 visibility gate interfering (the scene
        // has no lighting, so visible_cells would be empty under any restrictive mode).
        let mut ws = wdoc(world_id, ws_id, "world-settings");
        ws.owner = Some(gm.user_id);
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
            &gm,
            vec![Operation::Create { doc: ws }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm.user_id);
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // Token owned (writable) by the player, at (0,0).
        let mut token = wdoc(world_id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.engine = Some(token_engine(0.0, 0.0));
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: token }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // A blocksMove wall on the diagonal x+y=10.
        let mut wall = wdoc(world_id, wall_id, "wall");
        wall.parent_id = Some(scene_id);
        wall.owner = Some(gm.user_id);
        wall.engine =
            Some(json!({ "seg": { "x1": 0, "y1": 10, "x2": 10, "y2": 0 }, "blocksMove": true }));
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: wall }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // `/engine/x,y` are stored as `f64` (`TokenEngine`); the pre-image `old` must be typed
        // identically or the OCC check's `serde_json::Value` equality (which distinguishes an
        // integer `Number` variant from a float one) spuriously reports staleness.
        let mv = |nx: i64, ny: i64, ox: i64, oy: i64| Operation::Update {
            doc_id: token_id,
            changes: vec![
                FieldChange {
                    remove: false,
                    path: "/engine/x".into(),
                    old: json!(ox as f64),
                    new: json!(nx as f64),
                },
                FieldChange {
                    remove: false,
                    path: "/engine/y".into(),
                    old: json!(oy as f64),
                    new: json!(ny as f64),
                },
            ],
        };

        let seq_before = room.current_seq();
        // Forged bypass A: a single wholesale `/engine` write that relocates the token past the
        // wall must be caught (the post-image, not a leaf-path match, is validated).
        let whole = Operation::Update {
            doc_id: token_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/engine".into(),
                old: json!({ "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0 }),
                new: json!({ "x": 10.0, "y": 10.0, "w": 1.0, "h": 1.0, "rotation": 0.0 }),
            }],
        };
        assert!(matches!(
            room.publish(&repo, &player, vec![whole], 0, WriteOrigin::Client)
                .await,
            Err(crate::data::DataError::Forbidden)
        ));
        assert_eq!(room.current_seq(), seq_before);
        // Forged bypass B: duplicate `/engine/x` (safe-then-unsafe) — last write wins, so the
        // committed x=11 crosses; the gate validates against that, not the first change.
        let dup = Operation::Update {
            doc_id: token_id,
            changes: vec![
                FieldChange {
                    remove: false,
                    path: "/engine/x".into(),
                    old: json!(0.0),
                    new: json!(1.0),
                },
                FieldChange {
                    remove: false,
                    path: "/engine/x".into(),
                    old: json!(0.0),
                    new: json!(11.0),
                },
            ],
        };
        assert!(matches!(
            room.publish(&repo, &player, vec![dup], 0, WriteOrigin::Client)
                .await,
            Err(crate::data::DataError::Forbidden)
        ));
        assert_eq!(room.current_seq(), seq_before);

        // Player move (0,0)->(10,10) crosses the wall → rejected before the write.
        let blocked = room
            .publish(
                &repo,
                &player,
                vec![mv(10, 10, 0, 0)],
                0,
                WriteOrigin::Client,
            )
            .await;
        assert!(matches!(blocked, Err(crate::data::DataError::Forbidden)));
        assert_eq!(
            room.current_seq(),
            seq_before,
            "a blocked move consumes no seq"
        );

        // The same player move that does NOT cross is allowed (so the block above was the
        // collision gate, not an authorization failure).
        room.publish(&repo, &player, vec![mv(1, 1, 0, 0)], 0, WriteOrigin::Client)
            .await
            .unwrap();
        assert_eq!(room.current_seq(), seq_before + 1);

        // A GM move across the wall bypasses the collision gate (the "ignore walls" override).
        room.publish(&repo, &gm, vec![mv(10, 10, 1, 1)], 0, WriteOrigin::Client)
            .await
            .unwrap();
    }

    /// A `/system/x` write on a token is game-system data — it must not be treated as a move
    /// by `Room::publish`'s M9a/M10e-4 gate (which reads `/engine` exclusively), and the
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
    /// gate's replay (`SceneEcs::token_move`, consulted by `Room::publish`'s movement gate) or
    /// the commit path's replay (`apply_intent`'s sequential `command::apply_field_change` application) computes
    /// it — in BOTH possible orderings of the two changes. Both replay implementations apply
    /// `changes` via `command::apply_field_change` in array order independently; this pins them against silently
    /// diverging (which would let the gate validate one post-image while a different one
    /// actually lands).
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
                    &h.player,
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
                    &h.player,
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
    // M10e-4: movement-restriction gate
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

    impl MovementHandle {
        /// Build an `Operation::Update` that moves the token to `(x, y)`. Reads the
        /// current authoritative ECS position so the `old` fields satisfy optimistic
        /// concurrency checks within the same test.
        async fn mv_to(&self, x: f64, y: f64) -> Operation {
            use crate::data::command::FieldChange;
            let scene = self.room.scene().read().await;
            let (ox, oy) = scene
                .token_move(self.token_id, &[])
                .map(|(_, (ox, oy), _)| (ox, oy))
                .unwrap_or((50.0, 50.0));
            drop(scene);
            Operation::Update {
                doc_id: self.token_id,
                changes: vec![
                    FieldChange {
                        remove: false,
                        path: "/engine/x".into(),
                        old: serde_json::json!(ox),
                        new: serde_json::json!(x),
                    },
                    FieldChange {
                        remove: false,
                        path: "/engine/y".into(),
                        old: serde_json::json!(oy),
                        new: serde_json::json!(y),
                    },
                ],
            }
        }

        /// Move to the center of the diagonal-neighbor cell (1,1) at world coords (150,150).
        ///
        /// Geometry (grid size=100, light at (50,50), brightRadius=1.4 cells = 140 world units):
        ///   - Cell (1,1) CENTER at (150,150): dist = sqrt(100²+100²) ≈ 141.4 wu = 1.414 cells
        ///     → clearly OUTSIDE the 1.4-cell boundary (strict center-only sampling rejects).
        ///   - Cell (1,1) near CORNER at (100,100): dist = sqrt(50²+50²) ≈ 70.7 wu = 0.707 cells
        ///     → clearly INSIDE the boundary (lenient corner sampling admits).
        ///
        /// Margins: center is ~1% beyond the boundary (not on it); corner is ~50% inside.
        /// Neither sample touches the polygon edge, so the split is raycaster-stable.
        async fn mv_to_partial_cell(&self) -> Operation {
            self.mv_to(150.0, 150.0).await
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

    /// Hex-grid variant of `movement_scene`: identical world-settings/token/light layout,
    /// but the scene's `/engine` declares `grid.kind = "hex"` (`resolve_grid_shape` selects
    /// `HexGrid`, not `SquareGrid`) — exercises the gate's hex cell-index path rather than
    /// square. `scene.engine` must be set explicitly (unlike `movement_scene`'s square
    /// fixture, which relies on `resolve_grid_shape`'s fail-closed square default and never
    /// sets it): `scene_grid_sizes`/`resolve_grid_shape` read `SceneEngine` off `doc.engine`,
    /// never `doc.system`.
    async fn movement_scene_hex(restriction: &str, with_light: bool) -> MovementHandle {
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
            Uuid::from_u128(0x5CE4),
            Uuid::from_u128(0x5CE5),
            Uuid::from_u128(0x5CE6),
            Uuid::from_u128(0x5CE7),
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

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm.user_id);
        scene.system = json!({ "grid": { "kind": "hex", "size": 100 } });
        scene.engine =
            Some(json!({ "grid": { "kind": "hex", "size": 100.0 }, "background": null }));
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

    /// A wall-less, all-bright hex scene (pointy-top, size 100) with authored bounds 1500x1500 and
    /// one player-owned token at (50,50). Lighting is OFF (`lightingEnabled: false`), so the mask is
    /// pure LOS: every hex whose center lies inside the bounds-derived LOS rectangle. That rectangle
    /// is axis-aligned in PIXEL space, so its hex preimage is a sheared parallelogram — which is what
    /// lets a destination exist whose hex traversal leaves the mask while the square-indexed
    /// traversal of the same segment does not (`movement_restriction_hex_rejects_unseen_cell_a_
    /// square_indexed_gate_would_allow`). `partialCellLeniency: false` (strict center sampling), so
    /// the mask is exactly the §13 strict set.
    async fn movement_scene_hex_open(restriction: &str) -> MovementHandle {
        use crate::data::document::DocRole;
        use serde_json::json;

        let (repo, world_id, gm) = repo_with_world().await;
        let p = repo
            .create_user(
                "player_hex_open",
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
        let (scene_id, token_id, ws_id) = (
            Uuid::from_u128(0x5CF0),
            Uuid::from_u128(0x5CF1),
            Uuid::from_u128(0x5CF2),
        );

        let mut ws = wdoc(world_id, ws_id, "world-settings");
        ws.owner = Some(gm.user_id);
        ws.system = json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": false, "lightMode": "environmentLight",
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": restriction,
                "partialCellLeniency": false
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
        scene.system = json!({ "grid": { "kind": "hex", "size": 100 } });
        scene.engine = Some(
            json!({ "grid": { "kind": "hex", "size": 100.0 }, "background": null,
                                    "bounds": { "width": 1500.0, "height": 1500.0 } }),
        );
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

    /// Two lit pockets (near (50,50) and far (950,950)) with a dark gap between
    /// cells 2–8. movementRestriction="visible", partialCellLeniency=false.
    async fn movement_scene_two_lit_pockets() -> MovementHandle {
        use crate::data::document::DocRole;
        use serde_json::json;

        let (repo, world_id, gm) = repo_with_world().await;
        let p = repo
            .create_user("player2", None, crate::auth::role::ServerRole::User, 0)
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
        let (scene_id, token_id, ws_id) = (
            Uuid::from_u128(0xB0C0),
            Uuid::from_u128(0xB0C1),
            Uuid::from_u128(0xB0C2),
        );
        let (light1, light2) = (Uuid::from_u128(0xB0C3), Uuid::from_u128(0xB0C4));

        let mut ws = wdoc(world_id, ws_id, "world-settings");
        ws.owner = Some(gm.user_id);
        ws.system = json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#000000", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "partialCellLeniency": false
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

        // Near pocket: radius 1.5 cells around (50,50) — covers cells (0,0).
        let mut l1 = wdoc(world_id, light1, "light");
        l1.parent_id = Some(scene_id);
        l1.owner = Some(gm.user_id);
        l1.system = json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 1.5, "dimRadius": 1.5, "enabled": true
        });
        l1.engine = Some(l1.system.clone());
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: l1 }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // Far pocket: radius 1.5 cells around (950,950) — covers cell (9,9).
        // Cells 2–8 between the pockets are unlit (gap).
        let mut l2 = wdoc(world_id, light2, "light");
        l2.parent_id = Some(scene_id);
        l2.owner = Some(gm.user_id);
        l2.system = json!({
            "x": 950.0, "y": 950.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 1.5, "dimRadius": 1.5, "enabled": true
        });
        l2.engine = Some(l2.system.clone());
        room.publish(
            &repo,
            &gm,
            vec![Operation::Create { doc: l2 }],
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
            lit_goal: (50.0, 150.0),
            adj: (150.0, 50.0),
            adj2: (250.0, 50.0),
        }
    }

    /// Scene for partial-cell leniency pair-test. Light at (50,50) with brightRadius=1.4
    /// cells (140 world units, grid size=100). The diagonal-neighbor cell (1,1) at world
    /// coords (150,150) has:
    ///   - CENTER at dist ≈ 141.4 wu (1.414 cells) → just outside the 1.4-cell boundary;
    ///     strict center-only sampling rejects the cell.
    ///   - Near CORNER at (100,100) at dist ≈ 70.7 wu (0.707 cells) → well inside the
    ///     boundary; lenient corner-sampling admits the cell.
    ///
    /// Neither sample point is on the polygon edge, so the classification is raycaster-stable
    /// with comfortable margin (~1% outside for center, ~50% inside for corner).
    async fn movement_scene_partial_cell(lenient: bool) -> MovementHandle {
        use crate::data::document::DocRole;
        use serde_json::json;

        let (repo, world_id, gm) = repo_with_world().await;
        let p = repo
            .create_user("player3", None, crate::auth::role::ServerRole::User, 0)
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
            Uuid::from_u128(0xC0DE),
            Uuid::from_u128(0xC0DF),
            Uuid::from_u128(0xC0E0),
            Uuid::from_u128(0xC0E1),
        );

        let mut ws = wdoc(world_id, ws_id, "world-settings");
        ws.owner = Some(gm.user_id);
        ws.system = json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#000000", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "partialCellLeniency": lenient
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

        // brightRadius=1.4 cells = 140 wu. Cell (1,1) center at (150,150) is ~141.4 wu
        // away — just outside the boundary (strict rejects). Its near corner at (100,100)
        // is ~70.7 wu away — well inside (lenient admits). Neither point is on the edge.
        let mut light = wdoc(world_id, light_id, "light");
        light.parent_id = Some(scene_id);
        light.owner = Some(gm.user_id);
        light.system = json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 1.4, "dimRadius": 1.4, "enabled": true
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
    async fn movement_restriction_visible_blocks_move_into_darkness() {
        // Gate: movementRestriction="visible", env intensity=0 so only the placed light illuminates.
        // Invariant: a player move into an unlit cell is Forbidden before the write (no seq consumed);
        // a move within the lit radius is allowed; GM is exempt from the gate.
        let h = movement_scene("visible", /*with_light=*/ true).await;
        let seq0 = h.room.current_seq();

        let op = h.mv_to(2000.0, 2000.0).await;
        let blocked = h
            .room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await;
        assert!(matches!(blocked, Err(crate::data::DataError::Forbidden)));
        assert_eq!(h.room.current_seq(), seq0, "blocked move consumes no seq");

        let op = h.mv_to(60.0, 60.0).await;
        h.room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await
            .unwrap();
        assert_eq!(h.room.current_seq(), seq0 + 1);

        // GM bypasses the visibility gate — token is now at (60,60) in ECS.
        let op = h.mv_to(2000.0, 2000.0).await;
        h.room
            .publish(&h.repo, &h.gm, vec![op], 0, WriteOrigin::Client)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn movement_restriction_unrestricted_allows_move_into_darkness() {
        // Unrestricted: only the M9a wall gate applies; a non-wall-crossing move into
        // an unlit cell is allowed regardless of visibility.
        let h = movement_scene("unrestricted", /*with_light=*/ false).await;
        let op = h.mv_to(2000.0, 2000.0).await;
        h.room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn movement_restriction_revealed_allows_move_into_explored_memory() {
        // "revealed" mode: explored-memory cells extend the allowed zone beyond current
        // visibility. Cells never seen and currently unlit remain forbidden.
        let h = movement_scene("revealed", /*with_light=*/ true).await;
        let cell = 100.0_f64;

        // Seed the explored set with ALL cells in the bounding box (0,0)–(5,5):
        // a rectangle covering the full path from token (50,50) to destination (550,550).
        // This ensures every supercover cell on the move segment is in explored ∪ visible,
        // which is what "revealed" mode requires — the gate checks the whole path.
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

        // Move to center of explored cell (5,5) — allowed via explored memory.
        let op = h.mv_to(550.0, 550.0).await;
        h.room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await
            .unwrap();

        // Move from (550,550) to a never-seen, never-explored, unlit cell — forbidden.
        let op = h.mv_to(9000.0, 9000.0).await;
        let blocked = h
            .room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await;
        assert!(matches!(blocked, Err(crate::data::DataError::Forbidden)));
    }

    #[tokio::test]
    async fn movement_restriction_checks_entire_move_not_just_endpoint() {
        // Supercover gate: a move whose endpoint is in the far lit pocket but whose
        // path traverses a dark gap between the two pockets must be rejected.
        let h = movement_scene_two_lit_pockets().await;
        let op = h.mv_to(950.0, 950.0).await;
        let blocked = h
            .room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await;
        assert!(
            matches!(blocked, Err(crate::data::DataError::Forbidden)),
            "dark gap on the path blocks the move even when endpoint is lit"
        );
    }

    #[tokio::test]
    async fn movement_restriction_lenient_allows_partial_cell() {
        // partialCellLeniency=true: a move to diagonal-neighbor cell (1,1) whose CENTER
        // is ~1.414 cells from the light (outside the 1.4-cell boundary) but whose near
        // CORNER is ~0.707 cells away (well inside) is allowed by lenient corner sampling.
        // The same move is rejected by strict center-only sampling. Geometry is stable:
        // neither sample point lies on the polygon boundary (see movement_scene_partial_cell).
        let lenient = movement_scene_partial_cell(/*lenient=*/ true).await;
        let op = lenient.mv_to_partial_cell().await;
        lenient
            .room
            .publish(
                &lenient.repo,
                &lenient.player,
                vec![op],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();

        let strict = movement_scene_partial_cell(/*lenient=*/ false).await;
        let op = strict.mv_to_partial_cell().await;
        let blocked = strict
            .room
            .publish(
                &strict.repo,
                &strict.player,
                vec![op],
                0,
                WriteOrigin::Client,
            )
            .await;
        assert!(matches!(blocked, Err(crate::data::DataError::Forbidden)));
    }

    /// The guard tests BOTH endpoints, not just the destination. A token whose COMMITTED position
    /// is already over the bound (a legacy/pre-existing state — `TokenEngine::validate` now
    /// closes every document-write path, GM included, so this can no longer arise from a live
    /// write) must not be moveable by a player even to an in-bound target: `a0` still feeds
    /// `blocks_move` and `line_traversal`, whose guarantees lapse beyond the bound. Without this
    /// case the `a0` disjuncts could be deleted with the suite still green.
    #[tokio::test]
    async fn publish_move_gate_rejects_an_over_magnitude_start_coordinate() {
        use crate::data::command::FieldChange;
        let h = movement_scene("unrestricted", /*with_light=*/ false).await;
        let over = crate::scene::move_exec::MAX_GATE_WALK_COORD + 1.0;

        // Seed the ECS's committed position directly (bypassing document-write ingress
        // validation entirely) to simulate a pre-existing out-of-bound token — the only way
        // such a position can exist now that `TokenEngine::validate` gates every write.
        // `apply_op` is the documented seam for reflecting an already-committed op into the
        // derived world (`scene::mod`'s doc comment), which is exactly what this simulates.
        let seed = h.mv_to(over, 50.0).await;
        h.room.scene().write().await.apply_op(&seed);

        // The player now moves to a perfectly ordinary in-bound destination. Only `a0` is over.
        let seq0 = h.room.current_seq();
        let op = Operation::Update {
            doc_id: h.token_id,
            changes: vec![
                FieldChange {
                    remove: false,
                    path: "/engine/x".into(),
                    old: serde_json::json!(over),
                    new: serde_json::json!(150.0),
                },
                FieldChange {
                    remove: false,
                    path: "/engine/y".into(),
                    old: serde_json::json!(50.0),
                    new: serde_json::json!(150.0),
                },
            ],
        };
        let blocked = h
            .room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await;
        assert!(
            matches!(blocked, Err(crate::data::DataError::Forbidden)),
            "an over-magnitude START coordinate must fail closed even with an in-bound destination"
        );
        assert_eq!(
            h.room.current_seq(),
            seq0,
            "rejected move consumes no seq (pre-write rejection)"
        );
    }

    #[tokio::test]
    async fn publish_move_gate_rejects_over_magnitude_coordinate_on_a_square_scene() {
        // The `publish` gate and `move_exec::gate_walk` must agree on which coordinates are
        // ADMISSIBLE, not only on which cells are visible. `unrestricted` is the discriminating
        // mode: the mask check is skipped there, so the ONLY thing that can reject this move is
        // the shared `MAX_GATE_WALK_COORD` bound.
        let h = movement_scene("unrestricted", /*with_light=*/ false).await;
        let seq0 = h.room.current_seq();

        let over = crate::scene::move_exec::MAX_GATE_WALK_COORD + 1.0;
        let op = h.mv_to(over, 50.0).await;
        let blocked = h
            .room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await;
        assert!(
            matches!(blocked, Err(crate::data::DataError::Forbidden)),
            "over-magnitude endpoint must fail closed at the publish gate"
        );
        assert_eq!(
            h.room.current_seq(),
            seq0,
            "rejected move consumes no seq (pre-write rejection)"
        );

        // Legitimate play is unaffected: an ordinary adjacent move still commits.
        let op = h.mv_to(h.adj.0, h.adj.1).await;
        h.room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await
            .unwrap();
        assert_eq!(h.room.current_seq(), seq0 + 1);
    }

    #[tokio::test]
    async fn publish_move_gate_rejects_over_magnitude_coordinate_on_a_hex_scene() {
        // Same bound on the hex path: the guard precedes `resolve_grid_shape`/`line_traversal`,
        // so it is grid-kind-independent by construction — pinned here rather than assumed.
        let h = movement_scene_hex("unrestricted", /*with_light=*/ false).await;
        let seq0 = h.room.current_seq();

        let over = crate::scene::move_exec::MAX_GATE_WALK_COORD + 1.0;
        let op = h.mv_to(50.0, over).await;
        let blocked = h
            .room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await;
        assert!(
            matches!(blocked, Err(crate::data::DataError::Forbidden)),
            "over-magnitude endpoint must fail closed on hex too"
        );
        assert_eq!(h.room.current_seq(), seq0);

        let op = h.mv_to(h.adj.0, h.adj.1).await;
        h.room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn publish_move_gate_admissibility_bound_equals_gate_walks() {
        // Anti-drift: the two gates share ONE constant and ONE comparison sense (strictly `>`).
        // Exercised behaviorally at the exact boundary. Both gates read the one shared symbol, so
        // changing its VALUE moves them together and correctly breaks nothing here; what this
        // detects is an edit that stops sharing the constant (one side hardcoding a literal), or
        // that flips `>` to `>=` on either side.
        use crate::scene::move_exec::{gate_walk, MAX_GATE_WALK_COORD};
        let cell = 100.0_f64;
        let at = MAX_GATE_WALK_COORD;
        let over = MAX_GATE_WALK_COORD + 1.0;

        // gate_walk side.
        assert!(gate_walk(&[(at - cell, 50.0), (at, 50.0)], cell).is_some());
        assert!(gate_walk(&[(over - cell, 50.0), (over, 50.0)], cell).is_none());

        // publish side, same two magnitudes, mask-free (`unrestricted`) scene.
        let h = movement_scene("unrestricted", /*with_light=*/ false).await;
        let op = h.mv_to(at, 50.0).await;
        h.room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await
            .expect("a coordinate exactly AT the bound is admissible on both gates");

        let op = h.mv_to(over, 50.0).await;
        let blocked = h
            .room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await;
        assert!(matches!(blocked, Err(crate::data::DataError::Forbidden)));
    }

    #[tokio::test]
    async fn movement_restriction_hex_grid_discriminates_on_hex_cells_not_square() {
        // Regression for the pre-fix gate: `Room::publish` called the SQUARE
        // `movement::supercover_cells` free function even on a hex scene, testing
        // square-indexed cells against a hex-indexed (`visible_cells_cached`) mask — two
        // incompatible coordinate systems. The fix routes the gate through the scene's own
        // `resolve_grid_shape(...).line_traversal(...)`, the same primitive
        // `move_exec::execute_move` already uses, so the two now agree on hex scenes too.
        //
        // Move geometry (hex size=100, light at (50,50), brightRadius=150, dimRadius=300):
        // the hex traversal of (50,50)->(250,50) is exactly {(0,0),(1,0)} — both within the
        // light's bright radius, so the move must be ALLOWED. The pre-fix square supercover
        // of the SAME segment is {(0,0),(1,0),(2,0)}: reinterpreting square cell (2,0) as a
        // HEX axial coordinate lands its center ~300.6 world units from the light (just past
        // dimRadius), so the pre-fix code rejects this exact move as Forbidden.
        let h = movement_scene_hex("visible", /*with_light=*/ true).await;
        let dest = (250.0, 50.0);

        let op = h.mv_to(dest.0, dest.1).await;
        h.room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await
            .expect("gate must allow a move whose HEX traversal is fully within the light");

        // GM bypasses the visibility gate (unchanged behavior, mirrors the square tests).
        let op = h.mv_to(2000.0, 2000.0).await;
        h.room
            .publish(&h.repo, &h.gm, vec![op], 0, WriteOrigin::Client)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn movement_restriction_hex_revealed_unions_hex_indexed_explored_memory() {
        // On a hex scene the explored set is HEX-axial-indexed (via `ExploredSet::mark_polygons`
        // routing through `GridShape`) and must compose with the `Revealed` gate's hex
        // `line_traversal` move-cells: an unseen hex cell seeded into explored via the HEX grid is
        // reachable, while a never-seen/never-explored hex cell is rejected.
        //
        // Discriminates hex vs square indexing: the explored corridor is seeded as five TIGHT 20×20
        // boxes, one around each hex center on the +r axial path (0,0)->(0,4). Under hex indexing
        // each box marks exactly that hex, so explored = {(0,0),(0,1),(0,2),(0,3),(0,4)}. A square
        // (floor(x/cell),floor(y/cell)) indexing of the same pixel centers yields
        // {(0,0),(0,1),(1,3),(2,4),(3,6)}, which lacks hex move-cells (0,2)/(0,3)/(0,4) — such a
        // set rejects this exact move, so a passing move proves the set is hex-indexed.
        let h = movement_scene_hex("revealed", /*with_light=*/ true).await;
        let cell = 100.0_f64;
        let grid = {
            let scene = h.room.scene().read().await;
            scene.resolve_grid_shape(h.scene_id, cell)
        };

        // Seed explored with one tight box per hex cell on the (0,0)->(0,4) axial path.
        let path_cells = [(0, 0), (0, 1), (0, 2), (0, 3), (0, 4)];
        let polys: Vec<Vec<f64>> = path_cells
            .iter()
            .map(|&c| {
                let (cx, cy) = grid.cell_center(c);
                vec![
                    cx - 10.0,
                    cy - 10.0,
                    cx + 10.0,
                    cy - 10.0,
                    cx + 10.0,
                    cy + 10.0,
                    cx - 10.0,
                    cy + 10.0,
                ]
            })
            .collect();
        let mut seed = crate::scene::explored::ExploredSet::new();
        seed.mark_polygons(&polys, grid.as_ref(), cell);
        assert_eq!(
            seed.len(),
            5,
            "each tight box marks exactly one hex axial cell under hex indexing"
        );
        for &c in &path_cells {
            assert!(
                seed.contains(c),
                "hex axial {c:?} must be in the seeded explored set"
            );
        }
        h.repo
            .set_explored(h.world_id, h.scene_id, h.player.user_id, &seed.to_bytes())
            .await
            .unwrap();

        // ALLOW: move into hex (0,4) — unseen (center well past the light's dimRadius) but explored.
        // Under Revealed, visible ∪ explored covers the whole hex traversal.
        let dest = grid.cell_center((0, 4));
        let op = h.mv_to(dest.0, dest.1).await;
        h.room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await
            .expect("Revealed must allow a move into a hex-indexed explored cell");

        // REJECT: from (0,4), move to a never-seen, never-explored, unlit cell — forbidden.
        let op = h.mv_to(9000.0, 9000.0).await;
        let blocked = h
            .room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await;
        assert!(
            matches!(blocked, Err(crate::data::DataError::Forbidden)),
            "a move into an unexplored + unseen hex cell must be rejected"
        );
    }

    /// SECRECY (reject) direction of the hex movement gate, and the one direction a square-indexed
    /// gate gets WRONG toward over-reveal: a move whose HEX traversal enters unseen hex cells, but
    /// whose SQUARE-indexed traversal of the same segment lies entirely inside the (hex-indexed)
    /// mask, must be Forbidden.
    ///
    /// Non-vacuity is asserted in-test, not merely argued: the test reads the gate's own mask off
    /// the ECS and pins BOTH halves — at least one hex traversal cell is outside it (so the correct
    /// gate rejects) AND every square supercover cell of the same segment is inside it (so a
    /// square-indexed gate would have allowed this exact move). The geometry works because the
    /// bounds-derived LOS rectangle is axis-aligned in PIXEL space while the mask is indexed in
    /// AXIAL space: toward -x/+y the hex preimage of that rectangle shears away from the square
    /// index rectangle, so square indices stay inside the mask where hex cells have already left it.
    #[tokio::test]
    async fn movement_restriction_hex_rejects_unseen_cell_a_square_indexed_gate_would_allow() {
        let h = movement_scene_hex_open("visible").await;
        let cell = 100.0_f64;
        let dest = (-200.0, 825.0);

        let (mask, hex_cells, square_cells) = {
            let scene = h.room.scene().read().await;
            let lenient = scene.resolve_scene(h.scene_id).partial_cell_leniency;
            let grid = scene.resolve_grid_shape(h.scene_id, cell);
            let mask = scene.visible_cells(h.player.user_id, h.scene_id, lenient);
            let hex_cells = grid
                .line_traversal(h.start, dest, cell)
                .expect("bounded hex traversal");
            let square_cells = crate::scene::movement::supercover_cells(h.start, dest, cell)
                .expect("bounded square supercover");
            (mask, hex_cells, square_cells)
        };

        assert!(
            hex_cells.iter().any(|c| !mask.contains(c)),
            "fixture must put at least one HEX traversal cell outside the mask: hex={hex_cells:?}"
        );
        assert!(
            square_cells.iter().all(|c| mask.contains(c)),
            "fixture must keep every SQUARE supercover cell inside the mask (otherwise the reject \
             below would also fire under square math and prove nothing): square={square_cells:?}"
        );

        let op = h.mv_to(dest.0, dest.1).await;
        let blocked = h
            .room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await;
        assert!(
            matches!(blocked, Err(crate::data::DataError::Forbidden)),
            "a move whose hex traversal enters unseen hex cells must be rejected"
        );
    }

    /// `Revealed` on hex, both directions, with the mask under exact test control: the scene is
    /// fully dark (`environment` intensity 0, no lights), so `visible_cells` is empty and the gate's
    /// mask IS the explored set. Seeding explored with the SQUARE supercover of the move (read as
    /// axial indices) must still reject — those cells are not the hex cells the move traverses —
    /// while seeding it with the HEX traversal allows. The reject half is the non-vacuous one: it is
    /// precisely the set a square-indexed gate would have checked and passed.
    #[tokio::test]
    async fn movement_restriction_hex_revealed_rejects_a_square_indexed_explored_corridor() {
        let h = movement_scene_hex("revealed", /*with_light=*/ false).await;
        let cell = 100.0_f64;
        let dest = (-200.0, 825.0);

        let (grid, hex_cells, square_cells) = {
            let scene = h.room.scene().read().await;
            assert!(
                scene
                    .visible_cells(h.player.user_id, h.scene_id, true)
                    .is_empty(),
                "a dark scene has an empty visible mask, so explored alone drives the gate"
            );
            let grid = scene.resolve_grid_shape(h.scene_id, cell);
            let hex_cells = grid.line_traversal(h.start, dest, cell).expect("bounded");
            let square_cells =
                crate::scene::movement::supercover_cells(h.start, dest, cell).expect("bounded");
            (grid, hex_cells, square_cells)
        };
        assert!(
            hex_cells.iter().any(|c| !square_cells.contains(c)),
            "fixture must have a hex traversal cell the square supercover omits"
        );

        // Seed explored from a set of cells by marking one tight box around each cell's HEX center.
        let seed_explored = |cells: Vec<(i32, i32)>| {
            let polys: Vec<Vec<f64>> = cells
                .iter()
                .map(|&c| {
                    let (cx, cy) = grid.cell_center(c);
                    vec![
                        cx - 10.0,
                        cy - 10.0,
                        cx + 10.0,
                        cy - 10.0,
                        cx + 10.0,
                        cy + 10.0,
                        cx - 10.0,
                        cy + 10.0,
                    ]
                })
                .collect();
            let mut set = crate::scene::explored::ExploredSet::new();
            set.mark_polygons(&polys, grid.as_ref(), cell);
            set
        };

        // REJECT: only the square-indexed corridor is explored.
        let seeded = seed_explored(square_cells.iter().copied().collect());
        h.repo
            .set_explored(h.world_id, h.scene_id, h.player.user_id, &seeded.to_bytes())
            .await
            .unwrap();
        let op = h.mv_to(dest.0, dest.1).await;
        let blocked = h
            .room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await;
        assert!(
            matches!(blocked, Err(crate::data::DataError::Forbidden)),
            "explored cells taken from the SQUARE supercover do not cover the hex traversal"
        );

        // ALLOW: the same move, with the hex traversal explored instead.
        let seeded = seed_explored(hex_cells.iter().copied().collect());
        h.repo
            .set_explored(h.world_id, h.scene_id, h.player.user_id, &seeded.to_bytes())
            .await
            .unwrap();
        let op = h.mv_to(dest.0, dest.1).await;
        h.room
            .publish(&h.repo, &h.player, vec![op], 0, WriteOrigin::Client)
            .await
            .expect("Revealed allows a move whose whole hex traversal is explored");
    }

    // -----------------------------------------------------------------------
    // M1: commit_ops_locked direct test — gate-free authoritative write path
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
    /// explicitly `"continuous"` (M10f-3 §6): proves `execute_move` gates an any-angle route
    /// from a scene genuinely marked continuous, not just incidentally sent a diagonal path.
    /// Functionally inert on the server today — `execute_move` has no `movementModel` branch
    /// (engine-agnostic since M10f-2); this mirrors `movement_scene`'s body (this file's
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
        // Proves the M10f-2 unified sampled executor gates a genuinely any-angle
        // (non-grid-aligned) polyline exactly like a grid path — no movementModel branch
        // anywhere on this path (M10f-3 §3.2). Goal (110,130) is a 3-4-5 triangle scaled ×20
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
