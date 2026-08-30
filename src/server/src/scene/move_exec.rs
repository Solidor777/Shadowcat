//! Pure, lock-free per-path move executor. Server-authoritative movement, engine-agnostic.
//!
//! `execute_move` is built on `gate_walk`, which subdivides ANY input polyline — grid A*
//! cell-center vertices ≤1 cell apart, or any-angle continuous vertices arbitrarily far apart —
//! into a dense walk where every consecutive pair is at most one cell apart (Chebyshev),
//! preserving already-≤1-cell segments as an identity. The per-step gate then runs over this
//! dense walk, validating each dense sub-step against the SAME predicate set
//! `pathfinding::cell_enterable` uses for routing:
//! - a footprint-disc clearance test AND a center-to-center segment-crossing test against every
//!   `blocksMove` wall in the AUTHORITATIVE wall set (`ecs.move_walls(scene, None)`) — both are
//!   required; the disc test alone lets a wall between two adjacent cell centers become
//!   permeable at the default 0.4-cell footprint,
//! - the caller-supplied `visible` mask (skipped for `Unrestricted`) over
//!   `footprint_cells ∪ line_traversal`, not the center point alone,
//! - the region field: impassable is footprint-gated (a wide body cannot fit past
//!   impassable terrain any more than a wall); arrest and terrain stay CENTER-CELL only, mirroring
//!   `cell_enterable`'s documented asymmetry (they act on the mover's own position, not solid
//!   geometry it must clear). Always reads the AUTHORITATIVE field (`ecs.region_field(scene,
//!   None)`) — this executor springs every region regardless of what the mover's own pathfind
//!   preview could see.
//!
//! Returns the stop cell + the legal prefix render-path + accumulated cost. `truncated` is true
//! when the move stops before `path.last()` for any reason (wall, mask, region-impassable,
//! region-arrest, or an exhausted movement `MoveGateInputs::budget`), including a region-arrest
//! on the final path step.
//!
//! INVARIANT (route-gate parity): step 2 calls `GridShape::line_traversal(prev,
//! next, cell)` (via `ecs.resolve_grid_shape`) and checks `all ∈ visible` over
//! `footprint_cells ∪ line_traversal` — square delegates to `movement::supercover_cells`, hex to
//! a ψ-crossing supercover, so the predicate agrees on every cell on BOTH grid kinds, not square
//! alone. This is the SAME predicate set `pathfinding::cell_enterable` uses when routing: on
//! `GridStepped`, route-admissible ⇔ gate-admissible for a non-GM mover (the parity tests in this
//! file pin the equivalence); on `Continuous` the two evaluate at different granularity (dense
//! `gate_walk` sampling vs the router's cell-center check), so only the weaker route ⊆
//! gate-allowed direction holds. `execute_move` is the sole per-cell traversal decision —
//! there is no separate gate to keep in sync. The caller pre-computes `visible` off the ECS read
//! lock, so this executor is pure and imposes no lock ordering.
//!
//! Coupling: `token_position` is the ECS committed-position seam; any rename
//! must update both this caller and `SceneEcs::token_move`.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeSet;

use uuid::Uuid;

use crate::scene::grid_shape::euclidean_span_cells;
use crate::scene::{MovementModel, MovementRestriction, SceneEcs};

/// Epsilon for `path[0]`-vs-committed-position comparison (scene units).
/// A client rounding the center-of-cell to the nearest float can drift by at most
/// a few ULPs at typical coordinate magnitudes; 1e-6 is strict but not pedantic.
const EPS: f64 = 1e-6;

/// DoS guard for `gate_walk`: a walk requiring more than this many dense samples is
/// rejected outright, never truncated. Arc-length/cell-count based — a single continuous
/// segment can be arbitrarily long, so an authored-vertex-count cap is not the right invariant:
/// the bound must count dense samples, not authored waypoints.
pub(crate) const MAX_GATE_WALK_SAMPLES: usize = 4096;

/// Magnitude ceiling (scene units) for any `gate_walk` input path coordinate, checked
/// structurally BEFORE the per-step tolerance arithmetic in `gate_walk` (mirrors `navmesh::
/// MAX_NAVMESH_COORD`'s convention: bound the input before any downstream arithmetic that is
/// sensitive to magnitude, not after).
///
/// This value is deliberately much smaller than `navmesh::MAX_NAVMESH_COORD` (1e15) — the two
/// bounds guard against DIFFERENT failure modes and neither number transfers to the other's
/// module. `MAX_NAVMESH_COORD` guards an `f64 -> f32` cast that only saturates near `f32::MAX`
/// (~3.4e38), so 1e15 is safe there with enormous headroom. Here, `gate_walk`'s per-step tolerance
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
/// The per-step Chebyshev comparison uses a magnitude-SCALED float tolerance (built from
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
        // Magnitude-relative tolerance (mirrors `movement::supercover_cells`'s corner-test
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
    /// `true` when the move stopped before `path.last()` — wall, mask, region-impassable,
    /// region-arrest, OR an exhausted movement `MoveGateInputs::budget`, including a
    /// region-arrest on the FINAL step (where `stop_index == path.len()-1` would make the
    /// index comparison alone report false; a `stopped_early` bool ensures that case is
    /// reported correctly). Threaded onto the `MoveStream` wire
    /// frame via `MoveExecution::truncated`, and trusted-only there: a clipped observer
    /// receives `None`.
    pub truncated: bool,
    /// Total terrain-weighted cost accumulated over the walked prefix — the router's number for
    /// the same route (see the unified step-price note on `MoveGateInputs::budget`); consumed by
    /// the movement-budget gate (`MoveGateInputs::budget`) and exposed on the wire.
    pub cost: f64,
}

/// Reason an `execute_move` call was rejected before any walking.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MoveReject {
    /// `token` is not a token entity in the ECS (unknown id or wrong doc_type).
    NotAToken,
    /// `path` has fewer than 2 points (no step to walk).
    EmptyPath,
    /// `gate_walk` returned `None`: either the path's dense walk (arc-length/sample-count,
    /// not authored-vertex-count) would exceed `MAX_GATE_WALK_SAMPLES` — the DoS bound is
    /// arc-length based because a single arbitrarily-long continuous segment is the DoS
    /// surface, not the number of authored waypoints — or a path coordinate's
    /// magnitude exceeds `MAX_GATE_WALK_COORD` (a distinct, unreachable-in-practice degenerate
    /// case sharing this variant rather than `Degenerate`, since both originate from the same
    /// `gate_walk` fail-closed `None`).
    TooLong,
    /// A structural invariant was violated: non-finite coords, or `path[0]` not at the
    /// token's committed position. Does NOT cover a non-adjacent king-step jump: that case is
    /// subdivided-and-gated instead of rejected — see `gate_walk`.
    Degenerate,
    /// The token's scene has no document — refuse rather than synthesize a grid.
    SceneUnknown,
    /// The mover's token belongs to a combatant that does not hold the current turn under
    /// `Enforcement::Hard`. Never constructed by this module — `Room::execute_move` resolves the
    /// combat's turn state above the scene read guard and returns `DataError::Forbidden`
    /// directly; this variant exists so that refusal has one named reason, recorded in a
    /// `tracing::debug!` alongside it.
    NotYourTurn,
    /// The combat names a movement resource the budget cannot be derived from: no
    /// `grid.distance` under `Interpretation::PerCell`, or no such resource entry on the
    /// combatant. Never constructed by this module, for the same reason as `NotYourTurn`.
    BudgetUnresolvable,
}

/// The scene state `execute_move`'s per-cell gate runs against, all of it resolved by the caller
/// (`Room::execute_move`) before or around the scene read guard rather than re-derived here.
///
/// INVARIANT: `scene` is DERIVED FROM THE TOKEN (`SceneEcs::token_move`), never taken from the
/// client's frame — every other field must be resolved for that same derived scene, or the gate
/// evaluates one scene's geometry against another's mask.
pub(crate) struct MoveGateInputs<'a> {
    /// Scene the token lives in. An absent document is `MoveReject::SceneUnknown`, which binds a
    /// GM exactly as it binds a player.
    pub scene: Uuid,
    /// Movement restriction mode pre-resolved by the caller from `SceneEcs::resolve_scene`;
    /// `Unrestricted` means the mask gate is skipped.
    pub restriction: MovementRestriction,
    /// The resolved mask the gate decision uses. Ignored when `Unrestricted`. For `Visible` this
    /// is `SceneEcs::visible_cells`; for `Revealed` the caller MUST pass
    /// `visible_cells ∪ explored` — this function does not union it.
    pub visible: &'a BTreeSet<(i32, i32)>,
    /// Grid cell size in scene units. Non-finite or non-positive is `MoveReject::Degenerate`,
    /// which binds a GM exactly as it binds a player.
    pub cell: f64,
    /// Remaining movement in cells; the walk stops before the first transition whose cumulative
    /// cost would exceed it. `None` = unlimited. Like `visible`, a GM is structurally exempt
    /// from this gate (`execute_move`'s own `check_budget = !is_gm`, matching `check_walls`/
    /// `check_regions`/`check_mask`) regardless of what the caller passes here — the caller
    /// convention is still to pass `None` for a GM, but a future caller that forgets cannot
    /// truncate a GM's move. A non-finite `Some` value is `MoveReject::Degenerate`, matching
    /// every other `f64` input this function accepts. Cost still accrues for a GM regardless of
    /// this exemption; only the STOP is gated.
    pub budget: Option<f64>,
}

/// Walk `path` step by step, validating each step against the wall gate (step 1), the
/// vision-mask gate (step 2), and the region field (step 3).
///
/// # Engine-agnostic gate walk
///
/// `path` may be ANY polyline — grid A* emits cell-center vertices ≤1 cell apart; the
/// polyanya router emits any-angle vertices arbitrarily far apart. `gate_walk` subdivides it
/// into a dense walk where every consecutive pair is ≤1 cell apart, preserving already-≤1-cell
/// segments EXACTLY (identity on grid input — see `gate_walk`'s doc comment). The per-step
/// gate runs over this DENSE walk; the coarse `render_path` returned to the caller is
/// reconstructed from the authored vertices actually traversed plus the exact stop point.
///
/// # Parity with `pathfinding::cell_enterable` — per-cell decision and per-step cost
///
/// The per-cell decision (step 1 + step 2) uses the SAME primitives as the router's
/// `cell_enterable`: wall-segment crossing (`segments_cross`, the primitive `blocks_move`
/// wraps), footprint-disc clearance, `GridShape::line_traversal` (a supercover on both grid
/// kinds — square cell-walk, hex ψ-crossing, per `ecs.resolve_grid_shape`), and the
/// pre-computed `visible` set over `footprint_cells ∪ line_traversal`. There is no third gate
/// this executor and the router must independently agree with — `execute_move` is the sole
/// per-cell traversal decision, so this parity is pinned directly between the two, not
/// mediated through a shared middle gate. On `GridStepped` the two are equivalent
/// (route-admissible ⇔ gate-admissible) for a non-GM mover; on `Continuous` only the weaker
/// route ⊆ gate-allowed direction holds, since `gate_walk`'s dense sampling and the router's
/// cell-center evaluation operate at different granularity. For a grid input, this executor's
/// outcome is pinned to literal expected fixtures that nothing computes at runtime, so any
/// change to its grid behaviour requires a deliberate fixture edit — see
/// `frozen_parity_king_step_grid_outcomes`.
///
/// A >1-cell authored jump is subdivided by `gate_walk` and gated per crossed cell, exactly as if
/// the client had sent the explicit intermediate waypoints — no new capability, since a
/// well-formed sequence of intermediate waypoints was always legal.
///
/// GM-ness is folded into `restriction == Unrestricted` by the caller (mirroring `publish`'s
/// `if !Unrestricted { continue }` skip).
///
/// # Arguments
///
/// - `ecs` — ECS to query for token position and wall geometry.
/// - `gate` — The scene state the caller resolved off the read lock (see `MoveGateInputs`).
/// - `token` — Token doc id.
/// - `path` — Proposed path (cell centers for grid, any-angle vertices for continuous);
///   `path[0]` must equal the token's committed position within `EPS`.
/// - `is_gm` — When true, every gameplay gate (walls, mask, impassable, arrest, budget
///   truncation) is bypassed, matching `publish`'s own GM position write. Resource
///   guards (`gate_walk`'s `MAX_GATE_WALK_COORD`/`MAX_GATE_WALK_SAMPLES`, the non-finite and
///   scene-existence refusals, and the `footprint_radius_cells` range guard) are never
///   exempted. Terrain cost still accrues regardless of the exemption; only the STOP is gated.
/// - `footprint_radius_cells` — The mover's bounding-disc radius in grid cells (see
///   `SceneEcs::resolve_token_footprint`). Must be in `[0, pathfinding::MAX_FOOTPRINT_CELLS]`;
///   out-of-range (including NaN) is `MoveReject::Degenerate`, unconditionally — a GM's wider
///   gameplay exemption does not cover this input guard.
//
// `is_gm` stays a standalone parameter, never a field of `MoveGateInputs`: that struct mixes
// inputs a GM is exempt from (`restriction` and `visible`, read only under `check_mask`) with
// inputs that bind a GM unconditionally (`scene`, whose absent document is
// `MoveReject::SceneUnknown`; `cell`, whose non-finite or non-positive value is
// `MoveReject::Degenerate`). The exemption switch does not belong in the same value as the
// guards it must never exempt.
pub(crate) fn execute_move(
    ecs: &SceneEcs,
    gate: MoveGateInputs<'_>,
    token: Uuid,
    path: &[(f64, f64)],
    is_gm: bool,
    footprint_radius_cells: f64,
) -> Result<MoveOutcome, MoveReject> {
    let MoveGateInputs {
        scene,
        restriction,
        visible,
        cell,
        budget,
    } = gate;
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
    // A resource/admissibility guard, never exempted for a GM. `contains` rejects NaN and
    // ±Inf (NaN comparisons are always false; Inf > MAX_FOOTPRINT_CELLS).
    if !(0.0..=crate::scene::pathfinding::MAX_FOOTPRINT_CELLS).contains(&footprint_radius_cells) {
        return Err(MoveReject::Degenerate);
    }
    // `budget` is validated exactly like every other `f64` input this function accepts
    // (`cell`, path coordinates, `footprint_radius_cells`): a non-finite value is rejected
    // BEFORE it reaches the per-transition `budget_admits_step` comparison, where a
    // NaN `b` would otherwise make every such comparison evaluate `false` (IEEE-754 NaN
    // comparisons never return `true`) and silently behave as `None` (unlimited). A finite
    // negative budget is NOT rejected here — it degrades gracefully at the first transition
    // instead (every non-negative `step_cost` immediately exceeds it), so only non-finite
    // values are a structural admissibility failure.
    if let Some(b) = budget {
        if !b.is_finite() {
            return Err(MoveReject::Degenerate);
        }
    }

    // path[0] must equal the token's committed position. The ECS is authoritative; the
    // client must request from the real position, not a claimed one.
    let cur = ecs.token_position(token).ok_or(MoveReject::NotAToken)?;
    if (cur.0 - path[0].0).abs() > EPS || (cur.1 - path[0].1).abs() > EPS {
        return Err(MoveReject::Degenerate);
    }

    // Subdivide into the dense ≤1-cell gate walk; identity on
    // grid input. `None` means the walk would exceed MAX_GATE_WALK_SAMPLES — fail closed.
    let walk = gate_walk(path, cell).ok_or(MoveReject::TooLong)?;
    // walk.len() >= 2 always here: path.len() >= 2 is already guaranteed by the `EmptyPath`
    // refusal, and the loop
    // inside gate_walk appends at least one sample per authored segment.

    // Gameplay gates apply to non-GMs only. A GM may make an illegal move: they move with or
    // without pathfinding, and a placement lands where asked, matching `publish`'s GM
    // position write. Resource guards — `gate_walk`'s MAX_GATE_WALK_COORD / MAX_GATE_WALK_SAMPLES,
    // non-finite refusal, and the scene-existence refusal — are NOT exempted for a GM.
    let check_walls = !is_gm;
    let check_regions = !is_gm;
    let check_mask = !is_gm && !matches!(restriction, MovementRestriction::Unrestricted);
    // Structural GM exemption, matching `check_walls`/`check_regions`/`check_mask` above: this
    // function itself guarantees a GM is never truncated by budget, rather than relying on
    // every future caller to remember to pass `budget: None` for a GM. Cost still accrues
    // regardless (see the `budget` field's own doc comment) — only the STOP is gated.
    let check_budget = !is_gm;

    // Authoritative region field: always the full field, never filtered — this
    // executor springs secret regions regardless of what the mover's pathfind preview
    // could see.
    let Some(regions) = ecs.region_field(scene, None) else {
        return Err(MoveReject::SceneUnknown);
    };

    // The mask gate's segment-crossing set is engine-agnostic geometry (`GridShape::
    // line_traversal`), routed through the scene's own resolved grid shape (square or hex) rather
    // than a hardcoded square-grid call.
    let grid = ecs.resolve_grid_shape(scene, cell);

    // The executor always reads the AUTHORITATIVE wall set: a `gm_only` wall omitted from the
    // requester's route springs here, exactly as a secret region does.
    let gate_walls = ecs.move_walls(scene, None);

    // Constant for the whole walk: the footprint disc radius in world units, mirroring
    // `cell_enterable`'s `r_scene`. The radius is already stated in the grid's OWN cells by
    // `footprint::resolve_footprint_cells` — the authored block's half-diagonal on square, the
    // circumscribing radius of the authored hex count on hex — so it converts through the INDEXING
    // scale, not `GridShape::world_units_per_cell`. Scaling it through the world-unit conversion
    // would give a 1-hex token a disc past its own hex's circumradius and make a medium creature
    // occupy seven hexes, which is a rules change rather than a unit fix.
    let r_scene = footprint_radius_cells.max(0.0) * cell;

    // Region-cell lookup goes through the SAME resolved grid shape as rasterization
    // (`region_field`) and the mask's `line_traversal` — `grid.cell_of` is the hex-correct point→
    // cell mapping (square: `floor(p/cell)`; hex: pixel→axial round). A hardcoded square `floor`
    // here would test hex moves against square-indexed cells while `region_field` rasterized onto
    // axial cells — two incompatible coordinate systems on a hex scene.
    let to_cell = |p: (f64, f64)| -> (i32, i32) { grid.cell_of(p) };

    // Movement model + step-price bookkeeping, shared with the router's own cost so a route
    // preview and its execution report the same number (see this function's module doc, "Parity
    // with `pathfinding::cell_enterable`"). `parity` threads the `Alternating` diagonal-rule bit
    // across GridStepped transitions, exactly as `pathfinding::find`'s arrest replay threads it —
    // never reset per transition. `world_per_cell` is the authored-distance conversion (never
    // `cell`, the indexing scale — see `GridShape::world_units_per_cell`'s own note), used to
    // price a Continuous transition/tail span as the Euclidean length the polyanya/`los_smooth`
    // route reports for the same geometry.
    let movement_model = ecs.resolve_scene(scene).movement_model;
    let mut parity: u8 = 0;
    let world_per_cell = grid.world_units_per_cell();

    // --- Per-step walk over the DENSE gate walk ---
    let mut stop_idx = 0usize; // index into `walk`
    let mut stopped_early = false;
    let mut cost = 0.0;
    // The cell already accounted for by region/cost logic. The START cell is never itself
    // "entered": cost accrual begins at the first cell transition (`i = 1` / `to_cell(next)`).
    let mut last_region_cell = to_cell(walk[0].pos);
    // The position of the last CELL-ENTRY transition (distinct from `last_region_cell`, its
    // cell): on `Continuous`, several dense samples can sit inside one cell without a
    // transition, so the span charged at the NEXT transition must cover the whole distance since
    // this point, not just the one dense sub-step — matching the whole-polyline integration the
    // polyanya/`los_smooth` route reports for the same geometry.
    let mut last_transition_pos = walk[0].pos;

    for i in 1..walk.len() {
        let prev = walk[i - 1].pos;
        let next = walk[i].pos;
        let next_cell = to_cell(next);
        // The footprint disc's anchor for every CELL-MEMBERSHIP test (mask, impassable) is the
        // mover's true continuous position `next`, matching the wall-disc anchor exactly (both
        // checks below pass `next`, not a cell center). A positive-radius disc anchored on a
        // cell boundary has genuine positive-area overlap with every cell it touches there;
        // `footprint_cells`'s own r=0 tie-break (see its doc comment) resolves a zero-radius
        // point to a single canonical cell, so the mask/impassable disc and the wall disc can
        // share one anchor with no degenerate multi-cell admission at any radius.

        // Step 1: wall gate — every dense sub-segment, exempt for a GM. TWO checks,
        // both from `cell_enterable`: the footprint disc at
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
        // center — the same `footprint_cells ∪ line_traversal` union `cell_enterable` requires.
        // This density is exactly why gate_walk exists: line_traversal
        // is well-defined and dense enough to cover the swept footprint for an any-angle
        // segment, not just a king step.
        if check_mask {
            let Some(mut cells) = grid.line_traversal(prev, next, cell) else {
                stopped_early = true;
                break;
            };
            cells.extend(grid.footprint_cells(next_cell, next, r_scene, cell));
            if !cells.iter().all(|c| visible.contains(c)) {
                stopped_early = true;
                break;
            }
        }

        // Step 3: region gate, keyed on CELL-ENTRY TRANSITIONS, not per dense sample
        // — a continuous path subdivided into several sub-cell samples within the same cell
        // is evaluated exactly once for that cell. For grid input that is one accrual per
        // authored step, since every authored step crosses into a distinct new
        // cell. Center-cell only, mirroring the documented asymmetry against
        // the router's footprint-aware check (see `cell_enterable`'s docs).
        //
        // This transition-dedup relies on the router never emitting two consecutive dense
        // samples that map to the SAME cell: true for grid A* (`pathfinding::find`) and true
        // for `gate_walk`'s output here, since it only ever emits progressing samples along the
        // input polyline (no stationary/duplicate cell re-visits). There is no separate adjacency
        // guard rejecting a non-adjacent jump: this executor subdivides via `gate_walk` instead of
        // rejecting, so a duplicate-cell transition would silently fall through this dedup rather
        // than being caught by a separate check.
        if next_cell != last_region_cell {
            // Impassable IS footprint-gated (`cell_enterable`'s check 4):
            // a wide body cannot fit past impassable terrain any more than past a wall.
            if check_regions {
                let fp_cells = grid.footprint_cells(next_cell, next, r_scene, cell);
                if fp_cells.iter().any(|c| regions.is_impassable(*c)) {
                    stopped_early = true;
                    break;
                }
            }
            // Step price: the router's own number for the same transition. GridStepped ⇒ the
            // `GridShape::neighbors_with_cost` cost for this adjacent pair (parity threaded
            // exactly as `pathfinding::find`'s arrest replay threads it); a transition this
            // shape's own neighbor enumeration does not recognize (a >1-cell jump gated through a
            // `GridStepped` scene) falls back to the Euclidean span, mirroring the Continuous
            // rule. Continuous ⇒ the Euclidean span in cells since the last transition, which is
            // what `navmesh::los_smooth` and the polyanya router report for the same geometry.
            // Terrain multiplies the entered cell in both models. Cost accrues regardless of the
            // gameplay exemption; only `budget` can stop the walk on cost.
            let step = match movement_model {
                MovementModel::GridStepped => grid
                    .neighbors_with_cost(last_region_cell, parity)
                    .into_iter()
                    .find(|(c, _, _)| *c == next_cell)
                    .map(|(_, sc, p)| {
                        parity = p;
                        sc
                    })
                    .unwrap_or_else(|| euclidean_span_cells(prev, next, world_per_cell)),
                MovementModel::Continuous => {
                    euclidean_span_cells(last_transition_pos, next, world_per_cell)
                }
            };
            let step_cost = step * regions.terrain_multiplier(next_cell);
            if check_budget {
                if let Some(b) = budget {
                    if !crate::scene::pathfinding::budget_admits_step(cost, step_cost, b) {
                        stopped_early = true;
                        break;
                    }
                }
            }
            cost += step_cost;
            last_transition_pos = next;
            // Arrest and terrain stay CENTER-CELL only, mirroring `cell_enterable`'s documented
            // asymmetry: they act on the mover's own position rather
            // than solid geometry it must clear. Footprint-gating arrest here would make the gate
            // stricter than the router and break route-gate parity.
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

    // Continuous tail: the loop above prices only full cell-entry transitions, so a Continuous
    // move that halts partway through its final cell (wall/mask/region/budget stop, or simply
    // reaching a goal mid-cell) has not yet been charged for the distance since its last
    // transition. Priced at the stop cell's own terrain multiplier, mirroring the whole-polyline
    // integration `navmesh::los_smooth`/the polyanya router apply to the same geometry.
    // GridStepped needs no such tail: its dense samples land exactly on cell transitions, so the
    // per-transition step price above already reflects the router's own exact quantity.
    if matches!(movement_model, MovementModel::Continuous) {
        let stop_pos = walk[stop_idx].pos;
        let stop_cell = to_cell(stop_pos);
        cost += euclidean_span_cells(last_transition_pos, stop_pos, world_per_cell)
            * regions.terrain_multiplier(stop_cell);
    }

    // --- Coarse render_path: authored vertices fully traversed + the exact stop point ---
    let stop_sample = walk[stop_idx];
    let render_path = match stop_sample.authored_idx {
        // Stop lands exactly on an authored vertex (always true for grid input, since
        // gate_walk is identity there): the coarse path is the authored-vertex prefix.
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

    // Safe: walk.len() >= 2 is guaranteed by the `EmptyPath` refusal, so len() - 1 never
    // underflows.
    let truncated = stopped_early || stop_idx < walk.len() - 1;
    Ok(MoveOutcome {
        stop: stop_sample.pos,
        render_path,
        truncated,
        cost,
    })
}

#[cfg(test)]
mod tests;
