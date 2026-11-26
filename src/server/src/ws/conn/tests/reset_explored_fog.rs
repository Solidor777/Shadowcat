//! These tests exercise `handle_reset_explored_fog`'s authorization gate, its happy-path effect and
//! its user-scoped reset.
//!
//! The gate gives a uniform generic refusal for a non-GM and for a scene outside this room's world;
//! the happy path deletes rows and the broadcast is observable on a second subscriber. A
//! user-scoped reset is refused on a shared-pooling scene and works under individual pooling.

use super::*;

/// A world with a GM and a player, plus a scene created in it. Returns
/// `(repo, room, gm_ctx, player_ctx, scene_id)`.
async fn gm_room() -> (
    Arc<SqliteRepository>,
    Arc<crate::ws::room::Room>,
    PermissionContext,
    PermissionContext,
    Uuid,
) {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::membership::PermissionContext;
    use crate::ws::room::RoomRegistry;

    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", gm, 0).await.unwrap();
    let p = repo
        .create_user("player", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world.id, p, WorldRole::Player)
        .await
        .unwrap();
    let player_ctx = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };

    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;

    let scene_id = crate::test_ids::test_uuid(0xF001);
    let mut scene = wdoc(world.id, scene_id, "scene");
    scene.owner = Some(gm);
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    (repo, room, gm_ctx, player_ctx, scene_id)
}

/// Same shape as `gm_room`, but the scene is created with an explicit `vision.fogPooling`
/// override — the same per-scene leaf `GameSettingsPanel`'s editor writes. Returns
/// `(repo, room, gm_ctx, scene_id)`.
async fn gm_room_with_fog_pooling(
    pooling: &str,
) -> (
    Arc<SqliteRepository>,
    Arc<crate::ws::room::Room>,
    PermissionContext,
    Uuid,
) {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::membership::PermissionContext;
    use crate::ws::room::RoomRegistry;

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

    let reg = RoomRegistry::new();
    let room = reg
        .get_or_create(repo.as_ref(), world.id)
        .await
        .unwrap()
        .unwrap();
    let wdoc = crate::data::document::tests::world_scoped_doc;

    let scene_id = crate::test_ids::test_uuid(0xF003);
    let mut scene = wdoc(world.id, scene_id, "scene");
    scene.owner = Some(gm);
    scene.engine = Some(serde_json::json!({
        "grid": { "kind": "square", "size": 100.0 }, "background": null,
        "vision": { "fogPooling": pooling }
    }));
    room.publish(
        repo.as_ref(),
        &gm_ctx,
        vec![crate::data::command::Operation::Create { doc: scene }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    (repo, room, gm_ctx, scene_id)
}

#[tokio::test]
async fn reset_explored_fog_refused_for_a_non_gm() {
    let (repo, room, _gm_ctx, player_ctx, scene_id) = gm_room().await;

    let refused = handle_reset_explored_fog(
        &room,
        repo.as_ref(),
        &player_ctx,
        room.world_id,
        scene_id,
        None,
        None,
    )
    .await;

    assert!(
        matches!(
            refused,
            Some(ServerMsg::Error {
                code: WsErrorCode::Forbidden,
                ..
            })
        ),
        "a non-GM sender must be refused with the generic Forbidden error; got {refused:?}"
    );
}

#[tokio::test]
async fn reset_explored_fog_refused_for_a_scene_in_a_different_world() {
    use crate::data::document::WorldRole;
    use crate::data::membership::PermissionContext;
    use crate::ws::room::RoomRegistry;

    let (repo, room, gm_ctx, _player_ctx, _scene_id) = gm_room().await;

    // A second world, with its own scene, owned by the SAME user (now GM there too).
    let reg = RoomRegistry::new();
    let other_world = repo
        .create_world_owned("X", gm_ctx.user_id, 0)
        .await
        .unwrap();
    let other_room = reg
        .get_or_create(repo.as_ref(), other_world.id)
        .await
        .unwrap()
        .unwrap();
    let other_gm_ctx = PermissionContext {
        user_id: gm_ctx.user_id,
        world_role: WorldRole::Gm,
    };
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let foreign_scene_id = crate::test_ids::test_uuid(0xF002);
    let mut foreign_scene = wdoc(other_world.id, foreign_scene_id, "scene");
    foreign_scene.owner = Some(gm_ctx.user_id);
    other_room
        .publish(
            repo.as_ref(),
            &other_gm_ctx,
            vec![crate::data::command::Operation::Create { doc: foreign_scene }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    // `room` (the first world) refuses the SAME GM resetting the OTHER world's scene.
    let refused = handle_reset_explored_fog(
        &room,
        repo.as_ref(),
        &gm_ctx,
        room.world_id,
        foreign_scene_id,
        None,
        None,
    )
    .await;

    assert!(
        matches!(
            refused,
            Some(ServerMsg::Error {
                code: WsErrorCode::Forbidden,
                ..
            })
        ),
        "a scene outside this room's world must be refused with the SAME generic Forbidden \
         error as a non-GM sender — no existence-disclosing distinction; got {refused:?}"
    );
}

#[tokio::test]
async fn reset_explored_fog_gm_success_clears_rows_and_broadcasts() {
    let (repo, room, gm_ctx, _player_ctx, scene_id) = gm_room().await;

    repo.set_explored(room.world_id, scene_id, "", gm_ctx.user_id, &[1, 2, 3])
        .await
        .unwrap();
    assert_eq!(
        repo.get_explored(scene_id, "", gm_ctx.user_id)
            .await
            .unwrap(),
        Some(vec![1, 2, 3])
    );

    // Subscribe BEFORE issuing the reset so the broadcast is not missed.
    let (mut rx, _) = room.subscribe();

    let result = handle_reset_explored_fog(
        &room,
        repo.as_ref(),
        &gm_ctx,
        room.world_id,
        scene_id,
        None,
        None,
    )
    .await;
    assert!(
        result.is_none(),
        "success path must return None (no error frame); got {result:?}"
    );

    // The row is gone.
    assert_eq!(
        repo.get_explored(scene_id, "", gm_ctx.user_id)
            .await
            .unwrap(),
        None
    );

    // The broadcast ring carries an `ExploredFogReset` for this scene, scoped exactly as
    // requested (no user/level).
    let bcast = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match rx.recv().await {
                Ok(crate::ws::room::RoomEvent::Other(msg)) => {
                    if matches!(msg.as_ref(), ServerMsg::ExploredFogReset { .. }) {
                        return Some((*msg).clone());
                    }
                }
                Ok(crate::ws::room::RoomEvent::Event(_)) => {}
                Err(_) => return None,
            }
        }
    })
    .await
    .expect("broadcast observed within timeout");

    match bcast {
        Some(ServerMsg::ExploredFogReset { scene, user, level }) => {
            assert_eq!(scene, scene_id);
            assert_eq!(user, None);
            assert_eq!(level, None);
        }
        other => panic!("expected ExploredFogReset; got {other:?}"),
    }
}

/// A `user`-scoped reset on a `Shared`-pooling scene is refused OUTRIGHT with a distinct
/// `BadMessage` (never the generic `Forbidden`) — never silently remapped onto the pool
/// sentinel, and never reported as success while matching zero rows.
#[tokio::test]
async fn reset_explored_fog_user_scoped_reset_refused_on_a_shared_pooling_scene() {
    let (repo, room, gm_ctx, scene_id) = gm_room_with_fog_pooling("shared").await;

    // Seed the pool bucket, as if some player had already explored under Shared pooling.
    repo.set_explored(room.world_id, scene_id, "", Uuid::nil(), &[1, 2, 3])
        .await
        .unwrap();

    let some_player = crate::test_ids::test_uuid(0xBEEF);
    let refused = handle_reset_explored_fog(
        &room,
        repo.as_ref(),
        &gm_ctx,
        room.world_id,
        scene_id,
        Some(some_player),
        None,
    )
    .await;

    assert!(
        matches!(
            refused,
            Some(ServerMsg::Error {
                code: WsErrorCode::BadMessage,
                ..
            })
        ),
        "a user-scoped reset on a Shared scene must be refused with a distinct BadMessage, \
         never the generic Forbidden; got {refused:?}"
    );
    // Refusing means no write happened at all -- the pool bucket seeded above is untouched.
    assert_eq!(
        repo.get_explored(scene_id, "", Uuid::nil()).await.unwrap(),
        Some(vec![1, 2, 3]),
        "a refused reset must not silently remap onto (and wipe) the pool bucket"
    );
}

/// A `user`-scoped reset still works normally on an `Individual`-pooling (the default) scene —
/// the refusal above is specific to `Shared`, not a blanket ban on user-scoped resets.
#[tokio::test]
async fn reset_explored_fog_user_scoped_reset_still_works_under_individual_pooling() {
    let (repo, room, gm_ctx, scene_id) = gm_room_with_fog_pooling("individual").await;

    repo.set_explored(room.world_id, scene_id, "", gm_ctx.user_id, &[4, 5, 6])
        .await
        .unwrap();

    let result = handle_reset_explored_fog(
        &room,
        repo.as_ref(),
        &gm_ctx,
        room.world_id,
        scene_id,
        Some(gm_ctx.user_id),
        None,
    )
    .await;

    assert!(
        result.is_none(),
        "a user-scoped reset under Individual pooling must succeed; got {result:?}"
    );
    assert_eq!(
        repo.get_explored(scene_id, "", gm_ctx.user_id)
            .await
            .unwrap(),
        None
    );
}
