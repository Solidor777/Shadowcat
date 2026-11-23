use super::*;

fn bound() -> Rect {
    Rect {
        minx: -100.0,
        miny: -100.0,
        maxx: 100.0,
        maxy: 100.0,
    }
}

/// An ordinary (two-sided, immediate) wall segment: no one-way sign, no limited tier.
fn wall_seg(a: P, b: P) -> Seg {
    Seg {
        a,
        b,
        one_way: None,
        limited: false,
        open_within: None,
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
    let wall = [wall_seg((10.0, -5.0), (10.0, 5.0))];
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
fn a_wall_entirely_outside_bound_does_not_affect_the_polygon() {
    // Same occluding wall as `a_wall_casts_an_occlusion_shadow`, plus a second wall whose whole
    // bbox lies past `bound`'s edge (bound spans ±100) — the pre-filter must drop it, and the
    // polygon must be byte-for-byte identical to the no-extra-wall case.
    let near_wall = wall_seg((10.0, -5.0), (10.0, 5.0));
    let outside_wall = wall_seg((500.0, -5.0), (500.0, 5.0));
    let baseline = visibility_polygon((0.0, 0.0), &[near_wall], bound());
    let with_outside_wall = visibility_polygon((0.0, 0.0), &[near_wall, outside_wall], bound());
    assert_eq!(
        baseline, with_outside_wall,
        "a wall entirely outside bound must not change a single vertex"
    );
}

#[test]
fn a_wall_straddling_bounds_edge_still_cuts_the_polygon() {
    // A wall spanning x=[95,105] at y=-5: `bound()`'s maxx=100 cuts it at x=100, so half the
    // wall (x∈[100,105]) lies outside `bound` while the other half (x∈[95,100]) is inside. The
    // pre-filter must keep this wall (its bbox overlaps `bound`), so the in-bound portion still
    // occludes what's directly behind it.
    let straddling_wall = wall_seg((95.0, -5.0), (105.0, -5.0));
    let poly = visibility_polygon((0.0, 0.0), &[straddling_wall], bound());
    assert!(
        point_in_poly(&poly, (0.0, 50.0)),
        "off to the side of the straddling wall's narrow occlusion cone is still visible"
    );
    // (97, -5) sits on the wall itself (t=1 along the ray from the origin through it);
    // (98.94, -5.1) = 1.02 * (97, -5) is further along the SAME ray, still inside `bound`
    // (maxx=100), and therefore occluded by the nearer wall intersection at t=1.
    assert!(
        !point_in_poly(&poly, (98.94, -5.1)),
        "directly behind the straddling wall's in-bound portion is occluded"
    );
}

#[test]
fn enclosing_room_limits_vision_to_inside() {
    // A 4-wall room around the origin; a point outside a wall is not visible.
    let r = 20.0;
    let walls = [
        wall_seg((-r, -r), (r, -r)),
        wall_seg((r, -r), (r, r)),
        wall_seg((r, r), (-r, r)),
        wall_seg((-r, r), (-r, -r)),
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
    let wall = [wall_seg((-10.0, -5.0), (-10.0, 5.0))];
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
    let wall = [wall_seg((0.0, 0.0), (20.0, 0.0))];
    let poly = visibility_polygon((0.0, 0.0), &wall, bound());
    assert!(poly.iter().all(|(x, y)| x.is_finite() && y.is_finite()));
}

#[test]
fn bound_for_expands_around_walls_and_viewpoint() {
    let walls = [wall_seg((0.0, 0.0), (40.0, 0.0))];
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
    let envelope = g.world_extent(crate::scene::SceneBounds {
        cells: (3.0, 3.0),
        padding_cells: 0.0,
    });
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
    let walls = [wall_seg((-500.0, -500.0), (500.0, 500.0))];
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
    let far_walls = [wall_seg((1000.0, 1000.0), (2000.0, 2000.0))];
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

// --- One-way walls + the limited sight tier (`Seg::one_way`/`Seg::limited`) ---
//
// Shared geometry for the battery: a vertical wall on the y-axis, `a=(0,-10)`,
// `b=(0,10)`. The cross product `(b - a) × (p - a)` evaluates to `-20 * p.x`, so with
// `one_way: Some(1.0)` the BLOCKING side is `x < 0` (cross positive) and the transparent
// side is `x > 0`.

/// The fixture one-way wall of this battery: solid from `x < 0`, transparent from `x > 0`.
fn one_way_wall() -> Seg {
    Seg {
        a: (0.0, -10.0),
        b: (0.0, 10.0),
        one_way: Some(1.0),
        limited: false,
        open_within: None,
    }
}

#[test]
fn nearest_hit_treats_one_way_wall_as_transparent_from_its_open_side() {
    let bound = bound();
    let mut segs = vec![one_way_wall()];
    segs.extend(bound.edges());
    // A ray from the transparent side (x > 0) aimed at the wall must NOT terminate there:
    // it crosses the wall and ends on the far bound edge.
    let hit =
        nearest_hit((5.0, 0.0), (-1.0, 0.0), &segs).expect("the bound always terminates a ray");
    assert!(
        (hit.0 - -100.0).abs() < 1e-9 && hit.1.abs() < 1e-9,
        "transparent from the open side: the ray must reach the bound at (-100, 0), got {hit:?}"
    );
}

#[test]
fn nearest_hit_treats_one_way_wall_as_blocking_from_its_solid_side() {
    let bound = bound();
    let mut segs = vec![one_way_wall()];
    segs.extend(bound.edges());
    // A ray from the blocking side (x < 0) aimed at the wall terminates ON the wall.
    let hit = nearest_hit((-5.0, 0.0), (1.0, 0.0), &segs).expect("the wall terminates the ray");
    assert!(
        hit.0.abs() < 1e-9 && hit.1.abs() < 1e-9,
        "blocking from the solid side: the ray must stop at the wall (0, 0), got {hit:?}"
    );
}

#[test]
fn nearest_hit_degenerate_cross_product_fails_closed_to_blocking() {
    // Origin exactly ON the wall's line: the cross product is 0, an ambiguous side that must
    // read as BLOCKING — the same reading as a wall with no one-way sign — never transparent.
    let wall = one_way_wall();
    assert!(
        !wall.transparent_from((0.0, 5.0)),
        "an origin on the wall's own line must fail closed to blocking"
    );
    let hit = nearest_hit((0.0, 5.0), (1.0, 0.0), &[wall])
        .expect("a blocking wall terminates the ray at t=0");
    assert!(
        hit.0.abs() < 1e-9 && (hit.1 - 5.0).abs() < 1e-9,
        "the degenerate side case must terminate ON the wall, got {hit:?}"
    );
}

#[test]
fn transparent_from_fails_closed_on_a_non_unit_sign_or_non_finite_cross() {
    // `WallEngine::validate` keeps a non-unit sign out at ingress; the runtime predicate must
    // still fail CLOSED (blocking) if one ever reaches it — `cross.signum() != 0.0` would
    // otherwise read a stored 0.0 as transparent from both sides.
    let mut wall = one_way_wall();
    wall.one_way = Some(0.0);
    assert!(!wall.transparent_from((-5.0, 0.0)));
    assert!(!wall.transparent_from((5.0, 0.0)));
    // A non-finite origin (a NaN cross product) likewise blocks.
    assert!(!one_way_wall().transparent_from((f64::NAN, 0.0)));
}

#[test]
fn nearest_hit_limited_wall_lets_a_ray_cross_exactly_one_then_stops_at_a_second() {
    // Two parallel limited walls at x=0 and x=10, approached from x<0: the ray spends its
    // single crossing budget on the first and the second stops it.
    let first = Seg {
        a: (0.0, -10.0),
        b: (0.0, 10.0),
        one_way: None,
        limited: true,
        open_within: None,
    };
    let second = Seg {
        a: (10.0, -10.0),
        b: (10.0, 10.0),
        one_way: None,
        limited: true,
        open_within: None,
    };
    let hit = nearest_hit((-5.0, 0.0), (1.0, 0.0), &[first, second])
        .expect("the second limited wall terminates the ray");
    assert!(
        (hit.0 - 10.0).abs() < 1e-9 && hit.1.abs() < 1e-9,
        "one crossing is free, the second limited wall stops the ray at (10, 0), got {hit:?}"
    );
    // A single limited wall alone is fully see-through: the ray reaches the bound.
    let bound = bound();
    let mut segs = vec![first];
    segs.extend(bound.edges());
    let through = nearest_hit((-5.0, 0.0), (1.0, 0.0), &segs).expect("the bound terminates");
    assert!(
        (through.0 - 100.0).abs() < 1e-9 && through.1.abs() < 1e-9,
        "one limited wall is crossed outright: the ray must reach the bound at (100, 0), got {through:?}"
    );
}

#[test]
fn nearest_hit_limited_wall_still_stops_at_an_ordinary_wall_after_one_limited_crossing() {
    // A limited wall followed by an ORDINARY wall: the ordinary wall stops the ray (the
    // budget does not apply to it — it was never eligible for skipping in the first place).
    let limited = Seg {
        a: (0.0, -10.0),
        b: (0.0, 10.0),
        one_way: None,
        limited: true,
        open_within: None,
    };
    let ordinary = wall_seg((10.0, -10.0), (10.0, 10.0));
    let hit = nearest_hit((-5.0, 0.0), (1.0, 0.0), &[limited, ordinary])
        .expect("the ordinary wall terminates the ray");
    assert!(
        (hit.0 - 10.0).abs() < 1e-9 && hit.1.abs() < 1e-9,
        "an ordinary wall stops the ray after one limited crossing, got {hit:?}"
    );
    // Ordinary FIRST: the ray stops immediately and the limited wall behind it is irrelevant.
    let limited_behind = Seg {
        a: (10.0, -10.0),
        b: (10.0, 10.0),
        one_way: None,
        limited: true,
        open_within: None,
    };
    let ordinary_first = wall_seg((0.0, -10.0), (0.0, 10.0));
    let hit = nearest_hit((-5.0, 0.0), (1.0, 0.0), &[ordinary_first, limited_behind])
        .expect("the ordinary wall terminates the ray");
    assert!(
        hit.0.abs() < 1e-9 && hit.1.abs() < 1e-9,
        "an ordinary wall in front stops the ray at (0, 0), got {hit:?}"
    );
}

#[test]
fn visibility_polygon_bound_edges_are_never_one_way_or_limited() {
    // Positive control: `Rect::edges` terminate every ray unconditionally, so a bound edge
    // must never carry either skip behavior — a ray that escapes past every wall still stops
    // at the bound (this is what makes `nearest_hit`'s walk total).
    for edge in bound().edges() {
        assert_eq!(edge.one_way, None, "bound edges are never one-way");
        assert!(!edge.limited, "bound edges are never limited");
        assert_eq!(
            edge.open_within, None,
            "bound edges never open by proximity"
        );
    }
}

/// A vertical wall at x = 10 spanning y in [-5, 5] that opens within `distance` of an origin.
fn proximity_wall(distance: f64) -> Seg {
    Seg {
        a: (10.0, -5.0),
        b: (10.0, 5.0),
        one_way: None,
        limited: false,
        open_within: Some(OpenWithin {
            distance,
            gain: 1.0,
        }),
    }
}

#[test]
fn a_viewer_near_a_proximity_wall_sees_behind_it_and_a_far_viewer_does_not() {
    // Within 2 units the wall is absent from the viewpoint, so the cell behind it is visible;
    // from 3 units the same wall is ordinary and hides it. The control wall carries no tier and
    // hides the cell from BOTH distances, so the near case fails without the feature.
    let target = (30.0, 0.0);
    let tiered = [proximity_wall(2.0)];
    let plain = [wall_seg((10.0, -5.0), (10.0, 5.0))];
    let near = (9.0, 0.0);
    let far = (7.0, 0.0);
    assert!(point_in_poly(
        &visibility_polygon(near, &tiered, bound()),
        target
    ));
    assert!(!point_in_poly(
        &visibility_polygon(far, &tiered, bound()),
        target
    ));
    assert!(!point_in_poly(
        &visibility_polygon(near, &plain, bound()),
        target
    ));
    assert!(!point_in_poly(
        &visibility_polygon(far, &plain, bound()),
        target
    ));
}

#[test]
fn a_light_near_a_proximity_wall_lights_behind_it_and_a_far_light_does_not() {
    // The light raycast (`emitters::light_polygon`) reaches the same `nearest_hit` skip: a light
    // within the tier distance shines through, one beyond it is shadowed. The same wall with no
    // tier shadows both.
    let target = (30.0, 0.0);
    let tiered = [proximity_wall(2.0)];
    let plain = [wall_seg((10.0, -5.0), (10.0, 5.0))];
    let lit = |pos: P, walls: &[Seg]| {
        point_in_poly(
            &crate::scene::emitters::light_polygon(pos, walls, 100.0),
            target,
        )
    };
    assert!(lit((9.0, 0.0), &tiered));
    assert!(!lit((7.0, 0.0), &tiered));
    assert!(!lit((9.0, 0.0), &plain));
}

#[test]
fn open_from_is_inclusive_at_the_distance_and_closed_just_beyond() {
    let wall = proximity_wall(2.0);
    assert!(wall.open_from((8.0, 0.0)), "exactly at the distance opens");
    assert!(
        !wall.open_from((7.999, 0.0)),
        "just beyond the distance blocks"
    );
}

#[test]
fn open_from_measures_to_the_segment_not_its_line() {
    // Perpendicular distance to the infinite line is 1, but the nearest SEGMENT point is the
    // endpoint (10, 5), 4 units from (9, 9): the wall stays closed.
    let wall = proximity_wall(2.0);
    assert!(!wall.open_from((9.0, 9.0)));
    assert!(wall.open_from((9.0, 6.0)));
}

#[test]
fn open_from_fails_closed_on_bad_distance_or_origin() {
    for distance in [f64::NAN, f64::INFINITY, 0.0, -1.0] {
        assert!(
            !proximity_wall(distance).open_from((10.0, 0.0)),
            "distance {distance} must leave the wall closed"
        );
    }
    assert!(!proximity_wall(2.0).open_from((f64::NAN, 0.0)));
    assert!(!wall_seg((10.0, -5.0), (10.0, 5.0)).open_from((10.0, 0.0)));
}

#[test]
fn sight_light_and_sound_agree_on_one_proximity_wall_and_origin() {
    // Parity pin: the sight raycast, the light raycast (both `nearest_hit`) and the sound test
    // (`audibility::segment_occluded`) read the SAME `Seg::open_from`. For every origin the
    // wall is either open to all three or closed to all three. Mutating `open_from` from `<=`
    // to `<` flips the boundary row and fails this test.
    let wall = proximity_wall(2.0);
    let emitter = (30.0, 0.0);
    for (origin, open) in [
        ((9.0, 0.0), true),
        ((8.0, 0.0), true),
        ((7.5, 0.0), false),
        ((5.0, 0.0), false),
    ] {
        let mut segs = vec![wall];
        segs.extend(bound().edges());
        let ray_open =
            nearest_hit(origin, (1.0, 0.0), &segs).is_some_and(|hit| (hit.0 - 100.0).abs() < 1e-9);
        let (blocked, _) = crate::scene::audibility::segment_occluded(origin, emitter, &[wall]);
        assert_eq!(ray_open, open, "raycast verdict at {origin:?}");
        assert_eq!(!blocked, open, "sound verdict at {origin:?}");
        assert_eq!(
            wall.open_from(origin),
            open,
            "predicate verdict at {origin:?}"
        );
    }
}

/// A facing arc of `arc_deg` degrees centred on `facing_deg`, with an all-round bubble of
/// `near` world units.
fn arc(facing_deg: f64, arc_deg: f64, near: f64) -> FacingArc {
    FacingArc {
        facing_rad: facing_deg.to_radians(),
        half_rad: (arc_deg / 2.0).to_radians(),
        near_world: near,
    }
}

#[test]
fn facing_polygon_keeps_the_arc_and_drops_what_is_behind_and_beside() {
    let poly = visibility_polygon_facing((0.0, 0.0), &[], bound(), arc(0.0, 90.0, 0.0));
    assert!(point_in_poly(&poly, (50.0, 0.0)), "straight ahead is seen");
    assert!(point_in_poly(&poly, (50.0, 20.0)), "inside the arc is seen");
    assert!(!point_in_poly(&poly, (-50.0, 0.0)), "behind is not seen");
    assert!(
        !point_in_poly(&poly, (0.0, 50.0)),
        "beside, outside the 45-degree half angle, is not seen"
    );
    assert!(!point_in_poly(&poly, (0.0, -50.0)));
}

#[test]
fn facing_polygon_bubble_is_all_round_and_bounded() {
    let poly = visibility_polygon_facing((0.0, 0.0), &[], bound(), arc(0.0, 90.0, 20.0));
    assert!(
        point_in_poly(&poly, (-10.0, 0.0)),
        "inside the bubble, behind"
    );
    assert!(
        point_in_poly(&poly, (0.0, 10.0)),
        "inside the bubble, beside"
    );
    assert!(
        !point_in_poly(&poly, (-30.0, 0.0)),
        "past the bubble, behind"
    );
    assert!(
        point_in_poly(&poly, (60.0, 0.0)),
        "the arc still runs far past the bubble"
    );
}

#[test]
fn facing_polygon_arc_spanning_the_pi_seam_includes_the_forward_points() {
    let poly = visibility_polygon_facing((0.0, 0.0), &[], bound(), arc(180.0, 90.0, 0.0));
    assert!(point_in_poly(&poly, (-50.0, 0.0)), "facing -x sees -x");
    assert!(point_in_poly(&poly, (-50.0, 20.0)));
    assert!(
        point_in_poly(&poly, (-50.0, -20.0)),
        "both sides of the seam"
    );
    assert!(
        !point_in_poly(&poly, (50.0, 0.0)),
        "behind a -x facing is +x"
    );
}

#[test]
fn facing_polygon_walls_still_occlude_inside_the_arc() {
    let wall = wall_seg((30.0, -100.0), (30.0, 100.0));
    let poly = visibility_polygon_facing((0.0, 0.0), &[wall], bound(), arc(0.0, 90.0, 0.0));
    assert!(point_in_poly(&poly, (20.0, 0.0)), "in front of the wall");
    assert!(!point_in_poly(&poly, (50.0, 0.0)), "behind the wall");
}

#[test]
fn facing_polygon_bubble_stops_at_a_wall_closer_than_its_radius() {
    // The wall sits at x = -5 behind the viewer; the bubble radius is 20. The ray cap is the
    // nearer of the two, so the bubble never reaches through the wall.
    let wall = wall_seg((-5.0, -100.0), (-5.0, 100.0));
    let poly = visibility_polygon_facing((0.0, 0.0), &[wall], bound(), arc(0.0, 90.0, 20.0));
    assert!(point_in_poly(&poly, (-3.0, 0.0)));
    assert!(!point_in_poly(&poly, (-10.0, 0.0)));
}

#[test]
fn facing_polygon_full_circle_arc_matches_the_unrestricted_polygon() {
    // `half_rad` just under pi is the widest representable arc: the only rays cut are the single
    // one exactly opposite the facing, which the bubble cap then bounds.
    let plain = visibility_polygon((0.0, 0.0), &[], bound());
    let wide = visibility_polygon_facing((0.0, 0.0), &[], bound(), arc(0.0, 359.0, 30.0));
    for p in [
        (90.0, 90.0),
        (-90.0, 90.0),
        (-90.0, -90.0),
        (90.0, -90.0),
        (0.0, 90.0),
    ] {
        assert_eq!(point_in_poly(&plain, p), point_in_poly(&wide, p), "{p:?}");
    }
}
