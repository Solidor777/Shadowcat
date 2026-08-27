use super::*;
use crate::scene::pathfinding::DiagonalRule;

#[test]
fn each_shape_reports_its_own_kind() {
    // Discrimination: fails if either impl returns the other's kind, which would make the
    // explored blob's tag disagree with the geometry that wrote it.
    let sq = SquareGrid {
        cell: 100.0,
        rule: DiagonalRule::Chebyshev,
    };
    let hx = HexGrid { size: 100.0 };
    assert_eq!(sq.kind(), crate::scene::GridKind::Square);
    assert_eq!(hx.kind(), crate::scene::GridKind::Hex);
}

#[test]
fn square_grid_cell_center_matches_pathfinding_cell_center() {
    let g = SquareGrid {
        cell: 100.0,
        rule: DiagonalRule::Chebyshev,
    };
    assert_eq!(
        g.cell_center((2, 3)),
        crate::scene::pathfinding::cell_center((2, 3), g.cell)
    );
}

#[test]
fn square_grid_cell_of_floors_to_cell_index() {
    let g = SquareGrid {
        cell: 100.0,
        rule: DiagonalRule::Chebyshev,
    };
    assert_eq!(g.cell_of((250.0, 149.0)), (2, 1));
    assert_eq!(g.cell_of((-10.0, -1.0)), (-1, -1));
}

#[test]
fn hex_grid_zero_radius_point_on_a_shared_edge_resolves_to_the_single_canonical_cell() {
    // A zero-radius point sitting exactly on the edge shared by axial hexes (0,0) and (1,0),
    // at the edge midpoint, is equidistant (== the inradius) from both hex centers — the
    // hex-grid analog of the square grid's boundary tie.
    let hx = HexGrid { size: 100.0 };
    let c0 = hx.cell_center((0, 0));
    let c1 = hx.cell_center((1, 0));
    let mid = ((c0.0 + c1.0) / 2.0, (c0.1 + c1.1) / 2.0);
    let canonical = hx.cell_of(mid);
    let cells = hx.footprint_cells(canonical, mid, 0.0, hx.size);
    assert_eq!(
        cells,
        vec![canonical],
        "a zero-radius boundary point must resolve to exactly the single canonical cell, \
         matching cell_of, mirroring the square grid's own resolution"
    );
}

#[test]
fn square_grid_neighbors_with_cost_matches_chebyshev_dirs_and_step_cost() {
    let g = SquareGrid {
        cell: 100.0,
        rule: DiagonalRule::Chebyshev,
    };
    let ns = g.neighbors_with_cost((0, 0), 0);
    assert_eq!(ns.len(), 8, "8-directional king-move expansion");
    // A diagonal neighbor costs 1.0 under Chebyshev.
    let diag = ns
        .iter()
        .find(|(c, _, _)| *c == (1, 1))
        .expect("diagonal neighbor present");
    assert!((diag.1 - 1.0).abs() < 1e-9);
    // An orthogonal neighbor costs 1.0 too.
    let ortho = ns
        .iter()
        .find(|(c, _, _)| *c == (1, 0))
        .expect("orthogonal neighbor present");
    assert!((ortho.1 - 1.0).abs() < 1e-9);
}

#[test]
fn square_grid_neighbors_with_cost_alternating_threads_parity() {
    let g = SquareGrid {
        cell: 100.0,
        rule: DiagonalRule::Alternating,
    };
    let ns = g.neighbors_with_cost((0, 0), 0);
    let diag = ns.iter().find(|(c, _, _)| *c == (1, 1)).unwrap();
    assert!(
        (diag.1 - 1.0).abs() < 1e-9,
        "first diagonal from parity 0 costs 1"
    );
    assert_eq!(diag.2, 1, "parity flips after a diagonal step");
    let ortho = ns.iter().find(|(c, _, _)| *c == (1, 0)).unwrap();
    assert_eq!(ortho.2, 0, "parity unchanged after an orthogonal step");
}

#[test]
fn square_grid_line_traversal_matches_supercover_cells() {
    let g = SquareGrid {
        cell: 100.0,
        rule: DiagonalRule::Chebyshev,
    };
    let a = (50.0, 50.0);
    let b = (250.0, 250.0);
    assert_eq!(
        g.line_traversal(a, b, g.cell),
        crate::scene::movement::supercover_cells(a, b, g.cell)
    );
}

#[test]
fn square_grid_footprint_cells_matches_pathfinding_footprint_cells() {
    let g = SquareGrid {
        cell: 100.0,
        rule: DiagonalRule::Chebyshev,
    };
    let anchor = (2, 2);
    let ctr = (250.0, 250.0);
    let mut got = g.footprint_cells(anchor, ctr, 60.0, g.cell);
    let mut want = crate::scene::pathfinding::footprint_cells(anchor, ctr, 60.0, g.cell);
    got.sort();
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn hex_grid_axial_round_trip_pixel_to_axial_to_pixel() {
    let g = HexGrid { size: 50.0 };
    // Round-tripping a cell's own center through pixel->axial->pixel should be a fixed point.
    let center = g.cell_center((2, -1));
    let (q, r) = g.pixel_to_axial(center);
    assert_eq!((q, r), (2, -1));
}

#[test]
fn hex_grid_neighbors_with_cost_returns_6_uniform_cost_neighbors() {
    let g = HexGrid { size: 50.0 };
    let ns = g.neighbors_with_cost((0, 0), 3);
    assert_eq!(ns.len(), 6, "hex has 6 neighbors, not 8");
    for (_, cost, parity) in &ns {
        assert!(
            (cost - 1.0).abs() < 1e-9,
            "every hex step costs 1.0 uniformly"
        );
        assert_eq!(
            *parity, 3,
            "hex never touches parity — passed through unchanged"
        );
    }
    // The 6 axial neighbor offsets (Red Blob Games pointy-top convention).
    let mut got: Vec<Cell> = ns.iter().map(|(c, _, _)| *c).collect();
    got.sort();
    let mut want = vec![(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn hex_grid_line_traversal_includes_start_and_end_cells() {
    let g = HexGrid { size: 50.0 };
    let a_center = g.cell_center((0, 0));
    let b_center = g.cell_center((3, 0));
    let cells = g.line_traversal(a_center, b_center, g.size).unwrap();
    assert!(cells.contains(&(0, 0)));
    assert!(cells.contains(&(3, 0)));
    // A straight 3-cell traversal along one axial direction crosses exactly cells (0,0)..(3,0).
    assert_eq!(cells.len(), 4);
}

#[test]
fn hex_grid_line_traversal_degenerate_same_point_returns_single_cell() {
    let g = HexGrid { size: 50.0 };
    let p = g.cell_center((2, -1));
    let cells = g.line_traversal(p, p, g.size).unwrap();
    assert_eq!(cells.len(), 1);
    assert!(cells.contains(&(2, -1)));
}

#[test]
fn hex_footprint_reaches_a_cell_the_disc_touches_near_a_shared_vertex() {
    // A pointy-top hex centred at the origin has a vertex at `(√3/2·size, -size/2)`, shared
    // with hexes (1,0) and (1,-1) — both of whose centres sit exactly `size` (the
    // circumradius) from that vertex. A disc of radius 2 centred one unit inside the origin
    // hex from that vertex overlaps all three by a clear margin at this fixture's size, so
    // no assertion here sits on the overlap boundary.
    //
    // Discrimination: a predicate comparing the CELL CENTRE distance against
    // `r_scene + inradius` compares roughly `size` against `2 + √3/2·size` for both
    // neighbours, which fails for every size larger than roughly 15, and emits only the anchor. The
    // assertion is the presence of the two neighbours, which no centre-distance-against-
    // inradius test can produce at this radius.
    let g = HexGrid { size: 50.0 };
    let half_x = g.size * 3.0_f64.sqrt() / 2.0;
    let vertex = (half_x, -g.size / 2.0);
    // One unit from the vertex along the direction back to the origin hex's centre.
    let p = (vertex.0 - 0.866, vertex.1 + 0.5);
    assert_eq!(
        g.cell_of(p),
        (0, 0),
        "fixture: the sample sits in the origin hex"
    );
    let cells = g.footprint_cells((0, 0), p, 2.0, g.size);
    assert!(cells.contains(&(0, 0)), "the anchor hex, got {cells:?}");
    assert!(
        cells.contains(&(1, 0)),
        "the hex across the shared edge, got {cells:?}"
    );
    assert!(
        cells.contains(&(1, -1)),
        "the third hex at that vertex, got {cells:?}"
    );
}

#[test]
fn hex_footprint_from_a_cell_centre_brackets_the_inradius_threshold() {
    // From a hex's own centre the nearest point of every neighbour is their shared edge, at
    // the inradius √3/2·size. A disc a clear 0.5 under that stays in one hex; a disc
    // a clear 0.5 over it reaches all six neighbours and nothing beyond (ring 2's nearest edge
    // is 2.598·size away). Both probes sit off the threshold, so a one-ULP difference in the
    // distance computation cannot decide either.
    //
    // Discrimination: fails if the threshold for a CENTRE-anchored disc moves in either
    // direction by more than half a unit — the case every existing hex parity fixture
    // exercises, and the property that makes this change inert for them.
    let g = HexGrid { size: 50.0 };
    let ctr = g.cell_center((0, 0));
    let inradius = g.size * 3.0_f64.sqrt() / 2.0;
    assert_eq!(
        g.footprint_cells((0, 0), ctr, inradius - 0.5, g.size),
        vec![(0, 0)]
    );
    let over = g.footprint_cells((0, 0), ctr, inradius + 0.5, g.size);
    assert_eq!(
        over.len(),
        7,
        "the disc reaches all six neighbours, got {over:?}"
    );
    assert!(over.contains(&(0, 0)) && over.contains(&(1, 0)) && over.contains(&(0, 1)));
}

#[test]
fn hex_footprint_returns_the_anchor_when_the_disc_overlaps_nothing_else() {
    // The zero-radius guarantee the square implementation also makes: a point footprint
    // yields exactly the anchor. Discrimination: fails if the empty-result fallback is
    // dropped, or if the predicate admits a cell the disc does not reach.
    let g = HexGrid { size: 50.0 };
    let ctr = g.cell_center((2, -1));
    assert_eq!(g.footprint_cells((2, -1), ctr, 0.0, g.size), vec![(2, -1)]);
}

#[test]
fn hex_grid_footprint_cells_always_includes_the_anchor() {
    let g = HexGrid { size: 50.0 };
    let anchor = (2, -1);
    let ctr = g.cell_center(anchor);
    // Zero-radius footprint: only the anchor cell.
    let cells = g.footprint_cells(anchor, ctr, 0.0, g.size);
    assert_eq!(cells, vec![anchor]);
}

#[test]
fn hex_grid_footprint_cells_large_radius_includes_neighbors() {
    let g = HexGrid { size: 50.0 };
    let anchor = (0, 0);
    let ctr = g.cell_center(anchor);
    // A footprint radius comparable to the hex's own size should pull in at least one neighbor:
    // any radius past the inradius `√3/2·size` reaches the shared edge.
    let cells = g.footprint_cells(anchor, ctr, g.size * 1.2, g.size);
    assert!(
        cells.len() > 1,
        "a large-enough footprint overlaps more than just the anchor"
    );
    assert!(cells.contains(&anchor));
}

#[test]
fn hex_grid_cell_of_matches_axial_round_for_a_known_point() {
    // size=50: cell (0,0)'s center is (0,0) in pointy-top axial pixel space (Red Blob Games
    // convention, matching the client's `pixelToAxial`/`axialToPixel` exactly).
    let g = HexGrid { size: 50.0 };
    assert_eq!(g.cell_of((0.0, 0.0)), (0, 0));
    // A point well inside cell (1,0)'s hex (center at axial (1,0) -> pixel via axialToPixel)
    // should resolve back to (1,0).
    let c10_center = g.cell_center((1, 0));
    assert_eq!(g.cell_of(c10_center), (1, 0));
}

/// Independent reimplementation of the intended `floor(min/cell)..=floor(max/cell)` row-major
/// rectangle, so the parity assertion isn't tautological with the impl under test.
fn hand_floor_rect(min: vision::P, max: vision::P, cell: f64) -> Vec<Cell> {
    let i0 = (min.0 / cell).floor() as i32;
    let i1 = (max.0 / cell).floor() as i32;
    let j0 = (min.1 / cell).floor() as i32;
    let j1 = (max.1 / cell).floor() as i32;
    let mut out = Vec::new();
    for i in i0..=i1 {
        for j in j0..=j1 {
            out.push((i, j));
        }
    }
    out
}

/// The DoS bound the vision/explored scans pass; the cell-span tests exercise `cells_in_bounds`
/// at this cap.
const CAP: i64 = crate::scene::explored::MAX_CELLS_PER_POLYGON;

#[test]
fn square_cells_in_bounds_equals_hand_written_floor_rectangle() {
    let g = SquareGrid {
        cell: 100.0,
        rule: DiagonalRule::Chebyshev,
    };
    // Origin-anchored 3×3.
    let (min, max) = ((0.0, 0.0), (250.0, 250.0));
    assert_eq!(
        g.cells_in_bounds(min, max, g.cell, CAP),
        Some(hand_floor_rect(min, max, g.cell))
    );
    // Straddling the origin (negative coords).
    let (min, max) = ((-150.0, -50.0), (50.0, 50.0));
    assert_eq!(
        g.cells_in_bounds(min, max, g.cell, CAP),
        Some(hand_floor_rect(min, max, g.cell))
    );
    // Wholly negative.
    let (min, max) = ((-330.0, -220.0), (-110.0, -140.0));
    assert_eq!(
        g.cells_in_bounds(min, max, g.cell, CAP),
        Some(hand_floor_rect(min, max, g.cell))
    );
}

#[test]
fn square_cells_in_bounds_is_row_major() {
    let g = SquareGrid {
        cell: 100.0,
        rule: DiagonalRule::Chebyshev,
    };
    let got = g
        .cells_in_bounds((0.0, 0.0), (150.0, 150.0), g.cell, CAP)
        .unwrap();
    // for i { for j } → (0,0),(0,1),(1,0),(1,1).
    assert_eq!(got, vec![(0, 0), (0, 1), (1, 0), (1, 1)]);
}

#[test]
fn cells_in_bounds_span_cap_returns_none() {
    // 3001×3001 ≈ 9.0M candidate cells > the 4M MAX_CELLS_PER_POLYGON cap → None.
    let g = SquareGrid {
        cell: 1.0,
        rule: DiagonalRule::Chebyshev,
    };
    assert_eq!(
        g.cells_in_bounds((0.0, 0.0), (3000.0, 3000.0), g.cell, CAP),
        None
    );
    // A hex scene with an equally-huge AABB is capped by the same span check.
    let hx = HexGrid { size: 1.0 };
    assert_eq!(
        hx.cells_in_bounds((0.0, 0.0), (5000.0, 5000.0), hx.size, CAP),
        None
    );
}

#[test]
fn cells_in_bounds_honors_the_caller_supplied_cap() {
    // A 3×3 (9-cell) AABB is accepted under the vision cap but rejected under a tighter
    // per-caller bound — the cap is the passed `max_cells`, never a hardcoded constant, so
    // routing a tighter-capped caller (region rasterization) through this primitive can't
    // loosen its bound.
    let g = SquareGrid {
        cell: 100.0,
        rule: DiagonalRule::Chebyshev,
    };
    let (min, max) = ((0.0, 0.0), (250.0, 250.0));
    assert!(g.cells_in_bounds(min, max, g.cell, CAP).is_some());
    assert_eq!(
        g.cells_in_bounds(min, max, g.cell, 8),
        None,
        "9 cells > cap of 8"
    );
    // Same for hex — the span cap is honored on both grid kinds.
    let hx = HexGrid { size: 50.0 };
    assert_eq!(
        hx.cells_in_bounds((0.0, 0.0), (200.0, 200.0), hx.size, 1),
        None
    );
}

#[test]
fn exceeds_cell_cap_is_strictly_greater_than_at_the_boundary() {
    // Anti-drift on the predicate's own polarity, pinned exactly at the boundary: a `>` → `>=`
    // flip would refuse an otherwise-legal exactly-at-cap scan at every one of the three sites
    // that call this predicate — the total mask loss `exceeds_cell_cap` exists to prevent.
    // `bounds = (0, 0, w-1, 0)` gives `candidate_span` exactly `w` (h = 1), so setting `w` to
    // `CAP` and to `CAP + 1` isolates the boundary through the span formula itself.
    // Discrimination: fails if `exceeds_cell_cap`'s `>` becomes `>=`.
    assert!(!exceeds_cell_cap((0, 0, (CAP - 1) as i32, 0), CAP));
    assert!(exceeds_cell_cap((0, 0, CAP as i32, 0), CAP));
}

#[test]
fn cells_in_bounds_fails_closed_on_degenerate_input() {
    let g = SquareGrid {
        cell: 100.0,
        rule: DiagonalRule::Chebyshev,
    };
    assert_eq!(
        g.cells_in_bounds((f64::NAN, 0.0), (10.0, 10.0), g.cell, CAP),
        None
    );
    assert_eq!(
        g.cells_in_bounds((0.0, 0.0), (f64::INFINITY, 10.0), g.cell, CAP),
        None
    );
    assert_eq!(g.cells_in_bounds((0.0, 0.0), (10.0, 10.0), 0.0, CAP), None);
    assert_eq!(g.cells_in_bounds((0.0, 0.0), (10.0, 10.0), -5.0, CAP), None);
    let hx = HexGrid { size: 50.0 };
    assert_eq!(
        hx.cells_in_bounds((0.0, 0.0), (10.0, f64::NAN), hx.size, CAP),
        None
    );
    assert_eq!(hx.cells_in_bounds((0.0, 0.0), (10.0, 10.0), 0.0, CAP), None);
}

#[test]
fn hex_cells_in_bounds_includes_every_cell_whose_center_is_in_the_aabb() {
    let g = HexGrid { size: 50.0 };
    let (min, max) = ((0.0, 0.0), (200.0, 200.0));
    let got = g
        .cells_in_bounds(min, max, g.size, CAP)
        .expect("bounded, not over-cap");
    let got_set: BTreeSet<Cell> = got.iter().copied().collect();
    // The candidate set must be a SUPERSET of every hex whose center lies in the AABB.
    let mut center_in_count = 0;
    for q in -30..=30 {
        for r in -30..=30 {
            let ctr = g.cell_center((q, r));
            if ctr.0 >= min.0 && ctr.0 <= max.0 && ctr.1 >= min.1 && ctr.1 <= max.1 {
                center_in_count += 1;
                assert!(
                    got_set.contains(&(q, r)),
                    "hex ({q},{r}) center {ctr:?} in AABB but missing from cells_in_bounds"
                );
            }
        }
    }
    assert!(
        center_in_count > 0,
        "fixture must exercise at least one in-bounds hex"
    );
    // Stays bounded — a tight superset of a ~4×4-hex region, not a runaway scan.
    assert!(
        got.len() < 100,
        "hex candidate set should stay small for a small AABB"
    );
}

#[test]
fn square_cell_bounds_is_the_corner_floor_box() {
    // Discrimination: pins `floor(min/cell)`/`floor(max/cell)` per axis, on signed-straddling
    // and wholly-negative input.
    let g = SquareGrid {
        cell: 100.0,
        rule: DiagonalRule::Chebyshev,
    };
    assert_eq!(
        g.cell_bounds((0.0, -250.0), (250.0, 50.0), g.cell),
        (0, -3, 2, 0)
    );
    assert_eq!(
        g.cell_bounds((-330.0, -220.0), (-110.0, -140.0), g.cell),
        (-4, -3, -2, -2)
    );
}

#[test]
fn hex_cell_bounds_contains_a_cell_a_square_floor_window_would_clip() {
    // Hex (70,-70): the "off the square diagonal" axial (1,-1) direction. Its pixel x is
    // `√3/2·70·size`, so a square `floor(x/cell)` q-bound sits far BELOW its axial q (=70) —
    // a square floor window would clip it here, spuriously reporting a reachable route
    // Unreachable. `cell_bounds`'s axial bounding box must not. The window margin is read from
    // `pathfinding::WINDOW_MARGIN` rather than restated, since the clipping this fixture
    // demonstrates is a property of that value.
    let g = HexGrid { size: 100.0 };
    let goal = (70, -70);
    let gc = g.cell_center(goal);
    // AABB of start (0,0) and the goal center.
    let (min, max) = ((0.0, gc.1), (gc.0, 0.0));
    let (i0, j0, i1, j1) = g.cell_bounds(min, max, g.size);
    assert!(
        i0 <= goal.0 && goal.0 <= i1,
        "axial q {} must lie in [{i0},{i1}]",
        goal.0
    );
    assert!(
        j0 <= goal.1 && goal.1 <= j1,
        "axial r {} must lie in [{j0},{j1}]",
        goal.1
    );
    // The square floor of the pixel-x max caps the q-bound strictly below the goal's axial q:
    // this is exactly the clipping the hex-correct bounds avoid.
    assert!(
        (gc.0 / g.size).floor() as i32 + pathfinding::WINDOW_MARGIN < goal.0,
        "a square floor(max_x/cell)+margin window would clip axial q={}",
        goal.0
    );
}

#[test]
fn square_cell_vertices_returns_four_corners_in_order() {
    // The four corners are the cell's own centre offset by half a cell on each axis, in
    // (top-left, top-right, bottom-left, bottom-right) order. Expressed against `cell_center`
    // rather than as a literal table, so it pins the RELATIONSHIP between the two primitives
    // instead of restating the corner formula the impl already has.
    // Discrimination: fails if the order changes, if a corner picks up an asymmetry, or if
    // the vertices stop being centred on `cell_center`'s point.
    let g = SquareGrid {
        cell: 100.0,
        rule: DiagonalRule::Chebyshev,
    };
    let c = g.cell_center((2, 3));
    let h = g.cell / 2.0;
    assert_eq!(
        g.cell_vertices((2, 3), g.cell),
        vec![
            (c.0 - h, c.1 - h),
            (c.0 + h, c.1 - h),
            (c.0 - h, c.1 + h),
            (c.0 + h, c.1 + h)
        ]
    );
}

/// Independent reference for the intended axial (cube) hex distance, so the assertion
/// isn't tautological with the impl under test.
fn axial_distance(from: Cell, to: Cell) -> f64 {
    let dq = to.0 as i64 - from.0 as i64;
    let dr = to.1 as i64 - from.1 as i64;
    ((dq.abs() + dr.abs() + (dq + dr).abs()) as f64) / 2.0
}

#[test]
fn hex_heuristic_equals_axial_distance_including_opposite_sign_deltas() {
    let g = HexGrid { size: 50.0 };
    // Same-sign axial delta (the (1,1) direction): distance = |dq| + |dr|.
    assert_eq!(g.heuristic((0, 0), (3, 3)), axial_distance((0, 0), (3, 3)));
    assert_eq!(g.heuristic((0, 0), (3, 3)), 6.0);
    // Opposite-sign axial delta (the (1,-1) direction): distance = max(|dq|, |dr|) — this is
    // the case the square Manhattan/Euclidean estimate OVERESTIMATES.
    assert_eq!(
        g.heuristic((0, 0), (4, -4)),
        axial_distance((0, 0), (4, -4))
    );
    assert_eq!(g.heuristic((0, 0), (4, -4)), 4.0);
    // Mixed / off-axis deltas, both delta orderings.
    assert_eq!(
        g.heuristic((2, -1), (5, -6)),
        axial_distance((2, -1), (5, -6))
    );
    assert_eq!(
        g.heuristic((5, -6), (2, -1)),
        axial_distance((5, -6), (2, -1))
    );
    // Symmetric.
    assert_eq!(g.heuristic((-3, 2), (1, -4)), g.heuristic((1, -4), (-3, 2)));
    // Zero delta.
    assert_eq!(g.heuristic((7, -2), (7, -2)), 0.0);
}

/// Shortest-path cost from `from` to every cell of the open box `|i| <= r`, `|j| <= r`,
/// relaxed to a fixpoint over the REAL `GridShape::neighbors_with_cost` edges (Bellman-Ford —
/// chosen over Dijkstra because it needs no ordering of `f64` keys and stays obviously correct
/// for the non-uniform square step costs). The reference cost therefore comes from the neighbor
/// function itself, never from a restatement of the heuristic's formula.
///
/// The axial box is closed under shortest hex paths: a shortest path can always be taken
/// monotone in cube coordinates (at most two of the six directions, each step moving `q` and
/// `r` monotonically toward the target), so every intermediate cell has `q` between the two
/// endpoints' `q` and `r` between their `r`. Confining the search to the box thus cannot
/// inflate the true cost between any two of its cells.
fn true_costs_from(g: &dyn GridShape, from: Cell, r: i32) -> std::collections::BTreeMap<Cell, f64> {
    let cells: Vec<Cell> = (-r..=r)
        .flat_map(|i| (-r..=r).map(move |j| (i, j)))
        .collect();
    let mut dist: std::collections::BTreeMap<Cell, f64> =
        cells.iter().map(|&c| (c, f64::INFINITY)).collect();
    dist.insert(from, 0.0);
    loop {
        let mut changed = false;
        for &c in &cells {
            let d = dist[&c];
            if !d.is_finite() {
                continue;
            }
            for (next, step, _) in g.neighbors_with_cost(c, 0) {
                if !dist.contains_key(&next) {
                    continue; // outside the searched box
                }
                if d + step < dist[&next] - 1e-12 {
                    dist.insert(next, d + step);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    dist.retain(|_, d| d.is_finite());
    dist
}

#[test]
fn hex_heuristic_never_exceeds_the_true_neighbor_graph_cost() {
    // Admissibility against the ACTUAL neighbor graph: for every ordered pair in an open
    // radius-4 axial box, the heuristic must lower-bound the cost of the cheapest route
    // `neighbors_with_cost` admits. A heuristic that overestimates any reachable pair makes A*
    // return suboptimal hex routes.
    const R: i32 = 4;
    let g = HexGrid { size: 50.0 };
    let cells: Vec<Cell> = (-R..=R)
        .flat_map(|q| (-R..=R).map(move |r| (q, r)))
        .collect();
    let mut checked = 0usize;
    let mut tight = 0usize;
    for &from in &cells {
        let costs = true_costs_from(&g, from, R);
        for &to in &cells {
            let true_cost = *costs
                .get(&to)
                .unwrap_or_else(|| panic!("open box is connected: {from:?} -> {to:?}"));
            let h = g.heuristic(from, to);
            assert!(
                h <= true_cost + 1e-9,
                "heuristic {h} exceeds true neighbor-graph cost {true_cost} for {from:?} -> {to:?}"
            );
            checked += 1;
            if to != from && (h - true_cost).abs() < 1e-9 {
                tight += 1;
            }
        }
    }
    assert_eq!(
        checked,
        cells.len() * cells.len(),
        "every ordered pair is covered"
    );
    // Non-vacuity: the bound is not passing merely because every true cost is huge — the
    // heuristic is EXACT (tight) on non-trivial pairs, which is what keeps A* efficient.
    assert!(
        tight > 0,
        "the heuristic must be tight on at least one non-trivial pair"
    );
}

#[test]
fn square_heuristic_never_exceeds_the_true_neighbor_graph_cost() {
    // Same independent check for every square diagonal rule, so the trait's admissibility
    // contract is proven on both grid kinds against their own neighbor functions. `Alternating`
    // is excluded: its step cost depends on the carried parity, so a parity-blind uniform-cost
    // search is not its true cost function (its admissibility is pinned by the square parity
    // tests in `pathfinding::alternating_five_ten_five_parity`).
    const R: i32 = 4;
    let cells: Vec<Cell> = (-R..=R)
        .flat_map(|i| (-R..=R).map(move |j| (i, j)))
        .collect();
    for rule in [
        DiagonalRule::Chebyshev,
        DiagonalRule::Manhattan,
        DiagonalRule::Euclidean,
    ] {
        let g = SquareGrid { cell: 100.0, rule };
        for &from in &cells {
            let costs = true_costs_from(&g, from, R);
            for &to in &cells {
                let true_cost = costs[&to];
                let h = g.heuristic(from, to);
                assert!(
                    h <= true_cost + 1e-9,
                    "{rule:?}: heuristic {h} exceeds true cost {true_cost} for {from:?} -> {to:?}"
                );
            }
        }
    }
}

#[test]
fn square_manhattan_overestimates_the_true_hex_distance_on_opposite_sign_deltas() {
    // Documents why a square-grid distance is not admissible as a hex heuristic: on the
    // (1,-1) axial line the true hex distance is `max(|dq|,|dr|)` but the square Manhattan
    // distance is `|dq|+|dr|` — a 2× overestimate. `HexGrid::heuristic` returns the
    // admissible value instead.
    let hex = HexGrid { size: 50.0 };
    let sq_manhattan = SquareGrid {
        cell: 50.0,
        rule: DiagonalRule::Manhattan,
    };
    let (from, to) = ((0, 0), (4, -4));
    assert_eq!(hex.heuristic(from, to), 4.0, "true hex distance");
    assert_eq!(
        sq_manhattan.heuristic(from, to),
        8.0,
        "square Manhattan overestimates 2x"
    );
    assert!(sq_manhattan.heuristic(from, to) > hex.heuristic(from, to));
}

#[test]
fn square_heuristic_is_byte_identical_to_the_free_pathfinding_heuristic() {
    // `astar_leg` calls `SquareGrid::heuristic` (via the `GridShape` trait) for every square
    // route; this pins it equal to the free `pathfinding::heuristic(self.rule, ...)` for every
    // rule, including `Alternating` (excluded from
    // `square_heuristic_never_exceeds_the_true_neighbor_graph_cost`).
    for rule in [
        DiagonalRule::Chebyshev,
        DiagonalRule::Manhattan,
        DiagonalRule::Euclidean,
        DiagonalRule::Alternating,
    ] {
        let g = SquareGrid { cell: 100.0, rule };
        for &(from, to) in &[((0, 0), (3, 5)), ((-2, 4), (7, -1)), ((5, 5), (5, 5))] {
            assert_eq!(
                g.heuristic(from, to),
                crate::scene::pathfinding::heuristic(rule, from, to),
                "square heuristic must equal the free rule-based heuristic for {rule:?}"
            );
        }
    }
}

/// EXACT oracle: the set of hexes whose INTERIOR the segment `a -> b` overlaps over a
/// positive-length interval, computed by Cyrus-Beck clipping of the segment against each
/// candidate hex's 6 half-planes (`cell_vertices`). Independent of `line_traversal`.
fn true_hexes_crossed(g: &HexGrid, a: vision::P, b: vision::P, eps: f64) -> BTreeSet<Cell> {
    let (ca, cb) = (g.cell_of(a), g.cell_of(b));
    let pad = 4;
    let (q0, q1) = (ca.0.min(cb.0) - pad, ca.0.max(cb.0) + pad);
    let (r0, r1) = (ca.1.min(cb.1) - pad, ca.1.max(cb.1) + pad);
    let d = (b.0 - a.0, b.1 - a.1);
    let len = (d.0 * d.0 + d.1 * d.1).sqrt();
    let mut out = BTreeSet::new();
    for q in q0..=q1 {
        for r in r0..=r1 {
            let c = (q, r);
            let verts = g.cell_vertices(c, 0.0);
            let ctr = g.cell_center(c);
            let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
            let mut empty = false;
            for k in 0..6 {
                let v0 = verts[k];
                let v1 = verts[(k + 1) % 6];
                // Inward normal for edge v0->v1 (sign fixed by the center being inside).
                let e = (v1.0 - v0.0, v1.1 - v0.1);
                let mut n = (-e.1, e.0);
                if n.0 * (ctr.0 - v0.0) + n.1 * (ctr.1 - v0.1) < 0.0 {
                    n = (-n.0, -n.1);
                }
                // f(t) = n . (a + t*d - v0) >= 0 keeps the point inside.
                let f0 = n.0 * (a.0 - v0.0) + n.1 * (a.1 - v0.1);
                let fd = n.0 * d.0 + n.1 * d.1;
                if fd.abs() < 1e-18 {
                    if f0 < 0.0 {
                        empty = true;
                        break;
                    }
                    continue;
                }
                let t = -f0 / fd;
                if fd > 0.0 {
                    t0 = t0.max(t);
                } else {
                    t1 = t1.min(t);
                }
                if t0 > t1 {
                    empty = true;
                    break;
                }
            }
            if !empty && (t1 - t0) * len > eps {
                out.insert(c);
            }
        }
    }
    out
}

/// Deterministic LCG so the search is reproducible.
struct Lcg(u64);
impl Lcg {
    fn next_f64(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }
}

/// Structured adversarial endpoint pool: hex centers, all 6 vertices, all 6 edge midpoints,
/// and small offsets off each, for a ring of hexes around the origin.
fn adversarial_points(g: &HexGrid) -> Vec<vision::P> {
    let mut pts = Vec::new();
    for q in -2..=2 {
        for r in -2..=2 {
            let c = (q, r);
            let ctr = g.cell_center(c);
            pts.push(ctr);
            let verts = g.cell_vertices(c, 0.0);
            for k in 0..6 {
                let v = verts[k];
                let w = verts[(k + 1) % 6];
                pts.push(v);
                pts.push(((v.0 + w.0) * 0.5, (v.1 + w.1) * 0.5));
                // Just inside / just outside the vertex, along the center->vertex ray.
                for f in [0.999_999, 1.000_001, 0.5] {
                    pts.push((ctr.0 + (v.0 - ctr.0) * f, ctr.1 + (v.1 - ctr.1) * f));
                }
            }
        }
    }
    pts
}

/// THE security property: `line_traversal` must never omit a hex the segment geometrically
/// crosses, or a non-GM could route a move that grazes an unseen hex without that hex being
/// checked against the visibility mask (a fog probe) — the exact failure the square supercover
/// is documented to prevent. Checked against `true_hexes_crossed`, an EXACT oracle (Cyrus-Beck
/// clipping against each hex's own `cell_vertices` polygon), not dense sampling — dense
/// sampling is only a subset oracle and would pass over a barely-clipped hex.
///
/// Search: 3 hex sizes × {random long segments, random sub-hex segments, pairs drawn from a
/// pool of hex centers/vertices/edge-midpoints/near-vertex offsets, and chords parallel to each
/// of the 6 hex edge directions at 121 perpendicular offsets each}.
#[test]
fn hex_line_traversal_never_omits_a_hex_the_segment_crosses() {
    use std::cell::{Cell as C, RefCell};
    let misses = C::new(0usize);
    let first: RefCell<Option<String>> = RefCell::new(None);
    let checked = C::new(0usize);
    let over = C::new(0usize);
    let over_max = C::new(0usize);
    let check = |g: &HexGrid, a: vision::P, b: vision::P, eps: f64| {
        let got = match g.line_traversal(a, b, g.size) {
            Some(s) => s,
            None => return,
        };
        let want = true_hexes_crossed(g, a, b, eps);
        checked.set(checked.get() + 1);
        let extra = got.len() - want.intersection(&got).count();
        over.set(over.get() + extra);
        over_max.set(over_max.get().max(extra));
        if !want.is_subset(&got) {
            misses.set(misses.get() + 1);
            if first.borrow().is_none() {
                let missing: Vec<Cell> = want.difference(&got).copied().collect();
                *first.borrow_mut() = Some(format!(
                    "size={} a={a:?} b={b:?}\n got={got:?}\nwant={want:?}\nmissing={missing:?}",
                    g.size
                ));
            }
        }
    };
    for &size in &[1.0_f64, 50.0, 137.5] {
        let g = HexGrid { size };
        let mut rng = Lcg(0x5EED_1234);
        // Random long + short segments.
        for _ in 0..8000 {
            let sp = size * 6.0;
            let a = (rng.range(-sp, sp), rng.range(-sp, sp));
            let b = (rng.range(-sp, sp), rng.range(-sp, sp));
            check(&g, a, b, 1e-9 * size);
            let tiny = size * 0.01;
            let b2 = (a.0 + rng.range(-tiny, tiny), a.1 + rng.range(-tiny, tiny));
            check(&g, a, b2, 1e-9 * size);
        }
        // Structured adversarial pairs.
        let pts = adversarial_points(&g);
        let mut rng2 = Lcg(0xC0FF_EE01);
        for _ in 0..20000 {
            let a = pts[(rng2.next_f64() * pts.len() as f64) as usize % pts.len()];
            let b = pts[(rng2.next_f64() * pts.len() as f64) as usize % pts.len()];
            check(&g, a, b, 1e-9 * size);
        }
        // Segments parallel to each of the three hex edge directions, at many offsets.
        for k in 0..6 {
            let ang = std::f64::consts::PI / 180.0 * (60.0 * k as f64);
            let (dx, dy) = (ang.cos(), ang.sin());
            let (px, py) = (-dy, dx);
            for i in -60..=60 {
                let off = i as f64 * size / 37.0;
                let a = (px * off - dx * size * 4.0, py * off - dy * size * 4.0);
                let b = (px * off + dx * size * 4.0, py * off + dy * size * 4.0);
                check(&g, a, b, 1e-9 * size);
            }
        }
    }
    if let Some(f) = first.borrow().clone() {
        panic!(
            "omitted a crossed hex in {}/{}: {f}",
            misses.get(),
            checked.get()
        );
    }
    assert!(
        checked.get() > 100_000,
        "the search must actually run: {}",
        checked.get()
    );
    // Over-inclusion is the SAFE direction (it can only over-restrict a move, never over-permit),
    // but it must stay bounded or legitimate moves get rejected. The boundary probe adds at most
    // a small constant per segment, and only for segments that genuinely run along or through a
    // hex boundary — generic segments (the random groups) get essentially none.
    assert!(
        over_max.get() <= 12,
        "per-segment over-inclusion must stay small, got {}",
        over_max.get()
    );
}

/// Generic (non-boundary-aligned) segments must be effectively EXACT — the conservative
/// boundary probe must not silently over-restrict ordinary moves.
#[test]
fn hex_line_traversal_is_near_exact_for_generic_segments() {
    let g = HexGrid { size: 50.0 };
    let mut rng = Lcg(0x5EED_1234);
    let (mut checked, mut extra_total) = (0usize, 0usize);
    for _ in 0..8000 {
        let a = (rng.range(-300.0, 300.0), rng.range(-300.0, 300.0));
        let b = (rng.range(-300.0, 300.0), rng.range(-300.0, 300.0));
        let Some(got) = g.line_traversal(a, b, g.size) else {
            continue;
        };
        let want = true_hexes_crossed(&g, a, b, 1e-9 * g.size);
        assert!(want.is_subset(&got), "a={a:?} b={b:?}");
        extra_total += got.len() - want.intersection(&got).count();
        checked += 1;
    }
    assert!(checked > 7900);
    assert!(
        // Measured true value is 0 spurious cells on generic segments; 1-in-1000 leaves 10x
        // slack without sleeping through a real regression (a 100x bound was shown to pass
        // even with the vertex probe inflated 100-fold).
        extra_total * 1000 < checked,
        "fewer than 1 in 1000 generic segments may gain a spurious hex: {extra_total}/{checked}"
    );
}

#[test]
fn hex_line_traversal_includes_the_far_endpoint_hex_on_a_sub_hex_step() {
    // Reduced counterexample. A step short enough that the max cube-axis delta rounds to 0 still
    // crosses a hex boundary: (-40, 0) sits in hex (0,0) (inradius 43.3), (-50, 0) in hex (-1,0).
    // A naive fixed-sample-count line-draw (sampling `n+1` points, `n` = max cube-axis delta)
    // takes `n = 0` here and would return ONLY the start hex, omitting even the far endpoint's
    // own hex — an unseen hex a token could step into ungated. `HexGrid::line_traversal`'s
    // ψ-crossing supercover must still catch it.
    let g = HexGrid { size: 50.0 };
    let (a, b) = ((-40.0, 0.0), (-50.0, 0.0));
    assert_eq!(g.cell_of(a), (0, 0));
    assert_eq!(g.cell_of(b), (-1, 0));
    let cells = g.line_traversal(a, b, g.size).expect("finite, in-bounds");
    assert!(cells.contains(&(0, 0)), "start hex: {cells:?}");
    assert!(
        cells.contains(&(-1, 0)),
        "far ENDPOINT hex must be present: {cells:?}"
    );
}

#[test]
fn hex_line_traversal_includes_a_corner_clipped_hex() {
    // Reduced counterexample. This segment clips a short sliver of hex (-1,0) near the vertex it
    // shares with (0,0)/(0,-1); a fixed one-hex-pitch sample spacing would straddle the sliver
    // entirely and never name it. `HexGrid::line_traversal`'s ψ-crossing supercover must
    // still catch it.
    let g = HexGrid { size: 50.0 };
    let a = (-51.640_685_867_085_56, 82.356_123_493_857_9);
    let b = (-32.076_474_134_396_726, -54.002_821_619_560_066);
    let cells = g.line_traversal(a, b, g.size).expect("finite, in-bounds");
    let want = true_hexes_crossed(&g, a, b, 1e-6);
    assert!(
        want.contains(&(-1, 0)),
        "fixture must actually clip (-1,0): {want:?}"
    );
    assert!(want.is_subset(&cells), "want={want:?} got={cells:?}");
}

#[test]
fn hex_line_traversal_emits_both_hexes_flanking_a_traversed_edge() {
    // A segment running exactly along the shared edge of two hexes touches both. Same deliberate
    // over-inclusion the square supercover documents for a shared corner, so a move cannot
    // thread an unseen hex by riding a boundary.
    let g = HexGrid { size: 50.0 };
    // Hexes (0,0) (centred on the origin) and (-1,0) (one pitch to its left) share the
    // vertical edge at `x = -√3/2·size`, spanning `y` in `[-size/2, size/2]`; the probe runs
    // along it, stopping short of both ends.
    let x = -g.size * 3.0_f64.sqrt() / 2.0;
    let y = g.size * 0.4;
    let cells = g
        .line_traversal((x, -y), (x, y), g.size)
        .expect("finite, in-bounds");
    assert!(cells.contains(&(0, 0)), "{cells:?}");
    assert!(cells.contains(&(-1, 0)), "{cells:?}");
}

#[test]
fn hex_line_traversal_fails_closed_on_non_finite_and_over_cap() {
    let g = HexGrid { size: 50.0 };
    assert_eq!(
        g.line_traversal((f64::NAN, 0.0), (10.0, 10.0), g.size),
        None
    );
    assert_eq!(
        g.line_traversal((0.0, 0.0), (f64::INFINITY, 0.0), g.size),
        None
    );
    assert_eq!(
        g.line_traversal((0.0, f64::NEG_INFINITY), (0.0, 0.0), g.size),
        None
    );
    // Past the 4096-hex span cap.
    assert_eq!(g.line_traversal((0.0, 0.0), (1.0e9, 0.0), g.size), None);
}

#[test]
fn hex_cell_vertices_returns_six_points_at_radius_size() {
    let g = HexGrid { size: 50.0 };
    let center = g.cell_center((1, -2));
    let verts = g.cell_vertices((1, -2), g.size);
    assert_eq!(verts.len(), 6, "a hex has 6 vertices");
    for &(x, y) in &verts {
        let d = ((x - center.0).powi(2) + (y - center.1).powi(2)).sqrt();
        assert!(
            (d - g.size).abs() < 1e-9,
            "each hex vertex sits at radius = size from center, got {d}"
        );
    }
}

#[test]
fn square_world_units_per_cell_is_the_cell_size() {
    // Discrimination: fails if the square implementation returns anything other than its own
    // `cell` — including a hex-shaped formula accidentally shared between the two impls.
    let g = SquareGrid {
        cell: 37.5,
        rule: DiagonalRule::Chebyshev,
    };
    assert_eq!(g.world_units_per_cell(), 37.5);
}

#[test]
fn hex_world_units_per_cell_equals_the_distance_between_adjacent_centers() {
    // Derived from the geometry, not from the implementation: the value must equal the
    // measured distance from a hex's centre to each of its six neighbours' centres.
    // Discrimination: fails for `size`, `1.5*size`, or any constant other than `√3*size`,
    // because the expectation is computed from `cell_center` rather than restated. This is a
    // SELF-consistency check only: the oracle is derived from `HexGrid`'s own `cell_center`, so
    // a formula change that updates `cell_center`/`world_units_per_cell` together stays
    // self-consistent and passes even if it drifts from the client's convention. Not a
    // cross-language parity pin — see the literal-value pin below for that.
    let g = HexGrid { size: 50.0 };
    let origin = g.cell_center((0, 0));
    for (dq, dr) in [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)] {
        let n = g.cell_center((dq, dr));
        let d = ((n.0 - origin.0).powi(2) + (n.1 - origin.1).powi(2)).sqrt();
        assert!(
            (d - g.world_units_per_cell()).abs() < 1e-9,
            "neighbour ({dq},{dr}) sits {d} away, world_units_per_cell reports {}",
            g.world_units_per_cell()
        );
    }
}

#[test]
fn hex_world_units_per_cell_matches_the_cross_language_pinned_literal() {
    // Cross-language parity pin. Neither side can share a symbol across the Rust/TS boundary,
    // so both this test and the client's `hex per-step distance (worldUnitsPerCell) equals
    // the cross-language pinned literal` sibling state the SAME literal
    // value (`50 * sqrt(3)`) independently, rather than each deriving its expectation from its
    // own implementation as the self-consistency test above does. A literal is not a
    // decision-maker two sites can disagree over — it is the ground truth both sides are
    // checked against — so a formula change on EITHER side alone now fails that side's own
    // comparison against this literal, where two self-derived oracles could update in lockstep
    // and silently diverge from the other language.
    let g = HexGrid { size: 50.0 };
    assert!((g.world_units_per_cell() - 86.60254037844386_f64).abs() < 1e-9);
}

#[test]
fn each_shapes_envelope_starts_at_its_own_origin_cells_lower_left_extreme() {
    let size = 50.0_f64;
    let sq = SquareGrid {
        cell: size,
        rule: DiagonalRule::Chebyshev,
    };
    let hx = HexGrid { size };

    // Discrimination: fails if `world_extent` returns an origin-anchored rectangle on hex, or if
    // the square arm gains a spurious negative margin from a shared normalisation path.
    let s = sq.world_extent((8.0, 6.0));
    assert_eq!(
        s.min,
        (0.0, 0.0),
        "a square block's origin cell starts AT the origin"
    );

    let h = hx.world_extent((8.0, 6.0));
    let (cx, cy) = hx.cell_center((0, 0));
    assert_eq!(
        cy, 0.0,
        "fixture guard: axial (0,0)'s centre is the origin row"
    );
    assert!(
        (h.min.0 - (cx - 3.0_f64.sqrt() / 2.0 * size)).abs() < 1e-9,
        "the envelope's x minimum is the origin hex's own left inradius, got {}",
        h.min.0
    );
    assert!(
        (h.min.1 - (cy - size)).abs() < 1e-9,
        "the envelope's y minimum is the origin hex's own bottom circumradius, got {}",
        h.min.1
    );
    assert!(
        h.width() > 0.0 && h.height() > 0.0,
        "a positive block has a positive envelope"
    );
}

#[test]
fn square_world_extent_wholly_contains_every_cell_of_the_integer_block() {
    // Square's guarantee is a FULL cover of the INTEGER block: every such cell's own rectangle
    // lies inside the extent. The authored dimensions here are already integral, so this
    // exercises the integer block only; the fractional case is separate.
    // Discrimination: fails if the square extent gains a margin, loses a cell through a
    // `w - 1` term, or picks up a per-axis asymmetry — the containment check is per cell and
    // the closed form is asserted separately.
    let g = SquareGrid {
        cell: 20.0,
        rule: DiagonalRule::Chebyshev,
    };
    let (w, h) = (8.0_f64, 5.0_f64);
    let e = g.world_extent((w, h));
    let (ex, ey) = e.max;
    let half = g.cell / 2.0;
    for i in 0..w as i32 {
        for j in 0..h as i32 {
            let c = g.cell_center((i, j));
            assert!(
                c.0 + half <= ex + 1e-9,
                "cell ({i},{j}) exceeds extent x {ex}"
            );
            assert!(
                c.1 + half <= ey + 1e-9,
                "cell ({i},{j}) exceeds extent y {ey}"
            );
            assert!(
                c.0 - half >= e.min.0 - 1e-9 && c.1 - half >= e.min.1 - 1e-9,
                "cell ({i},{j}) starts below the envelope's minimum {:?}",
                e.min
            );
        }
    }
    // The closed form, stated as NUMBERS rather than as `w · cell`: written as the product it
    // would restate the impl's own arithmetic and pin nothing. A change to `cell` fails it
    // loudly, which is the whole reason a literal is admissible for it and not for the
    // per-cell containment checks, where a stale half-cell would silently weaken them instead.
    assert_eq!((e.min, e.max), ((0.0, 0.0), (160.0, 100.0)));
}

#[test]
fn hex_world_extent_contains_every_cell_centre_and_the_far_cells_vertices() {
    // Hex's `max`-side guarantee over the INTEGER block is a CENTRE cover plus the extreme
    // cell's far vertices — not a full cover, since a partial column or row past `max` stays
    // outside. The authored dimensions here are integral, so this exercises the integer block
    // only.
    // Discrimination: fails for a `w*size`/`h*size` reading, for a per-axis pitch that omits
    // the axial shear, and for any formula that leaves the far cell's own far vertex outside
    // — every check is derived from `cell_center` plus the pointy-top half-extents.
    let g = HexGrid { size: 50.0 };
    let (w, h) = (9.0_f64, 7.0_f64);
    let e = g.world_extent((w, h));
    let (ex, ey) = e.max;
    let half_x = g.size * 3.0_f64.sqrt() / 2.0;
    for q in 0..w as i32 {
        for r in 0..h as i32 {
            let c = g.cell_center((q, r));
            assert!(
                c.0 >= e.min.0 - 1e-9 && c.1 >= e.min.1 - 1e-9,
                "hex ({q},{r}) centre is below the envelope's minimum {:?}",
                e.min
            );
            assert!(
                c.0 <= ex + 1e-9 && c.1 <= ey + 1e-9,
                "hex ({q},{r}) centre exceeds the extent"
            );
        }
    }
    let far = g.cell_center((w as i32 - 1, h as i32 - 1));
    assert!(
        far.0 + half_x <= ex + 1e-9,
        "the far hex's right vertex exceeds extent x {ex}"
    );
    assert!(
        far.1 + g.size <= ey + 1e-9,
        "the far hex's bottom vertex exceeds extent y {ey}"
    );
}

#[test]
fn hex_world_extent_contains_the_origin_cells_whole_polygon() {
    // The origin hex is centred ON the origin, so its lower vertex sits at `(0, -size)` and its
    // left vertices at `x = -√3/2·size`. Those are cells the GM authored, and the envelope
    // reaches them: its minimum IS that hex's own lower-left extreme. Checked through
    // `cell_vertices`, so the claim covers the whole polygon rather than two sampled points,
    // and the extremes come from the shape's own geometry rather than a restated formula.
    //
    // Discrimination, per assertion, each naming a mutation that breaks THAT assertion while
    // the other still holds:
    // - the origin loop fails if `HexGrid::world_extent`'s `min` moves to the origin, or if
    //   either half-extent shrinks on the min side. The far loop is unaffected by both.
    // - the far loop fails if `max` loses either half-extent, which leaves the far hex's own
    //   far vertices outside while every origin vertex stays inside.
    let g = HexGrid { size: 50.0 };
    let (w, h) = (9.0_f64, 7.0_f64);
    let e = g.world_extent((w, h));
    let inside = |p: (f64, f64)| {
        p.0 >= e.min.0 - 1e-9
            && p.1 >= e.min.1 - 1e-9
            && p.0 <= e.max.0 + 1e-9
            && p.1 <= e.max.1 + 1e-9
    };
    for v in g.cell_vertices((0, 0), g.size) {
        assert!(
            inside(v),
            "the origin hex's vertex {v:?} must lie inside the envelope {e:?}"
        );
    }
    for v in g.cell_vertices((w as i32 - 1, h as i32 - 1), g.size) {
        assert!(
            inside(v),
            "the far hex's vertex {v:?} must lie inside the envelope {e:?}"
        );
    }
}

#[test]
fn hex_world_extent_exceeds_the_bounds_size_product_on_both_axes() {
    // The axial shear makes a w×h block a rhombus, so its covering rectangle is wider than a
    // per-axis pitch product on both axes.
    // Discrimination: fails if the hex impl falls back to the square formula.
    let g = HexGrid { size: 50.0 };
    let (w, h) = (40.0_f64, 40.0_f64);
    let (ex, ey) = g.world_extent((w, h)).max;
    assert!(
        ex > w * g.size,
        "hex extent x {ex} must exceed {}",
        w * g.size
    );
    assert!(
        ey > h * g.size,
        "hex extent y {ey} must exceed {}",
        h * g.size
    );
}

#[test]
fn world_extent_stays_positive_below_one_cell_and_covers_at_most_the_origin_cell() {
    // A bound below one cell must not produce a zero-area or inverted envelope through a
    // `w - 1` term — and the envelope it does produce reaches no cell but the origin one,
    // because the integer block `[0, ⌊0.25⌋)` is empty. The two shapes answer that differently
    // and both answers are asserted: square covers NO cell at all (its origin cell's own span
    // does not fit), while hex's clamped envelope is exactly the origin hex's own bounding
    // box. "At most the origin cell" then follows from that equality rather than needing its
    // own assertion — a hex's bounding box is one hex across, and every neighbour's centre
    // sits a full pitch away.
    //
    // The hex ROUNDING-refutation runs at `w = 1.25`, not `0.25`: hex clamps `qmax` at zero, so
    // rounding `0.25` up to a whole cell leaves `qmax = 0` and the envelope unmoved, and the
    // assertion would hold either way. `1.25` takes `qmax` from `0.25` to `1` under rounding,
    // which pushes the far corner past one hex pitch. The `(0.25, 0.25)` hex arm is separate
    // and covers what `1.25` cannot: sub-one-cell on BOTH axes, the regime
    // `extent_rule_says_inside` excludes for hex.
    // Discrimination, per assertion, each naming an input or mutation that breaks THAT
    // assertion while the rest of this test still holds:
    // - the positivity loop fails if either impl subtracts one cell without clamping.
    // - the square cover-refutation fails if the square y axis is grown past a cell.
    // - the square exclusion fails if `SquareGrid::world_extent` ever grows its rectangle to
    //   half a cell or more at this bound, which the cover-refutation alone still admits.
    // - the hex far-corner assertion fails if hex starts rounding a fractional bound up to a
    //   whole cell, which pushes that corner past one pitch.
    // - the origin-hex bounding-box equality fails if either half-extent changes on either
    //   side, which no square assertion here and no far-corner assertion notices.
    // - the INVARIANCE assertion is the only one here stated over TWO different sub-one-cell
    //   bounds, so it is the only one that can see the clamp stop being a clamp; no positivity
    //   or cover claim can make that statement. It does not stand alone under every such
    //   mutation — one that varies the envelope at `(0.25, 0.25)` moves the bounding-box
    //   equality too — and what it adds is the second bound.
    // - the rule/measurement DISAGREEMENT fails if `extent_rule_says_inside`'s hex column arm
    //   drops its `− 1` term, which makes the rule answer "covered" and agree.
    // The invariance assertion and the rule/measurement disagreement each run after every
    // other assertion in this test, so a witness's run exercises those first rather than
    // leaving that to be argued.
    let sq = SquareGrid {
        cell: 20.0,
        rule: DiagonalRule::Chebyshev,
    };
    let hx = HexGrid { size: 20.0 };
    for e in [sq.world_extent((0.25, 0.25)), hx.world_extent((0.25, 0.25))] {
        assert!(
            e.width() > 0.0 && e.height() > 0.0,
            "extent must stay positive, got {e:?}"
        );
    }
    // Square cell (0,0) spans one cell per axis, so a quarter-cell rectangle cannot contain it.
    let sq_e = sq.world_extent((0.25, 0.25));
    assert!(
        sq_e.width() < sq.cell && sq_e.height() < sq.cell,
        "the square envelope {sq_e:?} must not cover cell (0,0)'s own {} span",
        sq.cell
    );
    // The stronger square claim, paired with the one it strengthens: the rectangle does not
    // even reach cell (0,0)'s CENTRE, which the cover-refutation alone still admits. Square
    // has no clamped term and therefore no excluded regime, so its rule describes this bound
    // exactly as it describes any other.
    assert!(
        sq.cell_center((0, 0)).0 > sq_e.max.0,
        "square excludes cell (0,0)'s centre at the same sub-one-cell bound"
    );
    // The far corner stays under one hex pitch (`√3·size`) — the clamp holds `qmax` at 0.25
    // rather than letting a rounded 2 push it past a pitch. Measured on the corner, not the
    // span, because the span carries the origin-side half-extent that the clamp does not
    // govern.
    let hx_far = hx.world_extent((1.25, 0.25)).max;
    assert!(
        hx_far.0 < hx.size * 3.0_f64.sqrt(),
        "the hex envelope's far corner x {} must be under one hex pitch",
        hx_far.0
    );
    // Below one cell on BOTH axes, which is the regime `extent_rule_says_inside` excludes for
    // hex and the sweep therefore never reaches: both clamps engage, so the envelope collapses
    // onto the origin hex's own bounding box — half the flats across in x and the circumradius
    // in y, on each side of the origin. Derived from `cell_vertices`, so the expectation comes
    // from the shape's geometry rather than restating the impl's constants.
    let sub = hx.world_extent((0.25, 0.25));
    let verts = hx.cell_vertices((0, 0), hx.size);
    let vmin = verts
        .iter()
        .fold((f64::MAX, f64::MAX), |a, v| (a.0.min(v.0), a.1.min(v.1)));
    let vmax = verts
        .iter()
        .fold((f64::MIN, f64::MIN), |a, v| (a.0.max(v.0), a.1.max(v.1)));
    assert!(
        (sub.min.0 - vmin.0).abs() < 1e-9
            && (sub.min.1 - vmin.1).abs() < 1e-9
            && (sub.max.0 - vmax.0).abs() < 1e-9
            && (sub.max.1 - vmax.1).abs() < 1e-9,
        "the clamped hex envelope {sub:?} must be the origin hex's own bounding box \
         ({vmin:?}, {vmax:?})"
    );
    // What the clamp DOES, stated as its observable consequence rather than as positivity:
    // `qmax`/`rmax` clamp at zero, so below one cell the envelope stops varying with the
    // bound at all. Two different sub-one-cell bounds must therefore yield ONE envelope — a
    // claim no positivity or cover assertion can make, and the precise reason the linear rule
    // (which varies continuously with the bound) cannot describe this regime.
    assert_eq!(
        sub,
        hx.world_extent((0.75, 0.9)),
        "two sub-one-cell bounds must yield one hex envelope"
    );
    // The disagreement itself, pinned rather than only fenced off: the linear rule answers
    // EXCLUDED for the origin hex while the envelope covers its centre, the origin hex being
    // centred ON the origin corner. Reached through `extent_rule_says_inside`'s
    // `BelowTheHexClamp` arm, which is the same body the sweep evaluates — one rule, with the
    // regime named at the call instead of a second entry point that could be reached without
    // naming one. Probed at `(0.25, 1.0)`, where the ROW axis is inside the rule's stated
    // domain and only the COLUMN axis is out, so the exclusion is attributable to one arm; at
    // `(0.25, 0.25)` both arms exclude and a mutation to either would leave the conjunction
    // unchanged.
    let clamped = hx.world_extent((0.25, 1.0));
    let origin = hx.cell_center((0, 0));
    assert!(
        !extent_rule_says_inside(
            GridKind::Hex,
            (0.25, 1.0),
            (0, 0),
            ExtentRuleDomain::BelowTheHexClamp
        ) && origin.0 >= clamped.min.0
            && origin.1 >= clamped.min.1
            && origin.0 <= clamped.max.0
            && origin.1 <= clamped.max.1,
        "the linear rule must exclude the origin hex {origin:?} where the envelope \
         {clamped:?} covers it"
    );
}

#[test]
fn a_fractional_bound_below_half_a_cell_leaves_its_partial_column_outside() {
    // A readable named instance of the membership rule the sweep enforces in bulk, with
    // `w = 3.2` (`f_w = 0.2`) and an INTEGRAL height, so only the column axis is fractional.
    //
    // Square is row-independent (`i ≤ w − 0.5`), so column 3 is outside at every row.
    //
    // Hex's column axis is row-DEPENDENT (`i ≤ w − 1 + (h − j)/2`): the shear's `(h − j)/2`
    // reach SHRINKS as the row index rises, so column 3 is outside at the LAST row of the
    // integer block and inside at row 0. Row 0 is not unconditionally inside — it holds at
    // `h = 2 ≥ 2·(1 − f_w) = 1.6` and fails at `h = 1`, and BOTH heights are probed at row 0,
    // because that pair is what makes the reach depend on the block's height at all.
    //
    // The block is TWO rows tall, and that is load-bearing rather than incidental: at four rows
    // the integer-block loop's own strongest bound (column ⌊w⌋−1 at the last row) already
    // exceeds what the row-0 assertion needs, so the row-0 assertion could not fail alone. At
    // two rows the loop's strongest bound falls short of column ⌊w⌋'s row-0 centre and the two
    // separate.
    //
    // Discrimination, per assertion, each naming a mutation that breaks THAT assertion while
    // every other assertion here still holds:
    // - the tall block's row-0 assertion fails if `HexGrid::world_extent`'s `√3/2·rmax` term is
    //   DROPPED: the rectangle then stops widening with the row count and no longer reaches
    //   column ⌊w⌋ at row 0, while it still covers the whole integer block.
    // - its short-block partner fails if that same `rmax` gains a positive floor, which lets a
    //   one-row block reach as far as a two-row one and kills the pair's discrimination.
    // - the integer-block loop fails if `HexGrid::axial_to_pixel`'s row shear GROWS, which
    //   pushes an interior column's last-row centre out of a rectangle that still reaches
    //   column ⌊w⌋ at row 0.
    // - the last-row assertion fails if `HexGrid::axial_to_pixel`'s own shear is dropped, since
    //   the centre then stops moving right as the row index rises and column ⌊w⌋ falls back
    //   inside. Negating `world_extent`'s shear cannot fail it — a smaller rectangle only makes
    //   `centre > extent` hold harder — which is why the two halves name different symbols.
    // - both shapes' partial-column assertions fail if either impl rounds a fractional bound
    //   up, which pulls column ⌊w⌋ inside on every arm.
    // - the square integer-block loop fails if `SquareGrid::world_extent` subtracts a cell.
    // Every expectation is computed from `cell_center` and the extent, never restated.
    let w = 3.2_f64;
    let h = 2.0_f64;
    let short_h = 1.0_f64;
    let f = w - w.floor();
    assert!(
        f != 0.0 && f < 0.5,
        "fixture: {w} must leave its partial column outside at the last row, f = {f}"
    );
    assert!(
        h >= 2.0 * (1.0 - f) && short_h < 2.0 * (1.0 - f),
        "fixture: the row-0 pair needs the heights to straddle 2*(1 - f_w) = {}, got {h} and {short_h}",
        2.0 * (1.0 - f)
    );
    assert_eq!(h, h.floor(), "fixture: the height must be integral");
    let partial = w.floor() as i32;
    let last_row = h as i32 - 1;

    let sq = SquareGrid {
        cell: 20.0,
        rule: DiagonalRule::Chebyshev,
    };
    let sx = sq.world_extent((w, h)).max.0;
    for i in 0..partial {
        assert!(
            sq.cell_center((i, last_row)).0 <= sx + 1e-9,
            "square column {i} of the integer block must be covered"
        );
    }
    assert!(
        sq.cell_center((partial, last_row)).0 > sx,
        "square's partial column {partial} sits outside the extent {sx}"
    );

    let hx = HexGrid { size: 50.0 };
    let hxx = hx.world_extent((w, h)).max.0;
    for q in 0..partial {
        assert!(
            hx.cell_center((q, last_row)).0 <= hxx + 1e-9,
            "hex column {q} of the integer block must be covered at the last row"
        );
    }
    // The row-0 pair: the SAME column at the SAME row flips on the block's height alone,
    // which no row-independent extent can produce. Both halves clear the integer-block loop's
    // own strongest bound, so each fails on its own rather than being masked by it.
    let short_x = hx.world_extent((w, short_h)).max.0;
    assert!(
        hx.cell_center((partial, 0)).0 <= hxx,
        "the tall block's shear reaches hex column {partial} at row 0, extent {hxx}"
    );
    assert!(
        hx.cell_center((partial, 0)).0 > short_x,
        "a {short_h}-row block of the same width does NOT reach it, extent {short_x}"
    );
    assert!(
        hx.cell_center((partial, last_row)).0 > hxx,
        "hex's partial column {partial} sits outside the extent {hxx} at row {last_row}"
    );
}

/// The membership rule `GridShape::world_extent` documents, as an executable predicate:
/// does the rule say cell `c`'s CENTRE lies within the `max` side of the envelope
/// `world_extent(bounds)` returns? Stated in index units, so it is a rule about which cells a
/// block covers rather than a second copy of either impl's coordinate arithmetic. The `min`
/// side excludes no cell of the swept block on either shape, so it carries no term here and is
/// measured directly by the sweep instead.
///
/// - **Square**, per axis and independent of the other: `i ≤ w − 0.5`, `j ≤ h − 0.5`. The
///   partial cell `i = ⌊w⌋` is covered exactly when its fractional part is `≥ 0.5`.
/// - **Hex x**, row-DEPENDENT: `q ≤ w − 1 + (h − r)/2`. The axial shear grants `(h − r)/2`
///   columns of extra reach at row `r`, so the reach SHRINKS as the row index rises. The
///   partial column `q = ⌊w⌋` is therefore covered at row `r` exactly when
///   `r ≤ h − 2·(1 − f_w)`, which at `r = 0` is the condition `h ≥ 2·(1 − f_w)`.
/// - **Hex y**, independent of the column: `r ≤ h − 1/3`. The partial row `r = ⌊h⌋` is
///   covered exactly when `f_h ≥ 1/3` — a different threshold from every other axis here.
///
/// **The domain is SHAPE-DEPENDENT, not shared.** Square holds for every `w > 0`, `h > 0`:
/// `SquareGrid::world_extent` is `w·cell` with no subtraction and nothing clamped, so a
/// sub-one-cell bound is the same rule, not a second regime. Hex is stated for `w ≥ 1` and
/// `h ≥ 1` only, because `HexGrid::world_extent` clamps its `w − 1`/`h − 1` terms at zero and
/// the rectangle then stops varying linearly with the bound — at `w = 0.25, h = 1.0` this rule
/// answers false for cell `(0,0)` while the measurement covers it. That hex regime is pinned
/// by `world_extent_stays_positive_below_one_cell_and_covers_at_most_the_origin_cell`.
///
/// The regime is an ARGUMENT rather than a second entry point, so every caller states which
/// side of the domain it is asking about and each side is guarded on its own terms: a bound
/// inside the stated domain cannot reach the out-of-domain arm, and an out-of-domain call
/// cannot be made without saying so at the call. The rule's arithmetic has a single body that
/// both guarded arms fall through to.
fn extent_rule_says_inside(
    kind: GridKind,
    bounds: (f64, f64),
    c: Cell,
    domain: ExtentRuleDomain,
) -> bool {
    let (w, h) = bounds;
    match domain {
        ExtentRuleDomain::Stated => {
            let floor = match kind {
                GridKind::Square => 0.0,
                GridKind::Hex => 1.0,
            };
            assert!(
                w > 0.0 && h > 0.0 && w >= floor && h >= floor,
                "fixture: {kind:?}'s rule is stated for w, h >= {floor} (and > 0), \
                 got {bounds:?}"
            );
        }
        ExtentRuleDomain::BelowTheHexClamp => assert!(
            kind == GridKind::Hex && (w < 1.0 || h < 1.0) && w > 0.0 && h > 0.0,
            "fixture: the out-of-domain arm is hex's clamped regime only, got {kind:?} \
             {bounds:?}"
        ),
    }
    // A tolerance in index units, so a bound sitting exactly on a threshold (`f = 0.5` on
    // square, `f = 1/3` on the hex row axis) resolves inclusively on this side exactly as the
    // caller's own `<= extent + tol` resolves it on the measured side.
    const TOL: f64 = 1e-9;
    let (i, j) = (f64::from(c.0), f64::from(c.1));
    match kind {
        GridKind::Square => i <= w - 0.5 + TOL && j <= h - 0.5 + TOL,
        GridKind::Hex => i <= w - 1.0 + (h - j) / 2.0 + TOL && j <= h - 1.0 / 3.0 + TOL,
    }
}

/// Which side of `extent_rule_says_inside`'s stated domain a caller is asking about. The rule
/// describes one regime and deliberately mispredicts the other, and an out-of-domain answer is
/// plausible rather than obviously wrong, so the choice is made at the call rather than
/// inherited from whichever entry point happened to be reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtentRuleDomain {
    /// The bounds the rule is stated for — every positive bound on square, `w, h >= 1` on hex.
    Stated,
    /// Hex below one cell on either axis, where `HexGrid::world_extent`'s clamps make the
    /// rectangle stop varying with the bound and the rule's prediction is known to be wrong.
    /// The only legitimate use is pinning that disagreement against the measurement.
    BelowTheHexClamp,
}

/// The number of `(scale, kind, bounds)` triples
/// `the_extent_membership_rule_predicts_every_cells_coverage_over_a_bounds_sweep` visits, and
/// therefore the number of envelope minimums it measures. Two scales; square sweeps three
/// integer parts per axis against hex's two, and square skips the bounds whose width or height
/// lands on the degenerate `0 + 0` pair (21 on each axis, one of them shared).
const MIN_SIDE_SWEEP_BOUNDS: usize = 2 * (3 * 7 * 3 * 7 - (21 + 21 - 1)) + 2 * (2 * 7 * 2 * 7);

#[test]
fn the_extent_membership_rule_predicts_every_cells_coverage_over_a_bounds_sweep() {
    // `world_extent`'s membership rule, executed rather than asserted in prose: for every cell
    // of the integer block PLUS the partial column and the partial row, the rule's prediction
    // must equal the actual comparison of `cell_center` against `world_extent`. The rule is
    // pinned by a run rather than by hand algebra, which cannot review its own derivation.
    //
    // The sweep varies the fractional part of EACH axis independently over
    // `0, 0.2, 1/3, 0.4, 0.5, 0.6, 0.8` — 1/3 is the hex row-axis threshold and 0.5 the square
    // one, so a shared-threshold reading of the two axes fails — and includes one-row-tall
    // blocks (`⌊h⌋ = 1`) alongside three-row ones, which exercises the hex partial column's
    // row-0 condition `h ≥ 2·(1 − f_w)` on both sides. The square arm additionally sweeps
    // sub-one-cell bounds (`⌊w⌋ = 0`), the regime hex has to fence off and square does not.
    //
    // The `min` side is swept too, per BOUND rather than per cell: the rule constrains only the
    // `max` side, so a broken minimum would slip through a rule-versus-measurement comparison
    // entirely. Each bound's envelope minimum is compared against the origin cell's own
    // lower-left extreme, derived from `cell_vertices` — the shape's own geometry, never a
    // restatement of the impl's constants — and the count of those comparisons is asserted
    // alongside the cell count so the min side cannot silently stop being visited.
    //
    // Discrimination: every expectation comes from `extent_rule_says_inside` and every measured
    // value from `cell_center`/`world_extent`, so this fails if the rule is misstated on either
    // axis of either shape and equally if either impl's closed form moves. The two wrong
    // readings worth naming do NOT fail at the same size, so their counts are stated
    // separately rather than as one magnitude: seeding the hex row axis with the `1/2`
    // threshold the other axes use disagrees with 232 of the 8136 cells swept, while seeding
    // the hex partial column as covered at row 0 unconditionally disagrees with 60. Both are
    // measured against this sweep as written, so the swept cell COUNT is asserted: dropping a
    // fractional part or an integer part would leave those two numbers describing a sweep that
    // no longer exists, with nothing failing. The min-side comparison fails on its own witness,
    // `HexGrid::world_extent`'s `min` moved to the origin, which no cell-coverage comparison
    // here notices.
    const TOL_CELLS: f64 = 1e-9;
    let fracs = [0.0, 0.2, 1.0 / 3.0, 0.4, 0.5, 0.6, 0.8];
    let mut checked = 0usize;
    let mut covered = 0usize;
    let mut min_side_checked = 0usize;
    let mut mismatches: Vec<String> = Vec::new();
    for scale in [20.0_f64, 50.0] {
        for kind in [GridKind::Square, GridKind::Hex] {
            let shape: Box<dyn GridShape> = match kind {
                GridKind::Square => Box::new(SquareGrid {
                    cell: scale,
                    rule: DiagonalRule::Chebyshev,
                }),
                GridKind::Hex => Box::new(HexGrid { size: scale }),
            };
            // The integer part `0` is swept on SQUARE only, because the domain is
            // shape-dependent: square's rule holds for every positive bound, while hex's
            // clamped `w − 1`/`h − 1` terms make it inapplicable below one cell. A `0 + 0`
            // pair is degenerate, refused by `world_extent` itself and outside both rules.
            let integer_parts: &[f64] = match kind {
                GridKind::Square => &[0.0, 1.0, 3.0],
                GridKind::Hex => &[1.0, 3.0],
            };
            for &wi in integer_parts {
                for fw in fracs {
                    for &hi in integer_parts {
                        for fh in fracs {
                            let bounds = (wi + fw, hi + fh);
                            if bounds.0 <= 0.0 || bounds.1 <= 0.0 {
                                continue;
                            }
                            let e = shape.world_extent(bounds);
                            let (ex, ey) = e.max;
                            let tol = TOL_CELLS * scale;
                            // The min side: the envelope's lower-left corner IS the origin
                            // cell's own, whatever the bound. Taken from the origin cell's
                            // vertex ring so the expectation is the shape's geometry rather
                            // than a second copy of the impl's half-extents.
                            let origin_verts = shape.cell_vertices((0, 0), scale);
                            let vmin = origin_verts
                                .iter()
                                .fold((f64::MAX, f64::MAX), |a, v| (a.0.min(v.0), a.1.min(v.1)));
                            min_side_checked += 1;
                            if (e.min.0 - vmin.0).abs() > tol || (e.min.1 - vmin.1).abs() > tol {
                                mismatches.push(format!(
                                    "{kind:?} scale {scale} bounds {bounds:?}: envelope \
                                     minimum {:?} is not the origin cell's own lower-left \
                                     extreme {vmin:?}",
                                    e.min
                                ));
                            }
                            for i in 0..=bounds.0.floor() as i32 {
                                for j in 0..=bounds.1.floor() as i32 {
                                    let c = shape.cell_center((i, j));
                                    let actual = c.0 <= ex + tol && c.1 <= ey + tol;
                                    let predicted = extent_rule_says_inside(
                                        kind,
                                        bounds,
                                        (i, j),
                                        ExtentRuleDomain::Stated,
                                    );
                                    checked += 1;
                                    covered += usize::from(actual);
                                    if predicted != actual {
                                        mismatches.push(format!(
                                            "{kind:?} scale {scale} bounds {bounds:?} cell \
                                             ({i},{j}): rule says inside={predicted}, centre \
                                             {c:?} against extent ({ex}, {ey}) says {actual}"
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // Fixture guard: the sweep is only meaningful while it spans both answers. An all-covered
    // or all-excluded cell range would agree with a rule that returned a constant.
    assert!(
        covered > 0 && covered < checked,
        "fixture: the swept cells must span both answers, {covered} of {checked} covered"
    );
    // The sweep SIZE, bound so the two stated mismatch counts stay falsifiable.
    assert_eq!(
        checked, 8136,
        "fixture: the swept cell count the stated mismatch counts are measured against"
    );
    // The min side is visited once per bound, and its count pins that PER-BOUND rate rather
    // than the sweep size the cell count already pins: moving this probe inside the per-cell
    // loop leaves `checked` at 8136 while this count explodes, and a sweep that stopped
    // reaching the corner at all would otherwise leave it unmeasured silently.
    assert_eq!(
        min_side_checked, MIN_SIDE_SWEEP_BOUNDS,
        "fixture: every swept bound must have its envelope minimum measured"
    );
    assert!(
        mismatches.is_empty(),
        "{} of the {checked} cell comparisons and {min_side_checked} minimum comparisons \
         disagree with the documented behaviour; first 8:\n{}",
        mismatches.len(),
        mismatches
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn both_shapes_answer_a_degenerate_bound_with_the_same_refused_envelope() {
    // One trait method, one answer to degenerate input, over the WHOLE degenerate class:
    // non-finite, zero, and negative alike. Without the shared normalisation the square arm
    // collapses to a zero-area envelope (which every extent guard refuses) while the hex arm
    // still adds its pointy-top half-extents and returns a positive-AREA envelope that BUILDS
    // — a tiny mesh that
    // makes everything outside it unreachable. Under-permissive rather than a leak, but the
    // two impls disagreeing is the defect, and it is the same defect for `0.0` as for `NaN`.
    //
    // The refusal is on AREA, not on a far corner: the envelope carries a minimum, so a
    // shape that answered `min = (-a, -b), max = (0, 0)` would present a zero far corner and a
    // POSITIVE span, which every guard would then accept.
    // Discrimination: fails if either impl stops normalising, and fails if the normaliser
    // narrows back to the non-finite half of the class, since the assertion compares the two
    // arms against each other AND against the refused span on every input.
    let sq = SquareGrid {
        cell: 50.0,
        rule: DiagonalRule::Chebyshev,
    };
    let hx = HexGrid { size: 50.0 };
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0] {
        for pair in [(bad, 5.0), (5.0, bad), (bad, bad)] {
            let s = sq.world_extent(pair);
            let h = hx.world_extent(pair);
            assert_eq!(s, h, "the two shapes disagree on {pair:?}: {s:?} vs {h:?}");
            assert!(
                s.width() <= 0.0 || s.height() <= 0.0,
                "a degenerate axis must yield an envelope the extent guards refuse, got {s:?}"
            );
        }
    }
}
