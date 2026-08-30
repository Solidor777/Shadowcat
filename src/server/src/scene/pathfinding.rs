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
    /// Movement-budget ceiling in cells; `Some` ⇒ the found route is cut at
    /// the last cell whose cumulative weighted cost fits, with
    /// `PathOutcome.truncated` set — the preview half of the executor's own
    /// budget stop, priced through the same per-step symbols.
    pub budget_cells: Option<f64>,
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
/// the disc iff the disc center is within `r_scene` of the cell's AABB.
///
/// INVARIANT: `anchor` is always the single cell `GridShape::cell_of` assigns `ctr` to (every
/// caller passes `anchor = to_cell(ctr)`). A positive-radius disc centered exactly on a shared
/// cell boundary or corner has genuine POSITIVE-AREA overlap with every cell touching that point
/// (not a spurious tie) and correctly admits all of them. A zero-radius `ctr` is a literal point,
/// not a disc, so "which cell does it belong to" is genuinely ambiguous exactly on a boundary;
/// this resolves it to `anchor` — the same single cell `cell_of`'s floor-based assignment gives —
/// rather than tying with every cell sharing that point, matching the trait contract
/// (`GridShape::footprint_cells`'s doc: "the anchor cell is always included").
pub(crate) fn footprint_cells(anchor: Cell, ctr: vision::P, r_scene: f64, cell: f64) -> Vec<Cell> {
    if r_scene <= 0.0 {
        return vec![anchor];
    }
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
/// (arrest/terrain apply elsewhere — see this function's region-gate step).
pub(crate) fn cell_enterable(grid: &PathGrid, from: Cell, to: Cell) -> bool {
    let (i0, j0, i1, j1) = grid.window;
    if to.0 < i0 || to.0 > i1 || to.1 < j0 || to.1 > j1 {
        return false;
    }
    // The footprint radius keeps the INDEXING scale rather than converting through
    // `GridShape::world_units_per_cell` like an authored distance would — that symbol's own note
    // states why: on square the radius is the authored block's half-diagonal; on hex (owner
    // ruling — a token's authored size counts HEXES) it is the circumscribing radius of the
    // authored hex count, `footprint::resolve_footprint_cells`'s own model — so rescaling it through
    // the world-unit conversion changes what a token occupies on either shape.
    // `move_exec::execute_move` derives its own `r_scene` the same way, and the route-vs-gate
    // footprint comparison depends on the two agreeing.
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
mod astar_tests;

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

/// The ONE budget-boundary decision every consumer shares: whether spending
/// `step_cells` more on top of `spent_cells` still fits `budget_cells`,
/// tolerance included. `move_exec::execute_move`'s budget stop, `find`'s
/// preview truncation and `navmesh::truncate_at_budget`'s span cut all call
/// this ONE predicate, so the preview and the executor cannot disagree at
/// the boundary by an epsilon only one of them applies.
pub(crate) fn budget_admits_step(spent_cells: f64, step_cells: f64, budget_cells: f64) -> bool {
    spent_cells + step_cells <= budget_cells + 1e-9
}

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
    /// The mover's movement budget truncated the route (the preview clamp).
    pub truncated: bool,
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
    // window: both derive from `grid.inputs.shape`, so a hex goal's axial cell always lands inside
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

    // Budget truncation, applied AFTER the arrest cut (running second over the survivor means
    // the nearer cut wins): replay the per-step cost across `cells` through the SAME
    // `neighbors_with_cost` + `terrain_multiplier` pricing the accumulation and the arrest
    // replay use, and cut at the last cell whose cumulative cost fits. A cut strictly before
    // the arrest cell means the preview no longer reaches the arrest, so `arrested` clears.
    let mut truncated = false;
    if let Some(budget) = grid.inputs.budget_cells {
        let mut p = 0u8;
        let mut cum = 0.0;
        let mut keep = cells.len();
        for (i, w) in cells.windows(2).enumerate() {
            let (_, sc, next_p) = grid
                .inputs
                .shape
                .neighbors_with_cost(w[0], p)
                .into_iter()
                .find(|(next, _, _)| *next == w[1])
                .expect("cells adjacent along an already-found route are a valid grid_shape neighbor pair");
            let step = sc
                * grid
                    .inputs
                    .regions
                    .map_or(1.0, |rf| rf.terrain_multiplier(w[1]));
            if !budget_admits_step(cum, step, budget) {
                keep = i + 1;
                truncated = true;
                break;
            }
            cum += step;
            p = next_p;
        }
        if truncated {
            cells.truncate(keep);
            total = cum;
            arrested = false;
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
        truncated,
    })
}

#[cfg(test)]
mod find_tests;

#[cfg(test)]
mod tests;
