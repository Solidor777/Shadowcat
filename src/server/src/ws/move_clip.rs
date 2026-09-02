//! Pure sample-clipping arithmetic for the per-recipient `MoveStream` egress clip.
//!
//! No locks, no I/O. `clip_move_stream` (in `conn`) resolves the clip target's authoritative
//! sight (`SceneEcs::recipient_sight`) and the scene's in-flight moves, then delegates the
//! per-sample decisions here through `clip_frame`: the position clip and the carried-light
//! admission. Both read the recipient's sight AT A SAMPLE'S INSTANT through the ONE
//! `ClipInputs::at` — the recipient's own in-flight tokens re-raycast at their instant
//! viewpoints and every in-flight torch composed in at its instant position — resolved ONCE
//! per distinct instant of the frame and shared by both gates, so the two can never disagree
//! about what the recipient sees at a given moment and no instant is paid for twice.
//!
//! INVARIANT (client parity): `chosen_vision_sample` implements the same rule as the client's
//! `chooseVisionSample` — greatest `t_ms <= elapsed`, first sample before
//! that — so the viewpoint and glow this module composes at an instant are exactly the ones the
//! recipient's sweeping fog and lighting show. It is ONE rule for every timed timeline (position,
//! vision and carried light alike); the shared fixture
//! `src/client/render/src/__fixtures__/chosen-vision-sample.json` is asserted by both sides.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;

use uuid::Uuid;

use crate::scene::vision::{point_in_poly, point_segment_distance, P};
use crate::scene::{InstantLight, InstantSight, RecipientSight};
use crate::ws::protocol::{LightSample, PosSample, VisionSample};

/// A timeline sample tagged with its elapsed offset from the timeline's origin.
pub(crate) trait Timed {
    /// Elapsed milliseconds from the owning move's `start_server_ms`.
    fn t_ms(&self) -> f64;
}

impl Timed for VisionSample {
    fn t_ms(&self) -> f64 {
        self.t_ms
    }
}

impl Timed for LightSample {
    fn t_ms(&self) -> f64 {
        self.t_ms
    }
}

impl Timed for PosSample {
    fn t_ms(&self) -> f64 {
        self.t_ms
    }
}

/// One in-flight move the clip reads timelines from: its wall-clock origin, its mover, the
/// moving token, its position samples (the mover's viewpoint per instant) and its carried-light
/// samples (the glow per instant, when the mover carries one).
pub(crate) struct InFlight<'a> {
    /// The move's `MoveStream.start_server_ms`.
    pub start_server_ms: f64,
    /// The move's `MoveStream.mover`.
    pub mover: Uuid,
    /// The move's `MoveStream.token_id`.
    pub token: Uuid,
    /// The move's position samples (elapsed-ms from `start_server_ms`).
    pub positions: &'a [PosSample],
    /// The move's carried-light samples, when the mover carries an enabled emission.
    pub light: Option<&'a [LightSample]>,
}

impl InFlight<'_> {
    /// Elapsed milliseconds into this move at absolute instant `t_abs_ms` — negative before
    /// the move starts, where `chosen_vision_sample` yields the FIRST sample: the position the
    /// token is still standing at (its committed position is already the move's END).
    fn elapsed_at(&self, t_abs_ms: f64) -> f64 {
        t_abs_ms - self.start_server_ms
    }
}

/// The per-recipient clip inputs for one frame: the clip target's sight plus the scene's
/// in-flight moves. `at` resolves the sight at one instant — THE per-instant read
/// `clip_frame` resolves once per distinct instant for both gates.
pub(crate) struct ClipInputs<'a> {
    /// The clip target's authoritative sight (`SceneEcs::recipient_sight`, with every in-flight
    /// mover's carried emission excluded from the committed field).
    pub sight: &'a RecipientSight,
    /// Every unexpired in-flight move in the frame's scene, the frame being clipped included.
    pub in_flight: &'a [InFlight<'a>],
    /// The clip target — whose own in-flight tokens are re-raycast at their instant viewpoints.
    pub target: Uuid,
}

impl ClipInputs<'_> {
    /// The sight at absolute instant `t_abs_ms`: the target's own moves substitute their
    /// chosen position sample as the moving token's viewpoint; every move with a carried
    /// light contributes its chosen light sample to the field. EVERY registered move
    /// contributes, started or not — a move's committed position is its END
    /// (`Room::execute_move` commits before it broadcasts), so for an instant BEFORE a move
    /// starts the only true position is its FIRST sample, which `chosen_vision_sample`
    /// yields for a negative elapsed time. Gating on "started" would judge the target's
    /// pre-start instants from its END viewpoint (over-admission on re-emit) and drop a
    /// not-yet-started torch from the field its committed emission was excluded from
    /// (under-reveal).
    pub(crate) fn at(&self, t_abs_ms: f64) -> (InstantSight<'_>, Vec<InstantLight>) {
        let mut moved: Vec<(Uuid, P)> = Vec::new();
        let mut lights: Vec<InstantLight> = Vec::new();
        for m in self.in_flight {
            let elapsed = m.elapsed_at(t_abs_ms);
            if m.mover == self.target {
                if let Some(s) = chosen_vision_sample(m.positions, elapsed) {
                    moved.push((m.token, (s.pos[0], s.pos[1])));
                }
            }
            if let Some(ls) = m.light.and_then(|l| chosen_vision_sample(l, elapsed)) {
                lights.push(self.sight.sample_light(ls));
            }
        }
        (self.sight.at(&moved), lights)
    }
}

/// The sample with the greatest `t_ms <= elapsed_ms`; the first sample when `elapsed_ms`
/// precedes every sample; `None` only when `samples` is empty. Generic over the sample kind:
/// the position, vision and carried-light timelines select through this one rule.
pub(crate) fn chosen_vision_sample<T: Timed>(samples: &[T], elapsed_ms: f64) -> Option<&T> {
    let mut chosen = samples.first()?;
    for s in samples {
        if s.t_ms() <= elapsed_ms {
            chosen = s;
        }
    }
    Some(chosen)
}

/// One recipient's clip of one frame: the position samples the target perceives at their
/// instants and the carried-light samples whose glow lights a cell the target sees at theirs
/// — `(visible positions, admitted light)`, the light `None` when the frame carries none or
/// no sample is admitted (never an empty list: the recipient learns nothing, not even that a
/// light moved). `ClipInputs::at` is resolved ONCE per distinct instant of the frame (a
/// position and a light sample at the same `t_ms` share it), then each sample is judged from
/// its instant: a position through `InstantSight::sees_token` (inside a source's line of
/// sight AND lit for that source, the mover's own carried light composed in — a torch bearer
/// lights the cell it stands in — OR within reach of a source's creature sense), a light
/// sample through `glow_reaches`. PRECONDITION: the frame's own move is among
/// `inputs.in_flight` (as `clip_move_stream` guarantees), so a light sample is composed into
/// the field it is judged against exactly once.
pub(crate) fn clip_frame(
    samples: &[PosSample],
    light: Option<&[LightSample]>,
    start_server_ms: f64,
    inputs: &ClipInputs<'_>,
) -> (Vec<PosSample>, Option<Vec<LightSample>>) {
    // Distinct instants by exact `t_ms` bits — the value both timelines were sampled with.
    let mut instants: HashMap<u64, (InstantSight<'_>, Vec<InstantLight>)> = HashMap::new();
    let ts = samples
        .iter()
        .map(|s| s.t_ms)
        .chain(light.into_iter().flatten().map(|l| l.t_ms));
    for t in ts {
        instants
            .entry(t.to_bits())
            .or_insert_with(|| inputs.at(start_server_ms + t));
    }
    let visible: Vec<PosSample> = samples
        .iter()
        .filter(|s| {
            instants
                .get(&s.t_ms.to_bits())
                .is_some_and(|(sight, lights)| sight.sees_token((s.pos[0], s.pos[1]), lights))
        })
        .copied()
        .collect();
    let admitted: Option<Vec<LightSample>> = light
        .map(|ls| {
            ls.iter()
                .filter(|l| {
                    instants
                        .get(&l.t_ms.to_bits())
                        .is_some_and(|(sight, lights)| glow_reaches(sight, lights, l))
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty());
    (visible, admitted)
}

/// The position half of `clip_frame` alone (a frame with no light timeline) — a test
/// convenience; production clips through `clip_frame`.
#[cfg(test)]
pub(crate) fn clip_samples(
    samples: &[PosSample],
    start_server_ms: f64,
    inputs: &ClipInputs<'_>,
) -> Vec<PosSample> {
    clip_frame(samples, None, start_server_ms, inputs).0
}

/// Whether the disc `(center, radius)` touches any polygon: the center lies inside one
/// (`point_in_poly`), or some polygon edge passes within `radius` of it
/// (`point_segment_distance`). Over-admission is the sanctioned direction for the carried-
/// light gate — a disc that merely grazes a corner counts — but a non-finite radius or
/// center admits nothing, and a non-positive radius reduces to the point test.
pub(crate) fn disc_intersects_polys<'a>(
    center: P,
    radius: f64,
    polys: impl IntoIterator<Item = &'a [P]>,
) -> bool {
    if !center.0.is_finite() || !center.1.is_finite() || !radius.is_finite() {
        return false;
    }
    polys.into_iter().any(|poly| {
        if poly.len() < 3 {
            return false;
        }
        if point_in_poly(poly, center) {
            return true;
        }
        radius > 0.0
            && (0..poly.len()).any(|i| {
                point_segment_distance(center, poly[i], poly[(i + 1) % poly.len()]) <= radius
            })
    })
}

/// Whether `sample`'s glow lights a cell the recipient sees at `sight`'s instant: some cell
/// center within `sample.dim` of its position that the sample's own light reaches
/// (`InstantSight::light_reaches` — `lighting::source_level` over the sample composed as an
/// `InstantLight`, so its occluder polygon and taper apply) AND that the recipient sees with
/// `lights` composed into the field (`InstantSight::sees` — the ONE visibility predicate:
/// line of sight, the composed illumination against the source's floor, darkvision ranges
/// and floors inherited). `lights` is the field at this instant (`ClipInputs::at`), which
/// carries this very sample as its own timeline's chosen light, so an ember below a normal-
/// vision recipient's dim floor lights nothing they see and is not admitted, while a
/// darkvision recipient within range is shown it — exactly the cells `player_lit_mask` would
/// light at rest. The disc test (`disc_intersects_polys`) is the cheap pre-filter. The fine
/// test ALWAYS runs and NEVER falls open: it scans the dim disc's box ∩ the recipient's
/// line-of-sight box (`InstantSight::los_bbox`) ∩ the sample's own occluder box — a cell
/// outside any of the three can be neither lit by this glow nor seen — under the lit mask's
/// own scan bound (`explored::MAX_CELLS_PER_POLYGON`), and a box past even that bound admits
/// nothing (fail-closed). A `dimRadius` is ingress-bounded by `MAX_FOOTPRINT_CELLS`
/// (`LightEmission::validate`), so no authored light reaches the bound. A non-finite or
/// non-positive reach admits nothing.
pub(crate) fn glow_reaches(
    sight: &InstantSight<'_>,
    lights: &[InstantLight],
    sample: &LightSample,
) -> bool {
    let (px, py) = (sample.pos[0], sample.pos[1]);
    let dim = sample.dim;
    if !px.is_finite() || !py.is_finite() || !dim.is_finite() || dim <= 0.0 {
        return false;
    }
    if !sight.disc_touches_los((px, py), dim) {
        return false;
    }
    let own = sight.sample_light(sample);
    let Some((mut lo, mut hi)) = sight.los_bbox() else {
        return false;
    };
    lo = (lo.0.max(px - dim), lo.1.max(py - dim));
    hi = (hi.0.min(px + dim), hi.1.min(py + dim));
    if let Some((olo, ohi)) = bbox_of(&own.occluder) {
        lo = (lo.0.max(olo.0), lo.1.max(olo.1));
        hi = (hi.0.min(ohi.0), hi.1.min(ohi.1));
    }
    if lo.0 > hi.0 || lo.1 > hi.1 {
        return false;
    }
    let Some(centers) =
        sight.cell_centers_in(lo, hi, crate::scene::explored::MAX_CELLS_PER_POLYGON)
    else {
        return false;
    };
    centers
        .into_iter()
        .any(|c| sight.light_reaches(&own, c) && sight.sees(c, lights))
}

/// The AABB `(min, max)` of `poly`'s vertices; `None` for an empty polygon.
fn bbox_of(poly: &[P]) -> Option<(P, P)> {
    let mut it = poly.iter().copied();
    let first = it.next()?;
    Some(it.fold((first, first), |(lo, hi), (x, y)| {
        ((lo.0.min(x), lo.1.min(y)), (hi.0.max(x), hi.1.max(y)))
    }))
}

/// The carried-light half of `clip_frame` alone (a frame with no position samples): `None`
/// in and `None` out; a timeline no sample of which reaches the recipient is `None` too — a
/// test convenience; production clips through `clip_frame`.
#[cfg(test)]
pub(crate) fn admit_light_samples(
    samples: Option<&[LightSample]>,
    start_server_ms: f64,
    inputs: &ClipInputs<'_>,
) -> Option<Vec<LightSample>> {
    clip_frame(&[], samples, start_server_ms, inputs).1
}

#[cfg(test)]
mod tests;
