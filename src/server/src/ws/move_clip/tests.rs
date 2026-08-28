use super::*;
use crate::ws::protocol::{PosSample, VisionSample};

/// Polygon whose first vertex x-coordinate encodes `idx`, so a chosen sample is identifiable.
fn tagged(idx: usize) -> Vec<Vec<[f64; 2]>> {
    let i = idx as f64;
    vec![vec![[i, 0.0], [i, 1.0], [i + 1.0, 1.0]]]
}

fn fixture_samples() -> (Vec<VisionSample>, Vec<(f64, usize)>) {
    let raw: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../client/render/src/__fixtures__/chosen-vision-sample.json"
    ))
    .unwrap();
    let samples = raw["samples"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, t)| VisionSample {
            t_ms: t.as_f64().unwrap(),
            polygons: tagged(i),
        })
        .collect();
    let probes = raw["probes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["elapsed"].as_f64().unwrap(),
                p["expectIndex"].as_u64().unwrap() as usize,
            )
        })
        .collect();
    (samples, probes)
}

#[test]
fn chosen_vision_sample_matches_the_shared_parity_fixture() {
    let (samples, probes) = fixture_samples();
    for (elapsed, expect) in probes {
        let got = chosen_vision_sample(&samples, elapsed).unwrap();
        assert_eq!(got.polygons, tagged(expect), "elapsed={elapsed}");
    }
}

#[test]
fn chosen_vision_sample_is_none_on_empty() {
    assert!(chosen_vision_sample(&[], 0.0).is_none());
}

/// A unit square at the origin, and one shifted to x in [10,11].
fn square(x0: f64) -> Vec<Vec<[f64; 2]>> {
    vec![vec![[x0, 0.0], [x0 + 1.0, 0.0], [x0 + 1.0, 1.0], [x0, 1.0]]]
}

#[test]
fn timeline_polys_at_is_none_before_any_stream_starts() {
    let v = vec![VisionSample {
        t_ms: 0.0,
        polygons: square(0.0),
    }];
    let streams = [TimelineStream {
        start_server_ms: 1000.0,
        vision: &v,
    }];
    assert!(timeline_polys_at(&streams, 999.0).is_none());
    assert!(timeline_polys_at(&streams, 1000.0).is_some());
}

#[test]
fn timeline_polys_at_unions_every_started_stream_and_uses_the_last_sample_past_its_end() {
    let a = vec![
        VisionSample {
            t_ms: 0.0,
            polygons: square(0.0),
        },
        VisionSample {
            t_ms: 100.0,
            polygons: square(10.0),
        },
    ];
    let b = vec![VisionSample {
        t_ms: 0.0,
        polygons: square(20.0),
    }];
    let streams = [
        TimelineStream {
            start_server_ms: 1000.0,
            vision: &a,
        },
        TimelineStream {
            start_server_ms: 1050.0,
            vision: &b,
        },
    ];
    // t=1040: only `a` started, at its first sample.
    let p = timeline_polys_at(&streams, 1040.0).unwrap();
    assert_eq!(
        p,
        vec![vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]]
    );
    // t=5000: both started; `a` is past its last sample (uses square(10)), `b` at its only sample.
    let p = timeline_polys_at(&streams, 5000.0).unwrap();
    assert_eq!(p.len(), 2);
    assert_eq!(p[0][0], (10.0, 0.0));
    assert_eq!(p[1][0], (20.0, 0.0));
}

#[test]
fn clip_samples_uses_the_timeline_only_at_instants_a_stream_is_active() {
    // A's samples: (0.5,0.5) at t=0 and (10.5,0.5) at t=200, starting at 1000.
    let samples = vec![
        PosSample {
            t_ms: 0.0,
            pos: [0.5, 0.5],
        },
        PosSample {
            t_ms: 200.0,
            pos: [10.5, 0.5],
        },
    ];
    // Static (committed) vision covers only the origin square.
    let static_polys: Vec<Vec<P>> = vec![vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]];
    // Target R's sweep starts at 1100 and sees the far square from its first sample.
    let r = vec![VisionSample {
        t_ms: 0.0,
        polygons: square(10.0),
    }];
    let streams = [TimelineStream {
        start_server_ms: 1100.0,
        vision: &r,
    }];
    let out = clip_samples(&samples, 1000.0, &static_polys, &streams);
    // t_abs=1000 → static → origin visible; t_abs=1200 → timeline → far square visible.
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].pos, [0.5, 0.5]);
    assert_eq!(out[1].pos, [10.5, 0.5]);
}

#[test]
fn clip_samples_with_no_streams_equals_static_clip() {
    let samples = vec![
        PosSample {
            t_ms: 0.0,
            pos: [0.5, 0.5],
        },
        PosSample {
            t_ms: 200.0,
            pos: [10.5, 0.5],
        },
    ];
    let static_polys: Vec<Vec<P>> = vec![vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]];
    let out = clip_samples(&samples, 1000.0, &static_polys, &[]);
    assert_eq!(
        out,
        vec![PosSample {
            t_ms: 0.0,
            pos: [0.5, 0.5]
        }]
    );
}
