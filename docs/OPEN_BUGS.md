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

## Client / panels (shipped FakeEngine)

- [Panels] The SHIPPED engine — `FakeEngine` (`src/modules/panels/src/engine/fake.ts`;
  `PanelHost.svelte` defaults to it, and nothing in `src/` injects `DockviewEngine`, which is
  exported but unwired outside its own tests) — loses width containment once a THIRD docked
  group is added to the same zone (`right`/`bottom`/`left`): with exactly 2 groups the zone
  stacks within its column width, but the 3rd group renders full-viewport-width, covering the
  stage canvas underneath it (confirmed via screenshot in the served app: two "full width"
  stacked panels with no narrow right-hand column at all). `FakeEngine.apply` renders each
  zone's groups as plain stacked `<div>`s appended after the center well — the suspect
  rendering path. Deterministic and reproducible with generous real-time waits
  between each dock (NOT a same-tick/rapid-fire timing race) — confirmed independent of the M12a
  B4 persisted-layout fix by reproducing it on the pre-B4-fix code via explicit rapid chip
  restores (`chip-<id>:panel` → `restore()`, always docks a fresh group into "right"). No UI
  affordance currently exists to un-dock/minimize a panel back out of a zone once docked (M12a
  ships no minimize/close control on a docked panel's tab — the interim default-placement design
  is `{kind:"minimized"}` for non-chat panels, restored only forward via `PanelsChipsView.restore`),
  so once a session docks 3 panels into one zone there is no way back. Fix belongs in
  `FakeEngine`'s zone layout; any engine wired in later (`DockviewEngine`'s "below" sibling-chain
  sizing is unverified until then) must pass the same 3-groups-in-one-zone containment check. Blocks
  `stage.spec.ts`'s "author an animated (frame-list) actor token" e2e test, marked `test.fixme`
  pending this fix (it needs `assets` + `settings` + `actors` all docked in "right" across its
  lifetime, on top of `chat`'s permanent default dock = 4 total). Surfaced by the M12a B4
  buddy-check fix's own mandated e2e verification, which correctly restores richer persisted
  layouts across a reload/re-entry than any prior test exercised.
