use super::*;

#[test]
fn trips_after_budget_then_window_slides() {
    let t = AuthThrottle::new();
    assert!(t.check("login:u:alice", 1_000, 2));
    assert!(t.check("login:u:alice", 1_500, 2));
    assert!(!t.check("login:u:alice", 1_800, 2)); // 3rd within 60s window
    assert!(t.check("login:u:alice", 62_001, 2)); // window slid
}

#[test]
fn keys_are_independent() {
    let t = AuthThrottle::new();
    assert!(t.check("login:u:alice", 1_000, 1));
    assert!(t.check("login:u:bob", 1_000, 1));
    assert!(!t.check("login:u:alice", 1_100, 1));
}

#[test]
fn at_capacity_new_keys_fail_closed_until_expired_keys_are_swept() {
    let t = AuthThrottle::with_capacity_for_test(2);
    assert!(t.check("k1", 1_000, 5));
    assert!(t.check("k2", 1_000, 5));
    // Map full, new key, nothing expired → fail closed (throttled).
    assert!(!t.check("k3", 1_500, 5));
    // 61s later k1/k2's hits are expired; the sweep frees room.
    assert!(t.check("k3", 62_000, 5));
}

#[test]
fn resolve_client_ip_trusted_peer_no_header_returns_peer() {
    let cfg = crate::config::Config {
        trusted_proxies: vec!["10.0.0.1".into()],
        ..Default::default()
    };
    let peer: std::net::IpAddr = "10.0.0.1".parse().unwrap();
    assert_eq!(resolve_client_ip(peer, None, &cfg), peer);
}

#[test]
fn resolve_client_ip_single_hop_header_returns_the_client() {
    let cfg = crate::config::Config {
        trusted_proxies: vec!["10.0.0.1".into()],
        ..Default::default()
    };
    let peer: std::net::IpAddr = "10.0.0.1".parse().unwrap();
    assert_eq!(
        resolve_client_ip(peer, Some("203.0.113.9"), &cfg),
        "203.0.113.9".parse::<std::net::IpAddr>().unwrap()
    );
}

#[test]
fn resolve_client_ip_walks_past_trusted_hops_to_the_untrusted_entry() {
    let cfg = crate::config::Config {
        trusted_proxies: vec!["10.0.0.1".into()],
        ..Default::default()
    };
    let peer: std::net::IpAddr = "10.0.0.1".parse().unwrap();
    assert_eq!(
        resolve_client_ip(peer, Some("203.0.113.9, 10.0.0.1"), &cfg),
        "203.0.113.9".parse::<std::net::IpAddr>().unwrap()
    );
}

// Discrimination check for the all-hops-trusted fallback: every header entry is itself a
// configured trusted proxy, so the walk must exhaust and fall back to `peer`, not return the
// last-scanned (leftmost) trusted entry. A version of `resolve_client_ip` that returned the
// last scanned trusted entry instead of falling back to `peer` would return `10.0.0.1` here
// (the leftmost/last-scanned entry) rather than `peer` (`10.0.0.1` too, since both proxies
// share the same address in this fixture) — verified below with a second fixture using two
// DISTINCT trusted addresses so the two outcomes are actually distinguishable.
#[test]
fn resolve_client_ip_falls_back_to_peer_when_every_hop_is_trusted() {
    let cfg = crate::config::Config {
        trusted_proxies: vec!["10.0.0.1".into(), "10.0.0.2".into()],
        ..Default::default()
    };
    let peer: std::net::IpAddr = "10.0.0.1".parse().unwrap();
    // Header's sole entry is the OTHER trusted proxy (10.0.0.2), distinct from `peer`
    // (10.0.0.1) — a last-scanned-trusted-entry implementation would return 10.0.0.2, while
    // the fall-back-to-peer implementation returns 10.0.0.1. The two are different addresses,
    // so this fixture actually discriminates between the two behaviors.
    assert_eq!(resolve_client_ip(peer, Some("10.0.0.2"), &cfg), peer);
}

#[test]
fn resolve_client_ip_malformed_entry_falls_back_to_peer() {
    let cfg = crate::config::Config {
        trusted_proxies: vec!["10.0.0.1".into()],
        ..Default::default()
    };
    let peer: std::net::IpAddr = "10.0.0.1".parse().unwrap();
    assert_eq!(resolve_client_ip(peer, Some("not-an-ip"), &cfg), peer);
}

#[test]
fn resolve_client_ip_empty_or_whitespace_header_falls_back_to_peer() {
    let cfg = crate::config::Config {
        trusted_proxies: vec!["10.0.0.1".into()],
        ..Default::default()
    };
    let peer: std::net::IpAddr = "10.0.0.1".parse().unwrap();
    assert_eq!(resolve_client_ip(peer, Some(""), &cfg), peer);
    assert_eq!(resolve_client_ip(peer, Some("  "), &cfg), peer);
}
