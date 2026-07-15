# Open Bugs

Currently open, confirmed-real defects. Deferrals belong in `TODO.md`, not here.

## Server / move-execution

- [Movement] `movement::supercover_cells` can spuriously fail-closed (return `None`, rejecting an
  otherwise-legal move) on a diagonal king-step whose leg lands exactly on a 4-way grid-line
  intersection at BOTH endpoints — the Amanatides–Woo corner-crossing branch fires repeatedly and
  drifts the traversal away from the target cell before the `MAX_MOVE_CELLS` guard catches it.
  Reproduced via `execute_move`'s frozen-fixture scenario "diagonal 3-step king path, full
  visible" (`(200,200)→(300,100)` leg): `supercover_cells((200.0,200.0), (300.0,100.0), 100.0)`
  returns `None` even though the move is otherwise fully legal (no wall, no fog, no region).
  Fails closed (never opens a forbidden move) so it is not a security bug, but it rejects a move a
  player would reasonably expect to succeed. Worth a dedicated look by whoever next touches
  `movement.rs`'s corner-crossing branch. (Surfaced by the M10f-2 Task 6 fixture-freeze.)

## Client / scene-rendering

- [Vision] `RenderEngine.onSceneFrame`/`flushPendingDerived` (`src/client/render/src/engine.ts`) has a
  frame-ordering monotonicity hole: if a vision frame at seq 5 is deferred (its `computedAtSeq` is
  ahead of `store.appliedSeq`) and a NEWER frame at seq 7 subsequently arrives and takes the
  IMMEDIATE-apply branch (its own `computedAtSeq` is not ahead), `lastAppliedSeq` advances to 7
  without clearing the still-pending seq-5 entry. When the store later catches up to seq 5,
  `flushPendingDerived` applies the older seq-5 payload and regresses `lastAppliedSeq` back to 5 —
  a stale-but-same-scene visibility frame overwrites a newer one. Not a secrecy leak (both frames
  re-filter to the current viewed scene, per the M12d fog-secrecy fix in `74165e4`), only a
  momentary/self-correcting flicker to an older-but-valid fog state. Pre-existing (predates M12d);
  surfaced by the M12d Task 4 buddy-check fix-confirmation while verifying the fog-secrecy fix.

## Client / panels (FakeEngine bespoke-fallback only)

- [Panels] The bespoke-fallback engine — `FakeEngine` (`src/modules/panels/src/engine/fake.ts`,
  used only when a caller constructs `PanelHost` without an explicit `engine` prop, e.g. tests) —
  loses width containment once a THIRD docked group is added to the same zone
  (`right`/`bottom`/`left`): with exactly 2 groups the zone stacks within its column width, but
  the 3rd group renders full-viewport-width, covering the stage canvas underneath it (confirmed
  via screenshot: two "full width" stacked panels with no narrow right-hand column at all).
  `FakeEngine.apply` renders each zone's groups as plain stacked `<div>`s appended after the
  center well — the suspect rendering path. Deterministic and reproducible with generous
  real-time waits between each dock (NOT a same-tick/rapid-fire timing race). No UI affordance
  currently exists to un-dock/minimize a panel back out of a zone once docked under FakeEngine
  (the M12a Task 9 `PanelMenu` — dock/float/minimize commands on a tab's menu button — is
  mounted by `DockviewEngine.createTabComponent` only; `FakeEngine`'s plain tab strip has no
  menu, and non-chat panels default to `{kind:"minimized"}`, restored only forward via
  `PanelsChipsView.restore`), so once a `FakeEngine` session docks 3 panels into one zone there is
  no way back.
  **Not present under the production engine**: `panels/src/index.ts` now wires `DockviewEngine`
  as the `panel-host` surface's engine (M12a Task 9 step 0), and `stage.spec.ts`'s "author an
  animated (frame-list) actor token" e2e — which docks 4 groups into "right" across its
  lifetime — passes cleanly under it (un-fixme'd; dockview's "below" sibling-chain sizing
  contains width correctly past 2 groups). Fix belongs in `FakeEngine`'s zone layout if/when a
  bespoke-fallback caller needs more than 2 docked groups in one zone; not release-blocking since
  production never reaches `FakeEngine`.
