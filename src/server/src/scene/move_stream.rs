//! Position trajectory sampler for `MoveStream` broadcast.
//!
//! Pure, no I/O. Converts a legal render-path and duration into time-tagged
//! position samples for client playback. Consumed by `Room::execute_move`,
//! which extends `MoveExecution` to carry `samples`.
//!
//! Algorithm: arc-length parameterisation — compute cumulative segment lengths,
//! place `n` samples at equal arc-length steps, map each onto the polyline via
//! linear interpolation, and assign `t_ms = s / L * duration_ms`.
//!
//! Coupling: `MAX_VISION_SAMPLES` is the shared cap for both position samples
//! and vision samples; vision samples are computed by `Room::execute_move` via
//! `SceneEcs::player_vision_inputs` + `VisionMoveInputs::polygons_at`. The cap
//! prevents a pathologically long path from flooding the broadcast.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

/// Maximum number of samples in a `MoveStream` (position or vision).
/// Shared cap across all sample types on a single `MoveStream` frame.
pub(crate) const MAX_VISION_SAMPLES: usize = 96;

/// Maximum vertices per vision polygon in a `MoveStream` `VisionSample`.
/// Visibility polygons in scenes with many wall segments can be large; beyond this
/// bound truncation is applied (fail-closed under-reveal: truncation never over-reveals).
pub(crate) const MAX_VISION_POLYGON_VERTS: usize = 512;

/// Target density of position samples (samples per cell of arc-length).
/// ~3 per cell gives smooth playback at normal animation speeds.
pub(crate) const SAMPLES_PER_CELL: f64 = 3.0;

/// A time-tagged vision sample for the mover's fog-sweep trajectory. `t_ms` matches
/// the corresponding `PosSamplePt.t_ms`; `polygons` are the visible regions computed
/// via `SceneEcs::player_vision_inputs` + `VisionMoveInputs::polygons_at` at the sample's
/// viewpoint, scene-local.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VisionSamplePt {
    /// Elapsed time in milliseconds from the move's `start_server_ms`.
    pub t_ms: f64,
    /// Visibility polygons (scene coords) visible at this instant. One polygon per owned
    /// token contributing to the union (moving token at its sample viewpoint; other owned
    /// tokens at committed positions).
    pub polygons: Vec<Vec<crate::scene::vision::P>>,
}

/// A time-tagged position sample for client playback.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PosSamplePt {
    /// Elapsed time in milliseconds from the move's `start_server_ms`.
    /// INVARIANT: `t_ms >= 0`; samples are strictly increasing (consecutive
    /// exact-equal values are de-duped defensively).
    pub t_ms: f64,
    /// Scene-coordinate position (x, y) at this sample instant.
    pub pos: (f64, f64),
}

/// Time-tag the legal render-path into position samples for client playback.
///
/// # Invariants
/// - `cell` > 0; `duration_ms` >= 0.
/// - Always includes the first and last vertex of `path`.
/// - Samples are strictly increasing in `t_ms` (consecutive exact-equal values
///   are removed defensively).
/// - Returns exactly one sample `{t_ms: 0, pos: path[0]}` when:
///   - `path` has fewer than 2 points, OR
///   - the total arc-length `L` is less than 1e-9 (degenerate/zero-length guard), OR
///   - `duration_ms` is 0, OR
///   - any path coordinate is non-finite (fail-closed; mirrors `supercover_cells`).
/// - Result count `n` satisfies:
///   `2 <= n <= MAX_VISION_SAMPLES` for any multi-point path with L > 0.
///
/// # Algorithm (arc-length parameterisation)
/// 1. Compute cumulative arc-lengths `cum[i]` for each segment endpoint.
/// 2. `L = cum.last()`.
/// 3. Target count: `n = min(MAX_VISION_SAMPLES, max(2, ceil(L/cell * SAMPLES_PER_CELL)))`.
/// 4. Place `n` samples at equal arc-length steps `s_i = i/(n-1) * L` (0..=n-1).
/// 5. Map each `s_i` onto the polyline (binary search segment, linear interp within).
/// 6. `t_ms_i = s_i / L * duration_ms`.
/// 7. De-dup consecutive samples with exact-equal `t_ms` (defensive; arc-length steps
///    are strictly increasing by construction so this never fires on a valid path).
pub(crate) fn sample_path(path: &[(f64, f64)], cell: f64, duration_ms: f64) -> Vec<PosSamplePt> {
    debug_assert!(cell > 0.0, "cell must be positive");

    // Fail-closed non-finite guard: a NaN/Inf coordinate propagates through `sqrt` into
    // `cum`, causing `binary_search_by(.partial_cmp().unwrap())` to panic. Mirrors the
    // fail-closed convention of `supercover_cells`. The empty-path case cannot enter here
    // (`iter().any()` returns false on an empty slice); the single-point-or-empty guard
    // handles it.
    if path.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
        debug_assert!(
            !path.is_empty(),
            "any() is false on an empty slice, so path is non-empty here"
        );
        return vec![PosSamplePt {
            t_ms: 0.0,
            pos: path[0],
        }];
    }

    // Single-point or empty guard: one sample at t=0 at path[0] (or origin for empty).
    if path.is_empty() {
        return vec![PosSamplePt {
            t_ms: 0.0,
            pos: (0.0, 0.0),
        }];
    }
    if path.len() == 1 || duration_ms < 1e-9 {
        return vec![PosSamplePt {
            t_ms: 0.0,
            pos: path[0],
        }];
    }

    // Cumulative arc-length table: cum[0]=0 at path[0]; cum[i] = length of path[0..=i].
    let mut cum: Vec<f64> = Vec::with_capacity(path.len());
    cum.push(0.0);
    for i in 1..path.len() {
        let dx = path[i].0 - path[i - 1].0;
        let dy = path[i].1 - path[i - 1].1;
        cum.push(cum[i - 1] + (dx * dx + dy * dy).sqrt());
    }
    let total_len = *cum.last().unwrap();

    // Zero-length guard (all vertices coincident, threshold < 1e-9): degenerate path → single sample.
    if total_len < 1e-9 {
        return vec![PosSamplePt {
            t_ms: 0.0,
            pos: path[0],
        }];
    }

    // Target sample count: density SAMPLES_PER_CELL per cell, floored at 2, capped at MAX.
    // Clamp to f64 before the usize cast to prevent overflow on 32-bit targets (mobile):
    // an uncapped `ceil()` on a very long path could exceed usize::MAX on 32-bit.
    let density = (total_len / cell * SAMPLES_PER_CELL)
        .ceil()
        .min(MAX_VISION_SAMPLES as f64) as usize;
    let n = density.clamp(2, MAX_VISION_SAMPLES);

    // Place n samples at equal arc-length steps.
    let mut samples: Vec<PosSamplePt> = Vec::with_capacity(n);
    for i in 0..n {
        // Clamp the last sample to exact total to avoid floating-point overshoot.
        let s = if i == n - 1 {
            total_len
        } else {
            // Division is safe: n >= 2 is invariant (enforced by `n`'s own `.clamp(2, …)`).
            (i as f64) / ((n - 1) as f64) * total_len
        };

        // Map s onto the polyline: binary search for the containing segment.
        // cum is non-decreasing; binary_search finds an exact match or the insertion point.
        let seg = match cum.binary_search_by(|c| c.partial_cmp(&s).unwrap()) {
            Ok(idx) => {
                // Exact cumulative hit: use the segment ending at this index.
                // saturating_sub(1) handles idx==0 (start); min(path.len()-2)
                // handles idx==path.len()-1 (end — use the last segment).
                idx.saturating_sub(1).min(path.len() - 2)
            }
            Err(idx) => {
                // idx is the first cum > s → segment is (idx-1, idx).
                // idx >= 1 always because cum[0]=0 <= s.
                (idx - 1).min(path.len() - 2)
            }
        };

        // Linear interpolation within the segment.
        let seg_len = cum[seg + 1] - cum[seg];
        let pos = if seg_len < 1e-12 {
            // Zero-length segment: snap to segment start.
            path[seg]
        } else {
            let t = ((s - cum[seg]) / seg_len).clamp(0.0, 1.0);
            (
                path[seg].0 + t * (path[seg + 1].0 - path[seg].0),
                path[seg].1 + t * (path[seg + 1].1 - path[seg].1),
            )
        };

        let t_ms = s / total_len * duration_ms;
        samples.push(PosSamplePt { t_ms, pos });
    }

    // Defensive de-dup: remove consecutive samples with exact-equal t_ms.
    // Arc-length steps s_i = i/(n-1)*L are strictly increasing for n>=2, L>0, so this
    // never fires on a valid path. Pure defence against any future caller deviation.
    // Exact equality is correct here — f64::EPSILON absolute tolerance was too tight to
    // fire for the rounding case and gave false assurance; samples are strictly increasing
    // by construction.
    samples.dedup_by(|b, a| b.t_ms == a.t_ms);

    samples
}

#[cfg(test)]
mod tests;
