//! Admission of one inbound text frame to a `ClientMsg`.
//!
//! `ClientMsg` is internally tagged (`#[serde(tag = "type")]`), and serde buffers every
//! non-tag field of an internally tagged enum into an intermediate `Content` tree before any
//! variant deserializer runs. A bound inside a variant (for example
//! `ws::ruler::deserialize_points`) therefore limits the converted value, never that
//! allocation. `parse_client_msg` bounds the frame BEFORE it reaches the buffering step:
//!
//! - nesting depth, by a byte scan that allocates nothing and stops at the first excess
//!   (`MAX_CLIENT_MSG_DEPTH`);
//! - byte length for every message except `ClientMsg::Intent`, which alone carries documents
//!   (`MAX_NON_INTENT_FRAME_BYTES`; an intent is bounded by the transport's own
//!   `WS_MAX_MESSAGE_BYTES`).
//!
//! The tag is read by `FrameTag`, an ordinary struct, which skips every other field without
//! buffering it.

use serde::de::Error as _;
use serde::Deserialize;

use super::MAX_DOCUMENT_BLOCK_BYTES;
use crate::ws::protocol::ClientMsg;

/// Deepest `[`/`{` nesting a client frame may carry. The envelope is depth 1; an intent's
/// documents nest a few levels inside it. Deliberately below `serde_json`'s own recursion
/// limit (128), so a deeply nested frame is refused by one explicit constant rather than by
/// the parser's internal limit.
pub(super) const MAX_CLIENT_MSG_DEPTH: usize = 64;

/// Largest frame, in bytes, of any message other than `ClientMsg::Intent`. The largest
/// legitimate one is `ClientMsg::SendMessage` with a `system` band at its per-block cap
/// (`MAX_DOCUMENT_BLOCK_BYTES`); 64 KiB covers its text and the JSON envelope.
pub(super) const MAX_NON_INTENT_FRAME_BYTES: usize = MAX_DOCUMENT_BLOCK_BYTES + 64 * 1024;

/// The wire tag of `ClientMsg::Intent` (`rename_all = "snake_case"`).
const INTENT_TAG: &str = "intent";

/// The `type` field of a frame and nothing else; every other field is skipped unbuffered.
#[derive(Deserialize)]
struct FrameTag {
    /// The `ClientMsg` variant name on the wire.
    #[serde(rename = "type")]
    ty: String,
}

/// Whether `text` nests `[`/`{` deeper than `max`. Brackets inside JSON strings do not count
/// (the scan tracks string and escape state). Allocation-free and single pass; stops at the
/// first excess. Every byte of a multi-byte UTF-8 scalar is `>= 0x80`, so it never matches an
/// ASCII structural byte.
fn exceeds_depth(text: &str, max: usize) -> bool {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for &b in text.as_bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > max {
                    return true;
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

/// Parses one inbound text frame into a `ClientMsg`, refusing a frame past
/// `MAX_CLIENT_MSG_DEPTH`, or past `MAX_NON_INTENT_FRAME_BYTES` unless it is an intent, before
/// any of it is buffered. Every refusal is an `Err`, answered like any other malformed frame.
pub(super) fn parse_client_msg(text: &str) -> Result<ClientMsg, serde_json::Error> {
    if exceeds_depth(text, MAX_CLIENT_MSG_DEPTH) {
        return Err(serde_json::Error::custom(format!(
            "frame nests deeper than {MAX_CLIENT_MSG_DEPTH} levels"
        )));
    }
    if text.len() > MAX_NON_INTENT_FRAME_BYTES {
        let tag: FrameTag = serde_json::from_str(text)?;
        if tag.ty != INTENT_TAG {
            return Err(serde_json::Error::custom(format!(
                "a non-intent frame exceeds {MAX_NON_INTENT_FRAME_BYTES} bytes"
            )));
        }
    }
    serde_json::from_str::<ClientMsg>(text)
}
