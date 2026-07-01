# M3 — Vision-Gated Pathfinder Parity + Router Region Hook Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close buddy-check P1 by making the M10e-6 grid-A* router's vision-mask
predicate a superset of the M1 move-executor's per-step predicate, and add a
same-shaped inert region-arrest hook to the router so M10g has one hook shape
to wire in both places.

**Architecture:** Both fixes land entirely inside `src/server/src/scene/pathfinding.rs`'s
`cell_enterable`, the single per-neighbor-step gate the A* search calls. No other
file changes. The mask fix reuses the exact `movement::supercover_cells` primitive
the gate (`move_exec.rs`) and legacy publish gate (`ws/room.rs`) already use, so
parity is by construction, not by re-derivation. **Load-bearing fact (buddy-check
Agreed finding):** the gate's mask check in both `move_exec.rs` and `ws/room.rs`'s
`publish` is the *raw* `supercover_cells(prev, next, cell)` result with no
footprint-disc term at all — it is not "supercover plus footprint," it's just
supercover. This is why the union in Task 1 must include the step's supercover
unconditionally (not only when it exceeds the footprint set): the gate requires
**both** endpoint cells of every step to be in the mask regardless of footprint
size, including the mover's own current (`from`) cell.

**Tech Stack:** Rust (server crate `shadowcat`), no new dependencies.

## Global Constraints

- Server crate is `shadowcat` (NOT `shadowcat-server`). Test: `cargo test -p shadowcat <name>`. Full gate: `cargo test -p shadowcat`, `cargo fmt --check`, `cargo clippy -p shadowcat --all-targets -- -D warnings`.
- The router's mask predicate must become a **superset of** (≥) the gate's `supercover_cells`-based predicate — additive only, never loosen an existing check.
- Fail closed: `movement::supercover_cells` returning `None` (degenerate/over-cap span) means the step is **not enterable**, mirroring `move_exec.rs` and `ws/room.rs::publish`'s `None ⇒ Forbidden` behavior.
- No changes to `move_exec.rs`, `ws/room.rs::publish`, or `PathGrid`'s public field shape (spec §3/§6).
- `pathfinding.rs` stays pure/headless — no ECS borrow, no I/O (module invariant, unchanged by this plan).
- The region-arrest hook stays a same-shaped **inert stub** (`fn(...) -> bool { false }`) — no new `PathGrid` field, no region data model (that's M10g, spec §4/§6).
- Per project `CLAUDE.md`: dispatch implementation via the `shadowcat-coder` agent (not generic subagent dispatch); dispatch review via the `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` two-reviewer pair. Escalate to the `-opus` twins on BLOCKED or shallow findings before asking the human.
- Spec: `docs/superpowers/specs/2026-07-01-m3-vision-gated-pathfinder-design.md`. Follow it exactly; this plan is its task-level decomposition.

---

### Task 1: Router mask predicate becomes a superset of the gate's (supercover parity)

**Files:**
- Modify: `src/server/src/scene/pathfinding.rs:30` (imports), `:86-118` (`cell_enterable`)
- Test: `src/server/src/scene/pathfinding.rs` (`mod tests` at the bottom of the same file, starting line 668)

**Interfaces:**
- Consumes: `crate::scene::movement::supercover_cells(a0: (f64,f64), a1: (f64,f64), cell: f64) -> Option<BTreeSet<(i32,i32)>>` (already `pub`, defined in `src/server/src/scene/movement.rs`; unchanged).
- Produces: `cell_enterable(grid: &PathGrid, from: Cell, to: Cell) -> bool` keeps its existing signature — no caller elsewhere needs to change (`astar_leg`/`find` call it unchanged).

- [ ] **Step 1: Write the failing regression test for buddy-check P1**

Add to the `mod tests` block at the bottom of `src/server/src/scene/pathfinding.rs` (after `footprint_cell_outside_mask_is_not_enterable`, before `cell_outside_window_is_not_enterable`):

```rust
    #[test]
    fn diagonal_step_missing_flanker_cell_is_not_enterable_small_footprint() {
        // Buddy-check P1 regression. A perfectly diagonal step (0,0)->(1,1) at cell=100 crosses
        // the shared corner exactly, so supercover_cells emits BOTH flanker cells (1,0) and (0,1)
        // in addition to the two endpoint cells. A small (point-sized) footprint disc at the
        // destination (1,1) only overlaps (1,1) itself — footprint_cells alone would not catch a
        // missing flanker. The mask below has every cell EXCEPT the (0,1) flanker: the step must
        // be rejected once the router's mask check includes the step's supercover, even though
        // the footprint-disc-only check (pre-fix behavior) would have passed it.
        let walls: Vec<Seg> = vec![];
        let mut mask = BTreeSet::new();
        mask.insert((0, 0));
        mask.insert((1, 0));
        mask.insert((1, 1));
        // (0, 1) deliberately absent — the missing flanker.
        let g = grid(&walls, Some(&mask), 0.1);
        assert!(
            !cell_enterable(&g, (0, 0), (1, 1)),
            "diagonal step must be rejected when a supercover flanker cell is outside the mask, \
             even though the footprint disc at the destination doesn't reach that flanker"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p shadowcat diagonal_step_missing_flanker_cell_is_not_enterable_small_footprint`
Expected: FAIL — `assertion failed: !cell_enterable(&g, (0, 0), (1, 1))` (today's `footprint_cells`-only
check passes the step because `(1,1)`'s point-sized footprint disc doesn't reach `(0,1)`).

- [ ] **Step 3: Write the failing degenerate-input fail-closed test**

Add directly after the test from Step 1, still inside `mod tests`:

```rust
    #[test]
    fn degenerate_step_supercover_is_not_enterable_even_if_mask_covers_destination() {
        // A step spanning an enormous cell distance makes `supercover_cells` return `None` (the
        // MAX_MOVE_CELLS span guard in movement.rs). The router must fail closed on `None`, same
        // as move_exec.rs and ws/room.rs::publish, regardless of what the mask contains at `to`.
        let walls: Vec<Seg> = vec![];
        let mut mask = BTreeSet::new();
        mask.insert((5000, 5000)); // covers the destination; would pass footprint_cells alone.
        let mut g = grid(&walls, Some(&mask), 0.0);
        g.window = (-10_000, -10_000, 10_000, 10_000);
        assert!(
            !cell_enterable(&g, (0, 0), (5000, 5000)),
            "an over-cap/degenerate step supercover must fail closed, not fall back to the \
             footprint-only mask check"
        );
    }
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `cargo test -p shadowcat degenerate_step_supercover_is_not_enterable_even_if_mask_covers_destination`
Expected: FAIL — today's code never calls `supercover_cells`, so the mask-covers-destination case
incorrectly passes (`footprint_cells` for a `0.0`-radius disc at `(5000,5000)` returns just
`(5000, 5000)`, which is in the mask; no wall crosses the step; the step is wrongly enterable).

- [ ] **Step 5: Implement the fix**

The existing `use` block at the top of `src/server/src/scene/pathfinding.rs` (lines 30-31) is:

```rust
use crate::scene::vision::{self, point_segment_distance};
use std::collections::BTreeSet;
```

Insert exactly one new line, `use crate::scene::movement;`, between them — the other two lines
already exist and must not be duplicated:

```rust
use crate::scene::vision::{self, point_segment_distance};
use crate::scene::movement;
use std::collections::BTreeSet;
```

Replace `cell_enterable` (currently lines 86-118) with:

```rust
/// Whether a token may step from `from` into `to`. INVARIANT (spec §4.3, M3 spec §3): full
/// geometric footprint clearance — (1) the footprint disc at `to` clears every `blocksMove` wall,
/// (2) every footprint-overlapped cell AND every cell the center-to-center step's supercover
/// crosses (including diagonal corner-flankers) is in the mask (non-GM), (3) the center step
/// `from→to` crosses no wall, (4) no region arrests entry (M3/M10g stub).
pub(crate) fn cell_enterable(grid: &PathGrid, from: Cell, to: Cell) -> bool {
    let (i0, j0, i1, j1) = grid.window;
    if to.0 < i0 || to.0 > i1 || to.1 < j0 || to.1 > j1 {
        return false;
    }
    let r_scene = grid.footprint_radius_cells.max(0.0) * grid.cell;
    let ctr = cell_center(to, grid.cell);
    let a = cell_center(from, grid.cell);

    // (1) Footprint disc vs every blocksMove wall.
    for w in grid.walls {
        if point_segment_distance(ctr, w.a, w.b) < r_scene {
            return false;
        }
    }
    // (2) Mask: every footprint-overlapped cell, AND every cell the center-to-center step's
    // supercover crosses, must be visible/revealed (non-GM).
    //
    // INVARIANT (spec §13 / M3 design §3): `movement::supercover_cells` is the SAME primitive
    // the M1 move executor (`move_exec.rs`) and the M10e-4 `ws/room.rs::publish` gate check per
    // step. The router's mask predicate must be a superset of the gate's, or a route this A*
    // search approves can be rejected at execution time (buddy-check P1: for a sub-0.5-cell
    // footprint, the destination footprint disc alone never reaches a diagonal step's corner
    // flanker cells). `None` (degenerate/over-cap span) fails closed: not enterable, mirroring
    // the gate's `None ⇒ Forbidden`.
    if let Some(mask) = grid.mask {
        for c in footprint_cells(to, ctr, r_scene, grid.cell) {
            if !mask.contains(&c) {
                return false;
            }
        }
        match movement::supercover_cells(a, ctr, grid.cell) {
            Some(step_cells) => {
                if !step_cells.iter().all(|c| mask.contains(c)) {
                    return false;
                }
            }
            None => return false,
        }
    }
    // (3) Center-to-center step clears every wall (reuses the M9 segment-cross predicate).
    for w in grid.walls {
        if crate::scene::segments_cross(a, ctr, w.a, w.b) {
            return false;
        }
    }
    true
}
```

Note: `let a = cell_center(from, grid.cell);` moves up from step (3) to right after `ctr` is
computed, since step (2) now needs it too — this is the only structural reshuffle; steps (1) and
(3)'s bodies are otherwise unchanged.

- [ ] **Step 6: Run both new tests to verify they pass**

Run: `cargo test -p shadowcat diagonal_step_missing_flanker_cell_is_not_enterable_small_footprint degenerate_step_supercover_is_not_enterable_even_if_mask_covers_destination`
Expected: both PASS.

- [ ] **Step 7: Fix a pre-existing test the union correctly turns red (buddy-check Agreed/Important)**

`footprint_cell_outside_mask_is_not_enterable` (lines 738-755) has a second assertion that the fix
changes from passing to failing, and the plan must not leave this undisclosed: the gate's mask
check (both here and in `move_exec.rs`) requires **every** cell of a step's supercover — which
always includes **both** endpoint cells — to be in the mask, not just the destination footprint.
The test's point-footprint case uses `mask = {(1,0)}` only, so `(1,1)` (the `from` cell) is now
correctly rejected as not-in-mask. This is not a bug in the fix — it's the fix correctly enforcing
that the mover's own current cell must be visible too, a case the original test didn't cover. Fix
the fixture, don't loosen the check.

Replace the test's second half (currently):

```rust
        // A point-sized footprint overlaps only (1,0) → enterable.
        let gp = grid(&walls, Some(&mask), 0.0);
        assert!(cell_enterable(&gp, (1, 1), (1, 0)));
```

with:

```rust
        // A point-sized footprint overlaps only (1,0) at the destination — but the M3 fix also
        // requires the FROM cell in the mask (supercover_cells always includes both endpoint
        // cells). Add (1,1) to represent a realistic case where both the mover's current cell
        // and its destination are visible.
        let mut mask_from_and_to = mask.clone();
        mask_from_and_to.insert((1, 1));
        let gp = grid(&walls, Some(&mask_from_and_to), 0.0);
        assert!(cell_enterable(&gp, (1, 1), (1, 0)));
```

- [ ] **Step 8: Write and pass a large-footprint-diagonal strengthening test (buddy-check Agreed/Minor)**

The existing large-footprint regression coverage (`footprint_cell_outside_mask_is_not_enterable`,
radius 0.6) only exercises an orthogonal step, whose supercover is always just its two endpoint
cells — already inside any footprint disc that dominates it. It cannot exercise the diagonal
corner-flanker union path at all. Add a dedicated large-footprint-diagonal case proving the union
doesn't introduce a new false rejection when the footprint disc already covers the flankers. Add
to `mod tests`, after the test from Task 1 Step 3:

```rust
    #[test]
    fn large_footprint_diagonal_step_with_flankers_in_mask_is_still_enterable() {
        // A large footprint (1.0 cell radius) at the destination of a diagonal step already
        // overlaps both corner-flanker cells — the footprint_cells check alone would pass this.
        // Prove the ADDED step-supercover union doesn't introduce a new false rejection when the
        // mask already covers everything the footprint disc covers.
        let walls: Vec<Seg> = vec![];
        let mask = visible_grid(6); // covers (0,0)..(5,5); large enough for both footprint + step.
        let g = grid(&walls, Some(&mask), 1.0);
        assert!(
            cell_enterable(&g, (2, 2), (3, 3)),
            "large footprint with a fully-visible mask must remain enterable after the union fix"
        );
    }
```

Note: `visible_grid` is defined in `pathfinding.rs`'s `astar_tests` module (line ~140), not the
`tests` module this test lives in — copy its body inline (it's a 4-line helper) rather than
importing across test modules:

```rust
    fn visible_grid(range: i32) -> BTreeSet<Cell> {
        (0..range)
            .flat_map(|i| (0..range).map(move |j| (i, j)))
            .collect()
    }
```

Add this helper function inside the `tests` module (near the `grid` helper), then the test above.

- [ ] **Step 9: Run the full existing `pathfinding.rs` test suite to verify no regression**

Run: `cargo test -p shadowcat --lib scene::pathfinding`
Expected: all PASS, including the just-updated `footprint_cell_outside_mask_is_not_enterable`, the
new `large_footprint_diagonal_step_with_flankers_in_mask_is_still_enterable`, and all
`astar_tests`/`find_tests` (all use `mask: None`, which skips the new check entirely).

- [ ] **Step 10: Run the full server suite + lints**

Run: `cargo test -p shadowcat && cargo fmt --check && cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: all green.

- [ ] **Step 11: Commit**

```bash
git add src/server/src/scene/pathfinding.rs
git commit -m "fix(m3): router mask predicate includes step supercover, closing buddy-check P1

The M10e-6 grid-A* router (cell_enterable) only checked the footprint disc at
the destination cell against the vision mask, missing the diagonal
corner-flanker cells the M1 move executor's supercover_cells check requires.
For sub-0.5-cell footprints this let the router approve a diagonal step the
executor then rejected. Union movement::supercover_cells(from, to, cell) into
the mask check so the router's predicate is a superset of the gate's by
construction (route ⊆ gate-allowed, spec §13/§3).

Also fixes footprint_cell_outside_mask_is_not_enterable's point-footprint case
(the union correctly requires the mover's own FROM cell in the mask too, which
the old fixture didn't model) and adds a large-footprint-diagonal test proving
the union introduces no new false rejection when the footprint already covers
the flankers."
```

---

### Task 2: Router region-arrest hook stub

**Files:**
- Modify: `src/server/src/scene/pathfinding.rs` (add stub function + call site in `cell_enterable`)
- Test: `src/server/src/scene/pathfinding.rs` (`mod tests`)

**Interfaces:**
- Consumes: nothing new (no ECS, no region data model — none exists yet).
- Produces: `fn region_arrests(to: Cell) -> bool` (private to `pathfinding.rs`; mirrors the shape,
  not the crate-visibility, of `move_exec.rs::region_arrests(ecs, scene, cell) -> bool` — this one
  takes no ECS handle since `pathfinding.rs` is pure/headless).

- [ ] **Step 1: Write the failing test asserting the stub is inert**

Add to `mod tests`, after the test added in Task 1 Step 3:

```rust
    #[test]
    fn region_arrest_stub_is_inert_and_does_not_block_an_otherwise_open_step() {
        // The router-side region-arrest hook (mirrors move_exec.rs::region_arrests) must be a
        // true no-op today — the region system doesn't exist until M10g. This guards against an
        // accidental future default flip: if `region_arrests` ever returns `true` unconditionally,
        // this test fails immediately instead of silently breaking every open-grid path.
        assert!(
            !region_arrests((3, 3)),
            "region-arrest hook must stay an inert stub until M10g provides real region data"
        );
        let walls: Vec<Seg> = vec![];
        let g = grid(&walls, None, 0.2);
        assert!(
            cell_enterable(&g, (0, 0), (1, 0)),
            "an otherwise-open step must remain enterable with the inert region hook in place"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p shadowcat region_arrest_stub_is_inert_and_does_not_block_an_otherwise_open_step`
Expected: FAIL with a compile error — `region_arrests` is not yet defined.

- [ ] **Step 3: Implement the stub + call site**

Add this function in `src/server/src/scene/pathfinding.rs`, directly above `cell_enterable` (i.e.,
right after the `footprint_cells` function, before the `cell_enterable` doc comment):

```rust
/// Region-arrest hook stub (mirrors `move_exec.rs::region_arrests`). Returns `true` when a region
/// halts entry into `to`. Currently always `false`; the region system (M10g) replaces this body.
/// Pure/headless: unlike `move_exec.rs`'s version, this takes no ECS handle — this module borrows
/// no ECS and owns no I/O (module invariant).
fn region_arrests(_to: Cell) -> bool {
    false
}
```

In `cell_enterable`, add a fourth check right before the final `true` (after the wall-crossing
loop from Task 1's step (3), i.e. at the very end of the function body):

```rust
    // (4) Region-arrest hook (M3/M10g stub) — mirrors move_exec.rs::region_arrests. Always false
    // today; M10g wires real region data into both this stub and move_exec's.
    if region_arrests(to) {
        return false;
    }

    true
```

(This replaces the bare trailing `true` that Task 1's rewritten function ends with.)

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p shadowcat region_arrest_stub_is_inert_and_does_not_block_an_otherwise_open_step`
Expected: PASS.

- [ ] **Step 5: Run the full `pathfinding.rs` test suite to verify no regression**

Run: `cargo test -p shadowcat --lib scene::pathfinding`
Expected: all PASS (the new check 4 is unconditionally a no-op, so every prior test's outcome is
unchanged).

- [ ] **Step 6: Run the full server suite + lints**

Run: `cargo test -p shadowcat && cargo fmt --check && cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add src/server/src/scene/pathfinding.rs
git commit -m "feat(m3): inert router-side region-arrest hook, mirrors move_exec.rs

pathfinding.rs's cell_enterable had no region awareness at all, so the router
could route through a cell that will arrest movement once M10g lands real
regions. Add a same-shaped stub (always false today, no ECS handle since this
module is pure/headless) so M10g wires one hook shape into both the gate and
the router instead of discovering the router needs one later."
```

---

## Self-Review Notes

- **Spec coverage:** §3 (mask parity) → Task 1. §4 (region hook) → Task 2. §5 (testing: P1
  regression, no large-footprint regression, degenerate fail-closed, stub-inertness) → covered by
  five new tests (the flanker regression, the degenerate-fail-closed case, the corrected
  pre-existing fixture, the large-footprint-diagonal strengthening test, and the stub-inertness
  test) plus Step 9/Step 5's full-suite reruns of every pre-existing test. §6 (out of scope) — no
  task touches `move_exec.rs`, `ws/room.rs`, or the region system. §7 (completion/doc-sync) is
  intentionally not a task — see the note below.
- **Placeholder scan:** none — every step has complete code, exact file paths, exact commands and
  expected output.
- **Type consistency:** `Cell = (i32, i32)` (existing alias, unchanged) used consistently;
  `region_arrests(to: Cell) -> bool` and `cell_enterable(grid: &PathGrid, from: Cell, to: Cell) -> bool`
  signatures match their call sites exactly; `movement::supercover_cells` signature matches its
  existing definition in `movement.rs` (verified against source, not re-derived).
- **Buddy-check pass (plan-level, `PHASE = spec`):** two independent reviewers (shadowcat-spec-
  reviewer, shadowcat-code-reviewer) fully converged after one debate round. Agreed/Important: the
  original Step 7 wrongly claimed `footprint_cell_outside_mask_is_not_enterable` would pass
  unchanged — fixed by the new Task 1 Step 7 (fixture update) above. Agreed/Minor (×3): the
  gate-has-no-footprint-term fact is now stated explicitly (Architecture section); the import
  instruction now says only one line is net-new (Step 5); a large-footprint-diagonal test was added
  (Step 8) to close a real coverage gap in the original testing plan. No unresolved disagreements.

## Post-implementation (not a task — handled by the SDD final-review / doc-sync gate)

Once both tasks are implemented and reviewed clean:
- Update `docs/PLAN.md`'s M10e status block: mark M3 done (per spec §7 — "e-1 through e-6 + M1/M2/M3
  all DONE"), noting the commit range on `m10e-5-movement-animation`.
- Update `docs/superpowers/specs/2026-06-25-server-authoritative-movement-design.md` §7's table row
  for M3 to done.
- Reviewed skill-update gate (project `CLAUDE.md`): check whether
  `shadowcat-codebase-scene-rendering` needs a line added/amended for the router/gate mask-parity
  invariant — dispatch `shadowcat-spec-reviewer` to confirm the skill diff (or the explicit "no
  update needed" statement) before merge/clear.
- This is the last checkpoint of the M10e-5 server-authoritative-movement redirect; M10f
  (continuous/Polyanya pathfinding) and M10g (regions) remain, unstarted.

## Buddy-check directives

- Plan buddy check: done 2026-07-01 (shadowcat-spec-reviewer + shadowcat-code-reviewer, PHASE=spec,
  plan vs. `2026-07-01-m3-vision-gated-pathfinder-design.md`). Converged after one debate round: 1
  Agreed/Important (Task 1's original Step 7 wrongly claimed a pre-existing test would pass
  unchanged) + 3 Agreed/Minor (gate-has-no-footprint-term now stated explicitly; import-instruction
  clarity; added large-footprint-diagonal test). No unresolved disagreements. All four findings
  folded into the plan above before execution.
- Flagged tasks: 1 — this task touches a security/secrecy-relevant algorithmic core (the vision-mask
  gate that is the sole thing preventing a route from leaking or admitting movement through unseen
  geometry) and a subtle spatial-data-structure predicate (grid supercover parity); buddy check
  pre-authorized to replace both single-reviewer stages at Task 1's review step. Task 2 is an inert
  stub with no behavioral effect — not flagged, normal single spec+code review.
- Unflagged tasks showing risk signals during execution: ask.
