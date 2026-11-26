//! A refused admission is visible in the log as one bounded summary, however many peers are
//! refused.
//!
//! The test runs on the default current-thread runtime so the log capture sees the lines.

use std::sync::Arc;

use shadowcat::http::peer_slots::PeerSlots;
use shadowcat_test_support::logs::capture_current_thread;

#[test]
fn many_refusals_produce_one_warning_naming_the_cap_and_the_last_peer_key() {
    let (_guard, log) = capture_current_thread();
    let slots = Arc::new(PeerSlots::new(1, 1).named("live connections"));
    let _held = slots.admit("192.0.2.1".parse().unwrap(), false).unwrap();
    for n in 0..50u8 {
        let peer = std::net::IpAddr::from([198, 51, 100, n]);
        assert!(slots.admit(peer, false).is_none());
    }
    let text = log.text();
    assert_eq!(
        text.matches("connection cap reached").count(),
        1,
        "refusals must be summarised, not logged one by one:\n{text}"
    );
    assert!(text.contains("live connections"), "{text}");
    assert!(text.contains("peer=198.51.100.0"), "{text}");
    assert!(text.contains("refused=1"), "{text}");
}

#[test]
fn an_admitted_connection_logs_nothing() {
    let (_guard, log) = capture_current_thread();
    let slots = Arc::new(PeerSlots::new(4, 4));
    let _held = slots.admit("192.0.2.1".parse().unwrap(), false).unwrap();
    assert!(!log.text().contains("connection cap reached"));
}

#[test]
fn a_per_peer_refusal_is_logged_like_a_global_one() {
    let (_guard, log) = capture_current_thread();
    let slots = Arc::new(PeerSlots::new(10, 1).named("pre-hand-off connections"));
    let peer = "192.0.2.7".parse().unwrap();
    let _held = slots.admit(peer, false).unwrap();
    assert!(slots.admit(peer, false).is_none());
    let text = log.text();
    assert!(text.contains("connection cap reached"), "{text}");
    assert!(text.contains("PerPeer"), "{text}");
    assert!(text.contains("peer=192.0.2.7"), "{text}");
}

#[test]
fn a_per_peer_refusal_names_trusted_proxies_as_the_remedy_and_a_global_one_does_not() {
    let (_guard, log) = capture_current_thread();
    let per_peer = Arc::new(PeerSlots::new(10, 1).named("live connections"));
    let peer = "192.0.2.7".parse().unwrap();
    let _held = per_peer.admit(peer, false).unwrap();
    assert!(per_peer.admit(peer, false).is_none());
    let text = log.text();
    assert!(text.contains("PerPeer"), "{text}");
    assert!(
        text.contains("list it in `trusted_proxies`"),
        "the per-peer refusal omits the remedy:
{text}"
    );

    let (_guard, log) = capture_current_thread();
    let global = Arc::new(PeerSlots::new(1, 10).named("live connections"));
    let _held = global.admit("192.0.2.1".parse().unwrap(), false).unwrap();
    assert!(global.admit("192.0.2.2".parse().unwrap(), false).is_none());
    let text = log.text();
    assert!(text.contains("Global"), "{text}");
    assert!(
        !text.contains("trusted_proxies"),
        "a global-cap refusal carries per-peer advice:
{text}"
    );
}
