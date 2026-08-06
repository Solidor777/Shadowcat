//! `GridShape` abstracts the per-cell geometry every movement/vision/pathfinding module needs,
//! so square and hex scenes share one code path instead of two. `SquareGrid` is a byte-identical
//! port of the pre-existing hardcoded square math; a later `HexGrid` will be the pointy-top axial
//! implementation mirroring the client's `Grid` class exactly.
//!
//! `cell_center`/`cells_in_bounds` are wired into `accumulate_visible_cells`, `player_lit_mask`,
//! `explored::mark_polygons`, and `regions::rasterize`;
//! `neighbors_with_cost`/`footprint_cells`/`line_traversal` are wired into
//! `pathfinding::astar_leg`/`cell_enterable`; `line_traversal` is also wired into
//! `move_exec::execute_move`; `cell_of` is wired into `move_exec::execute_move`'s region-cell
//! lookup (and `HexGrid::cells_in_bounds`'s corner mapping). `cell_vertices` is proven only by the
//! tests below until the leniency corner-clip cutover calls it — allowed dead code until then.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
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
    /// Admissible A* heuristic (lower bound on the true `neighbors_with_cost` path cost) from cell
    /// `from` to cell `to`. Guides search ORDER only — never gates a cell — so it cannot affect the
    /// `route ⊆ gate-allowed` invariant. `SquareGrid` returns the existing `DiagonalRule`-based
    /// square distance; `HexGrid` returns the axial (cube) hex distance, which the square distance
    /// OVERESTIMATES for opposite-sign axial deltas (non-admissible on hex → suboptimal routes).
    fn heuristic(&self, from: Cell, to: Cell) -> f64;
}

/// Axial-box padding for `HexGrid::cells_in_bounds`. The axial↔pixel map is affine, so a pixel-space
/// axis-aligned rectangle's axial preimage is a bounded parallelogram; the integer axial bounding
/// box of that parallelogram's 4 corner images, expanded by ONE ring, is a safe superset of every
/// hex whose center lies in the rectangle. The +1 absorbs the cube-rounding fixup discrepancy
/// between converting the 4 corners (each cube-rounded) and a candidate cell's own center
/// (cube-rounded) — a corner conversion can differ from a naive per-axis round by up to 1 on one
/// axis. Source: Red Blob Games axial/cube coordinates (public-domain computational geometry).
const HEX_BOUNDS_PAD: i32 = 1;

/// Byte-identical port of the pre-existing hardcoded square-grid math (`cell_center`/
/// `footprint_cells`, `movement::supercover_cells`, `astar_leg`'s 8-directional `dirs` +
/// `step_cost`). `cell` and `rule` are the scene's resolved
/// cell size and diagonal-cost rule.
pub(crate) struct SquareGrid {
    /// Cell size in scene units.
    pub cell: f64,
    /// Diagonal-cost rule (owns step cost + heuristic).
    pub rule: DiagonalRule,
}

impl GridShape for SquareGrid {
    fn cell_center(&self, c: Cell) -> vision::P {
        pathfinding::cell_center(c, self.cell)
    }

    fn cell_of(&self, p: vision::P) -> Cell {
        (
            (p.0 / self.cell).floor() as i32,
            (p.1 / self.cell).floor() as i32,
        )
    }

    fn neighbors_with_cost(&self, c: Cell, parity: u8) -> Vec<(Cell, f64, u8)> {
        const DIRS: [(i32, i32); 8] = [
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ];
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

    /// Byte-identical to the per-site square scans in `accumulate_visible_cells`/
    /// `ExploredSet::mark_polygons`/`player_lit_mask`: `floor(min/cell)..=floor(max/cell)` on each axis, row-major
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

    /// Byte-identical to the pre-existing free `pathfinding::heuristic(self.rule, ...)` — the same
    /// admissible+consistent `DiagonalRule`-based square estimate `astar_leg` used before the trait
    /// dispatch, so every square route (all 4 diagonal rules) is unchanged.
    fn heuristic(&self, from: Cell, to: Cell) -> f64 {
        pathfinding::heuristic(self.rule, from, to)
    }
}

/// Pointy-top axial hex grid (Red Blob Games convention), mirroring
/// the client's `Grid` class's hex math exactly — same coordinate formulas,
/// same `size` = outer-radius convention, so client and server cell indices always agree.
pub(crate) struct HexGrid {
    /// Hex size = OUTER radius (circumradius, center to vertex) in scene
    /// units — the client's `Grid.size` convention, not the across-flats width.
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
    /// is passed through unchanged.
    fn neighbors_with_cost(&self, c: Cell, parity: u8) -> Vec<(Cell, f64, u8)> {
        const AXIAL_DIRS: [(i32, i32); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
        AXIAL_DIRS
            .iter()
            .map(|&(dq, dr)| ((c.0 + dq, c.1 + dr), 1.0, parity))
            .collect()
    }
    /// Clean-room hex SUPERCOVER — every hex the segment `a -> b` touches, never a thin line.
    /// Hex equivalent of `movement::supercover_cells`, and conservative in the same direction:
    /// over-inclusion can only over-restrict a move, omission would let a move graze an unseen hex.
    ///
    /// Method (public-domain computational geometry; Red Blob Games cube
    /// coordinates for the axial↔cube map). `cell_of` is cube-rounding, i.e. nearest-center
    /// assignment, so a hex is the Voronoi cell of its center: the intersection of the six
    /// perpendicular-bisector half-planes against its six neighbors. Writing the fractional cube
    /// coordinates of `p` as `x,y,z` (affine in `p`, summing to 0), those bisectors are exactly the
    /// level sets `ψ₁ = x−y`, `ψ₂ = z−y`, `ψ₃ = x−z` at INTEGER values — three families of parallel
    /// lines that therefore carry every hex boundary, so `cell_of` is constant on each face of
    /// their arrangement. (The naive `x`/`y`/`z`-at-half-integer families do NOT: they bound the
    /// smaller inner hexagon where all three coordinates round independently, leaving the six
    /// cube-rounding fixup triangles — real parts of the hex — unbounded.) Collecting every integer
    /// `ψ` crossing parameter `t` and sampling the MIDPOINT of each resulting interval visits every
    /// face the segment passes through, hence every hex it enters.
    ///
    /// A fixed-count cube lerp cannot do this: it samples at a spacing of one full hex pitch, so it
    /// skips hexes the segment genuinely crosses — and when the cube-axis delta rounds to 0 it
    /// omits even the far ENDPOINT's own hex.
    ///
    /// A crossing point is additionally probed one `VERTEX_PROBE` perpendicular offset to each
    /// side, so a segment running exactly along a hex edge, or exactly through a hex vertex, emits
    /// the hexes flanking it — the same deliberate corner over-inclusion the square supercover
    /// documents, so a segment cannot thread an unseen hex along a boundary.
    ///
    /// `None` ⇒ caller must fail closed: a non-finite endpoint, or a span whose cube-axis delta
    /// exceeds `MAX_HEX_LINE_SAMPLES`.
    fn line_traversal(&self, a: vision::P, b: vision::P, cell: f64) -> Option<BTreeSet<Cell>> {
        if !a.0.is_finite() || !a.1.is_finite() || !b.0.is_finite() || !b.1.is_finite() {
            return None;
        }
        let (aq, ar) = self.pixel_to_axial_frac(a);
        let (bq, br) = self.pixel_to_axial_frac(b);
        let (ax, az) = (aq, ar);
        let ay = -ax - az;
        let (bx, bz) = (bq, br);
        let by = -bx - bz;
        // DoS bound on the hex-distance span, unchanged from the pre-supercover sampler. What it
        // bounds changed: the boundary-crossing count is `Σ|Δψₖ| + 3 ≤ 4n + 3` rather than the old
        // `n + 1` samples — same order, so the worst case is ~16k crossings at the same n cap.
        const MAX_HEX_LINE_SAMPLES: i64 = 4096;
        let n = ((ax - bx).abs().max((ay - by).abs()).max((az - bz).abs())).round() as i64;
        if !(0..=MAX_HEX_LINE_SAMPLES).contains(&n) {
            return None;
        }
        let _ = cell; // hex cell size is baked into `self.size`; kept only for GridShape trait-signature
                      // parity with SquareGrid, which DOES need it.

        let mut out = BTreeSet::new();
        // Endpoint hexes are always included, matching the square supercover's contract.
        out.insert(self.axial_round(aq, ar));
        out.insert(self.axial_round(bq, br));

        // Every integer crossing of each bisector coordinate ψ, as a parameter t in (0, 1).
        let mut ts: Vec<f64> = Vec::new();
        for (u0, u1) in [(ax - ay, bx - by), (az - ay, bz - by), (ax - az, bx - bz)] {
            let du = u1 - u0;
            if du == 0.0 {
                continue; // this family never crosses a boundary line
            }
            let (lo, hi) = if du > 0.0 { (u0, u1) } else { (u1, u0) };
            let k0 = lo.ceil();
            let k1 = hi.floor();
            if !k0.is_finite() || !k1.is_finite() || k1 < k0 {
                continue;
            }
            let steps = (k1 - k0) as i64;
            if steps > 4 * MAX_HEX_LINE_SAMPLES {
                // Fail closed rather than allocate an unbounded crossing list. Unreachable as
                // written: the `n <= MAX_HEX_LINE_SAMPLES` gate above already dominates it, since
                // |dpsi_k| <= 2n bounds `steps` at 8193 < 16384. Kept as a standalone backstop so
                // this loop stays bounded on its own terms if that gate is ever relaxed.
                return None;
            }
            for s in 0..=steps {
                let t = (k0 + s as f64 - u0) / du;
                if t > 0.0 && t < 1.0 {
                    ts.push(t);
                }
            }
        }
        ts.sort_by(|p, q| p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal));

        let d = (b.0 - a.0, b.1 - a.1);
        let at = |t: f64| (a.0 + d.0 * t, a.1 + d.1 * t);
        // Midpoint of each inter-crossing interval: `cell_of` is constant across the interval, so
        // this names exactly the hex the segment occupies there.
        let mut prev = 0.0_f64;
        for &t in ts.iter().chain(std::iter::once(&1.0)) {
            out.insert(self.cell_of(at((prev + t) * 0.5)));
            prev = t;
        }

        // Boundary-grazing over-inclusion (safe direction only): probe both sides of each crossing.
        let len = (d.0 * d.0 + d.1 * d.1).sqrt();
        if len > 0.0 {
            /// Perpendicular probe offset, as a fraction of the hex outer radius. Large enough to
            /// clear cube-rounding noise at scene-scale coordinates, small enough that it only ever
            /// pulls in a hex the segment is already touching to within a millionth of a cell.
            const VERTEX_PROBE: f64 = 1e-6;
            let off = self.size * VERTEX_PROBE;
            let nx = -d.1 / len * off;
            let ny = d.0 / len * off;
            // Endpoints are probed too: an endpoint sitting exactly ON a boundary produces its
            // crossing at t == 0 or t == 1, which is not an interior interval split, so the
            // midpoint pass alone would name only whichever side `cell_of` happens to pick.
            for &t in ts.iter().chain([0.0, 1.0].iter()) {
                let p = at(t);
                out.insert(self.cell_of((p.0 + nx, p.1 + ny)));
                out.insert(self.cell_of((p.0 - nx, p.1 - ny)));
            }
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
        let corners = [
            (min.0, min.1),
            (max.0, min.1),
            (min.0, max.1),
            (max.0, max.1),
        ];
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
    /// degrees, radius = `self.size`. Mirrors the client's `hexLines` vertex
    /// convention exactly (Red Blob Games pointy-top) so client and server hex geometry agree.
    fn cell_vertices(&self, c: Cell, _cell: f64) -> Vec<vision::P> {
        let center = self.cell_center(c);
        (0..6)
            .map(|k| {
                let ang = std::f64::consts::PI / 180.0 * (60.0 * k as f64 - 30.0);
                (
                    center.0 + self.size * ang.cos(),
                    center.1 + self.size * ang.sin(),
                )
            })
            .collect()
    }

    /// Admissible axial (cube) hex distance `(|dq| + |dr| + |dq+dr|)/2` — the exact minimum number
    /// of uniform 1-cost steps (`neighbors_with_cost`) between two hexes (Red Blob Games;
    /// public-source computational geometry), so it never overestimates the true path cost and A*
    /// stays optimal. Deltas widen to `i64` before the sum so a large-coordinate pair can't overflow
    /// `i32`. The square `DiagonalRule` distance the pre-trait code used here OVERESTIMATES this for
    /// opposite-sign deltas (e.g. Manhattan is 2× on the `(1,-1)` axial line), which is what made the
    /// old shared heuristic non-admissible on hex.
    fn heuristic(&self, from: Cell, to: Cell) -> f64 {
        let dq = to.0 as i64 - from.0 as i64;
        let dr = to.1 as i64 - from.1 as i64;
        ((dq.abs() + dr.abs() + (dq + dr).abs()) as f64) / 2.0
    }
}

/// Per-step move cost under `rule`, plus the carried parity for the next step.
///
/// Orthogonal steps always cost 1.0 and leave `parity` untouched. Diagonals cost per rule;
/// only `Alternating` consumes parity, charging 1.0/2.0 on alternate diagonals and flipping
/// the bit so the caller must thread the returned parity through consecutive steps.
///
/// The sole definition of this rule — `pathfinding::astar_leg` reaches it through the `GridShape`
/// trait's neighbor enumeration rather than duplicating it, so the A* cost and any other
/// consumer cannot drift apart. The client mirrors the same four rules in
/// the client's `Grid.distance`.
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
        let g = SquareGrid {
            cell: 100.0,
            rule: DiagonalRule::Chebyshev,
        };
        assert_eq!(
            g.cell_center((2, 3)),
            crate::scene::pathfinding::cell_center((2, 3), 100.0)
        );
    }

    #[test]
    fn square_grid_cell_of_floors_to_cell_index() {
        let g = SquareGrid {
            cell: 100.0,
            rule: DiagonalRule::Chebyshev,
        };
        assert_eq!(g.cell_of((250.0, 149.0)), (2, 1));
        assert_eq!(g.cell_of((-10.0, -1.0)), (-1, -1));
    }

    #[test]
    fn square_grid_neighbors_with_cost_matches_chebyshev_dirs_and_step_cost() {
        let g = SquareGrid {
            cell: 100.0,
            rule: DiagonalRule::Chebyshev,
        };
        let ns = g.neighbors_with_cost((0, 0), 0);
        assert_eq!(ns.len(), 8, "8-directional king-move expansion");
        // A diagonal neighbor costs 1.0 under Chebyshev.
        let diag = ns
            .iter()
            .find(|(c, _, _)| *c == (1, 1))
            .expect("diagonal neighbor present");
        assert!((diag.1 - 1.0).abs() < 1e-9);
        // An orthogonal neighbor costs 1.0 too.
        let ortho = ns
            .iter()
            .find(|(c, _, _)| *c == (1, 0))
            .expect("orthogonal neighbor present");
        assert!((ortho.1 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn square_grid_neighbors_with_cost_alternating_threads_parity() {
        let g = SquareGrid {
            cell: 100.0,
            rule: DiagonalRule::Alternating,
        };
        let ns = g.neighbors_with_cost((0, 0), 0);
        let diag = ns.iter().find(|(c, _, _)| *c == (1, 1)).unwrap();
        assert!(
            (diag.1 - 1.0).abs() < 1e-9,
            "first diagonal from parity 0 costs 1"
        );
        assert_eq!(diag.2, 1, "parity flips after a diagonal step");
        let ortho = ns.iter().find(|(c, _, _)| *c == (1, 0)).unwrap();
        assert_eq!(ortho.2, 0, "parity unchanged after an orthogonal step");
    }

    #[test]
    fn square_grid_line_traversal_matches_supercover_cells() {
        let g = SquareGrid {
            cell: 100.0,
            rule: DiagonalRule::Chebyshev,
        };
        let a = (50.0, 50.0);
        let b = (250.0, 250.0);
        assert_eq!(
            g.line_traversal(a, b, 100.0),
            crate::scene::movement::supercover_cells(a, b, 100.0)
        );
    }

    #[test]
    fn square_grid_footprint_cells_matches_pathfinding_footprint_cells() {
        let g = SquareGrid {
            cell: 100.0,
            rule: DiagonalRule::Chebyshev,
        };
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
            assert!(
                (cost - 1.0).abs() < 1e-9,
                "every hex step costs 1.0 uniformly"
            );
            assert_eq!(
                *parity, 3,
                "hex never touches parity — passed through unchanged"
            );
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
        assert!(
            cells.len() > 1,
            "a large-enough footprint overlaps more than just the anchor"
        );
        assert!(cells.contains(&anchor));
    }

    #[test]
    fn hex_grid_cell_of_matches_axial_round_for_a_known_point() {
        // size=50: cell (0,0)'s center is (0,0) in pointy-top axial pixel space (Red Blob Games
        // convention, matching the client's `pixelToAxial`/`axialToPixel` exactly).
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
        let g = SquareGrid {
            cell: 100.0,
            rule: DiagonalRule::Chebyshev,
        };
        // Origin-anchored 3×3.
        let (min, max) = ((0.0, 0.0), (250.0, 250.0));
        assert_eq!(
            g.cells_in_bounds(min, max, 100.0, CAP),
            Some(hand_floor_rect(min, max, 100.0))
        );
        // Straddling the origin (negative coords).
        let (min, max) = ((-150.0, -50.0), (50.0, 50.0));
        assert_eq!(
            g.cells_in_bounds(min, max, 100.0, CAP),
            Some(hand_floor_rect(min, max, 100.0))
        );
        // Wholly negative.
        let (min, max) = ((-330.0, -220.0), (-110.0, -140.0));
        assert_eq!(
            g.cells_in_bounds(min, max, 100.0, CAP),
            Some(hand_floor_rect(min, max, 100.0))
        );
    }

    #[test]
    fn square_cells_in_bounds_is_row_major() {
        let g = SquareGrid {
            cell: 100.0,
            rule: DiagonalRule::Chebyshev,
        };
        let got = g
            .cells_in_bounds((0.0, 0.0), (150.0, 150.0), 100.0, CAP)
            .unwrap();
        // for i { for j } → (0,0),(0,1),(1,0),(1,1).
        assert_eq!(got, vec![(0, 0), (0, 1), (1, 0), (1, 1)]);
    }

    #[test]
    fn cells_in_bounds_span_cap_returns_none() {
        // 3001×3001 ≈ 9.0M candidate cells > the 4M MAX_CELLS_PER_POLYGON cap → None.
        let g = SquareGrid {
            cell: 1.0,
            rule: DiagonalRule::Chebyshev,
        };
        assert_eq!(
            g.cells_in_bounds((0.0, 0.0), (3000.0, 3000.0), 1.0, CAP),
            None
        );
        // A hex scene with an equally-huge AABB is capped by the same span check.
        let hx = HexGrid { size: 1.0 };
        assert_eq!(
            hx.cells_in_bounds((0.0, 0.0), (5000.0, 5000.0), 1.0, CAP),
            None
        );
    }

    #[test]
    fn cells_in_bounds_honors_the_caller_supplied_cap() {
        // A 3×3 (9-cell) AABB is accepted under the vision cap but rejected under a tighter
        // per-caller bound — the cap is the passed `max_cells`, never a hardcoded constant, so
        // routing a tighter-capped caller (region rasterization) through this primitive can't
        // loosen its bound.
        let g = SquareGrid {
            cell: 100.0,
            rule: DiagonalRule::Chebyshev,
        };
        let (min, max) = ((0.0, 0.0), (250.0, 250.0));
        assert!(g.cells_in_bounds(min, max, 100.0, CAP).is_some());
        assert_eq!(
            g.cells_in_bounds(min, max, 100.0, 8),
            None,
            "9 cells > cap of 8"
        );
        // Same for hex — the span cap is honored on both grid kinds.
        let hx = HexGrid { size: 50.0 };
        assert_eq!(
            hx.cells_in_bounds((0.0, 0.0), (200.0, 200.0), 50.0, 1),
            None
        );
    }

    #[test]
    fn cells_in_bounds_fails_closed_on_degenerate_input() {
        let g = SquareGrid {
            cell: 100.0,
            rule: DiagonalRule::Chebyshev,
        };
        assert_eq!(
            g.cells_in_bounds((f64::NAN, 0.0), (10.0, 10.0), 100.0, CAP),
            None
        );
        assert_eq!(
            g.cells_in_bounds((0.0, 0.0), (f64::INFINITY, 10.0), 100.0, CAP),
            None
        );
        assert_eq!(g.cells_in_bounds((0.0, 0.0), (10.0, 10.0), 0.0, CAP), None);
        assert_eq!(g.cells_in_bounds((0.0, 0.0), (10.0, 10.0), -5.0, CAP), None);
        let hx = HexGrid { size: 50.0 };
        assert_eq!(
            hx.cells_in_bounds((0.0, 0.0), (10.0, f64::NAN), 50.0, CAP),
            None
        );
        assert_eq!(hx.cells_in_bounds((0.0, 0.0), (10.0, 10.0), 0.0, CAP), None);
    }

    #[test]
    fn hex_cells_in_bounds_includes_every_cell_whose_center_is_in_the_aabb() {
        let g = HexGrid { size: 50.0 };
        let (min, max) = ((0.0, 0.0), (200.0, 200.0));
        let got = g
            .cells_in_bounds(min, max, 50.0, CAP)
            .expect("bounded, not over-cap");
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
        assert!(
            center_in_count > 0,
            "fixture must exercise at least one in-bounds hex"
        );
        // Stays bounded — a tight superset of a ~4×4-hex region, not a runaway scan.
        assert!(
            got.len() < 100,
            "hex candidate set should stay small for a small AABB"
        );
    }

    #[test]
    fn square_cell_bounds_is_the_corner_floor_box() {
        // Byte-identical to the pre-hex A* window's `floor(min/cell)`/`floor(max/cell)` computation,
        // so square routes are unchanged.
        let g = SquareGrid {
            cell: 100.0,
            rule: DiagonalRule::Chebyshev,
        };
        assert_eq!(
            g.cell_bounds((0.0, -250.0), (250.0, 50.0), 100.0),
            (0, -3, 2, 0)
        );
        assert_eq!(
            g.cell_bounds((-330.0, -220.0), (-110.0, -140.0), 100.0),
            (-4, -3, -2, -2)
        );
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
        assert!(
            i0 <= goal.0 && goal.0 <= i1,
            "axial q {} must lie in [{i0},{i1}]",
            goal.0
        );
        assert!(
            j0 <= goal.1 && goal.1 <= j1,
            "axial r {} must lie in [{j0},{j1}]",
            goal.1
        );
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
        let g = SquareGrid {
            cell: 100.0,
            rule: DiagonalRule::Chebyshev,
        };
        assert_eq!(
            g.cell_vertices((2, 3), 100.0),
            vec![
                (200.0, 300.0),
                (300.0, 300.0),
                (200.0, 400.0),
                (300.0, 400.0)
            ]
        );
    }

    /// Independent reference for the intended axial (cube) hex distance, so the assertion below
    /// isn't tautological with the impl under test.
    fn axial_distance(from: Cell, to: Cell) -> f64 {
        let dq = to.0 as i64 - from.0 as i64;
        let dr = to.1 as i64 - from.1 as i64;
        ((dq.abs() + dr.abs() + (dq + dr).abs()) as f64) / 2.0
    }

    #[test]
    fn hex_heuristic_equals_axial_distance_including_opposite_sign_deltas() {
        let g = HexGrid { size: 50.0 };
        // Same-sign axial delta (the (1,1) direction): distance = |dq| + |dr|.
        assert_eq!(g.heuristic((0, 0), (3, 3)), axial_distance((0, 0), (3, 3)));
        assert_eq!(g.heuristic((0, 0), (3, 3)), 6.0);
        // Opposite-sign axial delta (the (1,-1) direction): distance = max(|dq|, |dr|) — this is
        // the case the square Manhattan/Euclidean estimate OVERESTIMATES.
        assert_eq!(
            g.heuristic((0, 0), (4, -4)),
            axial_distance((0, 0), (4, -4))
        );
        assert_eq!(g.heuristic((0, 0), (4, -4)), 4.0);
        // Mixed / off-axis deltas, both delta orderings.
        assert_eq!(
            g.heuristic((2, -1), (5, -6)),
            axial_distance((2, -1), (5, -6))
        );
        assert_eq!(
            g.heuristic((5, -6), (2, -1)),
            axial_distance((5, -6), (2, -1))
        );
        // Symmetric.
        assert_eq!(g.heuristic((-3, 2), (1, -4)), g.heuristic((1, -4), (-3, 2)));
        // Zero delta.
        assert_eq!(g.heuristic((7, -2), (7, -2)), 0.0);
    }

    /// Shortest-path cost from `from` to every cell of the open box `|i| <= r`, `|j| <= r`,
    /// relaxed to a fixpoint over the REAL `GridShape::neighbors_with_cost` edges (Bellman-Ford —
    /// chosen over Dijkstra because it needs no ordering of `f64` keys and stays obviously correct
    /// for the non-uniform square step costs). The reference cost therefore comes from the neighbor
    /// function itself, never from a restatement of the heuristic's formula.
    ///
    /// The axial box is closed under shortest hex paths: a shortest path can always be taken
    /// monotone in cube coordinates (at most two of the six directions, each step moving `q` and
    /// `r` monotonically toward the target), so every intermediate cell has `q` between the two
    /// endpoints' `q` and `r` between their `r`. Confining the search to the box thus cannot
    /// inflate the true cost between any two of its cells.
    fn true_costs_from(
        g: &dyn GridShape,
        from: Cell,
        r: i32,
    ) -> std::collections::BTreeMap<Cell, f64> {
        let cells: Vec<Cell> = (-r..=r)
            .flat_map(|i| (-r..=r).map(move |j| (i, j)))
            .collect();
        let mut dist: std::collections::BTreeMap<Cell, f64> =
            cells.iter().map(|&c| (c, f64::INFINITY)).collect();
        dist.insert(from, 0.0);
        loop {
            let mut changed = false;
            for &c in &cells {
                let d = dist[&c];
                if !d.is_finite() {
                    continue;
                }
                for (next, step, _) in g.neighbors_with_cost(c, 0) {
                    if !dist.contains_key(&next) {
                        continue; // outside the searched box
                    }
                    if d + step < dist[&next] - 1e-12 {
                        dist.insert(next, d + step);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        dist.retain(|_, d| d.is_finite());
        dist
    }

    #[test]
    fn hex_heuristic_never_exceeds_the_true_neighbor_graph_cost() {
        // Admissibility against the ACTUAL neighbor graph: for every ordered pair in an open
        // radius-4 axial box, the heuristic must lower-bound the cost of the cheapest route
        // `neighbors_with_cost` admits. A heuristic that overestimates any reachable pair makes A*
        // return suboptimal hex routes.
        const R: i32 = 4;
        let g = HexGrid { size: 50.0 };
        let cells: Vec<Cell> = (-R..=R)
            .flat_map(|q| (-R..=R).map(move |r| (q, r)))
            .collect();
        let mut checked = 0usize;
        let mut tight = 0usize;
        for &from in &cells {
            let costs = true_costs_from(&g, from, R);
            for &to in &cells {
                let true_cost = *costs
                    .get(&to)
                    .unwrap_or_else(|| panic!("open box is connected: {from:?} -> {to:?}"));
                let h = g.heuristic(from, to);
                assert!(
                    h <= true_cost + 1e-9,
                    "heuristic {h} exceeds true neighbor-graph cost {true_cost} for {from:?} -> {to:?}"
                );
                checked += 1;
                if to != from && (h - true_cost).abs() < 1e-9 {
                    tight += 1;
                }
            }
        }
        assert_eq!(
            checked,
            cells.len() * cells.len(),
            "every ordered pair is covered"
        );
        // Non-vacuity: the bound is not passing merely because every true cost is huge — the
        // heuristic is EXACT (tight) on non-trivial pairs, which is what keeps A* efficient.
        assert!(
            tight > 0,
            "the heuristic must be tight on at least one non-trivial pair"
        );
    }

    #[test]
    fn square_heuristic_never_exceeds_the_true_neighbor_graph_cost() {
        // Same independent check for every square diagonal rule, so the trait's admissibility
        // contract is proven on both grid kinds against their own neighbor functions. `Alternating`
        // is excluded: its step cost depends on the carried parity, so a parity-blind uniform-cost
        // search is not its true cost function (its admissibility is pinned by the square parity
        // tests in `pathfinding::alternating_five_ten_five_parity`).
        const R: i32 = 4;
        let cells: Vec<Cell> = (-R..=R)
            .flat_map(|i| (-R..=R).map(move |j| (i, j)))
            .collect();
        for rule in [
            DiagonalRule::Chebyshev,
            DiagonalRule::Manhattan,
            DiagonalRule::Euclidean,
        ] {
            let g = SquareGrid { cell: 100.0, rule };
            for &from in &cells {
                let costs = true_costs_from(&g, from, R);
                for &to in &cells {
                    let true_cost = costs[&to];
                    let h = g.heuristic(from, to);
                    assert!(
                        h <= true_cost + 1e-9,
                        "{rule:?}: heuristic {h} exceeds true cost {true_cost} for {from:?} -> {to:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn square_manhattan_overestimates_the_true_hex_distance_on_opposite_sign_deltas() {
        // Documents WHY the shared square heuristic was non-admissible on hex: on the (1,-1) axial
        // line the true hex distance is `max(|dq|,|dr|)` but the square Manhattan distance is
        // `|dq|+|dr|` — a 2× overestimate. The hex heuristic returns the admissible value instead.
        let hex = HexGrid { size: 50.0 };
        let sq_manhattan = SquareGrid {
            cell: 50.0,
            rule: DiagonalRule::Manhattan,
        };
        let (from, to) = ((0, 0), (4, -4));
        assert_eq!(hex.heuristic(from, to), 4.0, "true hex distance");
        assert_eq!(
            sq_manhattan.heuristic(from, to),
            8.0,
            "square Manhattan overestimates 2x"
        );
        assert!(sq_manhattan.heuristic(from, to) > hex.heuristic(from, to));
    }

    #[test]
    fn square_heuristic_is_byte_identical_to_the_free_pathfinding_heuristic() {
        // The trait dispatch must not change square scenes: `SquareGrid::heuristic` returns exactly
        // the free `pathfinding::heuristic(self.rule, ...)` `astar_leg` called before.
        for rule in [
            DiagonalRule::Chebyshev,
            DiagonalRule::Manhattan,
            DiagonalRule::Euclidean,
            DiagonalRule::Alternating,
        ] {
            let g = SquareGrid { cell: 100.0, rule };
            for &(from, to) in &[((0, 0), (3, 5)), ((-2, 4), (7, -1)), ((5, 5), (5, 5))] {
                assert_eq!(
                    g.heuristic(from, to),
                    crate::scene::pathfinding::heuristic(rule, from, to),
                    "square heuristic must equal the free rule-based heuristic for {rule:?}"
                );
            }
        }
    }

    /// EXACT oracle: the set of hexes whose INTERIOR the segment `a -> b` overlaps over a
    /// positive-length interval, computed by Cyrus-Beck clipping of the segment against each
    /// candidate hex's 6 half-planes (`cell_vertices`). Independent of `line_traversal`.
    fn true_hexes_crossed(g: &HexGrid, a: vision::P, b: vision::P, eps: f64) -> BTreeSet<Cell> {
        let (ca, cb) = (g.cell_of(a), g.cell_of(b));
        let pad = 4;
        let (q0, q1) = (ca.0.min(cb.0) - pad, ca.0.max(cb.0) + pad);
        let (r0, r1) = (ca.1.min(cb.1) - pad, ca.1.max(cb.1) + pad);
        let d = (b.0 - a.0, b.1 - a.1);
        let len = (d.0 * d.0 + d.1 * d.1).sqrt();
        let mut out = BTreeSet::new();
        for q in q0..=q1 {
            for r in r0..=r1 {
                let c = (q, r);
                let verts = g.cell_vertices(c, 0.0);
                let ctr = g.cell_center(c);
                let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
                let mut empty = false;
                for k in 0..6 {
                    let v0 = verts[k];
                    let v1 = verts[(k + 1) % 6];
                    // Inward normal for edge v0->v1 (sign fixed by the center being inside).
                    let e = (v1.0 - v0.0, v1.1 - v0.1);
                    let mut n = (-e.1, e.0);
                    if n.0 * (ctr.0 - v0.0) + n.1 * (ctr.1 - v0.1) < 0.0 {
                        n = (-n.0, -n.1);
                    }
                    // f(t) = n . (a + t*d - v0) >= 0 keeps the point inside.
                    let f0 = n.0 * (a.0 - v0.0) + n.1 * (a.1 - v0.1);
                    let fd = n.0 * d.0 + n.1 * d.1;
                    if fd.abs() < 1e-18 {
                        if f0 < 0.0 {
                            empty = true;
                            break;
                        }
                        continue;
                    }
                    let t = -f0 / fd;
                    if fd > 0.0 {
                        t0 = t0.max(t);
                    } else {
                        t1 = t1.min(t);
                    }
                    if t0 > t1 {
                        empty = true;
                        break;
                    }
                }
                if !empty && (t1 - t0) * len > eps {
                    out.insert(c);
                }
            }
        }
        out
    }

    /// Deterministic LCG so the search is reproducible.
    struct Lcg(u64);
    impl Lcg {
        fn next_f64(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
        }
        fn range(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * self.next_f64()
        }
    }

    /// Structured adversarial endpoint pool: hex centers, all 6 vertices, all 6 edge midpoints,
    /// and small offsets off each, for a ring of hexes around the origin.
    fn adversarial_points(g: &HexGrid) -> Vec<vision::P> {
        let mut pts = Vec::new();
        for q in -2..=2 {
            for r in -2..=2 {
                let c = (q, r);
                let ctr = g.cell_center(c);
                pts.push(ctr);
                let verts = g.cell_vertices(c, 0.0);
                for k in 0..6 {
                    let v = verts[k];
                    let w = verts[(k + 1) % 6];
                    pts.push(v);
                    pts.push(((v.0 + w.0) * 0.5, (v.1 + w.1) * 0.5));
                    // Just inside / just outside the vertex, along the center->vertex ray.
                    for f in [0.999_999, 1.000_001, 0.5] {
                        pts.push((ctr.0 + (v.0 - ctr.0) * f, ctr.1 + (v.1 - ctr.1) * f));
                    }
                }
            }
        }
        pts
    }

    /// THE security property: `line_traversal` must never omit a hex the segment geometrically
    /// crosses, or a non-GM could route a move that grazes an unseen hex without that hex being
    /// checked against the visibility mask (a fog probe) — the exact failure the square supercover
    /// is documented to prevent. Checked against `true_hexes_crossed`, an EXACT oracle (Cyrus-Beck
    /// clipping against each hex's own `cell_vertices` polygon), not dense sampling — dense
    /// sampling is only a subset oracle and would pass over a barely-clipped hex.
    ///
    /// Search: 3 hex sizes × {random long segments, random sub-hex segments, pairs drawn from a
    /// pool of hex centers/vertices/edge-midpoints/near-vertex offsets, and chords parallel to each
    /// of the 6 hex edge directions at 121 perpendicular offsets each}.
    #[test]
    fn hex_line_traversal_never_omits_a_hex_the_segment_crosses() {
        use std::cell::{Cell as C, RefCell};
        let misses = C::new(0usize);
        let first: RefCell<Option<String>> = RefCell::new(None);
        let checked = C::new(0usize);
        let over = C::new(0usize);
        let over_max = C::new(0usize);
        let check = |g: &HexGrid, a: vision::P, b: vision::P, eps: f64| {
            let got = match g.line_traversal(a, b, g.size) {
                Some(s) => s,
                None => return,
            };
            let want = true_hexes_crossed(g, a, b, eps);
            checked.set(checked.get() + 1);
            let extra = got.len() - want.intersection(&got).count();
            over.set(over.get() + extra);
            over_max.set(over_max.get().max(extra));
            if !want.is_subset(&got) {
                misses.set(misses.get() + 1);
                if first.borrow().is_none() {
                    let missing: Vec<Cell> = want.difference(&got).copied().collect();
                    *first.borrow_mut() = Some(format!(
                        "size={} a={a:?} b={b:?}\n got={got:?}\nwant={want:?}\nmissing={missing:?}",
                        g.size
                    ));
                }
            }
        };
        for &size in &[1.0_f64, 50.0, 137.5] {
            let g = HexGrid { size };
            let mut rng = Lcg(0x5EED_1234);
            // Random long + short segments.
            for _ in 0..8000 {
                let sp = size * 6.0;
                let a = (rng.range(-sp, sp), rng.range(-sp, sp));
                let b = (rng.range(-sp, sp), rng.range(-sp, sp));
                check(&g, a, b, 1e-9 * size);
                let tiny = size * 0.01;
                let b2 = (a.0 + rng.range(-tiny, tiny), a.1 + rng.range(-tiny, tiny));
                check(&g, a, b2, 1e-9 * size);
            }
            // Structured adversarial pairs.
            let pts = adversarial_points(&g);
            let mut rng2 = Lcg(0xC0FF_EE01);
            for _ in 0..20000 {
                let a = pts[(rng2.next_f64() * pts.len() as f64) as usize % pts.len()];
                let b = pts[(rng2.next_f64() * pts.len() as f64) as usize % pts.len()];
                check(&g, a, b, 1e-9 * size);
            }
            // Segments parallel to each of the three hex edge directions, at many offsets.
            for k in 0..6 {
                let ang = std::f64::consts::PI / 180.0 * (60.0 * k as f64);
                let (dx, dy) = (ang.cos(), ang.sin());
                let (px, py) = (-dy, dx);
                for i in -60..=60 {
                    let off = i as f64 * size / 37.0;
                    let a = (px * off - dx * size * 4.0, py * off - dy * size * 4.0);
                    let b = (px * off + dx * size * 4.0, py * off + dy * size * 4.0);
                    check(&g, a, b, 1e-9 * size);
                }
            }
        }
        if let Some(f) = first.borrow().clone() {
            panic!(
                "omitted a crossed hex in {}/{}: {f}",
                misses.get(),
                checked.get()
            );
        }
        assert!(
            checked.get() > 100_000,
            "the search must actually run: {}",
            checked.get()
        );
        // Over-inclusion is the SAFE direction (it can only over-restrict a move, never over-permit),
        // but it must stay bounded or legitimate moves get rejected. The boundary probe adds at most
        // a small constant per segment, and only for segments that genuinely run along or through a
        // hex boundary — generic segments (the random groups) get essentially none.
        assert!(
            over_max.get() <= 12,
            "per-segment over-inclusion must stay small, got {}",
            over_max.get()
        );
    }

    /// Generic (non-boundary-aligned) segments must be effectively EXACT — the conservative
    /// boundary probe must not silently over-restrict ordinary moves.
    #[test]
    fn hex_line_traversal_is_near_exact_for_generic_segments() {
        let g = HexGrid { size: 50.0 };
        let mut rng = Lcg(0x5EED_1234);
        let (mut checked, mut extra_total) = (0usize, 0usize);
        for _ in 0..8000 {
            let a = (rng.range(-300.0, 300.0), rng.range(-300.0, 300.0));
            let b = (rng.range(-300.0, 300.0), rng.range(-300.0, 300.0));
            let Some(got) = g.line_traversal(a, b, 50.0) else {
                continue;
            };
            let want = true_hexes_crossed(&g, a, b, 1e-9 * g.size);
            assert!(want.is_subset(&got), "a={a:?} b={b:?}");
            extra_total += got.len() - want.intersection(&got).count();
            checked += 1;
        }
        assert!(checked > 7900);
        assert!(
            // Measured true value is 0 spurious cells on generic segments; 1-in-1000 leaves 10x
            // slack without sleeping through a real regression (a 100x bound was shown to pass
            // even with the vertex probe inflated 100-fold).
            extra_total * 1000 < checked,
            "fewer than 1 in 1000 generic segments may gain a spurious hex: {extra_total}/{checked}"
        );
    }

    #[test]
    fn hex_line_traversal_includes_the_far_endpoint_hex_on_a_sub_hex_step() {
        // Reduced counterexample. A step short enough that the max cube-axis delta rounds to 0 still
        // crosses a hex boundary: (-40, 0) sits in hex (0,0) (inradius 43.3), (-50, 0) in hex (-1,0).
        // The pre-fix fixed-count cube lerp took `n = 0` and returned ONLY the start hex, omitting
        // even the far endpoint's own hex — an unseen hex a token could step into ungated.
        let g = HexGrid { size: 50.0 };
        let (a, b) = ((-40.0, 0.0), (-50.0, 0.0));
        assert_eq!(g.cell_of(a), (0, 0));
        assert_eq!(g.cell_of(b), (-1, 0));
        let cells = g.line_traversal(a, b, 50.0).expect("finite, in-bounds");
        assert!(cells.contains(&(0, 0)), "start hex: {cells:?}");
        assert!(
            cells.contains(&(-1, 0)),
            "far ENDPOINT hex must be present: {cells:?}"
        );
    }

    #[test]
    fn hex_line_traversal_includes_a_corner_clipped_hex() {
        // Reduced counterexample. This segment clips a short sliver of hex (-1,0) near the vertex it
        // shares with (0,0)/(0,-1); the pre-fix sampler's one-hex-pitch spacing straddled the sliver
        // entirely and never named it.
        let g = HexGrid { size: 50.0 };
        let a = (-51.640_685_867_085_56, 82.356_123_493_857_9);
        let b = (-32.076_474_134_396_726, -54.002_821_619_560_066);
        let cells = g.line_traversal(a, b, 50.0).expect("finite, in-bounds");
        let want = true_hexes_crossed(&g, a, b, 1e-6);
        assert!(
            want.contains(&(-1, 0)),
            "fixture must actually clip (-1,0): {want:?}"
        );
        assert!(want.is_subset(&cells), "want={want:?} got={cells:?}");
    }

    #[test]
    fn hex_line_traversal_emits_both_hexes_flanking_a_traversed_edge() {
        // A segment running exactly along the shared edge of two hexes touches both. Same deliberate
        // over-inclusion the square supercover documents for a shared corner, so a move cannot
        // thread an unseen hex by riding a boundary.
        let g = HexGrid { size: 50.0 };
        // Hexes (0,0) (center (0,0)) and (-1,0) (center (-86.6,0)) share the vertical edge x = -43.3
        // spanning y in [-25, 25].
        let x = -50.0 * 3.0_f64.sqrt() / 2.0;
        let cells = g
            .line_traversal((x, -20.0), (x, 20.0), 50.0)
            .expect("finite, in-bounds");
        assert!(cells.contains(&(0, 0)), "{cells:?}");
        assert!(cells.contains(&(-1, 0)), "{cells:?}");
    }

    #[test]
    fn hex_line_traversal_still_fails_closed_on_non_finite_and_over_cap() {
        let g = HexGrid { size: 50.0 };
        assert_eq!(g.line_traversal((f64::NAN, 0.0), (10.0, 10.0), 50.0), None);
        assert_eq!(
            g.line_traversal((0.0, 0.0), (f64::INFINITY, 0.0), 50.0),
            None
        );
        assert_eq!(
            g.line_traversal((0.0, f64::NEG_INFINITY), (0.0, 0.0), 50.0),
            None
        );
        // Past the 4096-hex span cap.
        assert_eq!(g.line_traversal((0.0, 0.0), (1.0e9, 0.0), 50.0), None);
    }

    #[test]
    fn hex_cell_vertices_returns_six_points_at_radius_size() {
        let g = HexGrid { size: 50.0 };
        let center = g.cell_center((1, -2));
        let verts = g.cell_vertices((1, -2), 50.0);
        assert_eq!(verts.len(), 6, "a hex has 6 vertices");
        for &(x, y) in &verts {
            let d = ((x - center.0).powi(2) + (y - center.1).powi(2)).sqrt();
            assert!(
                (d - 50.0).abs() < 1e-9,
                "each hex vertex sits at radius = size from center"
            );
        }
    }
}
