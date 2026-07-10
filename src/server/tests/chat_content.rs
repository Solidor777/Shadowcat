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
    handle_send_message, Audience, ChatContentPolicy, MessageKind, MessageSystem, Segment,
    SendMessageError, CHAT_SETTINGS_DOC_TYPE,
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
    bob_id: Uuid,
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
        Fixture {
            repo,
            room,
            rate: PingRateLimiter::new(),
            alice,
            bob_id,
        }
    }

    async fn stored_message_doc(&self, cmd: &Command) -> Document {
        let Operation::Create { doc } = &cmd.ops[0] else {
            panic!("expected a Create op");
        };
        self.repo
            .get_document(doc.id)
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
