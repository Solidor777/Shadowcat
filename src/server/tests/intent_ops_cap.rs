//! `ClientMsg::Intent.ops.len()` admission cap.
//!
//! `WS_MAX_MESSAGE_BYTES` derives its frame-size budget from `MAX_INTENT_DOCS` on the invariant
//! that no admitted intent carries more ops than that -- this pins the enforcement side of that
//! invariant end-to-end over a real socket, so the two caps cannot silently drift back apart (a
//! frame under the byte cap carrying an unbounded `ops.len()` of tiny documents).

use futures_util::{SinkExt, StreamExt};
use shadowcat::data::repository::Repository;
use shadowcat_test_support::{self as common, create_doc_op, intent_msg, test_doc_id};

/// `MAX_INTENT_DOCS` is private to `ws::conn`; this mirrors it rather than exporting the
/// constant, since the point of the test is to pin observable wire behavior against a number
/// that could otherwise drift from the enforcement site with nothing failing to notice.
const MAX_INTENT_DOCS: usize = 16;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_intent_over_the_op_cap_is_rejected_before_any_op_is_applied() {
    let h = common::spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome

    let ops: Vec<serde_json::Value> = (0..MAX_INTENT_DOCS + 1)
        .map(|i| create_doc_op(h.world, 9_000 + i as u128, None, "item"))
        .collect();
    ws.send(intent_msg(1, serde_json::json!(ops)))
        .await
        .unwrap();

    let Ok(Some(Ok(m))) = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await
    else {
        panic!("no frame received");
    };
    let frame: serde_json::Value = serde_json::from_str(m.to_text().unwrap()).unwrap();
    assert_eq!(frame["type"], "reject");
    assert_eq!(frame["reason"], "invalid");
    assert!(
        frame["detail"]
            .as_str()
            .unwrap()
            .contains(&format!("{}", MAX_INTENT_DOCS)),
        "detail should cite the limit: {frame:?}"
    );

    // Nothing from the over-cap batch reached apply_intent: the only op ever authoritatively
    // committed against this world is the join-time config seed.
    assert_eq!(h.repo.get_document(test_doc_id(9_000)).await.unwrap(), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_intent_at_exactly_the_op_cap_is_not_rejected_for_its_length() {
    let h = common::spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome

    let ops: Vec<serde_json::Value> = (0..MAX_INTENT_DOCS)
        .map(|i| create_doc_op(h.world, 10_000 + i as u128, None, "item"))
        .collect();
    ws.send(intent_msg(2, serde_json::json!(ops)))
        .await
        .unwrap();

    let Ok(Some(Ok(m))) = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await
    else {
        panic!("no frame received");
    };
    let frame: serde_json::Value = serde_json::from_str(m.to_text().unwrap()).unwrap();
    // Confirmed by the broadcast echo of the authored Event, never a Reject -- the op count
    // itself must not be the reason for any rejection at exactly the cap.
    assert_eq!(
        frame["type"], "event",
        "an at-cap intent must not be rejected for its length: {frame:?}"
    );

    assert!(h
        .repo
        .get_document(test_doc_id(10_000))
        .await
        .unwrap()
        .is_some());
}
