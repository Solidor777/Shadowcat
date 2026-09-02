use super::*;
use crate::scene::pathfinding::MoveTraits;
use crate::scene::vision::Seg;

const NO_WALLS: [Seg; 0] = [];

fn sq(cell: f64, rule: DiagonalRule) -> crate::scene::grid_shape::SquareGrid {
    crate::scene::grid_shape::SquareGrid { cell, rule }
}

#[test]
fn empty_waypoints_is_invalid() {
    let r = find(
        (50.0, 50.0),
        &[],
        PathInputs {
            footprint_radius_cells: 0.1,
            cell: 100.0,
            walls: &NO_WALLS,
            mask: None,
            regions: None,
            shape: &sq(100.0, DiagonalRule::Chebyshev),
            budget_cells: None,
            traits: MoveTraits::default(),
        },
    );
    assert_eq!(r, Err(PathFail::Invalid));
}

#[test]
fn nonfinite_or_bad_footprint_is_invalid() {
    // Non-finite start point.
    assert_eq!(
        find(
            (f64::NAN, 0.0),
            &[(150.0, 50.0)],
            PathInputs {
                footprint_radius_cells: 0.1,
                cell: 100.0,
                walls: &NO_WALLS,
                mask: None,
                regions: None,
                shape: &sq(100.0, DiagonalRule::Chebyshev),
                budget_cells: None,
                traits: MoveTraits::default(),
            }
        ),
        Err(PathFail::Invalid)
    );
    // Negative footprint_radius_cells.
    assert_eq!(
        find(
            (50.0, 50.0),
            &[(150.0, 50.0)],
            PathInputs {
                footprint_radius_cells: -1.0,
                cell: 100.0,
                walls: &NO_WALLS,
                mask: None,
                regions: None,
                shape: &sq(100.0, DiagonalRule::Chebyshev),
                budget_cells: None,
                traits: MoveTraits::default(),
            }
        ),
        Err(PathFail::Invalid)
    );
    // Non-positive cell size.
    assert_eq!(
        find(
            (50.0, 50.0),
            &[(150.0, 50.0)],
            PathInputs {
                footprint_radius_cells: 0.1,
                cell: 0.0,
                walls: &NO_WALLS,
                mask: None,
                regions: None,
                shape: &sq(0.0, DiagonalRule::Chebyshev),
                budget_cells: None,
                traits: MoveTraits::default(),
            }
        ),
        Err(PathFail::Invalid)
    );
    // NaN footprint_radius_cells — contains() returns false for NaN comparisons.
    assert_eq!(
        find(
            (50.0, 50.0),
            &[(150.0, 50.0)],
            PathInputs {
                footprint_radius_cells: f64::NAN,
                cell: 100.0,
                walls: &NO_WALLS,
                mask: None,
                regions: None,
                shape: &sq(100.0, DiagonalRule::Chebyshev),
                budget_cells: None,
                traits: MoveTraits::default(),
            }
        ),
        Err(PathFail::Invalid)
    );
    // Infinite footprint_radius_cells — exceeds MAX_FOOTPRINT_CELLS.
    assert_eq!(
        find(
            (50.0, 50.0),
            &[(150.0, 50.0)],
            PathInputs {
                footprint_radius_cells: f64::INFINITY,
                cell: 100.0,
                walls: &NO_WALLS,
                mask: None,
                regions: None,
                shape: &sq(100.0, DiagonalRule::Chebyshev),
                budget_cells: None,
                traits: MoveTraits::default(),
            }
        ),
        Err(PathFail::Invalid)
    );
    // footprint_radius_cells exactly one above the cap.
    assert_eq!(
        find(
            (50.0, 50.0),
            &[(150.0, 50.0)],
            PathInputs {
                footprint_radius_cells: MAX_FOOTPRINT_CELLS + 1.0,
                cell: 100.0,
                walls: &NO_WALLS,
                mask: None,
                regions: None,
                shape: &sq(100.0, DiagonalRule::Chebyshev),
                budget_cells: None,
                traits: MoveTraits::default(),
            }
        ),
        Err(PathFail::Invalid)
    );
}

#[test]
fn straight_route_returns_cell_centers_and_cost() {
    // (50,50)->(250,50): cells (0,0)->(2,0), 2 chebyshev steps. Points = centers of (0,0),(1,0),(2,0).
    let outcome = find(
        (50.0, 50.0),
        &[(250.0, 50.0)],
        PathInputs {
            footprint_radius_cells: 0.1,
            cell: 100.0,
            walls: &NO_WALLS,
            mask: None,
            regions: None,
            shape: &sq(100.0, DiagonalRule::Chebyshev),
            budget_cells: None,
            traits: MoveTraits::default(),
        },
    )
    .unwrap();
    assert!((outcome.cost - 2.0).abs() < 1e-9);
    assert_eq!(outcome.path.first(), Some(&(50.0, 50.0)));
    assert_eq!(outcome.path.last(), Some(&(250.0, 50.0)));
    assert_eq!(outcome.path.len(), 3);
}

#[test]
fn hex_route_off_the_square_diagonal_resolves_to_its_goal() {
    use crate::scene::grid_shape::{GridShape as _, HexGrid};
    // Hex scene: cell size == HexGrid outer radius (`resolve_grid_shape` passes `cell` as
    // `size`). Route along axial direction (1,-1) to (70,-70) — "off the square diagonal": the
    // goal's pixel x is ~0.866·70·cell, so a square `floor(x/cell)+margin` window (=68) clips
    // its axial q (=70), and a square pixel→cell map would resolve the goal to the WRONG axial
    // cell. Only the hex-correct window + `cell_of` (both from `grid.inputs.shape`) resolve
    // the route.
    let hex = HexGrid { size: 100.0 };
    let start = hex.cell_center((0, 0));
    let goal_cell = (70, -70);
    let goal = hex.cell_center(goal_cell);
    // This assertion documents why the hex-aware search window is load-bearing: a square-only
    // window would clip this goal.
    assert!(
        (goal.0 / hex.size).floor() as i32 + WINDOW_MARGIN < goal_cell.0,
        "fixture must exercise the window-clipping case"
    );
    let outcome = find(
        start,
        &[goal],
        PathInputs {
            footprint_radius_cells: 0.1,
            cell: hex.size,
            walls: &NO_WALLS,
            // GM / unconstrained: only the window (not a mask) can gate reachability here.
            mask: None,
            regions: None,
            shape: &hex,
            budget_cells: None,
            traits: MoveTraits::default(),
        },
    )
    .expect("a reachable hex route must resolve, not read Unreachable");
    assert_eq!(outcome.path.first(), Some(&start));
    assert_eq!(
        outcome.path.last(),
        Some(&goal),
        "route reaches the correct hex goal cell"
    );
    // 70 uniform-cost axial steps along the (1,-1) direction.
    assert!(
        (outcome.cost - 70.0).abs() < 1e-9,
        "cost = {}",
        outcome.cost
    );
}

#[test]
fn waypoint_legs_sum_cost_and_carry_alternating_parity() {
    // Leg A: (0,0)->(1,1) one diagonal (alternating cost 1, end parity 1).
    // Leg B: (1,1)->(2,2) one diagonal from parity 1 (cost 2). Total 3, not 1+1.
    let start = (50.0, 50.0);
    let wp = (150.0, 150.0);
    let goal = (250.0, 250.0);
    let outcome = find(
        start,
        &[wp, goal],
        PathInputs {
            footprint_radius_cells: 0.1,
            cell: 100.0,
            walls: &NO_WALLS,
            mask: None,
            regions: None,
            shape: &sq(100.0, DiagonalRule::Alternating),
            budget_cells: None,
            traits: MoveTraits::default(),
        },
    )
    .unwrap();
    assert!(
        (outcome.cost - 3.0).abs() < 1e-9,
        "parity carries across the waypoint (1 + 2)"
    );
}

#[test]
fn too_many_waypoints_is_invalid() {
    let wps: Vec<vision::P> = (0..(MAX_WAYPOINTS + 1))
        .map(|i| (i as f64 * 100.0 + 50.0, 50.0))
        .collect();
    assert_eq!(
        find(
            (50.0, 50.0),
            &wps,
            PathInputs {
                footprint_radius_cells: 0.1,
                cell: 100.0,
                walls: &NO_WALLS,
                mask: None,
                regions: None,
                shape: &sq(100.0, DiagonalRule::Chebyshev),
                budget_cells: None,
                traits: MoveTraits::default(),
            }
        ),
        Err(PathFail::Invalid)
    );
}

#[test]
fn empty_mask_makes_a_nongm_route_unreachable() {
    let mask = BTreeSet::new();
    assert_eq!(
        find(
            (50.0, 50.0),
            &[(250.0, 50.0)],
            PathInputs {
                footprint_radius_cells: 0.1,
                cell: 100.0,
                walls: &NO_WALLS,
                mask: Some(&mask),
                regions: None,
                shape: &sq(100.0, DiagonalRule::Chebyshev),
                budget_cells: None,
                traits: MoveTraits::default(),
            }
        ),
        Err(PathFail::Unreachable)
    );
}

#[test]
fn pathfind_reports_unreachable_when_route_exceeds_window_margin() {
    // A single wall endpoint sits at the extreme y-edge of the start/goal/wall-endpoint AABB
    // (y cell 3). Clearing that endpoint needs a 10-cell footprint clearance, but WINDOW_MARGIN
    // only extends the search 8 cells past the AABB — so every cell that WOULD clear the wall
    // lies outside the search window, even though a wider window would find a route around it.
    // Must fail closed as Unreachable, not panic or silently truncate.
    let c = 100.0;
    let walls = vec![Seg {
        a: (100.0, -200.0),
        b: (100.0, 300.0),
    }];
    let start = (50.0, 50.0);
    let goal = (150.0, 50.0);
    let result = find(
        start,
        &[goal],
        PathInputs {
            // footprint_radius_cells (10 cells) > WINDOW_MARGIN (8 cells)
            footprint_radius_cells: 10.0,
            cell: c,
            walls: &walls,
            mask: None,
            regions: None,
            shape: &sq(c, DiagonalRule::Chebyshev),
            budget_cells: None,
            traits: MoveTraits::default(),
        },
    );
    assert_eq!(result, Err(PathFail::Unreachable));
}

#[test]
fn arrest_region_truncates_the_route_and_flags_arrested() {
    use crate::scene::regions::{RegionBehavior, RegionField, RegionShape};
    let mut b = RegionField::builder();
    // Arrest region covering the ENTIRE i=2 column (x in [200,300), a wide y band). Deviation
    // from a narrower single-cell rect: under Chebyshev, orthogonal and diagonal steps cost
    // the same, so (0,0)->(4,0) has several tied-cost 4-step routes and this build's A*
    // tie-break (QNode ties favor the lexicographically LARGER (cell, parity) key) resolves to
    // a diagonal detour through (2,2), not the straight row — the same tie-break ambiguity
    // `terrain_region_raises_astar_step_cost`'s own comment documents. Covering
    // the whole column (rather than only the straight-line cell) makes the test robust to
    // which tied route is chosen while still proving arrest fires exactly when the route
    // FIRST reaches x in [200,300), at cost 2.0 (2 steps in).
    b.add(
        &RegionShape::Rect {
            x0: 200.0,
            y0: -300.0,
            x1: 300.0,
            y1: 300.0,
        },
        RegionBehavior::Arrest,
        1.0,
        100.0,
        &crate::scene::grid_shape::SquareGrid {
            cell: 100.0,
            rule: DiagonalRule::Chebyshev,
        },
    );
    let field = b.build();
    let outcome = find(
        (50.0, 50.0),
        &[(450.0, 50.0)],
        PathInputs {
            footprint_radius_cells: 0.1,
            cell: 100.0,
            walls: &NO_WALLS,
            mask: None,
            regions: Some(&field),
            shape: &sq(100.0, DiagonalRule::Chebyshev),
            budget_cells: None,
            traits: MoveTraits::default(),
        },
    )
    .unwrap();
    assert!(outcome.arrested);
    assert_eq!(
        outcome.path.last(),
        Some(&(250.0, 250.0)),
        "route truncates AT the arrest cell (2,2), the tie-broken A* route's column-2 cell"
    );
    assert!(
        (outcome.cost - 2.0).abs() < 1e-9,
        "2 steps to reach the arrest column"
    );
}

#[test]
fn no_regions_argument_is_backward_compatible() {
    let outcome = find(
        (50.0, 50.0),
        &[(250.0, 50.0)],
        PathInputs {
            footprint_radius_cells: 0.1,
            cell: 100.0,
            walls: &NO_WALLS,
            mask: None,
            regions: None,
            shape: &sq(100.0, DiagonalRule::Chebyshev),
            budget_cells: None,
            traits: MoveTraits::default(),
        },
    )
    .unwrap();
    assert!(!outcome.arrested);
    assert!((outcome.cost - 2.0).abs() < 1e-9);
}
