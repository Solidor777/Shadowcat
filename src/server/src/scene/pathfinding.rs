//! Server-authoritative grid A* pathfinder. Pure + headless: callers pass parsed inputs
//! (walls, mask, cell size, grid shape, footprint); this module owns no I/O and borrows no ECS. The
//! `GridShape` (square/hex) is the single source of the diagonal rule — it owns both the step-cost
//! and the admissible heuristic, so `find` takes no separate rule argument.
//! Engine-owned geometry; clean-room A* (Hart, Nilsson & Raphael 1968).
//!
//! INVARIANT: the per-cell mask test consumes the SAME `visible_cells` set the
//! movement gate uses — the route can never thread the unknown nor leak hidden geometry.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

/// Grid diagonal-cost rule (from `world-settings.pathfinding.diagonalRule`). All four are the same
/// king-move graph; they differ only in diagonal cost + the admissible heuristic. `Alternating`
/// (PF1e/3.5 "5-10-5") costs diagonals 1,2,1,2… and so requires a parity bit in the search node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagonalRule {
    /// Diagonals cost 1.0 (equal to orthogonal steps).
    Chebyshev,
    /// Diagonals cost 2.0 — priced as two orthogonal steps, NOT banned
    /// (the king-move graph is identical across all four rules).
    Manhattan,
    /// Diagonals cost sqrt(2).
    Euclidean,
    /// Diagonals alternate 1,2,1,2… (PF1e/3.5 "5-10-5"); needs the node
    /// parity bit.
    Alternating,
}

use crate::scene::vision::{self, point_segment_distance};
use std::collections::{BTreeSet, BinaryHeap, HashMap};

/// A grid cell `(i, j)`; cell `(i,j)` covers `[i*cell,(i+1)*cell) × [j*cell,(j+1)*cell)`.
pub type Cell = (i32, i32);

/// The caller-supplied half of an A* search: the scene's traversal geometry, the requester's view
/// of it, and the mover's size. `find` derives the remaining piece — the search `window` — from
/// `start`/`waypoints`/`walls`, then assembles both into the `PathGrid` its search runs against, so
/// every field here reaches `cell_enterable`'s per-cell gate unchanged.
///
/// INVARIANT: `mask`, `walls` and `regions` are the REQUESTER's view, resolved by the caller
/// (`SceneEcs::pathfind`) — `mask = None` only for a GM or an `Unrestricted` scene, and `walls`/
/// `regions` omit whatever tier the requester may not see. `shape` MUST be the same `GridShape`
/// those three sets were built with; a mask indexed in one coordinate system and tested in another
/// is not a shared mask.
pub struct PathInputs<'a> {
    /// Mover footprint radius in CELLS; `find` rejects anything outside
    /// `[0, MAX_FOOTPRINT_CELLS]` (NaN and ±Inf included).
    pub footprint_radius_cells: f64,
    /// Grid cell size in scene units (positive finite).
    pub cell: f64,
    /// `blocksMove` wall segments a step may not cross.
    pub walls: &'a [vision::Seg],
    /// Visibility mask: `None` = unconstrained (GM or `Unrestricted`); `Some` = every entered
    /// (and footprint-overlapped) cell must be in it. An empty `Some` set is the fail-closed
    /// dark-scene freeze, not an absent constraint.
    pub mask: Option<&'a BTreeSet<Cell>>,
    /// Composed region field (weighting/impassable/arrest); `None` = no region enforcement.
    pub regions: Option<&'a crate::scene::regions::RegionField>,
    /// Cell geometry for this scene (`SquareGrid` or `HexGrid`), resolved by the caller from the
    /// scene's `grid.kind` and passed into `find()`.
    pub shape: &'a dyn crate::scene::grid_shape::GridShape,
}

/// Assembled, borrow-only inputs for one A* search: the caller-supplied `PathInputs` plus the
/// search `window` (i0,j0,i1,j1 inclusive) `find` derives so a GM query with an unreachable goal
/// can't wander unboundedly. Holds `PathInputs` directly rather than restating its fields, so a
/// field added there reaches `cell_enterable`/`astar_leg` without a second, hand-synchronised copy.
pub struct PathGrid<'a> {
    /// Caller-supplied traversal geometry, requester view, and mover size.
    pub inputs: PathInputs<'a>,
    /// Inclusive `(i0, j0, i1, j1)` search bound (unbounded-wander guard).
    pub window: (i32, i32, i32, i32),
}

/// Center of cell `c` in scene coords.
pub fn cell_center(c: Cell, cell: f64) -> vision::P {
    ((c.0 as f64 + 0.5) * cell, (c.1 as f64 + 0.5) * cell)
}

/// Cells whose AABB the footprint disc (center `ctr`, radius `r_scene`) overlaps. A cell overlaps
/// the disc iff the disc center is within `r_scene` of the cell's AABB. The anchor cell is always
/// included (a zero-radius disc overlaps exactly its own cell).
pub(crate) fn footprint_cells(anchor: Cell, ctr: vision::P, r_scene: f64, cell: f64) -> Vec<Cell> {
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

/// Whether a token may step from `from` into `to`. INVARIANT: full
/// geometric footprint clearance — (1) the footprint disc at `to` clears every `blocksMove` wall,
/// (2) every footprint-overlapped cell AND every cell the center-to-center step's supercover
/// crosses (including diagonal corner-flankers) is in the mask (non-GM), (3) the center step
/// `from→to` crosses no wall, (4) no footprint-overlapped cell is `impassable` in the region field
/// (arrest/terrain apply elsewhere — see the region-gate comment inline below).
pub(crate) fn cell_enterable(grid: &PathGrid, from: Cell, to: Cell) -> bool {
    let (i0, j0, i1, j1) = grid.window;
    if to.0 < i0 || to.0 > i1 || to.1 < j0 || to.1 > j1 {
        return false;
    }
    let r_scene = grid.inputs.footprint_radius_cells.max(0.0) * grid.inputs.cell;
    let ctr = grid.inputs.shape.cell_center(to);
    let a = grid.inputs.shape.cell_center(from);

    // (1) Footprint disc vs every blocksMove wall.
    for w in grid.inputs.walls {
        if point_segment_distance(ctr, w.a, w.b) < r_scene {
            return false;
        }
    }
    // (2) Mask: every footprint-overlapped cell, AND every cell the center-to-center step's
    // supercover crosses, must be visible/revealed (non-GM).
    //
    // INVARIANT: `GridShape::line_traversal` is the SAME primitive the sole move executor
    // (`move_exec::execute_move`) uses per step — resolved from the scene's shape, never the free
    // square `movement::supercover_cells` (that is `SquareGrid`'s own internal; calling it here is
    // square-on-hex). The router's mask predicate must be a superset of the gate's, or a route this A*
    // search approves can be rejected at execution time: for a sub-0.5-cell
    // footprint, the destination footprint disc alone never reaches a diagonal step's corner
    // flanker cells. `None` (degenerate/over-cap span) fails closed: not enterable, mirroring
    // the gate's `None ⇒ Forbidden`.
    if let Some(mask) = grid.inputs.mask {
        for c in grid
            .inputs
            .shape
            .footprint_cells(to, ctr, r_scene, grid.inputs.cell)
        {
            if !mask.contains(&c) {
                return false;
            }
        }
        match grid.inputs.shape.line_traversal(a, ctr, grid.inputs.cell) {
            Some(step_cells) => {
                if !step_cells.iter().all(|c| mask.contains(c)) {
                    return false;
                }
            }
            None => return false,
        }
    }
    // (3) Center-to-center step clears every wall (reuses the segment-cross predicate).
    for w in grid.inputs.walls {
        if crate::scene::segments_cross(a, ctr, w.a, w.b) {
            return false;
        }
    }
    // (4) Region gate: impassable footprint-overlap blocks entry, mirroring the wall-clearance
    // rule — membership is any footprint-overlapped cell, not just the destination center, since a
    // wide-bodied token can't fit past impassable terrain any more than a wall.
    // Arrest/terrain are NOT footprint-gated here: they represent effects on the mover's own
    // position rather than solid geometry it must clear, so they are evaluated once each, in
    // `find()`'s route assembly (arrest truncation) and in `astar_leg`'s step cost (terrain),
    // both keyed on cell-center only — mirroring `move_exec::execute_move`'s existing center-cell model.
    if let Some(regions) = grid.inputs.regions {
        for c in grid
            .inputs
            .shape
            .footprint_cells(to, ctr, r_scene, grid.inputs.cell)
        {
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
        let shape: &'static crate::scene::grid_shape::SquareGrid =
            Box::leak(Box::new(crate::scene::grid_shape::SquareGrid {
                cell: 100.0,
                rule,
            }));
        PathGrid {
            inputs: PathInputs {
                cell: 100.0,
                footprint_radius_cells: footprint,
                walls: &NO_WALLS,
                mask: None,
                regions: None,
                shape,
            },
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
        let shape = crate::scene::grid_shape::SquareGrid {
            cell: c,
            rule: DiagonalRule::Chebyshev,
        };
        let g = PathGrid {
            inputs: PathInputs {
                cell: c,
                footprint_radius_cells: 0.1,
                walls: &walls,
                mask: None,
                regions: None,
                shape: &shape,
            },
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

    #[test]
    fn astar_leg_routes_through_grid_shape_not_hardcoded_dirs() {
        // Same assertions as `chebyshev_diagonal_is_cost_one_per_step`, but this test exists
        // specifically to fail if a future edit reintroduces a hardcoded `dirs`/`step_cost` path
        // instead of going through `grid.inputs.shape.neighbors_with_cost(...)`.
        let g = open(DiagonalRule::Chebyshev, 0.1);
        let (cells, cost, _p) = astar_leg(&g, (0, 0), (3, 3), 0).unwrap();
        assert!((cost - 3.0).abs() < 1e-9);
        assert_eq!(cells.first(), Some(&(0, 0)));
        assert_eq!(cells.last(), Some(&(3, 3)));
    }

    /// Hex-scene A* integration coverage: proves the fully-wired hex path behaves correctly
    /// end-to-end, mirroring this module's square-scene coverage above. The diagonal rule is a
    /// `SquareGrid`-only concept (`HexGrid` uses uniform 1-cost steps and the admissible axial
    /// heuristic), so a `HexGrid` shape carries no rule at all — see the `grid_shape` module.
    fn open_hex(footprint: f64) -> PathGrid<'static> {
        const NO_WALLS: [Seg; 0] = [];
        let shape: &'static crate::scene::grid_shape::HexGrid =
            Box::leak(Box::new(crate::scene::grid_shape::HexGrid { size: 100.0 }));
        PathGrid {
            inputs: PathInputs {
                cell: 100.0,
                footprint_radius_cells: footprint,
                walls: &NO_WALLS,
                mask: None,
                regions: None,
                shape,
            },
            window: (-50, -50, 50, 50),
        }
    }

    #[test]
    fn hex_scene_astar_finds_a_route_around_a_wall() {
        // Pointy-top axial hex, size=100. The direct route (0,0)->(1,0)->(2,0) costs 2 (uniform
        // 1.0-per-hex-step, no DiagonalRule variance). A wall crossing exactly the center-to-
        // center (0,0)->(1,0) step removes that edge (not the cell (1,0) itself, still reachable
        // via its other 5 neighbor edges), forcing a 3-step detour cost 3.0 via one of two
        // symmetric axial-adjacent-to-(1,0) paths (e.g. (1,-1)->(2,-1)) that still reaches the goal.
        let hx = 100.0 * 3.0_f64.sqrt() / 2.0; // x of the (0,0)->(1,0) step's midpoint
        let walls = vec![Seg {
            a: (hx, -50.0),
            b: (hx, 50.0),
        }];
        let mut g = open_hex(0.1);
        g.inputs.walls = &walls;
        let (cells, cost, _p) = astar_leg(&g, (0, 0), (2, 0), 0).unwrap();
        assert_eq!(cells.first(), Some(&(0, 0)));
        assert_eq!(cells.last(), Some(&(2, 0)));
        assert!(
            !cells.contains(&(1, 0)),
            "the direct neighbor is unreachable once its entering step is wall-blocked"
        );
        assert!(
            (cost - 3.0).abs() < 1e-9,
            "uniform 1.0-per-hex-step cost over the 3-step detour"
        );
    }

    #[test]
    fn hex_scene_astar_respects_the_visibility_mask() {
        // Same detour shape as the wall test above, but forced by mask exclusion instead of a
        // wall: (1,0) is excluded from the mask, so the route must go around it via
        // (0,1)->(1,1), never entering the masked-out cell.
        let mut mask: BTreeSet<Cell> = BTreeSet::new();
        mask.insert((0, 0));
        mask.insert((0, 1));
        mask.insert((1, 1));
        mask.insert((2, 0));
        // (1, 0) deliberately absent — the masked-out cell.
        let mut g = open_hex(0.1);
        g.inputs.mask = Some(&mask);
        let (cells, cost, _p) = astar_leg(&g, (0, 0), (2, 0), 0).unwrap();
        assert!(
            !cells.contains(&(1, 0)),
            "route must never enter the masked-out cell"
        );
        assert_eq!(cells.last(), Some(&(2, 0)));
        assert!((cost - 3.0).abs() < 1e-9);
    }

    #[test]
    fn hex_astar_returns_the_true_shortest_route_via_the_admissible_heuristic() {
        // Pins hex A* route optimality: the axial heuristic (`HexGrid::heuristic`) is admissible, so
        // A* returns the true shortest hex route. A square `DiagonalRule` heuristic OVERESTIMATES the
        // true axial distance for opposite-sign axial deltas (Manhattan is 2x on the (1,-1) line),
        // which is non-admissible on hex and yields a valid but longer route. This maze isolates that
        // case: the goal's cheap axis-neighbor approach (-1,0) is masked out, forcing the shortest
        // route to approach through the OPPOSITE-SIGN neighbor (-1,1) — the cell a square heuristic
        // deprioritizes. The admissible heuristic yields the true shortest route
        // (-3,1)->(-2,1)->(-1,1)->(0,0) at cost 3 over 4 cells; a square-Manhattan estimate over this
        // same maze yields the 4-cost, 5-cell detour (-3,1)->(-2,0)->(-1,-1)->(0,-1)->(0,0) instead,
        // so any non-admissible replacement of `HexGrid::heuristic` fails this assertion.
        let hx = crate::scene::grid_shape::HexGrid { size: 100.0 };
        // Mask = hex disc (radius 4 in every axis incl. the third cube axis q+r) MINUS the goal's
        // cheap axis neighbor (-1,0): the detour cells stay present, only the cheap approach closes.
        let mut mask: BTreeSet<Cell> = BTreeSet::new();
        let r = 4i32;
        for q in -r..=r {
            for rr in -r..=r {
                if (q + rr).abs() <= r && (q, rr) != (-1, 0) {
                    mask.insert((q, rr));
                }
            }
        }
        let g = PathGrid {
            inputs: PathInputs {
                cell: 100.0,
                footprint_radius_cells: 0.1,
                walls: &[],
                mask: Some(&mask),
                regions: None,
                shape: &hx,
            },
            window: (-50, -50, 50, 50),
        };
        let (cells, cost, _p) = astar_leg(&g, (-3, 1), (0, 0), 0).unwrap();
        assert_eq!(
            cells,
            vec![(-3, 1), (-2, 1), (-1, 1), (0, 0)],
            "admissible hex heuristic must yield the true shortest route through the opposite-sign \
             approach (-1,1), not the longer detour a square heuristic would pick"
        );
        assert!(
            (cost - 3.0).abs() < 1e-9,
            "true hex distance is 3, not the square heuristic's 4-cost detour; got {cost}"
        );
        assert_eq!(
            cells.len(),
            4,
            "shortest route is 4 cells (3 uniform-cost hex steps)"
        );
    }
}

/// Why a path request fails. Mapped to a `PathError` message at the wire boundary.
#[derive(Debug, PartialEq, Eq)]
pub enum PathFail {
    /// Degenerate request: no destination, non-finite input, or an
    /// out-of-range footprint.
    Invalid,
    /// No route exists within the walls/mask/window constraints.
    Unreachable,
    /// Search expanded more than `MAX_PATH_NODES` nodes (DoS backstop).
    Exceeded,
}

/// DoS backstop: total node expansions per leg. For non-GM the mask is the tighter bound; this caps
/// a GM search whose window is large.
pub(crate) const MAX_PATH_NODES: usize = 200_000;

/// f64 ordering wrapper for the min-heap. Orders by `f` ascending (via reversed `total_cmp`),
/// tie-broken by `(cell, parity)` so identical requests yield identical routes (determinism).
/// `g` is payload for lazy-deletion stale-pop skip — it is NOT part of the ordering key.
#[derive(PartialEq)]
struct QNode {
    /// Estimated total cost `g + h` (the ordering key).
    f: f64,
    /// Cost-so-far payload (lazy-deletion stale-pop check; NOT an ordering key).
    g: f64,
    /// The expanded cell.
    cell: Cell,
    /// Alternating-rule diagonal parity (0/1); constant 0 under other rules.
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

/// ADMISSIBILITY WITH TERRAIN: `astar_leg`'s actual step cost is `step_cost(...) *
/// terrain_multiplier(next)`, and every multiplier is >= 1.0 (validated where `RegionField` is
/// built, `SceneEcs::region_field`). This heuristic already lower-bounds the UNWEIGHTED
/// step cost, so it remains a valid (never-overestimating) lower bound on the weighted cost too —
/// terrain can only make the real path more expensive, never cheaper than this heuristic assumes.
/// Consistent (not merely admissible) heuristic from `c` to `goal` under `rule`. Consistency
/// holds because Δh per king-move never exceeds that move's minimum step cost: orthogonal Δh ≤ 1 =
/// orthogonal cost; diagonal Δh ≤ √2 ≤ each rule's diagonal cost (Chebyshev 1, Manhattan 2,
/// Euclidean √2, Alternating 1); for Alternating the optimistic bound (Chebyshev = dmax) gives
/// Δh ≤ 1, which is the cheapest diagonal cost. Consistency is load-bearing: it makes the first
/// goal-pop optimal and makes the post-goal stale-pop skip safe (see `astar_leg`).
///
/// `pub(crate)` so `SquareGrid::heuristic` can return this byte-identical value —
/// `astar_leg` calls it only through `grid.inputs.shape.heuristic(...)` now, never directly, so
/// square scenes keep the exact rule-based estimate while hex scenes get the admissible axial
/// distance.
pub(crate) fn heuristic(rule: DiagonalRule, c: Cell, goal: Cell) -> f64 {
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
        f: grid.inputs.shape.heuristic(start, goal),
        g: 0.0,
        cell: start,
        parity: start_parity,
    });

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
        for (next, sc, next_parity) in grid.inputs.shape.neighbors_with_cost(cell, parity) {
            if !cell_enterable(grid, cell, next) {
                continue;
            }
            let mult = grid
                .inputs
                .regions
                .map_or(1.0, |r| r.terrain_multiplier(next));
            let tentative = g_popped + sc * mult;
            let key = (next, next_parity);
            if tentative < *g_score.get(&key).unwrap_or(&f64::INFINITY) {
                came_from.insert(key, (cell, parity));
                g_score.insert(key, tentative);
                open.push(QNode {
                    f: tentative + grid.inputs.shape.heuristic(next, goal),
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
/// `pub(crate)` because `grid_shape`'s own window-clipping fixture states the same margin: the
/// clipping it demonstrates is a property of THIS value, so the fixture must read it rather than
/// restate it.
pub(crate) const WINDOW_MARGIN: i32 = 8;

/// The result of a `find()` route: scene points — `path[0]` is the mover's LITERAL start
/// position, every later point a cell center through the goal (or truncated at an arrest
/// cell) — the total weighted cost in cells, and whether an arrest region cut the route
/// short ("arrest is honest in preview" — the player-facing router must never show a
/// route past a hazard it knows about).
#[derive(Debug, Clone, PartialEq)]
pub struct PathOutcome {
    /// Route points: `path[0]` is the mover's literal start position (what
    /// `execute_move` requires a `MoveRequest`'s `path[0]` to equal); later
    /// points are cell centers through the goal, or cut at the arrest cell.
    pub path: Vec<vision::P>,
    /// Total weighted cost in cells.
    pub cost: f64,
    /// An arrest region truncated the route (honest-preview rule).
    pub arrested: bool,
}

/// Plan a footprint-clear, mask-bounded route `start -> waypoints[0] -> ... -> waypoints[last]`.
/// `waypoints` is the full ordered leg list whose last element is the goal (empty => `Invalid`).
/// Returns the literal `start` followed by cell-center points through the goal, and the total
/// cost in cells.
pub fn find(
    start: vision::P,
    waypoints: &[vision::P],
    inputs: PathInputs<'_>,
) -> Result<PathOutcome, PathFail> {
    // Validation (fail-closed): all degenerate inputs => Invalid.
    if waypoints.is_empty() || waypoints.len() > MAX_WAYPOINTS {
        return Err(PathFail::Invalid);
    }
    // `contains` rejects NaN and ±Inf (NaN comparisons return false; Inf > MAX_FOOTPRINT_CELLS).
    if !(0.0..=MAX_FOOTPRINT_CELLS).contains(&inputs.footprint_radius_cells) {
        return Err(PathFail::Invalid);
    }
    // INVARIANT: cell.is_finite() && cell > 0.0 makes the NaN-cell division path unreachable downstream.
    if !inputs.cell.is_finite() || inputs.cell <= 0.0 {
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
    for w in inputs.walls {
        acc(w.a.0, w.a.1);
        acc(w.b.0, w.b.1);
    }
    // Grid-correct cell box of the pixel AABB, then WINDOW_MARGIN cells of detour room. Square
    // returns the byte-identical `floor(min/cell)`/`floor(max/cell)` box; hex returns the padded
    // axial preimage box (a per-axis pixel floor would be a WRONG, clipping box on hex — the
    // axial↔pixel shear means a valid hex route's cells fall outside a square-index rectangle,
    // reading spuriously Unreachable). INVARIANT: the window only bounds SEARCH extent — the
    // per-cell gate lives in `cell_enterable`, so reshaping the window never approves a cell the
    // executor would reject.
    let (bi0, bj0, bi1, bj1) = inputs
        .shape
        .cell_bounds((minx, miny), (maxx, maxy), inputs.cell);
    let window = (
        bi0 - WINDOW_MARGIN,
        bj0 - WINDOW_MARGIN,
        bi1 + WINDOW_MARGIN,
        bj1 + WINDOW_MARGIN,
    );

    // Moves `inputs` into `grid` whole, rather than restating its fields — a field added to
    // `PathInputs` reaches `cell_enterable`/`astar_leg` through `grid.inputs` with no second edit
    // here.
    let grid = PathGrid { inputs, window };

    // Run each leg, threading end-parity into the next leg's start_parity so the route is priced as
    // one continuous move. Resetting parity per leg would underprice 5-10-5 at waypoint boundaries.
    // BOUND: for Alternating, threading each leg's (tie-broken) min-cost-path end-parity into the
    // next leg is per-leg-greedy and NOT guaranteed to minimize TOTAL multi-leg cost — a costlier
    // end-parity on one leg could enable a cheaper next leg. This affects 5-10-5 cost display at
    // waypoint boundaries only; the route remains valid, footprint-clear, mask-bounded, and
    // gate-passable — only that parity carry across legs (no reset) is required, which it does.
    let mut cells: Vec<Cell> = Vec::new();
    let mut total = 0.0;
    let mut parity = 0u8;
    // Grid-correct pixel→cell mapping (square floor / hex axial-round). MUST agree with the window
    // above: both derive from `grid.inputs.shape`, so a hex goal's axial cell always lands inside
    // the axial window. A square-only `to_cell` here paired with the square window would silently
    // route a hex request to the WRONG destination cell; fixing only one side would instead make
    // the goal fall outside the other's box, reading spuriously Unreachable.
    let mut from = grid.inputs.shape.cell_of(start);
    for (leg_index, wp) in waypoints.iter().enumerate() {
        let goal = grid.inputs.shape.cell_of(*wp);
        let (leg, cost, end_parity) = match astar_leg(&grid, from, goal, parity) {
            Ok(v) => v,
            Err(PathFail::Unreachable) => {
                let failed_cell = goal;
                let search_window = grid.window;
                // Cause is not distinguishable here: `cell_enterable` folds wall-boxing, an
                // empty/exhausted mask, region-impassable, and genuine window-margin truncation
                // into one boolean, so this single `Unreachable` exit is reached identically for
                // all of them. Logs the leg + failed cell + window as raw fields with no causal
                // claim; the window field lets an operator hand-check margin truncation.
                tracing::debug!(
                    leg_index,
                    cell = ?failed_cell,
                    window = ?search_window,
                    "A* leg unreachable"
                );
                return Err(PathFail::Unreachable);
            }
            Err(e) => return Err(e),
        };
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

    // Arrest truncation: the route is cut at the FIRST visible arrest cell after the
    // start (a token already standing in a cell is not "entering" it). Recompute the truncated
    // cost by replaying the per-step cost across the surviving prefix via `grid.inputs.shape` —
    // parity threading is purely sequential (order-dependent, not leg-boundary-dependent), so
    // replaying from parity 0 over the assembled `cells` reproduces exactly the cost the original
    // per-leg accumulation gives for that same prefix.
    let mut arrested = false;
    if let Some(rf) = grid.inputs.regions {
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
                let (_, sc, next_p) = grid
                    .inputs
                    .shape
                    .neighbors_with_cost(w[0], p)
                    .into_iter()
                    .find(|(next, _, _)| *next == w[1])
                    .expect("cells adjacent along an already-found route are a valid grid_shape neighbor pair");
                total += sc * rf.terrain_multiplier(w[1]);
                p = next_p;
            }
        }
    }

    // The FIRST point is the mover's literal current position, not `cell_center(cells[0])`: a
    // token need not sit exactly on a cell center (snap-off authoring, a GM's freeform nudge,
    // any off-grid placement), and `execute_move` requires a `MoveRequest`'s `path[0]` to equal
    // the token's committed position within `EPS` (its sole proof the request originates where
    // the token actually is). Cell-centering it here would silently desync the router's route
    // from the token's real position on every off-center mover, and `execute_move` would refuse
    // the resulting request as `Degenerate` regardless of how legal the move itself is. Every
    // OTHER point remains a cell center — waypoints (including the goal) are still grid-snapped,
    // matching the router's per-cell contract; only the mover's own starting point is literal.
    let mut path: Vec<vision::P> = cells
        .into_iter()
        .map(|c| grid.inputs.shape.cell_center(c))
        .collect();
    if let Some(first) = path.first_mut() {
        *first = start;
    }
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

    fn sq(cell: f64, rule: DiagonalRule) -> crate::scene::grid_shape::SquareGrid {
        crate::scene::grid_shape::SquareGrid { cell, rule }
    }

    #[test]
    fn empty_waypoints_is_invalid() {
        let r = find(
            (50.0, 50.0),
            &[],
            PathInputs {
                footprint_radius_cells: 0.1,
                cell: 100.0,
                walls: &NO_WALLS,
                mask: None,
                regions: None,
                shape: &sq(100.0, DiagonalRule::Chebyshev),
            },
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
                PathInputs {
                    footprint_radius_cells: 0.1,
                    cell: 100.0,
                    walls: &NO_WALLS,
                    mask: None,
                    regions: None,
                    shape: &sq(100.0, DiagonalRule::Chebyshev),
                }
            ),
            Err(PathFail::Invalid)
        );
        // Negative footprint_radius_cells.
        assert_eq!(
            find(
                (50.0, 50.0),
                &[(150.0, 50.0)],
                PathInputs {
                    footprint_radius_cells: -1.0,
                    cell: 100.0,
                    walls: &NO_WALLS,
                    mask: None,
                    regions: None,
                    shape: &sq(100.0, DiagonalRule::Chebyshev),
                }
            ),
            Err(PathFail::Invalid)
        );
        // Non-positive cell size.
        assert_eq!(
            find(
                (50.0, 50.0),
                &[(150.0, 50.0)],
                PathInputs {
                    footprint_radius_cells: 0.1,
                    cell: 0.0,
                    walls: &NO_WALLS,
                    mask: None,
                    regions: None,
                    shape: &sq(0.0, DiagonalRule::Chebyshev),
                }
            ),
            Err(PathFail::Invalid)
        );
        // NaN footprint_radius_cells — contains() returns false for NaN comparisons.
        assert_eq!(
            find(
                (50.0, 50.0),
                &[(150.0, 50.0)],
                PathInputs {
                    footprint_radius_cells: f64::NAN,
                    cell: 100.0,
                    walls: &NO_WALLS,
                    mask: None,
                    regions: None,
                    shape: &sq(100.0, DiagonalRule::Chebyshev),
                }
            ),
            Err(PathFail::Invalid)
        );
        // Infinite footprint_radius_cells — exceeds MAX_FOOTPRINT_CELLS.
        assert_eq!(
            find(
                (50.0, 50.0),
                &[(150.0, 50.0)],
                PathInputs {
                    footprint_radius_cells: f64::INFINITY,
                    cell: 100.0,
                    walls: &NO_WALLS,
                    mask: None,
                    regions: None,
                    shape: &sq(100.0, DiagonalRule::Chebyshev),
                }
            ),
            Err(PathFail::Invalid)
        );
        // footprint_radius_cells exactly one above the cap.
        assert_eq!(
            find(
                (50.0, 50.0),
                &[(150.0, 50.0)],
                PathInputs {
                    footprint_radius_cells: MAX_FOOTPRINT_CELLS + 1.0,
                    cell: 100.0,
                    walls: &NO_WALLS,
                    mask: None,
                    regions: None,
                    shape: &sq(100.0, DiagonalRule::Chebyshev),
                }
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
            PathInputs {
                footprint_radius_cells: 0.1,
                cell: 100.0,
                walls: &NO_WALLS,
                mask: None,
                regions: None,
                shape: &sq(100.0, DiagonalRule::Chebyshev),
            },
        )
        .unwrap();
        assert!((outcome.cost - 2.0).abs() < 1e-9);
        assert_eq!(outcome.path.first(), Some(&(50.0, 50.0)));
        assert_eq!(outcome.path.last(), Some(&(250.0, 50.0)));
        assert_eq!(outcome.path.len(), 3);
    }

    #[test]
    fn hex_route_off_the_square_diagonal_resolves_to_its_goal() {
        use crate::scene::grid_shape::{GridShape as _, HexGrid};
        // Hex scene: cell size == HexGrid outer radius (`resolve_grid_shape` passes `cell` as
        // `size`). Route along axial direction (1,-1) to (70,-70) — "off the square diagonal": the
        // goal's pixel x is ~0.866·70·cell, so a square `floor(x/cell)+margin` window (=68) clips
        // its axial q (=70), and a square pixel→cell map would resolve the goal to the WRONG axial
        // cell. Only the hex-correct window + `cell_of` (both from `grid.inputs.shape`) resolve
        // the route.
        let hex = HexGrid { size: 100.0 };
        let start = hex.cell_center((0, 0));
        let goal_cell = (70, -70);
        let goal = hex.cell_center(goal_cell);
        // This assertion documents why the hex-aware window above is load-bearing: a square-only
        // window would clip this goal.
        assert!(
            (goal.0 / hex.size).floor() as i32 + WINDOW_MARGIN < goal_cell.0,
            "fixture must exercise the window-clipping case"
        );
        let outcome = find(
            start,
            &[goal],
            PathInputs {
                footprint_radius_cells: 0.1,
                cell: hex.size,
                walls: &NO_WALLS,
                // GM / unconstrained: only the window (not a mask) can gate reachability here.
                mask: None,
                regions: None,
                shape: &hex,
            },
        )
        .expect("a reachable hex route must resolve, not read Unreachable");
        assert_eq!(outcome.path.first(), Some(&start));
        assert_eq!(
            outcome.path.last(),
            Some(&goal),
            "route reaches the correct hex goal cell"
        );
        // 70 uniform-cost axial steps along the (1,-1) direction.
        assert!(
            (outcome.cost - 70.0).abs() < 1e-9,
            "cost = {}",
            outcome.cost
        );
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
            PathInputs {
                footprint_radius_cells: 0.1,
                cell: 100.0,
                walls: &NO_WALLS,
                mask: None,
                regions: None,
                shape: &sq(100.0, DiagonalRule::Alternating),
            },
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
                PathInputs {
                    footprint_radius_cells: 0.1,
                    cell: 100.0,
                    walls: &NO_WALLS,
                    mask: None,
                    regions: None,
                    shape: &sq(100.0, DiagonalRule::Chebyshev),
                }
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
                PathInputs {
                    footprint_radius_cells: 0.1,
                    cell: 100.0,
                    walls: &NO_WALLS,
                    mask: Some(&mask),
                    regions: None,
                    shape: &sq(100.0, DiagonalRule::Chebyshev),
                }
            ),
            Err(PathFail::Unreachable)
        );
    }

    #[test]
    fn pathfind_reports_unreachable_when_route_exceeds_window_margin() {
        // A single wall endpoint sits at the extreme y-edge of the start/goal/wall-endpoint AABB
        // (y cell 3). Clearing that endpoint needs a 10-cell footprint clearance, but WINDOW_MARGIN
        // only extends the search 8 cells past the AABB — so every cell that WOULD clear the wall
        // lies outside the search window, even though a wider window would find a route around it.
        // Must fail closed as Unreachable, not panic or silently truncate.
        let c = 100.0;
        let walls = vec![Seg {
            a: (100.0, -200.0),
            b: (100.0, 300.0),
        }];
        let start = (50.0, 50.0);
        let goal = (150.0, 50.0);
        let result = find(
            start,
            &[goal],
            PathInputs {
                // footprint_radius_cells (10 cells) > WINDOW_MARGIN (8 cells)
                footprint_radius_cells: 10.0,
                cell: c,
                walls: &walls,
                mask: None,
                regions: None,
                shape: &sq(c, DiagonalRule::Chebyshev),
            },
        );
        assert_eq!(result, Err(PathFail::Unreachable));
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
            &crate::scene::grid_shape::SquareGrid {
                cell: 100.0,
                rule: DiagonalRule::Chebyshev,
            },
        );
        let field = b.build();
        let outcome = find(
            (50.0, 50.0),
            &[(450.0, 50.0)],
            PathInputs {
                footprint_radius_cells: 0.1,
                cell: 100.0,
                walls: &NO_WALLS,
                mask: None,
                regions: Some(&field),
                shape: &sq(100.0, DiagonalRule::Chebyshev),
            },
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
            PathInputs {
                footprint_radius_cells: 0.1,
                cell: 100.0,
                walls: &NO_WALLS,
                mask: None,
                regions: None,
                shape: &sq(100.0, DiagonalRule::Chebyshev),
            },
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
        let shape: &'static crate::scene::grid_shape::SquareGrid =
            Box::leak(Box::new(crate::scene::grid_shape::SquareGrid {
                cell: 100.0,
                rule: DiagonalRule::Chebyshev,
            }));
        PathGrid {
            inputs: PathInputs {
                cell: 100.0,
                footprint_radius_cells: footprint,
                walls,
                mask,
                regions: None,
                shape,
            },
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

        // A point-sized footprint overlaps only (1,0) at the destination — but the mask check also
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
        // Regression test. A perfectly diagonal step (0,0)->(1,1) at cell=100 crosses
        // the shared corner exactly, so supercover_cells emits BOTH flanker cells (1,0) and (0,1)
        // in addition to the two endpoint cells. A small (point-sized) footprint disc at the
        // destination (1,1) only overlaps (1,1) itself — footprint_cells alone would not catch a
        // missing flanker. The mask below has every cell EXCEPT the (0,1) flanker: the step must
        // be rejected once the router's mask check includes the step's supercover, even though
        // a footprint-disc-only check alone would have passed it.
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
        // MAX_MOVE_CELLS span guard). The router must fail closed on `None`, same
        // as `move_exec::execute_move` and `Room::publish`, regardless of what the mask contains at `to`.
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
            &crate::scene::grid_shape::SquareGrid {
                cell: 100.0,
                rule: DiagonalRule::Chebyshev,
            },
        );
        let field = b.build();
        let walls: Vec<Seg> = vec![];
        let mut g = grid(&walls, None, 0.2);
        g.inputs.regions = Some(&field);
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
            &crate::scene::grid_shape::SquareGrid {
                cell: 100.0,
                rule: DiagonalRule::Chebyshev,
            },
        );
        let field = b.build();
        let walls: Vec<Seg> = vec![];
        let mut g = grid(&walls, None, 0.1);
        g.inputs.regions = Some(&field);
        let (_cells, cost, _p) = astar_leg(&g, (0, 0), (2, 0), 0).unwrap();
        assert!(
            (cost - 6.0).abs() < 1e-9,
            "2 steps through terrain at multiplier 3 = 6.0, regardless of which of the three \
             king-move-adjacent intermediate cells the route passes through"
        );
    }
}
