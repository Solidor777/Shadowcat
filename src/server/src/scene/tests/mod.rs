use super::*;
use crate::data::document::WorldCapDefaults;
use grid_shape::GridShape as _;
use serde_json::json;

fn doc(id: u128, parent: Option<u128>, ty: &str) -> Document {
    let mut d = crate::data::document::tests::world_scoped_doc(
        Uuid::from_u128(9),
        Uuid::from_u128(id),
        ty,
    );
    d.parent_id = parent.map(Uuid::from_u128);
    d
}

/// The one value read at every site in this module where a hex scene's declared `grid.size`
/// and the `HexGrid` a test derives COORDINATES from have to be the SAME number: the scene's
/// own `grid.size`, the shape the test builds, and any gate `cell` it passes alongside them.
/// A test whose expectations come from `cell_center`/`cell_vertices` is measuring the scene it
/// declared only while those agree, and nothing else makes them agree.
///
/// That scope is a PREDICATE, not a list, so a hex scene outside it is outside it because the
/// predicate does not hold, never by exemption. Two are worth naming only because they read
/// like members. The two `resolve_grid_shape_*` tests do build a shape and compare cell
/// centres, but their subject is that shape resolution takes its SIZE from the caller's
/// parameter and never from the document, so the declared size and the compared shape are
/// required to be INDEPENDENT — binding them would assert away the property under test, and a
/// parameter/expectation mismatch fails their `cell_center` comparison outright rather than
/// silently.
const HEX_FIXTURE_SIZE: f64 = 50.0;

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

/// Builds a scene-entity fixture with `engine` set to `body` (`system` stays `{}`), used by
/// every fixture whose doc_type the `scene` module's production code reads through
/// `engine_as`/a typed `*Engine` struct — every derivation reader there, `token_move`
/// included (movement position lives exclusively in `/engine`).
fn entity_doc_eng(id: u128, parent: u128, ty: &str, body: serde_json::Value) -> Document {
    let mut d = doc(id, Some(parent), ty);
    d.engine = Some(body);
    d
}

/// World-scoped (parentless) counterpart of `entity_doc_eng`, for config-docs
/// (`world-settings`/`vision-modes`/`light-gradation`) and `actor` docs.
fn entity_doc_top_eng(id: u128, ty: &str, body: serde_json::Value) -> Document {
    let mut d = doc(id, None, ty);
    d.engine = Some(body);
    d
}

/// A minimal, structurally-complete `ActorEngine` body (`displayName`/`visual`/`size`/
/// `shape`/`conditions`/`prototype` are all required, non-`Option` fields) with `vision` set
/// to the caller's assignment array — the vision-floor tests only ever vary `vision`.
fn actor_body(vision: serde_json::Value) -> serde_json::Value {
    json!({
        "displayName": "Fixture Actor",
        "visual": { "kind": "image", "asset": "a.png" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "conditions": [],
        "prototype": true,
        "vision": vision,
    })
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

fn fc(path: &str, new: serde_json::Value) -> crate::data::command::FieldChange {
    crate::data::command::FieldChange {
        remove: false,
        path: path.into(),
        old: json!(0),
        new,
    }
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
    let other_scene_wall =
        json!({ "seg": {"x1":20,"y1":30,"x2":30,"y2":20}, "blocksMove": true });

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
        "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
        "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true }),
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
    ecs.set_world_config(Some(ws), None, None);
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
    ecs.set_world_config(None, None, Some(vm));
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
        crate::data::permission::token_actor_link(&db_token)
            .and_then(|id| actor_index.get(&id)),
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
    ecs.set_world_config(None, None, Some(vm));
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
fn scene_with_linked_token_sized_kind(
    kind: &str,
    shape: &str,
    w: f64,
    h: f64,
) -> (SceneEcs, Uuid) {
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
    let for_player =
        ecs.resolved_footprints(&footprint_player_ctx(), &WorldCapDefaults::default());
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
    let for_player =
        ecs.resolved_footprints(&footprint_player_ctx(), &WorldCapDefaults::default());
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
    let for_player =
        ecs.resolved_footprints(&footprint_player_ctx(), &WorldCapDefaults::default());
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
    let for_player =
        ecs.resolved_footprints(&footprint_player_ctx(), &WorldCapDefaults::default());
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
    let for_player =
        ecs.resolved_footprints(&footprint_player_ctx(), &WorldCapDefaults::default());
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
    let for_player =
        ecs.resolved_footprints(&footprint_player_ctx(), &WorldCapDefaults::default());
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
    let for_player =
        ecs.resolved_footprints(&footprint_player_ctx(), &WorldCapDefaults::default());
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
                    "x": 50.0, "y": 50.0, "color": "#ffeeaa", "intensity": 1.0,
                    "brightRadius": 2.0, "dimRadius": 6.0, "enabled": true
                }),
            ),
            entity_doc_eng(
                21,
                10,
                "light",
                json!({ "x": 0.0, "y": 0.0, "color": "#fff",
                "intensity": 1.0, "brightRadius": 1.0, "dimRadius": 2.0, "enabled": false }),
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
                    "x": 50.0, "y": 50.0, "color": "#ffeeaa", "intensity": 1.0,
                    "brightRadius": 2.0, "dimRadius": 6.0, "enabled": true
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
                    "x": 10.0, "y": 10.0, "color": "#ffffff", "intensity": 0.8,
                    "brightRadius": 3.0, "dimRadius": 8.0, "enabled": true
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
        dark.player_lit_mask(player)
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
        "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
        "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true }),
    );
    let lit = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok.clone(), light], 0);
    let mask = lit.player_lit_mask(player);
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
    let ab = SceneEcs::from_documents(vec![bright_scene, ntok], 0).player_lit_mask(player);
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
    let dvmask =
        SceneEcs::from_documents(vec![doc(10, None, "scene"), dv], 0).player_lit_mask(player);
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
    let mask = ecs.player_lit_mask(player);
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
        "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
        "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true }),
    );
    let lit = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok2, light], 0);
    let mask2 = lit.player_lit_mask(player2);
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
    ecs.set_world_config(Some(ws), None, None);
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
    ecs.set_world_config(None, None, Some(vm));
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

// --- Test helpers for movement-restriction resolution tests ---

/// Set `world_settings` to a doc whose `engine` is `json_engine` (test-only).
/// Mirrors `document::tests::world_scoped_doc`, the same test helper `ws::room`'s own tests
/// use to build a world-settings config doc.
#[cfg(test)]
impl SceneEcs {
    pub(crate) fn set_world_settings_for_test(&mut self, json_engine: serde_json::Value) {
        let mut d = crate::data::document::tests::world_scoped_doc(
            Uuid::from_u128(9),
            Uuid::from_u128(100),
            "world-settings",
        );
        d.engine = Some(json_engine);
        self.world_settings = Some(d);
    }

    pub(crate) fn insert_scene_for_test(
        &mut self,
        scene_id: Uuid,
        json_engine: serde_json::Value,
    ) {
        let mut d = crate::data::document::tests::world_scoped_doc(
            Uuid::from_u128(9),
            scene_id,
            "scene",
        );
        d.engine = Some(json_engine);
        // Remove stale entity if re-inserting.
        if let Some(old_e) = self.index.remove(&scene_id) {
            let _ = self.world.despawn(old_e);
        }
        let e = self.world.spawn((SceneEntity { doc: d },));
        self.index.insert(scene_id, e);
    }
}

/// A COMPLETE `WorldSettingsEngine` body (all `WorldSceneDefaults` fields present, per the
/// ingress-validated struct's `deny_unknown_fields` contract) with `patches` applied over the
/// built-in default via JSON-pointer `set_pointer` — lets each test express only the field(s)
/// it cares about instead of re-typing the full 9-field `scene` object every time.
fn ws_body(patches: &[(&str, serde_json::Value)]) -> serde_json::Value {
    use crate::data::command::set_pointer;
    let mut v = serde_json::to_value(eng::WorldSettingsEngine::default()).unwrap();
    for (path, val) in patches {
        let _ = set_pointer(&mut v, path, val.clone());
    }
    v
}

#[test]
fn diagonal_rule_defaults_to_chebyshev_without_world_settings() {
    let ecs = SceneEcs::new();
    assert_eq!(
        ecs.resolved_diagonal_rule(),
        crate::scene::pathfinding::DiagonalRule::Chebyshev
    );
}

#[test]
fn diagonal_rule_falls_back_when_structural_keys_absent() {
    // A world-settings doc with `pathfinding.diagonalRule:"alternating"` but missing `scene`
    // or `animation` must resolve to `Chebyshev` — the structural guard (same as resolve_scene)
    // must reject a partial doc rather than partially resolving.
    use serde_json::json;
    let mut ecs = SceneEcs::new();

    // Missing `scene` key entirely.
    ecs.set_world_settings_for_test(json!({
        "pathfinding": { "diagonalRule": "alternating" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    assert_eq!(
        ecs.resolved_diagonal_rule(),
        crate::scene::pathfinding::DiagonalRule::Chebyshev,
        "missing scene key must fall back to Chebyshev"
    );

    // Missing `animation` key entirely.
    ecs.set_world_settings_for_test(json!({
        "scene": { "movementRestriction": "visible" },
        "pathfinding": { "diagonalRule": "alternating" }
    }));
    assert_eq!(
        ecs.resolved_diagonal_rule(),
        crate::scene::pathfinding::DiagonalRule::Chebyshev,
        "missing animation key must fall back to Chebyshev"
    );
}

#[test]
fn diagonal_rule_reads_world_settings_and_unknown_falls_back() {
    use serde_json::json;
    let mut ecs = SceneEcs::new();
    ecs.set_world_settings_for_test(ws_body(&[(
        "/pathfinding/diagonalRule",
        json!("alternating"),
    )]));
    assert_eq!(
        ecs.resolved_diagonal_rule(),
        crate::scene::pathfinding::DiagonalRule::Alternating
    );

    ecs.set_world_settings_for_test(json!({
        "scene": {}, "pathfinding": { "diagonalRule": "bogus" }, "animation": {}
    }));
    assert_eq!(
        ecs.resolved_diagonal_rule(),
        crate::scene::pathfinding::DiagonalRule::Chebyshev,
        "unknown rule fails to chebyshev (mirrors client default)"
    );
}

#[test]
fn resolve_scene_movement_restriction_defaults_to_visible_and_lenient() {
    // No world-settings doc, no scene override → built-in defaults.
    let ecs = SceneEcs::new();
    let r = ecs.resolve_scene(Uuid::from_u128(1));
    assert_eq!(r.movement_restriction, MovementRestriction::Visible);
    assert!(r.partial_cell_leniency);
}

#[test]
fn resolve_scene_movement_restriction_world_override_and_leniency_off() {
    use serde_json::json;
    let mut ecs = SceneEcs::new();
    ecs.set_world_settings_for_test(ws_body(&[
        ("/scene/movementRestriction", json!("revealed")),
        ("/scene/partialCellLeniency", json!(false)),
    ]));
    let r = ecs.resolve_scene(Uuid::from_u128(1));
    assert_eq!(r.movement_restriction, MovementRestriction::Revealed);
    assert!(
        !r.partial_cell_leniency,
        "partialCellLeniency is world-only and was set false"
    );
}

#[test]
fn resolve_scene_movement_restriction_scene_override_beats_world() {
    use serde_json::json;
    let mut ecs = SceneEcs::new();
    let scene_id = Uuid::from_u128(7);
    ecs.set_world_settings_for_test(ws_body(&[(
        "/scene/movementRestriction",
        json!("visible"),
    )]));
    // Scene overrides vision.movementRestriction to "unrestricted".
    ecs.insert_scene_for_test(
        scene_id,
        json!({
            "grid": { "kind": "square", "size": 100 },
            "background": null,
            "vision": { "movementRestriction": "unrestricted" }
        }),
    );
    let r = ecs.resolve_scene(scene_id);
    assert_eq!(r.movement_restriction, MovementRestriction::Unrestricted);
    // partialCellLeniency has NO scene override → still the world default (true here).
    assert!(r.partial_cell_leniency);
}

#[test]
fn resolve_scene_movement_restriction_null_override_inherits_world() {
    use serde_json::json;
    let mut ecs = SceneEcs::new();
    let scene_id = Uuid::from_u128(8);
    ecs.set_world_settings_for_test(ws_body(&[(
        "/scene/movementRestriction",
        json!("revealed"),
    )]));
    // null clears the override → inherit world "revealed" (mirrors `?? d.scene.movementRestriction`).
    ecs.insert_scene_for_test(
        scene_id,
        json!({
            "grid": { "kind": "square", "size": 100 },
            "background": null,
            "vision": { "movementRestriction": null }
        }),
    );
    let r = ecs.resolve_scene(scene_id);
    assert_eq!(r.movement_restriction, MovementRestriction::Revealed);
}

#[test]
fn resolve_scene_movement_model_defaults_to_grid_stepped() {
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
    let r = ecs.resolve_scene(Uuid::from_u128(10));
    assert_eq!(r.movement_model, MovementModel::GridStepped);
}

#[test]
fn resolve_scene_movement_model_world_override_to_continuous() {
    let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": true, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#0a0e1a", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "movementModel": "continuous",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    let r = ecs.resolve_scene(Uuid::from_u128(10));
    assert_eq!(r.movement_model, MovementModel::Continuous);
}

#[test]
fn resolve_scene_movement_model_scene_override_beats_world() {
    let mut ecs = SceneEcs::from_documents(
        vec![entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "vision": { "movementModel": "continuous" } }),
        )],
        0,
    );
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": true, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#0a0e1a", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "movementModel": "grid-stepped",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    let r = ecs.resolve_scene(Uuid::from_u128(10));
    assert_eq!(r.movement_model, MovementModel::Continuous);
}

#[test]
fn resolve_scene_movement_model_null_scene_override_inherits_world() {
    let mut ecs = SceneEcs::from_documents(
        vec![entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "vision": { "movementModel": null } }),
        )],
        0,
    );
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": true, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#0a0e1a", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "movementModel": "continuous",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    let r = ecs.resolve_scene(Uuid::from_u128(10));
    assert_eq!(r.movement_model, MovementModel::Continuous);
}

#[test]
fn resolve_scene_bounds_defaults_when_absent() {
    let ecs = SceneEcs::new();
    let r = ecs.resolve_scene(Uuid::from_u128(1));
    assert_eq!(r.bounds, (100.0, 100.0));
}

#[test]
fn resolve_scene_bounds_reads_authored_value() {
    use serde_json::json;
    let mut ecs = SceneEcs::new();
    let scene_id = Uuid::from_u128(2);
    ecs.insert_scene_for_test(
        scene_id,
        json!({
            "grid": { "kind": "square", "size": 100 },
            "bounds": { "width": 40.0, "height": 25.0 }
        }),
    );
    let r = ecs.resolve_scene(scene_id);
    assert_eq!(r.bounds, (40.0, 25.0));
}

#[test]
fn resolve_scene_bounds_fail_closed_on_degenerate() {
    use serde_json::json;
    let mut ecs = SceneEcs::new();
    let scene_id = Uuid::from_u128(3);
    // Zero width + negative height are degenerate for a navmesh rectangle → default.
    ecs.insert_scene_for_test(
        scene_id,
        json!({
            "grid": { "kind": "square", "size": 100 },
            "bounds": { "width": 0.0, "height": -5.0 }
        }),
    );
    let r = ecs.resolve_scene(scene_id);
    assert_eq!(r.bounds, (100.0, 100.0));
}

#[test]
fn the_resolved_shape_reports_the_resolved_kind() {
    // The three readers of the same decision — the shape a scene resolves to, the kind its
    // settings carry, and the ECS resolver — must not be able to disagree.
    // Discrimination: fails if `resolve_grid_shape_with_rule` stops constructing its shape
    // from `resolve_grid_kind`, or if `resolve_scene` stops reading the same pure helper,
    // which are the only ways the three can diverge. The unrecognised spelling pins the
    // fail-closed default in the same loop.
    for (engine, expect) in [
        (
            json!({ "grid": { "kind": "hex", "size": 50 }, "background": null }),
            GridKind::Hex,
        ),
        (
            json!({ "grid": { "kind": "square", "size": 50 }, "background": null }),
            GridKind::Square,
        ),
        (
            json!({ "grid": { "kind": "wobbly", "size": 50 }, "background": null }),
            GridKind::Square,
        ),
    ] {
        let ecs = SceneEcs::from_documents(vec![entity_doc_top_eng(10, "scene", engine)], 0);
        let scene = Uuid::from_u128(10);
        assert_eq!(ecs.resolve_grid_kind(scene), expect);
        assert_eq!(ecs.resolve_scene(scene).grid_kind, expect);
        let declared = *ecs
            .scene_grid_sizes()
            .get(&scene)
            .expect("the fixture's scene declares a grid size");
        assert_eq!(ecs.resolve_grid_shape(scene, declared).kind(), expect);
    }
}

#[test]
fn changing_a_scenes_grid_kind_invalidates_the_cached_visibility_mask() {
    // The cache is value-comparison based, so any input the mask depends on must appear in
    // the snapshot. The grid kind decides every cell index in the mask.
    // Discrimination: fails if `ResolvedScene` carries no kind or the snapshot omits it,
    // because the snapshot then compares equal across the mutation and the stale mask is
    // returned. It cannot pass vacuously: the fixture asserts the two masks differ, so a
    // scene whose kind change produced no geometric difference fails the guard.
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 20 }, "background": null }),
    );
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 10, "y": 10, "w": 20.0, "h": 20.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let light = entity_doc_eng(
        20,
        10,
        "light",
        json!({
            "x": 10.0, "y": 10.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 5.0, "dimRadius": 8.0, "enabled": true
        }),
    );
    let mut ecs = SceneEcs::from_documents(vec![scene, tok, light], 0);
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": true, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#000000", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));

    let before = ecs.visible_cells_cached(user, scene_id, false);

    // Mutate the scene document's grid kind through apply_op, matching how a real write
    // would reach the ECS.
    ecs.apply_op(&Operation::Update {
        doc_id: scene_id,
        changes: vec![FieldChange {
            path: "/engine/grid/kind".to_string(),
            old: json!("square"),
            new: json!("hex"),
            remove: false,
        }],
    });

    let after = ecs.visible_cells_cached(user, scene_id, false);
    assert_ne!(
        before, after,
        "a grid-kind change must produce a different mask"
    );
}

#[test]
fn lit_mask_suppresses_hint_when_normal_floor_wins_in_bright_cell() {
    use serde_json::json;
    // Combined-token suppression: an owned token whose embedded actor has
    // BOTH normal (floor=dim 0.34) AND darkvision (floor=dark 0.0).  Standing in a brightly-lit
    // cell (light placed at the token), normal's floor (0.34) is higher than darkvision's (0.0),
    // so normal is the highest-admitting mode → its hint (None) wins → lit cells carry no hint.
    let player = Uuid::from_u128(42);
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
            a.engine = Some(actor_body(json!([
                { "mode": "normal",     "range": 0 },
                { "mode": "darkvision", "range": 6 }
            ])));
            a
        }],
    );
    // A bright light at the token location illuminates the cell at (0,0) past the dim threshold.
    let light = entity_doc_eng(
        20,
        10,
        "light",
        json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true
        }),
    );
    let scene_id = Uuid::from_u128(10);
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, light], 0);
    let mask = ecs.player_lit_mask(player);
    let lit_cells: Vec<_> = mask.iter().flat_map(|s| s.cells.iter()).collect();
    assert!(
        !lit_cells.is_empty(),
        "token with normal+darkvision under bright light must see at least one cell"
    );
    // Which cells are BRIGHT is read from the data, not restated: the light sits at the
    // token's own position, so that cell's own illumination band names the brightest band
    // present, and every cell sharing it is a brightly-lit cell.
    let cell = *ecs
        .scene_grid_sizes()
        .get(&scene_id)
        .expect("the fixture's scene has a grid size");
    let token_cell = ecs.resolve_grid_shape(scene_id, cell).cell_of((50.0, 50.0));
    let bright_band = lit_cells
        .iter()
        .find(|(i, j, _, _, _)| (*i, *j) == token_cell)
        .map(|(_, _, b, _, _)| *b)
        .expect("the cell holding both the token and the light is lit");
    // Derived, not merely read back: bands are ordered brightest-first and the fixture's light
    // sits on the token, so the brightest band is index 0. Without this the hint comparison
    // would be output-against-output and a uniform band shift would stay green.
    assert_eq!(
        bright_band, 0,
        "the cell holding the light must resolve to the brightest band"
    );
    // Every brightly-lit cell must carry None: normal's floor (0.34) > darkvision's floor
    // (0.0), so normal is the highest-admitting mode there and its None hint suppresses
    // desaturate.
    assert!(
        lit_cells
            .iter()
            .filter(|(_, _, b, _, _)| *b == bright_band)
            .all(|(_, _, _, _, h)| h.is_none()),
        "normal-floor wins in bright cell: desaturate hint must be suppressed (None)"
    );
    // Hint suppression is a per-cell DECISION, not an inert hint field: the same mask's
    // darker cells — which only darkvision admits, since normal's floor excludes them — do
    // carry the hint. Without this the assertion would hold just as well if hints were never
    // emitted at all.
    assert!(
        lit_cells
            .iter()
            .any(|(_, _, b, _, h)| *b != bright_band && h.is_some()),
        "a darker cell admitted only by darkvision must carry its hint, so the bright cell's \
         None is a suppression rather than an absence"
    );
}

#[test]
fn cell_visible_predicate_honors_floor_and_range() {
    // floors: (floor_min_value, range_cells, render_hint). A normal mode (floor "dim" ~0.34),
    // range 0 = unbounded. Lit level 1.0 ≥ 0.34 → visible; 0.1 < 0.34 → not.
    let normal = vec![(0.34_f64, 0.0_f64, None)];
    assert!(cell_visible(&normal, 1.0, 5.0));
    assert!(!cell_visible(&normal, 0.1, 5.0));
    // Darkvision floor 0.0 within range 6 admits an unlit cell; beyond range it does not.
    let dark = vec![(0.0_f64, 6.0_f64, Some("desaturate".into()))];
    assert!(
        cell_visible(&dark, 0.0, 3.0),
        "unlit but within darkvision range"
    );
    assert!(
        !cell_visible(&dark, 0.0, 9.0),
        "beyond darkvision range, unlit → not visible"
    );
    // No in-range mode → not visible (fail closed).
    assert!(!cell_visible(&[], 1.0, 1.0));
}

/// Builds a SceneEcs with one scene (id 10), one player-owned token at (50, 50), and one
/// enabled white light at (50, 50) with bright=3 / dim=6 cells. The token has normal vision
/// (default), so cells within the lit radius are visible. Returns `(ecs, user, scene_id)`.
fn scene_with_lit_player_token() -> (SceneEcs, Uuid, Uuid) {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let light = entity_doc_eng(
        20,
        10,
        "light",
        json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true
        }),
    );
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, light], 0);
    (ecs, user, scene_id)
}

/// Builds a SceneEcs whose light boundary crosses through cell (1,1), guaranteeing a
/// lenient-only cell. Cell (1,1) has center at (150, 150), distance ≈ 141.4 from the light
/// at (50, 50) — just beyond `dimRadius = 140` (1.4 cells × 100 units/cell), so
/// `cell_illumination` returns 0 for the center and strict rejects it. Corner (100, 100) is
/// at distance ≈ 70.7 < 140 → illuminated → lenient admits it. No sight walls + los_restriction
/// defaults false → the bound-box polygon covers all cells, so the LOS test never rejects a
/// corner. Returns `(ecs, user, scene_id)`.
fn scene_with_boundary_crossing_light() -> (SceneEcs, Uuid, Uuid) {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    // dimRadius = 1.4 cells (140 scene units): center of (1,1) at distance ≈141.4 > 140 (strict miss);
    // corner (100,100) at distance ≈70.7 < 140 (lenient hit).
    let light = entity_doc_eng(
        20,
        10,
        "light",
        json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 0.5, "dimRadius": 1.4, "enabled": true
        }),
    );
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, light], 0);
    (ecs, user, scene_id)
}

#[test]
fn visible_cells_strict_equals_player_lit_mask_cells() {
    // Parity: under strict (center-only) sampling, the movement gate mask must equal the
    // egress secrecy mask for the scene. Both paths use the same cell_visible predicate and
    // lighting_inputs, so any divergence is a sampling or illumination bug.
    let (ecs, user, scene) = scene_with_lit_player_token();
    let strict: std::collections::BTreeSet<(i32, i32)> = ecs.visible_cells(user, scene, false);
    let egress: std::collections::BTreeSet<(i32, i32)> = ecs
        .player_lit_mask(user)
        .into_iter()
        .filter(|s| s.scene == scene)
        .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
        .collect();
    assert_eq!(
        strict, egress,
        "strict gate mask must equal the egress secrecy mask"
    );
    assert!(!strict.is_empty());
}

#[test]
fn a_square_light_reaches_its_authored_bright_radius_past_the_bound_margin() {
    // `scene_with_lit_player_token` authors a 3-cell bright radius at cell size 100 (300
    // world units), with no wall anywhere near the lamp. Cell (2,0), center (250,50), is 200
    // world units from the lamp at (50,50) — inside the bright radius and full intensity, but
    // past the lamp's occlusion-polygon bound margin (100 world units) if that bound never
    // grows past it. A cap at the margin leaves this cell dark regardless of the authored
    // radius; the authored radius, not the margin, must decide it.
    let (ecs, user, scene) = scene_with_lit_player_token();
    let cells = mask_cells(&ecs, user, scene);
    assert!(
        cells.contains(&(2, 0)),
        "a cell 200 world units from a 300-world-unit bright radius must be lit, got {cells:?}"
    );
    let mask = ecs.visible_cells(user, scene, false);
    assert!(
        mask.contains(&(2, 0)),
        "the movement-gate mask agrees with the egress mask"
    );
}

/// `scene_with_lit_player_token`'s lamp/token scene, plus one `blocksLight` wall standing
/// between the lamp at (50,50) and cell (2,0) (center (250,50)) — the same cell
/// `a_square_light_reaches_its_authored_bright_radius_past_the_bound_margin` proves is lit
/// with no wall present. The wall runs the full y-span the light's grown occlusion bound could
/// reach, so a bound that grows to cover the reach but stops respecting occlusion (e.g. degrades
/// to an unoccluded disc) cannot be distinguished from a correctly-occluded one by any other
/// test — this fixture is what catches it.
fn scene_with_lit_player_token_and_occluding_wall() -> (SceneEcs, Uuid, Uuid) {
    let (ecs, user, scene_id) = scene_with_lit_player_token();
    let mut docs: Vec<crate::data::document::Document> = ecs
        .world
        .query::<&SceneEntity>()
        .iter()
        .map(|e| e.doc.clone())
        .collect();
    docs.push(entity_doc_eng(
        30,
        10,
        "wall",
        json!({ "seg": {"x1": 150.0, "y1": -600.0, "x2": 150.0, "y2": 600.0},
                "blocksSight": false, "blocksMove": false, "blocksLight": true }),
    ));
    (SceneEcs::from_documents(docs, 0), user, scene_id)
}

#[test]
fn a_square_light_occludes_behind_a_wall_within_its_grown_reach() {
    // The occlusion polygon's bound must be able to grow to the light's authored reach
    // (proven by the sibling test above) while remaining an occlusion polygon: a
    // `blocksLight` wall between the lamp and a cell inside the authored radius must leave
    // that cell unlit.
    let (ecs, user, scene) = scene_with_lit_player_token_and_occluding_wall();
    let cells = mask_cells(&ecs, user, scene);
    assert!(
        !cells.contains(&(2, 0)),
        "a blocksLight wall between the lamp and cell (2,0) must occlude it, got {cells:?}"
    );
    let mask = ecs.visible_cells(user, scene, false);
    assert!(
        !mask.contains(&(2, 0)),
        "the movement-gate mask agrees with the occluded egress mask"
    );
    // Positive control: the near side of the wall is lit, so the absence of (2,0) above is
    // the wall occluding it, not the light failing to reach anything at all.
    assert!(
        cells.contains(&(0, 0)),
        "the lamp's own cell, on the near side of the wall, must stay lit, got {cells:?}"
    );
}

/// Gate-vs-egress parity helper: asserts `visible_cells(user, scene, false)` == the `(i,j)` set of
/// `player_lit_mask(user)` filtered to `scene`, and that neither set is empty (non-vacuous).
fn assert_strict_parity(ecs: &SceneEcs, user: Uuid, scene: Uuid) {
    let strict: std::collections::BTreeSet<(i32, i32)> = ecs.visible_cells(user, scene, false);
    let egress: std::collections::BTreeSet<(i32, i32)> = ecs
        .player_lit_mask(user)
        .into_iter()
        .filter(|s| s.scene == scene)
        .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
        .collect();
    assert!(
        !strict.is_empty(),
        "parity check must be non-vacuous (strict set empty)"
    );
    assert!(
        !egress.is_empty(),
        "parity check must be non-vacuous (egress set empty)"
    );
    assert_eq!(
        strict, egress,
        "strict gate mask must equal the egress secrecy mask"
    );
}

/// An `environmentLight` scene at env intensity 1.0 with a player-owned normal-vision token
/// at cell (0,0) and a 4-wall box sealing cell (3,3) (center (350,350)). The walls are
/// `blocksSight` (so `bound_for` grows the scan to include the room) but LOS is OFF (so the
/// LOS polygon is the plain bound rectangle — no LOS occlusion, isolating the env-occlusion
/// effect). `blocks_light` toggles whether the box occludes the boundary-projected environment
/// light: `true` seals the interior (env cannot reach), `false` is the no-occlusion baseline.
fn env_lit_scene_with_room(blocks_light: bool) -> (SceneEcs, Uuid, Uuid) {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let wall = |id: u128, x1: f64, y1: f64, x2: f64, y2: f64| {
        entity_doc_eng(
            id,
            10,
            "wall",
            json!({ "seg": {"x1":x1,"y1":y1,"x2":x2,"y2":y2},
                    "blocksSight": true, "blocksMove": false, "blocksLight": blocks_light }),
        )
    };
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                "bounds": { "width": 6.0, "height": 6.0 } }),
    );
    let mut ecs = SceneEcs::from_documents(
        vec![
            scene,
            tok,
            wall(31, 300.0, 300.0, 400.0, 300.0),
            wall(32, 400.0, 300.0, 400.0, 400.0),
            wall(33, 400.0, 400.0, 300.0, 400.0),
            wall(34, 300.0, 400.0, 300.0, 300.0),
        ],
        0,
    );
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "movementModel": "grid-stepped",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    (ecs, user, scene_id)
}

/// The `(i,j)` cell set of `player_lit_mask(user)` restricted to `scene`.
fn mask_cells(
    ecs: &SceneEcs,
    user: Uuid,
    scene: Uuid,
) -> std::collections::BTreeSet<(i32, i32)> {
    ecs.player_lit_mask(user)
        .into_iter()
        .filter(|s| s.scene == scene)
        .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
        .collect()
}

#[test]
fn env_light_occlusion_narrows_the_mask_and_seals_the_interior() {
    // Option B: env light is a genuine (fail-closed) visibility input. A blocksLight-sealed
    // interior stops being visible to a normal-vision player; occlusion only REMOVES cells.
    let (ecs_after, user, scene) = env_lit_scene_with_room(true); // sealed
    let (ecs_before, _, _) = env_lit_scene_with_room(false); // no occlusion baseline
    let after = mask_cells(&ecs_after, user, scene);
    let before = mask_cells(&ecs_before, user, scene);
    let interior = (3, 3); // center (350,350), inside the sealed box
    assert!(
        before.contains(&interior),
        "baseline (blocksLight:false) lights the interior"
    );
    assert!(
        !after.contains(&interior),
        "a blocksLight-sealed interior must drop out of a normal-vision player's mask"
    );
    assert!(
        after.is_subset(&before),
        "env occlusion is strictly narrowing: it only removes cells, never adds any"
    );
    assert!(
        after.contains(&(0, 0)),
        "the open exterior (the token's own cell) stays lit and visible"
    );
}

/// The hex analogue of `env_lit_scene_with_room`: an `environmentLight` scene on a pointy-top
/// hex grid, with a player-owned normal-vision token at hex `(0,0)` and the six edges of hex
/// `HEX_SEALED_CELL` walled off. The seal's segments are derived from `HexGrid::cell_vertices`
/// rather than restated, so the box is exactly that hex's own boundary on both toggles.
///
/// This is the only fixture that reaches `lighting::env_light_polys` on a hex scene: the
/// remaining hex fixtures either disable lighting or route as an unrestricted GM, so no mask
/// is built and the environment-light path never runs on hex without it. The extent that path
/// walks the perimeter of is grid-kind-dependent, and grid kind and movement model are
/// independent axes that combine.
fn hex_env_lit_scene_with_room(blocks_light: bool) -> (SceneEcs, Uuid, Uuid) {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let g = grid_shape::HexGrid {
        size: HEX_FIXTURE_SIZE,
    };
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let verts = grid_shape::GridShape::cell_vertices(&g, HEX_SEALED_CELL, g.size);
    let mut docs = vec![
        entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "hex", "size": g.size }, "background": null,
                    "bounds": { "width": 6.0, "height": 4.0 } }),
        ),
        tok,
    ];
    for (k, a) in verts.iter().enumerate() {
        let b = verts[(k + 1) % verts.len()];
        docs.push(entity_doc_eng(
            31 + k as u128,
            10,
            "wall",
            json!({ "seg": {"x1": a.0, "y1": a.1, "x2": b.0, "y2": b.1},
                    "blocksSight": true, "blocksMove": false, "blocksLight": blocks_light }),
        ));
    }
    let mut ecs = SceneEcs::from_documents(docs, 0);
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "movementModel": "grid-stepped",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    (ecs, user, scene_id)
}

/// The hex whose six edges `hex_env_lit_scene_with_room` seals, and whose drop-out the test
/// asserts. Named once so the fixture and its assertion cannot name different cells.
const HEX_SEALED_CELL: (i32, i32) = (3, 1);

#[test]
fn hex_env_light_occlusion_seals_the_interior_like_the_square_path() {
    // The hex arm of the environment-occlusion property, which no other fixture reaches. It
    // depends on the hex extent: `env_light_polys` walks the perimeter of the rectangle
    // `world_extent` produces, so a wrong hex extent moves every boundary sample and changes
    // which cells the environment reaches.
    // Discrimination: the baseline assertion fails if the sealed hex is never lit in the first
    // place (a vacuous seal), the sealed assertion fails if `blocksLight` stops occluding, and
    // the subset assertion fails if occlusion ever ADDS a cell. Under a mutation of the hex
    // extent formula the perimeter walk changes and this test moves.
    let (ecs_after, user, scene) = hex_env_lit_scene_with_room(true);
    let (ecs_before, _, _) = hex_env_lit_scene_with_room(false);
    let after = mask_cells(&ecs_after, user, scene);
    let before = mask_cells(&ecs_before, user, scene);
    assert!(
        before.contains(&HEX_SEALED_CELL),
        "baseline (blocksLight:false) lights hex {HEX_SEALED_CELL:?}"
    );
    assert!(
        !after.contains(&HEX_SEALED_CELL),
        "a blocksLight-sealed hex must drop out of a normal-vision player's mask"
    );
    assert!(
        after.is_subset(&before),
        "env occlusion is strictly narrowing on hex too: it only removes cells"
    );
    assert!(
        after.contains(&(0, 0)),
        "the open exterior (the token's own hex) stays lit and visible"
    );
}

#[test]
fn hex_env_light_walks_the_blocks_real_origin_side_edges() {
    // What the envelope buys the environment-light perimeter walk: it starts at the block's
    // real boundary — half the flats left of the origin hex's centre and one circumradius
    // below it — rather than at the origin, so the origin side gets boundary samples of its
    // own and the hexes just outside the block there fall inside the raycast bound those
    // samples terminate on. Square has always had that one-cell margin (its own origin cell's
    // corner IS the origin); this is hex reaching the same behaviour.
    //
    // Discrimination: both probes are hexes the ORIGIN-ANCHORED walk cannot reach — their
    // centres sit outside a bound anchored at the origin and expanded by the same margin —
    // and each names a different edge of the block, so a minimum that moved back to the origin
    // on either axis alone fails one of them. The in-block control fails if the walk stops
    // lighting the authored area at all, which would make both probes vacuous.
    let (ecs, user, scene) = hex_env_lit_scene_with_room(true);
    let lit = mask_cells(&ecs, user, scene);
    // The fixture's own geometry, read rather than restated: the margin the raycast bound adds
    // past the envelope is the scene's indexing scale.
    let envelope = ecs.scene_world_extent(scene);
    let g = grid_shape::HexGrid {
        size: HEX_FIXTURE_SIZE,
    };
    for (probe, edge) in [((-1, 0), "left"), ((0, -1), "bottom")] {
        let c = grid_shape::GridShape::cell_center(&g, probe);
        assert!(
            c.0 < 0.0 || c.1 < 0.0,
            "fixture: hex {probe:?}'s centre {c:?} must sit on the block's origin side"
        );
        assert!(
            c.0 < -HEX_FIXTURE_SIZE || c.1 < -HEX_FIXTURE_SIZE,
            "fixture: hex {probe:?}'s centre {c:?} must lie outside a bound anchored at the \
             origin and grown by the {HEX_FIXTURE_SIZE} margin"
        );
        assert!(
            c.0 >= envelope.min.0 - HEX_FIXTURE_SIZE
                && c.1 >= envelope.min.1 - HEX_FIXTURE_SIZE,
            "fixture: hex {probe:?}'s centre {c:?} must lie inside the envelope \
             {envelope:?} grown by that same margin"
        );
        assert!(
            lit.contains(&probe),
            "hex {probe:?}, across the block's real {edge} edge, must be environment-lit"
        );
    }
    assert!(
        lit.contains(&(1, 1)),
        "the authored block's own interior stays lit"
    );
}

/// A wall-less `environmentLight`/`globalIllumination` scene (env 1.0) with a player-owned
/// normal-vision token, used to prove open-scene equivalence: with nothing to occlude, the
/// edge-projected env light reaches every LOS cell exactly as the flat all-bright fill does.
fn open_env_lit_scene(mode: &str) -> (SceneEcs, Uuid, Uuid) {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                "bounds": { "width": 6.0, "height": 6.0 } }),
    );
    let mut ecs = SceneEcs::from_documents(vec![scene, tok], 0);
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": true, "lightMode": mode,
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "movementModel": "grid-stepped",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    (ecs, user, scene_id)
}

#[test]
fn env_light_open_scene_equals_global_illumination_no_holes() {
    // Open-scene equivalence: where there is nothing to occlude, edge-projected env at
    // intensity 1.0 must reach every LOS cell — identical to globalIllumination's all-bright
    // fill. A spurious occlusion hole (e.g. too-sparse boundary sampling) would drop a cell
    // here and break the equality.
    let (env_ecs, user, scene) = open_env_lit_scene("environmentLight");
    let (gi_ecs, _, _) = open_env_lit_scene("globalIllumination");
    let env_mask = mask_cells(&env_ecs, user, scene);
    let gi_mask = mask_cells(&gi_ecs, user, scene);
    assert!(!env_mask.is_empty(), "open env-lit scene is non-empty");
    assert_eq!(
        env_mask, gi_mask,
        "wall-less env=1.0 mask equals the flat all-bright mask (no occlusion holes)"
    );
}

#[test]
fn strict_parity_holds_with_env_light_occlusion() {
    // Gate-vs-egress anti-drift with env occlusion active: the movement gate (visible_cells strict) must
    // still equal the egress secrecy mask (player_lit_mask cells) when a blocksLight-sealed
    // interior narrows both. Both consume the SAME env_polys via the same cell_illumination.
    let (ecs, user, scene) = env_lit_scene_with_room(true);
    assert_strict_parity(&ecs, user, scene);
}

#[test]
fn visible_cells_strict_parity_global_illumination() {
    // Parity under globalIllumination: all LOS cells are all_bright. With no placed lights
    // the all_bright arm fires for both paths, so any divergence would be in the all_bright
    // branch — this guards it.
    use serde_json::json;
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    // lightMode = globalIllumination: lighting_enabled true, all cells bright, env tint applied.
    // No placed lights — confirms the all_bright short-circuit path in both player_lit_mask
    // and visible_cells fires identically.
    let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok], 0);
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": true, "lightMode": "globalIllumination",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "movementModel": "grid-stepped",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    assert_strict_parity(&ecs, user, scene_id);
}

#[test]
fn visible_cells_strict_parity_darkvision() {
    // Parity for a darkvision token in a dark scene (no placed lights, env intensity=0).
    // The darkvision floor (0.0) admits unlit-but-in-range cells; normal vision would see
    // nothing. Both paths must agree on exactly those cells.
    use serde_json::json;
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    // Embedded actor granting darkvision range 6 (mirrors lit_mask_gates_los test pattern).
    tok.embedded.insert(
        "actor".into(),
        vec![{
            let mut a = doc(99, None, "actor");
            a.engine = Some(actor_body(json!([{ "mode": "darkvision", "range": 6 }])));
            a
        }],
    );
    // Dark scene: lighting on, environmentLight, env intensity=0, no lights → only darkvision
    // cells are visible. losRestriction=false keeps the LOS polygon as the full bound box so
    // every in-range unlit cell is admitted — the test is purely about the floor/range gate.
    let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok], 0);
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#000000", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    assert_strict_parity(&ecs, user, scene_id);
}

#[test]
fn visible_cells_strict_parity_los_restriction_with_occluding_wall() {
    // Parity with losRestriction=true and a blocksSight wall that occludes some cells.
    // Both paths use source_los_poly (the shared raycast), so any divergence would be in
    // per-cell sampling AFTER the LOS polygon is built — this guards the occluded-scene path.
    use serde_json::json;
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    // A blocksSight wall at x=200 (column 2) cuts off the right half of the scene from
    // the token at (50,50). Combined with a bright light at the token, cells to the left of
    // the wall are lit+visible; cells beyond the wall are occluded by LOS.
    let wall = entity_doc_eng(
        30,
        10,
        "wall",
        json!({ "seg": { "x1": 200, "y1": -200, "x2": 200, "y2": 400 }, "blocksSight": true }),
    );
    let light = entity_doc_eng(
        20,
        10,
        "light",
        json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 5.0, "dimRadius": 8.0, "enabled": true
        }),
    );
    let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, wall, light], 0);
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": true, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#000000", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    assert_strict_parity(&ecs, user, scene_id);
}

#[test]
fn visible_cells_lenient_is_a_superset_of_strict() {
    // Uses scene_with_boundary_crossing_light: the light boundary (dimRadius 1.4 cells =
    // 140 scene units) cuts through cell (1,1). Center (150,150) is ~141.4 units away →
    // outside dim radius → strict rejects it. Corner (100,100) is ~70.7 units away → inside
    // dim radius → lenient admits it. This guarantees at least one lenient-only cell, so the
    // corner-sampling path is live and proven, not vacuously skipped.
    let (ecs, user, scene) = scene_with_boundary_crossing_light();
    let strict = ecs.visible_cells(user, scene, false);
    let lenient = ecs.visible_cells(user, scene, true);
    // Subset invariant: every strict cell is also in lenient.
    assert!(
        strict.iter().all(|c| lenient.contains(c)),
        "lenient ⊇ strict"
    );
    // Strict superset: lenient must contain at least one cell strict does not.
    assert!(
        lenient.len() > strict.len(),
        "lenient must admit at least one corner-only cell not in strict"
    );
    // Non-empty difference set: the corner path is proven live.
    assert!(
        lenient.difference(&strict).next().is_some(),
        "difference(lenient, strict) must be non-empty"
    );
}

#[test]
fn visible_cells_empty_when_user_has_no_source_in_scene() {
    let (ecs, _user, scene) = scene_with_lit_player_token();
    let stranger = Uuid::from_u128(999);
    assert!(
        ecs.visible_cells(stranger, scene, true).is_empty(),
        "no sources → empty (fail closed)"
    );
}

#[test]
fn movement_gate_mask_cache_invalidates_on_wall_mutation() {
    // `visible_cells_cached` must never serve a cell an occluding wall has since removed from
    // view: a stale cache must fail toward recompute, never toward a wider mask.
    use serde_json::json;
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    // Grid size 20 (not the usual 100) so the target cell sits comfortably inside the
    // no-walls-yet `bound_for` margin box (±100 around the token) rather than on its edge.
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 20 }, "background": null }),
    );
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 10, "y": 10, "w": 20.0, "h": 20.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let light = entity_doc_eng(
        20,
        10,
        "light",
        json!({
            "x": 10.0, "y": 10.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 5.0, "dimRadius": 8.0, "enabled": true
        }),
    );
    let mut ecs = SceneEcs::from_documents(vec![scene, tok, light], 0);
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": true, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#000000", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));

    // Cell (2,0), center (50,10): 40 scene units (2 cells) from the token at (10,10), well
    // within the 160-unit (8-cell) dim radius, and on the token's LOS with no wall present.
    let target_cell = (2, 0);
    let mask1 = ecs.visible_cells_cached(user, scene_id, false);
    assert!(
        mask1.contains(&target_cell),
        "cell visible before a blocksSight wall is added: {mask1:?}"
    );

    // A vertical blocksSight wall at x=30, between the token (x=10) and target_cell's center
    // (x=50) — an ordinary `apply_op` Create, the same path a real `Room::publish` Create op
    // takes.
    let wall = entity_doc_eng(
        30,
        10,
        "wall",
        json!({ "seg": { "x1": 30, "y1": -400, "x2": 30, "y2": 400 }, "blocksSight": true }),
    );
    ecs.apply_op(&Operation::Create { doc: wall });

    let mask2 = ecs.visible_cells_cached(user, scene_id, false);
    assert!(
        !mask2.contains(&target_cell),
        "cache must invalidate on wall mutation, never serve a stale wider mask: {mask2:?}"
    );
}

#[test]
fn movement_gate_mask_cache_reused_across_repeated_moves_with_no_scene_change() {
    let (ecs, user, scene) = scene_with_lit_player_token();

    let mask1 = ecs.visible_cells_cached(user, scene, false);
    assert_eq!(
        ecs.visible_cells_recompute_count(),
        1,
        "first call is always a recompute (cold cache)"
    );

    let mask2 = ecs.visible_cells_cached(user, scene, false);
    assert_eq!(mask1, mask2);
    assert_eq!(
        ecs.visible_cells_recompute_count(),
        1,
        "a repeated call with no input change must be served from the cache, not recomputed"
    );

    // Sanity: `visible_cells` (the uncached primitive `visible_cells_cached` wraps) agrees.
    assert_eq!(mask1, ecs.visible_cells(user, scene, false));
}

/// Build an ECS with one `blocksMove` wall and one non-blocking wall in the same scene.
/// The blocking wall runs from (100,0) to (100,200); the non-blocking wall runs elsewhere.
fn scene_with_two_walls_one_blocking() -> (SceneEcs, Uuid) {
    let scene = Uuid::from_u128(10);
    let blocking =
        json!({ "seg": {"x1": 100, "y1": 0, "x2": 100, "y2": 200}, "blocksMove": true });
    let non_blocking =
        json!({ "seg": {"x1": 0, "y1": 0, "x2": 50, "y2": 50}, "blocksMove": false });
    let ecs = SceneEcs::from_documents(
        vec![
            doc(10, None, "scene"),
            entity_doc_eng(11, 10, "wall", blocking),
            entity_doc_eng(12, 10, "wall", non_blocking),
        ],
        0,
    );
    (ecs, scene)
}

#[test]
fn region_field_authoritative_includes_secret_regions_visible_excludes_them() {
    use crate::data::document::Visibility;
    let scene_id = Uuid::from_u128(10);
    let owner = Uuid::from_u128(1);
    let player = Uuid::from_u128(2);

    let mut secret = crate::data::document::tests::world_scoped_doc(
        Uuid::from_u128(9),
        Uuid::from_u128(20),
        "region",
    );
    secret.parent_id = Some(scene_id);
    secret.owner = Some(owner);
    secret.engine = Some(serde_json::json!({
        "shape": { "kind": "rect", "points": [0.0, 0.0, 100.0, 100.0] },
        "behavior": "impassable",
        "cost": 1.0,
        "enabled": true,
    }));
    secret
        .permissions
        .property_overrides
        .insert("/engine".into(), Visibility::GmOnly);

    let mut visible = crate::data::document::tests::world_scoped_doc(
        Uuid::from_u128(9),
        Uuid::from_u128(21),
        "region",
    );
    visible.parent_id = Some(scene_id);
    visible.engine = Some(serde_json::json!({
        "shape": { "kind": "rect", "points": [200.0, 0.0, 300.0, 100.0] },
        "behavior": "terrain",
        "cost": 2.0,
        "enabled": true,
    }));

    let ecs = SceneEcs::from_documents(
        vec![
            crate::data::document::tests::world_scoped_doc(
                Uuid::from_u128(9),
                scene_id,
                "scene",
            ),
            secret,
            visible,
        ],
        0,
    );

    let authoritative = ecs.region_field(scene_id, None).expect("scene exists");
    assert!(
        authoritative.is_impassable((0, 0)),
        "authoritative field includes the secret region"
    );
    assert_eq!(authoritative.terrain_multiplier((2, 0)), 2.0);

    let player_field = ecs
        .region_field(scene_id, Some(player))
        .expect("scene exists");
    assert!(
        !player_field.is_impassable((0, 0)),
        "secret region absent from a non-owner player's field"
    );
    assert_eq!(
        player_field.terrain_multiplier((2, 0)),
        2.0,
        "the region whose tier this player can see weights their field"
    );
}

#[test]
fn region_field_ignores_disabled_regions() {
    let scene_id = Uuid::from_u128(10);
    let mut disabled = crate::data::document::tests::world_scoped_doc(
        Uuid::from_u128(9),
        Uuid::from_u128(20),
        "region",
    );
    disabled.parent_id = Some(scene_id);
    disabled.engine = Some(serde_json::json!({
        "shape": { "kind": "rect", "points": [0.0, 0.0, 100.0, 100.0] },
        "behavior": "impassable",
        "cost": 1.0,
        "enabled": false,
    }));
    let ecs = SceneEcs::from_documents(
        vec![
            crate::data::document::tests::world_scoped_doc(
                Uuid::from_u128(9),
                scene_id,
                "scene",
            ),
            disabled,
        ],
        0,
    );
    assert!(!ecs
        .region_field(scene_id, None)
        .expect("scene exists")
        .is_impassable((0, 0)));
}

#[test]
fn move_walls_returns_only_blocks_move_segments_for_the_scene() {
    // A scene with one blocksMove wall and one non-blocksMove wall yields exactly the blocking segment.
    let (ecs, scene) = scene_with_two_walls_one_blocking();
    let walls = ecs.move_walls(scene, None);
    assert_eq!(walls.len(), 1, "only the blocksMove wall is returned");
    let w = walls[0];
    assert_eq!((w.a, w.b), ((100.0, 0.0), (100.0, 200.0)));
}

/// A scene with grid size `cell` and no walls.
fn scene_with_grid(cell: f64) -> (SceneEcs, Uuid) {
    let scene_id = Uuid::from_u128(10);
    let ecs = SceneEcs::from_documents(
        vec![entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": cell }, "background": null }),
        )],
        0,
    );
    (ecs, scene_id)
}

/// A `wall` doc parented to `scene`, blocksMove+blocksSight+blocksLight all true.
fn wall_doc_eng(scene: Uuid, a: (f64, f64), b: (f64, f64)) -> Document {
    let mut d = crate::data::document::tests::world_scoped_doc(
        Uuid::from_u128(9),
        Uuid::new_v4(),
        "wall",
    );
    d.parent_id = Some(scene);
    d.engine = Some(json!({
        "seg": { "x1": a.0, "y1": a.1, "x2": b.0, "y2": b.1 },
        "blocksMove": true,
        "blocksSight": true,
        "blocksLight": true,
    }));
    d
}

/// One public blocksMove wall at x=100 and one `gm_only` blocksMove wall at x=150.
/// Both also carry blocksSight+blocksLight so
/// `vision_and_lighting_keep_a_gm_only_wall_that_routing_drops` can observe them in the
/// vision sets.
fn scene_with_public_and_secret_move_walls() -> (SceneEcs, Uuid, Uuid) {
    let (mut ecs, scene) = scene_with_grid(100.0);
    let player = Uuid::new_v4();
    let public = wall_doc_eng(scene, (100.0, 0.0), (100.0, 200.0));
    let mut secret = wall_doc_eng(scene, (150.0, 0.0), (150.0, 200.0));
    secret
        .permissions
        .property_overrides
        .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
    ecs.apply_op(&Operation::Create { doc: public });
    ecs.apply_op(&Operation::Create { doc: secret });
    (ecs, scene, player)
}

/// One wall with blocksSight:false, blocksMove:true, default permissions.
fn scene_with_invisible_barrier_wall() -> (SceneEcs, Uuid, Uuid) {
    let (mut ecs, scene) = scene_with_grid(100.0);
    let player = Uuid::new_v4();
    let mut barrier = wall_doc_eng(scene, (100.0, 0.0), (100.0, 200.0));
    barrier.engine = Some(json!({
        "seg": { "x1": 100.0, "y1": 0.0, "x2": 100.0, "y2": 200.0 },
        "blocksMove": true,
        "blocksSight": false,
        "blocksLight": true,
    }));
    ecs.apply_op(&Operation::Create { doc: barrier });
    (ecs, scene, player)
}

#[test]
fn move_walls_omits_a_gm_only_wall_for_a_player_viewer() {
    let (ecs, scene, player) = scene_with_public_and_secret_move_walls();
    assert_eq!(
        ecs.move_walls(scene, None).len(),
        2,
        "authoritative view carries every blocksMove wall"
    );
    let visible = ecs.move_walls(scene, Some(player));
    assert_eq!(
        visible.len(),
        1,
        "a gm_only wall is omitted from a player's routing set"
    );
    assert_eq!(
        (visible[0].a, visible[0].b),
        ((100.0, 0.0), (100.0, 200.0)),
        "the surviving wall is the public one"
    );
}

#[test]
fn move_walls_keeps_a_blocks_sight_false_wall_for_a_player() {
    // An invisible BARRIER (blocksSight:false, blocksMove:true) is a PUBLIC document: the router
    // must honor it. Only document-level secrecy filters — the two kinds are not the same axis.
    let (ecs, scene, player) = scene_with_invisible_barrier_wall();
    assert_eq!(
        ecs.move_walls(scene, Some(player)).len(),
        1,
        "a blocksSight:false wall is public geometry and stays in the player's routing set"
    );
}

/// Anti-drift: vision and lighting keep the FULL wall set; only routing filters. This is a
/// must-NOT-converge constraint, so it gets a test rather than only a doc comment.
#[test]
fn vision_and_lighting_keep_a_gm_only_wall_that_routing_drops() {
    let (ecs, scene, player) = scene_with_public_and_secret_move_walls();
    assert_eq!(
        ecs.sight_walls(scene).len(),
        2,
        "sight_walls keeps the gm_only wall"
    );
    assert_eq!(
        ecs.light_walls(scene).len(),
        2,
        "light_walls keeps the gm_only wall"
    );
    assert_eq!(
        ecs.move_walls(scene, Some(player)).len(),
        1,
        "only the ROUTING set filters per requester"
    );
}

/// Anti-drift pin for `engine_tier_visible`, the single symbol `move_walls` and
/// `region_field` both call for the `/engine`-tier per-requester decision. A re-inline at
/// either call site, or a semantic change to the predicate itself, fails this test.
#[test]
fn engine_tier_visible_admits_authoritative_and_rejects_a_non_owner_player_on_gm_only() {
    let mut doc = crate::data::document::tests::world_scoped_doc(
        Uuid::from_u128(9),
        Uuid::from_u128(50),
        "wall",
    );
    doc.permissions
        .property_overrides
        .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
    let player = Uuid::from_u128(99);
    assert!(
        engine_tier_visible(&doc, None),
        "the authoritative (None) viewer sees a gm_only doc"
    );
    assert!(
        !engine_tier_visible(&doc, Some(player)),
        "a non-owner player is rejected from a gm_only doc"
    );
}

#[test]
fn absent_scene_yields_empty_visible_cells_not_a_synthesized_grid() {
    let (ecs, user, _scene) = scene_with_lit_player_token();
    let ghost_scene = Uuid::from_u128(0xDEAD);
    assert!(ecs.visible_cells(user, ghost_scene, false).is_empty());
    assert!(ecs.visible_cells(user, ghost_scene, true).is_empty());
    assert!(ecs
        .visible_cells_cached(user, ghost_scene, false)
        .is_empty());
}

#[test]
fn absent_scene_region_field_is_none() {
    let (ecs, _user, _scene) = scene_with_lit_player_token();
    assert!(ecs.region_field(Uuid::from_u128(0xDEAD), None).is_none());
}

#[test]
fn absent_scene_navmesh_for_is_none() {
    let (ecs, _user, _scene) = scene_with_lit_player_token();
    assert!(ecs.navmesh_for(Uuid::from_u128(0xDEAD), 0.5, &[]).is_none());
}

#[test]
fn navmesh_for_is_memoized_across_calls() {
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
    let scene = Uuid::from_u128(10);
    let a = ecs.navmesh_for(scene, 0.4, &[]).expect("navmesh builds");
    let b = ecs.navmesh_for(scene, 0.4, &[]).expect("navmesh builds");
    assert!(
        std::sync::Arc::ptr_eq(&a, &b),
        "same (scene, radius, walls) must return the SAME cached Arc, not rebuild"
    );
}

#[test]
fn navmesh_for_distinguishes_footprint_radii() {
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
    let scene = Uuid::from_u128(10);
    let a = ecs.navmesh_for(scene, 0.4, &[]).expect("navmesh builds");
    let b = ecs.navmesh_for(scene, 0.9, &[]).expect("navmesh builds");
    assert!(
        !std::sync::Arc::ptr_eq(&a, &b),
        "distinct footprint radii must get distinct cached meshes"
    );
}

#[test]
fn navmesh_for_rejects_degenerate_radius_even_after_cache_primed_at_zero() {
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
    let scene = Uuid::from_u128(10);
    // Prime the cache at footprint_radius_cells == 0.0: quantized key (scene, 0, []).
    let primed = ecs.navmesh_for(scene, 0.0, &[]);
    assert!(
        primed.is_some(),
        "radius 0.0 must build and cache successfully"
    );

    // f64 as i64 saturates NaN to 0, colliding with the radius-0.0 key primed by this
    // test's first call. Without an
    // upfront validation guard this would return the CACHED radius-0.0 mesh instead of
    // failing closed.
    assert!(
        ecs.navmesh_for(scene, f64::NAN, &[]).is_none(),
        "NaN footprint radius must fail closed, not reuse the cached radius-0.0 mesh"
    );

    // A small negative rounds to -0 under `(x * 1000.0).round() as i64`, which also casts
    // to the same colliding key.
    assert!(
        ecs.navmesh_for(scene, -0.0001, &[]).is_none(),
        "negative footprint radius must fail closed, not reuse the cached radius-0.0 mesh"
    );
}

#[test]
fn navmesh_for_does_not_share_a_mesh_across_differing_wall_sets() {
    let (ecs, scene, player) = scene_with_public_and_secret_move_walls();
    let gm_walls = ecs.move_walls(scene, None);
    let player_walls = ecs.move_walls(scene, Some(player));
    let gm_mesh = ecs
        .navmesh_for(scene, 0.4, &gm_walls)
        .expect("gm mesh builds");
    let player_mesh = ecs
        .navmesh_for(scene, 0.4, &player_walls)
        .expect("player mesh builds");
    assert!(
        !std::sync::Arc::ptr_eq(&gm_mesh, &player_mesh),
        "a differing wall set must not be served a mesh built from another set"
    );
}

#[test]
fn navmesh_for_shares_a_mesh_across_identical_wall_sets() {
    let (ecs, scene, _player) = scene_with_public_and_secret_move_walls();
    let walls = ecs.move_walls(scene, None);
    let a = ecs.navmesh_for(scene, 0.4, &walls).expect("first build");
    let b = ecs.navmesh_for(scene, 0.4, &walls).expect("second build");
    assert!(
        std::sync::Arc::ptr_eq(&a, &b),
        "an identical wall set reuses the memoized mesh"
    );
}

#[test]
fn navmesh_for_wall_key_is_order_independent() {
    // `hecs` iteration order is not stable, so the same set produced in a different order must
    // still hit the cache.
    let (ecs, scene, _player) = scene_with_public_and_secret_move_walls();
    let walls = ecs.move_walls(scene, None);
    let mut reversed = walls.clone();
    reversed.reverse();
    let a = ecs.navmesh_for(scene, 0.4, &walls).expect("first build");
    let b = ecs
        .navmesh_for(scene, 0.4, &reversed)
        .expect("reordered lookup");
    assert!(
        std::sync::Arc::ptr_eq(&a, &b),
        "wall-set key is order-independent"
    );
}

#[test]
fn wall_mutation_invalidates_the_navmesh_cache() {
    let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
    let scene = Uuid::from_u128(10);
    let walls = ecs.move_walls(scene, None);
    let a = ecs.navmesh_for(scene, 0.4, &walls).expect("navmesh builds");
    ecs.apply_op(&Operation::Create {
        doc: entity_doc_eng(
            20,
            10,
            "wall",
            json!({ "seg": { "x1": 10.0, "y1": 0.0, "x2": 10.0, "y2": 50.0 },
                    "blocksMove": true, "blocksSight": false, "blocksLight": false }),
        ),
    });
    let walls = ecs.move_walls(scene, None);
    let b = ecs
        .navmesh_for(scene, 0.4, &walls)
        .expect("navmesh rebuilds");
    assert!(
        !std::sync::Arc::ptr_eq(&a, &b),
        "adding a blocksMove wall must invalidate the cached navmesh"
    );
}

#[test]
fn bounds_mutation_invalidates_the_navmesh_cache() {
    let mut ecs = SceneEcs::from_documents(
        vec![entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
        )],
        0,
    );
    let scene = Uuid::from_u128(10);
    let a = ecs.navmesh_for(scene, 0.4, &[]).expect("navmesh builds");
    ecs.apply_op(&Operation::Update {
        doc_id: scene,
        changes: vec![crate::data::command::FieldChange {
            remove: false,
            path: "/engine/bounds".into(),
            old: json!(null),
            new: json!({ "width": 40, "height": 40 }),
        }],
    });
    let b = ecs.navmesh_for(scene, 0.4, &[]).expect("navmesh rebuilds");
    assert!(
        !std::sync::Arc::ptr_eq(&a, &b),
        "changing scene bounds must invalidate the cached navmesh"
    );
}

/// Builds a SceneEcs with one scene (id 10), one player-owned token at (50, 50), and
/// world-settings that set `movementRestriction = "revealed"` with no placed lights (env
/// intensity = 0). The visible mask is therefore empty; only explored memory can admit cells.
/// Returns `(ecs, user, scene_id)`.
fn scene_revealed_player_token() -> (SceneEcs, Uuid, Uuid) {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok], 0);
    // Dark scene + revealed restriction: visible cells = ∅, so only explored memory admits moves.
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#000000", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "revealed",
            "movementModel": "grid-stepped",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    (ecs, user, scene_id)
}

#[test]
fn pathfind_gm_unconstrained_routes_without_a_mask() {
    // GM (is_gm=true): no mask; an open scene routes start→goal at chebyshev cost.
    let (ecs, _user, scene) = scene_with_lit_player_token();
    let r = ecs.pathfind(
        RouteRequester {
            user: Uuid::from_u128(1),
            is_gm: true,
            explored: None,
        },
        scene,
        (50.0, 50.0),
        &[(250.0, 50.0)],
        0.1,
    );
    let outcome = r.expect("GM route");
    assert!((outcome.cost - 2.0).abs() < 1e-9);
    assert_eq!(outcome.path.last(), Some(&(250.0, 50.0)));
}

#[test]
fn pathfind_dispatches_to_the_navmesh_router_for_a_continuous_scene() {
    let mut ecs = SceneEcs::from_documents(
        vec![entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "vision": { "movementModel": "continuous" } }),
        )],
        0,
    );
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "movementModel": "continuous",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    let outcome = ecs
        .pathfind(
            RouteRequester {
user: Uuid::from_u128(1),
is_gm: true,
explored: // GM: unrestricted mask
            None,
},
            Uuid::from_u128(10),
            (50.0, 50.0),
            &[(950.0, 50.0)],
            0.1,
        )
        .expect("continuous route over an open bounded scene");
    // Euclidean straight line ≈ 900 scene units, unlike a grid diagonal-rule cost — proves
    // the navmesh path was actually taken, not the grid router — converted to the wire's cell
    // unit by dividing through the fixture's cell size (900 / 100 = 9); the tolerance is a
    // 2.0 world-unit slack under that same conversion (2.0 / 100 = 0.02).
    assert!(
        (outcome.cost - 9.0).abs() < 0.02,
        "expected ~9 cells (900 Euclidean scene units / cell 100), got {}",
        outcome.cost
    );
}

#[test]
fn pathfind_grid_and_continuous_report_the_same_cell_cost_for_a_straight_route() {
    // Anti-drift witness for the `pathfind` boundary conversion this task installs: the wire
    // contract (`ws::protocol`'s `PathResult` doc comment) declares ONE unit, cells, for
    // BOTH movement models. A straight horizontal route has an identical Chebyshev and
    // Euclidean length, so the two engines' cell costs for the SAME route geometry must agree
    // exactly regardless of which one ran — a future re-fork of either conversion (the
    // weighted branch reintroducing a `* world_units_per_cell` multiply, or the pure-polyanya
    // branch losing its boundary division) breaks this equality.
    let grid_ecs = SceneEcs::from_documents(
        vec![entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
        )],
        0,
    );
    let mut continuous_ecs = SceneEcs::from_documents(continuous_scene_docs(), 0);
    continuous_ecs.set_world_settings_for_test(continuous_world_settings());

    let requester = || RouteRequester {
        user: Uuid::from_u128(1),
        is_gm: true,
        explored: None,
    };
    let start = (50.0, 50.0);
    let goal = (550.0, 50.0);

    let grid_out = grid_ecs
        .pathfind(requester(), Uuid::from_u128(10), start, &[goal], 0.1)
        .expect("grid-stepped straight route");
    let continuous_out = continuous_ecs
        .pathfind(requester(), Uuid::from_u128(10), start, &[goal], 0.1)
        .expect("continuous straight route");

    assert!(
        (grid_out.cost - 5.0).abs() < 1e-9,
        "grid-stepped: 5 orthogonal cells, got {}",
        grid_out.cost
    );
    assert!(
        (continuous_out.cost - 5.0).abs() < 0.05,
        "continuous: 500 Euclidean scene units / cell 100 = 5 cells, got {}",
        continuous_out.cost
    );
    assert!(
        (grid_out.cost - continuous_out.cost).abs() < 0.05,
        "both engines must report the SAME cell cost for identical straight-route geometry: \
         grid={}, continuous={}",
        grid_out.cost,
        continuous_out.cost
    );
}

#[test]
fn pathfind_continuous_start_equals_goal_is_a_single_point_zero_cost() {
    // Mirrors `astar_tests::start_equals_goal_is_a_single_cell_zero_cost` (the grid-stepped
    // engine's trivial-success case) for the continuous engine: routing to the point you're
    // already standing on must succeed with a single-point, zero-cost route, not
    // `PathFail::Unreachable`.
    let mut ecs = SceneEcs::from_documents(
        vec![entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "vision": { "movementModel": "continuous" } }),
        )],
        0,
    );
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "movementModel": "continuous",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    let outcome = ecs
        .pathfind(
            RouteRequester {
user: Uuid::from_u128(1),
is_gm: true,
explored: // GM: unrestricted mask
            None,
},
            Uuid::from_u128(10),
            (50.0, 50.0),
            &[(50.0, 50.0)],
            0.1,
        )
        .expect("start == goal must succeed, not Unreachable");
    assert_eq!(outcome.path, vec![(50.0, 50.0)]);
    assert_eq!(outcome.cost, 0.0);
    assert!(!outcome.arrested);
}

/// Mirrors `scene_with_lit_player_token` (same token/light geometry) but the scene doc
/// declares `vision.movementModel: "continuous"`, so the fixture drives the REAL non-GM
/// `visible_cells` mask through the continuous dispatch branch instead of a hand-built
/// `BTreeSet` test double.
fn scene_with_lit_player_token_continuous() -> (SceneEcs, Uuid, Uuid) {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let light = entity_doc_eng(
        20,
        10,
        "light",
        json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true
        }),
    );
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                "vision": { "movementModel": "continuous" } }),
    );
    let ecs = SceneEcs::from_documents(vec![scene, tok, light], 0);
    (ecs, user, scene_id)
}

fn continuous_scene_docs() -> Vec<crate::data::document::Document> {
    vec![entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                "vision": { "movementModel": "continuous" } }),
    )]
}

fn continuous_world_settings() -> serde_json::Value {
    json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "movementModel": "continuous",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    })
}

/// A rect region's corners in scene units, ordered as the `points` array the
/// `"rect"` shape expects.
struct RegionRect {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

fn region_doc_top(
    id: u128,
    parent: u128,
    behavior: &str,
    cost: f64,
    rect: RegionRect,
) -> Document {
    entity_doc_eng(
        id,
        parent,
        "region",
        json!({ "shape": { "kind": "rect", "points": [rect.x0, rect.y0, rect.x1, rect.y1] },
                "behavior": behavior, "cost": cost, "enabled": true }),
    )
}

#[test]
fn pathfind_continuous_terrain_bends_the_route_and_costs_cells() {
    // Continuous scene, terrain mult 5 on cell (1,0) = Rect [100,0]-[200,100] between start and
    // goal. The weighted grid route (forced Euclidean) detours through row 1 (2 diagonal steps,
    // ~2*sqrt(2) cells) instead of straight through the mult-5 cell (would be 1+5 = 6 cells).
    // Proves terrain BENDS the continuous route and that the weighted sub-path's cost is
    // already in cells — `pathfinding::find`'s own unit, matching `PathResult`'s wire contract
    // (`ws::protocol`) with no conversion needed.
    let mut docs = continuous_scene_docs();
    docs.push(region_doc_top(
        12,
        10,
        "terrain",
        5.0,
        RegionRect {
            x0: 100.0,
            y0: 0.0,
            x1: 200.0,
            y1: 100.0,
        },
    ));
    let mut ecs = SceneEcs::from_documents(docs, 0);
    ecs.set_world_settings_for_test(continuous_world_settings());
    let out = ecs
        .pathfind(
            RouteRequester {
                user: Uuid::from_u128(1),
                is_gm: true,
                explored: None,
            },
            Uuid::from_u128(10),
            (50.0, 50.0),
            &[(250.0, 50.0)],
            0.1,
        )
        .expect("weighted continuous route");
    // Tight pin (not a loose range): the forced-Euclidean detour is exactly 2 diagonal steps
    // (each √2 cells) around the mult-5 cell, so the cost is 2·√2 ≈ 2.828 cells. A loose bound
    // here would silently pass a regression to the world diagonal rule (Chebyshev diagonals
    // cost 1 → 2 cells) — that reversion is precisely the forced-Euclidean gap this pin
    // guards, so the expected value must be the Euclidean one, epsilon-tight. Tolerance is the
    // pre-conversion 0.5-scene-unit bound divided through the fixture's cell size (100).
    let expected = 2.0 * std::f64::consts::SQRT_2;
    assert!(
        (out.cost - expected).abs() < 0.005,
        "forced-Euclidean detour cost is 2·√2 ≈ {expected:.3} cells, got {}",
        out.cost
    );
    assert!(
        out.path.iter().any(|p| p.1 > 90.0),
        "route bends off the y=50 line to avoid the terrain: {:?}",
        out.path
    );
}

/// Hex + continuous: grid kind and movement model are INDEPENDENT axes, so this scene is
/// reachable through the ordinary authoring path (`resolve_grid_shape` keys only on
/// `grid.kind`; `pathfind` dispatches only on `movement_model`).
fn hex_continuous_scene_docs() -> Vec<crate::data::document::Document> {
    vec![entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "hex", "size": HEX_FIXTURE_SIZE }, "background": null,
                "vision": { "movementModel": "continuous" } }),
    )]
}

#[test]
fn pathfind_hex_continuous_arrest_truncates_at_the_axial_hex_not_the_square_cell() {
    // Call-site wiring proof for `navmesh::truncate_at_arrest`: `pathfind` must hand the
    // continuous engine the SAME `resolve_grid_shape`-derived shape `region_field` rasterized
    // the arrest region with. Arrest-only ⇒ `has_terrain_or_impassable()` is false ⇒ the pure
    // polyanya branch. Route runs along the r=1 hex row from hex (0,1) to hex (4,1); the
    // arrest region covers ONLY hex (3,1). Reading the same axial key (3,1) as a SQUARE cell
    // would place it at `[3·size, 4·size)` — a different location, short of the hex — cutting
    // the preview roughly a full hex early.
    //
    // The region rect is the arrest hex's own centre padded by half a size on each axis, so it
    // moves with the shape rather than having to be re-derived by hand; the pad stays well
    // inside the hex's inradius (`√3/2·size`), which is what keeps exactly one centre inside
    // it, and the neighbour loop asserts that rather than leaving it to the pad's arithmetic.
    let g = grid_shape::HexGrid {
        size: HEX_FIXTURE_SIZE,
    };
    let arrest_cell = (3, 1);
    let arrest_ctr = g.cell_center(arrest_cell);
    let pad = g.size / 2.0;
    let mut docs = hex_continuous_scene_docs();
    docs.push(region_doc_top(
        12,
        10,
        "arrest",
        1.0,
        RegionRect {
            x0: arrest_ctr.0 - pad,
            y0: arrest_ctr.1 - pad,
            x1: arrest_ctr.0 + pad,
            y1: arrest_ctr.1 + pad,
        },
    ));
    let mut ecs = SceneEcs::from_documents(docs, 0);
    ecs.set_world_settings_for_test(continuous_world_settings());
    // Fixture guard: exactly one hex arrests, and it is the axial cell the assertions name.
    // The truncation assertions are only about the arrest hex's own boundary while no
    // neighbour arrests too, so the whole ring is checked rather than the two cells the route
    // happens to pass through.
    let field = ecs
        .region_field(Uuid::from_u128(10), None)
        .expect("scene exists");
    assert!(
        field.is_arrest(arrest_cell),
        "fixture: arrest is on axial {arrest_cell:?}"
    );
    for (n, _, _) in g.neighbors_with_cost(arrest_cell, 0) {
        assert!(
            !field.is_arrest(n),
            "fixture: hex {n:?} neighbours the arrest hex and must stay clear"
        );
    }

    let out = ecs
        .pathfind(
            RouteRequester {
                user: Uuid::from_u128(1),
                is_gm: true,
                explored: None,
            },
            Uuid::from_u128(10),
            g.cell_center((0, 1)),
            &[g.cell_center((4, 1))],
            0.1,
        )
        .expect("hex continuous route");
    assert!(out.arrested, "the arrest hex truncates the preview");
    let last = *out.path.last().unwrap();
    assert_eq!(
        g.cell_of(last),
        arrest_cell,
        "truncation lands on the arrest hex itself, last = {last:?}"
    );
    // Arrest stops AT ENTRY, so the cut sits in the near half of the arrest hex rather than
    // anywhere inside it — the only claim about `last`'s position that the landing-cell
    // assertion does not already imply, since `cell_of` is nearest-centre and therefore
    // already bounds `last` to that hex's own span. Both bounds come from `cell_center`, so a change
    // to the fixture size relocates them with the hex instead of leaving a threshold a
    // truncation one hex early would still satisfy.
    assert!(
        last.0 < arrest_ctr.0,
        "truncation is at the arrest hex's ENTRY boundary, not past its centre \
         ({}), last x = {}",
        arrest_ctr.0,
        last.0
    );
}

#[test]
fn pathfind_continuous_no_region_is_a_straight_polyanya_route() {
    // Same scene WITHOUT a region: the pure polyanya path is taken — a straight 200px route,
    // 200 Euclidean scene units / cell(100) = 2 cells at the `pathfind` boundary conversion.
    let mut ecs = SceneEcs::from_documents(continuous_scene_docs(), 0);
    ecs.set_world_settings_for_test(continuous_world_settings());
    let out = ecs
        .pathfind(
            RouteRequester {
                user: Uuid::from_u128(1),
                is_gm: true,
                explored: None,
            },
            Uuid::from_u128(10),
            (50.0, 50.0),
            &[(250.0, 50.0)],
            0.1,
        )
        .expect("polyanya route");
    // Tolerance is the pre-conversion 3.0-scene-unit bound divided through the fixture's
    // cell size (100).
    assert!(
        (out.cost - 2.0).abs() < 0.03,
        "straight Euclidean ~2 cells (200 scene units / cell 100), got {}",
        out.cost
    );
}

#[test]
fn pathfind_continuous_impassable_routes_around() {
    // Impassable wall-of-cells on column 1 (Rect [100,0]-[200,300]) blocks the straight line;
    // the weighted route must detour and still reach the goal.
    let mut docs = continuous_scene_docs();
    docs.push(region_doc_top(
        12,
        10,
        "impassable",
        1.0,
        RegionRect {
            x0: 100.0,
            y0: 0.0,
            x1: 200.0,
            y1: 300.0,
        },
    ));
    let mut ecs = SceneEcs::from_documents(docs, 0);
    ecs.set_world_settings_for_test(continuous_world_settings());
    let out = ecs
        .pathfind(
            RouteRequester {
                user: Uuid::from_u128(1),
                is_gm: true,
                explored: None,
            },
            Uuid::from_u128(10),
            (50.0, 50.0),
            &[(250.0, 350.0)],
            0.1,
        )
        .expect("route around impassable");
    // No route point falls inside an impassable cell (column 1, y in [0,300)).
    assert!(
        !out.path
            .iter()
            .any(|p| p.0 >= 100.0 && p.0 < 200.0 && p.1 >= 0.0 && p.1 < 300.0),
        "route threads no impassable cell: {:?}",
        out.path
    );
}

#[test]
fn pathfind_continuous_secret_terrain_absent_from_player_route_present_for_gm() {
    // gm_only terrain (mult 5) on cell (1,0). A player (non-GM) never sees it: their route is
    // the straight polyanya line (no bend, ~200 scene units = 2 cells). The GM's route bends
    // (weighted).
    let mut docs = continuous_scene_docs();
    let mut secret = region_doc_top(
        12,
        10,
        "terrain",
        5.0,
        RegionRect {
            x0: 100.0,
            y0: 0.0,
            x1: 200.0,
            y1: 100.0,
        },
    );
    // Mark the region gm_only via the SAME `/engine` property-visibility override
    // `region_field`'s per-requester filter checks
    // (`move_exec::authoritative_field_springs_a_secret_region_a_player_was_routed_through`
    // uses the identical convention for its own gm_only region fixture).
    secret
        .permissions
        .property_overrides
        .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
    docs.push(secret);
    let mut ecs = SceneEcs::from_documents(docs, 0);
    ecs.set_world_settings_for_test(continuous_world_settings());
    let player = Uuid::from_u128(2);
    // Player (non-GM, unrestricted movement => no mask): secret terrain absent => straight route.
    let p = ecs
        .pathfind(
            RouteRequester {
                user: player,
                is_gm: false,
                explored: None,
            },
            Uuid::from_u128(10),
            (50.0, 50.0),
            &[(250.0, 50.0)],
            0.1,
        )
        .expect("player route");
    // Pure-polyanya sub-path: 200 scene units / cell(100) = 2 cells. Tolerance is the
    // pre-conversion 5.0-scene-unit bound divided through the same cell size.
    assert!(
        (p.cost - 2.0).abs() < 0.05,
        "secret terrain does not bend the player route, got {}",
        p.cost
    );
    // GM sees the authoritative field => bends.
    let g = ecs
        .pathfind(
            RouteRequester {
                user: Uuid::from_u128(1),
                is_gm: true,
                explored: None,
            },
            Uuid::from_u128(10),
            (50.0, 50.0),
            &[(250.0, 50.0)],
            0.1,
        )
        .expect("gm route");
    // Weighted sub-path: `pathfinding::find`'s cost is already in cells, no conversion — the
    // pre-conversion 150.0..400.0-scene-unit range divided through the fixture's cell size
    // (100).
    assert!(
        g.cost < 4.0 && g.cost > 1.5,
        "GM route is weighted, got {}",
        g.cost
    );
}

#[test]
fn pathfind_continuous_nongm_route_clips_to_the_visible_mask() {
    // System-level gate-vs-router coverage: the two existing continuous `pathfind` tests
    // (`pathfind_dispatches_to_the_navmesh_router_for_a_continuous_scene`,
    // `pathfind_continuous_start_equals_goal_is_a_single_point_zero_cost`) both pass
    // `is_gm: true`, so `mask` is always `None` and `clip_to_visible_mask` runs as a pure
    // pass-through — nothing is ever actually clipped. This test drives a non-GM request
    // through the FULL chain (`pathfind` → dispatch → `navmesh_for` → `navmesh_find` →
    // `clip_to_visible_mask`) with the REAL per-(user,scene) `visible_cells` mask, proving a
    // future fork/null of the mask on the `Continuous` branch would fail this test.
    let (ecs, user, scene) = scene_with_lit_player_token_continuous();
    let lenient = ecs.resolve_scene(scene).partial_cell_leniency;
    let mask = ecs.visible_cells(user, scene, lenient);
    assert!(!mask.is_empty(), "the lit token has a non-empty mask");

    // Far goal well outside the light radius (dimRadius 6 cells = 600 scene units) but still
    // inside the scene's default 100x100-cell bounds, so navmesh construction over the
    // bounds rect itself never fails — only the visibility clip should stop the route short.
    let far_goal = (9500.0, 9500.0);
    let outcome = ecs
        .pathfind(
            RouteRequester {
                user,
                is_gm: false,
                explored: None,
            },
            scene,
            (50.0, 50.0),
            &[far_goal],
            0.1,
        )
        .expect(
            "clip truncates the route short of the unseen goal rather than failing outright",
        );
    // `outcome.cost` is now in CELLS (the `pathfind` boundary conversion) while a raw
    // Euclidean distance over scene coordinates is in scene units — divide through the
    // fixture's cell size (100) so both sides of the comparison are the same unit.
    let dist_to_goal_cells =
        ((far_goal.0 - 50.0_f64).powi(2) + (far_goal.1 - 50.0_f64).powi(2)).sqrt() / 100.0;
    assert!(
        outcome.cost < dist_to_goal_cells / 2.0,
        "route must truncate well short of the unseen far goal: cost {} vs distance {} cells",
        outcome.cost,
        dist_to_goal_cells
    );
    let (lx, ly) = *outcome.path.last().expect("non-empty truncated path");
    let dist_from_start = ((lx - 50.0_f64).powi(2) + (ly - 50.0_f64).powi(2)).sqrt();
    assert!(
        dist_from_start < 700.0,
        "truncated endpoint must stay near the lit token, got ({lx}, {ly})"
    );
}

#[test]
fn pathfind_continuous_weighted_nongm_route_clips_to_the_visible_mask() {
    // `pathfind_continuous_nongm_route_clips_to_the_visible_mask` only drives the PURE-POLYANYA
    // sub-path (no terrain/impassable region present, so `has_terrain_or_impassable()` is
    // false). This test adds a terrain region so `pathfind`'s `Continuous` dispatch takes
    // the WEIGHTED sub-path (`pathfinding::find` forced Euclidean + `navmesh::los_smooth`)
    // for a non-GM requester under a real RESTRICTING `visible_cells` mask (default
    // fail-closed `MovementRestriction::Visible`, same fixture as the pure-polyanya test —
    // its default settings already yield a small, genuinely restricting mask).
    let user = Uuid::from_u128(7);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let light = entity_doc_eng(
        20,
        10,
        "light",
        json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true
        }),
    );
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                "vision": { "movementModel": "continuous" } }),
    );
    // Terrain mult 5 FAR outside the token's (small, default-settings) visible mask — its
    // mere existence anywhere on the scene is what flips `has_terrain_or_impassable()` true
    // and routes `pathfind`'s `Continuous` dispatch to the WEIGHTED sub-path; it is
    // deliberately placed off the requester's route so this test isolates "does the weighted
    // sub-path correctly enforce the mask" from "does terrain bend the route" (already
    // covered by `pathfind_continuous_terrain_bends_the_route_and_costs_cells`).
    let terrain = region_doc_top(
        12,
        10,
        "terrain",
        5.0,
        RegionRect {
            x0: 5000.0,
            y0: 5000.0,
            x1: 5100.0,
            y1: 5100.0,
        },
    );
    let ecs = SceneEcs::from_documents(vec![scene, tok, light, terrain], 0);
    let scene_id = Uuid::from_u128(10);
    let cell = 100.0;

    let lenient = ecs.resolve_scene(scene_id).partial_cell_leniency;
    let mask = ecs.visible_cells(user, scene_id, lenient);
    assert!(!mask.is_empty(), "the lit token has a non-empty mask");
    assert!(
        ecs.region_field(scene_id, Some(user))
            .expect("scene exists")
            .has_terrain_or_impassable(),
        "the terrain region flips the Continuous dispatch to the weighted sub-path"
    );

    // Near goal, still within the small visible mask: the weighted route must succeed and
    // stay entirely inside the mask (the grid A* mask check IS the enforcement mechanism for
    // this sub-path, so a route can never even be found outside the mask).
    let near_goal = (150.0, 50.0);
    let near = ecs
        .pathfind(
            RouteRequester {
                user,
                is_gm: false,
                explored: None,
            },
            scene_id,
            (50.0, 50.0),
            &[near_goal],
            0.1,
        )
        .expect("weighted route to a visible goal succeeds");
    for &(px, py) in &near.path {
        let c = ((px / cell).floor() as i32, (py / cell).floor() as i32);
        assert!(
            mask.contains(&c),
            "weighted route point ({px},{py}) -> cell {c:?} lies outside the visible mask"
        );
    }

    // Far goal, well outside the visible mask: the weighted grid search cannot even discover
    // a route through the unseen cells surrounding it (the mask check is baked into the A*
    // search itself, not a post-hoc clip), so it fails closed (`Unreachable`) rather than
    // returning a route that threads unseen cells.
    let far_goal = (9500.0, 9500.0);
    let far = ecs.pathfind(
        RouteRequester {
            user,
            is_gm: false,
            explored: None,
        },
        scene_id,
        (50.0, 50.0),
        &[far_goal],
        0.1,
    );
    assert!(
        far.is_err(),
        "weighted route to an unseen goal fails closed rather than routing through fog: {far:?}"
    );
}

#[test]
fn pathfind_continuous_secret_arrest_absent_from_player_preview_but_springs_at_execution() {
    // gm_only arrest region on cell (2,0) = Rect [200,0]-[300,100]. No terrain/impassable
    // region exists, so `has_terrain_or_impassable()` is false and `pathfind` takes the PURE
    // POLYANYA branch (`navmesh_find` -> `clip_to_visible_mask` -> `truncate_at_arrest`),
    // distinct from the weighted-grid branch. A player's per-requester region field omits the
    // secret region entirely, so their route preview is the full straight line with no
    // truncation; the GM's authoritative field truncates at the arrest cell. `move_exec`
    // always reads the authoritative field regardless of requester, so committing the
    // player's own (untruncated) preview still arrests at the same cell.
    let mut docs = continuous_scene_docs();
    let mut secret = region_doc_top(
        12,
        10,
        "arrest",
        1.0,
        RegionRect {
            x0: 200.0,
            y0: 0.0,
            x1: 300.0,
            y1: 100.0,
        },
    );
    secret
        .permissions
        .property_overrides
        .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
    docs.push(secret);
    let player = Uuid::from_u128(2);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(player);
    docs.push(tok);
    let mut ecs = SceneEcs::from_documents(docs, 0);
    ecs.set_world_settings_for_test(continuous_world_settings());
    let scene = Uuid::from_u128(10);
    let token = Uuid::from_u128(11);

    // Player (non-GM): secret arrest is invisible to their per-requester field, so the
    // preview is the full, untruncated straight polyanya route.
    let p = ecs
        .pathfind(
            RouteRequester {
                user: player,
                is_gm: false,
                explored: None,
            },
            scene,
            (50.0, 50.0),
            &[(450.0, 50.0)],
            0.1,
        )
        .expect("player route");
    assert!(
        !p.arrested,
        "secret arrest region does not truncate the player's own route preview"
    );
    // Pure-polyanya sub-path: 400 Euclidean scene units / cell(100) = 4 cells. Tolerance is
    // the pre-conversion 5.0-scene-unit bound divided through the same cell size.
    assert!(
        (p.cost - 4.0).abs() < 0.05,
        "player route reaches the full goal (~4 cells, 400 Euclidean scene units / cell 100), got {}",
        p.cost
    );

    // GM: authoritative field truncates the route at the arrest cell entry.
    let g = ecs
        .pathfind(
            RouteRequester {
                user: Uuid::from_u128(1),
                is_gm: true,
                explored: None,
            },
            scene,
            (50.0, 50.0),
            &[(450.0, 50.0)],
            0.1,
        )
        .expect("gm route");
    assert!(
        g.arrested,
        "GM sees the secret region and it truncates their route"
    );

    // `move_exec` always reads the AUTHORITATIVE field: committing the player's own
    // (untruncated) previewed route still springs the arrest at the same cell.
    let visible: std::collections::BTreeSet<(i32, i32)> = std::collections::BTreeSet::new();
    let exec_out = crate::scene::move_exec::execute_move(
        &ecs,
        crate::scene::move_exec::MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &visible,
            cell: *ecs
                .scene_grid_sizes()
                .get(&scene)
                .expect("the fixture's scene declares a grid size"),
        },
        token,
        &p.path,
        false,
        0.4,
    )
    .expect("move_exec handles the player's committed route");
    assert!(
        exec_out.truncated,
        "the authoritative field springs the secret arrest at execution"
    );
    assert!(
        exec_out.stop.0 < 400.0,
        "execution stops before the full player-preview route length, got {:?}",
        exec_out.stop
    );
}

/// A scene whose corridor from (50,50) to (250,50) is crossed by a FINITE `gm_only`
/// blocksMove wall at x=150 spanning y∈[0,100]. Continuous movement model (so the router
/// goes through `navmesh_for`'s per-requester obstacle set — the mechanism this fixture
/// exercises). `movement_restriction: unrestricted` so the visibility mask is not the
/// variable under test. The authored 4x4 block of cells at cell 100 gives a 400x400 world
/// rectangle, wide enough that a detour around the wall's y=100
/// endpoint exists. Returns `(ecs, scene, user, token)`; `owner_is_gm` only selects which
/// fixed user id is returned (routing GM-ness is the separate `is_gm` argument callers pass
/// to `pathfind` directly — this fixture places no GM/player distinction on the token or
/// wall doc itself, mirroring `scene_with_public_and_secret_move_walls`).
fn scene_with_secret_wall_between_two_cells(owner_is_gm: bool) -> (SceneEcs, Uuid, Uuid, Uuid) {
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                "bounds": { "width": 4.0, "height": 4.0 },
                "vision": { "movementModel": "continuous" } }),
    );
    let scene_id = Uuid::from_u128(10);
    let user = Uuid::from_u128(if owner_is_gm { 1 } else { 2 });
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let token_id = Uuid::from_u128(11);
    let mut wall = wall_doc_eng(scene_id, (150.0, 0.0), (150.0, 100.0));
    wall.permissions
        .property_overrides
        .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
    let mut ecs = SceneEcs::from_documents(vec![scene, tok, wall], 0);
    ecs.set_world_settings_for_test(continuous_world_settings());
    (ecs, scene_id, user, token_id)
}

#[test]
fn non_gm_route_crosses_a_gm_only_wall_that_springs_at_execution() {
    // The router cannot see the secret wall, so it routes straight through it; the executor reads
    // the authoritative set and stops there. Same spring-at-execution shape as a secret region.
    let (ecs, scene, player, token) = scene_with_secret_wall_between_two_cells(false);
    let out = ecs
        .pathfind(
            RouteRequester {
                user: player,
                is_gm: false,
                explored: None,
            },
            scene,
            (50.0, 50.0),
            &[(250.0, 50.0)],
            0.4,
        )
        .expect("the player's route ignores a wall it cannot see");
    assert!(
        out.path.len() >= 2,
        "a route is produced despite the secret wall across it"
    );

    let visible = ecs.visible_cells(player, scene, false);
    let exec = crate::scene::move_exec::execute_move(
        &ecs,
        crate::scene::move_exec::MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &visible,
            cell: *ecs
                .scene_grid_sizes()
                .get(&scene)
                .expect("the fixture's scene declares a grid size"),
        },
        token,
        &out.path,
        false,
        0.4,
    )
    .expect("execution is admissible");
    assert!(
        exec.truncated,
        "the secret wall springs at execution and truncates the move"
    );
}

#[test]
fn gm_route_does_not_cross_a_gm_only_wall() {
    // A GM passes viewer=None, so the secret wall IS in their routing set and no route SEGMENT
    // may cross the wall segment. Asserted structurally via segments_cross — NOT by testing
    // distance from the wall's x-line, which a legitimate detour around a finite wall's endpoint
    // necessarily crosses (and which, at cell size 100, every column-1 cell center sits exactly on).
    let (ecs, scene, gm, _token) = scene_with_secret_wall_between_two_cells(true);
    let out = ecs
        .pathfind(
            RouteRequester {
                user: gm,
                is_gm: true,
                explored: None,
            },
            scene,
            (50.0, 50.0),
            &[(250.0, 50.0)],
            0.4,
        )
        .expect("a GM route exists (bounds admit a detour around the wall's endpoint)");
    let wall = ((150.0, 0.0), (150.0, 100.0));
    for seg in out.path.windows(2) {
        assert!(
            !crate::scene::segments_cross(seg[0], seg[1], wall.0, wall.1),
            "no GM route segment crosses the wall it can see: {:?}",
            seg
        );
    }
}

#[test]
fn pathfind_grid_stepped_scene_is_byte_for_byte_unchanged() {
    // Same fixture/assertions as the existing `pathfind_gm_unconstrained_routes_without_a_mask`
    // test, proving the default (grid-stepped) dispatch branch is unaffected by the
    // continuous-engine dispatch.
    let (ecs, _user, scene) = scene_with_lit_player_token();
    let r = ecs.pathfind(
        RouteRequester {
            user: Uuid::from_u128(1),
            is_gm: true,
            explored: None,
        },
        scene,
        (50.0, 50.0),
        &[(250.0, 50.0)],
        0.1,
    );
    let outcome = r.expect("GM route");
    assert!(
        (outcome.cost - 2.0).abs() < 1e-9,
        "grid Chebyshev cost unchanged"
    );
}

#[test]
fn pathfind_nongm_visible_is_bounded_by_the_mask() {
    // Non-GM under movementRestriction "visible": a goal outside the lit mask is Unreachable.
    let (ecs, user, scene) = scene_with_lit_player_token();
    let lenient = ecs.resolve_scene(scene).partial_cell_leniency;
    let mask = ecs.visible_cells(user, scene, lenient);
    assert!(!mask.is_empty(), "the lit token has a non-empty mask");
    // A far goal well outside the lit radius → Unreachable.
    let far = ecs.pathfind(
        RouteRequester {
            user,
            is_gm: false,
            explored: None,
        },
        scene,
        (50.0, 50.0),
        &[(5000.0, 5000.0)],
        0.1,
    );
    assert_eq!(far, Err(crate::scene::pathfinding::PathFail::Unreachable));
}

#[test]
fn pathfind_revealed_unions_explored_memory() {
    // movementRestriction "revealed": an explored corridor covering start..goal makes an otherwise-unlit
    // goal routable.
    let (ecs, user, scene) = scene_revealed_player_token();
    let cell = *ecs
        .scene_grid_sizes()
        .get(&scene)
        .expect("the fixture's scene declares a grid size");
    let mut explored = crate::scene::explored::ExploredSet::new();
    // Mark cells (0,0)..(3,0) as explored (a straight corridor).
    let grid = crate::scene::grid_shape::SquareGrid {
        cell,
        rule: crate::scene::pathfinding::DiagonalRule::Chebyshev,
    };
    explored.mark_polygons(
        &[vec![0.0, 0.0, 4.0 * cell, 0.0, 4.0 * cell, cell, 0.0, cell]],
        &grid,
        cell,
    );
    let r = ecs.pathfind(
        RouteRequester {
            user,
            is_gm: false,
            explored: Some(&explored),
        },
        scene,
        (50.0, 50.0),
        &[(350.0, 50.0)],
        0.1,
    );
    assert!(
        r.is_ok(),
        "explored corridor makes the goal routable under revealed"
    );
}

// --- player_vision_polygons_at: mover vision trajectory ---

/// Advancing past a `blocksSight` wall changes the visibility polygon: a point beyond
/// the wall is invisible from the near viewpoint but visible from the far viewpoint.
#[test]
fn vision_at_grows_as_token_advances() {
    // Vertical blocksSight wall at x=100 (y ±200). Token committed at (50,50).
    // The wall spans the relevant y range of the bounding box so the test point
    // (150,50) is directly occluded from the near side.
    let scene = Uuid::from_u128(10);
    let user = Uuid::from_u128(7);
    let token_id = Uuid::from_u128(11);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let wall = entity_doc_eng(
        12,
        10,
        "wall",
        json!({ "seg": {"x1": 100, "y1": -200, "x2": 100, "y2": 200},
                "blocksSight": true }),
    );
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, wall], 0);

    // Near viewpoint (50,50): wall at x=100 occludes (150,50) — ray crosses the wall.
    let polys_near = ecs.player_vision_polygons_at(user, scene, token_id, (50.0, 50.0));
    assert!(
        !polys_near.is_empty(),
        "must return a polygon for an owned token"
    );
    let in_near = vision::point_in_poly(&polys_near[0], (150.0, 50.0));

    // Far viewpoint (200,50): token is past the wall; (150,50) is between wall and viewpoint
    // on the same side, so it IS visible.
    let polys_far = ecs.player_vision_polygons_at(user, scene, token_id, (200.0, 50.0));
    assert!(
        !polys_far.is_empty(),
        "must return a polygon for an owned token"
    );
    let in_far = vision::point_in_poly(&polys_far[0], (150.0, 50.0));

    assert!(
        !in_near,
        "near viewpoint (50,50) must NOT see (150,50) past the wall at x=100"
    );
    assert!(
        in_far,
        "far viewpoint (200,50) must see (150,50) between wall and viewpoint"
    );
}

/// A `blocksSight` wall with `gm_only` permissions (DocRole::None default — players cannot
/// read this wall doc) must produce the SAME occlusion as an identically-placed normal wall.
/// Invariant: `sight_walls` uses the FULL ECS wall set regardless of doc permissions;
/// the server never leaks the wall's existence, only uses it for raycast geometry.
#[test]
fn vision_at_uses_full_wall_set() {
    use crate::data::document::DocRole;
    let scene = Uuid::from_u128(10);
    let user = Uuid::from_u128(7);
    let token_id = Uuid::from_u128(11);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let wall_sys =
        json!({ "seg": {"x1": 100, "y1": -200, "x2": 100, "y2": 200}, "blocksSight": true });

    // Normal wall (default permissions): occludes from (50,50).
    let normal_wall = entity_doc_eng(12, 10, "wall", wall_sys.clone());
    let ecs_normal =
        SceneEcs::from_documents(vec![doc(10, None, "scene"), tok.clone(), normal_wall], 0);
    let polys_normal =
        ecs_normal.player_vision_polygons_at(user, scene, token_id, (50.0, 50.0));
    assert!(!polys_normal.is_empty());

    // gm_only wall (DocRole::None): players cannot access this doc, but must occlude equally.
    let mut gm_wall = entity_doc_eng(12, 10, "wall", wall_sys);
    gm_wall.permissions.default = DocRole::None;
    let ecs_gm = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, gm_wall], 0);
    let polys_gm = ecs_gm.player_vision_polygons_at(user, scene, token_id, (50.0, 50.0));
    assert!(!polys_gm.is_empty());

    // Both walls must produce identical polygons — sight_walls is permission-blind.
    assert_eq!(
        polys_normal[0], polys_gm[0],
        "gm_only wall must occlude identically to a normal wall with the same geometry"
    );

    // Cross-check: the occluded point (150,50) is NOT visible from (50,50) with either wall.
    assert!(
        !vision::point_in_poly(&polys_gm[0], (150.0, 50.0)),
        "gm_only wall must occlude (150,50): point must not be inside the polygon"
    );
}

// --- wall-less scene full intrascene vision ---

/// A wall-less scene authored as a 5x5 block of cells at cell 100 — a 500x500 world
/// rectangle — must reveal its own full extent, not a small `VISION_BOUND_MARGIN` box around
/// the viewpoint.
#[test]
fn wall_less_scene_gives_full_intrascene_vision_not_a_degenerate_box() {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 5.0, "y": 5.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                "bounds": { "width": 5.0, "height": 5.0 } }),
    );
    let ecs = SceneEcs::from_documents(vec![scene, tok], 0);

    let polys = ecs.player_vision_polygons(user);
    let (_, poly) = polys
        .iter()
        .find(|(sid, _)| *sid == scene_id)
        .expect("scene present");

    let far_corner = (490.0, 490.0);
    assert!(
        vision::point_in_poly(poly, far_corner),
        "a wall-less scene must reveal its own full bounded extent, not a small box around the viewpoint"
    );
}

/// Each scene's vision bound uses ITS OWN extent, never a neighbour's. The viewpoint loop
/// spans every scene the user owns a token in, so an extent resolved once OUTSIDE that loop
/// would measure one scene's bound against another scene's rectangle — and the memoisation
/// that avoids re-scanning the entity table per viewpoint is exactly where that mistake fits.
#[test]
fn each_scenes_vision_bound_uses_its_own_extent_not_a_neighbours() {
    // Two wall-less scenes with deliberately mismatched extents: scene 10 is a 5x5 block at
    // cell 100 (a 500-unit square), scene 20 a 1x1 block at cell 100 (a 100-unit square). The
    // probe points are read from each scene's OWN resolved extent, so neither is a literal.
    // Discrimination: with a single hoisted extent both scenes answer with the same rectangle,
    // so whichever scene is not the source of that value fails one of its two assertions —
    // the small scene reveals a point beyond its own extent, or the large scene stops short of
    // one inside its own.
    let user = Uuid::from_u128(7);
    let mut docs = Vec::new();
    for (scene_id, token_id, block) in [(10u128, 11u128, 5.0_f64), (20, 21, 1.0)] {
        let mut tok = entity_doc_eng(
            token_id,
            scene_id,
            "token",
            json!({ "x": 5.0, "y": 5.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        docs.push(entity_doc_top_eng(
            scene_id,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "bounds": { "width": block, "height": block } }),
        ));
        docs.push(tok);
    }
    let ecs = SceneEcs::from_documents(docs, 0);
    let polys = ecs.player_vision_polygons(user);
    assert_eq!(
        polys.len(),
        2,
        "one polygon per scene the user owns a token in"
    );

    let extents: Vec<grid_shape::WorldExtent> = [10u128, 20]
        .iter()
        .map(|&s| ecs.scene_world_extent(Uuid::from_u128(s)))
        .collect();
    assert!(
        extents[0].max.0 > extents[1].max.0,
        "fixture: the two scenes must have different extents, got {extents:?}"
    );

    for (i, scene_id) in [10u128, 20].iter().enumerate() {
        let (_, poly) = polys
            .iter()
            .find(|(sid, _)| *sid == Uuid::from_u128(*scene_id))
            .expect("scene present");
        let (ex, ey) = extents[i].max;
        // Just inside this scene's own extent, on the diagonal from the viewpoint.
        let inside = (ex - 10.0, ey - 10.0);
        assert!(
            vision::point_in_poly(poly, inside),
            "scene {scene_id} must reveal {inside:?}, inside its own extent {:?}",
            extents[i]
        );
        // Beyond this scene's own extent AND beyond the wall-less margin box around (5,5).
        let outside = (
            ex + VISION_BOUND_MARGIN + 10.0,
            ey + VISION_BOUND_MARGIN + 10.0,
        );
        assert!(
            !vision::point_in_poly(poly, outside),
            "scene {scene_id} must not reveal {outside:?}, beyond its own extent {:?}",
            extents[i]
        );
    }
}

/// The wall-less-scene vision fix must stay bounded to the scene's own extent — never
/// unbounded, never leaking beyond `bounds`.
#[test]
fn wall_less_scene_vision_does_not_leak_beyond_its_own_bounds() {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 5.0, "y": 5.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                "bounds": { "width": 5.0, "height": 5.0 } }),
    );
    let ecs = SceneEcs::from_documents(vec![scene, tok], 0);

    let polys = ecs.player_vision_polygons(user);
    let (_, poly) = polys.iter().find(|(sid, _)| *sid == scene_id).unwrap();

    let beyond_bounds = (1000.0, 1000.0);
    assert!(
        !vision::point_in_poly(poly, beyond_bounds),
        "vision must stay bounded to the scene's own extent, never unbounded"
    );
}

/// `player_vision_polygons` and `player_vision_inputs` (via its `polygons_at` per-sample
/// path) must not fork: same wall set (empty), same scene-bounds-aware bound.
#[test]
fn player_vision_polygons_and_player_vision_inputs_agree_on_wall_less_bound() {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 5.0, "y": 5.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                "bounds": { "width": 5.0, "height": 5.0 } }),
    );
    let ecs = SceneEcs::from_documents(vec![scene, tok], 0);

    let poly_from_polygons = ecs
        .player_vision_polygons(user)
        .into_iter()
        .find(|(sid, _)| *sid == scene_id)
        .map(|(_, p)| p);
    let poly_from_inputs = ecs
        .player_vision_polygons_at(user, scene_id, token_id, (5.0, 5.0))
        .into_iter()
        .next();

    assert_eq!(
        poly_from_polygons, poly_from_inputs,
        "player_vision_polygons and player_vision_inputs must compute the identical bound for the same wall-less scene"
    );
}

/// Returns empty when the user owns no token in the scene, even when `moving_token`
/// points to an existing token owned by another user.
#[test]
fn vision_at_empty_when_user_owns_no_token() {
    let scene = Uuid::from_u128(10);
    let user = Uuid::from_u128(7);
    let stranger = Uuid::from_u128(999);
    let token_id = Uuid::from_u128(11);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user); // owned by user, NOT stranger
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok], 0);

    // Stranger owns no token in this scene → empty (fail-closed).
    let polys = ecs.player_vision_polygons_at(stranger, scene, token_id, (50.0, 50.0));
    assert!(
        polys.is_empty(),
        "user with no owned token must get empty polygons (fail-closed)"
    );
}

// --- source_los_poly wall-less degenerate box (player_lit_mask/visible_cells) ---

/// A wall-less scene authored as a 5x5 block of cells at cell 100 — a 500x500 world
/// rectangle — with all-bright lighting (isolates the bound-box defect from
/// illumination), `losRestriction` off (so `source_los_poly` takes the plain-rectangle branch,
/// the branch susceptible to the bound-box defect). Cell (4,4) — center (450,450) — lies within the
/// scene's authored extent but strictly outside a degenerate
/// `viewpoint±VISION_BOUND_MARGIN(100)` box around the token at (50,50): `[-50,-50]..[150,150]`.
fn wall_less_large_scene_all_bright() -> (SceneEcs, Uuid, Uuid) {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                "bounds": { "width": 5.0, "height": 5.0 } }),
    );
    let mut ecs = SceneEcs::from_documents(vec![scene, tok], 0);
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": false, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "movementModel": "grid-stepped",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    (ecs, user, scene_id)
}

/// `player_lit_mask` (the egress/secrecy gate) must cover a wall-less scene's full authored
/// bounds, not a degenerate box around the viewpoint — the same bound-box defect class fixed in
/// `player_vision_polygons`/`player_vision_inputs`, applied here to `source_los_poly`, the
/// primitive `player_lit_mask` shares with `visible_cells`.
#[test]
fn player_lit_mask_wall_less_scene_covers_full_bounds_not_a_degenerate_box() {
    let (ecs, user, scene_id) = wall_less_large_scene_all_bright();
    let cells: std::collections::BTreeSet<(i32, i32)> = ecs
        .player_lit_mask(user)
        .into_iter()
        .filter(|s| s.scene == scene_id)
        .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
        .collect();
    assert!(
        cells.contains(&(4, 4)),
        "a wall-less scene's lit mask must cover its full authored bounds, not a degenerate box around the viewpoint"
    );
}

/// `visible_cells` (the movement gate) must cover a wall-less scene's full authored bounds,
/// not a degenerate box — the same defect class
/// `player_lit_mask_wall_less_scene_covers_full_bounds_not_a_degenerate_box` pins on the
/// egress gate, mirrored to the movement-gate consumer.
#[test]
fn visible_cells_wall_less_scene_covers_full_bounds_not_a_degenerate_box() {
    let (ecs, user, scene_id) = wall_less_large_scene_all_bright();
    let mask = ecs.visible_cells(user, scene_id, false);
    assert!(
        mask.contains(&(4, 4)),
        "a wall-less scene's movement-gate mask must cover its full authored bounds, not a degenerate box around the viewpoint"
    );
}

/// No-fork parity: `source_los_poly`'s bound (as exercised via `visible_cells`) must agree
/// with `player_vision_polygons`'s bound (via `vision::bound_for_scene` directly) on the same
/// wall-less scene — closing the "two/three vision paths diverge" defect class, generalized to
/// this third path.
#[test]
fn visible_cells_agrees_with_player_vision_polygons_bound_on_wall_less_scene() {
    let (ecs, user, scene_id) = wall_less_large_scene_all_bright();

    let polys = ecs.player_vision_polygons(user);
    let (_, poly) = polys
        .iter()
        .find(|(sid, _)| *sid == scene_id)
        .expect("scene present");
    let far_corner = (490.0, 490.0);
    assert!(
        vision::point_in_poly(poly, far_corner),
        "player_vision_polygons must reveal the scene's own full bounded extent"
    );

    let mask = ecs.visible_cells(user, scene_id, false);
    assert!(
        mask.contains(&(4, 4)),
        "visible_cells (via source_los_poly) must not diverge from player_vision_polygons' bound for the same wall-less scene"
    );
}

/// Pins the exact movement-gate cell set for an open all-bright scene. `accumulate_visible_cells`
/// computes each candidate cell's CENTER via `GridShape::cell_center`; `SquareGrid::cell_center`
/// equals the hardcoded `((i+0.5)*cell,(j+0.5)*cell)` square formula, so a regression to
/// non-square center math in that function diverges from this frozen set immediately, without
/// depending on the broader frozen-fixture parity battery. Reuses
/// `wall_less_large_scene_all_bright` (one owned token, no walls, all-bright, a 5x5
/// block at cell 100).
#[test]
fn accumulate_visible_cells_routes_through_grid_shape_cell_center_not_hardcoded() {
    let (ecs, user, scene_id) = wall_less_large_scene_all_bright();
    let got = ecs.visible_cells(user, scene_id, false);
    let expected: std::collections::BTreeSet<(i32, i32)> = (-1..=4)
        .flat_map(|i| (-1..=4).map(move |j| (i, j)))
        .collect();
    assert_eq!(got, expected);
}

/// Pins the exact secrecy-egress cell set `player_lit_mask` emits for an open all-bright scene.
/// `player_lit_mask` computes each candidate cell's CENTER via `GridShape::cell_center`;
/// `SquareGrid::cell_center` equals the hardcoded `((i+0.5)*cell,(j+0.5)*cell)` square formula,
/// so a regression to non-square center math in that function diverges from this frozen set
/// immediately. Companion to `accumulate_visible_cells_routes_through_grid_shape_cell_center_not_hardcoded`,
/// applied to the OTHER (separate) secrecy-egress call site; the pinned set matches the strict
/// movement-gate set (`visible_cells` strict ≡ `player_lit_mask` cells). Reuses
/// `wall_less_large_scene_all_bright` (one owned token, no walls, all-bright, a 5x5
/// block at cell 100).
#[test]
fn player_lit_mask_routes_through_grid_shape_cell_center_not_hardcoded() {
    let (ecs, user, scene_id) = wall_less_large_scene_all_bright();
    let got: std::collections::BTreeSet<(i32, i32)> = ecs
        .player_lit_mask(user)
        .into_iter()
        .filter(|s| s.scene == scene_id)
        .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
        .collect();
    let expected: std::collections::BTreeSet<(i32, i32)> = (-1..=4)
        .flat_map(|i| (-1..=4).map(move |j| (i, j)))
        .collect();
    assert_eq!(got, expected);
}

/// `hex_open_scene_with_vision_range` at the unlimited-range setting: a wall-less pointy-top
/// hex scene at `HEX_FIXTURE_SIZE`, all-bright, LOS off, one owned instanced token at hex
/// (0,0) = pixel (0,0) with unlimited "normal" vision. That constructor's own doc carries the
/// geometry every dependant of either form reads.
fn hex_open_scene() -> (SceneEcs, Uuid, Uuid) {
    hex_open_scene_with_vision_range(None)
}

/// A wall-less pointy-top hex scene at `HEX_FIXTURE_SIZE`, all-bright, LOS off, one owned
/// instanced token at hex (0,0) = pixel (0,0), with the token's sight distance under the
/// caller's control. `None` leaves the token with no embedded actor at all, so
/// `token_vision_floors` falls back to normal at unlimited range; `Some(cells)` gives it an
/// embedded actor whose single "normal" assignment carries that range in GRID CELLS. Nothing
/// else varies between the two, so a bounded and an unbounded token measure the same geometry
/// rather than two fixtures that have to be kept in step.
///
/// The range rides `VisionAssignment.range`, which `token_vision_floors` reads directly when
/// present; an absent `range` instead resolves to the mode's own `VisionMode.default_range`.
/// This fixture always authors an explicit `range`, so it never exercises that fallback.
///
/// The authored block is 3.2 x 3.0 hexes, which is fractional because a hex block's world
/// rectangle is a shear-dependent function of the block rather than a per-axis product.
/// `HexGrid::world_extent((3.2, 3.0))` answers a two-corner envelope. Its `max` evaluates
/// `(√3·size·(2.2 + 1.0) + √3/2·size, size·1.5·2 + size)`, which collapses to
/// `(3.7·√3·size, 4·size)` — so along axial row 0, where a hex's centre sits `q` PITCHES
/// (`√3·size`) from the origin and its left vertices half a pitch nearer, the envelope reaches
/// `q = 3.7`. Its `min` is the origin hex's own lower-left extreme, `(-√3/2·size, -size)` =
/// `(-43.3, -50)` at this fixture's size. Pitches are the unit its dependants
/// name cells in; a dependant that states a coordinate rather than a pitch must re-derive it
/// against this fixture's own size.
/// `source_los_poly` is then `[min(-VISION_BOUND_MARGIN, extent.min),
/// max(VISION_BOUND_MARGIN, extent.max)]` per axis, and the two sides are dominated by
/// different terms at this fixture's size: the envelope's maximum wins on the high side, while
/// the margin (100) wins on the low side, the envelope's own minimum reaching only -43.3 and
/// -50. That dominance is why this fixture's dependants measure the same mask an
/// origin-anchored rectangle would give — a property of this size, not of the conversion.
fn hex_open_scene_with_vision_range(range_cells: Option<f64>) -> (SceneEcs, Uuid, Uuid) {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    if let Some(range) = range_cells {
        tok.embedded.insert(
            "actor".into(),
            vec![{
                let mut a = doc(99, None, "actor");
                a.engine = Some(actor_body(json!([{ "mode": "normal", "range": range }])));
                a
            }],
        );
    }
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "hex", "size": HEX_FIXTURE_SIZE }, "background": null,
                "bounds": { "width": 3.2, "height": 3.0 } }),
    );
    let mut ecs = SceneEcs::from_documents(vec![scene, tok], 0);
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": false, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "movementModel": "grid-stepped",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    (ecs, user, scene_id)
}

/// REJECT direction on a hex scene: a hex cell whose HEX CENTER falls outside the vision mask
/// is excluded from `visible_cells`. Measured in pitches along axial row 0, where
/// `hex_open_scene`'s LOS rectangle reaches 3.7: hex (2,0)'s centre sits at 2 and is visible;
/// hex (5,0)'s centre sits at 5 and its nearest (left) vertices at 4.5, both past 3.7, so it
/// is excluded under BOTH strict and lenient sampling. Guards that the hex candidate
/// enumeration cannot admit an out-of-mask hex cell.
#[test]
fn visible_cells_hex_excludes_cell_whose_center_is_outside_the_mask() {
    let (ecs, user, scene) = hex_open_scene();
    let strict = ecs.visible_cells(user, scene, false);
    assert!(
        strict.contains(&(2, 0)),
        "hex (2,0) center is inside the LOS rectangle"
    );
    assert!(
        !strict.contains(&(5, 0)),
        "hex (5,0) center is outside the mask -> excluded"
    );
    // Even leniency (corner sampling) cannot pull (5,0) in: its nearest vertex is still outside.
    let lenient = ecs.visible_cells(user, scene, true);
    assert!(
        !lenient.contains(&(5, 0)),
        "hex (5,0) has no vertex inside the mask either"
    );
}

/// Leniency on a hex scene samples the SIX hex vertices (`GridShape::cell_vertices`), not four
/// square corners. In pitches along axial row 0, against `hex_open_scene`'s reach of 3.7: hex
/// (4,0)'s centre sits at 4, just outside, so strict excludes it; its left vertices sit at
/// 3.5, inside, so lenient includes it. The strict->lenient flip proves the hex corner
/// geometry is wired.
#[test]
fn visible_cells_hex_lenient_includes_cell_whose_vertex_clips_the_mask() {
    let (ecs, user, scene) = hex_open_scene();
    let strict = ecs.visible_cells(user, scene, false);
    assert!(
        !strict.contains(&(4, 0)),
        "hex (4,0) center is outside -> strict excludes"
    );
    let lenient = ecs.visible_cells(user, scene, true);
    assert!(
        lenient.contains(&(4, 0)),
        "hex (4,0) vertex clips the mask -> lenient includes"
    );
}

/// End-to-end composition of the hex leniency path: `GridShape::cell_vertices` (six hex corners)
/// widens `visible_cells`, and the widened mask is what the authoritative executor gates
/// against. The SAME move into hex (4,0) — whose center is outside the LOS rectangle but whose
/// left vertices clip it — completes under leniency and truncates under strict center sampling.
/// This is the composed behavior no per-site test covers: leniency is only meaningful if the
/// executor consumes the widened mask.
#[test]
fn hex_lenient_mask_lets_the_executor_enter_a_cell_the_strict_mask_stops_at() {
    let (ecs, user, scene) = hex_open_scene();
    let cell = HEX_FIXTURE_SIZE;
    let token = Uuid::from_u128(11);
    let grid = ecs.resolve_grid_shape(scene, cell);
    let dest = grid.cell_center((4, 0));

    let lenient_mask = ecs.visible_cells(user, scene, true);
    let out = crate::scene::move_exec::execute_move(
        &ecs,
        crate::scene::move_exec::MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &lenient_mask,
            cell,
        },
        token,
        &[(0.0, 0.0), dest],
        false,
        0.4,
    )
    .expect("a token move on a hex scene executes");
    assert!(
        !out.truncated,
        "the lenient mask admits every traversed hex cell"
    );
    assert_eq!(grid.cell_of(out.stop), (4, 0), "the move reaches hex (4,0)");

    let strict_mask = ecs.visible_cells(user, scene, false);
    let out = crate::scene::move_exec::execute_move(
        &ecs,
        crate::scene::move_exec::MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &strict_mask,
            cell,
        },
        token,
        &[(0.0, 0.0), dest],
        false,
        0.4,
    )
    .expect("a token move on a hex scene executes");
    assert!(out.truncated, "strict center sampling excludes hex (4,0)");
    assert_ne!(
        grid.cell_of(out.stop),
        (4, 0),
        "the strict move never enters hex (4,0)"
    );
}

/// Sight distance in GRID CELLS the hex range fixtures give their token. Half a cell clear of
/// both probes — hex (2,0) sits 2.0 grid steps from the source and hex (3,0) sits 3.0 — so
/// neither assertion turns on an equality between computed floats.
const HEX_VISION_RANGE_CELLS: f64 = 2.5;

/// Asserts hex `(q, 0)`'s centre lies inside the scene's own world-unit envelope, so a test
/// asserting that hex is ABSENT from a mask is measuring the quantity it names rather than a
/// hex nothing reached. Fixture guard, not a property under test.
///
/// The envelope answers for two separate reaches at once. `source_los_poly`'s scan box is a
/// union OVER the envelope (`vision::bound_for_scene`), so clearing the envelope's high edge
/// clears the scan's. And where a fixture walls its `blocksLight` room along that same
/// envelope, a hex inside it is inside the room, hence not cut off by the light's own
/// occlusion polygon. Row 0 needs the x axis only — the envelope reaches a full circumradius
/// below the origin on y, and the scan's margin reaches further still.
fn assert_hex_row_zero_is_scanned(ecs: &SceneEcs, scene: Uuid, q: i32) {
    let grid = ecs.resolve_grid_shape(scene, HEX_FIXTURE_SIZE);
    let centre = grid.cell_center((q, 0));
    let extent = grid.world_extent(ecs.resolve_scene(scene).bounds);
    assert!(
        centre.0 < extent.max.0,
        "fixture: hex ({q},0)'s centre {} must sit inside the scanned envelope, which reaches {}",
        centre.0,
        extent.max.0
    );
}

#[test]
fn a_hex_vision_range_is_measured_in_grid_steps() {
    // A sight range authored in cells must reach the hex two grid steps away and not the hex
    // three steps away. On a pointy-top hex those centres are 2·√3·size and 3·√3·size scene
    // units out, i.e. 2.0 and 3.0 grid steps; dividing by the indexing scale instead reports
    // 3.46 and 5.20.
    //
    // Discrimination: under the indexing-scale divisor (2,0) reads as 3.46 cells and drops
    // out, so the first assertion fails; under any divisor more than 20% larger than √3·size,
    // (3,0) reads as under 2.5 cells and joins the mask, so the second fails. The pair
    // brackets the conversion from both sides with half a cell of clearance on each, and the
    // call path is `visible_cells`, the production movement-gate mask rather than a helper.
    let (ecs, user, scene) = hex_open_scene_with_vision_range(Some(HEX_VISION_RANGE_CELLS));
    assert_hex_row_zero_is_scanned(&ecs, scene, 3);
    let mask = ecs.visible_cells(user, scene, false);
    assert!(
        mask.contains(&(2, 0)),
        "two grid steps is inside a {HEX_VISION_RANGE_CELLS}-cell range, got {mask:?}"
    );
    assert!(
        !mask.contains(&(3, 0)),
        "three grid steps is outside a {HEX_VISION_RANGE_CELLS}-cell range"
    );
}

#[test]
fn a_hex_vision_range_bounds_the_lit_egress_the_same_way() {
    // `player_lit_mask` computes its own `dist_cells` rather than routing through
    // `point_qualifies`, so the range conversion has two independent homes and a test through
    // one proves nothing about the other. Under strict sampling the two masks must agree.
    //
    // Discrimination: fails if `player_lit_mask`'s divisor keeps the indexing scale, because
    // (2,0) then reads as 3.46 cells and is not shipped, while
    // `a_hex_vision_range_is_measured_in_grid_steps` still passes once its own divisor is
    // converted. Both read `hex_open_scene_with_vision_range`, so a divergence between the
    // gate and the egress shows up as exactly one of the two failing.
    let (ecs, user, scene) = hex_open_scene_with_vision_range(Some(HEX_VISION_RANGE_CELLS));
    assert_hex_row_zero_is_scanned(&ecs, scene, 3);
    let cells = mask_cells(&ecs, user, scene);
    assert!(
        cells.contains(&(2, 0)),
        "two grid steps is inside a {HEX_VISION_RANGE_CELLS}-cell range, got {cells:?}"
    );
    assert!(
        !cells.contains(&(3, 0)),
        "three grid steps is outside a {HEX_VISION_RANGE_CELLS}-cell range"
    );
}

/// Bright radius, in GRID CELLS, of `hex_lit_scene`'s lamp: half a cell past hex (2,0), which
/// sits 2.0 grid steps out.
const HEX_LIGHT_BRIGHT_CELLS: f64 = 2.5;
/// Dim radius, in GRID CELLS, of `hex_lit_scene`'s lamp: half a cell short of hex (4,0), which
/// sits 4.0 grid steps out.
const HEX_LIGHT_DIM_CELLS: f64 = 3.5;

/// The authored block, in hexes, of `hex_lit_scene`. Row 0 of the envelope it produces reaches
/// well past hex (4,0), which both of that fixture's reaches depend on.
const HEX_LIGHT_BLOCK: (f64, f64) = (6.0, 4.0);

/// A pointy-top hex scene at `HEX_FIXTURE_SIZE` with lighting ENABLED, an environment
/// intensity of zero (so the lamp is the only illumination any cell receives), one
/// player-owned token at hex (0,0) with unlimited normal vision, and one lamp at that same
/// point carrying `HEX_LIGHT_BRIGHT_CELLS`/`HEX_LIGHT_DIM_CELLS` radii. Wall-less: a light's
/// occlusion-polygon bound grows to cover its own authored reach
/// (`vision::bound_for_reach`), so nothing needs to be walled in for the probes to fall
/// inside the polygon and hand the decision to the radii. `blocksSight` is off on every
/// document here (there are none), so the LOS polygon stays the plain rectangle and vision
/// never gates a probe either.
fn hex_lit_scene() -> (SceneEcs, Uuid, Uuid) {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let light = entity_doc_eng(
        20,
        10,
        "light",
        json!({
            "x": 0.0, "y": 0.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": HEX_LIGHT_BRIGHT_CELLS, "dimRadius": HEX_LIGHT_DIM_CELLS,
            "enabled": true
        }),
    );
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "hex", "size": HEX_FIXTURE_SIZE }, "background": null,
                "bounds": { "width": HEX_LIGHT_BLOCK.0, "height": HEX_LIGHT_BLOCK.1 } }),
    );
    let docs = vec![scene, tok, light];
    let mut ecs = SceneEcs::from_documents(docs, 0);
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 0.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "movementModel": "grid-stepped",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    (ecs, user, scene_id)
}

#[test]
fn a_hex_light_radius_is_measured_in_grid_steps() {
    // A lamp's radii are authored in cells, so a 2.5-cell bright radius must light the hex two
    // grid steps away and a 3.5-cell dim radius must leave the hex four steps away dark. Those
    // distances are the same 2.0 and 4.0 grid steps as the range fixture's; the divisor is the
    // only thing under test.
    //
    // Discrimination: fails whenever `cell_illumination` receives the indexing scale, because
    // 2 grid steps then read as 3.46 cells, which is inside neither radius by enough to clear
    // a normal token's floor, and the cell reports dark. Both masks are asserted because
    // `cell_illumination` has two production callers — `player_lit_mask`'s per-cell closure
    // and `point_qualifies` — and converting one without the other forks the gate from the
    // egress.
    let (ecs, user, scene) = hex_lit_scene();
    assert_hex_row_zero_is_scanned(&ecs, scene, 4);
    let cells = mask_cells(&ecs, user, scene);
    assert!(
        cells.contains(&(2, 0)),
        "two grid steps is inside a {HEX_LIGHT_BRIGHT_CELLS}-cell bright radius, got {cells:?}"
    );
    assert!(
        !cells.contains(&(4, 0)),
        "four grid steps is beyond the {HEX_LIGHT_DIM_CELLS}-cell dim radius"
    );
    let mask = ecs.visible_cells(user, scene, false);
    assert!(
        mask.contains(&(2, 0)),
        "the gate mask agrees with the egress mask, got {mask:?}"
    );
    assert!(
        !mask.contains(&(4, 0)),
        "the gate mask agrees with the egress mask"
    );
}

/// REQUIREMENT this scene has to satisfy, which is what every test reading it depends on: a
/// single source's candidate scan must exceed `MAX_CELLS_PER_POLYGON`. The width supplies that
/// over-cap product with an enormous margin — the authored block is measured in grid units
/// (cells), which `GridShape::world_extent` multiplies by the cell size, so the scan clears
/// the cap by two further orders of magnitude than the authored number alone suggests. A scan
/// under the cap never engages the clamp, and the assertions would then hold for a reason they
/// do not name. The height is small so the CLAMPED scan is a few thousand cells and the tests
/// run in a unit suite. Wall-less, all-bright, LOS off, one owned token at the origin cell, so
/// the whole scan is a single source's.
fn over_cap_scan_scene() -> (SceneEcs, Uuid, Uuid) {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                "bounds": { "width": 200_000_000.0, "height": 5.0 } }),
    );
    let mut ecs = SceneEcs::from_documents(vec![scene, tok], 0);
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": false, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "movementModel": "grid-stepped",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    (ecs, user, scene_id)
}

#[test]
fn an_over_cap_visibility_scan_yields_a_bounded_mask_not_an_empty_one() {
    // Under `MovementRestriction::Visible` an empty mask refuses every move, so the
    // over-cap outcome must be a bounded neighbourhood of the source rather than nothing.
    // Discrimination: fails if `accumulate_visible_cells` hands the unclamped bbox to
    // `cells_in_bounds`, because the cap then returns `None` and the source is skipped,
    // leaving the mask empty. It cannot pass vacuously: the second assertion requires the
    // mask to STOP somewhere, so a scan that ignored the cap entirely also fails.
    let (ecs, user, scene) = over_cap_scan_scene();
    let mask = ecs.visible_cells(user, scene, false);
    assert!(mask.contains(&(0, 0)), "the source's own cell is visible");
    let outside = crate::scene::explored::SCAN_WINDOW_HALF_CELLS as i32 + 10;
    assert!(
        !mask.contains(&(outside, 0)),
        "a cell beyond the scan window is not in the mask"
    );
}

#[test]
fn an_over_cap_lit_mask_scan_yields_a_bounded_cell_set_not_an_empty_one() {
    // The egress half of the same scan, which is a separate call site and would otherwise be
    // converted independently. Discrimination: identical to the mask test, applied to
    // `player_lit_mask`'s own scan.
    let (ecs, user, scene) = over_cap_scan_scene();
    let cells: std::collections::BTreeSet<(i32, i32)> = ecs
        .player_lit_mask(user)
        .into_iter()
        .filter(|s| s.scene == scene)
        .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
        .collect();
    assert!(cells.contains(&(0, 0)), "the source's own cell is lit");
    let outside = crate::scene::explored::SCAN_WINDOW_HALF_CELLS as i32 + 10;
    assert!(
        !cells.contains(&(outside, 0)),
        "a cell beyond the scan window is not shipped"
    );
}

/// A scene sized so the STRICT (unpadded) candidate scan's own span sits exactly at
/// `explored::MAX_CELLS_PER_POLYGON` (2000×2000 cells, product 4,000,000, returned unclamped
/// by itself) while the LENIENT (one-cell-padded) scan's own span exceeds it (2002×2002,
/// product 4,008,004, clamped by itself) — the band where the two invocations' own spans
/// straddle the cap on either side of it. Wall-less, all-bright, LOS off, one owned token at
/// `(100, 100)`. The grid size is 1, so a `1999 × 1999` authored block converts to a
/// `1999 × 1999` world rectangle — the one grid size at which a block measured in grid units
/// and its world span coincide, which is what keeps the two candidate spans this doc names
/// exactly at the cap.
/// `source_los_poly`'s bound rectangle is therefore exactly `[0, 0]–[1999, 1999]`
/// (`VISION_BOUND_MARGIN` cancels against the token's own offset on the low edge; the scene's
/// extent dominates the high edge).
fn strict_lenient_clamp_band_scene() -> (SceneEcs, Uuid, Uuid, Uuid) {
    let user = Uuid::from_u128(8);
    let scene_id = Uuid::from_u128(20);
    let token_id = Uuid::from_u128(21);
    let mut tok = entity_doc_eng(
        21,
        20,
        "token",
        json!({ "x": 100.0, "y": 100.0, "w": 1.0, "h": 1.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let scene = entity_doc_top_eng(
        20,
        "scene",
        json!({ "grid": { "kind": "square", "size": 1 }, "background": null,
                "bounds": { "width": 1999.0, "height": 1999.0 } }),
    );
    let mut ecs = SceneEcs::from_documents(vec![scene, tok], 0);
    ecs.set_world_settings_for_test(json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": false, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "movementModel": "grid-stepped",
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    (ecs, user, scene_id, token_id)
}

/// The scan box `strict_lenient_clamp_band_scene`'s single source produces, and the strict/
/// lenient spans that box's candidate scan enumerates. Every input is READ from the fixture —
/// the resolved scene settings, the scene's own grid size, the resolved grid shape, the
/// token's own position, and `source_los_poly` itself — rather than restated as a literal, so
/// a change to `VISION_BOUND_MARGIN`, the fixture's authored bounds, its token position, or
/// its grid size changes what this computes too, instead of leaving it stale.
fn strict_lenient_band_span(ecs: &SceneEcs, scene: Uuid, token: Uuid) -> (i64, i64) {
    let settings = ecs.resolve_scene(scene);
    let cell = *ecs
        .scene_grid_sizes()
        .get(&scene)
        .expect("the fixture's scene has a grid size");
    let grid = ecs.resolve_grid_shape(scene, cell);
    let vp = ecs
        .token_position(token)
        .expect("the fixture's token has a position");
    let walls = ecs.sight_walls(scene);
    let poly = source_los_poly(
        vp,
        &walls,
        settings.los_restriction,
        grid.world_extent(settings.bounds),
    );
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(x, y) in &poly {
        minx = minx.min(x);
        miny = miny.min(y);
        maxx = maxx.max(x);
        maxy = maxy.max(y);
    }
    let bbox = ((minx, miny), (maxx, maxy));
    let padded = crate::scene::explored::pad_box(bbox, cell);
    let strict_span = grid_shape::candidate_span(grid.cell_bounds(bbox.0, bbox.1, cell));
    let lenient_span = grid_shape::candidate_span(grid.cell_bounds(padded.0, padded.1, cell));
    (strict_span, lenient_span)
}

#[test]
fn lenient_visibility_scan_stays_a_superset_of_strict_at_the_clamp_boundary() {
    // The strict scan's own span sits exactly at the cap; the lenient scan's own (padded)
    // span exceeds it. Discrimination: fails if the clamp decision for each invocation is
    // computed from that invocation's own box instead of a box shared across both — the
    // unclamped strict result then reaches a candidate column the clamped lenient result's
    // own (independently-decided) window never enumerates, and `is_subset` catches it.
    //
    // The fixture's whole value depends on actually landing in the band — `strict_lenient_band_span`
    // reads every input from the fixture itself rather than restating one, so a change to
    // `VISION_BOUND_MARGIN`, the authored bounds, the token position, or the grid size that
    // moves the scene out of the band fails this test's two span assertions instead of
    // leaving them vacuously true.
    let (ecs, user, scene, token) = strict_lenient_clamp_band_scene();
    let (strict_span, lenient_span) = strict_lenient_band_span(&ecs, scene, token);
    assert!(
        strict_span <= crate::scene::explored::MAX_CELLS_PER_POLYGON,
        "fixture: the strict span must sit at or under the cap ({strict_span})"
    );
    assert!(
        lenient_span > crate::scene::explored::MAX_CELLS_PER_POLYGON,
        "fixture: the padded span must exceed the cap ({lenient_span})"
    );
    let strict = ecs.visible_cells(user, scene, false);
    let lenient = ecs.visible_cells(user, scene, true);
    assert!(
        !strict.is_empty(),
        "the strict scan must reach at least one cell"
    );
    assert!(
        strict.is_subset(&lenient),
        "strict must never see a cell the lenient scan does not"
    );
    let outside = crate::scene::explored::SCAN_WINDOW_HALF_CELLS as i32 + 200;
    assert!(
        !strict.contains(&(outside, 0)),
        "a cell well beyond the window is not in the strict mask — proves the clamp binds"
    );
}

#[test]
fn parity_holds_inside_the_clamp_band() {
    // `player_lit_mask` and the strict `visible_cells` scan must enumerate identical candidate
    // sets for the same source (`cell_visible`'s own doc states this as an invariant); pinned
    // specifically inside the clamp band, not only in the scenes outside it the other
    // `assert_strict_parity` call sites already cover.
    let (ecs, user, scene, _token) = strict_lenient_clamp_band_scene();
    assert_strict_parity(&ecs, user, scene);
}

#[test]
fn scene_world_extent_agrees_with_the_shapes_own_conversion() {
    // Two call shapes exist — the ECS helper for callers holding only a scene id, and the
    // inline `grid.world_extent(settings.bounds)` for callers already holding both — and a
    // divergence between them would fork the vision bound from the lit mask.
    // Discrimination: fails if either shape starts reading a different bounds value or a
    // different shape, which is the only way the two can disagree.
    let (ecs, _user, scene) = hex_open_scene();
    // The cell is read from the grid-size lookup, not restated: that is the resolution the
    // production sites perform, so an inline arm using a literal would not be the arm that
    // runs in production.
    let cell = *ecs
        .scene_grid_sizes()
        .get(&scene)
        .expect("the fixture's scene has a grid size");
    let inline = ecs
        .resolve_grid_shape(scene, cell)
        .world_extent(ecs.resolve_scene(scene).bounds);
    assert_eq!(ecs.scene_world_extent(scene), inline);
}

#[test]
fn hex_continuous_routes_along_axial_row_zero_strictly_inside_the_mesh() {
    // Every hex in axial row `r = 0` has centre `y` exactly `0`, and the envelope the mesh is
    // triangulated from reaches `y = -size` — the origin row's own bottom circumradius — so
    // those centres sit one circumradius ABOVE the mesh's bottom edge, strictly interior.
    // Their routability therefore rests on the mesh containing them, not on whether the
    // routing library's point-in-polygon test admits an exactly-on-boundary point. Pinned
    // rather than assumed: an envelope that stopped covering the origin row would make an
    // entire authored hex row unroutable with nothing else in the tree failing.
    // Discrimination: the endpoints are `cell_center` values with `y == 0.0` asserted, so the
    // test cannot drift onto an interior row and keep passing; the interior-margin assertion
    // is read from the scene's own converted envelope, so it fails if the minimum moves back
    // to the origin; and the cost is bounded on both sides by the straight-line distance, so a
    // route that detoured off the row fails too.
    let g = grid_shape::HexGrid {
        size: HEX_FIXTURE_SIZE,
    };
    let docs = vec![entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "hex", "size": g.size }, "background": null,
                "bounds": { "width": 20.0, "height": 20.0 },
                "vision": { "movementModel": "continuous" } }),
    )];
    let mut ecs = SceneEcs::from_documents(docs, 0);
    ecs.set_world_settings_for_test(continuous_world_settings());
    let corner = g.cell_center((0, 0));
    let far = g.cell_center((5, 0));
    assert_eq!(
        (corner.1, far.1),
        (0.0, 0.0),
        "fixture: both endpoints must sit on axial row 0"
    );
    assert_eq!(
        corner,
        (0.0, 0.0),
        "fixture: the origin hex is at the origin"
    );
    // The row is strictly interior: the envelope's bottom edge sits a full circumradius under
    // these centres, and its left edge half the flats to the left of the leftmost one.
    let envelope = ecs.scene_world_extent(Uuid::from_u128(10));
    assert!(
        envelope.min.1 < corner.1 - g.size * 0.99 && envelope.min.0 < corner.0,
        "the origin row must sit strictly inside the envelope {envelope:?}"
    );
    // Pure-polyanya sub-path (no region docs): `out.cost` is the `pathfind` boundary's
    // cell-converted value, so the straight-line comparison must divide through the same
    // `world_units_per_cell` conversion rather than comparing against the raw scene-unit span.
    let straight_cells = (far.0 - corner.0) / g.world_units_per_cell();
    for (from, to, label) in [
        (corner, far, "outward along row 0"),
        (far, corner, "inward along row 0"),
    ] {
        let out = ecs
            .pathfind(
                RouteRequester {
                    user: Uuid::from_u128(1),
                    is_gm: true,
                    explored: None,
                },
                Uuid::from_u128(10),
                from,
                &[to],
                0.1,
            )
            .unwrap_or_else(|e| panic!("routing {label} along row 0 must succeed, got {e:?}"));
        assert!(
            out.cost >= straight_cells * 0.99 && out.cost <= straight_cells * 1.01,
            "routing {label} must cost the straight-line distance {straight_cells} cells, got {}",
            out.cost
        );
    }
}

#[test]
fn hex_continuous_routes_below_the_origin_row_inside_its_own_hexes() {
    // The behaviour the envelope buys, at the consumer that pays for it most sharply: two
    // points strictly BELOW `y = 0`, both inside axial row 0's own hexes, are on the mesh and
    // route between each other. A mesh triangulated from an origin-anchored rectangle starts
    // at `y = 0`, so both endpoints would be off-mesh and the route would report unreachable.
    // Discrimination: the endpoints are derived from `cell_center` plus a fraction of the
    // circumradius, so they sit inside the authored hexes by construction; the fixture guards
    // assert both are below `y = 0` and inside the hexes the envelope must cover, so the test
    // cannot drift onto an interior row; and the cost is bounded on both sides by the
    // straight-line distance, so a route detouring up over `y = 0` fails too. Mutating
    // `HexGrid::world_extent`'s `min` to `(0.0, 0.0)` fails it.
    let g = grid_shape::HexGrid {
        size: HEX_FIXTURE_SIZE,
    };
    let docs = vec![entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "hex", "size": g.size }, "background": null,
                "bounds": { "width": 20.0, "height": 20.0 },
                "vision": { "movementModel": "continuous" } }),
    )];
    let mut ecs = SceneEcs::from_documents(docs, 0);
    ecs.set_world_settings_for_test(continuous_world_settings());
    // Half a circumradius below each hex's centre is well inside that hex (the nearest edge on
    // that bearing is the inradius, `√3/2·size`, away) and well below `y = 0`.
    let drop = g.size * 0.5;
    let from = {
        let c = g.cell_center((1, 0));
        (c.0, c.1 - drop)
    };
    let to = {
        let c = g.cell_center((6, 0));
        (c.0, c.1 - drop)
    };
    assert!(
        from.1 < 0.0 && to.1 < 0.0,
        "fixture: both endpoints must sit below the origin, got {from:?} and {to:?}"
    );
    assert_eq!(
        (g.cell_of(from), g.cell_of(to)),
        ((1, 0), (6, 0)),
        "fixture: both endpoints must sit inside axial row 0's own hexes"
    );
    let out = ecs
        .pathfind(
            RouteRequester {
                user: Uuid::from_u128(1),
                is_gm: true,
                explored: None,
            },
            Uuid::from_u128(10),
            from,
            &[to],
            0.1,
        )
        .expect("a position inside an authored hex must be on-mesh and routable");
    // Same conversion as `hex_continuous_routes_along_axial_row_zero_strictly_inside_the_mesh`:
    // `out.cost` is cell-converted at the `pathfind` boundary, so the comparison value must be
    // too.
    let straight_cells = (to.0 - from.0) / g.world_units_per_cell();
    assert!(
        out.cost >= straight_cells * 0.99 && out.cost <= straight_cells * 1.01,
        "the route must run straight below the origin row at cost {straight_cells} cells, got {}",
        out.cost
    );
}

#[test]
fn hex_continuous_navmesh_spans_the_authored_play_area() {
    // A hex scene authored a square block of grid units must route to a hex near the far edge
    // of that authored area. Hex (18,1)'s centre sits beyond the product of the authored bound
    // and the cell size, so a rectangle built from that product excludes the destination and
    // the route reports unreachable.
    // Discrimination: fails if `world_extent` returns the bounds×size product on hex, because
    // the destination is derived from `cell_center`, not from the extent. The guard's product
    // is computed from the block and the shape's own size rather than restated, so raising
    // either cannot leave it expressing a smaller bound than the scene actually declares.
    let g = grid_shape::HexGrid {
        size: HEX_FIXTURE_SIZE,
    };
    let block_cells = 20.0_f64;
    let docs = vec![entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "hex", "size": g.size }, "background": null,
                "bounds": { "width": block_cells, "height": block_cells },
                "vision": { "movementModel": "continuous" } }),
    )];
    let mut ecs = SceneEcs::from_documents(docs, 0);
    ecs.set_world_settings_for_test(continuous_world_settings());
    let dest = g.cell_center((18, 1));
    let product = block_cells * g.size;
    assert!(
        dest.0 > product,
        "fixture: the destination must sit beyond the bounds×size product ({product}), got {}",
        dest.0
    );
    let out = ecs
        .pathfind(
            RouteRequester {
                user: Uuid::from_u128(1),
                is_gm: true,
                explored: None,
            },
            Uuid::from_u128(10),
            g.cell_center((1, 1)),
            &[dest],
            0.1,
        )
        .expect("a hex cell inside the authored bounds must be routable");
    assert!(
        out.path.len() >= 2,
        "route must reach the destination, got {:?}",
        out.path
    );
}

#[test]
fn hex_continuous_weighted_cost_is_reported_in_cells() {
    // A terrain region flips the continuous dispatch to the weighted grid sub-path, whose
    // cost is `pathfinding::find`'s own unit (cells) — `PathResult`'s wire contract, no
    // conversion. The comparison value below converts the straight-line scene-unit distance
    // between the endpoints through the same `world_units_per_cell` (on hex, √3·size per
    // step) so both sides of the assertion share a unit.
    // Discrimination: the expectation is LOWER-BOUNDED by the straight-line distance between
    // the two endpoints, computed from `cell_center`, not from the router's own output.
    let g = grid_shape::HexGrid {
        size: HEX_FIXTURE_SIZE,
    };
    let mut docs = vec![entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "hex", "size": g.size }, "background": null,
                "bounds": { "width": 20.0, "height": 20.0 },
                "vision": { "movementModel": "continuous" } }),
    )];
    // A terrain region well away from the route: present only to select the weighted path.
    docs.push(region_doc_top(
        13,
        10,
        "terrain",
        5.0,
        RegionRect {
            x0: 1200.0,
            y0: 600.0,
            x1: 1260.0,
            y1: 660.0,
        },
    ));
    let mut ecs = SceneEcs::from_documents(docs, 0);
    ecs.set_world_settings_for_test(continuous_world_settings());
    // The whole test is about the WEIGHTED sub-path, which only runs when the dispatch
    // predicate fires. Asserted rather than assumed: with an empty field the pure-polyanya
    // path runs instead and the cost assertion would be measuring a different function.
    let field = ecs
        .region_field(Uuid::from_u128(10), None)
        .expect("the fixture's scene resolves a region field");
    assert!(
        field.has_terrain_or_impassable(),
        "fixture: the terrain region must select the weighted sub-path"
    );
    let a = g.cell_center((1, 1));
    let b = g.cell_center((10, 1));
    let straight_cells =
        ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt() / g.world_units_per_cell();
    let out = ecs
        .pathfind(
            RouteRequester {
                user: Uuid::from_u128(1),
                is_gm: true,
                explored: None,
            },
            Uuid::from_u128(10),
            a,
            &[b],
            0.1,
        )
        .expect("hex continuous weighted route");
    // Bounded on BOTH sides. The endpoints are nine collinear hex steps apart with no terrain
    // between them, so the true cell cost is exactly the straight-line distance; a lower
    // bound alone also passes for any wrong-but-larger factor, `2·size` included.
    assert!(
        out.cost >= straight_cells * 0.99 && out.cost <= straight_cells * 1.01,
        "cost {} must equal the straight-line cell distance {straight_cells}",
        out.cost
    );
}

#[test]
fn a_degenerate_authored_grid_size_never_reaches_the_extent_conversion() {
    // Why the degenerate-`cell` refusal has no expression at `navmesh_for`: `scene_grid_sizes`
    // filters a non-positive authored size and substitutes the positive default, so the value
    // `navmesh_for` converts through `world_extent` is always positive and the resulting extent
    // is never degenerate. The refusal itself lives on `build_navmesh`'s extent parameter,
    // pinned by `navmesh::tests::degenerate_extent_fails_closed` at the level that value
    // enters.
    // Discrimination: fails if `scene_grid_sizes` ever starts passing a non-positive authored
    // size through, which would make a collapsed rectangle reachable from a scene document —
    // and the second assertion fails if the substituted size stops producing a usable mesh.
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 0.0 }, "background": null,
                "bounds": { "width": 10.0, "height": 10.0 } }),
    );
    let ecs = SceneEcs::from_documents(vec![scene], 0);
    let cell = ecs
        .scene_grid_sizes()
        .get(&Uuid::from_u128(10))
        .copied()
        .expect("a live scene always carries a grid size");
    assert!(
        cell > 0.0,
        "a non-positive authored grid size must be hardened before it converts, got {cell}"
    );
    let e = ecs.scene_world_extent(Uuid::from_u128(10));
    assert!(
        e.width() > 0.0 && e.height() > 0.0,
        "the converted envelope is therefore never degenerate, got {e:?}"
    );
    assert!(ecs.navmesh_for(Uuid::from_u128(10), 0.4, &[]).is_some());
}

#[test]
fn navmesh_for_refuses_a_radius_over_the_footprint_cap() {
    // The radius-RANGE refusal is `navmesh_for`'s own: `build_navmesh` receives an
    // already-converted world distance and refuses only on that distance's magnitude, so an
    // over-cap radius whose converted distance stays under `MAX_NAVMESH_COORD` would build a
    // mesh if `navmesh_for` stopped checking the range.
    // Discrimination: the radius is derived from `MAX_FOOTPRINT_CELLS` itself, and the
    // in-range sibling assertion fails if the guard is widened into rejecting
    // legitimate radii.
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                "bounds": { "width": 10.0, "height": 10.0 } }),
    );
    let ecs = SceneEcs::from_documents(vec![scene], 0);
    let over_cap = crate::scene::pathfinding::MAX_FOOTPRINT_CELLS + 1.0;
    assert!(ecs
        .navmesh_for(Uuid::from_u128(10), over_cap, &[])
        .is_none());
    assert!(ecs
        .navmesh_for(
            Uuid::from_u128(10),
            crate::scene::pathfinding::MAX_FOOTPRINT_CELLS,
            &[]
        )
        .is_some());
}

#[test]
fn navmesh_for_refuses_a_scene_whose_converted_extent_is_over_magnitude() {
    // The magnitude bound on the CONVERSION, pinned where the conversion now happens: neither
    // the authored bound nor the cell size alone is oversized, but `world_extent`'s product
    // exceeds `navmesh::MAX_NAVMESH_COORD`, which saturates on the `f64 -> f32` cast and
    // panics inside the triangulation.
    // Discrimination: the sibling assertion uses the same cell size with a bound small enough
    // to keep the product under the ceiling, so a guard that refused on the cell size alone
    // fails it.
    let over = entity_doc_top_eng(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 1e10 }, "background": null,
                "bounds": { "width": 1e10, "height": 100.0 } }),
    );
    let ecs = SceneEcs::from_documents(vec![over], 0);
    assert!(ecs.navmesh_for(Uuid::from_u128(10), 0.4, &[]).is_none());

    let under = entity_doc_top_eng(
        11,
        "scene",
        json!({ "grid": { "kind": "square", "size": 1e10 }, "background": null,
                "bounds": { "width": 10.0, "height": 10.0 } }),
    );
    let ecs = SceneEcs::from_documents(vec![under], 0);
    assert!(ecs.navmesh_for(Uuid::from_u128(11), 0.4, &[]).is_some());
}
