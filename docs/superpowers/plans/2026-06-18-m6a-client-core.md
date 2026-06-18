# M6a — Client Core Foundation Implementation Plan

> Executed via `mainline-plan-execution` (TS): inline TDD per task, per-task
> enumerative spec-compliance check, ONE dispatched final branch review.
> Branch: `m6a-client-core`. Spec:
> `docs/superpowers/specs/2026-06-18-m6a-client-core-design.md`.

**Goal:** `@shadowcat/core` gains a WS client, a single Zod-validated document
store, and optimistic-apply + rollback over the M5 protocol. No server changes,
no modules/hooks/search/UI.

## Approved decisions
Hand-written Zod + CI type-assignability guard; client-side `author`+seq FIFO
correlation (no server change); minimal `subscribe`/`subscribeDoc` observer;
one active world per client instance; **test transport refinement** — CI
integration via a TS in-process mock of the M5 protocol (the `web` job has no
Rust toolchain); real Node↔Rust e2e deferred to a separate local/CI harness.

## Constraints
- Pure TS, Svelte-free, framework-neutral; ESM. Runs in the `web` CI job
  (`pnpm -r typecheck`, `pnpm -r test`, `pnpm lint`).
- Wire types come from `@shadowcat/types` (ts-rs output) as compile-time types;
  Zod validates inbound frames at runtime.
- Commit trailers on every commit:
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Htozbntnxh8N3meNWAeoNp
  ```

---

### Task 1: Zod wire schemas + drift guard + document store

**Files:** `src/client/core/package.json` (add `zod`), `src/client/core/src/`
(`wire.ts` schemas, `store.ts`, tests).

- [ ] **Step 1:** add `zod` to `@shadowcat/core` deps; `pnpm install`.
- [ ] **Step 2 — `wire.ts`:** Zod schemas for the inbound/outbound frames and
  payloads actually used: `Scope`, `PermissionSet` (+ `CapabilityGrants`),
  `Document`, `FieldChange`, `Operation` (tagged on `op`), `Command`,
  `ServerMsg` (tagged on `type`: welcome/event/reject/resync_begin/resync_end/
  time_pong/ping/error), `ClientMsg` builders (hello/intent/resync_request/
  time_ping/pong). `serde_json::Value` fields (`system`, `old`, `new`) →
  `z.unknown()`. i64/bigint: ts-rs emits `bigint`; parse JSON numbers/strings to
  `bigint` (seq/ts) — decide the number representation (Step 2a).
- [ ] **Step 2a — number rep:** JSON serializes i64 as a number; ts-rs types it
  `bigint`. Choose: parse seq/ts as `number` in the client (safe < 2^53 for
  seq/ms-ts) with a Zod `z.number()`, and document the divergence from the
  `bigint` ts-rs type, OR coerce to `bigint`. Recommend `number` for ergonomics;
  the drift guard (Step 4) will need a targeted exception or a small adapter.
- [ ] **Step 3 — `store.ts`:** `DocumentStore` holding `Map<string, Document>` +
  `appliedSeq`. `applyCommand(cmd)`: Create → set; Delete → delete by id;
  Update → clone doc, apply each `FieldChange` via a JSON-pointer set
  (mirroring server `set_pointer`: set-only, create intermediate objects),
  re-validate with the `Document` schema. `get(id)`, `query(docType)`.
- [ ] **Step 4 — drift guard test:** a `*.test-d.ts` (vitest `expectTypeOf` or
  tsd) asserting `z.infer<typeof DocumentSchema>` is assignable to/from the
  ts-rs `Document` (modulo the documented number/bigint exception). Fails CI on
  drift.
- [ ] **Step 5 — unit tests:** store apply for Create/Update/Delete, pointer
  set into nested/`/system`/`/embedded`, validation rejection on a malformed doc.
- [ ] **Step 6:** `pnpm --filter @shadowcat/core typecheck && test`. Commit.

---

### Task 2: WS client (connect, sequence guard, resync, reconnect, time)

**Files:** `src/client/core/src/` (`transport.ts` interface, `ws-client.ts`),
tests (`mock-server.ts` TS protocol mock + tests).

- [ ] **Step 1 — transport interface:** a thin `Transport` abstraction
  (`send(text)`, `onMessage`, `onOpen/onClose`, `close()`) so production uses the
  platform `WebSocket` and tests inject a fake/mock. Keeps `ws-client.ts`
  transport-agnostic (browser `WebSocket` can't set a cookie header; Node tests
  need that — the abstraction sidesteps it for unit/mock tests).
- [ ] **Step 2 — `mock-server.ts`:** an in-memory TS implementation of the M5
  server protocol over a paired in-process transport: tracks a world seq, applies
  intents (minimal: assign seq, echo `Event`; supports a scripted `Reject` and a
  conflict rule for tests), serves `Welcome`/resync on request. Enough to drive
  the client end-to-end without Rust.
- [ ] **Step 3 — `ws-client.ts`:** Welcome handling (seed `appliedSeq`,
  `next_expected`), the client **sequence guard** (drop `<`, apply `==`, gap `>`
  → `ResyncRequest`), `ResyncBegin/Event*/ResyncEnd` replay application, reconnect
  with exponential backoff + jitter (re-Welcome → `ResyncRequest` from
  `appliedSeq+1`), `TimePing`/`TimePong` offset + `serverNow()`, `Ping`/`Pong`.
  Routes applied commands into the `DocumentStore`.
- [ ] **Step 4 — tests** (against the mock): join → receive events in order;
  injected gap → `ResyncRequest` → replay converges; reconnect → resync;
  duplicate/old event dropped; time offset computed.
- [ ] **Step 5:** typecheck + test. Commit.

---

### Task 3: Optimistic-apply + rollback engine

**Files:** `src/client/core/src/` (`optimistic.ts` or fold into the store/client),
tests.

- [ ] **Step 1 — invert:** a TS `invert(op)` mirroring the Rust `Operation::invert`
  (Create↔Delete; Update swaps each change's old/new, reversed). Unit-tested for
  round-trip.
- [ ] **Step 2 — engine:** `view = base + ordered pending`. `applyIntent(ops)`:
  uuid `intent_id`, push `{intent_id, ops, inverse}` to `pending`, send
  `Intent`, recompute `view`, return a handle (resolve on confirm / reject with
  reason). Confirm: own authored echo (`author==self`, FIFO) → apply to `base`,
  pop pending, recompute. Reject: drop pending entry, recompute, reject handle.
- [ ] **Step 3 — tests** (mock server): optimistic create → confirm replaces
  prediction; optimistic update → `Reject{conflict}` → rollback; interleaved peer
  event keeps `view` correct; two simulated clients converge to the same state.
- [ ] **Step 4:** typecheck + test. Commit.

---

### Task 4: Lint, types, docs; CI wiring; (deferred) real e2e

- [ ] ESLint over the new `.ts` (note: the repo's flat config currently skips TS
  — `docs/TODO.md`. If still unconfigured, add a `files: ["**/*.ts"]` +
  typescript-eslint block scoped to client/core, or document the gap). `pnpm
  lint` clean.
- [ ] `pnpm -r typecheck`, `pnpm -r test` green; confirm the `web` CI job runs
  them. Public API surface exported from `@shadowcat/core` `index.ts`.
- [ ] Mark M6a in `docs/PLAN.md`; log the deferred Node↔Rust e2e harness in
  `docs/TODO.md`.
- [ ] Commit; final dispatched branch review → finishing-a-development-branch.

## Self-review
- Spec §5 WS client → T2; §6 store → T1; §7 optimistic → T3; §4 Zod+guard → T1.
- Refinement vs spec §10.5: CI integration uses a TS protocol mock (the `web`
  job has no Rust); real Node↔Rust e2e deferred (T4 / follow-up). Flagged.
- Open: i64→number vs bigint (T1 §2a); ESLint-for-TS gap (T4).
