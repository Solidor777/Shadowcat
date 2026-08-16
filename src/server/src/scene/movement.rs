//! Movement-segment rasterization for the movement-restriction gate. Pure, clean-room,
//! headless. INVARIANT: `supercover_cells` is the SAME cell set the gate tests against the
//! visibility mask, so the authoritative move gate and path preview agree.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeSet;

/// A grid cell coordinate `(i, j)`; cell `(i,j)` covers `[i*cell,(i+1)*cell) × [j*cell,(j+1)*cell)`.
pub type Cell = (i32, i32);

/// DoS guard: a single move may not rasterize more than this many candidate cells. A non-GM
/// move spanning more is rejected (fail-closed), never truncated. Sized to a generous drag at a
/// fine grid; far below a coordinate-overflow stall.
pub(crate) const MAX_MOVE_CELLS: i64 = 1_000_000;

/// Every grid cell the segment `a0→a1` passes through (supercover, not a thin line). Source:
/// supercover line of Euclidean segments — the symmetric extension of Amanatides & Woo (1987)
/// voxel traversal that also emits both cells flanking a shared corner, chosen over Bresenham so
/// a diagonal cannot thread an unseen cell. Both endpoint cells are always included.
///
/// `None` ⇒ caller must fail closed: cell is not a positive finite number, any coordinate is
/// non-finite, or the candidate span exceeds `MAX_MOVE_CELLS`.
pub fn supercover_cells(a0: (f64, f64), a1: (f64, f64), cell: f64) -> Option<BTreeSet<Cell>> {
    // Fail-closed on degenerate cell size. `partial_cmp` returns None for NaN and Some(Less/Equal)
    // for zero/negative values; every non-Greater result (including NaN) → None (fail-closed).
    if !matches!(cell.partial_cmp(&0.0), Some(std::cmp::Ordering::Greater)) {
        return None;
    }
    // Fail-closed on non-finite endpoints: a NaN or Inf coordinate cannot index a cell.
    if !a0.0.is_finite() || !a0.1.is_finite() || !a1.0.is_finite() || !a1.1.is_finite() {
        return None;
    }

    let to_cell = |v: f64| (v / cell).floor() as i32;
    let (x0, y0) = a0;
    let (x1, y1) = a1;
    let (mut ci, mut cj) = (to_cell(x0), to_cell(y0));
    let (ei, ej) = (to_cell(x1), to_cell(y1));

    // Span guard (bbox of endpoint cells) before any allocation/iteration.
    let span = (ci as i64 - ei as i64)
        .abs()
        .saturating_add(1)
        .saturating_mul((cj as i64 - ej as i64).abs().saturating_add(1));
    if span > MAX_MOVE_CELLS {
        return None;
    }

    let mut out = BTreeSet::new();
    out.insert((ci, cj));
    if (ci, cj) == (ei, ej) {
        return Some(out); // intra-cell move (covers a0 == a1)
    }

    let dx = x1 - x0;
    let dy = y1 - y0;
    let step_i = if dx > 0.0 { 1 } else { -1 };
    let step_j = if dy > 0.0 { 1 } else { -1 };

    // Parametric grid traversal: t_max_i/t_max_j = parameter t∈[0,1] at the next vertical/
    // horizontal grid line the traversal has not yet crossed, recomputed from the current cell
    // index on every step. A near-zero component yields INFINITY (that axis never steps), so
    // axis-aligned moves degrade to a 1-D walk.
    let next_boundary = |c: i32, step: i32, origin: f64, d: f64| -> f64 {
        if d == 0.0 {
            return f64::INFINITY;
        }
        let line = if step > 0 {
            (c + 1) as f64 * cell
        } else {
            c as f64 * cell
        };
        (line - origin) / d
    };
    let mut t_max_i = next_boundary(ci, step_i, x0, dx);
    let mut t_max_j = next_boundary(cj, step_j, y0, dy);

    // Per-axis step BUDGET: the exact number of grid-line crossings still needed on each axis to
    // reach (ei,ej). Root-cause fix for the corner-crossing drift bug: the tMax tie test alone
    // cannot distinguish a genuine
    // mid-path corner crossing (more path beyond, both axes still owe a step) from a tie that
    // merely COINCIDES with an axis that has already arrived at its target coordinate (e.g. the
    // segment's own endpoint sits exactly on a lattice intersection, or an earlier forced
    // single-axis step — itself caused by a0 starting exactly on a grid line — put t_max_i and
    // t_max_j into permanent lockstep for the rest of the traversal). Gating the diagonal
    // corner-step on `remaining_i > 0 && remaining_j > 0` makes convergence a property of the
    // step budget (bounded by remaining_i + remaining_j, decremented every iteration), not of
    // floating-point tie-breaking, while still emitting both flankers whenever a real corner
    // crossing has path remaining on both axes (unchanged safe-over-include behavior).
    let mut remaining_i = (ei as i64 - ci as i64).abs();
    let mut remaining_j = (ej as i64 - cj as i64).abs();

    let mut guard: i64 = 0;
    while (ci, cj) != (ei, ej) {
        guard += 1;
        if guard > MAX_MOVE_CELLS {
            return None; // belt-and-suspenders against a pathological loop
        }

        // Corner-crossing tolerance. `t_max_i`/`t_max_j` are each computed from the current cell
        // index by one subtraction and one division, so each carries that much rounding error;
        // the relative form scales the tolerance with the magnitude of the parametric values
        // being compared.
        //
        // When one component is INFINITY (axis-aligned move): INF - INF = NaN; NaN < any finite
        // is false, so no corner branch fires — correct, axis-aligned steps are single-axis.
        //
        // Safe failure direction: over-detecting a near-corner only over-includes flanking cells
        // (rejects a fine move), never under-includes (never lets a forbidden move through).
        let tol = (t_max_i.abs() + t_max_j.abs() + 1.0) * f64::EPSILON * 64.0;
        let tied = (t_max_i - t_max_j).abs() < tol;
        if tied && remaining_i > 0 && remaining_j > 0 {
            // Genuine corner crossing with path remaining on both axes: emit BOTH flanking
            // cells (supercover), then step diagonally.
            out.insert((ci + step_i, cj));
            out.insert((ci, cj + step_j));
            ci += step_i;
            cj += step_j;
            remaining_i -= 1;
            remaining_j -= 1;
            t_max_i = next_boundary(ci, step_i, x0, dx);
            t_max_j = next_boundary(cj, step_j, y0, dy);
        } else if remaining_j == 0 || (remaining_i > 0 && t_max_i < t_max_j) {
            // Either j has already arrived at ej (must not overshoot it), or i is genuinely the
            // next boundary crossed — step i alone.
            ci += step_i;
            remaining_i -= 1;
            t_max_i = next_boundary(ci, step_i, x0, dx);
        } else {
            // Either i has already arrived at ei, or j is the next boundary crossed.
            cj += step_j;
            remaining_j -= 1;
            t_max_j = next_boundary(cj, step_j, y0, dy);
        }
        out.insert((ci, cj));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
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
}
