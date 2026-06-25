//! Server-authoritative grid A* pathfinder (M10e-6). Pure + headless: callers pass parsed inputs
//! (walls, mask, cell size, rule, footprint); this module owns no I/O and borrows no ECS.
//! Engine-owned geometry (ARCHITECTURE §6 exception); clean-room A* (Hart, Nilsson & Raphael 1968).
//!
//! INVARIANT (spec §13): the per-cell mask test consumes the SAME `visible_cells` set the M10e-4
//! movement gate uses — the route can never thread the unknown nor leak hidden geometry.

/// Grid diagonal-cost rule (from `world-settings.pathfinding.diagonalRule`). All four are the same
/// king-move graph; they differ only in diagonal cost + the admissible heuristic. `Alternating`
/// (PF1e/3.5 "5-10-5") costs diagonals 1,2,1,2… and so requires a parity bit in the search node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagonalRule {
    Chebyshev,
    Manhattan,
    Euclidean,
    Alternating,
}

/// Parse the diagonal-rule string; unknown/missing ⇒ `Chebyshev` (mirrors the client
/// `DEFAULT_WORLD_SETTINGS.pathfinding.diagonalRule` in `scene-docs.ts`).
// TODO: remove allow once the ECS pathfind handler calls this.
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
pub type Cell = (i32, i32);

/// Assembled, borrow-only inputs for one A* search. `mask = None` ⇒ unconstrained (GM or
/// `unrestricted`); `Some(set)` ⇒ a cell (and every footprint-overlapped cell) must be in the set.
/// `window` (i0,j0,i1,j1 inclusive) bounds the search so a GM query with an unreachable goal can't
/// wander unboundedly.
pub struct PathGrid<'a> {
    pub cell: f64,
    pub rule: DiagonalRule,
    pub footprint_radius_cells: f64,
    pub walls: &'a [vision::Seg],
    pub mask: Option<&'a BTreeSet<Cell>>,
    pub window: (i32, i32, i32, i32),
}

/// Center of cell `c` in scene coords.
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
mod astar_tests {
    use super::*;
    use crate::scene::vision::Seg;

    fn open(rule: DiagonalRule, footprint: f64) -> PathGrid<'static> {
        const NO_WALLS: [Seg; 0] = [];
        PathGrid {
            cell: 100.0,
            rule,
            footprint_radius_cells: footprint,
            walls: &NO_WALLS,
            mask: None,
            window: (-50, -50, 50, 50),
        }
    }

    #[test]
    fn chebyshev_diagonal_is_cost_one_per_step() {
        let g = open(DiagonalRule::Chebyshev, 0.1);
        let (cells, cost, _p) = astar_leg(&g, (0, 0), (3, 3), 0).unwrap();
        assert!((cost - 3.0).abs() < 1e-9, "3 diagonal steps at cost 1 each");
        assert_eq!(cells.first(), Some(&(0, 0)));
        assert_eq!(cells.last(), Some(&(3, 3)));
    }

    #[test]
    fn manhattan_diagonal_costs_two() {
        let g = open(DiagonalRule::Manhattan, 0.1);
        let (_c, cost, _p) = astar_leg(&g, (0, 0), (2, 2), 0).unwrap();
        // Manhattan distance to (2,2) is 4 whether via diagonals (cost 2 each) or orthogonals.
        assert!((cost - 4.0).abs() < 1e-9);
    }

    #[test]
    fn euclidean_diagonal_costs_sqrt2() {
        let g = open(DiagonalRule::Euclidean, 0.1);
        let (_c, cost, _p) = astar_leg(&g, (0, 0), (1, 1), 0).unwrap();
        assert!((cost - std::f64::consts::SQRT_2).abs() < 1e-9);
    }

    #[test]
    fn alternating_five_ten_five_parity() {
        // Two consecutive diagonals from parity 0: first costs 1, second costs 2 → total 3, end parity 0.
        let g = open(DiagonalRule::Alternating, 0.1);
        let (_c, cost, parity) = astar_leg(&g, (0, 0), (2, 2), 0).unwrap();
        assert!((cost - 3.0).abs() < 1e-9, "5-10-5: diagonals cost 1 then 2");
        assert_eq!(parity, 0, "two diagonals → parity back to 0");

        // Three diagonals from parity 0: 1 + 2 + 1 = 4, end parity 1.
        let (_c, cost3, parity3) = astar_leg(&g, (0, 0), (3, 3), 0).unwrap();
        assert!((cost3 - 4.0).abs() < 1e-9);
        assert_eq!(parity3, 1);
    }

    #[test]
    fn walled_off_goal_is_unreachable() {
        // Box the goal cell in with blocksMove walls on all four sides → Unreachable (terminates,
        // bounded by the window).
        let c = 100.0;
        let walls = vec![
            Seg {
                a: (3.0 * c, 3.0 * c),
                b: (4.0 * c, 3.0 * c),
            },
            Seg {
                a: (3.0 * c, 4.0 * c),
                b: (4.0 * c, 4.0 * c),
            },
            Seg {
                a: (3.0 * c, 3.0 * c),
                b: (3.0 * c, 4.0 * c),
            },
            Seg {
                a: (4.0 * c, 3.0 * c),
                b: (4.0 * c, 4.0 * c),
            },
        ];
        let g = PathGrid {
            cell: c,
            rule: DiagonalRule::Chebyshev,
            footprint_radius_cells: 0.1,
            walls: &walls,
            mask: None,
            window: (-10, -10, 10, 10),
        };
        assert_eq!(astar_leg(&g, (0, 0), (3, 3), 0), Err(PathFail::Unreachable));
    }

    #[test]
    fn start_equals_goal_is_a_single_cell_zero_cost() {
        let g = open(DiagonalRule::Chebyshev, 0.1);
        let (cells, cost, p) = astar_leg(&g, (2, 2), (2, 2), 1).unwrap();
        assert_eq!(cells, vec![(2, 2)]);
        assert!(cost.abs() < 1e-9);
        assert_eq!(p, 1, "parity is carried unchanged when no step is taken");
    }
}

use std::collections::{BinaryHeap, HashMap};

/// Why a path request fails. Mapped to a `PathError` message at the wire boundary.
// TODO: remove allow once the `find` dispatcher calls this.
#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq)]
pub enum PathFail {
    Invalid,     // degenerate request (no destination, non-finite, out-of-range footprint)
    Unreachable, // no route within walls/mask/window
    Exceeded,    // search exceeded MAX_PATH_NODES (DoS backstop)
}

/// DoS backstop: total node expansions per leg. For non-GM the mask is the tighter bound; this caps
/// a GM search whose window is large.
// TODO: remove allow once the `find` dispatcher calls `astar_leg`.
#[allow(dead_code)]
pub(crate) const MAX_PATH_NODES: usize = 200_000;

/// f64 ordering wrapper for the min-heap. Orders by `f` ascending (via reversed `total_cmp`),
/// tie-broken by `(cell, parity)` so identical requests yield identical routes (determinism).
#[allow(dead_code)]
#[derive(PartialEq)]
struct QNode {
    f: f64,
    cell: Cell,
    parity: u8,
}
impl Eq for QNode {}
impl Ord for QNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Min-heap: smaller f is "greater". Reverse the f comparison; tie-break ascending on key.
        other
            .f
            .total_cmp(&self.f)
            .then_with(|| self.cell.cmp(&other.cell))
            .then_with(|| self.parity.cmp(&other.parity))
    }
}
impl PartialOrd for QNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Step cost and successor parity for moving by `(di, dj)` (each in -1..=1, not both 0) under `rule`
/// from a node with diagonal-parity `parity`. Source: standard grid metrics; `Alternating` is the
/// PF1e/3.5 5-10-5 rule (k-th diagonal costs 1 if k odd else 2).
#[allow(dead_code)]
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

/// Admissible + consistent heuristic from `c` to `goal` under `rule`.
#[allow(dead_code)]
fn heuristic(rule: DiagonalRule, c: Cell, goal: Cell) -> f64 {
    let di = (goal.0 - c.0).abs();
    let dj = (goal.1 - c.1).abs();
    let (dmax, dmin) = (di.max(dj) as f64, di.min(dj) as f64);
    match rule {
        // Alternating's optimistic bound assumes every diagonal is cheap (cost 1) → Chebyshev.
        DiagonalRule::Chebyshev | DiagonalRule::Alternating => dmax,
        DiagonalRule::Manhattan => (di + dj) as f64,
        DiagonalRule::Euclidean => (dmax - dmin) + std::f64::consts::SQRT_2 * dmin,
    }
}

/// A* over one leg `start → goal`. Node = `(cell, parity)`; goal is any node with `cell == goal`.
/// Returns the leg's cells (start..=goal), its cost, and the end parity (to thread into the next leg).
// TODO: remove allow once the `find` dispatcher calls this.
#[allow(dead_code)]
pub(crate) fn astar_leg(
    grid: &PathGrid,
    start: Cell,
    goal: Cell,
    start_parity: u8,
) -> Result<(Vec<Cell>, f64, u8), PathFail> {
    if start == goal {
        return Ok((vec![start], 0.0, start_parity));
    }
    let mut g_score: HashMap<(Cell, u8), f64> = HashMap::new();
    let mut came_from: HashMap<(Cell, u8), (Cell, u8)> = HashMap::new();
    let mut open = BinaryHeap::new();
    g_score.insert((start, start_parity), 0.0);
    open.push(QNode {
        f: heuristic(grid.rule, start, goal),
        cell: start,
        parity: start_parity,
    });

    let dirs = [
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (1, -1),
        (-1, 1),
        (-1, -1),
    ];
    let mut expansions = 0usize;

    while let Some(QNode { cell, parity, .. }) = open.pop() {
        let g = *g_score.get(&(cell, parity)).unwrap_or(&f64::INFINITY);
        if cell == goal {
            // Reconstruct start..=goal.
            let mut path = vec![cell];
            let mut node = (cell, parity);
            while let Some(&prev) = came_from.get(&node) {
                path.push(prev.0);
                node = prev;
            }
            path.reverse();
            return Ok((path, g, parity));
        }
        expansions += 1;
        if expansions > MAX_PATH_NODES {
            return Err(PathFail::Exceeded);
        }
        for (di, dj) in dirs {
            let next = (cell.0 + di, cell.1 + dj);
            if !cell_enterable(grid, cell, next) {
                continue;
            }
            let (sc, next_parity) = step_cost(grid.rule, di, dj, parity);
            let tentative = g + sc;
            let key = (next, next_parity);
            if tentative < *g_score.get(&key).unwrap_or(&f64::INFINITY) {
                came_from.insert(key, (cell, parity));
                g_score.insert(key, tentative);
                open.push(QNode {
                    f: tentative + heuristic(grid.rule, next, goal),
                    cell: next,
                    parity: next_parity,
                });
            }
        }
    }
    Err(PathFail::Unreachable)
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
        PathGrid {
            cell: 100.0,
            rule: DiagonalRule::Chebyshev,
            footprint_radius_cells: footprint,
            walls,
            mask,
            window: (-100, -100, 100, 100),
        }
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
        let walls = vec![Seg {
            a: (100.0, 0.0),
            b: (100.0, 200.0),
        }];
        let g = grid(&walls, None, 0.2);
        assert!(
            !cell_enterable(&g, (0, 0), (1, 0)),
            "center step crosses the wall"
        );
    }

    #[test]
    fn footprint_disc_too_wide_for_a_gap_is_not_enterable() {
        // Two walls one cell apart (x=100 and x=200). A footprint radius 0.7 cell (=70 units) at the
        // center of cell (1,0) (center x=150) is within 50 units of BOTH walls → blocked (the body
        // can't fit the 1-cell gap). A small radius (0.2 cell = 20 units) clears it.
        let walls = vec![
            Seg {
                a: (100.0, 0.0),
                b: (100.0, 200.0),
            },
            Seg {
                a: (200.0, 0.0),
                b: (200.0, 200.0),
            },
        ];
        let wide = grid(&walls, None, 0.7);
        let narrow = grid(&walls, None, 0.2);
        // Use a step that does not itself cross a wall: (1,1)->(1,0) (vertical, x=150 throughout).
        assert!(
            !cell_enterable(&wide, (1, 1), (1, 0)),
            "wide footprint cannot fit the gap"
        );
        assert!(
            cell_enterable(&narrow, (1, 1), (1, 0)),
            "narrow footprint fits"
        );
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
        assert!(
            !cell_enterable(&g, (1, 1), (1, 0)),
            "overlapped neighbor cells not in mask"
        );

        // A point-sized footprint overlaps only (1,0) → enterable.
        let gp = grid(&walls, Some(&mask), 0.0);
        assert!(cell_enterable(&gp, (1, 1), (1, 0)));
    }

    #[test]
    fn cell_outside_window_is_not_enterable() {
        let walls: Vec<Seg> = vec![];
        let mut g = grid(&walls, None, 0.2);
        g.window = (0, 0, 2, 2);
        assert!(
            !cell_enterable(&g, (2, 2), (3, 2)),
            "outside the search window"
        );
    }
}
