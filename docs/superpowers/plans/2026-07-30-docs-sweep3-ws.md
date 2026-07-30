# Docs Sweep 3 — Realtime (ws/) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: on a Fable-class model use
> `mainline-plan-execution`; otherwise superpowers:subagent-driven-development.
> Steps use checkbox (`- [ ]`) syntax.

**Goal:** Document `src/server/src/ws/` — measured backlog 157 (`protocol.rs`
98, `room.rs` 33, `conn.rs` 22, `mod.rs` 4; `time.rs` + `test_support.rs`
already clean) — then flip the whole `ws/` tree to deny.

**Architecture:** Same calibrated pattern (Sweep-1/2a/2b plans' Global
Constraints verbatim: docs-only, doctest policy, scoped-count gates, per-task
cargo gates, ts-rs bindings regenerated + shape-checked + committed per task,
enforcing-function citations cross-checked against
`shadowcat-codebase-realtime-sync` + `-scene-rendering` + `-chat`). Branch
`docs-sweep3-ws`. Ship with the LOCAL matrix (no GitHub watch). Reviews under
the no-shell protocol (pre-generated `.docs-tmp/review-diff.patch` + relayed
gate evidence; no cargo test for reviewers — it writes tracked bindings; no
edits/commits while reviewers run).

**Sweep-specific truthfulness hot spots (verify while writing, cite enforcers):**
- `protocol.rs` is the WIRE SURFACE: frame docs must agree with the client Zod
  mirror (`src/client/core/src/wire.ts`) AND the docs site's
  `docs/site/protocol.md` frame catalog (Phase 1 shipped it; any discrepancy
  found = fix whichever side is wrong, log it). Per-recipient filtering claims
  cite `filter_command`/`send_filtered`; clipped-frame fields (MoveStream
  nullable cost, mover_vision) cite the egress clip.
- `room.rs`/`conn.rs`: gate claims post-D9 — the per-cell traversal decision is
  `move_exec::execute_move`/`gate_walk`, NEVER `Room::publish` (2b's review
  caught this stale citation twice; do not reintroduce). Hot-path claims (no
  pool reads in `send_filtered`) cite the in-memory actor-table join.

## Model/Effort directives

Mainline (Fable 5, effort high) per standing directive. Final review: the
no-shell reviewer pair, findings fixed pre-merge.

## Buddy-check directives

No high-risk signals (docs + lint attrs only). Standard final review only.

---

### Task 1: protocol.rs (98)

- [ ] **Step 1:** Scoped count — expect 98; enumerate live.
- [ ] **Step 2:** Document every frame struct/enum/field. Server→client and
  client→server framing, correlation ids, sequencing semantics, per-recipient
  redaction points — each frame doc one purpose sentence + field docs stating
  units/semantics. ts-rs docs land in `ServerMsg.ts`/`ClientMsg.ts` etc. that
  the docs-site protocol page links — write for both audiences.
- [ ] **Step 3:** Doctests per policy (wire types are serde — runnable
  construction/serde round-trips where cheap; pure helpers runnable; private →
  text).
- [ ] **Step 4:** Gates + bindings shape-check + cross-check vs
  `docs/site/protocol.md` (no drift between the page and the new frame docs).
- [ ] **Step 5:** Commit.

### Task 2: room.rs (33) + mod.rs (4)

- [ ] Same steps. Room lifecycle (cold-hydration, broadcast fan-out, resync,
  eviction), gate-claim discipline per the hot-spots note. Commit.

### Task 3: conn.rs (22)

- [ ] Same steps. Connection lifecycle (hello/welcome, watermark replay,
  subscription re-establishment, enrich_vision_explored's fail-closed absent-
  grid behavior — cite it exactly; it's in the core skill's never-fork table).
  Commit.

### Task 4: Deny flip + verify + sync + ship

- [ ] **Step 1:** Inner deny attr pair in `protocol.rs`, `room.rs`, `conn.rs`,
  `time.rs`, `test_support.rs`, then `mod.rs` (cascade legal once all children
  are 0 — same end-state as data/).
- [ ] **Step 2:** Mutation proof on protocol.rs + the mod.rs cascade; restore
  via python re-insert (never `git restore`).
- [ ] **Step 3:** Full local matrix (cargo suite/fmt/clippy -D, pnpm -r
  typecheck/test, lint, script tests, docs:check-examples, docs:build, both
  e2e suites).
- [ ] **Step 4:** Docs-sync: PLAN.md Sweep-3 entry; realtime-sync skill Gotcha
  (ratchet live on ws/; ts-rs flow note). No-shell spec-review gate on the
  skill diff folded into the final review.
- [ ] **Step 5:** No-shell final review pair (pre-generated diff + relayed
  evidence), fix findings, merge `--ff-only`, push, local matrix stands in for
  CI, delete branch, memory update.

---

## Deferred (logged, not dropped)

- Sweep 4+: http/+auth/, scene/ (~200), chat/+dice/ (~160), client packages,
  modules. Then buddy-check convergence → final ratchet → skills reference pass.
