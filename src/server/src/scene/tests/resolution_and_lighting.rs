//! Scene-settings resolution (diagonal rule, movement restriction/model, bounds), lighting/environment-light occlusion, the movement-gate visibility mask, and the navmesh cache's invalidation/memoization.
use super::*;

#[test]
fn diagonal_rule_defaults_to_chebyshev_without_world_settings() {
    let ecs = SceneEcs::new();
    assert_eq!(
        ecs.resolved_diagonal_rule(),
        crate::scene::pathfinding::DiagonalRule::Chebyshev
    );
}

#[test]
fn partial_world_doc_authored_rule_applies_and_absent_leaf_falls_back() {
    // Overlay semantics: a partial world-settings body contributes exactly its
    // authored leaves — there is no structural completeness requirement.
    use serde_json::json;
    let mut ecs = SceneEcs::new();

    // Authored pathfinding leaf, no scene/animation keys: the leaf applies.
    ecs.set_world_settings_for_test(json!({
        "pathfinding": { "diagonalRule": "alternating" }
    }));
    assert_eq!(
        ecs.resolved_diagonal_rule(),
        crate::scene::pathfinding::DiagonalRule::Alternating,
        "a partial doc's authored leaf applies"
    );

    // No pathfinding leaf at all: the engine literal.
    ecs.set_world_settings_for_test(json!({
        "scene": { "movementRestriction": "visible" }
    }));
    assert_eq!(
        ecs.resolved_diagonal_rule(),
        crate::scene::pathfinding::DiagonalRule::Chebyshev,
        "an unauthored leaf falls back to the engine literal"
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
        "unknown rule fails to chebyshev (the engine literal the client mirrors)"
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
    ecs.set_world_settings_for_test(ws_body(&[("/scene/movementRestriction", json!("visible"))]));
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
            c.0 >= envelope.min.0 - HEX_FIXTURE_SIZE && c.1 >= envelope.min.1 - HEX_FIXTURE_SIZE,
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
    let blocking = json!({ "seg": {"x1": 100, "y1": 0, "x2": 100, "y2": 200}, "blocksMove": true });
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
            crate::data::document::tests::world_scoped_doc(Uuid::from_u128(9), scene_id, "scene"),
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
            crate::data::document::tests::world_scoped_doc(Uuid::from_u128(9), scene_id, "scene"),
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
fn trigger_region_identity_rows_and_the_composed_field_share_one_rasterization() {
    // Anti-drift: `SceneEcs::trigger_regions`'s identity rows and the composed
    // `region_field` must cover the same cells for the same region, because one
    // rasterizer feeds both. A divergence would let a move spring a region's
    // movement behavior without firing its triggers (or the reverse).
    let scene_id = Uuid::from_u128(10);
    let region = |id: u128, x0: f64, behavior: &str, enabled: bool, triggers: serde_json::Value| {
        let mut d = crate::data::document::tests::world_scoped_doc(
            Uuid::from_u128(9),
            Uuid::from_u128(id),
            "region",
        );
        d.parent_id = Some(scene_id);
        d.engine = Some(serde_json::json!({
            "shape": { "kind": "rect", "points": [x0, 0.0, x0 + 100.0, 100.0] },
            "behavior": behavior,
            "cost": 1.0,
            "enabled": enabled,
            "triggers": triggers,
        }));
        d
    };
    let enter_notice = serde_json::json!([
        { "on": "enter", "effect": { "type": "chat_notice", "text": "hi", "audience": "gm_only" } }
    ]);
    let ecs = SceneEcs::from_documents(
        vec![
            crate::data::document::tests::world_scoped_doc(Uuid::from_u128(9), scene_id, "scene"),
            region(20, 0.0, "terrain", true, enter_notice.clone()),
            region(21, 200.0, "arrest", true, enter_notice.clone()),
            region(22, 400.0, "impassable", true, serde_json::json!([])),
            region(23, 600.0, "terrain", false, enter_notice),
        ],
        0,
    );

    let field = ecs.region_field(scene_id, None).expect("scene exists");
    let field_cells: std::collections::BTreeSet<_> = field.iter_cells().map(|(c, _)| c).collect();
    let rows = ecs.trigger_regions(scene_id).expect("scene exists");
    assert_eq!(
        rows.len(),
        2,
        "only enabled, trigger-bearing regions get identity rows"
    );
    for row in &rows {
        assert!(!row.cells.is_empty());
        for cell in &row.cells {
            assert!(
                field_cells.contains(cell),
                "identity row for region {} covers {cell:?} but the composed field does not",
                row.region_id,
            );
        }
    }
    // The trigger-less region is in the field without an identity row; the
    // disabled region is in neither.
    assert!(field_cells.contains(&(4, 0)));
    assert!(!field_cells.contains(&(6, 0)));
    assert!(!rows.iter().any(|r| r.cells.contains(&(4, 0))));
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
