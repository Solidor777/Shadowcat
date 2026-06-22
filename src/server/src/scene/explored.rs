//! Per-(scene, player) explored fog memory (M9c): a sparse set of visited grid cells,
//! accumulated monotonically from each vision recompute. Engine-owned geometry (#6),
//! headless + pure (the DB round-trip lives in the repository). Clean-room.
//!
//! A cell `(i, j)` covers world rect `[i*size, (i+1)*size) × [j*size, (j+1)*size)`. A vision
//! recompute marks every cell whose CENTER lies inside any `visible` polygon (resolution = one
//! grid cell — sufficient for the dimmed "explored memory" layer per spec §7). Accumulation is a
//! set union, so revisiting marks nothing new (bounded by O(explored area), no growth on revisit).

use std::collections::BTreeSet;

/// A grid-cell coordinate. `BTreeSet` ordering gives a deterministic serialization.
pub type Cell = (i32, i32);

/// Hard cap on candidate cells scanned per polygon. The visibility polygon's bbox is bounded by
/// the scene's wall/viewpoint extent, but a wall authored at an extreme coordinate with a tiny
/// grid size could otherwise span billions of cells and stall the dispatch path. Exceeding the cap
/// skips the polygon (marks no cells → under-reveal, the fail-safe direction).
const MAX_CELLS_PER_POLYGON: i64 = 4_000_000;

/// A sparse explored-cell set for one (scene, player).
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct ExploredSet {
    cells: BTreeSet<Cell>,
}

impl ExploredSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn contains(&self, c: Cell) -> bool {
        self.cells.contains(&c)
    }

    /// The cells in ascending (i, j) order.
    pub fn iter(&self) -> impl Iterator<Item = Cell> + '_ {
        self.cells.iter().copied()
    }

    /// Mark every cell whose center lies inside any polygon in `polys` (flat `[x,y,…]` coords),
    /// at `cell_size` world units per cell. Returns the count of newly-added cells (0 ⇒ no growth).
    /// Each polygon's candidate cells are bounded by its bbox (the visibility polygon is
    /// wall/viewpoint-bounded for a sane scene); a polygon whose bbox would span more than
    /// `MAX_CELLS_PER_POLYGON` cells is skipped (under-reveal) to bound the dispatch-path cost.
    pub fn mark_polygons(&mut self, polys: &[Vec<f64>], cell_size: f64) -> usize {
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
            // `f64 as i32` saturates (no UB) on an extreme coordinate; the cap below then skips it.
            let i0 = (minx / cell_size).floor() as i32;
            let i1 = (maxx / cell_size).floor() as i32;
            let j0 = (miny / cell_size).floor() as i32;
            let j1 = (maxy / cell_size).floor() as i32;
            let span = (i1 as i64 - i0 as i64 + 1) * (j1 as i64 - j0 as i64 + 1);
            if span > MAX_CELLS_PER_POLYGON {
                tracing::warn!(span, "explored cell scan exceeds cap; skipping polygon");
                continue;
            }
            for i in i0..=i1 {
                for j in j0..=j1 {
                    let cx = (i as f64 + 0.5) * cell_size;
                    let cy = (j as f64 + 0.5) * cell_size;
                    if point_in_poly(&pts, cx, cy) {
                        self.cells.insert((i, j));
                    }
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

    /// A square covering one cell's center marks exactly that cell (resolution = cell).
    #[test]
    fn marks_cells_whose_center_is_inside() {
        let mut set = ExploredSet::new();
        // A 100×100 square from (0,0) to (100,100); cell_size 100 → cell (0,0) center (50,50) is in.
        let poly = vec![0.0, 0.0, 100.0, 0.0, 100.0, 100.0, 0.0, 100.0];
        let grew = set.mark_polygons(&[poly], 100.0);
        assert_eq!(grew, 1);
        assert!(set.contains((0, 0)));
        assert!(!set.contains((1, 0)));
    }

    #[test]
    fn accumulation_is_monotone_no_growth_on_revisit() {
        let mut set = ExploredSet::new();
        let poly = vec![0.0, 0.0, 300.0, 0.0, 300.0, 300.0, 0.0, 300.0];
        let first = set.mark_polygons(std::slice::from_ref(&poly), 100.0);
        assert_eq!(first, 9); // a 3×3 block of cells
        let again = set.mark_polygons(&[poly], 100.0);
        assert_eq!(again, 0, "revisiting the same area adds no cells");
        assert_eq!(set.len(), 9);
    }

    #[test]
    fn round_trips_through_bytes_deterministically() {
        let mut set = ExploredSet::new();
        set.mark_polygons(
            &[vec![0.0, 0.0, 250.0, 0.0, 250.0, 250.0, 0.0, 250.0]],
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
        assert_eq!(set.mark_polygons(&[big], 1.0), 0);
        assert!(set.is_empty());
    }

    #[test]
    fn empty_polygon_and_nonpositive_cell_size_mark_nothing() {
        let mut set = ExploredSet::new();
        assert_eq!(set.mark_polygons(&[vec![0.0, 0.0]], 100.0), 0); // < 3 points
        assert_eq!(
            set.mark_polygons(&[vec![0.0, 0.0, 9.0, 0.0, 9.0, 9.0]], 0.0),
            0
        ); // bad size
        assert!(set.is_empty());
    }
}
