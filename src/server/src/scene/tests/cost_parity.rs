use super::*;
use crate::scene::pathfinding::MoveTraits;
use serde_json::json;

/// Route a path with the router, execute it with the executor, and assert the
/// two costs agree — per diagonal rule, on a grid-stepped scene with terrain.
#[test]
fn router_preview_cost_equals_executor_cost_per_diagonal_rule() {
    for rule in ["chebyshev", "manhattan", "euclidean", "alternating"] {
        let mut ecs = SceneEcs::new();
        ecs.set_world_settings_for_test(ws_body(&[("/pathfinding/diagonalRule", json!(rule))]));
        let scene_id = Uuid::from_u128(1);
        ecs.insert_scene_for_test(
            scene_id,
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                "vision": { "movementRestriction": "unrestricted" } }),
        );
        let token = entity_doc_eng(
            2,
            1,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        ecs.apply_op(&Operation::Create { doc: token });
        let region = region_doc(3, 1, "terrain", 2.0, (200.0, 0.0, 400.0, 400.0));
        ecs.apply_op(&Operation::Create { doc: region });
        let gm = Uuid::from_u128(9);
        let route = ecs
            .pathfind(
                RouteRequester {
                    user: gm,
                    is_gm: true,
                    world_role: WorldRole::Gm,
                    world_defaults: &no_world_grants(),
                    explored: None,
                },
                scene_id,
                (50.0, 50.0),
                &[(450.0, 350.0)],
                crate::scene::RouteMover {
                    footprint_radius: 0.4,
                    budget_cells: None,
                    traits: MoveTraits::default(),
                },
            )
            .expect("routable");
        let out = crate::scene::move_exec::execute_move(
            &ecs,
            crate::scene::move_exec::MoveGateInputs {
                scene: scene_id,
                restriction: MovementRestriction::Unrestricted,
                visible: &Default::default(),
                cell: 100.0,
                budget: None,
                traits: MoveTraits::default(),
            },
            Uuid::from_u128(2),
            &route.path,
            true,
            0.4,
        )
        .expect("admissible");
        assert!(
            (route.cost - out.cost).abs() < 1e-9,
            "{rule}: preview {} vs execution {}",
            route.cost,
            out.cost
        );
    }
}

#[test]
fn continuous_smoothed_preview_cost_equals_executor_cost() {
    let mut ecs = SceneEcs::new();
    ecs.set_world_settings_for_test(ws_body(&[("/scene/movementModel", json!("continuous"))]));
    let scene_id = Uuid::from_u128(1);
    ecs.insert_scene_for_test(
        scene_id,
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
            "bounds": { "width": 10, "height": 10 },
            "vision": { "movementRestriction": "unrestricted" } }),
    );
    ecs.apply_op(&Operation::Create {
        doc: entity_doc_eng(
            2,
            1,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        ),
    });
    ecs.apply_op(&Operation::Create {
        doc: region_doc(3, 1, "terrain", 3.0, (300.0, 0.0, 400.0, 1000.0)),
    });
    let route = ecs
        .pathfind(
            RouteRequester {
                user: Uuid::from_u128(9),
                is_gm: true,
                world_role: WorldRole::Gm,
                world_defaults: &no_world_grants(),
                explored: None,
            },
            scene_id,
            (50.0, 50.0),
            &[(650.0, 250.0)],
            crate::scene::RouteMover {
                footprint_radius: 0.4,
                budget_cells: None,
                traits: MoveTraits::default(),
            },
        )
        .expect("routable");
    let out = crate::scene::move_exec::execute_move(
        &ecs,
        crate::scene::move_exec::MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Unrestricted,
            visible: &Default::default(),
            cell: 100.0,
            budget: None,
            traits: MoveTraits::default(),
        },
        Uuid::from_u128(2),
        &route.path,
        true,
        0.4,
    )
    .expect("admissible");
    assert!(
        (route.cost - out.cost).abs() < 1e-9,
        "preview {} vs execution {}",
        route.cost,
        out.cost
    );
}

/// `ignore_terrain` flags — the shape both seams hand the router and executor for a mover
/// carrying a reserved terrain-exempt tag.
const EXEMPT_TRAITS: MoveTraits = MoveTraits {
    ignore_terrain: true,
};

#[test]
fn exempt_mover_grid_preview_equals_executor_and_both_read_unweighted() {
    // The SAME scene and terrain the non-exempt parity test routes through: the exempt mover's
    // preview and execution agree with each other AND with the unweighted king distance — while
    // the non-exempt control pays strictly more, so the exemption is what did the work.
    let mut ecs = SceneEcs::new();
    ecs.set_world_settings_for_test(ws_body(&[(
        "/pathfinding/diagonalRule",
        json!("chebyshev"),
    )]));
    let scene_id = Uuid::from_u128(1);
    ecs.insert_scene_for_test(
        scene_id,
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
            "vision": { "movementRestriction": "unrestricted" } }),
    );
    ecs.apply_op(&Operation::Create {
        doc: entity_doc_eng(
            2,
            1,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        ),
    });
    ecs.apply_op(&Operation::Create {
        doc: region_doc(3, 1, "terrain", 2.0, (200.0, 0.0, 400.0, 400.0)),
    });
    let grants = no_world_grants();
    let requester = || RouteRequester {
        user: Uuid::from_u128(9),
        is_gm: true,
        world_role: WorldRole::Gm,
        world_defaults: &grants,
        explored: None,
    };
    let grounded_route = ecs
        .pathfind(
            requester(),
            scene_id,
            (50.0, 50.0),
            &[(450.0, 350.0)],
            crate::scene::RouteMover {
                footprint_radius: 0.4,
                budget_cells: None,
                traits: MoveTraits::default(),
            },
        )
        .expect("routable");
    assert!(
        grounded_route.cost > 4.0,
        "the ground mover pays the terrain surcharge, got {}",
        grounded_route.cost
    );
    let route = ecs
        .pathfind(
            requester(),
            scene_id,
            (50.0, 50.0),
            &[(450.0, 350.0)],
            crate::scene::RouteMover {
                footprint_radius: 0.4,
                budget_cells: None,
                traits: EXEMPT_TRAITS,
            },
        )
        .expect("routable");
    assert!(
        (route.cost - 4.0).abs() < 1e-9,
        "exempt preview is the unweighted Chebyshev distance, got {}",
        route.cost
    );
    let out = crate::scene::move_exec::execute_move(
        &ecs,
        crate::scene::move_exec::MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Unrestricted,
            visible: &Default::default(),
            cell: 100.0,
            budget: None,
            traits: EXEMPT_TRAITS,
        },
        Uuid::from_u128(2),
        &route.path,
        true,
        0.4,
    )
    .expect("admissible");
    assert!(
        (route.cost - out.cost).abs() < 1e-9,
        "preview {} vs execution {}",
        route.cost,
        out.cost
    );
}

#[test]
fn exempt_mover_continuous_takes_the_plain_any_angle_route_through_terrain() {
    // The SAME continuous terrain scene the non-exempt parity test uses: with only weighted
    // terrain present, the exempt mover does NOT dispatch the weighted grid sub-path — its
    // route is the straight polyanya chord, priced (and executed) unweighted.
    //
    // The goal sits OFF its cell center on purpose: the pure-polyanya route ends at the literal
    // goal, whereas the weighted grid sub-path snaps every waypoint to its containing cell's
    // center (`pathfinding::find`'s per-waypoint contract) and `los_smooth` can only straighten
    // between those snapped vertices. A goal on a cell center would let both sub-paths collapse
    // to the same `[start, goal]` chord at the same cost, and the dispatch fork under test would
    // be indistinguishable from an always-weighted one.
    let mut ecs = SceneEcs::new();
    ecs.set_world_settings_for_test(ws_body(&[("/scene/movementModel", json!("continuous"))]));
    let scene_id = Uuid::from_u128(1);
    ecs.insert_scene_for_test(
        scene_id,
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
            "bounds": { "width": 10, "height": 10 },
            "vision": { "movementRestriction": "unrestricted" } }),
    );
    ecs.apply_op(&Operation::Create {
        doc: entity_doc_eng(
            2,
            1,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        ),
    });
    ecs.apply_op(&Operation::Create {
        doc: region_doc(3, 1, "terrain", 3.0, (300.0, 0.0, 400.0, 1000.0)),
    });
    let route = ecs
        .pathfind(
            RouteRequester {
                user: Uuid::from_u128(9),
                is_gm: true,
                world_role: WorldRole::Gm,
                world_defaults: &no_world_grants(),
                explored: None,
            },
            scene_id,
            (50.0, 50.0),
            &[(640.0, 260.0)],
            crate::scene::RouteMover {
                footprint_radius: 0.4,
                budget_cells: None,
                traits: EXEMPT_TRAITS,
            },
        )
        .expect("routable");
    assert_eq!(
        route.path.len(),
        2,
        "terrain alone never bends an exempt mover's route: the straight chord survives"
    );
    let goal = route.path[1];
    assert!(
        (goal.0 - 640.0).abs() < 1e-9 && (goal.1 - 260.0).abs() < 1e-9,
        "the pure any-angle route ends at the literal off-center goal; the weighted sub-path would have snapped it to the cell center (650, 250), got {goal:?}"
    );
    let expected = (590.0f64.hypot(210.0)) / 100.0;
    assert!(
        (route.cost - expected).abs() < 1e-9,
        "exempt preview is the raw Euclidean span, got {}",
        route.cost
    );
    let out = crate::scene::move_exec::execute_move(
        &ecs,
        crate::scene::move_exec::MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Unrestricted,
            visible: &Default::default(),
            cell: 100.0,
            budget: None,
            traits: EXEMPT_TRAITS,
        },
        Uuid::from_u128(2),
        &route.path,
        true,
        0.4,
    )
    .expect("admissible");
    assert!(
        (route.cost - out.cost).abs() < 1e-9,
        "preview {} vs execution {}",
        route.cost,
        out.cost
    );
}

#[test]
fn exempt_mover_continuous_still_routes_around_impassable() {
    // Impassable-only field, exempt mover: the exemption is terrain COST, never solidity — the
    // weighted sub-path still dispatches, the route bends around the blocked band, and preview
    // and execution agree.
    let mut ecs = SceneEcs::new();
    ecs.set_world_settings_for_test(ws_body(&[("/scene/movementModel", json!("continuous"))]));
    let scene_id = Uuid::from_u128(1);
    ecs.insert_scene_for_test(
        scene_id,
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
            "bounds": { "width": 10, "height": 10 },
            "vision": { "movementRestriction": "unrestricted" } }),
    );
    ecs.apply_op(&Operation::Create {
        doc: entity_doc_eng(
            2,
            1,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        ),
    });
    ecs.apply_op(&Operation::Create {
        doc: region_doc(3, 1, "impassable", 1.0, (300.0, 0.0, 400.0, 700.0)),
    });
    let route = ecs
        .pathfind(
            RouteRequester {
                user: Uuid::from_u128(9),
                is_gm: true,
                world_role: WorldRole::Gm,
                world_defaults: &no_world_grants(),
                explored: None,
            },
            scene_id,
            (50.0, 50.0),
            &[(650.0, 250.0)],
            crate::scene::RouteMover {
                footprint_radius: 0.4,
                budget_cells: None,
                traits: EXEMPT_TRAITS,
            },
        )
        .expect("a route around the band exists");
    assert!(
        route.path.len() > 2,
        "the exempt mover still detours: impassable is not terrain COST, got {:?}",
        route.path
    );
    let out = crate::scene::move_exec::execute_move(
        &ecs,
        crate::scene::move_exec::MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Unrestricted,
            visible: &Default::default(),
            cell: 100.0,
            budget: None,
            traits: EXEMPT_TRAITS,
        },
        Uuid::from_u128(2),
        &route.path,
        true,
        0.4,
    )
    .expect("admissible");
    assert!(
        (route.cost - out.cost).abs() < 1e-9,
        "preview {} vs execution {}",
        route.cost,
        out.cost
    );
}

#[test]
fn exempt_mover_continuous_weighted_subpath_chords_through_terrain_at_unweighted_cost() {
    // Impassable AND weighted terrain on a continuous scene: the impassable band forces the
    // weighted grid sub-path for every mover, then `los_smooth` straightens it. For the exempt
    // mover the chord rule ignores terrain and every span prices at 1.0, so the preview cost is
    // the route's raw Euclidean length in cells, strictly below the ground mover's — and the
    // executor reproduces it exactly (the tail/transition pricing reads the same flag).
    let mut ecs = SceneEcs::new();
    ecs.set_world_settings_for_test(ws_body(&[("/scene/movementModel", json!("continuous"))]));
    let scene_id = Uuid::from_u128(1);
    ecs.insert_scene_for_test(
        scene_id,
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
            "bounds": { "width": 10, "height": 10 },
            "vision": { "movementRestriction": "unrestricted" } }),
    );
    ecs.apply_op(&Operation::Create {
        doc: entity_doc_eng(
            2,
            1,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        ),
    });
    ecs.apply_op(&Operation::Create {
        doc: region_doc(3, 1, "impassable", 1.0, (300.0, 0.0, 400.0, 700.0)),
    });
    ecs.apply_op(&Operation::Create {
        doc: region_doc(4, 1, "terrain", 3.0, (500.0, 0.0, 600.0, 1000.0)),
    });
    let grants = no_world_grants();
    let requester = || RouteRequester {
        user: Uuid::from_u128(9),
        is_gm: true,
        world_role: WorldRole::Gm,
        world_defaults: &grants,
        explored: None,
    };
    let goal = (650.0, 250.0);
    let grounded = ecs
        .pathfind(
            requester(),
            scene_id,
            (50.0, 50.0),
            &[goal],
            crate::scene::RouteMover {
                footprint_radius: 0.4,
                budget_cells: None,
                traits: MoveTraits::default(),
            },
        )
        .expect("routable");
    let route = ecs
        .pathfind(
            requester(),
            scene_id,
            (50.0, 50.0),
            &[goal],
            crate::scene::RouteMover {
                footprint_radius: 0.4,
                budget_cells: None,
                traits: EXEMPT_TRAITS,
            },
        )
        .expect("routable");
    assert!(
        route.path.len() > 2,
        "impassable still forces the detour for the exempt mover, got {:?}",
        route.path
    );
    assert!(
        route.path.len() < grounded.path.len(),
        "the chord rule ignores terrain for the exempt mover, so its route keeps fewer grid steps than the ground mover's: {} vs {} vertices",
        route.path.len(),
        grounded.path.len()
    );
    let raw_len_cells: f64 = route
        .path
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1) / 100.0)
        .sum();
    assert!(
        (route.cost - raw_len_cells).abs() < 1e-9,
        "exempt preview is the unweighted length of its own polyline: {} vs {}",
        route.cost,
        raw_len_cells
    );
    assert!(
        route.cost < grounded.cost,
        "the ground mover pays the terrain band the exempt mover chords through: {} vs {}",
        grounded.cost,
        route.cost
    );
    let out = crate::scene::move_exec::execute_move(
        &ecs,
        crate::scene::move_exec::MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Unrestricted,
            visible: &Default::default(),
            cell: 100.0,
            budget: None,
            traits: EXEMPT_TRAITS,
        },
        Uuid::from_u128(2),
        &route.path,
        true,
        0.4,
    )
    .expect("admissible");
    assert!(
        (route.cost - out.cost).abs() < 1e-9,
        "preview {} vs execution {}",
        route.cost,
        out.cost
    );
}
