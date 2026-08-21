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

use crate::scene::grid_shape::{exceeds_cell_cap, GridShape};
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

/// Hard cap on candidate cells scanned per polygon/source. A wall or LOS bbox authored at an
/// extreme coordinate with a tiny grid size could otherwise span billions of cells and stall the
/// dispatch path. Every production call site (`mark_polygons`, `accumulate_visible_cells`,
/// `player_lit_mask`) reaches its scan box through `scan_box_for`, which intersects an over-cap box
/// with a bounded window around the source's own focus before any of them enumerate.
/// `cells_in_bounds` enforces this cap directly and returns `None` — the candidate set is then
/// skipped, under-reveal — when the box it receives is still over it (a degenerate window) or for
/// any caller that bypasses `scan_box_for`.
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

/// Which sampling mode a caller of `scan_box_for` wants for the box IT scans — never which box the
/// clamp decision is made from; `scan_box_for` always decides from the padded box regardless of
/// `mode` (see its doc).
pub(crate) enum ScanMode {
    /// The source's box as authored, unpadded: `mark_polygons`, `player_lit_mask`'s scan, and
    /// `accumulate_visible_cells`'s strict invocation.
    Strict,
    /// The box padded by one cell on every side (corner-sampling headroom):
    /// `accumulate_visible_cells`'s lenient invocation.
    Lenient,
}

/// Pad an AABB by `pad` scene units on every side. The one place a leniency pad is spelled out —
/// `scan_box_for` derives both the box a `Lenient` caller scans AND the (always-padded) decision
/// box from this single call, so widening leniency is a one-line change to `scan_box_for` alone.
pub(crate) fn pad_box(bbox: (vision::P, vision::P), pad: f64) -> (vision::P, vision::P) {
    let (min, max) = bbox;
    ((min.0 - pad, min.1 - pad), (max.0 + pad, max.1 + pad))
}

/// The ONE symbol that owns scan geometry for a source. Given the grid, the source's own focus
/// (a viewpoint, or a polygon's bbox centre), its unpadded `bbox`, the cell size and the cap,
/// returns the box a caller wanting `mode` should scan. No call site computes a pad, a decision
/// box, a span, or a cap comparison of its own — every production caller (`mark_polygons`,
/// `player_lit_mask`'s scan, both `accumulate_visible_cells` invocations) calls this and scans
/// exactly what it returns.
///
/// The clamp DECISION is always made from `pad_box(bbox, cell)` — the one-cell-padded box —
/// regardless of `mode`: it is the largest box ANY mode would scan for this source, so every
/// mode's own box, padded or not, is intersected against the SAME window. `Strict`'s unpadded box
/// is a subset of `Lenient`'s padded one (`A ⊆ A'`), and intersecting both with one window `W`
/// gives `A ∩ W ⊆ A' ∩ W` — GIVEN the PRECONDITION stated here, `strict ⊆ lenient` holds structurally,
/// not by argument about which branch ran; `clamp_scan_window`'s inverted-window fallback is the
/// one branch this does NOT hold across, since it returns `actual` unchanged rather than
/// `actual ∩ W` and is reachable only when the precondition fails (see PRECONDITION). Deciding
/// independently per mode instead leaves a reachable band where the smaller unpadded box sits at
/// or under the cap (returned whole) while the larger padded box exceeds it (windowed), so the
/// unclamped result can hold a cell the windowed one does not.
///
/// This is also why `player_lit_mask`'s scan — which has no lenient counterpart of its own — must
/// still call this with `Strict` rather than compute its box directly: its decision is the SAME
/// padded box `accumulate_visible_cells`'s strict call decides from, so the two produce IDENTICAL
/// candidate sets for the same source (`cell_visible`'s own doc states this parity as an
/// invariant). A caller that instead decided from its own unpadded box alone would sit below the
/// cap in a band where the padded decision clamps — enumerating strictly more cells than the
/// movement gate's own strict scan for the same source, an under-permissive divergence between
/// what a player is shown and what they may move through.
///
/// `mark_polygons` has no lenient counterpart at all — its one call always asks for `Strict` with
/// its own box as `bbox` — so the "largest box any mode would scan" it shares a decision with is
/// hypothetical, never actually scanned. For it, sharing the decision means: in the band where its
/// own unpadded box sits at or under the cap but the padded box does not, it clamps and marks a
/// bounded subset of the cells its own box alone would have covered. This is deliberate — uniform
/// treatment across all three callers is worth more than the narrow band it costs, and the
/// direction is under-reveal (fewer cells remembered), the same fail-safe direction every other
/// clamp outcome in this module takes.
///
/// PRECONDITION: `focus` lies inside `bbox` (and therefore inside the padded box, which only grows
/// it). Every caller satisfies it: a visibility source sits inside its own LOS polygon's bbox, and
/// `mark_polygons` uses that bbox's own centre. A focus far enough outside `bbox` that the window
/// misses the box being scanned returns that box unchanged and lets the callee's own cap decide,
/// rather than yielding an inverted, enumerates-nothing rectangle — this is the inverted-window
/// fallback this doc names.
///
/// Returns the mode's own box unchanged for a degenerate `cell`, `focus`, or `bbox` as well — the
/// callee's fail-closed `None` on a degenerate input is the correct outcome there and must not be
/// masked.
pub(crate) fn scan_box_for(
    grid: &dyn GridShape,
    focus: vision::P,
    bbox: (vision::P, vision::P),
    cell: f64,
    max_cells: i64,
    mode: ScanMode,
) -> (vision::P, vision::P) {
    let padded = pad_box(bbox, cell);
    let actual = match mode {
        ScanMode::Strict => bbox,
        ScanMode::Lenient => padded,
    };
    clamp_scan_window(grid, focus, actual, padded, cell, max_cells)
}

/// Intersect a candidate-scan AABB `actual = (min, max)` with a window of `SCAN_WINDOW_HALF_CELLS`
/// cells around `focus`, but ONLY when `decision = (decision_min, decision_max)`'s own candidate
/// count exceeds `max_cells`. The low-level intersection primitive `scan_box_for` builds on — see
/// that function's doc for why `decision` and `actual` must be allowed to differ.
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
/// cells are in the mask a player moves through. So the span is computed first, and `exceeds_cell_cap`
/// — the SAME predicate `cells_in_bounds` enforces against — decides whether `decision` is over the
/// cap; a `decision` within the cap returns `actual` untouched.
///
/// PRECONDITION: `focus` lies inside `actual` and inside `decision`. A focus far enough outside
/// `actual` that the window misses it would otherwise yield `min > max`, an inverted rectangle that
/// enumerates nothing, so that case returns `actual` unchanged and lets the callee's own cap
/// decide.
///
/// Returns `actual` unchanged for a degenerate `cell`, `focus`, or either box as well — the
/// callee's fail-closed `None` on a degenerate input is the correct outcome there and must not be
/// masked.
fn clamp_scan_window(
    grid: &dyn GridShape,
    focus: vision::P,
    actual: (vision::P, vision::P),
    decision: (vision::P, vision::P),
    cell: f64,
    max_cells: i64,
) -> (vision::P, vision::P) {
    let (min, max) = actual;
    let (decision_min, decision_max) = decision;
    if !cell.is_finite()
        || cell <= 0.0
        || !focus.0.is_finite()
        || !focus.1.is_finite()
        || !min.0.is_finite()
        || !min.1.is_finite()
        || !max.0.is_finite()
        || !max.1.is_finite()
        || !decision_min.0.is_finite()
        || !decision_min.1.is_finite()
        || !decision_max.0.is_finite()
        || !decision_max.1.is_finite()
    {
        return (min, max);
    }
    let bounds = grid.cell_bounds(decision_min, decision_max, cell);
    if !exceeds_cell_cap(bounds, max_cells) {
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
    /// indexed through `grid` at the `cell_size` INDEXING scale — the scalar `GridShape::cell_of`
    /// and `GridShape::cell_center` index against, never `GridShape::world_units_per_cell`, which
    /// measures an authored distance and is the larger of the two on hex. Returns the count of
    /// newly-added cells (0 ⇒ no growth). `grid` supplies both the candidate-cell enumeration
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
            let pts: Vec<(f64, f64)> = poly
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| (c[0], c[1]))
                .collect();
            let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for &(x, y) in &pts {
                minx = minx.min(x);
                miny = miny.min(y);
                maxx = maxx.max(x);
                maxy = maxy.max(y);
            }
            // The polygon's own centre is its focus; a single-mode caller, so `scan_box_for` marks
            // a bounded SUBSET of the bbox (under-reveal) whenever the bbox itself is over cap.
            let focus = ((minx + maxx) * 0.5, (miny + maxy) * 0.5);
            let bbox = ((minx, miny), (maxx, maxy));
            let (scan_min, scan_max) = scan_box_for(
                grid,
                focus,
                bbox,
                cell_size,
                MAX_CELLS_PER_POLYGON,
                ScanMode::Strict,
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
        for rec in b[EXPLORED_HEADER_LEN..].as_chunks::<8>().0 {
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
    use crate::scene::grid_shape::{candidate_span, HexGrid, SquareGrid};
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
        set.mark_polygons(&[poly], &g, g.size);
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
        let grew = set.mark_polygons(&[poly], &g, g.size);
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
        // fixture guard on the box's reach past the window keeps the test honest if
        // `SCAN_WINDOW_HALF_CELLS` ever changes.
        let cell = 100.0;
        let g = sq(cell);
        let focus = (50.0, 50.0);
        let min = (-50.0, -50.0);
        let max = (150_000.0, 150_000.0); // 1502 × 1502 = 2_256_004 candidates, under the cap
        assert!(
            max.0 - focus.0 > SCAN_WINDOW_HALF_CELLS as f64 * cell,
            "fixture: the box must reach past the window, or the test proves nothing"
        );
        let (out_min, out_max) = clamp_scan_window(
            &g,
            focus,
            (min, max),
            (min, max),
            cell,
            MAX_CELLS_PER_POLYGON,
        );
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
        let (out_min, out_max) = clamp_scan_window(
            &g,
            focus,
            (min, max),
            (min, max),
            cell,
            MAX_CELLS_PER_POLYGON,
        );
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
            clamp_scan_window(
                &g,
                (-1.0e8, -1.0e8),
                (min, max),
                (min, max),
                cell,
                MAX_CELLS_PER_POLYGON
            ),
            (min, max)
        );
    }

    #[test]
    fn clamp_scan_window_decides_from_the_decision_box_not_the_actual_box() {
        // A thin, wide actual box: comfortably under the cap on its own (span ≈ 40,002). The
        // decision box is far over the cap. Discrimination: fails if the clamp decision is
        // computed from the actual box instead of the decision box — the actual box would then
        // be returned unchanged, since it is under the cap by itself.
        let cell = 100.0;
        let g = sq(cell);
        let focus = (50.0, 50.0);
        let actual = ((-1.0e6, -50.0), (1.0e6, 50.0));
        let decision = ((-1.0e8, -1.0e8), (1.0e8, 1.0e8));
        let (out_min, out_max) =
            clamp_scan_window(&g, focus, actual, decision, cell, MAX_CELLS_PER_POLYGON);
        assert_ne!(
            (out_min, out_max),
            actual,
            "an over-cap decision box must clamp the actual box even when it is small alone"
        );
    }

    #[test]
    fn a_clamped_square_window_stays_inside_the_per_polygon_cap() {
        // The window exists so that `cells_in_bounds` cannot refuse it.
        // Discrimination: fails if `SCAN_WINDOW_HALF_CELLS` is raised such that
        // `(2*half + 1)^2 > MAX_CELLS_PER_POLYGON`.
        let side = 2 * SCAN_WINDOW_HALF_CELLS + 1;
        let bounds = (0, 0, (side - 1) as i32, (side - 1) as i32);
        assert!(
            !exceeds_cell_cap(bounds, MAX_CELLS_PER_POLYGON),
            "the window enumerates {} cells against a {MAX_CELLS_PER_POLYGON} cap",
            candidate_span(bounds)
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
        let bounds = g.cell_bounds((-half_px, -half_px), (half_px, half_px), size);
        assert!(
            !exceeds_cell_cap(bounds, MAX_CELLS_PER_POLYGON),
            "a clamped hex window enumerates {} cells against a {MAX_CELLS_PER_POLYGON} cap",
            candidate_span(bounds)
        );
    }

    #[test]
    fn scan_box_for_decides_from_the_padded_box_regardless_of_mode() {
        // Discrimination: fails if `scan_box_for` decides from the mode's own box (the unpadded
        // box for `Strict`) instead of always from the padded box — the strict box would then
        // return unclamped in a band where its padded counterpart is over cap, so its candidate
        // cells could reach past the (independently-windowed) lenient result's own edge.
        let cell = 1.0;
        let g = sq(cell);
        let focus = (100.0, 100.0);
        let bbox = ((0.0, 0.0), (1999.0, 1999.0));
        let padded = pad_box(bbox, cell);
        assert_eq!(
            candidate_span(g.cell_bounds(bbox.0, bbox.1, cell)),
            4_000_000,
            "fixture: the unpadded span must sit exactly at the cap"
        );
        assert!(
            candidate_span(g.cell_bounds(padded.0, padded.1, cell)) > MAX_CELLS_PER_POLYGON,
            "fixture: the padded span must exceed the cap"
        );
        let strict = scan_box_for(
            &g,
            focus,
            bbox,
            cell,
            MAX_CELLS_PER_POLYGON,
            ScanMode::Strict,
        );
        let half_px = SCAN_WINDOW_HALF_CELLS as f64 * cell;
        let expected = (
            (
                bbox.0 .0.max(focus.0 - half_px),
                bbox.0 .1.max(focus.1 - half_px),
            ),
            (
                bbox.1 .0.min(focus.0 + half_px),
                bbox.1 .1.min(focus.1 + half_px),
            ),
        );
        assert_eq!(
            strict, expected,
            "the strict box must be the window itself, not merely a different box"
        );
    }

    #[test]
    fn scan_box_for_lenient_mode_scans_the_padded_box() {
        // Discrimination: fails if `Lenient` returns the unpadded box, or pads by an amount other
        // than what `pad_box` derives.
        let cell = 100.0;
        let g = sq(cell);
        let focus = (50.0, 50.0);
        let bbox = ((0.0, 0.0), (100.0, 100.0)); // comfortably under the cap, padded or not
        let got = scan_box_for(
            &g,
            focus,
            bbox,
            cell,
            MAX_CELLS_PER_POLYGON,
            ScanMode::Lenient,
        );
        assert_eq!(got, pad_box(bbox, cell));
    }

    #[test]
    fn hex_strict_candidate_cells_nest_inside_lenient_candidate_cells_at_the_clamp_boundary() {
        // Hex twin of the square band fixture: box inclusion on hex runs through the axial
        // preimage bbox, `cube_round` and `HEX_BOUNDS_PAD` — never argued to inherit square's
        // floor monotonicity. Discrimination: fails if `scan_box_for`'s shared-decision fix does
        // not hold on hex, or if hex candidate-BOX inclusion does not imply candidate-CELL-set
        // inclusion (the concern square's `floor` argument cannot settle).
        let size = 1.0;
        let g = HexGrid { size };
        let focus = (0.0, 0.0);
        let bbox = ((0.0, 0.0), (2562.0, 2562.0));
        let padded = pad_box(bbox, size);
        let strict_span = candidate_span(g.cell_bounds(bbox.0, bbox.1, size));
        let lenient_span = candidate_span(g.cell_bounds(padded.0, padded.1, size));
        assert!(
            strict_span <= MAX_CELLS_PER_POLYGON,
            "fixture: the unpadded span must sit at or under the cap ({strict_span})"
        );
        assert!(
            lenient_span > MAX_CELLS_PER_POLYGON,
            "fixture: the padded span must exceed the cap ({lenient_span})"
        );
        let (strict_min, strict_max) = scan_box_for(
            &g,
            focus,
            bbox,
            size,
            MAX_CELLS_PER_POLYGON,
            ScanMode::Strict,
        );
        let (lenient_min, lenient_max) = scan_box_for(
            &g,
            focus,
            bbox,
            size,
            MAX_CELLS_PER_POLYGON,
            ScanMode::Lenient,
        );
        let strict_cells: BTreeSet<Cell> = g
            .cells_in_bounds(strict_min, strict_max, size, MAX_CELLS_PER_POLYGON)
            .expect("strict window stays inside the cap by construction")
            .into_iter()
            .collect();
        let lenient_cells: BTreeSet<Cell> = g
            .cells_in_bounds(lenient_min, lenient_max, size, MAX_CELLS_PER_POLYGON)
            .expect("lenient window stays inside the cap by construction")
            .into_iter()
            .collect();
        assert!(
            !strict_cells.is_empty(),
            "fixture: the strict scan must reach at least one candidate cell"
        );
        assert!(
            strict_cells.is_subset(&lenient_cells),
            "strict candidate cells must nest inside lenient candidate cells on hex too"
        );
    }
}
