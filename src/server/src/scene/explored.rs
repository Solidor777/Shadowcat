//! Per-(scene, player) explored fog memory: a sparse set of visited grid cells,
//! accumulated monotonically from each vision recompute. Engine-owned geometry,
//! headless + pure (the DB round-trip lives in the repository). Clean-room.
//!
//! A cell `(i, j)` covers world rect `[i*size, (i+1)*size) × [j*size, (j+1)*size)`. A vision
//! recompute marks every cell whose CENTER lies inside any `visible` polygon (resolution = one
//! grid cell — sufficient for the dimmed "explored memory" layer). Accumulation is a
//! set union, so revisiting marks nothing new (bounded by O(explored area), no growth on revisit).

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::scene::grid_shape::GridShape;
use crate::scene::vision;
use crate::scene::GridKind;
use std::collections::BTreeSet;

/// Magic prefix of a serialized `ExploredSet`. A blob without it states no coordinate system for
/// its records and is unusable rather than assumed.
const EXPLORED_MAGIC: [u8; 4] = *b"SCEF";

/// Serialization format version. A blob at any other version is not decoded.
const EXPLORED_VERSION: u8 = 1;

/// Header length: magic, version, grid-kind tag.
const EXPLORED_HEADER_LEN: usize = EXPLORED_MAGIC.len() + 2;

/// Grid-kind tag byte for `kind`.
fn kind_tag(kind: GridKind) -> u8 {
    match kind {
        GridKind::Square => 0,
        GridKind::Hex => 1,
    }
}

/// A grid-cell coordinate. `BTreeSet` ordering gives a deterministic serialization.
pub type Cell = (i32, i32);

/// Hard cap on candidate cells scanned per polygon. The visibility polygon's bbox is bounded by
/// the scene's wall/viewpoint extent, but a wall authored at an extreme coordinate with a tiny
/// grid size could otherwise span billions of cells and stall the dispatch path. Exceeding the cap
/// skips the polygon (marks no cells → under-reveal, the fail-safe direction).
pub(crate) const MAX_CELLS_PER_POLYGON: i64 = 4_000_000;

/// Half-extent, in CELLS, of the window an over-cap candidate scan is clamped to.
///
/// Sized so the window itself can never be refused by `MAX_CELLS_PER_POLYGON`: a square window of
/// `2*HALF + 1` cells per side enumerates `(2*HALF + 1)^2` cells, and `HALF` is the largest value
/// keeping that product at or under the cap. Hex enumerates FEWER cells for the same pixel window
/// — the axial preimage of a pixel box is a sheared parallelogram whose integer bounding box is
/// smaller than the square index rectangle of the same box — so bounding the square case bounds
/// both, and `a_clamped_hex_window_also_stays_inside_the_per_polygon_cap` measures that through
/// `HexGrid::cell_bounds` rather than assuming it.
pub(crate) const SCAN_WINDOW_HALF_CELLS: i64 = 999;

/// Intersect a candidate-scan AABB with a window of `SCAN_WINDOW_HALF_CELLS` cells around `focus`,
/// but ONLY when the AABB's own candidate count exceeds `max_cells`.
///
/// An over-cap scan makes `GridShape::cells_in_bounds` return `None`, and every caller of that
/// primitive treats `None` as "skip this source/polygon" — an empty mask, which on the movement
/// gate refuses every move and on egress ships no cells. Clamping keeps such a scan enumerable, at
/// a bounded SUBSET of the unclamped candidate set: each caller's fail direction stays the
/// under-revealing one (fewer cells admitted, fewer cells shipped, fewer cells remembered), and
/// the outcome is a degradation the source survives rather than the source's whole contribution.
///
/// The span test is what keeps this from taking cells away from a scan that was never in trouble.
/// The cap bounds a PRODUCT of two cell counts; the window bounds a PER-AXIS distance from a focus
/// that sits wherever the source does, not at the box's centre. A box can therefore reach far
/// beyond the window on both axes and still enumerate fewer cells than the cap allows, and those
/// cells are in the mask a player moves through. So the span is computed first — through the same
/// `cell_bounds` + `saturating_mul` arithmetic `cells_in_bounds` applies — and a span within
/// `max_cells` returns `min`/`max` untouched.
///
/// PRECONDITION: `focus` lies inside `[min, max]`. All three callers satisfy it — a visibility
/// source sits inside its own LOS polygon's bbox, and `mark_polygons` uses that bbox's own centre.
/// A focus far enough outside that the window misses the box would otherwise yield `min > max`, an
/// inverted rectangle that enumerates nothing, so that case returns the box unchanged and lets the
/// callee's own cap decide.
///
/// Returns `min`/`max` unchanged for a degenerate `cell`, `focus` or box as well — the callee's
/// fail-closed `None` on a degenerate input is the correct outcome there and must not be masked.
pub(crate) fn clamp_scan_window(
    grid: &dyn GridShape,
    focus: vision::P,
    min: vision::P,
    max: vision::P,
    cell: f64,
    max_cells: i64,
) -> (vision::P, vision::P) {
    if !cell.is_finite()
        || cell <= 0.0
        || !focus.0.is_finite()
        || !focus.1.is_finite()
        || !min.0.is_finite()
        || !min.1.is_finite()
        || !max.0.is_finite()
        || !max.1.is_finite()
    {
        return (min, max);
    }
    let (i0, j0, i1, j1) = grid.cell_bounds(min, max, cell);
    let w = i1 as i64 - i0 as i64 + 1;
    let h = j1 as i64 - j0 as i64 + 1;
    if w.saturating_mul(h) <= max_cells {
        return (min, max);
    }
    let half_px = SCAN_WINDOW_HALF_CELLS as f64 * cell;
    let win_min = (min.0.max(focus.0 - half_px), min.1.max(focus.1 - half_px));
    let win_max = (max.0.min(focus.0 + half_px), max.1.min(focus.1 + half_px));
    if win_min.0 > win_max.0 || win_min.1 > win_max.1 {
        return (min, max);
    }
    (win_min, win_max)
}

/// A sparse explored-cell set for one (scene, player).
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct ExploredSet {
    /// Explored cells, ordered (deterministic wire/persistence output).
    cells: BTreeSet<Cell>,
}

impl ExploredSet {
    /// An empty set.
    ///
    /// # Examples
    ///
    /// ```
    /// let s = shadowcat::scene::explored::ExploredSet::new();
    /// assert!(s.is_empty());
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of explored cells.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether no cell has been explored.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Whether `c` is in the explored memory (the `Revealed` gate's second arm).
    pub fn contains(&self, c: Cell) -> bool {
        self.cells.contains(&c)
    }

    /// The cells in ascending (i, j) order.
    pub fn iter(&self) -> impl Iterator<Item = Cell> + '_ {
        self.cells.iter().copied()
    }

    /// Mark every cell whose center lies inside any polygon in `polys` (flat `[x,y,…]` coords),
    /// indexed through `grid` at `cell_size` world units per cell. Returns the count of newly-added
    /// cells (0 ⇒ no growth). `grid` supplies both the candidate-cell enumeration
    /// (`GridShape::cells_in_bounds`) and each candidate's center (`GridShape::cell_center`), so a
    /// hex scene indexes hex axial cells while a square scene's candidate enumeration and cell-center
    /// math reduce to exactly `floor(min/cell)..=floor(max/cell)` and `(i+0.5)*cell`.
    /// Correctness (the `Revealed` gate composes this set with `GridShape::line_traversal`
    /// move-cells) requires `grid` to be the SAME resolved shape (`resolve_grid_shape`) the gate and
    /// the vision mask use for this scene. A polygon whose bbox enumerates more than
    /// `MAX_CELLS_PER_POLYGON` candidate cells is clamped to a `SCAN_WINDOW_HALF_CELLS` window
    /// around that bbox's centre, marking a bounded subset; a bbox within the cap is enumerated
    /// whole. A DEGENERATE polygon (`cells_in_bounds` → `None`) is skipped (under-reveal) to bound
    /// the dispatch-path cost.
    pub(crate) fn mark_polygons(
        &mut self,
        polys: &[Vec<f64>],
        grid: &dyn GridShape,
        cell_size: f64,
    ) -> usize {
        if cell_size <= 0.0 {
            return 0;
        }
        let before = self.cells.len();
        for poly in polys {
            if poly.len() < 6 {
                continue; // need ≥3 points for an area
            }
            let pts: Vec<(f64, f64)> = poly.chunks_exact(2).map(|c| (c[0], c[1])).collect();
            let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for &(x, y) in &pts {
                minx = minx.min(x);
                miny = miny.min(y);
                maxx = maxx.max(x);
                maxy = maxy.max(y);
            }
            // Clamp before enumerating: a bbox whose candidate count exceeds the cap is
            // intersected with a window around its own centre, so the polygon marks a bounded
            // SUBSET of the cells that bbox covers. Fail direction: fewer cells remembered, which
            // under-reveals. `cells_in_bounds` still applies the cap, so a degenerate input fails
            // closed.
            let focus = ((minx + maxx) * 0.5, (miny + maxy) * 0.5);
            let (scan_min, scan_max) = clamp_scan_window(
                grid,
                focus,
                (minx, miny),
                (maxx, maxy),
                cell_size,
                MAX_CELLS_PER_POLYGON,
            );
            let Some(candidates) =
                grid.cells_in_bounds(scan_min, scan_max, cell_size, MAX_CELLS_PER_POLYGON)
            else {
                tracing::warn!("explored cell scan degenerate; skipping polygon");
                continue;
            };
            for c in candidates {
                let (cx, cy) = grid.cell_center(c);
                if point_in_poly(&pts, cx, cy) {
                    self.cells.insert(c);
                }
            }
        }
        self.cells.len() - before
    }

    /// Serialize as `SCEF`, a version byte, a grid-kind tag, then 8 bytes per cell (i32 i, i32 j,
    /// little-endian) in ascending order. `kind` is the grid family the cell indices are
    /// expressed in; `from_bytes` refuses a blob whose tag disagrees with the scene's current
    /// kind, because a square index and a hex axial index are different coordinate systems that
    /// share a representation.
    pub fn to_bytes(&self, kind: GridKind) -> Vec<u8> {
        let mut out = Vec::with_capacity(EXPLORED_HEADER_LEN + self.cells.len() * 8);
        out.extend_from_slice(&EXPLORED_MAGIC);
        out.push(EXPLORED_VERSION);
        out.push(kind_tag(kind));
        for &(i, j) in &self.cells {
            out.extend_from_slice(&i.to_le_bytes());
            out.extend_from_slice(&j.to_le_bytes());
        }
        out
    }

    /// Deserialize the `to_bytes` layout, refusing anything that is not this format at this
    /// version indexed in `kind`. Every refusal yields an EMPTY set: explored memory is
    /// best-effort and an empty set under-reveals, which is the safe direction for a fog gate. A
    /// trailing partial record is likewise dropped rather than erroring.
    pub fn from_bytes(b: &[u8], kind: GridKind) -> Self {
        if b.len() < EXPLORED_HEADER_LEN
            || b[..EXPLORED_MAGIC.len()] != EXPLORED_MAGIC
            || b[EXPLORED_MAGIC.len()] != EXPLORED_VERSION
            || b[EXPLORED_MAGIC.len() + 1] != kind_tag(kind)
        {
            return Self::default();
        }
        let mut cells = BTreeSet::new();
        for rec in b[EXPLORED_HEADER_LEN..].chunks_exact(8) {
            let i = i32::from_le_bytes([rec[0], rec[1], rec[2], rec[3]]);
            let j = i32::from_le_bytes([rec[4], rec[5], rec[6], rec[7]]);
            cells.insert((i, j));
        }
        Self { cells }
    }
}

/// Even-odd ray-cast point-in-polygon. Source: standard CG (Shimrat 1962; de Berg et al.).
fn point_in_poly(poly: &[(f64, f64)], px: f64, py: f64) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if ((yi > py) != (yj > py)) && (px < (xj - xi) * (py - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::grid_shape::{HexGrid, SquareGrid};
    use crate::scene::pathfinding::DiagonalRule;

    /// A square grid at `cell` size — the shape every square-parity test indexes through. The
    /// diagonal rule is irrelevant to `cells_in_bounds`/`cell_center`, so any rule serves.
    fn sq(cell: f64) -> SquareGrid {
        SquareGrid {
            cell,
            rule: DiagonalRule::Chebyshev,
        }
    }

    /// A square covering one cell's center marks exactly that cell (resolution = cell).
    #[test]
    fn marks_cells_whose_center_is_inside() {
        let mut set = ExploredSet::new();
        // A 100×100 square from (0,0) to (100,100); cell_size 100 → cell (0,0) center (50,50) is in.
        let poly = vec![0.0, 0.0, 100.0, 0.0, 100.0, 100.0, 0.0, 100.0];
        let grew = set.mark_polygons(&[poly], &sq(100.0), 100.0);
        assert_eq!(grew, 1);
        assert!(set.contains((0, 0)));
        assert!(!set.contains((1, 0)));
    }

    #[test]
    fn accumulation_is_monotone_no_growth_on_revisit() {
        let mut set = ExploredSet::new();
        let poly = vec![0.0, 0.0, 300.0, 0.0, 300.0, 300.0, 0.0, 300.0];
        let first = set.mark_polygons(std::slice::from_ref(&poly), &sq(100.0), 100.0);
        assert_eq!(first, 9); // a 3×3 block of cells
        let again = set.mark_polygons(&[poly], &sq(100.0), 100.0);
        assert_eq!(again, 0, "revisiting the same area adds no cells");
        assert_eq!(set.len(), 9);
    }

    #[test]
    fn round_trips_through_bytes_deterministically() {
        let mut set = ExploredSet::new();
        set.mark_polygons(
            &[vec![0.0, 0.0, 250.0, 0.0, 250.0, 250.0, 0.0, 250.0]],
            &sq(100.0),
            100.0,
        );
        let bytes = set.to_bytes(crate::scene::GridKind::Square);
        assert_eq!(bytes.len(), EXPLORED_HEADER_LEN + set.len() * 8);
        let back = ExploredSet::from_bytes(&bytes, crate::scene::GridKind::Square);
        assert_eq!(set, back);
    }

    #[test]
    fn from_bytes_drops_a_truncated_trailing_record() {
        let mut set = ExploredSet::new();
        set.mark_polygons(
            &[vec![0.0, 0.0, 100.0, 0.0, 100.0, 100.0, 0.0, 100.0]],
            &sq(100.0),
            100.0,
        );
        // One cell, via the real encoder, then a 2-byte truncated tail appended by hand.
        let mut bytes = set.to_bytes(crate::scene::GridKind::Square);
        bytes.extend_from_slice(&[0xAB, 0xCD]);
        let decoded = ExploredSet::from_bytes(&bytes, crate::scene::GridKind::Square);
        assert_eq!(decoded, set);
    }

    #[test]
    fn bounds_a_polygon_whose_bbox_exceeds_the_cell_cap_to_the_scan_window() {
        // A long thin strip: bbox ~9,000,000 × 4 cells at cell_size 1, over the 4M cap. The
        // enumeration is clamped to a window around the bbox centre rather than skipped, so
        // the polygon does bounded work instead of none.
        let mut set = ExploredSet::new();
        let strip = vec![0.0, 0.0, 9_000_000.0, 0.0, 9_000_000.0, 3.0, 0.0, 3.0];
        let grew = set.mark_polygons(&[strip], &sq(1.0), 1.0);
        assert!(
            grew > 0,
            "the clamped scan marks a bounded neighbourhood, not nothing"
        );
        // The bbox centre's own column sits at x = 4_500_000.
        assert!(
            set.contains((4_500_000, 0)),
            "the cell at the bbox centre's own column is marked"
        );
        let outside = 4_500_000 + SCAN_WINDOW_HALF_CELLS as i32 + 10;
        assert!(
            !set.contains((outside, 0)),
            "a cell far outside the window is not marked"
        );
    }

    #[test]
    fn empty_polygon_and_nonpositive_cell_size_mark_nothing() {
        let mut set = ExploredSet::new();
        assert_eq!(set.mark_polygons(&[vec![0.0, 0.0]], &sq(100.0), 100.0), 0); // < 3 points
        assert_eq!(
            set.mark_polygons(&[vec![0.0, 0.0, 9.0, 0.0, 9.0, 9.0]], &sq(1.0), 0.0),
            0
        ); // bad size
        assert!(set.is_empty());
    }

    #[test]
    fn a_blob_written_under_one_grid_kind_does_not_decode_under_the_other() {
        // Discrimination: fails if the header is absent, ignored on read, or compared loosely —
        // the assertion is that the SAME bytes yield cells under one kind and none under the
        // other, which no format lacking the tag can satisfy.
        let mut set = ExploredSet::new();
        set.mark_polygons(
            &[vec![0.0, 0.0, 250.0, 0.0, 250.0, 250.0, 0.0, 250.0]],
            &sq(100.0),
            100.0,
        );
        let bytes = set.to_bytes(crate::scene::GridKind::Square);
        assert_eq!(
            ExploredSet::from_bytes(&bytes, crate::scene::GridKind::Square),
            set
        );
        assert!(
            ExploredSet::from_bytes(&bytes, crate::scene::GridKind::Hex).is_empty(),
            "square-indexed fog must not be reinterpreted as hex axial cells"
        );
    }

    #[test]
    fn a_headerless_blob_decodes_to_nothing() {
        // A blob with no header states no coordinate system for its records, so it is unusable
        // rather than assumed. Under-reveal is the safe direction for fog memory.
        // Discrimination: fails if `from_bytes` parses bare 8-byte records.
        let mut bare = (1_i32).to_le_bytes().to_vec();
        bare.extend_from_slice(&(2_i32).to_le_bytes());
        assert!(ExploredSet::from_bytes(&bare, crate::scene::GridKind::Square).is_empty());
    }

    #[test]
    fn a_hex_blob_round_trips_under_its_own_kind() {
        // Discrimination: fails if the header is written but the record payload is mis-offset,
        // which a square-only round-trip test would not catch.
        let g = HexGrid { size: 100.0 };
        let (cx, cy) = g.cell_center((1, 0));
        let poly = vec![
            cx - 10.0,
            cy - 10.0,
            cx + 10.0,
            cy - 10.0,
            cx + 10.0,
            cy + 10.0,
            cx - 10.0,
            cy + 10.0,
        ];
        let mut set = ExploredSet::new();
        set.mark_polygons(&[poly], &g, 100.0);
        let bytes = set.to_bytes(crate::scene::GridKind::Hex);
        assert_eq!(
            ExploredSet::from_bytes(&bytes, crate::scene::GridKind::Hex),
            set
        );
    }

    /// On a hex grid, `mark_polygons` indexes HEX axial cells (through `GridShape`), not square
    /// cells: a polygon around a hex cell's center marks that hex's own axial coordinate, and the
    /// resulting `contains` composes with hex `line_traversal` move-cells. Proves the indexing is
    /// grid-shape-driven, not hardcoded square math.
    #[test]
    fn hex_grid_marks_the_hex_axial_cell_containing_a_covered_center() {
        let g = HexGrid { size: 100.0 };
        // Cover a small AABB tightly around hex (1,0)'s center; only that hex's center falls in it.
        let (cx, cy) = g.cell_center((1, 0));
        let poly = vec![
            cx - 10.0,
            cy - 10.0,
            cx + 10.0,
            cy - 10.0,
            cx + 10.0,
            cy + 10.0,
            cx - 10.0,
            cy + 10.0,
        ];
        let mut set = ExploredSet::new();
        let grew = set.mark_polygons(&[poly], &g, 100.0);
        assert_eq!(grew, 1, "exactly the one hex whose center the AABB covers");
        assert!(
            set.contains((1, 0)),
            "hex axial (1,0) is marked, not a square index"
        );
    }

    #[test]
    fn a_scan_wider_than_the_window_but_under_the_cap_is_returned_unchanged() {
        // The property the conditional application exists for: the cap bounds a PRODUCT while the
        // window bounds a PER-AXIS distance, so a box can reach far past the window on both axes
        // and still enumerate fewer cells than the cap allows. Such a box must not lose a single
        // candidate — its cells are in the mask today and a player can move to them.
        //
        // Discrimination: fails if the window is applied whenever the box is wider than it,
        // because the returned max would then be the window edge rather than the box edge. The
        // guard below keeps the test honest if `SCAN_WINDOW_HALF_CELLS` ever changes.
        let cell = 100.0;
        let g = sq(cell);
        let focus = (50.0, 50.0);
        let min = (-50.0, -50.0);
        let max = (150_000.0, 150_000.0); // 1502 × 1502 = 2_256_004 candidates, under the cap
        assert!(
            max.0 - focus.0 > SCAN_WINDOW_HALF_CELLS as f64 * cell,
            "fixture: the box must reach past the window, or the test proves nothing"
        );
        let (out_min, out_max) =
            clamp_scan_window(&g, focus, min, max, cell, MAX_CELLS_PER_POLYGON);
        assert_eq!((out_min, out_max), (min, max));
    }

    #[test]
    fn clamp_scan_window_bounds_a_scan_that_exceeds_the_cap() {
        // Discrimination: fails if the window is not centred on `focus`, if its half-extent is not
        // `SCAN_WINDOW_HALF_CELLS` cells, or if it expands rather than intersects — the low edges
        // already sit inside the window and must come back unchanged, while the high edges must
        // come back at the window.
        let cell = 100.0;
        let g = sq(cell);
        let focus = (50.0, 50.0);
        let half_px = SCAN_WINDOW_HALF_CELLS as f64 * cell;
        let (min, max) = ((-50.0, -50.0), (1.0e9, 1.0e9));
        let (out_min, out_max) =
            clamp_scan_window(&g, focus, min, max, cell, MAX_CELLS_PER_POLYGON);
        assert_eq!(
            out_min, min,
            "an edge already inside the window is untouched"
        );
        assert_eq!(out_max, (focus.0 + half_px, focus.1 + half_px));
    }

    #[test]
    fn a_window_that_misses_the_scan_box_leaves_it_unchanged() {
        // The precondition `clamp_scan_window` states: `focus` lies inside the box. A focus far
        // outside it would otherwise produce min > max — an inverted rectangle that enumerates
        // nothing, which is the total loss this clamp exists to remove, reintroduced as a silent
        // empty result.
        // Discrimination: fails if the intersection is returned without the emptiness check.
        let cell = 100.0;
        let g = sq(cell);
        let (min, max) = ((0.0, 0.0), (1.0e9, 1.0e9));
        assert_eq!(
            clamp_scan_window(&g, (-1.0e8, -1.0e8), min, max, cell, MAX_CELLS_PER_POLYGON),
            (min, max)
        );
    }

    #[test]
    fn a_clamped_square_window_stays_inside_the_per_polygon_cap() {
        // The window exists so that `cells_in_bounds` cannot refuse it.
        // Discrimination: fails if `SCAN_WINDOW_HALF_CELLS` is raised such that
        // `(2*half + 1)^2 > MAX_CELLS_PER_POLYGON`.
        let side = 2 * SCAN_WINDOW_HALF_CELLS + 1;
        assert!(
            side * side <= MAX_CELLS_PER_POLYGON,
            "the window enumerates {} cells against a {MAX_CELLS_PER_POLYGON} cap",
            side * side
        );
    }

    #[test]
    fn a_clamped_hex_window_also_stays_inside_the_per_polygon_cap() {
        // Square is the denser of the two shapes per unit of pixel area only if hex's axial
        // preimage of the same pixel box enumerates fewer cells. That is a claim about
        // `HexGrid::cell_bounds`, so it is measured through that function rather than argued in
        // prose. Discrimination: fails if the axial padding or the preimage arithmetic changes
        // such that a clamped hex window can be refused by the cap.
        let size = 100.0;
        let g = HexGrid { size };
        let half_px = SCAN_WINDOW_HALF_CELLS as f64 * size;
        let (q0, r0, q1, r1) = g.cell_bounds((-half_px, -half_px), (half_px, half_px), size);
        let span = (q1 as i64 - q0 as i64 + 1) * (r1 as i64 - r0 as i64 + 1);
        assert!(
            span <= MAX_CELLS_PER_POLYGON,
            "a clamped hex window enumerates {span} cells against a {MAX_CELLS_PER_POLYGON} cap"
        );
    }
}
