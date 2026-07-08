//! Chat domain: the server-authoritative message model and ingest.
//!
//! Messages are ordinary sequenced `Document`s with an opaque `system` body
//! (this module's `MessageSystem`), authored ONLY by the server from a
//! `SendMessage` intent — never built by a client. INVARIANT: a `message`
//! doc_type reaches `apply_intent` only via `handle_send_message`; the ingress
//! guard (`ops_target_message`) rejects any client-authored message op.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Top-level doc_type for chat messages.
pub const MESSAGE_DOC_TYPE: &str = "message";

/// Attribution of a message to an actor: a linked canonical `Actor` document,
/// or an instanced actor resolved through its token. Carried on the
/// `SendMessage` frame and stored in `MessageSystem`. No ID newtypes exist —
/// identifiers are bare `Uuid` (rendered `string` in TS).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActorOwnerRef {
    Actor { actor_id: Uuid },
    TokenInstance { token_id: Uuid },
}

/// Message subtype, orthogonal to channel. Rides the opaque body (no ts-rs).
/// c-1 only ever produces `Normal`; `Emote`/`Roll` are set by c-3's command
/// parser, `System` by server-authored notices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    #[default]
    Normal,
    Emote,
    Roll,
    System,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn actor_owner_ref_tagged_roundtrip() {
        let a = ActorOwnerRef::Actor {
            actor_id: Uuid::from_u128(1),
        };
        let j = serde_json::to_value(&a).unwrap();
        assert_eq!(j["kind"], "actor");
        assert_eq!(a, serde_json::from_value(j).unwrap());

        let t = ActorOwnerRef::TokenInstance {
            token_id: Uuid::from_u128(2),
        };
        let j = serde_json::to_value(&t).unwrap();
        assert_eq!(j["kind"], "token_instance");
        assert_eq!(t, serde_json::from_value(j).unwrap());
    }

    #[test]
    fn message_kind_defaults_normal_snake_case() {
        assert_eq!(MessageKind::default(), MessageKind::Normal);
        assert_eq!(
            serde_json::to_value(MessageKind::System).unwrap(),
            serde_json::json!("system")
        );
    }
}
