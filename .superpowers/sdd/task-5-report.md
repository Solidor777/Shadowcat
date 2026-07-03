# Task 5 Report: `scene/navmesh.rs` — query (`navmesh_find`)

**Status:** DONE

**Commit:** `a3cae41` — "feat(m10f-1): navmesh query - any-angle multi-leg routing,
input-validation parity with the grid engine"

---

## Summary

Implemented `navmesh_find` in `src/server/src/scene/navmesh.rs` per the task-5 brief, and added
`Clone` to `pathfinding::PathOutcome`'s derive list. Extended the brief's validation with a
magnitude bound the brief itself omitted (see below).

## Files changed

- `src/server/src/scene/navmesh.rs` — added `navmesh_find` (Step 4) + 7 tests (the brief's 6
  tests for `navmesh_find` + 1 additional magnitude-bound test).
- `src/server/src/scene/pathfinding.rs` — `PathOutcome` derive: `#[derive(Debug, PartialEq)]` →
  `#[derive(Debug, Clone, PartialEq)]` (Step 3, for Task 7's later needs).

## Deviation from the brief: magnitude-bound extension

The brief's `navmesh_find` validation (Step 4's code block) checks only `waypoints.len()` bounds
and `start`/`waypoints` finiteness. It does not bound their MAGNITUDE. Task 4's entire review
history (3 distinct Critical findings across 4 rounds, all documented in `MAX_NAVMESH_COORD`'s
doc comment in this same file) was about exactly this hazard: an unbounded-but-finite `f64`
coordinate saturates to `f32::INFINITY` on an `x as f32` cast — `is_finite()` does not catch this,
because the input WAS finite before the cast; only the cast result is infinite. That `Vec2`
then reaches `polyanya`/`spade`'s internal triangulation/spatial-query code, which panics via an
unhandled `.unwrap()` on `Err(InsertionError::TooLarge)` rather than failing closed.

`navmesh_find`'s `glam::Vec2::new(leg_start.0 as f32, leg_start.1 as f32)` /
`glam::Vec2::new(wp.0 as f32, wp.1 as f32)` casts (feeding `nav.mesh.path(from, to)`) are the
identical cast, on the identical class of untrusted input (a `Pathfind` request's `start`/
`waypoints`, unbounded-magnitude wire data), just on the query side rather than the
mesh-construction side Task 4 covered.

Fix: added a magnitude check reusing the existing `MAX_NAVMESH_COORD` constant (defined in this
same file by Task 4; not redefined) — `start.0.abs() <= MAX_NAVMESH_COORD && start.1.abs() <=
MAX_NAVMESH_COORD && waypoints.iter().all(|w| w.0.abs() <= MAX_NAVMESH_COORD && w.1.abs() <=
MAX_NAVMESH_COORD)` — returning `PathFail::Invalid` on violation, placed after the finiteness
check and before any leg is processed. This runs even before the first `nav.mesh.path` call, so
an oversized `start` alone (with no waypoints processed yet) also fails closed.

Added a regression test, `oversized_but_finite_start_or_waypoint_fails_closed_not_panic`,
mirroring Task 4's `oversized_but_finite_bounds_fail_closed_not_panic` naming/shape: asserts
`1e40` (finite, saturates on cast) in `start` and in a waypoint both return
`Err(PathFail::Invalid)`, not a panic.

No other deviations from the brief. The 6 brief-specified tests were added verbatim (Step 1).

## Test results

- `cargo test -p shadowcat --lib scene::navmesh -- --nocapture` — **23 passed, 0 failed** (14
  pre-existing Task 4 `tests`-module tests + 6 brief tests for `navmesh_find` + the 1 additional
  magnitude-bound test + the pre-existing 2-test `smoke` module).
- Full lib suite: `cargo test -p shadowcat --lib` — **373 passed, 0 failed**.

## Lint/format/typecheck status

- `cargo fmt --check` — clean (ran `cargo fmt` once to normalize the added test block, then
  verified `--check` passes).
- `cargo clippy --all-targets -- -D warnings` — clean for all code touched by this task. One
  pre-existing, unrelated failure remains in `src/server/src/scene/move_exec.rs` (`region_doc`,
  8/7 too-many-arguments) — confirmed via `git status`/`git log` that this file was not touched
  by this task; it is a pre-existing gap already logged in project memory
  (`8d61217 docs(m10f-0): log pre-existing clippy gap`), not introduced or worsened here.

## Deviations from the task spec

Only the magnitude-bound extension documented above (which the dispatcher pre-authorized in the
task instructions as resolving a gap in the brief). No other deviations — `navmesh_find`'s
implementation otherwise matches the brief verbatim, including keeping it `#[allow(dead_code)]` +
`pub(crate)` (mirroring `build_navmesh`'s existing pattern, since the movementModel
dispatch/caller has not landed yet).

## Residual risks / skill-update notes

None new for the codebase skill — `MAX_NAVMESH_COORD`'s existing doc comment already documents
the general saturating-cast hazard class; this task is a straightforward application of the same
guard to a second call site in the same module, not a new invariant. If/when the dispatcher
updates `shadowcat-codebase-scene-rendering` for the M10f-1 checkpoint as a whole, it should note
that `navmesh_find` (query side) enforces the SAME `MAX_NAVMESH_COORD` bound as `build_navmesh`
(construction side) — both sides of the navmesh boundary are now covered against the
saturating-cast class of panic.

---

## Commit hashes

`a3cae41` (single commit; first == last for this task)

---

## Fix: comment/doc accuracy

Post-review findings on this task (documentation-accuracy only, no logic change):

- **[Important] Corrected the magnitude-guard rationale.** The original doc comments (on
  `navmesh_find`, the inline comment above its magnitude check, and the regression test
  `oversized_but_finite_start_or_waypoint_fails_closed_not_panic`) claimed the query-side guard
  fixed "the same hazard class" as Task 4's construction-side `spade` triangulation panic. A
  reviewer empirically verified (scratch project against the pinned `polyanya = "0.16.1"`, calling
  `Mesh::path` directly with oversized/infinite coordinates) that this is FALSE for the query
  side: `Mesh::path` → `path_on_layers` → `get_closest_point_on_layers` does only bounded
  point-in-polygon containment checks — no `spade` triangulation call — and returns `None` for an
  out-of-range point, which `navmesh_find` already converts to `Err(PathFail::Unreachable)`. So
  without the guard, an oversized `start`/waypoint already fails closed safely; the guard changes
  only which safe error code is returned (`Invalid` vs `Unreachable`), not panic-vs-no-panic.
  Kept the guard (harmless, reasonable defense-in-depth against untrusted wire input reaching a
  third-party numeric library, gives a more precise error code) but rewrote the doc comment on
  `navmesh_find`, the inline comment above the magnitude check, `MAX_NAVMESH_COORD`'s doc comment,
  and the regression test's name (renamed to
  `oversized_but_finite_start_or_waypoint_is_rejected_by_the_magnitude_guard`) + docstring to state
  the accurate defense-in-depth rationale instead of the fabricated panic claim.
- **[Minor] Extended `MAX_NAVMESH_COORD`'s doc comment** to name the fourth coordinate surface it
  bounds (`navmesh_find`'s `start`/`waypoints`, added by this task) alongside the three
  construction-side surfaces from Task 4 (`w_px`/`h_px`, wall-segment endpoints,
  `footprint_scene`).
- **[Minor] Corrected this report's test-count bookkeeping**: "Files changed" now says 7 tests (6
  brief + 1 additional magnitude-bound test, not 8/2) and "Test results" now says 14 pre-existing
  Task 4 `tests`-module tests (not 15) — matching the "Test results" section's own correct
  "1 additional magnitude-bound test" phrasing, which the earlier count contradicted. The final
  total of 23 (14 Task-4 `tests` + 2 Task-1 `smoke` + 7 new Task-5) was already correct and is
  unchanged.
- **[Minor, optional] Retitled `multi_leg_route_concatenates_without_a_duplicated_join_vertex`'s
  docstring** (name kept — still accurate at the observable-behavior level) to state that it
  verifies the OBSERVABLE outcome (no duplicate consecutive vertex at a leg join), not the
  internal skip-branch (`dx*dx+dy*dy < 1e-6 → continue`) that was believed to guard it: per the
  real `polyanya` source, `Path::path` never actually includes a leg's start point, so that skip
  branch is dead code under normal circumstances and this test would pass identically if it were
  deleted.

No runtime/logic changes — comments, doc text, and one test name/docstring only.

### Verification

- `cargo test -p shadowcat --lib scene::navmesh` — 23 passed, 0 failed (unchanged from before the
  fix, confirming no logic changed).
- `cargo clippy --lib -- -D warnings` — clean.
