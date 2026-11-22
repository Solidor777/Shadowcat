//! Region sense occluders end to end: a region flagged `occludes` changes the player's visible
//! mask (sight, light) the way a wall ring would, and nothing changes without the flag.

use super::*;

/// The lit-player fixture (scene 10 on a 100-unit square grid, player token and a white light
/// at (50, 50)) plus an enabled region over the cells `[x0, x1] × [y0, y1]` carrying `occludes`.
/// `viewer_x` moves the token (and the light with it) when a test needs the viewer inside.
fn fixture_with_region(
    occludes: Option<serde_json::Value>,
    rect: (f64, f64, f64, f64),
    viewer_x: f64,
) -> (SceneEcs, Uuid, Uuid) {
    let user = crate::test_ids::test_uuid(7);
    let scene_id = crate::test_ids::test_uuid(10);
    let mut scene = doc(10, None, "scene");
    scene.engine = Some(json!({ "grid": { "kind": "square", "size": 100 }, "background": null }));
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": viewer_x, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let light = entity_doc_eng(
        20,
        10,
        "light",
        json!({
            "x": viewer_x, "y": 50.0,
            "emission": { "color": "#ffffff", "intensity": 1.0, "brightRadius": 8.0, "dimRadius": 10.0, "enabled": true }
        }),
    );
    let (x0, y0, x1, y1) = rect;
    let mut body = json!({
        "shape": { "kind": "rect", "points": [x0, y0, x1, y1] },
        "behavior": "terrain", "cost": 1.0, "enabled": true,
    });
    if let Some(o) = occludes {
        body["occludes"] = o;
    }
    let region = entity_doc_eng(30, 10, "region", body);
    let ecs = SceneEcs::from_documents(vec![scene, tok, light, region], 0);
    (ecs, user, scene_id)
}

/// A region over cells 2 and 3 of row 0 (and the rows above and below).
const BANK: (f64, f64, f64, f64) = (200.0, -100.0, 400.0, 200.0);

#[test]
fn a_sight_occluding_region_hides_the_cells_behind_it() {
    let seen = |occludes| {
        let (ecs, user, scene) = fixture_with_region(occludes, BANK, 50.0);
        mask_cells(&ecs, user, scene)
    };
    assert!(
        seen(None).contains(&(5, 0)),
        "without the flag the far cell is visible (the control)"
    );
    let blocked = seen(Some(
        json!({ "sight": true, "light": false, "sound": false }),
    ));
    assert!(
        blocked.contains(&(1, 0)) && !blocked.contains(&(5, 0)),
        "the region's edge stops the sight ray: {blocked:?}"
    );
}

#[test]
fn a_light_occluding_region_keeps_the_light_from_the_cells_behind_it() {
    let lit = |occludes| {
        let (ecs, user, scene) = fixture_with_region(occludes, BANK, 50.0);
        mask_cells(&ecs, user, scene)
    };
    assert!(lit(None).contains(&(5, 0)));
    let sealed = lit(Some(
        json!({ "sight": false, "light": true, "sound": false }),
    ));
    assert!(
        sealed.contains(&(1, 0)) && !sealed.contains(&(5, 0)),
        "the light stops at the region boundary: {sealed:?}"
    );
}

#[test]
fn a_viewer_inside_a_sight_occluding_region_sees_to_its_edge() {
    let (ecs, user, scene) = fixture_with_region(
        Some(json!({ "sight": true, "light": false, "sound": false })),
        BANK,
        250.0,
    );
    let seen = mask_cells(&ecs, user, scene);
    assert!(
        seen.contains(&(2, 0)) && seen.contains(&(3, 0)),
        "the viewer sees the cells inside the region: {seen:?}"
    );
    assert!(
        !seen.contains(&(5, 0)) && !seen.contains(&(0, 0)),
        "and nothing past its edge: {seen:?}"
    );
}

#[test]
fn a_hidden_sight_occluding_region_still_blocks_a_players_view() {
    let user = crate::test_ids::test_uuid(7);
    let (mut ecs, _, scene) = fixture_with_region(
        Some(json!({ "sight": true, "light": false, "sound": false })),
        BANK,
        50.0,
    );
    let mut region = doc(30, Some(10), "region");
    region.engine = Some(json!({
        "shape": { "kind": "rect", "points": [BANK.0, BANK.1, BANK.2, BANK.3] },
        "behavior": "terrain", "cost": 1.0, "enabled": true,
        "occludes": { "sight": true, "light": false, "sound": false },
    }));
    region
        .permissions
        .property_overrides
        .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
    ecs.apply_op(&Operation::Delete {
        doc: doc(30, Some(10), "region"),
    });
    ecs.apply_op(&Operation::Create { doc: region });
    let seen = mask_cells(&ecs, user, scene);
    assert!(
        seen.contains(&(1, 0)) && !seen.contains(&(5, 0)),
        "a gm_only region blocks sight like a wall the player cannot see: {seen:?}"
    );
}

#[test]
fn editing_occludes_changes_the_cached_visible_mask() {
    let (mut ecs, user, scene) = fixture_with_region(None, BANK, 50.0);
    let read = |ecs: &SceneEcs| {
        ecs.visible_cells_cached(
            user,
            WorldRole::Player,
            &no_world_grants(),
            scene,
            false,
            0.0,
        )
    };
    assert!(
        read(&ecs).contains(&(5, 0)),
        "uncached control: far cell visible"
    );
    ecs.apply_op(&Operation::Update {
        doc_id: crate::test_ids::test_uuid(30),
        changes: vec![fc(
            "/engine/occludes",
            json!({ "sight": true, "light": false, "sound": false }),
        )],
    });
    assert!(
        !read(&ecs).contains(&(5, 0)),
        "the cached mask must follow the new occluder"
    );
}

#[test]
fn a_region_with_occludes_does_not_change_the_movement_gate_walls() {
    let (ecs, _user, scene) = fixture_with_region(
        Some(json!({ "sight": true, "light": true, "sound": true })),
        BANK,
        50.0,
    );
    assert!(ecs.move_wall_entries(scene, None).is_empty());
}
