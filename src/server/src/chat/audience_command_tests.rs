//! These tests drive the audience and display-kind chat commands through the ingest path.
//!
//! They call `handle_send_message`/`handle_edit_message` directly against an in-memory
//! repository and assert the stored `MessageEngine` (kind, audience, source).

use super::*;
use crate::auth::role::ServerRole;
use crate::data::document::WorldRole;
use crate::data::sqlite::SqliteRepository;
use crate::ws::room::RoomRegistry;

/// A world with a GM and two players (`alice`, `bob`) plus a third player, `sender`, who
/// authors the messages under test.
struct Fixture {
    /// In-memory repository.
    repo: SqliteRepository,
    /// The world's room.
    room: std::sync::Arc<Room>,
    /// The world uuid.
    world: Uuid,
    /// The world's GM.
    gm: Uuid,
    /// First other player.
    alice: Uuid,
    /// Second other player.
    bob: Uuid,
    /// The author of the messages under test.
    sender: Uuid,
    /// Shared flood limiter.
    rate: PingRateLimiter,
}

impl Fixture {
    /// Builds the world and seats every member.
    async fn new() -> Self {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        crate::data::world_seed::seed_test_channel_registry(&repo, w.id, &[]).await;
        let mut ids = Vec::new();
        for name in ["alice", "bob", "sender"] {
            let id = repo
                .create_user(name, None, ServerRole::User, 0)
                .await
                .unwrap();
            repo.add_member(w.id, id, WorldRole::Player).await.unwrap();
            ids.push(id);
        }
        let room = RoomRegistry::new()
            .get_or_create(&repo, w.id)
            .await
            .unwrap()
            .unwrap();
        Self {
            repo,
            room,
            world: w.id,
            gm,
            alice: ids[0],
            bob: ids[1],
            sender: ids[2],
            rate: PingRateLimiter::new(),
        }
    }

    /// The permission context of `user` (GM role for the GM, player otherwise).
    fn ctx(&self, user: Uuid) -> PermissionContext {
        PermissionContext {
            user_id: user,
            world_role: if user == self.gm {
                WorldRole::Gm
            } else {
                WorldRole::Player
            },
        }
    }

    /// Sends `content` as `user` with the frame audience `audience`.
    async fn try_send(
        &self,
        user: Uuid,
        content: &str,
        audience: Audience,
        now: i64,
    ) -> Result<Document, SendMessageError> {
        let ctx = self.ctx(user);
        let (cmd, _pending) = handle_send_message(
            MessageRequestCtx {
                room: &self.room,
                repo: &self.repo,
                ctx: &ctx,
                rate: &self.rate,
                vfx_rate: &self.rate,
                preview: LinkPreviewDeps {
                    client: &super::link_preview::build_client_allow_loopback(),
                    cache: &LinkPreviewCache::new(),
                    rate: &PreviewRateLimiter::new(),
                },
                now,
                budget_per_min: 1000,
            },
            "general".into(),
            content.into(),
            None,
            audience,
            None,
        )
        .await?
        .expect("a message-authoring send");
        match &cmd.ops[0] {
            Operation::Create { doc } => Ok(doc.clone()),
            other => panic!("expected Create, got {other:?}"),
        }
    }

    /// Sends `content` as `user` with a public frame audience and unwraps success.
    async fn send(&self, user: Uuid, content: &str, now: i64) -> MessageEngine {
        engine_of(
            &self
                .try_send(user, content, Audience::Public, now)
                .await
                .unwrap(),
        )
    }

    /// Edits message `id` as `user`.
    async fn edit(&self, user: Uuid, id: Uuid, content: &str) -> Result<(), SendMessageError> {
        let ctx = self.ctx(user);
        handle_edit_message(
            MessageRequestCtx {
                room: &self.room,
                repo: &self.repo,
                ctx: &ctx,
                rate: &self.rate,
                vfx_rate: &self.rate,
                preview: LinkPreviewDeps {
                    client: &super::link_preview::build_client_allow_loopback(),
                    cache: &LinkPreviewCache::new(),
                    rate: &PreviewRateLimiter::new(),
                },
                now: 5_000,
                budget_per_min: 1000,
            },
            id,
            content.into(),
        )
        .await
        .map(|_| ())
    }
}

/// Decodes a stored message document's engine body.
fn engine_of(doc: &Document) -> MessageEngine {
    serde_json::from_value(doc.engine.clone().unwrap()).unwrap()
}

#[tokio::test]
async fn gmroll_authors_a_roll_readable_by_the_gm_only() {
    let f = Fixture::new().await;
    let m = f.send(f.sender, "/gmroll 1d20", 10).await;
    assert_eq!(m.kind, MessageKind::Roll);
    assert_eq!(m.audience, Audience::GmOnly);
    assert_eq!(m.source.as_deref(), Some("1d20"));
}

#[tokio::test]
async fn publicroll_overrides_a_non_public_frame_audience() {
    let f = Fixture::new().await;
    let doc = f
        .try_send(f.sender, "/publicroll 1d20", Audience::GmOnly, 10)
        .await
        .unwrap();
    let m = engine_of(&doc);
    assert_eq!(m.kind, MessageKind::Roll);
    assert_eq!(m.audience, Audience::Public);
}

#[tokio::test]
async fn gm_command_sends_normal_text_to_the_gm_only() {
    let f = Fixture::new().await;
    let m = f.send(f.sender, "/gm the door is trapped", 10).await;
    assert_eq!(m.kind, MessageKind::Normal);
    assert_eq!(m.audience, Audience::GmOnly);
    assert_eq!(m.source.as_deref(), Some("the door is trapped"));
}

#[tokio::test]
async fn ooc_sets_the_kind_without_touching_the_audience_and_ic_resets_it() {
    let f = Fixture::new().await;
    let ooc = f.send(f.sender, "/ooc brb", 10).await;
    assert_eq!(ooc.kind, MessageKind::Ooc);
    assert_eq!(ooc.audience, Audience::Public);
    let ic = f.send(f.sender, "/ic back", 11).await;
    assert_eq!(ic.kind, MessageKind::Normal);
    assert_eq!(ic.audience, Audience::Public);
}

#[tokio::test]
async fn players_snapshots_the_non_gm_members_at_send_time() {
    let f = Fixture::new().await;
    let first = f.send(f.sender, "/players hi", 10).await;
    let roster = |m: &MessageEngine| -> std::collections::BTreeSet<Uuid> {
        match &m.audience {
            Audience::Whisper { recipients } => recipients.iter().copied().collect(),
            other => panic!("expected Whisper, got {other:?}"),
        }
    };
    assert_eq!(
        roster(&first),
        [f.alice, f.bob].into_iter().collect(),
        "every non-GM member except the sender"
    );

    // A second GM and a late player join between the two sends.
    let gm2 = f
        .repo
        .create_user("gm2", None, ServerRole::User, 0)
        .await
        .unwrap();
    f.repo
        .add_member(f.world, gm2, WorldRole::Gm)
        .await
        .unwrap();
    let carol = f
        .repo
        .create_user("carol", None, ServerRole::User, 0)
        .await
        .unwrap();
    f.repo
        .add_member(f.world, carol, WorldRole::Player)
        .await
        .unwrap();
    let second = f.send(f.sender, "/players again", 11).await;
    assert_eq!(
        roster(&second),
        [f.alice, f.bob, carol].into_iter().collect(),
        "the second GM is excluded and the late player is included"
    );
    assert!(!roster(&second).contains(&gm2));
    assert!(!roster(&second).contains(&f.gm));
    assert!(
        !roster(&first).contains(&carol),
        "the first snapshot is not retroactively changed"
    );
}

#[tokio::test]
async fn reply_without_a_prior_whisper_is_no_reply_target() {
    let f = Fixture::new().await;
    // A whisper the sender WROTE and a whisper between two others do not count.
    f.send(f.sender, "/w @alice hello", 10).await;
    f.send(f.alice, "/w @bob psst", 11).await;
    let err = f
        .try_send(f.sender, "/reply hi", Audience::Public, 12)
        .await
        .unwrap_err();
    assert!(matches!(err, SendMessageError::NoReplyTarget));
    assert_eq!(err.to_string(), "There is no whisper to reply to.");
}

#[tokio::test]
async fn reply_targets_the_most_recent_whisper_not_the_first() {
    let f = Fixture::new().await;
    f.send(f.alice, "/w @sender first", 10).await;
    f.send(f.bob, "/w @sender second", 11).await;
    let m = f.send(f.sender, "/reply got it", 12).await;
    assert_eq!(m.kind, MessageKind::Normal);
    assert_eq!(
        m.audience,
        Audience::Whisper {
            recipients: vec![f.bob]
        }
    );
    assert_eq!(m.source.as_deref(), Some("got it"));

    // Another whisper from alice moves the target back to her.
    f.send(f.alice, "/w @sender third", 13).await;
    let m = f.send(f.sender, "/reply and you", 14).await;
    assert_eq!(
        m.audience,
        Audience::Whisper {
            recipients: vec![f.alice]
        }
    );
}

#[tokio::test]
async fn reply_skips_a_deleted_whisper() {
    let f = Fixture::new().await;
    f.send(f.alice, "/w @sender first", 10).await;
    let newest = f
        .try_send(f.bob, "/w @sender second", Audience::Public, 11)
        .await
        .unwrap();
    handle_delete_message(
        &f.room,
        &f.repo,
        &f.ctx(f.bob),
        &f.rate,
        newest.id,
        12,
        1000,
    )
    .await
    .unwrap();
    let m = f.send(f.sender, "/reply ok", 13).await;
    assert_eq!(
        m.audience,
        Audience::Whisper {
            recipients: vec![f.alice]
        }
    );
}

#[tokio::test]
async fn editing_in_any_audience_command_is_audience_locked() {
    let f = Fixture::new().await;
    let id = f
        .try_send(f.sender, "hello", Audience::Public, 10)
        .await
        .unwrap()
        .id;
    for content in [
        "/w @alice hi",
        "/gm hi",
        "/gmroll 1d20",
        "/publicroll 1d20",
        "/blindroll 1d20",
        "/br 1d20",
        "/selfroll 1d20",
        "/sr 1d20",
        "/players hi",
        "/reply hi",
    ] {
        let err = f.edit(f.sender, id, content).await.unwrap_err();
        assert!(
            matches!(err, SendMessageError::AudienceLocked),
            "{content}: {err:?}"
        );
    }
    // `/ooc` carries no audience, so an edit may use it.
    f.edit(f.sender, id, "/ooc hi").await.unwrap();
}
