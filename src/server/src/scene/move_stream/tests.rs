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

/// Any-angle diagonal path (non-grid-aligned vertices, no 45°/90° structure): endpoints
/// close (within tolerance — matches this module's other position assertions, not
/// zero-tolerance), `t_ms` strictly increasing (arc-length monotonic), AND an interior
/// sample's position is hand-derived from the arc-length formula and checked against the
/// actual output — proving the diagonal segment-selection + lerp math is correct for a
/// genuinely any-angle segment (not just the t=0/t=1 boundary identities every
/// interpolation, correct or broken, satisfies).
#[test]
fn diagonal_any_angle_path_samples_endpoints_with_monotonic_time() {
    let p0 = (0.0_f64, 0.0_f64);
    let p1 = (137.5_f64, 84.2_f64);
    let p2 = (310.0_f64, 10.0_f64);
    let path = vec![p0, p1, p2];
    let cell = 100.0_f64;
    let duration_ms = 1500.0_f64;
    let samples = sample_path(&path, cell, duration_ms);

    let first = &samples[0];
    let last = samples.last().unwrap();

    assert!((first.t_ms - 0.0).abs() < 1e-9, "first t_ms {}", first.t_ms);
    assert!(
        (first.pos.0 - p0.0).abs() < 1e-9 && (first.pos.1 - p0.1).abs() < 1e-9,
        "first pos should be {:?}, got {:?}",
        p0,
        first.pos
    );
    assert!(
        (last.t_ms - duration_ms).abs() < 1e-6,
        "last t_ms {}",
        last.t_ms
    );
    assert!(
        (last.pos.0 - p2.0).abs() < 1e-9 && (last.pos.1 - p2.1).abs() < 1e-9,
        "last pos should be {:?}, got {:?}",
        p2,
        last.pos
    );

    // Hand-derive an interior sample's expected position from the same arc-length
    // formula `sample_path` uses (not by calling into its internals): segment lengths,
    // target sample count `n`, then the arc-length `s_i` at index `i`, mapped onto
    // whichever segment it falls in. `len1 = 161.2324098932966`, `len2 =
    // 187.78149536096467`, `total_len = 349.01390525426126`; for `cell=100`,
    // `SAMPLES_PER_CELL=3`: `density = ceil(349.0139.../100*3) = 11`, `n =
    // 11.clamp(2, 96) = 11`. Index `i=2` gives `s_2 = 2/10*total_len ≈ 69.803`, which
    // falls well inside segment 1 (`s_2 < len1`, and not close to the `len1` boundary —
    // keeps the test unambiguous about which segment it exercises).
    let len1 = (p1.0 - p0.0).hypot(p1.1 - p0.1);
    let len2 = (p2.0 - p1.0).hypot(p2.1 - p1.1);
    let total_len = len1 + len2;
    let density = (total_len / cell * SAMPLES_PER_CELL)
        .ceil()
        .min(MAX_VISION_SAMPLES as f64) as usize;
    let n = density.clamp(2, MAX_VISION_SAMPLES);
    assert_eq!(samples.len(), n, "sample count should equal derived n");

    let i = 2;
    assert!(i < n - 1, "chosen index must be a genuine interior sample");
    let s_i = (i as f64) / ((n - 1) as f64) * total_len;
    assert!(
        s_i < len1 * 0.9,
        "chosen sample must fall clearly within segment 1, away from the len1 boundary"
    );
    let t = s_i / len1;
    let expected_pos = (p0.0 + t * (p1.0 - p0.0), p0.1 + t * (p1.1 - p0.1));

    let actual = &samples[i];
    assert!(
        (actual.pos.0 - expected_pos.0).abs() < 1e-9
            && (actual.pos.1 - expected_pos.1).abs() < 1e-9,
        "interior sample {} pos should be {:?}, got {:?}",
        i,
        expected_pos,
        actual.pos
    );

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
