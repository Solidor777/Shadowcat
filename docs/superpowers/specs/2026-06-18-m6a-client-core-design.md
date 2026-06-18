# M6a — Client Core Foundation: Design Spec

> Status: **DRAFT for review.** First of the three M6 sub-milestones (M6a client
> core, M6b modules+capabilities, M6c search). Scope: the framework-neutral TS
> client core — WS client, single Zod-validated document store, and
> optimistic-apply + rollback — over the existing M5 protocol. No modules, no
> hooks, no search, no UI (those are M6b/M6c/M7).

## 1. Goal

A Svelte-free, framework-neutral TypeScript module (`src/client/core/`) that:
1. connects to the server's `/ws` and maintains a faithful, **Zod-validated**
   mirror of one world's document tree, recovering correctly across gaps and
   reconnects;
2. lets a caller **optimistically** apply a write locally, send it as an M5
   `Intent`, and **roll it back** on `Reject` — using the M2 reversible op
   representation — while staying convergent with the authoritative log.

It is the substrate the document store, hooks, modules (M6b), search (M6c), and
eventually the UI (M7) build on. Tested headless against the M4/M5 test-server.

## 2. Non-goals (explicitly later)

Hooks / service registry, module manifest+loader, capability *awareness* and the
declarative capability model (all M6b); FTS5 / `Core.search` (M6c); any UI/
framework binding (M7). Server changes — M6a consumes the M5 protocol **as-is**
(no new server endpoints or frames).

## 3. Package & boundaries

- Lives in `src/client/core/` (currently a stub). Pure TS, no Svelte/DOM deps,
  buildable to a framework-neutral ESM module. Tests via vitest.
- **Wire contract:** the server already emits ts-rs types to
  `src/types/generated/` (`ServerMsg`, `ClientMsg`, `Command`, `Operation`,
  `FieldChange`, `Document`, `Scope`, `PermissionSet`, …). The client imports
  these as **compile-time** types — the single source of truth for wire shape.
- **Runtime validation:** every inbound server frame crosses a trust boundary,
  so it is parsed with **Zod** schemas before use (PLAN: "Zod-validated store").
  See §4 for the ts-rs↔Zod drift question.

## 4. Wire contract & validation

ts-rs gives TS *types* but not *runtime* validators. Options for the Zod layer:

- **(A) Hand-written Zod schemas** for the wire types, kept in the client core.
  Simple; risks drift from the Rust types.
- **(B) Generate Zod from the Rust types** (e.g. a ts-rs companion or a schema
  emitter) so they cannot drift. More tooling up front.
- **(C) Minimal/structural validation** (tag + shape checks) without full Zod.
  Lightest; weakest guarantee, and contradicts the "Zod-validated" directive.

Recommendation: **(A) now, with a CI guard** — a test that asserts each
hand-written schema's inferred type is assignable to the corresponding generated
ts-rs type (a `expectTypeOf`/`tsd`-style check), so drift fails CI. Revisit (B)
if the surface grows. (Open decision §10.1.)

## 5. WS client

- **Connect** to `ws(s)://…/ws?world=<id>` reusing the session cookie (the
  browser sends it automatically; tests pass it as a header, as the M5 harness
  does). First server frame is `Welcome { world, current_seq, server_time }`.
- **Frame handling:** parse each frame with Zod; dispatch by tag. Unknown/invalid
  frames are logged and dropped (never throw into the socket loop).
- **Client sequence guard** (mirrors the server egress guard, inverted):
  track `next_expected` (= `current_seq + 1` from `Welcome`). On `Event{command}`:
  `seq < next_expected` → drop (duplicate/replay); `seq == next_expected` →
  apply, advance; `seq > next_expected` → **gap** → send
  `ResyncRequest { from_seq: next_expected }` and buffer/ignore until the replay
  fills it. `ResyncBegin/ResyncEnd` bracket replayed `Event`s; apply them in
  order; `ResyncEnd.current_seq` reconciles the watermark.
- **Reconnect** with exponential backoff + jitter. On reconnect, re-`Welcome`;
  if `current_seq > last_applied`, `ResyncRequest { from_seq: last_applied + 1 }`
  to catch up from the ring/log (cold tier covers arbitrary gaps).
- **Time sync:** periodic `TimePing { client_t0 }` → `TimePong { client_t0,
  server_t }` yields a server-time offset (consumed later for scheduling); store
  the offset, expose `serverNow()`.
- **Heartbeat:** reply to `Ping` (and/or send `Pong`) to keep the socket alive.

## 6. Document store

- Holds one world's documents keyed by `id`, plus the applied `seq`.
- `applyCommand(cmd)`: for each op — `Create{doc}` inserts; `Delete{doc}` removes
  by id; `Update{doc_id, changes}` applies each `FieldChange` by JSON-pointer set
  (mirroring the server's `set_pointer`: set-only; `/system`, `/embedded`, etc.).
  Each resulting document is Zod-validated; a validation failure is surfaced as a
  store error (indicates client/server schema drift), not silently swallowed.
- Read API: `get(id)`, `query(docType)`, and a **framework-neutral subscription**
  (see §8). The store is the *single* source of truth (PLAN: "built once here").

## 7. Optimistic-apply + rollback (the core of M6a)

Model the visible state as **authoritative base + an ordered list of pending
optimistic intents**:

- `base` — the last authoritative state (everything applied from confirmed
  `Event`s, including other clients').
- `pending` — ordered `[{ intent_id, ops, inverse }]`, where `inverse` is the
  reverse ops computed from the M2 representation (`Operation::invert`:
  Create↔Delete; Update swaps each `FieldChange.old`/`new`, reversed order).
- `view` — `base` with all `pending` ops applied in order. **This is what callers
  observe.** Recomputed whenever `base` or `pending` changes.

Flow:
- `applyIntent(ops) -> intentId`: generate `intent_id` (uuid), compute `inverse`,
  push to `pending`, send `ClientMsg::Intent { intent_id, ops }`, recompute
  `view` (instant local feedback). Returns a handle resolving on confirm/reject.
- **Confirm** (the originator's own authored echo): the server broadcasts
  `Event { command, intent_id: None }`. The client correlates its own events by
  `command.author == self && command.seq` in **FIFO order** of `pending` (M5's
  documented approach — no server change). On match: apply the command to `base`,
  pop that `pending` entry, recompute `view`. (Convergence: `base` now holds the
  authoritative result; the optimistic prediction is discarded in favor of it.)
- **Reject** `ServerMsg::Reject { intent_id, reason }`: remove that `pending`
  entry (no `base` change), recompute `view` (the optimistic change vanishes),
  and reject the caller's handle with `reason` (`Forbidden`/`Conflict`/`Invalid`).
- **Interleaving / conflicts:** other clients' `Event`s update `base`; `view`
  stays `base` + remaining `pending`. If our intent is rejected for `Conflict`
  (a peer changed the pre-image first), `base` already reflects the peer's change
  and dropping our pending yields the correct converged `view`. No manual merge.

This is deliberately a **last-writer-authoritative reconciliation**, not OT/CRDT:
the server is the single source of truth; optimism is a local prediction that is
either confirmed (replaced by authoritative) or rolled back.

> Correlation edge: a *single* connection's own events arrive in seq order, so
> FIFO matching of `pending` is sound. (Two writing connections for the same
> user is the M5-noted nuance; M6a assumes one writing client per session, which
> is the norm. Documented as a limitation.)

## 8. Reactivity / subscription API

Framework-neutral (no Svelte): a minimal observer surface — `subscribe(listener)`
returning an unsubscribe, plus targeted `subscribeDoc(id, listener)`. Emits on
`view` changes. M7's Svelte UI adapts this to runes; M6b hooks tap the same
events. Keep it tiny and synchronous; no framework primitives leak into core.

## 9. Testing

- **Unit:** store apply (Create/Update/Delete, pointer sets), the sequence guard
  (drop/apply/gap), and the reconciliation engine (optimistic apply → confirm
  replaces; → reject rolls back; interleaved peer events; conflict rollback) —
  all without a socket, driving the store/reducer directly.
- **Integration (headless):** spin the in-process M4/M5 test-server (as the Rust
  harness does) and drive the real TS client over a real WS: join → optimistic
  create → confirm; optimistic update → server `Reject{conflict}` → rollback;
  two clients converge; reconnect + resync. Asserted against the authoritative
  `world_events` tail. (Cross-runtime: Node WS client against the Rust server.)

## 10. Open decisions (for review)

1. **Zod source** — hand-written schemas + a CI type-assignability guard
   (recommended) vs generating Zod from the Rust types.
2. **Intent correlation** — client-side `author`+seq FIFO (recommended, no
   server change) vs adding an originator-directed confirm carrying `intent_id`
   (small server change to M5's protocol).
3. **Reactivity primitive** — minimal `subscribe`/`subscribeDoc` observer
   (recommended) vs adopting a tiny signals lib now.
4. **Store scope** — single active world per client instance (recommended for
   M6a) vs multi-world from the start.
5. **Integration-test transport** — reuse the existing Rust `test_server` binary
   / in-process server over a real port (recommended) vs a TS-side mock server.

## 11. Phase-1-of-M6a delivery slices (proposed)

1. Package scaffold + Zod wire schemas (+ CI drift guard) + the document store
   with `applyCommand` and subscriptions (unit-tested).
2. WS client: connect/Welcome, sequence guard, resync, reconnect/backoff,
   time/heartbeat (integration-tested against the test-server, read-only).
3. Optimistic engine: `applyIntent`, confirm/reject reconciliation, rollback
   (unit + integration, incl. conflict rollback and two-client convergence).
