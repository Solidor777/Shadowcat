# Server-Side Hex-Grid Movement Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the server the same hex-grid movement authority it already has for square grids — wall-blocking, vision-mask gating (secrecy + movement), region gating, and A* pathfinding — by generalizing the existing square-only cell-geometry code behind a `GridShape` abstraction, proving byte-identical square behavior first, then adding a `HexGrid` implementation.

**Architecture:** A new `scene/grid_shape.rs` module defines a `GridShape` trait covering cell-center, point-to-cell, neighbor+cost enumeration, line-traversal ("supercover" equivalent), and footprint-overlap. `SquareGrid` ports today's hardcoded square math into this trait, byte-identical, proven via a frozen-fixture parity battery before any call site cuts over. `movement.rs`, `pathfinding.rs`, `scene/mod.rs`'s visibility-mask cell iteration, and `regions.rs`'s rasterization are then refactored to call through the trait instead of hardcoded square formulas. Only once that refactor is proven equivalent does `HexGrid` (pointy-top axial, mirroring the client's `grid.ts` exactly) get built and wired in.

**Tech Stack:** Rust (server), no new dependencies. Existing test conventions: `#[cfg(test)] mod tests` per module, `cargo test --manifest-path src/server/Cargo.toml`.

## Global Constraints

- No new crate dependencies (clean-room geometry per ARCHITECTURE §7, matching this codebase's existing convention for `movement.rs`/`vision.rs`/`pathfinding.rs`).
- The router's mask predicate must remain a superset of the move-executor's gate (existing load-bearing invariant — do not let hex diverge from this).
- Every degenerate/malformed input (non-finite coords, zero/negative cell size, unrecognized `grid.kind`) must fail closed to the already-hardened square behavior, never silently produce a wrong or partial result.
- `SquareGrid`'s behavior must be byte-identical to today's hardcoded square code before any call site cuts over (frozen-fixture parity gate, Task 8) — this is the regression-safety mechanism the whole plan depends on.
- Full server gate after every task: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`.

## Model/Effort directives

Written mainline in this session (Sonnet 5), per explicit user choice at the writing-plans tier-switch checkpoint — no dedicated plan-writer model switch.

**Execution dispatcher:** this mainline session owns the subagent-driven-development loop directly (not a delegated `sdd-dispatcher` subagent), per explicit user choice at the SDD dispatcher-tier checkpoint. Implementer/reviewer subagents still run at their own configured tiers (see task-by-task dispatch decisions below); this session's own reasoning effort is kept low for routing/bookkeeping between dispatches.

## Buddy-check directives

This plan touches the movement-gate/vision-mask secrecy boundary (`scene/mod.rs`'s `accumulate_visible_cells`, which feeds both `player_lit_mask`'s egress and `visible_cells`/`visible_cells_cached`'s movement gate) and the wall/region movement-authority code (`movement.rs`, `pathfinding.rs`). Per this project's established pattern (Tasks 4/5/9/11/12/43 of the phase1-cleanup-burndown plan all required a mandatory security buddy-check for comparable or smaller changes to this same code family), **Tasks 5, 6, and 8 require a mandatory two-reviewer security buddy-check** before being marked complete — Task 5 (visibility-mask cell iteration refactor), Task 6 (regions refactor touching the impassable-gate), and Task 8 (the frozen-fixture parity gate itself, since a false-positive "parity proven" would let a real behavior change through undetected). Escalate to the opus-tier reviewer twins (`shadowcat-code-reviewer-opus`/`shadowcat-spec-reviewer-opus`) for these three, matching this codebase's precedent for security-adjacent engine-geometry changes.

---

## Task 1: `GridShape` trait + `SquareGrid` (parity-proven port, not yet wired anywhere)

**Files:**
- Create: `src/server/src/scene/grid_shape.rs`
- Modify: `src/server/src/scene/mod.rs:1` area (add `pub(crate) mod grid_shape;` to the module's `mod` declarations — find the existing `mod movement;`/`mod pathfinding;`/`mod regions;` block and add alongside)
- Test: same file, `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `pub(crate) trait GridShape { fn cell_center(&self, c: Cell) -> vision::P; fn cell_of(&self, p: vision::P) -> Cell; fn neighbors_with_cost(&self, c: Cell, parity: u8) -> Vec<(Cell, f64, u8)>; fn line_traversal(&self, a: vision::P, b: vision::P, cell: f64) -> Option<BTreeSet<Cell>>; fn footprint_cells(&self, anchor: Cell, ctr: vision::P, r_scene: f64, cell: f64) -> Vec<Cell>; }` and `pub(crate) struct SquareGrid { pub cell: f64, pub rule: crate::scene::pathfinding::DiagonalRule }` implementing it. `Cell = (i32, i32)` re-exported from `crate::scene::pathfinding::Cell` (already `pub type Cell = (i32, i32);` — do not redefine, import it).
- Consumes: `crate::scene::vision::P` (existing `(f64, f64)` point type), `crate::scene::movement::supercover_cells` (reused verbatim inside `SquareGrid::line_traversal`, not reimplemented), `crate::scene::pathfinding::{footprint_cells as square_footprint_cells, DiagonalRule}` (existing free functions, reused verbatim inside `SquareGrid`'s methods).

`neighbors_with_cost`'s `parity: u8` in/out threading exists ONLY because square's `Alternating` diagonal rule needs a parity bit (per-node search-state, per `pathfinding.rs`'s existing `QNode`/`step_cost` design) — `HexGrid` (Task 10) will always return the same `parity` it was given (hex has no rule-dependent parity concept), keeping the trait signature uniform across both shapes.

- [ ] **Step 1: Write the failing test**

```rust
// src/server/src/scene/grid_shape.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::pathfinding::DiagonalRule;

    #[test]
    fn square_grid_cell_center_matches_pathfinding_cell_center() {
        let g = SquareGrid { cell: 100.0, rule: DiagonalRule::Chebyshev };
        assert_eq!(g.cell_center((2, 3)), crate::scene::pathfinding::cell_center((2, 3), 100.0));
    }

    #[test]
    fn square_grid_cell_of_floors_to_cell_index() {
        let g = SquareGrid { cell: 100.0, rule: DiagonalRule::Chebyshev };
        assert_eq!(g.cell_of((250.0, 149.0)), (2, 1));
        assert_eq!(g.cell_of((-10.0, -1.0)), (-1, -1));
    }

    #[test]
    fn square_grid_neighbors_with_cost_matches_chebyshev_dirs_and_step_cost() {
        let g = SquareGrid { cell: 100.0, rule: DiagonalRule::Chebyshev };
        let ns = g.neighbors_with_cost((0, 0), 0);
        assert_eq!(ns.len(), 8, "8-directional king-move expansion");
        // A diagonal neighbor costs 1.0 under Chebyshev.
        let diag = ns.iter().find(|(c, _, _)| *c == (1, 1)).expect("diagonal neighbor present");
        assert!((diag.1 - 1.0).abs() < 1e-9);
        // An orthogonal neighbor costs 1.0 too.
        let ortho = ns.iter().find(|(c, _, _)| *c == (1, 0)).expect("orthogonal neighbor present");
        assert!((ortho.1 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn square_grid_neighbors_with_cost_alternating_threads_parity() {
        let g = SquareGrid { cell: 100.0, rule: DiagonalRule::Alternating };
        let ns = g.neighbors_with_cost((0, 0), 0);
        let diag = ns.iter().find(|(c, _, _)| *c == (1, 1)).unwrap();
        assert!((diag.1 - 1.0).abs() < 1e-9, "first diagonal from parity 0 costs 1");
        assert_eq!(diag.2, 1, "parity flips after a diagonal step");
        let ortho = ns.iter().find(|(c, _, _)| *c == (1, 0)).unwrap();
        assert_eq!(ortho.2, 0, "parity unchanged after an orthogonal step");
    }

    #[test]
    fn square_grid_line_traversal_matches_supercover_cells() {
        let g = SquareGrid { cell: 100.0, rule: DiagonalRule::Chebyshev };
        let a = (50.0, 50.0);
        let b = (250.0, 250.0);
        assert_eq!(
            g.line_traversal(a, b, 100.0),
            crate::scene::movement::supercover_cells(a, b, 100.0)
        );
    }

    #[test]
    fn square_grid_footprint_cells_matches_pathfinding_footprint_cells() {
        let g = SquareGrid { cell: 100.0, rule: DiagonalRule::Chebyshev };
        let anchor = (2, 2);
        let ctr = (250.0, 250.0);
        let mut got = g.footprint_cells(anchor, ctr, 60.0, 100.0);
        let mut want = crate::scene::pathfinding::footprint_cells(anchor, ctr, 60.0, 100.0);
        got.sort();
        want.sort();
        assert_eq!(got, want);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src/server/Cargo.toml grid_shape -- --nocapture`
Expected: FAIL with a compile error — `grid_shape` module and `GridShape`/`SquareGrid` don't exist yet, and `crate::scene::pathfinding::cell_center`/`footprint_cells` are not `pub(crate)` yet (they're currently `pub fn cell_center` and `pub(crate) fn footprint_cells` respectively — confirm each's exact visibility in `pathfinding.rs` and adjust only if needed; do not widen visibility beyond `pub(crate)`).

- [ ] **Step 3: Implement `GridShape` + `SquareGrid`**

```rust
// src/server/src/scene/grid_shape.rs
//! `GridShape` abstracts the per-cell geometry every movement/vision/pathfinding module needs,
//! so square and hex scenes share one code path instead of two. `SquareGrid` is a byte-identical
//! port of the pre-existing hardcoded square math (proven via the Task 8 frozen-fixture parity
//! battery before any call site is cut over); `HexGrid` (Task 10) is the new pointy-top axial
//! implementation mirroring the client's `grid.ts` exactly.

use crate::scene::pathfinding::{self, Cell, DiagonalRule};
use crate::scene::vision;
use std::collections::BTreeSet;

/// Per-scene cell geometry: cell-center point, point-to-cell mapping, neighbor+cost enumeration,
/// line-traversal ("supercover"), and footprint-vs-cell overlap. One trait, two implementations
/// (`SquareGrid`, `HexGrid`) — every caller (movement gate, A* router, visibility mask, region
/// rasterization) works against this interface, never against square/hex math directly.
pub(crate) trait GridShape {
    /// Center of cell `c` in scene coordinates.
    fn cell_center(&self, c: Cell) -> vision::P;
    /// The cell containing scene point `p`.
    fn cell_of(&self, p: vision::P) -> Cell;
    /// Every reachable neighbor of `c` under diagonal-parity `parity`, as `(neighbor, step_cost,
    /// next_parity)`. `parity` exists only for square's `Alternating` rule (PF1e/3.5 5-10-5);
    /// `HexGrid` always returns the same `parity` it was given (no rule-dependent state).
    fn neighbors_with_cost(&self, c: Cell, parity: u8) -> Vec<(Cell, f64, u8)>;
    /// Every cell the segment `a -> b` crosses (supercover, not a thin line) at the given cell
    /// size. `None` on a degenerate/over-cap span — callers fail closed on `None`.
    fn line_traversal(&self, a: vision::P, b: vision::P, cell: f64) -> Option<BTreeSet<Cell>>;
    /// Cells whose geometry the footprint disc (center `ctr`, radius `r_scene`) overlaps. The
    /// anchor cell is always included (mirrors the square implementation's zero-radius guarantee).
    fn footprint_cells(&self, anchor: Cell, ctr: vision::P, r_scene: f64, cell: f64) -> Vec<Cell>;
}

/// Byte-identical port of the pre-existing hardcoded square-grid math (`pathfinding.rs`'s
/// `cell_center`/`footprint_cells`, `movement.rs`'s `supercover_cells`, `pathfinding.rs`'s
/// `astar_leg`'s 8-directional `dirs` + `step_cost`). `cell` and `rule` are the scene's resolved
/// cell size and diagonal-cost rule.
pub(crate) struct SquareGrid {
    pub cell: f64,
    pub rule: DiagonalRule,
}

impl GridShape for SquareGrid {
    fn cell_center(&self, c: Cell) -> vision::P {
        pathfinding::cell_center(c, self.cell)
    }

    fn cell_of(&self, p: vision::P) -> Cell {
        ((p.0 / self.cell).floor() as i32, (p.1 / self.cell).floor() as i32)
    }

    fn neighbors_with_cost(&self, c: Cell, parity: u8) -> Vec<(Cell, f64, u8)> {
        const DIRS: [(i32, i32); 8] =
            [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];
        DIRS.iter()
            .map(|&(di, dj)| {
                let next = (c.0 + di, c.1 + dj);
                let (cost, next_parity) = step_cost(self.rule, di, dj, parity);
                (next, cost, next_parity)
            })
            .collect()
    }

    fn line_traversal(&self, a: vision::P, b: vision::P, cell: f64) -> Option<BTreeSet<Cell>> {
        crate::scene::movement::supercover_cells(a, b, cell)
    }

    fn footprint_cells(&self, anchor: Cell, ctr: vision::P, r_scene: f64, cell: f64) -> Vec<Cell> {
        pathfinding::footprint_cells(anchor, ctr, r_scene, cell)
    }
}

/// Ported verbatim from `pathfinding.rs`'s private `step_cost` (Task 3 will delete the original
/// once `SquareGrid` is proven equivalent, per this plan's parity-then-cutover sequencing).
fn step_cost(rule: DiagonalRule, di: i32, dj: i32, parity: u8) -> (f64, u8) {
    let diagonal = di != 0 && dj != 0;
    if !diagonal {
        return (1.0, parity);
    }
    match rule {
        DiagonalRule::Chebyshev => (1.0, parity),
        DiagonalRule::Manhattan => (2.0, parity),
        DiagonalRule::Euclidean => (std::f64::consts::SQRT_2, parity),
        DiagonalRule::Alternating => {
            let cost = if parity == 0 { 1.0 } else { 2.0 };
            (cost, 1 - parity)
        }
    }
}
```

Add `pub(crate) mod grid_shape;` to `src/server/src/scene/mod.rs`'s module-declaration block (alongside the existing `mod movement;` etc — find the exact line via `grep -n "^mod \|^pub(crate) mod " src/server/src/scene/mod.rs` and insert alphabetically or adjacent to the related modules).

Confirm `pathfinding::cell_center` and `pathfinding::footprint_cells` are reachable as `crate::scene::pathfinding::{cell_center, footprint_cells}` from `grid_shape.rs` — both are currently declared in `pathfinding.rs` (`cell_center` as `pub fn`, `footprint_cells` as `pub(crate) fn`); no visibility change needed since `grid_shape.rs` lives in the same crate.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src/server/Cargo.toml grid_shape -- --nocapture`
Expected: PASS, all 5 tests.

- [ ] **Step 5: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/grid_shape.rs src/server/src/scene/mod.rs
git commit -m "feat(server/scene): GridShape trait + SquareGrid (not yet wired)

Byte-identical port of the existing hardcoded square-grid cell math
behind a new abstraction. Nothing calls through it yet — this task
only proves the port is correct against the existing square functions
it wraps."
```

---

## Task 2: Wire `pathfinding.rs`'s A* search through `GridShape` (square-only, parity-proven)

**Files:**
- Modify: `src/server/src/scene/pathfinding.rs` (`PathGrid` struct, `astar_leg`'s neighbor-expansion loop, `find`'s `PathGrid` construction)
- Test: `src/server/src/scene/pathfinding.rs`'s existing `astar_tests` module (must all still pass unmodified — this task changes internals, not behavior)

**Interfaces:**
- Consumes: `crate::scene::grid_shape::{GridShape, SquareGrid}` (Task 1).
- Produces: `PathGrid` gains a `shape: &'a dyn GridShape` field; `astar_leg` and `cell_enterable` read cell geometry through `grid.shape` instead of the free `cell_center`/`footprint_cells` functions and the hardcoded `dirs`/`step_cost` loop.

- [ ] **Step 1: Write the failing test**

Add to `pathfinding.rs`'s existing `astar_tests` module (these are NEW tests proving the `GridShape`-routed search still produces the exact costs/parity the pre-existing tests already assert — the pre-existing tests themselves are the primary parity proof and must not be deleted or weakened):

```rust
#[test]
fn astar_leg_routes_through_grid_shape_not_hardcoded_dirs() {
    // Same assertions as `chebyshev_diagonal_is_cost_one_per_step`, but this test exists
    // specifically to fail if a future edit reintroduces a hardcoded `dirs`/`step_cost` path
    // instead of going through `grid.shape.neighbors_with_cost(...)`.
    let g = open(DiagonalRule::Chebyshev, 0.1);
    let (cells, cost, _p) = astar_leg(&g, (0, 0), (3, 3), 0).unwrap();
    assert!((cost - 3.0).abs() < 1e-9);
    assert_eq!(cells.first(), Some(&(0, 0)));
    assert_eq!(cells.last(), Some(&(3, 3)));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src/server/Cargo.toml pathfinding -- --nocapture`
Expected: this specific new test will actually PASS immediately if `PathGrid` isn't changed yet (it's checking the same public behavior `chebyshev_diagonal_is_cost_one_per_step` already checks) — that's fine, it's a pinning test for Step 3's refactor, not a red/green TDD gate on its own. Proceed to Step 3 regardless.

- [ ] **Step 3: Refactor `PathGrid`/`astar_leg`/`cell_enterable` to route through `GridShape`**

In `pathfinding.rs`, change `PathGrid`'s struct definition (currently at line ~30-38) to add a `shape` field:

```rust
pub struct PathGrid<'a> {
    pub cell: f64,
    pub rule: DiagonalRule,
    pub footprint_radius_cells: f64,
    pub walls: &'a [vision::Seg],
    pub mask: Option<&'a BTreeSet<Cell>>,
    pub regions: Option<&'a crate::scene::regions::RegionField>,
    pub window: (i32, i32, i32, i32),
    /// Cell geometry for this scene — `SquareGrid` today; `HexGrid` once Task 10 lands. Owns no
    /// state beyond `cell`/`rule`, constructed fresh per search in `find()`.
    pub shape: &'a dyn crate::scene::grid_shape::GridShape,
}
```

In `cell_enterable` (currently ~line 80-141), replace every call to the free `cell_center(to, grid.cell)`/`cell_center(from, grid.cell)` with `grid.shape.cell_center(to)`/`grid.shape.cell_center(from)`, and replace the `footprint_cells(to, ctr, r_scene, grid.cell)` calls (there are two — the mask check and the region-impassable check) with `grid.shape.footprint_cells(to, ctr, r_scene, grid.cell)`. Replace the mask check's `movement::supercover_cells(a, ctr, grid.cell)` call with `grid.shape.line_traversal(a, ctr, grid.cell)`.

In `astar_leg` (currently ~line 329-413), replace the hardcoded `dirs` array + inner `for (di, dj) in dirs { let next = (cell.0+di, cell.1+dj); ... let (sc, next_parity) = step_cost(grid.rule, di, dj, parity); ...}` loop with:

```rust
for (next, sc, next_parity) in grid.shape.neighbors_with_cost(cell, parity) {
    if !cell_enterable(grid, cell, next) {
        continue;
    }
    let mult = grid.regions.map_or(1.0, |r| r.terrain_multiplier(next));
    let tentative = g_popped + sc * mult;
    let key = (next, next_parity);
    if tentative < *g_score.get(&key).unwrap_or(&f64::INFINITY) {
        came_from.insert(key, (cell, parity));
        g_score.insert(key, tentative);
        open.push(QNode {
            f: tentative + heuristic(grid.rule, next, goal),
            g: tentative,
            cell: next,
            parity: next_parity,
        });
    }
}
```

Delete the now-unused private `step_cost` function from `pathfinding.rs` (it's ported into `grid_shape.rs`'s `SquareGrid` in Task 1 — confirm no other call site in `pathfinding.rs` still references the local `step_cost` before deleting; `heuristic` stays, it's unaffected).

In `find()` (currently ~line 441-500+), construct the `SquareGrid` and pass it into the `PathGrid` literal:

```rust
let shape = crate::scene::grid_shape::SquareGrid { cell, rule };
let grid = PathGrid {
    cell,
    rule,
    footprint_radius_cells: footprint_radius,
    walls,
    mask,
    regions,
    window,
    shape: &shape,
};
```

Update every test-fixture `PathGrid { ... }` literal in `astar_tests` (the `open()` helper and `walled_off_goal_is_unreachable`'s inline literal) to also construct a `SquareGrid` and set `shape: &shape` — since `PathGrid` gains a required field, every existing literal must be updated or the crate fails to compile. This is expected: it does NOT change what any existing test asserts, only how the fixture is built.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src/server/Cargo.toml pathfinding -- --nocapture`
Expected: PASS — every pre-existing `astar_tests` test (chebyshev/manhattan/euclidean/alternating/walled-off/start-equals-goal) passes UNCHANGED, plus the new pinning test.

- [ ] **Step 5: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/pathfinding.rs
git commit -m "refactor(server/scene): route pathfinding's A* search through GridShape

PathGrid gains a shape field; cell_enterable and astar_leg's neighbor
expansion now call through GridShape instead of hardcoded square
cell-center/footprint/dirs/step_cost math. All existing astar_tests
pass unchanged — SquareGrid is behaviorally identical to the code it
replaces."
```

---

## Task 3: Wire `movement.rs`'s `supercover_cells` call sites through `GridShape` (square-only, parity-proven)

**Files:**
- Modify: `src/server/src/scene/move_exec.rs` (`gate_walk`'s call to `movement::supercover_cells`, and `execute_move`'s call, per the module doc-comment's description of both call sites)
- Modify: `src/server/src/scene/mod.rs` (any direct `movement::supercover_cells` call sites outside `move_exec.rs` — grep to confirm the full set)
- Test: existing `move_exec.rs` test suite (must pass unchanged)

**Interfaces:**
- Consumes: `crate::scene::grid_shape::{GridShape, SquareGrid}` (Task 1).

- [ ] **Step 1: Locate every `supercover_cells` call site**

Run: `grep -rn "supercover_cells" src/server/src/scene/`

Expected results: the definition in `movement.rs`, the `SquareGrid::line_traversal` wrapper added in Task 1 (`grid_shape.rs`), the `pathfinding.rs` call site already migrated to `grid.shape.line_traversal(...)` in Task 2, and one or more direct call sites in `move_exec.rs`'s `gate_walk` (per the module's own doc comment: "the mask gate — (1) wall gate ... (2) vision-mask gate (`supercover_cells` + `visible` membership...)").

- [ ] **Step 2: Write the failing test**

`move_exec.rs` already has extensive existing coverage for `gate_walk`'s per-cell-entry mask gating (per the module's doc comment describing frozen fixture parity tests from the M10f-2 refactor). Add one new pinning test asserting the gate still produces identical results when routed through `GridShape`:

```rust
// In move_exec.rs's existing #[cfg(test)] mod tests, alongside the other gate_walk tests:
#[test]
fn gate_walk_mask_gate_routes_through_grid_shape_not_hardcoded_supercover() {
    // Reuses this module's existing test scaffolding for a simple diagonal move across 2 cells,
    // one of which is OUTSIDE the mask — confirms the gate still rejects entry at that cell after
    // the supercover_cells call site is routed through GridShape::line_traversal.
    // (Match the exact fixture-construction helper this file's other gate_walk tests already use
    // — read the surrounding tests first to reuse the established pattern rather than inventing
    // a new one.)
}
```

- [ ] **Step 3: Run test to verify it fails or passes as a pin**

Run: `cargo test --manifest-path src/server/Cargo.toml move_exec -- --nocapture`
Expected: the new test passes immediately (pinning today's behavior, same caveat as Task 2 Step 2) — proceed to Step 4 regardless.

- [ ] **Step 4: Refactor the call site(s)**

In `move_exec.rs`, wherever `movement::supercover_cells(a, b, cell)` is called directly (not through `pathfinding.rs`, which Task 2 already migrated), construct a `SquareGrid { cell, rule }` — using whatever `DiagonalRule` value is already in scope at that call site (per `move_exec.rs`'s existing signature, it likely already threads a resolved rule or can default to `DiagonalRule::Chebyshev` if the call site's mask-gate use of `supercover_cells` doesn't itself depend on the rule — confirm which by reading the call site's surrounding context before deciding; `line_traversal`'s SEGMENT-CROSSING result does not depend on `rule` at all, since `rule` only affects A* step COST, not which cells a segment crosses, so any valid `DiagonalRule` value produces an identical `line_traversal` result — prefer threading the scene's actual resolved rule for clarity even though it's inert here) — and call `.line_traversal(a, b, cell)` instead of the free function.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --manifest-path src/server/Cargo.toml move_exec -- --nocapture`
Expected: PASS, all existing + new tests, unchanged assertions.

- [ ] **Step 6: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`

- [ ] **Step 7: Commit**

```bash
git add src/server/src/scene/move_exec.rs
git commit -m "refactor(server/scene): route move_exec's mask gate through GridShape

gate_walk's supercover_cells call now goes through
SquareGrid::line_traversal instead of the free function directly.
Behavior unchanged — DiagonalRule doesn't affect which cells a
segment crosses, only A* step cost."
```

---

## Task 4: Wire `regions.rs`'s `rasterize` through `GridShape` (square-only, parity-proven)

**Files:**
- Modify: `src/server/src/scene/regions.rs` (`rasterize`'s cell-candidate enumeration, currently a square AABB scan over `i0f..=i1f` / `j0f..=j1f`)
- Test: `src/server/src/scene/regions.rs`'s existing `rasterize` test suite (must pass unchanged)

**Interfaces:**
- Consumes: `crate::scene::grid_shape::{GridShape, SquareGrid}` (Task 1).
- Produces: `rasterize(shape: &RegionShape, cell: f64, grid: &dyn GridShape) -> Option<Vec<Cell>>` — note the parameter name collision (`RegionShape` = the region's own authored geometry, unrelated to the new `GridShape` trait; keep both names as-is, they're in different modules and this is the existing naming in `regions.rs` — do not rename `RegionShape` to avoid confusion, just be precise in the plan/commit about which is which).

- [ ] **Step 1: Read `rasterize`'s full current body**

Read `src/server/src/scene/regions.rs` in full (it's short, ~150-200 lines) to see the AABB-scan loop after the bounds-computation logic already shown in this plan's context-gathering (the `i0f`/`i1f`/`j0f`/`j1f` computation and `MAX_CELL_COORD` fail-closed check). The loop after that point iterates `(i, j)` and tests each candidate cell's CENTER against the shape (per the function's doc comment: "Rasterize `shape` to the grid cells whose CENTER falls inside it").

- [ ] **Step 2: Write the failing test**

Add to `regions.rs`'s existing test module:

```rust
#[test]
fn rasterize_routes_through_grid_shape_cell_center_not_hardcoded() {
    use crate::scene::grid_shape::SquareGrid;
    use crate::scene::pathfinding::DiagonalRule;
    let shape = RegionShape::Rect { x0: 0.0, y0: 0.0, x1: 250.0, y1: 250.0 };
    let grid_shape = SquareGrid { cell: 100.0, rule: DiagonalRule::Chebyshev };
    let cells = rasterize(&shape, 100.0, &grid_shape).unwrap();
    // A 250x250 rect at origin covers cell-centers (50,50),(150,50),(50,150),(150,150) at
    // cell=100 (cells (0,0),(1,0),(0,1),(1,1)) — (250,250) itself is the far corner, whose
    // containing cell's center (250,250 falls in cell (2,2), center (250,250)) is exactly on
    // the rect boundary; confirm against this test's actual computed value once run rather than
    // asserting a guessed exact set — the load-bearing assertion is `cells.len() >= 4` and that
    // (0,0)/(1,0)/(0,1)/(1,1) are all present.
    assert!(cells.contains(&(0, 0)));
    assert!(cells.contains(&(1, 0)));
    assert!(cells.contains(&(0, 1)));
    assert!(cells.contains(&(1, 1)));
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --manifest-path src/server/Cargo.toml regions -- --nocapture`
Expected: FAIL — compile error, `rasterize` doesn't yet take a `grid: &dyn GridShape` parameter.

- [ ] **Step 4: Refactor `rasterize`**

Add a `grid: &dyn crate::scene::grid_shape::GridShape` parameter to `rasterize`'s signature. Replace the final cell-iteration loop's direct `(i as f64 + 0.5) * cell, (j as f64 + 0.5) * cell)`-style center computation (if present — confirm the exact form by reading Step 1's output) with `grid.cell_center((i, j))`, and replace the shape-containment test's cell-center input accordingly. Update `rasterize`'s two call sites (`SceneEcs::region_field`'s builder, per the module doc comment — grep `regions::rasterize\(` across `src/server/src/scene/` to find both) to pass a constructed `SquareGrid` (or, once Task 10 lands, the scene's resolved `GridShape`).

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --manifest-path src/server/Cargo.toml regions -- --nocapture`
Expected: PASS, all existing + new tests.

- [ ] **Step 6: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`

- [ ] **Step 7: Commit**

```bash
git add src/server/src/scene/regions.rs
git commit -m "refactor(server/scene): route rasterize's cell-center test through GridShape

Region rasterization's cell-candidate center computation now goes
through GridShape instead of hardcoded square math. Behavior
unchanged for square scenes."
```

---

## Task 5: Wire `scene/mod.rs`'s `accumulate_visible_cells` through `GridShape` (square-only, parity-proven) `[sec]`

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`accumulate_visible_cells`'s cell-iteration loop, currently `for i in i0..=i1 { for j in j0..=j1 { ... } }` per the earlier context-gathering read of lines 1981-2050+)
- Test: `scene/mod.rs`'s existing `visible_cells`/`player_lit_mask`/`visible_cells_cached` test suite (must pass unchanged — this is a secrecy-boundary hot path, per the design doc's own framing)

**Interfaces:**
- Consumes: `crate::scene::grid_shape::{GridShape, SquareGrid}` (Task 1).
- Produces: `accumulate_visible_cells` gains a `grid: &dyn GridShape` parameter (or reads it from an already-in-scope `settings: &ResolvedScene`-derived value — confirm which by reading the function's full current signature and its 1-2 callers, `visible_cells` and `visible_cells_cached`, before deciding the exact threading point).

`[sec]`: this function feeds BOTH `player_lit_mask` (secrecy egress) and `visible_cells`/`visible_cells_cached` (the movement gate) — per this plan's Buddy-check directives, this task requires a mandatory two-reviewer security buddy-check before being marked complete.

- [ ] **Step 1: Read `accumulate_visible_cells`'s full current body and both call sites**

Read `src/server/src/scene/mod.rs`'s `accumulate_visible_cells` (found earlier at line ~1981) in full, plus its two callers `visible_cells` (line ~1665) and `visible_cells_cached` — confirm exactly how `cell: f64` currently reaches this function (a plain parameter) and what's available at each call site to also construct a `DiagonalRule`/`GridShape` (likely `settings: &ResolvedScene`, which per `resolved_diagonal_rule`'s existing pattern elsewhere in this file already resolves the world's diagonal rule — confirm the exact accessor name via `grep -n "resolved_diagonal_rule\|fn diagonal_rule" src/server/src/scene/mod.rs`).

- [ ] **Step 2: Write the failing test**

Add to `scene/mod.rs`'s existing visibility test module (find the existing test fixture helpers for `visible_cells`/`accumulate_visible_cells` first and reuse them):

```rust
#[test]
fn accumulate_visible_cells_routes_through_grid_shape_cell_center_not_hardcoded() {
    // Reuses this module's existing visibility test fixture (a simple open scene, one VisSrc at
    // a known viewpoint, no walls) to confirm the cell set returned by accumulate_visible_cells
    // is IDENTICAL before and after the GridShape refactor — this is a pinning test, the primary
    // parity proof is Task 8's frozen-fixture battery, but this test exists specifically so a
    // future regression to hardcoded square math in THIS function is caught immediately by its
    // own test suite, not only by the broader parity gate.
    // (Match the exact fixture-construction helper this file's other accumulate_visible_cells /
    // visible_cells tests already use — read them first before writing this test's body.)
}
```

- [ ] **Step 3: Run test to verify it fails or passes as a pin**

Run: `cargo test --manifest-path src/server/Cargo.toml accumulate_visible_cells -- --nocapture`
Expected: passes immediately as a pin (same caveat as Tasks 2/3's Step 2) — proceed to Step 4.

- [ ] **Step 4: Refactor `accumulate_visible_cells`'s cell-iteration loop**

Thread a `grid: &dyn GridShape` into `accumulate_visible_cells` (constructed by each caller from the scene's resolved cell size + diagonal rule via `SquareGrid`). Inside the function's per-source loop, replace the direct `let center = ((i as f64 + 0.5) * cell, (j as f64 + 0.5) * cell);` computation with `let center = grid.cell_center((i, j));`. The bounding-box computation (`i0`/`i1`/`j0`/`j1` from the polygon's min/max, plus the `MAX_CELLS_PER_POLYGON` DoS cap check) stays UNCHANGED — it's a coordinate-space AABB over the raycast polygon, not cell-shape-dependent, and changing it is explicitly out of scope for this task (hex's bounding-box-to-candidate-cells enumeration, if it ever needs a tighter hex-shaped candidate set instead of a square AABB super-set, is a Task 13 concern, not this one — an AABB over-approximation is always safe, just potentially less tight for hex).

Leave the `corners` array (used only under `lenient`) unchanged for now — hex's corner-sampling equivalent is deferred to Task 13 alongside `HexGrid`'s footprint-overlap work; `SquareGrid`'s corners stay the same 4-corner square computation since this task only proves square parity.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --manifest-path src/server/Cargo.toml --lib` (run the FULL lib suite, not a filtered subset — this function is load-bearing for both `player_lit_mask` and the movement gate, per the design doc's own framing; a filtered test run risks missing a downstream regression in either consumer)
Expected: PASS, every pre-existing test in `scene/mod.rs` unchanged, plus the new pinning test.

- [ ] **Step 6: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`

- [ ] **Step 7: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "refactor(server/scene): route accumulate_visible_cells through GridShape [sec]

The visibility-mask cell-center computation (feeds both
player_lit_mask's secrecy egress and the movement gate) now goes
through GridShape instead of hardcoded square math. Bounding-box scan
and DoS cap unchanged. Behavior unchanged for square scenes."
```

- [ ] **Step 8: Mandatory security buddy-check** — dispatch two independent reviewers (opus-tier per this plan's Buddy-check directives) to confirm: (a) the refactor is genuinely behavior-preserving for every existing square-scene test, not just the ones re-run in Step 5; (b) no new code path can produce a WIDER visibility/movement mask than before (fail-closed direction preserved); (c) `player_lit_mask` and `visible_cells`/`visible_cells_cached` both still derive from the identical `accumulate_visible_cells` call, no divergence introduced between the two consumers.

**Post-task finding (both security buddy-check reviewers independently confirmed; reviewer B raised it, reviewer A's own point 3 assumed it away and needs correction):** point (c) above is FALSE as originally stated. `player_lit_mask` does NOT call `accumulate_visible_cells` — it has its OWN separate hardcoded cell-center loop (`scene/mod.rs`, inside `player_lit_mask`, `let cx = (i as f64 + 0.5) * cell; let cy = (j as f64 + 0.5) * cell;`). Task 5 only migrated `visible_cells`/`visible_cells_cached`'s shared helper. For square scenes this is harmless (both formulas are byte-identical today), but it means `player_lit_mask` (secrecy egress) and `visible_cells` (movement gate) are TWO independent center-math sites, not one — a real gap the original Task 5 brief's framing missed. Added as Task 5b below, with the same rigor (opus-tier, mandatory buddy-check) since it's the identical secrecy-critical code class.

---

## Task 5b: Wire `player_lit_mask`'s own separate cell-center loop through `GridShape` `[sec]`

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`player_lit_mask`, the `let cx = (i as f64 + 0.5) * cell; let cy = (j as f64 + 0.5) * cell;` lines inside its per-scene cell-accumulation loop — read the full function, currently starting at `pub fn player_lit_mask(&self, user: Uuid) -> Vec<LitScene> {`, to find the exact loop and the `cell` local it resolves per-scene via a `grid.get(&scene)` lookup)
- Test: `scene/mod.rs`'s existing `player_lit_mask` test suite (must pass unchanged)

**Interfaces:** Same pattern as Task 5 — construct a `SquareGrid { cell, rule }` from the loop's already-resolved per-scene `cell` (and the scene's resolved diagonal rule, via whatever accessor Task 5's implementation used — mirror it exactly for consistency) and replace `(cx, cy) = ((i+0.5)*cell, (j+0.5)*cell)` with `grid.cell_center((i, j))`. Bounding-box scan and the `MAX_CELLS_PER_POLYGON`-equivalent cap in this function stay untouched, mirroring Task 5's scope discipline.

- [ ] **Step 1: Write the failing test**

Add a pinning test to `scene/mod.rs`'s `player_lit_mask` test module asserting the exact returned `LitScene` cell set for a fixed fixture (mirror Task 5's `accumulate_visible_cells_routes_through_grid_shape_cell_center_not_hardcoded` pattern — same style, applied to `player_lit_mask`'s output instead).

- [ ] **Step 2: Run test to verify it fails or passes as a pin**

Run: `cargo test --manifest-path src/server/Cargo.toml player_lit_mask -- --nocapture`
Expected: passes immediately as a pin (same caveat as this plan's other pinning-test steps) — proceed to Step 3 regardless.

- [ ] **Step 3: Refactor**

Replace the inline `cx`/`cy` computation with `grid.cell_center((i, j))`, threading a `SquareGrid` constructed from the loop's per-scene `cell` local.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src/server/Cargo.toml --lib` (full lib suite, same rationale as Task 5 — this is the OTHER secrecy-egress consumer)
Expected: PASS, every pre-existing `player_lit_mask` test unchanged.

- [ ] **Step 5: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "refactor(server/scene): route player_lit_mask's cell-center through GridShape [sec]

player_lit_mask had its own separate hardcoded cell-center loop,
independent of accumulate_visible_cells (Task 5 only migrated the
visible_cells/visible_cells_cached shared helper). Both secrecy-egress
and movement-gate center math now go through the same GridShape
abstraction. Behavior unchanged for square scenes."
```

- [ ] **Step 7: Mandatory security buddy-check** — same requirements as Task 5's Step 8, applied to this function: behavior-preservation for every existing test, fail-closed/no-widening direction, and (now genuinely) confirm `player_lit_mask` and `visible_cells`/`visible_cells_cached` compute IDENTICAL cell centers for the same `(i,j,cell)` input even though they're two separate call sites.

---

## Task 6: Frozen-fixture parity gate — prove `SquareGrid`-via-abstraction matches pre-refactor square behavior end-to-end `[sec]`

**Files:**
- Create: `src/server/src/scene/grid_shape_parity_tests.rs` (a dedicated test module, added to `scene/mod.rs`'s test-only module declarations via `#[cfg(test)] mod grid_shape_parity_tests;`)

**Interfaces:** Test-only, no production code change. This is the regression-safety gate the whole plan depends on (per the design doc's H3 decision, mirroring the M10f-2 precedent) — it must pass before Task 7 (old-code cleanup) or Task 10+ (HexGrid) proceed.

`[sec]`: per this plan's Buddy-check directives, this task requires a mandatory two-reviewer security buddy-check — a false-positive "parity proven" here would let a real behavior change through every downstream consumer undetected.

- [ ] **Step 1: Assemble the frozen fixture battery**

This is NOT new test logic — it is a curated set of INPUT scenarios (wall configurations, footprint radii, region layouts, start/goal pairs) run through BOTH: (a) the current (post-Tasks-1-5) `GridShape`-routed code, and (b) a frozen snapshot of the pre-refactor hardcoded square behavior, asserting the two outputs are IDENTICAL. Since Tasks 1-5 already replaced the hardcoded code in place (there is no separate "old" code path still compiled), the frozen snapshot instead comes from PINNING the exact numeric/structural outputs of representative existing test scenarios AS LITERAL EXPECTED VALUES — reusing the actual scenarios from `pathfinding.rs`'s `astar_tests`, `move_exec.rs`'s `gate_walk` tests, `scene/mod.rs`'s `visible_cells` tests, and `regions.rs`'s `rasterize` tests, but asserting the FULL result (every cell in a returned set, every route waypoint, every cost value) rather than a partial spot-check, so a subtle drift in any single cell would be caught.

Write at minimum:
- One A* route test with all 4 `DiagonalRule` variants on the same wall/footprint layout, asserting the exact returned `PathOutcome.path`/`.cost` for each rule (reuse `pathfinding.rs`'s `astar_tests` fixture conventions).
- One `gate_walk` mask-gate test on a multi-cell diagonal move crossing a mask boundary, asserting the exact `MoveOutcome`/truncation point.
- One `visible_cells`/`accumulate_visible_cells` test on an open scene with 2+ light/vision sources, asserting the exact returned cell `BTreeSet` (not just membership of a few sample cells).
- One `rasterize` test per `RegionShape` variant (Rect/Circle/Polygon), asserting the exact returned cell `Vec` (sorted, for determinism).

- [ ] **Step 2: Run the parity battery**

Run: `cargo test --manifest-path src/server/Cargo.toml grid_shape_parity -- --nocapture`
Expected: PASS. If ANY assertion fails, this means Tasks 1-5's refactor introduced a real behavior change — STOP and fix the refactor (do not adjust the expected value to match a drifted result; the expected values are the ground truth, pinned from the pre-refactor behavior).

- [ ] **Step 3: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`

- [ ] **Step 4: Commit**

```bash
git add src/server/src/scene/grid_shape_parity_tests.rs src/server/src/scene/mod.rs
git commit -m "test(server/scene): frozen-fixture parity gate for the GridShape refactor [sec]

Proves SquareGrid-via-GridShape produces identical A*/mask-gate/
visibility/region-rasterization output to the pre-refactor hardcoded
square code, across every DiagonalRule variant and all three
RegionShape kinds. This is the regression-safety gate before any
hex-specific work begins."
```

- [ ] **Step 5: Mandatory security buddy-check** — two independent reviewers (opus-tier) must confirm the fixture battery is genuinely exhaustive enough to catch a real regression (not vacuous/tautological), and independently re-derive at least 2 of the pinned expected values by hand against the actual pre-Task-1 square math to confirm the pins themselves are correct (not merely self-consistent with a potentially-already-wrong refactor).

---

## Task 7: Delete now-dead hardcoded square-only code paths

**Files:**
- Modify: `src/server/src/scene/pathfinding.rs` (confirm `step_cost` was already deleted in Task 2 — if any other now-unreachable square-hardcoded helper remains, remove it here)
- Modify: `src/server/src/scene/movement.rs` (the free `supercover_cells` function itself STAYS — `SquareGrid::line_traversal` calls it, per Task 1's design; do NOT delete it)
- Modify: `src/server/src/scene/mod.rs` / `regions.rs` (any now-dead direct square-math helper made redundant by Tasks 4-5's refactor)

**Interfaces:** Cleanup only, no behavior change. This task exists to keep the codebase from carrying two square implementations (one hardcoded, one behind `GridShape`) once parity is proven — per the design doc's H3 sequencing ("`HexGrid` is only built and wired in once that parity is proven").

- [ ] **Step 1: Grep for dead code**

Run: `cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings -W dead_code` and separately grep for any private helper function in `pathfinding.rs`/`regions.rs`/`scene/mod.rs` that Tasks 2-5's refactors made unreachable (their call sites were replaced with `grid.shape.*`/`grid.cell_center(...)` calls) but that wasn't explicitly deleted in those tasks' own steps.

- [ ] **Step 2: Delete confirmed-dead code**

Remove any function clippy's `dead_code` lint flags that this plan's earlier tasks made obsolete. Do NOT remove `movement::supercover_cells`, `pathfinding::cell_center`, or `pathfinding::footprint_cells` — these are still called BY `SquareGrid`'s trait implementation (Task 1), just no longer called directly by `pathfinding.rs`/`move_exec.rs`/`regions.rs`/`scene/mod.rs`.

- [ ] **Step 3: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`
Expected: PASS, zero dead-code warnings, all tests (including Task 6's parity battery) still green.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore(server/scene): remove code made dead by the GridShape refactor

Cleanup only, no behavior change. Confirmed via the frozen-fixture
parity gate (Task 6) that nothing load-bearing was removed."
```

---

## Task 8: `HexGrid` coordinate math (cell-center, point-to-cell, no wiring yet)

**Files:**
- Modify: `src/server/src/scene/grid_shape.rs` (add `HexGrid` alongside `SquareGrid`)
- Test: same file's `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `pub(crate) struct HexGrid { pub size: f64 }` (pointy-top axial; `size` = outer radius, matching the client's `GridSpec.size` convention for hex per `grid.ts:11`). Implements `GridShape::cell_center`/`cell_of` in this task; `neighbors_with_cost`/`line_traversal`/`footprint_cells` are added in Tasks 9-11 (each independently testable).

- [ ] **Step 1: Write the failing test**

Port the client's `grid.ts` hex coordinate formulas exactly and cross-check them:

```rust
// Add to grid_shape.rs's existing #[cfg(test)] mod tests
#[test]
fn hex_grid_axial_round_trip_pixel_to_axial_to_pixel() {
    let g = HexGrid { size: 50.0 };
    // Round-tripping a cell's own center through pixel->axial->pixel should be a fixed point.
    let center = g.cell_center((2, -1));
    let (q, r) = g.pixel_to_axial(center);
    assert_eq!((q, r), (2, -1));
}

#[test]
fn hex_grid_cell_of_matches_axial_round_for_a_known_point() {
    // size=50: cell (0,0)'s center is (0,0) in pointy-top axial pixel space (Red Blob Games
    // convention, matching client/src/render/src/grid.ts's pixelToAxial/axialToPixel exactly).
    let g = HexGrid { size: 50.0 };
    assert_eq!(g.cell_of((0.0, 0.0)), (0, 0));
    // A point well inside cell (1,0)'s hex (center at axial (1,0) -> pixel via axialToPixel)
    // should resolve back to (1,0).
    let c10_center = g.cell_center((1, 0));
    assert_eq!(g.cell_of(c10_center), (1, 0));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src/server/Cargo.toml hex_grid -- --nocapture`
Expected: FAIL — `HexGrid` doesn't exist yet.

- [ ] **Step 3: Implement `HexGrid`'s coordinate math**

Port `grid.ts`'s `pixelToAxial`/`axialToPixel`/`axialRound` exactly (same formulas, same pointy-top orientation, same `size` = outer-radius convention — read `src/client/render/src/grid.ts` lines 111-140+ for the exact current formulas before transcribing, since this plan's earlier context-gathering only saw the `pixelToAxial` formula's first 2 lines):

```rust
// Add to grid_shape.rs, alongside SquareGrid:

/// Pointy-top axial hex grid (Red Blob Games convention), mirroring
/// `src/client/render/src/grid.ts`'s `Grid` class's hex math exactly — same coordinate formulas,
/// same `size` = outer-radius convention, so client and server cell indices always agree.
pub(crate) struct HexGrid {
    pub size: f64,
}

impl HexGrid {
    /// Fractional axial coordinates for scene point `p` (pre-rounding).
    fn pixel_to_axial_frac(&self, p: vision::P) -> (f64, f64) {
        let q = ((3.0_f64.sqrt() / 3.0) * p.0 - (1.0 / 3.0) * p.1) / self.size;
        let r = (2.0 / 3.0 * p.1) / self.size;
        (q, r)
    }

    /// Round fractional axial `(q, r)` to the nearest integer hex, via cube-coordinate rounding
    /// (Red Blob Games's standard technique: round each cube axis independently, then fix up the
    /// axis with the largest rounding error so `x + y + z == 0` is restored exactly).
    fn axial_round(&self, qf: f64, rf: f64) -> (i32, i32) {
        let xf = qf;
        let zf = rf;
        let yf = -xf - zf;
        let mut x = xf.round();
        let mut y = yf.round();
        let mut z = zf.round();
        let dx = (x - xf).abs();
        let dy = (y - yf).abs();
        let dz = (z - zf).abs();
        if dx > dy && dx > dz {
            x = -y - z;
        } else if dy > dz {
            y = -x - z;
        } else {
            z = -x - y;
        }
        (x as i32, z as i32)
    }

    /// Scene-space center of axial hex `(q, r)`.
    fn axial_to_pixel(&self, q: i32, r: i32) -> vision::P {
        let x = self.size * (3.0_f64.sqrt() * q as f64 + 3.0_f64.sqrt() / 2.0 * r as f64);
        let y = self.size * (3.0 / 2.0 * r as f64);
        (x, y)
    }

    /// Exposed for the round-trip test above; not part of the `GridShape` trait.
    fn pixel_to_axial(&self, p: vision::P) -> (i32, i32) {
        let (qf, rf) = self.pixel_to_axial_frac(p);
        self.axial_round(qf, rf)
    }
}

impl GridShape for HexGrid {
    fn cell_center(&self, c: Cell) -> vision::P {
        self.axial_to_pixel(c.0, c.1)
    }

    fn cell_of(&self, p: vision::P) -> Cell {
        self.pixel_to_axial(p)
    }

    // neighbors_with_cost/line_traversal/footprint_cells: Task 9-11.
    fn neighbors_with_cost(&self, _c: Cell, _parity: u8) -> Vec<(Cell, f64, u8)> {
        unimplemented!("Task 9")
    }
    fn line_traversal(&self, _a: vision::P, _b: vision::P, _cell: f64) -> Option<BTreeSet<Cell>> {
        unimplemented!("Task 10")
    }
    fn footprint_cells(&self, _anchor: Cell, _ctr: vision::P, _r_scene: f64, _cell: f64) -> Vec<Cell> {
        unimplemented!("Task 11")
    }
}
```

**Before finalizing:** read `src/client/render/src/grid.ts`'s actual current `pixelToAxial`/`axialToPixel`/`axialRound` bodies in full (lines ~111-140+) and confirm every constant/formula above matches exactly — this plan's context-gathering saw only the first 2 lines of `pixelToAxial`; do not proceed on partial information, re-derive from the real file.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src/server/Cargo.toml hex_grid -- --nocapture`
Expected: PASS (the 2 coordinate tests; the 3 `unimplemented!` trait methods are not exercised by these tests).

- [ ] **Step 5: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`
Expected: PASS — `cargo test` does not execute `unimplemented!` code paths since nothing calls those 3 methods yet; clippy will not flag `unimplemented!` as an error by default (confirm; if it does, this task is not yet integration-tested enough to compile clean under `-D warnings` — in that case stub the 3 methods with a trivial non-panicking placeholder like `Vec::new()`/`None` instead of `unimplemented!`, since this plan's own "No Placeholders" rule for the PLAN doc doesn't forbid an intentionally-incomplete trait impl mid-plan, but the CODE itself must compile and pass clippy cleanly at every commit).

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/grid_shape.rs
git commit -m "feat(server/scene): HexGrid coordinate math (cell_center/cell_of)

Pointy-top axial hex, mirroring the client's grid.ts formulas exactly
(pixelToAxial/axialToPixel/axialRound). neighbors_with_cost/
line_traversal/footprint_cells land in the next 3 tasks."
```

---

## Task 9: `HexGrid::neighbors_with_cost` (uniform 1-per-step, no `DiagonalRule` analog)

**Files:**
- Modify: `src/server/src/scene/grid_shape.rs`

**Interfaces:** Completes `HexGrid`'s `neighbors_with_cost`. Per the design doc's H4/H5 decisions: hex movement is uniform 1-per-step (no diagonal-rule variants), and `parity` is passed through unchanged (hex has no rule-dependent search state).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn hex_grid_neighbors_with_cost_returns_6_uniform_cost_neighbors() {
    let g = HexGrid { size: 50.0 };
    let ns = g.neighbors_with_cost((0, 0), 3);
    assert_eq!(ns.len(), 6, "hex has 6 neighbors, not 8");
    for (_, cost, parity) in &ns {
        assert!((cost - 1.0).abs() < 1e-9, "every hex step costs 1.0 uniformly");
        assert_eq!(*parity, 3, "hex never touches parity — passed through unchanged");
    }
    // The 6 axial neighbor offsets (Red Blob Games pointy-top convention).
    let mut got: Vec<Cell> = ns.iter().map(|(c, _, _)| *c).collect();
    got.sort();
    let mut want = vec![(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
    want.sort();
    assert_eq!(got, want);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src/server/Cargo.toml hex_grid_neighbors -- --nocapture`
Expected: FAIL — `neighbors_with_cost` is `unimplemented!()`.

- [ ] **Step 3: Implement**

```rust
impl GridShape for HexGrid {
    // ... cell_center/cell_of unchanged from Task 8 ...

    fn neighbors_with_cost(&self, c: Cell, parity: u8) -> Vec<(Cell, f64, u8)> {
        const AXIAL_DIRS: [(i32, i32); 6] =
            [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
        AXIAL_DIRS
            .iter()
            .map(|&(dq, dr)| ((c.0 + dq, c.1 + dr), 1.0, parity))
            .collect()
    }

    // line_traversal/footprint_cells: Tasks 10-11.
    fn line_traversal(&self, _a: vision::P, _b: vision::P, _cell: f64) -> Option<BTreeSet<Cell>> {
        unimplemented!("Task 10")
    }
    fn footprint_cells(&self, _anchor: Cell, _ctr: vision::P, _r_scene: f64, _cell: f64) -> Vec<Cell> {
        unimplemented!("Task 11")
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src/server/Cargo.toml hex_grid_neighbors -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/grid_shape.rs
git commit -m "feat(server/scene): HexGrid::neighbors_with_cost (uniform 1-per-step)

6 axial neighbors, cost 1.0 each, parity passed through unchanged —
hex has no diagonal-rule analog per the design doc's H4/H5 decisions."
```

---

## Task 10: `HexGrid::line_traversal` (cube-interpolation hex line-drawing)

**Files:**
- Modify: `src/server/src/scene/grid_shape.rs`

**Interfaces:** Completes `HexGrid`'s `line_traversal` — the hex equivalent of `movement::supercover_cells`, used by the mask gate and move-executor once wired (Task 13).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn hex_grid_line_traversal_includes_start_and_end_cells() {
    let g = HexGrid { size: 50.0 };
    let a_center = g.cell_center((0, 0));
    let b_center = g.cell_center((3, 0));
    let cells = g.line_traversal(a_center, b_center, 50.0).unwrap();
    assert!(cells.contains(&(0, 0)));
    assert!(cells.contains(&(3, 0)));
    // A straight 3-cell traversal along one axial direction crosses exactly cells (0,0)..(3,0).
    assert_eq!(cells.len(), 4);
}

#[test]
fn hex_grid_line_traversal_degenerate_same_point_returns_single_cell() {
    let g = HexGrid { size: 50.0 };
    let p = g.cell_center((2, -1));
    let cells = g.line_traversal(p, p, 50.0).unwrap();
    assert_eq!(cells.len(), 1);
    assert!(cells.contains(&(2, -1)));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src/server/Cargo.toml hex_grid_line_traversal -- --nocapture`
Expected: FAIL — `line_traversal` is `unimplemented!()`.

- [ ] **Step 3: Implement**

Clean-room hex line-drawing via cube-coordinate linear interpolation + hex-round per sample (the standard algorithm, Red Blob Games's cited approach — source: public-domain computational geometry, matching this codebase's citation convention per ARCHITECTURE §7):

```rust
impl GridShape for HexGrid {
    // ... cell_center/cell_of/neighbors_with_cost unchanged ...

    fn line_traversal(&self, a: vision::P, b: vision::P, cell: f64) -> Option<BTreeSet<Cell>> {
        if !a.0.is_finite() || !a.1.is_finite() || !b.0.is_finite() || !b.1.is_finite() {
            return None;
        }
        let (aq, ar) = self.pixel_to_axial_frac(a);
        let (bq, br) = self.pixel_to_axial_frac(b);
        // Cube distance between the two fractional hexes = number of samples needed (one per
        // hex crossed, at minimum) — same "N = max axial delta" idea as a square Bresenham/DDA
        // line needing max(|dx|,|dy|) samples.
        let ax = aq;
        let az = ar;
        let ay = -ax - az;
        let bx = bq;
        let bz = br;
        let by = -bx - bz;
        let n = ((ax - bx).abs().max((ay - by).abs()).max((az - bz).abs())).round() as i64;
        const MAX_HEX_LINE_SAMPLES: i64 = 4096; // DoS bound, mirrors movement.rs's MAX_MOVE_CELLS class of guard
        if n < 0 || n > MAX_HEX_LINE_SAMPLES {
            return None;
        }
        let mut out = BTreeSet::new();
        if n == 0 {
            out.insert(self.axial_round(aq, ar));
            return Some(out);
        }
        for i in 0..=n {
            let t = i as f64 / n as f64;
            let qf = aq + (bq - aq) * t;
            let rf = ar + (br - ar) * t;
            out.insert(self.axial_round(qf, rf));
        }
        let _ = cell; // hex cell size is baked into `self.size`; the `cell` param is kept only
                      // for GridShape trait-signature parity with SquareGrid, which DOES need it.
        Some(out)
    }

    fn footprint_cells(&self, _anchor: Cell, _ctr: vision::P, _r_scene: f64, _cell: f64) -> Vec<Cell> {
        unimplemented!("Task 11")
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src/server/Cargo.toml hex_grid_line_traversal -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/grid_shape.rs
git commit -m "feat(server/scene): HexGrid::line_traversal (cube-interpolation hex line)

Clean-room hex-grid line-drawing: linear cube-coordinate interpolation
+ hex-round per sample, DoS-bounded at 4096 samples. Hex equivalent of
movement.rs's supercover_cells."
```

---

## Task 11: `HexGrid::footprint_cells` (footprint-disc vs. hex-cell overlap)

**Files:**
- Modify: `src/server/src/scene/grid_shape.rs`

**Interfaces:** Completes `HexGrid`, making it a fully functional `GridShape` implementation.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn hex_grid_footprint_cells_always_includes_the_anchor() {
    let g = HexGrid { size: 50.0 };
    let anchor = (2, -1);
    let ctr = g.cell_center(anchor);
    // Zero-radius footprint: only the anchor cell.
    let cells = g.footprint_cells(anchor, ctr, 0.0, 50.0);
    assert_eq!(cells, vec![anchor]);
}

#[test]
fn hex_grid_footprint_cells_large_radius_includes_neighbors() {
    let g = HexGrid { size: 50.0 };
    let anchor = (0, 0);
    let ctr = g.cell_center(anchor);
    // A footprint radius comparable to the hex's own size should pull in at least one neighbor.
    let cells = g.footprint_cells(anchor, ctr, 60.0, 50.0);
    assert!(cells.len() > 1, "a large-enough footprint overlaps more than just the anchor");
    assert!(cells.contains(&anchor));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src/server/Cargo.toml hex_grid_footprint -- --nocapture`
Expected: FAIL — `footprint_cells` is `unimplemented!()`.

- [ ] **Step 3: Implement**

Per the design doc's H7 decision, footprint radius stays a scene-space distance; only the disc-vs-cell overlap test needs a hex-shaped equivalent of the square implementation's AABB check. A conservative, correct approach: scan a small ring of candidate hexes around the anchor (using cube distance, since a footprint radius is bounded by `MAX_FOOTPRINT_CELLS` per `pathfinding.rs`'s existing DoS guard) and test each candidate's CENTER-TO-DISC-CENTER distance against `r_scene` plus the hex's own inradius (a hex cell "overlaps" the disc if any part of the hex is within `r_scene` of the disc center — using center-distance ≤ `r_scene + hex_inradius` is a conservative, always-safe over-approximation, mirroring the square implementation's own AABB-vs-disc distance test which is similarly a conservative overlap test, not an exact one):

```rust
impl GridShape for HexGrid {
    // ... cell_center/cell_of/neighbors_with_cost/line_traversal unchanged ...

    fn footprint_cells(&self, anchor: Cell, ctr: vision::P, r_scene: f64, _cell: f64) -> Vec<Cell> {
        let mut out = Vec::new();
        // Hex inradius (center-to-edge distance) for a pointy-top hex with outer radius `size`.
        let inradius = self.size * 3.0_f64.sqrt() / 2.0;
        // Scan radius in hex rings: a disc of radius r_scene can overlap hexes up to
        // ceil(r_scene / (size * 1.5)) rings out (1.5*size is the hex row/column pitch) — bounded
        // and small for any realistic footprint (MAX_FOOTPRINT_CELLS caps r_scene upstream).
        let ring_radius = ((r_scene / (self.size * 1.5)).ceil() as i32).max(0) + 1;
        for dq in -ring_radius..=ring_radius {
            for dr in -ring_radius..=ring_radius {
                let ds = -dq - dr;
                if ds.abs() > ring_radius {
                    continue; // outside the hex-shaped scan region
                }
                let c = (anchor.0 + dq, anchor.1 + dr);
                let center = self.cell_center(c);
                let dx = center.0 - ctr.0;
                let dy = center.1 - ctr.1;
                if (dx * dx + dy * dy).sqrt() <= r_scene + inradius {
                    out.push(c);
                }
            }
        }
        if out.is_empty() {
            out.push(anchor);
        }
        out
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src/server/Cargo.toml hex_grid_footprint -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`
Expected: PASS — `HexGrid` now fully implements `GridShape` with no `unimplemented!()` remaining; confirm clippy doesn't flag anything about the now-fully-implemented trait.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/grid_shape.rs
git commit -m "feat(server/scene): HexGrid::footprint_cells, completing GridShape

Conservative disc-vs-hex-cell overlap via center-distance <= r_scene +
hex_inradius (always-safe over-approximation, same conservatism class
as the square implementation's AABB-vs-disc test). HexGrid now fully
implements GridShape."
```

---

## Task 12: Resolve a scene's `GridShape` from `SceneEngine.grid.kind`, wire every call site

**Files:**
- Modify: `src/server/src/scene/mod.rs` (add a `resolve_grid_shape` helper near the existing `resolved_diagonal_rule`-style resolvers; wire it into every call site that currently constructs a `SquareGrid` directly — `pathfinding.rs::find`, `move_exec.rs`'s mask-gate call site, `regions.rs::rasterize`'s 2 call sites, `accumulate_visible_cells`'s 2 callers)
- Test: new integration tests confirming a scene with `grid.kind == "hex"` actually gets a `HexGrid`, and an unrecognized/malformed `kind` fails closed to `SquareGrid`

**Interfaces:**
- Produces: `pub(crate) fn resolve_grid_shape(settings: &ResolvedScene) -> Box<dyn crate::scene::grid_shape::GridShape>` (boxed, since the concrete type varies per scene and callers need a uniform return type) — `settings.grid.kind == "hex"` selects `HexGrid { size: settings.grid.size }`; anything else (including `"square"` and any unrecognized string) selects `SquareGrid { cell: settings.grid.size, rule: resolved_diagonal_rule(settings) }` (confirm `ResolvedScene`'s exact field name for the grid spec and the existing diagonal-rule resolver's exact name via `grep -n "struct ResolvedScene\|resolved_diagonal_rule" src/server/src/scene/mod.rs` before finalizing — this plan's context-gathering confirmed `resolved_diagonal_rule` exists per the scene-rendering skill's documentation of `mod.rs`, but the exact `ResolvedScene` field path for grid settings must be read directly).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn resolve_grid_shape_selects_hex_grid_for_hex_kind_scenes() {
    // Construct a ResolvedScene (or the minimal settings struct resolve_grid_shape actually
    // takes, confirmed by reading the real signature) with grid.kind == "hex", grid.size == 50.0.
    // Confirm the returned Box<dyn GridShape>'s behavior matches a directly-constructed
    // HexGrid { size: 50.0 } — e.g. by comparing cell_center((1,0)) output, since GridShape
    // doesn't derive PartialEq/Debug (trait objects can't) and this is the practical way to
    // assert "the right concrete type was selected" without downcasting.
    // (Fill in the exact ResolvedScene/settings construction using this file's existing test
    // fixture helpers — read them first.)
}

#[test]
fn resolve_grid_shape_falls_back_to_square_grid_for_unrecognized_kind() {
    // grid.kind == "triangle" (or any non-"hex" string) must resolve to SquareGrid, fail-closed
    // toward the already-hardened behavior, per the design doc's H8 decision.
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src/server/Cargo.toml resolve_grid_shape -- --nocapture`
Expected: FAIL — `resolve_grid_shape` doesn't exist yet.

- [ ] **Step 3: Implement `resolve_grid_shape` and wire every call site**

```rust
// In scene/mod.rs, near the existing resolved_diagonal_rule-style resolvers:
pub(crate) fn resolve_grid_shape(settings: &ResolvedScene) -> Box<dyn crate::scene::grid_shape::GridShape> {
    if settings.grid.kind == "hex" {
        Box::new(crate::scene::grid_shape::HexGrid { size: settings.grid.size })
    } else {
        Box::new(crate::scene::grid_shape::SquareGrid {
            cell: settings.grid.size,
            rule: resolved_diagonal_rule(settings),
        })
    }
}
```

(Adjust the exact `settings.grid.kind`/`settings.grid.size` field-access path to match `ResolvedScene`'s real shape once read in Step 1 — this plan's context-gathering confirmed `data/engine/scene.rs`'s `Grid { kind: String, size: f64, ... }` exists on the raw `SceneEngine`, but `ResolvedScene`'s own field may wrap or rename it; use whatever the real resolver already exposes.)

Update every call site Tasks 2-5 wired to construct a `SquareGrid` directly (`pathfinding.rs::find`, `move_exec.rs`'s mask-gate call, `regions.rs::rasterize`'s 2 call sites in `SceneEcs::region_field`, and `accumulate_visible_cells`'s 2 callers `visible_cells`/`visible_cells_cached`) to instead call `resolve_grid_shape(settings)` and pass the boxed trait object through (adjusting each function's parameter type from `shape: &dyn GridShape` to accept either a `&dyn GridShape` borrowed from the box, or thread the `Box<dyn GridShape>` itself where a function currently takes ownership-adjacent parameters — match each call site's existing borrow/ownership pattern rather than inventing a new convention).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src/server/Cargo.toml resolve_grid_shape -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`
Expected: PASS — including Task 6's frozen-fixture parity battery (confirms wiring `resolve_grid_shape` in place of direct `SquareGrid` construction didn't change square-scene behavior).

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/mod.rs src/server/src/scene/pathfinding.rs src/server/src/scene/move_exec.rs src/server/src/scene/regions.rs
git commit -m "feat(server/scene): resolve_grid_shape selects square/hex per scene

Every movement/pathfinding/vision/region call site now resolves its
GridShape from the scene's grid.kind instead of hardcoding SquareGrid.
Unrecognized/malformed kind fails closed to SquareGrid."
```

---

## Task 13: Hex-scene integration tests (wall/mask/region-gated move + A* route)

**Files:**
- Modify: `src/server/src/scene/pathfinding.rs` (new `#[cfg(test)]` tests in `astar_tests`, hex-scene variants)
- Modify: `src/server/src/scene/move_exec.rs` (new hex-scene `gate_walk` tests)
- Test: this task IS the tests

**Interfaces:** No production code change — this task proves the fully-wired hex path (Tasks 8-12) behaves correctly end-to-end, mirroring the square integration-test coverage this codebase already has.

- [ ] **Step 1: Write hex A* route tests**

```rust
// In pathfinding.rs's astar_tests module:
#[test]
fn hex_scene_astar_finds_a_route_around_a_wall() {
    // Construct a PathGrid with shape: &HexGrid{size:...} (via a new open()-style helper mirroring
    // the existing square `open()` fixture, or a dedicated `open_hex()` helper) and a wall
    // configuration that forces a detour. Assert astar_leg returns a non-empty route reaching the
    // goal, with cost reflecting the uniform 1.0-per-hex-step model (no DiagonalRule variance).
}

#[test]
fn hex_scene_astar_respects_the_visibility_mask() {
    // A hex scene with a mask excluding a subset of cells; assert the route never enters a
    // masked-out cell, mirroring the square mask test's structure.
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src/server/Cargo.toml hex_scene_astar -- --nocapture`
Expected: FAIL initially if the test scaffolding (`open_hex()` helper) doesn't exist yet — write it first, matching `open()`'s existing structure but with `rule` irrelevant/omitted (hex has none) and `shape: &HexGrid{size}`.

- [ ] **Step 3: Confirm they pass against the already-wired hex path**

Run: `cargo test --manifest-path src/server/Cargo.toml hex_scene_astar -- --nocapture`
Expected: PASS once the test scaffolding correctly constructs a hex `PathGrid` — no production code change needed here, Tasks 1-12 already wired the full path; this task is verification.

- [ ] **Step 4: Write hex `gate_walk` mask/wall gate tests**

```rust
// In move_exec.rs's test module:
#[test]
fn hex_scene_gate_walk_blocks_entry_at_a_wall() {
    // Mirror this file's existing square gate_walk wall test, but on a hex scene — confirm the
    // executor correctly rejects a move whose hex-traversal crosses a blocksMove wall.
}

#[test]
fn hex_scene_gate_walk_respects_the_visibility_mask() {
    // Mirror the existing square mask test on a hex scene.
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src/server/Cargo.toml hex_scene_gate_walk -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`

- [ ] **Step 7: Commit**

```bash
git add src/server/src/scene/pathfinding.rs src/server/src/scene/move_exec.rs
git commit -m "test(server/scene): hex-scene A* + gate_walk integration tests

Proves the fully-wired hex path (walls/mask/A*) behaves correctly
end-to-end, mirroring this codebase's existing square-scene coverage.
No production code change — Tasks 1-12 already wired the full path."
```

---

## Scope extension (Tasks 14a–14c): why this plan reaches into the client

The approved design doc (`docs/superpowers/specs/2026-07-22-hex-grid-server-movement-design.md`) scoped this project as **server-only** ("no client changes"), on the premise that the client was already hex-capable and only the server lagged. Executing Task 14 (the original client e2e) surfaced that premise was **half true**: `render/src/grid.ts`'s hex math and `Stage.svelte`'s `onDocs` (which reads `engine.grid.kind`/`.size` and calls `e.setGrid(...)`) ARE already hex-aware, but there is **no GM-facing control anywhere to set a scene's `grid.kind` to `"hex"`** — grid kind is not authorable in the product at all. Without that control, every server-side task in this plan (1–13) is unreachable dead code: a GM cannot create a hex scene to exercise it, and the client e2e cannot author one to prove the round-trip.

Per the project's Definition-of-Blocked rule (build a needed, unscoped, simple feature rather than deferring it), the fix is to build the missing authoring surface, not to descope the proof. This is a deliberate, logged extension of the design's server-only scope into a **minimal** client surface: one authoring control (14a), one observability signal the e2e needs (14b), and the e2e itself (14c). No render/wire/engine changes — the client already renders hex; these tasks only expose it and observe move outcomes.

**Buddy-check:** Tasks 14a/14b/14c are client-side and **NOT `[sec]`** — none touches a secrecy boundary (14a is GM-only authoring over an existing wire field; 14b is a read-out of the mover's OWN outcome, already delivered to that client; 14c is a test). Standard task review. **Task 14d IS `[sec]`** — it fixes the `Room::publish` movement-restriction (secrecy) gate — mandatory two-reviewer opus-tier buddy-check.

**Discovery during 14c execution (why 14d exists):** attempting the client e2e surfaced two facts. (1) A **shipped production bug** — `ToolRail.svelte` never forwarded `ctx.moveRequest` into its `ToolController`, so the measure-tool's route-commit (the only client path that calls `moveRequest`) was a silent permanent no-op in the real app; fixed independently on this branch (commit `73b44bd`, + regression test). (2) A **hex coverage gap the GridShape refactor missed** — `Room::publish`'s movement-restriction gate (the PRIMARY player path: a select-tool DRAG writes `/engine/x,y` directly, NOT via `moveRequest`) still calls `movement::supercover_cells` directly (square line-traversal) at `room.rs:264`, while the mask it tests against is hex-aware. Task 3's refactor grep was scoped to `scene/` and never audited `ws/room.rs`. `execute_move` (the `moveRequest` path) is already hex-correct; only this one `publish` call site is square-on-hex, violating move_exec.rs's documented "publish and execute_move agree on every cell" invariant on hex scenes. → **Task 14d** (below) closes it. Because GMs bypass this gate entirely (`room.rs:215`) and the client always consults the server's wall-aware router before a `moveRequest`, an honest client-level proof of server-side gating requires a NON-GM player account — hence Task 14c is rewritten as a two-session (GM + player) e2e asserting the server rolls back a player's illegal drag.

---

## Task 14a: GM authoring control for a scene's grid kind + size (client)

**Files:**
- Modify: `src/modules/game-settings/src/GameSettingsPanel.svelte` (the per-scene `<fieldset>` — add a grid-kind `<select>` and grid-size `<input>` alongside the existing `/engine/grid/distance` controls at ~lines 484–504)
- Modify: the game-settings i18n locale file(s) — add `gameSettings.scene.gridKind` + `gameSettings.scene.gridSize` keys (grep `"distancePerCell"` under `src/` to locate the exact locale JSON, and add the new keys beside it in EVERY locale file present, not just one)
- Test: `src/modules/game-settings/` component/unit test (follow the package's existing test convention; if the panel has no spec yet, add a focused Vitest spec asserting the new controls dispatch the right intent)

**Interfaces:** No new engine/wire type — `SceneEngine.grid` already is `{ kind: "square" | "hex", size: number, distance: {...} | null }` (`buildSceneDoc`'s default is `{ kind: "square", size: 100, distance: null }`). The client is ALREADY hex-aware end-to-end (`Stage.svelte`'s `onDocs` reads `engine.grid.kind`/`.size` → `e.setGrid(spec)`; `render/src/grid.ts` implements the hex math), so exposing this control is the ONLY missing piece for a GM to author and render a hex scene. This task adds no server, wire, or render code.

Key correctness constraints (mirror the rest of this panel):
- Grid kind/size are per-scene INTRINSIC values, NOT inherit-from-world tri-states (there is no world-level grid kind) — so they follow the panel's plain-value pattern (like the `bounds` width/height controls at ~lines 507–518), NOT the `"" = inherit` tri-state used by the vision/lighting overrides. No `<option value="">inherit</option>`.
- OCC pre-image MUST be the field's real current value: `setScene("/engine/grid/kind", ssys.grid?.kind ?? "square", newKind)` and `setScene("/engine/grid/size", ssys.grid?.size ?? 100, newSize)`. Never a hardcoded pre-image (the panel-wide OCC rule — see the `set`/`setScene` doc comments).
- Kind `<select>` options are exactly `["square", "hex"]` (declare `const GRID_KIND = ["square", "hex"] as const;` alongside the file's other option arrays). Size is a number `<input min="1" step="1">`.
- Do NOT change `Stage.svelte`'s bootstrap `grid: { kind: "square", size: 100 }` or `buildSceneDoc`'s square default — square stays the sensible default; this control lets the GM switch a scene to hex.

- [ ] **Step 1: Write the failing test** — a Vitest spec asserting that changing the kind select to `"hex"` dispatches an `update` intent with a `/engine/grid/kind` change whose `new` is `"hex"` and `old` is the scene's current kind; likewise the size input for `/engine/grid/size`.
- [ ] **Step 2: Implement** the two controls + i18n keys, following the per-scene fieldset's exact `setScene` pattern.
- [ ] **Step 3: Gate** `pnpm --filter @shadowcat/game-settings test && pnpm --filter @shadowcat/game-settings typecheck` (+ the repo lint).
- [ ] **Step 4: Commit** `feat(client/game-settings): GM control to set a scene's grid kind (square/hex) + size`.

---

## Task 14b: Surface the server's move-resolution outcome as a stage observability signal (client)

**Files:**
- Modify: `src/modules/stage/src/Stage.svelte` (add `host.dataset.lastMoveOutcome`, fed by the move-resolution callback path, mirroring `onPing` → `host.dataset.lastPing` at ~lines 166–169 and its teardown at ~line 192)
- Possibly modify: `src/client/ui-kit/src/appContext.ts` and/or `src/client/shell/src/lib/worldSession.svelte.ts` — ONLY if no existing ctx callback already surfaces the local player's move resolution; if one must be added, mirror the existing `onPing`/`onAssetChanged` subscription seam exactly (no new architecture)
- Test: `src/modules/stage/src/Stage.test.ts` — assert the dataset signal updates on a simulated move resolution

**Interfaces:** A new read-only observability attribute `data-last-move-outcome` on `.stage-host`, valued as a stable token string — `"executed"`, `"truncated"`, or `"rejected"` (optionally suffixed with the resolved end cell, e.g. `"truncated:3,2"`, only if trivially available). Mirrors the existing `data-last-ping`/`data-token-count`/`data-vision-mode` signals the stage already emits. No behavior change to movement — this only exposes an outcome the client already receives.

- [ ] **Step 1: Map the move-resolution data flow FIRST (do not assume a callback exists).** Trace how a move resolution reaches the client for the LOCAL player's own token: `MoveError` (mover-only reject, `ws/conn.rs`'s `etx` path) and `MoveStream` (executed/truncated trajectory, the M2 egress with its `truncated` flag). Read `worldSession.svelte.ts`, `appContext.ts`, `render/src/engine.ts`, and `scene-tools/src/controller.svelte.ts` to find where each is received today and whether the reject reason / truncation flag is already surfaced to any client seam. Report the actual flow. **If Step 1 finds the client genuinely never learns a move was server-rejected/truncated, STOP and report** — that is itself a real product gap worth a design decision, not something to paper over with a hollow always-`"executed"` signal.
- [ ] **Step 2: Write the failing test** — drive a fake move resolution through the seam Step 1 identified; assert `host.dataset.lastMoveOutcome` becomes the expected token.
- [ ] **Step 3: Implement** — wire the callback in Stage's init `$effect` (alongside `offPing = onPing(...)`), set `host.dataset.lastMoveOutcome`, and add its teardown to the cleanup return (mirror `offPing?.()`).
- [ ] **Step 4: Gate** `pnpm -r test && pnpm -r typecheck && pnpm -r lint`.
- [ ] **Step 5: Commit** `feat(client/stage): expose server move-resolution outcome as data-last-move-outcome`.

---

## Task 14d: Route `Room::publish`'s movement gate through `GridShape` (hex-correct the drag path) `[sec]`

**Files:**
- Modify: `src/server/src/ws/room.rs` (the M10e-4 movement-restriction gate in `publish`, ~lines 256–287 — replace the direct `movement::supercover_cells(a0, a1, cell)` at ~line 263–264 with the scene's resolved `GridShape::line_traversal`)
- Modify: `src/server/src/scene/move_exec.rs` (doc-comment lines ~20–23 and ~232–234 — update the stale "same `supercover_cells`" / "agree on every cell" wording to reflect that both paths now route through `GridShape::line_traversal`, on both grid kinds)
- Test: `room.rs`'s test module — add a hex-scene publish-gate test

**Interfaces:**
- Consumes: `SceneEcs::resolve_grid_shape(&self, scene: Uuid, cell: f64) -> Box<dyn GridShape>` (Task 12) — the SAME resolution `execute_move` uses internally (`move_exec.rs:309`).

`[sec]`: this is the movement-restriction secrecy gate (Visible/Revealed modes prevent moving into unseen cells — a fog-probing vector). Mandatory two-reviewer opus-tier buddy-check.

- [ ] **Step 1: Write the failing/pinning hex test** — a hex scene (`grid.kind="hex"`) with `movementRestriction="visible"`, a non-GM player token, and a visible/unseen hex-cell split. Assert a drag whose HEX line-traversal enters an unseen hex cell is rejected (`Err(Forbidden)`) while an in-mask hex drag is accepted. At minimum, assert `publish`'s gate and `move_exec::execute_move` return the SAME accept/reject for the SAME hex move + visible set (the documented "agree on every cell" invariant, now on hex). Mirror the existing `movement_blocked_for_player_crossing_wall_but_gm_bypasses` fixture (~room.rs:935).

- [ ] **Step 2: Apply the one-call-site fix**
```rust
let grid = scene.resolve_grid_shape(scene_id, cell);
let Some(move_cells) = grid.line_traversal(a0, a1, cell) else {
    return Err(DataError::Forbidden);
};
```
Reuse the `cell` already resolved just above. Do NOT touch the wall gate (`blocks_move` — pure geometry, grid-agnostic). Preserve the `None` → `Err(Forbidden)` fail-closed branch exactly.

- [ ] **Step 3: Confirm square parity** — every existing square publish-gate test passes UNCHANGED (`SquareGrid::line_traversal` delegates to `supercover_cells` verbatim). Do not modify those tests.

- [ ] **Step 4: Update the move_exec.rs doc-comments** so they no longer claim the two gates share `supercover_cells` — they now share `GridShape::line_traversal`, agreeing on every cell on BOTH grid kinds.

- [ ] **Step 5: Full server gate** — `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`.

- [ ] **Step 6: Commit** `fix(server/scene): route Room::publish's movement gate through GridShape (hex-correct drag path) [sec]`.

- [ ] **Step 7: Mandatory security buddy-check** — two independent opus-tier reviewers confirm: (a) byte-identical for square scenes (every existing gate test unchanged); (b) the hex path gates on hex cells matching `execute_move` + the hex-aware `visible_cells_cached` mask; (c) no new path widens the mask (fail-closed preserved); (d) `publish` and `execute_move` genuinely agree on hex, closing the invariant violation.

---

## Task 14f: Admin user creation + GM member-add surface `[sec]` — unblocks Task 14c

**Why (found by 14c's Step-1 investigation, dispatcher-verified against source):** no second user account can exist in the shipped binary, so Task 14c has no player to test with. This is a product gap, not a test gap — a hosted Shadowcat instance today can never have a second player.
- No registration route exists anywhere in the table (`http/mod.rs:62-140`).
- The only creation path, `create_admin_if_none` (`data/sqlite.rs:542-561`), is SQL-guarded `WHERE NOT EXISTS (… server_role='admin')` and yields `None` once an admin exists.
- `Repository::create_user` (`sqlite.rs:416`) has no production caller — every `http/` hit is inside the `#[cfg(test)]` module that begins at `http/mod.rs:153`. No CLI subcommand, no client UI.
- Even a second account would not be a player: `permission_context` (`sqlite.rs:394-414`) short-circuits `ServerRole::Admin → WorldRole::Gm` for every world, and GMs bypass the movement gate (`ws/room.rs:215`).
- The JOIN half already works: `POST /api/worlds/{id}/members` (`routes.rs:371-388`, `require_gm`-gated) seats an existing user — it just has no user id to seat.

**Design decisions (dispatcher-made; the buddy-check must scrutinize each):**
1. `POST /api/users` — **admin-only**. Body `{username, password, server_role?}`, defaulting to `ServerRole::User`. Returns `{id, username, server_role}` and NEVER the password hash. Requires a new `require_admin` guard (only `require_gm` exists today, `routes.rs:267`) — it must gate on `ServerRole::Admin`, never on world role.
2. `GET /api/users` — **admin-only** listing, for the admin panel. Deliberately NOT exposed to GMs.
3. Extend `add_member` to accept a **username** as an alternative to a `user: Uuid`, keeping the existing UUID form working. Rationale: a GM needs to seat a player without a user-directory endpoint, so the GM surface takes the name the admin issued rather than being handed the whole user list. Tradeoff accepted and flagged: a GM can probe for username existence — strictly less exposure than a directory, and GMs are already trusted with world-wide authority. **A GM must never be able to set a SERVER role through this path** — `WorldRole` only.
4. Client: an admin-only user-management section in the settings module (`src/modules/settings`), and a GM member-add affordance beside the existing `listMembers` (`src/client/shell/src/lib/api.ts:47`).

**Security requirements (the `[sec]` bar):** password hashed through the existing `auth::password::hash_password`, never logged or returned; duplicate username rejected cleanly (unique constraint, not a 500); non-admin callers get 403 on both new routes; the username branch of `add_member` cannot escalate `ServerRole`; no PII in tests — synthetic names and RFC 2606 domains only.

- [ ] **Step 1:** server routes + `require_admin` guard, with tests covering the authz matrix (admin allowed; GM, player, and anonymous each rejected on each new route).
- [ ] **Step 2:** the `add_member` username branch + tests, including the escalation-refusal case.
- [ ] **Step 3:** client UI for both surfaces + tests.
- [ ] **Step 4:** gates — `cargo test --all-targets`, `cargo clippy --all-targets -- -D warnings`, `pnpm -r test && pnpm -r typecheck && pnpm -r lint`.
- [ ] **Step 5:** commit (stage by explicit path, never `git add -A`).
- [ ] **Step 6:** mandatory two-reviewer opus buddy-check on the authz surface.

---

## Task 14g: Replace username-seating with an invite/accept flow `[sec]`

**Why:** Task 14f's `add_member`-by-username branch (design decision #3 — the dispatcher's, and wrong) returns 404 for an unknown username and 204 for a known one, making it a **username-existence oracle**. It is not GM-limited in any meaningful sense: `create_world` (`routes.rs:437`) requires only `AuthUser`, so any authenticated account can create a world, become its GM, and probe arbitrary usernames. On a hit the target is silently seated into the prober's world, which also hands over the victim's user UUID via `list_members`. This directly contradicts the codebase's explicit anti-enumeration stance — `anti_enumeration_phc` (`routes.rs:130-160`) spends a full constant-time Argon2 verify on unknown-user logins purely to keep username existence secret — and undercuts the stated intent at `routes.rs:319` that a world GM is "deliberately never handed a server-wide user directory."

Returning a uniform 204 does not fully close it: seating-on-hit is itself observable via `list_members`. The disclosure is inherent to naming a target, so the fix removes naming.

**Shape:** a GM mints an invite for their own world; the invited user redeems it from their own session. The GM never names a user, and nobody is seated without consent.
- `POST /api/worlds/{id}/invites` — GM of that world only. Body `{role: WorldRole}`. Returns a single-use invite code. `WorldRole` only — an invite must never be able to confer a `ServerRole`.
- `GET /api/worlds/{id}/invites` + `DELETE /api/worlds/{id}/invites/{code_id}` — GM of that world: list and revoke.
- `POST /api/invites/{code}/accept` — any authenticated user; seats the caller into the invite's world at the invite's role.
- **Remove the `add_member` username branch** added in 14f. Keep the existing `user: Uuid` form (unchanged, still GM-gated) and 14f's 404/422 fixes for it.

**Security requirements:**
- The code is a bearer credential: generate from a CSPRNG with ≥128 bits of entropy; store only a hash and compare in constant time, mirroring how passwords are handled.
- Redemption failures must be **uniform** — invalid, expired, revoked, and already-used must be indistinguishable to the caller, or the oracle simply moves.
- Accepting must disclose nothing about a world the caller has no invite for.
- Single-use, with an expiry. Revocation takes effect immediately.
- Only a GM **of that world** may mint, list, or revoke its invites; verify a GM of world A cannot touch world B's.
- Brute-forcing a code should be infeasible at the stated entropy — state the arithmetic rather than asserting it.

**Also fold in (sec-14f-a Minor, same area):** `create_user_unique`'s doc comment states as an invariant that usernames are ASCII-restricted at the HTTP boundary, but `/api/setup` (`routes.rs:200`) and `bootstrap_admin` (`setup.rs:37`) reach `create_admin_if_none` without validation. A non-ASCII first admin is not case-folded by SQLite's ASCII-only `NOCASE`, so a homoglyph (`аdmin` vs `admin`) cannot collide and is indistinguishable in a roster — the exact impersonation the policy exists to prevent. Apply `validate_username` on those paths so the documented invariant is true at every insertion point.

- [ ] **Step 1:** migration + repository layer (invite mint/lookup/revoke/consume), single-statement consume so redemption cannot double-seat under concurrency.
- [ ] **Step 2:** routes + the full authz matrix (GM-of-world allowed; GM-of-other-world, plain player, anonymous each rejected on each route), plus uniform-failure tests for redemption.
- [ ] **Step 3:** remove the username branch; update 14f's tests accordingly.
- [ ] **Step 4:** the ASCII-validation fix above.
- [ ] **Step 5:** client surfaces — GM mints/copies/revokes an invite; invitee redeems one.
- [ ] **Step 6:** gates — `cargo test --all-targets`, `cargo clippy --all-targets -- -D warnings`, `pnpm -r test && pnpm -r typecheck && pnpm -r lint`.
- [ ] **Step 7:** commit (stage by explicit path) + mandatory two-reviewer opus buddy-check.

**Task 14c depends on this** — the player account it needs is now seated via invite rather than by username.

---

## Task 14h: Ungate the player-facing scene tools `[sec]` — unblocks 14c

**Why (found by 14c's second attempt, dispatcher-verified at source):** a non-GM has NO scene tools at all. `ToolRail.svelte:78` wraps the entire rail — select/move included — in `{#if isGm}`, and `new ToolController` exists in exactly one non-test place (`ToolRail.svelte:11`) inside that gated component. With no active tool a player's canvas drag falls through to camera pan. Consequence: the whole server-authoritative movement stack (M9a walls, M10e-4 vision gating, the M1 executor, and this campaign's hex work) has **no client path a player can reach**.

**Decision (user):** a non-GM gets **select/move, measure, and ping**. Every authoring tool — wall, region, draw, template, place — stays GM-only.

- [ ] **Step 1:** split the rail's gating per-tool rather than wrapping the whole component. The `ToolController` must be constructed for every user; only the authoring entries are GM-conditional.
- [ ] **Step 2:** verify each ungated tool's write path is one the server already polices for a non-GM — select/move → `Room::publish`'s gate; measure route-commit → `moveRequest`/`execute_move`; ping → the existing per-user broadcast. **If any ungated tool can write something the server does NOT gate for a non-GM, STOP and report** — that is a privilege hole, not a UI change.
- [ ] **Step 3:** tests — a non-GM sees exactly {select/move, measure, ping} and NO authoring tool; a GM still sees the full rail. Assert the absent ones by negative assertion, not by counting.
- [ ] **Step 4:** gates + commit (stage by explicit path) + mandatory two-reviewer opus buddy-check (this is an authz surface).

---

## Task 14i: Token ownership — actor-inherited with a per-token override `[sec]` — unblocks 14c

**Why:** no UI can give a player a token they can write to. `buildTokenDoc` sets `owner: null` and `permissions.default = "observer"` (READ only, `permission.rs`), `Update` needs WRITE_FIELDS, and the server never stamps `owner` on Create. The only `permissions` write anywhere in `src/modules` is the actor name-privacy toggle. So the server's owner checks currently have nothing to check.

**Decision (user):** a token linked to an actor **inherits that actor's ownership**; a GM may **override on the individual token**. Ownership lives with the character, assigned once rather than per placed token, and unlinked or one-off tokens stay assignable.

**Resolution rule:** `effective_owner(token) = token's own override, else the linked actor's owner`. **Resolve this SERVER-SIDE at authz time**, not by stamping a copy at create time — a stamped copy drifts the moment actor ownership changes, and a client-side rule can be bypassed. Mirror the existing actor-join precedent (`token_vision_floors` already resolves through the token→actor link, and `resolveTokenActor` is the client semantics the server must equal — verify against that SOURCE, not a paraphrase).

- [ ] **Step 1:** server — effective-owner resolution on the authz path, fail-closed on a degenerate/missing link (no owner ⇒ no write, never a default-allow).
- [ ] **Step 2:** client — a GM-facing owner control (per-token override; actor ownership assigned on the actor).
- [ ] **Step 3:** tests — a player can write `/engine/x,y` on a token they effectively own and CANNOT on one they don't; inheritance and override each pinned; changing the actor's owner changes the token's effective owner with no re-stamp.
- [ ] **Step 4:** gates + commit + mandatory two-reviewer opus buddy-check.

---

## Task 14c: Client end-to-end test — a NON-GM player's illegal hex move is rolled back by the server

**Depends on Tasks 14f, 14g, 14h and 14i.** The Step-1 observable is already BUILT (commit `a98715b`: `data-token-positions` on the stage host, fed by `Stage.svelte`'s existing `onDocs` pass).

**TRAP — the test can pass for the wrong reason.** `publish`'s movement gate runs BEFORE `apply_intent`, so a PERMISSION denial and a WALL denial are both `Forbidden` and are indistinguishable client-side. A spec that only asserts "the move was rolled back" therefore proves nothing about the wall. **Include a control leg:** on a `movementRestriction: "unrestricted"` scene where only `blocks_move` can reject, a legal drag must SUCCEED — with the same token, the same player, the same session. Without that, an ownership regression reads as a passing wall test.

Step 1's investigation is already DONE — do not redo it:
- Account creation is 14f's admin-gated `POST /api/users`. Seating is 14g's invite flow: the GM mints via `POST /api/worlds/{id}/invites` `{role}`, and the invitee redeems from their OWN session via `POST /api/invites/accept` `{code}` (the code is in the BODY, not the URL). `add_member`-by-username no longer exists — a GM cannot seat a player by naming them, which is the point of the flow.
- **The rollback observable does NOT exist and `data-last-move-outcome` cannot serve this test.** `WorldSession` emits `onMoveOutcome` only off the `moveRequest` promise (`worldSession.svelte.ts:255-282`) — i.e. the measure-tool route-commit. The select-tool DRAG writes `/engine/x,y` as a plain optimistic Update and emits nothing; `data-token-count` is a count, not a position. A minimal test-only `data-*` position signal added to `Stage.svelte`'s existing `onDocs` pass (`:151`) closes this, mirroring the `data-last-ping` / `data-last-move-outcome` pattern.

**Files:**
- Create: `src/client/shell/e2e/hex-movement.spec.ts`
- Modify: `src/modules/stage/src/Stage.svelte` (the position observable above)

**Interfaces:** No production MOVEMENT change — depends on Task 14a (hex authoring UI) and Task 14d (hex-correct `publish` gate), plus the already-landed `ToolRail` fix (commit `73b44bd`). This is the FIRST multi-session (two-account) e2e in the suite: it proves a NON-GM player, on a hex scene authored through the real UI, has an illegal token move gated by the server. GMs bypass the movement gate entirely (`room.rs:215`), so a GM-only spec cannot prove server-side gating — hence the player account. The existing specs authenticate only as the world-owner GM; this task establishes the player-account pattern.

Key facts to build on:
- A non-GM player crossing a `blocksMove` WALL is rejected by `Room::publish` regardless of restriction mode (`blocks_move` is checked unconditionally for non-GM, before the `Unrestricted` short-circuit) — the simplest reliably-automatable gated scenario. The hex-SPECIFIC mask/line-traversal gate that Task 14d fixes is unit/integration-proven at the server level in Task 14d; 14c's job is the full-stack player round-trip, which the wall path exercises without needing a vision/lighting fixture.
- The select/move DRAG writes `/engine/x,y` via an optimistic Update; on server rejection the client rolls back, reverting the token's committed position.

- [ ] **Step 1: Establish the two-session + player-account pattern and the rollback observable — investigate FIRST; STOP-and-report if it needs infra beyond this task's reasonable scope.**
  - Determine how a second (non-GM) player account authenticates and JOINS the GM's world in Playwright (read the auth/login flow + how a non-owner joins a world — invite/join path, or any logged-in user joins by world id/name?). Use a second `browser.newContext()`/page for the player session.
  - Determine how to OBSERVE that the player's dragged token was rolled back (its committed `/engine/x,y` never crossed the wall). If no DOM-observable for a token's position exists, add a MINIMAL, test-only one, mirroring the existing stage `data-*` signal pattern (e.g. a tracked-token position attribute) — NOT a movement change.
  - If the player-join flow or the rollback observable genuinely needs infrastructure beyond a focused addition (e.g. no non-owner can join a world yet), STOP and report — a real product gap for a design decision, not something to fake.

- [ ] **Step 2: Write the e2e** — GM session: log in, create world, author a hex scene via Task 14a's grid-kind control, draw a wall, ensure a token the player owns exists on one side (confirm how token ownership is assigned — GM places a player-owned token, or the player places their own via the real ownership flow). Player session: join the same world, select that token, drag it so its path crosses the wall. Assert the server rejected it — the token's committed position never crossed the wall (via the Step-1 observable), i.e. the optimistic drag was rolled back.

- [ ] **Step 3: Run** `pnpm --filter <shell package> exec playwright test hex-movement` (confirm the shell package's real filter name from its `package.json`). Iterate to a genuine pass; a failure revealing a real gap is a finding to report, never to paper over.

- [ ] **Step 4: Full client gate** `pnpm -r test && pnpm -r typecheck && pnpm -r lint`.

- [ ] **Step 5: Commit**

Stage only the files this task changed, by explicit path — never `git add -A`. Other agents' work and stray scratch files live in this tree.

```bash
git add src/client/shell/e2e/hex-movement.spec.ts   # plus any other file THIS task changed
git commit -m "test(client/e2e): a non-GM player's illegal hex-scene move is rolled back by the server

First two-session (GM + player) e2e in the suite. A player, on a hex scene
authored through the real game-settings grid-kind control, drags a token
across a wall; the server's Room::publish gate rejects the optimistic
Update and the client rolls the position back — proving server-side
movement authority holds end-to-end for a real player on a hex scene."
```

---

## Task 14e: Systematic hex-correctness sweep of the movement/vision/explored/region secrecy family `[sec]`

**Why:** the 14d buddy-check confirmed the original GridShape refactor (Tasks 1–13) was incomplete — its grep was scoped to `scene/` and the Task-6 parity gate only exercised strict-mode paths. A dispatcher audit of ALL of `src/server/src` found the "square index-range → `cell_center`" candidate-cell enumeration (and other square cell math) recurs, un-migrated, at several secrecy-relevant production sites. On a hex scene these compute cells in a square interpretation while the rest of the pipeline is hex — a fog-probe-widening / gameplay-correctness class of bug. This task closes the whole class in one coherent, parity-gated pass. `[sec]` throughout where it touches the movement/vision mask.

**Confirmed sites (dispatcher-verified against source):**
- `ExploredSet::mark_polygons` (`explored.rs:72–90`) — square index range + `(i+0.5)*cell` centers; feeds the `Revealed` movement gate + `pathfind`'s revealed union.
- `accumulate_visible_cells` (`mod.rs:2048–2051` range; `2067–2072` lenient corners) — the move/vision mask (center already hex; range + corners still square). The corners are the item Task 5 explicitly deferred and never revisited.
- `player_lit_mask` (`mod.rs:1589–1602`) — secrecy egress; Task 5b migrated the center but not the candidate range.
- `regions::rasterize` (`regions.rs:79–108`) enumeration + `execute_move`'s region `to_cell` (`move_exec.rs:297–298`) — region (impassable/arrest) gating on hex.
- `pathfinding.rs:547–550` A* search window — assess whether the square-derived window clips reachable hex cells.
- `navmesh.rs` `clip_to_visible_mask` / `los_smooth::chord_ok` / `truncate_at_arrest` — **added by 14e-6's audit; see Task 14e-7.** This audit originally excused them as "the continuous-model router, orthogonal to grid kind." That reasoning was backwards: grid kind and movement model are INDEPENDENT axes, so they combine (`hex` + `continuous`) rather than exclude. Independence is a reason to check a site, never a reason to skip it — the same inference error that let `ws/room.rs` (14d) ship square-on-hex.
- (Legitimate square, DO NOT touch: `SquareGrid`'s own impls behind the trait — `pathfinding.rs`'s free `cell_center`/`footprint_cells`/`cell_of`, `movement.rs`'s `supercover_cells` internals.)

**Design (the unifying primitives):** add to the `GridShape` trait:
- `fn cells_in_bounds(&self, min: vision::P, max: vision::P, cell: f64) -> Option<Vec<Cell>>` — candidate cells whose geometry could overlap the pixel-space AABB. `SquareGrid` returns EXACTLY today's `floor(min/cell)..=floor(max/cell)` index rectangle (byte-identical). `HexGrid` converts the 4 AABB corners via `cell_of`, takes the padded axial bounding box (safe superset — axial↔pixel is affine, so a pixel rectangle's axial preimage is a bounded parallelogram), and enumerates it. `None` on the same DoS-cap/degenerate conditions the existing per-site `MAX_CELLS_PER_POLYGON` checks enforce (preserve the cap).
- `fn cell_vertices(&self, c: Cell, cell: f64) -> Vec<vision::P>` — the cell's polygon vertices for leniency corner-clip tests. `SquareGrid` → the existing 4 corners; `HexGrid` → the 6 pointy-top hex vertices.

Every migrated site keeps its per-cell center/vertex membership test unchanged; only the candidate ENUMERATION and the corner geometry move behind the trait. Square behavior MUST stay byte-identical — extend the Task-6 frozen-fixture parity battery to cover each migrated site, and it must stay green.

### Task 14e-1: Add `cells_in_bounds` + `cell_vertices` to `GridShape` (both impls, unit-tested, not yet wired)
- Add both methods to the trait + `SquareGrid` (byte-identical to the current square math) + `HexGrid` (correct axial enumeration + 6 vertices). Unit tests: square `cells_in_bounds` equals the current `floor` rectangle for representative AABBs; hex `cells_in_bounds` is a superset that includes every cell whose center lies in the AABB and stays within a bounded pad; `cell_vertices` returns 4 (square) / 6 (hex) correct points. Full server gate + commit. Not yet `[sec]` (no wiring).

### Task 14e-2: Migrate the vision/movement mask enumeration `[sec]`
- Route `accumulate_visible_cells` (candidate range → `cells_in_bounds`; lenient corners → `cell_vertices`) AND `player_lit_mask`'s own bbox scan through the primitives. Preserve the `MAX_CELLS_PER_POLYGON` cap (now via `cells_in_bounds`' `None`). Square byte-identical (extend Task 6's parity battery + confirm green). Add hex tests: an out-of-mask hex move-cell is EXCLUDED (the reject direction 14d's test lacked), and a lenient-corner-clip hex cell qualifies. Mandatory opus buddy-check.

### Task 14e-3: Migrate `ExploredSet` through `GridShape` (Revealed mode) `[sec]`
- Thread a `&dyn GridShape` into `ExploredSet::mark_polygons` (candidate range → `cells_in_bounds`, centers → `cell_center`) so explored cells are hex-indexed on hex scenes; update every caller (the `Revealed` gate in `Room::publish` + `Room::execute_move`, and `SceneEcs::pathfind`'s revealed union). Persistence note: the `(i32,i32)` byte format is unchanged; a scene's grid kind is fixed, so its stored blob has one consistent interpretation (a GM switching a live scene square↔hex reinterpreting an existing blob is an accepted edge, logged). Square byte-identical. Add a hex `Revealed`-mode gate test (a move into an unexplored+unseen hex cell is rejected; an explored one is allowed). Mandatory opus buddy-check.

### Task 14e-4: Migrate region rasterization + region-cell lookup `[sec]`
- Route `regions::rasterize`'s candidate enumeration through `cells_in_bounds` (center test already via `GridShape` from Task 4) and `execute_move`'s region `to_cell` through `grid.cell_of`, so region (impassable/arrest) gating aligns to hex cells and the rasterize-side and lookup-side agree on hex. Square byte-identical (Task 6 rasterize fixtures stay green). Add a hex region-gate test (an impassable/arrest hex region stops a hex move at the right hex cell). Buddy-check ([sec] — arrest/impassable is a movement-authority gate).

### Task 14e-5: Pathfinding A* search window on hex
- Assess `pathfinding.rs:547–550`'s window: does the square-derived index range clip hex cells the router must reach? If so, derive the window via `cells_in_bounds`/axial bounds; if provably a safe superset for hex already, document why and add a test proving a hex route near the window edge still resolves. Full gate + commit.

### Task 14e-5b: Hex-admissible A* heuristic (route optimality on hex)
- Discovered during 14e-5: `astar_leg`'s `heuristic(grid.rule, next, goal)` uses a `DiagonalRule`-based square distance (manhattan/euclidean/chebyshev). On hex axial coords with opposite-sign deltas it OVERESTIMATES the true hex distance → non-admissible → A* returns VALID but SUBOPTIMAL (longer-than-shortest) hex routes. NOT a secrecy/authority bug (routes stay gate-allowed via `cell_enterable`); a route-optimality correctness gap.
- Fix: add `fn heuristic(&self, from: Cell, to: Cell) -> f64` to `GridShape`. `SquareGrid` returns the existing rule-based `heuristic(self.rule, ...)` (byte-identical). `HexGrid` returns the admissible axial (cube) distance `(|dq| + |dr| + |dq+dr|)/2`. Route `astar_leg` through `grid.shape.heuristic(next, goal)` instead of the free `heuristic(grid.rule, ...)`.
- INVARIANTS: the heuristic must stay admissible (≤ true cost) so A* stays optimal; it only guides search ORDER, never gates a cell, so the `route ⊆ gate-allowed` superset invariant is untouched. Square byte-identical (existing `astar_tests` green).
- Test: a hex route where the square heuristic would yield a suboptimal path — assert the returned route is the true shortest hex path (correct cost + length). Not `[sec]`.

### Task 14e-6: Final audit confirmation + comprehensive hex integration tests
- Re-run the whole-`src/server` grep for square cell-index/center math (`+ 0.5) * cell`, `/ cell).floor()`, raw `(i*cell, j*cell)` corner arrays) and confirm every remaining hit is a legitimate `SquareGrid` internal or genuinely grid-kind-agnostic. Add end-to-end hex integration tests spanning mask + Revealed + region + lenient on one hex scene, on the **grid-stepped** movement model (`navmesh.rs` is 14e-7's). `log()`-equivalent: record any deliberately-out-of-scope residual in `docs/TODO.md`. Full gate + commit.
- OUTCOME: the audit found one category-3 site family → Task 14e-7. This task's earlier framing of `navmesh.rs` as "explicitly out of scope" was the misclassification 14e-7 corrects.

### Task 14e-7: Route the continuous engine's cell indexing through `GridShape` `[sec]`
- Found by 14e-6's audit and verified against source. `navmesh.rs` indexes route samples with square `floor(p/cell)` at three sites on the CONTINUOUS routing path — `clip_to_visible_mask` (`navmesh.rs:356–363`), `los_smooth::chord_ok` (`452–457`), `truncate_at_arrest` (`533–534`) — each using the free `pathfinding::footprint_cells` / `movement::supercover_cells` rather than the trait methods, then testing membership in `mask` (hex-axial since 14e-2) and `RegionField` (hex-axial since 14e-3/14e-4).
- REACHABLE ON HEX: grid kind and movement model are independent axes. `resolve_grid_shape` (`mod.rs:712–722`) keys only on `SceneEngine.grid.kind`; `pathfind` (`mod.rs:1161`) dispatches only on `settings.movement_model`. `grid.kind:"hex"` + `movementModel:"continuous"` is a live scene. In the weighted branch the mismatch is inside ONE call chain: `mod.rs:1195` builds the hex-aware `euclid_shape`, `find` returns a hex-axial route, and `mod.rs:1213` feeds that route to `los_smooth`, which re-gates it with square indices.
- FAILURE DIRECTION — BOTH. Axial and square are different affine maps into the same `(i32,i32)` space, so membership tests are effectively arbitrary. Worked over-reveal case on `hex_open_scene` (size 50): a sample inside hex `(0,2)` (center ≈`(86.6,150)`) has square index `(1,3)`, and axial `(1,3)` is a different, VISIBLE hex — so an occluded `(0,2)` passes the mask check. Under-reveal (spurious `Unreachable`) is the commoner direction at distance. `clip_to_visible_mask` is the ONLY fog gate on the pure-polyanya branch (the polyanya search itself is mask-blind) ⇒ genuine preview-route information disclosure: route shape reveals unseen wall/obstacle layout.
- BLAST RADIUS BOUNDED (not a movement-authority bug): `move_exec`/`gate_walk` and the `publish` gate are hex-correct (14d/14e-3), so no movement into fog. Preview `PathResult` only, plus dishonest arrest/terrain preview on hex.
- Fix: thread `&dyn GridShape` into all three functions; `to_cell` → `grid.cell_of`, `footprint_cells` → `grid.footprint_cells`, `supercover_cells` → `grid.line_traversal`. Square must stay byte-identical (Task-6 parity battery unchanged, no fixture edits).
- Tests: hex + continuous regression coverage for each of the three sites — a mask-occluded sample truncating the clip, a chord refused by `chord_ok`, and an arrest landing on the correct axial cell. Each must be shown to fail under square indexing (non-vacuity), matching 14e-2/14e-3's discrimination standard.
- `[sec]` + mandatory opus buddy-check (two reviewers).

---

## Task 15: Reviewed skill-update gate

**Files:**
- Modify: `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md`

**Interfaces:** Documentation only — the mandatory CLAUDE.md reviewed skill-update gate for this whole plan.

- [ ] **Step 1: Update the scene-rendering skill**

Document the `GridShape` abstraction (`grid_shape.rs`, `SquareGrid`/`HexGrid`, `resolve_grid_shape`), note that `movement.rs`/`pathfinding.rs`/`accumulate_visible_cells`/`regions.rs::rasterize` now all route through it instead of hardcoded square math, and note the frozen-fixture parity gate (Task 6) as the regression-safety mechanism for this refactor — mirroring how the skill already documents the M10f-2 `gate_walk`/`execute_move_kingstep_oracle` precedent this plan's own regression strategy is modeled on. Also record: (1) the new client authoring seam — `GameSettingsPanel`'s per-scene grid-kind/size control (Task 14a) is the product path that makes a hex scene reachable; (2) `Stage.svelte`'s `data-last-move-outcome` observability signal (Task 14b), fed by the measure-tool route-commit `moveRequest` path (now reachable after the `ToolRail`→`moveRequest` production fix, commit `73b44bd`); (3) the **`MoveStream` wire-gap** — server `MoveOutcome.truncated` (move_exec.rs) is never copied into `ServerMsg::MoveStream`, so the client infers truncated-vs-executed by comparing the mover's resolved `stop` against the requested goal (an arrest exactly AT the goal reads "executed" by design); a future consumer needing the authoritative bit should add it to the mover's frame like `mover_vision`/`cost`; (4) **both movement gates now route through `GridShape::line_traversal`** — `Room::publish`'s M10e-4 gate (Task 14d) as well as `execute_move` — closing the prior square-on-hex gap on the drag path and restoring the "publish and execute_move agree on every cell" invariant on both grid kinds.

- [ ] **Step 2: Dispatch `shadowcat-spec-reviewer` to confirm the skill diff is accurate**

Per CLAUDE.md's mandatory reviewed skill-update gate.

- [ ] **Step 3: Fix any findings, re-verify**

- [ ] **Step 4: Commit**

```bash
git add .claude/skills/shadowcat-codebase-scene-rendering/SKILL.md
git commit -m "docs(skills): document the GridShape abstraction + hex-grid movement support

Reviewed by shadowcat-spec-reviewer: PASS."
```

---

## Self-Review

**Spec coverage:** H1 (GridShape abstraction, not parallel modules) → Tasks 1-5, 12. H2 (SquareGrid/HexGrid concrete impls) → Task 1 (Square), Tasks 8-11 (Hex). H3 (frozen-fixture parity before cutover) → Task 6 (proof) + Task 7 (cutover/cleanup), sequenced strictly before Tasks 8-14 per the design doc's explicit ordering. H4 (uniform hex cost) → Task 9. H5 (no hex corner-tie bug class) → implicitly satisfied by Task 9's uniform-neighbor design (no parity/rule branching exists to have a corner-tie bug in). H6 (clean-room hex line-traversal) → Task 10. H7 (footprint radius stays scene-space) → Task 11 (the `r_scene` parameter is unchanged from the square signature). H8 (resolve GridShape from `grid.kind`, fail-closed) → Task 12. Testing section → Tasks 6 (parity), 13 (hex integration), 14a-c (hex authoring UI + move-outcome signal + client e2e round-trip).

**Scope boundary correction (Tasks 14a-d):** the design doc's "no client changes" boundary assumed the client was already fully hex-authorable; executing Task 14 disproved that (no GM control sets `grid.kind`) AND surfaced that the design's server work itself was incomplete. The boundary is deliberately extended: 14a (client authoring control over the existing wire field), 14b (client move-outcome observability signal), **14d (a SERVER fix the original plan missed — `Room::publish`'s movement-restriction gate still used square `supercover_cells` on the drag path; `[sec]`)**, 14c (a two-session GM+player e2e proving a real player's illegal hex move is server-gated). No new grid kind beyond square/hex; no new wire type. 14d is a genuine correction to the design's own "give the server hex movement authority" goal — the drag path (the primary player-movement path) was not covered because Task 3's grep was scoped to `scene/` and missed `ws/room.rs`. Also fixed in passing: a shipped production bug (`ToolRail` never wired `moveRequest`, commit `73b44bd`). All logged in the design-doc §6 addendum; client-scope + 14d execution user-consented.

**Placeholder scan:** No TBD/TODO in any task. Tasks 8/9's `unimplemented!()` stubs are intentional, explicitly-justified mid-plan scaffolding (each is completed by name in the very next task), not unresolved placeholders — flagged explicitly in Task 8's own text as a "confirm this compiles clean" checkpoint. Task 13/14's illustrative test snippets are explicitly flagged as needing confirmation against real fixture-helper names before finalizing, per this codebase's own established plan-writing convention for not-yet-confirmed e2e/integration scaffolding (matching how the approved design doc's own Task 39 precedent in the phase1-cleanup-burndown plan handled the same situation).

**Type consistency:** `GridShape` trait signature (`cell_center`, `cell_of`, `neighbors_with_cost`, `line_traversal`, `footprint_cells`) is defined once in Task 1 and used identically by name across every subsequent task (2-5, 8-12) — no renaming drift. `Cell = (i32, i32)` is reused from `pathfinding::Cell` throughout, never redefined. `resolve_grid_shape`'s `Box<dyn GridShape>` return type is introduced in Task 12 and is the terminal form every call site converges on.

**Sequencing:** Tasks 1-7 (square-only, parity-then-cutover) MUST land before Tasks 8-14 (hex-specific), per the design doc's H3 regression-safety decision — this ordering is load-bearing, not incidental, and is called out explicitly in Task 7's own framing ("`HexGrid` is only built and wired in once that parity is proven").
