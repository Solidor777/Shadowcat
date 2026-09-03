use super::*;
use crate::scene::pathfinding::MoveTraits;
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
                traits: MoveTraits::default(),
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
            traits: MoveTraits::default(),
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
            traits: MoveTraits::default(),
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
            traits: MoveTraits::default(),
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
fn gm_move_ignores_budget_but_still_accrues_cost() {
    // Cost is information, not a gate — a GM's move never truncates on budget, mirroring
    // `gm_move_accrues_terrain_cost`'s precedent that terrain accrual is independent of the
    // gameplay exemption.
    let (ecs, scene, token) = scene_with_terrain_multiplier_3();
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &empty_mask(),
            cell: FIXTURE_GRID_SIZE,
            budget: Some(0.0),
            traits: MoveTraits::default(),
        },
        token,
        &[(50.0, 50.0), (150.0, 50.0)],
        true,
        0.4,
    )
    .expect("admissible");
    assert!(!out.truncated, "a GM move is not truncated by budget");
    assert!(
        out.cost >= 3.0,
        "terrain still accrues for a GM, got {}",
        out.cost
    );
}

#[test]
fn non_finite_budget_is_rejected_as_degenerate() {
    let (ecs, scene, token) = clear_scene();
    let err = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &empty_mask(),
            cell: FIXTURE_GRID_SIZE,
            budget: Some(f64::NAN),
            traits: MoveTraits::default(),
        },
        token,
        &[(0.0, 0.0), (100.0, 100.0)],
        false,
        0.4,
    )
    .expect_err("a NaN budget must fail closed, not behave as unlimited");
    assert!(matches!(err, MoveReject::Degenerate), "got {err:?}");
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
            traits: MoveTraits::default(),
        },
        token,
        &[a, b],
        true,
        0.4,
    )
    .expect("admissible");
    assert!((out.cost - 1.0).abs() < 1e-9);
}

/// The exempt-mover flag set: the shape `MoveGateInputs.traits` takes when the caller resolved
/// a terrain-exempt tag off the mover.
const EXEMPT_TRAITS: MoveTraits = MoveTraits {
    ignore_terrain: true,
};

#[test]
fn exempt_mover_pays_unweighted_through_terrain_where_the_ground_mover_pays_the_multiplier() {
    // Both movers walk the SAME step through the SAME ×3 cell: the exemption flattens exactly
    // the terrain cost and nothing else.
    let (ecs, scene, token) = scene_with_terrain_multiplier_3();
    let path = [(50.0, 50.0), (150.0, 50.0)];
    let mask = empty_mask();
    let gate = |traits| MoveGateInputs {
        scene,
        restriction: MovementRestriction::Unrestricted,
        visible: &mask,
        cell: FIXTURE_GRID_SIZE,
        budget: None,
        traits,
    };
    let grounded = execute_move(&ecs, gate(MoveTraits::default()), token, &path, false, 0.4)
        .expect("admissible");
    assert!(
        (grounded.cost - 3.0).abs() < 1e-9,
        "ground mover pays ×3, got {}",
        grounded.cost
    );
    let exempt =
        execute_move(&ecs, gate(EXEMPT_TRAITS), token, &path, false, 0.4).expect("admissible");
    assert!(
        (exempt.cost - 1.0).abs() < 1e-9,
        "exempt mover pays the unweighted step, got {}",
        exempt.cost
    );
    assert_eq!(exempt.stop, (150.0, 50.0));
    assert!(!exempt.truncated);
}

#[test]
fn exempt_mover_budget_stop_uses_the_exempt_cost() {
    // Budget 2 cells over a two-step path through ×3 terrain: the ground mover cannot afford
    // the FIRST step (3 > 2 — `budget_truncates_at_the_last_affordable_step_and_reports_truncated`
    // pins that), while the exempt mover's 1+1 fits exactly.
    let (ecs, scene, token) = scene_with_terrain_multiplier_3();
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &empty_mask(),
            cell: FIXTURE_GRID_SIZE,
            budget: Some(2.0),
            traits: EXEMPT_TRAITS,
        },
        token,
        &[(50.0, 50.0), (150.0, 50.0), (250.0, 50.0)],
        false,
        0.4,
    )
    .expect("admissible");
    assert_eq!(out.stop, (250.0, 50.0), "the exempt cost 1+1 fits budget 2");
    assert!(!out.truncated);
    assert!((out.cost - 2.0).abs() < 1e-9, "got {}", out.cost);
}

#[test]
fn exempt_mover_continuous_transition_and_tail_both_read_unweighted() {
    // A continuous move halting mid-cell inside the ×3 region: `gate_walk` subdivides
    // (50,50)->(160,50) at (105,50), so the cost is a TRANSITION charge (the 0.55-cell span
    // entering the region cell, priced at its multiplier) plus a TAIL charge (the remaining
    // 0.55 cells to the mid-cell stop, priced at the stop cell's multiplier) — both must read
    // the exempt 1.0.
    let (mut ecs, scene, token) = scene_with_terrain_multiplier_3();
    ecs.set_world_settings_for_test(super::super::super::tests::ws_body(&[(
        "/scene/movementModel",
        json!("continuous"),
    )]));
    let mask = empty_mask();
    let gate = |traits| MoveGateInputs {
        scene,
        restriction: MovementRestriction::Unrestricted,
        visible: &mask,
        cell: FIXTURE_GRID_SIZE,
        budget: None,
        traits,
    };
    let path = [(50.0, 50.0), (160.0, 50.0)];
    let grounded = execute_move(&ecs, gate(MoveTraits::default()), token, &path, false, 0.4)
        .expect("admissible");
    assert!(
        (grounded.cost - 1.1 * 3.0).abs() < 1e-9,
        "ground mover pays ×3 on transition AND tail, got {}",
        grounded.cost
    );
    let exempt =
        execute_move(&ecs, gate(EXEMPT_TRAITS), token, &path, false, 0.4).expect("admissible");
    assert!(
        (exempt.cost - 1.1).abs() < 1e-9,
        "exempt mover pays the raw Euclidean span, got {}",
        exempt.cost
    );
}

#[test]
fn exemption_never_covers_impassable_or_arrest() {
    // The reserved tags ignore terrain COST only: the impassable cell still stops the walk
    // before entry and the arrest cell still truncates, exactly as for a ground mover.
    let (ecs, scene, token) = scene_with_impassable_then_arrest_region();
    let mask = empty_mask();
    let gate = || MoveGateInputs {
        scene,
        restriction: MovementRestriction::Unrestricted,
        visible: &mask,
        cell: FIXTURE_GRID_SIZE,
        budget: None,
        traits: EXEMPT_TRAITS,
    };
    let blocked = execute_move(
        &ecs,
        gate(),
        token,
        &[(50.0, 50.0), (150.0, 50.0)],
        false,
        0.4,
    )
    .expect("a stopped walk is still a well-formed outcome");
    assert_eq!(
        blocked.stop,
        (50.0, 50.0),
        "impassable stops an exempt mover at the threshold, exactly as anyone else"
    );
    assert!(blocked.truncated);
    assert!(
        blocked.cost.abs() < 1e-9,
        "nothing entered, nothing charged"
    );

    // Detouring around the impassable cell and then turning INTO the arrest cell: the arrest
    // entry is charged (unweighted) and stops the walk.
    let out = execute_move(
        &ecs,
        gate(),
        token,
        &[
            (50.0, 50.0),
            (50.0, 150.0),
            (150.0, 150.0),
            (250.0, 150.0),
            (250.0, 50.0),
            (350.0, 50.0),
        ],
        false,
        0.4,
    )
    .expect("admissible");
    assert_eq!(out.stop, (250.0, 50.0), "arrest stops the exempt mover too");
    assert!(out.truncated);
    assert!(
        (out.cost - 4.0).abs() < 1e-9,
        "four unweighted steps incl. the arrest entry, got {}",
        out.cost
    );
}
