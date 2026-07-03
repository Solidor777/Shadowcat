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

use crate::scene::movement;
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
    pub regions: Option<&'a crate::scene::regions::RegionField>,
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

/// Whether a token may step from `from` into `to`. INVARIANT (spec §4.3, M3 spec §3): full
/// geometric footprint clearance — (1) the footprint disc at `to` clears every `blocksMove` wall,
/// (2) every footprint-overlapped cell AND every cell the center-to-center step's supercover
/// crosses (including diagonal corner-flankers) is in the mask (non-GM), (3) the center step
/// `from→to` crosses no wall, (4) no footprint-overlapped cell is `impassable` in the region field
/// (M10g; arrest/terrain apply elsewhere — see the region-gate comment inline below).
pub(crate) fn cell_enterable(grid: &PathGrid, from: Cell, to: Cell) -> bool {
    let (i0, j0, i1, j1) = grid.window;
    if to.0 < i0 || to.0 > i1 || to.1 < j0 || to.1 > j1 {
        return false;
    }
    let r_scene = grid.footprint_radius_cells.max(0.0) * grid.cell;
    let ctr = cell_center(to, grid.cell);
    let a = cell_center(from, grid.cell);

    // (1) Footprint disc vs every blocksMove wall.
    for w in grid.walls {
        if point_segment_distance(ctr, w.a, w.b) < r_scene {
            return false;
        }
    }
    // (2) Mask: every footprint-overlapped cell, AND every cell the center-to-center step's
    // supercover crosses, must be visible/revealed (non-GM).
    //
    // INVARIANT (spec §13 / M3 design §3): `movement::supercover_cells` is the SAME primitive
    // the M1 move executor (`move_exec.rs`) and the M10e-4 `ws/room.rs::publish` gate check per
    // step. The router's mask predicate must be a superset of the gate's, or a route this A*
    // search approves can be rejected at execution time (buddy-check P1: for a sub-0.5-cell
    // footprint, the destination footprint disc alone never reaches a diagonal step's corner
    // flanker cells). `None` (degenerate/over-cap span) fails closed: not enterable, mirroring
    // the gate's `None ⇒ Forbidden`.
    if let Some(mask) = grid.mask {
        for c in footprint_cells(to, ctr, r_scene, grid.cell) {
            if !mask.contains(&c) {
                return false;
            }
        }
        match movement::supercover_cells(a, ctr, grid.cell) {
            Some(step_cells) => {
                if !step_cells.iter().all(|c| mask.contains(c)) {
                    return false;
                }
            }
            None => return false,
        }
    }
    // (3) Center-to-center step clears every wall (reuses the M9 segment-cross predicate).
    for w in grid.walls {
        if crate::scene::segments_cross(a, ctr, w.a, w.b) {
            return false;
        }
    }
    // (4) Region gate: impassable footprint-overlap blocks entry, mirroring the wall-clearance
    // rule (§4 membership: any footprint-overlapped cell counts, not just the destination
    // center — a wide-bodied token can't fit past impassable terrain any more than a wall).
    // Arrest/terrain are NOT footprint-gated here: they represent effects on the mover's own
    // position rather than solid geometry it must clear, so they are evaluated once each, in
    // `find()`'s route assembly (arrest truncation) and in `astar_leg`'s step cost (terrain),
    // both keyed on cell-center only — mirroring `move_exec.rs`'s existing center-cell model.
    if let Some(regions) = grid.regions {
        for c in footprint_cells(to, ctr, r_scene, grid.cell) {
            if regions.is_impassable(c) {
                return false;
            }
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
            regions: None,
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
            regions: None,
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

/// ADMISSIBILITY WITH TERRAIN (M10g): `astar_leg`'s actual step cost is `step_cost(...) *
/// terrain_multiplier(next)`, and every multiplier is >= 1.0 (validated where `RegionField` is
/// built, `SceneEcs::region_field`/spec §3). This heuristic already lower-bounds the UNWEIGHTED
/// step cost, so it remains a valid (never-overestimating) lower bound on the weighted cost too —
/// terrain can only make the real path more expensive, never cheaper than this heuristic assumes.
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
            let mult = grid.regions.map_or(1.0, |r| r.terrain_multiplier(next));
            let tentative = g_popped + sc * mult;
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

/// The result of a `find()` route: cell-center scene points (incl. start + goal, or truncated at
/// an arrest cell), the total weighted cost in cells, and whether an arrest region cut the route
/// short (spec §5: "arrest is honest in preview" — the player-facing router must never show a
/// route past a hazard it knows about).
#[derive(Debug, Clone, PartialEq)]
pub struct PathOutcome {
    pub path: Vec<vision::P>,
    pub cost: f64,
    pub arrested: bool,
}

/// Plan a footprint-clear, mask-bounded route `start -> waypoints[0] -> ... -> waypoints[last]`.
/// `waypoints` is the full ordered leg list whose last element is the goal (empty => `Invalid`).
/// Returns cell-center scene points (incl. start and goal) and the total cost in cells.
#[allow(clippy::too_many_arguments)]
pub fn find(
    start: vision::P,
    waypoints: &[vision::P],
    footprint_radius: f64,
    cell: f64,
    rule: DiagonalRule,
    walls: &[vision::Seg],
    mask: Option<&BTreeSet<Cell>>,
    regions: Option<&crate::scene::regions::RegionField>,
) -> Result<PathOutcome, PathFail> {
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
        regions,
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

    // Arrest truncation (spec §5): the route is cut at the FIRST visible arrest cell after the
    // start (a token already standing in a cell is not "entering" it). Recompute the truncated
    // cost by replaying `step_cost` across the surviving prefix — parity threading is purely
    // sequential (order-dependent, not leg-boundary-dependent), so replaying from parity 0 over
    // the assembled `cells` reproduces exactly the cost the original per-leg accumulation gives
    // for that same prefix.
    let mut arrested = false;
    if let Some(rf) = regions {
        if let Some(idx) = cells
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, c)| rf.is_arrest(**c))
            .map(|(i, _)| i)
        {
            cells.truncate(idx + 1);
            arrested = true;
            let mut p = 0u8;
            total = 0.0;
            for w in cells.windows(2) {
                let (di, dj) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
                let (sc, next_p) = step_cost(rule, di, dj, p);
                total += sc * rf.terrain_multiplier(w[1]);
                p = next_p;
            }
        }
    }

    let path: Vec<vision::P> = cells.into_iter().map(|c| cell_center(c, cell)).collect();
    Ok(PathOutcome {
        path,
        cost: total,
        arrested,
    })
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
                None,
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
                None,
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
                None,
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
                None,
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
                None,
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
                None,
                None
            ),
            Err(PathFail::Invalid)
        );
    }

    #[test]
    fn straight_route_returns_cell_centers_and_cost() {
        // (50,50)->(250,50): cells (0,0)->(2,0), 2 chebyshev steps. Points = centers of (0,0),(1,0),(2,0).
        let outcome = find(
            (50.0, 50.0),
            &[(250.0, 50.0)],
            0.1,
            100.0,
            DiagonalRule::Chebyshev,
            &NO_WALLS,
            None,
            None,
        )
        .unwrap();
        assert!((outcome.cost - 2.0).abs() < 1e-9);
        assert_eq!(outcome.path.first(), Some(&(50.0, 50.0)));
        assert_eq!(outcome.path.last(), Some(&(250.0, 50.0)));
        assert_eq!(outcome.path.len(), 3);
    }

    #[test]
    fn waypoint_legs_sum_cost_and_carry_alternating_parity() {
        // Leg A: (0,0)->(1,1) one diagonal (alternating cost 1, end parity 1).
        // Leg B: (1,1)->(2,2) one diagonal from parity 1 (cost 2). Total 3, not 1+1.
        let start = (50.0, 50.0);
        let wp = (150.0, 150.0);
        let goal = (250.0, 250.0);
        let outcome = find(
            start,
            &[wp, goal],
            0.1,
            100.0,
            DiagonalRule::Alternating,
            &NO_WALLS,
            None,
            None,
        )
        .unwrap();
        assert!(
            (outcome.cost - 3.0).abs() < 1e-9,
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
                None,
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
                Some(&mask),
                None
            ),
            Err(PathFail::Unreachable)
        );
    }

    #[test]
    fn arrest_region_truncates_the_route_and_flags_arrested() {
        use crate::scene::regions::{RegionBehavior, RegionField, RegionShape};
        let mut b = RegionField::builder();
        // Arrest region covering the ENTIRE i=2 column (x in [200,300), a wide y band). Deviation
        // from a narrower single-cell rect: under Chebyshev, orthogonal and diagonal steps cost
        // the same, so (0,0)->(4,0) has several tied-cost 4-step routes and this build's A*
        // tie-break (QNode ties favor the lexicographically LARGER (cell, parity) key) resolves to
        // a diagonal detour through (2,2), not the straight row — the same tie-break ambiguity the
        // pre-existing `terrain_region_raises_astar_step_cost` test's comment documents. Covering
        // the whole column (rather than only the straight-line cell) makes the test robust to
        // which tied route is chosen while still proving arrest fires exactly when the route
        // FIRST reaches x in [200,300), at cost 2.0 (2 steps in).
        b.add(
            &RegionShape::Rect {
                x0: 200.0,
                y0: -300.0,
                x1: 300.0,
                y1: 300.0,
            },
            RegionBehavior::Arrest,
            1.0,
            100.0,
        );
        let field = b.build();
        let outcome = find(
            (50.0, 50.0),
            &[(450.0, 50.0)],
            0.1,
            100.0,
            DiagonalRule::Chebyshev,
            &NO_WALLS,
            None,
            Some(&field),
        )
        .unwrap();
        assert!(outcome.arrested);
        assert_eq!(
            outcome.path.last(),
            Some(&(250.0, 250.0)),
            "route truncates AT the arrest cell (2,2), the tie-broken A* route's column-2 cell"
        );
        assert!(
            (outcome.cost - 2.0).abs() < 1e-9,
            "2 steps to reach the arrest column"
        );
    }

    #[test]
    fn no_regions_argument_is_backward_compatible() {
        let outcome = find(
            (50.0, 50.0),
            &[(250.0, 50.0)],
            0.1,
            100.0,
            DiagonalRule::Chebyshev,
            &NO_WALLS,
            None,
            None,
        )
        .unwrap();
        assert!(!outcome.arrested);
        assert!((outcome.cost - 2.0).abs() < 1e-9);
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
            regions: None,
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

        // A point-sized footprint overlaps only (1,0) at the destination — but the M3 fix also
        // requires the FROM cell in the mask (supercover_cells always includes both endpoint
        // cells). Add (1,1) to represent a realistic case where both the mover's current cell
        // and its destination are visible.
        let mut mask_from_and_to = mask.clone();
        mask_from_and_to.insert((1, 1));
        let gp = grid(&walls, Some(&mask_from_and_to), 0.0);
        assert!(cell_enterable(&gp, (1, 1), (1, 0)));
    }

    #[test]
    fn diagonal_step_missing_flanker_cell_is_not_enterable_small_footprint() {
        // Buddy-check P1 regression. A perfectly diagonal step (0,0)->(1,1) at cell=100 crosses
        // the shared corner exactly, so supercover_cells emits BOTH flanker cells (1,0) and (0,1)
        // in addition to the two endpoint cells. A small (point-sized) footprint disc at the
        // destination (1,1) only overlaps (1,1) itself — footprint_cells alone would not catch a
        // missing flanker. The mask below has every cell EXCEPT the (0,1) flanker: the step must
        // be rejected once the router's mask check includes the step's supercover, even though
        // the footprint-disc-only check (pre-fix behavior) would have passed it.
        let walls: Vec<Seg> = vec![];
        let mut mask = BTreeSet::new();
        mask.insert((0, 0));
        mask.insert((1, 0));
        mask.insert((1, 1));
        // (0, 1) deliberately absent — the missing flanker.
        let g = grid(&walls, Some(&mask), 0.1);
        assert!(
            !cell_enterable(&g, (0, 0), (1, 1)),
            "diagonal step must be rejected when a supercover flanker cell is outside the mask, \
             even though the footprint disc at the destination doesn't reach that flanker"
        );
    }

    #[test]
    fn degenerate_step_supercover_is_not_enterable_even_if_mask_covers_destination() {
        // A step spanning an enormous cell distance makes `supercover_cells` return `None` (the
        // MAX_MOVE_CELLS span guard in movement.rs). The router must fail closed on `None`, same
        // as move_exec.rs and ws/room.rs::publish, regardless of what the mask contains at `to`.
        let walls: Vec<Seg> = vec![];
        let mut mask = BTreeSet::new();
        mask.insert((5000, 5000)); // covers the destination; would pass footprint_cells alone.
        let mut g = grid(&walls, Some(&mask), 0.0);
        g.window = (-10_000, -10_000, 10_000, 10_000);
        assert!(
            !cell_enterable(&g, (0, 0), (5000, 5000)),
            "an over-cap/degenerate step supercover must fail closed, not fall back to the \
             footprint-only mask check"
        );
    }

    fn visible_grid(range: i32) -> BTreeSet<Cell> {
        (0..range)
            .flat_map(|i| (0..range).map(move |j| (i, j)))
            .collect()
    }

    #[test]
    fn large_footprint_diagonal_step_with_flankers_in_mask_is_still_enterable() {
        // A large footprint (1.0 cell radius) at the destination of a diagonal step already
        // overlaps both corner-flanker cells — the footprint_cells check alone would pass this.
        // Prove the ADDED step-supercover union doesn't introduce a new false rejection when the
        // mask already covers everything the footprint disc covers.
        let walls: Vec<Seg> = vec![];
        let mask = visible_grid(6); // covers (0,0)..(5,5); large enough for both footprint + step.
        let g = grid(&walls, Some(&mask), 1.0);
        assert!(
            cell_enterable(&g, (2, 2), (3, 3)),
            "large footprint with a fully-visible mask must remain enterable after the union fix"
        );
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

    #[test]
    fn impassable_region_blocks_entry_like_a_wall() {
        use crate::scene::regions::{RegionBehavior, RegionField, RegionShape};
        let mut b = RegionField::builder();
        b.add(
            &RegionShape::Rect {
                x0: 100.0,
                y0: 0.0,
                x1: 200.0,
                y1: 100.0,
            },
            RegionBehavior::Impassable,
            1.0,
            100.0,
        );
        let field = b.build();
        let walls: Vec<Seg> = vec![];
        let mut g = grid(&walls, None, 0.2);
        g.regions = Some(&field);
        assert!(
            !cell_enterable(&g, (0, 0), (1, 0)),
            "cell (1,0) center (150,50) is inside the impassable rect"
        );
    }

    #[test]
    fn terrain_region_raises_astar_step_cost() {
        use crate::scene::regions::{RegionBehavior, RegionField, RegionShape};
        let mut b = RegionField::builder();
        // Terrain cost 3 covering every cell reachable in a single king-move step from BOTH the
        // start and the goal — i.e. every possible intermediate cell of a 2-step (0,0)->(2,0)
        // route, not merely the direct orthogonal row. Deviation from a narrower single-row rect:
        // under Chebyshev a narrower rect leaves a same-length diagonal detour (via (1,1) or
        // (1,-1)) that pays the multiplier only once, at cost 4.0 not 6.0 — that isn't a routing
        // bug, it's terrain-weighting correctly picking the cheaper of two equal-length routes.
        // Covering all three king-move-adjacent intermediates (1,-1)/(1,0)/(1,1) removes that
        // escape so every legal 2-step route pays the multiplier twice, proving the weighting is
        // applied per entered cell regardless of which route is chosen.
        b.add(
            &RegionShape::Rect {
                x0: 0.0,
                y0: -100.0,
                x1: 300.0,
                y1: 200.0,
            },
            RegionBehavior::Terrain,
            3.0,
            100.0,
        );
        let field = b.build();
        let walls: Vec<Seg> = vec![];
        let mut g = grid(&walls, None, 0.1);
        g.regions = Some(&field);
        let (_cells, cost, _p) = astar_leg(&g, (0, 0), (2, 0), 0).unwrap();
        assert!(
            (cost - 6.0).abs() < 1e-9,
            "2 steps through terrain at multiplier 3 = 6.0, regardless of which of the three \
             king-move-adjacent intermediate cells the route passes through"
        );
    }
}
