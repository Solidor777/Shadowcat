//! The grid and continuous (navmesh) pathfinders, streamed per-move vision sampling, and hex-grid vision/lighting range parity.
use super::*;

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
        None,
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
            None,
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
fn pathfind_continuous_budget_cut_truncates_the_final_span_at_the_boundary() {
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
                explored: None,
            },
            Uuid::from_u128(10),
            (50.0, 50.0),
            &[(950.0, 50.0)],
            0.1,
            Some(4.0),
        )
        .expect("continuous route over an open bounded scene");
    assert!(outcome.truncated, "a ~9-cell route cuts at a 4-cell budget");
    assert!(
        (outcome.cost - 4.0).abs() < 1e-9,
        "cut exactly at the budget boundary, got {}",
        outcome.cost
    );
    let last = *outcome.path.last().unwrap();
    assert!(
        (last.0 - 450.0).abs() < 1e-6 && (last.1 - 50.0).abs() < 1e-6,
        "the final span is cut 400 scene units from the start, got {last:?}"
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
        .pathfind(requester(), Uuid::from_u128(10), start, &[goal], 0.1, None)
        .expect("grid-stepped straight route");
    let continuous_out = continuous_ecs
        .pathfind(requester(), Uuid::from_u128(10), start, &[goal], 0.1, None)
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
            None,
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
            "x": 50.0, "y": 50.0, "emission": { "color": "#ffffff", "intensity": 1.0, "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true }
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

fn region_doc_top(id: u128, parent: u128, behavior: &str, cost: f64, rect: RegionRect) -> Document {
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
        )
        .expect("clip truncates the route short of the unseen goal rather than failing outright");
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
            "x": 50.0, "y": 50.0, "emission": { "color": "#ffffff", "intensity": 1.0, "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true }
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
            None,
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
        None,
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
            None,
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
            None,
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
            budget: None,
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
            None,
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
            budget: None,
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
            None,
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
        None,
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
        None,
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
        None,
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
    let polys_normal = ecs_normal.player_vision_polygons_at(user, scene, token_id, (50.0, 50.0));
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
        .player_lit_mask(user, &ecs.resolved_bands())
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
        .player_lit_mask(user, &ecs.resolved_bands())
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
            budget: None,
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
            budget: None,
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
            "x": 0.0, "y": 0.0, "emission": { "color": "#ffffff", "intensity": 1.0, "brightRadius": HEX_LIGHT_BRIGHT_CELLS, "dimRadius": HEX_LIGHT_DIM_CELLS, "enabled": true }
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
        .player_lit_mask(user, &ecs.resolved_bands())
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
                None,
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
            None,
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
            None,
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
            None,
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
