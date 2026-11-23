//! These tests check that `handle_pathfind` reads the explored-fog row
//! `scene::explored::explored_fog_key` names for the scene's `FogPooling`: the shared sentinel
//! row under `Shared`, the requester's own row under `Individual`.

use super::*;
use crate::auth::role::ServerRole;
use crate::data::document::{DocRole, WorldRole};
use crate::data::membership::PermissionContext;
use crate::ws::room::RoomRegistry;
use serde_json::json;

/// Whether a player's `Pathfind` to `(250, 50)` in a dark `revealed` scene with the given
/// `fogPooling` returns a route, when the explored cells `(0, 0)` through `(2, 0)` are stored
/// for the pool (`owner_is_pool`) or for the player. The dark scene has an empty visible mask,
/// so only the explored row can admit the route.
async fn pathfind_routes_with_blob_owned_by(pooling: &str, owner_is_pool: bool) -> bool {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let p = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world.id, p, WorldRole::Player)
        .await
        .unwrap();
    let player = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (scene_id, token_id, ws_id) = (
        crate::test_ids::test_uuid(0xA5F1),
        crate::test_ids::test_uuid(0xA5F2),
        crate::test_ids::test_uuid(0xA5F3),
    );

    let mut ws = wdoc(world.id, ws_id, "world-settings");
    ws.owner = Some(gm);
    ws.system = json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#000000", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "revealed",
            "partialCellLeniency": false,
            "fogPooling": pooling
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    });
    ws.engine = Some(ws.system.clone());
    let mut scene = wdoc(world.id, scene_id, "scene");
    scene.owner = Some(gm);
    scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
    let mut token = wdoc(world.id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(p);
    token.permissions.users.insert(p, DocRole::Owner);
    token.engine = Some(token_engine(50.0, 50.0));
    for doc in [ws, scene, token] {
        room.publish(
            repo.as_ref(),
            &gm_ctx,
            vec![crate::data::command::Operation::Create { doc }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    }

    let mut seed = crate::scene::explored::ExploredSet::new();
    seed.mark_cells((0..3).map(|i| (i, 0)));
    let owner = if owner_is_pool { Uuid::nil() } else { p };
    repo.set_explored(
        world.id,
        scene_id,
        "",
        owner,
        &seed.to_bytes(crate::scene::GridKind::Square),
    )
    .await
    .unwrap();

    let reply = handle_pathfind(
        PathfindRequest {
            request_id: crate::test_ids::test_uuid(0xF5F1),
            scene: scene_id,
            start: (50.0, 50.0),
            waypoints: vec![(250.0, 50.0)],
            footprint_radius: 0.1,
            token: None,
        },
        &player,
        &room,
        repo.as_ref(),
    )
    .await;
    matches!(reply, ServerMsg::PathResult { .. })
}

#[tokio::test]
async fn pathfind_reads_the_shared_row_under_shared_pooling() {
    assert!(pathfind_routes_with_blob_owned_by("shared", true).await);
}

#[tokio::test]
async fn pathfind_ignores_the_users_own_row_under_shared_pooling() {
    assert!(!pathfind_routes_with_blob_owned_by("shared", false).await);
}

#[tokio::test]
async fn pathfind_ignores_the_shared_row_under_individual_pooling() {
    assert!(!pathfind_routes_with_blob_owned_by("individual", true).await);
}

#[tokio::test]
async fn pathfind_reads_the_users_own_row_under_individual_pooling() {
    assert!(pathfind_routes_with_blob_owned_by("individual", false).await);
}
