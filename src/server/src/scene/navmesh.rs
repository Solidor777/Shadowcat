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
mod smoke {
    // Locks down the real polyanya 0.16 headless API before the adapter is built on top: a bare
    // rectangle with no obstacles, queried start->goal, must return a straight path whose length
    // equals the Euclidean distance.
    #[test]
    fn bare_rectangle_paths_straight_line() {
        let outer = [
            glam::Vec2::new(0.0, 0.0),
            glam::Vec2::new(1000.0, 0.0),
            glam::Vec2::new(1000.0, 1000.0),
            glam::Vec2::new(0.0, 1000.0),
        ];
        let tri = polyanya::Triangulation::from_outer_edges(&outer);
        let mesh = tri.as_navmesh();
        let path = mesh
            .path(glam::Vec2::new(50.0, 50.0), glam::Vec2::new(950.0, 50.0))
            .expect("straight route across an empty rectangle must exist");
        assert!(
            (path.length - 900.0).abs() < 1.0,
            "expected ~900, got {}",
            path.length
        );
        // `Path::path` holds only turning points plus the destination (the start is implicit,
        // per polyanya's `Path` doc comment) — a straight, unobstructed route is a single vertex.
        assert!(!path.path.is_empty(), "path must have at least 1 vertex");
        let last = path.path.last().unwrap();
        assert!(
            (last.x - 950.0).abs() < 1.0 && (last.y - 50.0).abs() < 1.0,
            "last vertex must be the goal, got {:?}",
            last
        );
    }

    // `SceneEcs::navmesh_cache` caches `polyanya::Mesh` behind a `std::sync::Mutex`, which itself
    // lives behind a `tokio::sync::RwLock` shared across connection tasks — this requires
    // `Mesh: Send + Sync`. Assert the bound at the point the dependency enters the tree so a
    // violation surfaces here, not after a cache is already built on top of it.
    #[test]
    fn mesh_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<polyanya::Mesh>();
    }
}

use crate::scene::vision::Seg;
use geo::algorithm::buffer::Buffer;
use geo::Line;

/// DoS guard: a scene with more `blocksMove` segments than this fails closed (no navmesh) rather
/// than triangulating an unbounded obstacle count. Generous relative to a hand-authored scene
/// (mirrors the generosity of `movement::MAX_MOVE_CELLS` / `regions::MAX_REGION_CELLS`).
pub(crate) const MAX_NAVMESH_OBSTACLE_SEGMENTS: usize = 5_000;

/// Magnitude ceiling (scene-pixel units) for any coordinate that reaches an `f64 -> f32` cast in
/// this module: the CONSTRUCTION-side surfaces (derived `w_px`/`h_px` scene-pixel bounds, a raw
/// wall-segment endpoint, and `footprint_scene` — the footprint-inflation distance passed to
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

/// Build a footprint-inflated navmesh from a scene's bounds (grid units; converted to scene
/// pixels via `cell`) and `blocksMove` wall segments. Fails closed (`None`) on: non-finite/
/// non-positive bounds or cell size, a non-finite/negative/over-cap footprint radius, an obstacle
/// count over `MAX_NAVMESH_OBSTACLE_SEGMENTS`, or a triangulation/mesh-build failure — callers
/// MUST treat `None` as "no navmesh" (the scene reports `Unreachable`, never a silent all-pass).
pub(crate) fn build_navmesh(
    bounds: (f64, f64),
    cell: f64,
    walls: &[Seg],
    footprint_radius_cells: f64,
) -> Option<NavMesh> {
    let (w, h) = bounds;
    if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return None;
    }
    if !cell.is_finite() || cell <= 0.0 {
        return None;
    }
    // The grid engine's `pathfinding::find` bounds `footprint_radius` to
    // `0.0..=MAX_FOOTPRINT_CELLS` before doing any work; this continuous adapter reuses the SAME
    // ceiling so an untrusted `footprintRadius` on the wire cannot drive an unbounded
    // `geo::Buffer` inflation here, nor an unbounded footprint-cell scan downstream — no new DoS
    // surface distinct from the grid router's.
    if !(0.0..=crate::scene::pathfinding::MAX_FOOTPRINT_CELLS).contains(&footprint_radius_cells) {
        return None;
    }
    if walls.len() > MAX_NAVMESH_OBSTACLE_SEGMENTS {
        return None;
    }
    let footprint_scene = (footprint_radius_cells * cell).max(0.01);
    // `footprint_scene` is a single scene-wide value (not per-wall), so an oversized value here
    // fails the whole build rather than skipping one segment. `cell` has no upper-magnitude bound
    // elsewhere in this module (only `is_finite() && > 0.0`), so a tiny-but-finite `bounds` paired
    // with an extreme `cell` can keep `w_px`/`h_px` under `MAX_NAVMESH_COORD` while still driving
    // `footprint_scene` past it (e.g. `bounds=(1e-23,1e-23)`, `cell=1e37`,
    // `footprint_radius_cells=MAX_FOOTPRINT_CELLS` -> `footprint_scene ~= 6.4e38 > f32::MAX`),
    // which would otherwise saturate the buffered ring's vertices to infinity in the `as f32`
    // cast below and panic inside `spade`, same hazard as `MAX_NAVMESH_COORD`'s doc comment.
    if footprint_scene.abs() > MAX_NAVMESH_COORD {
        return None;
    }
    let (w_px, h_px) = (w * cell, h * cell);
    // Bound the DERIVED (post-multiplication) magnitude, not just the raw inputs: `w`/`cell`
    // individually finite-and-small can still multiply into an out-of-range product. Checked
    // before the `as f32` cast below — see `MAX_NAVMESH_COORD`'s doc comment for why.
    if w_px.abs() > MAX_NAVMESH_COORD || h_px.abs() > MAX_NAVMESH_COORD {
        return None;
    }

    let outer = [
        glam::Vec2::new(0.0, 0.0),
        glam::Vec2::new(w_px as f32, 0.0),
        glam::Vec2::new(w_px as f32, h_px as f32),
        glam::Vec2::new(0.0, h_px as f32),
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
        // Same saturating-cast hazard as `bounds`/`cell`, checked BEFORE `.buffer()`/`as f32`:
        // an ordinarily-authored-looking but oversized coordinate (e.g. 1e70) is finite and would
        // pass the check above, but saturates to `f32::INFINITY` on cast and panics inside
        // polyanya/spade. Skip the single malformed segment, matching the non-finite branch above
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
            // Believed unreachable: the `inflated.0.is_empty()` check above already catches
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
    // fix (see this function's doc comment above for the empirically-verified distinction from
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
/// `cell`/`footprint_radius_cells` input (see the guard below) — unlike `build_navmesh` (returns
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
    // verbatim. Fail-closed truncates to the start point (see the doc comment above) rather than
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
/// grid-stepped. `cost` and `arrested` are carried through unchanged (the grid weighted cost is a
/// valid, slightly-conservative budget for the straighter geometry — the same preview-vs-execution
/// divergence already present on the grid engine's own route cost). "Entered cells" = the
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

    crate::scene::pathfinding::PathOutcome {
        path: smoothed,
        cost: outcome.cost,
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
mod tests {
    use super::*;
    use crate::scene::pathfinding::PathOutcome;
    use crate::scene::regions::{RegionBehavior, RegionField, RegionShape};

    fn oc(path: Vec<(f64, f64)>) -> PathOutcome {
        PathOutcome {
            path,
            cost: 7.0,
            arrested: false,
        }
    }
    fn empty_field() -> RegionField {
        RegionField::builder().build()
    }
    fn test_grid() -> crate::scene::grid_shape::SquareGrid {
        crate::scene::grid_shape::SquareGrid {
            cell: 100.0,
            rule: crate::scene::pathfinding::DiagonalRule::Chebyshev,
        }
    }
    fn terrain_on(x0: f64, y0: f64, x1: f64, y1: f64, mult: f64) -> RegionField {
        let mut b = RegionField::builder();
        b.add(
            &RegionShape::Rect { x0, y0, x1, y1 },
            RegionBehavior::Terrain,
            mult,
            100.0,
            &test_grid(),
        );
        b.build()
    }
    fn arrest_on(x0: f64, y0: f64, x1: f64, y1: f64) -> RegionField {
        let mut b = RegionField::builder();
        b.add(
            &RegionShape::Rect { x0, y0, x1, y1 },
            RegionBehavior::Arrest,
            1.0,
            100.0,
            &test_grid(),
        );
        b.build()
    }

    #[test]
    fn truncate_at_arrest_cuts_at_first_visible_arrest_cell() {
        // Straight route (50,50)->(450,50): cells (0,0)..(4,0). Arrest on cell (2,0) = Rect
        // [200,0]-[300,100]. Route truncates on entry to (2,0); the surviving path stays within x<=300.
        let route = PathOutcome {
            path: vec![(50.0, 50.0), (450.0, 50.0)],
            cost: 400.0,
            arrested: false,
        };
        let out = truncate_at_arrest(
            route,
            &arrest_on(200.0, 0.0, 300.0, 100.0),
            100.0,
            &test_grid(),
        );
        assert!(out.arrested, "arrest flag set");
        assert!(
            out.path.last().unwrap().0 <= 300.0 + 1e-6,
            "truncated at/near the arrest cell entry"
        );
        assert!(
            out.path.last().unwrap().0 >= 200.0,
            "reached the arrest cell"
        );
    }

    #[test]
    fn truncate_at_arrest_no_arrest_is_unchanged() {
        let route = PathOutcome {
            path: vec![(50.0, 50.0), (450.0, 50.0)],
            cost: 400.0,
            arrested: false,
        };
        let out = truncate_at_arrest(route.clone(), &empty_field(), 100.0, &test_grid());
        assert_eq!(out.path, route.path, "no arrest region: route unchanged");
        assert!(!out.arrested);
    }

    #[test]
    fn truncate_at_arrest_start_cell_is_not_a_trigger() {
        // Arrest on the START cell (0,0); the token is already standing there, so it is not "entering".
        let route = PathOutcome {
            path: vec![(50.0, 50.0), (450.0, 50.0)],
            cost: 400.0,
            arrested: false,
        };
        let out = truncate_at_arrest(
            route,
            &arrest_on(0.0, 0.0, 100.0, 100.0),
            100.0,
            &test_grid(),
        );
        assert!(
            out.path.last().unwrap().0 > 100.0,
            "start-cell arrest does not immediately truncate"
        );
    }

    // Base L-route at cell=100: right two cells then up one. Cells (0,0),(1,0),(2,0),(2,1).
    // The straight shortcut (50,50)->(250,150) enters cell (1,1); the horizontal first leg
    // (50,50)->(250,50) enters only row-0 cells.
    const L_ROUTE: [(f64, f64); 3] = [(50.0, 50.0), (250.0, 50.0), (250.0, 150.0)];

    #[test]
    fn los_smooth_straightens_an_open_l_route() {
        let out = los_smooth(
            oc(L_ROUTE.to_vec()),
            &[],
            None,
            &empty_field(),
            100.0,
            0.1,
            &test_grid(),
        );
        assert_eq!(out.path.first().copied(), Some((50.0, 50.0)));
        assert_eq!(out.path.last().copied(), Some((250.0, 150.0)));
        assert_eq!(
            out.path.len(),
            2,
            "open route straightens to a single chord"
        );
        assert_eq!(out.cost, 7.0, "cost carried through unchanged");
        assert!(!out.arrested);
    }

    #[test]
    fn los_smooth_refuses_shortcut_through_terrain() {
        // Terrain (mult 2) on cell (1,1) = Rect [100,100]-[200,200]; the shortcut enters it.
        let field = terrain_on(100.0, 100.0, 200.0, 200.0, 2.0);
        let out = los_smooth(
            oc(L_ROUTE.to_vec()),
            &[],
            None,
            &field,
            100.0,
            0.1,
            &test_grid(),
        );
        assert_eq!(
            out.path,
            L_ROUTE.to_vec(),
            "terrain on the shortcut blocks straightening"
        );
    }

    #[test]
    fn los_smooth_refuses_shortcut_through_impassable() {
        let mut b = RegionField::builder();
        b.add(
            &RegionShape::Rect {
                x0: 100.0,
                y0: 100.0,
                x1: 200.0,
                y1: 200.0,
            },
            RegionBehavior::Impassable,
            1.0,
            100.0,
            &test_grid(),
        );
        let out = los_smooth(
            oc(L_ROUTE.to_vec()),
            &[],
            None,
            &b.build(),
            100.0,
            0.1,
            &test_grid(),
        );
        assert_eq!(
            out.path,
            L_ROUTE.to_vec(),
            "impassable on the shortcut blocks straightening"
        );
    }

    #[test]
    fn los_smooth_refuses_shortcut_across_a_wall() {
        // Vertical wall x=150, y in [80,200]. Shortcut (50,50)->(250,150) crosses it at (150,100);
        // the horizontal first leg at y=50 passes below the wall (y=50 < 80), so the middle vertex
        // is retained.
        let wall = Seg {
            a: (150.0, 80.0),
            b: (150.0, 200.0),
        };
        let out = los_smooth(
            oc(L_ROUTE.to_vec()),
            &[wall],
            None,
            &empty_field(),
            100.0,
            0.1,
            &test_grid(),
        );
        assert_eq!(
            out.path,
            L_ROUTE.to_vec(),
            "wall on the shortcut blocks straightening"
        );
    }

    #[test]
    fn los_smooth_refuses_shortcut_leaving_the_mask() {
        // Mask = every cell the L-route touches, MINUS (1,1) which only the shortcut enters.
        let mut mask: std::collections::BTreeSet<crate::scene::pathfinding::Cell> =
            std::collections::BTreeSet::new();
        for c in [(0, 0), (1, 0), (2, 0), (2, 1), (0, 1)] {
            mask.insert(c);
        }
        let out = los_smooth(
            oc(L_ROUTE.to_vec()),
            &[],
            Some(&mask),
            &empty_field(),
            100.0,
            0.1,
            &test_grid(),
        );
        assert_eq!(
            out.path,
            L_ROUTE.to_vec(),
            "a shortcut cell outside the mask blocks straightening"
        );
    }

    #[test]
    fn los_smooth_two_point_route_is_unchanged() {
        let out = los_smooth(
            oc(vec![(50.0, 50.0), (250.0, 50.0)]),
            &[],
            None,
            &empty_field(),
            100.0,
            0.1,
            &test_grid(),
        );
        assert_eq!(out.path.len(), 2, "nothing to straighten with < 3 vertices");
    }

    #[test]
    fn los_smooth_refuses_shortcut_with_coincident_endpoints_in_impassable_cell() {
        // A and C are coincident (distance 0 < sample_path's 1e-9 zero-length threshold), both
        // sitting in cell (0,0), which is entirely impassable. B is a distinct, unrelated
        // vertex the smoothing loop should not be able to skip over. Pins: `chord_ok` refuses a
        // degenerate (zero-length) chord outright rather than collapsing it to a single sample
        // and falling through to `true` without checking cell (0,0) at all.
        let mut b = RegionField::builder();
        b.add(
            &RegionShape::Rect {
                x0: 0.0,
                y0: 0.0,
                x1: 100.0,
                y1: 100.0,
            },
            RegionBehavior::Impassable,
            1.0,
            100.0,
            &test_grid(),
        );
        let field = b.build();
        let a = (50.0, 50.0);
        let c = (50.0, 50.0);
        let mid = (250.0, 50.0);
        let out = los_smooth(
            oc(vec![a, mid, c]),
            &[],
            None,
            &field,
            100.0,
            0.1,
            &test_grid(),
        );
        assert_eq!(
            out.path,
            vec![a, mid, c],
            "a coincident-endpoint chord through an impassable cell must not be straightened"
        );
    }

    #[test]
    fn los_smooth_degenerate_cell_fails_closed_to_input() {
        let out = los_smooth(
            oc(L_ROUTE.to_vec()),
            &[],
            None,
            &empty_field(),
            0.0,
            0.1,
            &test_grid(),
        );
        assert_eq!(
            out.path,
            L_ROUTE.to_vec(),
            "degenerate cell returns the grid route unchanged"
        );
    }

    #[test]
    fn degenerate_bounds_fail_closed() {
        assert!(build_navmesh((0.0, 100.0), 100.0, &[], 0.4).is_none());
        assert!(build_navmesh((100.0, -1.0), 100.0, &[], 0.4).is_none());
        assert!(build_navmesh((f64::NAN, 100.0), 100.0, &[], 0.4).is_none());
        assert!(build_navmesh((f64::INFINITY, 100.0), 100.0, &[], 0.4).is_none());
    }

    #[test]
    fn degenerate_cell_fails_closed() {
        assert!(build_navmesh((100.0, 100.0), 0.0, &[], 0.4).is_none());
        assert!(build_navmesh((100.0, 100.0), -1.0, &[], 0.4).is_none());
    }

    #[test]
    fn negative_or_non_finite_footprint_fails_closed() {
        assert!(build_navmesh((100.0, 100.0), 100.0, &[], -0.1).is_none());
        assert!(build_navmesh((100.0, 100.0), 100.0, &[], f64::NAN).is_none());
    }

    #[test]
    fn over_cap_footprint_radius_fails_closed() {
        // Mirrors `pathfinding::find`'s `MAX_FOOTPRINT_CELLS` ceiling — an untrusted wire
        // `footprintRadius` must not drive an unbounded geo::Buffer inflation.
        let over_cap = crate::scene::pathfinding::MAX_FOOTPRINT_CELLS + 1.0;
        assert!(build_navmesh((100.0, 100.0), 100.0, &[], over_cap).is_none());
        assert!(build_navmesh((100.0, 100.0), 100.0, &[], f64::INFINITY).is_none());
    }

    #[test]
    fn over_cap_obstacle_count_fails_closed() {
        let walls: Vec<Seg> = (0..(MAX_NAVMESH_OBSTACLE_SEGMENTS + 1))
            .map(|i| Seg {
                a: (i as f64, 0.0),
                b: (i as f64, 1.0),
            })
            .collect();
        assert!(build_navmesh((10_000.0, 100.0), 100.0, &walls, 0.4).is_none());
    }

    #[test]
    fn empty_scene_builds_a_navmesh() {
        assert!(build_navmesh((100.0, 100.0), 100.0, &[], 0.4).is_some());
    }

    #[test]
    fn a_malformed_wall_segment_is_skipped_not_fatal() {
        let walls = vec![Seg {
            a: (f64::NAN, 0.0),
            b: (10.0, 10.0),
        }];
        assert!(build_navmesh((100.0, 100.0), 100.0, &walls, 0.4).is_some());
    }

    #[test]
    fn oversized_but_finite_bounds_fail_closed_not_panic() {
        // `1e40` is finite and positive (passes the pre-existing `is_finite`/`> 0.0` guards) but
        // saturates an `f64 -> f32` cast to infinity, which would otherwise reach
        // `polyanya::Triangulation::as_navmesh` and panic inside `spade`'s `.unwrap()`.
        assert!(build_navmesh((1e40, 100.0), 1.0, &[], 0.4).is_none());
        assert!(build_navmesh((100.0, 1e40), 1.0, &[], 0.4).is_none());
    }

    #[test]
    fn oversized_derived_pixel_bounds_fail_closed() {
        // Neither `w` nor `cell` alone is oversized, but their product (`w_px`) is — the bound
        // must be checked on the DERIVED magnitude, not just the raw inputs.
        assert!(build_navmesh((1e10, 100.0), 1e10, &[], 0.4).is_none());
    }

    #[test]
    fn oversized_footprint_scene_fails_closed_not_panic() {
        // Reproducer independently verified by two reviewers: tiny-but-finite bounds keep
        // `w_px`/`h_px` well under `MAX_NAVMESH_COORD`, and the wall's raw endpoints are ordinary,
        // yet `footprint_radius_cells * cell` (64.0 * 1e37) overflows `f32::MAX` and would
        // saturate the buffered ring's vertices to infinity on cast, panicking inside `spade`.
        let walls = vec![Seg {
            a: (0.0, 0.0),
            b: (1.0, 1.0),
        }];
        assert!(build_navmesh(
            (1e-23, 1e-23),
            1e37,
            &walls,
            crate::scene::pathfinding::MAX_FOOTPRINT_CELLS,
        )
        .is_none());
    }

    #[test]
    fn a_footprint_buffer_that_collapses_a_wall_to_nothing_fails_the_whole_build() {
        // `footprint_scene` (buffer radius) very large relative to an individual wall segment's
        // own length drives `i_overlay`'s fixed-point quantization to collapse both of that
        // segment's endpoints onto the same integer point, which its degenerate-collapse guard
        // turns into an EMPTY buffer result — the wall would otherwise silently vanish from the
        // mesh (fail-open) rather than the build failing.
        // bounds=(1e-8,1e-8), cell=1e9 -> w_px=h_px=10 (an ordinary small domain); wall (5,0)-(5,10)
        // bisects it; footprint_radius_cells=MAX_FOOTPRINT_CELLS (64.0, within the existing cap)
        // -> footprint_scene = 6.4e10, ratio to the wall's own length (10) = 6.4e9, past the
        // collapse threshold.
        let walls = vec![Seg {
            a: (5.0, 0.0),
            b: (5.0, 10.0),
        }];
        assert!(
            build_navmesh(
                (1e-8, 1e-8),
                1e9,
                &walls,
                crate::scene::pathfinding::MAX_FOOTPRINT_CELLS,
            )
            .is_none(),
            "a wall whose Minkowski buffer silently collapses to zero polygons must fail the \
             whole build, not silently drop just that wall"
        );
    }

    #[test]
    fn zero_length_wall_segment_is_a_degenerate_noop_not_a_hard_failure() {
        // `seg.a == seg.b` is genuinely degenerate input (not a numerical-precision collapse of a
        // real wall) — an empty buffer result for it must not trigger the hard-failure path.
        let walls = vec![Seg {
            a: (5.0, 5.0),
            b: (5.0, 5.0),
        }];
        assert!(build_navmesh((100.0, 100.0), 100.0, &walls, 0.4).is_some());
    }

    #[test]
    fn ordinary_wall_and_footprint_still_build_successfully() {
        // Positive case: normal gameplay magnitudes (a small scene, a modest footprint radius, a
        // single ordinary interior wall) must still build a valid mesh, and the wall's obstacle
        // ring must actually be present in the resulting mesh (distinguishing "built successfully"
        // from "built successfully but the wall silently vanished").
        let walls = vec![Seg {
            a: (5.0, 0.0),
            b: (5.0, 10.0),
        }];
        let with_wall = build_navmesh((100.0, 100.0), 1.0, &walls, 0.4)
            .expect("an ordinary wall + footprint must build a navmesh");
        let without_wall = build_navmesh((100.0, 100.0), 1.0, &[], 0.4)
            .expect("the walls-absent baseline must also build");
        // Triangulating WITH an interior obstacle produces strictly more triangles than the empty
        // rectangle baseline — a cheap, robust proxy that the wall's obstacle ring was actually
        // incorporated into the triangulation rather than silently dropped.
        assert!(
            with_wall.mesh.layers[0].polygons.len() > without_wall.mesh.layers[0].polygons.len(),
            "a mesh built with an interior wall obstacle must have more polygons than the \
             walls-absent baseline"
        );
    }

    #[test]
    fn an_oversized_wall_segment_is_skipped_not_fatal_or_panicking() {
        // A single wall with a coordinate of 1e70 is finite (passes the non-finite guard) but
        // would saturate to f32::INFINITY on cast; it must be skipped like a malformed segment,
        // while the rest of the scene's geometry (here, none) still builds a valid mesh.
        let walls = vec![
            Seg {
                a: (1e70, 0.0),
                b: (10.0, 10.0),
            },
            Seg {
                a: (5.0, 5.0),
                b: (20.0, 20.0),
            },
        ];
        let mesh = build_navmesh((100.0, 100.0), 100.0, &walls, 0.4);
        assert!(
            mesh.is_some(),
            "the oversized segment must be skipped, not fail the whole build"
        );
    }

    #[test]
    fn empty_waypoints_is_invalid() {
        let nav = build_navmesh((100.0, 100.0), 100.0, &[], 0.4).unwrap();
        let r = navmesh_find(&nav, (50.0, 50.0), &[]);
        assert_eq!(r, Err(crate::scene::pathfinding::PathFail::Invalid));
    }

    #[test]
    fn straight_route_cost_is_euclidean() {
        let nav = build_navmesh((10.0, 10.0), 100.0, &[], 0.1).unwrap();
        let outcome = navmesh_find(&nav, (50.0, 50.0), &[(950.0, 50.0)]).unwrap();
        assert!(
            (outcome.cost - 900.0).abs() < 2.0,
            "expected ~900, got {}",
            outcome.cost
        );
        assert!(!outcome.arrested, "the navmesh carries no regions");
        assert_eq!(outcome.path.first(), Some(&(50.0, 50.0)));
        let last = *outcome.path.last().unwrap();
        assert!((last.0 - 950.0).abs() < 1.0 && (last.1 - 50.0).abs() < 1.0);
    }

    #[test]
    fn a_wall_in_the_direct_path_forces_a_detour() {
        // A vertical wall from (500,0) to (500,600) blocks the direct horizontal line at y=50,
        // forcing the route to detour around its bottom end (600 < 1000 scene height).
        let walls = vec![Seg {
            a: (500.0, 0.0),
            b: (500.0, 600.0),
        }];
        let nav = build_navmesh((10.0, 10.0), 100.0, &walls, 0.1).unwrap();
        let outcome = navmesh_find(&nav, (50.0, 50.0), &[(950.0, 50.0)]).unwrap();
        assert!(
            outcome.cost > 900.5,
            "a detour around the wall must cost more than the blocked straight line, got {}",
            outcome.cost
        );
    }

    #[test]
    fn multi_leg_route_concatenates_without_a_duplicated_join_vertex() {
        // Verifies the OBSERVABLE outcome (no duplicate consecutive vertex at a leg join), not
        // the internal skip-branch that guards against it: per the real `polyanya` source,
        // `Path::path` never actually includes a leg's start point, so this test would pass
        // identically even if that dedup branch were deleted.
        let nav = build_navmesh((10.0, 10.0), 100.0, &[], 0.1).unwrap();
        let outcome = navmesh_find(&nav, (50.0, 50.0), &[(500.0, 50.0), (950.0, 50.0)]).unwrap();
        // No two consecutive vertices should be exactly equal (a duplicated leg-join point).
        for w in outcome.path.windows(2) {
            assert_ne!(
                w[0], w[1],
                "consecutive duplicate vertex at a leg join: {:?}",
                w
            );
        }
    }

    #[test]
    fn over_cap_waypoints_is_invalid() {
        let nav = build_navmesh((100.0, 100.0), 100.0, &[], 0.1).unwrap();
        let waypoints: Vec<(f64, f64)> = (0..(crate::scene::pathfinding::MAX_WAYPOINTS + 1))
            .map(|i| (i as f64, 0.0))
            .collect();
        let r = navmesh_find(&nav, (50.0, 50.0), &waypoints);
        assert_eq!(r, Err(crate::scene::pathfinding::PathFail::Invalid));
    }

    #[test]
    fn non_finite_start_or_waypoint_is_invalid() {
        let nav = build_navmesh((100.0, 100.0), 100.0, &[], 0.1).unwrap();
        assert_eq!(
            navmesh_find(&nav, (f64::NAN, 50.0), &[(90.0, 50.0)]),
            Err(crate::scene::pathfinding::PathFail::Invalid)
        );
        assert_eq!(
            navmesh_find(&nav, (50.0, 50.0), &[(f64::INFINITY, 50.0)]),
            Err(crate::scene::pathfinding::PathFail::Invalid)
        );
    }

    #[test]
    fn oversized_but_finite_start_or_waypoint_is_rejected_by_the_magnitude_guard() {
        // `1e40` is finite (passes `is_finite()`) but exceeds `MAX_NAVMESH_COORD`. On the query
        // side this guard is defense-in-depth, not a proven panic fix: `Mesh::path`'s
        // point-in-polygon lookup already fails closed to `None` for an out-of-range point
        // (verified empirically against `polyanya = "0.16.1"`, no `spade` triangulation call on
        // this path) — without the guard this input would map to `PathFail::Unreachable` instead.
        // The guard just gives a more precise `Invalid` and bounds an untrusted wire magnitude
        // before it reaches a third-party numeric library.
        let nav = build_navmesh((100.0, 100.0), 100.0, &[], 0.1).unwrap();
        assert_eq!(
            navmesh_find(&nav, (1e40, 50.0), &[(90.0, 50.0)]),
            Err(crate::scene::pathfinding::PathFail::Invalid)
        );
        assert_eq!(
            navmesh_find(&nav, (50.0, 50.0), &[(1e40, 50.0)]),
            Err(crate::scene::pathfinding::PathFail::Invalid)
        );
    }

    use std::collections::BTreeSet;

    #[test]
    fn clip_returns_unchanged_when_mask_is_none_and_no_walls() {
        let outcome = crate::scene::pathfinding::PathOutcome {
            path: vec![(50.0, 50.0), (950.0, 50.0)],
            cost: 900.0,
            arrested: false,
        };
        let clipped = clip_to_visible_mask(outcome.clone(), None, 100.0, 0.1, &[], &test_grid());
        assert_eq!(clipped.path, outcome.path);
        assert_eq!(clipped.cost, outcome.cost);
    }

    #[test]
    fn clip_truncates_at_the_mask_boundary() {
        // A route from (50,50) to (950,50): only cells x=0..3 (i.e. up to x=400) are visible.
        let mut mask = BTreeSet::new();
        for i in 0..4 {
            mask.insert((i, 0));
        }
        let outcome = crate::scene::pathfinding::PathOutcome {
            path: vec![(50.0, 50.0), (950.0, 50.0)],
            cost: 900.0,
            arrested: false,
        };
        let clipped = clip_to_visible_mask(outcome, Some(&mask), 100.0, 0.1, &[], &test_grid());
        let last = *clipped.path.last().unwrap();
        assert!(
            last.0 <= 400.0 + 1e-6,
            "route must truncate at the visible-mask boundary, last x = {}",
            last.0
        );
        assert!(
            clipped.path.len() < 2 || clipped.cost < 900.0,
            "a truncated route must report a shorter cost than the full route"
        );
    }

    #[test]
    fn clip_leaves_a_fully_visible_route_untouched() {
        let mut mask = BTreeSet::new();
        for i in 0..10 {
            mask.insert((i, 0));
        }
        let outcome = crate::scene::pathfinding::PathOutcome {
            path: vec![(50.0, 50.0), (950.0, 50.0)],
            cost: 900.0,
            arrested: false,
        };
        let clipped =
            clip_to_visible_mask(outcome.clone(), Some(&mask), 100.0, 0.1, &[], &test_grid());
        let last_orig = *outcome.path.last().unwrap();
        let last_clipped = *clipped.path.last().unwrap();
        assert!((last_orig.0 - last_clipped.0).abs() < 1e-6);
    }

    // Design §9 requires "a goal outside the mask ⇒ Unreachable" as an exercised code path; a mask
    // that excludes the ENTIRE corridor beyond the start cell (as opposed to a partial-route
    // truncation to a still-substantial prefix, per `clip_truncates_at_the_mask_boundary` above)
    // must confine the returned route to that single visible cell, never reaching the goal.
    //
    // NOTE: the route retains 2 points here, not 1 — `sample_path`'s arc-length sampling places
    // >=1 intermediate sample inside a 100-unit-wide visible cell before the next sample crosses
    // into the (excluded) neighboring cell (verified: SAMPLES_PER_CELL=3 over a 900-unit path
    // yields ~34.6-unit sample spacing, so the second sample at x~=84.6 is still within cell
    // (0,0)). A single-point result would require the mask's visible span to be narrower than one
    // sample's arc-length step, which is not the case here.
    #[test]
    fn clip_with_a_mask_excluding_the_whole_corridor_confines_the_route_to_that_cell() {
        let mut mask = BTreeSet::new();
        mask.insert((0, 0)); // only the start cell is visible; the goal and everything beyond is not
        let outcome = crate::scene::pathfinding::PathOutcome {
            path: vec![(50.0, 50.0), (950.0, 50.0)],
            cost: 900.0,
            arrested: false,
        };
        let clipped = clip_to_visible_mask(outcome, Some(&mask), 100.0, 0.1, &[], &test_grid());
        let last = *clipped.path.last().unwrap();
        assert!(
            last.0 < 100.0,
            "a route confined to a single visible cell must never leave it, last x = {}",
            last.0
        );
        assert!(
            clipped.cost < 900.0 * 0.5,
            "a route truncated to a single visible cell must report a much shorter cost than \
             the full 900-unit route, got {}",
            clipped.cost
        );
    }

    // The mask check alone does not guarantee the returned preview never crosses a WALL — a
    // chord between two arc-length samples can cut across geometry the true navmesh polyline
    // routed around. For a PUBLIC wall this is a router-fidelity issue; for a `gm_only` wall the
    // caller closes the secrecy half by construction (a non-GM's `walls` slice never carries one —
    // see `clip_to_visible_mask`'s doc comment), so this test only exercises the fidelity half.
    // Verified independent of sample-cap spacing: any chord that geometrically crosses a
    // `blocksMove` wall must truncate there.
    #[test]
    fn clip_truncates_a_chord_that_crosses_a_wall() {
        // A wall directly bisecting the straight line from (50,50) to (950,50) at x=500.
        let walls = vec![crate::scene::vision::Seg {
            a: (500.0, -100.0),
            b: (500.0, 200.0),
        }];
        let outcome = crate::scene::pathfinding::PathOutcome {
            path: vec![(50.0, 50.0), (950.0, 50.0)],
            cost: 900.0,
            arrested: false,
        };
        let clipped = clip_to_visible_mask(outcome, None, 100.0, 0.1, &walls, &test_grid());
        let last = *clipped.path.last().unwrap();
        assert!(
            last.0 <= 500.0 + 1e-6,
            "a chord crossing a wall must truncate before the wall, last x = {}",
            last.0
        );
    }

    #[test]
    fn clip_with_over_cap_footprint_radius_fails_closed_without_hanging_or_panicking() {
        // An oversized-but-finite `footprint_radius_cells` (exceeding `MAX_FOOTPRINT_CELLS`) must
        // not be allowed to drive `pathfinding::footprint_cells`'s uncapped nested cell-scan loop
        // (which saturates to an `i32::MIN..=i32::MAX` range under `as i32`) — it must fail closed
        // to the truncated-to-start-point result instead.
        let mut mask = BTreeSet::new();
        mask.insert((0, 0));
        let outcome = crate::scene::pathfinding::PathOutcome {
            path: vec![(50.0, 50.0), (950.0, 50.0)],
            cost: 900.0,
            arrested: false,
        };
        let over_cap = crate::scene::pathfinding::MAX_FOOTPRINT_CELLS + 1.0;
        let clipped = clip_to_visible_mask(
            outcome.clone(),
            Some(&mask),
            100.0,
            over_cap,
            &[],
            &test_grid(),
        );
        assert_eq!(clipped.path, vec![outcome.path[0]]);
        assert_eq!(clipped.cost, 0.0);

        // Also verify an infinite footprint radius (would saturate the `as i32` cast even more
        // directly) is rejected the same way.
        let clipped_inf = clip_to_visible_mask(
            outcome.clone(),
            Some(&mask),
            100.0,
            f64::INFINITY,
            &[],
            &test_grid(),
        );
        assert_eq!(clipped_inf.path, vec![outcome.path[0]]);
        assert_eq!(clipped_inf.cost, 0.0);
    }

    #[test]
    fn clip_with_a_degenerate_cell_fails_closed() {
        let mut mask = BTreeSet::new();
        mask.insert((0, 0));
        let outcome = crate::scene::pathfinding::PathOutcome {
            path: vec![(50.0, 50.0), (950.0, 50.0)],
            cost: 900.0,
            arrested: false,
        };
        for bad_cell in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let clipped = clip_to_visible_mask(
                outcome.clone(),
                Some(&mask),
                bad_cell,
                0.4,
                &[],
                &test_grid(),
            );
            assert_eq!(
                clipped.path,
                vec![outcome.path[0]],
                "cell={bad_cell} must fail closed to a single-point truncation"
            );
            assert_eq!(clipped.cost, 0.0);
        }
    }

    #[test]
    fn clip_skips_a_non_finite_wall_without_hiding_a_crossing_against_other_walls() {
        // A malformed wall (NaN endpoint) must be skipped, not crash the call, and must NOT
        // blind the crossing check against an OTHER, well-formed wall that genuinely bisects the
        // route.
        let walls = vec![
            crate::scene::vision::Seg {
                a: (f64::NAN, -100.0),
                b: (f64::NAN, 200.0),
            },
            crate::scene::vision::Seg {
                a: (500.0, -100.0),
                b: (500.0, 200.0),
            },
        ];
        let outcome = crate::scene::pathfinding::PathOutcome {
            path: vec![(50.0, 50.0), (950.0, 50.0)],
            cost: 900.0,
            arrested: false,
        };
        let clipped = clip_to_visible_mask(outcome, None, 100.0, 0.1, &walls, &test_grid());
        let last = *clipped.path.last().unwrap();
        assert!(
            last.0 <= 500.0 + 1e-6,
            "the well-formed wall must still truncate the route even alongside a malformed one, \
             last x = {}",
            last.0
        );
    }

    // --- Hex + continuous regression coverage (grid kind × movement model are INDEPENDENT axes,
    // so `grid.kind:"hex"` + `movementModel:"continuous"` is a live scene). Each test pairs the
    // hex assertion with the SAME call driven by a `SquareGrid` of the same cell size: `SquareGrid`
    // delegates verbatim to the free `pathfinding::footprint_cells` / `movement::supercover_cells`
    // / `floor(p/cell)` math, so the square arm demonstrates the exact square-on-hex defect shape
    // these tests guard against, pinning each test's non-vacuity permanently rather than only at
    // authoring time.

    use crate::scene::grid_shape::{GridShape, HexGrid, SquareGrid};

    const HEX_SIZE: f64 = 50.0;

    fn hexg() -> HexGrid {
        HexGrid { size: HEX_SIZE }
    }
    /// Same cell size as `hexg()`, so the two arms differ ONLY in cell geometry.
    fn sq_same_size() -> SquareGrid {
        SquareGrid {
            cell: HEX_SIZE,
            rule: crate::scene::pathfinding::DiagonalRule::Chebyshev,
        }
    }
    fn hex_region(behavior: RegionBehavior, x0: f64, y0: f64, x1: f64, y1: f64) -> RegionField {
        let mut b = RegionField::builder();
        b.add(
            &RegionShape::Rect { x0, y0, x1, y1 },
            behavior,
            1.0,
            HEX_SIZE,
            &hexg(),
        );
        b.build()
    }

    #[test]
    fn clip_on_hex_truncates_before_an_occluded_hex_that_square_indexing_admits() {
        // OVER-REVEAL direction, the security-critical one: `clip_to_visible_mask` is the ONLY fog
        // gate on the pure-polyanya branch. Route = hex (0,0) center -> hex (0,2) center, straight
        // through hex (0,1). The mask is a 9x9 axial block minus hex (0,2) — that one hex is
        // occluded. Square indexing maps the same samples onto square cells (0,0),(0,1),(1,1),
        // (1,2),(1,3) — NONE of which is the excluded (0,2) key — so the occluded hex passes the
        // membership test and the whole route is admitted.
        let g = hexg();
        let start = g.cell_center((0, 0));
        let goal = g.cell_center((0, 2));
        let mut mask = BTreeSet::new();
        for q in -4..=4 {
            for r in -4..=4 {
                if (q, r) != (0, 2) {
                    mask.insert((q, r));
                }
            }
        }
        let outcome = crate::scene::pathfinding::PathOutcome {
            path: vec![start, goal],
            cost: 173.2,
            arrested: false,
        };

        let hexed = clip_to_visible_mask(outcome.clone(), Some(&mask), HEX_SIZE, 0.1, &[], &g);
        let last = *hexed.path.last().unwrap();
        assert_ne!(
            g.cell_of(last),
            (0, 2),
            "the clipped route must never reach the occluded hex, last = {last:?}"
        );
        let gap = ((last.0 - goal.0).powi(2) + (last.1 - goal.1).powi(2)).sqrt();
        assert!(
            gap > 40.0,
            "the route must stop clear of the occluded hex (footprint disc ∪ traversal), gap = {gap}"
        );

        let squared =
            clip_to_visible_mask(outcome, Some(&mask), HEX_SIZE, 0.1, &[], &sq_same_size());
        let sq_last = *squared.path.last().unwrap();
        assert!(
            (sq_last.0 - goal.0).abs() < 1e-6 && (sq_last.1 - goal.1).abs() < 1e-6,
            "square indexing admits the whole route into the occluded hex, \
             last = {sq_last:?}"
        );
    }

    #[test]
    fn los_smooth_on_hex_refuses_a_chord_through_an_impassable_hex_square_indexing_misses() {
        // `chord_ok`'s cell lookups must key the SAME axial space `RegionField` was rasterized in.
        // Route vertices: hex (0,0) -> hex (2,0) -> hex (4,-2) centers. The straight chord
        // (0,0)->(4,-2) runs through axial (1,0),(2,-1),(3,-1); its midpoint IS hex (2,-1)'s
        // center. Square indexing of that same chord yields (0,0),(0,-1),(1,-1),(1,-2),(2,-2),
        // (3,-2),(3,-3),(4,-3),(5,-3) — the axial key (2,-1) is never queried at all.
        let g = hexg();
        let field = hex_region(RegionBehavior::Impassable, 110.0, -95.0, 150.0, -55.0);
        assert!(
            field.is_impassable((2, -1)),
            "fixture: the impassable cell is axial (2,-1)"
        );
        assert!(
            !field.is_impassable((2, -2)) && !field.is_impassable((1, -1)),
            "fixture: exactly one hex is impassable"
        );

        let path = vec![
            g.cell_center((0, 0)),
            g.cell_center((2, 0)),
            g.cell_center((4, -2)),
        ];
        let hexed = los_smooth(oc(path.clone()), &[], None, &field, HEX_SIZE, 0.1, &g);
        assert_eq!(
            hexed.path, path,
            "the chord enters impassable hex (2,-1): the route must stay unsmoothed"
        );

        let squared = los_smooth(
            oc(path.clone()),
            &[],
            None,
            &field,
            HEX_SIZE,
            0.1,
            &sq_same_size(),
        );
        assert_eq!(
            squared.path,
            vec![path[0], path[2]],
            "square indexing never queries axial (2,-1) and wrongly straightens straight through \
             the impassable hex"
        );
    }

    #[test]
    fn truncate_at_arrest_on_hex_cuts_at_the_axial_arrest_cell_not_the_square_one() {
        // Straight route along the r=0 hex row: hex (0,0) center -> hex (4,0) center. Arrest on
        // hex (3,0) (center x ~259.8); the (2,0)/(3,0) boundary is x ~216.5. Square indexing reads
        // the same axial key (3,0) as the square cell x∈[150,200) — a DIFFERENT place on the map —
        // so it cuts the preview roughly a full hex early.
        let g = hexg();
        let field = hex_region(RegionBehavior::Arrest, 240.0, -20.0, 280.0, 20.0);
        assert!(field.is_arrest((3, 0)), "fixture: arrest is on axial (3,0)");
        assert!(
            !field.is_arrest((2, 0)) && !field.is_arrest((4, 0)),
            "fixture: exactly one hex arrests"
        );

        let route = crate::scene::pathfinding::PathOutcome {
            path: vec![g.cell_center((0, 0)), g.cell_center((4, 0))],
            cost: 346.4,
            arrested: false,
        };

        let hexed = truncate_at_arrest(route.clone(), &field, HEX_SIZE, &g);
        assert!(hexed.arrested, "the arrest hex truncates the preview");
        let last = *hexed.path.last().unwrap();
        assert_eq!(
            g.cell_of(last),
            (3, 0),
            "truncation lands ON the arrest hex's own cell, last = {last:?}"
        );
        assert!(
            last.0 > 216.5,
            "truncation is at the hex (2,0)/(3,0) boundary (x ~216.5), not the square one, \
             last x = {}",
            last.0
        );

        let squared = truncate_at_arrest(route, &field, HEX_SIZE, &sq_same_size());
        let sq_last = *squared.path.last().unwrap();
        assert!(
            sq_last.0 < 200.0,
            "square indexing cuts at square cell (3,0) = x∈[150,200), a different location on \
             the map, last x = {}",
            sq_last.0
        );
    }
}
