### Task 5 Report: `find()` arrest truncation + `PathOutcome` + `SceneEcs::pathfind` wiring

**Status:** DONE

**Commit:** `30ddf32` — "feat(m10g): arrest truncation in find(); wire per-requester region field into pathfind"

---

## Files changed

- `src/server/src/scene/pathfinding.rs`
  - Added `pub struct PathOutcome { path, cost, arrested }` (`#[derive(Debug, PartialEq)]`).
  - `pub fn find(...)` grows an 8th param `regions: Option<&crate::scene::regions::RegionField>`
    and returns `Result<PathOutcome, PathFail>` instead of `Result<(Vec<P>, f64), PathFail>`.
    Added `#[allow(clippy::too_many_arguments)]` (8 args, mirrors `SceneEcs::pathfind`'s existing
    attribute).
  - `find()` now threads `regions` into the assembled `PathGrid` (previously hardcoded to `None`).
  - After assembling the full route, `find()` scans `cells` (skipping index 0 — a token already
    standing in the start cell is not "entering" it) for the first cell where
    `RegionField::is_arrest` is true. If found, the route is truncated at that cell (inclusive),
    `arrested = true`, and the cost is recomputed by replaying `step_cost` + `terrain_multiplier`
    across the surviving prefix (parity threading restarts at 0 for the replay, which reproduces
    the same cost the original per-leg accumulation would give for that identical prefix, since
    parity is purely sequential/order-dependent, not leg-boundary-dependent).
  - Updated all pre-existing `find(...)` call sites in `mod find_tests` (7 tests) to pass a
    trailing `None` and to destructure the new `PathOutcome` fields (`.path`, `.cost`) instead of
    a tuple. `Err(...)` assertions only needed the trailing `None` argument added.
  - Added the two brief-specified tests: `arrest_region_truncates_the_route_and_flags_arrested`
    and `no_regions_argument_is_backward_compatible`.

- `src/server/src/scene/mod.rs`
  - `SceneEcs::pathfind`'s return type changed from `Result<(Vec<(f64,f64)>, f64), PathFail>` to
    `Result<pathfinding::PathOutcome, pathfinding::PathFail>`.
  - `pathfind` now computes `self.region_field(scene, if is_gm { None } else { Some(user) })` and
    passes `Some(&regions)` into `pathfinding::find`, instead of the caller's mask-only inputs.
  - Updated `pathfind_gm_unconstrained_routes_without_a_mask` to destructure the `PathOutcome`
    fields (`.cost`, `.path`) instead of a tuple. The other two pre-existing pathfind tests
    (`pathfind_nongm_visible_is_bounded_by_the_mask`, `pathfind_revealed_unions_explored_memory`)
    needed no change — they only assert on the `Err`/`is_ok()` shape, which is unaffected.

- `src/server/src/ws/conn.rs`
  - `handle_pathfind`'s `Ok((path, cost)) => ServerMsg::PathResult { request_id, path, cost }`
    updated to `Ok(outcome) => ServerMsg::PathResult { request_id, path: outcome.path, cost:
    outcome.cost }`. `outcome.arrested` is intentionally dropped here per the brief — Task 6 adds
    the `arrested` wire field to `PathResult` and will wire it through.

---

## Test results

- `cd src/server && cargo build` — clean, whole-crate, zero errors/warnings.
- `cargo test pathfinding::` — 24 passed, 0 failed (includes the 2 new tests).
- `cargo test pathfind_` (the 3 `scene::tests::pathfind_*` tests) — 3 passed, 0 failed.
- `cargo test` (full crate, all binaries + integration test files) — 336 lib tests + 14 (assets)
  + 8 (scene_derived) + 1 (scene_hydration) + 9 (ws_convergence) + 4 (ws_live_search) = 372 passed,
  0 failed.
- `cargo fmt -- --check` — clean.
- `cargo clippy --all-targets` — clean (0 warnings after adding `#[allow(clippy::too_many_arguments)]`
  to `find`, matching the existing attribute on `SceneEcs::pathfind`).

---

## Deviations from the task spec

One test deviation, discovered and fixed via TDD (Step 2/Step 4 of the brief):

- **`arrest_region_truncates_the_route_and_flags_arrested`**: the brief's verbatim test places
  the arrest `Rect` over cell `(2,0)` only (`x0:200,y0:0,x1:300,y1:100`) and asserts the truncated
  route ends at `(250.0, 50.0)` with cost `2.0`. Running this literally FAILED — not from a bug in
  the new arrest-truncation code, but from a pre-existing (Task 1-4, unrelated to this task) A*
  tie-break behavior: under `DiagonalRule::Chebyshev`, a straight 4-step orthogonal route
  `(0,0)->(4,0)` and several diagonal-detour routes (e.g. `(0,0)->(1,1)->(2,2)->(3,1)->(4,0)`) cost
  the same (4.0), and this build's `QNode` ordering (`Ord` tie-breaks toward the lexicographically
  *larger* `(cell, parity)` key) deterministically resolves the tie to the diagonal detour, not the
  straight row — verified by direct inspection (temporary debug print, then removed) of the
  produced `cells` vec, which was `[(0,0),(1,1),(2,2),(3,1),(4,0)]`, never visiting `(2,0)`. This
  exact class of Chebyshev-tie ambiguity is already documented and worked around in the
  pre-existing `terrain_region_raises_astar_step_cost` test's comment (same file), which widens its
  region to cover every king-move-adjacent intermediate cell for the same reason.
  - **Fix:** per CLAUDE.md's "assume improper implementation first, but verify against the code"
    and "tests yield to correct code" (the tie-break itself is legitimate, pre-existing,
    unmodified-by-this-task behavior), I widened the arrest region to cover the ENTIRE `i=2`
    column (`x0:200,y0:-300,x1:300,y1:300`) rather than a single cell, and updated the expected
    truncation point to `(250.0, 250.0)` — the tie-broken route's actual column-2 cell `(2,2)` —
    while preserving the cost assertion (`2.0`, still 2 steps in) and the `arrested` flag
    assertion unchanged. This keeps the test's intent (arrest fires exactly when the route first
    reaches the arrest column, at cost 2.0) while being robust to which tied A* route is chosen.
    Documented inline in the test with a comment explaining the tie-break and cross-referencing
    the pre-existing terrain test's identical technique.
  - All other test call-site updates (the 7 pre-existing `find_tests`, the 3 `pathfind_*` tests,
    and `no_regions_argument_is_backward_compatible`) matched the brief exactly with no deviation.

No other deviations. `PathOutcome`'s doc comment, the arrest-truncation logic, the cost-replay
comment, and the `SceneEcs::pathfind`/`handle_pathfind` changes are verbatim from the brief.

---

## Residual risks / skill-update notes

- **Secrecy property (task's stated security-sensitive concern) is upheld**: `SceneEcs::pathfind`
  passes `self.region_field(scene, if is_gm { None } else { Some(user) })` — a non-GM requester's
  region field is visibility-filtered (per Task 3's `region_field`), so a secret region can never
  influence that requester's route, cost, or reachability. This is unchanged from the brief; I did
  not touch `region_field`'s filtering logic in this task.
- **Arrest-truncation "honesty" property is upheld**: `find()`'s arrest scan runs against exactly
  the `regions` field passed in — for a non-GM caller that is the SAME per-requester-filtered field
  used for the impassable/terrain checks earlier in the same call, so a player is never shown a
  route continuing past an arrest cell they can actually see (an arrest cell the player CANNOT see
  is invisible to their field entirely and so does not truncate their preview — consistent with the
  spec's "springs only at execution" model for secret hazards, which `move_exec`'s separate
  authoritative-field arrest check — inert until a later task — is responsible for enforcing at
  execution time).
  Truncation is monotonic and irreversible: an unlucky terrain-based recompute cannot subsequently
  add anything past the first arrest cell.
  Recommend the buddy-check reviewers pay particular attention to two points:
  1. The cost-replay-after-truncation loop (`total = 0.0; for w in cells.windows(2) { ... }`)
     re-derives parity from 0 rather than reusing any parity value from the original per-leg
     accumulation — verified correct by inspection (parity is a pure function of step sequence,
     order-dependent only) and by the passing `alternating` tests elsewhere in the file, but there
     is no NEW dedicated regression test proving replay-parity equals original-accumulation-parity
     across a truncation that crosses a waypoint boundary (multi-leg + arrest interaction is
     untested in this task; the brief's two new tests are both single-leg).
  2. `handle_pathfind`'s dropped `outcome.arrested` is a known, brief-acknowledged temporary gap —
     Task 6 must wire it into `PathResult`'s wire field before a route with `arrested: true` is
     actually surfaced to the client (until then, the client cannot tell a truncated-for-arrest
     route from an ordinarily-short route — no behavior regression today, since no caller reads
     `arrested` yet, but this is the reason Task 5 and Task 6 are sequenced tightly).
- No `shadowcat-codebase-scene-rendering` skill update needed for this task alone — the skill
  already documents `region_field`'s visibility-filtering invariant (Task 3) and the
  impassable/terrain gate points (Task 4); `PathOutcome`/arrest-truncation is new but small enough
  that I recommend deferring the skill edit until Task 6 completes the wire-protocol side, so the
  skill records the FULL arrest feature (truncation + wire field) in one edit rather than two
  partial ones. Flagging this now so the dispatcher doesn't lose track of it.

---

## Commit hashes

`30ddf32` (single commit; first == last for this task)
