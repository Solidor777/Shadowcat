# M4 WebSocket Event Bus Implementation Plan

> **For agentic workers:** This plan is executed via the `mainline-plan-execution` skill (per user-scope guidance for Fable-class models) — tasks run inline in-session with a per-task inline spec-compliance check and ONE dispatched fresh-context branch review at the end. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the live realtime transport on top of M2's durable `world_events` log — a session-gated WebSocket bus with per-world rooms, ordered sequenced broadcasts, ring-buffer + log-backed resync, a server time source, telemetry, and a desync-convergence test harness.

**Architecture:** A `RoomRegistry` (`DashMap<WorldId, Arc<Room>>`) is the stable abstraction boundary. Each `Room` fans out via `tokio::sync::broadcast` (lossy; `Lagged` drives resync), holds an in-memory ring buffer, and serializes its publish path (allocate-seq → ring → send) under a per-world guard so broadcast order equals seq order. M4 carries a generic empty-ops command through the existing `apply_command` path; M5's real document commands reuse the same frame.

**Tech Stack:** Rust, axum 0.8 (`ws`), tokio (`broadcast`/`Mutex`/`time`), dashmap, ts-rs, sqlx/SQLite. Tests: tokio-tungstenite real WS clients.

## Global Constraints

- **Permissive licenses only** (ARCHITECTURE §2.9): new deps `dashmap` (MIT), `futures-util` (MIT/Apache), dev `tokio-tungstenite` (MIT/Apache). No GPL/AGPL/SSPL.
- **Single crate** `shadowcat`; modules under `src/server/src/`.
- **Source under `src/`**, build output in `dist/`.
- **ts-rs bindings emit to `src/types/generated/`, CI-enforced in sync** (`cargo test` regenerates; `git diff --exit-code src/types/generated`).
- **No debug code in release** — diagnostics via `tracing` levels only; no `dbg!`/`println!`.
- **Server-authoritative + ordered realtime** (invariants #1, #2): every broadcast carries the per-world monotonic seq; broadcast delivery order must equal seq order.
- **Cross-platform from day one**: portable paths (`std::path`), no OS-specific code.
- **Wire format:** JSON text frames, internally-tagged enums (`#[serde(tag = "type", rename_all = "snake_case")]`).
- **Commit message trailers** (every commit):
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Htozbntnxh8N3meNWAeoNp
  ```

## File Structure

| File | Responsibility |
|---|---|
| `src/server/Cargo.toml` (modify) | Add `dashmap`, `futures-util`, tokio `sync`+`time` features; dev `tokio-tungstenite`. |
| `src/server/src/lib.rs` (modify) | `pub mod ws;` |
| `src/server/src/data/repository.rs` (modify) | Add `get_world` to the `Repository` trait. |
| `src/server/src/data/sqlite.rs` (modify) | Move `get_world` into `impl Repository`. |
| `src/server/src/data/command.rs` (modify) | `#[derive(TS)] #[ts(export)]` on `Command`, `Operation`, `FieldChange`. |
| `src/server/src/data/document.rs` (modify) | Same derives on `Document`, `Scope`, `Source`, `PermissionSet`, `DocRole`, `Visibility`, `WorldRole`. |
| `src/server/src/ws/mod.rs` (create) | Re-exports; `WsState` = `RoomRegistry` handle. |
| `src/server/src/ws/protocol.rs` (create) | `ClientMsg`, `ServerMsg`, `ResyncSource`, `WsErrorCode` (+ serde/ts-rs). |
| `src/server/src/ws/room.rs` (create) | `RingBuffer`, `RoomStats`, `Room`, `RoomRegistry`, `RoomStatsSnapshot`. |
| `src/server/src/ws/time.rs` (create) | `now_millis`, `calibrate`. |
| `src/server/src/ws/conn.rs` (create) | `ws_handler` upgrade + per-connection ingress/egress tasks + sequence guard. |
| `src/server/src/http/mod.rs` (modify) | `ws` field in `AppState`; `/ws` + `/api/debug/rooms` routes. |
| `src/server/src/http/routes.rs` (modify) | `debug_rooms` handler (AdminUser-gated). |
| `src/server/src/bin/test_server.rs` (create) | Spawnable throwaway server (in-memory DB, seeded user + world). |
| `src/server/tests/ws_convergence.rs` (create) | Desync-convergence harness (real WS clients, faults). |

---

### Task 1: Dependencies + `ws` module skeleton

**Files:**
- Modify: `src/server/Cargo.toml`
- Modify: `src/server/src/lib.rs`
- Create: `src/server/src/ws/mod.rs`

**Interfaces:**
- Produces: `shadowcat::ws` module compiles (empty submodules declared in later tasks).

- [ ] **Step 1: Add dependencies.** In `src/server/Cargo.toml`, change the `tokio` line and add two deps:

```toml
tokio = { version = "1.52", features = ["macros", "rt-multi-thread", "sync", "time"] }
dashmap = "6"
futures-util = "0.3"
```

Under `[dev-dependencies]` add:

```toml
tokio-tungstenite = "0.24"
```

And ensure `axum` enables websockets (the `ws` feature ships in the default feature set of axum 0.8; if the build later errors on `axum::extract::ws`, set `axum = { version = "0.8", features = ["ws"] }`).

- [ ] **Step 2: Declare the module.** In `src/server/src/lib.rs` add `pub mod ws;` (keep alphabetical-ish order, after `pub mod http;`).

- [ ] **Step 3: Create `src/server/src/ws/mod.rs`:**

```rust
//! Realtime WebSocket event bus: per-world rooms, sequenced broadcasts,
//! ring-buffer + log-backed resync, a server time source, and telemetry.

pub mod conn;
pub mod protocol;
pub mod room;
pub mod time;

pub use room::RoomRegistry;
```

(The `conn`, `protocol`, `room`, `time` files are created in later tasks; this step will not compile until Task 2+. To keep the tree compiling now, create the four files as empty stubs: `room.rs`, `protocol.rs`, `time.rs`, `conn.rs` each containing only a `//! stub` line.)

- [ ] **Step 4: Verify it compiles.** Run: `cargo build -p shadowcat`
  Expected: builds clean (warnings about unused empty modules are acceptable at this stage).

- [ ] **Step 5: Commit.**

```bash
git add src/server/Cargo.toml src/server/src/lib.rs src/server/src/ws/
git commit -m "feat(m4): scaffold ws module + add broadcast/dashmap deps"
```

---

### Task 2: ts-rs derives across the command/document tree

**Files:**
- Modify: `src/server/src/data/command.rs`
- Modify: `src/server/src/data/document.rs`
- Test: existing `cargo test` regenerates bindings; CI sync check covers it.

**Interfaces:**
- Produces: `Command`, `Operation`, `FieldChange`, `Document`, `Scope`, `Source`, `PermissionSet`, `DocRole`, `Visibility`, `WorldRole` all `impl TS` and export to `src/types/generated/`.

- [ ] **Step 1: Add derives in `document.rs`.** Add `use ts_rs::TS;` and to each of `Scope`, `Source`, `DocRole`, `Visibility`, `WorldRole`, `PermissionSet`, `Document`, add `TS` to the derive list and `#[ts(export)]` above the struct/enum. Example for `Document`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct Document { /* unchanged fields */ }
```

For `system: serde_json::Value`, annotate the field so ts-rs emits a usable type:

```rust
    #[ts(type = "unknown")]
    pub system: serde_json::Value,
```

For `BTreeMap<Uuid, DocRole>` / `BTreeMap<String, Visibility>` / `BTreeMap<String, Vec<Document>>`, ts-rs maps these to `Record<...>` automatically — no annotation needed.

- [ ] **Step 2: Add derives in `command.rs`.** Add `use ts_rs::TS;` and `TS` + `#[ts(export)]` to `FieldChange`, `Operation`, `Command`. (Leave `UnsequencedCommand` without `TS` — it is server-internal and never crosses the wire.)

- [ ] **Step 3: Regenerate and verify.** Run: `cargo test -p shadowcat`
  Expected: PASS; new files appear under `src/types/generated/` (e.g. `Document.ts`, `Command.ts`, `Operation.ts`, `FieldChange.ts`, `Scope.ts`, `Source.ts`, `PermissionSet.ts`, `DocRole.ts`, `Visibility.ts`, `WorldRole.ts`).

- [ ] **Step 4: Confirm bindings exist.** Run: `git status --short src/types/generated`
  Expected: the new `.ts` files listed as untracked/added.

- [ ] **Step 5: Commit.**

```bash
git add src/server/src/data/command.rs src/server/src/data/document.rs src/types/generated
git commit -m "feat(m4): ts-rs derives across the command/document tree"
```

---

### Task 3: `RingBuffer`

**Files:**
- Modify: `src/server/src/ws/room.rs`
- Modify: `src/server/src/ws/protocol.rs` (minimal `ServerMsg` enough for the buffer — full protocol in Task 4)

**Interfaces:**
- Consumes: `ServerMsg::Event { command: Command }` with `command.seq`, `command.ts`.
- Produces: `RingBuffer::new()`, `push(&mut self, Arc<ServerMsg>)`, `range_from(&self, i64) -> Option<Vec<Arc<ServerMsg>>>`; constants `MAX_EVENTS = 1024`, `MAX_AGE_MS = 300_000`.

> Note: Task 4 writes the full `protocol.rs`. To let Task 3 build and test first, this task adds a minimal `ServerMsg` with just the `Event` variant plus the `event_seq`/`event_ts` helpers; Task 4 expands the enum (the helpers stay).

- [ ] **Step 1: Minimal `ServerMsg` + helpers in `protocol.rs`:**

```rust
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use crate::data::command::Command;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Event { command: Command },
}

impl ServerMsg {
    /// seq of an `Event` frame, else `None`. Only `Event`s are buffered.
    pub fn event_seq(&self) -> Option<i64> {
        match self {
            ServerMsg::Event { command } => Some(command.seq),
        }
    }
    /// server-stamped ts of an `Event` frame, else `None`.
    pub fn event_ts(&self) -> Option<i64> {
        match self {
            ServerMsg::Event { command } => Some(command.ts),
        }
    }
}
```

- [ ] **Step 2: Write the failing tests in `room.rs`:**

```rust
//! Per-world rooms, ring buffer, registry, and telemetry counters.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::ws::protocol::ServerMsg;

const MAX_EVENTS: usize = 1024;
const MAX_AGE_MS: i64 = 5 * 60 * 1000;

#[cfg(test)]
mod ring_tests {
    use super::*;
    use crate::data::command::Command;
    use uuid::Uuid;

    fn event(seq: i64, ts: i64) -> Arc<ServerMsg> {
        Arc::new(ServerMsg::Event {
            command: Command { seq, world_id: Uuid::from_u128(1), author: Uuid::from_u128(2), ts, ops: vec![] },
        })
    }

    #[test]
    fn evicts_by_count() {
        let mut rb = RingBuffer::new();
        for s in 1..=(MAX_EVENTS as i64 + 10) {
            rb.push(event(s, 0));
        }
        let all = rb.range_from(1).unwrap();
        assert_eq!(all.len(), MAX_EVENTS);
        assert_eq!(all.first().unwrap().event_seq().unwrap(), 11); // oldest 10 evicted
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
        for s in 1..=5 { rb.push(event(s, 0)); }
        let r = rb.range_from(3).unwrap();
        assert_eq!(r.iter().map(|m| m.event_seq().unwrap()).collect::<Vec<_>>(), vec![3, 4, 5]);
    }

    #[test]
    fn range_from_none_when_requested_seq_evicted() {
        let mut rb = RingBuffer::new();
        for s in 1..=(MAX_EVENTS as i64 + 5) { rb.push(event(s, 0)); }
        // oldest resident is 6; asking from 1 cannot be fully served from buffer.
        assert!(rb.range_from(1).is_none());
    }

    #[test]
    fn range_from_none_on_empty_buffer() {
        let rb = RingBuffer::new();
        assert!(rb.range_from(1).is_none());
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail.** Run: `cargo test -p shadowcat ring_tests`
  Expected: FAIL (`RingBuffer` not found).

- [ ] **Step 4: Implement `RingBuffer` in `room.rs`** (above the test module):

```rust
/// Recent `Event` frames for hot resync, bounded by count and age.
/// Age is measured relative to the newest buffered event's `ts`.
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

    /// Events with `seq >= from_seq`, but only if the whole requested range is
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
```

- [ ] **Step 5: Run tests to verify they pass.** Run: `cargo test -p shadowcat ring_tests`
  Expected: PASS (5 tests).

- [ ] **Step 6: Commit.**

```bash
git add src/server/src/ws/room.rs src/server/src/ws/protocol.rs src/types/generated
git commit -m "feat(m4): ring buffer with count+age eviction and residency-checked range_from"
```

---

### Task 4: Wire protocol (`ClientMsg` / `ServerMsg`)

**Files:**
- Modify: `src/server/src/ws/protocol.rs`

**Interfaces:**
- Produces: `ClientMsg` (`Hello`, `EmitTest`, `ResyncRequest`, `TimePing`, `Pong`), `ServerMsg` (`Welcome`, `Event`, `ResyncBegin`, `ResyncEnd`, `TimePong`, `Ping`, `Error`), `ResyncSource` (`Buffer`/`Log`), `WsErrorCode` (`WorldNotFound`/`BadMessage`/`PublishFailed`/`Internal`). `event_seq`/`event_ts` helpers retained.

- [ ] **Step 1: Write failing serde round-trip tests** at the bottom of `protocol.rs`:

```rust
#[cfg(test)]
mod protocol_tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn client_hello_round_trips_and_is_tagged() {
        let m = ClientMsg::Hello { world: Uuid::from_u128(7), last_seq: Some(3) };
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("\"type\":\"hello\""));
        let back: ClientMsg = serde_json::from_str(&s).unwrap();
        assert!(matches!(back, ClientMsg::Hello { last_seq: Some(3), .. }));
    }

    #[test]
    fn server_event_and_resync_round_trip() {
        let begin = ServerMsg::ResyncBegin { from_seq: 2, to_seq: 5, source: ResyncSource::Buffer };
        let s = serde_json::to_string(&begin).unwrap();
        assert!(s.contains("\"type\":\"resync_begin\""));
        assert!(s.contains("\"source\":\"buffer\""));
        let _back: ServerMsg = serde_json::from_str(&s).unwrap();
    }

    #[test]
    fn error_code_serializes_snake_case() {
        let e = ServerMsg::Error { code: WsErrorCode::WorldNotFound, message: "x".into() };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"code\":\"world_not_found\""));
    }
}
```

- [ ] **Step 2: Run to verify failure.** Run: `cargo test -p shadowcat protocol_tests`
  Expected: FAIL (`ClientMsg` / variants not found).

- [ ] **Step 3: Replace the minimal `ServerMsg` with the full protocol** (keep the `event_seq`/`event_ts` impl):

```rust
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello { world: Uuid, last_seq: Option<i64> },
    EmitTest { nonce: u64 },
    ResyncRequest { from_seq: i64 },
    TimePing { client_t0: i64 },
    Pong,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ResyncSource {
    Buffer,
    Log,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum WsErrorCode {
    WorldNotFound,
    BadMessage,
    PublishFailed,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Welcome { world: Uuid, current_seq: i64, server_time: i64 },
    Event { command: Command },
    ResyncBegin { from_seq: i64, to_seq: i64, source: ResyncSource },
    ResyncEnd { current_seq: i64 },
    TimePong { client_t0: i64, server_t: i64 },
    Ping,
    Error { code: WsErrorCode, message: String },
}
```

The `event_seq`/`event_ts` `impl ServerMsg` block now matches only the `Event` arm and returns `None` for the rest:

```rust
impl ServerMsg {
    pub fn event_seq(&self) -> Option<i64> {
        match self {
            ServerMsg::Event { command } => Some(command.seq),
            _ => None,
        }
    }
    pub fn event_ts(&self) -> Option<i64> {
        match self {
            ServerMsg::Event { command } => Some(command.ts),
            _ => None,
        }
    }
}
```

- [ ] **Step 4: Run protocol + ring tests + regenerate bindings.** Run: `cargo test -p shadowcat`
  Expected: PASS; new `ClientMsg.ts`, `ServerMsg.ts`, `ResyncSource.ts`, `WsErrorCode.ts` under `src/types/generated/`.

- [ ] **Step 5: Commit.**

```bash
git add src/server/src/ws/protocol.rs src/types/generated
git commit -m "feat(m4): ws wire protocol (ClientMsg/ServerMsg) + ts-rs bindings"
```

---

### Task 5: `Repository::get_world` + `Room` + `RoomRegistry` + telemetry

**Files:**
- Modify: `src/server/src/data/repository.rs`, `src/server/src/data/sqlite.rs`
- Modify: `src/server/src/ws/room.rs`

**Interfaces:**
- Consumes: `Repository::apply_command`, `Repository::events_since`, `Repository::get_world`; `ServerMsg`, `ResyncSource`, `RingBuffer`.
- Produces:
  - `RoomStats` (atomics) + `RoomStatsSnapshot { world_id, connections, current_seq, events_published, gaps_detected, resyncs_hot, resyncs_cold, lagged_drops }` (Serialize, TS).
  - `Room`: `subscribe(&self) -> (broadcast::Receiver<Arc<ServerMsg>>, i64)`, `current_seq(&self) -> i64`, `publish(&self, &dyn Repository, author: Uuid, ts: i64) -> Result<Command, DataError>`, `resync_range(&self, &dyn Repository, from_seq: i64) -> Result<(Vec<Arc<ServerMsg>>, ResyncSource), DataError>`, `stats: RoomStats`, `world_id: Uuid`.
  - `RoomRegistry`: `new()`, `get_or_create(&self, &dyn Repository, Uuid) -> Result<Option<Arc<Room>>, DataError>`, `get(&self, Uuid) -> Option<Arc<Room>>`, `snapshot(&self) -> Vec<RoomStatsSnapshot>`, `reap_if_empty(&self, Uuid)`.

- [ ] **Step 1: Add `get_world` to the `Repository` trait** in `repository.rs`, after `events_since`:

```rust
    async fn get_world(&self, id: Uuid) -> Result<Option<crate::data::document::World>, DataError>;
```

- [ ] **Step 2: Satisfy it for SQLite.** In `sqlite.rs`, move the existing inherent `get_world` (lines ~76-89) from the `impl SqliteRepository` block into the `impl Repository for SqliteRepository` block (delete the inherent copy). Existing callers continue to work as long as `Repository` is in scope; if any test breaks on resolution, add `use crate::data::repository::Repository;`.

- [ ] **Step 3: Run existing data tests to confirm no regression.** Run: `cargo test -p shadowcat data::`
  Expected: PASS (existing `events_since_returns_the_suffix`, etc.).

- [ ] **Step 4: Write failing Room/Registry tests** in `room.rs`:

```rust
#[cfg(test)]
mod room_tests {
    use super::*;
    use crate::data::sqlite::SqliteRepository;
    use crate::data::repository::Repository;
    use crate::auth::role::ServerRole;
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
        assert!(reg.get_or_create(&repo, Uuid::from_u128(999)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn resync_hot_then_cold_tiers() {
        let (repo, world_id, author) = repo_with_world().await;
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        for _ in 0..3 { room.publish(&repo, author, 0).await.unwrap(); } // seq 1,2,3

        // hot: from_seq 2 resident in buffer
        let (hot, src) = room.resync_range(&repo, 2).await.unwrap();
        assert_eq!(src, ResyncSource::Buffer);
        assert_eq!(hot.iter().map(|m| m.event_seq().unwrap()).collect::<Vec<_>>(), vec![2, 3]);
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
        for h in handles { h.await.unwrap(); }

        let mut seqs = vec![];
        for _ in 0..50 { seqs.push(rx.recv().await.unwrap().event_seq().unwrap()); }
        let mut sorted = seqs.clone();
        sorted.sort();
        assert_eq!(seqs, sorted, "broadcast delivery order must equal seq order");
        assert_eq!(seqs, (1..=50).collect::<Vec<_>>());
    }
}
```

> The concurrency test relies on `Arc<Room>` (`room.clone()`). Ensure `RoomRegistry` hands out `Arc<Room>`.

- [ ] **Step 5: Run to verify failure.** Run: `cargo test -p shadowcat room_tests`
  Expected: FAIL (`Room`/`RoomRegistry`/`RoomStats` not found).

- [ ] **Step 6: Implement telemetry, `Room`, `RoomRegistry`** in `room.rs`:

```rust
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use dashmap::DashMap;
use serde::Serialize;
use tokio::sync::{broadcast, Mutex};
use ts_rs::TS;
use uuid::Uuid;

use crate::data::command::UnsequencedCommand;
use crate::data::command::Command;
use crate::data::repository::Repository;
use crate::data::DataError;
use crate::ws::protocol::ResyncSource;

const BROADCAST_CAPACITY: usize = 256;

#[derive(Default)]
pub struct RoomStats {
    pub connections: AtomicI64,
    pub events_published: AtomicU64,
    pub gaps_detected: AtomicU64,
    pub resyncs_hot: AtomicU64,
    pub resyncs_cold: AtomicU64,
    pub lagged_drops: AtomicU64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
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
    pub async fn publish(&self, repo: &dyn Repository, author: Uuid, ts: i64) -> Result<Command, DataError> {
        let _guard = self.publish_guard.lock().await;
        let cmd = repo
            .apply_command(UnsequencedCommand { world_id: self.world_id, author, ts, ops: vec![] })
            .await?;
        let msg = Arc::new(ServerMsg::Event { command: cmd.clone() });
        self.ring.lock().await.push(msg.clone());
        self.current_seq.store(cmd.seq, Ordering::Release);
        let _ = self.tx.send(msg); // Err only when there are no receivers
        self.stats.events_published.fetch_add(1, Ordering::Relaxed);
        Ok(cmd)
    }

    /// Resolve a resync range: hot ring tier if fully resident, else cold
    /// `events_since` tier. Increments the matching telemetry counter.
    pub async fn resync_range(&self, repo: &dyn Repository, from_seq: i64) -> Result<(Vec<Arc<ServerMsg>>, ResyncSource), DataError> {
        if let Some(hot) = self.ring.lock().await.range_from(from_seq) {
            self.stats.resyncs_hot.fetch_add(1, Ordering::Relaxed);
            return Ok((hot, ResyncSource::Buffer));
        }
        let cmds = repo.events_since(self.world_id, from_seq - 1).await?;
        self.stats.resyncs_cold.fetch_add(1, Ordering::Relaxed);
        let frames = cmds.into_iter().map(|c| Arc::new(ServerMsg::Event { command: c })).collect();
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

pub struct RoomRegistry {
    rooms: DashMap<Uuid, Arc<Room>>,
}

impl RoomRegistry {
    pub fn new() -> Self {
        Self { rooms: DashMap::new() }
    }

    /// Get the room for an existing world, creating it (seeded from the world's
    /// current seq) on first join. `None` when the world does not exist.
    pub async fn get_or_create(&self, repo: &dyn Repository, world_id: Uuid) -> Result<Option<Arc<Room>>, DataError> {
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
```

Add the needed `use std::sync::Arc;` and `use crate::ws::protocol::ServerMsg;` at the top of `room.rs` if not already present from Task 3.

- [ ] **Step 7: Run tests.** Run: `cargo test -p shadowcat room_tests && cargo test -p shadowcat`
  Expected: PASS (all room tests, including ordering under concurrency; full suite green; `RoomStatsSnapshot.ts` generated).

- [ ] **Step 8: Commit.**

```bash
git add src/server/src/data/repository.rs src/server/src/data/sqlite.rs src/server/src/ws/room.rs src/types/generated
git commit -m "feat(m4): Room/RoomRegistry with ordered publish, tiered resync, telemetry"
```

---

### Task 6: Time source + calibration

**Files:**
- Modify: `src/server/src/ws/time.rs`

**Interfaces:**
- Produces: `now_millis() -> i64`, `calibrate(client_t0: i64, client_t1: i64, server_t: i64) -> (i64, i64)` returning `(offset, rtt)`.

- [ ] **Step 1: Write the failing test** in `time.rs`:

```rust
//! Server time source (wall-clock unix millis) + NTP-style offset calibration.

use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
mod time_tests {
    use super::*;

    #[test]
    fn calibrate_computes_offset_and_rtt() {
        // client sends at 1000, receives at 1100 (rtt 100); server stamped 2060.
        // midpoint = 1050; offset = 2060 - 1050 = 1010.
        let (offset, rtt) = calibrate(1000, 1100, 2060);
        assert_eq!(rtt, 100);
        assert_eq!(offset, 1010);
    }

    #[test]
    fn now_millis_is_positive_and_monotone_enough() {
        let a = now_millis();
        let b = now_millis();
        assert!(a > 0 && b >= a);
    }
}
```

- [ ] **Step 2: Run to verify failure.** Run: `cargo test -p shadowcat time_tests`
  Expected: FAIL (`calibrate`/`now_millis` not found).

- [ ] **Step 3: Implement** in `time.rs`:

```rust
/// Wall-clock unix milliseconds. Used for the server time source and event ts.
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// NTP-style calibration from a single ping/pong round trip.
/// `offset` = server_t - midpoint(client send, client recv); `rtt` = recv - send.
/// A positive offset means the server clock leads the client clock.
pub fn calibrate(client_t0: i64, client_t1: i64, server_t: i64) -> (i64, i64) {
    let rtt = client_t1 - client_t0;
    let offset = server_t - (client_t0 + client_t1) / 2;
    (offset, rtt)
}
```

- [ ] **Step 4: Run tests.** Run: `cargo test -p shadowcat time_tests`
  Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add src/server/src/ws/time.rs
git commit -m "feat(m4): server time source + NTP-style offset calibration"
```

---

### Task 7: Connection handler (upgrade + ingress/egress + sequence guard)

**Files:**
- Modify: `src/server/src/ws/conn.rs`
- Modify: `src/server/src/ws/mod.rs` (add `WsState`)

**Interfaces:**
- Consumes: `AuthUser` (`.id`), `AppState` (`repo`, `ws`), `RoomRegistry`, `Room`, `ClientMsg`/`ServerMsg`, `time::now_millis`.
- Produces: `ws_handler(WebSocketUpgrade, AuthUser, State<AppState>, Query<WsQuery>) -> Response`; `WsState { rooms: RoomRegistry }` (defined in `mod.rs`).

> This task's correctness is proven by the integration harness (Task 9). It still lands with one focused smoke assertion via that harness's first test (`join → welcome → emit → receive`), so implement here and rely on Task 9 to run red→green. Build-only verification in this task.

- [ ] **Step 1: Define `WsState` in `mod.rs`.** Replace the `pub use room::RoomRegistry;` line with:

```rust
use std::sync::Arc;

pub use room::RoomRegistry;

/// Realtime state shared in `AppState`. A thin handle today; the seam for future
/// bus internals (actor pool / external broker) without touching callers.
#[derive(Clone)]
pub struct WsState {
    pub rooms: Arc<RoomRegistry>,
}

impl WsState {
    pub fn new() -> Self {
        Self { rooms: Arc::new(RoomRegistry::new()) }
    }
}

impl Default for WsState {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: Implement `conn.rs`:**

```rust
//! WebSocket upgrade and per-connection ingress/egress tasks.

use std::sync::atomic::Ordering;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use futures_util::stream::StreamExt;
use futures_util::SinkExt;
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::auth::session::AuthUser;
use crate::data::repository::Repository;
use crate::http::AppState;
use crate::ws::protocol::{ClientMsg, ResyncSource, ServerMsg, WsErrorCode};
use crate::ws::room::Room;
use crate::ws::time::now_millis;

#[derive(Debug, Deserialize)]
pub struct WsQuery {
    pub world: Uuid,
}

/// Session-gated upgrade. `AuthUser` enforces authentication (401 without a
/// session) before the socket is established.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    user: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<WsQuery>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state, user.id, q.world))
}

fn text(msg: &ServerMsg) -> Message {
    // Serialization of our own types never fails.
    Message::Text(serde_json::to_string(msg).unwrap().into())
}

async fn handle_socket(socket: WebSocket, state: AppState, user_id: Uuid, world_id: Uuid) {
    let repo = state.repo.clone();
    let room = match state.ws.rooms.get_or_create(repo.as_ref(), world_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            let mut s = socket;
            let _ = s
                .send(text(&ServerMsg::Error {
                    code: WsErrorCode::WorldNotFound,
                    message: "world not found".into(),
                }))
                .await;
            let _ = s.send(Message::Close(None)).await;
            return;
        }
        Err(_) => {
            let mut s = socket;
            let _ = s
                .send(text(&ServerMsg::Error { code: WsErrorCode::Internal, message: "internal".into() }))
                .await;
            return;
        }
    };

    room.stats.connections.fetch_add(1, Ordering::AcqRel);
    let (mut rx, current_seq) = room.subscribe();
    let (mut sink, mut stream) = socket.split();

    // Welcome.
    if sink
        .send(text(&ServerMsg::Welcome { world: world_id, current_seq, server_time: now_millis() }))
        .await
        .is_err()
    {
        finish(&state, &room, world_id);
        return;
    }

    // Egress task: live frames + lag-driven resync. Owns the sink.
    let egress_room = room.clone();
    let egress_repo = repo.clone();
    let mut egress = tokio::spawn(async move {
        let mut next_expected = current_seq + 1;
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if let Some(seq) = msg.event_seq() {
                        if seq < next_expected {
                            continue; // already delivered via a resync
                        }
                        if seq > next_expected {
                            egress_room.stats.gaps_detected.fetch_add(1, Ordering::Relaxed);
                            if replay(&mut sink, &egress_room, egress_repo.as_ref(), next_expected).await.is_err() {
                                break;
                            }
                            next_expected = egress_room.current_seq() + 1;
                            if seq < next_expected {
                                continue;
                            }
                        }
                        if sink.send(text(&msg)).await.is_err() {
                            break;
                        }
                        next_expected = seq + 1;
                    } else if sink.send(text(&msg)).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    egress_room.stats.lagged_drops.fetch_add(n, Ordering::Relaxed);
                    if replay(&mut sink, &egress_room, egress_repo.as_ref(), next_expected).await.is_err() {
                        break;
                    }
                    next_expected = egress_room.current_seq() + 1;
                }
                Err(RecvError::Closed) => break,
            }
        }
    });

    // Ingress: parse client frames. Publishes/time-sync use the room + repo.
    let ingress_room = room.clone();
    let ingress_repo = repo.clone();
    loop {
        tokio::select! {
            _ = &mut egress => break,
            frame = stream.next() => {
                let Some(Ok(frame)) = frame else { break };
                match frame {
                    Message::Text(t) => {
                        match serde_json::from_str::<ClientMsg>(t.as_str()) {
                            Ok(ClientMsg::EmitTest { .. }) => {
                                let _ = ingress_room.publish(ingress_repo.as_ref(), user_id, now_millis()).await;
                            }
                            Ok(ClientMsg::TimePing { client_t0 }) => {
                                // handled inline via a dedicated channel: send through the room's
                                // broadcast is wrong (per-connection). Instead, time replies go on
                                // the live stream by re-subscribing is overkill; M4 sends TimePong
                                // by briefly borrowing the sink is not possible here (sink moved to
                                // egress). See Step 3 note: TimePing is answered by the egress task.
                                let _ = client_t0; // see Step 3
                            }
                            Ok(ClientMsg::ResyncRequest { .. })
                            | Ok(ClientMsg::Hello { .. })
                            | Ok(ClientMsg::Pong) => { /* see Step 3 */ }
                            Err(_) => { /* malformed: ignored at ingress; see Step 3 */ }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }

    egress.abort();
    finish(&state, &room, world_id);
}

fn finish(state: &AppState, room: &Room, world_id: Uuid) {
    room.stats.connections.fetch_sub(1, Ordering::AcqRel);
    state.ws.rooms.reap_if_empty(world_id);
}

/// Replay `[from_seq, current]` to the sink as ResyncBegin .. Event* .. ResyncEnd.
async fn replay<S>(sink: &mut S, room: &Room, repo: &dyn Repository, from_seq: i64) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let (frames, source) = room.resync_range(repo, from_seq).await.map_err(|_| ())?;
    let to_seq = frames.last().and_then(|m| m.event_seq()).unwrap_or(from_seq - 1);
    sink.send(text(&ServerMsg::ResyncBegin { from_seq, to_seq, source }))
        .await
        .map_err(|_| ())?;
    let _ = source; // ResyncSource is Copy; already used above
    for f in frames {
        sink.send(text(&f)).await.map_err(|_| ())?;
    }
    sink.send(text(&ServerMsg::ResyncEnd { current_seq: room.current_seq() }))
        .await
        .map_err(|_| ())?;
    Ok(())
}
```

- [ ] **Step 3: Resolve the ingress design notes left in Step 2.** The sink is owned by the egress task, so ingress cannot write directly. Restructure so **all socket writes happen in the egress task** and ingress communicates intents to it via a `tokio::sync::mpsc` channel:
  - Define `enum Egress { Frame(Arc<ServerMsg>), TimePong { client_t0: i64, server_t: i64 } }`.
  - Egress `select!`s over the broadcast `rx` and the mpsc `Receiver<Egress>`; both paths write to the sink.
  - Ingress: `EmitTest` → `room.publish(...)` (the resulting Event arrives via broadcast, no direct send). `TimePing { client_t0 }` → `mpsc.send(Egress::TimePong { client_t0, server_t: now_millis() })`. `ResyncRequest { from_seq }` → signal egress to run `replay(from_seq)` (send an `Egress::Resync(from_seq)` variant). `Hello`/`Pong` → no-op in M4 (Hello's `last_seq` reconnect resync is exercised by the harness reconnecting with a fresh socket whose `current_seq` reflects the prior `last_seq`; explicit `Hello`-driven mid-stream resync is not needed because `subscribe` already returns `current_seq` for the Welcome). Malformed frame → `mpsc.send(Egress::Frame(Arc::new(ServerMsg::Error { code: BadMessage, .. })))`.

  Implement that mpsc-based structure (it replaces the placeholder arms). Keep `replay` as written; call it from the egress task on `Lagged`, on a detected gap, and on `Egress::Resync`.

- [ ] **Step 4: Wire `ws` field into `AppState`** — done in Task 8; for now verify the module compiles in isolation by building.

  Run: `cargo build -p shadowcat`
  Expected: compile errors only about `AppState` missing a `ws` field (resolved in Task 8). If other errors appear (signatures, moves), fix them now. Once Task 8 adds the field, this builds clean.

- [ ] **Step 5: Commit.**

```bash
git add src/server/src/ws/conn.rs src/server/src/ws/mod.rs
git commit -m "feat(m4): ws connection handler (upgrade, egress-owned sink, ingress intents)"
```

---

### Task 8: HTTP wiring — `AppState.ws`, `/ws`, `/api/debug/rooms`

**Files:**
- Modify: `src/server/src/http/mod.rs`
- Modify: `src/server/src/http/routes.rs`
- Modify: `src/server/src/main.rs`

**Interfaces:**
- Consumes: `WsState`, `ws::conn::ws_handler`, `AdminUser`, `RoomRegistry::snapshot`.
- Produces: `AppState { repo, config, setup_token, initialized, ws }`; routes `GET /ws`, `GET /api/debug/rooms`.

- [ ] **Step 1: Add the `ws` field to `AppState`** in `http/mod.rs`:

```rust
#[derive(Clone)]
pub struct AppState {
    pub repo: Arc<SqliteRepository>,
    pub config: Arc<Config>,
    pub setup_token: Option<String>,
    pub initialized: Arc<AtomicBool>,
    pub ws: crate::ws::WsState,
}
```

- [ ] **Step 2: Construct it** at every `AppState { .. }` site: `main.rs` (the real boot) and the two test helpers in `http/mod.rs` (`test_state`, and the `headless_bootstrap` test's inline state) — add `ws: crate::ws::WsState::new(),`. The `setup_requires_token_when_policy_demands_it` test mutates fields on a `test_state()` result, so it inherits the field automatically.

- [ ] **Step 3: Add the routes** in `http/mod.rs` `router()`:

```rust
        .route("/health", get(routes::health))
        .route("/ws", get(crate::ws::conn::ws_handler))
        .route("/api/debug/rooms", get(routes::debug_rooms))
        .route("/api/me", get(routes::me))
```

- [ ] **Step 4: Add the handler** in `routes.rs`:

```rust
use crate::auth::session::AdminUser;
use crate::ws::room::RoomStatsSnapshot;

/// Admin-only snapshot of live room telemetry.
pub async fn debug_rooms(
    _admin: AdminUser,
    axum::extract::State(state): axum::extract::State<crate::http::AppState>,
) -> axum::Json<Vec<RoomStatsSnapshot>> {
    axum::Json(state.ws.rooms.snapshot())
}
```

- [ ] **Step 5: Write a focused integration test** in `http/mod.rs` tests module:

```rust
    #[tokio::test]
    async fn debug_rooms_requires_admin() {
        let server = server_with_user("u", "pw", ServerRole::User).await;
        server.post("/api/login").json(&serde_json::json!({"username":"u","password":"pw"})).await;
        server.get("/api/debug/rooms").await.assert_status(axum::http::StatusCode::FORBIDDEN);
    }
```

- [ ] **Step 6: Run build + tests.** Run: `cargo test -p shadowcat`
  Expected: PASS, including `debug_rooms_requires_admin`; the whole crate (incl. `conn.rs`) now compiles.

- [ ] **Step 7: Commit.**

```bash
git add src/server/src/http/mod.rs src/server/src/http/routes.rs src/server/src/main.rs
git commit -m "feat(m4): wire ws state, /ws upgrade, and /api/debug/rooms into the router"
```

---

### Task 9: Desync-convergence harness + `test_server` bin

**Files:**
- Create: `src/server/tests/ws_convergence.rs`
- Create: `src/server/src/bin/test_server.rs`

**Interfaces:**
- Consumes: `shadowcat::http::{router, AppState}`, `shadowcat::data::sqlite::SqliteRepository`, `shadowcat::auth::password::hash_password`, `shadowcat::ws::WsState`, `shadowcat::config::Config`, `shadowcat::ws::time::now_millis`, `Repository` (for `create_world`/`events_since`).
- Produces: the convergence test suite; a runnable `test_server` binary.

- [ ] **Step 1: Add a shared spawn helper + the first (happy-path) test** in `tests/ws_convergence.rs`:

```rust
//! Desync-convergence harness: real WS clients against an ephemeral in-process
//! server. Faults are induced by client behavior (stop reading -> Lagged,
//! ignore frames, disconnect+reconnect). Convergence is asserted against the
//! authoritative `world_events` log.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use shadowcat::auth::password::hash_password;
use shadowcat::auth::role::ServerRole;
use shadowcat::config::Config;
use shadowcat::data::repository::Repository;
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::http::{self, AppState};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

struct Harness {
    addr: String,
    cookie: String,
    world: Uuid,
    repo: Arc<SqliteRepository>,
}

async fn spawn() -> Harness {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let world = repo.create_world("test", 0).await.unwrap();
    let hash = hash_password("pw").unwrap();
    repo.create_user("u", Some(&hash), ServerRole::User, 0).await.unwrap();

    let state = AppState {
        repo: repo.clone(),
        config: Arc::new(Config::default()),
        setup_token: None,
        initialized: Arc::new(AtomicBool::new(true)),
        ws: shadowcat::ws::WsState::new(),
    };
    let app = http::router(state).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Log in over HTTP to obtain the signed session cookie, then reuse it on WS.
    let client = reqwest::Client::builder().cookie_store(true).build().unwrap();
    let res = client
        .post(format!("http://{addr}/api/login"))
        .json(&serde_json::json!({ "username": "u", "password": "pw" }))
        .send()
        .await
        .unwrap();
    let cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    Harness { addr, cookie, world, repo }
}

impl Harness {
    async fn connect(&self) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
        let url = format!("ws://{}/ws?world={}", self.addr, self.world);
        let mut req = url.into_client_request().unwrap();
        req.headers_mut().insert("cookie", self.cookie.parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        ws
    }

    async fn authoritative_seqs(&self) -> Vec<i64> {
        self.repo.events_since(self.world, 0).await.unwrap().into_iter().map(|c| c.seq).collect()
    }
}

// reqwest is needed as a dev-dependency for login; add it in Step 0 below.

#[tokio::test]
async fn join_welcome_emit_receive() {
    let h = spawn().await;
    let mut ws = h.connect().await;

    // First server frame is Welcome.
    let first = ws.next().await.unwrap().unwrap();
    let welcome: serde_json::Value = serde_json::from_str(first.to_text().unwrap()).unwrap();
    assert_eq!(welcome["type"], "welcome");
    assert_eq!(welcome["current_seq"], 0);

    // Emit one test event; expect an Event with seq 1.
    ws.send(Message::Text(serde_json::json!({ "type": "emit_test", "nonce": 1 }).to_string().into()))
        .await
        .unwrap();
    let evt = loop {
        let m = ws.next().await.unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(m.to_text().unwrap()).unwrap();
        if v["type"] == "event" { break v; }
    };
    assert_eq!(evt["command"]["seq"], 1);
    assert_eq!(h.authoritative_seqs().await, vec![1]);
}
```

- [ ] **Step 0 (do before Step 1 compiles): add dev-deps.** In `src/server/Cargo.toml` `[dev-dependencies]` add:

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "cookies"] }
futures-util = "0.3"
```

(`futures-util` is already a normal dep; repeating under dev is harmless but optional — omit if it resolves.)

- [ ] **Step 2: Run the happy-path test.** Run: `cargo test -p shadowcat --test ws_convergence join_welcome_emit_receive`
  Expected: PASS. (If `Message::Text` type errors occur, adjust to the tungstenite `Utf8Bytes` API: `Message::Text(s.into())`.)

- [ ] **Step 3: Add the convergence test with client-driven faults.** Append:

```rust
/// Helper: read frames, collecting Event seqs, until `count` events seen or a
/// budget elapses. Returns collected seqs in arrival order.
async fn drain_event_seqs<S>(ws: &mut S, count: usize) -> Vec<i64>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let mut seqs = vec![];
    while seqs.len() < count {
        let Ok(Some(Ok(m))) = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await else {
            break;
        };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(m.to_text().unwrap_or("")) {
            if v["type"] == "event" {
                seqs.push(v["command"]["seq"].as_i64().unwrap());
            }
        }
    }
    seqs
}

#[tokio::test]
async fn all_clients_converge_after_reconnect() {
    let h = spawn().await;

    // Client A stays connected the whole time.
    let mut a = h.connect().await;
    let _ = a.next().await; // Welcome

    // Emit 5 events from a publisher client.
    let mut pubc = h.connect().await;
    let _ = pubc.next().await; // Welcome
    for n in 0..5 {
        pubc.send(Message::Text(serde_json::json!({ "type": "emit_test", "nonce": n }).to_string().into()))
            .await
            .unwrap();
    }

    // Client A receives all 5 live.
    let a_seqs = drain_event_seqs(&mut a, 5).await;
    assert_eq!(a_seqs, vec![1, 2, 3, 4, 5]);

    // Client B joins late, then resyncs from 0 via Hello/Welcome current_seq:
    // a fresh connection's Welcome reports current_seq=5; B requests resync.
    let mut b = h.connect().await;
    let _ = b.next().await; // Welcome (current_seq = 5)
    b.send(Message::Text(serde_json::json!({ "type": "resync_request", "from_seq": 1 }).to_string().into()))
        .await
        .unwrap();
    let b_seqs = drain_event_seqs(&mut b, 5).await;
    assert_eq!(b_seqs, vec![1, 2, 3, 4, 5]);

    // Authoritative log is the ground truth both converged to.
    assert_eq!(h.authoritative_seqs().await, vec![1, 2, 3, 4, 5]);
}

#[tokio::test]
async fn slow_reader_recovers_via_lagged_resync() {
    let h = spawn().await;
    let mut slow = h.connect().await;
    let _ = slow.next().await; // Welcome

    // Flood more events than the broadcast capacity (256) without reading,
    // forcing server-side Lagged for `slow`.
    let mut pubc = h.connect().await;
    let _ = pubc.next().await;
    for n in 0..400 {
        pubc.send(Message::Text(serde_json::json!({ "type": "emit_test", "nonce": n }).to_string().into()))
            .await
            .unwrap();
    }
    // Give the server time to process the publishes.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Now read: the server's egress should have replayed via the cold tier so
    // the final delivered seq reaches the authoritative tail (400).
    let seqs = drain_event_seqs(&mut slow, 400).await;
    assert_eq!(*seqs.last().unwrap(), 400);
    // Delivered stream is strictly increasing and ends at the authoritative max.
    let mut sorted = seqs.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(seqs, sorted, "no duplicates or reordering after resync");
    assert_eq!(*h.authoritative_seqs().await.last().unwrap(), 400);
}

#[tokio::test]
async fn time_sync_returns_pong() {
    let h = spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome
    ws.send(Message::Text(serde_json::json!({ "type": "time_ping", "client_t0": 1000 }).to_string().into()))
        .await
        .unwrap();
    let pong = loop {
        let m = ws.next().await.unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(m.to_text().unwrap()).unwrap();
        if v["type"] == "time_pong" { break v; }
    };
    assert_eq!(pong["client_t0"], 1000);
    assert!(pong["server_t"].as_i64().unwrap() > 0);
}
```

- [ ] **Step 4: Run the full harness.** Run: `cargo test -p shadowcat --test ws_convergence`
  Expected: PASS (4 tests). If `slow_reader_recovers_via_lagged_resync` is flaky on timing, increase the sleep or the drain timeout — do not reduce the asserted invariants.

- [ ] **Step 5: Create the `test_server` bin** at `src/server/src/bin/test_server.rs`:

```rust
//! Throwaway WS server for manual/external clients. In-memory DB, one seeded
//! user (u/pw) and one world; prints the bind address and world id.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use shadowcat::auth::password::hash_password;
use shadowcat::auth::role::ServerRole;
use shadowcat::config::Config;
use shadowcat::data::repository::Repository;
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::http::{self, AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await?);
    let world = repo.create_world("test", 0).await?;
    let hash = hash_password("pw")?;
    repo.create_user("u", Some(&hash), ServerRole::User, 0).await?;

    let state = AppState {
        repo,
        config: Arc::new(Config::default()),
        setup_token: None,
        initialized: Arc::new(AtomicBool::new(true)),
        ws: shadowcat::ws::WsState::new(),
    };
    let app = http::router(state).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tracing::info!(%addr, world = %world.id, "test_server listening (user: u / pw)");
    println!("test_server: http://{addr}  world={}  login u/pw", world.id);
    axum::serve(listener, app).await?;
    Ok(())
}
```

- [ ] **Step 6: Verify the bin builds.** Run: `cargo build -p shadowcat --bin test_server`
  Expected: builds clean.

- [ ] **Step 7: Commit.**

```bash
git add src/server/Cargo.toml src/server/tests/ws_convergence.rs src/server/src/bin/test_server.rs
git commit -m "feat(m4): desync-convergence harness + spawnable test_server bin"
```

---

### Task 10: Telemetry tracing, lint/format, docs sync

**Files:**
- Modify: `src/server/src/ws/conn.rs` (tracing events)
- Modify: `docs/PLAN.md`, `docs/TODO.md` (if any deferral logged)

**Interfaces:** none new.

- [ ] **Step 1: Add structured tracing** at the connection lifecycle and resync points in `conn.rs` (not `println!`): on connect (`tracing::info!(world=%world_id, user=%user_id, "ws connected")`), on disconnect, on gap-detected (`tracing::debug!`), on resync served (`tracing::debug!(?source, from_seq, "resync served")`), on lagged drop (`tracing::warn!(n, "broadcast lagged")`). Keep levels: lifecycle `info`/`debug`, lag `warn`.

- [ ] **Step 2: Format + lint.** Run:
```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
```
Expected: no diffs after fmt re-run; clippy clean. Fix any warnings inline.

- [ ] **Step 3: Full test + bindings sync.** Run:
```bash
cargo test -p shadowcat
git diff --exit-code src/types/generated
```
Expected: all tests pass; no binding drift.

- [ ] **Step 4: Update `docs/PLAN.md`.** Mark M4 complete: change `### M4 · WebSocket event bus` to `### M4 · WebSocket event bus ✅`. Update `docs/PLAN.md` cross-cutting lines for "Observability + desync telemetry: M4" and "Desync-convergence test: M4" only if their wording implies pending status (they describe ongoing maintenance — leave as-is). If any work was deferred during execution, log it to `docs/TODO.md` (not here).

- [ ] **Step 5: Commit.**

```bash
git add src/server/src/ws/conn.rs docs/PLAN.md docs/TODO.md
git commit -m "feat(m4): connection telemetry tracing; mark M4 complete"
```

---

## Self-Review

**1. Spec coverage** (each spec §11 decision → task):
- Generic event substrate (empty-ops command) → Task 5 (`Room::publish`), Task 9 (`emit_test`).
- Registry + broadcast behind `Room`/`RoomRegistry`; publish-ordering guard tested → Task 5 (`publish_is_ordered_under_concurrency`).
- WS auth = session + world-exists → Task 7 (`AuthUser` extractor), Task 5 (`get_or_create` → `None`).
- Resync tiers hot→cold; snapshot deferred → Task 5 (`resync_range`), Task 3 (`range_from`).
- JSON + ts-rs protocol → Task 4; ts-rs tree → Task 2.
- Wall-clock time + calibration → Task 6.
- Telemetry counters + tracing + `/api/debug/rooms` (AdminUser) → Task 5 (`RoomStats`/snapshot), Task 8 (route), Task 10 (tracing).
- Convergence harness (real clients, client-driven faults, assert vs `world_events`) + `test_server` bin → Task 9.
- Ring buffer 1024 / 5 min → Task 3.

**2. Placeholder scan:** Task 7 Step 2 deliberately contains design notes that Step 3 resolves into the mpsc-based egress structure — this is a two-step refinement, not a shipped placeholder; Step 3 specifies the exact enum and routing. No `TODO`/`TBD` remain in shipped code.

**3. Type consistency:** `ServerMsg::event_seq/event_ts` (Task 3) reused unchanged in Tasks 5/7. `resync_range -> (Vec<Arc<ServerMsg>>, ResyncSource)` (Task 5) consumed by `replay` (Task 7). `RoomStatsSnapshot` fields (Task 5) returned by `debug_rooms` (Task 8). `WsState { rooms: Arc<RoomRegistry> }` (Task 7) consumed by `AppState.ws` (Task 8) and harness (Task 9). `get_or_create(&dyn Repository, Uuid)` consistent across Tasks 5/7/9.

## Buddy-check directives

M4 carries genuine concurrency-correctness risk (the publish-ordering invariant, the egress resync/dedup state machine, broadcast `Lagged` recovery). A buddy-check (two independent blind reviewers debating) is **offered** for the final branch review, above mainline-plan-execution's single dispatched review. Default if not requested: the standard single fresh-context branch review.

**Outcome (recorded at handoff):** Offered at the post-implementation review checkpoint; user chose the **single fresh-context review**. No buddy-check run.
