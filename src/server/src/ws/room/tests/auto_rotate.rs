//! `Room::execute_move`'s auto-rotate write: the bearing `FieldChange` folded into the position
//! `Update`, its world-setting and per-token gates, and the GM-moves-rotate rule.

use super::*;
use crate::data::command::FieldChange;
use serde_json::json;

/// The `world-settings` document id `movement_scene_with_wall` publishes.
fn world_settings_id() -> Uuid {
    crate::test_ids::test_uuid(0xFA11_0003)
}

/// Commits one `Update` of `path` (`old` -> `new`) on `doc_id` as the GM.
async fn gm_update(
    h: &MovementHandle,
    doc_id: Uuid,
    path: &str,
    old: serde_json::Value,
    new: serde_json::Value,
) {
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: path.into(),
                    old,
                    new,
                }],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
}

/// Turns the world-level `auto_rotate_tokens` default on.
async fn enable_auto_rotate(h: &MovementHandle) {
    gm_update(
        h,
        world_settings_id(),
        "/engine/scene/autoRotateTokens",
        serde_json::Value::Null,
        json!(true),
    )
    .await;
}

/// The token's committed `(rotation, lock_rotation)` pair.
async fn rotation_state(h: &MovementHandle) -> (f64, bool) {
    h.room
        .scene()
        .read()
        .await
        .token_rotation_state(h.token_id)
        .expect("token in ECS")
}

/// Moves `who`'s token straight down one cell, `(50,50)` -> `(50,150)`: bearing 90 degrees.
async fn move_down(
    h: &MovementHandle,
    who: &PermissionContext,
) -> Result<crate::ws::room::MoveExecution, DataError> {
    h.room
        .execute_move(
            &h.repo,
            who,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, (50.0, 150.0)],
                ts: now_millis(),
                request_id: Uuid::nil(),
            },
        )
        .await
}

#[tokio::test]
async fn execute_move_rotates_token_to_face_movement_when_enabled() {
    let h = movement_scene_with_wall().await;
    enable_auto_rotate(&h).await;
    assert_eq!(rotation_state(&h).await, (0.0, false));
    move_down(&h, &h.player).await.unwrap();
    assert_eq!(h.committed_pos(h.token_id).await, (50.0, 150.0));
    assert_eq!(
        rotation_state(&h).await.0,
        90.0,
        "+y displacement is a quarter turn clockwise"
    );
}

#[tokio::test]
async fn execute_move_does_not_rotate_when_world_default_off() {
    let h = movement_scene_with_wall().await;
    move_down(&h, &h.player).await.unwrap();
    assert_eq!(h.committed_pos(h.token_id).await, (50.0, 150.0));
    assert_eq!(rotation_state(&h).await.0, 0.0);
}

#[tokio::test]
async fn execute_move_respects_per_token_lock_rotation() {
    let h = movement_scene_with_wall().await;
    enable_auto_rotate(&h).await;
    gm_update(
        &h,
        h.token_id,
        "/engine/lock_rotation",
        json!(false),
        json!(true),
    )
    .await;
    assert!(rotation_state(&h).await.1);
    move_down(&h, &h.player).await.unwrap();
    assert_eq!(h.committed_pos(h.token_id).await, (50.0, 150.0));
    assert_eq!(
        rotation_state(&h).await.0,
        0.0,
        "a locked token keeps its rotation"
    );
}

#[tokio::test]
async fn execute_move_rotates_a_gm_moved_token_when_enabled() {
    let h = movement_scene_with_wall().await;
    enable_auto_rotate(&h).await;
    move_down(&h, &h.gm).await.unwrap();
    assert_eq!(rotation_state(&h).await.0, 90.0);
}

#[tokio::test]
async fn execute_move_commits_position_and_rotation_in_one_update() {
    let h = movement_scene_with_wall().await;
    enable_auto_rotate(&h).await;
    let (mut rx, _) = h.room.subscribe();
    move_down(&h, &h.player).await.unwrap();
    let ev = loop {
        match rx.recv().await.unwrap() {
            crate::ws::room::RoomEvent::Event(ev) => break (*ev).clone(),
            crate::ws::room::RoomEvent::Other(_) => continue,
        }
    };
    assert_eq!(ev.command.ops.len(), 1, "one Update, not a second command");
    let Operation::Update { doc_id, changes } = &ev.command.ops[0] else {
        panic!("expected an Update");
    };
    assert_eq!(*doc_id, h.token_id);
    let paths: Vec<&str> = changes.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(paths, ["/engine/x", "/engine/y", "/engine/rotation"]);
    assert_eq!(changes[2].old, json!(0.0));
    assert_eq!(changes[2].new, json!(90.0));
}

#[tokio::test]
async fn execute_move_zero_progress_move_commits_no_update() {
    let h = movement_scene_with_wall().await;
    enable_auto_rotate(&h).await;
    // Park the token on the corner; the wall then blocks the very first step down, so the
    // move makes no progress and returns before any `Update` is built.
    gm_update(&h, h.token_id, "/engine/x", json!(50.0), json!(150.0)).await;
    let (mut rx, _) = h.room.subscribe();
    let res = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![(150.0, 50.0), h.adj2],
                ts: now_millis(),
                request_id: Uuid::nil(),
            },
        )
        .await
        .unwrap();
    assert_eq!(res.stop, (150.0, 50.0), "blocked at the first step");
    assert!(
        rx.try_recv().is_err(),
        "a zero-progress move commits nothing, so no rotation write either"
    );
    assert_eq!(rotation_state(&h).await.0, 0.0);
}
