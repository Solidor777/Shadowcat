//! Tests for the enable decision and the mapping task against loopback fakes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::*;
use crate::portmap::client::RetryPolicy;
use crate::portmap::status::PortMapProtocol;
use crate::portmap::test_support::{
    accept_loopback, natpmp_address_reply, natpmp_map_reply, natpmp_refusal, spawn_ssdp, spawn_udp,
    FakeUdp, Reply,
};

/// A non-loopback listener address; nothing binds it, the runtime only reads its port and ip.
fn exposed() -> SocketAddr {
    "10.0.0.5:30000".parse().unwrap()
}

/// Targets aimed at a fake gateway, with millisecond back-offs.
async fn targets(gateway: SocketAddr) -> Targets {
    Targets {
        gateway: Some(gateway),
        ssdp: spawn_ssdp(Vec::new()).await,
        ssdp_window: Duration::from_millis(100),
        accept: accept_loopback,
        retry: RetryPolicy {
            initial: Duration::from_millis(30),
            attempts: 2,
        },
        backoff_initial: Duration::from_millis(40),
        backoff_max: Duration::from_millis(80),
        delete_deadline: Duration::from_secs(2),
    }
}

/// A fake NAT-PMP gateway granting `lifetime` seconds, reporting `address`.
async fn gateway(lifetime: u32, address: [u8; 4]) -> FakeUdp {
    spawn_udp(move |request| match request.get(1) {
        Some(0) => vec![Reply::genuine(natpmp_address_reply(1, address))],
        Some(2) => {
            let granted = if request[8..12] == [0; 4] {
                0
            } else {
                lifetime
            };
            vec![Reply::genuine(natpmp_map_reply(2, 30000, 30000, granted))]
        }
        _ => Vec::new(),
    })
    .await
}

/// Counts the map and delete requests a fake gateway has seen: `(grants, deletes)`.
fn counts(fake: &FakeUdp) -> (usize, usize) {
    let requests = fake.requests.lock().unwrap();
    let maps = requests.iter().filter(|r| r.get(1) == Some(&2));
    let deletes = maps.clone().filter(|r| r[8..12] == [0; 4]).count();
    (maps.count() - deletes, deletes)
}

/// Waits (up to 10 s) until `done` holds for the runtime's status.
async fn wait_for(
    runtime: &PortMapRuntime,
    done: impl Fn(&PortMapStatus) -> bool,
) -> PortMapStatus {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let status = runtime.status();
            if done(&status) {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("status did not reach the expected state")
}

/// Waits (up to 10 s) until `done` holds for the fake gateway's counts.
async fn wait_for_counts(fake: &FakeUdp, done: impl Fn((usize, usize)) -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !done(counts(fake)) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("gateway did not see the expected requests");
}

#[test]
fn effective_enabled_lets_the_config_override_the_stored_toggle_both_ways() {
    let cases = [
        (None, false, false),
        (None, true, true),
        (Some(true), false, true),
        (Some(true), true, true),
        (Some(false), true, false),
        (Some(false), false, false),
    ];
    for (config_value, stored, expected) in cases {
        let config = Config {
            port_forward: config_value,
            ..Config::default()
        };
        assert_eq!(
            effective_enabled(&config, stored),
            expected,
            "config {config_value:?} stored {stored}"
        );
    }
}

#[tokio::test]
async fn the_stored_toggle_defaults_to_off_and_round_trips() {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    assert!(!read_stored_toggle(&repo).await);
    write_stored_toggle(&repo, true).await.unwrap();
    assert!(read_stored_toggle(&repo).await);
    write_stored_toggle(&repo, false).await.unwrap();
    assert!(!read_stored_toggle(&repo).await);
    repo.set_setting(STORED_TOGGLE_KEY, "garbage")
        .await
        .unwrap();
    assert!(!read_stored_toggle(&repo).await);
}

#[tokio::test]
async fn off_by_default_sends_nothing() {
    let fake = gateway(3600, [203, 0, 113, 9]).await;
    let runtime =
        PortMapRuntime::with_targets(Arc::new(Config::default()), targets(fake.addr).await);
    runtime.activate(exposed(), false).await;
    let status = runtime.status();
    assert_eq!(status.state, PortMapState::Disabled);
    assert!(!status.enabled && !status.locked_by_config);
    assert!(fake.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn enabling_maps_and_reports_the_grant() {
    let fake = gateway(3600, [203, 0, 113, 9]).await;
    let runtime =
        PortMapRuntime::with_targets(Arc::new(Config::default()), targets(fake.addr).await);
    runtime.activate(exposed(), true).await;
    let status = wait_for(&runtime, |s| s.state == PortMapState::Mapped).await;
    assert_eq!(status.protocol, Some(PortMapProtocol::NatPmp));
    assert_eq!(status.external_address.as_deref(), Some("203.0.113.9"));
    assert_eq!(status.external_port, Some(30000));
    assert!(status.lease_expires_in_secs.unwrap() > 3000);
    runtime.shutdown().await;
}

#[tokio::test]
async fn a_private_external_address_is_mapped_not_public() {
    let fake = gateway(3600, [192, 168, 1, 50]).await;
    let runtime =
        PortMapRuntime::with_targets(Arc::new(Config::default()), targets(fake.addr).await);
    runtime.activate(exposed(), true).await;
    let status = wait_for(&runtime, |s| s.state == PortMapState::MappedNotPublic).await;
    assert_eq!(status.external_address.as_deref(), Some("192.168.1.50"));
    assert_eq!(status.reason.as_deref(), Some("mapped_not_public"));
    runtime.shutdown().await;
}

#[tokio::test]
async fn the_public_address_is_known_only_for_a_public_mapping() {
    let off = PortMapRuntime::with_targets(
        Arc::new(Config::default()),
        targets(gateway(3600, [203, 0, 113, 9]).await.addr).await,
    );
    off.activate(exposed(), false).await;
    assert_eq!(off.public_address(), None, "a disabled runtime knows none");

    let fake = gateway(3600, [203, 0, 113, 9]).await;
    let runtime =
        PortMapRuntime::with_targets(Arc::new(Config::default()), targets(fake.addr).await);
    runtime.activate(exposed(), true).await;
    wait_for(&runtime, |s| s.state == PortMapState::Mapped).await;
    assert_eq!(
        runtime.public_address(),
        Some("203.0.113.9".parse().unwrap())
    );
    runtime.shutdown().await;

    let private = gateway(3600, [192, 168, 1, 50]).await;
    let behind_nat =
        PortMapRuntime::with_targets(Arc::new(Config::default()), targets(private.addr).await);
    behind_nat.activate(exposed(), true).await;
    wait_for(&behind_nat, |s| s.state == PortMapState::MappedNotPublic).await;
    assert_eq!(
        behind_nat.public_address(),
        None,
        "a router address that is not publicly routable is never a certificate name"
    );
    behind_nat.shutdown().await;
}

#[tokio::test]
async fn a_mapping_is_renewed_at_half_its_lease() {
    let fake = gateway(2, [203, 0, 113, 9]).await;
    let runtime =
        PortMapRuntime::with_targets(Arc::new(Config::default()), targets(fake.addr).await);
    runtime.activate(exposed(), true).await;
    wait_for_counts(&fake, |(grants, _)| grants >= 2).await;
    assert_eq!(runtime.status().state, PortMapState::Mapped);
    runtime.shutdown().await;
}

#[tokio::test]
async fn turning_the_toggle_off_deletes_the_mapping() {
    let fake = gateway(3600, [203, 0, 113, 9]).await;
    let runtime =
        PortMapRuntime::with_targets(Arc::new(Config::default()), targets(fake.addr).await);
    runtime.activate(exposed(), true).await;
    wait_for(&runtime, |s| s.state == PortMapState::Mapped).await;
    runtime.apply(false).await;
    let status = runtime.status();
    assert_eq!(status.state, PortMapState::Disabled);
    assert!(status.external_port.is_none());
    assert_eq!(counts(&fake).1, 1);
}

#[tokio::test]
async fn turning_the_toggle_on_at_runtime_starts_the_task() {
    let fake = gateway(3600, [203, 0, 113, 9]).await;
    let runtime =
        PortMapRuntime::with_targets(Arc::new(Config::default()), targets(fake.addr).await);
    runtime.activate(exposed(), false).await;
    assert_eq!(runtime.status().state, PortMapState::Disabled);
    runtime.apply(true).await;
    wait_for(&runtime, |s| s.state == PortMapState::Mapped).await;
    runtime.shutdown().await;
}

#[tokio::test]
async fn shutdown_deletes_exactly_the_held_mapping_once() {
    let fake = gateway(3600, [203, 0, 113, 9]).await;
    let runtime =
        PortMapRuntime::with_targets(Arc::new(Config::default()), targets(fake.addr).await);
    runtime.activate(exposed(), true).await;
    wait_for(&runtime, |s| s.state == PortMapState::Mapped).await;
    runtime.shutdown().await;
    runtime.shutdown().await;
    assert_eq!(counts(&fake).1, 1);
}

#[tokio::test]
async fn a_config_override_locks_the_toggle_in_both_directions() {
    let fake = gateway(3600, [203, 0, 113, 9]).await;
    let off = Config {
        port_forward: Some(false),
        ..Config::default()
    };
    let runtime = PortMapRuntime::with_targets(Arc::new(off), targets(fake.addr).await);
    runtime.activate(exposed(), true).await;
    let status = runtime.status();
    assert!(status.locked_by_config && !status.enabled);
    assert!(fake.requests.lock().unwrap().is_empty());

    let on = Config {
        port_forward: Some(true),
        ..Config::default()
    };
    let runtime = PortMapRuntime::with_targets(Arc::new(on), targets(fake.addr).await);
    runtime.activate(exposed(), false).await;
    wait_for(&runtime, |s| s.state == PortMapState::Mapped).await;
    assert!(runtime.status().locked_by_config);
    runtime.shutdown().await;
}

#[tokio::test]
async fn a_loopback_listener_is_unreachable_and_maps_nothing() {
    let fake = gateway(3600, [203, 0, 113, 9]).await;
    let runtime =
        PortMapRuntime::with_targets(Arc::new(Config::default()), targets(fake.addr).await);
    runtime
        .activate("127.0.0.1:30000".parse().unwrap(), true)
        .await;
    let status = runtime.status();
    assert_eq!(status.state, PortMapState::UnreachableBind);
    assert_eq!(status.reason.as_deref(), Some("unreachable_bind"));
    assert!(fake.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn enabling_before_the_listener_is_known_reports_not_listening() {
    let fake = gateway(3600, [203, 0, 113, 9]).await;
    let runtime =
        PortMapRuntime::with_targets(Arc::new(Config::default()), targets(fake.addr).await);
    runtime.apply(true).await;
    assert_eq!(runtime.status().reason.as_deref(), Some("not_listening"));
    assert!(fake.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_refusing_router_fails_with_a_reason_and_is_retried() {
    let healthy = std::sync::Arc::new(AtomicBool::new(false));
    let switch = std::sync::Arc::clone(&healthy);
    let fake = spawn_udp(move |request| match request.get(1) {
        Some(0) if switch.load(Ordering::SeqCst) => {
            vec![Reply::genuine(natpmp_address_reply(1, [203, 0, 113, 9]))]
        }
        Some(2) if switch.load(Ordering::SeqCst) => {
            vec![Reply::genuine(natpmp_map_reply(2, 30000, 30000, 3600))]
        }
        Some(_) => vec![Reply::genuine(natpmp_refusal(request, 2))],
        None => Vec::new(),
    })
    .await;
    let runtime =
        PortMapRuntime::with_targets(Arc::new(Config::default()), targets(fake.addr).await);
    runtime.activate(exposed(), true).await;
    let failed = wait_for(&runtime, |s| s.state == PortMapState::Failed).await;
    assert_eq!(failed.reason.as_deref(), Some("refused"));
    healthy.store(true, Ordering::SeqCst);
    wait_for(&runtime, |s| s.state == PortMapState::Mapped).await;
    runtime.shutdown().await;
}

#[tokio::test]
async fn a_failed_renewal_removes_the_mapping_and_rediscovers() {
    let alive = std::sync::Arc::new(AtomicBool::new(true));
    let switch = std::sync::Arc::clone(&alive);
    let fake = spawn_udp(move |request| {
        if !switch.load(Ordering::SeqCst) {
            return Vec::new();
        }
        match request.get(1) {
            Some(0) => vec![Reply::genuine(natpmp_address_reply(1, [203, 0, 113, 9]))],
            Some(2) => vec![Reply::genuine(natpmp_map_reply(2, 30000, 30000, 2))],
            _ => Vec::new(),
        }
    })
    .await;
    let runtime =
        PortMapRuntime::with_targets(Arc::new(Config::default()), targets(fake.addr).await);
    runtime.activate(exposed(), true).await;
    wait_for(&runtime, |s| s.state == PortMapState::Mapped).await;
    alive.store(false, Ordering::SeqCst);
    let status = wait_for(&runtime, |s| s.state == PortMapState::Failed).await;
    assert!(status.external_port.is_none());
    runtime.shutdown().await;
}

#[tokio::test]
async fn activation_returns_while_discovery_is_still_waiting() {
    let silent = spawn_udp(|_| Vec::new()).await;
    let mut slow = targets(silent.addr).await;
    slow.retry = RetryPolicy {
        initial: Duration::from_secs(5),
        attempts: 3,
    };
    let runtime = PortMapRuntime::with_targets(Arc::new(Config::default()), slow);
    let started = std::time::Instant::now();
    runtime.activate(exposed(), true).await;
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(runtime.status().state, PortMapState::Discovering);
    runtime.shutdown().await;
}

#[tokio::test]
async fn shutdown_while_discovering_stops_the_task_without_a_mapping() {
    let silent = spawn_udp(|_| Vec::new()).await;
    let mut slow = targets(silent.addr).await;
    slow.retry = RetryPolicy {
        initial: Duration::from_secs(5),
        attempts: 3,
    };
    let runtime = PortMapRuntime::with_targets(Arc::new(Config::default()), slow);
    runtime.activate(exposed(), true).await;
    runtime.shutdown().await;
    assert!(runtime.shared.slot.lock().unwrap().is_none());
    assert!(runtime.shared.task.lock().await.is_none());
}
