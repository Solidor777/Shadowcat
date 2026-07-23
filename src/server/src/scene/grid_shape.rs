//! `GridShape` abstracts the per-cell geometry every movement/vision/pathfinding module needs,
//! so square and hex scenes share one code path instead of two. `SquareGrid` is a byte-identical
//! port of the pre-existing hardcoded square math; a later `HexGrid` will be the pointy-top axial
//! implementation mirroring the client's `grid.ts` exactly.
//!
//! `cell_center`/`cells_in_bounds` are wired into `accumulate_visible_cells`, `player_lit_mask`,
//! `explored::mark_polygons`, and `regions::rasterize`;
//! `neighbors_with_cost`/`footprint_cells`/`line_traversal` are wired into
//! `pathfinding::astar_leg`/`cell_enterable`; `line_traversal` is also wired into
//! `move_exec::execute_move`; `cell_of` is wired into `move_exec::execute_move`'s region-cell
//! lookup (and `HexGrid::cells_in_bounds`'s corner mapping). `cell_vertices` is proven only by the
//! tests below until the leniency corner-clip cutover calls it — allowed dead code until then.
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
    /// Every candidate cell whose geometry could overlap the pixel-space AABB
    /// `[min.0, max.0] × [min.1, max.1]`. A SUPERSET filter, not an exact one — the caller's own
    /// per-cell center/vertex membership test narrows it. `None` on a degenerate input (non-finite
    /// `min`/`max`/`cell`, `cell <= 0.0`) or an over-cap span (> `max_cells`), mirroring the
    /// existing per-site scans' fail-closed `skip` — callers fail closed on `None`. `max_cells` is
    /// the caller's own DoS bound (vision/explored scans pass `explored::MAX_CELLS_PER_POLYGON`,
    /// region rasterization passes the 40× tighter `regions::MAX_REGION_CELLS`) — never hardcoded
    /// here, so routing a tighter-capped caller through this primitive can't LOOSEN its bound.
    /// MUST never MISS a cell whose center lies inside the AABB (proven by test).
    fn cells_in_bounds(
        &self,
        min: vision::P,
        max: vision::P,
        cell: f64,
        max_cells: i64,
    ) -> Option<Vec<Cell>>;
    /// The inclusive cell-coordinate bounding box `(min_i, min_j, max_i, max_j)` of every cell whose
    /// geometry could overlap the pixel-space AABB `[min.0, max.0] × [min.1, max.1]`. Square:
    /// `floor(min/cell)..=floor(max/cell)` per axis. Hex: the axial bounding box of the 4
    /// pixel-corner preimages, padded by `HEX_BOUNDS_PAD` — the axial↔pixel shear means a
    /// pixel-space axis-aligned box is NOT axial-aligned, so a per-axis floor of the pixel min/max
    /// would CLIP reachable hexes. `cells_in_bounds` enumerates this box, and the A* router seeds
    /// its search window from it (then adds its own detour margin). Assumes finite `min`/`max`/`cell`
    /// (finiteness is the caller's guard — `find()` validates it upstream, `cells_in_bounds` checks
    /// it before delegating here); an extreme coordinate saturates via `f64 as i32`.
    fn cell_bounds(&self, min: vision::P, max: vision::P, cell: f64) -> (i32, i32, i32, i32);
    /// The cell's polygon vertices in scene coordinates (for leniency corner-clip tests): 4 for a
    /// square, 6 for a pointy-top hex.
    fn cell_vertices(&self, c: Cell, cell: f64) -> Vec<vision::P>;
}

/// Axial-box padding for `HexGrid::cells_in_bounds`. The axial↔pixel map is affine, so a pixel-space
/// axis-aligned rectangle's axial preimage is a bounded parallelogram; the integer axial bounding
/// box of that parallelogram's 4 corner images, expanded by ONE ring, is a safe superset of every
/// hex whose center lies in the rectangle. The +1 absorbs the cube-rounding fixup discrepancy
/// between converting the 4 corners (each cube-rounded) and a candidate cell's own center
/// (cube-rounded) — a corner conversion can differ from a naive per-axis round by up to 1 on one
/// axis. Source: Red Blob Games axial/cube coordinates (public-domain computational geometry).
const HEX_BOUNDS_PAD: i32 = 1;

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

    /// Byte-identical to the per-site square scans in `accumulate_visible_cells`/`explored.rs`/
    /// `player_lit_mask`: `floor(min/cell)..=floor(max/cell)` on each axis, row-major
    /// (`for i { for j }`). `f64 as i32` saturates on an extreme coordinate; the `saturating_mul`
    /// span check then fails closed against the caller-supplied `max_cells` bound.
    fn cells_in_bounds(
        &self,
        min: vision::P,
        max: vision::P,
        cell: f64,
        max_cells: i64,
    ) -> Option<Vec<Cell>> {
        if !min.0.is_finite()
            || !min.1.is_finite()
            || !max.0.is_finite()
            || !max.1.is_finite()
            || !cell.is_finite()
            || cell <= 0.0
        {
            return None;
        }
        let (i0, j0, i1, j1) = self.cell_bounds(min, max, cell);
        let w = i1 as i64 - i0 as i64 + 1;
        let h = j1 as i64 - j0 as i64 + 1;
        if w.saturating_mul(h) > max_cells {
            return None;
        }
        let mut out = Vec::new();
        for i in i0..=i1 {
            for j in j0..=j1 {
                out.push((i, j));
            }
        }
        Some(out)
    }

    /// Byte-identical to the pre-hex window computation (`floor(min/cell)`/`floor(max/cell)` per
    /// axis) so square routes are unchanged. `f64 as i32` saturates on an extreme coordinate,
    /// matching `cells_in_bounds`' own overflow behavior.
    fn cell_bounds(&self, min: vision::P, max: vision::P, cell: f64) -> (i32, i32, i32, i32) {
        (
            (min.0 / cell).floor() as i32,
            (min.1 / cell).floor() as i32,
            (max.0 / cell).floor() as i32,
            (max.1 / cell).floor() as i32,
        )
    }

    /// The 4 corners of cell `(i, j)`, in the exact order `accumulate_visible_cells`'s `corners`
    /// array uses so leniency corner-clip tests stay byte-identical.
    fn cell_vertices(&self, c: Cell, cell: f64) -> Vec<vision::P> {
        let (i, j) = c;
        vec![
            (i as f64 * cell, j as f64 * cell),
            ((i + 1) as f64 * cell, j as f64 * cell),
            (i as f64 * cell, (j + 1) as f64 * cell),
            ((i + 1) as f64 * cell, (j + 1) as f64 * cell),
        ]
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

    /// Enumerate the padded axial rectangle `cell_bounds` returns. A safe SUPERSET — never misses a
    /// cell whose center lies in the AABB (proven by test); the caller's per-cell center/vertex test
    /// filters it. Same caller-supplied `max_cells` span cap and fail-closed degenerate handling as
    /// the square scan.
    fn cells_in_bounds(
        &self,
        min: vision::P,
        max: vision::P,
        cell: f64,
        max_cells: i64,
    ) -> Option<Vec<Cell>> {
        if !min.0.is_finite()
            || !min.1.is_finite()
            || !max.0.is_finite()
            || !max.1.is_finite()
            || !cell.is_finite()
            || cell <= 0.0
        {
            return None;
        }
        let (q0, r0, q1, r1) = self.cell_bounds(min, max, cell);
        let w = q1 as i64 - q0 as i64 + 1;
        let h = r1 as i64 - r0 as i64 + 1;
        if w.saturating_mul(h) > max_cells {
            return None;
        }
        let mut out = Vec::new();
        for q in q0..=q1 {
            for r in r0..=r1 {
                out.push((q, r));
            }
        }
        Some(out)
    }

    /// Convert the 4 AABB corners to axial via `cell_of`, take the axial bounding box, and pad by
    /// `HEX_BOUNDS_PAD` (see its doc for the affine-preimage safety argument). A safe SUPERSET of
    /// every hex whose center lies in the pixel AABB — the axial↔pixel shear makes a per-axis floor
    /// of the pixel min/max a WRONG (clipping) box on hex. `cell` is unused (hex cell size is baked
    /// into `self.size`); kept for `GridShape` signature parity with `SquareGrid`.
    fn cell_bounds(&self, min: vision::P, max: vision::P, _cell: f64) -> (i32, i32, i32, i32) {
        let corners = [(min.0, min.1), (max.0, min.1), (min.0, max.1), (max.0, max.1)];
        let (mut min_q, mut min_r, mut max_q, mut max_r) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &corner in &corners {
            let (q, r) = self.cell_of(corner);
            min_q = min_q.min(q);
            min_r = min_r.min(r);
            max_q = max_q.max(q);
            max_r = max_r.max(r);
        }
        (
            min_q.saturating_sub(HEX_BOUNDS_PAD),
            min_r.saturating_sub(HEX_BOUNDS_PAD),
            max_q.saturating_add(HEX_BOUNDS_PAD),
            max_r.saturating_add(HEX_BOUNDS_PAD),
        )
    }

    /// The 6 pointy-top hex vertices around `cell_center(c)`, vertex `k` at angle `60·k − 30`
    /// degrees, radius = `self.size`. Mirrors `src/client/render/src/grid.ts`'s `hexLines` vertex
    /// convention exactly (Red Blob Games pointy-top) so client and server hex geometry agree.
    fn cell_vertices(&self, c: Cell, _cell: f64) -> Vec<vision::P> {
        let center = self.cell_center(c);
        (0..6)
            .map(|k| {
                let ang = std::f64::consts::PI / 180.0 * (60.0 * k as f64 - 30.0);
                (center.0 + self.size * ang.cos(), center.1 + self.size * ang.sin())
            })
            .collect()
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

    /// Independent reimplementation of the intended `floor(min/cell)..=floor(max/cell)` row-major
    /// rectangle, so the parity assertion isn't tautological with the impl under test.
    fn hand_floor_rect(min: vision::P, max: vision::P, cell: f64) -> Vec<Cell> {
        let i0 = (min.0 / cell).floor() as i32;
        let i1 = (max.0 / cell).floor() as i32;
        let j0 = (min.1 / cell).floor() as i32;
        let j1 = (max.1 / cell).floor() as i32;
        let mut out = Vec::new();
        for i in i0..=i1 {
            for j in j0..=j1 {
                out.push((i, j));
            }
        }
        out
    }

    /// The DoS bound the vision/explored scans pass; the cell-span tests exercise `cells_in_bounds`
    /// at this cap.
    const CAP: i64 = crate::scene::explored::MAX_CELLS_PER_POLYGON;

    #[test]
    fn square_cells_in_bounds_equals_hand_written_floor_rectangle() {
        let g = SquareGrid { cell: 100.0, rule: DiagonalRule::Chebyshev };
        // Origin-anchored 3×3.
        let (min, max) = ((0.0, 0.0), (250.0, 250.0));
        assert_eq!(g.cells_in_bounds(min, max, 100.0, CAP), Some(hand_floor_rect(min, max, 100.0)));
        // Straddling the origin (negative coords).
        let (min, max) = ((-150.0, -50.0), (50.0, 50.0));
        assert_eq!(g.cells_in_bounds(min, max, 100.0, CAP), Some(hand_floor_rect(min, max, 100.0)));
        // Wholly negative.
        let (min, max) = ((-330.0, -220.0), (-110.0, -140.0));
        assert_eq!(g.cells_in_bounds(min, max, 100.0, CAP), Some(hand_floor_rect(min, max, 100.0)));
    }

    #[test]
    fn square_cells_in_bounds_is_row_major() {
        let g = SquareGrid { cell: 100.0, rule: DiagonalRule::Chebyshev };
        let got = g.cells_in_bounds((0.0, 0.0), (150.0, 150.0), 100.0, CAP).unwrap();
        // for i { for j } → (0,0),(0,1),(1,0),(1,1).
        assert_eq!(got, vec![(0, 0), (0, 1), (1, 0), (1, 1)]);
    }

    #[test]
    fn cells_in_bounds_span_cap_returns_none() {
        // 3001×3001 ≈ 9.0M candidate cells > the 4M MAX_CELLS_PER_POLYGON cap → None.
        let g = SquareGrid { cell: 1.0, rule: DiagonalRule::Chebyshev };
        assert_eq!(g.cells_in_bounds((0.0, 0.0), (3000.0, 3000.0), 1.0, CAP), None);
        // A hex scene with an equally-huge AABB is capped by the same span check.
        let hx = HexGrid { size: 1.0 };
        assert_eq!(hx.cells_in_bounds((0.0, 0.0), (5000.0, 5000.0), 1.0, CAP), None);
    }

    #[test]
    fn cells_in_bounds_honors_the_caller_supplied_cap() {
        // A 3×3 (9-cell) AABB is accepted under the vision cap but rejected under a tighter
        // per-caller bound — the cap is the passed `max_cells`, never a hardcoded constant, so
        // routing a tighter-capped caller (region rasterization) through this primitive can't
        // loosen its bound.
        let g = SquareGrid { cell: 100.0, rule: DiagonalRule::Chebyshev };
        let (min, max) = ((0.0, 0.0), (250.0, 250.0));
        assert!(g.cells_in_bounds(min, max, 100.0, CAP).is_some());
        assert_eq!(g.cells_in_bounds(min, max, 100.0, 8), None, "9 cells > cap of 8");
        // Same for hex — the span cap is honored on both grid kinds.
        let hx = HexGrid { size: 50.0 };
        assert_eq!(hx.cells_in_bounds((0.0, 0.0), (200.0, 200.0), 50.0, 1), None);
    }

    #[test]
    fn cells_in_bounds_fails_closed_on_degenerate_input() {
        let g = SquareGrid { cell: 100.0, rule: DiagonalRule::Chebyshev };
        assert_eq!(g.cells_in_bounds((f64::NAN, 0.0), (10.0, 10.0), 100.0, CAP), None);
        assert_eq!(g.cells_in_bounds((0.0, 0.0), (f64::INFINITY, 10.0), 100.0, CAP), None);
        assert_eq!(g.cells_in_bounds((0.0, 0.0), (10.0, 10.0), 0.0, CAP), None);
        assert_eq!(g.cells_in_bounds((0.0, 0.0), (10.0, 10.0), -5.0, CAP), None);
        let hx = HexGrid { size: 50.0 };
        assert_eq!(hx.cells_in_bounds((0.0, 0.0), (10.0, f64::NAN), 50.0, CAP), None);
        assert_eq!(hx.cells_in_bounds((0.0, 0.0), (10.0, 10.0), 0.0, CAP), None);
    }

    #[test]
    fn hex_cells_in_bounds_includes_every_cell_whose_center_is_in_the_aabb() {
        let g = HexGrid { size: 50.0 };
        let (min, max) = ((0.0, 0.0), (200.0, 200.0));
        let got = g.cells_in_bounds(min, max, 50.0, CAP).expect("bounded, not over-cap");
        let got_set: BTreeSet<Cell> = got.iter().copied().collect();
        // The candidate set must be a SUPERSET of every hex whose center lies in the AABB.
        let mut center_in_count = 0;
        for q in -30..=30 {
            for r in -30..=30 {
                let ctr = g.cell_center((q, r));
                if ctr.0 >= min.0 && ctr.0 <= max.0 && ctr.1 >= min.1 && ctr.1 <= max.1 {
                    center_in_count += 1;
                    assert!(
                        got_set.contains(&(q, r)),
                        "hex ({q},{r}) center {ctr:?} in AABB but missing from cells_in_bounds"
                    );
                }
            }
        }
        assert!(center_in_count > 0, "fixture must exercise at least one in-bounds hex");
        // Stays bounded — a tight superset of a ~4×4-hex region, not a runaway scan.
        assert!(got.len() < 100, "hex candidate set should stay small for a small AABB");
    }

    #[test]
    fn square_cell_bounds_is_the_corner_floor_box() {
        // Byte-identical to the pre-hex A* window's `floor(min/cell)`/`floor(max/cell)` computation,
        // so square routes are unchanged.
        let g = SquareGrid { cell: 100.0, rule: DiagonalRule::Chebyshev };
        assert_eq!(g.cell_bounds((0.0, -250.0), (250.0, 50.0), 100.0), (0, -3, 2, 0));
        assert_eq!(g.cell_bounds((-330.0, -220.0), (-110.0, -140.0), 100.0), (-4, -3, -2, -2));
    }

    #[test]
    fn hex_cell_bounds_contains_a_cell_a_square_floor_window_would_clip() {
        // Hex (70,-70): the "off the square diagonal" axial (1,-1) direction. Its pixel x is
        // ~0.866·70·size, so a square `floor(x/cell)` q-bound (=60) sits far BELOW its axial q (=70)
        // — the pre-hex window clipped it, spuriously reporting a reachable route Unreachable.
        let g = HexGrid { size: 100.0 };
        let goal = (70, -70);
        let gc = g.cell_center(goal); // (~6062, -10500)
                                      // AABB of start (0,0) and the goal center.
        let (min, max) = ((0.0, gc.1), (gc.0, 0.0));
        let (i0, j0, i1, j1) = g.cell_bounds(min, max, 100.0);
        assert!(i0 <= goal.0 && goal.0 <= i1, "axial q {} must lie in [{i0},{i1}]", goal.0);
        assert!(j0 <= goal.1 && goal.1 <= j1, "axial r {} must lie in [{j0},{j1}]", goal.1);
        // The square floor of the pixel-x max caps the q-bound strictly below the goal's axial q:
        // this is exactly the clipping the hex-correct bounds avoid.
        assert!(
            (gc.0 / 100.0).floor() as i32 + 8 < goal.0,
            "a square floor(max_x/cell)+margin window would clip axial q={}",
            goal.0
        );
    }

    #[test]
    fn square_cell_vertices_returns_four_corners_in_order() {
        let g = SquareGrid { cell: 100.0, rule: DiagonalRule::Chebyshev };
        assert_eq!(
            g.cell_vertices((2, 3), 100.0),
            vec![(200.0, 300.0), (300.0, 300.0), (200.0, 400.0), (300.0, 400.0)]
        );
    }

    #[test]
    fn hex_cell_vertices_returns_six_points_at_radius_size() {
        let g = HexGrid { size: 50.0 };
        let center = g.cell_center((1, -2));
        let verts = g.cell_vertices((1, -2), 50.0);
        assert_eq!(verts.len(), 6, "a hex has 6 vertices");
        for &(x, y) in &verts {
            let d = ((x - center.0).powi(2) + (y - center.1).powi(2)).sqrt();
            assert!((d - 50.0).abs() < 1e-9, "each hex vertex sits at radius = size from center");
        }
    }
}
