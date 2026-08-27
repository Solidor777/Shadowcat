use super::*;
use crate::scene::pathfinding::PathOutcome;
use crate::scene::regions::{RegionBehavior, RegionField, RegionShape};

/// An origin-anchored envelope of the given world-unit spans — the shape a SQUARE scene's
/// `GridShape::world_extent` produces. These fixtures exercise `build_navmesh`'s own
/// arithmetic and refusals on literal pixel spans, where the anchor is incidental; the
/// negative-minimum case a hex scene produces is exercised by the fixtures that name it.
fn origin_extent(w: f64, h: f64) -> WorldExtent {
    WorldExtent {
        min: (0.0, 0.0),
        max: (w, h),
    }
}

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
fn degenerate_extent_fails_closed() {
    assert!(build_navmesh(origin_extent(0.0, 10_000.0), 40.0, &[]).is_none());
    assert!(build_navmesh(origin_extent(10_000.0, -100.0), 40.0, &[]).is_none());
    assert!(build_navmesh(origin_extent(f64::NAN, 10_000.0), 40.0, &[]).is_none());
    assert!(build_navmesh(origin_extent(f64::INFINITY, 10_000.0), 40.0, &[]).is_none());
}

#[test]
fn a_degenerate_or_over_magnitude_minimum_fails_closed() {
    // The envelope carries a minimum, so every refusal the maximum gets the minimum needs
    // too: a non-finite corner on either axis, an inverted rectangle (which the span check
    // catches, since a minimum past the maximum is a non-positive span rather than a negative
    // corner), and a finite-but-enormous corner that saturates the `f64 -> f32` cast and
    // panics inside `spade`.
    // Discrimination: each case moves ONLY the minimum, and the last assertion pairs an
    // over-magnitude minimum against an in-range one with the same maximum, so a guard that
    // refused on the maximum alone, or refused every negative minimum, fails it. The two
    // non-finite rows are isolating against a finiteness check narrowed to the OTHER axis:
    // dropping the x-axis check leaves the `min.0` row unrefused, and narrowing it to the x
    // axis (dropping the y-axis check) leaves the `min.1` row unrefused. Both non-finite
    // minima reach `spade`'s triangulation as a NaN coordinate and refuse via a panic rather
    // than a clean `None` when their finiteness guard is the one dropped — the same failure
    // mode the guard exists to prevent, so the panic is stronger evidence than a clean
    // refusal, not weaker.
    let refuse = |min: (f64, f64)| WorldExtent {
        min,
        max: (10_000.0, 10_000.0),
    };
    assert!(build_navmesh(refuse((f64::NAN, 0.0)), 40.0, &[]).is_none());
    assert!(build_navmesh(refuse((0.0, f64::NAN)), 40.0, &[]).is_none());
    assert!(
        build_navmesh(refuse((20_000.0, 0.0)), 40.0, &[]).is_none(),
        "a minimum past the maximum is a non-positive span"
    );
    assert!(
        build_navmesh(refuse((-1e40, 0.0)), 40.0, &[]).is_none(),
        "an over-magnitude minimum saturates the f32 cast just as an over-magnitude maximum does"
    );
    assert!(
        build_navmesh(refuse((-1000.0, -1000.0)), 40.0, &[]).is_some(),
        "an ordinary negative minimum — what every hex block has — must build"
    );
}

#[test]
fn negative_or_non_finite_footprint_fails_closed() {
    assert!(build_navmesh(origin_extent(10_000.0, 10_000.0), -10.0, &[]).is_none());
    assert!(build_navmesh(origin_extent(10_000.0, 10_000.0), f64::NAN, &[]).is_none());
    assert!(build_navmesh(origin_extent(10_000.0, 10_000.0), f64::INFINITY, &[]).is_none());
}

#[test]
fn over_cap_obstacle_count_fails_closed() {
    let walls: Vec<Seg> = (0..(MAX_NAVMESH_OBSTACLE_SEGMENTS + 1))
        .map(|i| Seg {
            a: (i as f64, 0.0),
            b: (i as f64, 1.0),
        })
        .collect();
    assert!(build_navmesh(origin_extent(1_000_000.0, 10_000.0), 40.0, &walls).is_none());
}

#[test]
fn empty_scene_builds_a_navmesh() {
    assert!(build_navmesh(origin_extent(10_000.0, 10_000.0), 40.0, &[]).is_some());
}

#[test]
fn a_malformed_wall_segment_is_skipped_not_fatal() {
    let walls = vec![Seg {
        a: (f64::NAN, 0.0),
        b: (10.0, 10.0),
    }];
    assert!(build_navmesh(origin_extent(10_000.0, 10_000.0), 40.0, &walls).is_some());
}

#[test]
fn oversized_but_finite_extent_fails_closed_not_panic() {
    // `1e40` is finite and positive (passes the `is_finite`/`> 0.0` guards) but saturates an
    // `f64 -> f32` cast to infinity, which would otherwise reach
    // `polyanya::Triangulation::as_navmesh` and panic inside `spade`'s `.unwrap()`. The
    // magnitude that must be bounded is the extent this function RECEIVES; the bound on the
    // conversion that produces it is pinned at `SceneEcs::navmesh_for`.
    assert!(build_navmesh(origin_extent(1e40, 100.0), 0.4, &[]).is_none());
    assert!(build_navmesh(origin_extent(100.0, 1e40), 0.4, &[]).is_none());
}

#[test]
fn oversized_footprint_scene_fails_closed_not_panic() {
    // A tiny-but-finite extent stays well under `MAX_NAVMESH_COORD`, and the wall's raw
    // endpoints are ordinary, yet a footprint distance of `64.0 * 1e37` overflows `f32::MAX`
    // and would saturate the buffered ring's vertices to infinity on cast, panicking inside
    // `spade`.
    let walls = vec![Seg {
        a: (0.0, 0.0),
        b: (1.0, 1.0),
    }];
    assert!(build_navmesh(
        origin_extent(1e14, 1e14),
        crate::scene::pathfinding::MAX_FOOTPRINT_CELLS * 1e37,
        &walls,
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
    // An ordinary 10 × 10 domain, bisected by the wall (5,0)-(5,10); the footprint distance is
    // `MAX_FOOTPRINT_CELLS` (64.0, within the caller's cap) at a cell size of 1e9, giving
    // 6.4e10 — a ratio to the wall's own length (10) of 6.4e9, past the collapse threshold.
    let walls = vec![Seg {
        a: (5.0, 0.0),
        b: (5.0, 10.0),
    }];
    assert!(
        build_navmesh(
            origin_extent(10.0, 10.0),
            crate::scene::pathfinding::MAX_FOOTPRINT_CELLS * 1e9,
            &walls,
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
    assert!(build_navmesh(origin_extent(10_000.0, 10_000.0), 40.0, &walls).is_some());
}

#[test]
fn ordinary_wall_and_footprint_build_successfully() {
    // Positive case: normal gameplay magnitudes (a small scene, a modest footprint radius, a
    // single ordinary interior wall) must build a valid mesh, and the wall's obstacle
    // ring must actually be present in the resulting mesh (distinguishing "built successfully"
    // from "built successfully but the wall silently vanished").
    let walls = vec![Seg {
        a: (5.0, 0.0),
        b: (5.0, 10.0),
    }];
    let with_wall = build_navmesh(origin_extent(100.0, 100.0), 0.4, &walls)
        .expect("an ordinary wall + footprint must build a navmesh");
    let without_wall = build_navmesh(origin_extent(100.0, 100.0), 0.4, &[])
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
    let mesh = build_navmesh(origin_extent(10_000.0, 10_000.0), 40.0, &walls);
    assert!(
        mesh.is_some(),
        "the oversized segment must be skipped, not fail the whole build"
    );
}

#[test]
fn empty_waypoints_is_invalid() {
    let nav = build_navmesh(origin_extent(10_000.0, 10_000.0), 40.0, &[]).unwrap();
    let r = navmesh_find(&nav, (50.0, 50.0), &[]);
    assert_eq!(r, Err(crate::scene::pathfinding::PathFail::Invalid));
}

#[test]
fn straight_route_cost_is_euclidean() {
    let nav = build_navmesh(origin_extent(1000.0, 1000.0), 10.0, &[]).unwrap();
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
    let nav = build_navmesh(origin_extent(1000.0, 1000.0), 10.0, &walls).unwrap();
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
    let nav = build_navmesh(origin_extent(1000.0, 1000.0), 10.0, &[]).unwrap();
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
    let nav = build_navmesh(origin_extent(10_000.0, 10_000.0), 10.0, &[]).unwrap();
    let waypoints: Vec<(f64, f64)> = (0..(crate::scene::pathfinding::MAX_WAYPOINTS + 1))
        .map(|i| (i as f64, 0.0))
        .collect();
    let r = navmesh_find(&nav, (50.0, 50.0), &waypoints);
    assert_eq!(r, Err(crate::scene::pathfinding::PathFail::Invalid));
}

#[test]
fn non_finite_start_or_waypoint_is_invalid() {
    let nav = build_navmesh(origin_extent(10_000.0, 10_000.0), 10.0, &[]).unwrap();
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
    let nav = build_navmesh(origin_extent(10_000.0, 10_000.0), 10.0, &[]).unwrap();
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
    let clipped = clip_to_visible_mask(outcome.clone(), Some(&mask), 100.0, 0.1, &[], &test_grid());
    let last_orig = *outcome.path.last().unwrap();
    let last_clipped = *clipped.path.last().unwrap();
    assert!((last_orig.0 - last_clipped.0).abs() < 1e-6);
}

// "A goal outside the mask ⇒ Unreachable" must be an exercised code path, not only a
// reachable one; a mask
// that excludes the ENTIRE corridor beyond the start cell (as opposed to a partial-route
// truncation to a still-substantial prefix, per `clip_truncates_at_the_mask_boundary`)
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
        "the well-formed wall must truncate the route even alongside a malformed one, \
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
    // Expressed as a fraction of the hex's own size rather than as a distance: the clearance
    // the footprint disc ∪ traversal union buys is proportional to the geometry, so a literal
    // would silently become a weaker claim at a larger size.
    assert!(
        gap > HEX_SIZE * 0.8,
        "the route must stop clear of the occluded hex (footprint disc ∪ traversal), gap = {gap}"
    );

    let squared = clip_to_visible_mask(outcome, Some(&mask), HEX_SIZE, 0.1, &[], &sq_same_size());
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
    // The region rect is the target hex's own centre padded by half a size on each axis, so
    // it moves with the shape; the pad stays inside the hex's inradius (`√3/2·size`), which
    // is what keeps exactly one centre inside it, and the neighbour loop asserts that rather
    // than leaving it to the pad's arithmetic.
    let blocked = (2, -1);
    let ctr = g.cell_center(blocked);
    let pad = HEX_SIZE / 2.0;
    let field = hex_region(
        RegionBehavior::Impassable,
        ctr.0 - pad,
        ctr.1 - pad,
        ctr.0 + pad,
        ctr.1 + pad,
    );
    assert!(
        field.is_impassable(blocked),
        "fixture: the impassable cell is axial {blocked:?}"
    );
    for (n, _, _) in g.neighbors_with_cost(blocked, 0) {
        assert!(
            !field.is_impassable(n),
            "fixture: hex {n:?} neighbours the impassable hex and must stay clear"
        );
    }

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
    // Straight route along the r=0 hex row: hex (0,0) center -> hex (4,0) center, arresting on
    // hex (3,0). Square indexing reads the same axial key (3,0) as the square cell
    // `[3·size, 4·size)` — a DIFFERENT place on the map, short of the hex — so it cuts the
    // preview roughly a full hex early. Every coordinate here comes from the shape, so the two
    // arms' landing cells are compared rather than two hand-derived x thresholds, which at a
    // larger size would both stay satisfied by a cut a full hex early.
    let g = hexg();
    let arrest_cell = (3, 0);
    let arrest_ctr = g.cell_center(arrest_cell);
    let pad = HEX_SIZE / 2.0;
    let field = hex_region(
        RegionBehavior::Arrest,
        arrest_ctr.0 - pad,
        arrest_ctr.1 - pad,
        arrest_ctr.0 + pad,
        arrest_ctr.1 + pad,
    );
    assert!(
        field.is_arrest(arrest_cell),
        "fixture: arrest is on axial {arrest_cell:?}"
    );
    for (n, _, _) in g.neighbors_with_cost(arrest_cell, 0) {
        assert!(
            !field.is_arrest(n),
            "fixture: hex {n:?} neighbours the arrest hex and must stay clear"
        );
    }

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
        arrest_cell,
        "truncation lands ON the arrest hex's own cell, last = {last:?}"
    );
    // Arrest stops AT ENTRY, so the cut sits in the near half of the arrest hex — the one
    // claim about `last`'s position the cell assertion does not already imply, `cell_of`
    // being nearest-centre and therefore already bounding `last` to that hex's span.
    assert!(
        last.0 < arrest_ctr.0,
        "truncation is at the arrest hex's ENTRY boundary, not past its centre ({}), \
         last x = {}",
        arrest_ctr.0,
        last.0
    );

    let squared = truncate_at_arrest(route, &field, HEX_SIZE, &sq_same_size());
    let sq_last = *squared.path.last().unwrap();
    assert_ne!(
        g.cell_of(sq_last),
        arrest_cell,
        "square indexing cuts inside square cell {arrest_cell:?}, which is not even the \
         arrest HEX — a different location on the map, last x = {}",
        sq_last.0
    );
}
