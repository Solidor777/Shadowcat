use super::*;
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
                    explored: None,
                },
                scene_id,
                (50.0, 50.0),
                &[(450.0, 350.0)],
                0.4,
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
                explored: None,
            },
            scene_id,
            (50.0, 50.0),
            &[(650.0, 250.0)],
            0.4,
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
        },
        Uuid::from_u128(2),
        &route.path,
        true,
        0.4,
    )
    .expect("admissible");
    assert!(
        (route.cost - out.cost).abs() < 1e-6,
        "preview {} vs execution {}",
        route.cost,
        out.cost
    );
}
