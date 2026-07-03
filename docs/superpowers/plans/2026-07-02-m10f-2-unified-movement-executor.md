# M10f-2 — Unify the Movement Executor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor `move_exec::execute_move` from a king-step-per-authored-cell walk into an
engine-agnostic sampled-polyline executor that produces byte-identical outcomes to today's
executor on every grid input, while correctly gating any-angle continuous polylines at the unit
level (continuous execution is not wired end-to-end until M10f-3).

**Architecture:** A new pure `gate_walk` primitive subdivides an authored polyline into a dense
walk where consecutive samples are never more than one cell apart, preserving already-≤1-cell
input exactly (identity on grid input). `execute_move` runs the existing per-cell gate
(`blocks_move` → `supercover_cells` → region field) over this dense walk instead of over the
raw authored path, but still returns the coarse authored-vertex-shaped `render_path` externally.
Grid-parity is proven both structurally (identity subdivision) and empirically (a differential
test comparing the new executor against a temporarily-retained copy of today's king-step
executor, frozen as literal fixtures and then deleted once proven).

**Tech Stack:** Rust (`shadowcat` server crate), existing `scene::movement::supercover_cells`,
`scene::regions::RegionField`, no new dependencies.

## Global Constraints

- Server-only. No protocol frame change, no client change (spec §8/§9). Do not touch
  `src/client/**` or `Room::execute_move`'s logic — only its doc comments, if anything.
- `MoveOutcome { stop, render_path, truncated, cost }` and `execute_move`'s signature
  (`ecs, scene, token, path, restriction, visible, cell`) are unchanged externally — every
  caller (`Room::execute_move` in `src/server/src/ws/room.rs`) requires zero code changes.
- The king-step input-rejection guard is REMOVED; a >1-cell authored jump is now subdivided
  and gated per cell, never rejected outright by shape (spec §4.2).
- The DoS bound moves from an authored-vertex-count cap (`MAX_MOVE_PATH = 256`) to a
  gate-walk-sample-count cap (spec §4.3); `MoveReject::TooLong`'s meaning changes accordingly.
- The differential-oracle executor (`execute_move_kingstep_oracle`) is `#[cfg(test)]`-only,
  temporary, and MUST be deleted in the final task once its proven cases are frozen as literal
  fixtures (spec §4.4) — do not let it become permanent.
- No new crate dependency. Pure geometry; no `#[cfg]`-gated OS-specific code.
- Verification commands throughout: `cargo test -p shadowcat --lib scene::move_exec`,
  `cargo fmt`, `cargo clippy -p shadowcat --lib -- -D warnings` (use `--lib`, not
  `--all-targets` — `--all-targets` clippy trips a pre-existing, unrelated
  `too_many_arguments` lint on this file's `region_doc` test helper, logged since M10g; not
  a regression to fix here).

---

## File Structure

All work lives in one existing file — this refactor needs no new files or modules:

- **Modify:** `src/server/src/scene/move_exec.rs` — the entire scope of this checkpoint.
  - Task 1 adds `GateSample`, `MAX_GATE_WALK_SAMPLES`, and `gate_walk` (new pure primitive).
  - Task 2 adds `execute_move_kingstep_oracle` (`#[cfg(test)]`, a frozen verbatim copy of
    today's executor) — temporary scaffolding for Task 4, deleted in Task 6.
  - Task 3 refactors `execute_move` itself onto `gate_walk`, updates `MoveOutcome`/
    `MoveReject` doc comments, and updates the two pre-existing tests whose assertions the
    guard relaxation (Global Constraints, 3rd bullet) supersedes.
  - Task 4 adds the differential parity test suite (the load-bearing parity proof).
  - Task 5 adds continuous (any-angle, non-king-step) unit test coverage.
  - Task 6 (dispatched LAST, gated on the whole-branch buddy-check below converging PASS)
    freezes Task 4's cases as literal fixtures and deletes the oracle.
- **No other file changes.** `src/server/src/ws/room.rs`'s `Room::execute_move` needs no
  logic change; its existing tests (`execute_move_commits_stop_and_returns_render_path`,
  `execute_move_rejects_a_moving_token`, `execute_move_truncates_at_a_wall_atomically`,
  `execute_move_authoritative_field_arrests_a_region_the_players_route_never_saw`,
  `execute_move_revealed_union_allows_explored_cell`) staying green throughout is part of the
  caller-seam regression proof — run the full `cargo test -p shadowcat --lib ws::room` suite
  after Task 3 as a checkpoint (folded into Task 3's steps below).

---

## Model/Effort directives

- **Plan-writer:** this plan was written **mainline in the current session** (Sonnet 5, effort
  high) per explicit user direction — the user typed "You write the plan" immediately after
  switching `/model` to Sonnet 5, directly resolving the writing-plans tier-switch checkpoint
  in favor of option (a) (mainline continuation), not a dispatched `sdd-plan-writer-*` agent.
- **Execution tier (recommended, not yet confirmed by the user):** standard `shadowcat-coder`
  (sonnet, effort medium) per task, escalating to `shadowcat-coder-opus` on any
  BLOCKED/DONE_WITH_CONCERNS report — matches the M10f-0/M10f-1 cadence. Reviewers:
  `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` per task, escalating to their `-opus`
  twins if a review reads shallow/uncertain or a diff is judged genuinely tough (Tasks 1, 3, 4,
  5 are pre-authorized for the heavier two-reviewer buddy-check debate — see below, not just
  the standard escalation path).

## Buddy-check directives

Per explicit user direction: this checkpoint is security-critical (a defect here is a
movement-into-fog **leak**, not a cosmetic bug) and should pre-authorize buddy-check on
essentially every substantive task, mirroring the M10f-1 experience where buddy-check caught 6
real Critical/Important defects on a comparably-risky refactor.

- **Pre-authorized for per-task buddy-check** (two independent blind reviewers, debate to
  convergence): **Tasks 1, 3, 4, 5** — every task that introduces or changes actual gating/walk
  logic, or the test coverage proving it.
- **Standard single-reviewer gate only:** **Task 2** (a verbatim copy of already-reviewed code,
  zero new logic — low marginal value from a debate) and **Task 6** (mechanical
  freeze-and-delete cleanup, dispatched only after the whole-branch buddy-check below has
  already converged PASS on the substantive tasks).
- **Whole-branch buddy-check (mandatory):** after Task 5 and before Task 6, run a whole-branch
  buddy-check (two independent blind opus reviewers, debate to convergence) across the
  assembled Tasks 1–5 diff. Specifically direct the reviewers to check:
  1. **Grid-input identity** in `gate_walk` — confirm no subdivision path can ever fire on a
     real grid-A*-emitted king-step path (the structural parity claim).
  2. **The cell-entry-dedup region/cost logic** against a continuous path that grazes a cell
     corner or re-enters a cell it already passed through.
  3. **The coarse `render_path` reconstruction's `authored_idx` fallback branch** (the
     mid-subdivision-truncation case) for an off-by-one at a segment boundary.
  4. **`MoveReject::TooLong`'s redefinition** — confirm the new gate-walk-sample cap cannot
     silently admit a pathological path the old vertex-count cap used to reject.

  **Task 6 is gated on this convergence** — do not dispatch it until the whole-branch
  buddy-check reports PASS (or all findings are fixed and reverified).
- **Reviewed skill-update gate target:** `shadowcat-codebase-scene-rendering` — the
  `move_exec` seam becomes polyline-shaped and engine-agnostic; the new `gate_walk` primitive;
  the guard-relaxation and DoS-bound change; the differential-oracle technique's
  introduction-then-removal. Confirm via `shadowcat-spec-reviewer` before merge.

---

## Task 1: The `gate_walk` primitive

**Files:**
- Modify: `src/server/src/scene/move_exec.rs` (add above `execute_move`, after the existing
  `MAX_MOVE_PATH`/`EPS` constants)
- Test: same file, `#[cfg(test)] mod tests` (add new test functions; do not touch existing ones
  in this task)

**Interfaces:**
- Produces: `pub(crate) struct GateSample { pub pos: (f64, f64), pub authored_idx: Option<usize> }`,
  `pub(crate) const MAX_GATE_WALK_SAMPLES: usize`, `pub(crate) fn gate_walk(path: &[(f64,f64)], cell: f64) -> Option<Vec<GateSample>>`.
  Consumed by Task 3's refactored `execute_move` and Task 2's oracle is NOT changed by this
  task (the oracle doesn't exist yet).

- [ ] **Step 1: Write the failing tests**

```rust
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
    fn gate_walk_on_empty_path_returns_empty() {
        let walk = gate_walk(&[], 100.0).unwrap();
        assert!(walk.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat --lib scene::move_exec::tests::gate_walk -- --nocapture`
Expected: FAIL to compile — `gate_walk`/`GateSample`/`MAX_GATE_WALK_SAMPLES` not defined.

- [ ] **Step 3: Implement `GateSample`, `MAX_GATE_WALK_SAMPLES`, and `gate_walk`**

Add this above `execute_move` (after the existing `MAX_MOVE_PATH`/`EPS` constants — leave
those untouched for now, Task 3 removes `MAX_MOVE_PATH`):

```rust
/// DoS guard for `gate_walk` (§4.3): a walk requiring more than this many dense samples is
/// rejected outright, never truncated. Arc-length/cell-count based — a single continuous
/// segment can be arbitrarily long, so an authored-vertex-count cap is not the right invariant
/// (unlike the pre-M10f-2 `MAX_MOVE_PATH`, which this constant replaces).
pub(crate) const MAX_GATE_WALK_SAMPLES: usize = 4096;

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
/// preserving already-≤1-cell input segments EXACTLY — this is what makes the gate walk an
/// IDENTITY on grid input (cell-center vertices, ≤1 cell apart on every axis including
/// diagonals), and therefore what makes grid-parity a property of the code shape rather than
/// something proven only by testing (design spec §4.1).
///
/// `None` (fail-closed) on: a non-finite `path` coordinate, a non-finite or non-positive
/// `cell`, or an emitted sample count that would exceed `MAX_GATE_WALK_SAMPLES`.
pub(crate) fn gate_walk(path: &[(f64, f64)], cell: f64) -> Option<Vec<GateSample>> {
    if !cell.is_finite() || cell <= 0.0 {
        return None;
    }
    if path.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return None;
    }
    if path.is_empty() {
        return Some(Vec::new());
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
        let k_f = if cheby <= cell { 1.0 } else { (cheby / cell).ceil() };
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat --lib scene::move_exec::tests::gate_walk -- --nocapture`
Expected: all 8 tests PASS.

- [ ] **Step 5: Format and lint**

Run: `cargo fmt && cargo clippy -p shadowcat --lib -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/move_exec.rs
git commit -m "feat(m10f-2): add the gate_walk polyline-subdivision primitive"
```

---

## Task 2: Retain the king-step executor as a differential-oracle

**Files:**
- Modify: `src/server/src/scene/move_exec.rs` (add a `#[cfg(test)]` copy immediately after
  the existing `execute_move`, before Task 3 touches `execute_move` itself)

**Interfaces:**
- Consumes: nothing new (a verbatim copy of today's logic).
- Produces: `#[cfg(test)] pub(crate) fn execute_move_kingstep_oracle(ecs, scene, token, path,
  restriction, visible, cell) -> Result<MoveOutcome, MoveReject>` — same signature as
  `execute_move`. Consumed by Task 4's differential test. Deleted in Task 6.

- [ ] **Step 1: Copy today's `execute_move` body verbatim into a new, `#[cfg(test)]`-gated function**

Add this immediately after the current `execute_move` function (which Task 3 will refactor —
this copy captures its behavior exactly as it exists today, before any refactor):

```rust
/// Frozen differential-test oracle (M10f-2, TEMPORARY): a verbatim copy of the pre-refactor
/// king-step executor, kept ONLY so Task 4's differential test can prove the new sampled
/// executor agrees with it on every grid input. Deleted once that parity is proven and frozen
/// as literal fixtures (design spec §4.4) — do NOT let this become a permanently-maintained
/// second executor; that would reintroduce exactly the engine fork this refactor exists to
/// avoid.
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
```

Note: this references `MAX_MOVE_PATH`, which still exists at this point in the plan (Task 3
removes it). If Task 3 has not yet run, this compiles as-is.

- [ ] **Step 2: Compile-check — confirm zero behavior change**

Run: `cargo check -p shadowcat --lib`
Expected: compiles clean (two near-identical functions coexist; `execute_move_kingstep_oracle`
is unused so far, which is fine under `#[cfg(test)]` — it will be consumed in Task 4).

- [ ] **Step 3: Run the existing test suite to confirm nothing broke**

Run: `cargo test -p shadowcat --lib scene::move_exec`
Expected: all pre-existing tests PASS unchanged (this task touches no production logic).

- [ ] **Step 4: Format and lint**

Run: `cargo fmt && cargo clippy -p shadowcat --lib -- -D warnings`
Expected: clean. (If clippy flags the not-yet-called oracle as dead code, this resolves once
Task 4 calls it; if it flags it now, add a temporary `#[allow(dead_code)]` and remove it in
Task 4 when the function gains a caller — check before adding to avoid an unnecessary allow.)

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/move_exec.rs
git commit -m "test(m10f-2): freeze the king-step executor as a differential-test oracle"
```

---

## Task 3: Refactor `execute_move` onto `gate_walk`

**Files:**
- Modify: `src/server/src/scene/move_exec.rs`

**Interfaces:**
- Consumes: `gate_walk`/`GateSample` (Task 1).
- Produces: the SAME external signature and `MoveOutcome` shape — no change for callers.
  `MoveOutcome` gains `#[derive(Debug)]` (needed for Task 4's assert failure messages).

- [ ] **Step 1: Update `MoveOutcome` and `MoveReject` doc comments/derives**

Replace the `MoveOutcome` struct definition with:

```rust
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
```

Replace the `MoveReject` enum with:

```rust
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
```

- [ ] **Step 2: Replace the `execute_move` function body**

Replace the ENTIRE current `execute_move` function (signature through closing brace, i.e.
everything from `pub(crate) fn execute_move(` through the code that currently returns
`Ok(MoveOutcome { ... })`, NOT including the doc comment above it, NOT touching
`execute_move_kingstep_oracle` added in Task 2) with:

```rust
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
                last_region_cell = next_cell;
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
```

- [ ] **Step 3: Remove the now-unused `MAX_MOVE_PATH` constant**

Delete the `MAX_MOVE_PATH` constant definition (it is superseded by `MAX_GATE_WALK_SAMPLES`
from Task 1). Confirm no other production code in this file still references it — Task 2's
oracle (already committed) DOES still reference it, which is expected and correct (the oracle
is a frozen copy of pre-M10f-2 behavior, including its DoS bound); keep `MAX_MOVE_PATH`
available to the oracle by moving its definition into the oracle's own `#[cfg(test)]` scope
rather than deleting it outright:

```rust
// (Move this constant from its old top-level location into the oracle's cfg(test) block,
// immediately above `execute_move_kingstep_oracle`, so it is fully scoped to test-only code:)
#[cfg(test)]
const MAX_MOVE_PATH: usize = 256;
```

- [ ] **Step 4: Update the two pre-existing tests the guard relaxation supersedes**

Delete `rejects_overlong_or_nonadjacent_path` and `rejects_too_long_path` (their assertions
contradict the new, intended behavior) and replace them with:

```rust
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
```

- [ ] **Step 5: Run the full pre-existing + updated test suite for this file**

Run: `cargo test -p shadowcat --lib scene::move_exec`
Expected: ALL tests pass — every pre-existing test EXCEPT the two just replaced
(`full_clear_path_reaches_goal`, `wall_truncates_at_last_legal_cell`,
`unseen_cell_truncates_under_visible_restriction`, `revealed_mode_uses_caller_supplied_union_mask`,
`unrestricted_ignores_mask_but_not_walls`, `rejects_path_not_starting_at_token`,
`rejects_empty_path`, `rejects_unknown_token`, `unrestricted_full_path_no_walls`,
`impassable_region_stops_before_entry_like_a_wall`,
`arrest_region_stops_at_entry_including_final_step`,
`terrain_region_accumulates_weighted_cost`,
`authoritative_field_springs_a_secret_region_a_player_was_routed_through`) passes UNCHANGED
against the refactored executor — this is the empirical half of the grid-parity proof,
alongside Task 1's structural identity guarantee.

- [ ] **Step 6: Run the caller-seam regression suite**

Run: `cargo test -p shadowcat --lib ws::room`
Expected: `execute_move_commits_stop_and_returns_render_path`,
`execute_move_rejects_a_moving_token`, `execute_move_truncates_at_a_wall_atomically`,
`execute_move_authoritative_field_arrests_a_region_the_players_route_never_saw`, and
`execute_move_revealed_union_allows_explored_cell` all PASS unchanged — proving `Room::execute_move`
needs no code change (Global Constraints, 2nd bullet).

- [ ] **Step 7: Format and lint**

Run: `cargo fmt && cargo clippy -p shadowcat --lib -- -D warnings`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add src/server/src/scene/move_exec.rs
git commit -m "refactor(m10f-2): unify execute_move onto the gate_walk dense gate"
```

---

## Task 4: Differential parity test suite (load-bearing)

**Files:**
- Modify: `src/server/src/scene/move_exec.rs` (add new fixture helper + test to
  `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `execute_move` (Task 3), `execute_move_kingstep_oracle` (Task 2).
- Produces: nothing new for other tasks to consume directly, but Task 6 reads this task's
  `DiffCase` list and literal outcomes to build its frozen fixtures.

- [ ] **Step 1: Add a combined scene-fixture helper and the differential test**

```rust
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
            entity_doc(10, 0, "scene", json!({ "grid": { "size": 100 } })),
            entity_doc(11, 10, "token", json!({ "x": 0.0, "y": 0.0 })),
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
            let mut r = region_doc(13, 10, behavior, cost, x0, y0, x1, y1);
            if secret_region {
                r.permissions
                    .property_overrides
                    .insert("/system".into(), crate::data::document::Visibility::GmOnly);
            }
            docs.push(r);
        }
        (SceneEcs::from_documents(docs, 0), scene_id, token_id)
    }

    struct DiffCase {
        label: &'static str,
        wall: Option<(f64, f64, f64, f64)>,
        region: Option<(&'static str, f64, f64, f64, f64, f64)>,
        secret_region: bool,
        visible: BTreeSet<(i32, i32)>,
        restriction: MovementRestriction,
        path: Vec<(f64, f64)>,
    }

    fn assert_outcomes_match(
        label: &str,
        new: &Result<MoveOutcome, MoveReject>,
        oracle: &Result<MoveOutcome, MoveReject>,
    ) {
        match (new, oracle) {
            (Ok(n), Ok(o)) => {
                assert_eq!(n.stop, o.stop, "{label}: stop mismatch ({n:?} vs {o:?})");
                assert_eq!(
                    n.render_path, o.render_path,
                    "{label}: render_path mismatch"
                );
                assert_eq!(n.truncated, o.truncated, "{label}: truncated mismatch");
                assert!(
                    (n.cost - o.cost).abs() < 1e-9,
                    "{label}: cost mismatch ({} vs {})",
                    n.cost,
                    o.cost
                );
            }
            (Err(ne), Err(oe)) => {
                assert_eq!(ne, oe, "{label}: reject variant mismatch")
            }
            _ => panic!("{label}: new={new:?} oracle={oracle:?} — Ok/Err disagreement"),
        }
    }

    #[test]
    fn differential_parity_king_step_paths_match_oracle_across_scenarios() {
        let cases = vec![
            DiffCase {
                label: "clear scene, full visible, straight path",
                wall: None,
                region: None,
                secret_region: false,
                visible: visible_grid(3),
                restriction: MovementRestriction::Visible,
                path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            },
            DiffCase {
                label: "clear scene, Unrestricted, empty mask",
                wall: None,
                region: None,
                secret_region: false,
                visible: BTreeSet::new(),
                restriction: MovementRestriction::Unrestricted,
                path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            },
            DiffCase {
                label: "wall blocks second step, Visible",
                wall: Some((50.0, 50.0, 150.0, 50.0)),
                region: None,
                secret_region: false,
                visible: visible_grid(4),
                restriction: MovementRestriction::Visible,
                path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            },
            DiffCase {
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
            },
            DiffCase {
                label: "full mask allowed under Revealed",
                wall: None,
                region: None,
                secret_region: false,
                visible: visible_grid(3),
                restriction: MovementRestriction::Revealed,
                path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            },
            DiffCase {
                label: "impassable region stops before entry",
                wall: None,
                region: Some(("impassable", 1.0, 50.0, 0.0, 150.0, 100.0)),
                secret_region: false,
                visible: visible_grid(3),
                restriction: MovementRestriction::Unrestricted,
                path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            },
            DiffCase {
                label: "arrest region stops at entry on final step",
                wall: None,
                region: Some(("arrest", 1.0, 50.0, -50.0, 150.0, 50.0)),
                secret_region: false,
                visible: visible_grid(3),
                restriction: MovementRestriction::Unrestricted,
                path: vec![(0.0, 0.0), (100.0, 0.0)],
            },
            DiffCase {
                label: "terrain region accrues cost",
                wall: None,
                region: Some(("terrain", 2.5, 50.0, 0.0, 150.0, 100.0)),
                secret_region: false,
                visible: visible_grid(3),
                restriction: MovementRestriction::Unrestricted,
                path: vec![(0.0, 0.0), (100.0, 0.0)],
            },
            DiffCase {
                label: "authoritative field springs a secret impassable region under Visible",
                wall: None,
                region: Some(("impassable", 1.0, 50.0, 0.0, 150.0, 100.0)),
                secret_region: true,
                visible: visible_grid(3),
                restriction: MovementRestriction::Visible,
                path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            },
            DiffCase {
                label: "diagonal 3-step king path, full visible",
                wall: None,
                region: None,
                secret_region: false,
                visible: visible_grid(4),
                restriction: MovementRestriction::Visible,
                path: vec![
                    (0.0, 0.0),
                    (100.0, 100.0),
                    (200.0, 200.0),
                    (300.0, 100.0),
                ],
            },
        ];

        for case in &cases {
            let (ecs, scene, token) =
                scene_with_wall_and_region(case.wall, case.region, case.secret_region);
            let new_result = execute_move(
                &ecs,
                scene,
                token,
                &case.path,
                case.restriction,
                &case.visible,
                100.0,
            );
            let oracle_result = execute_move_kingstep_oracle(
                &ecs,
                scene,
                token,
                &case.path,
                case.restriction,
                &case.visible,
                100.0,
            );
            assert_outcomes_match(case.label, &new_result, &oracle_result);
        }
    }
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p shadowcat --lib scene::move_exec::tests::differential_parity -- --nocapture`
Expected: PASS — all 10 scenarios agree between the new executor and the oracle.

- [ ] **Step 3: Format and lint**

Run: `cargo fmt && cargo clippy -p shadowcat --lib -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/server/src/scene/move_exec.rs
git commit -m "test(m10f-2): differential parity suite vs the king-step oracle"
```

---

## Task 5: Continuous (any-angle, non-king-step) unit tests

**Files:**
- Modify: `src/server/src/scene/move_exec.rs` (add new tests to `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `execute_move` (Task 3) only — these paths are not king-step, so no oracle
  comparison is possible (the oracle rejects them by shape); these are pure behavioral tests.

- [ ] **Step 1: Write the tests**

```rust
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
                entity_doc(10, 0, "scene", json!({ "grid": { "size": 100 } })),
                entity_doc(11, 10, "token", json!({ "x": 0.0, "y": 0.0 })),
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
        )
        .unwrap();
        assert!(out.truncated, "must stop before crossing the wall mid-segment");
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
                entity_doc(10, 0, "scene", json!({ "grid": { "size": 100 } })),
                entity_doc(11, 10, "token", json!({ "x": 0.0, "y": 0.0 })),
                region_doc(12, 10, "impassable", 1.0, 300.0, -50.0, 500.0, 150.0),
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
                entity_doc(10, 0, "scene", json!({ "grid": { "size": 100 } })),
                entity_doc(11, 10, "token", json!({ "x": 0.0, "y": 0.0 })),
                region_doc(12, 10, "arrest", 1.0, 300.0, -50.0, 500.0, 150.0),
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
        )
        .unwrap();
        assert!(out.truncated);
        assert_eq!(
            out.stop,
            (300.0, 0.0),
            "arrest stops AT entry into cell (3,0), not before it"
        );
    }
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p shadowcat --lib scene::move_exec::tests::continuous -- --nocapture`
Expected: all 4 tests PASS.

- [ ] **Step 3: Format and lint**

Run: `cargo fmt && cargo clippy -p shadowcat --lib -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/server/src/scene/move_exec.rs
git commit -m "test(m10f-2): continuous any-angle executor coverage"
```

---

## [GATED — dispatch only after the whole-branch buddy-check below converges PASS]

## Task 6: Freeze the differential fixtures and delete the oracle

**Do not start this task until the mandatory whole-branch buddy-check (see "Buddy-check
directives" above) has converged PASS on Tasks 1–5**, including any fixes it required being
reverified.

**Files:**
- Modify: `src/server/src/scene/move_exec.rs`

**Interfaces:**
- Consumes: Task 4's `DiffCase` list and the values it already proved correct via the oracle.
- Produces: nothing new — this task only removes the oracle and freezes evidence.

- [ ] **Step 1: Add the frozen-fixture test ALONGSIDE the still-present oracle test**

Each `expected` value below was independently derived by tracing `execute_move`'s logic
against each scenario (cross-checked against the pre-existing named unit tests where they
overlap, e.g. `full_clear_path_reaches_goal`, `wall_truncates_at_last_legal_cell`,
`terrain_region_accumulates_weighted_cost`). Note the non-obvious fact this tracing surfaced:
`RegionField::terrain_multiplier` returns a **default `1.0` per step even with no region
present at all** (the `_ => 1.0` match arm), so `cost` is `1.0 × (successful steps taken)` as
a baseline, not `0.0`, whenever the walk isn't blocked before accruing it. Add this test to
`#[cfg(test)] mod tests`, directly below the existing (still-present, not yet removed)
`differential_parity_king_step_paths_match_oracle_across_scenarios`:

```rust
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

    /// Frozen parity fixtures (M10f-2): the SAME 10 scenarios as
    /// `differential_parity_king_step_paths_match_oracle_across_scenarios`, but compared
    /// against literal expected values instead of a live oracle call. This test is the
    /// permanent parity regression that survives after `execute_move_kingstep_oracle` (and
    /// the oracle-comparison test above) are deleted later in this task.
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
                label: "diagonal 3-step king path, full visible",
                wall: None,
                region: None,
                secret_region: false,
                visible: visible_grid(4),
                restriction: MovementRestriction::Visible,
                path: vec![(0.0, 0.0), (100.0, 100.0), (200.0, 200.0), (300.0, 100.0)],
                expected: ExpectedOutcome {
                    stop: (300.0, 100.0),
                    render_path: vec![
                        (0.0, 0.0),
                        (100.0, 100.0),
                        (200.0, 200.0),
                        (300.0, 100.0),
                    ],
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
```

- [ ] **Step 2: Run BOTH the oracle-comparison test and the new frozen test — this is the
      safety net**

Run: `cargo test -p shadowcat --lib scene::move_exec::tests -- --nocapture`
Expected: BOTH `differential_parity_king_step_paths_match_oracle_across_scenarios` (still
live-comparing against the oracle) AND
`frozen_parity_king_step_paths_match_previously_oracle_verified_outcomes` (comparing against
the literals above) PASS. If the frozen test fails, the hand-derived `expected` values above
have an error — fix them using the still-present oracle test as ground truth (it is not yet
deleted) before proceeding; do not delete the oracle until the frozen test passes on its own.

- [ ] **Step 3: Delete the oracle and its now-superseded differential test**

Remove, in this order:
1. `execute_move_kingstep_oracle` (added in Task 2) and its `#[cfg(test)] const
   MAX_MOVE_PATH`.
2. The `differential_parity_king_step_paths_match_oracle_across_scenarios` test (added in
   Task 4) and the `assert_outcomes_match` helper it used (both now fully superseded by the
   frozen test added in Step 1 above).

Keep `scene_with_wall_and_region` (still used by the frozen test) and `region_doc` (still used
by several pre-existing tests) — do not remove either.

- [ ] **Step 4: Run the full suite to confirm nothing depends on the oracle anymore**

Run: `cargo test -p shadowcat --lib scene::move_exec`
Expected: PASS, including the frozen-fixture test — with zero references to
`execute_move_kingstep_oracle`, the deleted `MAX_MOVE_PATH`, or `assert_outcomes_match`
remaining anywhere in the file.

Run: `cargo test -p shadowcat --lib ws::room`
Expected: PASS unchanged (Task 3's caller-seam guarantee still holds).

- [ ] **Step 5: Format and lint**

Run: `cargo fmt && cargo clippy -p shadowcat --lib -- -D warnings`
Expected: clean.

- [ ] **Step 6: Full crate suite as a final sanity pass**

Run: `cargo test -p shadowcat`
Expected: PASS, green across the whole crate.

- [ ] **Step 7: Commit**

```bash
git add src/server/src/scene/move_exec.rs
git commit -m "refactor(m10f-2): freeze differential fixtures, remove the oracle"
```

---

## Completion checklist (before merge)

- [ ] All 6 tasks committed, per-task reviewed (Tasks 1, 3, 4, 5 buddy-checked; Tasks 2, 6
      standard single-reviewer).
- [ ] Whole-branch buddy-check converged PASS across Tasks 1–5 before Task 6 was dispatched.
- [ ] `shadowcat-codebase-scene-rendering` updated (the `move_exec` seam + `gate_walk` +
      guard/DoS-bound changes + the oracle technique) and confirmed accurate by
      `shadowcat-spec-reviewer`.
- [ ] `docs/PLAN.md`'s M10f entry updated: mark M10f-2 DONE, matching the M10f-0/M10f-1 entry
      style (branch name, commit range, what shipped, buddy-check summary, "Next = M10f-3").
- [ ] Merge `m10f-2-unified-movement-executor` --no-ff to LOCAL `main` (merge gate = full
      M10f, per the standing M10f convention) — do not push unless the user directs otherwise.
