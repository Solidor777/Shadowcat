---
name: shadowcat-codebase-panels
description: "Use when touching the Shadowcat panel-manager: @shadowcat/module-panels (layout tree + LayoutOp reducer, EngineAdapter/DockviewEngine/FakeEngine, PanelsController, PanelHost/PanelMenu/DockChips/CompactSwitcher), the shell's PanelsBridge (AppContext.panels), the shadowcat.panel contract panels contribute into, per-world panelLayout persistence, pop-out (same-heap window) panels, or the stage-well veto rules (STAGE_ID — enforced inside panels; stage's own source is scene-rendering territory). Covers src/modules/panels/** + ui-kit panelsBridge. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Panel Manager (dockable panels)

Orientation for the dockable-panel system (dock/float/minimize/compact + pop-out
windows) that replaced the tabbed sidebar.

## Purpose

In-game UI panels (chat, assets, actors, factions, conditions, game-settings, settings) are
dockable/floatable/minimizable windows managed by `@shadowcat/module-panels`. Layout truth is a
**pure tree** (`PanelLayoutV1`) mutated only through a **reducer** (`applyOp`); the docking
engine (dockview-core) is an interchangeable presentation adapter behind `EngineAdapter` —
every engine gesture is intercepted, classified into a `LayoutOp`, and re-dispatched through
the reducer (intercept-and-redispatch), so the engine never owns state.

## Key files & seams

- `panels/layout`'s `applyOp` (the pure layout-tree reducer) — `PanelLayoutV1` (expanded zones right/bottom/left +
  floating + minimized + `poppedOut: string[]`, compact view state), `LayoutOp` (incl.
  `popOut`/`popIn`, `resizeFloating` — an already-floating panel's in-place rect update, mirroring
  `resizeZone`/`resizeGroup` rather than `float`'s detach-and-reinsert), `applyOp` reducer,
  `defaultLayout`, `locate`, `prune`,
  `placeNewRegistrations`, `placeFromPersistedLocation`. **Same-reference no-op contract**: an
  op that changes nothing returns the SAME layout object (callers/tests rely on `toBe`).
  `SHEET_CASCADE_BASE`/`SHEET_CASCADE_STEP` (the late-registration rehydration cascade) must stay
  numerically identical to `PanelsController`'s `REHYDRATE_FLOAT_BASE`/`REHYDRATE_FLOAT_STEP`
  (below) — two separate constants by design (no cross-module import, to avoid coupling the two
  call sites), but the SAME logical operation (persisted popped-out id → floating on reload) must
  land at the same position regardless of which registration-timing path runs. **Nothing in either
  module's types enforces that pairing — the anti-drift gate is `controller.test`'s "cascade
  parity at index %i" test**, which drives both call sites to the same floating index and demands
  the identical rect at n=0 (pins BASE), n=1 (pins STEP) and n=7 (pins the shared `% 6` wrap). Each
  side's OTHER cascade tests assert only that one side's own offsets differ from each other, which
  stays green when either pair drifts; only the parity test fails on divergence.
- `panels/layout`'s `encodeLayout`/`decodeLayout` (structural
  validation, unknown-id prune, reset-to-default on garbage). `decodeLayout` also returns the
  pre-prune `source` layout so late registrations restore their persisted spot (below).
  `poppedOut` back-compat: absent on a blob predating pop-out support normalizes to `[]` (`withPoppedOut`);
  present-but-malformed fails the whole blob.
- `EngineAdapter` seam (incl. optional
  `onNotice?(cb):()=>void`); `FakeEngine` = test double / bespoke-fallback (degrades pop-out to a
  floating window — production pop-out is dockview-only); `DockviewEngine` = production engine,
  the only module (plus its test, `dockview.test`) allowed to import dockview-core
  (`dockview-core@7.0.2` EXACT pin; boundary enforced by the ESLint `no-restricted-imports` rule
  in `eslint.config.js` — .ts files only; .svelte files are unlinted, where the boundary holds
  by the EngineAdapter seam's design).
- `panels/engine`'s `classifyDrop`/`opForMenuCommand` →
  `ClassifyResult` (op or veto); `MenuCommand` includes `"popOut"`; `STAGE_ID` vetoes live here
  AND as early-returns in `DockviewEngine` (two independent layers, both apply to pop-out too).
- `PanelsController` ($state layout owner):
  bridges engine gestures + imperative `PanelsApi` onto `applyOp`, persists via
  `getPanelLayout`/`setPanelLayout` (per-world `ui_state.worlds[w].panelLayout`), filters regs
  via `regsForRole` (gmOnly = client-advisory only, NOT security), `syncRegistrations` places
  late-registering panels from the retained persisted `source` (never resets a saved layout),
  `onOp` hook drives `PanelHost`'s a11y live-region, `onReset` fires the layout-reset toast key.
  `#rehydratePoppedOut()` (construction-time): converts persisted `poppedOut` ids to floating
  via `REHYDRATE_FLOAT_BASE`/`REHYDRATE_FLOAT_STEP` cascade (never re-opens a real popup — no user
  gesture at load), persists the change, and queues a `panels.popoutRestoredFloating` notice via
  `#pendingNotice`/`flushPendingNotice()` — the notice is deferred past first mount (fired from
  a `PanelHost` post-mount `$effect`), NOT called synchronously in the constructor: an
  `aria-live="polite"` region never announces content present at its own initial render.
- `PanelHost` — owns DOM/engine adapter + compact(<48rem)/
  expanded switch; builds the controller lazily at mount from AppContext and `bind()`s it into
  the shell's `PanelsBridge`. **Keep-mounted**: panels hide via CSS/slot adoption
  (`appendChild`), never `{#if}` — pop-out re-parents the same mounted instance into a second
  same-heap `Window`, it does not remount. `CompactSwitcher` adopts the stage into
  `.compact-stage`. `PanelMenu` = per-tab command menu (dock/float/minimize/pop-out, a11y).
- `PanelsBridge` (`AppContext.panels`):
  stable shell-owned handle; `#impl` is `$state` so pre-bind readers (chips) unfreeze when the
  host binds; pre-bind calls warn once. Implements `PanelsApi` + `PanelsChipsView`
  (`minimized`/`metaMap`/`restore`) — `DockChipsContribution` (statusbar `panel-dock` region)
  reads the same bridge reactively, no second controller.
- `panels: Module` (module wiring) — provides multi `PANEL_CONTRACT`
  (`shadowcat.panel`), contributes `PanelHost` into core-ui's singleton
  `shadowcat.surface:panel-host` with a fresh `new DockviewEngine(...)` per world session
  (`register` runs per install), and chips into statusbar's `shadowcat.surface:panel-dock`.
- `src/modules/stage/` — the canvas stage module; the stage center well is INVIOLABLE:
  never dockable-over, never floatable, never minimizable — `STAGE_ID` vetoed in both the drop
  and menu paths.
- Panel modules declare `Contribution.panel` metadata (`icon`, `labelKey`, `gmOnly?`,
  `defaultPlacement`); defaults: chat docked right, every other panel launcher-closed (absent
  from the layout tree, not a minimized chip) until opened from the topbar `LauncherMenu`
  ([[shadowcat-codebase-client-shell]]) — toggling the same launcher item again minimizes it
  back to a statusbar chip.
- `src/client/shell/public/popout.html` — same-origin loader document served at
  `/popout.html` by `static_handler` (exact-match lookup, real 404-on-miss — NOT a
  SPA catch-all; verified against `static_handler`). A popped-out panel's `Window`
  navigates here; `dockview-core`'s own `addStyles` clones stylesheets into the cross-document
  popup. `[[embed-dist-compile-ordering]]`: the client build must run before the server embeds
  `dist/` (`dist/popout.html` presence is part of the build-verification step).

## Hard invariants

- **All layout mutations flow through `applyOp`** — no direct engine-state writes; engine
  events are preventDefault-ed and re-dispatched as classified ops in BOTH the drop and menu
  wires.
- **Stage well is inviolable** — `STAGE_ID` ops are vetoed at policy AND handler layers; the
  stage never becomes a dockview panel.
- **dockview imports confined to `DockviewEngine` (+ its `dockview.test` module)** — everything
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
  [[async-completion-needs-object-identity-not-key]]; see `DockviewEngine.#floatTransitionIds`.
- **Pop-out is gesture-time imperative, never routed through `apply()`'s declarative
  reconcile.** The only producer of a `popOut` tree op is the menu-command handler
  `DockviewEngine.#requestPopOut`, which calls `window.open`-backed `addPopoutGroup` FIRST
  and emits the op only after that promise resolves `true`. No code path can need pop-out
  reconciled through the reducer's `apply()` diff and silently miss it, because `apply()` never
  originates a `popOut`/`popIn` op — it only consumes one already emitted imperatively. A
  browser popup cannot be opened outside a user gesture; this is why rehydration-on-load
  degrades persisted `poppedOut` ids to floating instead of re-opening a real window.
- **`#pendingPopouts` in-flight guard is required because dockview-core's `mutation()` wrapper
  does not span `addPopoutGroup`'s async gap** — its `finally` fires the instant the async
  function RETURNS the pending promise, not when it settles, and `getNextGroupId()` is fresh on
  every call. A second "Pop out" click on the same panel before the first resolves would
  otherwise fire two independent `window.open()` calls and corrupt `#poppedOutGroupPanels`. Set
  the guard before calling the driver; clear it in both `.then()`/`.catch()` and in `destroy()`.
- **A popped-out panel's origin group must be seeded into `apply()`'s `seenGroupIds`, via
  `#poppedOutOriginGroups`, or it is orphan-removed on the very next `apply()`.** dockview-core
  keeps that origin group alive-but-hidden (`setVisible(false)`) internally — its own
  window-close path expects to hand the panel back to that exact group object — but the
  reducer's tree no longer names it once `detach()` strips the now-empty group. Capture
  `panel.group.id` SYNCHRONOUSLY before the driver call (capturing after resolves to the wrong
  group). This and the in-flight guard above were both found via direct trace through the
  vendored `dockview-core@7.0.2` CJS source (`DockviewComponent`, `PopoutWindowService`), not
  from the wrapper code alone — re-verify against that source on any dockview-core version bump.
- **`#handleRemovePopoutGroup` (real window-close → `popIn`) has three branches that must all be
  covered by a test that actually fires `onDidRemovePopoutGroup`, not a synthetic op**: the
  `#applying`-suppression branch (a `popIn` must NOT fire when OUR OWN `apply()` reconcile is
  what caused the popout group's removal, e.g. a menu "dock" on a popped-out panel), the
  `event.group.model.panels` fallback when the panel isn't in `#poppedOutGroupPanels`, and the
  `STAGE_ID` skip. All three read correct on inspection but are exactly the class of bug found
  twice in this file under adversarial testing — do not trust inspection alone for changes here.
- **`#applying` is a synchronous-only guard — it cannot suppress an `AsapEvent` listener** (F3,
  live floating re-drag/resize sync). `DockviewApi.onDidLayoutChange` is dockview's `AsapEvent`:
  `.fire()` schedules delivery via `queueMicrotask`, so every listener runs on the
  NEXT microtask, after `apply()`'s synchronous `finally { this.#applying = false }` has already
  reset the flag. A handler bound to this event that checks `#applying` gets a permanent `false`
  regardless of cause — worse than no guard, since it reads as protected. `#handleFloatingLayoutChange`
  instead diffs the freshly-read `boundingBox` against `#lastFloatingRect`, a per-id cache
  `apply()`'s floating loop snapshots to the TREE's own rect on every reconcile (whether or not
  that iteration touched dockview); a `resizeFloating` op's own round trip re-snapshots the
  identical rect, so the diff reads unchanged and nothing re-fires, with no dependency on
  `apply()`'s synchronous window. Also why re-position sync can't reuse the per-group
  `onDidDimensionsChange` pattern used for docked zones — that event, owned by dockview-core's
  `PanelApiImpl`, only ever carries width/height, so a pure drag with no size change never fires
  it at all; only `onDidLayoutChange` (fed by `Overlay`'s `onDidChangeEnd`) covers both
  gestures. Found by tracing the vendored `dockview-core@7.0.2` source directly, not the wrapper
  code alone — re-verify on any dockview-core version bump, same as the pop-out invariants above.

## Gotchas

- `register()` runs once per world entry (fresh ModuleRegistry per WorldSession) — one
  DockviewEngine per session, not app-wide. **FakeEngine reaches NO production path**: it is the
  default only when a caller mounts `PanelHost` WITHOUT an `engine` prop
  (`PanelHost`'s `engine ?? new FakeEngine()`), and the shipped module's `register()` always
  supplies a `DockviewEngine` — so that branch is taken today only by the test suite. It remains a
  real fallback SEAM any bespoke host could use; it is not dead code, and it is not production code.
- **`EngineAdapter.focus` has no production caller at all.** Both engines implement it and nothing
  invokes either: `PanelsController.focus(id)` is `this.open(id)` (a `LayoutOp`, so the tab activates
  and floating z bumps in the TREE), and `PanelHost` calls `init`/`apply`/`onOp`/`onNotice`/`destroy`
  but never `eng.focus`. The reachable chain is `sheetsController` → `PanelsBridge` →
  `PanelsController.focus` and stops there. Consequence to know before wiring it up:
  `DockviewEngine.focus` early-returns on `STAGE_ID` (stage-well defense-in-depth) and
  `FakeEngine.focus` has NO such guard — a never-fork-a-decision violation that is inert only
  because the seam is uncalled. Logged in TODO.
- Panel modules `requires` `PANEL_CONTRACT`, which topologically activates `panels` FIRST —
  late panel registration is the ROUTINE order, not an edge case.
- dockview's `onDidRemovePanel` fires synchronously inside `removePanel` — transition guards
  must be armed before the call.
- RESOLVED: FakeEngine's zone width-containment defect (a zone `<div>` with no width of its own
  stretched to `host`'s full cross-size, `align-items: stretch`, once enough docked content made
  the always-present stretch visually register) — `init()` now nests a `row` flex container
  (left/center/right) with `bottom` full-width below it, and `apply()` applies `ZoneNode.size` as
  each zone's actual px width/height on every reconcile, with `min-width: 0`/`overflow: auto` on
  the zone and `width: 100%; min-width: 0` on each group `<div>` so oversized content scrolls
  within the zone instead of escaping it — see CLOSED_BUGS.
- jsdom cannot simulate a real pointer-drag gesture — `dockview.test` unit-tests
  `DockviewEngine` directly under jsdom (init/apply/DOM adoption) with duck-typed
  `DockviewWillDropEvent`s standing in for drops. NO e2e test exercises a real dockview tab
  drag either (`panels.spec` covers launcher-open→dock→reload-survival, re-toggle→
  minimize-to-chip, and the compact/expanded 48rem axis — launcher-closed defaults mean
  there is no chip on a fresh world until a panel is minimized); real-pointer drop-position
  classification fidelity is a manual-QA gap, logged in POST_WORK_FINDINGS.
- On any dockview-core version bump, re-verify `--z-popover` (1000, defined in `_semantic.scss`)
  still clears dockview's floating-overlay z-index (`--dv-overlay-z-index`, 999 at 7.0.2) — the
  popover menus stack above floating panel groups only by that numeric margin.
- Dragging a panel INTO an already-open popout group bypasses the reducer — `#groupWillDropSubs`
  is not wired for popout groups, so that specific gesture does not flow through `applyOp` (this
  gesture's scope was menu-only per spec §9/Decision 6; the drop-classification gap is logged in
  TODO, not a defect in shipped scope).
- `DockviewEngine.#expandGroupDockOp`'s "new group" index computation assumes the dragged whole
  group is not already a member of the target zone — a same-zone whole-group reorder is a
  KNOWN, code-documented index-computation gap (an inline `TODO` beside that method),
  distinct from the real-pointer-drop-fidelity manual-QA gap above: that one is about missing
  e2e coverage for an otherwise-correct path, this one is a specific case the logic itself
  doesn't handle correctly yet.
- Popped-out windows never survive a page reload — `#rehydratePoppedOut` always converts
  persisted `poppedOut` ids back to floating at construction; there is no "reopen the popup on
  load" affordance (a real browser cannot open a window without a fresh user gesture).
- `jsdom` cannot exercise the real `window.open`/`addStyles`/`onDidRemovePopoutGroup` DOM path —
  unit tests drive the translation logic via the injected `popoutDriver` and fire dockview's
  real event emitters directly (e.g. `api.component._onDidRemovePopoutGroup.fire(...)`,
  verified against the vendored source) rather than simulating a real popup; the actual
  cross-window re-parent + stylesheet clone is dockview's own (spike-verified) machinery plus a
  manual-QA item, same class as the existing real-pointer-drag gap below.
- jsdom never runs real layout, so `boundingBox` (backed by `getBoundingClientRect()`) always
  reads all-zero unless a test stubs it — `dockview.test`'s F3 tests assign a replacement
  `getBoundingClientRect` directly onto the floating panel's `group.element`, then fire
  `(api as any).component._bufferOnDidLayoutChange.fire()` and `await Promise.resolve()` twice
  (the `AsapEvent` microtask hop) rather than simulating a real resize-handle/title-bar drag.

## Pointers

- Design: `docs/superpowers/specs/` (panel-manager + pop-out windows); implementation plans
  under `docs/superpowers/plans/`.
- Relationships: `graphify query "panels controller layout tree engine adapter dockview bridge chips popout"`.
- Shell/AppContext side: [[shadowcat-codebase-client-shell]].
