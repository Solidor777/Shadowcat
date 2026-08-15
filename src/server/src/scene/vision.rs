//! Clean-room 2D visibility-polygon raycaster. Engine-owned geometry, server-authoritative.
//! No proprietary VTT/engine source consulted.
//!
//! Algorithm: the "ray casting to endpoints" angular sweep — for a viewpoint and a set of
//! occluding segments, cast rays toward every segment endpoint (and ±epsilon, to slip past
//! corners), take the nearest hit per ray, and order the hits by angle to form the visible
//! star-shaped polygon. Source: standard 2D visibility-polygon technique (Red Blob Games;
//! de Berg et al., *Computational Geometry*).

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

/// A point in scene coordinates.
pub type P = (f64, f64);

/// An occluding segment.
#[derive(Clone, Copy, PartialEq)]
pub struct Seg {
    /// First endpoint, scene units.
    pub a: P,
    /// Second endpoint, scene units.
    pub b: P,
}

/// An axis-aligned bound whose edges terminate rays that hit no wall.
#[derive(Clone, Copy)]
pub struct Rect {
    /// Left edge.
    pub minx: f64,
    /// Top edge.
    pub miny: f64,
    /// Right edge.
    pub maxx: f64,
    /// Bottom edge.
    pub maxy: f64,
}

/// Angular nudge (radians) cast on either side of each endpoint so a ray slips past the
/// corner to the geometry behind it (otherwise the polygon clips to the endpoint).
const EPS: f64 = 1e-4;

/// Normalize an angle into `[-π, π)` so the `±EPS` nudges near the `atan2` ±π seam sort into
/// true angular order (otherwise a nudged angle just past π lands at the wrong end of the
/// sorted list, producing a self-intersecting sliver at the -x axis).
fn wrap_angle(a: f64) -> f64 {
    use std::f64::consts::{PI, TAU};
    let mut a = a % TAU;
    if a < -PI {
        a += TAU;
    }
    if a >= PI {
        a -= TAU;
    }
    a
}

impl Rect {
    /// The four boundary edges as occluder segments (ray termination).
    fn edges(&self) -> [Seg; 4] {
        let tl = (self.minx, self.miny);
        let tr = (self.maxx, self.miny);
        let br = (self.maxx, self.maxy);
        let bl = (self.minx, self.maxy);
        [
            Seg { a: tl, b: tr },
            Seg { a: tr, b: br },
            Seg { a: br, b: bl },
            Seg { a: bl, b: tl },
        ]
    }
}

/// The bounding rect of `walls` + `viewpoint`, expanded by `margin` (so every ray terminates
/// on the box when it hits no wall). A wall-less scene yields a tiny box around the viewpoint —
/// callers computing vision for a specific scene should use `bound_for_scene` instead so a
/// wall-less (or near-wall-less) scene reveals its own full extent rather than this small box.
pub fn bound_for(viewpoint: P, walls: &[Seg], margin: f64) -> Rect {
    let mut minx = viewpoint.0;
    let mut miny = viewpoint.1;
    let mut maxx = viewpoint.0;
    let mut maxy = viewpoint.1;
    let mut grow = |p: P| {
        minx = minx.min(p.0);
        miny = miny.min(p.1);
        maxx = maxx.max(p.0);
        maxy = maxy.max(p.1);
    };
    for s in walls {
        grow(s.a);
        grow(s.b);
    }
    Rect {
        minx: minx - margin,
        miny: miny - margin,
        maxx: maxx + margin,
        maxy: maxy + margin,
    }
}

/// `bound_for`, UNIONED with a `reach`-radius square around `viewpoint` — never a replacement, so
/// the wall-endpoint growth `bound_for` already does still applies on top. `reach` is a WORLD-unit
/// distance (a caller converts an authored cell radius through `GridShape::world_units_per_cell`
/// before calling this); a non-finite or non-positive `reach` contributes nothing, leaving the
/// plain `margin` box `bound_for` already computes — the same fail-closed handling
/// `light_illumination` gives a degenerate radius, not an invented fallback distance. This is what
/// keeps a placed light's occlusion polygon growing to the light's OWN authored reach instead of
/// capping at `margin` regardless of how far the light was told to shine.
pub(crate) fn bound_for_reach(viewpoint: P, walls: &[Seg], margin: f64, reach: f64) -> Rect {
    let mut b = bound_for(viewpoint, walls, margin);
    if reach.is_finite() && reach > 0.0 {
        b.minx = b.minx.min(viewpoint.0 - reach);
        b.miny = b.miny.min(viewpoint.1 - reach);
        b.maxx = b.maxx.max(viewpoint.0 + reach);
        b.maxy = b.maxy.max(viewpoint.1 + reach);
    }
    b
}

/// `bound_for`, UNIONED with the scene's own world-unit envelope. `scene_extent` is in WORLD units
/// — a caller passes
/// `GridShape::world_extent` of the scene's authored bounds, never those raw bounds, which are
/// measured in grid units (cells), continuous, and would otherwise be compared here against wall
/// coordinates in world units. A wall-derived bound smaller than the scene's envelope is grown to
/// cover the whole scene instead, so a wall-less (or near-wall-less) scene reveals its own full
/// extent rather than a small `margin` box around the viewpoint. A wall-derived bound that already
/// exceeds the envelope on some edge (e.g. a
/// wall placed beyond the authored bounds) is left unchanged there: this only ever grows the bound.
///
/// Union, never replacement, on every edge — a degenerate envelope (the refused zero-area one) has
/// its `min` and `max` both at the origin, so it cannot SHRINK the wall-derived bound on any side.
/// It does not leave that bound untouched either: the low edges are still `min`ed against `(0.0,
/// 0.0)`, so a wall-derived bound sitting wholly clear of the origin is pulled back to it on both
/// low axes — the same clamp a square scene's own minimum applies, and the reason a refused
/// envelope is a widening of the bound rather than an absence from it.
pub(crate) fn bound_for_scene(
    viewpoint: P,
    walls: &[Seg],
    scene_extent: crate::scene::grid_shape::WorldExtent,
    margin: f64,
) -> Rect {
    let wall_bound = bound_for(viewpoint, walls, margin);
    Rect {
        minx: wall_bound.minx.min(scene_extent.min.0),
        miny: wall_bound.miny.min(scene_extent.min.1),
        maxx: wall_bound.maxx.max(scene_extent.max.0),
        maxy: wall_bound.maxy.max(scene_extent.max.1),
    }
}

/// `t >= 0` along the ray `origin + t*dir` where it first meets segment `s`, else `None`.
fn ray_segment(origin: P, dir: P, s: &Seg) -> Option<f64> {
    let (ox, oy) = origin;
    let (dx, dy) = dir;
    let sx = s.b.0 - s.a.0;
    let sy = s.b.1 - s.a.1;
    let denom = dx * sy - dy * sx;
    if denom.abs() < 1e-12 {
        return None; // parallel
    }
    let t = ((s.a.0 - ox) * sy - (s.a.1 - oy) * sx) / denom;
    let u = ((s.a.0 - ox) * dy - (s.a.1 - oy) * dx) / denom;
    if t >= 0.0 && (0.0..=1.0).contains(&u) {
        Some(t)
    } else {
        None
    }
}

/// Nearest occluder hit point along the ray, or `None` if it escapes (the bound box prevents
/// this in practice).
fn nearest_hit(origin: P, dir: P, segs: &[Seg]) -> Option<P> {
    let mut best: Option<f64> = None;
    for s in segs {
        if let Some(t) = ray_segment(origin, dir, s) {
            if best.is_none_or(|b| t < b) {
                best = Some(t);
            }
        }
    }
    best.map(|t| (origin.0 + t * dir.0, origin.1 + t * dir.1))
}

/// Even-odd ray-cast point-in-polygon. Source: standard CG (Shimrat 1962; de Berg et al.).
/// `poly` is a ring of vertices; `< 3` vertices ⇒ no area ⇒ false.
pub(crate) fn point_in_poly(poly: &[P], p: P) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let (px, py) = p;
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if ((yi > py) != (yj > py)) && (px < (xj - xi) * (py - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// The visibility polygon from `viewpoint`, occluded by `walls`, terminated by `bound`.
/// Vertices are in ascending-angle order (a star-shaped polygon around the viewpoint).
pub fn visibility_polygon(viewpoint: P, walls: &[Seg], bound: Rect) -> Vec<P> {
    let mut segs: Vec<Seg> = walls.to_vec();
    segs.extend(bound.edges());

    // Sample three angles per endpoint (θ, θ±ε) so rays slip past corners.
    let mut angles: Vec<f64> = Vec::with_capacity(segs.len() * 6);
    for s in &segs {
        for p in [s.a, s.b] {
            let ang = (p.1 - viewpoint.1).atan2(p.0 - viewpoint.0);
            angles.push(wrap_angle(ang - EPS));
            angles.push(ang);
            angles.push(wrap_angle(ang + EPS));
        }
    }
    angles.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let mut poly: Vec<P> = Vec::with_capacity(angles.len());
    for &ang in &angles {
        let dir = (ang.cos(), ang.sin());
        if let Some(hit) = nearest_hit(viewpoint, dir, &segs) {
            poly.push(hit);
        }
    }
    poly
}

/// Euclidean distance from point `p` to segment `a→b`, clamping the projection to the segment.
/// Source: standard point-to-segment projection (clean-room). Used by the pathfinder footprint
/// clearance: a footprint disc of radius R is wall-clear iff this distance ≥ R for every wall.
pub(crate) fn point_segment_distance(p: P, a: P, b: P) -> f64 {
    let (px, py) = p;
    let (ax, ay) = a;
    let (bx, by) = b;
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    // Geometry-scale threshold (grid/scene coordinates are unit-cell scale, not
    // near f64::EPSILON): a segment this short is degenerate at any cell size in use.
    let t = if len2 <= 1e-10 {
        0.0 // degenerate segment: distance to point `a`
    } else {
        (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
    };
    let (fx, fy) = (ax + t * dx, ay + t * dy);
    ((px - fx).powi(2) + (py - fy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
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
            "a refused envelope still pulls the low edges to the origin, got ({}, {})",
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
}
