use super::super::{DrawTableError, MAX_DRAWS_PER_REQUEST, MAX_DRAW_DEPTH};
use super::*;
use crate::auth::role::ServerRole;
use crate::data::command::{Operation, UnsequencedCommand};
use crate::data::document::{DocRole, Document, PermissionSet, Scope, WorldRole};
use crate::data::sqlite::SqliteRepository;
use uuid::Uuid;

fn weighted_table_doc(
    id: Uuid,
    world: Uuid,
    rows: serde_json::Value,
    default: DocRole,
) -> Document {
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
            default,
            ..Default::default()
        },
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::json!({
            "draw": { "kind": "weighted" },
            "rows": rows,
            "description": ""
        })),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    }
}

fn formula_table_doc(id: Uuid, world: Uuid, notation: &str, rows: serde_json::Value) -> Document {
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
            "draw": { "kind": "formula", "notation": notation },
            "rows": rows,
            "description": ""
        })),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    }
}

/// Fresh SQLite repo + world + a GM and a player member.
async fn test_world() -> (SqliteRepository, Uuid, Uuid, Uuid) {
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
    (repo, w.id, gm, player)
}

fn player_ctx(player: Uuid) -> PermissionContext {
    PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    }
}

fn gm_ctx(gm: Uuid) -> PermissionContext {
    PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    }
}

async fn base_cx<'a>(
    repo: &'a SqliteRepository,
    ctx: &'a PermissionContext,
    world: Uuid,
) -> DrawCtx<'a> {
    let world_defaults = repo.world_cap_defaults(world).await.unwrap();
    DrawCtx {
        repo,
        ctx,
        world_defaults: Box::leak(Box::new(world_defaults)),
        policy: Box::leak(Box::new(crate::chat::ChatContentPolicy::default())),
        world_id: world,
        chain: Vec::new(),
        budget: 0,
    }
}

#[test]
fn weighted_row_selects_first_row_whose_cumulative_weight_meets_the_total() {
    let rows = vec![
        crate::data::engine::TableRow {
            weight: 3,
            range: None,
            label: "a".into(),
            results: vec![],
        },
        crate::data::engine::TableRow {
            weight: 2,
            range: None,
            label: "b".into(),
            results: vec![],
        },
    ];
    // Cumulative: [3, 5]. total=1..3 -> row 0; total=4..5 -> row 1.
    assert_eq!(weighted_row(&rows, 1), Some(0));
    assert_eq!(weighted_row(&rows, 3), Some(0));
    assert_eq!(weighted_row(&rows, 4), Some(1));
    assert_eq!(weighted_row(&rows, 5), Some(1));
    assert_eq!(weighted_row(&rows, 6), None);
}

#[test]
fn ranged_row_matches_the_containing_range_else_none() {
    let rows = vec![
        crate::data::engine::TableRow {
            weight: 1,
            range: Some(crate::data::engine::RowRange { lo: 2, hi: 6 }),
            label: "a".into(),
            results: vec![],
        },
        crate::data::engine::TableRow {
            weight: 1,
            range: Some(crate::data::engine::RowRange { lo: 7, hi: 12 }),
            label: "b".into(),
            results: vec![],
        },
    ];
    assert_eq!(ranged_row(&rows, 4), Some(0));
    assert_eq!(ranged_row(&rows, 12), Some(1));
    assert_eq!(ranged_row(&rows, 1), None);
}

#[tokio::test]
async fn a_self_referencing_table_row_is_refused_with_cycle() {
    let (repo, world, _gm, player) = test_world().await;
    let table_id = Uuid::new_v4();
    let doc = weighted_table_doc(
        table_id,
        world,
        serde_json::json!([{
            "weight": 1,
            "label": "loops",
            "results": [{ "kind": "draw", "table_id": table_id, "count": 1 }]
        }]),
        DocRole::Observer,
    );
    repo.apply_command(UnsequencedCommand {
        world_id: world,
        author: player,
        ts: 0,
        ops: vec![Operation::Create { doc }],
    })
    .await
    .unwrap();

    let ctx = player_ctx(player);
    let mut cx = base_cx(&repo, &ctx, world).await;
    let err = draw_table(&mut cx, table_id, 0).await.unwrap_err();
    assert!(matches!(err, DrawTableError::Cycle));
}

#[tokio::test]
async fn a_two_table_cycle_a_to_b_to_a_is_refused() {
    let (repo, world, _gm, player) = test_world().await;
    let a_id = Uuid::new_v4();
    let b_id = Uuid::new_v4();
    let doc_a = weighted_table_doc(
        a_id,
        world,
        serde_json::json!([{
            "weight": 1, "label": "to-b",
            "results": [{ "kind": "draw", "table_id": b_id, "count": 1 }]
        }]),
        DocRole::Observer,
    );
    let doc_b = weighted_table_doc(
        b_id,
        world,
        serde_json::json!([{
            "weight": 1, "label": "to-a",
            "results": [{ "kind": "draw", "table_id": a_id, "count": 1 }]
        }]),
        DocRole::Observer,
    );
    repo.apply_command(UnsequencedCommand {
        world_id: world,
        author: player,
        ts: 0,
        ops: vec![
            Operation::Create { doc: doc_a },
            Operation::Create { doc: doc_b },
        ],
    })
    .await
    .unwrap();

    let ctx = player_ctx(player);
    let mut cx = base_cx(&repo, &ctx, world).await;
    let err = draw_table(&mut cx, a_id, 0).await.unwrap_err();
    assert!(matches!(err, DrawTableError::Cycle));
}

#[tokio::test]
async fn depth_over_the_max_is_too_deep() {
    let (repo, world, _gm, player) = test_world().await;
    let ctx = player_ctx(player);
    let mut cx = base_cx(&repo, &ctx, world).await;
    let err = draw_table(&mut cx, Uuid::new_v4(), MAX_DRAW_DEPTH + 1)
        .await
        .unwrap_err();
    assert!(matches!(err, DrawTableError::TooDeep));
}

#[tokio::test]
async fn budget_exhausted_is_too_many() {
    let (repo, world, _gm, player) = test_world().await;
    let ctx = player_ctx(player);
    let mut cx = base_cx(&repo, &ctx, world).await;
    cx.budget = MAX_DRAWS_PER_REQUEST;
    let err = draw_table(&mut cx, Uuid::new_v4(), 0).await.unwrap_err();
    assert!(matches!(err, DrawTableError::TooMany));
}

#[tokio::test]
async fn a_nested_table_with_default_none_is_forbidden_for_a_player_but_ok_for_the_gm() {
    let (repo, world, gm, player) = test_world().await;
    let table_id = Uuid::new_v4();
    let doc = weighted_table_doc(
        table_id,
        world,
        serde_json::json!([{ "weight": 1, "label": "a", "results": [] }]),
        DocRole::None,
    );
    repo.apply_command(UnsequencedCommand {
        world_id: world,
        author: gm,
        ts: 0,
        ops: vec![Operation::Create { doc }],
    })
    .await
    .unwrap();

    let player_ctx = player_ctx(player);
    let mut player_cx = base_cx(&repo, &player_ctx, world).await;
    let err = draw_table(&mut player_cx, table_id, 0).await.unwrap_err();
    assert!(matches!(err, DrawTableError::Forbidden));

    let gm_ctx = gm_ctx(gm);
    let mut gm_cx = base_cx(&repo, &gm_ctx, world).await;
    let ok = draw_table(&mut gm_cx, table_id, 0).await;
    assert!(ok.is_ok());
}

#[tokio::test]
async fn a_draw_naming_a_non_table_doc_is_not_found() {
    let (repo, world, gm, _player) = test_world().await;
    let not_a_table = Uuid::new_v4();
    // A minimal actor doc, unrelated doc_type.
    let doc = Document {
        id: not_a_table,
        scope: Scope::World { world_id: world },
        doc_type: "actor".into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: PermissionSet::default(),
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::json!({
            "displayName": "x", "visual": {"kind": "image", "asset": "a"},
            "size": {"w": 1.0, "h": 1.0}, "shape": "square", "faction": null,
            "conditions": [], "prototype": true
        })),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    };
    repo.apply_command(UnsequencedCommand {
        world_id: world,
        author: gm,
        ts: 0,
        ops: vec![Operation::Create { doc }],
    })
    .await
    .unwrap();

    let ctx = gm_ctx(gm);
    let mut cx = base_cx(&repo, &ctx, world).await;
    let err = draw_table(&mut cx, not_a_table, 0).await.unwrap_err();
    assert!(matches!(err, DrawTableError::NotFound));
}

#[tokio::test]
async fn an_empty_table_is_refused() {
    let (repo, world, gm, _player) = test_world().await;
    let table_id = Uuid::new_v4();
    let doc = weighted_table_doc(table_id, world, serde_json::json!([]), DocRole::Observer);
    repo.apply_command(UnsequencedCommand {
        world_id: world,
        author: gm,
        ts: 0,
        ops: vec![Operation::Create { doc }],
    })
    .await
    .unwrap();

    let ctx = gm_ctx(gm);
    let mut cx = base_cx(&repo, &ctx, world).await;
    let err = draw_table(&mut cx, table_id, 0).await.unwrap_err();
    assert!(matches!(err, DrawTableError::EmptyTable));
}

#[tokio::test]
async fn a_missing_asset_row_entry_is_refused() {
    let (repo, world, gm, _player) = test_world().await;
    let table_id = Uuid::new_v4();
    let doc = weighted_table_doc(
        table_id,
        world,
        serde_json::json!([{
            "weight": 1, "label": "img",
            "results": [{ "kind": "image", "asset_id": Uuid::new_v4(), "alt": "" }]
        }]),
        DocRole::Observer,
    );
    repo.apply_command(UnsequencedCommand {
        world_id: world,
        author: gm,
        ts: 0,
        ops: vec![Operation::Create { doc }],
    })
    .await
    .unwrap();

    let ctx = gm_ctx(gm);
    let mut cx = base_cx(&repo, &ctx, world).await;
    let err = draw_table(&mut cx, table_id, 0).await.unwrap_err();
    assert!(matches!(err, DrawTableError::MissingAsset));
}

#[tokio::test]
async fn a_formula_table_hit_and_miss() {
    let (repo, world, gm, _player) = test_world().await;
    let hit_id = Uuid::new_v4();
    let doc = formula_table_doc(
        hit_id,
        world,
        "2d6",
        serde_json::json!([
            { "weight": 1, "range": { "lo": 2, "hi": 12 }, "label": "always", "results": [] }
        ]),
    );
    repo.apply_command(UnsequencedCommand {
        world_id: world,
        author: gm,
        ts: 0,
        ops: vec![Operation::Create { doc }],
    })
    .await
    .unwrap();
    let ctx = gm_ctx(gm);
    let mut cx = base_cx(&repo, &ctx, world).await;
    let seg = draw_table(&mut cx, hit_id, 0).await.unwrap();
    assert!(seg.row.is_some());

    let miss_id = Uuid::new_v4();
    let doc2 = formula_table_doc(
        miss_id,
        world,
        "2d6",
        serde_json::json!([
            { "weight": 1, "range": { "lo": 100, "hi": 200 }, "label": "never", "results": [] }
        ]),
    );
    repo.apply_command(UnsequencedCommand {
        world_id: world,
        author: gm,
        ts: 0,
        ops: vec![Operation::Create { doc: doc2 }],
    })
    .await
    .unwrap();
    let mut cx2 = base_cx(&repo, &ctx, world).await;
    let seg2 = draw_table(&mut cx2, miss_id, 0).await.unwrap();
    assert!(
        seg2.row.is_none(),
        "a total outside every range draws no row"
    );
}

#[tokio::test]
async fn a_nested_draw_fans_out_count_times_in_order() {
    let (repo, world, gm, _player) = test_world().await;
    let child_id = Uuid::new_v4();
    let child = weighted_table_doc(
        child_id,
        world,
        serde_json::json!([{ "weight": 1, "label": "leaf", "results": [] }]),
        DocRole::Observer,
    );
    let parent_id = Uuid::new_v4();
    let parent = weighted_table_doc(
        parent_id,
        world,
        serde_json::json!([{
            "weight": 1, "label": "spawns",
            "results": [{ "kind": "draw", "table_id": child_id, "count": 2 }]
        }]),
        DocRole::Observer,
    );
    repo.apply_command(UnsequencedCommand {
        world_id: world,
        author: gm,
        ts: 0,
        ops: vec![
            Operation::Create { doc: child },
            Operation::Create { doc: parent },
        ],
    })
    .await
    .unwrap();

    let ctx = gm_ctx(gm);
    let mut cx = base_cx(&repo, &ctx, world).await;
    let seg = draw_table(&mut cx, parent_id, 0).await.unwrap();
    let row = seg.row.unwrap();
    assert_eq!(row.nested.len(), 2);
    for n in &row.nested {
        assert_eq!(n.table_id, child_id);
    }
}
