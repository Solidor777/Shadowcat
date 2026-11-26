//! Tests for the admin port-forwarding routes.

use std::time::Duration;

use super::*;
use crate::portmap::client::RetryPolicy;
use crate::portmap::mapping::Targets;
use crate::portmap::runtime::{PortMapRuntime, STORED_TOGGLE_KEY};
use crate::portmap::test_support::{
    accept_loopback, natpmp_address_reply, natpmp_map_reply, spawn_ssdp, spawn_udp, FakeUdp, Reply,
};

/// A fake NAT-PMP gateway that grants every mapping for an hour.
async fn fake_gateway() -> FakeUdp {
    spawn_udp(|request| match request.get(1) {
        Some(0) => vec![Reply::genuine(natpmp_address_reply(1, [203, 0, 113, 9]))],
        Some(2) => {
            let granted = if request[8..12] == [0; 4] { 0 } else { 3600 };
            vec![Reply::genuine(natpmp_map_reply(2, 30000, 30000, granted))]
        }
        _ => Vec::new(),
    })
    .await
}

/// Targets aimed at `gateway`.
async fn targets_for(gateway: &FakeUdp) -> Targets {
    Targets {
        gateway: Some(gateway.addr),
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

/// A state whose runtime is listening on a non-loopback address and talks to `gateway`.
async fn state_with(config: Config, gateway: &FakeUdp) -> AppState {
    let mut state = initialized_state().await;
    state.config = Arc::new(config.clone());
    state.portmap = PortMapRuntime::with_targets(Arc::new(config), targets_for(gateway).await);
    state
        .portmap
        .activate("10.0.0.5:30000".parse().unwrap(), false)
        .await;
    state
}

#[tokio::test]
async fn a_player_and_an_anonymous_caller_are_refused() {
    let gateway = fake_gateway().await;
    let state = state_with(Config::default(), &gateway).await;
    seed_user(&state, "player").await;
    let player = login_server(&state, "player").await;
    player
        .get("/api/admin/portmap")
        .await
        .assert_status(StatusCode::FORBIDDEN);
    player
        .put("/api/admin/portmap")
        .json(&serde_json::json!({ "enabled": true }))
        .await
        .assert_status(StatusCode::FORBIDDEN);
    anon_server(&state)
        .await
        .get("/api/admin/portmap")
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    assert_eq!(
        state.repo.get_setting(STORED_TOGGLE_KEY).await.unwrap(),
        None
    );
    assert!(gateway.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_admin_reads_a_disabled_status_with_every_field() {
    let gateway = fake_gateway().await;
    let state = state_with(Config::default(), &gateway).await;
    seed_admin(&state, "admin").await;
    let admin = login_server(&state, "admin").await;
    let status: serde_json::Value = admin.get("/api/admin/portmap").await.json();
    assert_eq!(
        status,
        serde_json::json!({
            "enabled": false,
            "locked_by_config": false,
            "state": "disabled",
            "protocol": null,
            "external_address": null,
            "external_port": null,
            "lease_expires_in_secs": null,
            "reason": null,
            "detail": null,
        })
    );
}

#[tokio::test]
async fn the_admin_toggle_is_stored_and_applied_at_once() {
    let gateway = fake_gateway().await;
    let state = state_with(Config::default(), &gateway).await;
    seed_admin(&state, "admin").await;
    let admin = login_server(&state, "admin").await;

    let on: serde_json::Value = admin
        .put("/api/admin/portmap")
        .json(&serde_json::json!({ "enabled": true }))
        .await
        .json();
    assert_eq!(on["enabled"], true);
    assert_eq!(
        state
            .repo
            .get_setting(STORED_TOGGLE_KEY)
            .await
            .unwrap()
            .as_deref(),
        Some("true")
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let status: serde_json::Value = admin.get("/api/admin/portmap").await.json();
            if status["state"] == "mapped" {
                assert_eq!(status["protocol"], "nat_pmp");
                assert_eq!(status["external_address"], "203.0.113.9");
                assert_eq!(status["external_port"], 30000);
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("mapping did not appear");

    let off: serde_json::Value = admin
        .put("/api/admin/portmap")
        .json(&serde_json::json!({ "enabled": false }))
        .await
        .json();
    assert_eq!(off["state"], "disabled");
    assert_eq!(
        state
            .repo
            .get_setting(STORED_TOGGLE_KEY)
            .await
            .unwrap()
            .as_deref(),
        Some("false")
    );
    let deletes = gateway
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.get(1) == Some(&2) && r[8..12] == [0; 4])
        .count();
    assert_eq!(deletes, 1);
}

#[tokio::test]
async fn a_config_locked_toggle_answers_409_naming_the_key_and_stores_nothing() {
    let gateway = fake_gateway().await;
    let config = Config {
        port_forward: Some(false),
        ..Config::default()
    };
    let state = state_with(config, &gateway).await;
    seed_admin(&state, "admin").await;
    let admin = login_server(&state, "admin").await;
    let response = admin
        .put("/api/admin/portmap")
        .json(&serde_json::json!({ "enabled": true }))
        .await;
    response.assert_status(StatusCode::CONFLICT);
    assert!(response.text().contains("port_forward"));
    assert_eq!(
        state.repo.get_setting(STORED_TOGGLE_KEY).await.unwrap(),
        None
    );
    let status: serde_json::Value = admin.get("/api/admin/portmap").await.json();
    assert_eq!(status["locked_by_config"], true);
    assert_eq!(status["enabled"], false);
    assert!(gateway.requests.lock().unwrap().is_empty());
}
