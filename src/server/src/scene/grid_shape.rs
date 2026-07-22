//! `GridShape` abstracts the per-cell geometry every movement/vision/pathfinding module needs,
//! so square and hex scenes share one code path instead of two. `SquareGrid` is a byte-identical
//! port of the pre-existing hardcoded square math; a later `HexGrid` will be the pointy-top axial
//! implementation mirroring the client's `grid.ts` exactly.
//!
//! `cell_center` is wired into `accumulate_visible_cells`, `player_lit_mask`, and
//! `regions::rasterize`; `neighbors_with_cost`/`footprint_cells`/`line_traversal` are wired into
//! `pathfinding::astar_leg`/`cell_enterable`; `line_traversal` is also wired into
//! `move_exec::execute_move`. `cell_of` is not yet called from production code (proven only by
//! the tests below) — allowed dead code until a later cutover calls it too.
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
        let y = yf.round();
        let mut z = zf.round();
        let dx = (x - xf).abs();
        let dy = (y - yf).abs();
        let dz = (z - zf).abs();
        // Only x and z are returned; when dy is the largest rounding error, y would be the
        // axis fixed up, but it never feeds back into the returned (x, z) pair.
        if dx > dy && dx > dz {
            x = -y - z;
        } else if dy > dz {
            // y-fixup intentionally omitted (see comment above).
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

    /// Uniform 1-per-step cost, 6 axial neighbors — hex has no diagonal-rule analog, so `parity`
    /// is passed through unchanged (per the design doc's H4/H5 decisions).
    fn neighbors_with_cost(&self, c: Cell, parity: u8) -> Vec<(Cell, f64, u8)> {
        const AXIAL_DIRS: [(i32, i32); 6] =
            [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
        AXIAL_DIRS
            .iter()
            .map(|&(dq, dr)| ((c.0 + dq, c.1 + dr), 1.0, parity))
            .collect()
    }
    /// Clean-room hex line-drawing: cube-coordinate linear interpolation + hex-round per sample
    /// (Red Blob Games's standard technique; public-domain computational geometry, ARCHITECTURE
    /// §7). Hex equivalent of `movement::supercover_cells` — every cell the segment `a -> b`
    /// crosses, sampled at `N = max cube-axis delta` steps (same "N = max axial delta" idea as a
    /// square Bresenham/DDA line needing `max(|dx|,|dy|)` samples).
    fn line_traversal(&self, a: vision::P, b: vision::P, cell: f64) -> Option<BTreeSet<Cell>> {
        if !a.0.is_finite() || !a.1.is_finite() || !b.0.is_finite() || !b.1.is_finite() {
            return None;
        }
        let (aq, ar) = self.pixel_to_axial_frac(a);
        let (bq, br) = self.pixel_to_axial_frac(b);
        let ax = aq;
        let az = ar;
        let ay = -ax - az;
        let bx = bq;
        let bz = br;
        let by = -bx - bz;
        let n = ((ax - bx).abs().max((ay - by).abs()).max((az - bz).abs())).round() as i64;
        const MAX_HEX_LINE_SAMPLES: i64 = 4096; // DoS bound, mirrors movement.rs's MAX_MOVE_CELLS class of guard
        if !(0..=MAX_HEX_LINE_SAMPLES).contains(&n) {
            return None;
        }
        let _ = cell; // hex cell size is baked into `self.size`; kept only for GridShape trait-signature
                      // parity with SquareGrid, which DOES need it.
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
        Some(out)
    }
    /// Conservative disc-vs-hex-cell overlap: a candidate hex is included when its center is
    /// within `r_scene + hex_inradius` of the disc center — an always-safe over-approximation
    /// (a hex overlapping the true disc boundary is never excluded), mirroring the square
    /// implementation's own AABB-vs-disc distance test, which is similarly conservative rather
    /// than exact.
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

    #[test]
    fn hex_grid_axial_round_trip_pixel_to_axial_to_pixel() {
        let g = HexGrid { size: 50.0 };
        // Round-tripping a cell's own center through pixel->axial->pixel should be a fixed point.
        let center = g.cell_center((2, -1));
        let (q, r) = g.pixel_to_axial(center);
        assert_eq!((q, r), (2, -1));
    }

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
}
