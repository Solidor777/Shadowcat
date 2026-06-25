//! Movement-segment rasterization for the M10e-4 movement-restriction gate. Pure, clean-room,
//! headless. INVARIANT: `supercover_cells` is the SAME cell set the gate tests against the
//! visibility mask, so the authoritative move gate and (M10e-6) path preview agree.

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
/// `None` ⇒ caller must fail closed: `cell <= 0.0` (degenerate grid) or the candidate span
/// exceeds `MAX_MOVE_CELLS`.
pub fn supercover_cells(a0: (f64, f64), a1: (f64, f64), cell: f64) -> Option<BTreeSet<Cell>> {
    if cell <= 0.0 {
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

    // Parametric grid traversal: tMaxI/tMaxJ = parameter t∈[0,1] at the next vertical/horizontal
    // grid line; tDeltaI/tDeltaJ = t advance per full cell. A near-zero component yields INFINITY
    // (that axis never steps), so axis-aligned moves degrade to a 1-D walk.
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
    let t_max_i_init = next_boundary(ci, step_i, x0, dx);
    let t_max_j_init = next_boundary(cj, step_j, y0, dy);
    let mut t_max_i = t_max_i_init;
    let mut t_max_j = t_max_j_init;
    let t_delta_i = if dx != 0.0 {
        (cell / dx).abs()
    } else {
        f64::INFINITY
    };
    let t_delta_j = if dy != 0.0 {
        (cell / dy).abs()
    } else {
        f64::INFINITY
    };

    let mut guard: i64 = 0;
    while (ci, cj) != (ei, ej) {
        guard += 1;
        if guard > MAX_MOVE_CELLS {
            return None; // belt-and-suspenders against a pathological loop
        }
        if (t_max_i - t_max_j).abs() < f64::EPSILON {
            // Exact corner crossing: emit BOTH flanking cells (supercover), then step diagonally.
            out.insert((ci + step_i, cj));
            out.insert((ci, cj + step_j));
            ci += step_i;
            cj += step_j;
            t_max_i += t_delta_i;
            t_max_j += t_delta_j;
        } else if t_max_i < t_max_j {
            ci += step_i;
            t_max_i += t_delta_i;
        } else {
            cj += step_j;
            t_max_j += t_delta_j;
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
        assert_eq!(c2, c, "still inside cell (0,0)");
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
        // Supercover includes the two diagonal cells AND both off-diagonal flankers — a thin
        // line would visit only (0,0),(1,1) and let a move slip past an unseen (1,0)/(0,1).
        let c = cells((50.0, 50.0), (150.0, 150.0), 100.0);
        assert!(c.contains(&(0, 0)) && c.contains(&(1, 1)));
        assert!(
            c.contains(&(1, 0)) || c.contains(&(0, 1)),
            "supercover includes at least one corner-flanking cell"
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
        assert!(supercover_cells((0.0, 0.0), (10.0, 10.0), 0.0).is_none());
    }

    #[test]
    fn oversized_move_exceeds_cap_returns_none() {
        // cell 1, a 10_000-long move → > MAX_MOVE_CELLS candidate span → None (caller rejects).
        assert!(supercover_cells((0.0, 0.0), (10_000.0, 10_000.0), 1.0).is_none());
    }
}
