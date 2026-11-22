//! Facing-limited vision end to end: the resolved switch, the shared actor-field resolver, the
//! lit mask / movement gate / streamed-sight consumers, the cache fingerprint and creature senses.

use super::*;
use crate::data::command::{FieldChange, Operation};
use crate::data::document::DocRole;
use std::collections::BTreeSet;

/// The scene id every fixture parents to.
fn scene_id() -> Uuid {
    crate::test_ids::test_uuid(10)
}

/// The viewing player.
fn viewer() -> Uuid {
    crate::test_ids::test_uuid(7)
}

/// The token at world (1050, 1050) — the centre of cell (10, 10) on the fixture's 100-unit grid.
const TOKEN_ID: u128 = 11;

/// A world-settings body: all-bright, so only line of sight decides the mask, with the facing
/// switch set to `switch` (`None` leaves it unauthored).
fn settings(switch: Option<bool>) -> serde_json::Value {
    let mut patches = vec![
        ("/scene/lightingEnabled", json!(false)),
        ("/scene/partialCellLeniency", json!(false)),
    ];
    if let Some(on) = switch {
        patches.push(("/scene/facingVision", json!(on)));
    }
    ws_body(&patches)
}

/// A scene of 20 x 20 cells holding `tokens`, under `settings`.
fn ecs_with(tokens: Vec<Document>, settings: serde_json::Value) -> SceneEcs {
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                "bounds": { "width": 20.0, "height": 20.0 } }),
    );
    let mut docs = vec![scene];
    docs.extend(tokens);
    let mut ecs = SceneEcs::from_documents(docs, 0);
    ecs.set_world_settings_for_test(settings);
    ecs
}

/// A raw (actorless) token owned by `owner` at the centre of cell (10, 10), rotated `rotation`
/// degrees, carrying `facing` as its own `facing_vision` when given.
fn raw_token(id: u128, owner: Uuid, rotation: f64, facing: Option<serde_json::Value>) -> Document {
    let mut body =
        json!({ "x": 1050.0, "y": 1050.0, "w": 100.0, "h": 100.0, "rotation": rotation });
    if let Some(f) = facing {
        body["facing_vision"] = f;
    }
    let mut t = entity_doc_eng(id, 10, "token", body);
    t.owner = Some(owner);
    t
}

/// The cells `player_lit_mask` ships to `user` in the fixture scene.
fn lit(ecs: &SceneEcs, user: Uuid) -> BTreeSet<(i32, i32)> {
    mask_cells(ecs, user, scene_id())
}

/// The movement-gate mask of `user` in the fixture scene.
fn gate(ecs: &SceneEcs, user: Uuid) -> BTreeSet<(i32, i32)> {
    ecs.visible_cells(
        user,
        WorldRole::Player,
        &no_world_grants(),
        scene_id(),
        false,
        0.0,
    )
}

/// 90 degrees of arc, no authored bubble.
fn ninety() -> serde_json::Value {
    json!({ "arc": 90.0, "near": 0.0 })
}

const FRONT: (i32, i32) = (14, 10);
const BEHIND: (i32, i32) = (6, 10);
const BESIDE: (i32, i32) = (10, 14);
const OWN: (i32, i32) = (10, 10);

#[test]
fn switch_off_ignores_facing_data_and_sees_all_round() {
    for switch in [None, Some(false)] {
        let ecs = ecs_with(
            vec![raw_token(TOKEN_ID, viewer(), 0.0, Some(ninety()))],
            settings(switch),
        );
        for cells in [lit(&ecs, viewer()), gate(&ecs, viewer())] {
            for c in [FRONT, BEHIND, BESIDE, OWN] {
                assert!(cells.contains(&c), "switch {switch:?} must not limit {c:?}");
            }
        }
    }
}

#[test]
fn switch_on_limits_the_lit_mask_and_the_gate_to_the_arc() {
    let ecs = ecs_with(
        vec![raw_token(TOKEN_ID, viewer(), 0.0, Some(ninety()))],
        settings(Some(true)),
    );
    for (name, cells) in [("lit", lit(&ecs, viewer())), ("gate", gate(&ecs, viewer()))] {
        assert!(cells.contains(&FRONT), "{name}: ahead is seen");
        assert!(cells.contains(&OWN), "{name}: the token keeps its own cell");
        assert!(!cells.contains(&BEHIND), "{name}: behind is hidden");
        assert!(!cells.contains(&BESIDE), "{name}: beside is hidden");
    }
    assert_strict_parity(&ecs, viewer(), scene_id());
}

#[test]
fn a_token_without_facing_data_stays_unrestricted_when_the_switch_is_on() {
    let ecs = ecs_with(
        vec![raw_token(TOKEN_ID, viewer(), 0.0, None)],
        settings(Some(true)),
    );
    assert!(lit(&ecs, viewer()).contains(&BEHIND));
}

#[test]
fn a_full_circle_arc_is_no_restriction() {
    let ecs = ecs_with(
        vec![raw_token(
            TOKEN_ID,
            viewer(),
            0.0,
            Some(json!({ "arc": 360.0, "near": 0.0 })),
        )],
        settings(Some(true)),
    );
    assert!(lit(&ecs, viewer()).contains(&BEHIND));
}

#[test]
fn the_bubble_sees_all_round_inside_its_radius_only() {
    let ecs = ecs_with(
        vec![raw_token(
            TOKEN_ID,
            viewer(),
            0.0,
            Some(json!({ "arc": 90.0, "near": 2.5 })),
        )],
        settings(Some(true)),
    );
    let cells = lit(&ecs, viewer());
    assert!(
        cells.contains(&(9, 10)),
        "one cell behind is inside the bubble"
    );
    assert!(
        cells.contains(&(8, 10)),
        "two cells behind is inside the bubble"
    );
    assert!(
        !cells.contains(&(7, 10)),
        "three cells behind is outside it"
    );
}

#[test]
fn an_invalid_stored_arc_fails_closed_to_the_bubble() {
    for arc in [0.0, -5.0, 400.0] {
        let ecs = ecs_with(
            vec![raw_token(
                TOKEN_ID,
                viewer(),
                0.0,
                Some(json!({ "arc": arc, "near": 1.5 })),
            )],
            settings(Some(true)),
        );
        let cells = lit(&ecs, viewer());
        assert!(cells.contains(&(9, 10)), "arc {arc}: the bubble is kept");
        assert!(
            !cells.contains(&FRONT),
            "arc {arc}: no forward arc, not full sight"
        );
        assert!(!cells.contains(&BEHIND), "arc {arc}: not full sight");
    }
}

#[test]
fn turning_the_token_turns_the_arc_and_misses_the_cache() {
    let mut ecs = ecs_with(
        vec![raw_token(TOKEN_ID, viewer(), 0.0, Some(ninety()))],
        settings(Some(true)),
    );
    let cached = |ecs: &SceneEcs| {
        ecs.visible_cells_cached(
            viewer(),
            WorldRole::Player,
            &no_world_grants(),
            scene_id(),
            false,
            0.0,
        )
    };
    let before = cached(&ecs);
    assert!(before.contains(&FRONT) && !before.contains(&BEHIND));
    assert_eq!(ecs.visible_cells_recompute_count(), 1);
    assert_eq!(cached(&ecs), before);
    assert_eq!(
        ecs.visible_cells_recompute_count(),
        1,
        "an unchanged scene is served from the cache"
    );

    ecs.apply_op(&Operation::Update {
        doc_id: crate::test_ids::test_uuid(TOKEN_ID),
        changes: vec![FieldChange {
            path: "/engine/rotation".to_string(),
            old: json!(0.0),
            new: json!(180.0),
            remove: false,
        }],
    });
    let after = cached(&ecs);
    assert_eq!(
        ecs.visible_cells_recompute_count(),
        2,
        "a rotation-only change must miss the cache"
    );
    assert!(after.contains(&BEHIND) && !after.contains(&FRONT));
    assert_eq!(after, gate(&ecs, viewer()), "cached and uncached agree");
}

#[test]
fn toggling_the_switch_misses_the_cache() {
    let mut ecs = ecs_with(
        vec![raw_token(TOKEN_ID, viewer(), 0.0, Some(ninety()))],
        settings(Some(false)),
    );
    let cached = |ecs: &SceneEcs| {
        ecs.visible_cells_cached(
            viewer(),
            WorldRole::Player,
            &no_world_grants(),
            scene_id(),
            false,
            0.0,
        )
    };
    assert!(cached(&ecs).contains(&BEHIND));
    ecs.set_world_settings_for_test(settings(Some(true)));
    assert!(!cached(&ecs).contains(&BEHIND));
}

#[test]
fn a_wall_in_front_still_hides_the_cells_behind_it() {
    let wall = wall_doc_eng(scene_id(), (1250.0, 0.0), (1250.0, 2000.0));
    let ecs = ecs_with(
        vec![raw_token(TOKEN_ID, viewer(), 0.0, Some(ninety())), wall],
        settings(Some(true)),
    );
    let cells = lit(&ecs, viewer());
    assert!(cells.contains(&(11, 10)), "before the wall");
    assert!(
        !cells.contains(&(14, 10)),
        "behind the wall, inside the arc"
    );
}

#[test]
fn facing_still_applies_when_line_of_sight_is_off() {
    let mut s = settings(Some(true));
    s["scene"]["losRestriction"] = json!(false);
    let ecs = ecs_with(vec![raw_token(TOKEN_ID, viewer(), 0.0, Some(ninety()))], s);
    let cells = lit(&ecs, viewer());
    assert!(cells.contains(&FRONT));
    assert!(
        !cells.contains(&BEHIND),
        "the arc limits a scene without wall occlusion too"
    );
}

/// A linked-actor body with `facing_vision` set to `facing`.
fn actor_with_facing(facing: serde_json::Value) -> serde_json::Value {
    let mut b = actor_body(json!([]));
    b["facing_vision"] = facing;
    b
}

#[test]
fn the_facing_resolver_follows_the_vision_precedence_walk() {
    let actor_id = 50u128;
    let actor = |facing: serde_json::Value| {
        let mut a = doc(actor_id, None, "actor");
        a.engine = Some(actor_with_facing(facing));
        a
    };
    let linked = |overrides: Option<serde_json::Value>| {
        let mut body = json!({
            "x": 1050.0, "y": 1050.0, "w": 100.0, "h": 100.0, "rotation": 0.0,
            "actor_id": crate::test_ids::test_uuid(actor_id),
        });
        if let Some(o) = overrides {
            body["overrides"] = o;
        }
        let mut t = entity_doc_eng(TOKEN_ID, 10, "token", body);
        t.owner = Some(viewer());
        t
    };
    let build = |a: Document, t: Document| {
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "bounds": { "width": 20.0, "height": 20.0 } }),
        );
        let mut ecs = SceneEcs::from_documents(vec![scene, t], 0);
        ecs.set_actors(vec![a]);
        ecs.set_world_settings_for_test(settings(Some(true)));
        ecs
    };

    // (a) A linked token inherits its actor's arc.
    let ecs = build(actor(ninety()), linked(None));
    assert!(!lit(&ecs, viewer()).contains(&BEHIND), "actor arc applies");

    // (b) `overrides.facing_vision` replaces it wholesale: a full circle lifts the restriction.
    let ecs = build(
        actor(ninety()),
        linked(Some(
            json!({ "facing_vision": { "arc": 360.0, "near": 0.0 } }),
        )),
    );
    assert!(
        lit(&ecs, viewer()).contains(&BEHIND),
        "override replaces the actor's arc"
    );

    // (c) A dangling link (actor absent) ignores the override and the token's own band.
    let ecs = build(
        doc(999, None, "actor"),
        linked(Some(
            json!({ "facing_vision": { "arc": 90.0, "near": 0.0 } }),
        )),
    );
    assert!(
        lit(&ecs, viewer()).contains(&BEHIND),
        "dangling link is unrestricted"
    );

    // (d) An instanced token reads its embedded actor.
    let mut instanced = raw_token(TOKEN_ID, viewer(), 0.0, None);
    let mut embedded = doc(1000 + TOKEN_ID, None, "actor");
    embedded.engine = Some(actor_with_facing(ninety()));
    instanced.embedded.insert("actor".into(), vec![embedded]);
    let ecs = ecs_with(vec![instanced], settings(Some(true)));
    assert!(
        !lit(&ecs, viewer()).contains(&BEHIND),
        "embedded actor arc applies"
    );
}

#[test]
fn an_observer_vision_source_uses_its_own_rotation_and_arc() {
    let owner = crate::test_ids::test_uuid(8);
    let stranger = crate::test_ids::test_uuid(999);
    // The stranger owns nothing; the observed token faces -x (rotation 180).
    let mut observed = raw_token(TOKEN_ID, owner, 180.0, Some(ninety()));
    observed
        .permissions
        .users
        .insert(stranger, DocRole::Observer);
    let mut s = settings(Some(true));
    s["scene"]["observerVision"] = json!(true);
    let ecs = ecs_with(vec![observed], s);
    let cells = lit(&ecs, stranger);
    assert!(
        cells.contains(&BEHIND),
        "the observed token looks toward -x"
    );
    assert!(!cells.contains(&FRONT), "its own +x is outside its arc");
}

#[test]
fn streamed_sight_re_raycast_keeps_the_arc() {
    let ecs = ecs_with(
        vec![raw_token(TOKEN_ID, viewer(), 0.0, Some(ninety()))],
        settings(Some(true)),
    );
    let sight = ecs.sight_sources(viewer(), WorldRole::Player, &no_world_grants(), scene_id());
    // The mover is sampled two cells further along +x.
    let moved = (1250.0, 1050.0);
    let polys = sight.los_at(&[(crate::test_ids::test_uuid(TOKEN_ID), moved)]);
    let (vp, poly) = &polys[0];
    assert_eq!(*vp, moved);
    assert!(
        vision::point_in_poly(poly, (1550.0, 1050.0)),
        "ahead of the sample"
    );
    assert!(
        !vision::point_in_poly(poly, (950.0, 1050.0)),
        "behind the sample"
    );
}

#[test]
fn creature_senses_stay_all_round_with_facing_on() {
    let source = {
        let mut t = raw_token(TOKEN_ID, viewer(), 0.0, Some(ninety()));
        let mut a = doc(1000 + TOKEN_ID, None, "actor");
        a.engine = Some(actor_body(json!([{ "mode": "tremorsense" }])));
        t.embedded.insert("actor".into(), vec![a]);
        t
    };
    // The target stands three cells BEHIND the source.
    let mut target = entity_doc_eng(
        12,
        10,
        "token",
        json!({ "x": 750.0, "y": 1050.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    target.owner = Some(crate::test_ids::test_uuid(8));
    target.permissions.default = DocRole::Observer;
    let mut s = settings(Some(true));
    s["scene"]["lightingEnabled"] = json!(true);
    let ecs = ecs_with(vec![source, target], s);
    let bands = ecs.resolved_bands();
    let mask = ecs.player_lit_mask(viewer(), WorldRole::Player, &no_world_grants(), &bands);
    let perceived = ecs.player_perceived_tokens(
        &crate::data::membership::PermissionContext {
            user_id: viewer(),
            world_role: WorldRole::Player,
        },
        &no_world_grants(),
        &mask,
    );
    let ids: Vec<Uuid> = perceived.into_iter().flat_map(|p| p.tokens).collect();
    assert_eq!(ids, vec![crate::test_ids::test_uuid(12)]);
}
