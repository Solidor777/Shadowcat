//! A carrier token moving must invalidate every lighting and visibility cache an anchored region
//! feeds: the ambient-light map (`LightingInputsSnapshot::ambient`, `VisibilityInputsSnapshot`)
//! and the region occluder wall entries both resolve through `SceneEcs::anchored_region_parts`, so
//! the value-compared snapshots change with the carrier's position.

use super::*;
use std::collections::BTreeSet;

const SCENE: u128 = 10;
const VIEWER: u128 = 11;
const CARRIER: u128 = 12;
const REGION: u128 = 30;

/// Move the carrier token to `x` through the document write path, with every cache warm.
fn move_carrier(ecs: &mut SceneEcs, x: f64) {
    ecs.apply_op(&Operation::Update {
        doc_id: crate::test_ids::test_uuid(CARRIER),
        changes: vec![fc("/engine/x", json!(x))],
    });
}

/// A global-illumination scene (every cell lit at the flat maximum) with a player-owned viewer at
/// (50, 50), a carrier token at `(carrier_x, 50)` and one region anchored to the carrier with the
/// body fields in `extra` merged over a `[-100, -100, 100, 100]` offset box.
fn anchored_scene(carrier_x: f64, extra: serde_json::Value) -> (SceneEcs, Uuid, Uuid) {
    let user = crate::test_ids::test_uuid(7);
    let scene_id = crate::test_ids::test_uuid(SCENE);
    let mut scene = doc(SCENE, None, "scene");
    scene.engine = Some(json!({ "grid": { "kind": "square", "size": 100 }, "background": null }));
    let mut viewer = entity_doc_eng(
        VIEWER,
        SCENE,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    viewer.owner = Some(user);
    let carrier = entity_doc_eng(
        CARRIER,
        SCENE,
        "token",
        json!({ "x": carrier_x, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    let mut body = json!({
        "shape": { "kind": "rect", "points": [-100.0, -100.0, 100.0, 100.0] },
        "behavior": "terrain",
        "cost": 1.0,
        "enabled": true,
        "anchor": {
            "token": crate::test_ids::test_uuid(CARRIER).to_string(),
            "include_carrier": true,
            "follow_rotation": false,
        },
    });
    for (k, v) in extra.as_object().expect("object") {
        body[k] = v.clone();
    }
    let region = entity_doc_eng(REGION, SCENE, "region", body);
    let mut ecs = SceneEcs::from_documents(vec![scene, viewer, carrier, region], 0);
    ecs.set_world_settings_for_test(json!({
        "scene": { "lightingEnabled": true, "lightMode": "globalIllumination" }
    }));
    (ecs, user, scene_id)
}

/// The movement-gate mask through the cached path.
fn cached_mask(ecs: &SceneEcs, user: Uuid, scene: Uuid) -> BTreeSet<(i32, i32)> {
    ecs.visible_cells_cached(
        user,
        WorldRole::Player,
        &no_world_grants(),
        scene,
        false,
        0.0,
    )
}

/// The same mask through the uncached path, the cache's ground truth.
fn fresh_mask(ecs: &SceneEcs, user: Uuid, scene: Uuid) -> BTreeSet<(i32, i32)> {
    ecs.visible_cells(
        user,
        WorldRole::Player,
        &no_world_grants(),
        scene,
        false,
        0.0,
    )
}

#[test]
fn a_carrier_move_misses_the_lighting_inputs_cache_for_an_anchored_ambient_region() {
    let (mut ecs, _user, scene) = anchored_scene(
        450.0,
        json!({
            "behavior": "ambient_light",
            "ambient_override": { "mode": "override", "level": 0.0 },
        }),
    );
    let settings = ecs.resolve_scene(scene);
    let ambient_cells = |ecs: &SceneEcs| -> BTreeSet<(i32, i32)> {
        ecs.lighting_inputs(scene, "", &settings, 100.0)
            .ambient
            .keys()
            .copied()
            .collect()
    };
    let before = ambient_cells(&ecs);
    assert!(before.contains(&(4, 0)), "the region covers the carrier");
    let warm = ecs.lighting_inputs_recompute_count();
    ambient_cells(&ecs);
    assert_eq!(
        ecs.lighting_inputs_recompute_count(),
        warm,
        "an unchanged scene is served from the cache (the control)"
    );

    move_carrier(&mut ecs, 1450.0);
    let after = ambient_cells(&ecs);
    assert!(
        after.contains(&(14, 0)) && !after.contains(&(4, 0)),
        "the ambient map follows the carrier: {after:?}"
    );
    assert_eq!(
        ecs.lighting_inputs_recompute_count(),
        warm + 1,
        "the carrier's move missed the cache and rebuilt the inputs"
    );
}

#[test]
fn a_carrier_move_misses_the_visibility_cache_for_an_anchored_ambient_region() {
    // The darkening region sits over the viewer's neighbours first, then follows the carrier
    // away: the cached movement-gate mask must grow back the cells it had hidden.
    let (mut ecs, user, scene) = anchored_scene(
        250.0,
        json!({
            "behavior": "ambient_light",
            "ambient_override": { "mode": "override", "level": 0.0 },
        }),
    );
    let hidden = cached_mask(&ecs, user, scene);
    assert_eq!(hidden, fresh_mask(&ecs, user, scene), "the cold cache");
    assert!(
        !hidden.contains(&(2, 0)),
        "the dark region hides its cells: {hidden:?}"
    );
    let warm = ecs.visible_cells_recompute_count();
    assert_eq!(cached_mask(&ecs, user, scene), hidden);
    assert_eq!(
        ecs.visible_cells_recompute_count(),
        warm,
        "an unchanged scene is served from the cache (the control)"
    );

    move_carrier(&mut ecs, 1450.0);
    let after = cached_mask(&ecs, user, scene);
    assert_eq!(
        after,
        fresh_mask(&ecs, user, scene),
        "the cached mask equals a fresh derivation after the carrier moved"
    );
    assert!(
        after.contains(&(2, 0)),
        "the cells the region left are visible again: {after:?}"
    );
    assert_eq!(ecs.visible_cells_recompute_count(), warm + 1);
}

#[test]
fn a_carrier_move_misses_the_visibility_cache_for_an_anchored_occluder() {
    let (mut ecs, user, scene) = anchored_scene(
        250.0,
        json!({ "occludes": { "sight": true, "light": false, "sound": false } }),
    );
    let blocked = cached_mask(&ecs, user, scene);
    assert_eq!(blocked, fresh_mask(&ecs, user, scene), "the cold cache");
    assert!(
        !blocked.contains(&(4, 0)),
        "the occluder's edge stops the sight ray: {blocked:?}"
    );

    move_carrier(&mut ecs, 1450.0);
    let after = cached_mask(&ecs, user, scene);
    assert_eq!(
        after,
        fresh_mask(&ecs, user, scene),
        "the cached mask equals a fresh derivation after the carrier moved"
    );
    assert!(
        after.contains(&(4, 0)),
        "the cells behind the old position are visible again: {after:?}"
    );
}
