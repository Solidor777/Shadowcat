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
#[derive(Debug, PartialEq, Eq)]
pub enum PathFail {
    Invalid,     // degenerate request (no destination, non-finite, out-of-range footprint)
    Unreachable, // no route within walls/mask/window
    Exceeded,    // search exceeded MAX_PATH_NODES (DoS backstop)
}

/// DoS backstop: total node expansions per leg. For non-GM the mask is the tighter bound; this caps
/// a GM search whose window is large.
pub(crate) const MAX_PATH_NODES: usize = 200_000;

/// f64 ordering wrapper for the min-heap. Orders by `f` ascending (via reversed `total_cmp`),
/// tie-broken by `(cell, parity)` so identical requests yield identical routes (determinism).
/// `g` is payload for lazy-deletion stale-pop skip — it is NOT part of the ordering key.
#[derive(PartialEq)]
struct QNode {
    f: f64,
    g: f64,
    cell: Cell,
    parity: u8,
}
impl Eq for QNode {}
impl Ord for QNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Min-heap: smaller f is "greater". Reverse the f comparison; tie-break ascending on key.
        // `g` is intentionally excluded — it is payload, not an ordering key.
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

/// Consistent (not merely admissible) heuristic from `c` to `goal` under `rule`. Consistency
/// holds because Δh per king-move never exceeds that move's minimum step cost: orthogonal Δh ≤ 1 =
/// orthogonal cost; diagonal Δh ≤ √2 ≤ each rule's diagonal cost (Chebyshev 1, Manhattan 2,
/// Euclidean √2, Alternating 1); for Alternating the optimistic bound (Chebyshev = dmax) gives
/// Δh ≤ 1, which is the cheapest diagonal cost. Consistency is load-bearing: it makes the first
/// goal-pop optimal and makes the post-goal stale-pop skip safe (see `astar_leg`).
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
        g: 0.0,
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

    while let Some(QNode {
        cell,
        parity,
        g: g_popped,
        ..
    }) = open.pop()
    {
        if cell == goal {
            // A stale goal pop is still optimal under a consistent heuristic — return g_popped.
            // Reconstruct start..=goal.
            let mut path = vec![cell];
            let mut node = (cell, parity);
            while let Some(&prev) = came_from.get(&node) {
                path.push(prev.0);
                node = prev;
            }
            path.reverse();
            return Ok((path, g_popped, parity));
        }
        // Lazy-deletion stale-pop skip: when a node is relaxed to a lower g, the old heap entry
        // stays. Compare the popped g against the current best; skip without burning an expansion
        // slot if stale. INVARIANT: placed AFTER the goal check (stale goal pops are still optimal).
        let best = *g_score.get(&(cell, parity)).unwrap_or(&f64::INFINITY);
        if g_popped > best + 1e-12 {
            continue;
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
            let tentative = g_popped + sc;
            let key = (next, next_parity);
            if tentative < *g_score.get(&key).unwrap_or(&f64::INFINITY) {
                came_from.insert(key, (cell, parity));
                g_score.insert(key, tentative);
                open.push(QNode {
                    f: tentative + heuristic(grid.rule, next, goal),
                    g: tentative,
                    cell: next,
                    parity: next_parity,
                });
            }
        }
    }
    Err(PathFail::Unreachable)
}

/// Max ordered waypoints (incl. goal) per request (DoS guard).
pub(crate) const MAX_WAYPOINTS: usize = 32;
/// Max footprint radius in cells (DoS guard on the per-cell footprint scan).
pub(crate) const MAX_FOOTPRINT_CELLS: f64 = 64.0;
/// Search-window margin (cells) added around the point/wall AABB so detours around walls stay reachable.
const WINDOW_MARGIN: i32 = 8;

fn to_cell(p: vision::P, cell: f64) -> Cell {
    ((p.0 / cell).floor() as i32, (p.1 / cell).floor() as i32)
}

/// Plan a footprint-clear, mask-bounded route `start -> waypoints[0] -> ... -> waypoints[last]`.
/// `waypoints` is the full ordered leg list whose last element is the goal (empty => `Invalid`).
/// Returns cell-center scene points (incl. start and goal) and the total cost in cells.
pub fn find(
    start: vision::P,
    waypoints: &[vision::P],
    footprint_radius: f64,
    cell: f64,
    rule: DiagonalRule,
    walls: &[vision::Seg],
    mask: Option<&BTreeSet<Cell>>,
) -> Result<(Vec<vision::P>, f64), PathFail> {
    // Validation (fail-closed): all degenerate inputs => Invalid.
    if waypoints.is_empty() || waypoints.len() > MAX_WAYPOINTS {
        return Err(PathFail::Invalid);
    }
    // `contains` rejects NaN and ±Inf (NaN comparisons return false; Inf > MAX_FOOTPRINT_CELLS).
    if !(0.0..=MAX_FOOTPRINT_CELLS).contains(&footprint_radius) {
        return Err(PathFail::Invalid);
    }
    // INVARIANT: cell.is_finite() && cell > 0.0 makes the NaN-cell division path unreachable downstream.
    if !cell.is_finite() || cell <= 0.0 {
        return Err(PathFail::Invalid);
    }
    let finite = |p: &vision::P| p.0.is_finite() && p.1.is_finite();
    if !finite(&start) || !waypoints.iter().all(finite) {
        return Err(PathFail::Invalid);
    }

    // Search window: AABB of {start, waypoints, wall endpoints} in cells, expanded by WINDOW_MARGIN
    // so detour paths around walls near the boundary remain reachable.
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let mut acc = |x: f64, y: f64| {
        minx = minx.min(x);
        miny = miny.min(y);
        maxx = maxx.max(x);
        maxy = maxy.max(y);
    };
    acc(start.0, start.1);
    for p in waypoints {
        acc(p.0, p.1);
    }
    for w in walls {
        acc(w.a.0, w.a.1);
        acc(w.b.0, w.b.1);
    }
    let window = (
        (minx / cell).floor() as i32 - WINDOW_MARGIN,
        (miny / cell).floor() as i32 - WINDOW_MARGIN,
        (maxx / cell).floor() as i32 + WINDOW_MARGIN,
        (maxy / cell).floor() as i32 + WINDOW_MARGIN,
    );

    let grid = PathGrid {
        cell,
        rule,
        footprint_radius_cells: footprint_radius,
        walls,
        mask,
        window,
    };

    // Run each leg, threading end-parity into the next leg's start_parity so the route is priced as
    // one continuous move. Resetting parity per leg would underprice 5-10-5 at waypoint boundaries.
    // BOUND: for Alternating, threading each leg's (tie-broken) min-cost-path end-parity into the
    // next leg is per-leg-greedy and NOT guaranteed to minimize TOTAL multi-leg cost — a costlier
    // end-parity on one leg could enable a cheaper next leg. This affects 5-10-5 cost display at
    // waypoint boundaries only; the route remains valid, footprint-clear, mask-bounded, and
    // gate-passable, and spec §4.2 requires only that parity carry (no reset), which it does.
    let mut cells: Vec<Cell> = Vec::new();
    let mut total = 0.0;
    let mut parity = 0u8;
    let mut from = to_cell(start, cell);
    for wp in waypoints {
        let goal = to_cell(*wp, cell);
        let (leg, cost, end_parity) = astar_leg(&grid, from, goal, parity)?;
        total += cost;
        parity = end_parity;
        if cells.is_empty() {
            cells.extend(leg);
        } else {
            // Skip the first cell of subsequent legs — it equals the last cell of the previous leg.
            cells.extend(leg.into_iter().skip(1));
        }
        from = goal;
    }

    let path: Vec<vision::P> = cells.into_iter().map(|c| cell_center(c, cell)).collect();
    Ok((path, total))
}

#[cfg(test)]
mod find_tests {
    use super::*;
    use crate::scene::vision::Seg;

    const NO_WALLS: [Seg; 0] = [];

    #[test]
    fn empty_waypoints_is_invalid() {
        let r = find(
            (50.0, 50.0),
            &[],
            0.1,
            100.0,
            DiagonalRule::Chebyshev,
            &NO_WALLS,
            None,
        );
        assert_eq!(r, Err(PathFail::Invalid));
    }

    #[test]
    fn nonfinite_or_bad_footprint_is_invalid() {
        // Non-finite start point.
        assert_eq!(
            find(
                (f64::NAN, 0.0),
                &[(150.0, 50.0)],
                0.1,
                100.0,
                DiagonalRule::Chebyshev,
                &NO_WALLS,
                None
            ),
            Err(PathFail::Invalid)
        );
        // Negative footprint_radius.
        assert_eq!(
            find(
                (50.0, 50.0),
                &[(150.0, 50.0)],
                -1.0,
                100.0,
                DiagonalRule::Chebyshev,
                &NO_WALLS,
                None
            ),
            Err(PathFail::Invalid)
        );
        // Non-positive cell size.
        assert_eq!(
            find(
                (50.0, 50.0),
                &[(150.0, 50.0)],
                0.1,
                0.0,
                DiagonalRule::Chebyshev,
                &NO_WALLS,
                None
            ),
            Err(PathFail::Invalid)
        );
        // NaN footprint_radius — contains() returns false for NaN comparisons.
        assert_eq!(
            find(
                (50.0, 50.0),
                &[(150.0, 50.0)],
                f64::NAN,
                100.0,
                DiagonalRule::Chebyshev,
                &NO_WALLS,
                None
            ),
            Err(PathFail::Invalid)
        );
        // Infinite footprint_radius — exceeds MAX_FOOTPRINT_CELLS.
        assert_eq!(
            find(
                (50.0, 50.0),
                &[(150.0, 50.0)],
                f64::INFINITY,
                100.0,
                DiagonalRule::Chebyshev,
                &NO_WALLS,
                None
            ),
            Err(PathFail::Invalid)
        );
        // footprint_radius exactly one above the cap.
        assert_eq!(
            find(
                (50.0, 50.0),
                &[(150.0, 50.0)],
                MAX_FOOTPRINT_CELLS + 1.0,
                100.0,
                DiagonalRule::Chebyshev,
                &NO_WALLS,
                None
            ),
            Err(PathFail::Invalid)
        );
    }

    #[test]
    fn straight_route_returns_cell_centers_and_cost() {
        // (50,50)->(250,50): cells (0,0)->(2,0), 2 chebyshev steps. Points = centers of (0,0),(1,0),(2,0).
        let (path, cost) = find(
            (50.0, 50.0),
            &[(250.0, 50.0)],
            0.1,
            100.0,
            DiagonalRule::Chebyshev,
            &NO_WALLS,
            None,
        )
        .unwrap();
        assert!((cost - 2.0).abs() < 1e-9);
        assert_eq!(path.first(), Some(&(50.0, 50.0)));
        assert_eq!(path.last(), Some(&(250.0, 50.0)));
        assert_eq!(path.len(), 3);
    }

    #[test]
    fn waypoint_legs_sum_cost_and_carry_alternating_parity() {
        // Leg A: (0,0)->(1,1) one diagonal (alternating cost 1, end parity 1).
        // Leg B: (1,1)->(2,2) one diagonal from parity 1 (cost 2). Total 3, not 1+1.
        let start = (50.0, 50.0);
        let wp = (150.0, 150.0);
        let goal = (250.0, 250.0);
        let (_p, cost) = find(
            start,
            &[wp, goal],
            0.1,
            100.0,
            DiagonalRule::Alternating,
            &NO_WALLS,
            None,
        )
        .unwrap();
        assert!(
            (cost - 3.0).abs() < 1e-9,
            "parity carries across the waypoint (1 + 2)"
        );
    }

    #[test]
    fn too_many_waypoints_is_invalid() {
        let wps: Vec<vision::P> = (0..(MAX_WAYPOINTS + 1))
            .map(|i| (i as f64 * 100.0 + 50.0, 50.0))
            .collect();
        assert_eq!(
            find(
                (50.0, 50.0),
                &wps,
                0.1,
                100.0,
                DiagonalRule::Chebyshev,
                &NO_WALLS,
                None
            ),
            Err(PathFail::Invalid)
        );
    }

    #[test]
    fn empty_mask_makes_a_nongm_route_unreachable() {
        let mask = BTreeSet::new();
        assert_eq!(
            find(
                (50.0, 50.0),
                &[(250.0, 50.0)],
                0.1,
                100.0,
                DiagonalRule::Chebyshev,
                &NO_WALLS,
                Some(&mask)
            ),
            Err(PathFail::Unreachable)
        );
    }
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
