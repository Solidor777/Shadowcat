# M14c-6 Combat Client Seams — Implementation Plan

> **For agentic workers:** Execute task-by-task in order; each task's steps use checkbox
> (`- [ ]`) syntax. Written for a sonnet-class implementer with NO conversation context: every
> shape you need is in the spec or in this file; read the cited code before editing it.

**Goal:** The combat clock becomes usable from the client: `AppContext.combat` (a
framework-neutral `CombatController` in `@shadowcat/core`, also the `shadowcat.service:combat`
service), the first nine `CoreHooks` entries derived from applied command deltas, server-resolved
resource numbers over a new `"combat"` scene-derived channel, a correlated `combat_result` reply
that fixes `WsClient.combat()`'s confirmation, and the `Warn` overage label in the route preview
over a new `PathResult.budget_cells`.

**Architecture:** Server: one reply frame, one `PathResult` field, one derived channel — no
transition, gate or document-shape change. Client core: `combat.ts` (controller), `combat-hooks.ts`
(declaration + pure derivation + queued emitter), `ws-client.ts` (correlation), `wire.ts` (Zod).
Shell: `WorldSession` wiring. scene-tools: label only.

**Tech Stack:** Rust (server crate, ts-rs), TypeScript (Vitest, Zod v4 mini), Svelte 5 (shell
wiring only), Playwright NOT used here (browser e2e is M14d).

**Spec:** `docs/superpowers/specs/2026-09-02-m14c-6-combat-client-seams-design.md` (decisions
S1–S14; read it first, in full).

**Worktree/branch:** `C:/Dev/Shadowcat-m14c6`, branch `m14c-6-combat-seams`. M14c-5
(`m14c-5-templates-merge`) merges to `main` before this plan executes; merge `main` into this
branch first (`git merge main`, never rebase) and re-run `pnpm install --frozen-lockfile` +
`pnpm build` so `dist/` exists before any cargo command.

## Execution directives

**Every dispatched agent's first prompt MUST contain this paragraph verbatim:**

> The iron rule is no deferrals of existing work, or new work as it comes up - we fix this now
> unless I give my EXPRESS authorization. The only exception is if a bug or to-do has a genuine
> blocker that is already logged in a milestone in PLAN.md that has not been started yet. Another
> iron clad is rule is that when faced with a design fork, determine the best long term shape in
> keeping with our plans and goals, and implement accordingly. You only need to ask me if the
> question "what is the best long term shape in keeping with our plans and goals?" is not able to
> answer the question. Churn is not a concern. This paragraph must be copied verbatim to any
> agents dispatched in this campaign.

…plus the reporting rule: a subagent must deliver its report as the Agent tool result OR write it
to a named document; state which in the prompt. Opus is banned for every dispatch (sonnet, or
fable for genuinely complex tasks). Reviewers get no Bash — pre-generate the diff for them.

## Buddy-check directives

Run buddy-checking (two blind reviewers + brokered debate) at:
1. After Task 3 — the server surface (`CombatResult`, `budget_cells`, the `"combat"` channel).
2. After Task 7 — the hook derivation table + emitter + `WorldSession` wiring.
3. Final: two-reviewer branch review before merge.

## Global constraints

- No lint suppressions of any kind (`#[allow]`, `#[expect]`, `eslint-disable`, `@ts-ignore`);
  no file-size allowlist entries — split instead (soft 5,000 / hard 10,000 lines).
- Rust test bodies in sibling files (`pnpm lint:inline-tests`); comments cite symbols, never
  files/lines; no milestone ids, spec pointers, dates or history narration in code comments,
  assert messages or test names (`pnpm lint:comments`).
- `dist/` must exist before any cargo build. Never run two cargo commands concurrently.
- Deletions via `trash`, never `rm`/`Remove-Item`/`git rm`; commits always `git add <paths>` +
  `git commit -- <paths>`; every commit message ends with the campaign trailer:
  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Do3ud57JJ1MpU52KpSJq1Z
  ```
- ts-rs: any Rust type change ⇒ regenerate (`cargo test` regenerates `src/types/generated`) and
  commit the bindings IN THE SAME COMMIT; `git diff --exit-code src/types/generated` must be
  clean at every commit that touches a `#[ts(export)]` type.
- Every public TS symbol needs a doc comment with an `@example` that typechecks
  (`pnpm docs:check-examples`); every Rust item needs a doc comment (`-D missing-docs`).
- Per-task gate (run before each commit): the crate/package tests the task touches plus
  `pnpm -r typecheck` for any client change. Full suite at Task 10.

---

### Task 1: `ServerMsg::CombatResult` + `handle_combat_intent` success reply

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (new `ServerMsg::CombatResult` variant beside
  `CombatError`, doc per spec §3.1)
- Modify: `src/server/src/combat/mod.rs` (`handle_combat_intent` returns
  `Some(ServerMsg::CombatResult { request_id, seq })` on success; `run_intent` returns the
  committed `Command`'s `seq` — read `Room::commit_combat`'s `Ok(Command)`)
- Modify: `src/server/src/ws/conn.rs` (only the comment on the `ClientMsg::Combat*` arm: "`Some`
  on success is the correlated `CombatResult`; a rejection is `CombatError`")
- Modify: `src/server/src/ws/conn/tests/combat_intents.rs` (tests below)
- Modify: `src/server/src/ws/protocol/protocol_tests.rs` (serde round-trip of the new variant)
- Generated: `src/types/generated/ServerMsg.ts`
- Modify: `src/client/core/src/wire.ts` (`ServerMsg` union member `combat_result` + Zod variant)
- Modify: `src/client/core/src/wire.test.ts` (drift guard case)

**Interfaces:**
- `ServerMsg::CombatResult { request_id: Uuid, seq: i64 }` — `#[serde(rename = "combat_result")]`
  under the existing tag convention; originator-only.

- [ ] **Step 1 (failing tests):** in `combat_intents.rs`, (a) a GM `CombatStart` yields exactly one
  `combat_result` on the GM's `erx` whose `seq` equals the broadcast `Event`'s `seq`; (b) a second
  connected player receives the `Event` and NO `combat_result`; (c) a rejected intent yields
  `combat_error` and no `combat_result`. Follow the file's existing fixture helpers for opening
  two connections.
- [ ] **Step 2:** implement the variant + return path. `cargo test -p shadowcat combat_intents`
  PASS; `cargo test -p shadowcat protocol` PASS.
- [ ] **Step 3:** regenerate bindings (a full `cargo test -p shadowcat` run regenerates), add the
  `wire.ts` member + schema, run `pnpm --filter @shadowcat/core test wire` PASS,
  `git diff --exit-code src/types/generated` clean after staging.
- [ ] **Step 4:** `cargo clippy --all-targets -- -D warnings` clean; `cargo fmt --all -- --check`
  clean.
- [ ] **Step 5:** `git add` the touched paths; `git commit -m "feat(ws): combat intents answer a correlated combat_result on success" -- src/server/src/ws/ src/server/src/combat/mod.rs src/types/generated/ServerMsg.ts src/client/core/src/wire.ts src/client/core/src/wire.test.ts`

### Task 2: `PathResult.budget_cells` through a shared `budget_cells` helper

**Files:**
- Modify: `src/server/src/ws/room.rs` (`pub(crate) fn budget_cells(current: f64, cost_to_resource: f64) -> f64`;
  `resolve_budget`'s `Resolved.budget_cells` uses it)
- Modify: `src/server/src/ws/conn.rs` (`handle_pathfind`: compute `budget_cells: Option<f64>` from
  the `Resolved { decrement: Some(d), .. }` branch when `bg.enforced`; thread it into the
  `PathResult` reply; keep the existing `Hard`-only truncation ceiling)
- Modify: `src/server/src/ws/protocol.rs` (`PathResult.budget_cells: Option<f64>`, doc per spec §3.2)
- Modify: `src/server/src/ws/room/tests/movement_budget.rs` + `src/server/src/ws/conn/tests/`
  (whichever file already covers `handle_pathfind`'s clamp — locate with
  `grep -rn "truncated" src/server/src/ws/conn/tests`)
- Generated: `src/types/generated/ServerMsg.ts`
- Modify: `src/client/core/src/ws-client.ts` (`PathResult.budget_cells: number | null`, map from the
  frame), `src/client/core/src/wire.ts` (schema), tests

**Interfaces:**
- `budget_cells(current, cost_to_resource) = current / cost_to_resource` — the ONE place this
  division lives; both the `Hard` ceiling and the preview number call it.
- `BudgetGate::enforced` is read where `handle_pathfind` already holds `bg` (it is a private
  field of `BudgetGate` — add a `pub(crate) fn enforced(&self) -> bool` accessor rather than
  widening the field).

- [ ] **Step 1 (failing tests):** owner under `Warn`: `budget_cells == Some(n)`, `truncated ==
  false`, full path; owner under `Hard` with a route longer than the budget: `truncated == true`
  AND `budget_cells == Some(n)`; GM: `Some(n)`, no truncation; a player whose token's combatant
  is hidden from them (`permissions.default: none`): `None`; a token bound to no combatant:
  `None`; `Spaces` interpretation: `budget_cells == current`; `PerCell` with `per_cell = 5` and
  `current = 30`: `Some(6.0)`.
- [ ] **Step 2:** parity test — for one fixture, `resolve_budget(..).budget_cells` under `Hard`
  equals `handle_pathfind`'s reported `budget_cells` for the same token; then sabotage by
  inlining a `* 1.0001` on one side, confirm the test fails, restore (record the run in the
  commit body).
- [ ] **Step 3:** implement; server tests PASS; regenerate bindings; client `PathResult` +
  schema + `ws-client.test.ts` case (a `path_result` frame with `budget_cells: 6` reaches the
  promise); `pnpm --filter @shadowcat/core test` PASS.
- [ ] **Step 4:** clippy/fmt clean; `git diff --exit-code src/types/generated` clean.
- [ ] **Step 5:** `git commit -m "feat(pathfind): route previews carry the mover's remaining movement budget" -- <paths>`

### Task 3: the `"combat"` derived channel

**Files:**
- Create: `src/server/src/combat/channel.rs` (`CombatsPayload`, `CombatView`, `CombatantView`,
  `ResolvedResourceView`, `ResourceBindingKind` — all `#[derive(Serialize, TS)]`,
  `#[ts(export, export_to = "../../types/generated/")]`, docs per spec §3.3)
- Create: `src/server/src/combat/channel/tests.rs` (declared `#[cfg(test)] mod tests;` from
  `channel.rs`)
- Modify: `src/server/src/combat/mod.rs` (`pub mod channel;`)
- Modify: `src/server/src/scene/mod.rs` (`SceneEcs::resolved_combats(&self, ctx, world_defaults)
  -> CombatsPayload`; `compute_derived`'s `"combat"` arm)
- Modify: `src/server/src/scene/tests/combat_index.rs` (channel tests over the ECS fixtures)
- Generated: `src/types/generated/CombatsPayload.ts`, `CombatView.ts`, `CombatantView.ts`,
  `ResolvedResourceView.ts`, `ResourceBindingKind.ts`; `src/types/index.ts` re-exports
- Modify: `src/client/core/src/wire.ts` (`CombatsPayloadSchema`, `parseCombats(payload: unknown):
  CombatsView` — fail-closed to `EMPTY_COMBATS` on a malformed payload, logged, mirroring
  `parseFootprints`), `src/client/core/src/index.ts` re-exports, `wire.test.ts`

**Interfaces (server):**
- Readability of a combat/combatant: `self.ctx_access(ctx, world_defaults, doc).has(cap::READ)`
  (the private `ctx_access` already exists — call it from the new method in the same `impl`).
- `/engine/resources` visibility: look the pointer up in `doc.permissions.property_overrides`
  (the `Visibility` enum), default `Visibility::All`, test `access.can_see(v)`. If a helper for
  "visibility of pointer P on doc D" already exists in `data::permission` (grep
  `property_overrides.get`), call it; do not write a second lookup.
- Resolution: `crate::combat::eval::resolved_resource(&binding, stored, host.as_ref())` with
  `stored = c.resources.get(key).map(|r| r.current)` and `host =
  self.combatant_formula_host(&c.kind)`; iterate `self.resource_registry_engine()`'s map in
  key order.
- `movement_cells`: `ce.movement.resource` → the resolved Tracked entry's `current`;
  `cost_to_resource` = `self.scene_per_cell(ce.scene_id)` under `PerCell` (None ⇒ None) or
  `1.0` under `Spaces`; then `crate::ws::room::budget_cells(current, ctr)`. Mirror-bound ⇒
  `None`.
- Sort combats by id and combatants by id (stable fingerprint).

- [ ] **Step 1 (failing tests):** in `scene/tests/combat_index.rs`, build an ECS with a registry
  (`movement: Tracked { max: "speed", recover … }`, `hp: Mirror { value: "hp" }`), one combat
  (active, `movement.resource = Some("movement")`, `PerCell`), three combatants (GM-owned NPC,
  player-owned PC with actor `system.speed = 30`, `system.hp = 12`, a hidden NPC), scene
  `per_cell = 5`. Assert: GM payload has 3 combatants with numbers (`movement: current 30 max
  30`, `hp: 12/12`, `movement_cells: 6`); player payload has 2 (hidden absent), own combatant
  with numbers, the NPC with `resources: None` + `movement_cells: None`; an unparseable formula
  yields `error: Some(..)` with `current`/`max` `None`; a materialized `stored = Some(10)` yields
  `current 10 / max 30`, `movement_cells: 2`; `Spaces` yields `movement_cells == current`; no
  `grid.distance` under `PerCell` yields `None`; a paused combat is present; two computations
  are equal (fingerprint stability).
- [ ] **Step 2:** parity test — `movement_cells` for the turn owner equals
  `resolve_budget(&budget_gate_for_token(..), false)`'s `budget_cells` under `Hard` for the same
  token; sabotage once each side (multiply by a constant), confirm failure, restore.
- [ ] **Step 3:** implement `channel.rs` + `resolved_combats` + the `compute_derived` arm. All
  server tests PASS; clippy (`-D warnings` AND the missing-docs invocation) clean; fmt clean.
- [ ] **Step 4:** regenerate bindings; add `src/types/index.ts` re-exports; Zod schema +
  `parseCombats` + `EMPTY_COMBATS` (`{ combats: [] }`) in core; `wire.test.ts` drift case + a
  malformed-payload fail-closed case. `pnpm -r typecheck` + `pnpm --filter @shadowcat/core test`
  PASS.
- [ ] **Step 5:** `git commit -m "feat(combat): per-recipient resolved resources over a combat derived channel" -- <paths>`
- [ ] **Step 6:** BUDDY CHECK 1 over Tasks 1–3 (`git diff main...HEAD -- src/server src/types`
  pre-generated to a scratch file for the two reviewers). Fold fixes in only after the debate
  converges; commit fixes with explicit paths.

### Task 4: `WsClient.combat()` correlation by `combat_result`

**Files:**
- Modify: `src/client/core/src/ws-client.ts` (`combatPending` entry gains `seq: number | null`;
  `case "combat_result"`; `applyEvent` post-apply sweep; delete the `case "event"` author-FIFO
  block; delete `WsClientOptions.selfUserId` and its doc; update `combat()`'s doc + `@example`)
- Modify: `src/client/core/src/ws-client.test.ts`
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts` (drop `selfUserId` from the
  `WsClient` construction — only that line here; the rest of the shell wiring is Task 7)

**Interfaces:**
- Resolution rule: an entry resolves when BOTH its `combat_result` has arrived AND
  `this.nextExpected > seq`. Implement as: on `combat_result`, if `msg.seq < this.nextExpected`
  resolve+delete now, else set `entry.seq = msg.seq`; in `applyEvent`, after `nextExpected`
  advances, resolve+delete every entry with `seq !== null && seq < this.nextExpected`.

- [ ] **Step 1 (failing tests):** (a) `combat_result` BEFORE the event: promise still pending
  after the reply, resolves after the `event` at that seq applies; (b) `combat_result` AFTER the
  event: resolves immediately; (c) an `event` authored by self with no `combat_result` resolves
  NOTHING (the old FIFO behaviour is gone); (d) `combat_error` rejects with the message; (e) the
  timeout rejects and clears the entry; (f) `failPending` on disconnect rejects; (g) two in-flight
  intents resolve independently by `request_id` regardless of reply order.
- [ ] **Step 2:** implement; `pnpm --filter @shadowcat/core test ws-client` PASS; `pnpm -r
  typecheck` PASS (the shell's `selfUserId` line removed).
- [ ] **Step 3:** `git commit -m "fix(ws-client): combat intents confirm by combat_result, never by an author-echo FIFO" -- src/client/core/src/ws-client.ts src/client/core/src/ws-client.test.ts src/client/shell/src/lib/worldSession.svelte.ts`

### Task 5: engine-defaults fixture + `newCombatEngine`

**Files:**
- Create: `src/client/core/src/__fixtures__/engine-combat-defaults.json`
- Modify: `src/client/core/src/scene-docs.ts` (export `ENGINE_COMBAT_DEFAULTS`; add
  `newCombatEngine(sceneId: string): CombatEngine`)
- Modify: `src/client/core/src/scene-docs.test.ts` (fixture equality; `newCombatEngine` shape)
- Modify: `src/client/core/src/index.ts` (re-exports)
- Modify: `src/server/src/data/engine/combat/tests.rs` (fixture equality — serialize
  `resolve_combat_rules(None, None, None)` into the client spelling; read the fixture by relative
  path the way `formula::tests::conformance` reads its corpus)

**Interfaces:**
- Fixture content: the camelCase `Required<CombatDefaults>` object (`movementResource: null,
  interpretation: "per_cell", enforcement: "none", turnControl: "owner_may_end", effectCleanup:
  true, effectLifecycle: { onCombatEnd: null, onTurnEnd: null, onAdvance: null }, rewindRestore:
  true, forwardRestore: false`).
- `newCombatEngine(sceneId)`: `{ scene_id, active: false, round: 0, turn: null, order: [],
  turn_control, movement: { resource: null, interpretation, enforcement }, effect_cleanup,
  rewind_restore, forward_restore, effect_lifecycle }` from `ENGINE_COMBAT_DEFAULTS`.

- [ ] **Step 1:** write the fixture + both tests (they pass on the current defaults — that is the
  point; sabotage one side's value, confirm both suites fail, restore).
- [ ] **Step 2:** implement `newCombatEngine`; `pnpm --filter @shadowcat/core test scene-docs`
  PASS; `cargo test -p shadowcat engine::combat` PASS.
- [ ] **Step 3:** `git commit -m "test(combat): engine combat defaults pinned by one fixture both suites read" -- <paths>`

### Task 6: `CombatController` (`@shadowcat/core`)

**Files:**
- Create: `src/client/core/src/combat.ts`, `src/client/core/src/combat.test.ts`
- Modify: `src/client/core/src/index.ts` (export `COMBAT_SERVICE`, `CombatController`, the
  types)

**Interfaces:** exactly spec §4.2 (`CombatControllerDeps` including `world: string`,
`CombatApi`, `NewCombatant`, `NewEvent`, `CombatAffordances`, `CombatClientError extends Error`
with a `code: "no-host" | "turn-owner" | "order-mismatch" | "not-found"` field).

- [ ] **Step 1 (failing tests):** over a `DocumentStore` seeded with a scene, three tokens (two
  linked actors, one instanced), a combat (active, `order` of three, `turn` = second) and
  combatants, plus one combat id in `order` that is NOT in the store (a hidden one):
  `combatsFor`/`activeFor` (active first), `combatants` (order preserved, missing id skipped,
  a parented combatant absent from `order` appended), `turnOf` (present; `null` when `turn`
  names the missing id), `resolvedFor` before/after `setResolved`, `subscribe` fires on
  `setResolved`; `createCombat` ops (`buildCombatDoc` + `newCombatEngine`, name);
  `addCombatants` — one intent, `create` per entry then one `update` on `/engine/order` with
  the exact `old`, owner from `effectiveOwner`, name fallback chain, `actorId`-only entry,
  `no-host` error; `addEvent`; `removeCombatant` — `order` update + `delete` with the store
  pre-image, `turn-owner` error on the current turn; `setHidden` both directions (paths, `remove:
  true` on the users entry); `reorder` (set mismatch error); `setInitiative`; every intent
  builds the right frame with a UUID `request_id` and propagates a rejection; `canAct` matrix
  (GM; owner under `owner_may_end` on own turn; owner under `gm_only`; non-owner; owner whose
  `canEdit(doc, "/engine")` is false — `roll`/`resource` false while `advance` stays true).
- [ ] **Step 2:** implement; `pnpm --filter @shadowcat/core test combat` PASS; `pnpm -r
  typecheck`; `pnpm docs:check-examples` for the new `@example`s.
- [ ] **Step 3:** `git commit -m "feat(core): CombatController — reads, clock intents and document helpers behind CombatApi" -- src/client/core/src/combat.ts src/client/core/src/combat.test.ts src/client/core/src/index.ts`

### Task 7: hooks — declaration, derivation, emitter, shell wiring

**Files:**
- Create: `src/client/core/src/combat-hooks.ts`, `src/client/core/src/combat-hooks.test.ts`
- Modify: `src/client/core/src/index.ts` (`defineCombatHooks`, `deriveCombatHookEvents`,
  `CombatHookEmitter`, `commandTouchesCombat`, `COMBAT_HOOK_VERSION`, `CombatHookEvent`)
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts` (hold `#hooks`/`#services`;
  `defineCombatHooks`; construct `#combat`/`#combatEmitter`; `services.provide(COMBAT_SERVICE, …)`;
  `get combat()`; `onCommand` pre-image capture + `emit`; `"combat"` subscription in `enter()`
  + teardown in `leave()`)
- Modify: `src/client/shell/src/lib/worldSession.test.ts`
- Modify: `src/client/ui-kit/src/appContext.ts` (`combat: CombatApi`), `src/client/ui-kit/src/
  __fixtures__/appContextTest.ts` (default `combat`), `src/client/shell/src/lib/Table.svelte`
  (`combat: session.combat`), `src/client/shell/src/lib/Table.test.ts`

**Interfaces:** spec §4.3 (the derivation table is normative — implement it as data where
possible: a list of `(predicate, emit)` steps over the per-combat `{ b, a }` pair) and §5.1–5.2.

- [ ] **Step 1 (failing tests, `combat-hooks.test.ts`):** a table of cases, each
  `{ before: WireDocument[], cmd: WireCommand, expect: CombatHookEvent[] }` applied through a
  real `DocumentStore` (seed `before`, snapshot pre-images, `applyCommand`, derive): initial
  start (round 0→1, turn null→A: `start{resumed:false}`, `round-start{1}`, `turn-start{A}`);
  resume (`start{resumed:true}` only); advance A→B same round; wrap B→A (`turn-end{B}`,
  `round-end{1}`, `round-start{2}`, `turn-start{A}`); event intermediate (A→C with event E's
  lifespan 2→1 in the command: `turn-end{A}`, `turn-start{E,event}`, `turn-end{E,event}`,
  `turn-start{C}`); event removal (E deleted: same, `kind` from the delete pre-image); hidden
  turn (`turn` → an id not in the store: `turn-start{kind:null}`); pause; end (delete of an
  active combat: `end{ended}` after the effect events); rewind across a round (`rewind` only);
  rewind within a round (`rewind` only); effect tick `null→2` and `3→2`, expiry `true→false`,
  nested item path; effect edit with no combat op ⇒ `[]`; a `CombatStart` swap (two combats:
  `end{paused}` for the old, `start` for the new, in op order); ordering within one command
  pinned by array equality. Emitter: two synchronous `emit` calls with an awaiting listener
  observe strict order; a throwing listener does not stall the queue.
- [ ] **Step 2:** implement `combat-hooks.ts`; `pnpm --filter @shadowcat/core test combat-hooks`
  PASS.
- [ ] **Step 3 (failing tests, shell):** `worldSession.test.ts` — `defineCombatHooks` ran (a
  module listener on `combat:start` fires on a start command delivered through the fake
  transport); no emission on `seedDocuments`; no emission on `applyIntent` (optimistic) nor on
  `reject`; `COMBAT_SERVICE` is `services.get`-able from a module's `ModuleContext`; the
  `"combat"` subscription is established after Welcome and `session.combat.resolved` reflects a
  delivered `scene_derived` frame; `leave()` resets to `EMPTY_COMBATS`. `Table.test.ts` —
  `AppContext.combat` is the session's controller.
- [ ] **Step 4:** implement the wiring; shell + ui-kit tests PASS; `pnpm -r typecheck` PASS;
  `pnpm lint` PASS.
- [ ] **Step 5:** `git commit -m "feat(core,shell): first CoreHooks entries — combat events derived from applied command deltas" -- <paths>`
- [ ] **Step 6:** BUDDY CHECK 2 over Tasks 4–7 (pre-generated diff of `src/client`).

### Task 8: scene-tools `Warn` overage label

**Files:**
- Modify: `src/modules/scene-tools/src/controller.svelte.ts` (`requestRoute` success branch per
  spec §5.3; `ROUTE_WARN_COLOR` beside `ROUTE_COLOR`)
- Modify: `src/modules/scene-tools/src/measure-tool.test.ts`
- Modify: `src/client/ui-kit/src/locales/en.ts` (`tools.overBudget`: `"{over} over budget"`,
  `tools.budgetStop`: `"stops at budget"`)

- [ ] **Step 1 (failing tests):** with a fake `ctx.combat.activeFor` returning a combat whose
  `movement.enforcement` is `"warn"` and a `pathfind` resolving `{ cost: 7, budget_cells: 5,
  truncated: false, arrested: false }` on a `perCell: 5, unit: "ft"` scene, `drawMeasure` gets
  `"35 ft · 10 ft over budget"` and `previewOverlay`'s stroke is `ROUTE_WARN_COLOR`; under
  `"hard"` with `truncated: true` the label is `"… · stops at budget"` and the stroke is
  `ROUTE_COLOR`; under `"none"`, or with `budget_cells: null`, or with no active combat, the
  label is the plain budget label; `arrested` keeps its `⚠` suffix in every case.
- [ ] **Step 2:** implement; `pnpm --filter @shadowcat/module-scene-tools test` PASS.
- [ ] **Step 3:** `git commit -m "feat(scene-tools): route preview shows the Warn overage and the Hard stop" -- src/modules/scene-tools/src/controller.svelte.ts src/modules/scene-tools/src/measure-tool.test.ts src/client/ui-kit/src/locales/en.ts`

### Task 9: core e2e — the seams end to end

**Files:**
- Create: `src/client/core/src/e2e/combat-seams.e2e.test.ts`
- Read first: `src/client/core/src/e2e/README.md`, `server-process.ts` (the seeded fixture:
  world, GM `ops`-class user, player `pl`/`pw`), `capabilities.e2e.test.ts` (the `nodeConnect`
  pattern), `src/server/src/bin/test_server.rs` (what the fixture seeds — add a `resource-registry`
  entry + a scene `grid.distance` to the fixture ONLY if the test cannot author them through
  ordinary GM intents; prefer intents).

- [ ] **Step 1:** write the scenario from spec §7 "Core e2e" (a)–(e) as one test with explicit
  waits (`for` + `sleep` polling on a predicate, the file's convention), asserting exact event
  lists via `deriveCombatHookEvents` fed from each client's own `DocumentStore`.
- [ ] **Step 2:** `pnpm build` (if `dist/` is stale) then `cargo build -p shadowcat --bin
  test_server`, then `pnpm --filter @shadowcat/core test:e2e` PASS (port contention: rerun alone
  if another server holds the port).
- [ ] **Step 3:** `git commit -m "test(e2e): combat seams — correlation, identical hook derivation, per-recipient channel, budget preview" -- src/client/core/src/e2e/combat-seams.e2e.test.ts`

### Task 10: docs, skills, gates, close-out

**Files:**
- Modify: `docs/site/protocol.md` (spec §8 bullet 1)
- Modify: `docs/PLAN.md` (M14c-6 → DONE pointer), `docs/HISTORY.md` (delivery entry: what
  shipped, the `WsClient.combat()` defect fixed in range, the decision-log summary),
  `docs/POST_WORK_FINDINGS.md` (anything found mid-run)
- Skills (plugin checkout `~/.claude/skills/shadowcat-codebase/skills/…`, commit + push in THAT
  repo with explicit paths; the checkout is shared — touch only the four files named):
  `shadowcat-codebase-combat/SKILL.md`, `shadowcat-codebase-client-shell/SKILL.md`,
  `shadowcat-codebase-realtime-sync/SKILL.md`, `shadowcat-codebase-scene-rendering/SKILL.md`
  per spec §8 bullet 2; dispatch `shadowcat-codebase:shadowcat-spec-reviewer` on the skill diff.

- [ ] **Step 1:** protocol page + skills + tracking docs.
- [ ] **Step 2:** FULL gate suite, each verdict recorded by name: `pnpm build`, `pnpm -r
  typecheck`, `pnpm -r test`, `pnpm lint`, `pnpm lint:docs`, `pnpm lint:props`,
  `pnpm lint:comments`, `pnpm lint:allowances`, `pnpm lint:file-size`, `pnpm lint:inline-tests`,
  `pnpm docs:check-examples`, `pnpm run test:scripts`, `pnpm --filter "shadowcat-example-*"
  build`; from `src/server/`: `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D
  warnings`, `cargo clippy --all-targets -- -D missing-docs -D clippy::missing-docs-in-private-items`,
  `cargo test --all`, `git diff --exit-code src/types/generated`; `pnpm --filter @shadowcat/core
  test:e2e`; `pnpm --filter @shadowcat/shell e2e` (unchanged specs must still pass — the
  `selfUserId` removal and the new frame touch the session path); local skill gates `node
  scripts/check-skill-symbol-refs-cli.mjs`, `node scripts/check-skill-api-refs-cli.mjs`.
- [ ] **Step 3:** `git commit -m "docs: M14c-6 closes — combat client seams" -- docs/`
- [ ] **Step 4:** FINAL two-reviewer branch review (`shadowcat-codebase:shadowcat-spec-reviewer`
  + `shadowcat-codebase:shadowcat-code-reviewer`, pre-generated `git diff main...HEAD`); fold
  findings; re-run the gates any fold-in touches. Report to the dispatcher; never push to
  `origin/main`.
