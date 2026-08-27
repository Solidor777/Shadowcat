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
mod tests;
