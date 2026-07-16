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

## Client / panels (FakeEngine bespoke-fallback only)

- [Panels] `FakeEngine` (`src/modules/panels/src/engine/fake.ts`) lost width containment once a
  third docked group was added to the same zone (`right`/`bottom`/`left`), rendering
  full-viewport-width and covering the stage canvas underneath it. Root cause: `FakeEngine.apply`
  never read `ZoneNode.size` — the zone's own px cross-size basis, already tracked by the
  reducer (`layout/tree.ts`) and driven by dockview's real splitter for the production engine —
  so a zone `<div>` carried no width of its own. `init()` built `host` as a single column flex
  container with `centerEl` and all three zone `<div>`s as plain unstyled siblings; a flex item
  with no explicit width, inside a column flex container, stretches to the container's full
  cross-size (`align-items: stretch`, the flex default) regardless of how many groups are docked
  into it — the "2 groups OK, 3rd breaks" threshold was purely a function of when a zone's
  aggregate content first grew wide enough to visually register the always-present stretch, not a
  structural change in the DOM/CSS at the 3rd group specifically. Fixed by giving `FakeEngine` a
  real docked-layout geometry: `init()` now nests a `row` flex container (`left` zone / `centerEl`
  / `right` zone side by side) inside `host`'s column flow, with `bottom` as a full-width row
  below it; each zone `<div>` (`#makeZoneEl`) carries `flex: 0 0 auto`, `min-width: 0` (so its own
  intrinsic content can never force it wider than its basis), and `overflow: auto` (oversized
  content scrolls WITHIN the zone instead of escaping it). `apply()` now applies `ZoneNode.size`
  as the zone's actual px width (right/left) or height (bottom) on every reconcile — 0 while the
  zone has no groups, so an empty zone reserves no layout space — and each group `<div>` gets
  `width: 100%; min-width: 0` so a wide panel's content can't push its own group wider than the
  zone. Regression test: `"FakeEngine constrains a zone's cross-size to ZoneNode.size once it has
  docked groups, past 2 groups"` (`src/modules/panels/src/engine/fake.test.ts`) — asserts the
  zone container's inline `width`/`flex`/`overflow` styles both at 2 and at 3 docked groups
  (jsdom has no layout engine, so this asserts the CSS containment contract, not computed
  pixels); confirmed failing (`eng.zoneEl is not a function`, then a missing `width` once the
  accessor was added) against the pre-fix code before the fix landed. Not present under the
  production engine (`DockviewEngine`), which was already unaffected — see
  `stage.spec.ts`'s "author an animated (frame-list) actor token" e2e.
## Server / data (OCC)

- [Critical, FIXED] `apply_intent`'s Phase-1 OCC pre-image comparison (`data/sqlite.rs`,
  `actual != ch.old`) used raw `serde_json::Value` equality, which spuriously rejected an
  otherwise up-to-date write. Mechanism: `serde_json::Value::Number` splits whole numbers into
  `PosInt`/`NegInt` and non-whole numbers into `Float`; the server stores an M13-0 `engine` `f64`
  field as `Float` even when its value is a whole number (e.g. `100.0`), but a JS client cannot
  preserve "this was a float" for a whole-number value through `JSON.parse`/re-serialize — the
  echoed OCC pre-image comes back as `PosInt`, and raw `==` treats `PosInt(100)` and `Float(100.0)`
  as unequal. Reachable via an ordinary token drag (`sendMoves`,
  `src/modules/scene-tools/src/controller.svelte.ts`) performed any time after a server-executed
  move (`execute_move`, which commits `/engine/x,y` as `Float`), and via the `ActorsPanel`
  vision-range editor and `GameSettingsPanel` numeric editors, whose pre-images are nested
  arrays/objects containing whole-number `Float` leaves. Fix: `values_semantically_eq`
  (`data/sqlite.rs`), a structural equality that recurses into `Object`/`Array` and treats
  mismatched-variant `Number` leaves as equal when numerically equal. Same-variant integer PAIRS
  (both PosInt/NegInt) are compared EXACTLY as `i128` with no magnitude limit; the `|n| <= 2^53`
  exactness guard applies only to the mixed case (one integer side, one `Float` side), where an
  `f64` comparison is unavoidable. Scoped to the OCC pre-image comparison only — Phase-2
  normalization and all other equality checks are untouched. Regression coverage: 9 unit tests on
  `values_semantically_eq` (whole-number Float/PosInt equality, genuinely stale rejection, nested
  array/object recursion, >2^53 mixed-case precision fallback, negative-number variant mismatch,
  large same-variant integer pairs that alias under f64 but must reject, opposite-sign
  same-magnitude rejection, trivially-equal small integers) plus an integration test
  (`ws::room::room_tests::client_update_with_posint_pre_image_after_execute_move_is_accepted`)
  reproducing the real `execute_move` → client-drag path end to end.
- [Critical, FIXED] Round 2: the fix above's Number-comparison branch had no magnitude guard when
  BOTH sides parsed as same-variant integers, falling through to the lossy `f64` equality used for
  the mixed case. Two distinct same-variant integers whose magnitude exceeds 2^53 (e.g. `2^62` vs
  `2^62 + 1`) alias to the same `f64` and were incorrectly reported equal — an OCC bypass in the
  silent-lost-update direction, strictly worse than raw equality for this case (raw equality would
  have correctly rejected them). Fix: the both-integers case now compares as `i128` exactly and
  never falls through to `f64`; the `f64`-tolerant path is reserved exclusively for the genuinely
  mixed integer/`Float` case. Regression coverage: 4 additional unit tests (large same-variant
  PosInt pair that aliases under `f64`, large same-variant NegInt pair, opposite-sign
  same-magnitude rejection, trivially-equal small-integer pair).
