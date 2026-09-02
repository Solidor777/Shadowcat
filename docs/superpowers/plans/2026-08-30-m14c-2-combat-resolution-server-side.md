# M14c-2 Combat Resolution Server-Side Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. THIS RUN: executed via mainline-plan-execution (Fable session) per the Model/Effort directives below.

**Goal:** Wire `crate::formula` into every combat consumer so the server evaluates all combat formulas itself — recoveries, Mirror, durations, lifecycle policies, the movement budget — with a trusted-only egress default for stored resolved scalars and a `Hard` route-preview clamp.

**Architecture:** A new `combat::eval` module owns formula-host resolution and all evaluation; consumers (`transition`, `effects`, the movement gate, `handle_pathfind`) call it and never re-derive the host join or the pricing. One stored home per value: `CombatantResource.max`, `EffectLifecycle.resolved`, and `ResolvedLifecycle` are deleted; absent Tracked entries / `Duration.remaining: None` mean "full", materialized on first mutation.

**Tech Stack:** Rust (server crate), ts-rs → generated TS + Zod mirrors, Vitest.

**Spec:** `docs/superpowers/specs/2026-08-30-m14c-2-combat-resolution-server-side-design.md`

## Model/Effort directives

Mainline execution in the Fable session (user directive this session: "write the plan and execute
with mainline development. You may dispatch sub agents as you wish, as per standard fable rules").
No sdd-* dispatch loop. Reviews use the `shadowcat-codebase:` reviewer pair.

## Buddy-check directives

Pre-authorized by the user ("You may use buddy checking as seems appropriate"). Run
buddy-checking at two checkpoints, plus the final review:
1. After Task 4 — the transition/effects evaluation core (Tasks 2–4 diff).
2. After Task 7 — the egress stamp + movement gate + preview clamp (Tasks 5–7 diff).
3. Final: two-reviewer branch review (`shadowcat-codebase:shadowcat-spec-reviewer` +
   `shadowcat-codebase:shadowcat-code-reviewer`) before merge, per mainline-plan-execution.

## Global Constraints

- No lint suppressions of any kind (`#[allow]`, `#[expect]`, `eslint-disable`, `@ts-ignore`); no
  file-size allowlist entries — split instead (soft 5,000 / hard 10,000 lines).
- Rust test bodies in sibling files (`pnpm lint:inline-tests`); comments cite symbols, never
  files/lines; no milestone ids, spec pointers, dates, or history narration in code comments or
  test names (`check-comment-refs`).
- `dist/` must exist before any cargo build (`pnpm build` first in a fresh worktree).
- Deletions via `trash`, never `rm`/`Remove-Item`; commits always `git commit -- <paths>`.
- Doc gates at completion: `cargo test`/`clippy`/`fmt`, `pnpm -r test`, `pnpm -r typecheck`,
  `pnpm lint`, `pnpm lint:docs`, `pnpm lint:props`, `pnpm lint:comments`, `pnpm lint:file-size`,
  `pnpm lint:inline-tests`, `pnpm docs:check-examples`, skill checkers
  (`node scripts/check-skill-symbol-refs-cli.mjs`, `pnpm run test:scripts`).
- SQL schema unchanged (no migration concerns in this sub-project).

## Setup (before Task 1)

- [ ] `git worktree add -b m14c-2-combat-resolution C:/Dev/Shadowcat-m14c2 main` (short path —
  long worktree paths break vitest on Windows), `pnpm install`, `pnpm build` (rust-embed needs
  `dist/`), then `cargo test -p shadowcat` from `src/server/` to confirm a green baseline.

---

### Task 1: `combat::eval` — evaluation core

**Files:**
- Create: `src/server/src/combat/eval.rs` (+ `pub(crate) mod eval;` in `src/server/src/combat/mod.rs`)
- Create: `src/server/src/combat/eval/tests.rs` (sibling test file, `#[cfg(test)] mod tests;` in `eval.rs`)

**Interfaces:**
- Consumes: `formula::{parse, evaluate, SystemLeafResolver, FormulaError}`,
  `CombatSnapshot.hosts`, `Combatant`, `ResourceBinding`, `Formula`,
  `EffectLifecycle`/`EffectLifecycleDefaults`.
- Produces (all `pub(crate)` in `combat::eval`):
  - `formula_host<'a>(hosts: &'a HashMap<Uuid, Document>, c: &Combatant) -> Option<&'a Document>`
    — token-embedded actor copy when the combatant names a `token_id` whose doc embeds an actor
    (`/embedded/actor/0`, the same shape `effects::walk_any_host` reads), else the `actor_id`
    host, else `None` (an `Event`, or hosts all absent). Returns the DOCUMENT the resolver reads:
    for the token case that is the embedded actor child doc itself.
  - `eval_formula(f: &Formula, host: Option<&Document>) -> Result<f64, FormulaError>` —
    `Number(n)` ⇒ `Ok(n)` (no host needed); `Text(t)` ⇒ `parse` then `evaluate` with
    `SystemLeafResolver::new(host)`; `Text` with `host: None` ⇒ the `unknown-ref` error the
    resolver would produce for its first reference (implemented as evaluating against an
    empty-system dummy is WRONG — instead return `FormulaError` kind `unknown-ref` with detail
    naming the first reference path; a reference-free `Text` like `"2 + 3"` still evaluates).
    Simplest correct shape: a `NoHostResolver` unit struct whose `resolve` always returns
    `unknown-ref` for the joined path — reference-free text then evaluates fine.
  - `struct ResolvedNums { current: f64, max: f64 }` and
    `resolved_resource(def: &ResourceBinding, stored: Option<f64>, host: Option<&Document>)
    -> Result<ResolvedNums, FormulaError>` — `Mirror { value }` ⇒ both = `eval(value)` (stored
    ignored); `Tracked { max, .. }` ⇒ `max = eval(max)`, `current = stored.unwrap_or(max)`
    clamped to `[0, max]`. A negative evaluated max clamps to `0` (ingress can no longer
    guarantee non-negativity for evaluated text).
  - `struct LifecycleFlags { on_combat_end: bool, on_turn_end: bool, on_advance: bool }` and
    `lifecycle_flags(authored: Option<&EffectLifecycle>, defaults: &EffectLifecycleDefaults,
    host: Option<&Document>) -> Result<LifecycleFlags, FormulaError>` — per flag: the effect's
    authored formula, else the combat-snapshot default formula, else the engine fallback
    (`on_combat_end` true, `on_turn_end` false, `on_advance` true — the fallbacks
    `EffectLifecycle`'s field docs state); truthy = evaluated value `!= 0.0`. First formula
    error wins.
  - `duration_amount(amount: &Formula, host: Option<&Document>) -> Result<u32, FormulaError>` —
    `floor(eval)`; `< 1.0` or non-finite ⇒ `FormulaError` (kind `type`, detail
    `"duration amount must be >= 1"` for the range case; eval errors pass through).

- [ ] **Step 1: write failing tests** in `eval/tests.rs` covering: host precedence (token-embedded
  copy beats linked actor; linked actor when token has no embedded copy; `None` for an `Event`);
  `eval_formula` number passthrough, text over a real `system` band
  (`{"stats":{"speed":{"final":30}}}` with `"stats.speed.final * 2"` ⇒ 60), reference-free text
  with no host, referencing text with no host ⇒ `unknown-ref`; `resolved_resource` all four
  (Mirror, Tracked stored, Tracked absent ⇒ current == max, negative-evaluated max ⇒ 0);
  `lifecycle_flags` authored-beats-default-beats-fallback per flag + truthiness + error
  propagation; `duration_amount` floor, `0.4` rejected, text error passthrough.
- [ ] **Step 2:** `cargo test -p shadowcat combat::eval` — FAIL (module absent).
- [ ] **Step 3:** implement `eval.rs` per the contracts above.
- [ ] **Step 4:** `cargo test -p shadowcat combat::eval` — PASS; `cargo clippy` clean.
- [ ] **Step 5:** `git add src/server/src/combat/ && git commit -m "feat(combat): evaluation core over the formula engine" -- src/server/src/combat/`

### Task 2: shapes — delete the client-resolution fields

**Files:**
- Modify: `src/server/src/data/engine/combat.rs` (`CombatantResource`, `EffectLifecycle`,
  `ResolvedLifecycle`, `Duration.remaining` doc, `CombatantEngine::validate`)
- Modify: `src/server/src/data/engine/combat/tests.rs`, `src/server/src/combat/tests/mod.rs`
  (fixtures), `src/server/src/combat/effects.rs` + `src/server/src/combat/transition.rs`
  (minimal compile-fix only — the behavioral rewire is Tasks 3–4; here the dead reads of
  `resolved`/`.max` become reads of the new shapes with behavior temporarily preserved via
  `eval` calls where trivially equivalent, and `todo`-free: Tasks 3–4 land in the same PR before
  any gate runs, but each task must still compile and pass its own suite)

**Interfaces:**
- Produces: `CombatantResource { current: f64 }` (struct kept, `max` gone;
  `CombatantEngine::validate` now checks only finiteness of `current`/`initiative`/`tiebreak`);
  `EffectLifecycle { on_combat_end, on_turn_end, on_advance }` (`resolved` gone,
  `ResolvedLifecycle` deleted); `Duration.remaining` doc comment: "Remaining `unit`s until
  expiry; server-written; `None` = not yet ticked (full duration)."

Note on ordering: to keep every intermediate state green, Task 2 folds the MINIMAL consumer
rewires that the field deletions force (`effects::tick`/`expire_by_policy` signatures change to
receive `LifecycleFlags` from the caller; `transition::recover`/`resource` read
`eval::resolved_resource`). Tasks 3–4 then finish the semantics (lazy-full writes, error
surfacing, text evaluation tests). If the fold makes Task 2 unwieldy in practice, execute Tasks
2–4 as one commit series without pushing intermediate red states — never commit a red suite.

- [ ] **Step 1:** delete the fields/struct; update `validate`; fix fixtures
  (`registry_with_movement` etc. lose `max` on combatant entries; effect fixtures lose
  `resolved`); adjust `effects.rs`/`transition.rs` call sites to the new signatures with
  caller-supplied flags/nums (behavioral tests updated in Tasks 3–4).
- [ ] **Step 2:** `cargo test -p shadowcat` — PASS (tests asserting the old skip semantics are
  reworked in Tasks 3–4; any that cannot survive the shape change alone are rewritten HERE to
  assert the new evaluated behavior — never deleted).
- [ ] **Step 3:** `git commit -m "refactor(combat): one stored home per value — drop client-resolved max/resolved shapes" -- src/server/src/data/engine/ src/server/src/combat/`

### Task 3: transitions — evaluated recoveries, resources, error surfacing

**Files:**
- Modify: `src/server/src/combat/transition.rs` (`recover`, `resource`, `Working`),
  `src/server/src/combat/mod.rs` (module doc invariant)
- Modify: `src/server/src/combat/tests/*.rs`

**Interfaces:**
- Consumes: `eval::{formula_host, resolved_resource, eval_formula}`.
- Produces: `Working.eval_failures: Vec<String>` (deduped detail strings) and a
  `Working::flush_eval_notices(world, author, now)` that, when non-empty, appends ONE
  `Operation::Create` of a GM-only message (`build_message_doc`, `channel: "combat"`,
  `Audience::GmOnly`, `MessageKind::Normal`, one `Segment::Text` joining the deduped details) —
  called by `start`/`advance` before `coalesce_updates`, and by `end` after its expiry pass.

**Behavior:**
- `recover`: for each registry `Tracked` resource — flags per phase evaluated via
  `eval_formula(phase_formula(...), formula_host(..))`; on error push to `eval_failures` and
  continue. `resolved_resource` supplies `current`/`max` (absent entry ⇒ full); write
  `/engine/resources/<key>/current` when the clamped result differs from the PRIOR STORED value
  — materializing an absent entry is an ordinary `set_engine` write at
  `/engine/resources/<key>/current`: `set_pointer` creates missing intermediate OBJECT keys
  (only array growth fails `BadPath`), and the real `apply_intent` path applies changes through
  the same `apply_field_change`, so the harness and production agree. (Corrected during
  execution — the original note wrongly claimed the leaf write would fail.)
- `resource` (the `CombatResource` intent): Mirror-bound key ⇒ `Err(CombatError::Forbidden)`
  (uniform wording); Tracked ⇒ clamp against `eval(max)`; eval error ⇒ `Err(CombatError::Data)`
  (uniform wording); absent entry ⇒ start from full then apply.
- Module doc: replace the "nothing here evaluates" INVARIANT paragraph with the evaluated model.

- [ ] **Step 1:** write failing tests: text recovery applies (fixture actor
  `system.stats.mv = 10`, recovery `"stats.mv / 2"` at turn_start ⇒ +5 clamped); absent entry
  recovers/writes full object; eval-error recovery skips + exactly one GM-only notice Create
  in the command; Mirror `CombatResource` refused; Tracked `CombatResource` against evaluated
  text max. Invert `text_recoveries_apply_nothing_server_side` into
  `text_recoveries_evaluate_server_side`.
- [ ] **Step 2:** run — FAIL. **Step 3:** implement. **Step 4:** `cargo test -p shadowcat` PASS.
- [ ] **Step 5:** `git commit -m "feat(combat): transitions evaluate recovery and resource formulas server-side" -- src/server/src/combat/`

### Task 4: effects — evaluated lifecycle + lazy durations

**Files:**
- Modify: `src/server/src/combat/effects.rs` (`tick`, `expire_by_policy`), callers in
  `transition.rs` (`run_boundary`, `run_turn_end`, `end`)
- Modify: `src/server/src/combat/tests/*.rs`, `src/server/src/combat/effects/tests.rs` (if the
  suite lives beside; follow the existing test layout)

**Interfaces:**
- `tick(hosts, refs, boundary, unit, ctx: &EvalCtx) -> Result<(Vec<Operation>, Vec<String>), CombatError>`
  and `expire_by_policy(hosts, refs, pick: fn(&LifecycleFlags) -> bool, ctx: &EvalCtx)` with the
  same pair — where `EvalCtx` carries the combat's snapshotted `EffectLifecycleDefaults` and a
  `&HashMap<Uuid, Document>`-keyed way to reach each ref's HOST document as formula host (an
  effect's formulas evaluate over the document that hosts it — for a token host, its embedded
  actor copy, i.e. the same `walk_any_host` shape). The `Vec<String>` returns eval-failure
  details for the caller's `Working.eval_failures`.

**Behavior:**
- `tick`: flags from `eval::lifecycle_flags`; `on_advance` false ⇒ no decrement. `remaining`:
  `Some(n)` ⇒ `n − 1` (existing path); `None` ⇒ `eval::duration_amount(amount) − 1` (the first
  tick materializes); reaching `0` also clears `active`. Eval error ⇒ skip + report.
- `expire_by_policy`: flags evaluated per ref; error ⇒ skip + report.
- The three-effect skip tests invert to evaluated assertions; every effect fixture gains a
  real `system` band on its host and formula-driven `amount`/policy cases.

- [ ] **Step 1:** failing tests (text amount `"stats.focus"` with host leaf 3 ⇒ first tick
  writes `remaining: 2`; authored policy formula overriding the chain default; chain default
  applying when authored absent; eval-error effect skipped and reported; `on_advance`
  chain-false freezes the countdown). **Step 2:** FAIL. **Step 3:** implement. **Step 4:** full
  `cargo test -p shadowcat` PASS.
- [ ] **Step 5:** `git commit -m "feat(combat): effect lifecycle and durations evaluated server-side" -- src/server/src/combat/`
- [ ] **Step 6: BUDDY-CHECK checkpoint 1** — dispatch the buddy-checking skill over the Tasks 2–4
  diff (`git diff main...HEAD -- src/server/src/combat src/server/src/data/engine`); fold fixes
  in after convergence per its protocol.

### Task 5: egress stamp for stored resource scalars

**Files:**
- Modify: `src/server/src/data/sqlite.rs` (the `Operation::Create` arm of `apply_intent`, beside
  the `COMBATANT_DOC_TYPE` parent check)
- Modify: `src/server/src/data/sqlite/tests/commands_and_intents.rs` (or the suite file the
  existing combatant-create tests live in)
- Modify: `src/client/core/src/scene-docs.ts` (`buildCombatantDoc` stamps the same default so
  the optimistic view matches egress) + its Vitest suite

**Behavior:**
- In the Create arm, for `doc.doc_type == COMBATANT_DOC_TYPE`: if
  `doc.permissions.property_overrides` has no key `"/engine/resources"`, insert it with
  `Visibility::OwnerOrGm` — BEFORE `validate_property_overrides` runs, so the stamped entry is
  validated like any authored one. An explicit entry of any tier (including `all`) is respected
  untouched. Updates never touch it.
- `buildCombatantDoc`: stamp `"/engine/resources": "owner_or_gm"` into the built doc's
  `permissions.property_overrides` unless the caller supplied an entry for that key.

- [ ] **Step 1:** failing tests — server: a combatant Created without the key carries it
  post-apply; one Created with explicit `"all"` keeps `all`; egress (`filter_command` /
  document fetch path, whichever the existing redaction tests exercise): a non-owner non-GM
  recipient's copy lacks `/engine/resources`, the owner's and GM's copies keep it. Client:
  builder stamps by default, respects explicit entry.
- [ ] **Step 2:** FAIL. **Step 3:** implement both sides. **Step 4:** `cargo test -p shadowcat`
  + `pnpm --filter @shadowcat/core test` PASS.
- [ ] **Step 5:** `git commit -m "feat(combat): stored resource scalars default to owner-or-GM egress" -- src/server/src/data/ src/client/core/`

### Task 6: movement gate on evaluated resources

**Files:**
- Modify: `src/server/src/ws/room.rs` (`BudgetGate` resolution + `ResolvedBudget` construction;
  extract `resolve_budget` shared with Task 7), `src/server/src/scene/mod.rs` (ECS accessors the
  gate needs: the registry's binding for the movement resource and the combatant's formula-host
  document — `SceneEcs` already caches all involved docs)
- Modify: `src/server/src/ws/room/tests/movement_budget.rs`

**Interfaces:**
- Produces: `pub(crate) fn resolve_budget(...) -> BudgetResolution` in `ws::room` (exact
  signature settled at implementation: takes the scene guard's combat lookup products —
  combat engine, combatant id/engine/access, registry binding for `movement.resource`, the
  formula-host document, `per_cell`, `is_gm` — and returns the enum
  `{ NoGate, Exempt, NotYourTurn, Unresolvable, Resolved { budget_cells: Option<f64>, decrement: ResolvedBudget } }`),
  consumed by BOTH `Room::execute_move` and Task 7's pathfind clamp. Behavior table identical
  to today's inline block, with these deltas:
  - the entry/`max` come from `eval::resolved_resource` over the combatant's formula host
    (absent entry ⇒ full budget, NOT `BudgetUnresolvable`);
  - a Mirror-bound movement resource ⇒ `Unresolvable` (refusal for enforced callers, free move
    for exempt ones — same split as today's unresolvable arms);
  - an eval error ⇒ `Unresolvable` same split;
  - the decrement path materializes an absent entry (write the whole `CombatantResource`
    object, as in Task 3).

- [ ] **Step 1:** failing tests in `movement_budget.rs`: text max evaluated from the host's
  `system` band gates the move; absent entry ⇒ full budget then decremented (entry
  materialized); Mirror-bound resource ⇒ enforced caller refused, GM moves freely with no
  decrement; eval-error same; existing cases stay green.
- [ ] **Step 2:** FAIL. **Step 3:** implement (extraction first, then deltas). **Step 4:** PASS.
- [ ] **Step 5:** `git commit -m "feat(combat): movement budget evaluates resource formulas via the shared resolution" -- src/server/src/ws/ src/server/src/scene/`

### Task 7: `Hard` route-preview clamp in Pathfind

**Files:**
- Modify: `src/server/src/ws/conn.rs` (`handle_pathfind`), `src/server/src/ws/room.rs` (reuse
  `resolve_budget`), `src/server/src/scene/mod.rs` + `src/server/src/scene/pathfinding.rs` (an
  optional `budget_cells: Option<f64>` threaded into `SceneEcs::pathfind`, truncating with the
  SAME per-step pricing the router already accumulates; `PathOutcome` gains `truncated: bool`)
- Modify: `src/server/src/ws/protocol.rs` (`PathResult` gains `truncated: bool`),
  `src/server/src/ws/conn/tests/mod.rs`, `src/server/src/scene/tests/pathfind_and_vision.rs`
- Modify: client wire mirror (`ServerMsg` Zod schema in `src/client/core` — find via the
  existing `PathResult` mirror) + regenerate ts-rs if `PathResult` is ts-rs exported

**Behavior:**
- `handle_pathfind`, in the Step-3 read guard, when a `token` is named and authorized: resolve
  the gate via the SAME lookups `execute_move` uses (`active_combat_for_scene`,
  `combatant_for_token` with `ctx` + `world_defaults` — fetch `world_cap_defaults` before the
  guard, mirroring `Room::execute_move`) and `resolve_budget`. Under `Hard` for an enforced
  non-GM caller: `NotYourTurn` ⇒ the generic `PathError { message: "unreachable" }`;
  `Resolved` ⇒ pass `budget_cells` into `pathfind`; `Unresolvable` ⇒ the generic `PathError`
  (mirror of the executor's refusal). `Warn`/`None`/GM/exempt/no-token requests: no budget
  passed, `truncated: false`.
- Router truncation: stop appending steps once cumulative cost would exceed the budget
  (grid: per-step via the leg walk that already sums `cost`; continuous: cut the final span at
  the budget boundary the same way the arrest cut works), set `truncated`. `arrested` semantics
  untouched.
- Parity test: same scene/combat/budget — the executor's stop cell equals the preview's last
  path cell for a straight and a diagonal-heavy route, exercised through `resolve_budget` +
  the shared pricing; one-time sabotage check (perturb the preview's budget by 0.5 ⇒ test
  fails; restore ⇒ empty diff), recorded here as performed, not kept.

- [ ] **Step 1:** failing tests (conn-level: not-your-turn preview refused generically;
  clamped preview truncates with flag; GM/Warn untouched; scene-level router truncation
  unit tests). **Step 2:** FAIL. **Step 3:** implement. **Step 4:** PASS incl. client
  typecheck. **Step 5:** `git commit -m "feat(combat): Hard route previews clamp at the movement budget" -- src/server/src/ src/client/core/`
- [ ] **Step 6: BUDDY-CHECK checkpoint 2** — buddy-check the Tasks 5–7 diff.

### Task 8: generated types + client ripples

**Files:**
- Regenerate: `src/types/generated/**` (ts-rs, via `cargo test` export or the repo's regen flow)
- Modify: `src/client/core/src/scene-docs.ts` + `src/client/core/src/index.ts` (Zod mirrors for
  the changed shapes: `CombatantResource` without `max`, `EffectLifecycle` without `resolved`,
  `PathResult.truncated`; `buildEffectDoc` doc/examples; delete any helper writing
  `resolved`/`remaining`/`max` — audit with `grep -rn "resolved\|remaining\|max" src/client/core/src/scene-docs.ts`)
- Run: `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint`

- [ ] **Step 1:** regenerate types; fix Zod mirrors + drift-guard tests to the new shapes; update
  builder examples. **Step 2:** full client suite + typecheck PASS. **Step 3:**
  `git commit -m "feat(types): regenerate combat shapes; client mirrors follow" -- src/types/ src/client/`

### Task 9: docs, skills, gates, merge

- [ ] Rewrite affected doc comments repo-side (module docs done in Tasks 3–4; sweep
  `grep -rn "client resolves\|never evaluates\|formula library" src/server/src/combat src/server/src/data/engine/combat.rs`
  for stragglers).
- [ ] Spec pointers: M14b spec §4.2–4.3/§5.1/§7.3 gain "*Amended (M14c-2)*" lines; PLAN.md marks
  M14c-2 done; HISTORY.md delivery entry.
- [ ] Skill updates in `~/.claude/skills/shadowcat-codebase/`: `shadowcat-codebase-combat`
  (evaluated-model invariant replaces the interim paragraph; `eval` seam documented),
  `shadowcat-codebase-formula` (combat consumer), `shadowcat-codebase-scene-rendering`
  (shared `resolve_budget`, preview clamp). Dispatch
  `shadowcat-codebase:shadowcat-spec-reviewer` on the skill diff; run
  `node scripts/check-skill-symbol-refs-cli.mjs` + `pnpm run test:scripts`; commit + push the
  plugin repo separately.
- [ ] Full gate run (Global Constraints list) in the worktree; fix anything red.
- [ ] Final two-reviewer branch review (Buddy-check directives item 3); address findings.
- [ ] Merge `--no-ff` to main, run both suites on main, push, `gh run watch`.
