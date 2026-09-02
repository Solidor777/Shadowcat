use super::*;
use crate::auth::role::ServerRole;
use crate::chat::{Audience, Segment};
use crate::data::command::{Operation, UnsequencedCommand};
use crate::data::document::{DocRole, Document, PermissionSet, Scope, WorldRole};
use crate::data::sqlite::SqliteRepository;
use crate::ws::room::RoomRegistry;

fn table_doc(id: Uuid, world: Uuid) -> Document {
    Document {
        id,
        scope: Scope::World { world_id: world },
        doc_type: "table".into(),
        schema_version: 1,
        name: Some("T".into()),
        source: None,
        base: None,
        owner: None,
        permissions: PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::json!({
            "draw": { "kind": "weighted" },
            "rows": [{ "weight": 1, "label": "a", "results": [] }],
            "description": ""
        })),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    }
}

async fn world_with_table() -> (SqliteRepository, Uuid, Uuid, Uuid, Uuid) {
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
    let table_id = Uuid::new_v4();
    repo.apply_command(UnsequencedCommand {
        world_id: w.id,
        author: gm,
        ts: 0,
        ops: vec![Operation::Create {
            doc: table_doc(table_id, w.id),
        }],
    })
    .await
    .unwrap();
    (repo, w.id, gm, player, table_id)
}

#[tokio::test]
async fn a_draw_posts_a_public_roll_message_with_a_table_draw_segment() {
    let (repo, world, gm, _player, table_id) = world_with_table().await;
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world).await.unwrap().unwrap();
    let rate = crate::ws::PingRateLimiter::new();

    let cmd = handle_draw_table(
        DrawTableRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            now: 100,
            budget_per_min: 30,
        },
        table_id,
        "general".into(),
        1,
        None,
        Audience::Public,
    )
    .await
    .unwrap();

    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc.clone(),
        other => panic!("expected Create, got {other:?}"),
    };
    let engine = doc.engine.unwrap();
    assert_eq!(engine["kind"], "roll");
    assert_eq!(engine["content"][0]["kind"], "table_draw");
    assert_eq!(engine["content"][0]["table_id"], table_id.to_string());
}

#[tokio::test]
async fn an_unknown_channel_is_refused() {
    let (repo, world, gm, _player, table_id) = world_with_table().await;
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world).await.unwrap().unwrap();
    let rate = crate::ws::PingRateLimiter::new();

    let err = handle_draw_table(
        DrawTableRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            now: 100,
            budget_per_min: 30,
        },
        table_id,
        "does-not-exist".into(),
        1,
        None,
        Audience::Public,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DrawTableError::UnknownChannel));
}

#[tokio::test]
async fn the_flood_budget_refuses_a_draw_over_the_limit() {
    let (repo, world, gm, _player, table_id) = world_with_table().await;
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world).await.unwrap().unwrap();
    let rate = crate::ws::PingRateLimiter::new();

    // Exhaust a budget of 1 with a first call, then confirm the second fails.
    let ok = handle_draw_table(
        DrawTableRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            now: 100,
            budget_per_min: 1,
        },
        table_id,
        "general".into(),
        1,
        None,
        Audience::Public,
    )
    .await;
    assert!(ok.is_ok());

    let err = handle_draw_table(
        DrawTableRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            now: 100,
            budget_per_min: 1,
        },
        table_id,
        "general".into(),
        1,
        None,
        Audience::Public,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DrawTableError::RateLimited));
}

#[tokio::test]
async fn a_count_over_max_top_level_draws_is_too_many() {
    let (repo, world, gm, _player, table_id) = world_with_table().await;
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world).await.unwrap().unwrap();
    let rate = crate::ws::PingRateLimiter::new();

    let err = handle_draw_table(
        DrawTableRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            now: 100,
            budget_per_min: 30,
        },
        table_id,
        "general".into(),
        MAX_TOP_LEVEL_DRAWS + 1,
        None,
        Audience::Public,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DrawTableError::TooMany));
}

#[tokio::test]
async fn recalc_on_a_table_draws_roll_id_is_roll_not_found() {
    let (repo, world, gm, _player, table_id) = world_with_table().await;
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world).await.unwrap().unwrap();
    let rate = crate::ws::PingRateLimiter::new();

    let cmd = handle_draw_table(
        DrawTableRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            now: 100,
            budget_per_min: 30,
        },
        table_id,
        "general".into(),
        1,
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let message_id = match &cmd.ops[0] {
        Operation::Create { doc } => doc.id,
        other => panic!("expected Create, got {other:?}"),
    };
    let doc = repo.get_document(message_id).await.unwrap().unwrap();
    let sys: crate::chat::MessageEngine =
        serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    let roll_id = match &sys.content[0] {
        Segment::TableDraw(seg) => seg.roll_id,
        other => panic!("expected TableDraw, got {other:?}"),
    };

    let err = crate::chat::handle_recalc_roll(
        crate::chat::RecalcRollRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            now: 100,
            budget_per_min: 30,
        },
        message_id,
        roll_id,
        vec![],
    )
    .await
    .unwrap_err();
    assert!(matches!(err, crate::chat::RecalcRollError::RollNotFound));
}

#[test]
fn draw_table_error_display_has_no_debug_artifacts() {
    let variants: Vec<DrawTableError> = vec![
        DrawTableError::RateLimited,
        DrawTableError::UnknownChannel,
        DrawTableError::UnknownRecipient,
        DrawTableError::ActorNotSpeakable,
        DrawTableError::Forbidden,
        DrawTableError::NotFound,
        DrawTableError::TooMany,
        DrawTableError::TooDeep,
        DrawTableError::Cycle,
        DrawTableError::EmptyTable,
        DrawTableError::MissingAsset,
        DrawTableError::Roll(crate::chat::rolls::RollError::Unterminated),
        DrawTableError::Data(crate::data::DataError::NotFound),
        DrawTableError::TooLong,
    ];
    assert_eq!(
        variants.len(),
        14,
        "update this test if a DrawTableError variant is added or removed"
    );
    for v in variants {
        let rendered = v.to_string();
        assert!(!rendered.contains("Some("), "{rendered}");
        assert!(!rendered.is_empty());
    }
}
