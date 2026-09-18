//! `parse_fx_body`/`try_handle_fx` unit cases plus a full `handle_send_message`
//! integration pass over a real `SqliteRepository` + `Room`: the success
//! broadcast, the whispered failure notice, and the no-existence-oracle rule.

use super::*;
use crate::auth::role::ServerRole;
use crate::chat::{
    handle_send_message, Audience, LinkPreviewCache, LinkPreviewDeps, MessageEngine, MessageKind,
    MessageRequestCtx, PreviewRateLimiter,
};
use crate::data::document::{DocRole, Document, PermissionSet, Scope, WorldRole};
use crate::data::sqlite::SqliteRepository;
use crate::ws::room::{Room, RoomRegistry};
use crate::ws::PingRateLimiter;

#[test]
fn parse_fx_body_splits_the_asset_word_from_the_at_tail() {
    let p = parse_fx_body("fireball @Big Red Dragon").unwrap();
    assert_eq!(p.0, "fireball");
    assert_eq!(p.1.as_deref(), Some("Big Red Dragon"));
    assert!(parse_fx_body("").is_none());
    assert!(parse_fx_body("   ").is_none());
    assert_eq!(parse_fx_body("fireball").unwrap().1, None);
    assert_eq!(parse_fx_body("fireball extra garbage").unwrap().1, None);
    // The FIRST whitespace-separated word is the asset ref even when it starts
    // with `@` itself — an `/fx @Bob` alone is therefore a NoTarget, an
    // accepted quirk of "first word is the asset ref".
    assert_eq!(parse_fx_body("@Bob").unwrap(), ("@Bob".to_string(), None));
    // An `@` with no name tail parses as an empty name (the caller refuses it).
    assert_eq!(parse_fx_body("fireball @").unwrap().1, Some(String::new()));
}

/// The shared integration fixture: a world with the "general" channel, an
/// active scene players may read, and a named token on it.
struct FxFixture {
    /// The repository behind everything.
    repo: SqliteRepository,
    /// The world's room.
    room: std::sync::Arc<Room>,
    /// The world's id.
    world: Uuid,
    /// The active scene's id.
    scene: Uuid,
    /// The player user id (a world member).
    player: Uuid,
}

impl FxFixture {
    /// Builds the world with one readable scene, one named token
    /// (`token_permissions` decides who may read it), and one asset named
    /// "fireball.webp".
    async fn new(token_permissions: DocRole) -> Self {
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
        crate::data::world_seed::seed_test_channel_registry(&repo, w.id, &[]).await;
        repo.add_member(w.id, player, WorldRole::Player)
            .await
            .unwrap();

        let scene = Uuid::new_v4();
        repo.seed_document_unvalidated(&Document {
            id: scene,
            scope: Scope::World { world_id: w.id },
            doc_type: "scene".into(),
            schema_version: 1,
            name: Some("Scene".into()),
            source: None,
            base: None,
            owner: None,
            permissions: PermissionSet {
                default: DocRole::Observer,
                users: Default::default(),
                property_overrides: Default::default(),
                capabilities: Default::default(),
                gm_role: None,
            },
            embedded: Default::default(),
            parent_id: None,
            engine: None,
            system: serde_json::json!({}),
            created_at: 0,
            updated_at: 0,
        })
        .await
        .unwrap();
        repo.seed_document_unvalidated(&Document {
            id: Uuid::new_v4(),
            scope: Scope::World { world_id: w.id },
            doc_type: "world-settings".into(),
            schema_version: 1,
            name: None,
            source: None,
            base: None,
            owner: None,
            permissions: PermissionSet {
                default: DocRole::Observer,
                users: Default::default(),
                property_overrides: Default::default(),
                capabilities: Default::default(),
                gm_role: None,
            },
            embedded: Default::default(),
            parent_id: None,
            engine: Some(serde_json::json!({ "activeScene": scene.to_string() })),
            system: serde_json::json!({}),
            created_at: 0,
            updated_at: 0,
        })
        .await
        .unwrap();
        repo.seed_document_unvalidated(&Document {
            id: Uuid::new_v4(),
            scope: Scope::World { world_id: w.id },
            doc_type: "token".into(),
            schema_version: 1,
            name: Some("Big Red Dragon".into()),
            source: None,
            base: None,
            owner: None,
            permissions: PermissionSet {
                default: token_permissions,
                users: Default::default(),
                property_overrides: Default::default(),
                capabilities: Default::default(),
                gm_role: None,
            },
            embedded: Default::default(),
            parent_id: Some(scene),
            engine: Some(serde_json::json!({
                "x": 10.0, "y": 20.0, "w": 1.0, "h": 1.0, "rotation": 0.0
            })),
            system: serde_json::json!({}),
            created_at: 0,
            updated_at: 0,
        })
        .await
        .unwrap();
        repo.insert_asset(&crate::data::asset::Asset {
            id: Uuid::new_v4(),
            world_id: w.id,
            storage_key: format!("{}/fireball", w.id),
            original_name: "fireball.webp".into(),
            content_type: "image/webp".into(),
            byte_size: 10,
            created_by: Some(gm),
            created_at: 0,
            version: 1,
            folder_id: None,
            tags: vec![],
            derived_tags: vec![],
            meta: crate::data::asset::AssetMeta::unprocessed("image/webp", 10),
        })
        .await
        .unwrap();

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        FxFixture {
            repo,
            room,
            world: w.id,
            scene,
            player,
        }
    }

    /// Sends `content` through the real `handle_send_message` as `sender` with
    /// `role`, returning its result.
    async fn send(
        &self,
        sender: Uuid,
        role: WorldRole,
        content: &str,
    ) -> Result<
        Option<(
            crate::data::command::Command,
            Vec<crate::chat::PendingEnrichment>,
        )>,
        crate::chat::SendMessageError,
    > {
        let ctx = PermissionContext {
            user_id: sender,
            world_role: role,
        };
        let rate = PingRateLimiter::new();
        let client = crate::chat::build_link_preview_client();
        handle_send_message(
            MessageRequestCtx {
                room: &self.room,
                repo: &self.repo,
                ctx: &ctx,
                rate: &rate,
                vfx_rate: &rate,

                preview: LinkPreviewDeps {
                    client: &client,
                    cache: &LinkPreviewCache::new(),
                    rate: &PreviewRateLimiter::new(),
                },
                now: 0,
                budget_per_min: 30,
            },
            "general".into(),
            content.into(),
            None,
            Audience::Public,
        )
        .await
    }
}

/// The whispered notice's `(kind, audience, text)` for a `/fx` failure send.
fn notice_of(
    result: &Option<(
        crate::data::command::Command,
        Vec<crate::chat::PendingEnrichment>,
    )>,
) -> (MessageKind, Audience, String) {
    let (cmd, _) = result.as_ref().expect("a failed /fx authors a notice");
    let doc = match &cmd.ops[0] {
        crate::data::command::Operation::Create { doc } => doc,
        other => panic!("expected Create, got {other:?}"),
    };
    let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    let text = match &sys.content[0] {
        crate::chat::Segment::Text { text } => text.clone(),
        other => panic!("expected a Text segment, got {other:?}"),
    };
    (sys.kind, sys.audience, text)
}

#[tokio::test]
async fn try_handle_fx_boundary_cases() {
    let f = FxFixture::new(DocRole::Observer).await;
    let ctx = PermissionContext {
        user_id: f.player,
        world_role: WorldRole::Player,
    };
    let vfx_rate = crate::ws::PingRateLimiter::new();
    // Not the command at all: falls through to `parse_command`.
    assert!(
        try_handle_fx(&f.repo, &f.room, &ctx, f.world, "hello", &vfx_rate, 0)
            .await
            .is_none()
    );
    // No word boundary: "/fxwhatever" is ordinary text, not the command.
    assert!(
        try_handle_fx(&f.repo, &f.room, &ctx, f.world, "/fxwhatever", &vfx_rate, 0)
            .await
            .is_none()
    );
    // A bare "/fx" IS the command and fails with the usage notice.
    assert!(matches!(
        try_handle_fx(&f.repo, &f.room, &ctx, f.world, "/fx", &vfx_rate, 0).await,
        Some(Err(FxError::NoTarget))
    ));
}

#[tokio::test]
async fn fx_charges_the_vfx_bucket_before_broadcast() {
    let f = FxFixture::new(DocRole::Observer).await;
    let ctx = PermissionContext {
        user_id: f.player,
        world_role: WorldRole::Player,
    };
    // A budget already spent by earlier plays refuses the next one — the SAME bucket the raw
    // `PlayVfx` frame spends against, so neither front door buys more plays than the other.
    let vfx_rate = crate::ws::PingRateLimiter::new();
    for i in 0..30 {
        assert!(vfx_rate.check(ctx.user_id, i, 30));
    }
    assert!(matches!(
        try_handle_fx(
            &f.repo,
            &f.room,
            &ctx,
            f.world,
            "/fx fireball.webp @Big Red Dragon",
            &vfx_rate,
            31
        )
        .await,
        Some(Err(FxError::Refused))
    ));
}

#[tokio::test]
async fn fx_by_asset_name_plays_at_the_named_tokens_center() {
    let f = FxFixture::new(DocRole::Observer).await;
    let (mut rx, _current) = f.room.subscribe();
    let result = f
        .send(
            f.player,
            WorldRole::Player,
            "/fx fireball.webp @Big Red Dragon",
        )
        .await
        .unwrap();
    assert!(
        result.is_none(),
        "a successful /fx authors no message document"
    );
    let frame = rx.recv().await.unwrap();
    let crate::ws::room::RoomEvent::Other(msg) = frame else {
        panic!("expected an out-of-band frame, got {frame:?}");
    };
    match &*msg {
        ServerMsg::Vfx {
            scene,
            user,
            asset,
            x,
            y,
            ..
        } => {
            assert_eq!(*scene, f.scene);
            assert_eq!(*user, f.player);
            assert_eq!((*x, *y), (10.0, 20.0));
            // The name resolved to the asset's real id.
            let named = f
                .repo
                .asset_id_by_name(f.world, "fireball.webp")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(asset, &named.to_string());
        }
        other => panic!("expected ServerMsg::Vfx, got {other:?}"),
    }
}

#[tokio::test]
async fn fx_with_an_unknown_token_whispers_a_system_notice() {
    let f = FxFixture::new(DocRole::Observer).await;
    let result = f
        .send(
            f.player,
            WorldRole::Player,
            "/fx fireball.webp @No Such Token",
        )
        .await
        .unwrap();
    let (kind, audience, text) = notice_of(&result);
    assert_eq!(kind, MessageKind::System);
    assert_eq!(
        audience,
        Audience::Whisper {
            recipients: vec![f.player]
        }
    );
    assert_eq!(text, FxError::UnknownToken.to_string());
}

#[tokio::test]
async fn fx_never_oracles_a_token_the_sender_cannot_read() {
    let f = FxFixture::new(DocRole::None).await;
    // The token exists (same name as the readable case) but the player has no
    // READ on it — the failure text is byte-identical to a genuinely
    // nonexistent name.
    let hidden = f
        .send(
            f.player,
            WorldRole::Player,
            "/fx fireball.webp @Big Red Dragon",
        )
        .await
        .unwrap();
    let (_, _, hidden_text) = notice_of(&hidden);
    let missing = f
        .send(
            f.player,
            WorldRole::Player,
            "/fx fireball.webp @Definitely Not Here",
        )
        .await
        .unwrap();
    let (_, _, missing_text) = notice_of(&missing);
    assert_eq!(hidden_text, missing_text);
    assert_eq!(hidden_text, FxError::UnknownToken.to_string());
}

#[tokio::test]
async fn fx_refuses_a_spectator_sender() {
    let f = FxFixture::new(DocRole::Observer).await;
    let (mut rx, _current) = f.room.subscribe();
    // The refusal reaches `vfx_permitted`, but authoring even the whispered
    // failure notice is a message write — which a spectator cannot make at all
    // (the same `Forbidden` every message they send meets), so the send errors
    // out and nothing broadcasts.
    let result = f
        .send(
            f.player,
            WorldRole::Spectator,
            "/fx fireball.webp @Big Red Dragon",
        )
        .await;
    assert!(
        result.is_err(),
        "a spectator cannot author the failure notice"
    );
    assert!(
        rx.try_recv().is_err(),
        "no vfx frame (and no notice) was broadcast"
    );
}

#[tokio::test]
async fn fx_with_an_unknown_asset_whispers_a_system_notice() {
    let f = FxFixture::new(DocRole::Observer).await;
    let result = f
        .send(
            f.player,
            WorldRole::Player,
            "/fx no-such-effect @Big Red Dragon",
        )
        .await
        .unwrap();
    let (_, _, text) = notice_of(&result);
    assert_eq!(text, FxError::UnknownAsset.to_string());
}

#[tokio::test]
async fn fx_without_a_target_whispers_the_usage_notice() {
    let f = FxFixture::new(DocRole::Observer).await;
    let result = f
        .send(f.player, WorldRole::Player, "/fx fireball.webp")
        .await
        .unwrap();
    let (_, _, text) = notice_of(&result);
    assert_eq!(text, FxError::NoTarget.to_string());
}
