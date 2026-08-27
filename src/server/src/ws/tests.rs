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
