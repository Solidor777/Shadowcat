---
name: shadowcat-codebase-panels
description: "Use when touching the Shadowcat panel-manager (M12a): @shadowcat/module-panels (layout tree + LayoutOp reducer, EngineAdapter/DockviewEngine/FakeEngine, PanelsController, PanelHost/PanelMenu/DockChips/CompactSwitcher), the shell's PanelsBridge (AppContext.panels), the shadowcat.panel contract panels contribute into, per-world panelLayout persistence, or the stage-well veto rules (STAGE_ID — enforced inside panels; stage's own source is scene-rendering territory). Covers src/modules/panels/** + ui-kit panelsBridge. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Panel Manager (dockable panels)

Orientation for the M12a dockable-panel system that replaced the tabbed sidebar.

## Purpose

In-game UI panels (chat, assets, actors, factions, conditions, game-settings, settings) are
dockable/floatable/minimizable windows managed by `@shadowcat/module-panels`. Layout truth is a
**pure tree** (`PanelLayoutV1`) mutated only through a **reducer** (`applyOp`); the docking
engine (dockview-core) is an interchangeable presentation adapter behind `EngineAdapter` —
every engine gesture is intercepted, classified into a `LayoutOp`, and re-dispatched through
the reducer (intercept-and-redispatch), so the engine never owns state.

## Key files & seams

- `src/modules/panels/src/layout/tree.ts` — `PanelLayoutV1` (expanded zones right/bottom/left +
  floating + minimized, compact view state), `LayoutOp`, `applyOp` reducer, `defaultLayout`,
  `locate`, `prune`, `placeNewRegistrations`. **Same-reference no-op contract**: an op that
  changes nothing returns the SAME layout object (callers/tests rely on `toBe`).
- `src/modules/panels/src/layout/persist.ts` — `encodeLayout`/`decodeLayout` (structural
  validation, unknown-id prune, reset-to-default on garbage). `decodeLayout` also returns the
  pre-prune `source` layout so late registrations restore their persisted spot (below).
- `src/modules/panels/src/engine/adapter.ts` — `EngineAdapter` seam; `fake.ts` = test double /
  bespoke-fallback; `dockview.ts` = production engine, the only file (plus its test,
  `dockview.test.ts`) allowed to import
  dockview-core (`dockview-core@7.0.2` EXACT pin; boundary enforced by the ESLint
  `no-restricted-imports` rule in `eslint.config.js` — .ts files only; .svelte files are
  unlinted, where the boundary holds by the EngineAdapter seam's design).
- `src/modules/panels/src/engine/policy.ts` — `classifyDrop`/`opForMenuCommand` →
  `ClassifyResult` (op or veto); `STAGE_ID` vetoes live here AND as early-returns in the
  dockview wire (two independent layers).
- `src/modules/panels/src/controller.svelte.ts` — `PanelsController` ($state layout owner):
  bridges engine gestures + imperative `PanelsApi` onto `applyOp`, persists via
  `getPanelLayout`/`setPanelLayout` (per-world `ui_state.worlds[w].panelLayout`), filters regs
  via `regsForRole` (gmOnly = client-advisory only, NOT security), `syncRegistrations` places
  late-registering panels from the retained persisted `source` (never resets a saved layout),
  `onOp` hook drives PanelHost's a11y live-region, `onReset` fires the layout-reset toast key.
- `src/modules/panels/src/PanelHost.svelte` — owns DOM/engine adapter + compact(<48rem)/
  expanded switch; builds the controller lazily at mount from AppContext and `bind()`s it into
  the shell's `PanelsBridge`. **Keep-mounted**: panels hide via CSS/slot adoption
  (`appendChild`), never `{#if}`. `CompactSwitcher.svelte` adopts the stage into
  `.compact-stage`. `PanelMenu.svelte` = per-tab command menu (dock/float/minimize, a11y).
- `src/client/ui-kit/src/panelsBridge.svelte.ts` — `PanelsBridge` (`AppContext.panels`):
  stable shell-owned handle; `#impl` is `$state` so pre-bind readers (chips) unfreeze when the
  host binds; pre-bind calls warn once. Implements `PanelsApi` + `PanelsChipsView`
  (`minimized`/`metaMap`/`restore`) — `DockChipsContribution` (statusbar `panel-dock` region)
  reads the same bridge reactively, no second controller.
- `src/modules/panels/src/index.ts` — module wiring: provides multi `PANEL_CONTRACT`
  (`shadowcat.panel`), contributes `PanelHost` into core-ui's singleton
  `shadowcat.surface:panel-host` with a fresh `new DockviewEngine(...)` per world session
  (register runs per install), and chips into statusbar's `shadowcat.surface:panel-dock`.
- `src/modules/stage/` — the canvas stage module; the stage center well is INVIOLABLE (W1–W3):
  never dockable-over, never floatable, never minimizable — `STAGE_ID` vetoed in both the drop
  and menu paths.
- Panel modules declare `Contribution.panel` metadata (`icon`, `labelKey`, `gmOnly?`,
  `defaultPlacement`); defaults: chat docked right, every other panel launcher-closed (absent
  from the layout tree, not a minimized chip) until opened from the topbar `LauncherMenu`
  ([[shadowcat-codebase-client-shell]]) — toggling the same launcher item again minimizes it
  back to a statusbar chip.

## Hard invariants

- **All layout mutations flow through `applyOp`** — no direct engine-state writes; engine
  events are preventDefault-ed and re-dispatched as classified ops in BOTH the drop and menu
  wires.
- **Stage well is inviolable** — `STAGE_ID` ops are vetoed at policy AND handler layers; the
  stage never becomes a dockview panel.
- **dockview imports confined to `engine/dockview.ts` (+ `dockview.test.ts`)** — everything
  else sees `EngineAdapter`.
- **Same-reference no-op**: `applyOp`/`prune` return the input object unchanged when nothing
  changes (persistence debounce + tests depend on it).
- **Keep-mounted panels**: hide via CSS/adoption, never `{#if}` — panel state and seed
  `$effects` must survive dock/float/minimize/compact transitions.
- **Late registrations must not reset a saved layout** — placement resolves against the
  retained pre-prune `source`, not `defaultPlacement` [[contribution-seed-reactive-before-resync]]-adjacent boot-order hazard.
- **`gmOnly` is client-advisory** — server remains sole authority over panel data.
- **Async engine callbacks need object identity, not just id-key guards** — a panel recreated
  mid-flight (float transition) invalidates key-equality staleness checks
  [[async-completion-needs-object-identity-not-key]]; see `#floatTransitionIds` in dockview.ts.

## Gotchas

- `register()` runs once per world entry (fresh ModuleRegistry per WorldSession) — one
  DockviewEngine per session, not app-wide; FakeEngine only reaches production via the
  explicit no-`engine`-prop bespoke-fallback seam.
- Panel modules `requires` `PANEL_CONTRACT`, which topologically activates `panels` FIRST —
  late panel registration is the ROUTINE order, not an edge case.
- dockview's `onDidRemovePanel` fires synchronously inside `removePanel` — transition guards
  must be armed before the call.
- FakeEngine has a known width-containment defect (3rd docked group full-width) absent under
  the production engine — see `docs/OPEN_BUGS.md` before "fixing" it in DockviewEngine terms.
- jsdom cannot simulate a real pointer-drag gesture — `dockview.test.ts` unit-tests
  `DockviewEngine` directly under jsdom (init/apply/DOM adoption) with duck-typed
  `DockviewWillDropEvent`s standing in for drops. NO e2e test exercises a real dockview tab
  drag either (`panels.spec.ts` covers chip-click dock + reload-persistence only); real-pointer
  drop-position classification fidelity is a manual-QA gap, logged in
  `docs/POST_WORK_FINDINGS.md` (M12a verification gap).

## Pointers

- Design: `docs/design/` M12 spec (approved f97dd62); plan
  `docs/superpowers/plans/2026-07-13-m12a-panel-manager-core.md`.
- Relationships: `graphify query "panels controller layout tree engine adapter dockview bridge chips"`.
- Shell/AppContext side: [[shadowcat-codebase-client-shell]].
