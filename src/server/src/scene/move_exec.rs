//! Pure, lock-free per-path move executor (M1 server-authoritative movement).
//!
//! Walks a proposed cell-path step by step, validating each step against:
//! - `blocks_move` wall geometry (M9a gate — always active),
//! - the caller-supplied `visible` mask (M10e-4 gate — skipped for `Unrestricted`),
//! - the region field (M10g): impassable stops before entry, arrest stops at entry, terrain
//!   accumulates weighted cost. Always reads the AUTHORITATIVE field (`ecs.region_field(scene,
//!   None)`) — this executor springs every region regardless of what the mover's own pathfind
//!   preview could see (spec §6).
//!
//! Returns the stop cell + the legal prefix render-path + accumulated cost. `truncated` is true
//! when the move stops before `path.last()` for any reason (wall, mask, region-impassable, or
//! region-arrest), including a region-arrest on the final path step.
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

/// Epsilon for path[0]-vs-committed-position comparison (scene units).
/// A client rounding the center-of-cell to the nearest float can drift by at most
/// a few ULPs at typical coordinate magnitudes; 1e-6 is strict but not pedantic.
const EPS: f64 = 1e-6;

/// DoS guard for `gate_walk`: a walk requiring more than this many dense samples is
/// rejected outright, never truncated. Arc-length/cell-count based — a single continuous
/// segment can be arbitrarily long, so an authored-vertex-count cap is not the right invariant
/// (unlike the pre-M10f-2 `MAX_MOVE_PATH`, which bounded the number of AUTHORED waypoints, not
/// dense samples).
pub(crate) const MAX_GATE_WALK_SAMPLES: usize = 4096;

/// Magnitude ceiling (scene units) for any `gate_walk` input path coordinate, checked
/// structurally BEFORE the per-step tolerance arithmetic below (mirrors `navmesh::
/// MAX_NAVMESH_COORD`'s convention: bound the input before any downstream arithmetic that is
/// sensitive to magnitude, not after).
///
/// This value is deliberately much smaller than `navmesh::MAX_NAVMESH_COORD` (1e15) — the two
/// bounds guard against DIFFERENT failure modes and neither number transfers to the other's
/// module. `MAX_NAVMESH_COORD` guards an `f64 -> f32` cast that only saturates near `f32::MAX`
/// (~3.4e38), so 1e15 is safe there with enormous headroom. Here, the per-step tolerance below
/// (`tol = (2*max(|px|,|nx|,|py|,|ny|) + cell + 1) * f64::EPSILON * 64`) scales linearly with
/// coordinate magnitude and empirically exceeds a full 1.0-unit overshoot margin once that
/// magnitude passes roughly `3.5e13` (verified directly against this formula) — reusing `1e15` as
/// this bound would NOT close the false-identity gap this constant exists to prevent (a segment
/// at base magnitude `1e14`, well under `1e15`, still misclassifies). `1e9` sits comfortably
/// below that ~3.5e13 threshold (tol stays under ~3e-5, negligible) while remaining generously
/// large for any real authored scene (`resolve_scene`'s default bounds are `100x100` grid units;
/// even an extreme scene would not need path coordinates near this ceiling).
///
/// Note: this bounds path COORDINATES, not `cell` itself (`scene_grid_sizes()` has no upper
/// cap). That is sufficient, not an oversight: the false-identity failure only manifests when
/// `cheby` (the per-step Chebyshev distance, itself bounded by `2 * MAX_GATE_WALK_COORD` once
/// path coordinates are capped) is close enough to `cell` to sit within `tol` of it. An
/// unboundedly large `cell` relative to a bounded `cheby` is never itself misclassified — it is
/// trivially and correctly identified as a single-cell step, not a false one — so bounding
/// coordinates alone closes the gap without needing to separately bound `cell`.
pub(crate) const MAX_GATE_WALK_COORD: f64 = 1.0e9;

/// One dense sample in a `gate_walk` output: a point at most one cell from its predecessor,
/// plus (when this sample exactly reproduces an authored input vertex) that vertex's index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GateSample {
    pub pos: (f64, f64),
    /// `Some(i)` exactly when `pos == path[i]` (this sample completes an authored segment);
    /// `None` for an interior subdivision point with no authored counterpart.
    pub authored_idx: Option<usize>,
}

/// Subdivide `path` into dense samples that are each at most one cell apart (Chebyshev),
/// preserving already-≤1-cell input segments EXACTLY — this makes the gate walk an IDENTITY on
/// grid input (cell-center vertices, ≤1 cell apart on every axis including diagonals), so
/// grid-parity is a property of the code shape rather than something proven only by testing.
///
/// `None` (fail-closed) on: a non-finite `path` coordinate, a `path` coordinate whose magnitude
/// exceeds `MAX_GATE_WALK_COORD` (see that constant's doc comment), a non-finite or non-positive
/// `cell`, an authored `path` longer than `MAX_GATE_WALK_SAMPLES` (checked before any allocation —
/// a valid walk output can never exceed the cap even in the non-subdividing case), or an emitted
/// sample count that would exceed `MAX_GATE_WALK_SAMPLES`.
///
/// # Coordinate bound (magnitude-independent, checked BEFORE any tolerance arithmetic)
///
/// The per-step Chebyshev comparison below uses a magnitude-SCALED float tolerance (built from
/// `px`/`nx`/`py`/`ny`'s own absolute values, mirroring `supercover_cells`'s corner-test
/// convention — see that comparison's doc comment) to absorb subtraction rounding error at
/// ordinary grid magnitudes. That scaling is unbounded: at a large enough coordinate magnitude
/// the tolerance itself can exceed a full cell length, which would silently misclassify a
/// genuinely-multi-cell segment as a single identity step (a real gate-skip: a swallowed
/// intermediate cell never gets a per-step wall/vision-mask/region check). `MAX_GATE_WALK_COORD`
/// eliminates the failure mode categorically — inputs beyond the bound are rejected outright, so
/// the tolerance can never reach a magnitude where this matters.
pub(crate) fn gate_walk(path: &[(f64, f64)], cell: f64) -> Option<Vec<GateSample>> {
    if !cell.is_finite() || cell <= 0.0 {
        return None;
    }
    if path.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return None;
    }
    if path
        .iter()
        .any(|p| p.0.abs() > MAX_GATE_WALK_COORD || p.1.abs() > MAX_GATE_WALK_COORD)
    {
        return None;
    }
    if path.is_empty() {
        return Some(Vec::new());
    }
    if path.len() > MAX_GATE_WALK_SAMPLES {
        return None;
    }

    let mut out = Vec::with_capacity(path.len());
    out.push(GateSample {
        pos: path[0],
        authored_idx: Some(0),
    });

    for i in 1..path.len() {
        let (px, py) = path[i - 1];
        let (nx, ny) = path[i];
        let cheby = (nx - px).abs().max((ny - py).abs());
        // Magnitude-relative tolerance (mirrors `movement.rs`'s `supercover_cells` corner-test
        // convention, `(a.abs() + b.abs() + 1.0) * f64::EPSILON * K`): a genuine single-cell
        // step's `cheby` is computed as `(nx-px).abs().max((ny-py).abs())` — two subtractions
        // whose rounding error scales with the magnitude of their OPERANDS (`px`/`nx`/`py`/`ny`),
        // not with the small result (`cheby`/`cell`) itself, so the tolerance must be built from
        // the operand magnitudes: at scene coordinates in the tens of thousands (an ordinary
        // multi-cell grid position), the subtraction can drift by ~1e-12, far above a
        // cheby/cell-scaled tolerance. A fixed absolute epsilon (e.g. `1e-9`) would itself fail
        // at larger coordinate magnitudes in the other direction, so the tolerance scales with
        // the magnitude of the coordinates actually subtracted; 64 ULPs matches
        // `supercover_cells`'s chosen constant.
        let tol = (px.abs().max(nx.abs()) + py.abs().max(ny.abs()) + cell.abs() + 1.0)
            * f64::EPSILON
            * 64.0;
        let k_f = if cheby <= cell + tol {
            1.0
        } else {
            (cheby / cell).ceil()
        };
        if !k_f.is_finite() || k_f < 1.0 || k_f > MAX_GATE_WALK_SAMPLES as f64 {
            return None;
        }
        let k = k_f as u64;
        for step in 1..=k {
            if out.len() >= MAX_GATE_WALK_SAMPLES {
                return None;
            }
            let pos = if step == k {
                (nx, ny) // exact endpoint, no float drift
            } else {
                let t = step as f64 / k as f64;
                (px + t * (nx - px), py + t * (ny - py))
            };
            let authored_idx = if step == k { Some(i) } else { None };
            out.push(GateSample { pos, authored_idx });
        }
    }
    Some(out)
}

/// The legal outcome of an `execute_move` call.
#[derive(Debug)]
pub(crate) struct MoveOutcome {
    /// The path coordinate of the last successfully reached step (`path[stop_index]`).
    pub stop: (f64, f64),
    /// The legal prefix of the input path that was actually walked: every authored vertex
    /// fully traversed, plus the exact stop point when the stop lands mid-subdivision (a
    /// continuous-path truncation that is not itself an authored vertex).
    pub render_path: Vec<(f64, f64)>,
    /// `true` when the move stopped before `path.last()` — wall, mask, region-impassable, OR
    /// region-arrest, including a region-arrest on the FINAL step (where `stop_index ==
    /// path.len()-1` would make the index comparison alone report false; a `stopped_early`
    /// bool ensures that case is reported correctly).
    #[allow(dead_code)]
    pub truncated: bool,
    /// Total terrain-weighted cost accumulated over the walked prefix. Not consumed by any
    /// per-turn movement-budget cap (none exists yet); exposed for the wire and future use.
    pub cost: f64,
}

/// Reason an `execute_move` call was rejected before any walking.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MoveReject {
    /// `token` is not a token entity in the ECS (unknown id or wrong doc_type).
    NotAToken,
    /// `path` has fewer than 2 points (no step to walk).
    EmptyPath,
    /// The path's gate-walk (§4.3: arc-length/sample-count, not authored-vertex-count) would
    /// exceed `MAX_GATE_WALK_SAMPLES` — the DoS bound. Replaces the pre-M10f-2 authored-vertex
    /// cap: a single arbitrarily-long continuous segment is now the relevant DoS surface, not
    /// the number of authored waypoints.
    TooLong,
    /// A structural invariant was violated: non-finite coords, or `path[0]` not at the
    /// token's committed position. (Pre-M10f-2 this variant also covered a non-adjacent
    /// king-step jump; that case is now subdivided-and-gated instead of rejected — see
    /// `gate_walk`, §4.2.)
    Degenerate,
}

/// Walk `path` step by step, validating each step against the wall gate (step 1), the
/// vision-mask gate (step 2), and the region field (step 3).
///
/// # Engine-agnostic gate walk (M10f-2)
///
/// `path` may be ANY polyline — grid A* emits cell-center vertices ≤1 cell apart; the
/// polyanya router emits any-angle vertices arbitrarily far apart. `gate_walk` subdivides it
/// into a dense walk where every consecutive pair is ≤1 cell apart, preserving already-≤1-cell
/// segments EXACTLY (identity on grid input — see `gate_walk`'s doc comment). The per-step
/// gate below runs over this DENSE walk; the coarse `render_path` returned to the caller is
/// reconstructed from the authored vertices actually traversed plus the exact stop point.
///
/// # Parity with M10e-4 (`Room::publish`) — per-cell decision only
///
/// The per-cell decision (step 1 + step 2) uses the SAME primitives as the M10e-4 gate in
/// `Room::publish`: `blocks_move`, `supercover_cells`, and the pre-computed `visible` set.
/// This executor and the legacy single-step `publish` gate agree on every cell for every
/// restriction mode. For a grid input this executor is byte-identical in outcome to the
/// pre-M10f-2 king-step executor (see `execute_move_kingstep_oracle` and the differential
/// parity test suite).
///
/// A >1-cell authored jump is no longer rejected outright: it is subdivided by `gate_walk`
/// and gated per crossed cell, exactly as if the client had sent the explicit intermediate
/// waypoints (§4.2) — no new capability, since a well-formed sequence of intermediate
/// waypoints was always legal.
///
/// GM-ness is folded into `restriction == Unrestricted` by the caller (mirroring `publish`'s
/// `if !Unrestricted { continue }` skip).
///
/// # Arguments
///
/// - `ecs` — ECS to query for token position and wall geometry.
/// - `scene` — Scene the token lives in.
/// - `token` — Token doc id.
/// - `path` — Proposed path (cell centers for grid, any-angle vertices for continuous);
///   `path[0]` must equal the token's committed position within `EPS`.
/// - `restriction` — Movement restriction mode pre-resolved by the caller from
///   `resolve_scene`; `Unrestricted` means mask is skipped.
/// - `visible` — The resolved mask the gate decision uses (caller resolves off the read
///   lock). Ignored when `Unrestricted`. For `Visible` this is `visible_cells(...)`; for
///   `Revealed` the caller MUST pass `visible_cells(...) ∪ explored`.
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
    if !cell.is_finite() || cell <= 0.0 {
        return Err(MoveReject::Degenerate);
    }
    if path.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return Err(MoveReject::Degenerate);
    }

    // path[0] must equal the token's committed position. The ECS is authoritative; the
    // client must request from the real position, not a claimed one.
    let cur = ecs.token_position(token).ok_or(MoveReject::NotAToken)?;
    if (cur.0 - path[0].0).abs() > EPS || (cur.1 - path[0].1).abs() > EPS {
        return Err(MoveReject::Degenerate);
    }

    // Subdivide into the dense ≤1-cell gate walk (§4.1/§4.3 of the design spec); identity on
    // grid input. `None` means the walk would exceed MAX_GATE_WALK_SAMPLES — fail closed.
    let walk = gate_walk(path, cell).ok_or(MoveReject::TooLong)?;
    // walk.len() >= 2 always here: path.len() >= 2 is already guaranteed above, and the loop
    // inside gate_walk appends at least one sample per authored segment.

    let to_cell = |p: (f64, f64)| -> (i32, i32) {
        ((p.0 / cell).floor() as i32, (p.1 / cell).floor() as i32)
    };

    // Whether the vision-mask check (step 2) applies for this restriction mode.
    let check_mask = !matches!(restriction, MovementRestriction::Unrestricted);

    // Authoritative region field (M10g): always the full field, never filtered — this
    // executor springs secret regions regardless of what the mover's pathfind preview
    // could see (§6).
    let regions = ecs.region_field(scene, None);

    // --- Per-step walk over the DENSE gate walk ---
    let mut stop_idx = 0usize; // index into `walk`
    let mut stopped_early = false;
    let mut cost = 0.0;
    // The cell already accounted for by region/cost logic. The START cell is never itself
    // "entered" (mirrors the pre-refactor loop, which begins cost accrual at i=1 /
    // to_cell(next)).
    let mut last_region_cell = to_cell(walk[0].pos);

    for i in 1..walk.len() {
        let prev = walk[i - 1].pos;
        let next = walk[i].pos;

        // Step 1: wall gate — unconditional, every dense sub-segment.
        if ecs.blocks_move(scene, prev, next) {
            stopped_early = true;
            break;
        }

        // Step 2: vision-mask gate — every dense sub-segment. This density is exactly why
        // gate_walk exists: supercover_cells is well-defined and dense enough to cover the
        // swept footprint for an any-angle segment, not just a king step.
        if check_mask {
            let Some(cells) = supercover_cells(prev, next, cell) else {
                stopped_early = true;
                break;
            };
            if !cells.iter().all(|c| visible.contains(c)) {
                stopped_early = true;
                break;
            }
        }

        // Step 3: region gate (M10g), keyed on CELL-ENTRY TRANSITIONS, not per dense sample
        // — a continuous path subdivided into several sub-cell samples within the same cell
        // is evaluated exactly once for that cell, matching the pre-refactor accrual count
        // for grid input (where every authored step already crossed into a distinct new
        // cell). Center-cell only, mirroring the pre-existing documented asymmetry against
        // the router's footprint-aware check (see pathfinding.rs's `cell_enterable` docs).
        let next_cell = to_cell(next);
        if next_cell != last_region_cell {
            if regions.is_impassable(next_cell) {
                stopped_early = true;
                break;
            }
            cost += regions.terrain_multiplier(next_cell);
            if regions.is_arrest(next_cell) {
                stop_idx = i;
                stopped_early = true;
                break;
            }
            last_region_cell = next_cell;
        }

        // All checks passed: advance to next.
        stop_idx = i;
    }

    // --- Coarse render_path: authored vertices fully traversed + the exact stop point ---
    let stop_sample = walk[stop_idx];
    let render_path = match stop_sample.authored_idx {
        // Stop lands exactly on an authored vertex (always true for grid input, since
        // gate_walk is identity there): the coarse path is the authored-vertex prefix,
        // byte-identical to the pre-refactor executor.
        Some(authored_i) => path[0..=authored_i].to_vec(),
        // Stop lands mid-subdivision (only possible for a genuinely long/any-angle
        // segment): the coarse path is every authored vertex fully passed, plus the exact
        // stop point.
        None => {
            let last_authored = walk[0..stop_idx]
                .iter()
                .rev()
                .find_map(|s| s.authored_idx)
                .unwrap_or(0);
            let mut rp = path[0..=last_authored].to_vec();
            rp.push(stop_sample.pos);
            rp
        }
    };

    // Safe: walk.len() >= 2 is guaranteed above, so len() - 1 never underflows.
    let truncated = stopped_early || stop_idx < walk.len() - 1;
    Ok(MoveOutcome {
        stop: stop_sample.pos,
        render_path,
        truncated,
        cost,
    })
}

/// Frozen differential-test oracle: a verbatim copy of the king-step executor's logic, kept
/// solely so a differential test can prove a sampled/refactored executor agrees with it on every
/// grid input. This is not a permanently-maintained second executor — reusing it as one would
/// reintroduce the engine fork movement unification exists to avoid.
///
/// TODO: delete once grid-input parity between the sampled executor and this oracle is proven and
/// frozen as literal fixtures.
/// DoS guard: a path longer than this is rejected outright (never truncated). Scoped to the
/// frozen pre-M10f-2 oracle only — superseded in production by `MAX_GATE_WALK_SAMPLES`.
#[cfg(test)]
const MAX_MOVE_PATH: usize = 256;

#[cfg(test)]
pub(crate) fn execute_move_kingstep_oracle(
    ecs: &SceneEcs,
    scene: Uuid,
    token: Uuid,
    path: &[(f64, f64)],
    restriction: MovementRestriction,
    visible: &BTreeSet<(i32, i32)>,
    cell: f64,
) -> Result<MoveOutcome, MoveReject> {
    if path.len() < 2 {
        return Err(MoveReject::EmptyPath);
    }
    if path.len() > MAX_MOVE_PATH {
        return Err(MoveReject::TooLong);
    }
    if !cell.is_finite() || cell <= 0.0 {
        return Err(MoveReject::Degenerate);
    }
    if path.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return Err(MoveReject::Degenerate);
    }

    let cur = ecs.token_position(token).ok_or(MoveReject::NotAToken)?;
    if (cur.0 - path[0].0).abs() > EPS || (cur.1 - path[0].1).abs() > EPS {
        return Err(MoveReject::Degenerate);
    }

    let to_cell = |p: (f64, f64)| -> (i32, i32) {
        ((p.0 / cell).floor() as i32, (p.1 / cell).floor() as i32)
    };

    let check_mask = !matches!(restriction, MovementRestriction::Unrestricted);
    let regions = ecs.region_field(scene, None);

    let mut stop_index = 0usize;
    let mut stopped_early = false;
    let mut cost = 0.0;
    for i in 1..path.len() {
        let prev = path[i - 1];
        let next = path[i];

        let (pc, nc) = (to_cell(prev), to_cell(next));
        if (pc.0 - nc.0).abs() > 1 || (pc.1 - nc.1).abs() > 1 {
            return Err(MoveReject::Degenerate);
        }

        if ecs.blocks_move(scene, prev, next) {
            stopped_early = true;
            break;
        }

        if check_mask {
            let Some(cells) = supercover_cells(prev, next, cell) else {
                stopped_early = true;
                break;
            };
            if !cells.iter().all(|c| visible.contains(c)) {
                stopped_early = true;
                break;
            }
        }

        let region_cell = to_cell(next);
        if regions.is_impassable(region_cell) {
            stopped_early = true;
            break;
        }
        cost += regions.terrain_multiplier(region_cell);
        if regions.is_arrest(region_cell) {
            stop_index = i;
            stopped_early = true;
            break;
        }

        stop_index = i;
    }

    let render_path = path[0..=stop_index].to_vec();
    let truncated = stopped_early || stop_index < path.len() - 1;
    Ok(MoveOutcome {
        stop: path[stop_index],
        render_path,
        truncated,
        cost,
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
    fn long_jump_is_subdivided_and_gated_not_rejected() {
        // A >1-cell authored jump is no longer rejected outright (§4.2): it is subdivided by
        // gate_walk and gated per crossed cell, exactly as if the client had sent the
        // explicit intermediate waypoints. All crossed cells here are visible and
        // wall-clear, so the jump succeeds.
        let (ecs, scene, token) = clear_scene();
        let visible = visible_grid(6);
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(0.0, 0.0), (500.0, 0.0)], // 5 cells in one authored jump
            MovementRestriction::Visible,
            &visible,
            100.0,
        )
        .unwrap();
        assert_eq!(out.stop, (500.0, 0.0));
        assert!(!out.truncated);
    }

    #[test]
    fn long_jump_truncates_at_the_fog_boundary_mid_segment() {
        // The subdivided jump crosses into an unseen cell partway through the authored
        // segment — the executor must truncate exactly at the fog boundary (a point that is
        // NOT an authored vertex), not admit the whole jump nor reject it outright.
        let (ecs, scene, token) = clear_scene();
        // Only cells (0,0),(1,0),(2,0) are visible; the 5-cell jump would reach unseen (3,0).
        let mut visible: BTreeSet<(i32, i32)> = BTreeSet::new();
        visible.insert((0, 0));
        visible.insert((1, 0));
        visible.insert((2, 0));
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(0.0, 0.0), (500.0, 0.0)],
            MovementRestriction::Visible,
            &visible,
            100.0,
        )
        .unwrap();
        assert!(out.truncated);
        assert_eq!(
            out.stop,
            (200.0, 0.0),
            "truncates entering cell (2,0), before unseen cell (3,0)"
        );
        assert_eq!(out.render_path, vec![(0.0, 0.0), (200.0, 0.0)]);
    }

    #[test]
    fn rejects_path_exceeding_gate_walk_cap() {
        // Replaces the old vertex-count TooLong check (§4.3): the DoS bound is now
        // arc-length/gate-walk-sample based. A single segment whose Chebyshev length would
        // require more than MAX_GATE_WALK_SAMPLES sub-steps fails closed, never truncated.
        let (ecs, scene, token) = clear_scene();
        let v: BTreeSet<(i32, i32)> = BTreeSet::new();
        assert!(matches!(
            execute_move(
                &ecs,
                scene,
                token,
                &[(0.0, 0.0), (1.0e7, 0.0)],
                MovementRestriction::Unrestricted,
                &v,
                1.0,
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

    fn region_doc(
        id: u128,
        parent: u128,
        behavior: &str,
        cost: f64,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    ) -> crate::data::document::Document {
        entity_doc(
            id,
            parent,
            "region",
            json!({
                "shape": { "kind": "rect", "points": [x0, y0, x1, y1] },
                "behavior": behavior,
                "cost": cost,
                "enabled": true,
            }),
        )
    }

    #[test]
    fn impassable_region_stops_before_entry_like_a_wall() {
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let ecs = SceneEcs::from_documents(
            vec![
                entity_doc(10, 0, "scene", json!({ "grid": { "size": 100 } })),
                entity_doc(11, 10, "token", json!({ "x": 0.0, "y": 0.0 })),
                region_doc(12, 10, "impassable", 1.0, 50.0, 0.0, 150.0, 100.0),
            ],
            0,
        );
        let visible = visible_grid(3);
        let out = execute_move(
            &ecs,
            scene_id,
            token_id,
            &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            MovementRestriction::Unrestricted,
            &visible,
            100.0,
        )
        .unwrap();
        assert_eq!(
            out.stop,
            (0.0, 0.0),
            "stops BEFORE entering the impassable cell, like a wall"
        );
        assert!(out.truncated);
    }

    #[test]
    fn arrest_region_stops_at_entry_including_final_step() {
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let ecs = SceneEcs::from_documents(
            vec![
                entity_doc(10, 0, "scene", json!({ "grid": { "size": 100 } })),
                entity_doc(11, 10, "token", json!({ "x": 0.0, "y": 0.0 })),
                region_doc(12, 10, "arrest", 1.0, 50.0, -50.0, 150.0, 50.0),
            ],
            0,
        );
        let visible = visible_grid(3);
        let out = execute_move(
            &ecs,
            scene_id,
            token_id,
            &[(0.0, 0.0), (100.0, 0.0)],
            MovementRestriction::Unrestricted,
            &visible,
            100.0,
        )
        .unwrap();
        assert_eq!(
            out.stop,
            (100.0, 0.0),
            "arrest stops AT the cell, not before it"
        );
        assert!(
            out.truncated,
            "final-step arrest must still report truncated=true"
        );
    }

    #[test]
    fn terrain_region_accumulates_weighted_cost() {
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let ecs = SceneEcs::from_documents(
            vec![
                entity_doc(10, 0, "scene", json!({ "grid": { "size": 100 } })),
                entity_doc(11, 10, "token", json!({ "x": 0.0, "y": 0.0 })),
                region_doc(12, 10, "terrain", 2.5, 50.0, 0.0, 150.0, 100.0),
            ],
            0,
        );
        let visible = visible_grid(3);
        let out = execute_move(
            &ecs,
            scene_id,
            token_id,
            &[(0.0, 0.0), (100.0, 0.0)],
            MovementRestriction::Unrestricted,
            &visible,
            100.0,
        )
        .unwrap();
        assert!((out.cost - 2.5).abs() < 1e-9);
    }

    #[test]
    fn authoritative_field_springs_a_secret_region_a_player_was_routed_through() {
        // A gm_only impassable region: move_exec must still enforce it (it always uses the
        // authoritative field, spec §6), even though a player's pathfind field never saw it.
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let mut secret = region_doc(12, 10, "impassable", 1.0, 50.0, 0.0, 150.0, 100.0);
        secret
            .permissions
            .property_overrides
            .insert("/system".into(), crate::data::document::Visibility::GmOnly);
        let ecs = SceneEcs::from_documents(
            vec![
                entity_doc(10, 0, "scene", json!({ "grid": { "size": 100 } })),
                entity_doc(11, 10, "token", json!({ "x": 0.0, "y": 0.0 })),
                secret,
            ],
            0,
        );
        let visible = visible_grid(3);
        let out = execute_move(
            &ecs,
            scene_id,
            token_id,
            &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            MovementRestriction::Visible,
            &visible,
            100.0,
        )
        .unwrap();
        assert_eq!(
            out.stop,
            (0.0, 0.0),
            "authoritative field springs the secret impassable region"
        );
    }

    // -----------------------------------------------------------------------
    // gate_walk tests
    // -----------------------------------------------------------------------

    #[test]
    fn gate_walk_is_identity_on_orthogonal_grid_steps() {
        let path = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)];
        let walk = gate_walk(&path, 100.0).unwrap();
        let positions: Vec<(f64, f64)> = walk.iter().map(|s| s.pos).collect();
        assert_eq!(positions, path.to_vec());
        let authored: Vec<Option<usize>> = walk.iter().map(|s| s.authored_idx).collect();
        assert_eq!(authored, vec![Some(0), Some(1), Some(2)]);
    }

    #[test]
    fn gate_walk_is_identity_on_diagonal_grid_steps() {
        let path = [(0.0, 0.0), (100.0, 100.0), (200.0, 200.0)];
        let walk = gate_walk(&path, 100.0).unwrap();
        let positions: Vec<(f64, f64)> = walk.iter().map(|s| s.pos).collect();
        assert_eq!(positions, path.to_vec());
    }

    #[test]
    fn gate_walk_subdivides_a_long_axis_aligned_segment_into_at_most_one_cell_steps() {
        // (0,0) -> (400,0) at cell=100: Chebyshev length 400 -> subdivided into 4 unit steps.
        let path = [(0.0, 0.0), (400.0, 0.0)];
        let walk = gate_walk(&path, 100.0).unwrap();
        assert_eq!(walk.first().unwrap().pos, (0.0, 0.0));
        assert_eq!(walk.last().unwrap().pos, (400.0, 0.0));
        for w in walk.windows(2) {
            let cheby = (w[1].pos.0 - w[0].pos.0)
                .abs()
                .max((w[1].pos.1 - w[0].pos.1).abs());
            assert!(
                cheby <= 100.0 + 1e-9,
                "step {:?}->{:?} exceeds 1 cell",
                w[0].pos,
                w[1].pos
            );
        }
        // Only the endpoints are authored; interior samples are not.
        assert_eq!(walk.first().unwrap().authored_idx, Some(0));
        assert_eq!(walk.last().unwrap().authored_idx, Some(1));
        assert!(walk[1..walk.len() - 1]
            .iter()
            .all(|s| s.authored_idx.is_none()));
    }

    #[test]
    fn gate_walk_subdivides_a_long_any_angle_segment() {
        // Continuous, non-axis-aligned: (0,0) -> (250, 90) at cell=100.
        // Chebyshev length = max(250, 90) = 250 -> ceil(250/100) = 3 substeps.
        let path = [(0.0, 0.0), (250.0, 90.0)];
        let walk = gate_walk(&path, 100.0).unwrap();
        assert_eq!(walk.len(), 4); // start + 3 substeps
        assert_eq!(walk.last().unwrap().pos, (250.0, 90.0));
        for w in walk.windows(2) {
            let cheby = (w[1].pos.0 - w[0].pos.0)
                .abs()
                .max((w[1].pos.1 - w[0].pos.1).abs());
            assert!(cheby <= 100.0 + 1e-9);
        }
    }

    #[test]
    fn gate_walk_fails_closed_on_non_finite_coordinate() {
        assert!(gate_walk(&[(0.0, 0.0), (f64::NAN, 0.0)], 100.0).is_none());
        assert!(gate_walk(&[(0.0, 0.0), (f64::INFINITY, 0.0)], 100.0).is_none());
    }

    #[test]
    fn gate_walk_fails_closed_on_degenerate_cell() {
        assert!(gate_walk(&[(0.0, 0.0), (100.0, 0.0)], 0.0).is_none());
        assert!(gate_walk(&[(0.0, 0.0), (100.0, 0.0)], -1.0).is_none());
        assert!(gate_walk(&[(0.0, 0.0), (100.0, 0.0)], f64::NAN).is_none());
    }

    #[test]
    fn gate_walk_fails_closed_when_over_the_sample_cap() {
        // A single segment whose subdivision count alone exceeds the cap.
        let path = [(0.0, 0.0), (1.0e7, 0.0)]; // cell=1.0 -> 10,000,000 substeps
        assert!(gate_walk(&path, 1.0).is_none());
    }

    #[test]
    fn gate_walk_fails_closed_when_a_single_segment_lands_exactly_on_the_sample_cap() {
        // k_f == MAX_GATE_WALK_SAMPLES exactly: the walk still needs 1 (start sample) +
        // MAX_GATE_WALK_SAMPLES total samples, one over the cap. Must fail closed, not
        // silently accept an off-by-one under-count.
        let cell = 1.0;
        let cheby = MAX_GATE_WALK_SAMPLES as f64 * cell;
        let path = [(0.0, 0.0), (cheby, 0.0)];
        assert!(gate_walk(&path, cell).is_none());
    }

    #[test]
    fn gate_walk_fails_closed_on_cumulative_cross_segment_sample_cap() {
        // Each segment is individually well under the per-segment cap, but the summed
        // sample count across segments exceeds MAX_GATE_WALK_SAMPLES. The pre-loop
        // per-segment check alone would miss this; only the loop-internal running-total
        // check (`out.len() >= MAX_GATE_WALK_SAMPLES`) catches it.
        let cell = 1.0;
        let seg_len = (MAX_GATE_WALK_SAMPLES / 2 + 100) as f64 * cell; // under the cap alone
        let path = [
            (0.0, 0.0),
            (seg_len, 0.0),
            (seg_len, seg_len), // second segment pushes the running total over the cap
        ];
        assert!(gate_walk(&path, cell).is_none());
    }

    #[test]
    fn gate_walk_rejects_an_authored_path_longer_than_the_sample_cap_before_allocating() {
        // Every step is a genuine 1-cell identity step (no subdivision at all), but the
        // authored vertex count alone exceeds the cap. Must fail closed rather than
        // pre-allocate `Vec::with_capacity(path.len())` for an arbitrarily large `path`.
        let cell = 100.0;
        let path: Vec<(f64, f64)> = (0..=(MAX_GATE_WALK_SAMPLES + 1))
            .map(|i| (i as f64 * cell, 0.0))
            .collect();
        assert!(gate_walk(&path, cell).is_none());
    }

    #[test]
    fn gate_walk_is_identity_on_non_round_cell_size_under_floating_point_noise() {
        // Non-round cell (a perfectly normal GM-configured value; `scene/mod.rs` puts no
        // round-number constraint on `cell`). A zero-tolerance `cheby <= cell` comparison
        // spuriously subdivides some fraction of genuine single-cell steps here due to
        // independent floating-point rounding in the two coordinate subtractions.
        let cell = 33.33_f64;
        for i in 0..2000u32 {
            let base = i as f64 * cell;
            // Orthogonal single-cell step.
            let ortho = [(base, 0.0), (base + cell, 0.0)];
            let walk = gate_walk(&ortho, cell).unwrap();
            assert_eq!(
                walk.len(),
                2,
                "orthogonal single-cell step at i={i} was spuriously subdivided: {walk:?}"
            );
            // Diagonal single-cell step.
            let diag = [(base, base), (base + cell, base + cell)];
            let walk = gate_walk(&diag, cell).unwrap();
            assert_eq!(
                walk.len(),
                2,
                "diagonal single-cell step at i={i} was spuriously subdivided: {walk:?}"
            );
        }
    }

    #[test]
    fn gate_walk_fails_closed_on_extreme_magnitude_coordinate_instead_of_false_identity() {
        // Second buddy-check round on the first tolerance fix: at large enough base
        // coordinates the magnitude-scaled tolerance can itself exceed a full cell length,
        // silently collapsing a genuinely-multi-cell segment (cheby == cell + 1.0, which must
        // subdivide into 2 substeps) into a false single-step identity. Reproduced directly at
        // base=1e14, cell=33.33 by both independent reviewers (tol there is ~2.84, already far
        // past the 1.0 excess this segment carries) — well above `MAX_GATE_WALK_COORD` (1e9), so
        // the bound must reject it outright (fail closed) rather than let the tolerance
        // misclassify it.
        let cell = 33.33_f64;
        let base = 1.0e14_f64;
        let path = [(base, 0.0), (base + cell + 1.0, 0.0)];
        assert!(
            gate_walk(&path, cell).is_none(),
            "extreme-magnitude segment must fail closed, not silently collapse to identity"
        );
    }

    #[test]
    fn gate_walk_fails_closed_on_coordinate_over_the_magnitude_bound() {
        // Direct test of the new bound itself: a coordinate just over `MAX_GATE_WALK_COORD`
        // must be rejected even on an otherwise-trivial single-cell step (isolates the bound
        // check from the tolerance-overshoot scenario above).
        let cell = 100.0_f64;
        let over = MAX_GATE_WALK_COORD + 1.0;
        assert!(gate_walk(&[(over, 0.0), (over + cell, 0.0)], cell).is_none());
        assert!(gate_walk(&[(0.0, over), (0.0, over + cell)], cell).is_none());
    }

    #[test]
    fn gate_walk_accepts_coordinate_at_the_magnitude_bound() {
        // A coordinate exactly AT `MAX_GATE_WALK_COORD` (not over it) must not be rejected by
        // the bound check itself — confirms the comparison is strictly `>`, not `>=`.
        let cell = 100.0_f64;
        let at = MAX_GATE_WALK_COORD;
        let walk = gate_walk(&[(at - cell, 0.0), (at, 0.0)], cell).unwrap();
        assert_eq!(walk.len(), 2);
    }

    #[test]
    fn gate_walk_on_empty_path_returns_empty() {
        let walk = gate_walk(&[], 100.0).unwrap();
        assert!(walk.is_empty());
    }
}
