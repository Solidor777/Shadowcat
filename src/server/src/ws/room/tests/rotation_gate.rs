//! `Room::publish`'s refusal of a non-GM token rotation write: the scene's `FacingTurnRule` and the
//! per-token `lock_rotation`, both decided by `scene::rotation_gate::rotation_refusal`.

use super::movement_budget::{budget_scene, BudgetHandle};
use super::*;
use crate::data::command::FieldChange;
use serde_json::json;

/// The `world-settings` document id `movement_scene` publishes.
fn world_settings_id() -> Uuid {
    crate::test_ids::test_uuid(0x5CE2)
}

/// Commits one `Update` of `path` (`old` -> `new`) on `doc_id` as `who`.
async fn update_as(
    h: &BudgetHandle,
    who: &PermissionContext,
    doc_id: Uuid,
    path: &str,
    old: serde_json::Value,
    new: serde_json::Value,
) -> Result<(), DataError> {
    h.room
        .publish(
            &h.repo,
            who,
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
        .map(|_| ())
}

/// Sets the world-level facing turn rule (`"free"` or `"own_turn"`) as the GM.
async fn set_turn_rule(h: &BudgetHandle, rule: &str) {
    update_as(
        h,
        &h.gm,
        world_settings_id(),
        "/engine/scene/facingTurnRule",
        serde_json::Value::Null,
        json!(rule),
    )
    .await
    .unwrap();
}

/// A combat scene whose player token holds the opening turn, under `rule`.
async fn scene_with_rule(rule: &str) -> BudgetHandle {
    let h = budget_scene("hard", "spaces", None).await;
    set_turn_rule(&h, rule).await;
    h
}

/// The player turns their own token from rotation 0 to `to`.
async fn player_rotates(h: &BudgetHandle, token: Uuid, to: f64) -> Result<(), DataError> {
    update_as(
        h,
        &h.player,
        token,
        "/engine/rotation",
        json!(0.0),
        json!(to),
    )
    .await
}

/// The token's committed rotation.
async fn rotation_of(h: &BudgetHandle, token: Uuid) -> f64 {
    h.room
        .scene()
        .read()
        .await
        .token_rotation_state(token)
        .expect("token in ECS")
        .0
}

#[tokio::test]
async fn free_rule_lets_a_player_turn_their_token_off_turn() {
    let h = scene_with_rule("free").await;
    h.set_turn(h.other_combatant).await;
    player_rotates(&h, h.token_id, 90.0).await.unwrap();
    assert_eq!(rotation_of(&h, h.token_id).await, 90.0);
}

#[tokio::test]
async fn own_turn_rule_lets_the_token_on_turn_rotate_and_refuses_it_off_turn() {
    let h = scene_with_rule("own_turn").await;
    player_rotates(&h, h.token_id, 45.0).await.unwrap();
    assert_eq!(rotation_of(&h, h.token_id).await, 45.0);

    h.set_turn(h.other_combatant).await;
    let refused = update_as(
        &h,
        &h.player,
        h.token_id,
        "/engine/rotation",
        json!(45.0),
        json!(90.0),
    )
    .await;
    assert!(matches!(refused, Err(DataError::Forbidden)), "{refused:?}");
    assert_eq!(rotation_of(&h, h.token_id).await, 45.0, "nothing committed");
}

#[tokio::test]
async fn own_turn_rule_never_blocks_a_gm() {
    let h = scene_with_rule("own_turn").await;
    h.set_turn(h.other_combatant).await;
    update_as(
        &h,
        &h.gm,
        h.token_id,
        "/engine/rotation",
        json!(0.0),
        json!(120.0),
    )
    .await
    .unwrap();
    assert_eq!(rotation_of(&h, h.token_id).await, 120.0);
}

#[tokio::test]
async fn own_turn_rule_ignores_a_token_that_names_no_combatant() {
    let h = scene_with_rule("own_turn").await;
    h.set_turn(h.other_combatant).await;
    player_rotates(&h, h.free_token, 30.0).await.unwrap();
    assert_eq!(rotation_of(&h, h.free_token).await, 30.0);
}

#[tokio::test]
async fn own_turn_rule_does_not_disclose_a_combatant_the_player_cannot_read() {
    let h = scene_with_rule("own_turn").await;
    h.set_turn(h.other_combatant).await;
    h.hide_player_combatant().await;
    player_rotates(&h, h.token_id, 60.0).await.unwrap();
    assert_eq!(rotation_of(&h, h.token_id).await, 60.0);
}

#[tokio::test]
async fn own_turn_rule_applies_only_to_a_rotation_change() {
    let h = scene_with_rule("own_turn").await;
    h.set_turn(h.other_combatant).await;
    // Re-stating the stored rotation is not a turn.
    player_rotates(&h, h.token_id, 0.0).await.unwrap();
    // Another engine field is untouched by the rule.
    update_as(
        &h,
        &h.player,
        h.token_id,
        "/engine/elevation",
        serde_json::Value::Null,
        json!(5.0),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn own_turn_rule_judges_a_wholesale_engine_write_by_its_post_image() {
    let h = scene_with_rule("own_turn").await;
    h.set_turn(h.other_combatant).await;
    let doc = h.repo.get_document(h.token_id).await.unwrap().unwrap();
    let old = doc.engine.clone().unwrap();
    let mut new = old.clone();
    new["rotation"] = json!(75.0);
    let refused = update_as(&h, &h.player, h.token_id, "/engine", old, new).await;
    assert!(matches!(refused, Err(DataError::Forbidden)), "{refused:?}");
}

#[tokio::test]
async fn a_locked_token_refuses_a_players_rotation_but_not_a_gms() {
    let h = scene_with_rule("free").await;
    update_as(
        &h,
        &h.gm,
        h.token_id,
        "/engine/lock_rotation",
        json!(false),
        json!(true),
    )
    .await
    .unwrap();
    let refused = player_rotates(&h, h.token_id, 90.0).await;
    assert!(matches!(refused, Err(DataError::Forbidden)), "{refused:?}");
    assert_eq!(rotation_of(&h, h.token_id).await, 0.0);

    // The same predicate holds under the turn rule, even on the token's own turn.
    set_turn_rule_override(&h, "own_turn").await;
    let refused = player_rotates(&h, h.token_id, 90.0).await;
    assert!(matches!(refused, Err(DataError::Forbidden)), "{refused:?}");

    update_as(
        &h,
        &h.gm,
        h.token_id,
        "/engine/rotation",
        json!(0.0),
        json!(90.0),
    )
    .await
    .unwrap();
    assert_eq!(rotation_of(&h, h.token_id).await, 90.0);
}

/// Switches the already-set world rule to `rule`.
async fn set_turn_rule_override(h: &BudgetHandle, rule: &str) {
    update_as(
        h,
        &h.gm,
        world_settings_id(),
        "/engine/scene/facingTurnRule",
        json!("free"),
        json!(rule),
    )
    .await
    .unwrap();
}
