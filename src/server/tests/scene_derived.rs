//! E2E: the SceneDerived channel — initial push, coalesced re-eval on scene
//! change, and unknown-channel error — over the real WS server.

mod common;

use common::*;
use futures_util::{SinkExt, StreamExt};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn identity_channel_pushes_on_scene_change() {
    let h = spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome

    // Subscribe to the debug identity channel → initial SceneDerived (count 0).
    ws.send(scene_subscribe(1, "identity")).await.unwrap();
    let first = drain_until_type(&mut ws, "scene_derived").await;
    assert_eq!(first["payload"]["entity_count"], 0);

    // Create a scene + child; after coalescing, expect a SceneDerived with the
    // new count and a computed_at_seq at or past the create's seq.
    ws.send(create_scene_with_children(h.world, 10, &[11]))
        .await
        .unwrap();
    let upd = drain_until_type(&mut ws, "scene_derived").await;
    assert_eq!(upd["payload"]["entity_count"], 2); // scene + child
    assert!(upd["computed_at_seq"].as_i64().unwrap() >= 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_channel_errors() {
    let h = spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome

    ws.send(scene_subscribe(2, "no_such_channel")).await.unwrap();
    let err = drain_until_type(&mut ws, "scene_error").await;
    assert!(err["message"].as_str().unwrap().contains("unknown channel"));
}
