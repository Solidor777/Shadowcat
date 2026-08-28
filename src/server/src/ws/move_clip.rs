//! Pure sample-clipping arithmetic for the per-recipient `MoveStream` egress clip.
//!
//! No locks, no I/O. `clip_move_stream` (in `conn`) resolves the clip target's committed
//! vision and in-flight move timelines, then delegates the per-sample decision here.
//!
//! INVARIANT (client parity): `chosen_vision_sample` implements the same rule as the client's
//! `chooseVisionSample` (`fog-blend.ts`) — greatest `t_ms <= elapsed`, first sample before
//! that — so a sample admitted here is exactly one the recipient's sweeping fog will show.
//! The shared fixture `src/client/render/src/__fixtures__/chosen-vision-sample.json` is
//! asserted by both sides.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::scene::vision::{point_in_poly, P};
use crate::ws::protocol::{PosSample, VisionSample};

/// One in-flight move of the clip target: its wall-clock origin and vision sweep.
pub(crate) struct TimelineStream<'a> {
    /// The move's `MoveStream.start_server_ms`.
    pub start_server_ms: f64,
    /// The move's `mover_vision` samples (elapsed-ms from `start_server_ms`).
    pub vision: &'a [VisionSample],
}

/// The sample with the greatest `t_ms <= elapsed_ms`; the first sample when `elapsed_ms`
/// precedes every sample; `None` only when `samples` is empty.
pub(crate) fn chosen_vision_sample(
    samples: &[VisionSample],
    elapsed_ms: f64,
) -> Option<&VisionSample> {
    let mut chosen = samples.first()?;
    for s in samples {
        if s.t_ms <= elapsed_ms {
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

/// Keep each sample whose position is inside the clip target's vision AT THAT SAMPLE'S
/// INSTANT: the timeline union while any target stream is active, else `static_polys`.
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
            let timeline = timeline_polys_at(streams, start_server_ms + s.t_ms);
            let polys: &[Vec<P>] = match &timeline {
                Some(t) => t,
                None => static_polys,
            };
            polys.iter().any(|poly| point_in_poly(poly, p))
        })
        .copied()
        .collect()
}

#[cfg(test)]
mod tests;
