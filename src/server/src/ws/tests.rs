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

#[test]
fn the_vfx_bucket_is_a_distinct_limiter_from_ping_emote_and_message() {
    // Structural separation pin: pointing any of the four fields at the same Arc (the failure
    // mode where a VFX burst starves chat, or a refactor re-shares one counter) fails here.
    let ws = WsState::new();
    for other in [&ws.ping_rate, &ws.emote_rate, &ws.message_rate] {
        assert!(
            !Arc::ptr_eq(&ws.vfx_rate, other),
            "vfx_rate must be its own limiter instance"
        );
    }
    // And the /fx + raw-frame call sites both charge 30/min/user — the same budget the ping
    // limiter test above exercises; a budget edit on either side belongs in BOTH arms (see
    // `WsState::vfx_rate`'s own doc).
    let ws2 = WsState::with_broadcast_capacity(1);
    for other in [&ws2.ping_rate, &ws2.emote_rate, &ws2.message_rate] {
        assert!(!Arc::ptr_eq(&ws2.vfx_rate, other));
    }
}

#[test]
fn vfx_rate_is_the_same_instance_rooms_charges_for_trigger_fired_plays() {
    // `WsState.vfx_rate` and `RoomRegistry::vfx_rate()` (which every `Room` it creates clones
    // into its own trigger-firing path) must be the identical `Arc<PingRateLimiter>` — never two
    // separately-constructed limiters — or a region-trigger-fired VFX would draw from a parallel
    // bucket instead of the one the raw `PlayVfx` frame and `/fx` already share.
    let ws = WsState::new();
    assert!(Arc::ptr_eq(&ws.vfx_rate, &ws.rooms.vfx_rate()));

    let ws2 = WsState::with_broadcast_capacity(1);
    assert!(Arc::ptr_eq(&ws2.vfx_rate, &ws2.rooms.vfx_rate()));
}
