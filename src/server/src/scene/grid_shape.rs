//! `GridShape` abstracts the per-cell geometry every movement/vision/pathfinding module needs,
//! so square and hex scenes share one code path instead of two. `SquareGrid` is a byte-identical
//! port of the pre-existing hardcoded square math; a later `HexGrid` will be the pointy-top axial
//! implementation mirroring the client's `grid.ts` exactly.
//!
//! `cell_center` is wired into `accumulate_visible_cells`, `player_lit_mask`, and
//! `regions::rasterize`; `cell_of`/`neighbors_with_cost`/`line_traversal`/`footprint_cells` are
//! not yet called from production code (proven only by the tests below) — allowed dead code until
//! a later cutover finishes replacing the hardcoded square math with calls through this trait.
#![allow(dead_code)]

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

/// Ported verbatim from `pathfinding.rs`'s private `step_cost`.
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
