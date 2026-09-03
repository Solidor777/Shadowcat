# M14d Combat Tracker Module + Settings Editors — Implementation Plan

> **For agentic workers:** Execute task-by-task in order; each task's steps use checkbox
> (`- [ ]`) syntax. Written for a sonnet-class implementer with NO conversation context. The
> plan is in four parts (A core + scaffold, B tracker, C settings editors, D e2e + close-out);
> the parts are sequential, and each part ends at a buddy-check.

**Goal:** Ship the default combat tracker panel (`@shadowcat/module-combat-tracker`) and the
combat settings editors (world/scene rules chain + resource registry, inside
`@shadowcat/module-game-settings`), covered end to end by Playwright, and close milestone M14.

**Architecture:** Pure presentation over the M14c-6 seams — `AppContext.combat` (`CombatApi`),
`ctx.documents`, `ctx.combat.resolved` (the `"combat"` channel), `ctx.hooks` (the `combat:*`
hooks), `ctx.chat`/`ctx.panels`/`ctx.notify`. No server change is planned; if the build finds a
server gap, fix it in range (the campaign's iron rule) and record it in the report.

**Tech Stack:** Svelte 5 (runes), TypeScript, SCSS with the shell's semantic tokens, Vitest +
`@testing-library/svelte` (jsdom), Playwright.

**Spec:** `docs/superpowers/specs/2026-09-02-m14d-tracker-module-settings-editors-design.md`
(decisions T1–T14; read it first, in full). Seam contract:
`docs/superpowers/specs/2026-09-02-m14c-6-combat-client-seams-design.md` §4.2–4.3, §5.

**Prerequisites (verify, do not assume):** M14c-6 merged to `main` (`git log main --oneline |
grep -i "m14c-6\|combat client seams"`), and the M16/M17/M18 branches merged (`git branch
--merged main`). Work in a fresh worktree on branch `m14d-tracker-settings` from `main`.
`pnpm install --frozen-lockfile`, `pnpm build`.

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
to a named document; state which in the prompt. Opus is banned for every dispatch. Reviewers get
no Bash — pre-generate diffs.

## Buddy-check directives

1. After Part A (Task 3) — `SettingPath` extension + module scaffold + model helpers.
2. After Part B (Task 8) — the tracker components.
3. After Part C (Task 11) — the settings editors.
4. Final: two-reviewer branch review before merge.

## Global constraints

- No lint suppressions; no file-size allowlist entries (every component is its own file; keep
  each under ~600 lines — split further if a component grows past that).
- Comments cite symbols, never files/lines; no milestone ids, spec pointers, dates or history
  narration in code comments, assert messages or test names.
- No `@shadowcat/module-*` import inside either module; only `@shadowcat/core`,
  `@shadowcat/types`, `@shadowcat/ui-kit`, `@shadowcat/formula`.
- Every reactive read of `ctx.documents` goes through a `createSubscriber` bridge (the
  `GameSettingsPanel`/`ConditionsPanel` pattern); every `ctx.combat.resolved` read through a
  second bridge over `ctx.combat.subscribe`. Every `old` in an `update` is the RAW stored value.
- Colors/spacing/radii only through `--…` tokens; no media queries — `sizeClass()` decides
  compact layout.
- All strings through `ctx.t` with keys in `src/client/ui-kit/src/locales/en.ts` (the only
  shipped catalog).
- Deletions via `trash`; commits `git add <paths>` + `git commit -- <paths>`; every message ends
  with the campaign trailer:
  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Do3ud57JJ1MpU52KpSJq1Z
  ```
- Per-task gate: the touched packages' `test` + `typecheck`, `pnpm lint`, `pnpm lint:comments`.
  Full suite in Task 14.

---

## Part A — core extension, scaffold, model

### Task 1: `SettingPath` gains the remaining six `combat.*` leaves

**Files:**
- Modify: `src/client/core/src/scene-docs.ts` (`SettingPath` union; six `case`s in
  `resolveSettingProvenance`)
- Modify: `src/client/core/src/scene-docs.test.ts`
- Modify (find first): the existing client/server provenance mirror test — run
  `grep -rn "resolveSettingProvenance" src/client/core/src/*.test.ts` and
  `grep -rn "resolve_combat_rules" src/server/src/data/engine/combat/tests.rs`. If a shared JSON
  case list exists, extend it; if not, create
  `src/client/core/src/__fixtures__/combat-provenance-cases.json` (cases:
  `{ system?, world?, scene?, path, expect: { value, source } }`) read by BOTH a Vitest test
  (through `resolveSettingProvenance` over a fixture store) and a Rust test (through
  `resolve_combat_rules` — source is not observable server-side, so the Rust side asserts the
  VALUE only; the Vitest side asserts both).

**Interfaces:**
- New paths: `combat.effectCleanup`, `combat.rewindRestore`, `combat.forwardRestore`,
  `combat.effectLifecycle.onCombatEnd`, `combat.effectLifecycle.onTurnEnd`,
  `combat.effectLifecycle.onAdvance`.
- Booleans through `resolvePick(scene?.combat?.x, world?.combat?.x, system?.combat?.x,
  ENGINE_COMBAT_DEFAULTS.x)`; lifecycle leaves through `resolvePick(scene?.combat?.effectLifecycle?.l,
  …, ENGINE_COMBAT_DEFAULTS.effectLifecycle.l)` — an engine `null` is a legitimate value
  (`source: "engine"`, meaning the server's built-in lifecycle behaviour).

- [ ] **Step 1 (failing tests):** each new path at each tier (engine/system/world/scene), a
  scene `null` lifecycle leaf falling through, the `default: return path satisfies never` still
  compiling.
- [ ] **Step 2:** implement; `pnpm --filter @shadowcat/core test scene-docs` PASS; the mirror
  cases PASS on both sides (`cargo test -p shadowcat engine::combat`).
- [ ] **Step 3:** `git commit -m "feat(core): resolveSettingProvenance covers every CombatDefaults leaf" -- <paths>`

### Task 2: module scaffold + registration + turn badge

**Files:**
- Create: `src/modules/combat-tracker/package.json` (name `@shadowcat/module-combat-tracker`;
  copy the asset-browser `package.json` shape), `svelte.config.js`, `tsconfig.json`,
  `typedoc.json`, `vitest.config.ts`, `vitest.setup.ts` (copy from `src/modules/conditions/`)
- Create: `src/modules/combat-tracker/src/index.ts`, `src/turnBadge.ts`,
  `src/CombatTrackerPanel.svelte` (placeholder `<section>` rendering `t("combatTracker.title")`
  — filled in Part B), `src/index.test.ts`, `src/turnBadge.test.ts`
- Modify: `src/client/shell/package.json` (`"@shadowcat/module-combat-tracker": "workspace:*"`
  beside `module-conditions`), `src/client/shell/src/App.svelte` (import `combatTracker` and
  add it to the default module array after `conditions`), and
  `src/client/shell/src/lib/defaultModuleOrder.test.ts` if it pins the list. `pnpm-workspace.yaml`
  (`src/modules/*`) and the root `typedoc.json` (`src/modules/*` entry points) already glob the
  new package — nothing to edit there; `importMap.test.ts` may enumerate `@shadowcat/*` runtime
  entries (`RUNTIME_ENTRIES`) — check and extend if it does.
- Modify: `src/client/ui-kit/src/locales/en.ts` (`combatTracker.tab`, `combatTracker.title`)

**Interfaces:**
- `index.ts` exactly spec §3.2; `TurnBadge implements PanelBadge` with `get()`, `subscribe(cb)`,
  `bind(isMine: (combatantId: string) => boolean, notify: () => void)`, `onTurnStart(p)`,
  `onTurnEnd(p)`, `clear()`. `onTurnStart`: if bound and `isMine(p.combatantId)` ⇒ count 1 +
  `notify()`; else count 0. `onTurnEnd`: count 0 when it names the current one. Listeners fire
  on every count change only.

- [ ] **Step 1 (failing tests):** `index.test.ts` — contribution id/contract/order/panel meta,
  `badge` present, the three hook listeners registered (a `ModuleContext` stub records `on`
  calls); `turnBadge.test.ts` — unbound stays 0 and never notifies; bound + mine ⇒ 1 + notify
  once; not mine ⇒ 0; `turn-end` ⇒ 0; `clear` ⇒ 0; subscribe fires only on change.
- [ ] **Step 2:** implement; `pnpm install` (new workspace package — the lockfile changes; commit
  `pnpm-lock.yaml`); `pnpm --filter @shadowcat/module-combat-tracker test` + `typecheck` PASS;
  `pnpm build` PASS (the shell imports the module); `pnpm -r typecheck` PASS.
- [ ] **Step 3:** `git commit -m "feat(combat-tracker): module scaffold, panel contribution, turn badge" -- <paths> pnpm-lock.yaml`

### Task 3: model helpers

**Files:**
- Create: `src/modules/combat-tracker/src/model.ts`, `src/model.test.ts`

**Interfaces:** spec §3.4 — `Row { doc: WireDocument; kind: "actor" | "event"; view:
CombatantView | null; name: string | null; art: { tokenId?: string; actorId?: string } }`,
`rowsFor(combat: WireDocument, combatants: WireDocument[], resolved: CombatsView): Row[]`,
`moveInOrder(order: string[], from: number, to: number): string[]`,
`rollTargets(rows: Row[], role: WorldRole, selfId: string): string[]`,
`firstChannel(documents: ReadableDocuments): string | null`,
`formatResource(view: ResolvedResourceView | undefined): string`.

- [ ] **Step 1 (failing tests):** `rowsFor` joins views by id, keeps input order, tags events;
  `moveInOrder` produces a permutation (same multiset), no-ops on equal indices, throws on
  out-of-range; `rollTargets` — GM: every actor row with `initiative === null`; player: own
  rows only; events never; `firstChannel` — first key of the registry map, `null` without one;
  `formatResource` — `"12 / 12"`, `"7 / 30"`, `"—"` for undefined, `"⚠"` for `error`.
- [ ] **Step 2:** implement; tests PASS.
- [ ] **Step 3:** `git commit -m "feat(combat-tracker): pure row/order/roll helpers" -- src/modules/combat-tracker/src/model.ts src/modules/combat-tracker/src/model.test.ts`
- [ ] **Step 4:** BUDDY CHECK 1 (Tasks 1–3 diff).

## Part B — the tracker

### Task 4: `CombatTrackerPanel.svelte` — scoping, picker, create, composition

**Files:**
- Modify: `src/modules/combat-tracker/src/CombatTrackerPanel.svelte`
- Create: `src/CombatTrackerPanel.test.ts`
- Modify: `en.ts` (`combatTracker.create`, `noCombat`, `noCombatPlayer`, `pick`, `busy`)

**Behaviour:** spec §3.3 "Scoping". Binds the `TurnBadge` on mount (get it through the
contribution's `panel.badge` — pass the badge instance as a `props` entry on the contribution
in `index.ts`, `props: { badge }`, so the panel receives it as a prop; `bind` with
`(id) => ctx.documents.get(id)?.owner === ctx.selfId && ctx.role !== "gm"` and
`() => ctx.notify(ctx.t("combatTracker.yourTurn"), "info")`). Holds the per-panel `busy` flag
and a `run(fn)` helper (`try { busy = true; await fn(); } catch (e) { ctx.notify(String(e.message), "warning"); } finally { busy = false; }`)
passed to the header and rows.

- [ ] **Step 1 (failing tests):** with `setAppContextForTest` over a store holding two scenes,
  a combat on each, and a fake `CombatApi` (a small class implementing every method with
  recorded calls + configurable `canAct`): the panel lists the viewed scene's combats only;
  active-first default selection; the picker appears with two combats on one scene and
  switching it changes the rendered rows; GM sees "Create combat" when none (click ⇒
  `createCombat(sceneId)`), player sees the hint; the badge is bound on mount (a
  `combat:turn-start` for an own combatant increments it and calls `notify`); a rejected intent
  surfaces through `notify` and `busy` resets.
- [ ] **Step 2:** implement; tests PASS; `typecheck`, `lint`.
- [ ] **Step 3:** `git commit -m "feat(combat-tracker): panel scoping, combat picker and creation" -- <paths>`

### Task 5: `CombatHeader.svelte`

**Files:**
- Create: `src/CombatHeader.svelte`, `src/CombatHeader.test.ts`
- Modify: `en.ts` (`round`, `notStarted`, `turn`, `noTurn`, `start`, `pause`, `end`,
  `endConfirm`, `advance`, `endMyTurn`, `rewind`, `sort`, `rollAll`, `notation`, `settings`,
  `delete`)

**Props:** `combat: WireDocument`, `rows: Row[]`, `busy: boolean`, `run: (fn) => Promise<void>`,
`notation: string` (bindable). Behaviour per spec §3.3 "Header". `End` uses a two-click confirm
(first click arms `confirming`, second within 5 s runs; a `window.confirm` is not testable and
not touch-friendly). `Roll all`: `ctx.combat.roll(combat.id, firstChannel(ctx.documents),
rollTargets(...).map((combatant_id) => ({ combatant_id, notation })))`; no channel ⇒ notify.

- [ ] **Step 1 (failing tests):** control visibility for GM / owner-on-turn under
  `owner_may_end` / owner under `gm_only` / non-owner (drive `canAct` results); `busy` disables
  every control; each control calls the matching `CombatApi` method with the combat id; `End`
  needs two clicks; `Roll all` builds the entries and channel; `Settings…` calls
  `panels.open("game-settings:panel")`; `Delete` only on an inactive combat.
- [ ] **Step 2:** implement; tests PASS.
- [ ] **Step 3:** `git commit -m "feat(combat-tracker): header — clock controls and initiative rolls" -- <paths>`

### Task 6: `CombatantRow.svelte`

**Files:**
- Create: `src/CombatantRow.svelte`, `src/CombatantRow.test.ts`
- Modify: `en.ts` (`hidden`, `visible`, `remove`, `removeTurnHint`, `unnamed`, `initiative`,
  `resourceError`, `roll`, `dragHandle`, `moveUp`, `moveDown`)

**Props:** `row: Row`, `combat: WireDocument`, `registry: [string, Resource][]` (registry
entries in `order`), `isTurn: boolean`, `can: CombatAffordances`, `busy`, `run`, `notation`,
`onDragStart(index, ev)`, `index`. Behaviour per spec §3.3 "Rows". Art: resolve the token's
face asset id the way `TokenView`/the actors panel does — locate the helper
(`grep -rn "faceAssetId\|visual.face\|RenderVisual" src/modules/actors/src/*.ts src/client/core/src/actor.ts`)
and reuse it; never hand-parse `RenderVisual`. Conditions: reuse `conditionTarget` +
`condition-registry` glyphs (the `ConditionsPanel` read). Resource cell: Tracked ⇒ `−`/`+`
buttons (`modifyResource(..., { kind: "delta", amount: ∓1 })`) around an editable number
(`{ kind: "set", value }` on change); Mirror ⇒ text; `error` ⇒ `⚠` with `title` = detail;
`view === null` or `resources === null` ⇒ blank cell.

- [ ] **Step 1 (failing tests):** current-turn marker (`aria-current`); name fallback; sheet
  open on name click with `{ tokenId }` / `{ docId }`; conditions glyphs; initiative input ⇒
  `setInitiative`; per-row roll ⇒ `roll` with one entry; steppers and direct entry ⇒
  `modifyResource` shapes; Mirror read-only; error glyph; blank when not visible; event row
  (lifespan `∞` / number, message preview, no resource cells, no roll); hidden toggle both
  directions ⇒ `setHidden`; remove ⇒ `removeCombatant`, disabled + hint on the turn; GM-only
  controls absent for a player; the drag handle is rendered only when `can.edit`.
- [ ] **Step 2:** implement; tests PASS.
- [ ] **Step 3:** `git commit -m "feat(combat-tracker): combatant and event rows" -- <paths>`

### Task 7: `AddCombatants.svelte` + reorder wiring

**Files:**
- Create: `src/AddCombatants.svelte`, `src/AddCombatants.test.ts`, `src/reorder.ts`
  (pointer-drag state machine over row elements: `beginDrag(index, ev)`, `move(ev)`,
  `end(ev) → { from, to } | null`, `cancel()`), `src/reorder.test.ts`
- Modify: `src/CombatTrackerPanel.svelte` (mount the rows list with the drag machinery and
  Alt+↑/↓ keyboard handling; dispatch `ctx.combat.reorder(combat.id, moveInOrder(order, from,
  to))`)
- Modify: `en.ts` (`addSelected`, `addEvent`, `eventName`, `eventLifespan`, `eventMessage`)

**Behaviour:** spec §3.3 "Add" and "Reorder" (T6, T8). `reorder.ts` is DOM-light: it takes the
row elements' `getBoundingClientRect` through an injected `rects(): DOMRect[]` so jsdom tests
can stub geometry.

- [ ] **Step 1 (failing tests):** `AddCombatants` — button label carries the count of selected
  tokens NOT already in the combat; click ⇒ `addCombatants(combatId, [{ tokenId, hidden }, …])`;
  the event form requires a name, parses lifespan blank ⇒ `null`, submits ⇒ `addEvent`;
  `reorder.test.ts` — drag from index 0 past the second row's midpoint ends at `{ from: 0, to:
  1 }`; Escape ⇒ `null`; panel test — a completed drag dispatches ONE `reorder` with
  `moveInOrder`'s result; Alt+ArrowDown on a focused row dispatches the one-step move; a player
  (no `edit`) gets neither.
- [ ] **Step 2:** implement; tests PASS.
- [ ] **Step 3:** `git commit -m "feat(combat-tracker): add from selection, add event, drag/keyboard reorder" -- <paths>`

### Task 8: compact reflow + styles + touch test

**Files:**
- Modify: the four `.svelte` files' `<style lang="scss">` (spec §9: grid rows, wrapped header,
  44 px targets in compact, tokens only)
- Create: `src/CombatTrackerPanel.touch.test.ts` (copy the `GameSettingsPanel.touch.test.ts`
  approach to force `sizeClass()` compact)

- [ ] **Step 1 (failing test):** in compact, every `button`/`input` inside the panel reports
  `min-height` ≥ 44 px (read computed style in jsdom via the class the compact branch applies —
  assert the class, and the SCSS declares the size; jsdom does not lay out), the header has the
  `compact` class, rows have the stacked class.
- [ ] **Step 2:** implement; tests PASS; `pnpm lint` (svelte a11y warnings are errors here —
  every icon button has an `aria-label`, the drag handle is a `button` with
  `aria-label={t("combatTracker.dragHandle")}`).
- [ ] **Step 3:** `git commit -m "feat(combat-tracker): compact reflow and themed styles" -- <paths>`
- [ ] **Step 4:** BUDDY CHECK 2 (Tasks 4–8 diff).

## Part C — settings editors

### Task 9: `CombatSettings.svelte` (world tier) + effective-rules summary

**Files:**
- Create: `src/modules/game-settings/src/CombatSettings.svelte`, `src/combat-settings.test.ts`
- Modify: `src/modules/game-settings/src/GameSettingsPanel.svelte` — ONE line
  `<CombatSettings {ws} {wsys} {set} {prov} scene={scene} />` inside the GM world-defaults block
  (after the animation controls) plus the import; pass the panel's existing `set` and `prov`
  helpers as props (do not duplicate them)
- Modify: `en.ts` (spec §4.5 `gameSettings.combat.*`)

**Behaviour:** spec §4.2 exactly. Write helper `writeCombat(next: CombatDefaults | null)` ⇒
`set(ws.id, "/engine/combat", wsys.combat ?? null, next)`; `leafSet(key, value)`,
`leafRemove(key)` (deleting the key; collapse to `null` when the object becomes empty;
`effectLifecycle` collapses the same way). `movementResource`: the select's `"__inherit"`
option removes the key, `"__none"` writes `null`, a key writes the string.

- [ ] **Step 1 (failing tests):** each select/input dispatches the whole-object write with the
  raw pre-image; inherit removes the key (and the object collapses to `null` when it was the
  last); `movementResource` None writes `null` and Inherit removes; lifecycle text `"1"` writes
  the number `1`, the invalid `"1 +"` shows the inline `parseFormula` error and writes nothing,
  and the valid `"max(hp, 0)"` writes the string (the formula grammar has no comparison
  operators — a lifecycle flag is "non-zero ⇒ act"); the provenance hint per leaf reads
  `prov`'s source and the reset button
  appears only for `world`; the effective-rules table shows eight rows with source badges for
  the selected scene; the registry keys populate the movement select.
- [ ] **Step 2:** implement; `pnpm --filter @shadowcat/module-game-settings test` PASS; the
  existing `GameSettingsPanel` tests still PASS (the one-line insertion).
- [ ] **Step 3:** `git commit -m "feat(game-settings): combat rules chain editor with provenance and effective summary" -- <paths>`

### Task 10: `CombatSceneOverrides.svelte` (scene tier)

**Files:**
- Create: `src/modules/game-settings/src/CombatSceneOverrides.svelte`, `src/combat-scene-overrides.test.ts`
- Modify: `GameSettingsPanel.svelte` — ONE line inside the per-scene `<fieldset>` (after the
  distance controls): `<CombatSceneOverrides {scene} {ssys} {setScene} />` + import
- Modify: `en.ts` (`gameSettings.combat.scene.title`)

- [ ] **Step 1 (failing tests):** same matrix as Task 9 against `/engine/combat` on the scene
  doc with `ssys.combat ?? null` as the pre-image; inherit removes; the summary in Task 9
  updates when the scene override changes (integration through the shared store).
- [ ] **Step 2:** implement; tests PASS.
- [ ] **Step 3:** `git commit -m "feat(game-settings): per-scene combat overrides" -- <paths>`

### Task 11: `ResourceRegistryEditor.svelte`

**Files:**
- Create: `src/modules/game-settings/src/ResourceRegistryEditor.svelte`,
  `src/resource-registry.test.ts`
- Modify: `GameSettingsPanel.svelte` — ONE line after the chat settings block:
  `<ResourceRegistryEditor />` + import (it reads the registry itself through its own bridge,
  like `ConditionsPanel`)
- Modify: `en.ts` (spec §4.5 `gameSettings.resources.*`)

**Behaviour:** spec §4.4. Value coercion helper `coerceFormula(text): number | string |
FormulaError` — a finite number literal ⇒ number, else `parseFormula(text)` ok ⇒ the trimmed
string, else the error. Paths: `/engine/resources/<key>/name`, `/order`,
`/binding/value`, `/binding/max`, `/binding/recover/turn_start` … (the wire spelling is
snake_case under `binding` — read `src/types/generated/engine/Recovery.ts` and
`ResourceBinding.ts` and use their exact keys).

- [ ] **Step 1 (failing tests):** rows render from a fixture registry in `order`; name/order
  edits dispatch field writes with pre-images; a Tracked `max` edit of `"speed"` writes the
  string, `"30"` writes the number, `"1 +"` shows the inline error and writes nothing; kind
  switch dispatches ONE `update` at `/engine/resources/<key>/binding` (raw stored binding as
  `old`) carrying the new kind's defaults, and leaves `name`/`order` untouched; add validates the key shape
  and uniqueness and writes the whole entry at `old: null`; remove rewrites the map without the
  key; non-GM renders nothing (the panel is `gmOnly`, but the component guards too).
- [ ] **Step 2:** implement; tests PASS; `pnpm lint:file-size` (GameSettingsPanel stays under
  the soft limit — it only gained three lines).
- [ ] **Step 3:** `git commit -m "feat(game-settings): resource registry editor" -- <paths>`
- [ ] **Step 4:** BUDDY CHECK 3 (Tasks 9–11 diff).

## Part D — e2e, docs, close-out

### Task 12: Playwright — `combat-tracker.spec.ts`

**Files:**
- Create: `src/client/shell/e2e/combat-tracker.spec.ts`
- Read first: `src/client/shell/e2e/hex-movement.spec.ts` (seating a player, two contexts,
  `data-render-ready`, the actors panel owner assignment, token placement), `panels.spec.ts`
  (launcher open + persist waits), `assets.spec.ts` (upload).
- Add `data-testid`s to the tracker where the spec needs stable hooks
  (`combat-tracker:create`, `combat-tracker:row-<combatantId>`, `combat-tracker:advance`,
  `combat-tracker:end-my-turn`, `combat-tracker:roll-all`, `combat-tracker:add-selected`,
  `combat-tracker:add-event`, `combat-tracker:hide-<id>`, `combat-tracker:rewind`,
  `combat-tracker:end`) — add them in the components in this task (a testid is not a feature).

- [ ] **Step 1:** write the scenario in spec §6 "Playwright" item 1 as one `test` (the suite's
  convention: one long two-context test with a `try/finally` closing the player context),
  asserting on BOTH pages at each step with generous timeouts (`15_000`), and the compact block
  at the end via `player.setViewportSize({ width: 390, height: 844 })`.
- [ ] **Step 2:** `pnpm --filter @shadowcat/shell e2e -- combat-tracker` PASS locally (port
  31999 must be free — rerun alone if another session's server is up).
- [ ] **Step 3:** `git commit -m "test(e2e): combat tracker — roll, start, player turn, event, hide/reveal, rewind, end, compact" -- <paths>`

### Task 13: Playwright — `combat-settings.spec.ts`

**Files:**
- Create: `src/client/shell/e2e/combat-settings.spec.ts`
- Add testids in the editors (`gameSettings:combat-<leaf>`, `gameSettings:resources-add`,
  `gameSettings:resources-<key>-<field>`, `gameSettings:combat-effective-<leaf>`,
  `gameSettings:combat-scene-<leaf>`) and `provenance:combat.<leaf>` hints (the panel's
  existing `provenance:<path>` convention).

- [ ] **Step 1:** write spec §6 item 2. For the actor's `system.speed`: open the actor sheet
  (actors panel → open) and use `SystemTreeEditor`'s add-field control (find its testids in
  `SystemTreeEditor.svelte`), or if that is impractical through the UI, set it via the sheet's
  number input after adding — never via a raw API call (the suite's convention is UI-only).
  For the movement gate proof, reuse `hex-movement.spec.ts`'s drag helpers on a square grid
  with `grid.distance` set through `gameSettings.scene.distancePerCell`.
- [ ] **Step 2:** `pnpm --filter @shadowcat/shell e2e -- combat-settings` PASS locally.
- [ ] **Step 3:** `git commit -m "test(e2e): combat settings — registry, chain provenance, scene override, Hard truncation and Warn overage" -- <paths>`

### Task 14: docs, skills, full gates, milestone close

**Files:**
- Create: `docs/site/modules/combat-tracker.md`; modify `docs/site/modules/index.md`,
  `docs/site/modules/game-settings.md`
- Modify: `docs/PLAN.md` (M14 entry → DONE, pointer to `HISTORY.md`), `docs/HISTORY.md` (M14d
  entry + the M14 milestone close line), `docs/TODO.md` / `docs/POST_WORK_FINDINGS.md` sweep
- Skills (plugin checkout, explicit paths, commit + push in that repo):
  `shadowcat-codebase-combat/SKILL.md`, `shadowcat-codebase-client-shell/SKILL.md`,
  `hooks/codebase-skill-reminder.py` (`SUBSYSTEMS`: `src/modules/combat-tracker` → combat) +
  its self-test; dispatch `shadowcat-codebase:shadowcat-spec-reviewer` on the skill diff.

- [ ] **Step 1:** docs + skills.
- [ ] **Step 2:** FULL gate suite, each verdict recorded by name: `pnpm build`, `pnpm -r
  typecheck`, `pnpm -r test`, `pnpm lint`, `pnpm lint:docs`, `pnpm lint:props`,
  `pnpm lint:comments`, `pnpm lint:allowances`, `pnpm lint:file-size`, `pnpm lint:inline-tests`,
  `pnpm docs:check-examples`, `pnpm run test:scripts`, `pnpm --filter "shadowcat-example-*"
  build`; from `src/server/`: `cargo fmt --all -- --check`, both clippy invocations,
  `cargo test --all`, `git diff --exit-code src/types/generated`; `pnpm --filter @shadowcat/core
  test:e2e`; `pnpm --filter @shadowcat/shell e2e` (the whole suite); `node
  scripts/check-skill-symbol-refs-cli.mjs`, `node scripts/check-skill-api-refs-cli.mjs`;
  `pnpm build:all` (the docs site builds with the new page and its link check passes).
- [ ] **Step 3:** `git commit -m "docs: M14d closes — combat tracker module + settings editors; M14 complete" -- docs/`
- [ ] **Step 4:** FINAL two-reviewer branch review (`shadowcat-codebase:shadowcat-spec-reviewer`
  + `shadowcat-codebase:shadowcat-code-reviewer`, pre-generated `git diff main...HEAD`); fold
  findings; re-run affected gates. Report to the dispatcher; never push to `origin/main`.
