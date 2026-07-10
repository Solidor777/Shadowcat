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
    handle_edit_message, handle_send_message, Audience, ChatContentPolicy, MessageKind,
    MessageSystem, Segment, SendMessageError, CHAT_SETTINGS_DOC_TYPE,
};
use shadowcat::data::command::{Command, Operation, WriteOrigin};
use shadowcat::data::document::{DocRole, Document, PermissionSet, Scope, WorldRole};
use shadowcat::data::membership::PermissionContext;
use shadowcat::data::repository::Repository;
use shadowcat::data::sqlite::SqliteRepository;
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
                source: None,
                owner: Some(gm),
                permissions: PermissionSet::default(),
                embedded: BTreeMap::new(),
                parent_id: None,
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
    let r = handle_edit_message(&f.room, &f.repo, &f.bob, &f.rate, id, "hax".into(), 2, 60).await;
    assert!(matches!(r, Err(SendMessageError::Forbidden)));
}

#[tokio::test]
async fn gm_can_edit_players_message() {
    let f = fixture().await;
    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
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
        id,
        "moderated".into(),
        2,
        60
    )
    .await
    .is_ok());
}

#[tokio::test]
async fn edit_cannot_retarget_audience() {
    let f = fixture().await;
    let sent = handle_send_message(
        &f.room,
        &f.repo,
        &f.alice,
        &f.rate,
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
/// `MessageKind::Roll` with the dice expression verbatim as a literal
/// `Segment::Text` — `Fixture::new()` seeds no `chat-settings` doc, so the
/// default content policy (markdown/html both off) applies and the
/// expression is never run through the markdown/HTML producer.
#[tokio::test]
async fn roll_command_produces_roll_kind_with_verbatim_expression() {
    let f = Fixture::new().await;
    let cmd = f.send("/roll 2d6+3").await.unwrap();
    let sys = f.stored_message_system(&cmd).await;
    assert_eq!(sys.kind, MessageKind::Roll);
    assert_eq!(
        sys.content,
        vec![Segment::Text {
            text: "2d6+3".into()
        }]
    );
}
