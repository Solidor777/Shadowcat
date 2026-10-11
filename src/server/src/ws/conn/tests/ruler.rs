//! Shared live ruler: the ingress payload bounds and the rate budget.
//!
//! The wire-level relay (rate, scene authorization, sender stamping, unclipped delivery) is
//! exercised end to end in `tests/ws_convergence.rs`.

use super::*;
use crate::ws::protocol::ClientMsg;
use crate::ws::ruler::{
    canonical, deserialize_points, origin_admissible, points_admissible, MAX_RULER_ORIGIN_BYTES,
    MAX_RULER_POINTS, RULER_RATE_PER_MIN,
};
use serde::de::value::{Error as ValueError, SeqDeserializer};

#[test]
fn points_admissible_bounds_count_finiteness_and_magnitude() {
    let ok = vec![(0.0, 0.0); MAX_RULER_POINTS];
    assert!(
        points_admissible(&ok),
        "exactly MAX_RULER_POINTS is admitted"
    );
    let over = vec![(0.0, 0.0); MAX_RULER_POINTS + 1];
    assert!(
        !points_admissible(&over),
        "one past MAX_RULER_POINTS is refused"
    );
    assert!(
        points_admissible(&[]),
        "an empty list is the stop signal and is admitted"
    );
    assert!(!points_admissible(&[(0.0, f64::NAN)]));
    assert!(!points_admissible(&[(f64::INFINITY, 0.0)]));
    let bound = crate::scene::move_exec::MAX_GATE_WALK_COORD;
    assert!(
        points_admissible(&[(bound, -bound)]),
        "the magnitude bound is inclusive"
    );
    assert!(!points_admissible(&[(bound + 1.0, 0.0)]));
    assert!(!points_admissible(&[(0.0, -bound - 1.0)]));
}

#[test]
fn origin_admissible_bounds_the_token_length_inclusively() {
    assert!(origin_admissible(""));
    assert!(origin_admissible(&"a".repeat(MAX_RULER_ORIGIN_BYTES)));
    assert!(!origin_admissible(&"a".repeat(MAX_RULER_ORIGIN_BYTES + 1)));
    // The bound is bytes: a multi-byte scalar counts for every byte it occupies.
    assert!(!origin_admissible(
        &"\u{e9}".repeat(MAX_RULER_ORIGIN_BYTES / 2 + 1)
    ));
    // A client's token is one UUID, comfortably within the bound.
    assert!(origin_admissible("123e4567-e89b-12d3-a456-426614174000"));
}

#[test]
fn ruler_budget_is_its_own_bucket_sized_above_the_client_send_rate() {
    // The most an honest client sends: one update per 200 ms and the stop ending each ruler,
    // 10 frames a second.
    const { assert!(RULER_RATE_PER_MIN >= 10 * 60) };
    let ws = crate::ws::WsState::new();
    for other in [
        &ws.ping_rate,
        &ws.emote_rate,
        &ws.message_rate,
        &ws.audio_rate,
        &ws.vfx_rate,
    ] {
        assert!(
            !Arc::ptr_eq(&ws.ruler_rate, other),
            "ruler_rate must be its own limiter instance"
        );
    }
    let u = crate::test_ids::test_uuid(7);
    for _ in 0..RULER_RATE_PER_MIN {
        assert!(ws.ruler_rate.check(u, 1_000, RULER_RATE_PER_MIN));
    }
    assert!(
        !ws.ruler_rate.check(u, 1_000, RULER_RATE_PER_MIN),
        "the budget is exhausted after RULER_RATE_PER_MIN frames"
    );
    assert!(
        ws.ping_rate.check(u, 1_000, 30),
        "ruler traffic must not spend the ping budget"
    );
}

/// The shared conformance fixture the TypeScript client's `RULER_LIMITS` test also reads: the
/// two sides' limits cannot drift apart.
#[test]
fn ruler_limits_match_the_shared_conformance_fixture() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../client/core/src/__fixtures__/ruler-limits.json"
    ))
    .unwrap();
    assert_eq!(fixture["maxPoints"], MAX_RULER_POINTS);
    assert_eq!(fixture["maxOriginBytes"], MAX_RULER_ORIGIN_BYTES);
    assert_eq!(fixture["ratePerMin"], RULER_RATE_PER_MIN);
    assert_eq!(
        fixture["maxCoord"].as_f64(),
        Some(crate::scene::move_exec::MAX_GATE_WALK_COORD)
    );
}

#[test]
fn canonical_stops_carry_no_points_and_empty_updates_are_stops() {
    let pts = vec![(0.0, 0.0), (1.0, 1.0)];
    assert_eq!(canonical(pts.clone(), true), (pts.clone(), true));
    assert_eq!(canonical(pts, false), (Vec::new(), false));
    assert_eq!(canonical(Vec::new(), true), (Vec::new(), false));
    assert_eq!(canonical(Vec::new(), false), (Vec::new(), false));
}

/// Through the production parse (`serde_json::from_str::<ClientMsg>`): a frame with exactly
/// `MAX_RULER_POINTS` vertices deserializes, and one with a vertex more fails to deserialize.
#[test]
fn over_cap_points_make_a_client_msg_fail_to_deserialize() {
    let frame = |n: usize| {
        let pts = vec![[0.0f64, 0.0f64]; n];
        serde_json::json!({
            "type": "ruler_update", "scene": crate::test_ids::test_uuid(1), "points": pts,
            "active": true
        })
        .to_string()
    };
    assert!(serde_json::from_str::<ClientMsg>(&frame(MAX_RULER_POINTS)).is_ok());
    assert!(serde_json::from_str::<ClientMsg>(&frame(MAX_RULER_POINTS + 1)).is_err());
}

/// The `deserialize_points` visitor on its own accepts exactly `MAX_RULER_POINTS` vertices and
/// rejects a longer sequence.
#[test]
fn deserialize_points_accepts_the_cap_and_rejects_one_more() {
    let seq = |n: usize| (0..n).map(|_| vec![0.0f64, 0.0f64]);
    let at_cap: Result<Vec<(f64, f64)>, ValueError> =
        deserialize_points(SeqDeserializer::new(seq(MAX_RULER_POINTS)));
    assert_eq!(at_cap.unwrap().len(), MAX_RULER_POINTS);
    let over: Result<Vec<(f64, f64)>, ValueError> =
        deserialize_points(SeqDeserializer::new(seq(MAX_RULER_POINTS + 1)));
    assert!(over.is_err());
}
