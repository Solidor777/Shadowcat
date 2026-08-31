//! `SceneEcs` hydration/apply-op, movement-collision geometry, the `footprints` derived channel, and the vision/lighting config-doc resolvers.
use super::*;

#[test]
fn hydrate_counts_scene_entities_only() {
    let ecs = SceneEcs::from_documents(
        vec![
            doc(10, None, "scene"),
            doc(11, Some(10), "token"),
            doc(99, None, "actor"), // not a scene entity → ignored
        ],
        0,
    );
    assert_eq!(ecs.entity_count(), 2);
    assert_eq!(ecs.committed_seq(), 0);
}

#[test]
fn a_combatant_hydrates_as_an_inert_scene_entity() {
    let scene_id = Uuid::from_u128(1);
    let combat_id = Uuid::from_u128(2);
    let mut combatant = crate::data::document::tests::sample_doc();
    combatant.id = Uuid::from_u128(3);
    combatant.doc_type = "combatant".into();
    combatant.parent_id = Some(combat_id);
    combatant.engine = crate::data::document::tests::default_test_engine("combatant");
    let mut scene = crate::data::document::tests::sample_doc();
    scene.id = scene_id;
    scene.doc_type = "scene".into();
    scene.engine = crate::data::document::tests::default_test_engine("scene");
    let ecs = SceneEcs::from_documents(vec![scene, combatant], 0);
    assert_eq!(ecs.entity_count(), 2);
}

#[test]
fn resolve_grid_shape_selects_hex_grid_for_hex_kind_scenes() {
    let scene_id = Uuid::from_u128(10);
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "hex", "size": 50.0 }, "background": null }),
    );
    let ecs = SceneEcs::from_documents(vec![scene], 0);
    let shape = ecs.resolve_grid_shape(scene_id, 50.0);
    let want = grid_shape::HexGrid { size: 50.0 };
    assert_eq!(shape.cell_center((1, 0)), want.cell_center((1, 0)));
    assert_ne!(
        shape.cell_center((1, 0)),
        grid_shape::SquareGrid {
            cell: 50.0,
            rule: pathfinding::DiagonalRule::Chebyshev
        }
        .cell_center((1, 0)),
        "hex and square cell centers must differ for the same cell index/size"
    );
}

#[test]
fn resolve_grid_shape_falls_back_to_square_grid_for_unrecognized_kind() {
    let scene_id = Uuid::from_u128(10);
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "triangle", "size": 50.0 }, "background": null }),
    );
    let ecs = SceneEcs::from_documents(vec![scene], 0);
    let shape = ecs.resolve_grid_shape(scene_id, 50.0);
    let want = grid_shape::SquareGrid {
        cell: 50.0,
        rule: ecs.resolved_diagonal_rule(),
    };
    assert_eq!(shape.cell_center((1, 0)), want.cell_center((1, 0)));
}

#[test]
fn engine_as_cache_invalidates_on_engine_mutation() {
    let mut ecs = SceneEcs::from_documents(
        vec![
            doc(10, None, "scene"),
            entity_doc_eng(
                12,
                10,
                "wall",
                json!({ "seg": {"x1":0,"y1":0,"x2":10,"y2":10}, "blocksSight": true }),
            ),
        ],
        0,
    );
    let wall_id = Uuid::from_u128(12);
    let doc1 = {
        let e = ecs.index[&wall_id];
        ecs.world.get::<&SceneEntity>(e).unwrap().doc.clone()
    };
    let decoded1: eng::WallEngine = ecs.engine_as_cached(wall_id, &doc1).unwrap();
    assert_eq!(decoded1.blocks_sight, Some(true));

    // Mutate the engine field through the real apply_op chokepoint.
    ecs.apply_op(&Operation::Update {
        doc_id: wall_id,
        changes: vec![crate::data::command::FieldChange {
            remove: false,
            path: "/engine/blocksSight".into(),
            old: json!(true),
            new: json!(false),
        }],
    });

    let doc2 = {
        let e = ecs.index[&wall_id];
        ecs.world.get::<&SceneEntity>(e).unwrap().doc.clone()
    };
    let decoded2: eng::WallEngine = ecs.engine_as_cached(wall_id, &doc2).unwrap();
    assert_eq!(
        decoded2.blocks_sight,
        Some(false),
        "cache must invalidate on engine mutation, not serve stale decode"
    );
}

#[test]
fn apply_op_create_update_delete() {
    let mut ecs = SceneEcs::new();
    ecs.apply_op(&Operation::Create {
        doc: doc(11, Some(10), "token"),
    });
    assert_eq!(ecs.entity_count(), 1);
    ecs.apply_op(&Operation::Update {
        doc_id: Uuid::from_u128(11),
        changes: vec![crate::data::command::FieldChange {
            remove: false,
            path: "/system/x".into(),
            old: json!(null),
            new: json!(5),
        }],
    });
    let e = ecs.index[&Uuid::from_u128(11)];
    let comp = ecs.world.get::<&SceneEntity>(e).unwrap();
    assert_eq!(comp.doc.system["x"], json!(5));
    drop(comp);
    ecs.apply_op(&Operation::Delete {
        doc: doc(11, Some(10), "token"),
    });
    assert_eq!(ecs.entity_count(), 0);
}

#[test]
fn segments_cross_truth_table() {
    assert!(segments_cross(
        (0.0, 0.0),
        (10.0, 10.0),
        (0.0, 10.0),
        (10.0, 0.0)
    )); // X crossing
    assert!(!segments_cross(
        (0.0, 0.0),
        (10.0, 0.0),
        (0.0, 5.0),
        (10.0, 5.0)
    )); // parallel
    assert!(!segments_cross(
        (0.0, 0.0),
        (5.0, 0.0),
        (10.0, 0.0),
        (15.0, 0.0)
    )); // collinear disjoint
    assert!(segments_cross(
        (0.0, 0.0),
        (5.0, 0.0),
        (5.0, 0.0),
        (5.0, 5.0)
    )); // touching endpoint (T)
    assert!(segments_cross(
        (0.0, 0.0),
        (5.0, 10.0),
        (0.0, 5.0),
        (10.0, 5.0)
    )); // crossing
    assert!(segments_cross(
        (2.0, 0.0),
        (8.0, 0.0),
        (0.0, 0.0),
        (10.0, 0.0)
    )); // collinear OVERLAP (sliding along a wall)
}

#[test]
fn blocks_move_geometry_scene_scoping_and_filters() {
    let scene = Uuid::from_u128(10);
    let other = Uuid::from_u128(20);
    let cross = json!({ "seg": {"x1":0,"y1":10,"x2":10,"y2":0}, "blocksMove": true });

    // Scene 10 has one crossing blocksMove wall.
    let ecs = SceneEcs::from_documents(
        vec![
            doc(10, None, "scene"),
            entity_doc_eng(12, 10, "wall", cross.clone()),
        ],
        0,
    );
    assert!(ecs.blocks_move(scene, (0.0, 0.0), (10.0, 10.0))); // crosses the wall
    assert!(!ecs.blocks_move(scene, (0.0, 0.0), (1.0, 1.0))); // misses (sum 2 < 10)
    assert!(!ecs.blocks_move(scene, (0.0, 0.0), (0.0, 0.0))); // a no-op move never blocks

    // Scene scoping: an identical crossing wall in scene 20 blocks a scene-20 move but NOT
    // a scene-10 move (the `parent_id == Some(scene)` filter).
    let ecs_scope = SceneEcs::from_documents(
        vec![
            doc(10, None, "scene"),
            doc(20, None, "scene"),
            entity_doc_eng(24, 20, "wall", cross.clone()),
        ],
        0,
    );
    assert!(ecs_scope.blocks_move(other, (0.0, 0.0), (10.0, 10.0))); // blocks in scene 20
    assert!(!ecs_scope.blocks_move(scene, (0.0, 0.0), (10.0, 10.0))); // not in scene 10

    // A scene whose only crossing wall is blocksMove:false must not block movement.
    let ecs2 = SceneEcs::from_documents(
        vec![
            doc(10, None, "scene"),
            entity_doc_eng(
                13,
                10,
                "wall",
                json!({ "seg": {"x1":0,"y1":10,"x2":10,"y2":0}, "blocksMove": false }),
            ),
        ],
        0,
    );
    assert!(!ecs2.blocks_move(scene, (0.0, 0.0), (10.0, 10.0)));
}

/// Anti-drift check: `blocks_move` (the reference implementation) must agree with the
/// production traversal path (`move_walls(scene, None)` filtered by `segments_cross`, as
/// used by `move_exec`'s per-cell wall gate) on every segment tried here, evaluated against
/// BOTH scenes in the fixture — including the case where a segment would cross a wall that
/// belongs to the OTHER scene. A mutation of either `blocks_move`'s or `move_walls`'s wall
/// filter is expected to fail this test.
#[test]
fn blocks_move_agrees_with_the_production_move_walls_segments_cross_path() {
    let scene = Uuid::from_u128(10);
    let other_scene = Uuid::from_u128(20);
    let blocking = json!({ "seg": {"x1":0,"y1":10,"x2":10,"y2":0}, "blocksMove": true });
    let non_blocking = json!({ "seg": {"x1":0,"y1":0,"x2":0,"y2":20}, "blocksMove": false });
    let other_scene_wall = json!({ "seg": {"x1":20,"y1":30,"x2":30,"y2":20}, "blocksMove": true });

    let ecs = SceneEcs::from_documents(
        vec![
            doc(10, None, "scene"),
            doc(20, None, "scene"),
            entity_doc_eng(12, 10, "wall", blocking),
            entity_doc_eng(13, 10, "wall", non_blocking),
            entity_doc_eng(24, 20, "wall", other_scene_wall),
        ],
        0,
    );

    let segments = [
        ((0.0, 0.0), (10.0, 10.0)),   // crosses the scene-10 blocking wall
        ((0.0, 0.0), (1.0, 1.0)),     // crosses nothing
        ((0.0, 5.0), (5.0, 5.0)),     // crosses the scene-10 non-blocking wall only
        ((20.0, 20.0), (30.0, 30.0)), // would cross the OTHER scene's wall, not scene 10's
    ];
    for (a0, a1) in segments {
        for s in [scene, other_scene] {
            let production = ecs
                .move_walls(s, None)
                .iter()
                .any(|w| segments_cross(a0, a1, w.a, w.b));
            assert_eq!(
                ecs.blocks_move(s, a0, a1),
                production,
                "blocks_move disagreed with the production move_walls/segments_cross path for {a0:?}->{a1:?} in scene {s:?}"
            );
        }
    }
}

#[test]
fn token_move_uses_post_image_resisting_forged_bypasses() {
    let ecs = SceneEcs::from_documents(
        vec![
            doc(10, None, "scene"),
            entity_doc_eng(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0 }),
            ),
        ],
        0,
    );
    let id = Uuid::from_u128(11);
    // A normal two-axis move.
    let (s, a0, a1) = ecs
        .token_move(
            id,
            &[fc("/engine/x", json!(10)), fc("/engine/y", json!(10))],
        )
        .unwrap();
    assert_eq!(s, Uuid::from_u128(10));
    assert_eq!(a0, (0.0, 0.0));
    assert_eq!(a1, (10.0, 10.0));
    // Bypass A: a wholesale `/engine` write — the post-image reads the new x/y.
    let whole = fc(
        "/engine",
        json!({ "x": 50, "y": 50, "w": 1.0, "h": 1.0, "rotation": 0.0 }),
    );
    assert_eq!(ecs.token_move(id, &[whole]).unwrap().2, (50.0, 50.0));
    // Bypass B: duplicate `/engine/x` — last write wins, mirroring apply_intent.
    let dup = ecs
        .token_move(id, &[fc("/engine/x", json!(5)), fc("/engine/x", json!(50))])
        .unwrap();
    assert_eq!(dup.2 .0, 50.0);
    // A non-position update is a no-op move (committed == post-image).
    let noop = ecs
        .token_move(id, &[fc("/engine/rotation", json!(1.5))])
        .unwrap();
    assert_eq!(noop.1, noop.2);
    // A `/system/x` write on a token never touches the gate — position lives exclusively
    // in `/engine`; this is game-system data the movement gate must not see.
    let system_decoy = ecs.token_move(id, &[fc("/system/x", json!(999))]).unwrap();
    assert_eq!(
        system_decoy.1, system_decoy.2,
        "/system writes are not position and must not move the gate's post-image"
    );
    // A non-token id resolves to nothing.
    assert!(ecs.token_move(Uuid::from_u128(99), &[]).is_none());
}

#[test]
fn vision_channel_is_per_recipient() {
    use crate::data::document::WorldRole;
    let player = Uuid::from_u128(7);
    let mut token = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    token.owner = Some(player);
    let ecs = SceneEcs::from_documents(
        vec![
            doc(10, None, "scene"),
            token,
            entity_doc_eng(
                12,
                10,
                "wall",
                json!({ "seg": {"x1":10,"y1":-5,"x2":10,"y2":5}, "blocksSight": true }),
            ),
        ],
        0,
    );
    let gm = PermissionContext {
        user_id: Uuid::from_u128(1),
        world_role: WorldRole::Gm,
    };
    let pl = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let other = PermissionContext {
        user_id: Uuid::from_u128(9),
        world_role: WorldRole::Player,
    };

    // GM sees all (no fog).
    assert_eq!(
        compute_derived("vision", &ecs, &gm, &WorldCapDefaults::default()).unwrap()["mode"],
        "all"
    );
    // The token owner gets one non-empty visibility polygon, tagged with its scene so the
    // client cuts holes only for the scene it renders (cross-scene leak guard).
    let pv = compute_derived("vision", &ecs, &pl, &WorldCapDefaults::default()).unwrap();
    assert_eq!(pv["mode"], "masked");
    assert_eq!(pv["polygons"].as_array().unwrap().len(), 1);
    assert_eq!(pv["polygons"][0]["scene"], json!(Uuid::from_u128(10)));
    assert!(!pv["polygons"][0]["points"].as_array().unwrap().is_empty());
    // A player who controls no token gets empty polygons → full fog (never see-all).
    let ov = compute_derived("vision", &ecs, &other, &WorldCapDefaults::default()).unwrap();
    assert_eq!(ov["mode"], "masked");
    assert!(ov["polygons"].as_array().unwrap().is_empty());
    // Unknown channel → None.
    assert!(compute_derived("nope", &ecs, &gm, &WorldCapDefaults::default()).is_none());
}

#[test]
fn vision_payload_carries_lit_mask_for_players_not_gm() {
    use crate::data::document::WorldRole;
    use serde_json::json;
    let player = Uuid::from_u128(7);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(player);
    let light = entity_doc_eng(
        20,
        10,
        "light",
        json!({
        "x": 50.0, "y": 50.0, "emission": { "color": "#ffffff", "intensity": 1.0, "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true } }),
    );
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, light], 0);

    let pl = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let pv = compute_derived("vision", &ecs, &pl, &WorldCapDefaults::default()).unwrap();
    assert_eq!(pv["mode"], "masked");
    let lit = pv["lit"]
        .as_array()
        .expect("lit present for masked payload");
    assert_eq!(lit.len(), 1);
    assert_eq!(lit[0]["scene"], json!(Uuid::from_u128(10)));
    let cells = lit[0]["cells"].as_array().unwrap();
    assert!(!cells.is_empty());
    assert_eq!(
        cells.len() % 5,
        0,
        "cells packed 5 ints/cell (i,j,band,tint,hint_idx)"
    );
    assert!(!pv["bands"].as_array().unwrap().is_empty()); // bands now top-level
    assert!(
        pv["renderHints"].is_array(),
        "renderHints table present at top level"
    );
    assert!(
        lit[0].get("bands").is_none(),
        "bands hoisted to top level, not per-entry"
    );

    // GM payload is unchanged — no lit key or bands key.
    let gm = PermissionContext {
        user_id: Uuid::from_u128(1),
        world_role: WorldRole::Gm,
    };
    let gv = compute_derived("vision", &ecs, &gm, &WorldCapDefaults::default()).unwrap();
    assert_eq!(gv["mode"], "all");
    assert!(gv.get("lit").is_none());
    assert!(gv.get("bands").is_none());
    assert!(gv.get("renderHints").is_none());
}

#[test]
fn vision_payload_resolves_render_hint_index() {
    use crate::data::document::WorldRole;
    use serde_json::json;
    let player = Uuid::from_u128(7);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(player);
    tok.embedded.insert(
        "actor".into(),
        vec![{
            let mut a = doc(99, None, "actor");
            a.engine = Some(actor_body(json!([{ "mode": "darkvision", "range": 6 }])));
            a
        }],
    );
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok], 0);
    let pl = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let pv = compute_derived("vision", &ecs, &pl, &WorldCapDefaults::default()).unwrap();
    let hints = pv["renderHints"].as_array().unwrap();
    assert!(hints.iter().any(|h| h == "desaturate"));
    let cells = pv["lit"][0]["cells"].as_array().unwrap();
    let hint_idx = cells[4].as_i64().unwrap(); // 5th int of the first cell
    assert!(
        hint_idx >= 0,
        "first cell must have a resolved hint, not -1"
    );
    assert_eq!(pv["renderHints"][hint_idx as usize], json!("desaturate"));
}

#[test]
fn resolvers_layer_world_then_scene_and_fail_closed() {
    use serde_json::json;
    let scene_id = Uuid::from_u128(10);
    let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);

    // No config docs → built-in defaults (lighting on, environmentLight, env intensity 0).
    let r0 = ecs.resolve_scene(scene_id);
    assert!(r0.lighting_enabled);
    assert!(matches!(r0.light_mode, LightMode::EnvironmentLight));
    assert_eq!(r0.env_intensity, 0.0);
    assert_eq!(ecs.resolved_bands()[0].name, "bright"); // default gradation
    assert_eq!(
        ecs.resolved_vision_modes()["darkvision"].illumination_floor,
        "dark"
    );

    // World default: lighting OFF, global illumination.
    let mut ws = doc(100, None, "world-settings");
    ws.engine = Some(ws_body(&[
        ("/scene/lightingEnabled", json!(false)),
        ("/scene/lightMode", json!("globalIllumination")),
        (
            "/scene/environment",
            json!({ "color": "#0a0e1a", "intensity": 0.25 }),
        ),
    ]));
    ecs.set_world_config(Some(ws), None, None, None, None);
    let r1 = ecs.resolve_scene(scene_id);
    assert!(!r1.lighting_enabled);
    assert!(matches!(r1.light_mode, LightMode::GlobalIllumination));
    assert_eq!(r1.env_color, 0x0A0E1A);
    assert!((r1.env_intensity - 0.25).abs() < 1e-9);

    // Per-scene override re-enables lighting (null/absent ⇒ inherit; a present value wins).
    let mut scene = doc(10, None, "scene");
    scene.engine = Some(
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                           "lighting": { "enabled": true } }),
    );
    ecs.apply_op(&Operation::Update {
        doc_id: scene_id,
        changes: vec![crate::data::command::FieldChange {
            remove: false,
            path: "/engine".into(),
            old: json!(null),
            new: scene.engine.clone().unwrap(),
        }],
    });
    assert!(ecs.resolve_scene(scene_id).lighting_enabled); // scene override beats world default
}

#[test]
fn vision_modes_doc_is_respected_not_reseeded() {
    use serde_json::json;
    let mut ecs = SceneEcs::new();
    // A doc with ONLY a custom mode → returned as-is; normal/darkvision are NOT re-seeded.
    let mut vm = doc(101, None, "vision-modes");
    vm.engine = Some(json!({ "modes": { "blindsight": {
        "id": "blindsight", "name": "Blindsight",
        "illuminationFloor": "dark", "defaultRange": 4
    } } }));
    ecs.set_world_config(None, None, Some(vm), None, None);
    let modes = ecs.resolved_vision_modes();
    assert!(modes.contains_key("blindsight"));
    assert!(
        !modes.contains_key("normal"),
        "an authored modes doc must not be re-seeded"
    );
    // No doc at all → built-in seed.
    let empty = SceneEcs::new();
    assert!(empty.resolved_vision_modes().contains_key("darkvision"));
}

#[test]
fn pathfind_refuses_a_scene_with_no_document() {
    // Anti-drift with the two movement gates (`Room::publish`, `Room::execute_move`), which
    // both refuse this input: a router that substituted a 100-unit default would happily
    // return a route through a scene whose grid it invented, for a scene the gate that must
    // later authorize the move rejects outright. GM requester, so no mask or presence check
    // can account for the refusal — with the default restored this routes successfully.
    let ecs = SceneEcs::new();
    let out = ecs.pathfind(
        RouteRequester {
            user: Uuid::from_u128(7),
            is_gm: true,
            explored: None,
        },
        Uuid::from_u128(404),
        (50.0, 50.0),
        &[(450.0, 50.0)],
        0.1,
        None,
    );
    assert!(
        matches!(out, Err(pathfinding::PathFail::Invalid)),
        "a scene with no document is not routable"
    );
}

#[test]
fn user_owns_token_in_scene_follows_the_actor_join_and_is_scene_scoped() {
    use serde_json::json;
    // The pathfind presence gate keys on this. It must agree with
    // `token_effective_owner` (never a raw `doc.owner` read) and must be scoped to the
    // named scene, or a player is either locked out of a scene they hold an
    // actor-inherited token in, or admitted to one they hold nothing in.
    let player = Uuid::from_u128(7);
    let stranger = Uuid::from_u128(8);
    let mut actor = entity_doc_top_eng(200, "actor", actor_body(json!([])));
    actor.owner = Some(player);

    // Scene 10 holds a token linked to the actor with NO per-token owner.
    let inherited = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                "actor_id": Uuid::from_u128(200).to_string() }),
    );
    // Scene 20 holds only a token nobody in this test owns.
    let unowned = entity_doc_eng(
        21,
        20,
        "token",
        json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );

    let mut ecs = SceneEcs::from_documents(
        vec![
            doc(10, None, "scene"),
            doc(20, None, "scene"),
            inherited,
            unowned,
        ],
        0,
    );
    ecs.set_actors(vec![actor.clone()]);

    assert!(
        ecs.user_owns_token_in_scene(player, Uuid::from_u128(10)),
        "an actor-inherited owner controls a token in scene 10"
    );
    assert!(
        !ecs.user_owns_token_in_scene(player, Uuid::from_u128(20)),
        "presence is scene-scoped: scene 20 holds no token of theirs"
    );
    assert!(
        !ecs.user_owns_token_in_scene(stranger, Uuid::from_u128(10)),
        "a stranger controls nothing"
    );

    // Re-assigning the ACTOR moves presence with it, with no write to any token.
    let mut reassigned = actor;
    reassigned.owner = Some(stranger);
    ecs.set_actors(vec![reassigned]);
    assert!(
        !ecs.user_owns_token_in_scene(player, Uuid::from_u128(10)),
        "presence follows the actor live — nothing is stamped on the token"
    );
    assert!(ecs.user_owns_token_in_scene(stranger, Uuid::from_u128(10)));
}

/// The derived ECS must reach the SAME document state the authoritative store
/// reaches for every `FieldChange`, `remove: true` included. `remove` deletes the
/// key and `new` is unused (conventionally Null) — an ECS that only ever calls
/// `set_pointer` writes `ch.new` where the DB wrote absence, and `new` is
/// unconstrained by the OCC/capability checks. Routed through ownership because
/// that is where the divergence is exploitable: `/engine/actor_id` is a
/// WRITE_FIELDS path, so a token's effective owner can submit this, leaving the
/// write path saying "unowned" while the vision/lit-mask family says "owned by
/// whoever `new` names" — vision widening exactly where write refuses.
#[test]
fn ecs_and_db_agree_on_ownership_after_a_remove_change_carrying_a_non_null_new() {
    use crate::data::command::FieldChange;
    use serde_json::json;

    let p = Uuid::from_u128(7); // owns actor A
    let q = Uuid::from_u128(8); // owns actor B — the injected target
    let a_id = Uuid::from_u128(200);
    let b_id = Uuid::from_u128(201);
    let mut actor_a = entity_doc_top_eng(200, "actor", actor_body(json!([])));
    actor_a.owner = Some(p);
    let mut actor_b = entity_doc_top_eng(201, "actor", actor_body(json!([])));
    actor_b.owner = Some(q);

    let token = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                "actor_id": a_id.to_string() }),
    );

    // The change a token's effective owner can legitimately submit: `/engine/actor_id`
    // maps to WRITE_FIELDS, the OCC pre-image matches, and an absent optional
    // `actor_id` clears `validate_engine_tree`.
    let changes = vec![FieldChange {
        remove: true,
        path: "/engine/actor_id".into(),
        old: json!(a_id.to_string()),
        new: json!(b_id.to_string()), // unconstrained; ignored by the authoritative path
    }];

    // What the AUTHORITATIVE store reaches. Calls the hoisted rule
    // (`command::apply_field_change`) that `apply_intent` Phase 2 itself calls, so
    // this oracle cannot drift from the store — hand-copying the remove/set branch
    // here would silently go stale the moment `apply_intent` changed. NOT tautological
    // with respect to what this test pins: reverting the ECS mirror to an
    // unconditional `set_pointer` still diverges from this oracle and still fails.
    let db_token: Document = {
        let mut v = serde_json::to_value(&token).unwrap();
        for ch in &changes {
            apply_field_change(&mut v, ch).unwrap();
        }
        serde_json::from_value(v).unwrap()
    };
    let mut actor_index = std::collections::HashMap::new();
    actor_index.insert(a_id, actor_a.clone());
    actor_index.insert(b_id, actor_b.clone());
    let db_owner = crate::data::permission::effective_owner(
        &db_token,
        crate::data::permission::token_actor_link(&db_token).and_then(|id| actor_index.get(&id)),
    );
    assert_eq!(
        db_owner, None,
        "the link is gone in the authoritative store"
    );

    let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), token.clone()], 0);
    ecs.set_actors(vec![actor_a, actor_b]);
    ecs.apply_op(&Operation::Update {
        doc_id: token.id,
        changes,
    });

    let e = ecs.index[&token.id];
    let ecs_doc = ecs.world.get::<&SceneEntity>(e).unwrap().doc.clone();
    assert_eq!(
        ecs.token_effective_owner(&ecs_doc),
        db_owner,
        "derived ECS ownership must equal authoritative ownership after a remove change"
    );

    // The exploitable observable: the injected actor's owner must NOT gain the token
    // as a vision source for a token the write path considers unowned.
    assert!(
        ecs.player_vision_polygons(q).is_empty(),
        "a removed actor link must not hand the token's vision to the injected `new` owner"
    );
    assert!(ecs.player_vision_polygons(p).is_empty());
}

/// Control for `ecs_and_db_agree_when_a_remove_change_unlinks_a_token`: with
/// `remove: false` the ECS and the store agree
/// (both `set_pointer`), so the assertion pair there is about `remove`, not about
/// re-linking in general.
#[test]
fn ecs_and_db_agree_when_a_set_change_relinks_a_token() {
    use crate::data::command::FieldChange;
    use serde_json::json;

    let p = Uuid::from_u128(7);
    let q = Uuid::from_u128(8);
    let a_id = Uuid::from_u128(200);
    let b_id = Uuid::from_u128(201);
    let mut actor_a = entity_doc_top_eng(200, "actor", actor_body(json!([])));
    actor_a.owner = Some(p);
    let mut actor_b = entity_doc_top_eng(201, "actor", actor_body(json!([])));
    actor_b.owner = Some(q);

    let token = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                "actor_id": a_id.to_string() }),
    );
    let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), token.clone()], 0);
    ecs.set_actors(vec![actor_a, actor_b]);
    ecs.apply_op(&Operation::Update {
        doc_id: token.id,
        changes: vec![FieldChange {
            remove: false,
            path: "/engine/actor_id".into(),
            old: json!(a_id.to_string()),
            new: json!(b_id.to_string()),
        }],
    });

    let e = ecs.index[&token.id];
    let ecs_doc = ecs.world.get::<&SceneEntity>(e).unwrap().doc.clone();
    assert_eq!(
        ecs.token_effective_owner(&ecs_doc),
        Some(q),
        "a plain set DOES re-link, and both paths see it"
    );
    assert_eq!(ecs.player_vision_polygons(q).len(), 1);
    assert!(ecs.player_vision_polygons(p).is_empty());
}

/// The same divergence on the `self.actors` index (`apply_op`'s second
/// `set_pointer` loop): removing an actor's `/owner` must leave the actor
/// UNOWNED in the ECS, matching the store — not owned by the injected `new`.
#[test]
fn ecs_actor_index_honors_a_remove_change_on_owner() {
    use crate::data::command::FieldChange;
    use serde_json::json;

    let p = Uuid::from_u128(7);
    let q = Uuid::from_u128(8);
    let a_id = Uuid::from_u128(200);
    let mut actor_a = entity_doc_top_eng(200, "actor", actor_body(json!([])));
    actor_a.owner = Some(p);
    let token = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                "actor_id": a_id.to_string() }),
    );
    let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), token.clone()], 0);
    ecs.set_actors(vec![actor_a]);
    ecs.apply_op(&Operation::Update {
        doc_id: a_id,
        changes: vec![FieldChange {
            remove: true,
            path: "/owner".into(),
            old: json!(p.to_string()),
            new: json!(q.to_string()),
        }],
    });
    assert_eq!(ecs.actor(&a_id).unwrap().owner, None);
    let e = ecs.index[&token.id];
    let ecs_doc = ecs.world.get::<&SceneEntity>(e).unwrap().doc.clone();
    assert_eq!(ecs.token_effective_owner(&ecs_doc), None);
    assert!(ecs.player_vision_polygons(q).is_empty());
}

/// Collects the `Level` of every event emitted on the current thread, so a test can
/// assert what a code path is allowed to log — not merely what it computes.
#[derive(Default, Clone)]
struct LevelCapture(std::sync::Arc<std::sync::Mutex<Vec<tracing::Level>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for LevelCapture {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.0.lock().unwrap().push(*event.metadata().level());
    }
}

/// Run `f` with a thread-local capturing subscriber and return the levels emitted.
fn captured_levels(f: impl FnOnce()) -> Vec<tracing::Level> {
    use tracing_subscriber::layer::SubscriberExt;
    let cap = LevelCapture::default();
    let subscriber = tracing_subscriber::registry().with(cap.clone());
    tracing::subscriber::with_default(subscriber, f);
    let out = cap.0.lock().unwrap().clone();
    out
}

/// `Room::publish` reaches `token_move` with RAW client changes, strictly before
/// `apply_intent` runs `validate_field_change`, so a malformed path is untrusted
/// input any authenticated client can send at will. It must (a) fail closed and
/// (b) NOT emit at `error` — otherwise a client can flood on demand the channel
/// that exists to surface real store/mirror divergence.
#[test]
fn a_malformed_proposed_path_fails_closed_without_an_error_level_log() {
    use crate::data::command::FieldChange;
    use serde_json::json;

    let token = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 10.0, "y": 20.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), token.clone()], 0);

    // Descends through a scalar (`/engine/x` is a number) -> BadPath.
    let malformed = vec![FieldChange {
        remove: false,
        path: "/engine/x/y/z".into(),
        old: json!(null),
        new: json!(1.0),
    }];

    let mut out = None;
    let levels = captured_levels(|| out = Some(ecs.token_move(token.id, &malformed)));

    // Fails closed: the position is untouched, so the projected target equals the
    // committed one — the gate derives nothing from a malformed change.
    assert_eq!(
        out.unwrap().map(|(_, from, to)| (from, to)),
        Some(((10.0, 20.0), (10.0, 20.0))),
        "a malformed proposed path must not move the projected target"
    );
    assert!(
        !levels.contains(&tracing::Level::ERROR),
        "client-proposed malformed input must not emit at error level, got {levels:?}"
    );
    assert!(
        levels.contains(&tracing::Level::DEBUG),
        "the divergence must nonetheless be reported, at debug: got {levels:?}"
    );
}

/// The counterpart: at a COMMITTED mirror the same failure is a should-never-happen
/// invariant breach — the store applied a change the mirror could not — and must
/// stay at `error`. Same helper, two meanings; this pins that they do not collapse.
#[test]
fn a_failed_committed_mirror_change_logs_at_error_level() {
    use crate::data::command::FieldChange;
    use serde_json::json;

    let token = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 10.0, "y": 20.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), token.clone()], 0);

    let levels = captured_levels(|| {
        ecs.apply_op(&Operation::Update {
            doc_id: token.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/engine/x/y/z".into(),
                old: json!(null),
                new: json!(1.0),
            }],
        })
    });
    assert!(
        levels.contains(&tracing::Level::ERROR),
        "a committed change the mirror cannot apply is a real divergence: got {levels:?}"
    );
}

/// `apply_config_update` (the world-settings / gradation / vision-modes singleton
/// mirror) is the third site that must obey the store-equal mutation rule. It had
/// no coverage: reverting it to an unconditional `set_pointer` survived the whole
/// suite. A `remove` on a config field must leave the ECS singleton matching the
/// store's ABSENCE, not holding `ch.new`.
#[test]
fn config_singleton_mirror_honors_a_remove_change() {
    use crate::data::command::{apply_field_change, FieldChange};
    use serde_json::json;

    // A vision-modes doc with two modes; the change removes one of them while
    // naming a replacement in `new` (the value an unconditional set would land).
    let vm_id = Uuid::from_u128(300);
    let mut vm = doc(300, None, "vision-modes");
    vm.engine = Some(json!({ "modes": {
        "darkvision": { "id": "darkvision", "name": "Darkvision",
                        "illuminationFloor": "dark", "defaultRange": 6 },
        "blindsight": { "id": "blindsight", "name": "Blindsight",
                        "illuminationFloor": "dark", "defaultRange": 4 }
    }}));

    let change = FieldChange {
        remove: true,
        path: "/engine/modes/blindsight".into(),
        old: json!(null),
        // Unconstrained by OCC/capability checks: what a forked mirror would store.
        new: json!({ "id": "smuggled", "name": "Smuggled",
                     "illuminationFloor": "bright", "defaultRange": 99 }),
    };

    // Oracle: the hoisted rule the authoritative store itself calls.
    let store_engine = {
        let mut v = serde_json::to_value(&vm).unwrap();
        apply_field_change(&mut v, &change).unwrap();
        serde_json::from_value::<Document>(v)
            .unwrap()
            .engine
            .unwrap()
    };

    let mut ecs = SceneEcs::new();
    ecs.set_world_config(None, None, Some(vm), None, None);
    ecs.apply_op(&Operation::Update {
        doc_id: vm_id,
        changes: vec![change],
    });

    let mirrored = ecs.vision_modes_doc().unwrap().engine.clone().unwrap();
    assert_eq!(
        mirrored, store_engine,
        "the config singleton mirror must reach the store's value for a remove change"
    );
    assert!(
        mirrored["modes"].get("blindsight").is_none(),
        "the removed mode must be ABSENT, not replaced by `new`"
    );
    // The derived read-through agrees: the removed mode is gone, the other remains.
    let modes = ecs.resolved_vision_modes();
    assert!(!modes.contains_key("blindsight"));
    assert!(
        !modes.contains_key("smuggled"),
        "`new` must never be stored"
    );
    assert!(modes.contains_key("darkvision"));
}

/// `token_move` (the pre-commit move projection) is the fourth site. It had no
/// coverage either: an unconditional `set_pointer` there survived the suite. The
/// projected target must be derived by the same rule the store uses — so a `remove`
/// clears the coordinate rather than yielding a target taken from `new`.
///
/// Hardening, not a reachable committed divergence: `TokenEngine.x`/`y` are required
/// `f64`, so a `/engine/x` removal fails `validate_engine_tree` on the post-image and
/// never commits. This pins store-equality by construction at the site rather than
/// relying on that downstream rejection to stay true.
#[test]
fn token_move_projection_honors_a_remove_change() {
    use crate::data::command::FieldChange;
    use serde_json::json;

    let token = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 10.0, "y": 20.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), token.clone()], 0);

    // Control: a plain set projects the new position from `new`.
    let moved = ecs.token_move(
        token.id,
        &[FieldChange {
            remove: false,
            path: "/engine/x".into(),
            old: json!(10.0),
            new: json!(70.0),
        }],
    );
    assert_eq!(
        moved.map(|(_, from, to)| (from, to)),
        Some(((10.0, 20.0), (70.0, 20.0))),
        "a set change projects `new` as the target"
    );

    // A REMOVE of the same path must clear `/engine/x`, so no target is derivable —
    // never a target built from `new`. An unconditional `set_pointer` here yields
    // (99.0, 20.0) instead.
    let removed = ecs.token_move(
        token.id,
        &[FieldChange {
            remove: true,
            path: "/engine/x".into(),
            old: json!(10.0),
            new: json!(99.0),
        }],
    );
    assert_eq!(
        removed, None,
        "a removed coordinate yields no projected move, never one derived from `new`"
    );
}

#[test]
fn token_ownership_resolves_through_the_actor_join_for_vision() {
    use serde_json::json;
    // Vision/mask ownership MUST be the same rule the write-authz path uses
    // (`permission::effective_owner`), or a player could move a token that
    // contributes no vision — or see through one they cannot move.
    let player = Uuid::from_u128(7);
    let other = Uuid::from_u128(8);
    let mut actor = entity_doc_top_eng(200, "actor", actor_body(json!([])));
    actor.owner = Some(player);

    // Linked, NO per-token owner: inherits the actor's owner.
    let inherited = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                "actor_id": Uuid::from_u128(200).to_string() }),
    );
    // Linked to the SAME actor but overridden to `other`: the override wins.
    let mut overridden = entity_doc_eng(
        12,
        10,
        "token",
        json!({ "x": 200.0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                "actor_id": Uuid::from_u128(200).to_string() }),
    );
    overridden.owner = Some(other);
    // Linked to an actor that does not exist: fails closed to no owner.
    let dangling = entity_doc_eng(
        13,
        10,
        "token",
        json!({ "x": 400.0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                "actor_id": Uuid::from_u128(999).to_string() }),
    );

    let mut ecs = SceneEcs::from_documents(
        vec![
            doc(10, None, "scene"),
            inherited.clone(),
            overridden.clone(),
            dangling.clone(),
        ],
        0,
    );
    ecs.set_actors(vec![actor.clone()]);

    assert_eq!(
        ecs.token_effective_owner(&inherited),
        Some(player),
        "a linked token with no override inherits the actor's owner"
    );
    assert_eq!(
        ecs.token_effective_owner(&overridden),
        Some(other),
        "the per-token override supersedes the actor's owner"
    );
    assert_eq!(
        ecs.token_effective_owner(&dangling),
        None,
        "a dangling link fails closed"
    );

    // The vision channel agrees: the inheriting player gets exactly the one
    // token they effectively own, and the override holder gets exactly theirs.
    let polys = ecs.player_vision_polygons(player);
    assert_eq!(
        polys.len(),
        1,
        "only the inherited token is a vision source for its inheriting owner"
    );
    assert_eq!(ecs.player_vision_polygons(other).len(), 1);
    assert!(
        ecs.player_vision_polygons(Uuid::from_u128(99)).is_empty(),
        "a stranger owns nothing"
    );

    // Re-assigning the ACTOR moves vision ownership with no write to any token.
    let mut reassigned = actor;
    reassigned.owner = Some(other);
    ecs.set_actors(vec![reassigned]);
    assert_eq!(
        ecs.token_effective_owner(&inherited),
        Some(other),
        "ownership follows the actor live — nothing is stamped on the token"
    );
    assert!(ecs.player_vision_polygons(player).is_empty());
    assert_eq!(ecs.player_vision_polygons(other).len(), 2);
}

#[test]
fn token_vision_floors_resolve_through_actor_join() {
    use serde_json::json;
    let mut ecs = SceneEcs::new();
    // An actor granting darkvision range 6.
    ecs.set_actors(vec![entity_doc_top_eng(
        200,
        "actor",
        actor_body(json!([{ "mode": "darkvision", "range": 6 }])),
    )]);

    // Linked token referencing the actor → darkvision floor (dark=0.0), range 6.
    let mut linked = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                "actor_id": Uuid::from_u128(200).to_string() }),
    );
    let floors = ecs.token_vision_floors(&linked);
    assert_eq!(floors.len(), 1);
    assert_eq!(floors[0], (0.0, 6.0, Some("desaturate".to_string()))); // dark floor, 6-cell range, darkvision hint

    // A per-token override REPLACES the actor's vision entirely.
    linked.engine.as_mut().unwrap()["overrides"] =
        json!({ "vision": [{ "mode": "normal", "range": 0 }] });
    let f2 = ecs.token_vision_floors(&linked);
    assert_eq!(f2[0], (0.34, 0.0, None)); // dim floor, unlimited range, no hint (normal mode has render_hint: None)

    // An actorless token → normal only.
    let raw = entity_doc_eng(
        12,
        10,
        "token",
        json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    assert_eq!(ecs.token_vision_floors(&raw), vec![(0.34, 0.0, None)]);

    // An explicit EMPTY override REPLACES (no fall-through to the linked actor → normal).
    let mut linked_empty = entity_doc_eng(
        13,
        10,
        "token",
        json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                "actor_id": Uuid::from_u128(200).to_string(),
                "overrides": { "vision": [] } }),
    );
    assert_eq!(
        ecs.token_vision_floors(&linked_empty),
        vec![(0.34, 0.0, None)]
    );

    // A token with BOTH actor_id AND an embedded actor resolves the LINKED actor (matches the
    // client's actor_id-first resolveTokenActor), NOT the embedded copy.
    linked_empty.engine.as_mut().unwrap()["overrides"] = json!({}); // no vision override
    linked_empty.embedded.insert(
        "actor".into(),
        vec![entity_doc_top_eng(
            201,
            "actor",
            actor_body(json!([{ "mode": "normal", "range": 0 }])),
        )],
    );
    // actor 200 grants darkvision range 6 → linked wins → (0.0, 6.0), not the embedded normal.
    assert_eq!(
        ecs.token_vision_floors(&linked_empty),
        vec![(0.0, 6.0, Some("desaturate".to_string()))]
    );

    // A DANGLING link (actor_id with no matching actor) + an overrides.vision is normal — the
    // client ignores overrides when the linked actor is absent.
    let dangling = entity_doc_eng(
        14,
        10,
        "token",
        json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                "actor_id": Uuid::from_u128(999).to_string(),
                "overrides": { "vision": [{ "mode": "darkvision", "range": 9 }] } }),
    );
    assert_eq!(ecs.token_vision_floors(&dangling), vec![(0.34, 0.0, None)]);
}

/// A minimal, structurally-complete `ActorEngine` body with the caller's `shape`/`size`, for
/// the footprint tests.
fn actor_body_shaped(shape: &str, w: f64, h: f64) -> serde_json::Value {
    json!({
        "displayName": "Fixture Actor",
        "visual": { "kind": "image", "asset": "a.png" },
        "size": { "w": w, "h": h },
        "shape": shape,
        "conditions": [],
        "prototype": true,
    })
}

/// A hydrated ECS with one LINKED token (id 11) referencing an actor (id 200) of the given
/// `shape`/`size`, no overrides. Square-kind scene (the `doc()` helper stamps no `engine` at
/// all, so `grid_kind_from` falls back to `GridKind::Square`).
fn scene_with_linked_token_sized(shape: &str, w: f64, h: f64) -> (SceneEcs, Uuid) {
    scene_with_linked_token_sized_kind("square", shape, w, h)
}

/// `scene_with_linked_token_sized` generalized to an explicit `grid.kind`.
fn scene_with_linked_token_sized_kind(kind: &str, shape: &str, w: f64, h: f64) -> (SceneEcs, Uuid) {
    let token_id = Uuid::from_u128(11);
    let mut ecs = SceneEcs::from_documents(
        vec![
            entity_doc_top_eng(
                10,
                "scene",
                json!({ "grid": { "kind": kind, "size": 100.0 }, "background": null }),
            ),
            entity_doc_eng(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                        "actor_id": Uuid::from_u128(200).to_string() }),
            ),
        ],
        0,
    );
    ecs.set_actors(vec![entity_doc_top_eng(
        200,
        "actor",
        actor_body_shaped(shape, w, h),
    )]);
    (ecs, token_id)
}

/// A hydrated ECS with one raw (actorless) token (id 12) — no `actor_id`, no embedded actor.
fn scene_with_raw_token_no_actor() -> (SceneEcs, Uuid) {
    let token_id = Uuid::from_u128(12);
    let ecs = SceneEcs::from_documents(
        vec![
            doc(10, None, "scene"),
            entity_doc_eng(
                12,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
        ],
        0,
    );
    (ecs, token_id)
}

/// A hydrated ECS with one LINKED token (id 11) referencing a "square" 1x1 actor (id 200),
/// with a per-token `overrides.size` of the caller's `shape`/`(w, h)`.
fn scene_with_linked_token_overriding_size(shape: &str, w: f64, h: f64) -> (SceneEcs, Uuid) {
    let token_id = Uuid::from_u128(11);
    let mut ecs = SceneEcs::from_documents(
        vec![
            doc(10, None, "scene"),
            entity_doc_eng(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                        "actor_id": Uuid::from_u128(200).to_string(),
                        "overrides": { "shape": shape, "size": { "w": w, "h": h } } }),
            ),
        ],
        0,
    );
    ecs.set_actors(vec![entity_doc_top_eng(
        200,
        "actor",
        actor_body_shaped("square", 1.0, 1.0),
    )]);
    (ecs, token_id)
}

/// The scene id every `scene_with_linked_token_sized*`/`scene_with_raw_token_no_actor`
/// fixture in this block builds its scene document at — read once rather than restated at
/// every `resolve_token_footprint` call site below.
const FOOTPRINT_TEST_SCENE: Uuid = Uuid::from_u128(10);

#[test]
fn footprint_radius_on_square_is_the_conservative_enclosure_of_the_authored_block() {
    // On a square scene the radius is the authored block's conservative enclosure:
    //   circle ⇒ max(w,h)/2 ; square (and any other shape) ⇒ hypot(w,h)/2
    // Representative + boundary cases; `Size` is a free {w,h} pair, so there is no finite
    // domain to enumerate exhaustively.
    let cases = [
        ("square", 1.0, 1.0, std::f64::consts::SQRT_2 / 2.0),
        ("square", 2.0, 2.0, std::f64::consts::SQRT_2),
        ("square", 1.0, 2.0, 5.0f64.sqrt() / 2.0),
        ("circle", 1.0, 1.0, 0.5),
        ("circle", 2.0, 3.0, 1.5),
        // Any shape outside {"circle","square"} takes the half-diagonal branch.
        ("blob", 1.0, 1.0, std::f64::consts::SQRT_2 / 2.0),
    ];
    for (shape, w, h, expected) in cases {
        let (ecs, token) = scene_with_linked_token_sized(shape, w, h);
        let got = ecs
            .resolve_token_footprint(token, FOOTPRINT_TEST_SCENE)
            .expect("in-range");
        assert!(
            (got - expected).abs() < 1e-12,
            "shape={shape} w={w} h={h}: want {expected}, got {got}"
        );
    }
}

#[test]
fn footprint_radius_on_hex_is_the_circumscribing_radius_shape_is_inert() {
    // A token's authored size counts HEXES, and the conservative enclosure of one hex is its
    // own circumradius (`1.0` in cell units) — never the square half-diagonal `hypot(1,1)/2 ≈
    // 0.707` a square/circle formula gives when applied on hex.
    let cases = [
        ("square", 1.0, 1.0, 1.0),
        ("circle", 1.0, 1.0, 1.0), // shape is inert on hex
        ("square", 2.0, 1.0, 2.0), // n = max(w, h)
    ];
    for (shape, w, h, expected) in cases {
        let (ecs, token) = scene_with_linked_token_sized_kind("hex", shape, w, h);
        let got = ecs
            .resolve_token_footprint(token, FOOTPRINT_TEST_SCENE)
            .expect("in-range");
        assert!(
            (got - expected).abs() < 1e-12,
            "shape={shape} w={w} h={h}: want {expected}, got {got}"
        );
    }
}

#[test]
fn footprint_radius_falls_back_to_the_default_for_an_actorless_token() {
    let (ecs, token) = scene_with_raw_token_no_actor();
    assert_eq!(
        ecs.resolve_token_footprint(token, FOOTPRINT_TEST_SCENE),
        Some(DEFAULT_FOOTPRINT_RADIUS_CELLS),
        "a token with no actor to size it takes the sub-cell default"
    );
}

#[test]
fn footprint_radius_honors_a_per_token_size_override() {
    let (ecs, token) = scene_with_linked_token_overriding_size("circle", 4.0, 4.0);
    assert!(
        (ecs.resolve_token_footprint(token, FOOTPRINT_TEST_SCENE)
            .expect("in-range")
            - 2.0)
            .abs()
            < 1e-12
    );
}

#[test]
fn footprint_radius_refuses_an_oversized_token_rather_than_clamping() {
    // w=h=1000 ⇒ ~707 cells, far over MAX_FOOTPRINT_CELLS (64.0). Clamping would gate a
    // map-scale token as a 64-cell disc — a geometric fail-open.
    let (ecs, token) = scene_with_linked_token_sized("square", 1000.0, 1000.0);
    assert_eq!(
        ecs.resolve_token_footprint(token, FOOTPRINT_TEST_SCENE),
        None,
        "an out-of-range footprint is refused"
    );
}

#[test]
fn footprint_radius_admits_a_token_exactly_at_the_bound() {
    let at = pathfinding::MAX_FOOTPRINT_CELLS; // 64.0
    let (ecs, token) = scene_with_linked_token_sized("circle", at * 2.0, at * 2.0);
    assert_eq!(
        ecs.resolve_token_footprint(token, FOOTPRINT_TEST_SCENE),
        Some(at),
        "AT the bound is admissible"
    );
}

/// A GM context, for the footprint-payload tests that are not about the read filter.
fn footprint_gm_ctx() -> PermissionContext {
    PermissionContext {
        user_id: Uuid::from_u128(1),
        world_role: crate::data::document::WorldRole::Gm,
    }
}

/// A player context, for the footprint-payload tests that ARE about the egress filter. One
/// identity for all of them: a test that hides a document from one user id and queries another
/// measures nothing.
fn footprint_player_ctx() -> PermissionContext {
    PermissionContext {
        user_id: Uuid::from_u128(77),
        world_role: crate::data::document::WorldRole::Player,
    }
}

/// A document a `WorldRole::Player` may READ. `PermissionSet::default` is `DocRole::None`, so
/// a fixture document is invisible to a player unless it says otherwise — which would make a
/// player-facing assertion about anything that document carries pass for the wrong reason.
fn readable(mut d: Document) -> Document {
    d.permissions.default = crate::data::document::DocRole::Observer;
    d
}

/// Declare `d`'s `/engine` band GM-only: the document stays READABLE and its geometry band is
/// nulled on egress. The same `property_overrides` declaration a secret region carries,
/// applied to whichever document a footprint entry would disclose geometry from.
fn engine_band_gm_only(mut d: Document) -> Document {
    d.permissions
        .property_overrides
        .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
    d
}

/// A structurally-complete `SceneEngine` body at the cell size every footprint test in this
/// block scales its expectations by.
fn scene_body(kind: &str) -> serde_json::Value {
    json!({ "grid": { "kind": kind, "size": 100.0 }, "background": null })
}

/// A scene-parented `token` document LINKED to actor `actor`, at the origin.
fn linked_token_doc(id: u128, scene: u128, actor: u128) -> Document {
    entity_doc_eng(
        id,
        scene,
        "token",
        json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                "actor_id": Uuid::from_u128(actor).to_string() }),
    )
}

/// A scene-parented INSTANCED `token` document carrying `actor` as its own embedded copy: no
/// `actor_id`, so `token_geometry_source` resolves the embedded child.
fn instanced_token_doc(id: u128, scene: u128, actor: Document) -> Document {
    let mut d = entity_doc_eng(
        id,
        scene,
        "token",
        json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    d.embedded.insert("actor".into(), vec![actor]);
    d
}

/// A scene document a `WorldRole::Player` may READ.
fn readable_scene_doc(id: u128, body: serde_json::Value) -> Document {
    readable(entity_doc_top_eng(id, "scene", body))
}

/// The token ids a footprint payload carries for `scene`'s entry, in payload order.
fn tokens_of(payload: &footprint::FootprintsPayload, scene: Uuid) -> Vec<Uuid> {
    payload
        .scenes
        .iter()
        .find(|s| s.scene == scene)
        .map(|s| s.tokens.iter().map(|t| t.token).collect())
        .unwrap_or_default()
}

/// The one scene entry of a footprint payload built for a GM.
fn only_scene_footprints(ecs: &SceneEcs) -> footprint::SceneFootprints {
    let mut p = ecs.resolved_footprints(&footprint_gm_ctx(), &WorldCapDefaults::default());
    assert_eq!(p.scenes.len(), 1, "fixture has exactly one scene");
    p.scenes.remove(0)
}

#[test]
fn footprints_payload_carries_a_square_token_extent_of_the_authored_block_in_scene_units() {
    let (ecs, token) = scene_with_linked_token_sized("square", 2.0, 3.0);
    let s = only_scene_footprints(&ecs);
    assert_eq!(s.unit, footprint::FootprintExtent { w: 100.0, h: 100.0 });
    assert_eq!(
        s.tokens,
        vec![footprint::TokenFootprint {
            token,
            extent: Some(footprint::FootprintExtent { w: 200.0, h: 300.0 }),
        }]
    );
}

#[test]
fn footprints_payload_carries_a_hex_token_extent_of_the_hexs_own_bounding_box() {
    // A 1-hex token on a circumradius-100 hex grid spans the hex it sits in: `√3·100` across
    // the flats, `2·100` point to point — never the `100 × 100` square a square formula gives.
    let (ecs, token) = scene_with_linked_token_sized_kind("hex", "square", 1.0, 1.0);
    let s = only_scene_footprints(&ecs);
    let want = footprint::FootprintExtent {
        w: 3f64.sqrt() * 100.0,
        h: 200.0,
    };
    assert_eq!(s.unit, want);
    assert_eq!(
        s.tokens,
        vec![footprint::TokenFootprint {
            token,
            extent: Some(want),
        }]
    );
}

#[test]
fn footprints_payload_states_a_refusal_as_a_null_extent_rather_than_a_size() {
    let (ecs, token) = scene_with_linked_token_sized("square", 1000.0, 1000.0);
    assert_eq!(
        ecs.resolve_token_footprint(token, FOOTPRINT_TEST_SCENE),
        None,
        "the gate refuses this token"
    );
    let s = only_scene_footprints(&ecs);
    assert_eq!(
        s.tokens,
        vec![footprint::TokenFootprint {
            token,
            extent: None,
        }],
        "the wire states the same refusal rather than a drawable size"
    );
}

#[test]
fn the_footprints_channel_serves_the_resolved_payload_and_an_unknown_channel_errors() {
    let (ecs, token) = scene_with_linked_token_sized_kind("hex", "square", 1.0, 1.0);
    let payload = compute_derived(
        "footprints",
        &ecs,
        &footprint_gm_ctx(),
        &WorldCapDefaults::default(),
    )
    .expect("the channel is recognized");
    let decoded: footprint::FootprintsPayload =
        serde_json::from_value(payload).expect("the wire payload round-trips");
    assert_eq!(
        decoded,
        ecs.resolved_footprints(&footprint_gm_ctx(), &WorldCapDefaults::default())
    );
    assert_eq!(decoded.scenes[0].tokens[0].token, token);
    assert!(compute_derived(
        "footprint",
        &ecs,
        &footprint_gm_ctx(),
        &WorldCapDefaults::default()
    )
    .is_none());
}

#[test]
fn footprints_payload_omits_a_token_no_actor_sizes() {
    let (ecs, _token) = scene_with_raw_token_no_actor();
    let s = only_scene_footprints(&ecs);
    assert!(
        s.tokens.is_empty(),
        "a token with no actor has no server-resolved extent; its document's own w/h stand"
    );
}

#[test]
fn footprints_payload_withholds_a_token_the_recipient_cannot_read() {
    let scene = Uuid::from_u128(10);
    let open_token = Uuid::from_u128(12);
    let mut ecs = SceneEcs::from_documents(
        vec![
            // Readable, so the scene entry survives its own READ gate and the token gate is
            // the only thing this test can be measuring.
            readable_scene_doc(10, scene_body("square")),
            // Token 11 keeps `PermissionSet::default`'s `DocRole::None`: the recipient's
            // document stream never delivers it at all.
            linked_token_doc(11, 10, 200),
            readable(linked_token_doc(12, 10, 200)),
        ],
        0,
    );
    ecs.set_actors(vec![readable(entity_doc_top_eng(
        200,
        "actor",
        actor_body_shaped("square", 1.0, 1.0),
    ))]);
    let for_player = ecs.resolved_footprints(&footprint_player_ctx(), &WorldCapDefaults::default());
    assert_eq!(
        tokens_of(&for_player, scene),
        vec![open_token],
        "a token whose document the recipient may not read discloses no extent, while its \
         readable sibling in the same payload discloses one — so the absence is the READ \
         decision rather than an empty payload"
    );
    let for_gm = ecs.resolved_footprints(&footprint_gm_ctx(), &WorldCapDefaults::default());
    assert_eq!(
        tokens_of(&for_gm, scene).len(),
        2,
        "the GM, who may read the withheld token, receives both"
    );
}

#[test]
fn footprints_payload_withholds_a_scene_entry_the_recipient_cannot_read() {
    let open_scene = Uuid::from_u128(10);
    let secret_scene = Uuid::from_u128(20);
    let mut secret = entity_doc_top_eng(
        20,
        "scene",
        json!({ "grid": { "kind": "hex", "size": 100.0 }, "background": null }),
    );
    secret.permissions.default = crate::data::document::DocRole::None;
    let mut ecs = SceneEcs::from_documents(
        vec![
            readable_scene_doc(
                10,
                json!({ "grid": { "kind": "square", "size": 100.0 }, "background": null }),
            ),
            secret,
            entity_doc_eng(
                21,
                20,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                        "actor_id": Uuid::from_u128(200).to_string() }),
            ),
        ],
        0,
    );
    ecs.set_actors(vec![entity_doc_top_eng(
        200,
        "actor",
        actor_body_shaped("square", 1.0, 1.0),
    )]);
    let for_player = ecs.resolved_footprints(&footprint_player_ctx(), &WorldCapDefaults::default());
    assert!(
        !for_player.scenes.iter().any(|s| s.scene == secret_scene),
        "the whole entry is absent — its id and its unit extent are the disclosure, so an \
         empty token list is not a redaction"
    );
    assert_eq!(
        for_player
            .scenes
            .iter()
            .map(|s| s.scene)
            .collect::<Vec<_>>(),
        vec![open_scene],
        "the scene the recipient may read is carried, so absence is the access decision \
         rather than an empty payload"
    );
    let for_gm = ecs.resolved_footprints(&footprint_gm_ctx(), &WorldCapDefaults::default());
    let gm_secret = for_gm
        .scenes
        .iter()
        .find(|s| s.scene == secret_scene)
        .expect("the GM, who may read the secret scene, receives its entry");
    assert_eq!(
        gm_secret.tokens.len(),
        1,
        "and receives the tokens parented to it"
    );
}

#[test]
fn footprints_payload_withholds_a_scene_whose_engine_band_the_recipient_may_not_see() {
    let open_scene = Uuid::from_u128(10);
    let banded_scene = Uuid::from_u128(20);
    let ecs = SceneEcs::from_documents(
        vec![
            readable_scene_doc(10, scene_body("square")),
            // READABLE as a document, with `/engine` — the band carrying `grid.kind` and
            // `grid.size`, the only inputs `unit` is a function of — declared GM-only.
            engine_band_gm_only(readable_scene_doc(20, scene_body("hex"))),
        ],
        0,
    );
    let for_player = ecs.resolved_footprints(&footprint_player_ctx(), &WorldCapDefaults::default());
    assert_eq!(
        for_player
            .scenes
            .iter()
            .map(|s| s.scene)
            .collect::<Vec<_>>(),
        vec![open_scene],
        "the whole entry is absent — `unit` is that scene's grid kind and size, which the \
         recipient's document stream nulls; and the scene whose band is open is carried, \
         so the absence is the tier decision rather than an empty payload"
    );
    let for_gm = ecs.resolved_footprints(&footprint_gm_ctx(), &WorldCapDefaults::default());
    assert!(
        for_gm.scenes.iter().any(|s| s.scene == banded_scene),
        "the GM, who receives the band, receives the entry"
    );
}

#[test]
fn footprints_payload_withholds_a_token_whose_engine_band_the_recipient_may_not_see() {
    let scene = Uuid::from_u128(10);
    let open_token = Uuid::from_u128(12);
    let mut ecs = SceneEcs::from_documents(
        vec![
            readable_scene_doc(10, scene_body("square")),
            // READABLE, with `/engine` — the band carrying `overrides.shape`/`overrides.size`
            // — declared GM-only.
            engine_band_gm_only(readable(linked_token_doc(11, 10, 200))),
            readable(linked_token_doc(12, 10, 200)),
        ],
        0,
    );
    ecs.set_actors(vec![readable(entity_doc_top_eng(
        200,
        "actor",
        actor_body_shaped("square", 1.0, 1.0),
    ))]);
    let for_player = ecs.resolved_footprints(&footprint_player_ctx(), &WorldCapDefaults::default());
    assert_eq!(
        tokens_of(&for_player, scene),
        vec![open_token],
        "the whole entry is absent — a token id paired with an extent is the disclosure, so a \
         null extent is not a redaction; the sibling whose band is open is carried"
    );
    let for_gm = ecs.resolved_footprints(&footprint_gm_ctx(), &WorldCapDefaults::default());
    assert_eq!(
        tokens_of(&for_gm, scene).len(),
        2,
        "the GM, who receives the band, receives both"
    );
}

#[test]
fn footprints_payload_withholds_a_token_whose_actors_engine_band_the_recipient_may_not_see() {
    let scene = Uuid::from_u128(10);
    let open_token = Uuid::from_u128(12);
    let mut ecs = SceneEcs::from_documents(
        vec![
            readable_scene_doc(10, scene_body("square")),
            readable(linked_token_doc(11, 10, 200)),
            readable(linked_token_doc(12, 10, 201)),
        ],
        0,
    );
    // The linked actor is where a token's `size`/`shape` are authored, so its band decides the
    // extent even when the token's own band is open.
    ecs.set_actors(vec![
        engine_band_gm_only(readable(entity_doc_top_eng(
            200,
            "actor",
            actor_body_shaped("square", 2.0, 3.0),
        ))),
        readable(entity_doc_top_eng(
            201,
            "actor",
            actor_body_shaped("square", 1.0, 1.0),
        )),
    ]);
    let for_player = ecs.resolved_footprints(&footprint_player_ctx(), &WorldCapDefaults::default());
    assert_eq!(
        tokens_of(&for_player, scene),
        vec![open_token],
        "a token whose own band is open discloses no extent while the band authoring \
         that extent is nulled for this recipient"
    );
    let for_gm = ecs.resolved_footprints(&footprint_gm_ctx(), &WorldCapDefaults::default());
    assert_eq!(
        tokens_of(&for_gm, scene).len(),
        2,
        "the GM, who receives the actor band, receives both"
    );
}

#[test]
fn footprints_payload_withholds_a_token_whose_actor_document_the_recipient_may_not_read() {
    let scene = Uuid::from_u128(10);
    let open_token = Uuid::from_u128(12);
    let mut ecs = SceneEcs::from_documents(
        vec![
            readable_scene_doc(10, scene_body("square")),
            readable(linked_token_doc(11, 10, 200)),
            readable(linked_token_doc(12, 10, 201)),
        ],
        0,
    );
    // Actor 200 keeps `PermissionSet::default`'s `DocRole::None`: the recipient's document
    // stream never delivers it at all, band or no band.
    ecs.set_actors(vec![
        entity_doc_top_eng(200, "actor", actor_body_shaped("square", 2.0, 3.0)),
        readable(entity_doc_top_eng(
            201,
            "actor",
            actor_body_shaped("square", 1.0, 1.0),
        )),
    ]);
    let for_player = ecs.resolved_footprints(&footprint_player_ctx(), &WorldCapDefaults::default());
    assert_eq!(
        tokens_of(&for_player, scene),
        vec![open_token],
        "the extent is the actor's authored geometry, so a recipient who may not read the \
         actor document receives no entry for the token it sizes"
    );
    let for_gm = ecs.resolved_footprints(&footprint_gm_ctx(), &WorldCapDefaults::default());
    assert_eq!(
        tokens_of(&for_gm, scene).len(),
        2,
        "the GM, who may read the actor document, receives both"
    );
}

#[test]
fn footprints_payload_withholds_a_token_whose_embedded_actors_band_the_recipient_may_not_see() {
    let scene = Uuid::from_u128(10);
    let open_token = Uuid::from_u128(12);
    let ecs = SceneEcs::from_documents(
        vec![
            readable_scene_doc(10, scene_body("square")),
            // An embedded child carries its OWN `property_overrides`, applied against the
            // access resolved for the parent it rides in.
            readable(instanced_token_doc(
                11,
                10,
                engine_band_gm_only(entity_doc_top_eng(
                    200,
                    "actor",
                    actor_body_shaped("square", 2.0, 3.0),
                )),
            )),
            readable(instanced_token_doc(
                12,
                10,
                entity_doc_top_eng(201, "actor", actor_body_shaped("square", 1.0, 1.0)),
            )),
        ],
        0,
    );
    let for_player = ecs.resolved_footprints(&footprint_player_ctx(), &WorldCapDefaults::default());
    assert_eq!(
        tokens_of(&for_player, scene),
        vec![open_token],
        "an instanced token's geometry is authored in its embedded copy, so that child's band \
         decides the extent exactly as a linked actor's does"
    );
    let for_gm = ecs.resolved_footprints(&footprint_gm_ctx(), &WorldCapDefaults::default());
    assert_eq!(
        tokens_of(&for_gm, scene).len(),
        2,
        "the GM, who receives the embedded band, receives both"
    );
}

#[test]
fn light_and_blockslight_wall_accessors_filter_by_scene() {
    use serde_json::json;
    let scene = Uuid::from_u128(10);
    let ecs = SceneEcs::from_documents(
        vec![
            doc(10, None, "scene"),
            entity_doc_eng(
                20,
                10,
                "light",
                json!({
                    "x": 50.0, "y": 50.0, "emission": { "color": "#ffeeaa", "intensity": 1.0, "brightRadius": 2.0, "dimRadius": 6.0, "enabled": true }
                }),
            ),
            entity_doc_eng(
                21,
                10,
                "light",
                json!({ "x": 0.0, "y": 0.0, "emission": { "color": "#fff", "intensity": 1.0, "brightRadius": 1.0, "dimRadius": 2.0, "enabled": false } }),
            ),
            entity_doc_eng(
                22,
                10,
                "wall",
                json!({ "seg": {"x1":0,"y1":0,"x2":10,"y2":0}, "blocksLight": true }),
            ),
            entity_doc_eng(
                23,
                10,
                "wall",
                json!({ "seg": {"x1":0,"y1":5,"x2":10,"y2":5}, "blocksLight": false }),
            ),
        ],
        0,
    );
    let lights = ecs.scene_lights(scene);
    assert_eq!(lights.len(), 1); // the disabled light is excluded
    assert_eq!(lights[0].color, 0xFFEEAA);
    assert_eq!(lights[0].bright_radius, 2.0);
    let walls = ecs.light_walls(scene);
    assert_eq!(walls.len(), 1); // only the blocksLight:true wall

    // Cross-scene isolation: a second scene (id 20) with its own enabled light and a
    // blocksLight:true wall must NOT appear in scene 10's results.
    let scene2 = Uuid::from_u128(20);
    let ecs2 = SceneEcs::from_documents(
        vec![
            doc(10, None, "scene"),
            entity_doc_eng(
                20,
                10,
                "light",
                json!({
                    "x": 50.0, "y": 50.0, "emission": { "color": "#ffeeaa", "intensity": 1.0, "brightRadius": 2.0, "dimRadius": 6.0, "enabled": true }
                }),
            ),
            entity_doc_eng(
                22,
                10,
                "wall",
                json!({ "seg": {"x1":0,"y1":0,"x2":10,"y2":0}, "blocksLight": true }),
            ),
            doc(30, None, "scene"), // scene id 20 (doc id 30 → Uuid 30; parent is None)
            entity_doc_eng(
                31,
                30,
                "light",
                json!({
                    "x": 10.0, "y": 10.0, "emission": { "color": "#ffffff", "intensity": 0.8, "brightRadius": 3.0, "dimRadius": 8.0, "enabled": true }
                }),
            ),
            entity_doc_eng(
                32,
                30,
                "wall",
                json!({ "seg": {"x1":5,"y1":0,"x2":15,"y2":0}, "blocksLight": true }),
            ),
        ],
        0,
    );
    // Scene 10 still yields exactly its own 1 light and 1 wall.
    assert_eq!(ecs2.scene_lights(scene).len(), 1);
    assert_eq!(ecs2.light_walls(scene).len(), 1);
    // The second scene (id 30 via Uuid) has its own light and wall.
    let scene3 = Uuid::from_u128(30);
    assert_eq!(ecs2.scene_lights(scene3).len(), 1);
    assert_eq!(ecs2.light_walls(scene3).len(), 1);
    // Cross-check: scene 10's light is NOT scene2's light and vice-versa.
    assert_ne!(
        ecs2.scene_lights(scene)[0].pos,
        ecs2.scene_lights(scene3)[0].pos
    );
    // The unused scene2 uuid (20) is not a scene doc → yields empty (no children parented to 20).
    assert_eq!(ecs2.scene_lights(scene2).len(), 0);
}

#[test]
fn parse_hex_color_handles_6_and_3_digit() {
    assert_eq!(parse_hex_color("#0a0e1a"), 0x0A0E1A);
    assert_eq!(parse_hex_color("#fff"), 0xFFFFFF); // shorthand expands
    assert_eq!(parse_hex_color("#abc"), 0xAABBCC);
    assert_eq!(parse_hex_color("bad"), 0); // malformed → fail-closed black
    assert_eq!(parse_hex_color("#12345"), 0); // wrong length → 0
}

#[test]
fn lit_mask_gates_los_by_illumination_and_darkvision() {
    use serde_json::json;
    let player = Uuid::from_u128(7);
    let scene = Uuid::from_u128(10);

    // A normal-vision token at origin in a walled-open scene. lightingEnabled defaults true,
    // environmentLight, env intensity 0 → with NO lights the scene is dark → normal vision sees
    // nothing (fail-closed): the lit mask is empty.
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(player);
    let dark = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok.clone()], 0);
    assert!(
        dark.player_lit_mask(player, &dark.resolved_bands())
            .iter()
            .all(|s| s.cells.is_empty()),
        "dark scene + normal vision → empty lit mask"
    );

    // Add a bright light covering the token's cell → that cell becomes visible at the bright band.
    let light = entity_doc_eng(
        20,
        10,
        "light",
        json!({
        "x": 50.0, "y": 50.0, "emission": { "color": "#ffffff", "intensity": 1.0, "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true } }),
    );
    let lit = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok.clone(), light], 0);
    let mask = lit.player_lit_mask(player, &lit.resolved_bands());
    let s = mask
        .iter()
        .find(|s| s.scene == scene)
        .expect("scene present");
    assert!(
        s.cells
            .iter()
            .any(|&(i, j, band, _, _)| i == 0 && j == 0 && band == 0),
        "the lit cell at (0,0) is visible at the bright band (cell_size 100)"
    );

    // all_bright: a scene with lighting disabled makes every LOS cell visible at the bright
    // band even for a normal-vision token with NO lights present.
    let mut bright_scene = doc(10, None, "scene");
    bright_scene.engine = Some(
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                                  "lighting": { "enabled": false } }),
    );
    let mut ntok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    ntok.owner = Some(player);
    let bright_ecs = SceneEcs::from_documents(vec![bright_scene, ntok], 0);
    let ab = bright_ecs.player_lit_mask(player, &bright_ecs.resolved_bands());
    let s = ab.iter().find(|s| s.scene == scene).expect("scene present");
    assert!(
        s.cells
            .iter()
            .any(|&(i, j, band, _, _)| i == 0 && j == 0 && band == 0),
        "lighting-disabled scene → LOS cell visible at the bright band"
    );

    // Darkvision token in the SAME dark scene (no light) sees within range despite darkness.
    // Uses an embedded actor (instanced token path) because overrides.vision only applies to
    // linked tokens with a resolved actor_id; an instanced token reads embedded.actor[0].engine.vision.
    let mut dv = entity_doc_eng(
        12,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    dv.embedded.insert(
        "actor".into(),
        vec![entity_doc_top_eng(
            900,
            "actor",
            actor_body(json!([{ "mode": "darkvision", "range": 6 }])),
        )],
    );
    dv.owner = Some(player);
    let dv_ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), dv], 0);
    let dvmask = dv_ecs.player_lit_mask(player, &dv_ecs.resolved_bands());
    assert!(
        dvmask.iter().any(|s| !s.cells.is_empty()),
        "darkvision sees in the dark within range"
    );
}

#[test]
fn lit_mask_tags_darkvision_only_cells_with_hint() {
    use serde_json::json;
    let player = Uuid::from_u128(7);
    // Dark scene (no lights, environmentLight, lighting on) → only darkvision admits cells.
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(player);
    tok.embedded.insert(
        "actor".into(),
        vec![{
            let mut a = doc(99, None, "actor");
            a.engine = Some(actor_body(json!([{ "mode": "darkvision", "range": 6 }])));
            a
        }],
    );
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok], 0);
    let mask = ecs.player_lit_mask(player, &ecs.resolved_bands());
    assert_eq!(mask.len(), 1);
    assert!(
        !mask[0].cells.is_empty(),
        "darkvision must see at least one cell in range"
    );
    assert!(
        mask[0]
            .cells
            .iter()
            .all(|(_, _, _, _, h)| h.as_deref() == Some("desaturate")),
        "dark cells perceived only via darkvision carry the desaturate hint"
    );

    // Bright cell under a light, seen by normal vision → no hint (normal floor suppresses it).
    let player2 = Uuid::from_u128(8);
    let mut tok2 = entity_doc_eng(
        12,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok2.owner = Some(player2); // no embedded vision → normal fallback
    let light = entity_doc_eng(
        20,
        10,
        "light",
        json!({
        "x": 50.0, "y": 50.0, "emission": { "color": "#ffffff", "intensity": 1.0, "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true } }),
    );
    let lit = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok2, light], 0);
    let mask2 = lit.player_lit_mask(player2, &lit.resolved_bands());
    assert!(
        mask2[0].cells.iter().any(|(_, _, _, _, h)| h.is_none()),
        "a normally-lit cell seen by normal vision carries no hint"
    );
}

#[test]
fn committed_seq_tracks_last_applied_command() {
    // The watermark is the seq emitted as `computed_at_seq`; it advances only
    // via set_committed_seq, called under the same write lock as apply_op so a
    // reader never sees a watermark ahead of (or behind) the entities.
    let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 3);
    assert_eq!(ecs.committed_seq(), 3);
    ecs.set_committed_seq(7);
    assert_eq!(ecs.committed_seq(), 7);
}

/// Exercises the world-settings/vision-modes side tables and the `actor` table's
/// `set_actors`/`apply_op` mirroring. Deliberately stays on `/system`, not the typed
/// `engine` band: `world_settings_doc()`/`vision_modes_doc()` return the raw side-table
/// `Document`, and this test's own `apply_op` Update targets `/system/scene/...` and asserts
/// against `.system.pointer(...)` directly — a doc-round-trip mechanism check against the
/// side-table storage itself, with no engine-band reader in the loop to exercise.
#[test]
fn config_and_actor_side_tables_track_ops() {
    use serde_json::json;
    let mut ecs = SceneEcs::new();
    // Seed via setters (the room-hydration path).
    let mut ws = doc(100, None, "world-settings");
    ws.system = json!({ "scene": { "lightingEnabled": false } });
    ecs.set_world_config(Some(ws), None, None, None, None);
    ecs.set_actors(vec![entity_doc_top_eng(
        200,
        "actor",
        json!({ "vision": [] }),
    )]);
    assert!(ecs.actor(&Uuid::from_u128(200)).is_some());

    // A live Create of a vision-modes doc lands in the side table.
    ecs.apply_op(&Operation::Create {
        doc: doc(101, None, "vision-modes"),
    });
    assert!(ecs.vision_modes_doc().is_some());

    // A second world-settings Create REPLACES the singleton (the current authoritative doc wins).
    ecs.apply_op(&Operation::Create {
        doc: doc(110, None, "world-settings"),
    });
    assert_eq!(ecs.world_settings_doc().unwrap().id, Uuid::from_u128(110));

    // A field Update to the current world-settings singleton (id 110) is mirrored.
    ecs.apply_op(&Operation::Update {
        doc_id: Uuid::from_u128(110),
        changes: vec![crate::data::command::FieldChange {
            remove: false,
            path: "/system/scene/lightingEnabled".into(),
            old: json!(null),
            new: json!(true),
        }],
    });
    assert_eq!(
        ecs.world_settings_doc()
            .unwrap()
            .system
            .pointer("/scene/lightingEnabled"),
        Some(&json!(true))
    );

    // A Delete of the actor removes it.
    ecs.apply_op(&Operation::Delete {
        doc: doc(200, None, "actor"),
    });
    assert!(ecs.actor(&Uuid::from_u128(200)).is_none());
}

#[test]
fn vision_modes_carry_render_hint() {
    use serde_json::json;
    // Absent doc → built-in seed mirrors the client's `SEED_VISION_MODES`: darkvision desaturates, normal does not.
    let seeded = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
    let m = seeded.resolved_vision_modes();
    assert_eq!(m["normal"].render_hint, None);
    assert_eq!(m["darkvision"].render_hint.as_deref(), Some("desaturate"));

    // Present doc → renderHint parsed; absent field → None.
    let mut vm = entity_doc_eng(30, 10, "vision-modes", json!({}));
    vm.doc_type = "vision-modes".into();
    vm.parent_id = None;
    vm.engine = Some(json!({ "modes": {
        "truesight": { "id": "truesight", "name": "Truesight",
                       "illuminationFloor": "dark", "defaultRange": 8, "renderHint": "outline" },
        "plain":     { "id": "plain", "name": "Plain",
                       "illuminationFloor": "dim",  "defaultRange": 0 }
    }}));
    let mut ecs = SceneEcs::new();
    ecs.set_world_config(None, None, Some(vm), None, None);
    let m = ecs.resolved_vision_modes();
    assert_eq!(m["truesight"].render_hint.as_deref(), Some("outline"));
    assert_eq!(m["plain"].render_hint, None);
}

#[test]
fn token_vision_floors_include_render_hint() {
    use serde_json::json;
    // Instanced token with embedded actor granting normal + darkvision.
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.embedded.insert(
        "actor".into(),
        vec![{
            let mut a = doc(99, None, "actor");
            a.engine = Some(actor_body(json!([
                { "mode": "normal", "range": 0 },
                { "mode": "darkvision", "range": 6 }
            ])));
            a
        }],
    );
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok.clone()], 0);
    let floors = ecs.token_vision_floors(&tok);
    // darkvision entry carries the desaturate hint; normal carries none.
    assert!(floors
        .iter()
        .any(|(_, _, h)| h.as_deref() == Some("desaturate")));
    assert!(floors.iter().any(|(_, _, h)| h.is_none()));
}

#[test]
fn token_vision_floors_falls_back_to_mode_default_range_when_assignment_omits_range() {
    use serde_json::json;
    // Instanced token with an embedded actor granting darkvision but omitting `range`.
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.embedded.insert(
        "actor".into(),
        vec![{
            let mut a = doc(99, None, "actor");
            a.engine = Some(actor_body(json!([{ "mode": "darkvision" }])));
            a
        }],
    );
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok.clone()], 0);
    let floors = ecs.token_vision_floors(&tok);
    // No vision-modes doc set -> `resolved_vision_modes`'s built-in fallback seed, whose
    // darkvision entry carries `default_range: 12.0`.
    assert_eq!(floors, vec![(0.0, 12.0, Some("desaturate".to_string()))]);
}
