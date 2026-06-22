//! E2E: scene-entity creation and the cascade-delete-as-events invariant over
//! the real WS server.

mod common;

use common::*;
use futures_util::{SinkExt, StreamExt};
use shadowcat::data::repository::Repository;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scene_delete_cascades_as_events() {
    let h = spawn().await;
    let mut ws = h.connect().await;
    // Drain Welcome.
    let _ = ws.next().await;

    // Create a scene + two child tokens (one intent, three create ops).
    ws.send(create_scene_with_children(h.world, 10, &[11, 12]))
        .await
        .unwrap();
    let evt = drain_until_event(&mut ws).await;
    assert_eq!(evt["command"]["ops"].as_array().unwrap().len(), 3);

    // Delete the scene; expect one Event whose command carries 3 Delete ops
    // (scene + both children), never a silent FK cascade.
    ws.send(delete_doc(h.world, 10)).await.unwrap();
    let evt = drain_until_event(&mut ws).await;
    let ops = evt["command"]["ops"].as_array().unwrap();
    assert_eq!(ops.len(), 3);
    assert!(ops.iter().all(|o| o["op"] == "delete"));
    // Authoritative store empty for the scene's children.
    assert!(h
        .repo
        .query_children(Uuid::from_u128(10))
        .await
        .unwrap()
        .is_empty());
}
