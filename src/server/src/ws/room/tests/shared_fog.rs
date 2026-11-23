//! These tests check that `Room::execute_move` and `Room::publish`'s token Create gate read the
//! explored-fog row `scene::explored::explored_fog_key` names for the scene's `FogPooling`: the
//! shared sentinel row under `Shared`, the mover's own row under `Individual`. The same key is
//! the one `ws::conn::enrich_vision_explored` accumulates into, so a reader keyed on the user
//! would read a row nothing writes under `Shared`.

use super::*;
use serde_json::json;

/// World-settings with `movementRestriction: revealed` and the given `fogPooling`, in the shape
/// `movement_scene` and `room_with_player_create_capability_and_revealed_corner` publish.
fn revealed_world_settings(pooling: &str) -> serde_json::Value {
    ws_engine(json!({
        "scene": {
            "losRestriction": true, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#000000", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "revealed",
            "partialCellLeniency": true,
            "fogPooling": pooling
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }))
}

/// Where the diagonal move stops with no explored memory admitting it: the last cell the
/// fixture's light (bright radius 1.5 cells around `(50, 50)`) makes visible.
const LIT_EDGE: (f64, f64) = (250.0, 250.0);

/// Cells `(0, 0)` through `(5, 5)`, serialized as a square-grid explored blob.
fn explored_blob() -> Vec<u8> {
    let mut seed = crate::scene::explored::ExploredSet::new();
    seed.mark_cells((0..6).flat_map(|i| (0..6).map(move |j| (i, j))));
    seed.to_bytes(crate::scene::GridKind::Square)
}

/// Runs the diagonal five-step move from `(50, 50)` to `(550, 550)` — beyond the fixture's lit
/// radius, inside `explored_blob`'s cells — under `pooling`, with the blob stored for `owner`,
/// and returns where the token stopped.
async fn move_stop_with_blob_owned_by(pooling: &str, owner_is_pool: bool) -> (f64, f64) {
    let h = movement_scene("revealed", /* with_light */ true).await;
    h.room
        .scene()
        .write()
        .await
        .set_world_settings_for_test(revealed_world_settings(pooling));
    let owner = if owner_is_pool {
        Uuid::nil()
    } else {
        h.player.user_id
    };
    h.repo
        .set_explored(h.world_id, h.scene_id, "", owner, &explored_blob())
        .await
        .unwrap();
    let path: Vec<(f64, f64)> = (0..=5)
        .map(|i| (50.0 + i as f64 * 100.0, 50.0 + i as f64 * 100.0))
        .collect();
    h.room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path,
                ts: now_millis(),
                request_id: crate::test_ids::test_uuid(0xF00D_5F01),
            },
        )
        .await
        .unwrap()
        .stop
}

#[tokio::test]
async fn execute_move_reads_the_shared_row_under_shared_pooling() {
    assert_eq!(
        move_stop_with_blob_owned_by("shared", true).await,
        (550.0, 550.0),
        "the pooled row's explored cells admit the move"
    );
}

#[tokio::test]
async fn execute_move_ignores_the_users_own_row_under_shared_pooling() {
    assert_eq!(
        move_stop_with_blob_owned_by("shared", false).await,
        LIT_EDGE,
        "a dormant per-user row grants nothing while the scene is pooled"
    );
}

#[tokio::test]
async fn execute_move_ignores_the_shared_row_under_individual_pooling() {
    assert_eq!(
        move_stop_with_blob_owned_by("individual", true).await,
        LIT_EDGE,
        "the pool's row grants nothing to a scene that is not pooled"
    );
}

/// Places a token at the explored-but-unlit cell `(550, 550)` as the fixture's player, under
/// `pooling`, with the explored blob stored for the pool or for the player, and returns whether
/// the Create was admitted.
async fn create_admitted_with_blob_owned_by(pooling: &str, owner_is_pool: bool) -> bool {
    let h = room_with_player_create_capability_and_revealed_corner().await;
    h.room
        .scene()
        .write()
        .await
        .set_world_settings_for_test(revealed_world_settings(pooling));
    let owner = if owner_is_pool {
        Uuid::nil()
    } else {
        h.player_ctx.user_id
    };
    h.repo
        .set_explored(h.world, h.scene, "", owner, &explored_blob())
        .await
        .unwrap();
    h.room
        .publish(
            &h.repo,
            &h.player_ctx,
            vec![Operation::Create {
                doc: token_doc_at(h.world, h.scene, 550.0, 550.0),
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .is_ok()
}

#[tokio::test]
async fn create_gate_reads_the_shared_row_under_shared_pooling() {
    assert!(create_admitted_with_blob_owned_by("shared", true).await);
}

#[tokio::test]
async fn create_gate_ignores_the_users_own_row_under_shared_pooling() {
    assert!(!create_admitted_with_blob_owned_by("shared", false).await);
}

#[tokio::test]
async fn create_gate_ignores_the_shared_row_under_individual_pooling() {
    assert!(!create_admitted_with_blob_owned_by("individual", true).await);
}
