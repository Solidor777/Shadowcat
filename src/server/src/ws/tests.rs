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
