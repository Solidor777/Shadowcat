# Task 5 Report — M10f-4: execution + cost-secrecy verification (tests only)

**Status:** DONE

**Commit:** `47419c3` — `test(m10f-4): prove engine-agnostic executor + cost-secrecy hold for weighted continuous routes`

---

## Summary
Test-only checkpoint task. Proved `move_exec`'s engine-agnostic executor correctly gates,
accrues terrain cost, and arrests on a weighted continuous polyline with zero production
change; confirmed the `MoveStream.cost` secrecy invariant is engine-agnostic; added an
additional router-secrecy test (surfaced by Task 4's buddy-check) proving a `gm_only` arrest
region on the pure-polyanya continuous path is invisible in a non-GM player's route preview
but still springs at `move_exec` execution time.

## Files changed
- `src/server/src/scene/move_exec.rs` — added
  `execute_move_handles_an_any_angle_weighted_continuous_polyline` (per the brief's Step 1,
  verbatim aside from `cargo fmt` reflow), placed just before the differential parity test
  suite section.
- `src/server/src/ws/conn.rs` — appended a one-line invariant comment to `clip_move_stream`'s
  existing `cost: None` clip site (the `no-cost-leak` block already documented this invariant
  from M10g; added the brief's exact phrasing rather than duplicating a new comment block).
- `src/server/src/scene/mod.rs` — added
  `pathfind_continuous_secret_arrest_absent_from_player_preview_but_springs_at_execution`,
  placed alongside the existing `pathfind_continuous_*` tests (after
  `pathfind_continuous_nongm_route_clips_to_the_visible_mask`).

## Tests added + result
```
$ cd src/server && cargo test -p shadowcat execute_move_handles_an_any_angle_weighted_continuous_polyline
test scene::move_exec::tests::execute_move_handles_an_any_angle_weighted_continuous_polyline ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 431 filtered out; finished in 0.00s

$ cargo test -p shadowcat pathfind_continuous_secret_arrest_absent_from_player_preview_but_springs_at_execution
test scene::tests::pathfind_continuous_secret_arrest_absent_from_player_preview_but_springs_at_execution ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 432 filtered out; finished in 0.01s
```
Both passed on the first run with **zero production code changes**, confirming the M10f-2/3
engine-agnostic-executor and M10g region-secrecy invariants hold for this checkpoint's new
weighted/smoothed continuous routing without further work — per the brief's halt-and-verify
instruction, this was not "fixed" — there was nothing to fix.

## Full regression suite
```
$ cd src/server && cargo test -p shadowcat
test result: ok. 433 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out   (lib)
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out     (main bin)
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out     (test_server bin)
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out    (tests/assets.rs)
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out     (tests/scene_derived.rs)
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out     (tests/scene_hydration.rs)
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out     (tests/ws_convergence.rs)
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out     (tests/ws_live_search.rs)
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out     (doc-tests)
```
All green — whole suite passes. Lib test count grew 431 → 433 (the two new tests).

## Lint/format/typecheck status
```
$ cargo fmt      -> clean (only reflows the new test code itself)
$ cargo clippy -- -D warnings
    Checking shadowcat v0.1.0 (...)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.83s
```
No warnings.

## Deviations from the task spec
- Step 1's test was added exactly as specified in the brief.
- Step 3 asked for a one-line comment "if not already present" at the `clip_move_stream` site —
  an equivalent `no-cost-leak` invariant comment already existed there (from M10g); appended the
  brief's exact phrasing to that existing block instead of creating a redundant second comment.
- The additional secrecy test (surfaced by Task 4's buddy-check) was placed in
  `src/server/src/scene/mod.rs` rather than `move_exec.rs`, per the dispatcher's explicit
  "use your judgment" instruction. Rationale: the first half of the claim (secret arrest absent
  from a player's route preview, present for the GM) is exclusively about `SceneEcs::pathfind`'s
  dispatch — the router's per-requester `region_field` filtering — the exact thing every sibling
  `pathfind_continuous_*` test in `mod.rs` already exercises. `move_exec.rs` already has its own
  `authoritative_field_springs_a_secret_region_a_player_was_routed_through` test proving the
  executor-side half in isolation (for `impassable`, on a king-step path). Rather than duplicate
  that proof for `arrest` in a second file, the new test does BOTH halves in one place — router
  preview via `pathfind`, then feeds the player's own previewed route straight into
  `move_exec::execute_move` — because the interesting claim is the END-TO-END round trip
  (preview hides it, commit springs it), which only `mod.rs` can express since it has direct
  access to both `pathfind` and (via `crate::scene::move_exec::execute_move`) the executor.
- One project-commenting-rule self-correction during drafting: an early comment draft on the new
  `mod.rs` test referenced "Task 4's buddy-check" (process-meta); rewritten as a present-tense
  code fact ("distinct from the weighted-grid branch") per the repo's commenting standard before
  the edit was accepted.

## Reference-pattern verification
Verified the pure-polyanya-branch claim directly against `SceneEcs::pathfind`'s `Continuous`
dispatch (`scene/mod.rs`): `RegionField::has_terrain_or_impassable()` returns `false` for a field
containing only an `Arrest` region (checked `regions.rs`'s
`has_terrain_or_impassable_detects_weight_but_not_arrest` test, which already documents this),
so an arrest-only scene takes the `navmesh_find` → `clip_to_visible_mask` → `truncate_at_arrest`
branch, not the weighted-grid-then-LOS-smoothed branch Task 4 already covered — confirming the
new test exercises the specific engine path the dispatcher's additional requirement asked for.

## Residual risks / skill-update notes
None. This task adds tests only; no seam, invariant, or gotcha changed. The
`shadowcat-codebase-scene-rendering` skill's existing documentation of `move_exec`'s
engine-agnostic-since-M10f-2 property and the region-secrecy two-value contract (`region_field`,
`gm_only` springing at execution) already fully covers what these tests prove. No new subsystem
opened. Stating explicitly per the skill-update gate: **no skill update needed for this task**
(Task 6 is the checkpoint's designated docs/skill-update task).

---

## Commit hashes

`47419c3` (single commit; first == last for this task)
