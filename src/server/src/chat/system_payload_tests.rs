//! Tests for the opaque `system` payload a `SendMessage` carries onto the stored message document.

use super::*;
use crate::auth::role::ServerRole;
use crate::data::sqlite::SqliteRepository;
use crate::ws::room::RoomRegistry;

/// A seeded in-memory world with one player, a registered `general` channel and a room.
struct Fixture {
    /// The backing repository.
    repo: SqliteRepository,
    /// The world's live room.
    room: std::sync::Arc<Room>,
    /// The player's request context.
    ctx: PermissionContext,
    /// Shared flood-budget limiter.
    rate: PingRateLimiter,
    /// Link-preview HTTP client (never exercised: the test bodies carry no link).
    client: reqwest::Client,
    /// Link-preview cache.
    cache: LinkPreviewCache,
    /// Link-preview fetch limiter.
    preview_rate: PreviewRateLimiter,
}

impl Fixture {
    /// Builds the world, membership and room.
    async fn new() -> Self {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, ServerRole::User, 0)
            .await
            .unwrap();
        let player = repo
            .create_user("pl", None, ServerRole::User, 0)
            .await
            .unwrap();
        let world = repo.create_world_owned("W", gm, 0).await.unwrap();
        crate::data::world_seed::seed_test_channel_registry(&repo, world.id, &[]).await;
        repo.add_member(world.id, player, WorldRole::Player)
            .await
            .unwrap();
        let room = RoomRegistry::new()
            .get_or_create(&repo, world.id)
            .await
            .unwrap()
            .unwrap();
        Self {
            repo,
            room,
            ctx: PermissionContext {
                user_id: player,
                world_role: WorldRole::Player,
            },
            rate: PingRateLimiter::new(),
            client: link_preview::build_client_allow_loopback(),
            cache: LinkPreviewCache::new(),
            preview_rate: PreviewRateLimiter::new(),
        }
    }

    /// Request context for a call at `now`.
    fn req(&self, now: i64) -> MessageRequestCtx<'_> {
        MessageRequestCtx {
            room: &self.room,
            repo: &self.repo,
            ctx: &self.ctx,
            rate: &self.rate,
            vfx_rate: &self.rate,
            preview: LinkPreviewDeps {
                client: &self.client,
                cache: &self.cache,
                rate: &self.preview_rate,
            },
            now,
            budget_per_min: 30,
        }
    }

    /// Sends a public `hello` carrying `system`, returning the stored document id.
    async fn send(&self, system: Option<serde_json::Value>) -> Result<Uuid, SendMessageError> {
        let (cmd, _) = handle_send_message(
            self.req(100),
            "general".into(),
            "hello".into(),
            None,
            Audience::Public,
            system,
        )
        .await?
        .unwrap();
        match &cmd.ops[0] {
            Operation::Create { doc } => Ok(doc.id),
            other => panic!("expected Create, got {other:?}"),
        }
    }

    /// The stored message document.
    async fn stored(&self, id: Uuid) -> Document {
        self.repo.get_document(id).await.unwrap().unwrap()
    }
}

#[tokio::test]
async fn a_sent_system_payload_is_stored_verbatim() {
    let fx = Fixture::new().await;
    let payload =
        serde_json::json!({ "card": { "kind": "attack", "hits": [3, 7], "crit": false } });
    let id = fx.send(Some(payload.clone())).await.unwrap();
    assert_eq!(fx.stored(id).await.system, payload);
}

#[tokio::test]
async fn an_absent_system_payload_stores_an_empty_object() {
    let fx = Fixture::new().await;
    let id = fx.send(None).await.unwrap();
    assert_eq!(fx.stored(id).await.system, serde_json::json!({}));
}

#[tokio::test]
async fn an_oversized_system_payload_is_refused_and_persists_nothing() {
    let fx = Fixture::new().await;
    let big = serde_json::json!({
        "blob": "x".repeat(crate::data::validation::MAX_SYSTEM_BYTES + 1)
    });
    let err = fx.send(Some(big)).await.unwrap_err();
    assert!(
        matches!(err, SendMessageError::Data(DataError::TooLarge(_))),
        "an oversized message `system` is refused like any other document's: {err:?}"
    );
}

#[tokio::test]
async fn an_edit_leaves_the_system_payload_untouched() {
    let fx = Fixture::new().await;
    let payload = serde_json::json!({ "n": 1 });
    let id = fx.send(Some(payload.clone())).await.unwrap();
    handle_edit_message(fx.req(101), id, "goodbye".into())
        .await
        .unwrap();
    let stored = fx.stored(id).await;
    let engine: MessageEngine = serde_json::from_value(stored.engine.unwrap()).unwrap();
    assert_eq!(engine.source, Some("goodbye".into()));
    assert_eq!(stored.system, payload);
}

#[tokio::test]
async fn a_delete_tombstone_clears_the_system_payload() {
    let fx = Fixture::new().await;
    let id = fx
        .send(Some(serde_json::json!({ "secret": "s" })))
        .await
        .unwrap();
    handle_delete_message(&fx.room, &fx.repo, &fx.ctx, &fx.rate, id, 102, 30)
        .await
        .unwrap();
    let stored = fx.stored(id).await;
    let engine: MessageEngine = serde_json::from_value(stored.engine.unwrap()).unwrap();
    assert!(engine.deleted_at.is_some(), "the message is tombstoned");
    assert_eq!(
        stored.system,
        serde_json::json!({}),
        "a module payload must not survive on a deleted message"
    );
}

#[tokio::test]
async fn deleting_a_message_without_a_payload_still_tombstones_it() {
    let fx = Fixture::new().await;
    let id = fx.send(None).await.unwrap();
    handle_delete_message(&fx.room, &fx.repo, &fx.ctx, &fx.rate, id, 102, 30)
        .await
        .unwrap();
    let stored = fx.stored(id).await;
    let engine: MessageEngine = serde_json::from_value(stored.engine.unwrap()).unwrap();
    assert!(engine.deleted_at.is_some());
    assert_eq!(stored.system, serde_json::json!({}));
}
