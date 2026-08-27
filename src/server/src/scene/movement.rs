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
mod tests;
