//! Keying, caps and release of [`PeerSlots`].

use super::*;

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

fn slots(global_cap: usize, per_peer_cap: u32) -> Arc<PeerSlots> {
    Arc::new(PeerSlots::new(global_cap, per_peer_cap))
}

#[test]
fn ipv6_peers_in_one_slash_64_share_a_cap() {
    let adm = slots(100, 2);
    let _a = adm.admit(ip("2001:db8:1:2::1"), false).unwrap();
    let _b = adm
        .admit(ip("2001:db8:1:2:ffff:ffff:ffff:ffff"), false)
        .unwrap();
    assert!(
        adm.admit(ip("2001:db8:1:2:1234::9"), false).is_none(),
        "a third address of the same /64 must hit the cap"
    );
    // A neighbouring /64 has its own budget.
    assert!(adm.admit(ip("2001:db8:1:3::1"), false).is_some());
}

#[test]
fn ipv4_mapped_peers_count_as_their_ipv4_address() {
    let adm = slots(100, 1);
    let _a = adm.admit(ip("192.0.2.9"), false).unwrap();
    assert!(
        adm.admit(ip("::ffff:192.0.2.9"), false).is_none(),
        "the mapped form is the same host"
    );
    assert!(adm.admit(ip("192.0.2.10"), false).is_some());
}

#[test]
fn many_synthetic_peers_each_get_their_own_budget_and_release_it() {
    let adm = slots(1000, 3);
    let mut held = Vec::new();
    for host in 1..=50u8 {
        let v4 = ip(&format!("198.51.100.{host}"));
        let v6 = ip(&format!("2001:db8:{host:x}::1"));
        for peer in [v4, v6] {
            for _ in 0..3 {
                held.push(adm.admit(peer, false).expect("within the per-peer cap"));
            }
            assert!(adm.admit(peer, false).is_none(), "{peer} is over its cap");
        }
    }
    assert_eq!(adm.held(), 300);
    held.clear();
    assert_eq!(adm.held(), 0);
    assert!(adm.per_peer.is_empty(), "every per-peer entry is released");
}

#[test]
fn an_exempt_peer_skips_the_per_peer_cap_but_not_the_global_cap() {
    let adm = slots(3, 1);
    let _a = adm.admit(ip("10.0.0.1"), true).unwrap();
    let _b = adm.admit(ip("10.0.0.1"), true).unwrap();
    let _c = adm.admit(ip("10.0.0.1"), true).unwrap();
    assert!(
        adm.admit(ip("10.0.0.1"), true).is_none(),
        "global cap binds"
    );
}

#[test]
fn a_refused_per_peer_admission_returns_its_global_reservation() {
    let adm = slots(5, 1);
    let _a = adm.admit(ip("192.0.2.1"), false).unwrap();
    assert!(adm.admit(ip("192.0.2.1"), false).is_none());
    assert_eq!(
        adm.held(),
        1,
        "the refused attempt must not leak a global slot"
    );
}

#[test]
fn the_first_refusal_warns_and_later_ones_are_counted_until_the_interval_passes() {
    let mut r = Refusals::default();
    let start = Instant::now();
    assert_eq!(
        r.record(start),
        Some(1),
        "the first refusal is reported at once"
    );
    assert_eq!(r.record(start + Duration::from_secs(1)), None);
    assert_eq!(r.record(start + Duration::from_secs(9)), None);
    assert_eq!(
        r.record(start + REFUSAL_WARN_INTERVAL),
        Some(3),
        "the next warning carries every refusal since the last one"
    );
    assert_eq!(
        r.record(start + REFUSAL_WARN_INTERVAL + Duration::from_secs(1)),
        None,
        "the count restarts after a warning"
    );
}

#[test]
fn refusal_state_does_not_grow_with_the_number_of_distinct_peers() {
    let adm = Arc::new(PeerSlots::new(1, 1));
    let _held = adm.admit(ip("192.0.2.1"), false).unwrap();
    for n in 0..5_000u32 {
        let peer = IpAddr::from([198, 51, (n >> 8) as u8, n as u8]);
        assert!(adm.admit(peer, false).is_none());
    }
    // Only the held slot's own key is tracked; refused peers leave nothing behind.
    assert_eq!(adm.per_peer.len(), 1);
}
