# M4 — WebSocket Event Bus: Design

Status: approved (brainstorm). Date: 2026-06-18.
Roadmap: [`docs/PLAN.md`](../../PLAN.md) M4. Architecture source of truth: [`docs/design/ARCHITECTURE.md`](../../design/ARCHITECTURE.md).

## 1. Goal

Stand up the live realtime transport on top of M2's durable `world_events` log: a session-gated WebSocket endpoint, per-world rooms, ordered sequenced broadcasts, an in-memory ring buffer with tiered resync, a client sequence guard, reconnect/resync, a server time source with client offset calibration, desync telemetry, a spawnable test-server binary, and the desync-convergence test harness — the project's highest-value test.

M4 carries a **generic sequenced event** so rooms / sequencing / ring buffer / resync / the convergence test are fully exercised before real document commands exist. Real document commands flow onto the same broadcast frame in M5 with no transport rework.

## 2. Scope & non-goals

**In scope**
- WebSocket upgrade endpoint, session-gated, joining an existing world.
- Per-world rooms behind a stable `Room` / `RoomRegistry` interface; ordered broadcasts.
- In-memory ring buffer (time + count bounded) with tiered resync (hot buffer → cold `world_events`).
- Client sequence guard; reconnect/resync handshake.
- Server time source (wall-clock) + NTP-style offset/RTT calibration.
- Desync telemetry: per-room atomic counters + structured tracing + an `AdminUser`-gated debug endpoint.
- Spawnable `test_server` binary.
- Desync-convergence integration harness.

**Out of scope (and why)**
- **Document CRUD over HTTP/WS** — M5. M4's seq-bearing content is a generic empty-ops command (see §6).
- **`PermissionContext` / per-recipient filtering** — M5. M4 sends the full unfiltered stream; the per-connection egress task is the seam where M5 inserts filtering (invariant #4).
- **Client-side optimistic-apply + rollback, the Zod client document store** — M6.
- **World-creation API, world-membership tables / per-world roles** — M5+. M4 authorizes on "authenticated session + world exists" only. Worlds are created directly via `repo.create_world` in tests/harness.
- **Doc-state snapshot resync** — M5/M6. In M4 the full event log is always replayable, so event replay alone covers resync; the snapshot tier is a documented future seam (invariant #2).

## 3. Crate & module layout

Stay a **single crate** (`shadowcat`), consistent with M3. New module tree under `src/server/src/`:

```
ws/
  mod.rs        # WsState (RoomRegistry handle) + re-exports
  room.rs       # Room, RoomRegistry, RingBuffer, per-room atomic counters
  protocol.rs   # ClientMsg / ServerMsg envelopes (serde + ts-rs)
  conn.rs       # upgrade handler; per-connection ingress + egress tasks; sequence guard
  time.rs       # server time source (wall-clock unix millis) + calibration reply
  telemetry.rs  # RoomStats snapshot type for /api/debug/rooms
bin/
  test_server.rs  # spawnable throwaway server (in-memory or tempfile DB, pre-created world)
```

`http/mod.rs` gains the `/ws` route, the `/api/debug/rooms` route, and `WsState` in `AppState`. The existing session layer gates the upgrade.

## 4. Dependencies

Added to `src/server/Cargo.toml` (all MIT/Apache-2.0 — satisfies the permissive-license invariant, ARCHITECTURE §2.9):

| Crate | Purpose |
|---|---|
| `axum` `ws` feature | WebSocket upgrade + message types (already on axum 0.8) |
| `dashmap` 6 | sharded `WorldId → Room` registry (lower contention than `RwLock<HashMap>`) |
| `futures-util` | stream/sink combinators for the split socket (if not already transitively present) |

Dev-dependencies:

| Crate | Purpose |
|---|---|
| `tokio-tungstenite` | real WebSocket client for the convergence harness |

`tokio` (with `time`, `sync`, `macros`, `rt-multi-thread`), `serde`, `serde_json`, `uuid`, `tracing`, `ts-rs` are already present.

## 5. Room model & ordering invariant

- **`RoomRegistry`** — `DashMap<Uuid, Arc<Room>>`. `get_or_create(world_id)`, `get(world_id)`. A room is reaped when its last subscriber disconnects (subscriber count reaches zero). This interface is the **abstraction boundary**: the internal broadcast primitive may later be replaced by a per-world actor or an external broker (Postgres `LISTEN/NOTIFY` / NATS) for multi-process scale-out without touching callers or connections.
- **`Room`** holds:
  - `tx: tokio::sync::broadcast::Sender<Arc<ServerMsg>>` — fan-out. Intentionally lossy: a lagging receiver gets `RecvError::Lagged(n)`, which is the gap signal that drives resync. A slow client never throttles the authoritative producer (the correct semantics for server-authoritative fan-out).
  - `ring: Mutex<RingBuffer>` — recent events for hot resync.
  - `publish_guard: tokio::sync::Mutex<()>` — serializes the publish critical section per world.
  - `stats: RoomStats` — atomic counters (§9).
- **Ordering invariant (load-bearing):** `Room::publish` holds `publish_guard` across the entire critical section — **allocate seq (`repo.apply_command`) → append to ring → `tx.send`** — so broadcast delivery order equals seq order. Seq allocation is already globally serialized by SQLite's single writer (`max_connections=1`), but the *send* happens after `apply_command` returns; without the guard, concurrent publishers could misorder frames. This is covered by a dedicated concurrency test (§10).

### RingBuffer
- `VecDeque<Arc<ServerMsg>>`, bounded by **both** count (`1024` events) and age (`5 minutes`); whichever bound is hit first evicts the oldest. Bounds are constants, tunable later.
- `push(msg)` appends and evicts; `range_from(seq)` returns events with `seq >= from_seq` if the whole requested range is still resident, else `None` (caller falls back to the cold tier).

## 6. Seq-bearing content (generic event substrate)

M4 introduces no document write path. The driver message `EmitTest { nonce }` produces an **empty-ops** `UnsequencedCommand { world_id, author, ts, ops: [] }` routed through `Room::publish` → `repo.apply_command`. This:
- allocates a real per-world seq (`UPDATE worlds SET seq = seq + 1 RETURNING seq`),
- appends a row to the existing `world_events` log (replayable by `events_since`),
- applies nothing to the document store (empty ops),
- broadcasts `ServerMsg::Event { command }`.

Field provenance: `author` = the authenticated session user id of the publishing connection; `ts` = server wall-clock millis stamped at publish (server-authoritative, not client-supplied); `world_id` = the joined world. `nonce` is a **client-side correlation token only** — it lets a publisher match its `EmitTest` to the resulting `Event` (the next seq it receives) and is **not** persisted or carried in `Command` (no domain-model change). Convergence is asserted on the seq-ordered command stream itself (`seq` is globally unique per world); `author`/`ts` are incidental. `Command` / `Operation` are untouched, and M5's real ops use the identical publish path and frame.

## 7. Wire protocol

JSON text frames, internally-tagged envelopes, generated to TypeScript via the existing ts-rs pipeline (consistent with the ts-rs / Zod validation invariant, ARCHITECTURE §3). Binary encodings are rejected for M4: they bypass the type-generation pipeline and reduce debuggability.

```
ClientMsg (tag = "type", snake_case):
  | Hello         { world: Uuid, last_seq: Option<i64> }   // first frame after upgrade
  | EmitTest      { nonce: u64 }                            // M4 driver → empty-ops command
  | ResyncRequest { from_seq: i64 }
  | TimePing      { client_t0: i64 }
  | Pong

ServerMsg (tag = "type", snake_case):
  | Welcome      { world: Uuid, current_seq: i64, server_time: i64 }
  | Event        { command: Command }                      // sequenced broadcast
  | ResyncBegin  { from_seq: i64, to_seq: i64, source: ResyncSource }  // "buffer" | "log"
  | ResyncEnd    { current_seq: i64 }
  | TimePong     { client_t0: i64, server_t: i64 }
  | Ping
  | Error        { code: WsErrorCode, message: String }
```

All `ServerMsg` / `ClientMsg` types derive `TS` and emit to `src/types/generated/` (CI-enforced sync). **Transitive scope (call out in planning):** because `Event` embeds `Command`, ts-rs derives propagate across the whole document tree — `Command`, `Operation`, `FieldChange`, `Document`, `Scope`, `Source`, `PermissionSet`, `DocRole`, `Visibility`, `WorldRole`. These currently derive only serde. Adding `#[derive(TS)]` + `#[ts(export)]` to them is mechanical but broader than the message envelopes; it is "ahead of need" work serving the M6 TypeScript client and is deliberately included to satisfy the ts-rs invariant and de-risk M6. `serde_json::Value` (`system` body) maps to `unknown`.

## 8. Data flow

- **Join / reconnect:** session-gated upgrade → client sends `Hello { world, last_seq }` → server verifies the world exists (else `Error` + close) → subscribe to the room → `Welcome { current_seq, server_time }`. If `last_seq` is `Some(n)` with `n < current_seq`, the server immediately runs the resync path (this is the reconnect case).
- **Publish (intent → broadcast):** `EmitTest { nonce }` → `Room::publish` under guard → `apply_command` (empty-ops) → ring append → `tx.send(Event { command })` to all subscribers.
- **Sequence guard + resync.** There are three distinct triggers, all converging on one server-side replay routine:
  1. **Server-initiated on lag** — the connection's egress task observes `broadcast::RecvError::Lagged(n)` (it skipped `n` frames for this slow subscriber). It owns the socket, so it proactively replays the missed range, then resumes live. No client message involved.
  2. **Client-initiated on reconnect** — `Hello { last_seq: Some(n) }` with `n < current_seq`: the server replays from `n + 1` right after `Welcome`.
  3. **Client-initiated explicit** — the client's own sequence guard tracks `expected = last_seq + 1`; on a detected gap (or to force recovery) it sends `ResyncRequest { from_seq }`. In M4 the harness clients implement this guard (the real client lands in M6).
  
  The replay routine resolves the tier by `from_seq`:
  - **hot tier** — `RingBuffer::range_from(from_seq)` if fully resident,
  - **cold tier** — otherwise `repo.events_since(world_id, from_seq - 1)`,
  emitting `ResyncBegin { from_seq, to_seq, source }` … one `Event` per command … `ResyncEnd { current_seq }`. Because the egress task is the sole writer to the socket, replay and live delivery never interleave; live events that arrive during replay are delivered after `ResyncEnd`, deduplicated by seq so none are lost or doubled.
- **Time sync:** `TimePing { client_t0 }` → `TimePong { client_t0, server_t = now_millis() }`. The client computes `offset ≈ server_t − (client_t0 + client_t1) / 2` and `rtt = client_t1 − client_t0`. Server time is **wall-clock unix milliseconds** (required for later audio/combat alignment, invariant #2); seq remains the sole ordering authority. The client side only records offset/RTT in M4 (no consumer yet).
- **Liveness:** server sends `Ping` on a heartbeat interval; a connection that misses `N` consecutive `Pong`s is dropped. (axum's WS layer also surfaces protocol-level ping/pong; the app-level heartbeat is what the telemetry and drop policy key on.)

## 9. Telemetry & error handling

**Telemetry**
- Per-`Room` `RoomStats` atomics: `connections`, `events_published`, `gaps_detected`, `resyncs_hot`, `resyncs_cold`, `lagged_drops`.
- Structured `tracing` events: connection open/close (with world id + request id), gap detected, resync served (with tier), lagged drop.
- `GET /api/debug/rooms` (**AdminUser**-gated) returns a JSON array of `{ world_id, connections, current_seq, ...counters }` snapshots.

**Error handling**
- Nonexistent / malformed `Hello` world → `Error` + close.
- Malformed frame → `Error`, connection kept.
- `apply_command` failure → `Error` to the publishing connection only; nothing is broadcast.
- Slow consumer → never blocks the producer; `Lagged` → resync.
- `test_server` boot/bind failure → log root cause, exit non-zero (CLAUDE.md active-remediation posture).

## 10. Testing

**Unit**
- `RingBuffer`: count eviction, age eviction, `range_from` resident-vs-evicted boundary.
- Resync tier selection: `from_seq` inside buffer → hot; older than buffer → cold (`events_since`).
- Protocol serde round-trips for every `ClientMsg` / `ServerMsg` variant; ts-rs bindings present and in sync.
- **Publish-ordering guard:** many concurrent `publish` calls on one room produce broadcasts in strict seq order (asserts the §5 invariant).

**Integration — desync-convergence harness (highest-value test)**
- Boot the server lib on an ephemeral TCP port; connect N real `tokio-tungstenite` clients to one world.
- Induce desync by **client behavior**: a client stops reading (forces server-side `Lagged`), ignores frames, or disconnects and reconnects with a stale `last_seq`.
- Drive an event stream via `EmitTest`.
- **Assert:** every client's final `(seq → command)` stream equals the authoritative `world_events` log for the world.
- Variants: hot-resync (gap within buffer); cold-resync (gap older than the buffer, forcing the `events_since` tier); reconnect with stale `last_seq`; all clients converge under mixed concurrent faults.
- Time-sync: a `TimePing` yields a `TimePong` with a sane `server_t`/RTT.

**`test_server` binary**
- Boots a throwaway server (in-memory or tempfile SQLite, a pre-created world) for manual/external WS clients; prints the bind address and world id.

## 11. Decisions locked in this brainstorm

1. **Generic event substrate.** M4 carries a generic sequenced frame driven by an empty-ops `EmitTest` command through the existing `apply_command` → `world_events` path; real document commands reuse the same frame and publish path in M5.
2. **Room model = registry + `tokio::broadcast` behind a clean `Room` / `RoomRegistry` interface.** The interface is the anti-rebuild boundary; the lossy `Lagged → resync` pairing is the correct server-authoritative fan-out semantics and directly exercises M4's core path. A per-world actor or external broker remains a localized future swap. The per-world publish-ordering guard is a tested invariant.
3. **WS auth = authenticated session + world-exists.** Every authenticated user may join any existing world and receives the full unfiltered stream; per-world roles + `PermissionContext` filtering attach to the per-connection egress task in M5.
4. **Resync tiers = hot ring buffer → cold `world_events`;** doc-state snapshot deferred to M5/M6.
5. **Wire format = JSON text frames, tagged envelopes, ts-rs-generated client types.**
6. **Server time = wall-clock unix millis + NTP-style offset/RTT calibration;** seq remains the ordering authority; client records offset/RTT with no consumer yet.
7. **Telemetry = per-room atomic counters + structured tracing + `AdminUser`-gated `GET /api/debug/rooms`.**
8. **Convergence harness = in-process server + real `tokio-tungstenite` clients + client-driven faults,** asserting each client's `(seq → command)` stream equals `world_events`; plus a spawnable `test_server` bin.
9. **Ring buffer bounds = 1024 events or 5 minutes, whichever first** (tunable constants).
