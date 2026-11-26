//! This module is a clean-room 2D visibility-polygon raycaster.
//!
//! It is engine-owned geometry, server-authoritative and clean-room: no proprietary VTT/engine
//! source informs it.
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
///
/// `one_way`/`limited` are the per-segment occlusion metadata `nearest_hit` consumes (the
/// movement gate shares the `one_way` half through `Seg::transparent_from`): `one_way:
/// Some(sign)` makes the wall solid only from the side where `(b - a) × (p - a)` carries the
/// sign `sign` and transparent from the other (a one-way arrow slit); `limited: true` lets a
/// sight ray cross this wall while the ray's `MAX_LIMITED_CROSSINGS` budget lasts ("see
/// through one such wall, not two"). `open_within: Some(..)` makes the wall absent for a ray or
/// sound path whose origin lies within the carried distance of it (`Seg::open_from`). All three
/// stay `None`/`false` for an ordinary wall and for every bound-box edge (`Rect::edges`), and
/// every construction site spells them out — never a `..` rest-default — so a forgotten
/// populate site fails to compile rather than silently reverting to symmetric.
///
/// # Examples
///
/// ```
/// let wall = shadowcat::scene::vision::Seg {
///     a: (0.0, 0.0),
///     b: (1.0, 0.0),
///     one_way: None,
///     limited: false,
///     open_within: None,
/// };
/// assert_eq!(wall.a, (0.0, 0.0));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seg {
    /// First endpoint, scene units.
    pub a: P,
    /// Second endpoint, scene units.
    pub b: P,
    /// One-way sign: `Some(s)` makes the wall solid only from the side of the segment where
    /// `(b - a) × (p - a)` has sign `s` (`s` is validated to `±1.0` at document ingress); the
    /// wall is skipped entirely for a ray/step whose origin sits on the other side. `None` =
    /// solid from both sides. Populated by the sight/movement collectors from
    /// `WallEngine::direction`; the light collectors always pass `None` (light occlusion is
    /// symmetric by design).
    pub one_way: Option<f64>,
    /// `true` = a sight ray may cross this wall once per ray while the
    /// `MAX_LIMITED_CROSSINGS` budget lasts. Populated by the sight collector from
    /// `WallEngine::sight_tier`; movement and light collectors always pass `false`.
    pub limited: bool,
    /// Proximity opening for the sense this segment was collected for: `Some` = the wall is
    /// absent for an origin within `OpenWithin::distance` of it. Populated by each sense's
    /// collector from `WallEngine::proximity` only when the tier names that sense; the
    /// movement collector and every bound edge pass `None`.
    pub open_within: Option<OpenWithin>,
}

/// A wall's proximity opening, in scene units, for one sense (`Seg::open_within`).
///
/// # Examples
///
/// ```
/// let open = shadowcat::scene::vision::OpenWithin { distance: 2.0, gain: 0.5 };
/// assert_eq!(open.gain, 0.5);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpenWithin {
    /// Origin-to-segment distance at or under which the wall is open, scene units.
    pub distance: f64,
    /// Gain a sound keeps crossing the open wall (`WallProximity::sound_gain`, `1.0` when
    /// absent). Read by `audibility::segment_occluded` only; sight and light ignore it.
    pub gain: f64,
}

impl Seg {
    /// Whether this segment is open to a ray or sound path whose ORIGIN is `from`: `open_within`
    /// is set and `from` lies within its distance of the segment (`point_segment_distance`,
    /// inclusive). The one proximity predicate sight (`nearest_hit` over the sight collector),
    /// light (the same `nearest_hit` over the light collector) and sound
    /// (`audibility::segment_occluded`) share, so the three senses cannot disagree about a
    /// given wall and origin. Fail-closed to `false` (the wall blocks) on a non-finite or
    /// non-positive distance or a non-finite `from`.
    pub(crate) fn open_from(&self, from: P) -> bool {
        let Some(open) = self.open_within else {
            return false;
        };
        if !open.distance.is_finite() || open.distance <= 0.0 {
            return false;
        }
        if !from.0.is_finite() || !from.1.is_finite() {
            return false;
        }
        point_segment_distance(from, self.a, self.b) <= open.distance
    }

    /// Whether this segment is transparent to a sight ray or movement step whose ORIGIN is
    /// `from` — the one cross-product side test every one-way consumer (`nearest_hit`,
    /// `move_exec`'s per-step wall filter, `pathfinding::cell_enterable`) shares, so vision
    /// and movement can never disagree about which side of a wall is solid. `true` iff
    /// `one_way` is set AND `from` lies strictly on the non-blocking side (the cross product
    /// `(b - a) × (from - a)` disagrees in sign with `one_way`). Fail-closed to `false`
    /// (blocking) on the degenerate cases: a zero cross product puts `from` exactly ON the
    /// wall's line (an ambiguous side must never open a sightline), and a non-finite cross
    /// product would make every comparison false.
    pub(crate) fn transparent_from(&self, from: P) -> bool {
        let Some(sign) = self.one_way else {
            return false;
        };
        let cross = (self.b.0 - self.a.0) * (from.1 - self.a.1)
            - (self.b.1 - self.a.1) * (from.0 - self.a.0);
        if cross == 0.0 || !cross.is_finite() {
            return false;
        }
        // `sign` is ingress-validated to ±1.0 (`WallEngine::validate`); a stray non-unit
        // value must fail CLOSED here too — `cross.signum() != 0.0` would otherwise read a
        // stored `0.0` as transparent from BOTH sides.
        if sign != 1.0 && sign != -1.0 {
            return false;
        }
        cross.signum() != sign
    }
}

/// An axis-aligned bound whose edges terminate rays that hit no wall.
///
/// # Examples
///
/// ```
/// let bound = shadowcat::scene::vision::Rect { minx: -5.0, miny: -5.0, maxx: 5.0, maxy: 5.0 };
/// assert!(bound.maxx > bound.minx);
/// ```
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
    /// The four boundary edges as occluder segments (ray termination). Bound edges are
    /// always symmetric and immediate (`one_way: None`, `limited: false`) — a ray must
    /// terminate at the bound regardless of any wall's authored one-way/limited behavior.
    fn edges(&self) -> [Seg; 4] {
        let tl = (self.minx, self.miny);
        let tr = (self.maxx, self.miny);
        let br = (self.maxx, self.maxy);
        let bl = (self.minx, self.maxy);
        [
            Seg {
                a: tl,
                b: tr,
                one_way: None,
                limited: false,
                open_within: None,
            },
            Seg {
                a: tr,
                b: br,
                one_way: None,
                limited: false,
                open_within: None,
            },
            Seg {
                a: br,
                b: bl,
                one_way: None,
                limited: false,
                open_within: None,
            },
            Seg {
                a: bl,
                b: tl,
                one_way: None,
                limited: false,
                open_within: None,
            },
        ]
    }
}

/// The bounding rect of `walls` + `viewpoint`, expanded by `margin` (so every ray terminates
/// on the box when it hits no wall). A wall-less scene yields a tiny box around the viewpoint —
/// callers computing vision for a specific scene should use `bound_for_scene` instead so a
/// wall-less (or near-wall-less) scene reveals its own full extent rather than this small box.
///
/// # Examples
///
/// ```
/// use shadowcat::scene::vision::{bound_for, Seg};
///
/// let wall = Seg { a: (2.0, 0.0), b: (2.0, 4.0), one_way: None, limited: false, open_within: None };
/// let bound = bound_for((0.0, 0.0), &[wall], 1.0);
/// assert_eq!(bound.maxx, 3.0);
/// assert_eq!(bound.miny, -1.0);
/// ```
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

/// Per-ray allowance of `limited`-wall crossings: a sight ray may pass ONE such wall and
/// stops at the next crossing of any wall ("see through one such wall, not two"). A
/// compile-time constant, never user-configurable.
pub(crate) const MAX_LIMITED_CROSSINGS: u8 = 1;

/// Nearest occluder hit point along the ray, or `None` if it escapes (the bound box prevents
/// this in practice: `Rect::edges` are always symmetric and immediate, so the walk below
/// always terminates there at the latest).
///
/// The walk: every hit is collected, sorted ascending by `t` (stable — ties keep the input
/// order), then walked with a fresh per-ray `MAX_LIMITED_CROSSINGS` budget. A wall set
/// carrying neither flag therefore returns the minimum-`t` hit, earliest in input order on
/// ties. Per hit, in order:
/// (a) a one-way wall whose transparent side faces `origin` (`Seg::transparent_from`) is
/// skipped outright — the segment is absent FOR THIS RAY, as is a wall `Seg::open_from` the
/// origin by proximity; (b) a `limited` wall is skipped
/// while budget remains (the budget is spent, so a second `limited` wall on the same ray
/// still stops it); (c) any other hit is the ray's terminus. The polygon's endpoint-angle
/// sampling in `visibility_polygon` is unconditional, so a skipped `limited` wall's own
/// endpoints are still angle-sampled as candidate vertices — the polygon boundary still bends
/// at its edge when the wall is the far side of the visible region.
///
/// A set with none of the three per-ray flags (`one_way`, `limited`, `open_within`) never skips
/// a hit, so its answer is the first minimum-`t` hit; that case is one allocation-free pass,
/// the common one for the movement and light collectors and for a scene with no such walls.
fn nearest_hit(origin: P, dir: P, segs: &[Seg]) -> Option<P> {
    if !segs
        .iter()
        .any(|s| s.one_way.is_some() || s.limited || s.open_within.is_some())
    {
        let mut best: Option<f64> = None;
        for s in segs {
            if let Some(t) = ray_segment(origin, dir, s) {
                // Strict `<` keeps the earliest input-order hit on a tie, as the stable sort does.
                if best.is_none_or(|b| t < b) {
                    best = Some(t);
                }
            }
        }
        return best.map(|t| (origin.0 + t * dir.0, origin.1 + t * dir.1));
    }
    let mut hits: Vec<(f64, &Seg)> = segs
        .iter()
        .filter_map(|s| ray_segment(origin, dir, s).map(|t| (t, s)))
        .collect();
    hits.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut budget = MAX_LIMITED_CROSSINGS;
    for (t, s) in hits {
        if s.transparent_from(origin) || s.open_from(origin) {
            continue;
        }
        if s.limited && budget > 0 {
            budget -= 1;
            continue;
        }
        return Some((origin.0 + t * dir.0, origin.1 + t * dir.1));
    }
    None
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

/// Whether `seg`'s own axis-aligned bounding box overlaps `r`'s. `viewpoint` lies inside `bound`
/// by every caller's construction (`bound_for`/`bound_for_scene` build it that way), so a segment
/// whose bbox has zero overlap with `bound`'s bbox lies entirely outside the rectangle; by
/// convexity, any ray leaving an interior point already crosses `bound`'s own edge (one of
/// `Rect::edges`, always present in `segs`) before it could reach that segment, so `nearest_hit`
/// never needs to consider it. Filtering it out here changes no polygon vertex, only how many
/// segments `nearest_hit` scans per ray.
fn seg_overlaps_rect(seg: &Seg, r: &Rect) -> bool {
    let (minx, maxx) = (seg.a.0.min(seg.b.0), seg.a.0.max(seg.b.0));
    let (miny, maxy) = (seg.a.1.min(seg.b.1), seg.a.1.max(seg.b.1));
    minx <= r.maxx && maxx >= r.minx && miny <= r.maxy && maxy >= r.miny
}

/// The visibility polygon from `viewpoint`, occluded by `walls`, terminated by `bound`.
/// Vertices are in ascending-angle order (a star-shaped polygon around the viewpoint).
///
/// # Examples
///
/// ```
/// use shadowcat::scene::vision::{visibility_polygon, Rect};
///
/// // No walls: the polygon is bounded by `bound` alone (a box around the viewpoint).
/// let bound = Rect { minx: -1.0, miny: -1.0, maxx: 1.0, maxy: 1.0 };
/// let poly = visibility_polygon((0.0, 0.0), &[], bound);
/// assert!(!poly.is_empty());
/// ```
pub fn visibility_polygon(viewpoint: P, walls: &[Seg], bound: Rect) -> Vec<P> {
    // Pre-filter to walls whose bbox overlaps `bound` before cloning/sampling: a large scene's
    // wall slice otherwise gets fully cloned and every endpoint angle-sampled regardless of how
    // few walls are actually near this viewpoint's bound.
    let mut segs: Vec<Seg> = walls
        .iter()
        .copied()
        .filter(|s| seg_overlaps_rect(s, &bound))
        .collect();
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

/// A source's facing-limited field of view: a forward arc plus an all-round bubble.
///
/// `facing_rad` is the direction the source looks (radians, `0` = +x, growing toward +y, the
/// convention of `scene::move_exec::token_facing_radians`). `half_rad` is half the arc's width: a
/// ray whose angle lies strictly within `half_rad` of `facing_rad` runs to its occluder, every
/// other ray is cut at `near_world`. `near_world` is the bubble radius in WORLD units (the
/// authored cell radius times `GridShape::world_units_per_cell`). `half_rad == 0.0` is the
/// bubble-only view an invalid stored value resolves to (fail closed).
///
/// # Examples
///
/// ```
/// use shadowcat::scene::vision::FacingArc;
///
/// let arc = FacingArc { facing_rad: 0.0, half_rad: std::f64::consts::FRAC_PI_4, near_world: 1.0 };
/// assert_eq!(arc.near_world, 1.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FacingArc {
    /// Direction the source looks, radians.
    pub facing_rad: f64,
    /// Half the arc's full width, radians, in `[0, π)`.
    pub half_rad: f64,
    /// All-round awareness radius, world units, `>= 0`.
    pub near_world: f64,
}

/// Angular spacing (radians, 10 degrees) of the extra samples `visibility_polygon_facing` adds so
/// the all-round bubble is round and not a few long chords.
const FACING_RING_STEP: f64 = std::f64::consts::PI / 18.0;

/// The visibility polygon of a facing-limited source: `visibility_polygon`'s rays, each cut at
/// `facing.near_world` when its angle lies outside the forward arc. Still one star-shaped polygon
/// about `viewpoint`, so `point_in_poly`, the lit mask, the egress clip and the client's fog hole
/// consume it unchanged.
///
/// Algorithm: the same endpoint angular sweep as `visibility_polygon`, with three extra sample
/// families so the arc boundary is exact and the bubble is round: both arc edges (each ±`EPS`, so
/// the radial step between "runs to the wall" and "capped at the bubble" is a near-radial edge),
/// and a ring sample every `FACING_RING_STEP`. Each sample's relative angle to the facing is
/// wrapped into `[-π, π)` before the arc test, so an arc spanning the ±π seam is handled.
///
/// # Examples
///
/// ```
/// use shadowcat::scene::vision::{visibility_polygon_facing, FacingArc, Rect};
///
/// let bound = Rect { minx: -10.0, miny: -10.0, maxx: 10.0, maxy: 10.0 };
/// let arc = FacingArc { facing_rad: 0.0, half_rad: std::f64::consts::FRAC_PI_4, near_world: 0.0 };
/// let poly = visibility_polygon_facing((0.0, 0.0), &[], bound, arc);
/// assert!(poly.len() >= 3);
/// ```
pub fn visibility_polygon_facing(
    viewpoint: P,
    walls: &[Seg],
    bound: Rect,
    facing: FacingArc,
) -> Vec<P> {
    let mut segs: Vec<Seg> = walls
        .iter()
        .copied()
        .filter(|s| seg_overlaps_rect(s, &bound))
        .collect();
    segs.extend(bound.edges());

    let mut angles: Vec<f64> = Vec::with_capacity(segs.len() * 6 + 80);
    for s in &segs {
        for p in [s.a, s.b] {
            let ang = (p.1 - viewpoint.1).atan2(p.0 - viewpoint.0);
            angles.push(wrap_angle(ang - EPS));
            angles.push(ang);
            angles.push(wrap_angle(ang + EPS));
        }
    }
    for edge in [
        facing.facing_rad - facing.half_rad,
        facing.facing_rad + facing.half_rad,
    ] {
        angles.push(wrap_angle(edge - EPS));
        angles.push(wrap_angle(edge));
        angles.push(wrap_angle(edge + EPS));
    }
    let ring = (std::f64::consts::TAU / FACING_RING_STEP).round() as usize;
    for k in 0..ring {
        angles.push(wrap_angle(
            -std::f64::consts::PI + k as f64 * FACING_RING_STEP,
        ));
    }
    angles.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let mut poly: Vec<P> = Vec::with_capacity(angles.len());
    for &ang in &angles {
        let dir = (ang.cos(), ang.sin());
        let Some(hit) = nearest_hit(viewpoint, dir, &segs) else {
            continue;
        };
        let rel = wrap_angle(ang - facing.facing_rad).abs();
        if rel < facing.half_rad {
            poly.push(hit);
            continue;
        }
        let dist = ((hit.0 - viewpoint.0).powi(2) + (hit.1 - viewpoint.1).powi(2)).sqrt();
        let reach = dist.min(facing.near_world);
        poly.push((viewpoint.0 + dir.0 * reach, viewpoint.1 + dir.1 * reach));
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
