//! Integration proof: `handle_send_message`'s roll stage actually
//! executes dice notation at ingest, authors a `MessageKind::System` whisper
//! notice on failure instead of the intended message, interleaves inline
//! rolls/buttons with sanitized text, and never re-executes a roll on edit.
//! Drives `handle_send_message`/`handle_edit_message` directly, mirroring
//! `chat_content`'s fixture shape.

use shadowcat::chat::{
    build_link_preview_client, handle_edit_message, handle_send_message, Audience,
    LinkPreviewCache, LinkPreviewDeps, MessageEngine, MessageKind, MessageRequestCtx,
    PreviewRateLimiter, Segment, SendMessageError,
};
use shadowcat::data::command::{Command, Operation};
use shadowcat::data::document::{Document, WorldRole};
use shadowcat::data::membership::PermissionContext;
use shadowcat::data::permission::{cap, resolve_access};
use shadowcat::data::repository::Repository;
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::ws::room::{Room, RoomRegistry};
use shadowcat::ws::PingRateLimiter;
use std::sync::Arc;
use uuid::Uuid;

use shadowcat::auth::role::ServerRole;

struct Fixture {
    repo: SqliteRepository,
    room: Arc<Room>,
    rate: PingRateLimiter,
    // Link-preview deps: no test built through `Fixture` enables `hyperlinks`, so
    // `previews_enabled()` is always false and `enrich` never fetches — a
    // production (non-loopback) client is safe here, it's simply never
    // dialed.
    preview_client: reqwest::Client,
    preview_cache: LinkPreviewCache,
    preview_rate: PreviewRateLimiter,
    alice: PermissionContext,
    alice_id: Uuid,
    bob_id: Uuid,
}

impl Fixture {
    /// GM + `alice` (Player) + `bob` (Player), no `chat-settings`/`dice-settings`
    /// docs (default Total/HighWins context, plain-text content policy).
    async fn new() -> Self {
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
            preview_client: build_link_preview_client(),
            preview_cache: LinkPreviewCache::new(),
            preview_rate: PreviewRateLimiter::new(),
            alice,
            alice_id,
            bob_id,
        }
    }

    async fn send(&self, content: &str) -> Result<Command, SendMessageError> {
        handle_send_message(
            MessageRequestCtx {
                room: &self.room,
                repo: &self.repo,
                ctx: &self.alice,
                rate: &self.rate,
                preview: LinkPreviewDeps {
                    client: &self.preview_client,
                    cache: &self.preview_cache,
                    rate: &self.preview_rate,
                },
                now: 1,
                budget_per_min: 60,
            },
            "all".into(),
            content.into(),
            None,
            Audience::Public,
        )
        .await
        .map(|(cmd, _pending)| cmd)
    }

    async fn stored_message_doc(&self, cmd: &Command) -> Document {
        let doc_id = match &cmd.ops[0] {
            Operation::Create { doc } => doc.id,
            Operation::Update { doc_id, .. } => *doc_id,
            Operation::Move { doc_id, .. } => *doc_id,
            Operation::Delete { doc } => doc.id,
        };
        self.repo
            .get_document(doc_id)
            .await
            .unwrap()
            .expect("message doc persisted")
    }

    async fn stored_message_system(&self, cmd: &Command) -> MessageEngine {
        let doc = self.stored_message_doc(cmd).await;
        serde_json::from_value(doc.engine.unwrap()).unwrap()
    }
}

/// (a) `/roll 2d6+3` end-to-end: one message, `kind: Roll`, content is
/// exactly one `RollEmbed`, `outcome.total` in `[5,15]`, two dice records,
/// and `source` keeps the raw command text verbatim (as every other kind).
#[tokio::test]
async fn roll_result_message_end_to_end() {
    let f = Fixture::new().await;
    let cmd = f.send("/roll 2d6+3").await.unwrap();
    let sys = f.stored_message_system(&cmd).await;
    assert_eq!(sys.kind, MessageKind::Roll);
    assert_eq!(sys.source, Some("/roll 2d6+3".into()));
    match sys.content.as_slice() {
        [Segment::RollEmbed {
            formula, outcome, ..
        }] => {
            assert_eq!(formula, "2d6+3");
            assert!(
                (5..=15).contains(&outcome.total),
                "total out of range: {}",
                outcome.total
            );
            assert_eq!(outcome.records.len(), 2, "2d6 rolls exactly two dice");
        }
        other => panic!("expected one RollEmbed segment, got {other:?}"),
    }
}

/// (b) An inline `[[1d6]]` span inside a `Normal` body interleaves with the
/// surrounding text in scan order: Text, RollEmbed, Text.
#[tokio::test]
async fn inline_roll_interleaves_with_surrounding_text() {
    let f = Fixture::new().await;
    let cmd = f.send("attack! [[1d6]] done").await.unwrap();
    let sys = f.stored_message_system(&cmd).await;
    assert_eq!(sys.kind, MessageKind::Normal);
    match sys.content.as_slice() {
        [Segment::Text { text: t1 }, Segment::RollEmbed {
            formula, outcome, ..
        }, Segment::Text { text: t2 }] => {
            assert_eq!(t1, "attack! ");
            assert_eq!(formula, "1d6");
            assert!((1..=6).contains(&outcome.total));
            assert_eq!(t2, " done");
        }
        other => panic!("expected [Text, RollEmbed, Text], got {other:?}"),
    }
}

/// (c) A `[[roll:1d20|Attack]]` span stores a `RollButton` WITHOUT executing
/// — trimmed formula, label preserved.
#[tokio::test]
async fn roll_button_stored_unexecuted_with_trimmed_formula() {
    let f = Fixture::new().await;
    let cmd = f.send("[[roll:1d20|Attack]]").await.unwrap();
    let sys = f.stored_message_system(&cmd).await;
    assert_eq!(sys.kind, MessageKind::Normal);
    assert_eq!(
        sys.content,
        vec![Segment::RollButton {
            formula: "1d20".into(),
            label: Some("Attack".into()),
        }]
    );
}

/// (d) `/roll garbage` fails to parse: NO roll message is created; instead
/// exactly one `MessageKind::System` message is authored, whispered to the
/// sender only, on the same channel, owned by the sender, with a readable
/// error as its sole content segment.
#[tokio::test]
async fn failed_roll_authors_system_notice_not_a_message() {
    let f = Fixture::new().await;
    let cmd = f.send("/roll garbage").await.unwrap();
    let sys = f.stored_message_system(&cmd).await;
    assert_eq!(sys.kind, MessageKind::System);
    assert_eq!(sys.channel, "all");
    assert_eq!(sys.user_owner, f.alice_id);
    assert_eq!(
        sys.audience,
        Audience::Whisper {
            recipients: vec![f.alice_id]
        }
    );
    match sys.content.as_slice() {
        [Segment::Text { text }] => assert!(!text.is_empty(), "error text must be non-empty"),
        other => panic!("expected one Text segment, got {other:?}"),
    }
    // Exactly one message total was persisted for this attempt — no separate
    // Roll-kind message exists alongside the notice.
    let events = f.repo.events_since(f.room.world_id, 0).await.unwrap();
    assert_eq!(events.len(), 1, "one message per send attempt: {events:?}");
}

/// (e) A whisper body's `kind` is `Normal` (whisper bodies are never parsed
/// as commands), so `scan_body` still applies inside it and an inline roll
/// executes — the whisper's own audience machinery then governs visibility:
/// bob (the recipient) can read the doc; a non-recipient cannot.
#[tokio::test]
async fn inline_roll_executes_inside_whisper_body_and_stays_recipient_only() {
    let f = Fixture::new().await;
    let carol = f
        .repo
        .create_user("carol", None, ServerRole::User, 0)
        .await
        .unwrap();
    f.repo
        .add_member(f.room.world_id, carol, WorldRole::Player)
        .await
        .unwrap();

    let cmd = f.send("/w @bob [[1d6]]").await.unwrap();
    let sys = f.stored_message_system(&cmd).await;
    assert_eq!(sys.kind, MessageKind::Normal);
    assert!(
        matches!(sys.content.as_slice(), [Segment::RollEmbed { .. }]),
        "inline roll must execute inside a whisper body: {:?}",
        sys.content
    );
    assert_eq!(
        sys.audience,
        Audience::Whisper {
            recipients: vec![f.bob_id]
        }
    );

    let doc = f.stored_message_doc(&cmd).await;
    let bob_access = resolve_access(f.bob_id, WorldRole::Player, &doc, doc.owner);
    assert!(
        bob_access.has(cap::READ),
        "the addressed recipient reads it"
    );
    let carol_access = resolve_access(carol, WorldRole::Player, &doc, doc.owner);
    assert!(
        !carol_access.has(cap::READ),
        "a non-recipient must not read the embedded roll"
    );
}

/// Editing a message whose STORED `kind == Roll` is rejected outright.
#[tokio::test]
async fn edit_of_roll_message_is_immutable() {
    let f = Fixture::new().await;
    let sent = f.send("/roll 1d6").await.unwrap();
    let id = f.stored_message_doc(&sent).await.id;
    let r = handle_edit_message(
        MessageRequestCtx {
            room: &f.room,
            repo: &f.repo,
            ctx: &f.alice,
            rate: &f.rate,
            preview: LinkPreviewDeps {
                client: &f.preview_client,
                cache: &f.preview_cache,
                rate: &f.preview_rate,
            },
            now: 2,
            budget_per_min: 60,
        },
        id,
        "/roll 1d20".into(),
    )
    .await
    .map(|(cmd, _pending)| cmd);
    assert!(matches!(r, Err(SendMessageError::RollImmutable)), "{r:?}");
}

/// Editing a PLAIN public message's content INTO `/roll ...` is
/// rejected the same way — no editing a message into becoming a roll.
#[tokio::test]
async fn edit_into_roll_is_rejected() {
    let f = Fixture::new().await;
    let sent = f.send("hello").await.unwrap();
    let id = f.stored_message_doc(&sent).await.id;
    let r = handle_edit_message(
        MessageRequestCtx {
            room: &f.room,
            repo: &f.repo,
            ctx: &f.alice,
            rate: &f.rate,
            preview: LinkPreviewDeps {
                client: &f.preview_client,
                cache: &f.preview_cache,
                rate: &f.preview_rate,
            },
            now: 2,
            budget_per_min: 60,
        },
        id,
        "/roll 1d6".into(),
    )
    .await
    .map(|(cmd, _pending)| cmd);
    assert!(matches!(r, Err(SendMessageError::RollImmutable)), "{r:?}");
    // The stored message must be untouched by the rejected edit attempt.
    let doc = f.repo.get_document(id).await.unwrap().unwrap();
    let sys: MessageEngine = serde_json::from_value(doc.engine.unwrap()).unwrap();
    assert_eq!(sys.kind, MessageKind::Normal);
    assert_eq!(
        sys.content,
        vec![Segment::Text {
            text: "hello".into()
        }]
    );
}

/// An edit's `[[...]]` content stays LITERAL text — `scan_body` never
/// runs on an edit, so an inline-roll-shaped edit body sanitizes as ordinary
/// text, never producing a `RollEmbed`.
#[tokio::test]
async fn edit_content_with_inline_span_stays_literal_text() {
    let f = Fixture::new().await;
    let sent = f.send("hello").await.unwrap();
    let id = f.stored_message_doc(&sent).await.id;
    let edited = handle_edit_message(
        MessageRequestCtx {
            room: &f.room,
            repo: &f.repo,
            ctx: &f.alice,
            rate: &f.rate,
            preview: LinkPreviewDeps {
                client: &f.preview_client,
                cache: &f.preview_cache,
                rate: &f.preview_rate,
            },
            now: 2,
            budget_per_min: 60,
        },
        id,
        "[[1d6]]".into(),
    )
    .await
    .map(|(cmd, _pending)| cmd)
    .unwrap();
    let sys = f.stored_message_system(&edited).await;
    assert_eq!(sys.kind, MessageKind::Normal);
    assert_eq!(
        sys.content,
        vec![Segment::Text {
            text: "[[1d6]]".into()
        }],
        "an edit must never execute an inline roll span"
    );
}

/// (g) A stored `MessageEngine` JSON with no roll segments still
/// round-trips — the roll `Segment` variants are additive. RollOutcome
/// missing-key back-compat is pinned separately in `dice::outcome`'s
/// `roll_outcome_missing_defaulted_keys_deserializes`.
#[test]
fn stored_message_without_roll_segments_still_deserializes() {
    let j = serde_json::json!({
        "channel": "all",
        "user_owner": Uuid::from_u128(1),
        "kind": "normal",
        "audience": { "kind": "public" },
        "content": [{ "kind": "text", "text": "hi" }],
    });
    let sys: MessageEngine = serde_json::from_value(j).unwrap();
    assert_eq!(sys.kind, MessageKind::Normal);
    assert_eq!(sys.content, vec![Segment::Text { text: "hi".into() }]);
}

/// (h) A body with more than `MAX_INLINE_ROLLS` (8) non-text spans fails
/// `scan_body`'s cap check: no message is created, a System notice is
/// authored instead — same one-message-per-attempt budget as any other roll
/// failure.
#[tokio::test]
async fn over_max_inline_rolls_authors_system_notice_not_a_message() {
    let f = Fixture::new().await;
    let body = "[[1d6]]".repeat(9); // MAX_INLINE_ROLLS = 8, one past the cap
    let cmd = f.send(&body).await.unwrap();
    let sys = f.stored_message_system(&cmd).await;
    assert_eq!(sys.kind, MessageKind::System);
    match sys.content.as_slice() {
        [Segment::Text { text }] => assert!(!text.is_empty()),
        other => panic!("expected one Text segment, got {other:?}"),
    }
    let events = f.repo.events_since(f.room.world_id, 0).await.unwrap();
    assert_eq!(events.len(), 1, "one message per send attempt: {events:?}");
}
