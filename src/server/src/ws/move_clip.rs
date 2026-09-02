//! Pure sample-clipping arithmetic for the per-recipient `MoveStream` egress clip.
//!
//! No locks, no I/O. `clip_move_stream` (in `conn`) resolves the clip target's committed
//! vision and in-flight move timelines, then delegates the per-sample decisions here: the
//! position clip (`clip_samples`) and the carried-light admission (`admit_light_samples`).
//! Both read the recipient's vision AT A SAMPLE'S INSTANT through the ONE `vision_at_instant`
//! — the timeline union while any target stream is active, the committed polygons otherwise —
//! so the two can never disagree about what the recipient sees at a given moment.
//!
//! INVARIANT (client parity): `chosen_vision_sample` implements the same rule as the client's
//! `chooseVisionSample` — greatest `t_ms <= elapsed`, first sample before
//! that — so a sample admitted here is exactly one the recipient's sweeping fog will show.
//! It is ONE rule for every timed timeline (vision and carried light alike); the shared
//! fixture `src/client/render/src/__fixtures__/chosen-vision-sample.json` is asserted by both
//! sides on both sample kinds.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::borrow::Cow;

use crate::scene::vision::{point_in_poly, point_segment_distance, P};
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

/// One in-flight move of the clip target: its wall-clock origin and vision sweep.
pub(crate) struct TimelineStream<'a> {
    /// The move's `MoveStream.start_server_ms`.
    pub start_server_ms: f64,
    /// The move's `mover_vision` samples (elapsed-ms from `start_server_ms`).
    pub vision: &'a [VisionSample],
}

/// The sample with the greatest `t_ms <= elapsed_ms`; the first sample when `elapsed_ms`
/// precedes every sample; `None` only when `samples` is empty. Generic over the sample kind:
/// the vision timeline the clip reads and the carried-light timeline the client sweeps
/// select through this one rule.
pub(crate) fn chosen_vision_sample<T: Timed>(samples: &[T], elapsed_ms: f64) -> Option<&T> {
    let mut chosen = samples.first()?;
    for s in samples {
        if s.t_ms() <= elapsed_ms {
            chosen = s;
        }
    }
    Some(chosen)
}

/// Union of the chosen-sample polygons of every stream that has started by `t_abs_ms`.
/// `None` when no stream has started (the caller falls back to committed vision).
pub(crate) fn timeline_polys_at(
    streams: &[TimelineStream<'_>],
    t_abs_ms: f64,
) -> Option<Vec<Vec<P>>> {
    let mut out: Vec<Vec<P>> = Vec::new();
    let mut any = false;
    for st in streams.iter().filter(|st| st.start_server_ms <= t_abs_ms) {
        any = true;
        if let Some(sample) = chosen_vision_sample(st.vision, t_abs_ms - st.start_server_ms) {
            out.extend(
                sample
                    .polygons
                    .iter()
                    .map(|poly| poly.iter().map(|v| (v[0], v[1])).collect()),
            );
        }
    }
    any.then_some(out)
}

/// The clip target's vision at absolute instant `t_abs_ms`: the timeline union while any
/// target stream has started (`timeline_polys_at`), else `static_polys` (committed vision).
/// THE per-instant vision read — `clip_samples` and `admit_light_samples` both go through it.
pub(crate) fn vision_at_instant<'a>(
    static_polys: &'a [Vec<P>],
    streams: &[TimelineStream<'_>],
    t_abs_ms: f64,
) -> Cow<'a, [Vec<P>]> {
    match timeline_polys_at(streams, t_abs_ms) {
        Some(t) => Cow::Owned(t),
        None => Cow::Borrowed(static_polys),
    }
}

/// Keep each sample whose position is inside the clip target's vision AT THAT SAMPLE'S
/// INSTANT (`vision_at_instant`).
pub(crate) fn clip_samples(
    samples: &[PosSample],
    start_server_ms: f64,
    static_polys: &[Vec<P>],
    streams: &[TimelineStream<'_>],
) -> Vec<PosSample> {
    samples
        .iter()
        .filter(|s| {
            let p = (s.pos[0], s.pos[1]);
            let polys = vision_at_instant(static_polys, streams, start_server_ms + s.t_ms);
            polys.iter().any(|poly| point_in_poly(poly, p))
        })
        .copied()
        .collect()
}

/// Whether the disc `(center, radius)` touches any polygon: the center lies inside one
/// (`point_in_poly`), or some polygon edge passes within `radius` of it
/// (`point_segment_distance`). Over-admission is the sanctioned direction for the carried-
/// light gate — a disc that merely grazes a corner counts — but a non-finite radius or
/// center admits nothing, and a non-positive radius reduces to the point test.
pub(crate) fn disc_intersects_polys(center: P, radius: f64, polys: &[Vec<P>]) -> bool {
    if !center.0.is_finite() || !center.1.is_finite() || !radius.is_finite() {
        return false;
    }
    polys.iter().any(|poly| {
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

/// Per-recipient admission of a carried-light timeline: keep each sample whose dim-reach disc
/// (`pos`, `dim`) intersects the clip target's vision AT THAT SAMPLE'S INSTANT — the SAME
/// `vision_at_instant` the position clip reads, never a second rule. `None` in and `None` out;
/// a timeline no sample of which reaches the recipient is `None` too (the recipient learns
/// nothing, not even that a light moved), never an empty list.
pub(crate) fn admit_light_samples(
    samples: Option<&[LightSample]>,
    start_server_ms: f64,
    static_polys: &[Vec<P>],
    streams: &[TimelineStream<'_>],
) -> Option<Vec<LightSample>> {
    let admitted: Vec<LightSample> = samples?
        .iter()
        .filter(|s| {
            let polys = vision_at_instant(static_polys, streams, start_server_ms + s.t_ms);
            disc_intersects_polys((s.pos[0], s.pos[1]), s.dim, &polys)
        })
        .cloned()
        .collect();
    (!admitted.is_empty()).then_some(admitted)
}

#[cfg(test)]
mod tests;
