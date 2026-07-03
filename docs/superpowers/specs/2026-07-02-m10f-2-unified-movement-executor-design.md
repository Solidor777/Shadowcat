# M10f-2 — Unify the Movement Executor — Design

> Checkpoint of the M10 Tokens milestone, M10f (continuous/navmesh movement)
> decomposition. Parent spec:
> `2026-07-02-m10f-continuous-navmesh-movement-design.md` §6.2 (unified executor)
> + §13 (per-cell parity invariant). Builds directly on the server-authoritative
> movement model (`move_exec::execute_move`, `move_stream::sample_path`,
> `Room::execute_move` — M10e-5 → M1/M2/M3) and the `movementModel` dispatch +
> polyanya router (M10f-1).
>
> **Status: design approved (user, 2026-07-02).** Server-only executor refactor;
> the highest-risk M10f checkpoint per the parent spec §12.

## 1. Goal

Refactor `move_exec::execute_move` from a **king-step-per-authored-cell** walk into
a **sampled-polyline executor** that is **engine-agnostic**: it accepts any polyline
(grid A\* emits cell-center vertices ≤1 cell apart; polyanya emits any-angle
vertices arbitrarily far apart), gates it against the **same** per-cell
visibility/region machinery M1/M2/M3 already use, and produces **identical**
admit/deny/arrest/cost outcomes to today's executor for every grid input.

This is the load-bearing prerequisite for continuous movement execution (M10f-3):
grid §13 parity must be **proven under heavy buddy-check before** continuous
execution is layered on top. Grid behavior on real (router-emitted) traffic is
unchanged; the executor merely becomes polyline-shaped and stops caring which engine
produced the polyline.

## 2. Why this is its own checkpoint (context)

The parent spec (§12) split this out from continuous execution deliberately: the
refactor touches **working, security-critical, server-authoritative M1/M2/M3 code**
(the move gate that is the sole thing stopping a player moving a token into fog).
A regression here is a movement-into-fog **leak**, not a cosmetic bug. Isolating the
executor refactor lets its grid-parity proof land and be reviewed **before** any
continuous path can reach it.

Verified against the code at design time:

- `execute_move` (`scene/move_exec.rs`) walks authored vertices `path[i-1] → path[i]`
  directly, **rejecting** any pair more than 1 cell apart (`MoveReject::Degenerate`,
  the king-step guard). It gates each step via `movement::supercover_cells(prev,
  next, cell) ⊆ visible` (§13 parity with `Room::publish`), then the region field
  (impassable stops before entry, arrest stops at entry, terrain accrues cost). It
  returns `MoveOutcome { stop, render_path, truncated, cost }`.
- `move_stream::sample_path(path, cell, duration_ms)` is already engine-agnostic but
  tuned for **animation**: ~`SAMPLES_PER_CELL = 3` per cell, **capped at
  `MAX_VISION_SAMPLES = 96`**. It is used **downstream** in `Room::execute_move` for
  the `MoveStream` broadcast — **not** for the gate.
- `Room::execute_move` (`ws/room.rs`) resolves gate inputs, calls `execute_move`,
  computes `distance`/`duration` from `render_path`, samples the stream via
  `sample_path`, computes per-sample `mover_vision`, and atomically commits `stop`.

## 3. The crux the parent spec left open

Parent §6.2 says "arc-length-sample via `move_stream::sample_path` **(or a shared
sub-routine)**." Reading the code shows `sample_path` is **not** directly reusable
for the gate, because the gate and the animation stream have **conflicting**
sampling requirements:

| Concern | Gate walk (security) | `sample_path` (animation) |
|---|---|---|
| Spacing | **must** be ≤1 cell, always | ~3/cell for smoothness |
| Count | uncapped (bounded by arc-length) | **capped at 96** for bandwidth |
| On a grid step | must stay identity (parity) | splits into thirds |
| Cost model | per-cell-entry | irrelevant |

Fusing them (reuse `sample_path` for the gate) would: resample grid cell-steps into
thirds (grid parity becomes a proof obligation, not a code property); let the 96-cap
put consecutive gate samples **more than 1 cell apart** past ~32 cells (a
`supercover_cells` gap = a **fog hole**); and require per-cell dedup to avoid
triple-counting terrain cost. So the two samplers stay **separate, independently
evolvable code**. This is the durability decision (§4.1).

## 4. Locked decisions (user, 2026-07-02)

### 4.1 Gate-walk sampling — subdivide-only, identity on grid

The executor turns its input polyline into the ≤1-cell gate walk via a new pure
helper that **subdivides only >1-cell segments and preserves already-≤1-cell
segments exactly**. For a grid path (cell centers, Chebyshev exactly 1 cell apart on
both orthogonal and diagonal steps) the output **equals the input**, so the per-step
gate loop is **byte-identical to today** — grid parity is a property of the code
shape, not something proven into existence. The security gate sampler and the
animation sampler (`sample_path`) remain decoupled: a future change to animation
smoothness or bandwidth cannot move the security boundary.

**Rejected alternatives:** (a) reuse `sample_path` for the gate — fuses conflicting
requirements, the 96-cap becomes a latent fog hole, parity becomes proof-by-test;
(b) keep the king-step executor and snap continuous polylines to cell vertices
upstream — forks the executor (parity-drift risk) and destroys the any-angle stop
precision continuous movement exists for, contradicting parent decision #3.

### 4.2 Engine-agnostic executor — guard relaxes to a subdivision invariant

The executor **never branches on `movementModel`**. The current king-step
**input-rejection** guard (reject any >1-cell authored jump) relaxes to an internal
**subdivision invariant** (every *gate-walk sample* pair is ≤1 cell by
construction). A >1-cell authored segment is **subdivided and gated per cell**, not
rejected — admitted only if every crossed cell is visible and wall/region-clear,
i.e. **identical to the client having sent the explicit intermediate points**, which
was always allowed. No new capability is granted; security lives entirely in the
per-cell gate, never in the shape check.

**Net effect on real traffic: zero.** The grid router only ever emits king-step
paths, so every legal route behaves exactly as today. The sole change is that a
malformed/hand-crafted non-king-step grid path is now handled **safely by the gate**
instead of blanket-rejected by shape.

### 4.3 DoS bound — arc-length-based

The `MAX_MOVE_PATH = 256` authored-vertex-count cap is replaced by a cap on the
total **gate-walk sample count** (equivalently, arc-length / cell). The gate-walk
helper returns `None` (→ caller fails closed to `Forbidden`) when a path would
subdivide past the cap. Vertex-count is meaningless when one continuous segment can
be arbitrarily long; length is the correct invariant.

### 4.4 Parity proof — differential oracle, delete after review

Today's king-step executor is **retained as a `#[cfg(test)]` oracle** for the
duration of the checkpoint. A differential test enumerates grid scenes (walls ×
regions × masks × restriction modes) × king-step paths and asserts the new sampled
executor and the oracle produce **identical** `MoveOutcome` (stop, render_path,
truncated, cost). It runs through development, buddy-check, and whole-branch review;
its verified outcomes are frozen as committed fixture assertions; then the **oracle
is deleted** — keeping a second full executor permanently would reintroduce exactly
the fork the architecture avoids. Empirical parity during development; structural
identity (§4.1) + committed fixtures as the durable guarantee.

## 5. Server — the `gate_walk` primitive (`scene/move_exec.rs` or a sibling, pure)

```
gate_walk(path: &[(f64,f64)], cell: f64) -> Option<Vec<(f64,f64)>>
```

- **Contract:** every consecutive pair in the output is ≤1 cell apart on each axis
  (Chebyshev ≤ `cell`); already-≤1-cell input segments are preserved exactly
  (identity on grid input, incl. diagonal cell-steps).
- **Algorithm:** for each authored segment `(p[i-1], p[i])`, let `k =
  ceil(chebyshev(p[i-1], p[i]) / cell)`; `k ≤ 1` emits `p[i]` unchanged, `k > 1`
  emits `k-1` equally-spaced interior points then `p[i]`. Always starts with `p[0]`.
- **Fail closed** (`None`): any non-finite coordinate, `cell ≤ 0`, or total emitted
  sample count > the DoS cap (§4.3). Mirrors `supercover_cells`/`sample_path`
  fail-closed convention (under-permit, never over-permit).

## 6. Server — refactored executor

`execute_move` keeps its signature (`path: &[(f64,f64)]`, restriction, `visible`,
`cell`) and its `MoveOutcome { stop, render_path, truncated, cost }` result shape.
Internally:

- **Dense gate, coarse result.** It builds the *dense* `gate_walk` output and runs
  the per-step gate over it; it **returns** the *coarse* legal-prefix polyline in
  `render_path` (authored vertices fully traversed + the exact `stop` point). For
  grid, `stop` is a cell center coinciding with an authored vertex, so `render_path`
  equals today's authored-vertex prefix byte-for-byte and the downstream
  `sample_path` re-densifies it identically.
- **Per-step gate, unchanged primitives (§13).** Each gate-walk step runs the **same**
  step-1 wall gate (`blocks_move`), step-2 vision-mask gate (`supercover_cells(prev,
  next, cell) ⊆ visible`, skipped for `Unrestricted`), and step-3 region gate. No new
  gate primitive; `supercover_cells` remains THE gate. A `None` from `supercover_cells`
  fails closed (stop at prev), exactly as today.
- **Cost + regions keyed on cell-entry.** Terrain cost, impassable-stop-before-entry,
  and arrest-stop-at-entry all key on **cell transitions** across the walk (the cell
  index changing between samples), not per sample. A grid step enters exactly one new
  cell, reproducing today's `cost += terrain_multiplier(next)` / impassable / arrest
  semantics exactly; continuous accrues the honest per-cell base that M10f-4 extends.
  The authoritative region field (`region_field(scene, None)`) is still read once
  before the walk — `move_exec` springs every region (incl. secret) regardless of the
  mover's preview (unchanged from M10g).
- **Truncation.** Stop lands on the last safe gate-walk sample. Grid: a cell center
  (parity). Continuous: a cell-granular point up to ~1 cell before the true boundary —
  the accepted caveat (parent §3.2); harmless here since no continuous path is executed
  end-to-end this checkpoint.
- **Guard.** The king-step input-rejection is removed (§4.2); `MoveReject::Degenerate`
  now covers only non-finite coords, bad start (`path[0]` ≠ committed position within
  `EPS`), and the `gate_walk` fail-closed `None` (over-cap / degenerate). `EmptyPath`,
  `TooLong` (recast against the §4.3 bound), `NotAToken` are unchanged in spirit.

## 7. Server — caller seam unchanged

`Room::execute_move` requires **no logic change** (comment updates only): it calls
the same `execute_move`, reads the same `MoveOutcome` fields, computes
`distance`/`duration` from `render_path`, samples the stream via `sample_path`, builds
`mover_vision`, and commits `stop`. The existing room-level move tests staying green is
part of the parity evidence. All `MoveReject` variants still map to
`DataError::Forbidden`, so the guard relaxation (§4.2) is invisible at the seam.

## 8. Protocol & client

**None.** No new or changed frames; no client change. Continuous `MoveRequest`s
remain disabled client-side via M10f-1's `commitRoute` gate — this checkpoint makes
the executor *ready* for continuous input and unit-tests it, but does **not** wire
continuous execution end-to-end (M10f-3).

## 9. Scope — explicit exclusions

1. **Continuous execution end-to-end** (client no-snap place/move, continuous
   `MoveRequest` reaching the executor, `MoveStream` over any-angle paths) → **M10f-3**.
   M10f-2 proves the executor gates continuous polylines correctly at the **unit**
   level only.
2. **Regions on the navmesh** (terrain cost-layers, impassable mesh holes on the
   continuous router) → **M10f-4**. M10f-2's cost model is the grid-shaped
   per-cell-entry base; it is not the polyanya cost-layer.
3. **Continuous cost refinement** (Euclidean-integrated terrain cost) → M10f-3/M10f-4.
4. No change to `sample_path`, the `MoveStream` egress clip, or any secrecy machinery.

## 10. Cross-platform

Pure geometry; `#[cfg]`-free; no I/O, no path handling. The three-OS CI matrix proves
portability. No new dependency (reuses `supercover_cells`, `region_field`, and the
existing sampler primitives).

## 11. Testing

- **`gate_walk` units:** identity on ≤1-cell input (orthogonal + diagonal cell-steps);
  subdivision produces ≤1-cell spacing on long/any-angle segments; fail-closed on
  non-finite / `cell ≤ 0` / over-cap.
- **Executor parity suite (load-bearing, mirrors M10e-4):** admit/deny/arrest/cost
  across env / global-illumination / darkvision / LOS+wall × `Visible`/`Revealed`/
  `Unrestricted`.
- **Differential oracle (§4.4):** enumerated grid scenes × king-step paths — new
  sampled executor `MoveOutcome` == retained king-step oracle `MoveOutcome`, exactly.
- **Continuous units:** `route ⊆ gate-allowed` — a continuous polyline through
  partly-unseen space truncates at the fog boundary (cell-granular); mid-segment wall
  truncation; impassable-stop-before / arrest-stop-at cell-entry on any-angle segments.
- **Caller:** existing `Room::execute_move` tests stay green (grid), proving the seam
  is unchanged.
- **DoS:** an over-cap path fails closed (`Forbidden`), never truncated silently.

## 12. Execution

Per-checkpoint plan via `writing-plans` → subagent-driven-development (fresh
`shadowcat-coder` per task; per-task `shadowcat-spec-reviewer` + `shadowcat-code-reviewer`
gate; **whole-branch buddy-check** — this checkpoint is security-critical and every
task should be pre-authorized for buddy-check per the M10f-1 experience). Fresh branch
mirroring the `m10f-1-movement-model-dispatch` cadence; local-merge-only (merge gate =
full M10f) unless the user directs otherwise. No new crate.

**Reviewed skill-update gate:** update `shadowcat-codebase-scene-rendering` (the
`move_exec` seam becomes polyline-shaped + engine-agnostic; the new `gate_walk`
primitive; the guard-relaxation and arc-length DoS bound; the differential-oracle
parity technique and its removal) and confirm via `shadowcat-spec-reviewer` before merge.
