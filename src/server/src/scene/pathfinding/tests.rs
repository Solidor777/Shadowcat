use super::*;
use crate::scene::pathfinding::MoveTraits;
use crate::scene::vision::Seg;
use std::collections::BTreeSet;

fn grid<'a>(walls: &'a [Seg], mask: Option<&'a BTreeSet<Cell>>, footprint: f64) -> PathGrid<'a> {
    let shape: &'static crate::scene::grid_shape::SquareGrid =
        Box::leak(Box::new(crate::scene::grid_shape::SquareGrid {
            cell: 100.0,
            rule: DiagonalRule::Chebyshev,
        }));
    PathGrid {
        inputs: PathInputs {
            cell: shape.cell,
            footprint_radius_cells: footprint,
            walls,
            mask,
            regions: None,
            shape,
            budget_cells: None,
            traits: MoveTraits::default(),
        },
        window: (-100, -100, 100, 100),
    }
}

#[test]
fn open_neighbor_is_enterable_no_walls_no_mask() {
    let walls: Vec<Seg> = vec![];
    let g = grid(&walls, None, 0.2);
    assert!(cell_enterable(&g, (0, 0), (1, 0)));
    assert!(cell_enterable(&g, (0, 0), (1, 1)));
}

#[test]
fn step_crossing_a_blocks_move_wall_is_not_enterable() {
    // A vertical wall on the x=100 grid line blocks the (0,0)->(1,0) center step (50,50)->(150,50).
    let walls = vec![Seg {
        a: (100.0, 0.0),
        b: (100.0, 200.0),
    }];
    let g = grid(&walls, None, 0.2);
    assert!(
        !cell_enterable(&g, (0, 0), (1, 0)),
        "center step crosses the wall"
    );
}

#[test]
fn footprint_disc_too_wide_for_a_gap_is_not_enterable() {
    // Two walls one cell apart (x=100 and x=200). A footprint radius 0.7 cell (=70 units) at the
    // center of cell (1,0) (center x=150) is within 50 units of BOTH walls → blocked (the body
    // can't fit the 1-cell gap). A small radius (0.2 cell = 20 units) clears it.
    let walls = vec![
        Seg {
            a: (100.0, 0.0),
            b: (100.0, 200.0),
        },
        Seg {
            a: (200.0, 0.0),
            b: (200.0, 200.0),
        },
    ];
    let wide = grid(&walls, None, 0.7);
    let narrow = grid(&walls, None, 0.2);
    // Use a step that does not itself cross a wall: (1,1)->(1,0) (vertical, x=150 throughout).
    assert!(
        !cell_enterable(&wide, (1, 1), (1, 0)),
        "wide footprint cannot fit the gap"
    );
    assert!(
        cell_enterable(&narrow, (1, 1), (1, 0)),
        "narrow footprint fits"
    );
}

#[test]
fn footprint_cell_outside_mask_is_not_enterable() {
    // Non-GM mask containing only cell (1,0). A footprint disc that overlaps neighbors requires
    // all overlapped cells in the mask. Radius 0.6 cell at center of (1,0) overlaps (0,0)/(2,0)/
    // (1,-1)/(1,1) edges → those must be in mask. With only (1,0) present → not enterable.
    let walls: Vec<Seg> = vec![];
    let mut mask = BTreeSet::new();
    mask.insert((1, 0));
    let g = grid(&walls, Some(&mask), 0.6);
    assert!(
        !cell_enterable(&g, (1, 1), (1, 0)),
        "overlapped neighbor cells not in mask"
    );

    // A point-sized footprint overlaps only (1,0) at the destination — but the mask check also
    // requires the FROM cell in the mask (supercover_cells always includes both endpoint
    // cells). Add (1,1) to represent a realistic case where both the mover's current cell
    // and its destination are visible.
    let mut mask_from_and_to = mask.clone();
    mask_from_and_to.insert((1, 1));
    let gp = grid(&walls, Some(&mask_from_and_to), 0.0);
    assert!(cell_enterable(&gp, (1, 1), (1, 0)));
}

#[test]
fn diagonal_step_missing_flanker_cell_is_not_enterable_small_footprint() {
    // Regression test. A perfectly diagonal step (0,0)->(1,1) at cell=100 crosses
    // the shared corner exactly, so supercover_cells emits BOTH flanker cells (1,0) and (0,1)
    // in addition to the two endpoint cells. A small (point-sized) footprint disc at the
    // destination (1,1) only overlaps (1,1) itself — footprint_cells alone would not catch a
    // missing flanker. The mask here has every cell EXCEPT the (0,1) flanker: the step must
    // be rejected once the router's mask check includes the step's supercover, even though
    // a footprint-disc-only check alone would have passed it.
    let walls: Vec<Seg> = vec![];
    let mut mask = BTreeSet::new();
    mask.insert((0, 0));
    mask.insert((1, 0));
    mask.insert((1, 1));
    // (0, 1) deliberately absent — the missing flanker.
    let g = grid(&walls, Some(&mask), 0.1);
    assert!(
        !cell_enterable(&g, (0, 0), (1, 1)),
        "diagonal step must be rejected when a supercover flanker cell is outside the mask, \
         even though the footprint disc at the destination doesn't reach that flanker"
    );
}

#[test]
fn degenerate_step_supercover_is_not_enterable_even_if_mask_covers_destination() {
    // A step spanning an enormous cell distance makes `supercover_cells` return `None` (the
    // MAX_MOVE_CELLS span guard). The router must fail closed on `None`, same
    // as `move_exec::execute_move` and `Room::publish`, regardless of what the mask contains at `to`.
    let walls: Vec<Seg> = vec![];
    let mut mask = BTreeSet::new();
    mask.insert((5000, 5000)); // covers the destination; would pass footprint_cells alone.
    let mut g = grid(&walls, Some(&mask), 0.0);
    g.window = (-10_000, -10_000, 10_000, 10_000);
    assert!(
        !cell_enterable(&g, (0, 0), (5000, 5000)),
        "an over-cap/degenerate step supercover must fail closed, not fall back to the \
         footprint-only mask check"
    );
}

fn visible_grid(range: i32) -> BTreeSet<Cell> {
    (0..range)
        .flat_map(|i| (0..range).map(move |j| (i, j)))
        .collect()
}

#[test]
fn large_footprint_diagonal_step_with_flankers_in_mask_is_enterable() {
    // A large footprint (1.0 cell radius) at the destination of a diagonal step already
    // overlaps both corner-flanker cells — the footprint_cells check alone would pass this.
    // The step-supercover union must not reject a step whose mask already covers everything
    // the footprint disc covers: the union widens what is checked, never what is refused
    // beyond it.
    let walls: Vec<Seg> = vec![];
    let mask = visible_grid(6); // covers (0,0)..(5,5); large enough for both footprint + step.
    let g = grid(&walls, Some(&mask), 1.0);
    assert!(
        cell_enterable(&g, (2, 2), (3, 3)),
        "a large footprint with a fully-visible mask is enterable"
    );
}

#[test]
fn cell_outside_window_is_not_enterable() {
    let walls: Vec<Seg> = vec![];
    let mut g = grid(&walls, None, 0.2);
    g.window = (0, 0, 2, 2);
    assert!(
        !cell_enterable(&g, (2, 2), (3, 2)),
        "outside the search window"
    );
}

#[test]
fn impassable_region_blocks_entry_like_a_wall() {
    use crate::scene::regions::{RegionBehavior, RegionField, RegionShape};
    let mut b = RegionField::builder();
    b.add(
        &RegionShape::Rect {
            x0: 100.0,
            y0: 0.0,
            x1: 200.0,
            y1: 100.0,
        },
        RegionBehavior::Impassable,
        1.0,
        100.0,
        &crate::scene::grid_shape::SquareGrid {
            cell: 100.0,
            rule: DiagonalRule::Chebyshev,
        },
    );
    let field = b.build();
    let walls: Vec<Seg> = vec![];
    let mut g = grid(&walls, None, 0.2);
    g.inputs.regions = Some(&field);
    assert!(
        !cell_enterable(&g, (0, 0), (1, 0)),
        "cell (1,0) center (150,50) is inside the impassable rect"
    );
}

// --- footprint_cells boundary-tie characterization ---
// Pins the geometric analysis behind the r=0 tie-break: a positive-radius disc centered
// exactly on a shared cell boundary has genuine positive-area overlap with every cell
// touching that point and must keep admitting all of them (NOT a bug); only the
// zero-radius point case is a true degenerate tie.

#[test]
fn positive_radius_disc_on_a_two_cell_edge_tie_admits_both_with_positive_area_overlap() {
    // cell=100, ctr=(200,150) sits exactly on the shared vertical edge between (1,1) and
    // (2,1). r=40 > 0: both cells have genuine positive-area overlap with the disc.
    let cells = footprint_cells((1, 1), (200.0, 150.0), 40.0, 100.0);
    assert!(
        cells.contains(&(1, 1)),
        "must admit the cell on the near side of the tie"
    );
    assert!(
        cells.contains(&(2, 1)),
        "must admit the cell on the far side of the tie"
    );
    // Positive-area spot check: (180,150) is inside both the disc (dist 20 < 40) and cell
    // (1,1) ([100,200)x[100,200)); (220,150) is inside both the disc and cell (2,1).
    let d1 = ((180.0_f64 - 200.0).powi(2) + (150.0_f64 - 150.0).powi(2)).sqrt();
    assert!(d1 < 40.0, "(180,150) genuinely inside the disc");
    let d2 = ((220.0_f64 - 200.0).powi(2) + (150.0_f64 - 150.0).powi(2)).sqrt();
    assert!(d2 < 40.0, "(220,150) genuinely inside the disc");
}

#[test]
fn positive_radius_disc_on_a_four_cell_corner_tie_admits_all_four_with_positive_area_overlap() {
    // cell=100, ctr=(200,200) sits exactly on the 4-way corner shared by (1,1),(2,1),(1,2),(2,2).
    // r=40 > 0.
    let cells = footprint_cells((1, 1), (200.0, 200.0), 40.0, 100.0);
    for c in [(1, 1), (2, 1), (1, 2), (2, 2)] {
        assert!(cells.contains(&c), "must admit corner-tied cell {c:?}");
    }
    // Positive-area spot check for each quadrant.
    for (px, py) in [
        (180.0, 180.0),
        (220.0, 180.0),
        (180.0, 220.0),
        (220.0, 220.0),
    ] {
        let d = ((px - 200.0_f64).powi(2) + (py - 200.0_f64).powi(2)).sqrt();
        assert!(d < 40.0, "({px},{py}) genuinely inside the disc");
    }
}

#[test]
fn zero_radius_point_on_a_two_cell_edge_tie_resolves_to_the_single_canonical_cell() {
    // r=0.0 exactly: a literal point on the shared edge between (1,1) and (2,1). Must
    // resolve to exactly the single cell `to_cell`/`GridShape::cell_of` (floor-based) would
    // assign: floor(200/100)=2, so (2,1) is canonical (the caller always passes anchor =
    // to_cell(ctr)).
    let cells = footprint_cells((2, 1), (200.0, 150.0), 0.0, 100.0);
    assert_eq!(
        cells,
        vec![(2, 1)],
        "a zero-radius boundary point must resolve to exactly one cell, matching cell_of"
    );
}

#[test]
fn zero_radius_point_on_a_four_cell_corner_tie_resolves_to_the_single_canonical_cell() {
    // r=0.0 exactly: a literal point on the 4-way corner shared by (1,1),(2,1),(1,2),(2,2).
    // floor(200/100)=2 on both axes, so (2,2) is canonical.
    let cells = footprint_cells((2, 2), (200.0, 200.0), 0.0, 100.0);
    assert_eq!(
        cells,
        vec![(2, 2)],
        "a zero-radius corner point must resolve to exactly one cell, matching cell_of"
    );
}

#[test]
fn terrain_region_raises_astar_step_cost() {
    use crate::scene::regions::{RegionBehavior, RegionField, RegionShape};
    let mut b = RegionField::builder();
    // Terrain cost 3 covering every cell reachable in a single king-move step from BOTH the
    // start and the goal — i.e. every possible intermediate cell of a 2-step (0,0)->(2,0)
    // route, not merely the direct orthogonal row. Deviation from a narrower single-row rect:
    // under Chebyshev a narrower rect leaves a same-length diagonal detour (via (1,1) or
    // (1,-1)) that pays the multiplier only once, at cost 4.0 not 6.0 — that isn't a routing
    // bug, it's terrain-weighting correctly picking the cheaper of two equal-length routes.
    // Covering all three king-move-adjacent intermediates (1,-1)/(1,0)/(1,1) removes that
    // escape so every legal 2-step route pays the multiplier twice, proving the weighting is
    // applied per entered cell regardless of which route is chosen.
    b.add(
        &RegionShape::Rect {
            x0: 0.0,
            y0: -100.0,
            x1: 300.0,
            y1: 200.0,
        },
        RegionBehavior::Terrain,
        3.0,
        100.0,
        &crate::scene::grid_shape::SquareGrid {
            cell: 100.0,
            rule: DiagonalRule::Chebyshev,
        },
    );
    let field = b.build();
    let walls: Vec<Seg> = vec![];
    let mut g = grid(&walls, None, 0.1);
    g.inputs.regions = Some(&field);
    let (_cells, cost, _p) = astar_leg(&g, (0, 0), (2, 0), 0).unwrap();
    assert!(
        (cost - 6.0).abs() < 1e-9,
        "2 steps through terrain at multiplier 3 = 6.0, regardless of which of the three \
         king-move-adjacent intermediate cells the route passes through"
    );
}
