//! Integration-style tests for the audio-monitor WS server: spawns `run_with_monitor` on an
//! ephemeral port against a `FakeMonitor`, then drives it with a real `tokio_tungstenite`
//! client (mirroring `shadowcat_test_support`'s spawn-then-connect shape for the main `/ws`
//! server).

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as TMessage;

use super::super::fake::FakeMonitor;
use super::super::SessionLevel;
use super::run_with_monitor;
use crate::config::AudioMonitorArgs;

/// Binds an ephemeral port and releases it again so `run_with_monitor` can rebind the SAME
/// port (its bound-port report goes to stdout, which this harness cannot capture).
async fn free_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

#[tokio::test]
async fn hello_then_levels_frames_are_watch_filtered() {
    let port = free_port().await;
    let args = AudioMonitorArgs {
        port,
        allow_origin: vec!["http://localhost:30000".to_string()],
        watch: vec!["discord".to_string()],
    };
    let monitor = FakeMonitor::new(vec![Ok(vec![
        SessionLevel {
            process: "discord".to_string(),
            peak: 0.5,
        },
        SessionLevel {
            process: "firefox".to_string(),
            peak: 0.9,
        },
    ])]);
    let server = tokio::spawn(run_with_monitor(args, true, None, Some(Box::new(monitor))));
    tokio::time::sleep(Duration::from_millis(100)).await; // give the bind a moment

    let mut req = format!("ws://127.0.0.1:{port}/levels")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("Origin", "http://localhost:30000".parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .expect("origin allowed");

    let hello = ws.next().await.unwrap().unwrap();
    let hello: serde_json::Value = serde_json::from_str(hello.to_text().unwrap()).unwrap();
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["supported"], true);

    let levels = ws.next().await.unwrap().unwrap();
    let levels: serde_json::Value = serde_json::from_str(levels.to_text().unwrap()).unwrap();
    assert_eq!(levels["type"], "levels");
    assert_eq!(levels["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(levels["sessions"][0]["process"], "discord");

    server.abort();
}

#[tokio::test]
async fn unlisted_origin_is_refused_before_the_hello_frame() {
    let port = free_port().await;
    let args = AudioMonitorArgs {
        port,
        allow_origin: vec!["http://localhost:30000".to_string()],
        watch: vec![],
    };
    let monitor = FakeMonitor::new(vec![Ok(vec![])]);
    let server = tokio::spawn(run_with_monitor(args, true, None, Some(Box::new(monitor))));
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut req = format!("ws://127.0.0.1:{port}/levels")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("Origin", "http://evil.example".parse().unwrap());
    let err = tokio_tungstenite::connect_async(req).await;
    assert!(
        err.is_err(),
        "an unlisted origin must be refused, never upgraded"
    );

    server.abort();
}

#[tokio::test]
async fn a_watch_frame_replaces_the_live_list_without_a_restart() {
    let port = free_port().await;
    let args = AudioMonitorArgs {
        port,
        allow_origin: vec!["http://localhost:30000".to_string()],
        watch: vec!["discord".to_string()],
    };
    let monitor = FakeMonitor::new(vec![Ok(vec![SessionLevel {
        process: "firefox".to_string(),
        peak: 0.9,
    }])]);
    let server = tokio::spawn(run_with_monitor(args, true, None, Some(Box::new(monitor))));
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut req = format!("ws://127.0.0.1:{port}/levels")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("Origin", "http://localhost:30000".parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let _hello = ws.next().await.unwrap().unwrap();
    let first_levels = ws.next().await.unwrap().unwrap();
    let first_levels: serde_json::Value =
        serde_json::from_str(first_levels.to_text().unwrap()).unwrap();
    assert_eq!(first_levels["sessions"].as_array().unwrap().len(), 0); // "firefox" not watched yet

    ws.send(TMessage::Text(
        serde_json::json!({ "type": "watch", "names": ["firefox"] })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    // Drain frames until one reflects the new watch list (the exact next tick may race the
    // watch frame's processing).
    let mut saw_firefox = false;
    for _ in 0..10 {
        let msg = ws.next().await.unwrap().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        if parsed["type"] == "levels"
            && parsed["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["process"] == "firefox")
        {
            saw_firefox = true;
            break;
        }
    }
    assert!(
        saw_firefox,
        "a watch frame must replace the live list without a restart"
    );

    server.abort();
}
