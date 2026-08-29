use super::*;
use serde_json::json;

#[test]
fn system_layer_beats_engine_fallback_when_world_is_absent() {
    let mut ecs = SceneEcs::new();
    ecs.set_system_defaults_for_test(
        json!({ "scene": { "fog": false, "movementRestriction": "unrestricted" } }),
    );
    let scene_id = Uuid::from_u128(7);
    ecs.insert_scene_for_test(
        scene_id,
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
    );
    let r = ecs.resolve_scene(scene_id);
    assert!(!r.fog);
    assert_eq!(r.movement_restriction, MovementRestriction::Unrestricted);
    assert!(
        r.los_restriction,
        "untouched field keeps the engine fallback"
    );
}

#[test]
fn world_layer_beats_system_layer_and_scene_beats_world() {
    let mut ecs = SceneEcs::new();
    ecs.set_system_defaults_for_test(json!({ "scene": { "fog": false } }));
    ecs.set_world_settings_for_test(ws_body(&[("/scene/fog", json!(true))]));
    let scene_id = Uuid::from_u128(7);
    ecs.insert_scene_for_test(
        scene_id,
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
    );
    assert!(ecs.resolve_scene(scene_id).fog);
    ecs.insert_scene_for_test(
        scene_id,
        json!({
            "grid": { "kind": "square", "size": 100 }, "background": null, "vision": { "fog": false }
        }),
    );
    assert!(!ecs.resolve_scene(scene_id).fog);
}

#[test]
fn diagonal_rule_and_animation_speed_read_the_system_layer() {
    let mut ecs = SceneEcs::new();
    ecs.set_system_defaults_for_test(json!({
        "pathfinding": { "diagonalRule": "manhattan" },
        "animation": { "speedCellsPerSec": 3.0 }
    }));
    assert_eq!(
        ecs.resolved_diagonal_rule(),
        pathfinding::DiagonalRule::Manhattan
    );
    assert!((ecs.resolved_animation_speed() - 3.0).abs() < 1e-9);
    ecs.set_world_settings_for_test(ws_body(&[(
        "/pathfinding/diagonalRule",
        json!("euclidean"),
    )]));
    assert_eq!(
        ecs.resolved_diagonal_rule(),
        pathfinding::DiagonalRule::Euclidean
    );
}

#[test]
fn apply_op_hydrates_and_retires_the_system_defaults_doc() {
    let mut ecs = SceneEcs::new();
    let sd = entity_doc_top_eng(
        0xABC,
        "system-defaults",
        json!({ "scene": { "fog": false } }),
    );
    ecs.apply_op(&Operation::Create { doc: sd.clone() });
    assert!(ecs.system_defaults_doc().is_some());
    ecs.apply_op(&Operation::Update {
        doc_id: sd.id,
        changes: vec![fc("/engine/scene/fog", json!(true))],
    });
    let scene_id = Uuid::from_u128(7);
    ecs.insert_scene_for_test(
        scene_id,
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
    );
    assert!(ecs.resolve_scene(scene_id).fog);
    ecs.apply_op(&Operation::Delete { doc: sd });
    assert!(ecs.system_defaults_doc().is_none());
}
