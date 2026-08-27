use super::*;

fn bound() -> Rect {
    Rect {
        minx: -100.0,
        miny: -100.0,
        maxx: 100.0,
        maxy: 100.0,
    }
}

#[test]
fn point_segment_distance_degenerate_segment_uses_geometry_scale_epsilon() {
    // A segment with near-zero (but not exactly zero) length, below the plain
    // f64::EPSILON threshold but meaningfully non-degenerate at scene scale.
    let a = (0.0, 0.0);
    let b = (1e-9, 0.0); // len2 = 1e-18, well below both thresholds — still degenerate
    let point = (5.0, 0.0);
    let dist = point_segment_distance(point, a, b);
    assert!(
        (dist - 5.0).abs() < 1e-6,
        "a genuinely-degenerate segment collapses to point-distance from `a`"
    );
}

#[test]
fn open_scene_sees_the_whole_bound() {
    let poly = visibility_polygon((0.0, 0.0), &[], bound());
    assert!(poly.len() >= 4);
    assert!(point_in_poly(&poly, (50.0, 50.0)), "open region is visible");
    assert!(
        !point_in_poly(&poly, (200.0, 200.0)),
        "beyond the bound is not visible"
    );
}

#[test]
fn a_wall_casts_an_occlusion_shadow() {
    // Viewpoint at origin; a vertical wall at x=10 spanning y∈[-5,5] (subtends ±~26.6°).
    let wall = [Seg {
        a: (10.0, -5.0),
        b: (10.0, 5.0),
    }];
    let poly = visibility_polygon((0.0, 0.0), &wall, bound());
    assert!(
        point_in_poly(&poly, (5.0, 0.0)),
        "in front of the wall is visible"
    );
    assert!(
        !point_in_poly(&poly, (50.0, 0.0)),
        "directly behind the wall is occluded"
    );
    assert!(
        point_in_poly(&poly, (50.0, 60.0)),
        "around the wall (outside its cone) is visible"
    );
}

#[test]
fn enclosing_room_limits_vision_to_inside() {
    // A 4-wall room around the origin; a point outside a wall is not visible.
    let r = 20.0;
    let walls = [
        Seg {
            a: (-r, -r),
            b: (r, -r),
        },
        Seg {
            a: (r, -r),
            b: (r, r),
        },
        Seg {
            a: (r, r),
            b: (-r, r),
        },
        Seg {
            a: (-r, r),
            b: (-r, -r),
        },
    ];
    let poly = visibility_polygon((0.0, 0.0), &walls, bound());
    assert!(
        point_in_poly(&poly, (0.0, 0.0)),
        "inside the room is visible"
    );
    assert!(
        !point_in_poly(&poly, (50.0, 0.0)),
        "outside the room wall is occluded"
    );
}

#[test]
fn wall_straddling_the_minus_x_seam_has_no_spurious_hole() {
    // A wall crossing the -x axis from the viewpoint exercises the atan2 ±π seam where the
    // ±EPS nudges wrap. The shadow behind it must be occluded and the front visible, with
    // no sliver hole punched at the seam (which would leak occluded geometry).
    let wall = [Seg {
        a: (-10.0, -5.0),
        b: (-10.0, 5.0),
    }];
    let poly = visibility_polygon((0.0, 0.0), &wall, bound());
    assert!(
        point_in_poly(&poly, (-5.0, 0.0)),
        "in front of the seam-straddling wall is visible"
    );
    assert!(
        !point_in_poly(&poly, (-50.0, 0.0)),
        "behind the seam-straddling wall is occluded (no seam sliver)"
    );
}

#[test]
fn viewpoint_on_a_wall_endpoint_does_not_panic() {
    // Degenerate: the viewpoint coincides with a wall endpoint (atan2(0,0)=0). Must yield a
    // finite polygon (under-reveal is acceptable; a panic or NaN vertex is not).
    let wall = [Seg {
        a: (0.0, 0.0),
        b: (20.0, 0.0),
    }];
    let poly = visibility_polygon((0.0, 0.0), &wall, bound());
    assert!(poly.iter().all(|(x, y)| x.is_finite() && y.is_finite()));
}

#[test]
fn bound_for_expands_around_walls_and_viewpoint() {
    let walls = [Seg {
        a: (0.0, 0.0),
        b: (40.0, 0.0),
    }];
    let b = bound_for((10.0, 10.0), &walls, 5.0);
    assert!(b.minx <= -5.0 && b.maxx >= 45.0 && b.maxy >= 15.0);
}

#[test]
fn bound_for_scene_unions_the_envelopes_minimum_and_a_refused_one_cannot_shrink_it() {
    use crate::scene::grid_shape::{GridShape, HexGrid, REFUSED_EXTENT};
    // The scene's contribution to the bound is a UNION on all four edges, so the envelope's
    // own minimum grows the bound wherever it reaches past the wall-derived one. A hex block
    // large relative to the caller's margin is where that bites: its origin cell reaches a
    // full circumradius below the origin, past a viewpoint-derived bound that only extends by
    // the margin.
    //
    // Discrimination, per assertion, each naming a mutation that breaks THAT assertion while
    // the others still hold:
    // - the low-edge assertions fail if the minimum is clamped to zero instead of unioned,
    //   and the fixture guard on the envelope's reach fails first if it stops reaching past
    //   the margin, so neither can pass vacuously.
    // - the high-edge assertion fails if the maximum stops being unioned, which the low edges
    //   cannot detect.
    // - the refused-envelope assertions fail if the union becomes a replacement, which the
    //   real-envelope assertions still tolerate, their corners being the outer ones anyway.
    // - the far-from-the-origin refused arm fails if the union starts SKIPPING a zero-area
    //   envelope, which every other assertion here tolerates.
    let g = HexGrid { size: 400.0 };
    let envelope = g.world_extent((3.0, 3.0));
    let margin = 100.0;
    let vp = (0.0, 0.0);
    assert!(
        envelope.min.0 < vp.0 - margin && envelope.min.1 < vp.1 - margin,
        "fixture: the envelope's minimum {:?} must reach past the margin box around {vp:?}",
        envelope.min
    );
    let b = bound_for_scene(vp, &[], envelope, margin);
    assert!(
        (b.minx - envelope.min.0).abs() < 1e-9 && (b.miny - envelope.min.1).abs() < 1e-9,
        "the bound's low edges must be the envelope's own minimum, got ({}, {})",
        b.minx,
        b.miny
    );
    assert!(
        (b.maxx - envelope.max.0).abs() < 1e-9 && (b.maxy - envelope.max.1).abs() < 1e-9,
        "the bound's high edges must be the envelope's own maximum, got ({}, {})",
        b.maxx,
        b.maxy
    );
    // A refused (zero-area) envelope cannot SHRINK the bound: it is unioned like any other,
    // never substituted for the wall-derived one. Its corners are the ORIGIN, though, so it is
    // not inert — which takes two arms to state, because a wall-derived bound that already
    // spans the origin cannot tell "untouched" from "clamped to the origin".
    let walls = [Seg {
        a: (-500.0, -500.0),
        b: (500.0, 500.0),
    }];
    let wall_only = bound_for(vp, &walls, margin);
    assert!(
        wall_only.minx < 0.0 && wall_only.miny < 0.0,
        "fixture: these walls must already span the origin, got ({}, {})",
        wall_only.minx,
        wall_only.miny
    );
    let refused = bound_for_scene(vp, &walls, REFUSED_EXTENT, margin);
    assert!(
        refused.minx == wall_only.minx
            && refused.miny == wall_only.miny
            && refused.maxx == wall_only.maxx
            && refused.maxy == wall_only.maxy,
        "a refused envelope must leave a bound that already spans the origin untouched"
    );
    // The other side of the same union: a wall-derived bound sitting wholly clear of the
    // origin IS moved, its low edges landing on the refused envelope's own corners rather than
    // on the wall-derived values. This is the arm a union that skipped a zero-area envelope
    // would fail while every other assertion here still passed.
    let far_walls = [Seg {
        a: (1000.0, 1000.0),
        b: (2000.0, 2000.0),
    }];
    let far_vp = (1500.0, 1500.0);
    let far_wall_only = bound_for(far_vp, &far_walls, margin);
    assert!(
        far_wall_only.minx > 0.0 && far_wall_only.miny > 0.0,
        "fixture: these walls must sit clear of the origin, got ({}, {})",
        far_wall_only.minx,
        far_wall_only.miny
    );
    let far_refused = bound_for_scene(far_vp, &far_walls, REFUSED_EXTENT, margin);
    assert!(
        far_refused.minx == 0.0 && far_refused.miny == 0.0,
        "a refused envelope pulls the low edges to the origin all the same, got ({}, {})",
        far_refused.minx,
        far_refused.miny
    );
}

#[test]
fn point_segment_distance_endpoints_midpoint_and_perpendicular() {
    let a = (0.0, 0.0);
    let b = (10.0, 0.0);
    // Perpendicular foot inside the segment.
    assert!((point_segment_distance((5.0, 3.0), a, b) - 3.0).abs() < 1e-9);
    // Beyond an endpoint clamps to that endpoint.
    assert!((point_segment_distance((-4.0, 0.0), a, b) - 4.0).abs() < 1e-9);
    // On the segment → 0.
    assert!(point_segment_distance((7.0, 0.0), a, b) < 1e-9);
    // Degenerate segment (a == b) → distance to the point.
    assert!((point_segment_distance((3.0, 4.0), (0.0, 0.0), (0.0, 0.0)) - 5.0).abs() < 1e-9);
}
