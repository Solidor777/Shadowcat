# M14d · Combat Tracker Module + Settings Editors — Design

**Status:** Approved by fork-resolution (design agent, 2026-09-02; every fork decided by "what is
the best long-term shape in keeping with our plans and goals?" and logged in §10). Last checkpoint
of M14 ([M14 design](2026-08-28-m14-combat-tracker-design.md) §7 and §12's M14d row), built on
the seams [M14c-6](2026-09-02-m14c-6-combat-client-seams-design.md) delivers. Where the M14 spec's
§7 disagrees with this spec, this spec wins.

## 0. Placement and what this ships

Two new pieces of first-party UI, both pure presentation over engine seams (M14 D17: "the module
layer holds only what would limit flexibility if fixed in the engine"):

1. **`@shadowcat/module-combat-tracker`** (`src/modules/combat-tracker`) — the default tracker
   panel: combats for the viewed scene, ordered rows, the clock controls, add/remove/hide/reorder,
   initiative rolls, resource editing, an event row kind, and a "your turn" notice. Any system or
   community module can replace it by contributing its own `shadowcat.panel` and ignoring this
   one; everything it does goes through `AppContext.combat`, `ctx.documents`, `ctx.chat`,
   `ctx.panels`, `ctx.hooks`.
2. **Combat settings editors inside `@shadowcat/module-game-settings`** — the world/scene
   combat-rules chain editor over `resolve_combat_rules`'s engine → system-defaults → world →
   scene precedence (all eight `CombatDefaults` leaves, with per-leaf provenance and reset), and
   the resource-registry editor.

Plus: Playwright coverage of both, a docs-site module page, the skills gate, and the M14
close-out (the milestone's `HISTORY.md` entry).

Depends on: M14c-6 merged (the seams), and the M16/M17/M18 branches — which merge before this
is built — only through stable seams: the M16 theme tokens are consumed as CSS custom properties
(never literal colors); the M18 `Condition.fx` is irrelevant to the tracker (it renders the
registry `icon` glyph exactly as `ConditionsPanel` does and never reads `fx`); M17's
`TokenVisionControl`/`TokenLightControl` live in the actors module and touch nothing here. M17
extends `GameSettingsPanel.svelte` by ~240 lines — this spec adds ONE `<CombatSettings />` line
to that file and keeps every combat editor in its own components (§4.4), so the merge conflict
surface is one line.

## 1. Scope

In: the tracker module (panel + components + module-local model helpers), the settings editors,
the `SettingPath` extension for the remaining `combat.*` leaves, i18n keys, the docs-site page,
two Playwright specs, skills.

Out: automation of attacks/damage (system-owned); audio/VFX cues (Phase 3); a statusbar turn
indicator (the panel badge + notice cover "your turn"; a second surface for the same fact is a
second thing to keep consistent — §10); anything server-side (M14c-6 finished the server work;
if the build finds a server gap it is fixed in range, not deferred).

## 2. Decisions

| # | Decision |
|---|---|
| T1 | **Package `@shadowcat/module-combat-tracker`, id `combat-tracker`, panel contribution `combat-tracker:panel`** — the repo's hyphenated descriptive naming (`asset-browser`, `scene-browser`, `game-settings`) over M14 D14's `src/modules/combat`. Not `gmOnly` (players end their turns and edit their own resources); launcher-closed by default like every panel except chat; `order: 2` (after chat 0 and asset-browser 1, alongside actors 2 — insertion order breaks the tie, and the launcher shows both). Icon `⚔️`, `labelKey: "combatTracker.tab"`. |
| T2 | **The settings editors live in `@shadowcat/module-game-settings`**, not in the tracker: they edit world config singletons (`world-settings`, `resource-registry`) and a scene document, which is exactly what that panel exists for, and the scene browser already deep-links into it. The tracker gets a "Settings…" affordance that opens `game-settings:panel` (the scene-browser's `ctx.panels.open` pattern). A replacement tracker keeps the editors for free. |
| T3 | **One tracker view per viewed scene** (`ctx.viewedSceneId`): the panel lists that scene's combats (active first), with the active one expanded by default and a picker when there are several. A GM roaming to another scene sees that scene's combats; players follow `activeScene`. No combat exists ⇒ a GM sees "Create combat"; a player sees an empty-state hint. |
| T4 | **Rows come from `CombatApi.combatants(combatId)`** — the server's `order`, already filtered to what this recipient may read; a hidden combatant is simply absent, and an unresolvable `turn` renders no active row (M14 §5). The tracker never inspects `permissions` to decide visibility. |
| T5 | **Resource numbers come only from `CombatApi.resolved`** (the `"combat"` channel): Mirror ⇒ read-only value; Tracked ⇒ `current` editable (`±` steppers + direct entry → `modifyResource`) with `max` shown; `error` ⇒ a warning glyph with the detail in a tooltip; `resources: null` ⇒ the cell is blank (this recipient may not see them). The tracker evaluates nothing. |
| T6 | **Reorder is a pointer-drag with a 44 px handle per row plus keyboard (Alt+↑/↓) on the focused row**, computing the new order through a pure `moveInOrder(order, from, to)` helper and dispatching ONE `reorder`. GM only (`canAct.edit`). |
| T7 | **Initiative rolls post to the channel registry's first channel** (`ChannelRegistryEngine.channels`, first key in map order — the M14c-4 GM pseudo-channel rule), with a per-panel "initiative notation" text input defaulting to `1d20` (a RAW template; references like `1d20 + init` resolve server-side per combatant). "Roll all" rolls every actor combatant the user may roll (GM: all; player: own) whose `initiative` is `null`; a per-row roll button rolls that one. Manual entry is an input on the row (GM/owner). |
| T8 | **Add combatants from the token selection** (`ctx.tokenSelection` on the viewed scene, one entry per selected token) and **add an event** through an inline form (name, lifespan or ∞, message, hidden). Both call the controller's single-intent helpers. Actor-only combatants (no token) are reachable by dragging nothing — out of the default UI (a system module can add them through the API); logged in §10. |
| T9 | **"Your turn" is a hook consumer**: the module registers a `combat:turn-start` listener (`ctx.hooks.on`) that, when the started combatant's `owner === ctx.selfId` and the user is not the GM, pushes `ctx.notify(t("combatTracker.yourTurn"), "info")` and drives the panel badge (`PanelBadge`: 1 while it is your turn, 0 otherwise). The first first-party hook consumer, exercising the seam the way a system module would. |
| T10 | **Settings editors write whole sub-objects, never null-descending leaves**: the world editor writes `/engine/combat` on `world-settings` and the scene editor writes `/engine/combat` on the scene document, each with the current object (or `null`) as the OCC pre-image and the merged object as `new` — a leaf reset removes the key from the object; an explicit "None" for `movementResource` sets the key to `null` (the doubly-optional clear). `resolveSettingProvenance` is extended to all eight leaves and stays the display mirror; the effective-rules summary under the editors shows what `CombatStart` would snapshot for the selected scene. |
| T11 | **The resource-registry editor is field-level over `/engine/resources/<key>`** (the `ConditionsPanel` pattern): key on create (immutable after), name, order, binding kind (switching kinds replaces the entry), formula inputs accepting a number or a formula string, validated client-side with `@shadowcat/formula`'s `parseFormula` for immediate feedback (the server's `Formula::validate` remains the authority and its rejection is surfaced through the intent's reject → `ctx.notify`). |
| T12 | **Mobile reflow through `sizeClass()`** (the ui-kit 48 rem axis, no media queries): compact rows stack art+name over initiative+resources, the header toolbar wraps to two rows, every interactive target is ≥ 44 px, the drag handle stays 44 px, the settings editors' fieldsets stack single-column. |
| T13 | **Theme tokens only**: every color, spacing and radius is a `--…` custom property from the shell's semantic token set (`--surface-raised`, `--border`, `--accent`, `--text-secondary`, `--space-1`, `--radius-1`, …); `Contribution.styling` stays the default (host-themed) so the M16 theme controller re-skins the tracker live. |
| T14 | **Refusals surface, never vanish**: every intent rejection (`CombatError` wording, a formula rejection, a `CombatClientError`) goes to `ctx.notify(message, "warning")`. |

## 3. The tracker module

### 3.1 Files

```
src/modules/combat-tracker/
  package.json  svelte.config.js  tsconfig.json  typedoc.json  vitest.config.ts  vitest.setup.ts
  src/index.ts                     module + contribution + the turn-start listener + badge
  src/CombatTrackerPanel.svelte    scene scoping, combat picker/create, composes the parts
  src/CombatHeader.svelte          round/turn label, clock controls, roll-all + notation, settings link
  src/CombatantRow.svelte          one row (actor or event)
  src/AddCombatants.svelte         add-selected-tokens + add-event form
  src/model.ts                     pure helpers: rowsFor, moveInOrder, rollTargets, firstChannel, formatResource
  src/turnBadge.ts                 PanelBadge implementation (count + subscribe)
  src/*.test.ts                    per component/model
```

Dependencies: `@shadowcat/core`, `@shadowcat/types`, `@shadowcat/ui-kit`; dev
`@testing-library/svelte`, `jsdom`, `sass` (the asset-browser `package.json` shape). No
`@shadowcat/module-*` import.

### 3.2 `index.ts`

```ts
export const combatTracker: Module = {
  manifest: { id: "combat-tracker", version: "0.1.0", dependencies: { "core-ui": "^0.1.0" },
              requires: [PANEL_CONTRACT], provides: [] },
  register(ctx) {
    const badge = new TurnBadge();
    ctx.contributions.contribute({
      id: "combat-tracker:panel", contract: PANEL_CONTRACT, order: 2,
      component: CombatTrackerPanel,
      panel: { icon: "⚔️", labelKey: "combatTracker.tab", badge },
    });
    ctx.hooks.on("combat:turn-start", (p) => badge.onTurnStart(p as CoreHooks["combat:turn-start"]), { requires: "^1.0.0" });
    ctx.hooks.on("combat:turn-end",   (p) => badge.onTurnEnd(p as CoreHooks["combat:turn-end"]));
    ctx.hooks.on("combat:end",        () => badge.clear());
  },
};
```

`register` receives a `ModuleContext`, which has `hooks` but no documents/notify — the badge
needs "is this combatant mine". Resolution: `TurnBadge` exposes `bind(resolve: (combatantId) =>
boolean, notify: () => void)`; `CombatTrackerPanel` binds it on mount from `AppContext`
(`ctx.documents.get(id)?.owner === ctx.selfId && ctx.role !== "gm"`, and `ctx.notify(...)`).
Until bound, the badge stays 0 and no notice fires (a keep-mounted panel binds once per session;
the panel's component is mounted by `PanelHost` even while launcher-closed — see the panels
skill's keep-mounted rule — so the notice works before the user ever opens the tracker).

### 3.3 Panel behaviour

**Scoping (T3).** `sceneId = ctx.viewedSceneId`; `combats = ctx.combat.combatsFor(sceneId)`
(reactive through the `createSubscriber(ctx.documents.subscribe)` bridge — the GameSettingsPanel
pattern; the `resolved` view through a second bridge over `ctx.combat.subscribe`). Selected
combat: the active one, else the first, else none; a `<select>` when more than one exists.
GM with none: a "Create combat" button (`ctx.combat.createCombat(sceneId)`); player: hint text.

**Header (`CombatHeader`).** Round label (`combatTracker.round {n}`, or "not started" when
`round === 0`), current-turn name; controls rendered by `ctx.combat.canAct(combatId)`:
`Start` (inactive) / `Pause` (active), `End` (with a confirm — it deletes the combat),
`Advance` (GM) or `End my turn` (owner on own turn), `Rewind` (GM, `round > 0`), `Sort` (GM),
`Roll all`, the notation input (T7), `Settings…` (GM; `ctx.panels.open("game-settings:panel")`),
and for a GM `Delete` on an inactive combat (`deleteCombat`). Each intent call is
`await`ed inside a try/catch → `ctx.notify` on rejection (T14); controls disable while a call is
in flight (a per-panel `busy` flag), since `combat()` resolves only after the store reflects the
event.

**Rows (`CombatantRow`).** For each `ctx.combat.combatants(combatId)` document, in order:
- art: the token's `engine.visual` face via `ctx.assets` when `kind.token_id` resolves, else the
  actor's, else a placeholder glyph; event rows show `📣`.
- name: `doc.name ?? t("combatTracker.unnamed")` (a redacted name is `null` on the wire);
  a click opens the sheet (`ctx.openDocument({ tokenId })` / `{ docId: actorId }`) when the
  document resolves.
- conditions: the token/actor's condition ids rendered as registry `icon` glyphs (the same
  `conditionTarget` + `condition-registry` read `ConditionsPanel` performs).
- initiative: a number input (GM/owner) or text; tiebreak shown as a subscript when non-zero.
- resources (T5): one cell per registry key in registry `order`.
- event rows: name + remaining lifespan (`∞` when `null`) + the message preview.
- current turn: `aria-current="true"` + accent border; the owner's rows carry a subtle marker.
- GM controls: hidden toggle (eye glyph, `setHidden`), remove (`removeCombatant`; disabled on
  the current turn with the tooltip `combatTracker.removeTurnHint` — T11 of the seams spec),
  drag handle (T6). Per-row roll button (T7) for actor rows the user may roll.

**Add (`AddCombatants`, T8).** "Add selected tokens (N)" — enabled when the viewed scene has
selected tokens not already in the combat (match by `kind.token_id`); `ctx.combat.addCombatants(
combatId, ids.map((tokenId) => ({ tokenId, hidden })))` with a "hidden" checkbox applying to the
batch. "Add event" — a disclosure form: name (required), lifespan (number or blank = ∞),
message (optional), hidden.

**Reorder (T6).** `pointerdown` on the handle starts a drag; `pointermove` computes the target
index from row bounding boxes; `pointerup` dispatches `reorder(moveInOrder(order, from, to))`
when `to !== from`; Escape cancels. Keyboard: Alt+ArrowUp/Down on a focused row moves it one
step. Only the ids the recipient can see are in `order`'s visible projection — the helper operates
on the FULL `engine.order` (hidden ids included, GM only, so a GM's `order` is complete; a
player never has `canAct.edit`).

### 3.4 Model helpers (`model.ts`, pure, unit-tested)

- `rowsFor(combat, documents, resolved) → Row[]` — joins each combatant doc with its
  `CombatantView` and its display name/art source.
- `moveInOrder(order: string[], from: number, to: number) → string[]` — same-set invariant.
- `rollTargets(combat, rows, role, selfId) → string[]` — the "Roll all" set (T7).
- `firstChannel(documents) → string | null` — the registry's first key.
- `formatResource(view: ResolvedResourceView) → string` — `"12 / 12"`, `"—"`, `"⚠"`.

### 3.5 i18n keys (`en.ts`, `combatTracker.*`)

`tab`, `title`, `create`, `noCombat`, `noCombatPlayer`, `pick`, `round`, `notStarted`, `turn`,
`noTurn`, `start`, `pause`, `end`, `endConfirm`, `advance`, `endMyTurn`, `rewind`, `sort`,
`rollAll`, `roll`, `notation`, `settings`, `delete`, `addSelected`, `addEvent`, `eventName`,
`eventLifespan`, `eventMessage`, `hidden`, `visible`, `remove`, `removeTurnHint`, `unnamed`,
`initiative`, `resourceError`, `dragHandle`, `moveUp`, `moveDown`, `yourTurn`, `busy`.

## 4. Settings editors (`@shadowcat/module-game-settings`)

### 4.1 `SettingPath` extension (core)

`SettingPath` gains `combat.effectCleanup`, `combat.rewindRestore`, `combat.forwardRestore`,
`combat.effectLifecycle.onCombatEnd`, `combat.effectLifecycle.onTurnEnd`,
`combat.effectLifecycle.onAdvance`; `resolveSettingProvenance` gains the six cases (the three
booleans through `resolvePick`; the three lifecycle leaves through `resolvePick` over
`layer.combat?.effectLifecycle?.<leaf>` with `ENGINE_COMBAT_DEFAULTS.effectLifecycle.<leaf>` as
the engine value — `null` at the engine tier means "engine fallback behaviour", exactly as
`resolve_combat_rules`'s `lifecycle_field` returns `None`). The existing mirror test between
`resolveSettingProvenance` and the server resolver (locate: `grep -rn "resolve_combat_rules\|resolveSettingProvenance" src/client/core/src/*.test.ts src/server/src/data/engine/combat/tests.rs`)
is extended to the new leaves; if no cross-language pin exists yet, add one on the M14c-6
fixture pattern (a JSON case list both suites read).

### 4.2 The chain editor (`CombatSettings.svelte`, world tier)

Rendered inside `GameSettingsPanel`'s GM world-defaults area as a `<fieldset>` "Combat rules",
one control per leaf, each followed by the panel's existing `provControl`-style provenance hint
and reset:

| Leaf | Control | Inherit / reset |
|---|---|---|
| `movementResource` | `<select>`: *Inherit*, *None*, then every registry key (name) | Inherit removes the key; None writes `null` (explicit clear) |
| `interpretation` | `<select>` per_cell / spaces / Inherit | remove |
| `enforcement` | `<select>` none / warn / hard / Inherit | remove |
| `turnControl` | `<select>` owner_may_end / gm_only / Inherit | remove |
| `effectCleanup`, `rewindRestore`, `forwardRestore` | `<select>` Inherit / On / Off (tri-state; a checkbox cannot express inherit) | remove |
| `effectLifecycle.onCombatEnd/onTurnEnd/onAdvance` | text input (number or formula), parsed client-side; blank = Inherit | remove the leaf; the `effectLifecycle` object is removed when its last leaf goes |

Writes (T10): `set(ws.id, "/engine/combat", wsys.combat ?? null, next)` where `next` is the
current object with the leaf set/removed; `next` becomes `null` when empty. The provenance hint
reads `prov("combat.<leaf>")` and shows the reset button when `source === "world"`.

**Effective rules summary**: below the fieldset, a read-only table of the eight resolved values
for the scene selected in the per-scene section (`resolveSettingProvenance(ctx.documents,
scene, path)` per leaf, with the source badge) — "what `CombatStart` will snapshot".

### 4.3 The scene tier (`CombatSceneOverrides.svelte`)

The same eight controls inside the existing per-scene `<fieldset>` (after grid/distance),
writing `/engine/combat` on the selected scene document (`ssys.combat ?? null` pre-image). The
inherit option here means "fall through to world", matching the vision/lighting overrides'
`null`-means-inherit convention in that section.

### 4.4 The registry editor (`ResourceRegistryEditor.svelte`)

A GM `<fieldset>` "Resources" in `GameSettingsPanel` (after the chat settings): a list of entries
(key, name, order, kind) with per-field inputs, and a kind-specific formula block:

- Mirror: `value`.
- Tracked: `max`, `recover.turnStart`, `recover.turnEnd`, `recover.roundStart`, `recover.roundEnd`.

Each input accepts a number (`"30"` ⇒ `30`) or a formula string; on change, `parseFormula(text)`
runs — an error shows inline (`gameSettings.resources.invalid {detail}`) and no write is sent;
a valid value dispatches ONE `update` on `/engine/resources/<key>/<field path>` with the raw
stored pre-image. Add: key input (validated `[a-z][a-z0-9_-]*`, unique) + kind ⇒ a whole-entry
`update` at `/engine/resources/<key>` (`old: null`) with `name = key`, `order = next`, and the
kind's defaults (`Mirror { value: 0 }` / `Tracked { max: 0, recover: all 0 }`). Kind switch:
ONE `update` at `/engine/resources/<key>/binding` replacing only the `ResourceBinding` with the
new kind's defaults, `old` = the raw stored binding — `name` and `order` are preserved. Remove: whole-map rewrite (the `ConditionsPanel.remove` shape). The
`movementResource` select in §4.2 lists these keys, so a GM sets up movement in two steps in one
panel.

`GameSettingsPanel.svelte` itself changes by three lines (three component tags) plus the
imports; every editor is its own file (the M17 merge-conflict bound from §0, and the file-size
gate — the panel is 725 lines today).

### 4.5 i18n keys (`en.ts`, `gameSettings.combat.*`, `gameSettings.resources.*`)

`combat.title`, `combat.movementResource`, `combat.none`, `combat.interpretation`,
`combat.enforcement`, `combat.turnControl`, `combat.effectCleanup`, `combat.rewindRestore`,
`combat.forwardRestore`, `combat.lifecycle`, `combat.onCombatEnd`, `combat.onTurnEnd`,
`combat.onAdvance`, `combat.on`, `combat.off`, `combat.effective`, `combat.scene.title`;
`resources.title`, `resources.key`, `resources.name`, `resources.order`, `resources.kind`,
`resources.mirror`, `resources.tracked`, `resources.value`, `resources.max`,
`resources.turnStart`, `resources.turnEnd`, `resources.roundStart`, `resources.roundEnd`,
`resources.add`, `resources.remove`, `resources.invalid`, `resources.keyTaken`,
`resources.keyShape`.

## 5. Docs-site page

`docs/site/modules/combat-tracker.md` (the `game-settings.md` shape: Purpose / Contributions
table / Components / Contracts & seams / Pointers) and a row in `docs/site/modules/index.md`;
`game-settings.md` gains the combat editors. The M15b retirement of `assets` in that index is
that branch's concern; if it has merged first, leave its row as it stands.

## 6. Testing

**Unit (Vitest, jsdom, `@testing-library/svelte`)** — per the repo's module conventions
(`setAppContextForTest` with a real `DocumentStore` + a fake `CombatApi` recording calls):
- `model.test.ts`: `rowsFor` (order, missing ids skipped, event rows, resolved join, `resources:
  null`), `moveInOrder` (same-set, bounds), `rollTargets` (GM all-null-initiative; player own
  only; events excluded), `firstChannel`, `formatResource`.
- `CombatTrackerPanel.test.ts`: scene scoping follows `viewedSceneId`; active-first selection;
  GM create button vs player hint; picker with two combats.
- `CombatHeader.test.ts`: control visibility per `canAct` matrix; `busy` disables; a rejected
  intent calls `notify` with the server message; `Roll all` builds the entry list with the
  notation and the first channel; settings link opens `game-settings:panel`.
- `CombatantRow.test.ts`: current-turn marker; hidden toggle both directions; remove disabled on
  the turn; initiative input dispatches `setInitiative`; resource stepper dispatches
  `modifyResource` with `delta`; direct entry dispatches `set`; Mirror read-only; `error` glyph;
  blank on `resources: null`; event row lifespan `∞`; sheet open on name click.
- `AddCombatants.test.ts`: selected-token diffing; batch hidden; event form validation.
- `reorder.test.ts`: drag → one `reorder` call with `moveInOrder`'s result; Escape cancels;
  Alt+arrow keyboard path.
- `turnBadge.test.ts` + `index.test.ts`: badge 1 on own `turn-start`, 0 on `turn-end`/`end`;
  `notify` fires once per own turn and never for the GM; contribution metadata.
- `CombatTrackerPanel.touch.test.ts`: compact `sizeClass` reflow (`GameSettingsPanel.touch.test.ts`
  precedent) — targets ≥ 44 px.
- game-settings: `combat-settings.test.ts` (each leaf's write shape incl. remove-vs-null for
  `movementResource`, the object-null collapse, provenance hint + reset), `combat-scene-overrides.test.ts`,
  `resource-registry.test.ts` (add/remove/kind switch/field write with pre-image/invalid formula
  blocked/key validation), `provenance.test.ts` extended for the six new paths.

**Playwright (`src/client/shell/e2e`)**, two browser contexts (GM + invited player, the
`hex-movement.spec.ts` seating flow), each spec self-contained:

1. `combat-tracker.spec.ts`
   - GM places two tokens (one assigned to the player via the actors panel's owner control),
     opens the tracker, adds both from the selection + an event (lifespan 1, message), rolls
     all with `1d20` → both rows show an initiative and two roll cards appear in chat; starts
     → round 1 and the first row is `aria-current` in BOTH browsers.
   - The player's browser shows the "your turn" notice when their row starts; `End my turn`
     advances; the GM's browser shows the next row; the event's turn posts its message to chat
     and the event row disappears (lifespan exhausted) — asserted on both browsers.
   - GM hides the NPC row → it vanishes from the player's tracker live; reveal → returns.
   - GM rewinds → the previous row is current again on both browsers; GM ends → the tracker
     shows the empty state and the combat is gone for the player.
   - Compact viewport (390×844) smoke: the panel opens, the header wraps, `End my turn` is
     clickable (≥ 44 px box) — one assertion block at the end reusing the player context.
2. `combat-settings.spec.ts`
   - GM opens game-settings: the resources editor adds `movement` (Tracked, `max: "speed"`,
     `turnStart: "speed"`) — the actor's `system.speed` is set through the actor sheet's
     `SystemTreeEditor`; the chain editor sets `movementResource = movement`,
     `interpretation = per_cell`, `enforcement = hard`; provenance hints read "World setting";
     the effective-rules table reflects them.
   - Scene tier: `enforcement = warn` on the scene → the summary reads "Scene override";
     reset → back to "World setting"; world reset → "Engine default".
   - Gate proof (the M14 §11 e2e items): with a combat started and `hard`, the player's route
     preview label carries the stop marker and a drag past the budget lands at the truncated
     cell (`data-last-move-outcome` = `"truncated"`); with `warn`, the preview label shows the
     overage text and the move executes in full (`"executed"`), and the tracker's row shows the
     decremented `current`.

## 7. Security & permissions

Unchanged from M14b/M14c: the tracker's gating is advisory (`canAct`, `canEdit`), the server
authorizes every intent and document write, hidden combatants never reach a non-GM (they are
absent from `combatants()` and from the channel), resource numbers reach only readers of
`/engine/resources`, and the `Warn`/`Hard` labels render server numbers. The registry editor and
the chain editors are GM-only by the `game-settings` panel's `gmOnly` (advisory) and by the
server's write authorization on config singletons (real).

## 8. Docs & skills

- Docs-site page + index row (§5); `docs/PLAN.md` M14 entry → DONE; `docs/HISTORY.md` M14d entry
  and the M14 milestone close; `docs/TODO.md`/`POST_WORK_FINDINGS.md` sweep.
- Skills (plugin checkout, reviewed gate): `shadowcat-codebase-combat` (the tracker + editors
  are the UI over the seams; the "no tracker/settings-editor UI exists" purpose line inverts;
  the parked `movementResource` provenance asymmetry gotcha gains "the editor renders it as
  Inherit/None"), `shadowcat-codebase-client-shell` (the module list gains `combat-tracker`; the
  game-settings description gains the combat editors), and the hook's `SUBSYSTEMS` map gains the
  `combat-tracker` glob under the combat skill.

## 9. Mobile reflow (T12)

- `CombatTrackerPanel`: `display: grid` rows; compact ⇒ `grid-template-columns: 1fr` with the
  art+name cell spanning and initiative/resources on the second line; the header's button
  group `flex-wrap: wrap` with `gap: var(--space-1)`; every `<button>`/`<input>` `min-height:
  44px` in compact (32 px expanded, the conditions panel's floor); the drag handle is a 44 px
  square in both.
- The settings fieldsets are single-column `display: grid` with `gap: var(--space-1)`; the
  effective-rules table becomes a definition list in compact.
- Verified by the touch test and the compact Playwright block.

## 10. Decision log

| Fork | Chosen | Alternatives and why not |
|---|---|---|
| Module name | `combat-tracker` (T1) | `combat` (M14 D14): the repo names first-party UI packages by what they show (`asset-browser`, `scene-browser`); `combat` reads as the engine subsystem. |
| Where the editors live | `game-settings` (T2) | A settings sub-panel inside the tracker (M14 §7): world config editing would then be split across two panels, the scene browser's deep-link would miss it, and a replacement tracker would lose the editors. |
| Panel gating | not `gmOnly` (T1) | A GM-only tracker would make players end turns through chat commands the engine does not have. |
| Rows source | `combatants()` (T4) | Reading `order` and joining documents in the module duplicates the controller's filtering. |
| Numbers | channel only (T5) | Any client evaluation reopens the fork M14c-6 closed. |
| Reorder mechanism | pointer drag + keyboard on `reorder` (T6) | Up/down buttons only: fails the M14 §7 touch-drag requirement; a drag library: a new dependency for one list. |
| Roll channel | registry's first channel (T7) | A channel picker per roll: more chrome for a decision the M14c-4 GM pseudo-channel already made. |
| Actor-only combatants | API only, no default UI (T8) | An actor picker in the add form: the actors panel already exists for browsing; the default tracker adds what is on the table. Logged, not deferred — the API supports it and a system module can surface it. |
| Your-turn signal | hook consumer + panel badge + notice (T9) | Deriving it from documents in the panel: works, but leaves `CoreHooks` with zero first-party consumers; a statusbar contribution: a second surface for one fact. |
| Settings write shape | whole `/engine/combat` object (T10) | Per-leaf pointers: `combat` is `null` until first written and a leaf pointer cannot descend through `null` (the same trap the hex-movement e2e documents for `vision`); two write shapes for one object would be a forked decision. |
| Registry write shape | per-field pointers (T11) | `resources` is always an object (server-seeded), so leaf pointers are safe and cheaper on OCC than whole-map rewrites. |
| Formula feedback | client `parseFormula` + server authority (T11) | Server-only: a bad formula would round-trip to a generic "rejected"; the M14c-1 corpus guarantees the two parsers agree. |
| Remove-on-turn | disabled with a hint | Letting the server reject: a generic wording for a predictable rule. |
| Statusbar indicator | none | See T9. |

## 11. Open questions for the user

None.
