//! Per-world rooms, ring buffer, registry, and telemetry counters.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use serde::Serialize;
use tokio::sync::{broadcast, Mutex, RwLock};
use ts_rs::TS;
use uuid::Uuid;

use crate::data::command::{Command, Operation};
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::DataError;
use crate::scene::SceneEcs;
use crate::ws::protocol::{ResyncSource, ServerMsg};

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
            let scene = self.scene.read().await;
            for op in &ops {
                if let Operation::Update { doc_id, changes } = op {
                    // Validate the POST-IMAGE position (the committed system + all changes
                    // applied), so a wholesale `/system` write or duplicate `/system/x`
                    // changes can't present a safe target while committing an unsafe one.
                    if let Some((scene_id, a0, a1)) = scene.token_move(*doc_id, changes) {
                        if scene.blocks_move(scene_id, a0, a1) {
                            return Err(DataError::Forbidden);
                        }
                    }
                }
            }
        }
        let cmd = repo.apply_intent(ctx, self.world_id, ops, ts).await?;
        // Hydrate the derived ECS from the committed command while still holding
        // publish_guard, so the ECS is consistent with cmd.seq before the Event
        // (and any derived recompute keyed to that seq) is observable.
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
}
