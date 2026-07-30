//! Per-(scene, player) explored fog memory (M9c): a sparse set of visited grid cells,
//! accumulated monotonically from each vision recompute. Engine-owned geometry (#6),
//! headless + pure (the DB round-trip lives in the repository). Clean-room.
//!
//! A cell `(i, j)` covers world rect `[i*size, (i+1)*size) × [j*size, (j+1)*size)`. A vision
//! recompute marks every cell whose CENTER lies inside any `visible` polygon (resolution = one
//! grid cell — sufficient for the dimmed "explored memory" layer per spec §7). Accumulation is a
//! set union, so revisiting marks nothing new (bounded by O(explored area), no growth on revisit).

use crate::scene::grid_shape::GridShape;
use std::collections::BTreeSet;

/// A grid-cell coordinate. `BTreeSet` ordering gives a deterministic serialization.
pub type Cell = (i32, i32);

/// Hard cap on candidate cells scanned per polygon. The visibility polygon's bbox is bounded by
/// the scene's wall/viewpoint extent, but a wall authored at an extreme coordinate with a tiny
/// grid size could otherwise span billions of cells and stall the dispatch path. Exceeding the cap
/// skips the polygon (marks no cells → under-reveal, the fail-safe direction).
pub(crate) const MAX_CELLS_PER_POLYGON: i64 = 4_000_000;

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
    /// hex scene indexes hex axial cells while a square scene stays byte-identical to the prior
    /// hardcoded `floor(min/cell)..=floor(max/cell)` rectangle + `(i+0.5)*cell` center math.
    /// Correctness (the `Revealed` gate composes this set with `GridShape::line_traversal`
    /// move-cells) requires `grid` to be the SAME resolved shape (`resolve_grid_shape`) the gate and
    /// the vision mask use for this scene. A polygon whose bbox is over-cap
    /// (> `MAX_CELLS_PER_POLYGON`) or degenerate (`cells_in_bounds` → `None`) is skipped
    /// (under-reveal) to bound the dispatch-path cost.
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
            // `cells_in_bounds` is a SUPERSET candidate filter (never misses a cell whose center is
            // in the AABB); `None` on the same over-cap/degenerate conditions the prior inline scan's
            // `MAX_CELLS_PER_POLYGON`/`saturating_mul` guard enforced → skip (under-reveal, fail-safe).
            let Some(candidates) =
                grid.cells_in_bounds((minx, miny), (maxx, maxy), cell_size, MAX_CELLS_PER_POLYGON)
            else {
                tracing::warn!("explored cell scan over-cap or degenerate; skipping polygon");
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

    /// Serialize to 8 bytes per cell (i32 i, i32 j, little-endian), in ascending order.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.cells.len() * 8);
        for &(i, j) in &self.cells {
            out.extend_from_slice(&i.to_le_bytes());
            out.extend_from_slice(&j.to_le_bytes());
        }
        out
    }

    /// Deserialize from the `to_bytes` layout. A trailing partial record (corrupt/truncated blob)
    /// is dropped rather than erroring — explored memory is best-effort, and dropping under-reveals.
    pub fn from_bytes(b: &[u8]) -> Self {
        let mut cells = BTreeSet::new();
        for rec in b.chunks_exact(8) {
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
        let bytes = set.to_bytes();
        assert_eq!(bytes.len(), set.len() * 8);
        let back = ExploredSet::from_bytes(&bytes);
        assert_eq!(set, back);
    }

    #[test]
    fn from_bytes_drops_a_truncated_trailing_record() {
        let mut bytes = ((1_i32).to_le_bytes()).to_vec();
        bytes.extend_from_slice(&(2_i32).to_le_bytes()); // one valid cell (1,2)
        bytes.extend_from_slice(&[0xAB, 0xCD]); // a 2-byte truncated tail
        let set = ExploredSet::from_bytes(&bytes);
        assert_eq!(set.len(), 1);
        assert!(set.contains((1, 2)));
    }

    #[test]
    fn skips_a_polygon_whose_bbox_exceeds_the_cell_cap() {
        let mut set = ExploredSet::new();
        // A 3000×3000 polygon at cell_size 1 → 9,000,000 candidate cells > the 4M cap → skipped
        // (under-reveal) rather than stalling the dispatch path.
        let big = vec![0.0, 0.0, 3000.0, 0.0, 3000.0, 3000.0, 0.0, 3000.0];
        assert_eq!(set.mark_polygons(&[big], &sq(1.0), 1.0), 0);
        assert!(set.is_empty());
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
}
