# Docs Sweep 2b — Data Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: on a Fable-class model use
> `mainline-plan-execution`; otherwise superpowers:subagent-driven-development.
> Steps use checkbox (`- [ ]`) syntax.

**Goal:** Document `src/server/src/data/engine/` — the typed engine-band structs
and the ingress-validation registry: `mod.rs` (examples on its 4 documented pub
fns + the private `normalize_engine`), `geometry.rs` (32), `registries.rs` (25),
`token.rs` (39), `scene.rs` (76) — then flip all five files to deny, completing
the `data/` subsystem's ratchet (2a covered the core ten).

**Architecture:** Same calibrated pattern as Sweeps 1/2a (their plans' Global
Constraints apply verbatim: docs-only, comment rules, doctest policy, scoped-count
gate, per-task cargo test/fmt/clippy `-D` green, ts-rs bindings regenerated +
shape-checked + committed per task, security/invariant claims cite enforcing
functions and get cross-checked against `shadowcat-codebase-actors-tokens` +
`shadowcat-codebase-scene-rendering` + `shadowcat-codebase-documents-permissions`).
Branch `docs-sweep2b-data-engine` off main; push at completion with the LOCAL CI
matrix (user constraint: no GitHub watch) — cargo suite, pnpm -r typecheck/test,
lint, script tests, docs:check-examples, example builds, docs:build, both e2e
suites.

**Measured backlog (re-verify per task):** scene.rs 76, token.rs 39,
geometry.rs 32, registries.rs 25, mod.rs 0 (examples only).

## Model/Effort directives

Mainline (Fable 5, effort high) per standing directive. Final review: the
reviewer pair under the NO-SHELL protocol — reviewers have no Bash (user
directive; enforced in the agent defs, which load on session start — a dispatch
from an older session must ALSO carry the strict no-mutation brief). Dispatcher
pre-generates `git diff main...HEAD > .docs-tmp/review-diff.patch` and relays
gate evidence in the brief; the dispatcher must NOT edit/stage/commit while any
reviewer runs.

## Buddy-check directives

No high-risk signals (docs + lint attributes only). Standard final review only.

## Doctest surface notes (verified at plan time)

- `mod.rs` pub fns are ideal RUNNABLE doctests: `is_engine_doc_type("token")`,
  `validate_engine("token", &json)` accepting a valid body and REJECTING an
  unknown field (pins deny_unknown_fields), `normalize_engine_opt` normalizing
  (absent optional → explicit null), `engine_of::<T>` defaulting on absent.
  Private `normalize_engine` → ` ```text `.
- The four struct files are field-doc heavy (serde structs). Every `validate`
  method (e.g. `TokenEngine::validate`) gets a runnable doctest that REJECTS a
  degenerate input (pins fail-closed) — the coordinate bound is shared with the
  movement gate STRUCTURALLY; docs must state that as a present constraint and
  cite the shared symbol, never restate the literal.
- Field docs for geometry/scene must be exact about units/coordinate spaces
  (scene units vs cells) and clipping/secrecy semantics where a field feeds the
  per-recipient egress (cross-check the scene-rendering skill; cite the
  enforcing function, e.g. the visibility mask or `clip_to_visible_mask`).

---

### Task 1: mod.rs examples + registries.rs (25)

- [ ] **Step 1:** Scoped counts — mod.rs expect 0 (examples still required on
  its 4 pub fns), registries.rs expect 25; enumerate live.
- [ ] **Step 2:** registries.rs field/variant docs (faction/condition/channel
  registry engine structs — state who reads each: sheets, render layer, chat).
- [ ] **Step 3:** Runnable doctests: mod.rs's 4 pub fns (incl. an
  unknown-field REJECTION example) + any registries pub fns; private → text.
- [ ] **Step 4:** Gates + bindings shape-check. **Step 5:** Commit.

### Task 2: geometry.rs (32)

- [ ] Same steps; unit/space exactness; validate fns pin fail-closed via
  runnable doctests. Commit.

### Task 3: token.rs (39)

- [ ] Same steps; ownership/link semantics cross-checked against
  actors-tokens skill (`actor_id` is THE link; instanced tokens excluded);
  `TokenEngine::validate`'s shared coordinate bound documented structurally.
  Commit.

### Task 4: scene.rs (76)

- [ ] Same steps; grid/movement-model/vision/lighting field docs cross-checked
  against scene-rendering skill (movementModel axis, grid kinds, fail-closed
  absent-grid semantics — no synthesized defaults). Commit.

### Task 5: Deny flip + verify + sync + ship

- [ ] **Step 1:** Inner deny attr pair (Sweep-1 wording) in all five engine
  files (mod.rs included — every child is now swept, no cascade hazard; this
  REPLACES 2a's item-scoped exception note: after this sweep, `data/mod.rs`
  can also take its inner attrs since the whole `data/` tree is swept — do
  that too, removing the item-scoped attr on `DataError`, and update the 2a
  notes in PLAN.md + the documents-permissions skill Gotcha accordingly).
- [ ] **Step 2:** Ratchet-bite mutation proof (executor-run, python
  remove/restore — never `git restore`), on one engine file AND on data/mod.rs.
- [ ] **Step 3:** Full local matrix (Architecture section list).
- [ ] **Step 4:** Docs-sync: PLAN.md (Sweep 2b complete; data/ fully ratcheted);
  skills — documents-permissions (update the Gotcha: whole data/ tree now
  deny'd incl. engine + mod.rs inner attrs), actors-tokens + scene-rendering
  (ratchet-live Gotcha for their engine files). Spec-reviewer gate on the skill
  diffs under the no-shell protocol.
- [ ] **Step 5:** Final review pair (no-shell protocol, pre-generated diff +
  relayed gate evidence), fix findings, merge `--ff-only`, push, local matrix
  stands in for CI, delete branch, memory update.

---

## Deferred (logged, not dropped)

- Sweep 3+: ws/ (~153: protocol 98, room 33, conn 22), http/+auth/, scene/,
  chat/+dice/, then client packages, then modules.
- Buddy-check convergence → final ratchet (crate-root deny; lib.rs) → skills
  documentation-reference pass.
