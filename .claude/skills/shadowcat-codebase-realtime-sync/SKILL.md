---
name: shadowcat-codebase-realtime-sync
description: "Use when touching Shadowcat realtime: WebSocket transport, per-world rooms, broadcast/egress, sequence numbers + resync, the client document store and optimistic/rollback, sessions/auth, or live search. Covers src/server/src/{ws,http,auth} + src/client/core store. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Realtime & Sync

Orientation for the realtime transport (ws/http/auth) and the client store with optimistic
application + rollback.

## Purpose

The client sends intents over a WebSocket; the server validates, applies, and broadcasts ordered
events to per-world rooms. Every broadcast carries a per-world monotonic sequence number; clients
detect gaps and resync from a bounded event buffer or a snapshot. The client may apply intents
optimistically and roll back on divergence.

## Key files & seams

- `src/server/src/ws/room.rs` — `Room` (per-world), `RingBuffer` (time/size-bounded event buffer)
  + `range_from(from_seq)` for gap resync, `subscribe() -> (Receiver, seq)`, `current_seq()`,
  `broadcast_aux()` (out-of-band), `RoomRegistry`. `get_or_create` cold-hydrates the scene ECS:
  scene entities (`query_scene_entities`) **plus** the M10e-2 world config-docs
  `world-settings`/`light-gradation`/`vision-modes` + actors (`query_documents`), seeded via
  `SceneEcs::set_world_config`/`set_actors`; the live `apply_op` path keeps the side-tables current.
  **M1 additions:**
  - `Room::commit_ops_locked(repo, ctx, ops, ts)` (`pub(crate)`) — gate-free authoritative write
    tail (apply_intent → ECS-hydrate → ring/seq → broadcast Event → stats). Extracted from
    `publish`; PRECONDITION: caller MUST already hold `publish_guard`. Non-reentrant — do NOT
    re-acquire `publish_guard` inside (tokio `Mutex` would deadlock). Both `publish` and
    `execute_move` call this as their commit step.
  - `Room::execute_move(repo, ctx, scene_id, token, path, ts)` — server-authoritative token move.
    Acquires `publish_guard` at the TOP and HOLDS it across the entire validate→commit critical
    section (mirrors `publish` atomicity). Scene read locks are scoped and dropped before the
    `get_explored().await` (no lock across await); `publish_guard` (tokio `Mutex`) is intentionally
    held across awaits. Calls `move_exec::execute_move` (pure, lock-free), then `commit_ops_locked`
    (single acquisition, no re-entry). Atomic single position write (`/system/x` + `/system/y`
    OCC pre-image ops). Returns `MoveExecution { stop, render_path, duration_ms }`.
  - `moving: Mutex<HashMap<Uuid, i64>>` — per-token moving lock: token → move-end epoch-ms. Lazy
    expiry (no timer); absent or expired entry allows the move. Updated after each successful commit
    (still inside `publish_guard`). In-memory only — cleared on server restart (move state is derived,
    not durable).
- `src/server/src/ws/protocol.rs` — client/server message frames; `ServerMsg`, `event_seq()`.
- `src/server/src/ws/conn.rs` — per-connection loop + egress; `ws/time.rs` — server time source +
  client offset calibration (exists before its consumer, per ARCHITECTURE §2 invariant 2).
- `src/server/src/http/{routes.rs,mod.rs}` — HTTP routes (login, assets, embed).
- `src/server/src/auth/session.rs` — `SqlxSqliteStore` (DB-backed sessions), `spawn_session_sweep`,
  `SessionUser`/`AuthUser`/`AdminUser`; `auth/{password,role}.rs`.
- `src/client/core/src/ws-client.ts` — client WS connection + resync.
- `src/client/core/src/store.ts` — `DocumentStore implements ReadableDocuments` (authoritative,
  rollback base).
- `src/client/core/src/optimistic.ts` — `OptimisticClient implements ReadableDocuments` (the
  optimistic view the UI/canvas render).

## Hard invariants

- **Ordered, recoverable realtime** (ARCHITECTURE §2 invariant 2): every broadcast carries a per-world
  monotonic seq from an atomic counter; clients gap-detect and resync from the `RingBuffer` or a
  full snapshot.
- **Optimistic with rollback** (ARCHITECTURE §2 invariant 3): `OptimisticClient` applies locally tagged with
  an intent id; the server confirmation reconciles; divergence rolls back to `DocumentStore`.
  `appliedSeq` is identical across the two so the derived watermark holds
  [[render-from-optimistic-view]].
- **Socket-buffer backpressure is non-portable** — `SO_SNDBUF`/`SO_RCVBUF` are advisory; test the
  generic egress sink with a credit-gated `Sink`, not real-socket TCP backpressure
  [[socket-buffer-backpressure-nonportable]].
- **Debounce on the leading edge, arm only when idle** (or cap max staleness) — re-arming on every
  event starves under load [[debounce-leading-edge-not-trailing-rearm]].
- **Check-then-act across two pool queries needs one transaction** [[two-query-guard-needs-tx]].

## Gotchas

- **Permissions filter every broadcast per recipient** — hidden fields are stripped before
  transmission (see `shadowcat-codebase-documents-permissions`), never sent-then-hidden.
- **Live search rides the broadcast** as top-N subscriptions over the same egress
  [[m6c-2-live-search]].
- **One-shot correlated request pairs** (`Search`→`SearchResult`/`SearchError`;
  `Pathfind`→`PathResult`/`PathError`) route replies to the requesting connection only (never
  broadcast); correlated by `request_id` via the `pending` map in `WsClient`. See
  `src/client/core/src/ws-client.ts` and `src/server/src/ws/protocol.rs`.
- **MoveRequest → MoveStream (M2, broadcast):** `MoveRequest` is still a one-shot correlated pair
  for the mover's promise (resolves on the matching `move_stream` frame via `pending` map), but
  `MoveStream` is broadcast to ALL scene viewers, not just the mover. The server clips the sample
  list per-recipient based on the viewer's vision mask (mover gets full trajectory + `moverVision`;
  observers get clipped samples + `moverVision: null`). `MoveError` remains mover-only, always
  generic (no path geometry / vision state disclosed — no-geometry-leak invariant). `conn.rs`
  `handle_move_request` dispatches `execute_move`, then broadcasts `MoveStream` to the scene.
  Client animation is driven by `TokenAnimator.animateSamples` (time-tagged playback, catch-up on
  late arrival, gap/occlusion detection: gap threshold = `minConsecutiveDelta × 1.5` where
  `minConsecutiveDelta` is the minimum positive inter-sample interval across all consecutive pairs;
  Infinity for < 3 samples — no interior gap detectable). `animateSamples` cancels any competing
  ease-to-stop `anim` entry (handles Event-before-MoveStream ordering); `setTarget` is a no-op
  while `samplesAnim` is live (handles MoveStream-before-Event ordering). Wired end-to-end:
  `WsClient.onMoveStream` → `worldSession` → `SceneInteractionBridge.animateSamples` →
  `RenderEngine` → `TokenView` / `TokenAnimator`. `onMoveStream` listeners survive reconnects
  (NOT cleared in `failPending`).
- **Gated moves are request-only + server-executed (M1/M2 invariant):** the client sends
  `MoveRequest` and waits; the server validates, executes, and broadcasts `MoveStream`. The client
  MUST NOT apply an optimistic position update for a gated move. The atomic position `Event` (from
  `commit_ops_locked`) is the authoritative document update; the `MoveStream.samples` drive
  cosmetic animation for all scene viewers. The `moveRequest` promise resolves on success (the
  `MoveStream` frame) but the animation is broadcast-driven — no local `animateAlongPath` call
  on the mover side.

## Pointers

- Rationale: `docs/design/ARCHITECTURE.md` §2 (invariants 1-4) + §3 (tokio/axum/sqlx/argon2).
- Relationships:
  `graphify query "websocket room broadcast egress optimistic rollback store session auth"`.
- History: [[m6a-client-core]], [[m6c-1-search]], [[m6c-2-live-search]].
