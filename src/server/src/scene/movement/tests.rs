use super::*;

fn cells(a0: (f64, f64), a1: (f64, f64), cell: f64) -> std::collections::BTreeSet<(i32, i32)> {
    supercover_cells(a0, a1, cell).expect("within cap")
}

#[test]
fn single_cell_when_endpoints_share_a_cell() {
    // a0 == a1 (no-op) and a tiny intra-cell move both → exactly the one cell.
    let c = cells((50.0, 50.0), (50.0, 50.0), 100.0);
    assert_eq!(c.len(), 1);
    assert!(c.contains(&(0, 0)));
    let c2 = cells((10.0, 10.0), (90.0, 90.0), 100.0);
    assert_eq!(c2, c, "both endpoints inside cell (0,0)");
}

#[test]
fn horizontal_move_covers_each_crossed_cell() {
    // (50,50)->(250,50) at cell 100 crosses cells x=0,1,2 at row 0.
    let c = cells((50.0, 50.0), (250.0, 50.0), 100.0);
    assert!(c.contains(&(0, 0)) && c.contains(&(1, 0)) && c.contains(&(2, 0)));
    assert_eq!(c.len(), 3);
}

#[test]
fn pure_diagonal_through_corner_includes_both_flanking_cells() {
    // (50,50)->(150,150): the line passes exactly through the shared corner (100,100).
    // Supercover includes the two diagonal cells AND BOTH off-diagonal flankers — a thin
    // line would visit only (0,0),(1,1) and let a move slip past an unseen (1,0)/(0,1).
    // INVARIANT: both flanking cells must be present (not just one), and the total cell
    // count must be exactly 4 (the two diagonal + both flankers, no more, no less).
    let c = cells((50.0, 50.0), (150.0, 150.0), 100.0);
    assert!(c.contains(&(0, 0)) && c.contains(&(1, 1)));
    assert!(c.contains(&(1, 0)), "flanking cell (1,0) must be present");
    assert!(c.contains(&(0, 1)), "flanking cell (0,1) must be present");
    assert_eq!(
        c.len(),
        4,
        "1-cell diagonal supercover visits exactly 4 cells"
    );
}

#[test]
fn endpoints_always_present_for_a_sloped_move() {
    let c = cells((50.0, 50.0), (370.0, 130.0), 100.0);
    assert!(c.contains(&(0, 0)), "start cell present");
    assert!(c.contains(&(3, 1)), "end cell present");
}

#[test]
fn nonpositive_cell_is_none() {
    // `!(cell > 0.0)` guard: catches 0.0, negative, and NaN.
    assert!(supercover_cells((0.0, 0.0), (10.0, 10.0), 0.0).is_none());
    assert!(supercover_cells((0.0, 0.0), (10.0, 10.0), -1.0).is_none());
    assert!(supercover_cells((0.0, 0.0), (10.0, 10.0), f64::NAN).is_none());
}

#[test]
fn non_finite_endpoint_is_none() {
    // Any non-finite coordinate in either endpoint → None (fail-closed).
    assert!(supercover_cells((f64::INFINITY, 0.0), (10.0, 10.0), 100.0).is_none());
    assert!(supercover_cells((0.0, f64::NAN), (10.0, 10.0), 100.0).is_none());
    assert!(supercover_cells((0.0, 0.0), (f64::NEG_INFINITY, 10.0), 100.0).is_none());
    assert!(supercover_cells((0.0, 0.0), (10.0, f64::NAN), 100.0).is_none());
}

#[test]
fn oversized_move_exceeds_cap_returns_none() {
    // cell 1, a 10_000-long move → > MAX_MOVE_CELLS candidate span → None (caller rejects).
    assert!(supercover_cells((0.0, 0.0), (10_000.0, 10_000.0), 1.0).is_none());
}

#[test]
fn negative_direction_move_covers_same_cells_as_forward() {
    // Exercises step_i = -1 / step_j = -1 boundary path.
    // (250,50)->(50,50) reversed: must cover the same three cells as the forward direction.
    let c = cells((250.0, 50.0), (50.0, 50.0), 100.0);
    assert!(c.contains(&(0, 0)), "cell (0,0) present in reversed move");
    assert!(c.contains(&(1, 0)), "cell (1,0) present in reversed move");
    assert!(c.contains(&(2, 0)), "cell (2,0) present in reversed move");
    assert_eq!(c.len(), 3);
}

#[test]
fn diagonal_leg_with_both_endpoints_on_lattice_corners_succeeds() {
    // Regression test: a diagonal king-step whose BOTH endpoints sit exactly on 4-way
    // grid-line intersections (here, a0's y-coordinate starts exactly on a grid line, forcing
    // a preceding single-axis step that puts t_max_i and t_max_j into lockstep). The per-axis
    // remaining-step budget (`remaining_i`/`remaining_j`) must gate the diagonal corner-step so
    // a tMax tie that merely coincides with an axis already at its target does not re-step
    // that axis and drift the traversal past (ei,ej).
    let c = cells((200.0, 200.0), (300.0, 100.0), 100.0);
    assert!(c.contains(&(2, 2)), "start cell present");
    assert!(c.contains(&(3, 1)), "end cell present");
}

#[test]
fn perfect_diagonal_across_many_lattice_corners_converges() {
    // A longer 45-degree diagonal crossing several lattice-corner ties in a row (dx == dy
    // exactly at every cell boundary) — a second instance of the same defect class, not just
    // the single-leg repro. Must converge and include every diagonal + flanking cell without
    // drifting past the endpoint.
    let c = cells((0.0, 0.0), (300.0, 300.0), 100.0);
    assert!(c.contains(&(0, 0)), "start cell present");
    assert!(c.contains(&(3, 3)), "end cell present");
    for k in 0..3 {
        assert!(c.contains(&(k, k)), "diagonal cell ({k},{k}) present");
    }
}

#[test]
fn single_endpoint_on_lattice_corner_includes_flankers() {
    // Only the START sits exactly on a lattice intersection; the end does not. A tie can
    // occur mid-path all the same, and the per-axis remaining-step budget that stops the
    // traversal overshooting its endpoint must not cost the flankers at that true corner
    // crossing: both flanking cells must be present.
    let c = cells((200.0, 200.0), (330.0, 70.0), 100.0);
    assert!(c.contains(&(2, 2)), "start cell present");
    assert!(c.contains(&(3, 0)), "end cell present");
    // The true corner crossing on this leg is (2,1)->(3,0); both flanking cells must
    // be present, not just the start/end.
    assert!(c.contains(&(3, 1)), "flanker cell present");
    assert!(c.contains(&(2, 0)), "flanker cell present");
}

/// Cells the segment provably enters, by dense midpoint sampling. A SUBSET oracle: it can
/// miss a cell the segment only grazes, so it may only ever be compared as
/// `oracle ⊆ supercover`.
fn densely_sampled_cells(a: (f64, f64), b: (f64, f64), cell: f64, n: usize) -> BTreeSet<Cell> {
    let mut out = BTreeSet::new();
    for k in 0..=n {
        let t = k as f64 / n as f64;
        let x = a.0 + (b.0 - a.0) * t;
        let y = a.1 + (b.1 - a.1) * t;
        out.insert(((x / cell).floor() as i32, (y / cell).floor() as i32));
    }
    out
}

#[test]
fn supercover_never_omits_a_densely_sampled_cell() {
    // The safety direction: the emitted set may never LOSE a cell the segment demonstrably
    // enters. Endpoints stay within ±200 at cell 10, so the endpoint-cell bounding box is at
    // most 41×41 and the span guard cannot fire.
    // Discrimination: fails on any edit that drops a cell from the emitted set on any of the
    // 2000 sampled segments — including a narrowed corner tolerance, which is what makes this
    // the pin that a tolerance change is not free.
    let cell = 10.0;
    let mut seed = 0x5EED_1234_u64;
    let mut next = |lo: f64, hi: f64| -> f64 {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        lo + ((seed >> 11) as f64 / (1u64 << 53) as f64) * (hi - lo)
    };
    for _ in 0..2000 {
        let a = (next(-200.0, 200.0), next(-200.0, 200.0));
        let b = (next(-200.0, 200.0), next(-200.0, 200.0));
        let sc = supercover_cells(a, b, cell).expect("finite endpoints inside the span cap");
        for c in densely_sampled_cells(a, b, cell, 4000) {
            assert!(sc.contains(&c), "{a:?}->{b:?} omits {c:?}");
        }
    }
}

#[test]
fn a_corner_free_long_diagonal_emits_a_plain_staircase() {
    // A segment offset off the exact diagonal by a fraction of a cell crosses each vertical
    // grid line strictly before or after the matching horizontal one, so no step is a corner
    // crossing and the emitted set is a staircase: exactly one new cell per crossing, never a
    // flanking PAIR. `n = 900` keeps the endpoint-cell bounding box at 901×901 = 811_801,
    // inside `MAX_MOVE_CELLS`.
    //
    // Discrimination: the assertion is a COUNT, so it fails on any edit that emits a flanking
    // pair on a step whose two crossings are separated. It does NOT restate the traversal:
    // the expected count is `2n + 1` from the staircase geometry, and the accompanying
    // accompanying superset check re-derives the same set from the dense-sample oracle.
    //
    // SCOPE, stated so this test is not read as more than it is: the two crossings on each
    // step of this construction are separated by `0.15/n` in the parametric variable, which is
    // many orders of magnitude above the tolerance `tol` evaluates to at these magnitudes. It
    // is therefore a REGRESSION PIN on the staircase property, not a reproducer of a tie
    // firing on separated crossings. A construction that reproduces that would need `tol` to
    // reach the crossing separation, which it does not at any coordinate magnitude the span
    // guard admits.
    let cell = 1.0;
    let n = 900_i32;
    let a = (0.25, 0.4);
    let b = (a.0 + n as f64, a.1 + n as f64);
    let sc = supercover_cells(a, b, cell).expect("within the span cap");
    assert_eq!(
        sc.len(),
        (2 * n + 1) as usize,
        "a corner-free long diagonal must produce a plain staircase, got {} cells",
        sc.len()
    );
    for c in densely_sampled_cells(a, b, cell, 20_000) {
        assert!(sc.contains(&c), "staircase omits {c:?}");
    }
}

#[test]
fn long_nonsymmetric_diagonal_corner_both_flankers_present() {
    // Segment (0,0)→(14,4) at cell = 1: dx = 14, dy = 4, so the step from cell (13,3) to
    // (14,4) is a genuine corner crossing — the segment reaches x = 14 and y = 4 at the same
    // parametric value. Supercover requires BOTH flanking cells there, not just the diagonal
    // pair: a thin line would let a move thread (14,3)/(13,4) unseen.
    // Discrimination: fails if the corner branch stops firing for a non-symmetric diagonal —
    // the two flanker assertions are the corner's signature and no staircase produces them.
    let c = supercover_cells((0.0, 0.0), (14.0, 4.0), 1.0).expect("within cap");
    assert!(c.contains(&(0, 0)), "start cell");
    assert!(c.contains(&(14, 4)), "end cell");
    assert!(c.contains(&(14, 3)), "flanker (14,3) at the lattice corner");
    assert!(c.contains(&(13, 4)), "flanker (13,4) at the lattice corner");
}
