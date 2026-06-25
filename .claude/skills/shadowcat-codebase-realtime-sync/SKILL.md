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

## Pointers

- Rationale: `docs/design/ARCHITECTURE.md` §2 (invariants 1-4) + §3 (tokio/axum/sqlx/argon2).
- Relationships:
  `graphify query "websocket room broadcast egress optimistic rollback store session auth"`.
- History: [[m6a-client-core]], [[m6c-1-search]], [[m6c-2-live-search]].
