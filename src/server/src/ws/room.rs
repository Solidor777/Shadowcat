//! Per-world rooms, ring buffer, registry, and telemetry counters.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use serde::Serialize;
use tokio::sync::{broadcast, Mutex, RwLock};
use ts_rs::TS;
use uuid::Uuid;

use crate::data::command::{Command, FieldChange, Operation};
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::DataError;
use crate::scene::SceneEcs;
use crate::ws::protocol::{ResyncSource, ServerMsg};

/// The room-facing result of a server-authoritative token move: the stop cell, the legal
/// prefix of the path that was walked (render animation input), and the animation duration.
pub struct MoveExecution {
    /// The last successfully reached path coordinate (the committed position after the move).
    pub stop: (f64, f64),
    /// The legal prefix of the requested path including `start` through `stop`.
    pub render_path: Vec<(f64, f64)>,
    /// Animation duration in milliseconds (distance / cell / speed * 1000). Zero when stop == start.
    pub duration_ms: f64,
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
    /// `DataError` without consuming a seq or broadcasting.
    pub async fn publish(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        ops: Vec<Operation>,
        ts: i64,
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
                for op in &ops {
                    if let Operation::Update { doc_id, changes } = op {
                        // Validate the POST-IMAGE position (the committed system + all changes
                        // applied), so a wholesale `/system` write or duplicate `/system/x`
                        // changes can't present a safe target while committing an unsafe one.
                        if let Some((scene_id, a0, a1)) = scene.token_move(*doc_id, changes) {
                            // M9a wall gate (unchanged): a wall crossing short-circuits before
                            // any mask work.
                            if scene.blocks_move(scene_id, a0, a1) {
                                return Err(DataError::Forbidden);
                            }
                            // M10e-4 movement-restriction gate.
                            let settings = scene.resolve_scene(scene_id);
                            if matches!(
                                settings.movement_restriction,
                                crate::scene::MovementRestriction::Unrestricted
                            ) {
                                continue;
                            }
                            let cell = scene
                                .scene_grid_sizes()
                                .get(&scene_id)
                                .copied()
                                .unwrap_or(100.0);
                            // Supercover of the move segment; None ⇒ over-cap or degenerate
                            // grid → fail closed (DoS guard, spec §8).
                            let Some(move_cells) =
                                crate::scene::movement::supercover_cells(a0, a1, cell)
                            else {
                                return Err(DataError::Forbidden);
                            };
                            let lenient = settings.partial_cell_leniency;
                            let visible = visible_cache
                                .entry((scene_id, lenient))
                                .or_insert_with(|| {
                                    scene.visible_cells(ctx.user_id, scene_id, lenient)
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
        return self.commit_ops_locked(repo, ctx, ops, ts).await;
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
    ) -> Result<Command, DataError> {
        let cmd = repo.apply_intent(ctx, self.world_id, ops, ts).await?;
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
    pub async fn execute_move(
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
        let visible_cells;
        let is_revealed;
        {
            let scene = self.scene.read().await;

            // Verify the token exists and get its committed position.
            start = scene.token_position(token).ok_or(DataError::Forbidden)?;

            let settings = scene.resolve_scene(scene_id);
            cell = scene
                .scene_grid_sizes()
                .get(&scene_id)
                .copied()
                .unwrap_or(100.0);

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
                scene.visible_cells(ctx.user_id, scene_id, lenient)
            };
        } // scene read guard dropped here — safe to await (publish_guard still held)

        // --- Revealed union: fetch explored AFTER dropping the scene read guard ---
        // INVARIANT (spec §13): for Revealed the `visible` set passed to execute_move MUST be
        // visible_cells ∪ explored. Fail-closed: error or missing blob → empty explored set
        // (falls back to visible-only, which is stricter but safe).
        let visible = if is_revealed {
            let mut union = visible_cells;
            let explored = match repo.get_explored(scene_id, ctx.user_id).await {
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

        // --- Pure path executor + animation speed (single ECS read acquisition) ---
        // Re-acquire the read lock now that the explored await is complete. Hold it only for
        // the synchronous executor call and the animation speed read, then drop before
        // commit_ops_locked (no lock-across-await on the write path; publish_guard already held).
        // Maps MoveReject → DataError::Forbidden (all reject reasons indicate the request
        // is invalid: unknown token, too-long path, bad start, non-adjacent step).
        let outcome;
        let speed_cells_per_sec;
        {
            let scene = self.scene.read().await;
            outcome = move_exec::execute_move(
                &scene,
                scene_id,
                token,
                &path,
                restriction,
                &visible,
                cell,
            )
            .map_err(|_| DataError::Forbidden)?;
            speed_cells_per_sec = scene.resolved_animation_speed();
        } // scene read lock dropped — commit_ops_locked awaits safely under publish_guard

        let distance: f64 = outcome
            .render_path
            .windows(2)
            .map(|w| {
                let dx = w[1].0 - w[0].0;
                let dy = w[1].1 - w[0].1;
                (dx * dx + dy * dy).sqrt()
            })
            .sum();

        let duration_ms = if distance < 1e-9 {
            0.0
        } else {
            (distance / cell) / speed_cells_per_sec * 1000.0
        };

        // Zero-progress move (stop == start): return immediately without writing.
        // Invariant: render_path always contains at least `start` (path.len() >= 2 was
        // validated by execute_move), so this only fires when the very first step was blocked.
        if (outcome.stop.0 - start.0).abs() < 1e-9 && (outcome.stop.1 - start.1).abs() < 1e-9 {
            return Ok(MoveExecution {
                stop: start,
                render_path: vec![start],
                duration_ms: 0.0,
            });
        }

        // --- Atomic commit (publish_guard already held — single acquisition, no re-entry) ---
        // PRECONDITION: commit_ops_locked requires the caller to hold publish_guard for its
        // full duration. The guard was acquired at the top of this function and is still held
        // here — no re-acquisition needed or allowed (tokio Mutex is non-reentrant; re-acquiring
        // would deadlock). The position ops mirror the field paths that `token_move` / `publish`
        // write (/system/x and /system/y), keyed on the authoritative ECS-read old values so the
        // optimistic-concurrency check in apply_intent passes as defense-in-depth.
        let pos_ops = vec![Operation::Update {
            doc_id: token,
            changes: vec![
                FieldChange {
                    path: "/system/x".into(),
                    old: serde_json::json!(start.0),
                    new: serde_json::json!(outcome.stop.0),
                },
                FieldChange {
                    path: "/system/y".into(),
                    old: serde_json::json!(start.1),
                    new: serde_json::json!(outcome.stop.1),
                },
            ],
        }];

        self.commit_ops_locked(repo, ctx, pos_ops, ts).await?;

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
            stop: outcome.stop,
            render_path: outcome.render_path,
            duration_ms,
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
    /// Broadcast ring capacity for rooms created by this registry. Production uses
    /// `BROADCAST_CAPACITY`; test harnesses shrink it to force the lag path.
    broadcast_capacity: usize,
}

impl RoomRegistry {
    pub fn new() -> Self {
        Self {
            rooms: DashMap::new(),
            broadcast_capacity: BROADCAST_CAPACITY,
        }
    }

    /// A registry whose rooms use a custom broadcast ring capacity. Test-only: a
    /// tiny capacity lets a non-reading client deterministically overflow the ring
    /// and exercise the `Lagged` → resync path.
    pub fn with_capacity(broadcast_capacity: usize) -> Self {
        Self {
            rooms: DashMap::new(),
            broadcast_capacity,
        }
    }

    /// Get the room for an existing world, creating it (seeded from the world's
    /// current seq) on first join. `None` when the world does not exist.
    pub async fn get_or_create(
        &self,
        repo: &dyn Repository,
        world_id: Uuid,
    ) -> Result<Option<Arc<Room>>, DataError> {
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
        // TODO: batch these four query_documents calls into one WHERE doc_type IN (...) query
        // to halve the DB round-trips on cold room creation.
        let world_settings = repo
            .query_documents(world_id, "world-settings")
            .await?
            .into_iter()
            .next();
        let gradation = repo
            .query_documents(world_id, "light-gradation")
            .await?
            .into_iter()
            .next();
        let vision_modes = repo
            .query_documents(world_id, "vision-modes")
            .await?
            .into_iter()
            .next();
        scene_ecs.set_world_config(world_settings, gradation, vision_modes);
        scene_ecs.set_actors(repo.query_documents(world_id, "actor").await?);
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
    async fn publish_hydrates_scene_ecs() {
        let (repo, world_id, ctx) = repo_with_world().await;
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        assert_eq!(room.scene().read().await.entity_count(), 0);

        // Publish a scene doc (a scene entity by doc_type, no parent FK needed).
        let mut scene =
            crate::data::document::tests::world_scoped_doc(world_id, Uuid::from_u128(20), "scene");
        scene.owner = Some(ctx.user_id);
        room.publish(&repo, &ctx, vec![Operation::Create { doc: scene }], 0)
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
        room.publish(&repo, &gm, vec![Operation::Create { doc: ws }], 0)
            .await
            .unwrap();

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm.user_id);
        room.publish(&repo, &gm, vec![Operation::Create { doc: scene }], 0)
            .await
            .unwrap();

        // Token owned (writable) by the player, at (0,0).
        let mut token = wdoc(world_id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.system = json!({ "x": 0, "y": 0 });
        room.publish(&repo, &gm, vec![Operation::Create { doc: token }], 0)
            .await
            .unwrap();

        // A blocksMove wall on the diagonal x+y=10.
        let mut wall = wdoc(world_id, wall_id, "wall");
        wall.parent_id = Some(scene_id);
        wall.owner = Some(gm.user_id);
        wall.system =
            json!({ "seg": { "x1": 0, "y1": 10, "x2": 10, "y2": 0 }, "blocksMove": true });
        room.publish(&repo, &gm, vec![Operation::Create { doc: wall }], 0)
            .await
            .unwrap();

        let mv = |nx: i64, ny: i64, ox: i64, oy: i64| Operation::Update {
            doc_id: token_id,
            changes: vec![
                FieldChange {
                    path: "/system/x".into(),
                    old: json!(ox),
                    new: json!(nx),
                },
                FieldChange {
                    path: "/system/y".into(),
                    old: json!(oy),
                    new: json!(ny),
                },
            ],
        };

        let seq_before = room.current_seq();
        // Forged bypass A: a single wholesale `/system` write that relocates the token past the
        // wall must be caught (the post-image, not a leaf-path match, is validated).
        let whole = Operation::Update {
            doc_id: token_id,
            changes: vec![FieldChange {
                path: "/system".into(),
                old: json!({ "x": 0, "y": 0 }),
                new: json!({ "x": 10, "y": 10 }),
            }],
        };
        assert!(matches!(
            room.publish(&repo, &player, vec![whole], 0).await,
            Err(crate::data::DataError::Forbidden)
        ));
        assert_eq!(room.current_seq(), seq_before);
        // Forged bypass B: duplicate `/system/x` (safe-then-unsafe) — last write wins, so the
        // committed x=11 crosses; the gate validates against that, not the first change.
        let dup = Operation::Update {
            doc_id: token_id,
            changes: vec![
                FieldChange {
                    path: "/system/x".into(),
                    old: json!(0),
                    new: json!(1),
                },
                FieldChange {
                    path: "/system/x".into(),
                    old: json!(0),
                    new: json!(11),
                },
            ],
        };
        assert!(matches!(
            room.publish(&repo, &player, vec![dup], 0).await,
            Err(crate::data::DataError::Forbidden)
        ));
        assert_eq!(room.current_seq(), seq_before);

        // Player move (0,0)->(10,10) crosses the wall → rejected before the write.
        let blocked = room
            .publish(&repo, &player, vec![mv(10, 10, 0, 0)], 0)
            .await;
        assert!(matches!(blocked, Err(crate::data::DataError::Forbidden)));
        assert_eq!(
            room.current_seq(),
            seq_before,
            "a blocked move consumes no seq"
        );

        // The same player move that does NOT cross is allowed (so the block above was the
        // collision gate, not an authorization failure).
        room.publish(&repo, &player, vec![mv(1, 1, 0, 0)], 0)
            .await
            .unwrap();
        assert_eq!(room.current_seq(), seq_before + 1);

        // A GM move across the wall bypasses the collision gate (the "ignore walls" override).
        room.publish(&repo, &gm, vec![mv(10, 10, 1, 1)], 0)
            .await
            .unwrap();
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
        room1
            .publish(&repo, &gm, vec![Operation::Create { doc: ws }], 0)
            .await
            .unwrap();

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm.user_id);
        scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
        room1
            .publish(&repo, &gm, vec![Operation::Create { doc: scene }], 0)
            .await
            .unwrap();

        let mut token = wdoc(world_id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.system = json!({ "x": 50, "y": 50 });
        room1
            .publish(&repo, &gm, vec![Operation::Create { doc: token }], 0)
            .await
            .unwrap();

        let mut light = wdoc(world_id, light_id, "light");
        light.parent_id = Some(scene_id);
        light.owner = Some(gm.user_id);
        light.system = json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true
        });
        room1
            .publish(&repo, &gm, vec![Operation::Create { doc: light }], 0)
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
    async fn publish_allocates_seq_buffers_and_broadcasts() {
        let (repo, world_id, ctx) = repo_with_world().await;
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let (mut rx, current) = room.subscribe();
        assert_eq!(current, 0);

        let cmd = room.publish(&repo, &ctx, vec![], 10).await.unwrap();
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
            room.publish(&repo, &ctx, vec![], 0).await.unwrap();
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
                room.publish(repo.as_ref(), &ctx, vec![], 0).await.unwrap();
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
                        path: "/system/x".into(),
                        old: serde_json::json!(ox),
                        new: serde_json::json!(x),
                    },
                    FieldChange {
                        path: "/system/y".into(),
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
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        room.publish(&repo, &gm, vec![Operation::Create { doc: ws }], 0)
            .await
            .unwrap();

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm.user_id);
        scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
        room.publish(&repo, &gm, vec![Operation::Create { doc: scene }], 0)
            .await
            .unwrap();

        let mut token = wdoc(world_id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.system = json!({ "x": 50.0, "y": 50.0 });
        room.publish(&repo, &gm, vec![Operation::Create { doc: token }], 0)
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
            room.publish(&repo, &gm, vec![Operation::Create { doc: light }], 0)
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
        room.publish(&repo, &gm, vec![Operation::Create { doc: ws }], 0)
            .await
            .unwrap();

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm.user_id);
        scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
        room.publish(&repo, &gm, vec![Operation::Create { doc: scene }], 0)
            .await
            .unwrap();

        let mut token = wdoc(world_id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.system = json!({ "x": 50.0, "y": 50.0 });
        room.publish(&repo, &gm, vec![Operation::Create { doc: token }], 0)
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
        room.publish(&repo, &gm, vec![Operation::Create { doc: l1 }], 0)
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
        room.publish(&repo, &gm, vec![Operation::Create { doc: l2 }], 0)
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
        room.publish(&repo, &gm, vec![Operation::Create { doc: ws }], 0)
            .await
            .unwrap();

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm.user_id);
        scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
        room.publish(&repo, &gm, vec![Operation::Create { doc: scene }], 0)
            .await
            .unwrap();

        let mut token = wdoc(world_id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.system = json!({ "x": 50.0, "y": 50.0 });
        room.publish(&repo, &gm, vec![Operation::Create { doc: token }], 0)
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
        room.publish(&repo, &gm, vec![Operation::Create { doc: light }], 0)
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
        let blocked = h.room.publish(&h.repo, &h.player, vec![op], 0).await;
        assert!(matches!(blocked, Err(crate::data::DataError::Forbidden)));
        assert_eq!(h.room.current_seq(), seq0, "blocked move consumes no seq");

        let op = h.mv_to(60.0, 60.0).await;
        h.room
            .publish(&h.repo, &h.player, vec![op], 0)
            .await
            .unwrap();
        assert_eq!(h.room.current_seq(), seq0 + 1);

        // GM bypasses the visibility gate — token is now at (60,60) in ECS.
        let op = h.mv_to(2000.0, 2000.0).await;
        h.room.publish(&h.repo, &h.gm, vec![op], 0).await.unwrap();
    }

    #[tokio::test]
    async fn movement_restriction_unrestricted_allows_move_into_darkness() {
        // Unrestricted: only the M9a wall gate applies; a non-wall-crossing move into
        // an unlit cell is allowed regardless of visibility.
        let h = movement_scene("unrestricted", /*with_light=*/ false).await;
        let op = h.mv_to(2000.0, 2000.0).await;
        h.room
            .publish(&h.repo, &h.player, vec![op], 0)
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
            cell,
        );
        h.repo
            .set_explored(h.world_id, h.scene_id, h.player.user_id, &seed.to_bytes())
            .await
            .unwrap();

        // Move to center of explored cell (5,5) — allowed via explored memory.
        let op = h.mv_to(550.0, 550.0).await;
        h.room
            .publish(&h.repo, &h.player, vec![op], 0)
            .await
            .unwrap();

        // Move from (550,550) to a never-seen, never-explored, unlit cell — forbidden.
        let op = h.mv_to(9000.0, 9000.0).await;
        let blocked = h.room.publish(&h.repo, &h.player, vec![op], 0).await;
        assert!(matches!(blocked, Err(crate::data::DataError::Forbidden)));
    }

    #[tokio::test]
    async fn movement_restriction_checks_entire_move_not_just_endpoint() {
        // Supercover gate: a move whose endpoint is in the far lit pocket but whose
        // path traverses a dark gap between the two pockets must be rejected.
        let h = movement_scene_two_lit_pockets().await;
        let op = h.mv_to(950.0, 950.0).await;
        let blocked = h.room.publish(&h.repo, &h.player, vec![op], 0).await;
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
            .publish(&lenient.repo, &lenient.player, vec![op], 0)
            .await
            .unwrap();

        let strict = movement_scene_partial_cell(/*lenient=*/ false).await;
        let op = strict.mv_to_partial_cell().await;
        let blocked = strict
            .room
            .publish(&strict.repo, &strict.player, vec![op], 0)
            .await;
        assert!(matches!(blocked, Err(crate::data::DataError::Forbidden)));
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
            .commit_ops_locked(&repo, &ctx, vec![op], 10)
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
        room.publish(&repo, &gm, vec![Operation::Create { doc: ws }], 0)
            .await
            .unwrap();

        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm.user_id);
        scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
        room.publish(&repo, &gm, vec![Operation::Create { doc: scene }], 0)
            .await
            .unwrap();

        let mut token = wdoc(world_id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.permissions.users.insert(p, DocRole::Owner);
        token.system = json!({ "x": 50.0, "y": 50.0 });
        room.publish(&repo, &gm, vec![Operation::Create { doc: token }], 0)
            .await
            .unwrap();

        // Horizontal wall at y=100, x ∈ [100,200]. Blocks vertical step (150,50)→(150,150).
        let mut wall = wdoc(world_id, wall_id, "wall");
        wall.parent_id = Some(scene_id);
        wall.owner = Some(gm.user_id);
        wall.system =
            json!({ "seg": { "x1": 100, "y1": 100, "x2": 200, "y2": 100 }, "blocksMove": true });
        room.publish(&repo, &gm, vec![Operation::Create { doc: wall }], 0)
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

    #[tokio::test]
    async fn execute_move_rejects_a_moving_token() {
        // First execute_move succeeds and stamps the moving lock (end epoch in the future).
        // An immediate second call on the same token must be Forbidden while the lock is held.
        let h = movement_scene("unrestricted", false).await;
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
}
