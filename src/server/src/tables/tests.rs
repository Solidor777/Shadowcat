use super::*;
use crate::auth::role::ServerRole;
use crate::chat::{Audience, ChatContentPolicy, Segment, CHAT_SETTINGS_DOC_TYPE};
use crate::data::command::{Operation, UnsequencedCommand, WriteOrigin};
use crate::data::document::{DocRole, Document, PermissionSet, Scope, WorldRole};
use crate::data::sqlite::SqliteRepository;
use crate::ws::room::RoomRegistry;
use std::collections::BTreeMap;

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

/// A three-row weighted table (cumulative weights [3, 6, 10], `1d10`) --
/// used where a test asserts WHICH row a seeded roll matches, rather than a
/// single-row always-absorbs-the-roll fixture.
fn multi_row_table_doc(id: Uuid, world: Uuid) -> Document {
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
            "rows": [
                { "weight": 3, "label": "a", "results": [] },
                { "weight": 3, "label": "b", "results": [] },
                { "weight": 4, "label": "c", "results": [] }
            ],
            "description": ""
        })),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    }
}

async fn world_with_multi_row_table() -> (SqliteRepository, Uuid, Uuid, Uuid, Uuid) {
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
            doc: multi_row_table_doc(table_id, w.id),
        }],
    })
    .await
    .unwrap();
    (repo, w.id, gm, player, table_id)
}

/// Extracts the drawn row's label from a `handle_draw_table` command's
/// single `Segment::TableDraw` content entry.
fn drawn_row_label(cmd: &crate::data::command::Command) -> Option<String> {
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc,
        other => panic!("expected Create, got {other:?}"),
    };
    let sys: crate::chat::MessageEngine =
        serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    match &sys.content[0] {
        Segment::TableDraw(seg) => seg.row.as_ref().map(|r| r.label.clone()),
        other => panic!("expected TableDraw, got {other:?}"),
    }
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

    let (cmd, _pending) = handle_draw_table(
        DrawTableRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: crate::chat::LinkPreviewDeps {
                client: &crate::chat::build_link_preview_client(),
                cache: &crate::chat::LinkPreviewCache::new(),
                rate: &crate::chat::PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
            seed: None,
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
            preview: crate::chat::LinkPreviewDeps {
                client: &crate::chat::build_link_preview_client(),
                cache: &crate::chat::LinkPreviewCache::new(),
                rate: &crate::chat::PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
            seed: None,
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
            preview: crate::chat::LinkPreviewDeps {
                client: &crate::chat::build_link_preview_client(),
                cache: &crate::chat::LinkPreviewCache::new(),
                rate: &crate::chat::PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 1,
            seed: None,
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
            preview: crate::chat::LinkPreviewDeps {
                client: &crate::chat::build_link_preview_client(),
                cache: &crate::chat::LinkPreviewCache::new(),
                rate: &crate::chat::PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 1,
            seed: None,
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
            preview: crate::chat::LinkPreviewDeps {
                client: &crate::chat::build_link_preview_client(),
                cache: &crate::chat::LinkPreviewCache::new(),
                rate: &crate::chat::PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
            seed: None,
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

    let (cmd, _pending) = handle_draw_table(
        DrawTableRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: crate::chat::LinkPreviewDeps {
                client: &crate::chat::build_link_preview_client(),
                cache: &crate::chat::LinkPreviewCache::new(),
                rate: &crate::chat::PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
            seed: None,
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

#[tokio::test]
async fn a_whisper_draw_reaches_only_its_recipients() {
    let (repo, world, gm, player, table_id) = world_with_multi_row_table().await;
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world).await.unwrap().unwrap();
    let rate = crate::ws::PingRateLimiter::new();

    let (cmd, _pending) = handle_draw_table(
        DrawTableRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: crate::chat::LinkPreviewDeps {
                client: &crate::chat::build_link_preview_client(),
                cache: &crate::chat::LinkPreviewCache::new(),
                rate: &crate::chat::PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
            // 1d10 total 1 under seed 5 -> row "a" (cumulative band [1,3]),
            // a genuine multi-row selection rather than a single always-hit
            // row absorbing whatever the (otherwise unseeded) roll produces.
            seed: Some(5),
        },
        table_id,
        "general".into(),
        1,
        None,
        Audience::Whisper {
            recipients: vec![player],
        },
    )
    .await
    .unwrap();

    assert_eq!(drawn_row_label(&cmd), Some("a".to_string()));
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc.clone(),
        other => panic!("expected Create, got {other:?}"),
    };
    // Whisper mapping (chat::build_message_doc): default None, gm_role
    // Some(DocRole::None) (a non-addressed GM does NOT see a whisper by
    // default), users = {owner: Owner, ...recipients: Observer}.
    assert_eq!(doc.permissions.default, DocRole::None);
    assert_eq!(doc.permissions.gm_role, Some(DocRole::None));
    assert_eq!(doc.permissions.users.get(&gm), Some(&DocRole::Owner));
    assert_eq!(doc.permissions.users.get(&player), Some(&DocRole::Observer));
    assert_eq!(doc.permissions.users.len(), 2);
}

#[tokio::test]
async fn a_gm_only_draw_reaches_no_player() {
    let (repo, world, gm, player, table_id) = world_with_multi_row_table().await;
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, world).await.unwrap().unwrap();
    let rate = crate::ws::PingRateLimiter::new();

    let (cmd, _pending) = handle_draw_table(
        DrawTableRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: crate::chat::LinkPreviewDeps {
                client: &crate::chat::build_link_preview_client(),
                cache: &crate::chat::LinkPreviewCache::new(),
                rate: &crate::chat::PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
            // 1d10 total 6 under seed 3 -> row "b" (cumulative band [4,6]).
            seed: Some(3),
        },
        table_id,
        "general".into(),
        1,
        None,
        Audience::GmOnly,
    )
    .await
    .unwrap();

    assert_eq!(drawn_row_label(&cmd), Some("b".to_string()));
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc.clone(),
        other => panic!("expected Create, got {other:?}"),
    };
    // GmOnly mapping: default None, gm_role Some(Observer) (any current GM
    // sees it, re-resolved dynamically), users = {owner: Owner} only -- the
    // player is named nowhere.
    assert_eq!(doc.permissions.default, DocRole::None);
    assert_eq!(doc.permissions.gm_role, Some(DocRole::Observer));
    assert_eq!(doc.permissions.users.get(&gm), Some(&DocRole::Owner));
    assert_eq!(doc.permissions.users.get(&player), None);
    assert_eq!(doc.permissions.users.len(), 1);
}

/// A table with one row whose `TableEntry::Text` carries a markdown inline
/// image, in a world whose `chat-settings` enables markdown+images.
fn table_with_image_row(id: Uuid, world: Uuid) -> Document {
    let mut doc = table_doc(id, world);
    doc.engine = Some(serde_json::json!({
        "draw": { "kind": "weighted" },
        "rows": [{
            "weight": 1,
            "label": "a",
            "results": [{ "kind": "text", "text": "![a map](https://x.example/a.png)" }],
        }],
        "description": ""
    }));
    doc
}

#[tokio::test]
async fn a_row_text_inline_image_reaches_link_preview_enrichment() {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    crate::data::world_seed::seed_test_channel_registry(&repo, w.id, &[]).await;
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    // Enable markdown+images so `chat::sanitize` collects `image_urls` for a
    // row's `TableEntry::Text` -- mirrors `chat::link_preview_ingest_tests`'
    // own `Fixture::new` chat-settings seeding.
    let policy = ChatContentPolicy {
        markdown: Some(true),
        images: Some(true),
        ..Default::default()
    };
    let settings_doc = Document {
        id: Uuid::new_v4(),
        scope: Scope::World { world_id: w.id },
        doc_type: CHAT_SETTINGS_DOC_TYPE.to_string(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: Some(gm),
        permissions: PermissionSet::default(),
        embedded: BTreeMap::new(),
        parent_id: None,
        engine: Some(serde_json::to_value(policy).unwrap()),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    };
    let table_id = Uuid::new_v4();
    repo.apply_intent(
        &gm_ctx,
        w.id,
        vec![
            Operation::Create { doc: settings_doc },
            Operation::Create {
                doc: table_with_image_row(table_id, w.id),
            },
        ],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = crate::ws::PingRateLimiter::new();

    let (_cmd, pending) = handle_draw_table(
        DrawTableRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &gm_ctx,
            rate: &rate,
            preview: crate::chat::LinkPreviewDeps {
                client: &crate::chat::build_link_preview_client(),
                cache: &crate::chat::LinkPreviewCache::new(),
                rate: &crate::chat::PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
            seed: None,
        },
        table_id,
        "general".into(),
        1,
        None,
        Audience::Public,
    )
    .await
    .unwrap();

    assert!(
        pending
            .iter()
            .any(|p| matches!(p, crate::chat::PendingEnrichment::InlineImage { image_url, .. } if image_url == "https://x.example/a.png")),
        "expected an InlineImage enrichment job for the row's markdown image, got {pending:?}"
    );
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
