//! Frozen-fixture parity gate for the `GridShape`/`SquareGrid` refactor.
//!
//! Every scene geometry consumer (grid A* router, per-step move executor, visibility mask,
//! region rasterization) now routes its per-cell math through `grid_shape::GridShape`. The
//! `SquareGrid` implementation is a byte-identical port of the pre-refactor hardcoded square
//! math, so its output MUST equal what the hardcoded code produced. This module pins the FULL
//! result of a curated scenario battery (every waypoint of a route, every cell of a returned set,
//! every cost value) so a single-cell drift — including a future `HexGrid` cutover mis-wiring the
//! square path — fails here before any downstream consumer silently ships the change.
//!
//! Every expected value is derived from the underlying square-grid geometry, not copied from a
//! run: each scenario is constructed so its optimal route / visible set / rasterized set is UNIQUE
//! (no A* tie-break ambiguity, no occlusion), making the pin independently re-derivable by hand.

use std::collections::BTreeSet;

use serde_json::{json, Value};
use uuid::Uuid;

use crate::data::document::Document;
use crate::scene::grid_shape::SquareGrid;
use crate::scene::move_exec::execute_move;
use crate::scene::pathfinding::{find, DiagonalRule, PathOutcome};
use crate::scene::regions::{rasterize, RegionBehavior, RegionField, RegionShape};
use crate::scene::{MovementRestriction, SceneEcs};

// --- Local fixture builders (this module cannot reach `mod tests`'s private helpers) ---

fn doc(id: u128, parent: Option<u128>, ty: &str) -> Document {
    let mut d =
        crate::data::document::tests::world_scoped_doc(Uuid::from_u128(9), Uuid::from_u128(id), ty);
    d.parent_id = parent.map(Uuid::from_u128);
    d
}

fn entity_doc(id: u128, parent: u128, ty: &str, body: Value) -> Document {
    let mut d = doc(id, Some(parent), ty);
    d.engine = Some(body);
    d
}

// -------------------------------------------------------------------------------------------
// Fixture 1: grid A* route, all four DiagonalRule variants on one layout.
//
// Layout: an open field (cell = 100, footprint radius 0.1), start (50,50) = cell (0,0), goal
// (250,250) = cell (2,2), with FOUR one-cell impassable regions blocking the orthogonal flanker
// cells (1,0), (0,1), (2,1), (1,2). Blocking the flankers forces the pure diagonal staircase
// (0,0) -> (1,1) -> (2,2) to be the UNIQUE optimal route under every rule (every alternative
// either enters a blocked cell or is strictly longer), so the returned `path` is identical across
// rules and only the diagonal-cost `cost` varies — isolating the per-rule step-cost math with no
// tie-break ambiguity. The impassable cells carry no terrain weight (multiplier 1.0) and no
// arrest, so `cost` is exactly the sum of the two diagonal step costs and `arrested` is false.
// -------------------------------------------------------------------------------------------

/// One-cell impassable rect tightly enclosing `center` (±40 units), so it rasterizes to exactly
/// the single cell whose center it contains — no boundary/inclusive-compare ambiguity.
fn cell_block(center: (f64, f64)) -> RegionShape {
    RegionShape::Rect {
        x0: center.0 - 40.0,
        y0: center.1 - 40.0,
        x1: center.0 + 40.0,
        y1: center.1 + 40.0,
    }
}

fn flanker_blocked_field() -> RegionField {
    // The diagonal-cost rule is inert for rasterization (cell_center is rule-independent).
    let grid = SquareGrid {
        cell: 100.0,
        rule: DiagonalRule::Chebyshev,
    };
    let mut b = RegionField::builder();
    for center in [(150.0, 50.0), (50.0, 150.0), (250.0, 150.0), (150.0, 250.0)] {
        b.add(
            &cell_block(center),
            RegionBehavior::Impassable,
            1.0,
            100.0,
            &grid,
        );
    }
    b.build()
}

fn route(rule: DiagonalRule, field: &RegionField) -> PathOutcome {
    find(
        (50.0, 50.0),
        &[(250.0, 250.0)],
        0.1,
        100.0,
        rule,
        &[],
        None,
        Some(field),
        &SquareGrid { cell: 100.0, rule },
    )
    .expect("forced diagonal staircase is reachable under every rule")
}

#[test]
fn astar_route_parity_all_diagonal_rules_pins_full_path_and_cost() {
    let field = flanker_blocked_field();
    // The forced-unique route's cell-center waypoints, identical under every rule.
    let expected_path = vec![(50.0, 50.0), (150.0, 150.0), (250.0, 250.0)];

    // Chebyshev: each diagonal costs 1.0 -> 2.0 total.
    let cheb = route(DiagonalRule::Chebyshev, &field);
    assert_eq!(cheb.path, expected_path);
    assert!((cheb.cost - 2.0).abs() < 1e-9, "chebyshev cost");
    assert!(!cheb.arrested);

    // Manhattan: each diagonal costs 2.0 -> 4.0 total.
    let manh = route(DiagonalRule::Manhattan, &field);
    assert_eq!(manh.path, expected_path);
    assert!((manh.cost - 4.0).abs() < 1e-9, "manhattan cost");
    assert!(!manh.arrested);

    // Euclidean: each diagonal costs sqrt(2) -> 2*sqrt(2) total.
    let eucl = route(DiagonalRule::Euclidean, &field);
    assert_eq!(eucl.path, expected_path);
    assert!(
        (eucl.cost - 2.0 * std::f64::consts::SQRT_2).abs() < 1e-9,
        "euclidean cost"
    );
    assert!(!eucl.arrested);

    // Alternating (5-10-5): first diagonal from parity 0 costs 1.0, second from parity 1 costs
    // 2.0 -> 3.0 total. Threads the parity across the two steps of the single leg.
    let alt = route(DiagonalRule::Alternating, &field);
    assert_eq!(alt.path, expected_path);
    assert!((alt.cost - 3.0).abs() < 1e-9, "alternating cost");
    assert!(!alt.arrested);
}

// -------------------------------------------------------------------------------------------
// Fixture 2: gate_walk mask gate on a multi-cell diagonal move crossing a mask boundary.
//
// Token at (0,0), cell = 100, no walls, restriction Visible. The authored path is a three-cell
// diagonal [(0,0),(100,100),(200,200),(300,300)] — gate_walk is the identity on this input (each
// vertex one cell from its predecessor). Each diagonal king-step crosses the shared lattice corner
// exactly, so `SquareGrid::line_traversal` (supercover) emits BOTH off-diagonal flanker cells in
// addition to the two endpoint cells:
//   (0,0)->(1,1): {(0,0),(1,1),(1,0),(0,1)}
//   (1,1)->(2,2): {(1,1),(2,2),(2,1),(1,2)}
//   (2,2)->(3,3): {(2,2),(3,3),(3,2),(2,3)}
// The mask covers steps 1 and 2 in full but omits every step-3 cell, so the move truncates on the
// step INTO (3,3): it stops at the last cleared sample (200,200) = cell (2,2). cost accrues the
// 1.0 baseline terrain multiplier per entered cell ((1,1) then (2,2)) = 2.0.
// -------------------------------------------------------------------------------------------

fn clear_scene() -> (SceneEcs, Uuid, Uuid) {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
        ],
        0,
    );
    (ecs, scene_id, token_id)
}

#[test]
fn gate_walk_mask_gate_parity_pins_diagonal_truncation_point() {
    let (ecs, scene, token) = clear_scene();
    // Steps 1 and 2 fully covered (incl. corner flankers); every step-3 cell omitted.
    let visible: BTreeSet<(i32, i32)> = [(0, 0), (0, 1), (1, 0), (1, 1), (1, 2), (2, 1), (2, 2)]
        .into_iter()
        .collect();
    let out = execute_move(
        &ecs,
        scene,
        token,
        &[(0.0, 0.0), (100.0, 100.0), (200.0, 200.0), (300.0, 300.0)],
        MovementRestriction::Visible,
        &visible,
        100.0,
    )
    .expect("clear token, in-bounds path");

    assert_eq!(out.stop, (200.0, 200.0), "truncates entering cell (2,2)");
    assert_eq!(
        out.render_path,
        vec![(0.0, 0.0), (100.0, 100.0), (200.0, 200.0)],
    );
    assert!(out.truncated);
    assert!(
        (out.cost - 2.0).abs() < 1e-9,
        "baseline cost for 2 entered cells"
    );
}

// -------------------------------------------------------------------------------------------
// Fixture 2b: gate_walk FLANKER enforcement — truncation driven by a hidden off-diagonal cell,
// not by a hidden endpoint. This is the discriminating case the whole gate exists to protect: a
// diagonal king-step crosses the shared lattice corner, so `SquareGrid::line_traversal`
// (supercover) MUST emit both off-diagonal flanker cells alongside the two endpoint cells,
// preventing a diagonal from threading an unseen cell (anti-corner-cutting).
//
// Same three-cell diagonal path as Fixture 2, but the mask is built so BOTH endpoints of the
// step-3 diagonal ((2,2)->(3,3)) ARE visible, along with one of that step's two flankers ((2,3)).
// Only the OTHER flanker (3,2) is masked out. Correct supercover for the step is
// {(2,2),(3,3),(3,2),(2,3)}; the missing (3,2) forces truncation on the step INTO (3,3), stopping
// at (200,200) = cell (2,2), cost 2.0 (baseline over the two entered cells (1,1),(2,2)).
//
// Discrimination proof (why this case has teeth Fixture 2 lacks): a hypothetical regression whose
// `line_traversal` emitted only the two diagonal ENDPOINT cells {(2,2),(3,3)} — dropping the
// flankers — would find both endpoints visible here and admit the step, running the move through
// to (300,300) UN-truncated (cost 3.0). That over-permissive outcome differs from the asserted one,
// so this fixture fails on exactly the flanker-dropping class Fixture 2 (which hides the endpoint
// (3,3) and would truncate identically with or without flankers) cannot detect.
// -------------------------------------------------------------------------------------------

#[test]
fn gate_walk_flanker_gate_truncates_with_both_diagonal_endpoints_visible() {
    let (ecs, scene, token) = clear_scene();
    // Steps 1 and 2 fully covered. Step 3: both endpoints (2,2),(3,3) visible + flanker (2,3);
    // the other flanker (3,2) is omitted so ONLY a hidden flanker can cause truncation.
    let visible: BTreeSet<(i32, i32)> = [
        (0, 0),
        (0, 1),
        (1, 0),
        (1, 1),
        (1, 2),
        (2, 1),
        (2, 2),
        (2, 3),
        (3, 3),
    ]
    .into_iter()
    .collect();
    let out = execute_move(
        &ecs,
        scene,
        token,
        &[(0.0, 0.0), (100.0, 100.0), (200.0, 200.0), (300.0, 300.0)],
        MovementRestriction::Visible,
        &visible,
        100.0,
    )
    .expect("clear token, in-bounds path");

    // Correct flanker-aware supercover truncates at (2,2) despite (2,2) AND (3,3) both visible; a
    // flanker-dropping regression would instead run through to (300,300) un-truncated.
    assert_eq!(
        out.stop,
        (200.0, 200.0),
        "hidden flanker (3,2) truncates the diagonal step"
    );
    assert_eq!(
        out.render_path,
        vec![(0.0, 0.0), (100.0, 100.0), (200.0, 200.0)],
    );
    assert!(
        out.truncated,
        "move must not reach (3,3) with a hidden flanker"
    );
    assert!(
        (out.cost - 2.0).abs() < 1e-9,
        "baseline cost for 2 entered cells"
    );
}

// -------------------------------------------------------------------------------------------
// Fixture 3: visible_cells on an open all-bright scene with two vision sources.
//
// A wall-less scene (bounds 500x500 grid units, cell 100), lighting disabled (all-bright), LOS
// restriction off, with TWO player-owned tokens: source A at (50,50) and source B at (150,150).
// With no occlusion each source reveals its whole bound box; source A's bound box (viewpoint minus
// margin unioned with the scene extent) yields the full set (-1..=4) x (-1..=4), and source B sits
// strictly inside it (its viewpoint+margin box never exceeds the scene extent, so its bound box is
// a subset of A's). The union is therefore exactly A's set — pinned in full, exercising
// `GridShape::cell_center` across both sources' per-cell scans.
// -------------------------------------------------------------------------------------------

fn two_source_open_scene() -> (SceneEcs, Uuid, Uuid) {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut source_a = entity_doc(
        11,
        10,
        "token",
        json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    source_a.owner = Some(user);
    let mut source_b = entity_doc(
        12,
        10,
        "token",
        json!({ "x": 150.0, "y": 150.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    source_b.owner = Some(user);
    let mut scene = doc(10, None, "scene");
    scene.engine = Some(json!({
        "grid": { "kind": "square", "size": 100 }, "background": null,
        "bounds": { "width": 500.0, "height": 500.0 }
    }));
    let mut ecs = SceneEcs::from_documents(vec![scene, source_a, source_b], 0);
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
fn visible_cells_parity_two_sources_pins_full_cell_set() {
    let (ecs, user, scene) = two_source_open_scene();
    let got = ecs.visible_cells(user, scene, false);
    let expected: BTreeSet<(i32, i32)> = (-1..=4)
        .flat_map(|i| (-1..=4).map(move |j| (i, j)))
        .collect();
    assert_eq!(got, expected);
}

// -------------------------------------------------------------------------------------------
// Fixture 4: region rasterization per RegionShape variant, exact cell Vec.
//
// `rasterize` iterates i outer, j inner, so its output Vec is already ascending in (i,j). Each
// shape below is chosen so no cell center lands exactly on an edge (avoiding boundary ambiguity),
// making the covered-center set — and thus the full Vec — independently re-derivable.
// -------------------------------------------------------------------------------------------

fn sq() -> SquareGrid {
    SquareGrid {
        cell: 100.0,
        rule: DiagonalRule::Chebyshev,
    }
}

#[test]
fn rasterize_parity_rect_pins_full_cell_vec() {
    // Rect [0,0]-[250,150]: covers columns 0,1,2 (centers x = 50,150,250 all <= 250) and rows
    // 0,1 (centers y = 50,150 all <= 150).
    let shape = RegionShape::Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 250.0,
        y1: 150.0,
    };
    let cells = rasterize(&shape, 100.0, &sq()).unwrap();
    assert_eq!(cells, vec![(0, 0), (0, 1), (1, 0), (1, 1), (2, 0), (2, 1)],);
}

#[test]
fn rasterize_parity_circle_pins_full_cell_vec() {
    // Circle center (150,150) radius 60: only cell (1,1)'s center (150,150) is within 60; every
    // other candidate center is >= 100 away.
    let shape = RegionShape::Circle {
        cx: 150.0,
        cy: 150.0,
        r: 60.0,
    };
    let cells = rasterize(&shape, 100.0, &sq()).unwrap();
    assert_eq!(cells, vec![(1, 1)]);
}

#[test]
fn rasterize_parity_polygon_pins_full_cell_vec() {
    // Right triangle (0,0)-(350,0)-(0,350): a cell center (x,y) is strictly inside iff x+y < 350.
    // Centers sum to 100*(i+j+1), never exactly 350, so no center lands on the hypotenuse. The
    // interior centers are exactly those with i+j <= 2.
    let shape = RegionShape::Polygon {
        points: vec![(0.0, 0.0), (350.0, 0.0), (0.0, 350.0)],
    };
    let cells = rasterize(&shape, 100.0, &sq()).unwrap();
    assert_eq!(cells, vec![(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (2, 0)],);
}

// -------------------------------------------------------------------------------------------
// Fixture 5: visible_cells under LENIENCY — byte-identity proof for the corner geometry.
//
// Fixture 3 pins the STRICT (center-only) set; leniency is the only mode where a cell's CORNERS
// decide membership, so this fixture pins the set that `accumulate_visible_cells`'s
// `GridShape::cell_vertices` corner geometry and pixel-space `pad_px` candidate expansion must
// reproduce exactly for a square grid.
//
// One owned instanced token at (30,30) (unlimited "normal" vision), all-bright, LOS off, cell 100,
// authored bounds 520x520. `source_los_poly` is therefore the rectangle [-70,520] x [-70,520]
// (bound_for_scene: min(30-100,0)=-70 on each low edge, max(30+100,520)=520 on each high edge).
// Every coordinate below is DERIVED, none run-copied:
//   STRICT: a cell qualifies iff its CENTER ((i+0.5)*100) lies in [-70,520] -> i,j in [-1,4].
//   LENIENT: also qualifies iff any CORNER (a multiple of 100) lies in the rectangle. A corner
//   coordinate k*100 is inside iff -70 < k*100 < 520 -> k in {0,1,2,3,4,5}; a cell has an inside
//   corner iff i in [-1,5] AND j in [-1,5]. The edges (-70, 520) are non-multiples of 100, so no
//   corner ever lands on an edge -> every corner test is unambiguous. Union -> lenient = [-1,5]^2.
// The 13 cells present ONLY under leniency (column i=5 + row j=5) are exactly what `cell_vertices`
// must reproduce; a center-only regression collapses lenient onto strict, and mis-wired square
// corner geometry diverges here.
// -------------------------------------------------------------------------------------------

fn lenient_corner_open_scene() -> (SceneEcs, Uuid, Uuid) {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut source = entity_doc(
        11,
        10,
        "token",
        json!({ "x": 30.0, "y": 30.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    source.owner = Some(user);
    let mut scene = doc(10, None, "scene");
    scene.engine = Some(json!({
        "grid": { "kind": "square", "size": 100 }, "background": null,
        "bounds": { "width": 520.0, "height": 520.0 }
    }));
    let mut ecs = SceneEcs::from_documents(vec![scene, source], 0);
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
fn visible_cells_lenient_parity_pins_full_cell_set_including_corner_ring() {
    let (ecs, user, scene) = lenient_corner_open_scene();

    // STRICT: center-only membership -> [-1,4]^2 (companion to Fixture 3's pin).
    let strict = ecs.visible_cells(user, scene, false);
    let expected_strict: BTreeSet<(i32, i32)> = (-1..=4)
        .flat_map(|i| (-1..=4).map(move |j| (i, j)))
        .collect();
    assert_eq!(strict, expected_strict, "strict center-only set");

    // LENIENT: corner-sampling adds column i=5 and row j=5 -> [-1,5]^2, a strict superset.
    let lenient = ecs.visible_cells(user, scene, true);
    let expected_lenient: BTreeSet<(i32, i32)> = (-1..=5)
        .flat_map(|i| (-1..=5).map(move |j| (i, j)))
        .collect();
    assert_eq!(lenient, expected_lenient, "lenient corner-inclusive set");

    // The corner ring is exactly the 13 cells strict lacks — proves `cell_vertices` is
    // load-bearing here (a center-only regression would make lenient == strict).
    assert!(expected_lenient.is_superset(&expected_strict));
    assert_eq!(expected_lenient.len() - expected_strict.len(), 13);
}
