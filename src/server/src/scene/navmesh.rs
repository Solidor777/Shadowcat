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
/// this module (derived `w_px`/`h_px` scene-pixel bounds, a raw wall-segment endpoint, or
/// `footprint_scene` — the footprint-inflation distance passed to `line.buffer(...)`). An
/// `f64 -> f32` cast SATURATES an
/// out-of-range-but-finite value to `f32::INFINITY` rather than panicking or producing NaN, so
/// `is_finite()` alone (checked upstream) never catches it — the resulting `Vec2` then reaches
/// `polyanya::Triangulation::as_navmesh` -> `spade`'s `cdt.insert(...).unwrap()`, and `spade`
/// 2.15.1 rejects any coordinate past `MAX_ALLOWED_VALUE = 2^201` with `Err(InsertionError::
/// TooLarge)` which polyanya's unhandled `.unwrap()` turns into a PANIC. `1e15` is comfortably
/// below both `f32::MAX` (~3.4e38, so the cast itself cannot saturate under this bound) and
/// spade's `2^201` (~3.2e60) ceiling, while being generously large for any real authored scene
/// (mirrors `regions::MAX_CELL_COORD`'s reasoning: bound the input BEFORE any downstream
/// arithmetic/cast, not after).
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
        for poly in inflated.iter() {
            let ring: Vec<glam::Vec2> = poly
                .exterior()
                .points()
                .map(|p| glam::Vec2::new(p.x() as f32, p.y() as f32))
                .collect();
            if ring.len() >= 3 {
                tri.add_obstacle(ring);
            }
        }
    }

    Some(NavMesh {
        mesh: tri.as_navmesh(),
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
}
