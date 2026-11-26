//! `SceneEcs::token_rotation_write`, `SceneEcs::token_turn_standing` and the `facing_turn_rule`
//! setting's resolution through the settings chain.

use super::*;
use crate::data::engine::FacingTurnRule;
use crate::scene::rotation_gate::TurnStanding;

/// The scene every fixture token parents to.
fn scene_id() -> Uuid {
    crate::test_ids::test_uuid(10)
}

/// The fixture token.
fn token_id() -> Uuid {
    crate::test_ids::test_uuid(11)
}

/// A scene holding one raw token at `rotation`, locked when `lock_rotation`.
fn ecs_with_token(rotation: f64, lock_rotation: bool) -> SceneEcs {
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
    );
    let token = entity_doc_eng(
        11,
        10,
        "token",
        json!({
            "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0,
            "rotation": rotation, "lock_rotation": lock_rotation
        }),
    );
    SceneEcs::from_documents(vec![scene, token], 0)
}

#[test]
fn token_rotation_write_reports_a_change_against_the_committed_rotation() {
    let ecs = ecs_with_token(10.0, true);
    let same = ecs
        .token_rotation_write(token_id(), &[fc("/engine/rotation", json!(10.0))])
        .unwrap();
    assert_eq!(
        (same.scene, same.locked, same.rotation_changed),
        (scene_id(), true, false)
    );

    let changed = ecs
        .token_rotation_write(token_id(), &[fc("/engine/rotation", json!(11.0))])
        .unwrap();
    assert!(changed.rotation_changed);

    let other_field = ecs
        .token_rotation_write(token_id(), &[fc("/engine/elevation", json!(2.0))])
        .unwrap();
    assert!(!other_field.rotation_changed);
}

#[test]
fn token_rotation_write_judges_the_last_write_of_a_duplicated_path() {
    let ecs = ecs_with_token(10.0, false);
    let restored = [
        fc("/engine/rotation", json!(99.0)),
        fc("/engine/rotation", json!(10.0)),
    ];
    assert!(
        !ecs.token_rotation_write(token_id(), &restored)
            .unwrap()
            .rotation_changed
    );
    let moved = [
        fc("/engine/rotation", json!(10.0)),
        fc("/engine/rotation", json!(99.0)),
    ];
    assert!(
        ecs.token_rotation_write(token_id(), &moved)
            .unwrap()
            .rotation_changed
    );
}

#[test]
fn token_rotation_write_judges_a_wholesale_engine_write_by_its_post_image() {
    let ecs = ecs_with_token(10.0, false);
    let engine = json!({
        "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0,
        "rotation": 200.0, "lock_rotation": false
    });
    assert!(
        ecs.token_rotation_write(token_id(), &[fc("/engine", engine)])
            .unwrap()
            .rotation_changed
    );
}

#[test]
fn a_post_image_without_a_numeric_rotation_counts_as_a_change() {
    let ecs = ecs_with_token(10.0, false);
    let w = ecs
        .token_rotation_write(token_id(), &[fc("/engine/rotation", json!("north"))])
        .unwrap();
    assert!(w.rotation_changed);
}

#[test]
fn token_rotation_write_is_none_for_a_non_token() {
    let ecs = ecs_with_token(0.0, false);
    assert!(ecs.token_rotation_write(scene_id(), &[]).is_none());
}

#[test]
fn an_unconstrained_standing_when_no_combat_applies() {
    let ecs = ecs_with_token(0.0, false);
    let ctx = PermissionContext {
        user_id: crate::test_ids::test_uuid(7),
        world_role: WorldRole::Player,
    };
    assert_eq!(
        ecs.token_turn_standing(scene_id(), token_id(), &ctx, &no_world_grants()),
        TurnStanding::Unconstrained
    );
}

/// `facing_turn_rule` folds through the same per-leaf chain as `facing_vision`: engine literal
/// (`Free`) < system-defaults < world-settings < per-scene override, and a scene `null` inherits.
#[test]
fn facing_turn_rule_resolves_through_system_world_and_scene_layers() {
    let mut ecs = SceneEcs::new();
    let scene_id = crate::test_ids::test_uuid(7);
    let base = json!({ "grid": { "kind": "square", "size": 100 }, "background": null });
    ecs.insert_scene_for_test(scene_id, base.clone());
    assert_eq!(
        ecs.resolve_scene(scene_id).facing_turn_rule,
        FacingTurnRule::Free,
        "the engine literal is free"
    );

    ecs.set_system_defaults_for_test(json!({ "scene": { "facingTurnRule": "own_turn" } }));
    assert_eq!(
        ecs.resolve_scene(scene_id).facing_turn_rule,
        FacingTurnRule::OwnTurn
    );

    ecs.set_world_settings_for_test(ws_body(&[("/scene/facingTurnRule", json!("free"))]));
    assert_eq!(
        ecs.resolve_scene(scene_id).facing_turn_rule,
        FacingTurnRule::Free,
        "world-settings beats system-defaults"
    );

    ecs.insert_scene_for_test(
        scene_id,
        json!({
            "grid": { "kind": "square", "size": 100 }, "background": null,
            "vision": { "facingTurnRule": "own_turn" }
        }),
    );
    assert_eq!(
        ecs.resolve_scene(scene_id).facing_turn_rule,
        FacingTurnRule::OwnTurn,
        "a per-scene override beats world-settings"
    );

    ecs.insert_scene_for_test(
        scene_id,
        json!({
            "grid": { "kind": "square", "size": 100 }, "background": null,
            "vision": { "facingTurnRule": null }
        }),
    );
    assert_eq!(
        ecs.resolve_scene(scene_id).facing_turn_rule,
        FacingTurnRule::Free,
        "a null scene override inherits the world value"
    );
}
