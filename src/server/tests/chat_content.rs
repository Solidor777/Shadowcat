//! Integration proof (M11c-3): `handle_send_message` runs a `SendMessage`
//! frame's raw content through the command parser and content sanitizer
//! before persisting — `/me` yields `MessageKind::Emote`, `/w @name` resolves
//! a real member's username to their uuid and builds `Audience::Whisper`
//! (rejecting the whole send if the name is unknown), and the world's
//! `chat-settings` policy governs whether the stored content is a literal
//! `Segment::Text` or a sanitized `Segment::Html` run. Drives
//! `handle_send_message` directly (repo + room + ctx), mirroring
//! `chat/mod.rs`'s own `handle_send_message_*` unit-test harness rather than
//! the full WS transport used by `chat_audience.rs`.

use shadowcat::auth::role::ServerRole;
use shadowcat::chat::{
    build_link_preview_client, build_message_doc, handle_delete_message, handle_edit_message,
    handle_send_message, Audience, ChatContentPolicy, LinkPreviewCache, MessageKind, MessageSystem,
    PreviewRateLimiter, Segment, SendMessageError, CHAT_SETTINGS_DOC_TYPE,
};
use shadowcat::data::command::{Command, FieldChange, Operation, WriteOrigin};
use shadowcat::data::document::{DocRole, Document, PermissionSet, Scope, WorldRole};
use shadowcat::data::membership::PermissionContext;
use shadowcat::data::repository::Repository;
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::data::DataError;
use shadowcat::ws::room::Room;
use shadowcat::ws::room::RoomRegistry;
use shadowcat::ws::PingRateLimiter;
use std::collections::BTreeMap;
use std::sync::Arc;
use uuid::Uuid;

struct Fixture {
    repo: SqliteRepository,
    room: Arc<Room>,
    rate: PingRateLimiter,
    // Link-preview deps: every test in this file leaves `hyperlinks` off
    // (default policy or an explicit override not setting it), so
    // `previews_enabled()` is always false and `enrich` never fetches — a
    // production (non-loopback) client is safe here, it's simply never
    // dialed.
    preview_client: reqwest::Client,
    preview_cache: LinkPreviewCache,
    preview_rate: PreviewRateLimiter,
    alice: PermissionContext,
    bob: PermissionContext,
    bob_id: Uuid,
    gm: PermissionContext,
}

impl Fixture {
    /// GM + `alice` (Player) + `bob` (Player), no chat-settings doc (default
    /// policy — every enrichment toggle off).
    async fn new() -> Self {
        Self::with_policy(None).await
    }

    /// Same seed, plus a `chat-settings` doc holding `policy` when given.
    async fn with_policy(policy: Option<ChatContentPolicy>) -> Self {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let alice_id = repo
            .create_user("alice", None, ServerRole::User, 0)
            .await
            .unwrap();
        let bob_id = repo
            .create_user("bob", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, alice_id, WorldRole::Player)
            .await
            .unwrap();
        repo.add_member(w.id, bob_id, WorldRole::Player)
            .await
            .unwrap();

        if let Some(policy) = policy {
            let gm_ctx = PermissionContext {
                user_id: gm,
                world_role: WorldRole::Gm,
            };
            let doc = Document {
                id: Uuid::new_v4(),
                scope: Scope::World { world_id: w.id },
                doc_type: CHAT_SETTINGS_DOC_TYPE.to_string(),
                schema_version: 1,
                name: None,
                source: None,
                owner: Some(gm),
                permissions: PermissionSet::default(),
                embedded: BTreeMap::new(),
                parent_id: None,
                engine: None,
                system: serde_json::to_value(policy).unwrap(),
                created_at: 0,
                updated_at: 0,
            };
            repo.apply_intent(
                &gm_ctx,
                w.id,
                vec![Operation::Create { doc }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
        }

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let alice = PermissionContext {
            user_id: alice_id,
            world_role: WorldRole::Player,
        };
        let bob = PermissionContext {
            user_id: bob_id,
            world_role: WorldRole::Player,
        };
        let gm_ctx = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };
        Fixture {
            repo,
            room,
            rate: PingRateLimiter::new(),
            preview_client: build_link_preview_client(),
            preview_cache: LinkPreviewCache::new(),
            preview_rate: PreviewRateLimiter::new(),
            alice,
            bob,
            bob_id,
            gm: gm_ctx,
        }
    }

    /// The `doc.id` of a `Create`d message, resolved from the returned `Command`.
    async fn message_id(&self, cmd: &Command) -> Uuid {
        self.stored_message_doc(cmd).await.id
    }

    /// Resolves the stored message doc from a `Command`, whether it authored a
    /// `Create` (a fresh `SendMessage`) or an `Update` (a `handle_edit_message`
    /// revision) — both carry the message's doc id, just in different ops.
    async fn stored_message_doc(&self, cmd: &Command) -> Document {
        let doc_id = match &cmd.ops[0] {
            Operation::Create { doc } => doc.id,
            Operation::Update { doc_id, .. } => *doc_id,
            Operation::Delete { doc } => doc.id,
        };
        self.repo
            .get_document(doc_id)
            .await
            .unwrap()
            .expect("message doc persisted")
    }

    async fn stored_message_system(&self, cmd: &Command) -> MessageSystem {
        let doc = self.stored_message_doc(cmd).await;
        serde_json::from_value(doc.system).unwrap()
    }

    async fn send(&self, content: &str) -> Result<Command, SendMessageError> {
        handle_send_message(
            &self.room,
            &self.repo,
            &self.alice,
            &self.rate,
            &self.preview_client,
            &self.preview_cache,
            &self.preview_rate,
            "all".into(),
            content.into(),
            None,
            Audience::Public,
            1,
            60,
        )
        .await
    }
}

/// GM + `alice`/`bob` (Player), no `chat-settings` doc (default content policy).
async fn fixture() -> Fixture {
    Fixture::new().await
}

/// Same seed, with a `chat-settings` doc holding `policy`.
async fn fixture_with_policy(policy: ChatContentPolicy) -> Fixture {
    Fixture::with_policy(Some(policy)).await
}

#[tokio::test]
async fn owner_can_edit_and_content_resanitizes() {
    let f = fixture_with_policy(ChatContentPolicy {
        markdown: true,
        ..Default::default()
    })
    .await;
    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        "all".into(),
        "first".into(),
        None,
        Audience::Public,
        1,
        60,
    )
    .await
    .unwrap();
    let id = f.message_id(&sent).await;
    let edited = handle_edit_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        id,
        "**second**".into(),
        2,
        60,
    )
    .await
    .unwrap();
    let sys = f.stored_message_system(&edited).await;
    assert!(matches!(sys.content.as_slice(), [Segment::Html { .. }]));
    assert_eq!(sys.edited_at, Some(2));
}

#[tokio::test]
async fn non_owner_non_gm_cannot_edit() {
    let f = fixture().await;
    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        "all".into(),
        "hi".into(),
        None,
        Audience::Public,
        1,
        60,
    )
    .await
    .unwrap();
    let id = f.message_id(&sent).await;
    let r = handle_edit_message(
        &f.room,
        &f.repo,
        &f.bob,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        id,
        "hax".into(),
        2,
        60,
    )
    .await;
    assert!(matches!(r, Err(SendMessageError::Forbidden)));
}

/// An already soft-deleted message cannot be edited — from the edit path's
/// perspective a tombstone is gone, not a live message with resurrectable
/// content. Without this check, an owner/GM could bring `content` back
/// (re-indexed into FTS) while `deleted_at` stays set.
#[tokio::test]
async fn cannot_edit_already_deleted_message() {
    let f = fixture().await;
    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        "all".into(),
        "secret".into(),
        None,
        Audience::Public,
        1,
        60,
    )
    .await
    .unwrap();
    let id = f.message_id(&sent).await;
    handle_delete_message(&f.room, &f.repo, &f.alice, &f.rate, id, 2, 60)
        .await
        .unwrap();
    let r = handle_edit_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        id,
        "resurrected".into(),
        3,
        60,
    )
    .await;
    assert!(
        matches!(r, Err(SendMessageError::NotFound)),
        "editing a tombstoned message must return NotFound: {r:?}"
    );
    let doc = f.repo.get_document(id).await.unwrap().unwrap();
    let sys: MessageSystem = serde_json::from_value(doc.system).unwrap();
    assert!(
        sys.content.is_empty(),
        "content must stay empty — the rejected edit must not persist"
    );
}

#[tokio::test]
async fn gm_can_edit_players_message() {
    let f = fixture().await;
    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        "all".into(),
        "hi".into(),
        None,
        Audience::Public,
        1,
        60,
    )
    .await
    .unwrap();
    let id = f.message_id(&sent).await;
    assert!(handle_edit_message(
        &f.room,
        &f.repo,
        &f.gm,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        id,
        "moderated".into(),
        2,
        60
    )
    .await
    .is_ok());
}

/// A GM moderating chat must be able to edit ANY message regardless of its
/// restricted audience — the message's own `gm_role`/`users` fields exist to
/// gate ordinary READ visibility for OTHER recipients, not the server's own
/// moderation capability. Here the GM is neither the owner nor individually
/// listed among the whisper's recipients (only `bob` is), so this proves the
/// GM's edit authority does not depend on being an addressee.
#[tokio::test]
async fn gm_can_edit_whisper_message_not_addressed_to_gm() {
    let f = fixture().await;
    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        "whispers".into(),
        "hi".into(),
        None,
        Audience::Whisper {
            recipients: vec![f.bob_id],
        },
        1,
        60,
    )
    .await
    .unwrap();
    let id = f.message_id(&sent).await;
    let r = handle_edit_message(
        &f.room,
        &f.repo,
        &f.gm,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        id,
        "moderated".into(),
        2,
        60,
    )
    .await;
    assert!(
        r.is_ok(),
        "GM moderation must override whisper audience gating: {r:?}"
    );
}

/// Same proof for `Audience::GmOnly`: the message's own `gm_role` resolves to
/// `DocRole::Observer` (READ-only) for a GM not individually listed in
/// `permissions.users` — the server's moderation authority must not be capped
/// by that per-document READ floor.
#[tokio::test]
async fn gm_can_edit_gm_only_message_not_individually_listed() {
    let f = fixture().await;
    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        "gm".into(),
        "hi".into(),
        None,
        Audience::GmOnly,
        1,
        60,
    )
    .await
    .unwrap();
    let id = f.message_id(&sent).await;
    let r = handle_edit_message(
        &f.room,
        &f.repo,
        &f.gm,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        id,
        "moderated".into(),
        2,
        60,
    )
    .await;
    assert!(
        r.is_ok(),
        "GM moderation must override gm_only audience gating: {r:?}"
    );
}

#[tokio::test]
async fn edit_cannot_retarget_audience() {
    let f = fixture().await;
    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        "all".into(),
        "hi".into(),
        None,
        Audience::Public,
        1,
        60,
    )
    .await
    .unwrap();
    let id = f.message_id(&sent).await;
    let r = handle_edit_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        id,
        "/w @bob sneaky".into(),
        2,
        60,
    )
    .await;
    assert!(matches!(r, Err(SendMessageError::AudienceLocked)));
}

#[tokio::test]
async fn me_command_produces_emote() {
    let f = Fixture::new().await;
    let cmd = f.send("/me waves").await.unwrap();
    let sys = f.stored_message_system(&cmd).await;
    assert_eq!(sys.kind, MessageKind::Emote);
    // md() default off unless enabled: plain text segment, body stripped of the token.
    assert_eq!(
        sys.content,
        vec![Segment::Text {
            text: "waves".into()
        }]
    );
}

#[tokio::test]
async fn whisper_command_targets_named_user() {
    let f = Fixture::new().await;
    let cmd = f.send("/w @bob secret").await.unwrap();
    let doc = f.stored_message_doc(&cmd).await;
    assert_eq!(
        doc.permissions.default,
        DocRole::None,
        "whisper hides from the world"
    );
    assert!(
        doc.permissions.users.contains_key(&f.bob_id),
        "bob included"
    );
    let sys = f.stored_message_system(&cmd).await;
    assert_eq!(
        sys.audience,
        Audience::Whisper {
            recipients: vec![f.bob_id]
        }
    );
    assert_eq!(
        sys.content,
        vec![Segment::Text {
            text: "secret".into()
        }]
    );
}

#[tokio::test]
async fn unknown_whisper_target_rejects_whole_send() {
    let f = Fixture::new().await;
    let r = f.send("/w @nobody hi").await;
    assert!(matches!(r, Err(SendMessageError::UnknownRecipient)));
    // Nothing persisted — the seq was never consumed.
    assert!(f
        .repo
        .events_since(f.room.world_id, 0)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn markdown_enriched_when_policy_on() {
    let f = Fixture::with_policy(Some(ChatContentPolicy {
        markdown: true,
        ..Default::default()
    }))
    .await;
    let cmd = f.send("**bold**").await.unwrap();
    let sys = f.stored_message_system(&cmd).await;
    assert!(
        matches!(sys.content.as_slice(), [Segment::Html { .. }]),
        "got {:?}",
        sys.content
    );
}

/// A `/w` in the content overrides the frame's `Audience::Public` field — the
/// "content /w wins over frame audience" reconciliation rule.
#[tokio::test]
async fn content_whisper_overrides_frame_audience() {
    let f = Fixture::new().await;
    // The frame's `audience` argument inside `Fixture::send` is always
    // `Audience::Public`; a `/w` in the content must still produce a
    // Whisper-shaped stored doc.
    let cmd = f.send("/w @bob overridden").await.unwrap();
    let sys = f.stored_message_system(&cmd).await;
    assert_eq!(
        sys.audience,
        Audience::Whisper {
            recipients: vec![f.bob_id]
        },
        "content-level /w must win over the frame's Public audience"
    );
}

/// An over-cap `/w @name...` list in the content is rejected as `TooLong`
/// WITHOUT ever attempting to resolve a single username. None of the names
/// here belong to any real member — if the cap were checked after (or
/// during) username resolution, the first unresolvable name would trigger
/// `UnknownRecipient` well before the cap could ever be reached, since the
/// cap is only checked once the whole list has been walked. Getting
/// `TooLong` instead proves the cap check runs first, bounding the number of
/// `member_id_by_username` DB calls by construction.
#[tokio::test]
async fn content_whisper_over_cap_rejects_before_username_resolution() {
    use shadowcat::chat::MAX_WHISPER_RECIPIENTS;

    let f = Fixture::new().await;
    let names: String = (0..(MAX_WHISPER_RECIPIENTS + 1))
        .map(|i| format!("@no-such-user-{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let content = format!("/w {names} hi");
    let r = f.send(&content).await;
    assert!(matches!(r, Err(SendMessageError::TooLong)), "got {r:?}");
    assert!(
        f.repo
            .events_since(f.room.world_id, 0)
            .await
            .unwrap()
            .is_empty(),
        "an over-cap whisper must persist nothing"
    );
}

/// `/w @bob` with no trailing message text leaves `parsed.body` empty —
/// this must be rejected the same way an empty raw `content` is, not
/// silently persisted as a message with no content.
#[tokio::test]
async fn whisper_with_no_body_text_is_rejected_as_empty() {
    let f = Fixture::new().await;
    let r = f.send("/w @bob").await;
    assert!(matches!(r, Err(SendMessageError::Empty)), "got {r:?}");
    assert!(
        f.repo
            .events_since(f.room.world_id, 0)
            .await
            .unwrap()
            .is_empty(),
        "an empty-body whisper must persist nothing"
    );
}

/// `/roll 2d6+3` driven through the full `handle_send_message` pipeline
/// (not just the pure parser unit test in `commands.rs`) stores
/// `MessageKind::Roll` with the formula EXECUTED (M11d-2): content is one
/// `Segment::RollEmbed`, never a literal `Text` of the unexecuted expression
/// — see `chat_rolls.rs` for the full roll-execution integration matrix.
#[tokio::test]
async fn roll_command_produces_roll_kind_with_executed_embed() {
    let f = Fixture::new().await;
    let cmd = f.send("/roll 2d6+3").await.unwrap();
    let sys = f.stored_message_system(&cmd).await;
    assert_eq!(sys.kind, MessageKind::Roll);
    match sys.content.as_slice() {
        [Segment::RollEmbed { formula, outcome }] => {
            assert_eq!(formula, "2d6+3");
            assert!(
                (5..=15).contains(&outcome.total),
                "2d6+3 total out of range: {}",
                outcome.total
            );
        }
        other => panic!("expected one RollEmbed segment, got {other:?}"),
    }
}

#[tokio::test]
async fn owner_soft_delete_clears_content_and_keeps_doc() {
    let f = fixture().await;
    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        "all".into(),
        "secret".into(),
        None,
        Audience::Public,
        1,
        60,
    )
    .await
    .unwrap();
    let id = f.message_id(&sent).await;
    handle_delete_message(&f.room, &f.repo, &f.alice, &f.rate, id, 2, 60)
        .await
        .unwrap();
    let doc = f
        .repo
        .get_document(id)
        .await
        .unwrap()
        .expect("doc still present (tombstone)");
    let sys: MessageSystem = serde_json::from_value(doc.system).unwrap();
    assert!(sys.content.is_empty(), "content cleared");
    assert_eq!(sys.deleted_at, Some(2));
}

#[tokio::test]
async fn non_owner_non_gm_cannot_delete() {
    let f = fixture().await;
    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        "all".into(),
        "hi".into(),
        None,
        Audience::Public,
        1,
        60,
    )
    .await
    .unwrap();
    let id = f.message_id(&sent).await;
    assert!(matches!(
        handle_delete_message(&f.room, &f.repo, &f.bob, &f.rate, id, 2, 60).await,
        Err(SendMessageError::Forbidden)
    ));
}

/// Repeated `DeleteMessage` calls against the SAME message are rate-limited
/// like `SendMessage`/`EditMessage` — without this, the OCC pre-image always
/// matches the freshly stored doc and `deleted_at` is re-stamped each call,
/// so an owner/GM could otherwise repeatedly delete one message for unbounded
/// write/broadcast/FTS-reindex amplification from a single cheap frame.
#[tokio::test]
async fn repeated_delete_of_same_message_is_rate_limited() {
    let f = fixture().await;
    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        "all".into(),
        "secret".into(),
        None,
        Audience::Public,
        1,
        60,
    )
    .await
    .unwrap();
    let id = f.message_id(&sent).await;

    // A dedicated limiter with a tight budget so the SECOND delete trips it.
    let tight_rate = PingRateLimiter::new();
    let first = handle_delete_message(&f.room, &f.repo, &f.alice, &tight_rate, id, 2, 1).await;
    assert!(first.is_ok(), "first delete within budget: {first:?}");
    let second = handle_delete_message(&f.room, &f.repo, &f.alice, &tight_rate, id, 3, 1).await;
    assert!(
        matches!(second, Err(SendMessageError::RateLimited)),
        "second delete of the same message must trip the flood budget: {second:?}"
    );
}

/// Deleted doc stays IN the sequenced log — not just readable via
/// `get_document`, but present through `events_since` too, proving the
/// tombstone did not create a sequence gap (it's an `Operation::Update`,
/// same as an edit, not a hard `Delete`).
#[tokio::test]
async fn soft_delete_leaves_doc_in_sequenced_log() {
    let f = fixture().await;
    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        "all".into(),
        "secret".into(),
        None,
        Audience::Public,
        1,
        60,
    )
    .await
    .unwrap();
    let id = f.message_id(&sent).await;
    let cmd = handle_delete_message(&f.room, &f.repo, &f.alice, &f.rate, id, 2, 60)
        .await
        .unwrap();
    assert_eq!(cmd.seq, 2, "delete consumes the next sequence number");
    let events = f.repo.events_since(f.room.world_id, 0).await.unwrap();
    assert_eq!(events.len(), 2, "both the create and the delete are logged");
}

/// A GM moderating chat must be able to delete ANY message regardless of its
/// restricted audience, the same rule the edit path enforces — the
/// `apply_intent` Update exemption is generic to
/// `WriteOrigin::ServerMessageRevision`, not edit-specific. The GM here is
/// not individually listed among the whisper's recipients (only `bob` is).
#[tokio::test]
async fn gm_can_delete_whisper_message_not_addressed_to_gm() {
    let f = fixture().await;
    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        "whispers".into(),
        "hi".into(),
        None,
        Audience::Whisper {
            recipients: vec![f.bob_id],
        },
        1,
        60,
    )
    .await
    .unwrap();
    let id = f.message_id(&sent).await;
    let r = handle_delete_message(&f.room, &f.repo, &f.gm, &f.rate, id, 2, 60).await;
    assert!(
        r.is_ok(),
        "GM moderation must override whisper audience gating: {r:?}"
    );
    let doc = f.repo.get_document(id).await.unwrap().unwrap();
    let sys: MessageSystem = serde_json::from_value(doc.system).unwrap();
    assert!(sys.content.is_empty());
    assert_eq!(sys.deleted_at, Some(2));
}

/// Same proof for `Audience::GmOnly`: a GM not individually listed in
/// `permissions.users` (only `gm_role: Some(Observer)` grants them READ) must
/// still be able to delete it.
#[tokio::test]
async fn gm_can_delete_gm_only_message_not_individually_listed() {
    let f = fixture().await;
    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        "gm".into(),
        "hi".into(),
        None,
        Audience::GmOnly,
        1,
        60,
    )
    .await
    .unwrap();
    let id = f.message_id(&sent).await;
    let r = handle_delete_message(&f.room, &f.repo, &f.gm, &f.rate, id, 2, 60).await;
    assert!(
        r.is_ok(),
        "GM moderation must override gm_only audience gating: {r:?}"
    );
}

/// A non-recipient of a whisper (`bob` was never listed; the whisper was
/// addressed elsewhere) must see nothing about the message even after it is
/// soft-deleted — per-recipient redaction of the tombstone is unaffected by
/// the delete.
#[tokio::test]
async fn non_recipient_still_cannot_see_deleted_whisper() {
    use shadowcat::data::permission::{cap, resolve_access};

    let f = fixture().await;
    // A distinct real user (member_id is a FK) so bob is provably never a
    // recipient of this whisper.
    let recipient = f
        .repo
        .create_user("carol", None, ServerRole::User, 0)
        .await
        .unwrap();
    f.repo
        .add_member(f.room.world_id, recipient, WorldRole::Player)
        .await
        .unwrap();
    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        "whispers".into(),
        "hi".into(),
        None,
        Audience::Whisper {
            recipients: vec![recipient],
        },
        1,
        60,
    )
    .await
    .unwrap();
    let id = f.message_id(&sent).await;
    handle_delete_message(&f.room, &f.repo, &f.alice, &f.rate, id, 2, 60)
        .await
        .unwrap();
    let doc = f.repo.get_document(id).await.unwrap().unwrap();
    let access = resolve_access(f.bob.user_id, WorldRole::Player, &doc);
    assert!(
        !access.has(cap::READ),
        "bob has no READ access on the tombstoned whisper doc"
    );
}

/// A non-recipient of a whisper must see NO trace of an EDITED (live,
/// non-empty) message's content — distinct from
/// `non_recipient_still_cannot_see_deleted_whisper`, whose tombstone has no
/// content to leak in the first place, so it cannot prove real content is
/// withheld. Drives `repo.search` (the same egress surface
/// `posted_message_is_searchable_by_members` proves messages ride) as the
/// non-recipient: neither the ORIGINAL nor the EDITED content text is
/// findable — a bare `!access.has(READ)` check alone wouldn't rule out a
/// snippet or index leak of the actual post-edit words.
#[tokio::test]
async fn non_recipient_finds_no_trace_of_edited_whisper_content() {
    let f = fixture().await;
    let recipient = f
        .repo
        .create_user("carol", None, ServerRole::User, 0)
        .await
        .unwrap();
    f.repo
        .add_member(f.room.world_id, recipient, WorldRole::Player)
        .await
        .unwrap();
    let non_recipient = f
        .repo
        .create_user("dave", None, ServerRole::User, 0)
        .await
        .unwrap();
    f.repo
        .add_member(f.room.world_id, non_recipient, WorldRole::Player)
        .await
        .unwrap();
    let non_recipient_ctx = PermissionContext {
        user_id: non_recipient,
        world_role: WorldRole::Player,
    };

    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        "whispers".into(),
        "griffonroost".into(),
        None,
        Audience::Whisper {
            recipients: vec![recipient],
        },
        1,
        60,
    )
    .await
    .unwrap();
    let id = f.message_id(&sent).await;
    let edited = handle_edit_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
        &f.preview_client,
        &f.preview_cache,
        &f.preview_rate,
        id,
        "phoenixnest".into(),
        2,
        60,
    )
    .await
    .unwrap();
    let sys = f.stored_message_system(&edited).await;
    assert_eq!(
        sys.content,
        vec![Segment::Text {
            text: "phoenixnest".into()
        }],
        "sanity: the edit actually replaced the content"
    );

    let recipient_ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Player,
    };
    let recipient_hits = f
        .repo
        .search(&recipient_ctx, f.room.world_id, "phoenixnest", 10, None)
        .await
        .unwrap();
    assert_eq!(
        recipient_hits.hits.len(),
        1,
        "sanity: the addressed recipient DOES find the edited content"
    );

    for query in ["phoenixnest", "griffonroost"] {
        let page = f
            .repo
            .search(&non_recipient_ctx, f.room.world_id, query, 10, None)
            .await
            .unwrap();
        assert!(
            page.hits.is_empty(),
            "non-recipient must find no trace of edited whisper content for query {query:?}: {:?}",
            page.hits
        );
    }
}

/// Anchor proof (M11c-3, §6 coupled seam): a raw client `Intent` `Update`
/// attempting to forge `/system/kind` on an existing, legitimately-owned
/// message to `"system"` (impersonating a server-authored notice) is still
/// blanket-rejected by `apply_intent`'s `Update` branch, even though the
/// requester genuinely holds `DocRole::Owner` on the doc (which would
/// otherwise satisfy the ordinary WRITE_FIELDS check). Distinct from
/// `sqlite.rs`'s `message_update_rejected_for_client_allowed_for_server_revision`,
/// which forges `/system/content` — this proves the rejection is not scoped
/// to any one field path, closing the specific "forge kind=System" angle the
/// task brief calls out.
#[tokio::test]
async fn client_intent_update_to_message_still_forbidden() {
    let f = fixture().await;
    let sent = f.send("hi").await.unwrap();
    let id = f.message_id(&sent).await;
    let op = Operation::Update {
        doc_id: id,
        changes: vec![FieldChange {
            path: "/system/kind".into(),
            old: serde_json::json!("normal"),
            new: serde_json::json!("system"),
        }],
    };
    let r = f
        .repo
        .apply_intent(&f.alice, f.room.world_id, vec![op], 2, WriteOrigin::Client)
        .await;
    assert!(
        matches!(r, Err(DataError::Forbidden)),
        "client forgery of kind=System must be rejected: {r:?}"
    );
    // Confirm the rejection actually held the line: the stored doc's kind is
    // unchanged, not merely that the call returned an error.
    let doc = f.repo.get_document(id).await.unwrap().unwrap();
    let sys: MessageSystem = serde_json::from_value(doc.system).unwrap();
    assert_eq!(sys.kind, MessageKind::Normal, "kind must be unaltered");
}

/// The `ServerMessageRevision` exemption grants only READ + WRITE_FIELDS, not
/// `all: true` — a hypothetical future `ServerMessageRevision`-origin write
/// targeting `/permissions` (neither `handle_edit_message` nor
/// `handle_delete_message` ever construct such an op) must still be rejected,
/// proving the narrowed `Access` doesn't grant `EDIT_PERMISSIONS` by accident.
#[tokio::test]
async fn server_message_revision_does_not_grant_permissions_write() {
    let f = fixture().await;
    let sent = f.send("hi").await.unwrap();
    let id = f.message_id(&sent).await;
    let doc = f.repo.get_document(id).await.unwrap().unwrap();
    let op = Operation::Update {
        doc_id: id,
        changes: vec![FieldChange {
            path: "/permissions/default".into(),
            old: serde_json::to_value(doc.permissions.default).unwrap(),
            new: serde_json::json!("owner"),
        }],
    };
    let r = f
        .repo
        .apply_intent(
            &f.alice,
            f.room.world_id,
            vec![op],
            2,
            WriteOrigin::ServerMessageRevision,
        )
        .await;
    assert!(
        matches!(r, Err(DataError::Forbidden)),
        "ServerMessageRevision must not grant EDIT_PERMISSIONS: {r:?}"
    );
}

/// Anchor proof (M11c-3, §6 coupled seam): the WS/HTTP ingress guard
/// (`ops_target_message`) is keyed purely on the op's `doc_type`, not on any
/// content inside the payload — so an attacker cannot evade it by crafting a
/// `Create`/`Delete` whose `system` body impersonates a server-authored
/// notice (`kind: System`) while still targeting `doc_type: "message"`. This
/// is a distinct angle from `chat/mod.rs`'s existing
/// `ops_target_message_detects_message_create_and_update` (which only proves
/// detection for an ordinary `MessageKind::Normal` doc): here the payload is
/// deliberately forged to look server-authored, proving the guard cannot be
/// evaded by lying about `kind` inside the `system` body — only `doc_type`
/// (which the client cannot change without also changing what the guard
/// matches on) determines rejection.
#[test]
fn client_forged_system_kind_create_and_delete_still_blocked_at_ingress() {
    let world = Uuid::new_v4();
    let attacker = Uuid::new_v4();
    let mut forged = build_message_doc(
        world,
        attacker,
        "all".into(),
        None,
        Audience::Public,
        MessageKind::Normal,
        vec![],
        None,
        0,
    );
    // Forge the payload to impersonate a server-authored System notice —
    // the guard must not be fooled by this; it never inspects `system`.
    let mut sys: serde_json::Value = forged.system.clone();
    sys["kind"] = serde_json::json!("system");
    forged.system = sys;
    assert_eq!(
        forged.doc_type,
        shadowcat::chat::MESSAGE_DOC_TYPE,
        "sanity: still a message doc_type"
    );

    assert!(
        shadowcat::chat::ops_target_message(&[Operation::Create {
            doc: forged.clone()
        }]),
        "forged System-kind Create must still be blocked at ingress"
    );
    assert!(
        shadowcat::chat::ops_target_message(&[Operation::Delete { doc: forged }]),
        "forged System-kind Delete must still be blocked at ingress"
    );
}
