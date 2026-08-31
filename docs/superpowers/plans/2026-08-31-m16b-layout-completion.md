# M16b — Layout Completion — Implementation Plan

> **For agentic workers:** implement task-by-task. Checkbox (`- [ ]`) steps. Commit per green task.

**Goal:** Complete the panel engine's drag-resize story (hit targets, keyboard move/resize,
tree→widget reconcile for already-floating panels) and persist the multi-window arrangement
(pop-out grouping + screen geometry) with a one-gesture "Reopen windows" restore.

**Architecture:** All layout mutations keep flowing through `applyOp`. Keyboard gestures and
pop-out geometry capture both emit `LayoutOp`s; `DockviewEngine.apply()` gains the
already-floating reconcile branch so non-pointer rect sources reach the widget. The tree's
`ExpandedLayout.poppedOut: string[]` becomes `popouts: PopoutWindowLayout[]` with a codec
migration. Notifications gain an action slot to carry the restore gesture.

**Tech Stack:** Svelte 5 runes, SCSS, vitest+jsdom, Playwright, dockview-core@7.0.2 (vendored —
read its source before every coupling).

**Spec:** `docs/superpowers/specs/2026-08-31-m16-layout-theming-completion-design.md` §E–§F
(binding).

## Model/Effort directives

Kimi session; coder subagents per phase. Every subagent prompt carries the campaign owner's
verbatim directive paragraph + the report-delivery requirement.

## Global Constraints

- Comments cite SYMBOLS, never files/lines; no ephemeral refs in code/comments/test names
  (RULES 15/16; `pnpm lint:comments`).
- Doc coverage gates are errors: every new export documented (`lint:docs`/`lint:props`).
- **Same-reference no-op contract**: `applyOp` returns the input object when nothing changes.
- **Never fork a decision**: one reducer, one codec; engine gestures classified into ops.
- All layout mutations through `applyOp`; stage well (`STAGE_ID`) vetoes stay two-layered.
- The vendored dockview-core@7.0.2 couplings carry the re-verify-on-bump note pattern.
- Safe deletion only (`trash`). Gates before every commit: `pnpm -r test && pnpm -r typecheck
  && pnpm lint`; phase end adds `lint:docs`, `lint:props`, `lint:comments`.
- Worktree `C:/Dev/Shadowcat-m16`, branch `m16-layout-theming`.

---

## Phase 1 — Tree + codec + reducer ops

### Task 1: `PopoutWindowLayout` tree shape + codec migration

**Files:**
- Modify: `src/modules/panels/src/layout/tree.ts` — `ExpandedLayout.poppedOut: string[]` becomes
  `popouts: PopoutWindowLayout[]` where `PopoutWindowLayout = { key: string; panels: string[];
  rect: { left: number; top: number; width: number; height: number } | null }` (doc every field).
- Modify: `src/modules/panels/src/layout/persist.ts` — `encodeLayout`/`decodeLayout`:
  legacy blobs carrying `poppedOut: string[]` migrate to one single-panel window per id
  (`rect: null`, key minted deterministically from the panel id, e.g. `"legacy-<id>"` — decode
  must stay pure/deterministic); a present-but-malformed `popouts` fails the whole blob like
  today; absent both → `[]`. Rect validation: finite numbers, width/height > 0.
- Update every `poppedOut` reader (`locate`, `prune`, `placeFromPersistedLocation`,
  `placeNewRegistrations`, `defaultLayout`, and any tests) to the new shape.
- Tests: `layout/persist.test.ts`, `layout/tree.test.ts`.

- [x] Failing tests first: legacy `poppedOut` blob migrates; malformed `popouts` entry fails the
  blob; rect validation rejects garbage; round-trip preserves windows.
- [x] Implement; update all readers. Gates + commit.

### Task 2: Reducer ops — popOut payload, updatePopoutGeometry, popOutInto

**Files:**
- Modify: `src/modules/panels/src/layout/tree.ts` — `LayoutOp.popOut` gains `{ key: string;
  rect: ScreenRect | null }`; new `LayoutOp.updatePopoutGeometry { key, rect }` (no-op when the
  stored rect deep-equals); new `LayoutOp.popOutInto { id, key }` (moves a panel into an existing
  window's `panels`; no-op when already there); `LayoutOp.popIn` removes the panel from its
  window and drops empty window entries.
- Tests: `layout/tree.test.ts` — incl. same-reference no-op assertions for every op's
  change-nothing path, and prune dropping unknown ids from `panels`.

- [x] Failing tests first (op effects + no-op identity + window lifecycle).
- [x] Implement. Gates + commit.

### Task 3: Controller — rehydrate with grouping adjacency + retained arrangement

**Files:**
- Modify: `src/modules/panels/src/panels/controller.svelte.ts` — `#rehydratePoppedOut` reads
  `popouts`, floats each panel via the existing `REHYDRATE_FLOAT_BASE/STEP` cascade
  (constants and the `SHEET_CASCADE_*` parity test untouched), ordering so panels of one saved
  window cascade adjacently; KEEPS the `popouts` records (with rects) on the layout, marked
  dormant by virtue of their panels now being floating — decide the minimal truthful marker
  (e.g. a `dormant: true` field, or keep the window entries and let `locate` treat a window
  whose panels are all floating as restorable — prefer the explicit field, documented).
- The `panels.popoutRestoredFloating` notice path now also carries the information that a
  restore action is available (Task 6 wires the actual action).
- Tests: `controller.test.ts` — existing cascade tests must stay green; new: grouping-adjacent
  cascade order, retained arrangement, legacy-blob rehydrate.

- [x] Failing tests first. Implement. Gates + commit.

## Phase 2 — Engine wiring (DockviewEngine)

### Task 4: `apply()` already-floating reconcile branch

**Files:**
- Modify: `src/modules/panels/src/engine/dockview.ts` — the floating loop: when the panel is
  already floating and the tree rect differs from the live `boundingBox`, push the tree rect to
  the widget (`group.api.setSize({width, height})` for size; position via the internal overlay
  — read the vendored `FloatingGroupService`/`Overlay` for the position write, isolate it in one
  private method with the vendored-source re-verify note). Preserve `#lastFloatingRect`
  snapshot discipline so a reconcile never echoes back as an op through
  `#handleFloatingLayoutChange`.
- Test: `dockview.test.ts` — stub `getBoundingClientRect` per the existing live-sync tests;
  assert a tree-side rect change repositions the widget AND fires no `resizeFloating` echo.

- [ ] Failing tests first. Implement. Gates + commit.

### Task 5: Keyboard move/resize of floating windows

**Files:**
- Modify: `src/modules/panels/src/engine/dockview.ts` — in/around `#wireFloatingA11y`: keydown on
  the floating dialog wrapper (`role="dialog"` element): Arrow = move 8px, Shift+Arrow = 32px,
  Ctrl+Arrow = resize 8px (bottom/right edges), Ctrl+Shift+Arrow = 32px; each keydown emits
  `LayoutOp.resizeFloating` through the controller dispatch (via the existing op callback — NOT
  direct widget mutation; Task 4's reconcile moves the widget). Only when the event target is
  the dialog wrapper itself (never inside inputs/content). Update the dialog's `aria-label`
  hint text (i18n keys in en.ts) to document the shortcuts.
- Because ops flow through `applyOp`, FakeEngine honors keyboard resize with no new code —
  pin that with a `fake.test.ts` case (emit the op via the test helper, assert the float
  container's inline style updates).
- Test: `dockview.test.ts` keyboard emission (fire keydown on the wrapper, assert the op) +
  no-op when focus is in an input.

- [ ] Failing tests first. Implement. Gates + commit.

### Task 6: Notification actions + "Reopen windows" restore

**Files:**
- Modify: `src/client/core/src/notifications.ts` — `Notification` gains optional
  `action: { label: string; run: () => void }` (label pre-resolved like `message`; doc'd);
  `push` signature extended (backward compatible).
- Modify: `src/client/ui-kit/src/NotificationHost.svelte` — renders the action button when
  present; clicking runs `action.run()` and dismisses the notification.
- Modify: `src/modules/panels/src/panels/controller.svelte.ts` + `dockview.ts` — the
  rehydrate notice gains the restore action (i18n key e.g. `panels.reopenWindows`): one click
  (the required user gesture) re-opens each saved popout window via a new controller/engine
  method `restorePopouts()` — for each dormant window: pop out its first panel with
  `addPopoutGroup(panel, { position: savedRect })` (through the same `#requestPopOut` machinery —
  extract the shared core rather than duplicating it), then `popOutInto` the rest.
- Modify: `#requestPopOut` — when the panel has a saved popout rect (dormant window containing
  it), pass `position:` to the driver (clamped to the current screen's available bounds).
- Tests: core notifications test (action carried, push back-compat), NotificationHost test,
  controller/dockview tests (restore flow via the injected driver; gesture-time rect reuse).

- [ ] Failing tests first. Implement. Gates + commit.

### Task 7: Pop-out geometry capture

**Files:**
- Modify: `src/modules/panels/src/engine/dockview.ts` — subscribe
  `api.onDidPopoutGroupSizeChange` / `api.onDidPopoutGroupPositionChange`; on either, read the
  geometry from the popout entry's own window/dimensions (e.g. via `api.getPopouts()` matching
  the group, or `PopoutWindow.dimensions()` — verify against the vendored source) — NEVER the
  event payload (`onDidPopoutGroupPositionChange.screenY` is populated from `screenX` in the
  vendored 7.0.2) — and emit `LayoutOp.updatePopoutGeometry`. Dispose subscriptions in
  `destroy()` and when the popout closes.
- Modify: `docs/OPEN_BUGS.md` — log the upstream dockview defect + our avoidance (per campaign
  rule; informational — we never consume the field).
- Test: `dockview.test.ts` — fire the real emitters (existing tests show how,
  `(api as any).component._bufferOnDidLayoutChange.fire()` pattern / the popout event emitters
  on the component), stub the entry dimensions, assert the op payload.

- [ ] Failing tests first. Implement. Gates + commit.

### Task 8: Resize hit targets

**Files:**
- Modify: `src/modules/panels/src/panels.scss` — `.dv-resize-handle-*`: keep the 4px visual edge
  but grow the hit zone to ≥24px (transparent pseudo-element or negative-inset), 44px under
  `@media (pointer: coarse)`. Token-only colors.

- [ ] Implement; visual smoke via dev server screenshots (floating panel, all four edges).
  Gates + commit.

## Phase 3 — e2e + close-out

### Task 9: e2e — keyboard resize + arrangement persistence/restore

**Files:**
- Test: `src/client/shell/e2e/panels-floating.spec.ts` (or extend `panels.spec.ts` if it fits
  the file's scope better)

- [ ] Keyboard: float a panel (via PanelMenu), focus the floating dialog, Ctrl+Arrow → assert
  the ui-state PUT payload's layout carries the resized rect; reload → size survives.
- [ ] Arrangement: pop out a panel (real popup via `context.waitForEvent('page')`), resize/move
  the popup, close the popup or reload → assert the persisted `popouts` carries the rect;
  reload → floating rehydrate notice shows the Reopen action → click → new popup at the saved
  rect with the same panel set.
- [ ] Full e2e suite green. Commit.

## Final gates (whole M16b)

- [ ] All repo gates green incl. `pnpm docs:check-examples`, `pnpm lint:file-size`,
  `pnpm build:all`, full e2e.
- [ ] Review checkpoint over the M16b diff.
