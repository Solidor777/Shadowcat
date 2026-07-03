//! M10f-1 continuous (navmesh) pathfinding adapter. Pure geometry: builds a footprint-inflated
//! `polyanya::Mesh` from a scene's bounds + `blocksMove` wall segments, and queries any-angle
//! routes over it. Engine-owned geometry (ARCHITECTURE §6 exception), mirroring the grid A*
//! router's fail-closed discipline (`scene/pathfinding.rs`) — this checkpoint carries WALLS ONLY;
//! impassable/terrain regions land in M10f-4 (parent spec §7/§10).

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

    // A later checkpoint caches `polyanya::Mesh` behind a `std::sync::Mutex` on `SceneEcs`, which
    // itself lives behind a `tokio::sync::RwLock` shared across connection tasks — this requires
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
// TODO: remove once the navmesh cache/dispatch caller lands and reads this constant.
#[allow(dead_code)]
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
// TODO: remove once the navmesh cache/dispatch caller lands and constructs this.
#[allow(dead_code)]
pub(crate) struct NavMesh {
    pub(crate) mesh: polyanya::Mesh,
}

/// Build a footprint-inflated navmesh from a scene's bounds (grid units; converted to scene
/// pixels via `cell`) and `blocksMove` wall segments. Fails closed (`None`) on: non-finite/
/// non-positive bounds or cell size, a non-finite/negative/over-cap footprint radius, an obstacle
/// count over `MAX_NAVMESH_OBSTACLE_SEGMENTS`, or a triangulation/mesh-build failure — callers
/// MUST treat `None` as "no navmesh" (the scene reports `Unreachable`, never a silent all-pass).
// TODO: remove once the navmesh cache/dispatch caller (movementModel dispatch) lands and calls this.
#[allow(dead_code)]
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
/// route ⇒ `Unreachable`. `arrested` is always `false` — this checkpoint's navmesh carries no
/// region field.
// TODO: remove once the navmesh cache/dispatch caller lands and calls this.
#[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!outcome.arrested, "M10f-1 navmesh carries no regions");
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
}
