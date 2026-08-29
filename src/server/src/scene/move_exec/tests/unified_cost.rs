use super::*;
use serde_json::json;

/// Scene identical to `clear_scene`, with `world-settings.pathfinding.diagonalRule` set to
/// `rule` — every diagonal-rule cost-parity case shares this one scene shape.
fn scene_with_rule(rule: &str) -> (SceneEcs, Uuid, Uuid) {
    let (mut ecs, scene, token) = clear_scene();
    ecs.set_world_settings_for_test(super::super::super::tests::ws_body(&[(
        "/pathfinding/diagonalRule",
        json!(rule),
    )]));
    (ecs, scene, token)
}

#[test]
fn diagonal_steps_are_priced_by_the_world_rule() {
    for (rule, expected) in [
        ("chebyshev", 1.0),
        ("manhattan", 2.0),
        ("euclidean", std::f64::consts::SQRT_2),
        ("alternating", 1.0),
    ] {
        let (ecs, scene, token) = scene_with_rule(rule);
        let out = execute_move(
            &ecs,
            MoveGateInputs {
                scene,
                restriction: MovementRestriction::Unrestricted,
                visible: &empty_mask(),
                cell: FIXTURE_GRID_SIZE,
                budget: None,
            },
            token,
            &[(0.0, 0.0), (100.0, 100.0)],
            true,
            0.4,
        )
        .expect("admissible");
        assert!(
            (out.cost - expected).abs() < 1e-9,
            "{rule}: got {}",
            out.cost
        );
    }
}

#[test]
fn alternating_rule_threads_parity_across_consecutive_diagonals() {
    let (ecs, scene, token) = scene_with_rule("alternating");
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &empty_mask(),
            cell: FIXTURE_GRID_SIZE,
            budget: None,
        },
        token,
        &[(0.0, 0.0), (100.0, 100.0), (200.0, 200.0), (300.0, 300.0)],
        true,
        0.4,
    )
    .expect("admissible");
    assert!((out.cost - 4.0).abs() < 1e-9, "1 + 2 + 1, got {}", out.cost);
}

#[test]
fn budget_truncates_at_the_last_affordable_step_and_reports_truncated() {
    let (ecs, scene, token) = scene_with_terrain_multiplier_3();
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &empty_mask(),
            cell: FIXTURE_GRID_SIZE,
            budget: Some(2.0),
        },
        token,
        &[(50.0, 50.0), (150.0, 50.0), (250.0, 50.0)],
        false,
        0.4,
    )
    .expect("admissible");
    assert_eq!(out.stop, (50.0, 50.0), "the first step costs 3 > budget 2");
    assert!(out.truncated);
    assert!((out.cost).abs() < 1e-9, "nothing walked, nothing charged");
}

#[test]
fn budget_exactly_equal_to_the_cost_is_affordable() {
    let (ecs, scene, token) = scene_with_terrain_multiplier_3();
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &empty_mask(),
            cell: FIXTURE_GRID_SIZE,
            budget: Some(3.0),
        },
        token,
        &[(50.0, 50.0), (150.0, 50.0)],
        false,
        0.4,
    )
    .expect("admissible");
    assert_eq!(out.stop, (150.0, 50.0));
    assert!(!out.truncated);
}

#[test]
fn hex_steps_cost_one_regardless_of_the_square_rule() {
    let (mut ecs, scene, token) = hex_clear_scene();
    ecs.set_world_settings_for_test(super::super::super::tests::ws_body(&[(
        "/pathfinding/diagonalRule",
        json!("manhattan"),
    )]));
    let a = hex_cell_center(0, 0);
    let b = hex_cell_center(1, -1);
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &empty_mask(),
            cell: FIXTURE_GRID_SIZE,
            budget: None,
        },
        token,
        &[a, b],
        true,
        0.4,
    )
    .expect("admissible");
    assert!((out.cost - 1.0).abs() < 1e-9);
}
