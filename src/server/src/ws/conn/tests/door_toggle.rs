//! These tests check `handle_toggle_door`'s reply shape.
//!
//! The reply is `None` on success (the broadcast `Event` echo is the confirmation — the chat-ops
//! asymmetric pattern), a correlated `DoorToggleError` to the originating connection only on
//! refusal, with the role-conditioned wording applied at THIS layer (`door_toggle_wire_message`
//! against the caller's role).

use super::*;

/// A scene (grid 100, unrestricted, environment-lit) with a player-owned token at (50,50) and
/// a door wall at x=150, y∈[100,200] carrying `door` — the same geometry
/// `ws::room::tests::door_toggle` uses, where reach and line of sight both hold for the token.
async fn door_room(
    door: &str,
) -> (
    std::sync::Arc<SqliteRepository>,
    Arc<crate::ws::room::Room>,
    PermissionContext,
    PermissionContext,
    Uuid,
    Uuid,
) {
    use crate::data::document::{DocRole, WorldRole};
    use crate::data::membership::PermissionContext;
    use crate::ws::room::RoomRegistry;

    let repo = std::sync::Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let author = repo
        .create_user("gm", None, crate::auth::role::ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", author, 0).await.unwrap();
    let gm = PermissionContext {
        user_id: author,
        world_role: WorldRole::Gm,
    };
    let p = repo
        .create_user("player", None, crate::auth::role::ServerRole::User, 0)
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

    let (scene_id, token_id, ws_id, wall_id) = (
        crate::test_ids::test_uuid(0xD201),
        crate::test_ids::test_uuid(0xD202),
        crate::test_ids::test_uuid(0xD203),
        crate::test_ids::test_uuid(0xD204),
    );

    let mut ws = wdoc(world.id, ws_id, "world-settings");
    ws.owner = Some(author);
    ws.system = json!({
        "scene": {
            "losRestriction": false, "fog": false,
            "lightingEnabled": false, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    });
    ws.engine = Some(ws_engine(ws.system.clone()));
    room.publish(
        repo.as_ref(),
        &gm,
        vec![crate::data::command::Operation::Create { doc: ws }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut scene = wdoc(world.id, scene_id, "scene");
    scene.owner = Some(author);
    scene.system = json!({ "grid": { "kind": "square", "size": 100 } });
    room.publish(
        repo.as_ref(),
        &gm,
        vec![crate::data::command::Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut token = wdoc(world.id, token_id, "token");
    token.parent_id = Some(scene_id);
    token.owner = Some(p);
    token.permissions.users.insert(p, DocRole::Owner);
    token.engine = Some(token_engine(50.0, 50.0));
    room.publish(
        repo.as_ref(),
        &gm,
        vec![crate::data::command::Operation::Create { doc: token }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut wall = wdoc(world.id, wall_id, "wall");
    wall.parent_id = Some(scene_id);
    wall.owner = Some(author);
    wall.permissions.default = DocRole::Observer;
    wall.engine = Some(json!({
        "seg": { "x1": 150.0, "y1": 100.0, "x2": 150.0, "y2": 200.0 },
        "blocksMove": true, "blocksSight": true, "blocksLight": true,
        "door": door,
    }));
    room.publish(
        repo.as_ref(),
        &gm,
        vec![crate::data::command::Operation::Create { doc: wall }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    (repo, room, gm, player, scene_id, wall_id)
}

/// The one-line request builder.
fn door_req(
    request_id: Uuid,
    scene_id: Uuid,
    wall_id: Uuid,
    to: crate::data::engine::DoorState,
) -> crate::ws::room::ToggleDoorInputs {
    crate::ws::room::ToggleDoorInputs {
        request_id,
        scene_id,
        wall_id,
        to,
        ts: 0,
    }
}

#[tokio::test]
async fn handle_toggle_door_returns_no_frame_on_success() {
    let (repo, room, _gm, player, scene_id, wall_id) = door_room("closed").await;
    let request_id = crate::test_ids::test_uuid(0xD210);
    let res = handle_toggle_door(
        &room,
        repo.as_ref(),
        &player,
        door_req(
            request_id,
            scene_id,
            wall_id,
            crate::data::engine::DoorState::Open,
        ),
    )
    .await;
    assert!(
        res.is_none(),
        "success must return None — the broadcast Event echo is the confirmation; got {res:?}"
    );
    let stored = repo.get_document(wall_id).await.unwrap().unwrap();
    assert_eq!(
        stored
            .engine
            .as_ref()
            .and_then(|e| e.get("door"))
            .and_then(|d| d.as_str()),
        Some("open")
    );
}

#[tokio::test]
async fn handle_toggle_door_refusal_is_a_correlated_uniform_door_toggle_error_for_a_player() {
    let (repo, room, _gm, player, scene_id, wall_id) = door_room("locked").await;
    let request_id = crate::test_ids::test_uuid(0xD211);
    let res = handle_toggle_door(
        &room,
        repo.as_ref(),
        &player,
        door_req(
            request_id,
            scene_id,
            wall_id,
            crate::data::engine::DoorState::Open,
        ),
    )
    .await;
    match res {
        Some(ServerMsg::DoorToggleError {
            request_id: rid,
            message,
        }) => {
            assert_eq!(rid, request_id, "the refusal correlates by request_id");
            assert_eq!(
                message, "door rejected",
                "a non-GM's refusal is the uniform wording — the variant would disclose why"
            );
        }
        other => panic!("expected a DoorToggleError, got {other:?}"),
    }
}

#[tokio::test]
async fn handle_toggle_door_refusal_may_carry_the_diagnostic_for_a_gm() {
    let (repo, room, gm, _player, scene_id, wall_id) = door_room("locked").await;
    let request_id = crate::test_ids::test_uuid(0xD212);
    // A GM proposing an unknown wall id gets the UnknownWall diagnostic, never the
    // uniform wording — the caller may already read every wall in the world, so the
    // distinction discloses nothing.
    let res = handle_toggle_door(
        &room,
        repo.as_ref(),
        &gm,
        door_req(
            request_id,
            scene_id,
            crate::test_ids::test_uuid(0xD213),
            crate::data::engine::DoorState::Open,
        ),
    )
    .await;
    match res {
        Some(ServerMsg::DoorToggleError {
            request_id: rid,
            message,
        }) => {
            assert_eq!(rid, request_id);
            assert_eq!(message, "no such wall");
        }
        other => panic!("expected a DoorToggleError, got {other:?}"),
    }
    // Sanity: the wall id the fixture DID create is untouched by the refusal above.
    let stored = repo.get_document(wall_id).await.unwrap().unwrap();
    assert_eq!(
        stored
            .engine
            .as_ref()
            .and_then(|e| e.get("door"))
            .and_then(|d| d.as_str()),
        Some("locked")
    );
}
