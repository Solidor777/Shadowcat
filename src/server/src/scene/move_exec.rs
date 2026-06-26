//! Pure, lock-free per-path move executor (M1 server-authoritative movement).
//!
//! Walks a proposed cell-path step by step, validating each step against:
//! - `blocks_move` wall geometry (M9a gate — always active),
//! - the caller-supplied `visible` mask (M10e-4 gate — skipped for `Unrestricted`),
//! - a region-arrest hook (M3/M10g stub, always returns false for now).
//!
//! Returns the stop cell + the legal prefix render-path. `truncated` is true when the
//! move stops before `path.last()` for any reason (wall, mask, or region-arrest),
//! including a region-arrest on the final path step.
//!
//! INVARIANT (spec §13 / M10e-4 per-cell parity): step 2 calls the SAME
//! `crate::scene::movement::supercover_cells(prev, next, cell)` and checks
//! `all ∈ visible` that the M10e-4 gate in `Room::publish` does. The caller
//! pre-computes `visible` off the ECS read lock (mirroring `publish`'s
//! `visible_cache`), so this executor is pure and imposes no lock ordering.
//!
//! Coupling: `token_position` is the ECS committed-position seam; any rename
//! must update both this caller and `token_move` in `scene/mod.rs`.

use std::collections::BTreeSet;

use uuid::Uuid;

use crate::scene::{movement::supercover_cells, MovementRestriction, SceneEcs};

/// DoS guard: a path longer than this is rejected outright (never truncated).
/// Sized to a generous multi-waypoint route; far below a coordinate-overflow risk.
pub(crate) const MAX_MOVE_PATH: usize = 256;

/// Epsilon for path[0]-vs-committed-position comparison (scene units).
/// A client rounding the center-of-cell to the nearest float can drift by at most
/// a few ULPs at typical coordinate magnitudes; 1e-6 is strict but not pedantic.
const EPS: f64 = 1e-6;

/// The legal outcome of an `execute_move` call.
pub(crate) struct MoveOutcome {
    /// The path coordinate of the last successfully reached step (`path[stop_index]`).
    pub stop: (f64, f64),
    /// The legal prefix of the input path that was actually walked: `path[0..=stop_index]`.
    pub render_path: Vec<(f64, f64)>,
    /// `true` when the move stopped before `path.last()` — wall, mask, OR region-arrest,
    /// including a region-arrest on the FINAL step (where `stop_index == path.len()-1`
    /// would make the index comparison alone report false; a `stopped_early` bool ensures
    /// that case is reported correctly).
    // The room layer derives truncation from `stop != path.last()`; the field is
    // read by move_exec unit tests (see tests module). Suppress the dead_code lint
    // so the structural information remains available without cluttering call sites.
    #[allow(dead_code)]
    pub truncated: bool,
}

/// Reason an `execute_move` call was rejected before any walking.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MoveReject {
    /// `token` is not a token entity in the ECS (unknown id or wrong doc_type).
    NotAToken,
    /// `path` has fewer than 2 points (no step to walk).
    EmptyPath,
    /// `path` has more than `MAX_MOVE_PATH` points.
    TooLong,
    /// A structural invariant was violated: non-finite coords, non-positive cell,
    /// `path[0]` not at the token's committed position, or a non-adjacent king-step.
    Degenerate,
}

/// Region-arrest hook stub. Returns `true` when a region halts a token entering
/// `cell_center`. Currently always false; the region system (M10g) replaces this body.
///
/// Coupling: the region system updates this function body only; the call site in
/// `execute_move` (step 3 of the walk loop) is the stable hook entry point.
fn region_arrests(_ecs: &SceneEcs, _scene: Uuid, _cell_center: (f64, f64)) -> bool {
    false
}

/// Walk `path` step by step, validating each step against the wall gate (step 1),
/// the vision-mask gate (step 2), and the region-arrest hook (step 3).
///
/// # Parity with M10e-4 (`Room::publish`) — per-cell decision only
///
/// The per-cell decision (step 1 + step 2) uses the SAME primitives as the M10e-4
/// gate in `Room::publish`: `blocks_move`, `supercover_cells`, and the pre-computed
/// `visible` set. This executor and the legacy single-step `publish` gate agree on
/// every cell for every restriction mode.
///
/// This executor is additionally STRICTER than `publish` on path shape: it requires
/// king-step adjacency between every consecutive waypoint pair (≤ 1 cell on each
/// axis). The `publish` whole-segment gate does not enforce this per-step adjacency.
///
/// GM-ness is folded into `restriction == Unrestricted` by the caller (mirroring
/// `publish`'s `if !Unrestricted { continue }` skip).
///
/// # Arguments
///
/// - `ecs` — ECS to query for token position and wall geometry.
/// - `scene` — Scene the token lives in.
/// - `token` — Token doc id.
/// - `path` — Proposed cell-center path; `path[0]` must equal the token's
///   committed position within `EPS`.
/// - `restriction` — Movement restriction mode pre-resolved by the caller from
///   `resolve_scene`; `Unrestricted` means mask is skipped.
/// - `visible` — The resolved mask the gate decision uses (caller resolves off the
///   read lock). Ignored when `Unrestricted`. For `Visible` this is `visible_cells(...)`;
///   for `Revealed` the caller MUST pass `visible_cells(...) ∪ explored` — the same
///   union `publish` tests with `visible.contains(c) || explored.contains(c)` — NOT
///   the raw `visible_cache` alone (§13 parity contract: this executor and the
///   `publish` gate must agree on every cell for every restriction mode).
/// - `cell` — Grid cell size in scene units (positive finite).
pub(crate) fn execute_move(
    ecs: &SceneEcs,
    scene: Uuid,
    token: Uuid,
    path: &[(f64, f64)],
    restriction: MovementRestriction,
    visible: &BTreeSet<(i32, i32)>,
    cell: f64,
) -> Result<MoveOutcome, MoveReject> {
    // --- Input validation (fail closed on every degenerate input) ---
    if path.len() < 2 {
        return Err(MoveReject::EmptyPath);
    }
    if path.len() > MAX_MOVE_PATH {
        return Err(MoveReject::TooLong);
    }
    // NaN is already caught by `is_finite()`; `<= 0.0` rejects non-positive finite values.
    if !cell.is_finite() || cell <= 0.0 {
        return Err(MoveReject::Degenerate);
    }
    if path.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return Err(MoveReject::Degenerate);
    }

    // path[0] must equal the token's committed position. The ECS is authoritative;
    // the client must request from the real position, not a claimed one.
    let cur = ecs.token_position(token).ok_or(MoveReject::NotAToken)?;
    if (cur.0 - path[0].0).abs() > EPS || (cur.1 - path[0].1).abs() > EPS {
        return Err(MoveReject::Degenerate);
    }

    // Cell-index helper: floor-division mapping scene coords to (i, j).
    let to_cell = |p: (f64, f64)| -> (i32, i32) {
        ((p.0 / cell).floor() as i32, (p.1 / cell).floor() as i32)
    };

    // Whether the vision-mask check (step 2) applies for this restriction mode.
    // `Unrestricted` skips the mask; `Visible` and `Revealed` require it.
    // (The caller folds GM-ness into `Unrestricted`, mirroring `publish`.)
    let check_mask = !matches!(restriction, MovementRestriction::Unrestricted);

    // --- Per-step walk ---
    // `stop_index` tracks the last successfully reached path index; starts at 0 (start cell).
    // `stopped_early` is set when the loop breaks due to region-arrest: a region-arrest on
    // the FINAL step sets stop_index == path.len()-1, so the index comparison alone would
    // report truncated=false — the flag captures that case correctly.
    let mut stop_index = 0usize;
    let mut stopped_early = false;
    for i in 1..path.len() {
        let prev = path[i - 1];
        let next = path[i];

        // King-step adjacency guard: each consecutive cell pair must be at most 1 apart
        // on each axis. A jump of 2+ cells is Degenerate (fail closed), not a truncation.
        let (pc, nc) = (to_cell(prev), to_cell(next));
        if (pc.0 - nc.0).abs() > 1 || (pc.1 - nc.1).abs() > 1 {
            return Err(MoveReject::Degenerate);
        }

        // Step 1: wall gate — mirrors `publish` line 199: `if scene.blocks_move(...)`.
        // Active for ALL restriction modes including Unrestricted. GMs mapped to
        // Unrestricted above (mask-skip) still reach this gate: walls are honored for
        // GMs by design in this executor. This intentionally diverges from `publish`'s
        // legacy GM wall-bypass, which is to be retired — do NOT re-grant bypass here.
        if ecs.blocks_move(scene, prev, next) {
            // Stop at prev (the last safely reached cell); truncated.
            stopped_early = true;
            break;
        }

        // Step 2: vision-mask gate — mirrors `publish` lines 217-233.
        // INVARIANT (§13): uses the SAME `supercover_cells` + `visible` set as the
        // M10e-4 gate. `supercover_cells(prev, next, cell)` returns None on any
        // degenerate input; we fail closed (stop at prev) consistent with `publish`'s
        // `return Err(DataError::Forbidden)` on None.
        if check_mask {
            let Some(cells) = supercover_cells(prev, next, cell) else {
                // Degenerate supercover (span overflow or bad coords) → stop here.
                stopped_early = true;
                break;
            };
            if !cells.iter().all(|c| visible.contains(c)) {
                // A supercover cell is not in the visible set → stop at prev.
                stopped_early = true;
                break;
            }
        }

        // Step 3: region-arrest hook — M3/M10g stub (always false).
        // Arrest fires AFTER entering the cell: stop = next, still advanced. A final-step
        // arrest sets stop_index = path.len()-1 and stopped_early = true so truncated is
        // correctly reported even though the index equals the last position.
        if region_arrests(ecs, scene, next) {
            stop_index = i;
            stopped_early = true;
            break;
        }

        // All checks passed: advance to next.
        stop_index = i;
    }

    let render_path = path[0..=stop_index].to_vec();
    // Safe: path.len() >= 2 is already guarded above, so len() - 1 never underflows.
    // `stopped_early` captures arrest on the final step where the index comparison alone
    // would be false (stop_index == path.len()-1 yet the move was arrested).
    let truncated = stopped_early || stop_index < path.len() - 1;
    Ok(MoveOutcome {
        stop: path[stop_index],
        render_path,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- Fixture helpers (mirrors scene/mod.rs test helpers verbatim) ---

    fn doc(id: u128, parent: Option<u128>, ty: &str) -> crate::data::document::Document {
        let mut d = crate::data::document::tests::world_scoped_doc(
            Uuid::from_u128(9),
            Uuid::from_u128(id),
            ty,
        );
        d.parent_id = parent.map(Uuid::from_u128);
        d
    }

    fn entity_doc(
        id: u128,
        parent: u128,
        ty: &str,
        system: serde_json::Value,
    ) -> crate::data::document::Document {
        let mut d = doc(id, Some(parent), ty);
        d.system = system;
        d
    }

    /// Scene with a token at (0,0), no walls, cell=100.
    fn clear_scene() -> (SceneEcs, Uuid, Uuid) {
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let ecs = SceneEcs::from_documents(
            vec![
                entity_doc(10, 0, "scene", json!({ "grid": { "size": 100 } })),
                entity_doc(11, 10, "token", json!({ "x": 0.0, "y": 0.0 })),
            ],
            0,
        );
        (ecs, scene_id, token_id)
    }

    /// Visible set covering all (i,j) in [0,range) × [0,range).
    fn visible_grid(range: i32) -> BTreeSet<(i32, i32)> {
        (0..range)
            .flat_map(|i| (0..range).map(move |j| (i, j)))
            .collect()
    }

    /// Scene with a token at (0,0) and a wall blocking the step (100,0)→(100,100).
    /// Wall segment: x1=50,y1=100,x2=150,y2=100 — horizontal wall at y=100
    /// crossing any vertical move between y<100 and y>100 in the x=[50,150] band.
    fn walled_scene() -> (SceneEcs, Uuid, Uuid) {
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        // Wall segment perpendicular to the (100,0)→(100,100) step: a horizontal
        // line at y=50 that the vertical segment from (100,0) to (100,100) crosses.
        let ecs = SceneEcs::from_documents(
            vec![
                entity_doc(10, 0, "scene", json!({ "grid": { "size": 100 } })),
                entity_doc(11, 10, "token", json!({ "x": 0.0, "y": 0.0 })),
                entity_doc(
                    12,
                    10,
                    "wall",
                    json!({
                        "seg": { "x1": 50, "y1": 50, "x2": 150, "y2": 50 },
                        "blocksMove": true
                    }),
                ),
            ],
            0,
        );
        (ecs, scene_id, token_id)
    }

    // -----------------------------------------------------------------------
    // Tests (binding assertions per brief)
    // -----------------------------------------------------------------------

    #[test]
    fn full_clear_path_reaches_goal() {
        let (ecs, scene, token) = clear_scene();
        // Cells (0,0), (1,0), (1,1) — all visible.
        let visible: BTreeSet<(i32, i32)> =
            (0..3).flat_map(|i| (0..3).map(move |j| (i, j))).collect();
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            MovementRestriction::Visible,
            &visible,
            100.0,
        )
        .unwrap();
        assert_eq!(out.stop, (100.0, 100.0));
        assert_eq!(out.render_path.len(), 3);
        assert!(!out.truncated);
    }

    #[test]
    fn wall_truncates_at_last_legal_cell() {
        let (ecs, scene, token) = walled_scene();
        // Wall at y=50 blocks (100,0)→(100,100); first step (0,0)→(100,0) is clear.
        let visible = visible_grid(4);
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            MovementRestriction::Visible,
            &visible,
            100.0,
        )
        .unwrap();
        assert_eq!(out.stop, (100.0, 0.0)); // stopped before the wall
        assert!(out.truncated);
        assert_eq!(out.render_path, vec![(0.0, 0.0), (100.0, 0.0)]);
    }

    #[test]
    fn unseen_cell_truncates_under_visible_restriction() {
        let (ecs, scene, token) = clear_scene();
        // (0,0) and (1,0) visible; (1,1) NOT in the set.
        let mut visible: BTreeSet<(i32, i32)> = BTreeSet::new();
        visible.insert((0, 0));
        visible.insert((1, 0));
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            MovementRestriction::Visible,
            &visible,
            100.0,
        )
        .unwrap();
        assert_eq!(out.stop, (100.0, 0.0));
        assert!(out.truncated);
    }

    /// Documents the `Revealed`-mode caller contract: the `visible` argument must be
    /// `visible_cells(...) ∪ explored`. When the union includes an otherwise-unseen cell
    /// the move proceeds through it; when the union omits the cell the move truncates there.
    #[test]
    fn revealed_mode_uses_caller_supplied_union_mask() {
        let (ecs, scene, token) = clear_scene();
        // Cell (1,1) is NOT in the raw visible set but IS in the explored union.
        // The caller is responsible for supplying the union; the executor treats it as opaque.
        let mut union_mask: BTreeSet<(i32, i32)> = BTreeSet::new();
        union_mask.insert((0, 0));
        union_mask.insert((1, 0));
        union_mask.insert((1, 1)); // explored cell included by caller in the union

        // With the union mask: all supercover cells are present → reaches goal.
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            MovementRestriction::Revealed,
            &union_mask,
            100.0,
        )
        .unwrap();
        assert_eq!(out.stop, (100.0, 100.0));
        assert!(!out.truncated);

        // Without cell (1,1) in the mask: move truncates at (100,0).
        let mut raw_mask: BTreeSet<(i32, i32)> = BTreeSet::new();
        raw_mask.insert((0, 0));
        raw_mask.insert((1, 0));
        // (1,1) absent — caller did NOT union in explored; step (100,0)→(100,100) blocked.
        let out2 = execute_move(
            &ecs,
            scene,
            token,
            &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            MovementRestriction::Revealed,
            &raw_mask,
            100.0,
        )
        .unwrap();
        assert_eq!(out2.stop, (100.0, 0.0));
        assert!(out2.truncated);
    }

    #[test]
    fn unrestricted_ignores_mask_but_not_walls() {
        let (ecs, scene, token) = walled_scene();
        // Empty mask — mask is ignored under Unrestricted, but the wall still stops it.
        let empty: BTreeSet<(i32, i32)> = BTreeSet::new();
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            MovementRestriction::Unrestricted,
            &empty,
            100.0,
        )
        .unwrap();
        assert_eq!(out.stop, (100.0, 0.0)); // mask ignored, wall still stops it
    }

    #[test]
    fn rejects_path_not_starting_at_token() {
        let (ecs, scene, token) = clear_scene();
        let v: BTreeSet<(i32, i32)> = BTreeSet::new();
        assert!(matches!(
            execute_move(
                &ecs,
                scene,
                token,
                &[(500.0, 0.0), (600.0, 0.0)],
                MovementRestriction::Unrestricted,
                &v,
                100.0
            ),
            Err(MoveReject::Degenerate)
        ));
    }

    #[test]
    fn rejects_overlong_or_nonadjacent_path() {
        let (ecs, scene, token) = clear_scene();
        let v: BTreeSet<(i32, i32)> = BTreeSet::new();
        // Non-adjacent jump: (0,0)→(500,0) skips 4 cells.
        assert!(matches!(
            execute_move(
                &ecs,
                scene,
                token,
                &[(0.0, 0.0), (500.0, 0.0)],
                MovementRestriction::Unrestricted,
                &v,
                100.0
            ),
            Err(MoveReject::Degenerate)
        ));
    }

    #[test]
    fn rejects_too_long_path() {
        let (ecs, scene, token) = clear_scene();
        let v: BTreeSet<(i32, i32)> = BTreeSet::new();
        // Build a path longer than MAX_MOVE_PATH. All steps are (0,0) repeated.
        let path: Vec<(f64, f64)> = std::iter::repeat_n((0.0, 0.0), MAX_MOVE_PATH + 1).collect();
        assert!(matches!(
            execute_move(
                &ecs,
                scene,
                token,
                &path,
                MovementRestriction::Unrestricted,
                &v,
                100.0
            ),
            Err(MoveReject::TooLong)
        ));
    }

    #[test]
    fn rejects_empty_path() {
        let (ecs, scene, token) = clear_scene();
        let v: BTreeSet<(i32, i32)> = BTreeSet::new();
        assert!(matches!(
            execute_move(
                &ecs,
                scene,
                token,
                &[(0.0, 0.0)],
                MovementRestriction::Unrestricted,
                &v,
                100.0
            ),
            Err(MoveReject::EmptyPath)
        ));
    }

    #[test]
    fn rejects_unknown_token() {
        let (ecs, scene, _token) = clear_scene();
        let v: BTreeSet<(i32, i32)> = BTreeSet::new();
        let unknown = Uuid::from_u128(999);
        assert!(matches!(
            execute_move(
                &ecs,
                scene,
                unknown,
                &[(0.0, 0.0), (100.0, 0.0)],
                MovementRestriction::Unrestricted,
                &v,
                100.0
            ),
            Err(MoveReject::NotAToken)
        ));
    }

    #[test]
    fn unrestricted_full_path_no_walls() {
        let (ecs, scene, token) = clear_scene();
        let empty: BTreeSet<(i32, i32)> = BTreeSet::new();
        // Unrestricted with empty mask should reach the goal with no walls.
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            MovementRestriction::Unrestricted,
            &empty,
            100.0,
        )
        .unwrap();
        assert_eq!(out.stop, (100.0, 100.0));
        assert!(!out.truncated);
        assert_eq!(out.render_path.len(), 3);
    }
}
