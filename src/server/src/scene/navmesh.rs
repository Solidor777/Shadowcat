//! Continuous (navmesh) pathfinding adapter. Pure geometry: builds a footprint-inflated
//! `polyanya::Mesh` from a scene's bounds + `blocksMove` wall segments, and queries any-angle
//! routes over it. Engine-owned geometry, mirroring the grid A*
//! router's fail-closed discipline (`pathfinding::find`) — the mesh itself carries WALLS ONLY;
//! impassable/terrain region weighting is handled separately, via `SceneEcs::pathfind`'s dispatch
//! to the weighted grid A* plus this module's `los_smooth`/`truncate_at_arrest`, never baked into
//! the mesh.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

#[cfg(test)]
mod smoke;

use crate::scene::grid_shape::WorldExtent;
use crate::scene::vision::Seg;
use geo::algorithm::buffer::Buffer;
use geo::Line;

/// DoS guard: a scene with more `blocksMove` segments than this fails closed (no navmesh) rather
/// than triangulating an unbounded obstacle count. Generous relative to a hand-authored scene
/// (mirrors the generosity of `movement::MAX_MOVE_CELLS` / `regions::MAX_REGION_CELLS`).
pub(crate) const MAX_NAVMESH_OBSTACLE_SEGMENTS: usize = 5_000;

/// Magnitude ceiling (scene-pixel units) for any coordinate that reaches an `f64 -> f32` cast in
/// this module: the CONSTRUCTION-side surfaces (the caller-supplied world `extent` rectangle, a
/// raw wall-segment endpoint, and `footprint_scene` — the footprint-inflation distance passed to
/// `line.buffer(...)`), and the QUERY-side surface (`navmesh_find`'s `start`/`waypoints`). An
/// `f64 -> f32` cast SATURATES an
/// out-of-range-but-finite value to `f32::INFINITY` rather than panicking or producing NaN, so
/// `is_finite()` alone (checked upstream) never catches it. On the construction side the
/// resulting `Vec2` reaches
/// `polyanya::Triangulation::as_navmesh` -> `spade`'s `cdt.insert(...).unwrap()`, and `spade`
/// 2.15.1 rejects any coordinate past `MAX_ALLOWED_VALUE = 2^201` with `Err(InsertionError::
/// TooLarge)` which polyanya's unhandled `.unwrap()` turns into a PANIC. `1e15` is comfortably
/// below both `f32::MAX` (~3.4e38, so the cast itself cannot saturate under this bound) and
/// spade's `2^201` (~3.2e60) ceiling, while being generously large for any real authored scene
/// (mirrors `regions::MAX_CELL_COORD`'s reasoning: bound the input BEFORE any downstream
/// arithmetic/cast, not after). On the query side (`navmesh_find`), `start`/`waypoints` reach
/// `Mesh::path` -> `path_on_layers` -> `get_closest_point_on_layers`, which does only bounded
/// point-in-polygon containment checks (no `spade` triangulation call) and returns `None` for an
/// out-of-range point rather than panicking — so this ceiling is defense-in-depth / input hygiene
/// against untrusted wire magnitudes reaching a third-party numeric library, not a proven panic
/// fix on that side (see `navmesh_find`'s doc comment for the empirically-verified distinction).
pub(crate) const MAX_NAVMESH_COORD: f64 = 1e15;

/// A built, footprint-inflated navmesh for one `(scene, footprint radius)` pair. Immutable after
/// construction — a caller-side cache rebuilds a new one on wall/bounds mutation rather than
/// mutating this in place.
pub(crate) struct NavMesh {
    /// The underlying polyanya search mesh (scene-pixel coordinates).
    pub(crate) mesh: polyanya::Mesh,
}

/// Build a footprint-inflated navmesh from a scene's world-unit `extent` (the envelope
/// `GridShape::world_extent` produces from the scene's authored grid-unit bounds, whose `min` is
/// the origin only on square) and its `blocksMove` wall segments, inflating each wall by
/// `footprint_scene` (the mover's footprint radius in world units — the radius in cells times the
/// shape's INDEXING scale, which is what a footprint is measured in). Fails closed (`None`) on: a
/// non-finite or over-magnitude corner on EITHER side, a non-positive `width()`/`height()`, a
/// non-finite/negative/over-magnitude `footprint_scene`, an obstacle count over
/// `MAX_NAVMESH_OBSTACLE_SEGMENTS`, or a triangulation/mesh-build failure — callers MUST treat
/// `None` as "no navmesh" (the scene reports `Unreachable`, never a silent all-pass). The
/// radius-RANGE refusal (`0.0..=MAX_FOOTPRINT_CELLS`) lives at the caller, which must apply it
/// before its cache key is computed; a degenerate `cell` reaches this function as a zero-area
/// envelope and is refused here.
pub(crate) fn build_navmesh(
    extent: WorldExtent,
    footprint_scene: f64,
    walls: &[Seg],
) -> Option<NavMesh> {
    let (min_x, min_y) = extent.min;
    let (max_x, max_y) = extent.max;
    if !min_x.is_finite()
        || !min_y.is_finite()
        || !max_x.is_finite()
        || !max_y.is_finite()
        || extent.width() <= 0.0
        || extent.height() <= 0.0
    {
        return None;
    }
    // `footprint_scene` is a single scene-wide value (not per-wall), so an oversized value here
    // fails the whole build rather than skipping one segment. Bounded before it reaches
    // `line.buffer(...)`: an out-of-range-but-finite value saturates to infinity in the `as f32`
    // cast that reaches `line.buffer(...)`, and panics inside `spade`'s triangulation.
    if !footprint_scene.is_finite() || footprint_scene < 0.0 {
        return None;
    }
    let footprint_scene = footprint_scene.max(0.01);
    if footprint_scene > MAX_NAVMESH_COORD {
        return None;
    }
    if walls.len() > MAX_NAVMESH_OBSTACLE_SEGMENTS {
        return None;
    }
    // Bound the magnitude of BOTH corners before the `as f32` cast that builds the outer
    // rectangle — see `MAX_NAVMESH_COORD`. A finite-but-enormous minimum saturates that cast exactly as a maximum
    // does, so neither corner may be left unchecked.
    if min_x.abs() > MAX_NAVMESH_COORD
        || min_y.abs() > MAX_NAVMESH_COORD
        || max_x.abs() > MAX_NAVMESH_COORD
        || max_y.abs() > MAX_NAVMESH_COORD
    {
        return None;
    }

    let outer = [
        glam::Vec2::new(min_x as f32, min_y as f32),
        glam::Vec2::new(max_x as f32, min_y as f32),
        glam::Vec2::new(max_x as f32, max_y as f32),
        glam::Vec2::new(min_x as f32, max_y as f32),
    ];
    let mut tri = polyanya::Triangulation::from_outer_edges(&outer);

    for seg in walls {
        if !seg.a.0.is_finite()
            || !seg.a.1.is_finite()
            || !seg.b.0.is_finite()
            || !seg.b.1.is_finite()
        {
            continue; // a malformed wall segment is skipped, never turned into a bogus obstacle
        }
        // Same saturating-cast hazard as `extent`, checked BEFORE `.buffer()`/`as f32`:
        // an ordinarily-authored-looking but oversized coordinate (e.g. 1e70) is finite and would
        // pass the `is_finite` check, but saturates to `f32::INFINITY` on cast and panics inside
        // polyanya/spade. Skip the single malformed segment, matching the non-finite branch
        // — a whole-build failure is not warranted for one bad wall.
        if seg.a.0.abs() > MAX_NAVMESH_COORD
            || seg.a.1.abs() > MAX_NAVMESH_COORD
            || seg.b.0.abs() > MAX_NAVMESH_COORD
            || seg.b.1.abs() > MAX_NAVMESH_COORD
        {
            continue;
        }
        // `blocksMove` walls have no thickness field — inflating the zero-width segment by the
        // agent's footprint radius is the correct Minkowski obstacle for a disc-footprint agent.
        let line = Line::new((seg.a.0, seg.a.1), (seg.b.0, seg.b.1));
        let inflated = line.buffer(footprint_scene);
        // `geo`'s `Buffer` is backed by `i_overlay`'s fixed-point quantization
        // (`FloatPointAdapter`), which scales its integer grid relative to
        // `max(buffer_radius, geometry_extent)`. When `footprint_scene` is very large relative to
        // this wall segment's own length, both endpoints can quantize to the SAME integer point;
        // `i_overlay::StrokeBuilder::open_segments` has an explicit degenerate-collapse guard that
        // then returns zero segments, so `.buffer()` silently returns an EMPTY `MultiPolygon` for
        // an otherwise well-formed wall — not a panic, a silent obstacle-drop (fail-OPEN: a route
        // could cross straight through a wall the scene author placed). A zero-length segment
        // (`seg.a == seg.b`) is genuinely degenerate input and may legitimately buffer to nothing;
        // that case is a no-op skip, not a build failure. Any other empty-buffer result is treated
        // as a hard, whole-build failure (`None`) rather than a per-wall skip: unlike malformed/
        // non-finite input (safe to drop), a well-formed wall that collapses only because of the
        // buffer library's internal fixed-point precision limit is real scene geometry — dropping
        // just that one wall would itself still be a silent fail-open for that specific obstacle.
        if inflated.0.is_empty() && seg.a != seg.b {
            return None;
        }
        for poly in inflated.iter() {
            let ring: Vec<glam::Vec2> = poly
                .exterior()
                .points()
                .map(|p| glam::Vec2::new(p.x() as f32, p.y() as f32))
                .collect();
            // Believed unreachable: the `inflated.0.is_empty()` check already catches
            // `i_overlay`'s degenerate-collapse guard, which is all-or-nothing at the whole-buffer
            // level (either a normal `MultiPolygon` or an empty one) — not a case where the
            // `MultiPolygon` is non-empty but an individual ring within it has <3 points. Kept as
            // a defensive filter, not a silent truncation path.
            if ring.len() >= 3 {
                tri.add_obstacle(ring);
            }
        }
    }

    Some(NavMesh {
        mesh: tri.as_navmesh(),
    })
}

/// Any-angle route `start -> waypoints[0] -> ... -> waypoints[last]` over a built navmesh.
/// Euclidean distance; concatenates per-leg polylines without a duplicated join vertex.
/// Validation mirrors `pathfinding::find`'s `Invalid` guard: waypoints non-empty and bounded by
/// `MAX_WAYPOINTS`, `start`/every waypoint finite AND bounded by `MAX_NAVMESH_COORD`. Unlike
/// `build_navmesh`'s construction-side guards (which fix a REPRODUCED `spade` panic), this
/// magnitude bound on `start`/`waypoints` is defense-in-depth / input hygiene: an oversized-but-
/// finite value here reaches `Mesh::path` -> `path_on_layers` -> `get_closest_point_on_layers`,
/// which does only bounded point-in-polygon containment checks (no `spade` triangulation call) and
/// already fails closed to `None` (this function converts that to `Err(PathFail::Unreachable)`)
/// for an out-of-range point — empirically verified against the pinned `polyanya = "0.16.1"` by
/// calling `Mesh::path` directly with oversized/infinite coordinates. The guard is kept anyway to
/// bound an untrusted wire magnitude before it reaches a third-party numeric library, and gives a
/// more precise `PathFail::Invalid` instead of an indistinguishable `Unreachable`. Any leg with no
/// route ⇒ `Unreachable`. `arrested` is always `false` — this navmesh carries walls only, no
/// region field.
pub(crate) fn navmesh_find(
    nav: &NavMesh,
    start: crate::scene::vision::P,
    waypoints: &[crate::scene::vision::P],
) -> Result<crate::scene::pathfinding::PathOutcome, crate::scene::pathfinding::PathFail> {
    use crate::scene::pathfinding::{PathFail, PathOutcome, MAX_WAYPOINTS};

    if waypoints.is_empty() || waypoints.len() > MAX_WAYPOINTS {
        return Err(PathFail::Invalid);
    }
    let all_finite = start.0.is_finite()
        && start.1.is_finite()
        && waypoints.iter().all(|w| w.0.is_finite() && w.1.is_finite());
    if !all_finite {
        return Err(PathFail::Invalid);
    }
    // Magnitude bound (not just finiteness) — defense-in-depth against an untrusted wire
    // magnitude reaching `Mesh::path`'s third-party point-in-polygon lookup, not a proven panic
    // fix (see `navmesh_find`'s doc comment for the empirically-verified distinction from
    // `build_navmesh`'s construction-side guards). Reuses `MAX_NAVMESH_COORD` rather than
    // defining a second ceiling for the query side.
    let in_bounds = start.0.abs() <= MAX_NAVMESH_COORD
        && start.1.abs() <= MAX_NAVMESH_COORD
        && waypoints
            .iter()
            .all(|w| w.0.abs() <= MAX_NAVMESH_COORD && w.1.abs() <= MAX_NAVMESH_COORD);
    if !in_bounds {
        return Err(PathFail::Invalid);
    }

    let mut full_path: Vec<crate::scene::vision::P> = vec![start];
    let mut cost = 0.0_f64;
    let mut leg_start = start;

    for &wp in waypoints {
        let from = glam::Vec2::new(leg_start.0 as f32, leg_start.1 as f32);
        let to = glam::Vec2::new(wp.0 as f32, wp.1 as f32);
        let Some(path) = nav.mesh.path(from, to) else {
            return Err(PathFail::Unreachable);
        };
        cost += path.length as f64;

        for (i, v) in path.path.iter().enumerate() {
            let pt = (v.x as f64, v.y as f64);
            if i == 0 {
                // polyanya's returned polyline may or may not repeat the query start vertex;
                // skip it only if it coincides with the point we already have, so the assembled
                // polyline never gets a duplicated join vertex regardless of that detail.
                let dx = pt.0 - leg_start.0;
                let dy = pt.1 - leg_start.1;
                if (dx * dx + dy * dy).sqrt() < 1e-6 {
                    continue;
                }
            }
            full_path.push(pt);
        }
        leg_start = wp;
    }

    Ok(PathOutcome {
        path: full_path,
        cost,
        arrested: false,
    })
}

/// Arc-length-samples `outcome.path` and truncates it at the first sample whose chord (from the
/// previous retained sample) either (a) touches a cell outside `mask` (footprint disc ∪ the
/// step's line traversal) or (b) crosses a `blocksMove` wall. `mask: None` skips check (a) — this
/// reuses the same `footprint_cells` ∪ `line_traversal` union `pathfinding::cell_enterable`'s
/// mask check applies, adapted to a continuous sample position rather than a grid
/// cell center; no forked visibility decision, so a continuous preview is fog-safe and
/// `route ⊆ gate-allowed` holds across both engines. Every cell index here is
/// produced by `grid` (`cell_of`/`footprint_cells`/`line_traversal`), NEVER by square
/// `floor(p/cell)` math: `mask` is built in the scene's own `GridShape` coordinate space, and
/// grid kind and movement model are independent axes, so a hex + continuous scene tested with
/// square indices compares two different affine maps into the same `(i32,i32)` space — an
/// arbitrary membership answer in BOTH directions (an occluded hex admitted, a visible one
/// refused). `grid` MUST be the same `resolve_grid_shape`-derived shape `mask` was built with.
/// Check (b) always runs, independent of `mask`. **Two checks, both secrecy-relevant, neither
/// substitutes for the other.** The mask check is a secrecy gate (route ⊆ gate-allowed). The wall
/// check is a router-FIDELITY guarantee for PUBLIC walls (the navmesh's true polyline may detour
/// around a wall corner, but once downsampled to at most `MAX_VISION_SAMPLES` arc-length samples,
/// a chord between two samples straddling that corner could otherwise cross the wall the true
/// route avoided) AND a secrecy gate whenever the `walls` slice carries geometry the requester
/// cannot see. The caller closes the secrecy half by construction: `SceneEcs::pathfind` passes the
/// PER-REQUESTER `move_walls(scene, Some(user))` set for a non-GM, so a `gm_only` wall never
/// reaches this function on a non-GM's behalf and cannot truncate their route into a shape that
/// discloses it. `mask: None` and `walls: &[]` together ⇒ returned unchanged.
///
/// A zero/one-sample truncation (the very first sample already fails a check) yields a
/// single-point path at `outcome.path[0]` with `cost: 0.0` — the caller is responsible for
/// treating a degenerate result as appropriate for its context. The SAME single-point-at-
/// `path[0]`/`cost: 0.0` shape is also this function's fail-closed response to a degenerate
/// `cell`/`footprint_radius_cells` input (see this function's own range guard) — unlike
/// `build_navmesh` (returns
/// `Option<NavMesh>`, can simply return `None`), this function's return type has no "absent"
/// state, so truncating to just the start point is the most restrictive output it can produce.
pub(crate) fn clip_to_visible_mask(
    outcome: crate::scene::pathfinding::PathOutcome,
    mask: Option<&std::collections::BTreeSet<crate::scene::pathfinding::Cell>>,
    cell: f64,
    footprint_radius_cells: f64,
    walls: &[crate::scene::vision::Seg],
    grid: &dyn crate::scene::grid_shape::GridShape,
) -> crate::scene::pathfinding::PathOutcome {
    if outcome.path.len() < 2 {
        return outcome;
    }
    // Defense-in-depth, mirroring `build_navmesh`'s guard style/ordering (same file, same
    // convention): `cell` and `footprint_radius_cells` flow into `r_scene` and then into the
    // shape's `footprint_cells`. `SquareGrid`'s impl delegates to `pathfinding::footprint_cells`,
    // which has NO internal cap on its nested cell-scan loop (`HexGrid`'s own impl is ring-bounded,
    // so this guard is load-bearing specifically for the square path) — an
    // ordinary-looking oversized-but-finite `footprint_radius_cells` (e.g. `1e6`, no NaN/Inf
    // needed) drives a catastrophic iteration count, and an extreme value saturates the `as i32`
    // cast (Rust's `f64 as i32` is a stable saturating cast: `Infinity -> i32::MAX`, `-Infinity ->
    // i32::MIN`, `NaN -> 0`), making the loop range `i32::MIN..=i32::MAX`. `build_navmesh` already
    // guards the identical value with the same `MAX_FOOTPRINT_CELLS` range check; reused here
    // verbatim. Fail-closed truncates to the start point (see `clip_to_visible_mask`'s doc
    // comment) rather than
    // panicking or returning the original unclipped outcome.
    if !cell.is_finite()
        || cell <= 0.0
        || !(0.0..=crate::scene::pathfinding::MAX_FOOTPRINT_CELLS).contains(&footprint_radius_cells)
    {
        return crate::scene::pathfinding::PathOutcome {
            path: vec![outcome.path[0]],
            cost: 0.0,
            arrested: outcome.arrested,
        };
    }
    if mask.is_none() && walls.is_empty() {
        return outcome;
    }

    // Indexing scale, never `GridShape::world_units_per_cell` — that symbol's own note states why
    // a footprint radius is not the class of authored distance that converts. The clip's whole
    // purpose is to apply the SAME footprint predicate `pathfinding::cell_enterable` applies, so a
    // disc sized differently here would break the `route ⊆ gate-allowed` invariant outright.
    let r_scene = footprint_radius_cells.max(0.0) * cell;
    // Dummy duration: `sample_path` is a time/arc-length sampler shared with `MoveStream`
    // playback; only `.pos` is used here, so the duration value is immaterial.
    let samples = crate::scene::move_stream::sample_path(&outcome.path, cell, 1.0);

    let mut truncated: Vec<(f64, f64)> = vec![samples[0].pos];
    let mut prev = samples[0].pos;
    for s in samples.iter().skip(1) {
        let mask_ok = match mask {
            None => true,
            Some(mask) => {
                let to_cell = grid.cell_of(s.pos);
                let footprint = grid.footprint_cells(to_cell, s.pos, r_scene, cell);
                footprint.iter().all(|c| mask.contains(c))
                    && match grid.line_traversal(prev, s.pos, cell) {
                        Some(cells) => cells.iter().all(|c| mask.contains(c)),
                        None => false, // fail-closed: a degenerate/over-cap span truncates here
                    }
            }
        };
        // Skip a single malformed (non-finite-endpoint) wall segment rather than rejecting the
        // whole call — mirrors `build_navmesh`'s per-segment skip semantics (same file). A NaN
        // wall coordinate makes EVERY comparison inside `segments_cross` evaluate `false` (no
        // crossing detected), which would otherwise silently fail-OPEN the wall-crossing check
        // this function exists to enforce; a malformed individual wall must not blind the check
        // against every OTHER valid wall.
        let wall_ok = !walls
            .iter()
            .filter(|w| {
                w.a.0.is_finite() && w.a.1.is_finite() && w.b.0.is_finite() && w.b.1.is_finite()
            })
            .any(|w| crate::scene::segments_cross(prev, s.pos, w.a, w.b));
        if !mask_ok || !wall_ok {
            break;
        }
        truncated.push(s.pos);
        prev = s.pos;
    }

    // Recompute cost as the Euclidean length of the truncated polyline (the original `cost`
    // is only valid for the full, untruncated route).
    let new_cost: f64 = truncated
        .windows(2)
        .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
        .sum();

    crate::scene::pathfinding::PathOutcome {
        path: truncated,
        cost: new_cost,
        arrested: outcome.arrested,
    }
}

/// Line-of-sight smoothing (string-pull) for a WEIGHTED continuous route. Input is the
/// cell-center polyline `pathfinding::find` produced over the region field; output restores
/// any-angle geometry by straightening spans that pass ONLY through plain, visible, unobstructed
/// cells. A span `path[i]..path[j]` (j >= i+2) is straightened only when every cell its chord
/// enters is (a) in `mask` when `Some`, (b) crossed by no `blocksMove` wall, (c) not impassable,
/// (d) not arrest, (e) not weighted terrain (`terrain_multiplier <= 1.0`). Conditions (c)-(e) keep
/// smoothing away from any "special" cell, so a straightened chord can never shortcut INTO
/// terrain/impassable/arrest the weighted search routed around or truncated at — the smoothed
/// route's gate/secrecy/cost properties are therefore <= the grid route's. The single grid step
/// `path[i] -> path[i+1]` is ALWAYS kept unconditionally (it already passed `find`'s per-cell
/// gate), so progress to the goal is guaranteed and cells adjacent to special terrain stay
/// grid-stepped.
///
/// `cost` is recomputed EXACTLY, not carried through unchanged: for every smoothed span (kept
/// grid step or straightened chord), `world_per_cell` (the authored-distance conversion — never
/// `cell`, the indexing scale) converts the span's Euclidean length to cells, and the span is
/// priced at ITS OWN destination cell's terrain multiplier. This is exact rather than
/// approximate for both span kinds: a straightened chord's every entered cell (destination
/// included) carries multiplier `<= 1.0` by construction (condition (e) above), so a per-window
/// closed form and a per-transition telescoped sum over any denser sampling agree exactly; a kept
/// grid step is exactly the ONE king-move edge `find`/`astar_leg` already priced at its
/// destination cell's multiplier, and `gate_walk`'s identity property means `move_exec::
/// execute_move` re-walking this same span produces no intermediate transition to disagree with
/// — so this is the same number `execute_move` reports for the identical route (see
/// `cost_parity`'s `continuous_smoothed_preview_cost_equals_executor_cost`). "Entered cells" = the
/// destination footprint disc ∪ the step line traversal, the SAME union
/// `pathfinding::cell_enterable` and `clip_to_visible_mask` apply, indexed through `grid` — which
/// MUST be the shape both `mask` and `field` were built with (`resolve_grid_shape`), since the
/// weighted route this smooths is itself a route of `grid`-space cell centers. Fail-closed on two
/// independent levels: (1) whole-input
/// short-circuit — `< 3` vertices, or a degenerate `cell`/`footprint_radius_cells`, returns the
/// input unchanged; (2) per-span fallback — an over-cap/degenerate `line_traversal` for one
/// candidate chord fails only that chord, leaving that span at its single unconditional grid step
/// while smoothing continues over the rest of the path.
pub(crate) fn los_smooth(
    outcome: crate::scene::pathfinding::PathOutcome,
    walls: &[crate::scene::vision::Seg],
    mask: Option<&std::collections::BTreeSet<crate::scene::pathfinding::Cell>>,
    field: &crate::scene::regions::RegionField,
    cell: f64,
    footprint_radius_cells: f64,
    grid: &dyn crate::scene::grid_shape::GridShape,
) -> crate::scene::pathfinding::PathOutcome {
    use crate::scene::pathfinding::Cell;
    if outcome.path.len() < 3
        || !cell.is_finite()
        || cell <= 0.0
        || !(0.0..=crate::scene::pathfinding::MAX_FOOTPRINT_CELLS).contains(&footprint_radius_cells)
    {
        return outcome;
    }
    // Indexing scale, never `GridShape::world_units_per_cell` — that symbol's own note states why
    // a footprint radius is not the class of authored distance that converts. A straightened chord
    // is admitted by the same footprint predicate the weighted search already applied per cell, so
    // this disc must be the one `pathfinding::cell_enterable` sized.
    let r_scene = footprint_radius_cells.max(0.0) * cell;
    let path = outcome.path.clone();

    // True iff the straight chord a->b passes only through plain, visible, unobstructed cells.
    let chord_ok = |a: (f64, f64), b: (f64, f64)| -> bool {
        let samples = crate::scene::move_stream::sample_path(&[a, b], cell, 1.0);
        if samples.len() < 2 {
            // Coincident/near-coincident endpoints (`sample_path`'s `total_len < 1e-9` guard):
            // no traversal span exists to check the chord's own cell against the region field,
            // mask, or walls. Refuse rather than silently passing an unchecked cell.
            return false;
        }
        let mut prev = samples[0].pos;
        for s in samples.iter().skip(1) {
            let to = grid.cell_of(s.pos);
            let mut entered: Vec<Cell> = grid.footprint_cells(to, s.pos, r_scene, cell);
            match grid.line_traversal(prev, s.pos, cell) {
                Some(sc) => entered.extend(sc),
                None => return false, // degenerate/over-cap span: fail closed (do not straighten)
            }
            for c in &entered {
                if field.is_impassable(*c)
                    || field.is_arrest(*c)
                    || field.terrain_multiplier(*c) > 1.0
                {
                    return false;
                }
                if let Some(m) = mask {
                    if !m.contains(c) {
                        return false;
                    }
                }
            }
            // Wall crossing, checked independent of mask. A secrecy gate whenever `walls` carries
            // geometry the requester cannot see (the caller passes the per-requester set — see
            // `clip_to_visible_mask`'s doc comment), a fidelity guarantee for public walls
            // otherwise. Skip a malformed (non-finite-endpoint) wall so one NaN wall cannot
            // fail-open the check — mirrors `clip_to_visible_mask`.
            if walls
                .iter()
                .filter(|w| {
                    w.a.0.is_finite() && w.a.1.is_finite() && w.b.0.is_finite() && w.b.1.is_finite()
                })
                .any(|w| crate::scene::segments_cross(prev, s.pos, w.a, w.b))
            {
                return false;
            }
            prev = s.pos;
        }
        true
    };

    let n = path.len();
    let mut smoothed: Vec<(f64, f64)> = vec![path[0]];
    let mut i = 0usize;
    while i < n - 1 {
        // Keep the single grid step unconditionally; greedily extend as far as a chord stays clear.
        let mut best = i + 1;
        let mut j = i + 2;
        while j < n && chord_ok(path[i], path[j]) {
            best = j;
            j += 1;
        }
        smoothed.push(path[best]);
        i = best;
    }

    // Exact per-span cost: `world_per_cell` (the authored-distance conversion — never `cell`, the
    // indexing scale) converts each window's Euclidean length to cells; the window is priced at
    // ITS OWN destination cell's terrain multiplier. See this function's doc comment for why a
    // closed form over the (unsampled) window endpoints is exact for both a straightened chord
    // (every entered cell, destination included, carries multiplier `<= 1.0` by construction) and
    // a kept single grid step (the one king-move edge `find` already priced this way).
    let world_per_cell = grid.world_units_per_cell();
    let cost: f64 = smoothed
        .windows(2)
        .map(|w| {
            let dist = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
            (dist / world_per_cell) * field.terrain_multiplier(grid.cell_of(w[1]))
        })
        .sum();

    crate::scene::pathfinding::PathOutcome {
        path: smoothed,
        cost,
        arrested: outcome.arrested,
    }
}

/// Truncate a continuous (polyanya) route at the first VISIBLE arrest cell TRANSITION, mirroring
/// `pathfinding::find`'s arrest truncation ("arrest is honest in preview") for the
/// walls-only continuous path — which does not go through `find`. Arc-length-samples the route
/// (`move_stream::sample_path`'s `SAMPLES_PER_CELL` density puts several consecutive samples in
/// the same cell) and cuts at the first sample whose cell differs from the last distinct cell seen
/// and is an arrest cell in `field` (the per-requester field, so a secret arrest is absent and
/// never truncates a player's preview — it springs at `move_exec`). Cell-entry-transition dedup,
/// not a raw per-sample check: the start cell is never a trigger even while several samples still
/// sit inside it — a token already standing somewhere is not "entering" it, parity with `find`'s
/// `.skip(1)` over CELLS. Cells come from `grid.cell_of`, which MUST be the shape `field` was
/// rasterized with — a square index tested against a hex-axial field truncates the preview at a
/// different place on the map entirely. A route with no arrest transition is returned UNCHANGED
/// (no resample).
/// On truncation, cost is recomputed as the Euclidean length of the surviving polyline.
pub(crate) fn truncate_at_arrest(
    outcome: crate::scene::pathfinding::PathOutcome,
    field: &crate::scene::regions::RegionField,
    cell: f64,
    grid: &dyn crate::scene::grid_shape::GridShape,
) -> crate::scene::pathfinding::PathOutcome {
    if outcome.path.len() < 2 || !cell.is_finite() || cell <= 0.0 {
        return outcome;
    }
    let samples = crate::scene::move_stream::sample_path(&outcome.path, cell, 1.0);
    let to_cell = |p: (f64, f64)| -> (i32, i32) { grid.cell_of(p) };
    let mut prev_cell = to_cell(samples[0].pos);
    let mut hit = None;
    for (i, s) in samples.iter().enumerate().skip(1) {
        let c = to_cell(s.pos);
        if c == prev_cell {
            continue;
        }
        if field.is_arrest(c) {
            hit = Some(i);
            break;
        }
        prev_cell = c;
    }
    let Some(end) = hit else {
        return outcome; // no arrest cell transition on the route: unchanged
    };
    let kept: Vec<(f64, f64)> = samples[..=end].iter().map(|s| s.pos).collect();
    let cost: f64 = kept
        .windows(2)
        .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
        .sum();
    crate::scene::pathfinding::PathOutcome {
        path: kept,
        cost,
        arrested: true,
    }
}

#[cfg(test)]
mod tests;
