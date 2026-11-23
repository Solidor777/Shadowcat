//! Inbound frame admission over a real socket: a frame nested past the depth cap, or a
//! non-intent frame past the size cap, is answered with the generic `bad_message` error, and the
//! connection stays usable.

use futures_util::SinkExt;
use shadowcat_test_support::{self as common};
use tokio_tungstenite::tungstenite::Message;

/// Sends `frame` and asserts the next non-presence reply is a `bad_message` error, then proves
/// the connection survived with a `time_ping` round trip.
async fn assert_refused_and_connection_survives(frame: String) {
    let h = common::spawn().await;
    let mut ws = h.connect().await;
    common::drain_until_type(&mut ws, "welcome").await;

    ws.send(Message::Text(frame)).await.unwrap();
    let reply = common::next_non_presence_frame(&mut ws).await;
    assert_eq!(reply["type"], "error", "{reply:?}");
    assert_eq!(reply["code"], "bad_message", "{reply:?}");

    ws.send(Message::Text(
        serde_json::json!({ "type": "time_ping", "client_t0": 1 }).to_string(),
    ))
    .await
    .unwrap();
    let pong = common::drain_until_type(&mut ws, "time_pong").await;
    assert_eq!(pong["type"], "time_pong");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_frame_nested_past_the_depth_cap_is_a_bad_message() {
    // 65 arrays inside the 1-deep envelope: past the 64-level cap, within serde_json's own
    // 128-level limit, so only the explicit cap refuses it.
    let frame = format!(
        r#"{{"type":"send_message","request_id":"{}","channel":"c","content":"x","system":{}0{}}}"#,
        uuid::Uuid::from_u128(1),
        "[".repeat(65),
        "]".repeat(65)
    );
    assert_refused_and_connection_survives(frame).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_non_intent_frame_past_the_size_cap_is_a_bad_message() {
    // 400 KiB of message text: far under the transport's frame cap, past the 320 KiB non-intent
    // cap.
    let frame = format!(
        r#"{{"type":"send_message","request_id":"{}","channel":"c","content":"{}"}}"#,
        uuid::Uuid::from_u128(1),
        "a".repeat(400 * 1024)
    );
    assert_refused_and_connection_survives(frame).await;
}
