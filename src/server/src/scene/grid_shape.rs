//! `GridShape` abstracts the per-cell geometry every movement/vision/pathfinding module needs, so
//! square and hex scenes share one code path rather than each caller branching on grid kind.
//! `SquareGrid` implements the ordinary square-grid formulas; `HexGrid` is the pointy-top axial
//! implementation mirroring the client's `Grid` class exactly.
//!
//! `cell_center`/`cells_in_bounds` are wired into `accumulate_visible_cells`, `player_lit_mask`,
//! `explored::mark_polygons`, and `regions::rasterize`;
//! `neighbors_with_cost`/`footprint_cells`/`line_traversal` are wired into
//! `pathfinding::astar_leg`/`cell_enterable`; `line_traversal` is also wired into
//! `move_exec::execute_move`; `neighbors_with_cost` is ALSO wired into `move_exec::execute_move`,
//! which prices its own transitions through it too, so the router's preview cost and the
//! executor's execution cost are the same number; `cell_of` is wired into
//! `move_exec::execute_move`'s region-cell lookup (and `HexGrid::cells_in_bounds`'s corner
//! mapping); `cell_vertices` is wired into `accumulate_visible_cells`'s lenient corner-sampling
//! branch.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::scene::pathfinding::{self, Cell, DiagonalRule};
use crate::scene::vision;
use crate::scene::GridKind;
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
    /// `min`/`max`/`cell`, `cell <= 0.0`) or an over-cap span (> `max_cells`); every caller
    /// (`accumulate_visible_cells`, `player_lit_mask`, `explored::mark_polygons`,
    /// `regions::rasterize`) fails closed on `None`. `max_cells` is the caller's own DoS bound
    /// (vision/explored scans pass `explored::MAX_CELLS_PER_POLYGON`, region rasterization passes
    /// the 40× tighter `regions::MAX_REGION_CELLS`) — never hardcoded here, so routing a
    /// tighter-capped caller through this primitive can't LOOSEN its bound.
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
    /// World-space distance represented by ONE unit of grid distance on this shape — the
    /// distance between two adjacent cell centres.
    ///
    /// This is the conversion for the CLASS of quantity a GM authors as a DISTANCE in cells that
    /// the engine must measure in world units — light bright/dim radii, a token's
    /// `VisionAssignment::range`, animation speed, and the router's reported cost all belong to it.
    /// `VisionMode::default_range` belongs to the class as an AUTHORED quantity but is not measured
    /// here either: `token_vision_floors` resolves an omitted `VisionAssignment::range` against it
    /// BEFORE any mask reads the pair, so the mode default reaches the mask through the exact same
    /// cells-vs-`dist_cells` comparison the assignment's own `range` always used — never a second,
    /// independently-converted path.
    ///
    /// Square returns its own cell size, so nothing changes there; pointy-top hex returns
    /// `√3 · size`, because all six axial neighbours sit that far from a hex's centre while `size`
    /// is only its circumradius.
    ///
    /// A whole-RECTANGLE quantity is authored in cells too and does NOT convert through this
    /// scalar: the scene's authored bounds go through `world_extent` instead, because a hex block's
    /// world rectangle is a shear-dependent function of the block rather than a per-axis product of
    /// any single distance.
    ///
    /// NOT the cell INDEXING scale. `cell_of`, `cell_center`, `cells_in_bounds`, `cell_bounds`,
    /// `footprint_cells` and `line_traversal` index against the shape's own stored scale and must
    /// never be re-scaled by this value. The two coincide on square, which is exactly why a site
    /// that confuses them stays invisible until a hex scene runs through it.
    ///
    /// NOT the token footprint scale either. A token's footprint radius derives from its OWN
    /// shape's model — a square block's half-diagonal on square, the circumscribing radius of the
    /// authored hex count on hex (owner ruling: authored size counts HEXES on hex, never a square
    /// approximation; see `footprint::resolve_footprint_cells`) — and both convert through the
    /// indexing scale, not this one: scaling a hex footprint here would give a 1-hex token a disc
    /// past its own hex's circumradius, which is a rules change rather than a unit fix.
    fn world_units_per_cell(&self) -> f64;
    /// The world-unit ENVELOPE of `bounds_cells`, a per-axis dimension measured in grid units
    /// (cells), continuous — never world units, and not required to be integral. Both corners are
    /// returned, because `min` is the origin only on square: a pointy-top hex block's origin cell
    /// is centred ON the origin, so the block reaches `-√3/2·size` in x and `-size` in y.
    ///
    /// **Both guarantees are stated for the INTEGER block `[0, ⌊w⌋) × [0, ⌊h⌋)`**, and
    /// consumers must not assume the stronger one:
    /// - **Square** — an exact COVER of that block. Cell `(i,j)` occupies `[i·cell,(i+1)·cell)`
    ///   per axis, so `⌊w⌋ × ⌊h⌋` cells occupy `(⌊w⌋·cell, ⌊h⌋·cell) ≤ (w·cell, h·cell)` with no
    ///   shear and no overhang, anchored at `min = (0, 0)`.
    /// - **Hex** — a CENTRE cover of that block on the `max` side. Every such cell's centre lies
    ///   inside, and the extreme cell's far vertices lie inside; the origin cell's own lower-left
    ///   extreme is exactly `min`, so the axial rhombus's origin-side margin is INSIDE the
    ///   envelope. A partial column or row past `max` remains outside, and claiming a full cover
    ///   of the whole authored `w × h` would still be false.
    ///
    /// **Which cells the envelope covers is decided on its `max` side alone for every cell of the
    /// integer block, and it is a per-axis rule whose statement and DOMAIN the two shapes share
    /// neither of.** The `min` side excludes no such cell on either shape — square's is the origin
    /// and every block cell's centre is at least half a cell past it, hex's is the origin cell's
    /// own lower-left extreme and no cell of the block reaches below it — so the rule is stated as
    /// an upper bound only. Writing `f_w`/`f_h` for the fractional parts of `w`/`h`, cell
    /// `(i, j)`'s CENTRE is inside exactly when:
    /// - **Square**, for every `w > 0` and `h > 0` — `i ≤ w − 0.5` and `j ≤ h − 0.5`, each axis
    ///   independent of the other, so the partial cell `⌊w⌋` is left outside exactly when
    ///   `f_w < 0.5`, and likewise on `h`. Square additionally never FULLY covers a partial cell
    ///   for any `f ≠ 0`, since it always overhangs. `SquareGrid::world_extent` is `w·cell` with
    ///   no subtraction and nothing clamped, so the same rule describes a sub-one-cell bound: at
    ///   `w = 0.25` the rectangle is `0.25·cell` and cell 0's centre at `0.5·cell` is outside,
    ///   which is what `i ≤ w − 0.5` predicts.
    /// - **Hex x**, row-DEPENDENT — `i ≤ w − 1 + (h − j)/2`. The axial shear grants `(h − j)/2`
    ///   columns of extra reach at row `j`, so the reach SHRINKS as the row index rises, and the
    ///   partial column `⌊w⌋` falls outside from row `h − 2·(1 − f_w)` upward. At row `0` that
    ///   column is inside only when `h ≥ 2·(1 − f_w)`: at `w = 3.2` a block two rows tall covers
    ///   column 3 at row 0, one row tall does not.
    /// - **Hex y**, column-independent — `j ≤ h − 1/3`. The partial row `⌊h⌋` is left outside
    ///   exactly when `f_h < 1/3`, NOT when `f_h < 1/2`: at `h = 4.4` row 4 is covered, at
    ///   `h = 4.2` it is not.
    ///
    /// **Both hex axes are stated for `w ≥ 1` and `h ≥ 1`, and that restriction is hex's alone.**
    /// `HexGrid::world_extent` clamps its `w − 1` and `h − 1` terms at zero, so below one cell the
    /// rectangle stops shrinking with the bound and no longer varies linearly with it: at
    /// `w = 0.25, h = 1.0` the x rule predicts column 0 excluded while the measurement covers it,
    /// the origin hex being centred ON the origin corner. That regime is pinned separately by
    /// `world_extent_stays_positive_below_one_cell_and_covers_at_most_the_origin_cell`.
    ///
    /// That rule is EXECUTABLE, not merely stated: `extent_rule_says_inside` is it, and
    /// `the_extent_membership_rule_predicts_every_cells_coverage_over_a_bounds_sweep` asserts its
    /// prediction against the measured comparison for every cell of the integer block plus the
    /// partial column and the partial row, across both shapes and both axes' fractional parts —
    /// and, on the square arm, over sub-one-cell bounds as well, so the regime hex has to fence
    /// off is proved on the shape that does not. A misstatement of the rule fails a run instead of
    /// surviving a reading. That same sweep measures the `min` side on every bound it visits,
    /// against the origin cell's own lower-left extreme derived from `cell_center`, so the corner
    /// the rule declines to constrain is not left unmeasured.
    ///
    /// A fractional bound is authorable through the ordinary settings editor, which marks a
    /// fractional width invalid without sanitizing it.
    ///
    /// What a partial column or row past `max` costs each consumer — over-covering is NOT free
    /// for all of them:
    /// - `navmesh::build_navmesh` triangulates this envelope, so a position outside it is off-mesh
    ///   and routes as unreachable: an excluded partial column. Every integer-block cell CENTRE —
    ///   the only position a grid-snapped token occupies — is on-mesh, and on hex axial row
    ///   `r = 0`'s centres are STRICTLY INTERIOR, one circumradius above the envelope's bottom
    ///   edge, so their routability does not depend on whether the mesh's containment test admits
    ///   an exactly-on-boundary point.
    /// - `lighting::env_light_polys` walks this envelope's perimeter, so an excluded margin gets
    ///   no boundary sample of its own and is lit only through neighbouring samples: under-reveal.
    /// - `vision::bound_for_scene` unions this envelope with the wall-derived bound and expands by
    ///   its own `margin`, so the shortfall shows only where that margin is smaller than it:
    ///   under-reveal again.
    ///
    /// Growing the envelope — by rounding a fractional bound UP, say — is not a free hedge in the
    /// other direction, which is why neither impl does: the vision bound and the lit mask are
    /// BUILT from it rather than merely gated by it, so a larger rectangle widens what a player is
    /// told they can see, and the environment-light perimeter walk is not mask-gated at all.
    /// Rounding up would also make two distinct authored dimensions produce one rectangle, so a
    /// stored setting would stop determining its own effect. Reaching the origin cell's own
    /// lower-left extreme is not that hedge: it covers cells the GM authored rather than inventing
    /// any.
    ///
    /// A degenerate axis — non-finite, zero, or negative — yields the zero-AREA envelope
    /// `min = max = (0.0, 0.0)` from both impls, which every consumer's span guard refuses, rather
    /// than two shapes disagreeing about degenerate input. `normalize_bounds_cells` is where that
    /// whole class is refused, once, for both.
    fn world_extent(&self, bounds_cells: (f64, f64)) -> WorldExtent;
    /// Admissible A* heuristic (lower bound on the true `neighbors_with_cost` path cost) from cell
    /// `from` to cell `to`. Guides search ORDER only — never gates a cell — so it cannot affect the
    /// `route ⊆ gate-allowed` invariant. `SquareGrid` returns the existing `DiagonalRule`-based
    /// square distance; `HexGrid` returns the axial (cube) hex distance, which the square distance
    /// OVERESTIMATES for opposite-sign axial deltas (non-admissible on hex → suboptimal routes).
    fn heuristic(&self, from: Cell, to: Cell) -> f64;
    /// This shape's geometry family. Lets any holder of a resolved shape reach the kind without a
    /// second per-scene map that could disagree with it: `resolve_grid_shape_with_rule` builds
    /// the shape FROM `SceneEcs::resolve_grid_kind`, so the two are the same decision by
    /// construction rather than by convention.
    fn kind(&self) -> GridKind;
}

/// A scene's world-unit envelope: the axis-aligned rectangle that contains every cell of the
/// authored integer block, as both corners rather than a far corner alone.
///
/// `min` is not the origin on every shape. A pointy-top hex block's origin cell is CENTRED on the
/// origin, so its own polygon reaches `-(√3/2)·size` in x and `-size` in y; a square block's origin
/// cell has its corner there, so `min` is the origin exactly. Consumers that triangulate, walk, or
/// bound this rectangle read both corners, which is why the type carries both — an anchor a caller
/// supplies itself is an anchor each caller can get wrong independently.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WorldExtent {
    /// Lower-left corner in world units.
    pub(crate) min: (f64, f64),
    /// Upper-right corner in world units.
    pub(crate) max: (f64, f64),
}

impl WorldExtent {
    /// The envelope's x span. Zero or negative marks an envelope every consumer refuses.
    pub(crate) fn width(&self) -> f64 {
        self.max.0 - self.min.0
    }

    /// The envelope's y span. Zero or negative marks an envelope every consumer refuses.
    pub(crate) fn height(&self) -> f64 {
        self.max.1 - self.min.1
    }
}

/// Axial-box padding for `HexGrid::cells_in_bounds`. The axial↔pixel map is affine, so a pixel-space
/// axis-aligned rectangle's axial preimage is a bounded parallelogram; the integer axial bounding
/// box of that parallelogram's 4 corner images, expanded by ONE ring, is a safe superset of every
/// hex whose center lies in the rectangle. The +1 absorbs the cube-rounding fixup discrepancy
/// between converting the 4 corners (each cube-rounded) and a candidate cell's own center
/// (cube-rounded) — a corner conversion can differ from a naive per-axis round by up to 1 on one
/// axis. Source: Red Blob Games axial/cube coordinates (public-domain computational geometry).
const HEX_BOUNDS_PAD: i32 = 1;

/// The candidate-cell COUNT a `cell_bounds` rectangle `(i0, j0, i1, j1)` enumerates:
/// `(i1-i0+1) * (j1-j0+1)`, widened to `i64` and `saturating_mul`ed so an extreme-magnitude
/// bounding box can't overflow before a cap comparison runs.
pub(crate) fn candidate_span(bounds: (i32, i32, i32, i32)) -> i64 {
    let (i0, j0, i1, j1) = bounds;
    let w = i1 as i64 - i0 as i64 + 1;
    let h = j1 as i64 - j0 as i64 + 1;
    w.saturating_mul(h)
}

/// Whether a `cell_bounds` rectangle's own `candidate_span` exceeds `max_cells`. The ONE predicate
/// symbol every cap check in this subsystem calls: `SquareGrid::cells_in_bounds`,
/// `HexGrid::cells_in_bounds`, and `explored::scan_box_for`'s pre-check (negated) all read this
/// rather than each writing its own `<=`/`>` comparison against `candidate_span` — a span shared but
/// compared independently at three sites is the same fork one level down, and exactly-at-cap is the
/// input where a flipped operator at any one of them silently disagrees with the other two.
pub(crate) fn exceeds_cell_cap(bounds: (i32, i32, i32, i32), max_cells: i64) -> bool {
    candidate_span(bounds) > max_cells
}

/// Euclidean distance from `a` to `b`, in CELLS — divided by `world_units_per_cell` (the
/// authored-distance conversion; never the shape's INDEXING scale — see that method's own note
/// on the distinction). The single shared formula for "Euclidean span between two scene points,
/// converted to cells": `move_exec::execute_move`'s `Continuous` transition/tail pricing and its
/// `GridStepped` fallback for a transition the shape's own `neighbors_with_cost` does not
/// recognize as adjacent, and `navmesh::los_smooth`'s exact per-window cost, both price a span
/// this way and MUST agree bit-for-bit — a route preview and its later execution report the same
/// number for the same route (pinned by `cost_parity`'s parity tests). Uses `f64::hypot`, which is
/// more numerically robust against overflow/underflow at extreme magnitudes than the equivalent
/// `(dx*dx + dy*dy).sqrt()` — the two are not always bit-identical, so a caller computing the same
/// quantity by hand rather than through this function silently reintroduces that mismatch.
pub(crate) fn euclidean_span_cells(a: (f64, f64), b: (f64, f64), world_units_per_cell: f64) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1) / world_units_per_cell
}

/// The ordinary square-grid formulas: `cell_center` is `((i+0.5)*cell, (j+0.5)*cell)`;
/// `footprint_cells` is disc-vs-cell overlap; `line_traversal` delegates to
/// `movement::supercover_cells`; `neighbors_with_cost` is the 8-directional `dirs` array +
/// `step_cost`. `cell` and `rule` are the scene's resolved cell size and diagonal-cost rule.
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

    /// `floor(min/cell)..=floor(max/cell)` on each axis, row-major (`for i { for j }`). `f64 as
    /// i32` saturates on an extreme coordinate; the `saturating_mul` span check then fails closed
    /// against the caller-supplied `max_cells` bound. `accumulate_visible_cells`,
    /// `ExploredSet::mark_polygons`, and `player_lit_mask` all reach this through the `GridShape`
    /// trait rather than scanning a square grid themselves.
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
        let bounds = self.cell_bounds(min, max, cell);
        if exceeds_cell_cap(bounds, max_cells) {
            return None;
        }
        let (i0, j0, i1, j1) = bounds;
        let mut out = Vec::new();
        for i in i0..=i1 {
            for j in j0..=j1 {
                out.push((i, j));
            }
        }
        Some(out)
    }

    /// `floor(min/cell)`/`floor(max/cell)` per axis. `f64 as i32` saturates on an extreme
    /// coordinate, matching `cells_in_bounds`' own overflow behavior.
    fn cell_bounds(&self, min: vision::P, max: vision::P, cell: f64) -> (i32, i32, i32, i32) {
        (
            (min.0 / cell).floor() as i32,
            (min.1 / cell).floor() as i32,
            (max.0 / cell).floor() as i32,
            (max.1 / cell).floor() as i32,
        )
    }

    /// The 4 corners of cell `(i, j)`: `(i,j)`, `(i+1,j)`, `(i,j+1)`, `(i+1,j+1)` — bottom-left,
    /// bottom-right, top-left, top-right. `accumulate_visible_cells`'s lenient corner-probe loop
    /// iterates the result unordered, so this fixed order exists for deterministic tests
    /// (`square_cell_vertices_returns_four_corners_in_order`), not a functional dependency on it.
    fn cell_vertices(&self, c: Cell, cell: f64) -> Vec<vision::P> {
        let (i, j) = c;
        vec![
            (i as f64 * cell, j as f64 * cell),
            ((i + 1) as f64 * cell, j as f64 * cell),
            (i as f64 * cell, (j + 1) as f64 * cell),
            ((i + 1) as f64 * cell, (j + 1) as f64 * cell),
        ]
    }

    /// Delegates to `pathfinding::heuristic(self.rule, ...)` — the admissible+consistent
    /// `DiagonalRule`-based square estimate, covering all 4 diagonal rules.
    fn heuristic(&self, from: Cell, to: Cell) -> f64 {
        pathfinding::heuristic(self.rule, from, to)
    }

    fn world_units_per_cell(&self) -> f64 {
        self.cell
    }

    fn world_extent(&self, bounds_cells: (f64, f64)) -> WorldExtent {
        // Cell (i,j) covers [i*cell,(i+1)*cell) on each axis, so cell (0,0)'s own lower-left corner
        // IS the origin and a w × h block spans exactly (w*cell, h*cell) from it, with no shear and
        // no overhang.
        let Some((w, h)) = normalize_bounds_cells(bounds_cells) else {
            return REFUSED_EXTENT;
        };
        WorldExtent {
            min: (0.0, 0.0),
            max: (w * self.cell, h * self.cell),
        }
    }

    fn kind(&self) -> GridKind {
        GridKind::Square
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
            // y-fixup intentionally omitted: y is not part of the returned pair, so fixing
            // it up cannot change (x, z).
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

    /// Exposed for `hex_grid_axial_round_trip_pixel_to_axial_to_pixel`; not part of the
    /// `GridShape` trait.
    fn pixel_to_axial(&self, p: vision::P) -> (i32, i32) {
        let (qf, rf) = self.pixel_to_axial_frac(p);
        self.axial_round(qf, rf)
    }

    /// Distance from `p` to cell `c`'s hexagon: zero when `p` lies inside it, else the smallest
    /// distance to any of its six edges. Reads the SAME vertex ring `cell_vertices` supplies to
    /// the leniency corner test, so the footprint predicate and the corner sampler cannot
    /// disagree about a hex's geometry.
    fn distance_to_cell_polygon(&self, c: Cell, p: vision::P, cell: f64) -> f64 {
        let verts = self.cell_vertices(c, cell);
        if vision::point_in_poly(&verts, p) {
            return 0.0;
        }
        let mut best = f64::INFINITY;
        for k in 0..verts.len() {
            let a = verts[k];
            let b = verts[(k + 1) % verts.len()];
            best = best.min(vision::point_segment_distance(p, a, b));
        }
        best
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
        // DoS bound on the hex-distance span: the boundary-crossing count is
        // `Σ|Δψₖ| + 3 ≤ 4n + 3`, the same order as an `n + 1`-sample bound, so the worst case is
        // ~16k crossings at the same n cap.
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
                // written: the `n <= MAX_HEX_LINE_SAMPLES` gate already dominates it, since
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
    /// Cells whose hex geometry the footprint disc (center `ctr`, radius `r_scene`) overlaps.
    ///
    /// EXACT and INDEPENDENT of where `ctr` sits relative to `anchor`: a hex is included iff the
    /// distance from `ctr` to that hex's own polygon is at most `r_scene`. A centre-distance test
    /// against the inradius is not a safe substitute — a hex the disc reaches near one of its
    /// VERTICES has its centre up to `r_scene + size` away, and `size > √3/2·size` — and the
    /// callers that pass an arc-length sample point rather than a cell centre
    /// (`navmesh::clip_to_visible_mask`, `navmesh::los_smooth`) are exactly the ones that would
    /// lose those cells. Losing them LOOSENS the gates that read this set: a cell the token's body
    /// covers is then never required to be visible.
    ///
    /// Two cheap bounds settle most candidates without the polygon walk, both exact rather than
    /// approximate: a hex whose centre is within `r_scene + √3/2·size` necessarily overlaps
    /// (every hex contains its own inscribed disc of that radius), and a hex whose centre is
    /// beyond `r_scene + size` necessarily does not (every hex lies inside its circumscribed disc
    /// of that radius). Only the annulus between them needs the six edge distances.
    ///
    /// The scan is a hex-shaped ring neighbourhood of `anchor`, sized so it cannot miss a
    /// reachable hex: ring `k`'s centres are at least `1.5·size` per ring from `anchor`'s, `ctr`
    /// is at most `size` from `anchor`'s centre, and an overlapping hex's centre is at most
    /// `r_scene + size` from `ctr`. `anchor` is returned alone when nothing overlaps, mirroring
    /// the square implementation's zero-radius guarantee.
    fn footprint_cells(&self, anchor: Cell, ctr: vision::P, r_scene: f64, cell: f64) -> Vec<Cell> {
        // A zero-radius `ctr` is a literal point, not a disc: exactly on a shared hex edge or
        // vertex it sits at distance 0 from EVERY touching hex's polygon, which the annulus/
        // inradius tests below would otherwise tie across 2 or 3 hexes. `anchor` is always
        // `cell_of(ctr)` (the caller's own single-cell resolution), so this matches
        // `pathfinding::footprint_cells`'s identical r=0 resolution and the trait contract
        // (`GridShape::footprint_cells`'s doc: "the anchor cell is always included").
        if r_scene <= 0.0 {
            return vec![anchor];
        }
        let mut out = Vec::new();
        let r = r_scene.max(0.0);
        let inradius = self.size * 3.0_f64.sqrt() / 2.0;
        let ring_radius = ((r / (self.size * 1.5)).ceil() as i32).max(0) + 2;
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
                let d_center = (dx * dx + dy * dy).sqrt();
                if d_center <= r + inradius
                    || (d_center <= r + self.size
                        && self.distance_to_cell_polygon(c, ctr, cell) <= r)
                {
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
        let bounds = self.cell_bounds(min, max, cell);
        if exceeds_cell_cap(bounds, max_cells) {
            return None;
        }
        let (q0, r0, q1, r1) = bounds;
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
    /// `i32`. A square `DiagonalRule` distance OVERESTIMATES this for opposite-sign axial deltas
    /// (e.g. Manhattan is 2× on the `(1,-1)` axial line), which is why hex needs its own admissible
    /// heuristic rather than reusing a square one.
    fn heuristic(&self, from: Cell, to: Cell) -> f64 {
        let dq = to.0 as i64 - from.0 as i64;
        let dr = to.1 as i64 - from.1 as i64;
        ((dq.abs() + dr.abs() + (dq + dr).abs()) as f64) / 2.0
    }

    fn world_units_per_cell(&self) -> f64 {
        // Every axial neighbour is √3·size away: (1,0) → (√3·size, 0); (0,1) →
        // (√3/2·size, 3/2·size), whose length is size·√(3/4 + 9/4) = √3·size.
        self.size * 3.0_f64.sqrt()
    }

    fn world_extent(&self, bounds_cells: (f64, f64)) -> WorldExtent {
        // The far corner of the axial block is cell (w-1, h-1): `axial_to_pixel` is monotone
        // increasing in q on x, and in r on BOTH axes (the shear), so that cell maximises each
        // coordinate. The near corner is cell (0,0), which `axial_to_pixel` places AT the origin,
        // so it minimises each coordinate. Both corners then move out by the same pointy-top
        // half-extents — √3/2·size across the flats (x) and the circumradius `size` to a vertex
        // (y) — so each extreme hex's own far vertices are inside.
        let Some((w, h)) = normalize_bounds_cells(bounds_cells) else {
            return REFUSED_EXTENT;
        };
        let qmax = (w - 1.0).max(0.0);
        let rmax = (h - 1.0).max(0.0);
        let sqrt3 = 3.0_f64.sqrt();
        let half_x = self.size * sqrt3 / 2.0;
        let half_y = self.size;
        WorldExtent {
            min: (-half_x, -half_y),
            max: (
                self.size * (sqrt3 * qmax + sqrt3 / 2.0 * rmax) + half_x,
                self.size * 1.5 * rmax + half_y,
            ),
        }
    }

    fn kind(&self) -> GridKind {
        GridKind::Hex
    }
}

/// The ONE envelope that stands for "no usable rectangle here": zero AREA, not merely a zero far
/// corner. Every consumer refuses it on span (`WorldExtent::width`/`height`), so the refusal
/// survives the envelope carrying a `min` a caller could otherwise have subtracted a positive
/// width out of.
///
/// Both corners sitting at the origin is what keeps it from SHRINKING
/// `vision::bound_for_scene`'s union on any edge — the one consumer that unions it rather than
/// refusing it. It is not inert there: that union still pulls a low edge down to the origin
/// whenever the wall-derived bound does not already span it, exactly as a square scene's own
/// minimum does.
///
/// Two callers reach it for two different reasons and must not spell it two ways: both
/// `world_extent` impls answer a degenerate `bounds_cells` with it (via
/// `normalize_bounds_cells`), and `SceneEcs::world_extent_from` substitutes it for a scene its
/// grid-size map no longer carries.
pub(crate) const REFUSED_EXTENT: WorldExtent = WorldExtent {
    min: (0.0, 0.0),
    max: (0.0, 0.0),
};

/// Admit an authored `bounds_cells` pair to the range every `world_extent` impl computes over —
/// both axes finite and strictly positive — or `None` for anything else.
///
/// `None` is the ONE degenerate answer both impls give, and both return `REFUSED_EXTENT` for it — a
/// zero-area envelope the extent guards (`navmesh::build_navmesh`, `lighting::env_light_polys`)
/// already refuse. Clamping a degenerate axis to `0.0` and continuing is NOT sufficient, which is
/// why this returns an `Option`: `SquareGrid` then yields a zero span on that axis, but `HexGrid`
/// adds the pointy-top half-extents afterwards and yields a positive-area envelope that BUILDS. The
/// divergence, not either answer, is the defect — a mesh that small makes everything outside it
/// unreachable, so the direction is under-permissive rather than a leak, but one trait method must
/// not have two answers.
///
/// Non-finite and non-positive are the SAME input class here, not two: `0.0` and a negative axis
/// reach the two impls exactly as `NaN` does and split them exactly as far apart. Both are
/// unreachable from a scene document — `SceneEcs::resolve_scene`'s bounds filter rejects
/// non-finite and non-positive alike, substituting `DEFAULT_SCENE_BOUNDS_UNITS` — so this refusal
/// is the trait's own contract rather than a live guard, and it must cover the whole class or the
/// contract is only true of part of it.
///
/// A refused axis zeroes BOTH, since the guards refuse on either axis and a single degenerate
/// answer is what keeps the two impls comparable.
fn normalize_bounds_cells(bounds_cells: (f64, f64)) -> Option<(f64, f64)> {
    let (w, h) = bounds_cells;
    // The finiteness test is not redundant with the positivity test: `NaN` fails every comparison
    // and `+∞` satisfies `> 0.0`, so either would otherwise pass through as `Some`.
    if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return None;
    }
    Some((w, h))
}

/// Per-step move cost under `rule`, plus the carried parity for the next step.
///
/// Orthogonal steps always cost 1.0 and leave `parity` untouched. Diagonals cost per rule;
/// only `Alternating` consumes parity, charging 1.0/2.0 on alternate diagonals and flipping
/// the bit so the caller must thread the returned parity through consecutive steps.
///
/// The sole definition of this rule — `pathfinding::astar_leg` AND `move_exec::execute_move` both
/// reach it through the `GridShape` trait's neighbor enumeration rather than duplicating it, so
/// the A* cost, the executor's cost, and any other consumer cannot drift apart. The client
/// mirrors the same four rules in the client's `Grid.distance`.
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
mod tests;
