# Closed Bugs

Confirmed-real defects that have since been fixed, kept for provenance. New fixes append a new
`##` section (or bullet under an existing one); do not delete resolved entries.

## Server / move-execution

- [Movement] `movement::supercover_cells` spuriously failed-closed (returned `None`, rejecting an
  otherwise-legal move) on a diagonal king-step whose leg landed exactly on a 4-way grid-line
  intersection at BOTH endpoints. Root cause: the corner-crossing branch stepped BOTH axes on
  every `tMax` tie without checking whether an axis had already reached its target cell — once a
  preceding forced single-axis step (caused by a start coordinate sitting exactly on a grid line)
  put `t_max_i`/`t_max_j` into permanent lockstep, every later tie re-stepped the already-arrived
  axis too, drifting the traversal past `(ei,ej)` until `MAX_MOVE_CELLS` aborted it. Fixed by
  gating the diagonal corner-step on a per-axis remaining-step budget (`remaining_i`/`remaining_j`,
  the exact number of grid-line crossings still owed on each axis): the corner branch now only
  fires when BOTH axes still owe a step (a genuine mid-path crossing with more path beyond); once
  one axis's budget hits zero, only the other axis steps, regardless of any `tMax` tie. This makes
  convergence a property of the (bounded) step budget rather than floating-point tie-breaking, and
  preserves the existing safe-over-include behavior for genuine mid-path corner crossings (both
  flanking cells still emitted whenever real path remains on both axes). Regression tests:
  `diagonal_leg_with_both_endpoints_on_lattice_corners_succeeds` (the exact bug-report repro),
  `perfect_diagonal_across_many_lattice_corners_converges` (a longer 45-degree diagonal crossing
  several lattice-corner ties in a row), `single_endpoint_on_lattice_corner_still_includes_flankers`
  (proves flanker emission is unregressed when only one endpoint is lattice-aligned) —
  `src/server/src/scene/movement.rs`. `execute_move`'s frozen-fixture scenario "diagonal 3-step
  king path, full visible" (`src/server/src/scene/move_exec.rs`) — previously frozen at the
  buggy `truncated: true, cost: 2.0` outcome with an explicit comment marking it as a known,
  pre-existing defect — is now updated to the correct `truncated: false, cost: 3.0` outcome.

## Client / scene-rendering

- [Vision] `RenderEngine.onSceneFrame`/`flushPendingDerived` (`src/client/render/src/engine.ts`)
  had a frame-ordering monotonicity hole: if a vision frame at seq 5 deferred into `pendingDerived`
  (its `computedAtSeq` ahead of `store.appliedSeq`) and a NEWER frame at seq 7 subsequently arrived
  and took the IMMEDIATE-apply branch (its own `computedAtSeq` no longer ahead), `lastAppliedSeq`
  advanced to 7 without clearing the still-set seq-5 entry — `onSceneFrame`'s immediate branch never
  touched `pendingDerived`. A later `flushPendingDerived` call (triggered by any subsequent store
  commit) re-checked only `store.appliedSeq >= p.seq`, found it satisfied, and re-applied the stale
  seq-5 payload — regressing `lastAppliedSeq` back to 5 and overwriting the newer seq-7 mask with an
  older-but-valid one. Not a secrecy leak (both frames re-filter to the current viewed scene, per
  the M12d fog-secrecy fix in `74165e4`), only a momentary/self-correcting flicker. Fixed by adding
  a monotonicity guard to `flushPendingDerived`: a pending entry is now applied only when its `seq`
  is still greater than `lastAppliedSeq` at flush time — otherwise it is DISCARDED, never applied.
  The pending slot is unconditionally cleared as soon as the watermark condition
  (`store.appliedSeq >= p.seq`) is met, whether the entry is applied or discarded, so a superseded
  entry never lingers past this check. The pre-existing scene re-filter-at-flush-time behavior
  (M12d) is untouched — this fix only adds the seq-ordering guard, applied AFTER the existing
  `store.appliedSeq >= p.seq` watermark check and BEFORE the `toVisibility` re-filter call.
  Regression test: `"a stale deferred frame superseded by a later immediate-apply frame is
  discarded, not re-applied, on flush (no lastAppliedSeq regression)"` (`src/client/render/src/
  engine.test.ts`) — drives the exact repro sequence by mutating `store.appliedSeq` directly (a
  plain field) to isolate `RenderEngine`'s own watermark contract from `DocumentStore`'s incidental
  commit-triggers-flush coupling; confirmed failing against the pre-fix code before the fix landed.
