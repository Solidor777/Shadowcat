use super::*;
use uuid::Uuid;

#[test]
fn ping_limit_is_shared_across_connections_per_user() {
    let lim = PingRateLimiter::new();
    let u = Uuid::from_u128(1);
    for i in 0..30 {
        assert!(lim.check(u, 1_000 + i, 30), "first 30 allowed");
    }
    assert!(!lim.check(u, 1_031, 30), "31st in window denied (per-user)");
    // A different user has an independent budget.
    assert!(lim.check(Uuid::from_u128(2), 1_032, 30));
}

#[test]
fn audio_rate_is_its_own_bucket_and_caps_at_audio_rate_per_min() {
    let state = WsState::new();
    let u = Uuid::from_u128(1);
    for i in 0..crate::ws::AUDIO_RATE_PER_MIN {
        assert!(
            state
                .audio_rate
                .check(u, 1_000 + i as i64, crate::ws::AUDIO_RATE_PER_MIN),
            "first {i} allowed"
        );
    }
    assert!(
        !state
            .audio_rate
            .check(u, 1_100, crate::ws::AUDIO_RATE_PER_MIN),
        "the transport budget is exhausted"
    );
    // The other buckets are untouched by transport spam (and vice versa — one shared
    // limiter would let either flood starve the other).
    assert!(state.ping_rate.check(u, 1_101, 30));
    assert!(state.emote_rate.check(u, 1_101, 30));
    assert!(state.message_rate.check(u, 1_101, 30));
}
