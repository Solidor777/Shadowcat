//! Pure, lock-free per-path move executor (M1 server-authoritative movement; engine-agnostic
//! since M10f-2).
//!
//! `execute_move` is built on `gate_walk`, which subdivides ANY input polyline — grid A*
//! cell-center vertices ≤1 cell apart, or any-angle continuous vertices arbitrarily far apart —
//! into a dense walk where every consecutive pair is at most one cell apart (Chebyshev),
//! preserving already-≤1-cell segments as an identity. The per-step gate then runs over this
//! dense walk, validating each dense sub-step against the SAME predicate set
//! `pathfinding::cell_enterable` uses for routing (D4):
//! - a footprint-disc clearance test AND a center-to-center segment-crossing test against every
//!   `blocksMove` wall in the AUTHORITATIVE wall set (`ecs.move_walls(scene, None)`) — both are
//!   required; the disc test alone lets a wall between two adjacent cell centers become
//!   permeable at the default 0.4-cell footprint,
//! - the caller-supplied `visible` mask (M10e-4 gate — skipped for `Unrestricted`) over
//!   `footprint_cells ∪ line_traversal`, not the center point alone,
//! - the region field (M10g): impassable is footprint-gated (a wide body cannot fit past
//!   impassable terrain any more than a wall); arrest and terrain stay CENTER-CELL only, mirroring
//!   `cell_enterable`'s documented asymmetry (they act on the mover's own position, not solid
//!   geometry it must clear). Always reads the AUTHORITATIVE field (`ecs.region_field(scene,
//!   None)`) — this executor springs every region regardless of what the mover's own pathfind
//!   preview could see (spec §6).
//!
//! Returns the stop cell + the legal prefix render-path + accumulated cost. `truncated` is true
//! when the move stops before `path.last()` for any reason (wall, mask, region-impassable, or
//! region-arrest), including a region-arrest on the final path step.
//!
//! INVARIANT (spec §13 / I4 route-gate parity): step 2 calls `GridShape::line_traversal(prev,
//! next, cell)` (via `ecs.resolve_grid_shape`) and checks `all ∈ visible` over
//! `footprint_cells ∪ line_traversal` — square delegates to `movement::supercover_cells`, hex to
//! a ψ-crossing supercover, so the predicate agrees on every cell on BOTH grid kinds, not square
//! alone. This is the SAME predicate set `pathfinding::cell_enterable` uses when routing: on
//! `GridStepped`, route-admissible ⇔ gate-admissible for a non-GM mover (the parity tests in this
//! file pin the equivalence); on `Continuous` the two evaluate at different granularity (dense
//! `gate_walk` sampling vs the router's cell-center check), so only the weaker route ⊆
//! gate-allowed direction holds. `execute_move` is the sole per-cell traversal decision (I2) —
//! there is no separate gate to keep in sync. The caller pre-computes `visible` off the ECS read
//! lock, so this executor is pure and imposes no lock ordering.
//!
//! Coupling: `token_position` is the ECS committed-position seam; any rename
//! must update both this caller and `token_move` in `scene/mod.rs`.

use std::collections::BTreeSet;

use uuid::Uuid;

use crate::scene::{MovementRestriction, SceneEcs};

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
    /// Sample position, scene units (at most one cell from its predecessor).
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
    /// `gate_walk` returned `None`: either the path's dense walk (§4.3: arc-length/sample-count,
    /// not authored-vertex-count) would exceed `MAX_GATE_WALK_SAMPLES` — the DoS bound, replacing
    /// the pre-M10f-2 authored-vertex cap since a single arbitrarily-long continuous segment is
    /// now the relevant DoS surface, not the number of authored waypoints — or a path coordinate's
    /// magnitude exceeds `MAX_GATE_WALK_COORD` (a distinct, unreachable-in-practice degenerate
    /// case sharing this variant rather than `Degenerate`, since both originate from the same
    /// `gate_walk` fail-closed `None`).
    TooLong,
    /// A structural invariant was violated: non-finite coords, or `path[0]` not at the
    /// token's committed position. (Pre-M10f-2 this variant also covered a non-adjacent
    /// king-step jump; that case is now subdivided-and-gated instead of rejected — see
    /// `gate_walk`, §4.2.)
    Degenerate,
    /// The token's scene has no document — refuse rather than synthesize a grid.
    SceneUnknown,
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
/// # Parity with `pathfinding::cell_enterable` — per-cell decision only
///
/// The per-cell decision (step 1 + step 2) uses the SAME primitives as the router's
/// `cell_enterable`: wall-segment crossing (`segments_cross`, the primitive `blocks_move`
/// wraps), footprint-disc clearance, `GridShape::line_traversal` (a supercover on both grid
/// kinds — square cell-walk, hex ψ-crossing, per `ecs.resolve_grid_shape`), and the
/// pre-computed `visible` set over `footprint_cells ∪ line_traversal`. There is no third gate
/// this executor and the router must independently agree with — `execute_move` is the sole
/// per-cell traversal decision (I2) — so this parity is pinned directly between the two, not
/// mediated through a shared middle gate. On `GridStepped` the two are equivalent
/// (route-admissible ⇔ gate-admissible) for a non-GM mover; on `Continuous` only the weaker
/// route ⊆ gate-allowed direction holds, since `gate_walk`'s dense sampling and the router's
/// cell-center evaluation operate at different granularity. For a grid input this executor is
/// byte-identical in outcome to
/// the pre-M10f-2 king-step executor (proved via a differential oracle during M10f-2 and now
/// frozen as literal fixtures — see
/// `frozen_parity_king_step_paths_match_previously_oracle_verified_outcomes`).
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
/// - `is_gm` — When true, every gameplay gate (walls, mask, impassable, arrest) is bypassed
///   (M9 §5's "ignore walls" override, matching `publish`'s own GM position write). Resource
///   guards (`gate_walk`'s `MAX_GATE_WALK_COORD`/`MAX_GATE_WALK_SAMPLES`, the non-finite and
///   scene-existence refusals, and the `footprint_radius_cells` range guard below) are never
///   exempted. Terrain cost still accrues regardless.
/// - `footprint_radius_cells` — The mover's bounding-disc radius in grid cells (see
///   `SceneEcs::resolve_token_footprint`). Must be in `[0, pathfinding::MAX_FOOTPRINT_CELLS]`;
///   out-of-range (including NaN) is `MoveReject::Degenerate`, unconditionally — a GM's wider
///   gameplay exemption does not cover this input guard (I1).
// A plain `is_gm` flag with early-outs, not a shared profile struct: after D9 there is exactly
// one gate, and a shared symbol with a single consumer is indirection with no second party to
// keep honest.
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_move(
    ecs: &SceneEcs,
    scene: Uuid,
    token: Uuid,
    path: &[(f64, f64)],
    restriction: MovementRestriction,
    visible: &BTreeSet<(i32, i32)>,
    cell: f64,
    is_gm: bool,
    footprint_radius_cells: f64,
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
    // I1: a resource/admissibility guard, never exempted for a GM. `contains` rejects NaN and
    // ±Inf (NaN comparisons are always false; Inf > MAX_FOOTPRINT_CELLS).
    if !(0.0..=crate::scene::pathfinding::MAX_FOOTPRINT_CELLS).contains(&footprint_radius_cells) {
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

    // Gameplay gates apply to non-GMs only. A GM may make an illegal move: they move with or
    // without pathfinding, and a placement lands where asked (M9 §5), matching `publish`'s GM
    // position write. Resource guards — `gate_walk`'s MAX_GATE_WALK_COORD / MAX_GATE_WALK_SAMPLES,
    // non-finite refusal, and the scene-existence refusal — are NOT exempted for a GM (I1).
    let check_walls = !is_gm;
    let check_regions = !is_gm;
    let check_mask = !is_gm && !matches!(restriction, MovementRestriction::Unrestricted);

    // Authoritative region field (M10g): always the full field, never filtered — this
    // executor springs secret regions regardless of what the mover's pathfind preview
    // could see (§6).
    let Some(regions) = ecs.region_field(scene, None) else {
        return Err(MoveReject::SceneUnknown);
    };

    // The mask gate's segment-crossing set is engine-agnostic geometry (`GridShape::
    // line_traversal`), routed through the scene's own resolved grid shape (square or hex) rather
    // than a hardcoded square-grid call.
    let grid = ecs.resolve_grid_shape(scene, cell);

    // The executor always reads the AUTHORITATIVE wall set: a `gm_only` wall omitted from the
    // requester's route springs here, exactly as a secret region does (D10, I3).
    let gate_walls = ecs.move_walls(scene, None);

    // Constant for the whole walk: the footprint disc radius in scene units, mirroring
    // `cell_enterable`'s `r_scene` (`pathfinding.rs:88`).
    let r_scene = footprint_radius_cells.max(0.0) * cell;

    // Region-cell lookup goes through the SAME resolved grid shape as rasterization
    // (`region_field`) and the mask's `line_traversal` — `grid.cell_of` is the hex-correct point→
    // cell mapping (square: `floor(p/cell)`; hex: pixel→axial round). A hardcoded square `floor`
    // here would test hex moves against square-indexed cells while `region_field` rasterized onto
    // axial cells — two incompatible coordinate systems on a hex scene.
    let to_cell = |p: (f64, f64)| -> (i32, i32) { grid.cell_of(p) };

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
        let next_cell = to_cell(next);
        // The footprint disc's anchor for every CELL-MEMBERSHIP test below (mask, impassable):
        // the cell's own center, exactly mirroring `cell_enterable`'s `ctr = cell_center(to)`
        // (`pathfinding.rs:89`). This is NOT optional polish — anchoring at the raw dense-walk
        // point `next` instead is degenerate whenever `next` lands exactly on a cell boundary
        // (common: any axis-aligned continuous step subdivides into boundary-exact samples),
        // where `footprint_cells`'s zero-distance-to-AABB test spuriously admits every cell
        // touching that corner, not just the cells the footprint actually occupies.
        let fp_ctr = grid.cell_center(next_cell);

        // Step 1: wall gate — every dense sub-segment, exempt for a GM (M9 §5). TWO checks,
        // both from `cell_enterable` (`pathfinding.rs:92-97,131-136`): the footprint disc at
        // `next` must clear every wall, AND the center-to-center step segment must cross none.
        // The disc alone is insufficient — at a 0.4-cell footprint a wall midway between
        // adjacent cell centers sits 0.5 cell away and would pass, making walls permeable on
        // the sole remaining check.
        if check_walls {
            let disc_blocked = gate_walls
                .iter()
                .any(|w| crate::scene::vision::point_segment_distance(next, w.a, w.b) < r_scene);
            let crossed = gate_walls
                .iter()
                .any(|w| crate::scene::segments_cross(prev, next, w.a, w.b));
            if disc_blocked || crossed {
                stopped_early = true;
                break;
            }
        }

        // Step 2: vision-mask gate — every dense sub-segment, over the FOOTPRINT, not the
        // center — the same `footprint_cells ∪ line_traversal` union `cell_enterable` requires
        // (`pathfinding.rs:109-123`). This density is exactly why gate_walk exists: line_traversal
        // is well-defined and dense enough to cover the swept footprint for an any-angle
        // segment, not just a king step.
        if check_mask {
            let Some(mut cells) = grid.line_traversal(prev, next, cell) else {
                stopped_early = true;
                break;
            };
            cells.extend(grid.footprint_cells(next_cell, fp_ctr, r_scene, cell));
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
        //
        // This transition-dedup relies on the router never emitting two consecutive dense
        // samples that map to the SAME cell: true for grid A* (`pathfinding::find`) and true
        // for `gate_walk`'s output here, since it only ever emits progressing samples along the
        // input polyline (no stationary/duplicate cell re-visits). Unlike the pre-M10f-2
        // king-step executor, this is no longer independently enforced by an adjacency guard —
        // that guard used to reject a non-adjacent jump outright; this executor now subdivides
        // instead of rejecting (see `gate_walk`), so a duplicate-cell transition would silently
        // fall through this dedup rather than being caught by a separate check.
        if next_cell != last_region_cell {
            // Impassable IS footprint-gated (`cell_enterable`'s check 4, `pathfinding.rs:139-145`):
            // a wide body cannot fit past impassable terrain any more than past a wall.
            if check_regions {
                let fp_cells = grid.footprint_cells(next_cell, fp_ctr, r_scene, cell);
                if fp_cells.iter().any(|c| regions.is_impassable(*c)) {
                    stopped_early = true;
                    break;
                }
            }
            // Cost accrues regardless of the exemption: it is information, not a gate.
            cost += regions.terrain_multiplier(next_cell);
            // Arrest and terrain stay CENTER-CELL only, mirroring `cell_enterable`'s documented
            // asymmetry (`pathfinding.rs:133-136`): they act on the mover's own position rather
            // than solid geometry it must clear. Footprint-gating arrest here would make the gate
            // stricter than the router and break I4.
            if check_regions && regions.is_arrest(next_cell) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::MovementModel;
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

    /// Builds a scene-entity fixture with `engine` set to `body` (`system` stays `{}`) — every
    /// reader this file's tests exercise (`scene_grid_sizes`, `blocks_move`, `region_field`) is a
    /// typed `engine` read; a `token` fixture built through this helper carries no position data
    /// these tests consume (`execute_move` takes an explicit `path`, never the token doc's
    /// position), so re-rooting it here alongside scene/wall/region is harmless.
    fn entity_doc(
        id: u128,
        parent: u128,
        ty: &str,
        body: serde_json::Value,
    ) -> crate::data::document::Document {
        let mut d = doc(id, Some(parent), ty);
        d.engine = Some(body);
        d
    }

    /// Scene with a token at (0,0), no walls, cell=100.
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
            false,
            0.4,
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
            false,
            0.4,
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
            false,
            0.4,
        )
        .unwrap();
        assert_eq!(out.stop, (100.0, 0.0));
        assert!(out.truncated);
    }

    /// Pins the mask gate's behavior once its `supercover_cells` call routes through
    /// `SquareGrid::line_traversal` instead of the free function directly. A pure diagonal
    /// king-step from a lattice corner (`(0,0)->(100,100)`, cell=100) crosses the shared
    /// corner exactly, so the mask check must see all 4 cells the corner-crossing branch
    /// emits: the two diagonal cells AND both off-diagonal flankers (mirrors
    /// `movement::pure_diagonal_through_corner_includes_both_flanking_cells`). Excluding the
    /// flanker `(1,0)` from the mask must still truncate the move at the start cell.
    #[test]
    fn gate_walk_mask_gate_routes_through_grid_shape_not_hardcoded_supercover() {
        let (ecs, scene, token) = clear_scene();
        let mut visible: BTreeSet<(i32, i32)> = BTreeSet::new();
        visible.insert((0, 0));
        visible.insert((0, 1));
        visible.insert((1, 1));
        // (1,0) deliberately excluded — the corner-crossing flanker cell.
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(0.0, 0.0), (200.0, 200.0)],
            MovementRestriction::Visible,
            &visible,
            100.0,
            false,
            0.4,
        )
        .unwrap();
        assert_eq!(out.stop, (0.0, 0.0));
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
            false,
            0.4,
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
            false,
            0.4,
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
            false,
            0.4,
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
                100.0,
                false,
                0.4,
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
            false,
            0.4,
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
            false,
            0.4,
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
                false,
                0.4,
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
                100.0,
                false,
                0.4,
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
                100.0,
                false,
                0.4,
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
            false,
            0.4,
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
        rect: (f64, f64, f64, f64),
    ) -> crate::data::document::Document {
        let (x0, y0, x1, y1) = rect;
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
                region_doc(12, 10, "impassable", 1.0, (50.0, 0.0, 150.0, 100.0)),
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
            false,
            0.4,
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
                region_doc(12, 10, "arrest", 1.0, (50.0, -50.0, 150.0, 50.0)),
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
            false,
            0.4,
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
                region_doc(12, 10, "terrain", 2.5, (50.0, 0.0, 150.0, 100.0)),
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
            false,
            0.4,
        )
        .unwrap();
        assert!((out.cost - 2.5).abs() < 1e-9);
    }

    #[test]
    fn impassable_hex_region_stops_a_hex_move_at_the_correct_hex_cell() {
        use crate::scene::grid_shape::{GridShape, HexGrid};
        // Hex scene (size=50): rasterize's candidate enumeration (`GridShape::cells_in_bounds`) and
        // move_exec's region lookup (`grid.cell_of`) must both resolve hex (axial) cells. A rect
        // enclosing ONLY hex cell (1,0)'s center must (a) rasterize onto hex (1,0), and (b) stop a
        // move from (0,0) toward (1,0) before entry. A square-on-hex enumeration or lookup would
        // rasterize onto the wrong axial cell and the move would sail straight through.
        let hex = HexGrid { size: 50.0 };
        let c10 = hex.cell_center((1, 0)); // (50·√3, 0) ≈ (86.6, 0)

        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let ecs = SceneEcs::from_documents(
            vec![
                entity_doc(
                    10,
                    0,
                    "scene",
                    json!({ "grid": { "kind": "hex", "size": 50.0 }, "background": null }),
                ),
                entity_doc(
                    11,
                    10,
                    "token",
                    json!({ "x": 0.0, "y": 0.0, "w": 50.0, "h": 50.0, "rotation": 0.0 }),
                ),
                // Rect around hex (1,0)'s center only — excludes (0,0) at x=0, (2,0) at x≈173.2, and
                // every neighbor of (1,0) (their centers sit at |y|≈75, outside [-25,25]).
                region_doc(12, 10, "impassable", 1.0, (61.0, -25.0, 112.0, 25.0)),
            ],
            0,
        );

        // (a) The rect rasterizes onto hex cell (1,0) via GridShape, not a square index.
        let field = ecs.region_field(scene_id, None).expect("scene exists");
        assert!(
            field.is_impassable((1, 0)),
            "rect rasterizes onto hex cell (1,0)"
        );
        assert!(
            !field.is_impassable((0, 0)),
            "hex (0,0) is outside the rect"
        );

        // (b) The move stops before entering hex (1,0). Unrestricted → the vision mask is skipped.
        let visible = BTreeSet::new();
        let out = execute_move(
            &ecs,
            scene_id,
            token_id,
            &[(0.0, 0.0), c10],
            MovementRestriction::Unrestricted,
            &visible,
            50.0,
            false,
            0.4,
        )
        .unwrap();
        assert!(out.truncated, "an impassable hex region truncates the move");
        assert_eq!(
            hex.cell_of(out.stop),
            (0, 0),
            "the move never enters impassable hex (1,0); it halts in hex (0,0)"
        );
    }

    #[test]
    fn arrest_hex_region_stops_at_entry_composed_with_a_hex_visibility_mask() {
        use crate::scene::grid_shape::{GridShape, HexGrid};
        // Composes the two hex-indexed gates the executor runs per cell-entry — the `Visible` mask
        // gate and the region gate — on ONE hex scene (size=50), and pins both against the
        // square-indexed math that would be used if either lookup regressed.
        //
        // Geometry: a rect enclosing ONLY hex (2,0)'s center (173.2, 0) marks it `arrest`. The
        // square index of that same point is (3,0), so a square `floor(p/cell)` region lookup finds
        // no arrest there and the move sails through. The mask is the exact hex traversal
        // {(0,0),(1,0),(2,0),(3,0)}; the square supercover of the same segment reaches (5,0), so a
        // square-indexed mask gate would instead truncate early, for the wrong reason.
        let hex = HexGrid { size: 50.0 };
        let c30 = hex.cell_center((3, 0)); // (150·√3, 0) ≈ (259.8, 0)

        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let ecs = SceneEcs::from_documents(
            vec![
                entity_doc(
                    10,
                    0,
                    "scene",
                    json!({ "grid": { "kind": "hex", "size": 50.0 }, "background": null }),
                ),
                entity_doc(
                    11,
                    10,
                    "token",
                    json!({ "x": 0.0, "y": 0.0, "w": 50.0, "h": 50.0, "rotation": 0.0 }),
                ),
                region_doc(12, 10, "arrest", 1.0, (148.0, -25.0, 198.0, 25.0)),
            ],
            0,
        );

        let field = ecs.region_field(scene_id, None).expect("scene exists");
        assert!(
            field.is_arrest((2, 0)),
            "rect rasterizes onto hex cell (2,0)"
        );
        assert!(
            !field.is_arrest((3, 0)),
            "the SQUARE index of the arrest rect's location carries no arrest — a square-indexed \
             region lookup would miss it entirely"
        );

        let visible: BTreeSet<(i32, i32)> = [(0, 0), (1, 0), (2, 0), (3, 0)].into_iter().collect();
        let square_cells = crate::scene::movement::supercover_cells((0.0, 0.0), c30, 50.0)
            .expect("bounded square supercover");
        assert!(
            square_cells.iter().any(|c| !visible.contains(c)),
            "a square-indexed mask gate would truncate this move for the wrong reason: {square_cells:?}"
        );

        let out = execute_move(
            &ecs,
            scene_id,
            token_id,
            &[(0.0, 0.0), c30],
            MovementRestriction::Visible,
            &visible,
            50.0,
            false,
            0.4,
        )
        .unwrap();
        assert!(out.truncated, "the arrest hex cell truncates the move");
        assert_eq!(
            hex.cell_of(out.stop),
            (2, 0),
            "arrest stops AT entry into hex (2,0), never before it and never past it"
        );
        // Two cell entries accrue (hex (1,0) then the arrest cell (2,0)); no terrain weighting.
        assert!(
            (out.cost - 2.0).abs() < 1e-9,
            "cost {} accrues per hex cell entry",
            out.cost
        );
    }

    #[test]
    fn authoritative_field_springs_a_secret_region_a_player_was_routed_through() {
        // A gm_only impassable region: move_exec must still enforce it (it always uses the
        // authoritative field, spec §6), even though a player's pathfind field never saw it.
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let mut secret = region_doc(12, 10, "impassable", 1.0, (50.0, 0.0, 150.0, 100.0));
        secret
            .permissions
            .property_overrides
            .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
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
            false,
            0.4,
        )
        .unwrap();
        assert_eq!(
            out.stop,
            (0.0, 0.0),
            "authoritative field springs the secret impassable region"
        );
    }

    // -----------------------------------------------------------------------
    // GM gate-exemption tests (D8): a GM bypasses every gameplay gate (walls, mask,
    // impassable, arrest) but no resource guard, and terrain cost still accrues.
    // -----------------------------------------------------------------------

    /// Empty vision mask — irrelevant to every test below since `Unrestricted` skips it.
    fn empty_mask() -> BTreeSet<(i32, i32)> {
        BTreeSet::new()
    }

    /// Token committed at (50,50); a `blocksMove` wall crosses the path's first step
    /// (50,50)->(150,50) at x=100, spanning y∈[0,100].
    fn scene_with_wall_across_the_path() -> (SceneEcs, Uuid, Uuid) {
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
                    json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
                ),
                entity_doc(
                    12,
                    10,
                    "wall",
                    json!({
                        "seg": { "x1": 100, "y1": 0, "x2": 100, "y2": 100 },
                        "blocksMove": true
                    }),
                ),
            ],
            0,
        );
        (ecs, scene_id, token_id)
    }

    /// Token committed at (50,50); an impassable region covers [100,200)x[0,100) and an
    /// arrest region covers [200,300)x[0,100), so a straight path through both crosses the
    /// impassable cell first, then the arrest cell.
    fn scene_with_impassable_then_arrest_region() -> (SceneEcs, Uuid, Uuid) {
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
                    json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
                ),
                region_doc(12, 10, "impassable", 1.0, (100.0, 0.0, 200.0, 100.0)),
                region_doc(13, 10, "arrest", 1.0, (200.0, 0.0, 300.0, 100.0)),
            ],
            0,
        );
        (ecs, scene_id, token_id)
    }

    /// Token committed at (50,50); a terrain region with multiplier 3 covers the cell entered
    /// by the step (50,50)->(150,50).
    fn scene_with_terrain_multiplier_3() -> (SceneEcs, Uuid, Uuid) {
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
                    json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
                ),
                region_doc(12, 10, "terrain", 3.0, (100.0, 0.0, 200.0, 100.0)),
            ],
            0,
        );
        (ecs, scene_id, token_id)
    }

    #[test]
    fn gm_move_crosses_a_blocks_move_wall_untruncated() {
        let (ecs, scene, token) = scene_with_wall_across_the_path();
        let path = [(50.0, 50.0), (150.0, 50.0), (250.0, 50.0)];
        let out = execute_move(
            &ecs,
            scene,
            token,
            &path,
            MovementRestriction::Unrestricted,
            &empty_mask(),
            100.0,
            true,
            0.4,
        )
        .expect("a GM move is admissible");
        assert!(
            !out.truncated,
            "a GM move is not truncated by a wall (M9 §5)"
        );
        assert_eq!(
            out.render_path.last().copied(),
            Some((250.0, 50.0)),
            "the GM lands at the requested destination"
        );
    }

    #[test]
    fn gm_move_ignores_impassable_and_arrest_regions() {
        let (ecs, scene, token) = scene_with_impassable_then_arrest_region();
        let path = [(50.0, 50.0), (150.0, 50.0), (250.0, 50.0)];
        let out = execute_move(
            &ecs,
            scene,
            token,
            &path,
            MovementRestriction::Unrestricted,
            &empty_mask(),
            100.0,
            true,
            0.4,
        )
        .expect("admissible");
        assert!(!out.truncated, "neither impassable nor arrest stops a GM");
    }

    #[test]
    fn non_gm_move_is_still_blocked_by_the_same_wall() {
        // The exemption must not widen: pin that non-GM behavior is unchanged.
        let (ecs, scene, token) = scene_with_wall_across_the_path();
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(50.0, 50.0), (150.0, 50.0)],
            MovementRestriction::Unrestricted,
            &empty_mask(),
            100.0,
            false,
            0.4,
        )
        .expect("admissible");
        assert!(out.truncated, "a non-GM is still stopped by the wall");
    }

    #[test]
    fn gm_move_is_still_refused_beyond_the_coordinate_bound() {
        // I1: a GM bypasses gameplay gates and NO resource guard.
        let (ecs, scene, token) = scene_with_wall_across_the_path();
        let over = MAX_GATE_WALK_COORD + 1.0;
        let err = execute_move(
            &ecs,
            scene,
            token,
            &[(50.0, 50.0), (over, 50.0)],
            MovementRestriction::Unrestricted,
            &empty_mask(),
            100.0,
            true,
            0.4,
        )
        .expect_err("a resource guard is never exempted");
        assert!(matches!(err, MoveReject::TooLong), "got {err:?}");
    }

    #[test]
    fn gm_move_still_accrues_terrain_cost() {
        // Cost is information, not a gate — accrual is independent of the exemption.
        let (ecs, scene, token) = scene_with_terrain_multiplier_3();
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(50.0, 50.0), (150.0, 50.0)],
            MovementRestriction::Unrestricted,
            &empty_mask(),
            100.0,
            true,
            0.4,
        )
        .expect("admissible");
        assert!(
            out.cost >= 3.0,
            "terrain still accrues for a GM, got {}",
            out.cost
        );
    }

    // -----------------------------------------------------------------------
    // Continuous (any-angle, non-king-step) unit tests
    //
    // These paths are not king-step, so the frozen oracle rejects them by shape and no
    // differential comparison is possible — pure behavioral tests against `execute_move` only.
    // -----------------------------------------------------------------------

    #[test]
    fn continuous_any_angle_path_reaches_goal_when_fully_visible() {
        let (ecs, scene, token) = clear_scene();
        let visible = visible_grid(4);
        // Any-angle single segment, not axis-aligned, not diagonal-45.
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(0.0, 0.0), (350.0, 120.0)],
            MovementRestriction::Visible,
            &visible,
            100.0,
            false,
            0.4,
        )
        .unwrap();
        assert_eq!(out.stop, (350.0, 120.0));
        assert!(!out.truncated);
        assert_eq!(out.render_path, vec![(0.0, 0.0), (350.0, 120.0)]);
    }

    #[test]
    fn continuous_path_truncates_at_a_wall_crossed_mid_segment() {
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        // Vertical wall at x=250, spanning y in [-50,50] — crosses a horizontal move at y=0.
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
                entity_doc(
                    12,
                    10,
                    "wall",
                    json!({
                        "seg": { "x1": 250, "y1": -50, "x2": 250, "y2": 50 },
                        "blocksMove": true
                    }),
                ),
            ],
            0,
        );
        let visible = visible_grid(5);
        // Single authored segment far longer than 1 cell — subdivided by gate_walk into 4
        // dense substeps of 100 units each; the wall sits inside the third substep.
        let out = execute_move(
            &ecs,
            scene_id,
            token_id,
            &[(0.0, 0.0), (400.0, 0.0)],
            MovementRestriction::Unrestricted,
            &visible,
            100.0,
            false,
            0.4,
        )
        .unwrap();
        assert!(
            out.truncated,
            "must stop before crossing the wall mid-segment"
        );
        assert_eq!(
            out.stop,
            (200.0, 0.0),
            "stops at the last dense sample before the wall crossing"
        );
    }

    #[test]
    fn continuous_path_stops_before_entering_an_impassable_region_mid_segment() {
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
                region_doc(12, 10, "impassable", 1.0, (300.0, -50.0, 500.0, 150.0)),
            ],
            0,
        );
        let visible = visible_grid(5);
        let out = execute_move(
            &ecs,
            scene_id,
            token_id,
            &[(0.0, 0.0), (400.0, 0.0)],
            MovementRestriction::Unrestricted,
            &visible,
            100.0,
            false,
            0.4,
        )
        .unwrap();
        assert!(out.truncated);
        assert_eq!(
            out.stop,
            (200.0, 0.0),
            "stops BEFORE entering impassable cell (3,0) [x=300..400)"
        );
    }

    #[test]
    fn continuous_path_arrest_stops_at_entry_mid_segment_not_before() {
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
                region_doc(12, 10, "arrest", 1.0, (300.0, -50.0, 500.0, 150.0)),
            ],
            0,
        );
        let visible = visible_grid(5);
        let out = execute_move(
            &ecs,
            scene_id,
            token_id,
            &[(0.0, 0.0), (400.0, 0.0)],
            MovementRestriction::Unrestricted,
            &visible,
            100.0,
            false,
            0.4,
        )
        .unwrap();
        assert!(out.truncated);
        assert_eq!(
            out.stop,
            (300.0, 0.0),
            "arrest stops AT entry into cell (3,0), not before it"
        );
    }

    #[test]
    fn execute_move_handles_an_any_angle_weighted_continuous_polyline() {
        // A continuous (any-angle) route whose vertices are > 1 cell apart, crossing a terrain
        // cell (mult 3) and stopping at an arrest cell. Proves gate_walk gates + accrues
        // terrain cost + arrests on a continuous polyline with no executor change.
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
                    json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
                ),
                region_doc(12, 10, "terrain", 3.0, (100.0, 0.0, 200.0, 100.0)),
                region_doc(13, 10, "arrest", 1.0, (300.0, 0.0, 400.0, 100.0)),
            ],
            0,
        );
        let visible = visible_grid(6);
        // Any-angle polyline: (50,50) -> (250,50) -> (350,50); the first leg is 2 cells in one hop.
        let out = execute_move(
            &ecs,
            scene_id,
            token_id,
            &[(50.0, 50.0), (250.0, 50.0), (350.0, 50.0)],
            MovementRestriction::Unrestricted,
            &visible,
            100.0,
            false,
            0.4,
        )
        .expect("executor handles the any-angle weighted polyline");
        assert!(out.truncated, "arrest cell (3,0) truncates the move");
        // Terrain cell (1,0) mult 3 was entered once before the arrest; cost reflects the multiplier.
        assert!(
            out.cost >= 3.0,
            "terrain multiplier accrued, got {}",
            out.cost
        );
    }

    // -----------------------------------------------------------------------
    // Differential parity test suite vs the frozen king-step oracle
    // -----------------------------------------------------------------------

    /// Builds a scene with an optional wall and/or region for differential-test scenarios.
    /// `wall`: `Some((x1,y1,x2,y2))` adds a `blocksMove` wall segment.
    /// `region`: `Some((behavior, cost, x0,y0,x1,y1))` adds a rect region.
    /// `secret_region`: when true and `region` is `Some`, marks the region `gm_only`.
    fn scene_with_wall_and_region(
        wall: Option<(f64, f64, f64, f64)>,
        region: Option<(&str, f64, f64, f64, f64, f64)>,
        secret_region: bool,
    ) -> (SceneEcs, Uuid, Uuid) {
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let mut docs = vec![
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
        ];
        if let Some((x1, y1, x2, y2)) = wall {
            docs.push(entity_doc(
                12,
                10,
                "wall",
                json!({
                    "seg": { "x1": x1, "y1": y1, "x2": x2, "y2": y2 },
                    "blocksMove": true
                }),
            ));
        }
        if let Some((behavior, cost, x0, y0, x1, y1)) = region {
            let mut r = region_doc(13, 10, behavior, cost, (x0, y0, x1, y1));
            if secret_region {
                r.permissions
                    .property_overrides
                    .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
            }
            docs.push(r);
        }
        (SceneEcs::from_documents(docs, 0), scene_id, token_id)
    }

    struct ExpectedOutcome {
        stop: (f64, f64),
        render_path: Vec<(f64, f64)>,
        truncated: bool,
        cost: f64,
    }

    struct FrozenCase {
        label: &'static str,
        wall: Option<(f64, f64, f64, f64)>,
        region: Option<(&'static str, f64, f64, f64, f64, f64)>,
        secret_region: bool,
        visible: BTreeSet<(i32, i32)>,
        restriction: MovementRestriction,
        path: Vec<(f64, f64)>,
        expected: ExpectedOutcome,
    }

    /// Frozen parity fixtures (M10f-2): 10 grid-input scenarios whose expected outcomes were
    /// independently derived and cross-checked live against the pre-M10f-2 king-step oracle
    /// during this task, before the oracle was deleted. This is the permanent parity
    /// regression that proves `execute_move` still agrees with the pre-refactor executor on
    /// every grid input, now that no oracle remains to compare against at runtime.
    #[test]
    fn frozen_parity_king_step_paths_match_previously_oracle_verified_outcomes() {
        let cases = vec![
            FrozenCase {
                label: "clear scene, full visible, straight path",
                wall: None,
                region: None,
                secret_region: false,
                visible: visible_grid(3),
                restriction: MovementRestriction::Visible,
                path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
                expected: ExpectedOutcome {
                    stop: (100.0, 100.0),
                    render_path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
                    truncated: false,
                    cost: 2.0,
                },
            },
            FrozenCase {
                label: "clear scene, Unrestricted, empty mask",
                wall: None,
                region: None,
                secret_region: false,
                visible: BTreeSet::new(),
                restriction: MovementRestriction::Unrestricted,
                path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
                expected: ExpectedOutcome {
                    stop: (100.0, 100.0),
                    render_path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
                    truncated: false,
                    cost: 2.0,
                },
            },
            FrozenCase {
                label: "wall blocks second step, Visible",
                wall: Some((50.0, 50.0, 150.0, 50.0)),
                region: None,
                secret_region: false,
                visible: visible_grid(4),
                restriction: MovementRestriction::Visible,
                path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
                expected: ExpectedOutcome {
                    stop: (100.0, 0.0),
                    render_path: vec![(0.0, 0.0), (100.0, 0.0)],
                    truncated: true,
                    cost: 1.0,
                },
            },
            FrozenCase {
                label: "partial mask truncates at unseen cell, Visible",
                wall: None,
                region: None,
                secret_region: false,
                visible: {
                    let mut v = BTreeSet::new();
                    v.insert((0, 0));
                    v.insert((1, 0));
                    v
                },
                restriction: MovementRestriction::Visible,
                path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
                expected: ExpectedOutcome {
                    stop: (100.0, 0.0),
                    render_path: vec![(0.0, 0.0), (100.0, 0.0)],
                    truncated: true,
                    cost: 1.0,
                },
            },
            FrozenCase {
                label: "full mask allowed under Revealed",
                wall: None,
                region: None,
                secret_region: false,
                visible: visible_grid(3),
                restriction: MovementRestriction::Revealed,
                path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
                expected: ExpectedOutcome {
                    stop: (100.0, 100.0),
                    render_path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
                    truncated: false,
                    cost: 2.0,
                },
            },
            FrozenCase {
                label: "impassable region stops before entry",
                wall: None,
                region: Some(("impassable", 1.0, 50.0, 0.0, 150.0, 100.0)),
                secret_region: false,
                visible: visible_grid(3),
                restriction: MovementRestriction::Unrestricted,
                path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
                expected: ExpectedOutcome {
                    stop: (0.0, 0.0),
                    render_path: vec![(0.0, 0.0)],
                    truncated: true,
                    cost: 0.0,
                },
            },
            FrozenCase {
                label: "arrest region stops at entry on final step",
                wall: None,
                region: Some(("arrest", 1.0, 50.0, -50.0, 150.0, 50.0)),
                secret_region: false,
                visible: visible_grid(3),
                restriction: MovementRestriction::Unrestricted,
                path: vec![(0.0, 0.0), (100.0, 0.0)],
                expected: ExpectedOutcome {
                    stop: (100.0, 0.0),
                    render_path: vec![(0.0, 0.0), (100.0, 0.0)],
                    truncated: true,
                    cost: 1.0,
                },
            },
            FrozenCase {
                label: "terrain region accrues cost",
                wall: None,
                region: Some(("terrain", 2.5, 50.0, 0.0, 150.0, 100.0)),
                secret_region: false,
                visible: visible_grid(3),
                restriction: MovementRestriction::Unrestricted,
                path: vec![(0.0, 0.0), (100.0, 0.0)],
                expected: ExpectedOutcome {
                    stop: (100.0, 0.0),
                    render_path: vec![(0.0, 0.0), (100.0, 0.0)],
                    truncated: false,
                    cost: 2.5,
                },
            },
            FrozenCase {
                label: "authoritative field springs a secret impassable region under Visible",
                wall: None,
                region: Some(("impassable", 1.0, 50.0, 0.0, 150.0, 100.0)),
                secret_region: true,
                visible: visible_grid(3),
                restriction: MovementRestriction::Visible,
                path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
                expected: ExpectedOutcome {
                    stop: (0.0, 0.0),
                    render_path: vec![(0.0, 0.0)],
                    truncated: true,
                    cost: 0.0,
                },
            },
            FrozenCase {
                // The (200,200)->(300,100) leg has both endpoints exactly on 4-way grid-line
                // intersections. `movement::supercover_cells`'s corner-crossing branch used to
                // spuriously fail-closed on this shape (docs/CLOSED_BUGS.md) — fixed by gating
                // the diagonal corner-step on a per-axis remaining-step budget so a tMax tie
                // that merely coincides with an axis already at its target can no longer drift
                // the traversal past (ei,ej). This is now the CORRECT (non-truncated) outcome.
                label: "diagonal 3-step king path, full visible",
                wall: None,
                region: None,
                secret_region: false,
                visible: visible_grid(4),
                restriction: MovementRestriction::Visible,
                path: vec![(0.0, 0.0), (100.0, 100.0), (200.0, 200.0), (300.0, 100.0)],
                expected: ExpectedOutcome {
                    stop: (300.0, 100.0),
                    render_path: vec![(0.0, 0.0), (100.0, 100.0), (200.0, 200.0), (300.0, 100.0)],
                    truncated: false,
                    cost: 3.0,
                },
            },
        ];

        for case in &cases {
            let (ecs, scene, token) =
                scene_with_wall_and_region(case.wall, case.region, case.secret_region);
            let result = execute_move(
                &ecs,
                scene,
                token,
                &case.path,
                case.restriction,
                &case.visible,
                100.0,
                false,
                0.4,
            );
            let actual =
                result.unwrap_or_else(|e| panic!("{}: expected Ok, got Err({e:?})", case.label));
            assert_eq!(actual.stop, case.expected.stop, "{}: stop", case.label);
            assert_eq!(
                actual.render_path, case.expected.render_path,
                "{}: render_path",
                case.label
            );
            assert_eq!(
                actual.truncated, case.expected.truncated,
                "{}: truncated",
                case.label
            );
            assert!(
                (actual.cost - case.expected.cost).abs() < 1e-9,
                "{}: cost mismatch ({} vs {})",
                case.label,
                actual.cost,
                case.expected.cost
            );
        }
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

    // -----------------------------------------------------------------------
    // Hex-scene integration coverage: proves the fully-wired hex path (walls + the
    // visibility mask) behaves correctly end-to-end through `execute_move`, mirroring this
    // module's square-scene wall/mask tests above.
    // -----------------------------------------------------------------------

    /// Pointy-top axial hex center, matching `grid_shape::HexGrid`'s own formula exactly
    /// (size=100, Red Blob Games convention).
    fn hex_cell_center(q: i32, r: i32) -> (f64, f64) {
        let size = 100.0;
        let x = size * (3.0_f64.sqrt() * q as f64 + 3.0_f64.sqrt() / 2.0 * r as f64);
        let y = size * (1.5 * r as f64);
        (x, y)
    }

    /// Hex scene (`grid.kind: "hex"`, size=100) with a token at axial (0,0), no walls.
    fn hex_clear_scene() -> (SceneEcs, Uuid, Uuid) {
        let scene_id = Uuid::from_u128(20);
        let token_id = Uuid::from_u128(21);
        let ecs = SceneEcs::from_documents(
            vec![
                entity_doc(
                    20,
                    0,
                    "scene",
                    json!({ "grid": { "kind": "hex", "size": 100 }, "background": null }),
                ),
                entity_doc(
                    21,
                    20,
                    "token",
                    json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
                ),
            ],
            0,
        );
        (ecs, scene_id, token_id)
    }

    /// Hex scene with a vertical `blocksMove` wall at `wall_x`.
    fn hex_walled_scene(wall_x: f64) -> (SceneEcs, Uuid, Uuid) {
        let scene_id = Uuid::from_u128(20);
        let token_id = Uuid::from_u128(21);
        let ecs = SceneEcs::from_documents(
            vec![
                entity_doc(
                    20,
                    0,
                    "scene",
                    json!({ "grid": { "kind": "hex", "size": 100 }, "background": null }),
                ),
                entity_doc(
                    21,
                    20,
                    "token",
                    json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
                ),
                entity_doc(
                    22,
                    20,
                    "wall",
                    json!({
                        "seg": { "x1": wall_x, "y1": -50.0, "x2": wall_x, "y2": 50.0 },
                        "blocksMove": true
                    }),
                ),
            ],
            0,
        );
        (ecs, scene_id, token_id)
    }

    #[test]
    fn hex_scene_gate_walk_blocks_entry_at_a_wall() {
        // Axial path (0,0)->(1,0)->(2,0). A wall crossing exactly the (1,0)->(2,0) center step
        // stops the executor at (1,0), never reaching the goal.
        let c1 = hex_cell_center(1, 0);
        let c2 = hex_cell_center(2, 0);
        let wall_x = (c1.0 + c2.0) / 2.0;
        let (ecs, scene, token) = hex_walled_scene(wall_x);
        let empty: BTreeSet<(i32, i32)> = BTreeSet::new();
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(0.0, 0.0), c1, c2],
            MovementRestriction::Unrestricted,
            &empty,
            100.0,
            false,
            0.4,
        )
        .unwrap();
        assert!(out.truncated, "must stop before crossing the hex wall");
        assert!(
            (out.stop.0 - c1.0).abs() < 1e-6 && (out.stop.1 - c1.1).abs() < 1e-6,
            "stops at the last legal hex cell before the wall, got {:?}",
            out.stop
        );
    }

    #[test]
    fn hex_scene_gate_walk_respects_the_visibility_mask() {
        // Axial path (0,0)->(1,0)->(2,0) under Visible restriction; the mask covers (0,0) and
        // (1,0) but excludes (2,0) — the executor must stop before entering the excluded cell.
        let (ecs, scene, token) = hex_clear_scene();
        let c1 = hex_cell_center(1, 0);
        let c2 = hex_cell_center(2, 0);
        let mut visible: BTreeSet<(i32, i32)> = BTreeSet::new();
        visible.insert((0, 0));
        visible.insert((1, 0));
        // (2, 0) deliberately excluded.
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(0.0, 0.0), c1, c2],
            MovementRestriction::Visible,
            &visible,
            100.0,
            false,
            0.4,
        )
        .unwrap();
        assert!(
            out.truncated,
            "must not reach the masked-out hex cell (2,0)"
        );
        assert!(
            out.stop.0 < c2.0 - 1e-6,
            "stop must land before the excluded cell's center, got {:?}",
            out.stop
        );
    }

    // -----------------------------------------------------------------------
    // Footprint-aware gate tests (D4): `execute_move` adopts `cell_enterable`'s footprint
    // predicate set instead of gating on the mover's center cell alone.
    // -----------------------------------------------------------------------

    /// Parentless config/actor doc builder (mirrors `entity_doc` for the parented case).
    fn entity_doc_top(
        id: u128,
        ty: &str,
        body: serde_json::Value,
    ) -> crate::data::document::Document {
        let mut d = doc(id, None, ty);
        d.engine = Some(body);
        d
    }

    /// A minimal, structurally-complete `ActorEngine` body with no vision override (falls back
    /// to the unlimited-range default), mirroring `scene::mod`'s own `actor_body_shaped` fixture.
    fn actor_body_shaped(shape: &str, w: f64, h: f64) -> serde_json::Value {
        json!({
            "displayName": "Fixture Actor",
            "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": w, "h": h },
            "shape": shape,
            "conditions": [],
            "prototype": true,
        })
    }

    /// Same as `actor_body_shaped`, plus an explicit vision assignment (range in grid cells).
    fn actor_body_shaped_with_vision(
        shape: &str,
        w: f64,
        h: f64,
        vision: serde_json::Value,
    ) -> serde_json::Value {
        json!({
            "displayName": "Fixture Actor",
            "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": w, "h": h },
            "shape": shape,
            "conditions": [],
            "prototype": true,
            "vision": vision,
        })
    }

    /// World settings for the footprint fixtures below: `visible`/losRestriction off/lighting
    /// all-bright (globalIllumination) — vision is driven entirely by each token's OWN vision
    /// assignment (range) rather than by wall/light geometry, keeping every fixture's mask
    /// derivable by hand from `resolve_token_footprint` + `visible_cells` alone.
    fn footprint_world_settings(model: &str) -> serde_json::Value {
        json!({
            "scene": {
                "losRestriction": false, "fog": true,
                "lightingEnabled": true, "lightMode": "globalIllumination",
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "movementModel": model,
                "partialCellLeniency": false
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        })
    }

    /// A corridor split by a single vertical wall (with a 300-unit gap centered on the token's
    /// row) between grid-aligned start/goal cells 4 columns apart, wide enough for a
    /// footprint-1.6-cell token (r_scene=80) to clear with margin. `start`/`goal` are computed
    /// from the ACTUAL grid shape's own `cell_center` (square: `(i+0.5)*cell`; hex: the axial
    /// pointy-top formula `hex_cell_center` uses) rather than a single literal shared across both
    /// kinds — hex cell centers do not fall on square-grid-aligned coordinates, and `execute_move`
    /// requires `path[0]` to equal the token's committed position exactly. Returns `(ecs, scene,
    /// token, user, start, goal)`. Vision is unlimited (no vision override) so the whole corridor
    /// is visible to `user`, isolating the wall/footprint interaction under test.
    fn scene_with_narrow_gap_and_wide_token(
        kind: &str,
        model: MovementModel,
    ) -> (SceneEcs, Uuid, Uuid, Uuid, (f64, f64), (f64, f64)) {
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let user = Uuid::from_u128(1);
        let model_str = match model {
            MovementModel::GridStepped => "grid-stepped",
            MovementModel::Continuous => "continuous",
        };
        let (start, goal) = if kind == "hex" {
            (hex_cell_center(0, 2), hex_cell_center(4, 2))
        } else {
            (((0.5) * 100.0, 2.5 * 100.0), (4.5 * 100.0, 2.5 * 100.0))
        };
        // Both grids place `start`/`goal` on the same row (`y` depends only on the row index, not
        // the column), so a single wall gap centered on that shared `y` clears both kinds.
        let row_y = start.1;
        let wall_x = (start.0 + goal.0) / 2.0;
        let mut tok = entity_doc(
            11,
            10,
            "token",
            json!({ "x": start.0, "y": start.1, "w": 100.0, "h": 100.0, "rotation": 0.0,
                    "actor_id": Uuid::from_u128(200).to_string() }),
        );
        tok.owner = Some(user);
        let wall = |id: u128, x1: f64, y1: f64, x2: f64, y2: f64| {
            entity_doc(
                id,
                10,
                "wall",
                json!({ "seg": {"x1":x1,"y1":y1,"x2":x2,"y2":y2}, "blocksMove": true }),
            )
        };
        let scene = entity_doc(
            10,
            0,
            "scene",
            json!({ "grid": { "kind": kind, "size": 100 }, "background": null,
                    "bounds": { "width": goal.0 + 400.0, "height": row_y + 400.0 } }),
        );
        let mut ecs = SceneEcs::from_documents(
            vec![
                scene,
                tok,
                wall(12, wall_x, 0.0, wall_x, row_y - 150.0),
                wall(13, wall_x, row_y + 150.0, wall_x, row_y + 2000.0),
            ],
            0,
        );
        ecs.set_actors(vec![entity_doc_top(
            200,
            "actor",
            actor_body_shaped("circle", 1.6, 1.6),
        )]);
        ecs.set_world_settings_for_test(footprint_world_settings(model_str));
        (ecs, scene_id, token_id, user, start, goal)
    }

    /// A `blocksMove` wall at x=100, sitting exactly midway (0.5 cell) between cell (0,0)'s center
    /// (50,50) and cell (1,0)'s center (150,50) — clears the default 0.4-cell footprint disc test
    /// (0.5 > 0.4) but still crosses the direct center-to-center step. The wall's y-span is huge
    /// (±2000, far past the search window) so no detour around either end exists: the reverse-I4
    /// test below needs column 1 genuinely UNREACHABLE, not merely blocked on the direct step (a
    /// finite wall's north/south end would let a detour reach the same destination cell via a
    /// different route, which the test cannot distinguish from a gate/router disagreement). The
    /// token carries no actor, so `resolve_token_footprint` falls back to the 0.4 default.
    fn scene_with_wall_between_adjacent_cells_and_default_footprint() -> (SceneEcs, Uuid, Uuid, Uuid)
    {
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let user = Uuid::from_u128(1);
        let mut tok = entity_doc(
            11,
            10,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let wall = entity_doc(
            12,
            10,
            "wall",
            json!({ "seg": {"x1":100.0,"y1":-2000.0,"x2":100.0,"y2":2000.0}, "blocksMove": true }),
        );
        let scene = entity_doc(
            10,
            0,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "bounds": { "width": 300.0, "height": 300.0 } }),
        );
        let mut ecs = SceneEcs::from_documents(vec![scene, tok, wall], 0);
        ecs.set_world_settings_for_test(footprint_world_settings("grid-stepped"));
        (ecs, scene_id, token_id, user)
    }

    /// A wide token (circle, size 1.2 cells ⇒ footprint radius 0.6, over the 0.5-cell half-width)
    /// whose vision is explicitly range-limited to 1.2 cells: this lights the token's own cell
    /// (0,0) and the straight-ahead cell (1,0) (both within range), but excludes the destination
    /// cell's PERPENDICULAR neighbors (1,-1)/(1,1) (~1.414 cells away) and the next cell along the
    /// row (2,0) (2.0 cells away) — every one of which the 0.6-radius footprint disc at (1,0)
    /// overlaps, so each is a candidate the mask must include for the move to succeed.
    fn scene_with_lit_center_line_only() -> (SceneEcs, Uuid, Uuid, Uuid) {
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let user = Uuid::from_u128(1);
        let mut tok = entity_doc(
            11,
            10,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        tok.embedded.insert(
            "actor".into(),
            vec![{
                let mut a = doc(99, None, "actor");
                a.engine = Some(actor_body_shaped_with_vision(
                    "circle",
                    1.2,
                    1.2,
                    json!([{ "mode": "normal", "range": 1.2 }]),
                ));
                a
            }],
        );
        let scene = entity_doc(
            10,
            0,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "bounds": { "width": 300.0, "height": 300.0 } }),
        );
        let mut ecs = SceneEcs::from_documents(vec![scene, tok], 0);
        ecs.set_world_settings_for_test(footprint_world_settings("grid-stepped"));
        (ecs, scene_id, token_id, user)
    }

    /// A fully open, fully visible scene (no walls, no vision override ⇒ unlimited range) — the
    /// `is_gm`-free counterpart of the wall/region fixtures above, for tests that need admissible
    /// footprint movement with nothing else in play.
    fn scene_with_open_lit_area() -> (SceneEcs, Uuid, Uuid, Uuid) {
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let user = Uuid::from_u128(1);
        let mut tok = entity_doc(
            11,
            10,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let scene = entity_doc(
            10,
            0,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "bounds": { "width": 300.0, "height": 300.0 } }),
        );
        let mut ecs = SceneEcs::from_documents(vec![scene, tok], 0);
        ecs.set_world_settings_for_test(footprint_world_settings("grid-stepped"));
        (ecs, scene_id, token_id, user)
    }

    /// A wide token (circle, size 1.2 ⇒ footprint radius 0.6) stepping (0,0)->(1,0) with an
    /// `arrest` region tightly enclosing NEIGHBOR cell (1,1)'s center only — a cell the 0.6-radius
    /// footprint disc at (1,0) overlaps (so it must stay in the mask, hence no vision override:
    /// unlimited range) but the mover's CENTER never enters. Arrest must not spring here.
    fn scene_with_arrest_cell_beside_the_path_and_wide_token() -> (SceneEcs, Uuid, Uuid, Uuid) {
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let user = Uuid::from_u128(1);
        let mut tok = entity_doc(
            11,
            10,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                    "actor_id": Uuid::from_u128(200).to_string() }),
        );
        tok.owner = Some(user);
        let scene = entity_doc(
            10,
            0,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "bounds": { "width": 300.0, "height": 300.0 } }),
        );
        let mut ecs = SceneEcs::from_documents(
            vec![
                scene,
                tok,
                region_doc(12, 10, "arrest", 1.0, (100.0, 100.0, 200.0, 200.0)),
            ],
            0,
        );
        ecs.set_actors(vec![entity_doc_top(
            200,
            "actor",
            actor_body_shaped("circle", 1.2, 1.2),
        )]);
        ecs.set_world_settings_for_test(footprint_world_settings("grid-stepped"));
        (ecs, scene_id, token_id, user)
    }

    /// Empty vision mask helper reused by `execute_move_refuses_an_out_of_range_footprint`
    /// (already defined above as `empty_mask`); this scene just needs a wall the Degenerate guard
    /// must reject BEFORE any per-step gating runs.
    fn scene_with_wall_across_the_path_for_footprint_guard() -> (SceneEcs, Uuid, Uuid) {
        scene_with_wall_across_the_path()
    }

    #[test]
    fn route_admissible_implies_gate_admissible_for_a_non_gm_grid() {
        // I4 forward direction, GridStepped-scoped: there `gate_walk` is the identity on
        // cell-center input, so the gate's sample points ARE the cell centers `cell_enterable`
        // evaluates at, giving the STRONG route ⇔ gate equivalence. This test only asserts the
        // ⇒ half (the ⇐ half is `gate_refused_steps_are_absent_from_every_route_non_gm_grid`
        // below). Continuous is NOT covered here — `gate_walk`'s dense sampling and the router's
        // cell-center evaluation operate at different granularity there, so only the WEAKER
        // route ⊆ gate-allowed direction holds; see
        // `route_admissible_implies_gate_admissible_for_a_non_gm_continuous` below.
        for kind in ["square", "hex"] {
            let (ecs, scene, token, user, start, goal) =
                scene_with_narrow_gap_and_wide_token(kind, MovementModel::GridStepped);
            let fp = ecs.resolve_token_footprint(token).expect("in-range");
            let mask = ecs.visible_cells(user, scene, false);
            // NOT `if let Ok` — a fixture that yields no route must fail the test, not skip it.
            let route = ecs
                .pathfind(user, scene, start, &[goal], fp, false, None)
                .expect("the fixture is routable for this footprint");
            let out = execute_move(
                &ecs,
                scene,
                token,
                &route.path,
                MovementRestriction::Visible,
                &mask,
                100.0,
                false,
                fp,
            )
            .expect("a routed path is admissible");
            assert!(
                !out.truncated,
                "kind={kind}: the gate accepts every routed step"
            );
        }
    }

    #[test]
    fn route_admissible_implies_gate_admissible_for_a_non_gm_continuous() {
        // I4 WEAKER direction, Continuous-scoped: `gate_walk`'s dense sampling and the router's
        // cell-center evaluation operate at different granularity on this model, so only route ⊆
        // gate-allowed holds here — NOT the reverse/equivalence, which the plan scopes to
        // GridStepped only (see the GridStepped test above). This reuses
        // `scene_with_narrow_gap_and_wide_token`'s existing `Continuous` dispatch arm: no region
        // is present, so `pathfind` takes the pure-polyanya branch and returns a genuine
        // multi-sample any-angle route through the 300-unit gap (footprint radius 80), not a
        // degenerate single-point route — the length assertion below rules out a vacuous pass.
        let (ecs, scene, token, user, start, goal) =
            scene_with_narrow_gap_and_wide_token("square", MovementModel::Continuous);
        let fp = ecs.resolve_token_footprint(token).expect("in-range");
        let mask = ecs.visible_cells(user, scene, false);
        let route = ecs
            .pathfind(user, scene, start, &[goal], fp, false, None)
            .expect("the fixture is routable for this footprint");
        assert!(
            route.path.len() >= 2,
            "the any-angle polyanya route must actually traverse the gap, not collapse to a point"
        );
        let out = execute_move(
            &ecs,
            scene,
            token,
            &route.path,
            MovementRestriction::Visible,
            &mask,
            100.0,
            false,
            fp,
        )
        .expect("a router-admissible continuous route is gate-admissible");
        assert!(
            !out.truncated,
            "the gate accepts every step of the router's own any-angle route"
        );
    }

    #[test]
    fn gate_refused_steps_are_absent_from_every_route_non_gm_grid() {
        // I4 REVERSE direction, which the spec requires and which is what catches a gate MORE
        // permissive than the router (e.g. a dropped segments_cross check).
        let (ecs, scene, token, user) =
            scene_with_wall_between_adjacent_cells_and_default_footprint();
        let fp = ecs.resolve_token_footprint(token).expect("in-range"); // 0.4
        let mask = ecs.visible_cells(user, scene, false);
        let candidates = [
            [(50.0, 50.0), (150.0, 50.0)],
            [(50.0, 50.0), (150.0, 150.0)],
            [(50.0, 50.0), (50.0, 150.0)],
        ];
        for path in candidates {
            let out = execute_move(
                &ecs,
                scene,
                token,
                &path,
                MovementRestriction::Visible,
                &mask,
                100.0,
                false,
                fp,
            )
            .expect("admissible input");
            if out.truncated {
                let route = ecs.pathfind(user, scene, path[0], &[path[1]], fp, false, None);
                if let Ok(r) = route {
                    assert!(
                        r.path.last().copied() != Some(path[1]),
                        "the gate refuses {:?} but a route reaches it — the gate is more \
                         permissive than the router",
                        path
                    );
                }
            }
        }
    }

    #[test]
    fn a_default_footprint_step_across_a_wall_is_still_truncated() {
        // Regression for the dropped segments_cross check: a wall between two adjacent cell
        // centers sits 0.5 cell from each, so the 0.4-radius disc test alone would pass it.
        let (ecs, scene, token, user) =
            scene_with_wall_between_adjacent_cells_and_default_footprint();
        let mask = ecs.visible_cells(user, scene, false);
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(50.0, 50.0), (150.0, 50.0)],
            MovementRestriction::Visible,
            &mask,
            100.0,
            false,
            0.4,
        )
        .expect("admissible");
        assert!(
            out.truncated,
            "the wall still blocks a default-footprint step"
        );
    }

    #[test]
    fn a_wide_token_cannot_enter_a_cell_whose_footprint_overlaps_fog() {
        let (ecs, scene, token, user) = scene_with_lit_center_line_only();
        let fp = ecs.resolve_token_footprint(token).expect("in-range"); // > 0.5
        let mask = ecs.visible_cells(user, scene, false);
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(50.0, 50.0), (150.0, 50.0)],
            MovementRestriction::Visible,
            &mask,
            100.0,
            false,
            fp,
        )
        .expect("admissible");
        assert!(
            out.truncated,
            "a footprint cell outside the mask stops a wide token"
        );
    }

    #[test]
    fn a_sub_half_cell_footprint_diagonal_stays_admissible() {
        // The buddy-check P1 case: a small footprint's diagonal must not regress.
        let (ecs, scene, token, user) = scene_with_open_lit_area();
        let mask = ecs.visible_cells(user, scene, false);
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(50.0, 50.0), (150.0, 150.0)],
            MovementRestriction::Visible,
            &mask,
            100.0,
            false,
            0.4,
        )
        .expect("admissible");
        assert!(
            !out.truncated,
            "a 0.4-radius diagonal step is still allowed"
        );
    }

    #[test]
    fn arrest_stays_center_cell_matching_the_router() {
        // `cell_enterable` does NOT footprint-gate arrest (`pathfinding.rs:133-136`). A wide
        // token whose FOOTPRINT touches an arrest cell but whose CENTER does not must not be
        // arrested, or the gate becomes stricter than the router and I4 breaks.
        let (ecs, scene, token, user) = scene_with_arrest_cell_beside_the_path_and_wide_token();
        let fp = ecs.resolve_token_footprint(token).expect("in-range"); // > 0.5
        let mask = ecs.visible_cells(user, scene, false);
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[(50.0, 50.0), (150.0, 50.0)],
            MovementRestriction::Visible,
            &mask,
            100.0,
            false,
            fp,
        )
        .expect("admissible");
        assert!(
            !out.truncated,
            "arrest is center-cell only, matching the router"
        );
    }

    #[test]
    fn a_gm_is_exempt_from_every_footprint_check() {
        let (ecs, scene, token, _user, start, goal) =
            scene_with_narrow_gap_and_wide_token("square", MovementModel::GridStepped);
        let out = execute_move(
            &ecs,
            scene,
            token,
            &[start, goal],
            MovementRestriction::Unrestricted,
            &empty_mask(),
            100.0,
            true,
            5.0,
        )
        .expect("admissible");
        assert!(
            !out.truncated,
            "a GM squeezes a wide token through anything"
        );
    }

    #[test]
    fn execute_move_refuses_an_out_of_range_footprint() {
        // I1: the new gate input gets an admissibility guard like every other. A NaN radius
        // would make every `dist < r_scene` comparison false — fail-open.
        let (ecs, scene, token) = scene_with_wall_across_the_path_for_footprint_guard();
        for bad in [
            f64::NAN,
            -1.0,
            crate::scene::pathfinding::MAX_FOOTPRINT_CELLS + 1.0,
        ] {
            let err = execute_move(
                &ecs,
                scene,
                token,
                &[(50.0, 50.0), (150.0, 50.0)],
                MovementRestriction::Unrestricted,
                &empty_mask(),
                100.0,
                false,
                bad,
            )
            .expect_err("an out-of-range footprint is refused");
            assert!(
                matches!(err, MoveReject::Degenerate),
                "bad={bad}: got {err:?}"
            );
        }
    }
}
