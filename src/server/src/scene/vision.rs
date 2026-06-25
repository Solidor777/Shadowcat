//! Clean-room 2D visibility-polygon raycaster (M9b). Engine-owned geometry (#6 exception),
//! server-authoritative (#3). No proprietary VTT/engine source consulted.
//!
//! Algorithm: the "ray casting to endpoints" angular sweep — for a viewpoint and a set of
//! occluding segments, cast rays toward every segment endpoint (and ±epsilon, to slip past
//! corners), take the nearest hit per ray, and order the hits by angle to form the visible
//! star-shaped polygon. Source: standard 2D visibility-polygon technique (Red Blob Games;
//! de Berg et al., *Computational Geometry*).

/// A point in scene coordinates.
pub type P = (f64, f64);

/// An occluding segment.
#[derive(Clone, Copy)]
pub struct Seg {
    pub a: P,
    pub b: P,
}

/// An axis-aligned bound whose edges terminate rays that hit no wall.
#[derive(Clone, Copy)]
pub struct Rect {
    pub minx: f64,
    pub miny: f64,
    pub maxx: f64,
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
/// on the box when it hits no wall). A wall-less scene yields a tiny box around the viewpoint.
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
#[allow(dead_code)] // TODO: remove once the grid pathfinder calls this
pub(crate) fn point_segment_distance(p: P, a: P, b: P) -> f64 {
    let (px, py) = p;
    let (ax, ay) = a;
    let (bx, by) = b;
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= f64::EPSILON {
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
