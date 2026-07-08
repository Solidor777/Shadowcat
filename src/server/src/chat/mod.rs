//! Chat domain: the server-authoritative message model and ingest.
//!
//! Messages are ordinary sequenced `Document`s with an opaque `system` body
//! (this module's `MessageSystem`), authored ONLY by the server from a
//! `SendMessage` intent — never built by a client. INVARIANT: a `message`
//! doc_type reaches `apply_intent` only via `handle_send_message`; the ingress
//! guard (`ops_target_message`) rejects any client-authored message op.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::data::document::{DocRole, Document, PermissionSet, Scope};

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

/// One piece of a message's sanitized content model. Serialized into the
/// message's opaque `system` body (no ts-rs — M11d declares its own Zod mirror).
/// Extensible: later checkpoints add the variants they produce (c-3 marks/links/
/// images, c-4 preview cards, M11d roll embeds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Segment {
    /// Literal text. Rendered as a DOM text node by the client (never innerHTML),
    /// so any markup it contains is inert.
    Text { text: String },
}

/// The c-1 producer: wrap raw input as a single literal-text segment. Rich
/// producers (markdown/HTML) are added in c-3, feeding this same content model.
pub fn plain_text_content(raw: &str) -> Vec<Segment> {
    vec![Segment::Text {
        text: raw.to_string(),
    }]
}

/// The message document's `system` body. Opaque on the wire (no ts-rs); the
/// client declares its own Zod mirror in M11d. `recipients` (whispers) is added
/// in c-2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSystem {
    pub channel: String,
    /// The owning user; server-set to the authenticated poster (== `Document.owner`).
    pub user_owner: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_owner: Option<ActorOwnerRef>,
    pub kind: MessageKind,
    pub content: Vec<Segment>,
}

/// Server-construct a message `Document`. INVARIANT: only the server calls this
/// (via `handle_send_message`); clients never build message docs. Sets the
/// author as `owner` + `Owner` permission (satisfies the create WRITE_FIELDS
/// floor) with `default = Observer` so all world members may read it.
pub fn build_message_doc(
    world_id: Uuid,
    user: Uuid,
    channel: String,
    actor_owner: Option<ActorOwnerRef>,
    content: Vec<Segment>,
    now: i64,
) -> Document {
    let mut users = BTreeMap::new();
    users.insert(user, DocRole::Owner);
    let system = MessageSystem {
        channel,
        user_owner: user,
        actor_owner,
        kind: MessageKind::Normal,
        content,
    };
    Document {
        id: Uuid::new_v4(),
        scope: Scope::World { world_id },
        doc_type: MESSAGE_DOC_TYPE.to_string(),
        schema_version: 1,
        source: None,
        owner: Some(user),
        permissions: PermissionSet {
            default: DocRole::Observer,
            users,
            ..Default::default()
        },
        embedded: BTreeMap::new(),
        parent_id: None,
        system: serde_json::to_value(system).expect("MessageSystem serializes"),
        created_at: now,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::document::{DocRole, Scope};
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

    #[test]
    fn plain_text_produces_single_text_segment() {
        let segs = plain_text_content("hello <b>world</b>");
        assert_eq!(
            segs,
            vec![Segment::Text {
                text: "hello <b>world</b>".into()
            }]
        );
        // Producer stores raw text verbatim; markup is inert data, rendered as text (M11d).
        let j = serde_json::to_value(&segs[0]).unwrap();
        assert_eq!(j["kind"], "text");
        assert_eq!(j["text"], "hello <b>world</b>");
    }

    #[test]
    fn plain_text_empty_is_empty_segment() {
        assert_eq!(
            plain_text_content(""),
            vec![Segment::Text {
                text: String::new()
            }]
        );
    }

    #[test]
    fn build_message_doc_is_server_owned_message() {
        let world = Uuid::from_u128(10);
        let user = Uuid::from_u128(20);
        let doc = build_message_doc(
            world,
            user,
            "all".into(),
            None,
            plain_text_content("hi"),
            1234,
        );
        assert_eq!(doc.doc_type, MESSAGE_DOC_TYPE);
        assert_eq!(doc.owner, Some(user));
        assert_eq!(doc.scope, Scope::World { world_id: world });
        assert_eq!(doc.created_at, 1234);
        // Author gets the Owner floor so the create WRITE_FIELDS check passes;
        // default Observer so every world member can read it.
        assert_eq!(doc.permissions.default, DocRole::Observer);
        assert_eq!(doc.permissions.users.get(&user), Some(&DocRole::Owner));
        // Body round-trips back to a MessageSystem with server-set user_owner.
        let sys: MessageSystem = serde_json::from_value(doc.system.clone()).unwrap();
        assert_eq!(sys.user_owner, user);
        assert_eq!(sys.channel, "all");
        assert_eq!(sys.kind, MessageKind::Normal);
        assert_eq!(sys.content, vec![Segment::Text { text: "hi".into() }]);
    }
}
