//! Server-authoritative grid A* pathfinder (M10e-6). Pure + headless: callers pass parsed inputs
//! (walls, mask, cell size, rule, footprint); this module owns no I/O and borrows no ECS.
//! Engine-owned geometry (ARCHITECTURE §6 exception); clean-room A* (Hart, Nilsson & Raphael 1968).
//!
//! INVARIANT (spec §13): the per-cell mask test consumes the SAME `visible_cells` set the M10e-4
//! movement gate uses — the route can never thread the unknown nor leak hidden geometry.

/// Grid diagonal-cost rule (from `world-settings.pathfinding.diagonalRule`). All four are the same
/// king-move graph; they differ only in diagonal cost + the admissible heuristic. `Alternating`
/// (PF1e/3.5 "5-10-5") costs diagonals 1,2,1,2… and so requires a parity bit in the search node.
// TODO: remove once the A* body consumes this enum.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagonalRule {
    Chebyshev,
    Manhattan,
    Euclidean,
    Alternating,
}

/// Parse the diagonal-rule string; unknown/missing ⇒ `Chebyshev` (mirrors the client
/// `DEFAULT_WORLD_SETTINGS.pathfinding.diagonalRule` in `scene-docs.ts`).
// TODO: remove once the A* body calls this.
#[allow(dead_code)]
pub fn parse_diagonal_rule(s: &str) -> DiagonalRule {
    match s {
        "manhattan" => DiagonalRule::Manhattan,
        "euclidean" => DiagonalRule::Euclidean,
        "alternating" => DiagonalRule::Alternating,
        _ => DiagonalRule::Chebyshev,
    }
}

use crate::scene::vision::{self, point_segment_distance};
use std::collections::BTreeSet;

/// A grid cell `(i, j)`; cell `(i,j)` covers `[i*cell,(i+1)*cell) × [j*cell,(j+1)*cell)`.
// TODO: remove once the A* body uses this.
#[allow(dead_code)]
pub type Cell = (i32, i32);

/// Assembled, borrow-only inputs for one A* search. `mask = None` ⇒ unconstrained (GM or
/// `unrestricted`); `Some(set)` ⇒ a cell (and every footprint-overlapped cell) must be in the set.
/// `window` (i0,j0,i1,j1 inclusive) bounds the search so a GM query with an unreachable goal can't
/// wander unboundedly.
// TODO: remove once the A* body uses this.
#[allow(dead_code)]
pub struct PathGrid<'a> {
    pub cell: f64,
    pub rule: DiagonalRule,
    pub footprint_radius_cells: f64,
    pub walls: &'a [vision::Seg],
    pub mask: Option<&'a BTreeSet<Cell>>,
    pub window: (i32, i32, i32, i32),
}

/// Center of cell `c` in scene coords.
// TODO: remove once the A* body uses this.
#[allow(dead_code)]
pub fn cell_center(c: Cell, cell: f64) -> vision::P {
    ((c.0 as f64 + 0.5) * cell, (c.1 as f64 + 0.5) * cell)
}

/// Cells whose AABB the footprint disc (center `ctr`, radius `r_scene`) overlaps. A cell overlaps
/// the disc iff the disc center is within `r_scene` of the cell's AABB. The anchor cell is always
/// included (a zero-radius disc overlaps exactly its own cell).
fn footprint_cells(anchor: Cell, ctr: vision::P, r_scene: f64, cell: f64) -> Vec<Cell> {
    let mut out = Vec::new();
    let i0 = ((ctr.0 - r_scene) / cell).floor() as i32;
    let i1 = ((ctr.0 + r_scene) / cell).floor() as i32;
    let j0 = ((ctr.1 - r_scene) / cell).floor() as i32;
    let j1 = ((ctr.1 + r_scene) / cell).floor() as i32;
    for i in i0..=i1 {
        for j in j0..=j1 {
            // Distance from disc center to this cell's AABB.
            let minx = i as f64 * cell;
            let maxx = (i + 1) as f64 * cell;
            let miny = j as f64 * cell;
            let maxy = (j + 1) as f64 * cell;
            let dx = (minx - ctr.0).max(0.0).max(ctr.0 - maxx);
            let dy = (miny - ctr.1).max(0.0).max(ctr.1 - maxy);
            if dx * dx + dy * dy <= r_scene * r_scene {
                out.push((i, j));
            }
        }
    }
    if out.is_empty() {
        out.push(anchor);
    }
    out
}

/// Whether a token may step from `from` into `to`. INVARIANT (spec §4.3): full geometric footprint
/// clearance — (1) the footprint disc at `to` clears every `blocksMove` wall, (2) every
/// footprint-overlapped cell is in the mask (non-GM), (3) the center step `from→to` crosses no wall.
// TODO: remove once the A* body calls this.
#[allow(dead_code)]
pub(crate) fn cell_enterable(grid: &PathGrid, from: Cell, to: Cell) -> bool {
    let (i0, j0, i1, j1) = grid.window;
    if to.0 < i0 || to.0 > i1 || to.1 < j0 || to.1 > j1 {
        return false;
    }
    let r_scene = grid.footprint_radius_cells.max(0.0) * grid.cell;
    let ctr = cell_center(to, grid.cell);

    // (1) Footprint disc vs every blocksMove wall.
    for w in grid.walls {
        if point_segment_distance(ctr, w.a, w.b) < r_scene {
            return false;
        }
    }
    // (2) Mask: every footprint-overlapped cell must be visible/revealed (non-GM).
    if let Some(mask) = grid.mask {
        for c in footprint_cells(to, ctr, r_scene, grid.cell) {
            if !mask.contains(&c) {
                return false;
            }
        }
    }
    // (3) Center-to-center step clears every wall (reuses the M9 segment-cross predicate).
    let a = cell_center(from, grid.cell);
    for w in grid.walls {
        if crate::scene::segments_cross(a, ctr, w.a, w.b) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::vision::Seg;
    use std::collections::BTreeSet;

    fn grid<'a>(
        walls: &'a [Seg],
        mask: Option<&'a BTreeSet<Cell>>,
        footprint: f64,
    ) -> PathGrid<'a> {
        PathGrid { cell: 100.0, rule: DiagonalRule::Chebyshev, footprint_radius_cells: footprint,
                   walls, mask, window: (-100, -100, 100, 100) }
    }

    #[test]
    fn open_neighbor_is_enterable_no_walls_no_mask() {
        let walls: Vec<Seg> = vec![];
        let g = grid(&walls, None, 0.2);
        assert!(cell_enterable(&g, (0, 0), (1, 0)));
        assert!(cell_enterable(&g, (0, 0), (1, 1)));
    }

    #[test]
    fn step_crossing_a_blocks_move_wall_is_not_enterable() {
        // A vertical wall on the x=100 grid line blocks the (0,0)->(1,0) center step (50,50)->(150,50).
        let walls = vec![Seg { a: (100.0, 0.0), b: (100.0, 200.0) }];
        let g = grid(&walls, None, 0.2);
        assert!(!cell_enterable(&g, (0, 0), (1, 0)), "center step crosses the wall");
    }

    #[test]
    fn footprint_disc_too_wide_for_a_gap_is_not_enterable() {
        // Two walls one cell apart (x=100 and x=200). A footprint radius 0.7 cell (=70 units) at the
        // center of cell (1,0) (center x=150) is within 50 units of BOTH walls → blocked (the body
        // can't fit the 1-cell gap). A small radius (0.2 cell = 20 units) clears it.
        let walls = vec![
            Seg { a: (100.0, 0.0), b: (100.0, 200.0) },
            Seg { a: (200.0, 0.0), b: (200.0, 200.0) },
        ];
        let wide = grid(&walls, None, 0.7);
        let narrow = grid(&walls, None, 0.2);
        // Use a step that does not itself cross a wall: (1,1)->(1,0) (vertical, x=150 throughout).
        assert!(!cell_enterable(&wide, (1, 1), (1, 0)), "wide footprint cannot fit the gap");
        assert!(cell_enterable(&narrow, (1, 1), (1, 0)), "narrow footprint fits");
    }

    #[test]
    fn footprint_cell_outside_mask_is_not_enterable() {
        // Non-GM mask containing only cell (1,0). A footprint disc that overlaps neighbors requires
        // all overlapped cells in the mask. Radius 0.6 cell at center of (1,0) overlaps (0,0)/(2,0)/
        // (1,-1)/(1,1) edges → those must be in mask. With only (1,0) present → not enterable.
        let walls: Vec<Seg> = vec![];
        let mut mask = BTreeSet::new();
        mask.insert((1, 0));
        let g = grid(&walls, Some(&mask), 0.6);
        assert!(!cell_enterable(&g, (1, 1), (1, 0)), "overlapped neighbor cells not in mask");

        // A point-sized footprint overlaps only (1,0) → enterable.
        let gp = grid(&walls, Some(&mask), 0.0);
        assert!(cell_enterable(&gp, (1, 1), (1, 0)));
    }

    #[test]
    fn cell_outside_window_is_not_enterable() {
        let walls: Vec<Seg> = vec![];
        let mut g = grid(&walls, None, 0.2);
        g.window = (0, 0, 2, 2);
        assert!(!cell_enterable(&g, (2, 2), (3, 2)), "outside the search window");
    }
}
