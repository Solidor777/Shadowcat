//! Cone lights and the darkness-activation window as they reach the scene read-model: the
//! effective cone direction a carried emission resolves to, the carried-light timeline's
//! cone fields, and the window's agreement between the committed field and the timeline.

use super::*;

/// A cone emission body carried by an actor: 60 degrees wide, authored offset 10, following the
/// bearer's facing when `follow` is set.
fn cone_torch(follow: bool) -> serde_json::Value {
    json!({
        "color": "#ffcc66", "intensity": 1.0, "brightRadius": 2.0, "dimRadius": 4.0,
        "enabled": true, "angle": 60.0, "rotation": 10.0, "followTokenRotation": follow
    })
}

/// A scene (id 10) holding one linked token (id 11) at `(150, 150)` facing `rotation` degrees,
/// carrying `light` through actor 200.
fn bearer(rotation: f64, light: serde_json::Value) -> (SceneEcs, Uuid, Uuid) {
    let tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 150.0, "y": 150.0, "w": 100.0, "h": 100.0, "rotation": rotation,
                "actor_id": crate::test_ids::test_uuid(200).to_string() }),
    );
    let mut ecs = SceneEcs::from_documents(
        vec![
            entity_doc_top_eng(
                10,
                "scene",
                json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
            ),
            tok,
        ],
        0,
    );
    let mut actor = actor_body(json!([]));
    actor["light"] = light;
    ecs.set_actors(vec![entity_doc_top_eng(200, "actor", actor)]);
    (
        ecs,
        crate::test_ids::test_uuid(10),
        crate::test_ids::test_uuid(11),
    )
}

#[test]
fn token_light_emission_cone_follows_token_rotation_plus_offset() {
    let (ecs, scene, _) = bearer(90.0, cone_torch(true));
    let lights = ecs.scene_lights(scene);
    assert_eq!(lights.len(), 1);
    assert_eq!(lights[0].angle, Some(60.0));
    assert_eq!(
        lights[0].rotation, 100.0,
        "token facing 90 plus the authored offset 10"
    );
}

#[test]
fn a_cone_that_does_not_follow_keeps_its_authored_rotation() {
    let (ecs, scene, _) = bearer(90.0, cone_torch(false));
    assert_eq!(ecs.scene_lights(scene)[0].rotation, 10.0);
}

#[test]
fn follow_token_rotation_is_derived_per_read_and_never_stored() {
    let (mut ecs, scene, token) = bearer(90.0, cone_torch(true));
    ecs.apply_op(&Operation::Update {
        doc_id: token,
        changes: vec![fc("/engine/rotation", json!(180.0))],
    });
    assert_eq!(
        ecs.scene_lights(scene)[0].rotation,
        190.0,
        "a rotation write moves the cone on the next read"
    );
}

#[test]
fn follow_token_rotation_is_inert_on_a_standalone_light() {
    let light = entity_doc_eng(
        20,
        10,
        "light",
        json!({ "x": 50.0, "y": 50.0, "emission": cone_torch(true) }),
    );
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), light], 0);
    let lights = ecs.scene_lights(crate::test_ids::test_uuid(10));
    assert_eq!(
        lights[0].rotation, 10.0,
        "a placed light has no token to follow"
    );
}

#[test]
fn mover_light_sample_rotation_matches_the_committed_field_light() {
    // The in-flight sample and the at-rest field light must describe the same cone: the egress
    // clip composes the former into the field the latter belongs to.
    let (ecs, scene, token) = bearer(90.0, cone_torch(true));
    let inputs = ecs
        .mover_light_inputs(scene, token, 100.0, None)
        .expect("an enabled carried cone is sampled");
    let sample = inputs.sample_at(0.0, (150.0, 150.0));
    let committed = &ecs.scene_lights(scene)[0];
    assert_eq!(sample.angle, committed.angle);
    assert_eq!(sample.rotation, Some(committed.rotation));
    assert_eq!(sample.rotation, Some(100.0));
}

#[test]
fn a_mover_light_sample_points_along_the_rotation_the_move_commits() {
    // Mutation: ignoring `committed_rotation` in `mover_light_inputs` keeps the stored 90 degree
    // facing (cone rotation 100) and fails this; the cone must read the new bearing's 190.
    let (ecs, scene, token) = bearer(90.0, cone_torch(true));
    let sample = ecs
        .mover_light_inputs(scene, token, 100.0, Some(180.0))
        .expect("an enabled carried cone is sampled")
        .sample_at(0.0, (150.0, 150.0));
    assert_eq!(sample.rotation, Some(190.0));
}

#[test]
fn a_radial_mover_light_sample_carries_no_cone_fields() {
    let (ecs, scene, token) = bearer(
        90.0,
        json!({ "color": "#ffcc66", "intensity": 1.0, "brightRadius": 2.0, "dimRadius": 4.0,
                "enabled": true }),
    );
    let sample = ecs
        .mover_light_inputs(scene, token, 100.0, None)
        .expect("sampled")
        .sample_at(0.0, (150.0, 150.0));
    assert_eq!((sample.angle, sample.rotation), (None, None));
}

#[test]
fn mover_light_inputs_follow_the_darkness_window() {
    // Environment 0.5: a light active below 0.3 is off, so the move carries no timeline for it;
    // at environment 0.2 it is on. The committed field reads the same predicate.
    let mut light = cone_torch(false);
    light["activeBelowEnv"] = json!(0.3);
    let (mut ecs, scene, token) = bearer(0.0, light);
    ecs.set_world_settings_for_test(json!({
        "scene": { "environment": { "color": "#ffffff", "intensity": 0.5 } }
    }));
    assert!(
        ecs.mover_light_inputs(scene, token, 100.0, None).is_none(),
        "the window is closed at environment 0.5"
    );
    ecs.set_world_settings_for_test(json!({
        "scene": { "environment": { "color": "#ffffff", "intensity": 0.2 } }
    }));
    assert!(ecs.mover_light_inputs(scene, token, 100.0, None).is_some());
}
