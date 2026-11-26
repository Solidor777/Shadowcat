use super::*;
use crate::scene::pathfinding::MoveTraits;
use crate::scene::vision::Seg;

fn open(rule: DiagonalRule, footprint: f64) -> PathGrid<'static> {
    const NO_WALLS: [Seg; 0] = [];
    let shape: &'static crate::scene::grid_shape::SquareGrid =
        Box::leak(Box::new(crate::scene::grid_shape::SquareGrid {
            cell: 100.0,
            rule,
        }));
    PathGrid {
        inputs: PathInputs {
            cell: shape.cell,
            footprint_radius_cells: footprint,
            walls: &NO_WALLS,
            mask: None,
            regions: None,
            shape,
            budget_cells: None,
            traits: MoveTraits::default(),
        },
        window: (-50, -50, 50, 50),
    }
}

#[test]
fn chebyshev_diagonal_is_cost_one_per_step() {
    let g = open(DiagonalRule::Chebyshev, 0.1);
    let (cells, cost, _p) = astar_leg(&g, (0, 0), (3, 3), 0).unwrap();
    assert!((cost - 3.0).abs() < 1e-9, "3 diagonal steps at cost 1 each");
    assert_eq!(cells.first(), Some(&(0, 0)));
    assert_eq!(cells.last(), Some(&(3, 3)));
}

#[test]
fn manhattan_diagonal_costs_two() {
    let g = open(DiagonalRule::Manhattan, 0.1);
    let (_c, cost, _p) = astar_leg(&g, (0, 0), (2, 2), 0).unwrap();
    // Manhattan distance to (2,2) is 4 whether via diagonals (cost 2 each) or orthogonals.
    assert!((cost - 4.0).abs() < 1e-9);
}

#[test]
fn euclidean_diagonal_costs_sqrt2() {
    let g = open(DiagonalRule::Euclidean, 0.1);
    let (_c, cost, _p) = astar_leg(&g, (0, 0), (1, 1), 0).unwrap();
    assert!((cost - std::f64::consts::SQRT_2).abs() < 1e-9);
}

#[test]
fn approximate_diagonal_costs_one_point_five() {
    let g = open(DiagonalRule::Approximate, 0.1);
    let (_c, cost, _p) = astar_leg(&g, (0, 0), (1, 1), 0).unwrap();
    assert!(
        (cost - 1.5).abs() < 1e-9,
        "one diagonal step at 1.5x an orthogonal step, got {cost}"
    );
    // A longer mixed route: (0,0)->(4,2) = 2 diagonals + 2 orthogonals = 3 + 2.
    let (_c, cost, _p) = astar_leg(&g, (0, 0), (4, 2), 0).unwrap();
    assert!((cost - 5.0).abs() < 1e-9, "2*1.5 + 2*1, got {cost}");
}

#[test]
fn astar_leg_orthogonal_never_returns_a_diagonal_step() {
    // The adjacency change end-to-end: under `Orthogonal` the search graph has no diagonal
    // edges, so every consecutive pair of a returned route differs on exactly one axis by one
    // cell, and the route's cost IS the Manhattan distance (exact for this graph). Swept over a
    // battery of (start, goal) pairs — including pure-diagonal alignments, which route
    // as an L-shaped orthogonal detour rather than a straight diagonal.
    let g = open(DiagonalRule::Orthogonal, 0.1);
    for (start, goal) in [
        ((0, 0), (3, 3)),
        ((0, 0), (4, 2)),
        ((-2, 1), (2, -3)),
        ((1, 1), (1, 1)),
        ((0, 0), (0, 5)),
    ] {
        let (cells, cost, parity) = astar_leg(&g, start, goal, 0).unwrap();
        for w in cells.windows(2) {
            let step = (w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs();
            assert_eq!(
                step, 1,
                "route {start:?} -> {goal:?} contains a non-orthogonal step {w:?}"
            );
        }
        let manhattan = ((goal.0 - start.0).abs() + (goal.1 - start.1).abs()) as f64;
        assert!(
            (cost - manhattan).abs() < 1e-9,
            "{start:?} -> {goal:?}: cost {cost} must equal the Manhattan distance {manhattan}"
        );
        assert_eq!(parity, 0, "no diagonal step ever flips the parity bit");
    }
}

/// One conformance case: a rule, a cell offset, and the cost a direct wall-free route between
/// two cells that far apart must report — asserted by the server's `astar_leg` here and by the
/// client's `Grid.distance` against the same fixture (the shared-fixture pattern of
/// `chosen_vision_sample`), so the two languages cannot drift on what a rule costs.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagonalCostCase {
    /// Human-readable case label (uniqueness asserted below).
    name: String,
    /// The `diagonalRule` wire string.
    rule: String,
    /// Column delta between the two cells.
    d_col: i32,
    /// Row delta between the two cells.
    d_row: i32,
    /// The expected direct-route cost in cells.
    expect: f64,
}

/// The corpus envelope.
#[derive(serde::Deserialize)]
struct DiagonalCostCorpus {
    /// The case list.
    cases: Vec<DiagonalCostCase>,
}

#[test]
fn diagonal_cost_matches_the_shared_client_corpus() {
    let raw: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../client/render/src/__fixtures__/diagonal-cost-conformance.json"
    ))
    .expect("diagonal-cost-conformance.json parses");
    let corpus: DiagonalCostCorpus =
        serde_json::from_value(raw).expect("corpus matches the case schema");
    let mut seen = std::collections::HashSet::new();
    for case in &corpus.cases {
        assert!(seen.insert(case.name.as_str()), "duplicate case name");
        let rule = match case.rule.as_str() {
            "chebyshev" => DiagonalRule::Chebyshev,
            "alternating" => DiagonalRule::Alternating,
            "euclidean" => DiagonalRule::Euclidean,
            "manhattan" => DiagonalRule::Manhattan,
            "approximate" => DiagonalRule::Approximate,
            "orthogonal" => DiagonalRule::Orthogonal,
            other => panic!("corpus names an unknown rule {other:?}"),
        };
        let g = open(rule, 0.1);
        let (_cells, cost, _p) =
            astar_leg(&g, (0, 0), (case.d_col, case.d_row), 0).unwrap_or_else(|e| {
                panic!(
                    "case {:?} ({}) must be routable: {e:?}",
                    case.name, case.rule
                )
            });
        assert!(
            (cost - case.expect).abs() < 1e-9,
            "case {:?} ({}): astar cost {} != corpus {}",
            case.name,
            case.rule,
            cost,
            case.expect
        );
    }
    assert_eq!(
        corpus.cases.len(),
        6 * 8,
        "every rule x every offset is covered — a dropped case weakens the pin"
    );
}

#[test]
fn alternating_five_ten_five_parity() {
    // Two consecutive diagonals from parity 0: first costs 1, second costs 2 → total 3, end parity 0.
    let g = open(DiagonalRule::Alternating, 0.1);
    let (_c, cost, parity) = astar_leg(&g, (0, 0), (2, 2), 0).unwrap();
    assert!((cost - 3.0).abs() < 1e-9, "5-10-5: diagonals cost 1 then 2");
    assert_eq!(parity, 0, "two diagonals → parity back to 0");

    // Three diagonals from parity 0: 1 + 2 + 1 = 4, end parity 1.
    let (_c, cost3, parity3) = astar_leg(&g, (0, 0), (3, 3), 0).unwrap();
    assert!((cost3 - 4.0).abs() < 1e-9);
    assert_eq!(parity3, 1);
}

#[test]
fn walled_off_goal_is_unreachable() {
    // Box the goal cell in with blocksMove walls on all four sides → Unreachable (terminates,
    // bounded by the window).
    let c = 100.0;
    let walls = vec![
        Seg {
            a: (3.0 * c, 3.0 * c),
            b: (4.0 * c, 3.0 * c),
            one_way: None,
            limited: false,
            open_within: None,
        },
        Seg {
            a: (3.0 * c, 4.0 * c),
            b: (4.0 * c, 4.0 * c),
            one_way: None,
            limited: false,
            open_within: None,
        },
        Seg {
            a: (3.0 * c, 3.0 * c),
            b: (3.0 * c, 4.0 * c),
            one_way: None,
            limited: false,
            open_within: None,
        },
        Seg {
            a: (4.0 * c, 3.0 * c),
            b: (4.0 * c, 4.0 * c),
            one_way: None,
            limited: false,
            open_within: None,
        },
    ];
    let shape = crate::scene::grid_shape::SquareGrid {
        cell: c,
        rule: DiagonalRule::Chebyshev,
    };
    let g = PathGrid {
        inputs: PathInputs {
            cell: c,
            footprint_radius_cells: 0.1,
            walls: &walls,
            mask: None,
            regions: None,
            shape: &shape,
            budget_cells: None,
            traits: MoveTraits::default(),
        },
        window: (-10, -10, 10, 10),
    };
    assert_eq!(astar_leg(&g, (0, 0), (3, 3), 0), Err(PathFail::Unreachable));
}

#[test]
fn start_equals_goal_is_a_single_cell_zero_cost() {
    let g = open(DiagonalRule::Chebyshev, 0.1);
    let (cells, cost, p) = astar_leg(&g, (2, 2), (2, 2), 1).unwrap();
    assert_eq!(cells, vec![(2, 2)]);
    assert!(cost.abs() < 1e-9);
    assert_eq!(p, 1, "parity is carried unchanged when no step is taken");
}

#[test]
fn astar_leg_routes_through_grid_shape_not_hardcoded_dirs() {
    // Same assertions as `chebyshev_diagonal_is_cost_one_per_step`, but this test exists
    // specifically to fail if a future edit reintroduces a hardcoded `dirs`/`step_cost` path
    // instead of going through `grid.inputs.shape.neighbors_with_cost(...)`.
    let g = open(DiagonalRule::Chebyshev, 0.1);
    let (cells, cost, _p) = astar_leg(&g, (0, 0), (3, 3), 0).unwrap();
    assert!((cost - 3.0).abs() < 1e-9);
    assert_eq!(cells.first(), Some(&(0, 0)));
    assert_eq!(cells.last(), Some(&(3, 3)));
}

/// Hex-scene A* integration coverage: proves the fully-wired hex path behaves correctly
/// end-to-end, mirroring this module's square-scene coverage. The diagonal rule is a
/// `SquareGrid`-only concept (`HexGrid` uses uniform 1-cost steps and the admissible axial
/// heuristic), so a `HexGrid` shape carries no rule at all — see the `grid_shape` module.
fn open_hex(footprint: f64) -> PathGrid<'static> {
    const NO_WALLS: [Seg; 0] = [];
    let shape: &'static crate::scene::grid_shape::HexGrid =
        Box::leak(Box::new(crate::scene::grid_shape::HexGrid { size: 100.0 }));
    PathGrid {
        inputs: PathInputs {
            cell: shape.size,
            footprint_radius_cells: footprint,
            walls: &NO_WALLS,
            mask: None,
            regions: None,
            shape,
            budget_cells: None,
            traits: MoveTraits::default(),
        },
        window: (-50, -50, 50, 50),
    }
}

#[test]
fn hex_scene_astar_finds_a_route_around_a_wall() {
    // Pointy-top axial hex, size=100. The direct route (0,0)->(1,0)->(2,0) costs 2 (uniform
    // 1.0-per-hex-step, no DiagonalRule variance). A wall crossing exactly the center-to-
    // center (0,0)->(1,0) step removes that edge (not the cell (1,0) itself, still reachable
    // via its other 5 neighbor edges), forcing a 3-step detour cost 3.0 via one of two
    // symmetric axial-adjacent-to-(1,0) paths (e.g. (1,-1)->(2,-1)) that still reaches the goal.
    let hx = 100.0 * 3.0_f64.sqrt() / 2.0; // x of the (0,0)->(1,0) step's midpoint
    let walls = vec![Seg {
        a: (hx, -50.0),
        b: (hx, 50.0),
        one_way: None,
        limited: false,
        open_within: None,
    }];
    let mut g = open_hex(0.1);
    g.inputs.walls = &walls;
    let (cells, cost, _p) = astar_leg(&g, (0, 0), (2, 0), 0).unwrap();
    assert_eq!(cells.first(), Some(&(0, 0)));
    assert_eq!(cells.last(), Some(&(2, 0)));
    assert!(
        !cells.contains(&(1, 0)),
        "the direct neighbor is unreachable once its entering step is wall-blocked"
    );
    assert!(
        (cost - 3.0).abs() < 1e-9,
        "uniform 1.0-per-hex-step cost over the 3-step detour"
    );
}

#[test]
fn hex_scene_astar_respects_the_visibility_mask() {
    // Same detour shape as `hex_astar_routes_around_a_blocking_wall`, but forced by mask
    // exclusion instead of a
    // wall: (1,0) is excluded from the mask, so the route must go around it via
    // (0,1)->(1,1), never entering the masked-out cell.
    let mut mask: BTreeSet<Cell> = BTreeSet::new();
    mask.insert((0, 0));
    mask.insert((0, 1));
    mask.insert((1, 1));
    mask.insert((2, 0));
    // (1, 0) deliberately absent — the masked-out cell.
    let mut g = open_hex(0.1);
    g.inputs.mask = Some(&mask);
    let (cells, cost, _p) = astar_leg(&g, (0, 0), (2, 0), 0).unwrap();
    assert!(
        !cells.contains(&(1, 0)),
        "route must never enter the masked-out cell"
    );
    assert_eq!(cells.last(), Some(&(2, 0)));
    assert!((cost - 3.0).abs() < 1e-9);
}

#[test]
fn hex_astar_returns_the_true_shortest_route_via_the_admissible_heuristic() {
    // Pins hex A* route optimality: the axial heuristic (`HexGrid::heuristic`) is admissible, so
    // A* returns the true shortest hex route. A square `DiagonalRule` heuristic OVERESTIMATES the
    // true axial distance for opposite-sign axial deltas (Manhattan is 2x on the (1,-1) line),
    // which is non-admissible on hex and yields a valid but longer route. This maze isolates that
    // case: the goal's cheap axis-neighbor approach (-1,0) is masked out, forcing the shortest
    // route to approach through the OPPOSITE-SIGN neighbor (-1,1) — the cell a square heuristic
    // deprioritizes. The admissible heuristic yields the true shortest route
    // (-3,1)->(-2,1)->(-1,1)->(0,0) at cost 3 over 4 cells; a square-Manhattan estimate over this
    // same maze yields the 4-cost, 5-cell detour (-3,1)->(-2,0)->(-1,-1)->(0,-1)->(0,0) instead,
    // so any non-admissible replacement of `HexGrid::heuristic` fails this assertion.
    let hx = crate::scene::grid_shape::HexGrid { size: 100.0 };
    // Mask = hex disc (radius 4 in every axis incl. the third cube axis q+r) MINUS the goal's
    // cheap axis neighbor (-1,0): the detour cells stay present, only the cheap approach closes.
    let mut mask: BTreeSet<Cell> = BTreeSet::new();
    let r = 4i32;
    for q in -r..=r {
        for rr in -r..=r {
            if (q + rr).abs() <= r && (q, rr) != (-1, 0) {
                mask.insert((q, rr));
            }
        }
    }
    let g = PathGrid {
        inputs: PathInputs {
            cell: 100.0,
            footprint_radius_cells: 0.1,
            walls: &[],
            mask: Some(&mask),
            regions: None,
            shape: &hx,
            budget_cells: None,
            traits: MoveTraits::default(),
        },
        window: (-50, -50, 50, 50),
    };
    let (cells, cost, _p) = astar_leg(&g, (-3, 1), (0, 0), 0).unwrap();
    assert_eq!(
        cells,
        vec![(-3, 1), (-2, 1), (-1, 1), (0, 0)],
        "admissible hex heuristic must yield the true shortest route through the opposite-sign \
         approach (-1,1), not the longer detour a square heuristic would pick"
    );
    assert!(
        (cost - 3.0).abs() < 1e-9,
        "true hex distance is 3, not the square heuristic's 4-cost detour; got {cost}"
    );
    assert_eq!(
        cells.len(),
        4,
        "shortest route is 4 cells (3 uniform-cost hex steps)"
    );
}
