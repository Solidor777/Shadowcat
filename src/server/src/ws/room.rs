//! Per-world rooms, ring buffer, registry, and telemetry counters.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use serde::Serialize;
use tokio::sync::{broadcast, Mutex};
use ts_rs::TS;
use uuid::Uuid;

use crate::data::command::{Command, UnsequencedCommand};
use crate::data::repository::Repository;
use crate::data::DataError;
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
        Self { events: VecDeque::new() }
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
    pub stats: RoomStats,
}

impl Room {
    fn new(world_id: Uuid, seed_seq: i64) -> Self {
        let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            world_id,
            tx,
            ring: Mutex::new(RingBuffer::new()),
            publish_guard: Mutex::new(()),
            current_seq: AtomicI64::new(seed_seq),
            stats: RoomStats::default(),
        }
    }

    /// Subscribe to live frames; also returns the room's current seq so a joiner
    /// knows whether it needs to resync.
    pub fn subscribe(&self) -> (broadcast::Receiver<Arc<ServerMsg>>, i64) {
        (self.tx.subscribe(), self.current_seq.load(Ordering::Acquire))
    }

    pub fn current_seq(&self) -> i64 {
        self.current_seq.load(Ordering::Acquire)
    }

    /// Allocate seq (durable), append to the ring, and broadcast — serialized per
    /// world by `publish_guard` so broadcast order equals seq order. M4 publishes
    /// an empty-ops command; M5 supplies real ops on this same path.
    pub async fn publish(
        &self,
        repo: &dyn Repository,
        author: Uuid,
        ts: i64,
    ) -> Result<Command, DataError> {
        let _guard = self.publish_guard.lock().await;
        let cmd = repo
            .apply_command(UnsequencedCommand {
                world_id: self.world_id,
                author,
                ts,
                ops: vec![],
            })
            .await?;
        let msg = Arc::new(ServerMsg::Event { command: cmd.clone() });
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
            .map(|c| Arc::new(ServerMsg::Event { command: c }))
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
}

impl RoomRegistry {
    pub fn new() -> Self {
        Self { rooms: DashMap::new() }
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
        let room = self
            .rooms
            .entry(world_id)
            .or_insert_with(|| Arc::new(Room::new(world_id, world.seq)))
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
        self.rooms
            .remove_if(&world_id, |_, r| r.stats.connections.load(Ordering::Acquire) <= 0);
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
        assert!(rb.range_from(1).is_none(), "seq 1 evicted, range not fully resident");
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
    use crate::data::sqlite::SqliteRepository;
    use std::sync::atomic::Ordering;
    use uuid::Uuid;

    async fn repo_with_world() -> (SqliteRepository, Uuid, Uuid) {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let world = repo.create_world("W", 0).await.unwrap();
        let author = repo.create_user("a", None, ServerRole::User, 0).await.unwrap();
        (repo, world.id, author)
    }

    #[tokio::test]
    async fn publish_allocates_seq_buffers_and_broadcasts() {
        let (repo, world_id, author) = repo_with_world().await;
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let (mut rx, current) = room.subscribe();
        assert_eq!(current, 0);

        let cmd = room.publish(&repo, author, 10).await.unwrap();
        assert_eq!(cmd.seq, 1);
        assert_eq!(room.current_seq(), 1);

        let got = rx.recv().await.unwrap();
        assert_eq!(got.event_seq(), Some(1));
        assert_eq!(room.stats.events_published.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn get_or_create_returns_none_for_missing_world() {
        let (repo, _world_id, _author) = repo_with_world().await;
        let reg = RoomRegistry::new();
        assert!(reg
            .get_or_create(&repo, Uuid::from_u128(999))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn resync_hot_then_cold_tiers() {
        let (repo, world_id, author) = repo_with_world().await;
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        for _ in 0..3 {
            room.publish(&repo, author, 0).await.unwrap();
        } // seq 1,2,3

        // hot: from_seq 2 resident in buffer
        let (hot, src) = room.resync_range(&repo, 2).await.unwrap();
        assert_eq!(src, ResyncSource::Buffer);
        assert_eq!(
            hot.iter().map(|m| m.event_seq().unwrap()).collect::<Vec<_>>(),
            vec![2, 3]
        );
    }

    #[tokio::test]
    async fn publish_is_ordered_under_concurrency() {
        let (repo, world_id, author) = repo_with_world().await;
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let (mut rx, _) = room.subscribe();

        let repo = std::sync::Arc::new(repo);
        let mut handles = vec![];
        for _ in 0..50 {
            let room = room.clone();
            let repo = repo.clone();
            handles.push(tokio::spawn(async move {
                room.publish(repo.as_ref(), author, 0).await.unwrap();
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
        assert_eq!(seqs, sorted, "broadcast delivery order must equal seq order");
        assert_eq!(seqs, (1..=50).collect::<Vec<_>>());
    }
}
