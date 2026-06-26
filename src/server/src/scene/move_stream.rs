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
/// via `player_vision_polygons_at` at the sample's viewpoint, scene-local.
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
    // (`iter().any()` returns false on an empty slice); it is handled by the guard below.
    if path.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
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
            // Division is safe: n >= 2 is invariant (enforced by .clamp(2, …) above).
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
mod tests {
    use super::*;

    /// Straight two-cell path: path=[(0,0),(100,0),(200,0)], cell=100, dur=1000.
    /// Expect first sample at t=0 pos=(0,0), last at t=1000 pos=(200,0),
    /// count ≈ 2*SAMPLES_PER_CELL+1 (±1), and strictly increasing t_ms.
    #[test]
    fn straight_two_cell_path_samples_endpoints_and_interior() {
        let path = vec![(0.0_f64, 0.0_f64), (100.0, 0.0), (200.0, 0.0)];
        let samples = sample_path(&path, 100.0, 1000.0);

        let first = &samples[0];
        let last = samples.last().unwrap();

        assert!(
            (first.t_ms - 0.0).abs() < 1e-9,
            "first t_ms should be 0.0, got {}",
            first.t_ms
        );
        assert!(
            (first.pos.0 - 0.0).abs() < 1e-9 && (first.pos.1 - 0.0).abs() < 1e-9,
            "first pos should be (0,0), got {:?}",
            first.pos
        );
        assert!(
            (last.t_ms - 1000.0).abs() < 1e-9,
            "last t_ms should be 1000.0, got {}",
            last.t_ms
        );
        assert!(
            (last.pos.0 - 200.0).abs() < 1e-9 && (last.pos.1 - 0.0).abs() < 1e-9,
            "last pos should be (200,0), got {:?}",
            last.pos
        );

        // Count: ceil(2*3)=6 steps → 7 samples (0..=6), allow ±1.
        let expected_count = (2.0 * SAMPLES_PER_CELL).ceil() as usize + 1;
        assert!(
            samples.len() >= expected_count - 1 && samples.len() <= expected_count + 1,
            "count {}, expected ~{}",
            samples.len(),
            expected_count
        );

        // Strictly increasing t_ms.
        for w in samples.windows(2) {
            assert!(
                w[1].t_ms > w[0].t_ms,
                "t_ms not strictly increasing: {} then {}",
                w[0].t_ms,
                w[1].t_ms
            );
        }
    }

    /// A very long path (> MAX_VISION_SAMPLES/SAMPLES_PER_CELL cells) must be capped at
    /// MAX_VISION_SAMPLES with endpoints exact.
    #[test]
    fn cap_bounds_samples() {
        // 40 cells → uncapped density = ceil(40*3)=120 > MAX_VISION_SAMPLES(96).
        let n_cells: usize = 40;
        let cell = 100.0_f64;
        let path: Vec<(f64, f64)> = (0..=n_cells).map(|i| (i as f64 * cell, 0.0)).collect();
        let duration_ms = n_cells as f64 * 500.0;
        let samples = sample_path(&path, cell, duration_ms);

        assert!(
            samples.len() <= MAX_VISION_SAMPLES,
            "cap violated: {} > {}",
            samples.len(),
            MAX_VISION_SAMPLES
        );

        let first = &samples[0];
        let last = samples.last().unwrap();
        assert!((first.t_ms - 0.0).abs() < 1e-9, "first t_ms {}", first.t_ms);
        assert!(
            (first.pos.0 - 0.0).abs() < 1e-6,
            "first pos.x {}",
            first.pos.0
        );
        assert!(
            (last.t_ms - duration_ms).abs() < 1e-6,
            "last t_ms {}",
            last.t_ms
        );
        assert!(
            (last.pos.0 - (n_cells as f64 * cell)).abs() < 1e-6,
            "last pos.x {}",
            last.pos.0
        );
    }

    /// Zero-progress: path=[(0,0)] → exactly one sample at t_ms=0.
    #[test]
    fn zero_progress_returns_single_sample() {
        let samples = sample_path(&[(0.0, 0.0)], 100.0, 1000.0);
        assert_eq!(
            samples.len(),
            1,
            "expected single sample, got {}",
            samples.len()
        );
        assert!((samples[0].t_ms - 0.0).abs() < 1e-9);
        assert_eq!(samples[0].pos, (0.0, 0.0));
    }

    /// Zero duration: even a multi-point path → exactly one sample at t_ms=0.
    #[test]
    fn zero_duration_returns_single_sample() {
        let path = vec![(0.0, 0.0), (100.0, 0.0), (200.0, 0.0)];
        let samples = sample_path(&path, 100.0, 0.0);
        assert_eq!(samples.len(), 1, "expected single sample for zero duration");
        assert!((samples[0].t_ms - 0.0).abs() < 1e-9);
        assert_eq!(samples[0].pos, (0.0, 0.0));
    }

    /// Arc-length time mapping: L-route [(0,0),(100,0),(100,100)].
    /// Total arc-length = 200; corner (100,0) at arc-length 100 = half → t_ms ≈ 500.
    #[test]
    fn arc_length_time_mapping() {
        let path = vec![(0.0_f64, 0.0_f64), (100.0, 0.0), (100.0, 100.0)];
        let samples = sample_path(&path, 100.0, 1000.0);

        // Find the sample nearest to pos (100, 0) — the corner vertex.
        let corner_sample = samples
            .iter()
            .min_by(|a, b| {
                let da = (a.pos.0 - 100.0).hypot(a.pos.1 - 0.0);
                let db = (b.pos.0 - 100.0).hypot(b.pos.1 - 0.0);
                da.partial_cmp(&db).unwrap()
            })
            .unwrap();

        // Accept within one inter-sample interval plus a small epsilon.
        let interval = 1000.0 / (samples.len() as f64 - 1.0);
        assert!(
            (corner_sample.t_ms - 500.0).abs() < interval + 1.0,
            "corner t_ms {} not near 500; interval {}",
            corner_sample.t_ms,
            interval
        );
    }
}
