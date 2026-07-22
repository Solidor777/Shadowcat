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

Written and executed mainline in this session (Sonnet 5), per explicit user choice at the writing-plans tier-switch checkpoint — no dedicated plan-writer or execution-tier model switch.

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

## Task 14: Client end-to-end test — hex-scene move genuinely gated server-side

**Files:**
- Create or modify: `src/client/shell/e2e/hex-movement.spec.ts` (Playwright, matching this repo's established e2e conventions — reuse `loginAsGmAndEnterTestWorld`-style helpers per the existing e2e specs in `src/client/shell/e2e/`)

**Interfaces:** No production client code change — `grid.ts`'s hex math is already correct (per the design doc's context section). This test proves a hex scene's move request now genuinely round-trips through real server-side gating (Tasks 1-13) instead of the server silently treating it as square underneath.

- [ ] **Step 1: Read an existing e2e movement spec for conventions**

Read an existing scene/movement-related Playwright spec in `src/client/shell/e2e/` for setup/teardown/world-creation conventions (per this repo's established pattern, already used by prior e2e specs like `panels.spec.ts`).

- [ ] **Step 2: Write the failing e2e test**

```typescript
test("a move request on a hex scene is gated by the real server, not just client math", async ({ page }) => {
  await loginAsGmAndEnterTestWorld(page); // match the repo's existing e2e setup helper
  await createHexSceneWithWall(page); // via the scene browser UI + scene-tools wall-drawing, per this repo's e2e conventions for scene setup

  // Attempt a move that a correct hex-aware server gate must reject (crosses the wall).
  const result = await attemptTokenMoveAcrossWall(page);
  await expect(result).toBeRejectedOrTruncated(); // exact assertion mechanics depend on this
  // repo's established move-request e2e pattern — match whatever existing square-scene move e2e
  // spec (if any) already asserts a wall-blocked move, mirroring its structure for hex.
});
```

(Confirm the exact helper names/assertion mechanics against this repo's REAL existing e2e conventions before finalizing — the snippet above is illustrative of the required behavior, not literal existing helpers, per this codebase's established plan-writing convention for e2e specs whose exact scaffolding isn't yet confirmed.)

- [ ] **Step 3: Run test to verify it fails or passes**

Run: `pnpm --filter @shadowcat/shell exec playwright test hex-movement`
Expected: PASS if Tasks 1-13 are correctly wired (the server now genuinely gates hex movement); if it fails, that means some Task 1-13 wiring gap remains — do not adjust this test to pass trivially, fix the actual gap.

- [ ] **Step 4: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 5: Commit**

```bash
git add src/client/shell/e2e/hex-movement.spec.ts
git commit -m "test(client/e2e): hex-scene move request is gated by the real server

Closes the coverage gap this whole plan exists to fix — a hex scene's
move request now genuinely round-trips through server-side wall/mask
gating instead of relying on the client's own (already-correct) math
being the only thing standing between a player and an illegal move."
```

---

## Task 15: Reviewed skill-update gate

**Files:**
- Modify: `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md`

**Interfaces:** Documentation only — the mandatory CLAUDE.md reviewed skill-update gate for this whole plan.

- [ ] **Step 1: Update the scene-rendering skill**

Document the `GridShape` abstraction (`grid_shape.rs`, `SquareGrid`/`HexGrid`, `resolve_grid_shape`), note that `movement.rs`/`pathfinding.rs`/`accumulate_visible_cells`/`regions.rs::rasterize` now all route through it instead of hardcoded square math, and note the frozen-fixture parity gate (Task 6) as the regression-safety mechanism for this refactor — mirroring how the skill already documents the M10f-2 `gate_walk`/`execute_move_kingstep_oracle` precedent this plan's own regression strategy is modeled on.

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

**Spec coverage:** H1 (GridShape abstraction, not parallel modules) → Tasks 1-5, 12. H2 (SquareGrid/HexGrid concrete impls) → Task 1 (Square), Tasks 8-11 (Hex). H3 (frozen-fixture parity before cutover) → Task 6 (proof) + Task 7 (cutover/cleanup), sequenced strictly before Tasks 8-14 per the design doc's explicit ordering. H4 (uniform hex cost) → Task 9. H5 (no hex corner-tie bug class) → implicitly satisfied by Task 9's uniform-neighbor design (no parity/rule branching exists to have a corner-tie bug in). H6 (clean-room hex line-traversal) → Task 10. H7 (footprint radius stays scene-space) → Task 11 (the `r_scene` parameter is unchanged from the square signature). H8 (resolve GridShape from `grid.kind`, fail-closed) → Task 12. Testing section → Tasks 6 (parity), 13 (hex integration), 14 (client e2e). Scope boundaries (no new grid kind beyond square/hex, no client changes) → respected throughout; Task 14 explicitly notes no production client code change.

**Placeholder scan:** No TBD/TODO in any task. Tasks 8/9's `unimplemented!()` stubs are intentional, explicitly-justified mid-plan scaffolding (each is completed by name in the very next task), not unresolved placeholders — flagged explicitly in Task 8's own text as a "confirm this compiles clean" checkpoint. Task 13/14's illustrative test snippets are explicitly flagged as needing confirmation against real fixture-helper names before finalizing, per this codebase's own established plan-writing convention for not-yet-confirmed e2e/integration scaffolding (matching how the approved design doc's own Task 39 precedent in the phase1-cleanup-burndown plan handled the same situation).

**Type consistency:** `GridShape` trait signature (`cell_center`, `cell_of`, `neighbors_with_cost`, `line_traversal`, `footprint_cells`) is defined once in Task 1 and used identically by name across every subsequent task (2-5, 8-12) — no renaming drift. `Cell = (i32, i32)` is reused from `pathfinding::Cell` throughout, never redefined. `resolve_grid_shape`'s `Box<dyn GridShape>` return type is introduced in Task 12 and is the terminal form every call site converges on.

**Sequencing:** Tasks 1-7 (square-only, parity-then-cutover) MUST land before Tasks 8-14 (hex-specific), per the design doc's H3 regression-safety decision — this ordering is load-bearing, not incidental, and is called out explicitly in Task 7's own framing ("`HexGrid` is only built and wired in once that parity is proven").
