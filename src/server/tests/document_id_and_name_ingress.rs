//! Create-path ingress checks over a real WS socket: `Document.id` must carry a v4 or v7 UUID
//! (`data::validation::validate_document_id`), and `Document.name` must not exceed
//! `data::validation::MAX_DOCUMENT_NAME_BYTES` -- both enforced at `validate_system_size`'s single
//! chokepoint inside `apply_intent`'s Create-op validation, so this pins the wire-visible
//! rejection rather than only the unit-level validator.

use futures_util::{SinkExt, StreamExt};
use shadowcat::data::repository::Repository;
use shadowcat_test_support::{self as common, intent_msg};
use uuid::Uuid;

/// A raw `create` op for a minimal `actor` document with an explicit `id` and `name`, bypassing
/// `common::create_doc_op`'s own id derivation so nil/v1/v4/v7 ids can be exercised directly.
fn create_actor_with_id_and_name(world: Uuid, id: Uuid, name: &str) -> serde_json::Value {
    serde_json::json!({
        "op": "create",
        "doc": {
            "id": id,
            "scope": { "kind": "world", "world_id": world },
            "doc_type": "actor",
            "schema_version": 1,
            "name": name,
            "engine": {
                "displayName": "Test",
                "visual": { "kind": "image", "asset": "11111111-1111-1111-1111-111111111111" },
                "size": { "w": 1.0, "h": 1.0 },
                "shape": "square",
                "faction": null,
                "conditions": [],
                "prototype": true
            },
            "system": {},
            "created_at": 0,
            "updated_at": 0,
        }
    })
}

async fn send_create_and_recv_frame(id: Uuid, name: &str) -> (serde_json::Value, common::Harness) {
    let h = common::spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome
    ws.send(intent_msg(
        1,
        serde_json::json!([create_actor_with_id_and_name(h.world, id, name)]),
    ))
    .await
    .unwrap();
    let Ok(Some(Ok(m))) = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await
    else {
        panic!("no frame received");
    };
    let frame: serde_json::Value = serde_json::from_str(m.to_text().unwrap()).unwrap();
    (frame, h)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_nil_document_id_is_rejected() {
    let (frame, h) = send_create_and_recv_frame(Uuid::nil(), "Nil").await;
    assert_eq!(frame["type"], "reject");
    assert_eq!(frame["reason"], "invalid");
    assert!(
        h.repo.get_document(Uuid::nil()).await.unwrap().is_none(),
        "a rejected create must never persist"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_v1_document_id_is_rejected() {
    // A time-based (v1) UUID: version nibble 0x1, RFC4122 variant.
    let v1 = Uuid::from_bytes([
        0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x81, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
        0x11,
    ]);
    let (frame, _h) = send_create_and_recv_frame(v1, "V1").await;
    assert_eq!(frame["type"], "reject");
    assert_eq!(frame["reason"], "invalid");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_v4_document_id_is_accepted() {
    let id = Uuid::new_v4();
    let (frame, h) = send_create_and_recv_frame(id, "V4").await;
    assert_eq!(
        frame["type"], "event",
        "a valid v4 id must not be rejected: {frame:?}"
    );
    assert!(h.repo.get_document(id).await.unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_v7_document_id_is_accepted() {
    // The `uuid` crate's `v7` feature is not enabled in this workspace; build one by hand --
    // version nibble 0x7, RFC4122 variant, matching `Version::SortRand`'s bit layout.
    let id = Uuid::from_bytes([
        0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x72, 0x22, 0x82, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22,
        0x22,
    ]);
    let (frame, h) = send_create_and_recv_frame(id, "V7").await;
    assert_eq!(
        frame["type"], "event",
        "a valid v7 id must not be rejected: {frame:?}"
    );
    assert!(h.repo.get_document(id).await.unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_name_over_the_byte_cap_is_rejected() {
    let over = "x".repeat(shadowcat::data::validation::MAX_DOCUMENT_NAME_BYTES + 1);
    let (frame, h) = send_create_and_recv_frame(Uuid::new_v4(), &over).await;
    assert_eq!(frame["type"], "reject");
    assert_eq!(frame["reason"], "invalid");
    assert_eq!(
        h.repo.events_since(h.world, 0).await.unwrap().len(),
        1,
        "a rejected create adds nothing beyond the join-time config seed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_name_at_exactly_the_byte_cap_is_not_rejected_for_its_length() {
    let at_cap = "x".repeat(shadowcat::data::validation::MAX_DOCUMENT_NAME_BYTES);
    let id = Uuid::new_v4();
    let (frame, h) = send_create_and_recv_frame(id, &at_cap).await;
    assert_eq!(
        frame["type"], "event",
        "a name at exactly the cap must not be rejected: {frame:?}"
    );
    assert!(h.repo.get_document(id).await.unwrap().is_some());
}
