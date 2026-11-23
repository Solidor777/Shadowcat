//! These tests check `client_frame::parse_client_msg`'s admission bounds through the same entry
//! point `handle_socket` calls, never through an inner visitor.

use crate::ws::conn::client_frame::{
    parse_client_msg, MAX_CLIENT_MSG_DEPTH, MAX_NON_INTENT_FRAME_BYTES,
};
use crate::ws::protocol::ClientMsg;

/// A `send_message` frame whose `system` band is `levels` arrays deep, so the whole frame nests
/// `levels + 1` deep (the envelope object is depth 1).
fn nested_send_message(levels: usize) -> String {
    format!(
        r#"{{"type":"send_message","request_id":"{}","channel":"c","content":"x","system":{}0{}}}"#,
        crate::test_ids::test_uuid(1),
        "[".repeat(levels),
        "]".repeat(levels)
    )
}

/// A `send_message` frame padded with `content` bytes of text.
fn send_message_with_content(content_bytes: usize) -> String {
    format!(
        r#"{{"type":"send_message","request_id":"{}","channel":"c","content":"{}"}}"#,
        crate::test_ids::test_uuid(1),
        "a".repeat(content_bytes)
    )
}

#[test]
fn a_frame_at_the_depth_cap_parses_and_one_level_deeper_is_refused() {
    let at_cap = nested_send_message(MAX_CLIENT_MSG_DEPTH - 1);
    assert!(
        matches!(parse_client_msg(&at_cap), Ok(ClientMsg::SendMessage { .. })),
        "exactly MAX_CLIENT_MSG_DEPTH levels is admitted"
    );
    let past_cap = nested_send_message(MAX_CLIENT_MSG_DEPTH);
    assert!(
        parse_client_msg(&past_cap).is_err(),
        "one level past MAX_CLIENT_MSG_DEPTH is refused"
    );
}

#[test]
fn the_depth_cap_is_below_the_json_parsers_own_limit() {
    // serde_json refuses past 128 levels itself; the explicit cap must bite first, so a frame
    // between the two limits parses under a bare `from_str` and is refused here.
    let between = nested_send_message(MAX_CLIENT_MSG_DEPTH + 8);
    assert!(
        serde_json::from_str::<ClientMsg>(&between).is_ok(),
        "the frame is within serde_json's own recursion limit"
    );
    assert!(parse_client_msg(&between).is_err());
}

#[test]
fn brackets_inside_strings_and_escapes_do_not_count_toward_depth() {
    let brackets = "[{".repeat(MAX_CLIENT_MSG_DEPTH * 2);
    let frame = format!(
        r#"{{"type":"send_message","request_id":"{}","channel":"c","content":"{brackets}\"{brackets}\\"}}"#,
        crate::test_ids::test_uuid(1),
    );
    assert!(
        matches!(parse_client_msg(&frame), Ok(ClientMsg::SendMessage { .. })),
        "string content, including an escaped quote and an escaped backslash, is not structure"
    );
}

#[test]
fn a_non_intent_frame_past_the_size_cap_is_refused_and_one_at_the_cap_parses() {
    let overhead = send_message_with_content(0).len();
    let at_cap = send_message_with_content(MAX_NON_INTENT_FRAME_BYTES - overhead);
    assert_eq!(at_cap.len(), MAX_NON_INTENT_FRAME_BYTES);
    assert!(
        parse_client_msg(&at_cap).is_ok(),
        "a frame of exactly MAX_NON_INTENT_FRAME_BYTES is admitted"
    );
    let past_cap = send_message_with_content(MAX_NON_INTENT_FRAME_BYTES - overhead + 1);
    assert!(
        parse_client_msg(&past_cap).is_err(),
        "one byte past MAX_NON_INTENT_FRAME_BYTES is refused"
    );
}

#[test]
fn an_intent_frame_may_exceed_the_non_intent_cap() {
    // An unknown field is ignored by `ClientMsg::Intent`'s deserializer, so it pads the frame
    // without changing its meaning.
    let frame = format!(
        r#"{{"type":"intent","intent_id":"{}","ops":[],"pad":"{}"}}"#,
        crate::test_ids::test_uuid(2),
        "a".repeat(MAX_NON_INTENT_FRAME_BYTES * 2)
    );
    assert!(frame.len() > MAX_NON_INTENT_FRAME_BYTES);
    assert!(
        matches!(parse_client_msg(&frame), Ok(ClientMsg::Intent { .. })),
        "an intent is bounded by the transport frame cap, not the non-intent cap"
    );
}

#[test]
fn an_oversized_frame_without_a_readable_tag_is_refused() {
    let no_tag = format!(r#"{{"pad":"{}"}}"#, "a".repeat(MAX_NON_INTENT_FRAME_BYTES));
    assert!(parse_client_msg(&no_tag).is_err());
    let unknown = format!(
        r#"{{"type":"no_such_message","pad":"{}"}}"#,
        "a".repeat(MAX_NON_INTENT_FRAME_BYTES)
    );
    assert!(parse_client_msg(&unknown).is_err());
}
