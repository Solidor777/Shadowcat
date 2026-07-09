//! Chat domain: the server-authoritative message model and ingest.
//!
//! Messages are ordinary sequenced `Document`s with an opaque `system` body
//! (this module's `MessageSystem`), authored ONLY by the server from a
//! `SendMessage` intent — never built by a client. INVARIANT: a `message`
//! doc_type reaches `apply_intent` only via `handle_send_message`. Two
//! chokepoints jointly enforce this: the ingress guard (`ops_target_message`)
//! rejects any client-authored `message` Create/Delete op at the WS/HTTP
//! boundary; `apply_intent`'s `Update` branch separately rejects every
//! `Update` targeting a stored `message` doc (Updates carry no `doc_type`, so
//! they cannot be classified by `ops_target_message` and must be blocked
//! against the authoritative stored document instead).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::data::command::{Command, Operation};
use crate::data::document::{DocRole, Document, PermissionSet, Scope};
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::DataError;
use crate::ws::room::Room;
use crate::ws::PingRateLimiter;

/// Top-level doc_type for chat messages.
pub const MESSAGE_DOC_TYPE: &str = "message";

/// True if any op authors a `message` doc via the generic document path.
/// Clients must NOT author messages (only `handle_send_message` may); the WS
/// `Intent` and HTTP write paths reject ops for which this is true, keeping
/// message ingest server-authoritative.
///
/// `Operation::Update` carries no `doc_type` (just `doc_id` + field changes),
/// so it cannot be classified here; every `Update` targeting a stored
/// `message` doc is instead rejected in `apply_intent`'s `Update` branch
/// (classified there against the authoritative stored `doc_type`), since c-1
/// has no legitimate message-edit path.
pub fn ops_target_message(ops: &[Operation]) -> bool {
    ops.iter().any(|op| match op {
        Operation::Create { doc } | Operation::Delete { doc } => doc.doc_type == MESSAGE_DOC_TYPE,
        Operation::Update { .. } => false,
    })
}

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

/// The intended readership of a message, beyond the ordinary world-readable
/// default. Carried on the `SendMessage` frame and stored verbatim in
/// `MessageSystem`; drives the document's `PermissionSet` in
/// `build_message_doc` (see that function for the exact mapping). `channel`
/// stays a purely client-chosen label — the server never validates it or
/// derives audience from it; a client module choosing to post into a "GM"
/// channel is what sets `Audience::GmOnly`, not the channel string itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Audience {
    /// Every world member may read (c-1's original, unrestricted shape).
    #[default]
    Public,
    /// Only `recipients` (plus the sender) may read. The GM reads it ONLY if
    /// their own uuid is among `recipients` — not automatically.
    Whisper { recipients: Vec<Uuid> },
    /// Only whoever currently holds `WorldRole::Gm` (plus the sender) may
    /// read — resolved dynamically, not a frozen roster at send time.
    GmOnly,
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
/// client declares its own Zod mirror in M11d.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSystem {
    pub channel: String,
    /// The owning user; server-set to the authenticated poster (== `Document.owner`).
    pub user_owner: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_owner: Option<ActorOwnerRef>,
    pub kind: MessageKind,
    pub audience: Audience,
    pub content: Vec<Segment>,
}

/// Server-construct a message `Document`. INVARIANT: only the server calls
/// this (via `handle_send_message`); clients never build message docs.
/// `audience` drives the document's `PermissionSet`:
/// - `Public` — `default: Observer`, `gm_role: None` (c-1's original,
///   world-readable shape; the GM's unconditional access is unaffected).
/// - `Whisper { recipients }` — `default: None`, `gm_role: Some(None)` (the
///   GM reads only if individually listed), `users` holds `owner: Owner` plus
///   each recipient as `Observer`.
/// - `GmOnly` — `default: None`, `gm_role: Some(Observer)` (ANY current GM
///   reads, resolved dynamically — not a frozen roster), `users` holds only
///   `owner: Owner`.
///
/// In every case `owner` is inserted into `users` LAST, so a `Whisper` that
/// redundantly names the sender as their own recipient can never downgrade
/// them from `Owner` to `Observer` via map-insertion order.
pub fn build_message_doc(
    world_id: Uuid,
    user: Uuid,
    channel: String,
    actor_owner: Option<ActorOwnerRef>,
    audience: Audience,
    content: Vec<Segment>,
    now: i64,
) -> Document {
    let (default, gm_role, mut users) = match &audience {
        Audience::Public => (DocRole::Observer, None, BTreeMap::new()),
        Audience::Whisper { recipients } => {
            let mut users = BTreeMap::new();
            for &r in recipients {
                if r != user {
                    users.insert(r, DocRole::Observer);
                }
            }
            (DocRole::None, Some(DocRole::None), users)
        }
        Audience::GmOnly => (DocRole::None, Some(DocRole::Observer), BTreeMap::new()),
    };
    users.insert(user, DocRole::Owner);
    let system = MessageSystem {
        channel,
        user_owner: user,
        actor_owner,
        kind: MessageKind::Normal,
        audience,
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
            default,
            users,
            gm_role,
            ..Default::default()
        },
        embedded: BTreeMap::new(),
        parent_id: None,
        system: serde_json::to_value(system).expect("MessageSystem serializes"),
        created_at: now,
        updated_at: now,
    }
}

/// Max characters accepted for a single message's raw content (pre-producer).
pub const MAX_MESSAGE_CHARS: usize = 4096;

/// Max characters accepted for a message's `channel` name. Otherwise `channel`
/// is unbounded save for the 256 KB whole-document size cap.
pub const MAX_CHANNEL_CHARS: usize = 128;

/// Why `handle_send_message` refused to ingest a `SendMessage` frame.
#[derive(Debug)]
pub enum SendMessageError {
    /// Content is empty after trimming whitespace, or `channel` is empty
    /// after trimming whitespace.
    Empty,
    /// Content exceeds `MAX_MESSAGE_CHARS`, or `channel` exceeds
    /// `MAX_CHANNEL_CHARS`. Reused for both — the surface stays minimal since
    /// neither the caller nor the wire protocol distinguishes which field.
    TooLong,
    /// The user's per-minute flood budget is exhausted.
    RateLimited,
    /// An `Audience::Whisper` recipient uuid does not belong to this world.
    /// Fail-closed: the whole send is rejected, nothing is persisted.
    UnknownRecipient,
    /// The authoritative write (`Room::publish`) failed.
    Data(DataError),
}

/// Server-authoritative message ingest: flood-limit, validate, CONSTRUCT the
/// message doc, and publish it via the authoritative path. The sole message-
/// authoring entry point (see module-level INVARIANT comment) — a client can
/// only ever reach a stored `message` doc through this function.
#[allow(clippy::too_many_arguments)]
pub async fn handle_send_message(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    rate: &PingRateLimiter,
    channel: String,
    content: String,
    actor_owner: Option<ActorOwnerRef>,
    audience: Audience,
    now: i64,
    budget_per_min: usize,
) -> Result<Command, SendMessageError> {
    if content.trim().is_empty() {
        return Err(SendMessageError::Empty);
    }
    if content.chars().count() > MAX_MESSAGE_CHARS {
        return Err(SendMessageError::TooLong);
    }
    if channel.trim().is_empty() {
        return Err(SendMessageError::Empty);
    }
    if channel.chars().count() > MAX_CHANNEL_CHARS {
        return Err(SendMessageError::TooLong);
    }
    if !rate.check(ctx.user_id, now, budget_per_min) {
        return Err(SendMessageError::RateLimited);
    }
    if let Audience::Whisper { recipients } = &audience {
        for &r in recipients {
            let is_member = repo
                .member_role(room.world_id, r)
                .await
                .map_err(SendMessageError::Data)?
                .is_some();
            if !is_member {
                return Err(SendMessageError::UnknownRecipient);
            }
        }
    }
    let doc = build_message_doc(
        room.world_id,
        ctx.user_id,
        channel,
        actor_owner,
        audience,
        plain_text_content(&content),
        now,
    );
    room.publish(repo, ctx, vec![Operation::Create { doc }], now)
        .await
        .map_err(SendMessageError::Data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::command::Operation;
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
            Audience::Public,
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
        assert_eq!(sys.audience, Audience::Public);
        assert_eq!(sys.content, vec![Segment::Text { text: "hi".into() }]);
    }

    #[test]
    fn ops_target_message_detects_message_create_and_update() {
        let msg = build_message_doc(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            "all".into(),
            None,
            Audience::Public,
            vec![],
            0,
        );
        assert!(ops_target_message(&[Operation::Create {
            doc: msg.clone()
        }]));
        assert!(ops_target_message(&[Operation::Delete { doc: msg }]));

        let mut note = build_message_doc(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            "all".into(),
            None,
            Audience::Public,
            vec![],
            0,
        );
        note.doc_type = "note".into();
        assert!(!ops_target_message(&[Operation::Create { doc: note }]));
    }

    #[test]
    fn ops_target_message_detects_message_in_mixed_batch() {
        // A batch with one innocuous non-message op followed by a message
        // Create must still trip the guard: `.any()` must not short-circuit
        // on the first (non-matching) op.
        let mut note = build_message_doc(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            "all".into(),
            None,
            Audience::Public,
            vec![],
            0,
        );
        note.doc_type = "note".into();
        let msg = build_message_doc(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            "all".into(),
            None,
            Audience::Public,
            vec![],
            0,
        );
        assert!(ops_target_message(&[
            Operation::Create { doc: note },
            Operation::Create { doc: msg },
        ]));
    }

    #[test]
    fn audience_tagged_roundtrip_and_default() {
        let w = Audience::Whisper {
            recipients: vec![Uuid::from_u128(1)],
        };
        let j = serde_json::to_value(&w).unwrap();
        assert_eq!(j["kind"], "whisper");
        assert_eq!(w, serde_json::from_value(j).unwrap());
        assert_eq!(
            serde_json::to_value(Audience::GmOnly).unwrap()["kind"],
            "gm_only"
        );
        assert_eq!(Audience::default(), Audience::Public);
    }

    #[test]
    fn build_message_doc_public_matches_c1_shape() {
        let owner = Uuid::from_u128(1);
        let doc = build_message_doc(
            Uuid::from_u128(9),
            owner,
            "all".into(),
            None,
            Audience::Public,
            plain_text_content("hi"),
            0,
        );
        assert_eq!(doc.permissions.default, DocRole::Observer);
        assert_eq!(doc.permissions.gm_role, None);
        assert_eq!(doc.permissions.users.get(&owner), Some(&DocRole::Owner));
    }

    #[test]
    fn build_message_doc_whisper_restricts_default_and_gm() {
        let owner = Uuid::from_u128(1);
        let recipient = Uuid::from_u128(2);
        let doc = build_message_doc(
            Uuid::from_u128(9),
            owner,
            "whispers".into(),
            None,
            Audience::Whisper {
                recipients: vec![recipient],
            },
            plain_text_content("psst"),
            0,
        );
        assert_eq!(doc.permissions.default, DocRole::None);
        assert_eq!(doc.permissions.gm_role, Some(DocRole::None));
        assert_eq!(doc.permissions.users.get(&owner), Some(&DocRole::Owner));
        assert_eq!(
            doc.permissions.users.get(&recipient),
            Some(&DocRole::Observer)
        );
    }

    #[test]
    fn build_message_doc_whisper_self_recipient_does_not_downgrade_owner() {
        let owner = Uuid::from_u128(1);
        let doc = build_message_doc(
            Uuid::from_u128(9),
            owner,
            "whispers".into(),
            None,
            Audience::Whisper {
                recipients: vec![owner],
            },
            plain_text_content("note to self"),
            0,
        );
        assert_eq!(
            doc.permissions.users.get(&owner),
            Some(&DocRole::Owner),
            "a redundant self-recipient must never downgrade the owner to Observer"
        );
    }

    #[test]
    fn build_message_doc_gm_only_has_no_named_recipients() {
        let owner = Uuid::from_u128(1);
        let doc = build_message_doc(
            Uuid::from_u128(9),
            owner,
            "gm".into(),
            None,
            Audience::GmOnly,
            plain_text_content("only the GM sees this"),
            0,
        );
        assert_eq!(doc.permissions.default, DocRole::None);
        assert_eq!(doc.permissions.gm_role, Some(DocRole::Observer));
        assert_eq!(
            doc.permissions.users.len(),
            1,
            "only the owner is individually listed — every GM sees it dynamically via gm_role"
        );
        assert_eq!(doc.permissions.users.get(&owner), Some(&DocRole::Owner));
    }

    #[tokio::test]
    async fn handle_send_message_publishes_and_broadcasts() {
        use crate::auth::role::ServerRole;
        use crate::data::document::WorldRole;
        use crate::data::sqlite::SqliteRepository;
        use crate::ws::room::RoomRegistry;

        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let player = repo
            .create_user("pl", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, player, WorldRole::Player)
            .await
            .unwrap();
        let ctx = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let (mut rx, _current) = room.subscribe();
        let rate = PingRateLimiter::new();

        let cmd = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            "all".into(),
            "hello".into(),
            None,
            Audience::Public,
            100,
            30,
        )
        .await
        .unwrap();
        assert_eq!(cmd.seq, 1);
        let got = rx.recv().await.unwrap();
        assert_eq!(got.event_seq(), Some(1));

        // Rate limit: exhaust the budget then expect RateLimited.
        let rate2 = PingRateLimiter::new();
        for _ in 0..2 {
            let _ = handle_send_message(
                &room,
                &repo,
                &ctx,
                &rate2,
                "all".into(),
                "x".into(),
                None,
                Audience::Public,
                100,
                2,
            )
            .await;
        }
        let err = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate2,
            "all".into(),
            "x".into(),
            None,
            Audience::Public,
            100,
            2,
        )
        .await;
        assert!(matches!(err, Err(SendMessageError::RateLimited)));

        // Empty + too-long rejected before any publish.
        assert!(matches!(
            handle_send_message(
                &room,
                &repo,
                &ctx,
                &rate,
                "all".into(),
                "".into(),
                None,
                Audience::Public,
                100,
                30
            )
            .await,
            Err(SendMessageError::Empty)
        ));
        let long = "a".repeat(MAX_MESSAGE_CHARS + 1);
        assert!(matches!(
            handle_send_message(
                &room,
                &repo,
                &ctx,
                &rate,
                "all".into(),
                long,
                None,
                Audience::Public,
                100,
                30
            )
            .await,
            Err(SendMessageError::TooLong)
        ));

        // Empty/over-long channel rejected before any publish; seq unchanged.
        let seq_before = room.subscribe().1;
        assert!(matches!(
            handle_send_message(
                &room,
                &repo,
                &ctx,
                &rate,
                "".into(),
                "hi".into(),
                None,
                Audience::Public,
                100,
                30
            )
            .await,
            Err(SendMessageError::Empty)
        ));
        let long_channel = "c".repeat(MAX_CHANNEL_CHARS + 1);
        assert!(matches!(
            handle_send_message(
                &room,
                &repo,
                &ctx,
                &rate,
                long_channel,
                "hi".into(),
                None,
                Audience::Public,
                100,
                30
            )
            .await,
            Err(SendMessageError::TooLong)
        ));
        assert_eq!(
            room.subscribe().1,
            seq_before,
            "rejected channel must not publish"
        );
    }

    #[tokio::test]
    async fn handle_send_message_rejects_unknown_whisper_recipient() {
        use crate::auth::role::ServerRole;
        use crate::data::document::WorldRole;
        use crate::data::sqlite::SqliteRepository;
        use crate::ws::room::RoomRegistry;

        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let player = repo
            .create_user("pl", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, player, WorldRole::Player)
            .await
            .unwrap();
        let ctx = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let rate = PingRateLimiter::new();

        // A uuid that belongs to no user at all, let alone this world.
        let foreign = Uuid::from_u128(99_999);
        let err = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            "whispers".into(),
            "psst".into(),
            None,
            Audience::Whisper {
                recipients: vec![foreign],
            },
            100,
            30,
        )
        .await;
        assert!(matches!(err, Err(SendMessageError::UnknownRecipient)));

        // Nothing was persisted — the seq was never consumed.
        assert!(repo.events_since(w.id, 0).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn handle_send_message_accepts_a_whisper_to_a_real_member() {
        use crate::auth::role::ServerRole;
        use crate::data::document::WorldRole;
        use crate::data::sqlite::SqliteRepository;
        use crate::ws::room::RoomRegistry;

        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let player = repo
            .create_user("pl", None, ServerRole::User, 0)
            .await
            .unwrap();
        let recipient = repo
            .create_user("re", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, player, WorldRole::Player)
            .await
            .unwrap();
        repo.add_member(w.id, recipient, WorldRole::Player)
            .await
            .unwrap();
        let ctx = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let rate = PingRateLimiter::new();

        let cmd = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            "whispers".into(),
            "psst".into(),
            None,
            Audience::Whisper {
                recipients: vec![recipient],
            },
            100,
            30,
        )
        .await
        .unwrap();
        assert_eq!(cmd.seq, 1);
    }

    /// A message doc built via `build_message_doc` and committed via
    /// `apply_intent` under the posting Player's own ctx (the same write
    /// `handle_send_message` performs) is found by ANOTHER world member's
    /// `repo.search` — the message rides the existing search index with no
    /// message-specific indexing code, and its body text surfaces in the
    /// snippet.
    #[tokio::test]
    async fn posted_message_is_searchable_by_members() {
        use crate::auth::role::ServerRole;
        use crate::data::document::WorldRole;
        use crate::data::sqlite::SqliteRepository;

        let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = r
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let player = r
            .create_user("pl", None, ServerRole::User, 0)
            .await
            .unwrap();
        let other = r
            .create_user("ot", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();
        r.add_member(w.id, player, WorldRole::Player).await.unwrap();
        r.add_member(w.id, other, WorldRole::Player).await.unwrap();
        let pl_ctx = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        let ot_ctx = PermissionContext {
            user_id: other,
            world_role: WorldRole::Player,
        };

        let doc = build_message_doc(
            w.id,
            player,
            "all".into(),
            None,
            Audience::Public,
            plain_text_content("banshee wail"),
            1,
        );
        r.apply_intent(&pl_ctx, w.id, vec![Operation::Create { doc }], 1)
            .await
            .unwrap();

        let page = r.search(&ot_ctx, w.id, "banshee", 10, None).await.unwrap();
        assert_eq!(page.hits.len(), 1, "another member finds the message");
        assert!(page.hits[0].snippet.to_lowercase().contains("banshee"));
    }
}
