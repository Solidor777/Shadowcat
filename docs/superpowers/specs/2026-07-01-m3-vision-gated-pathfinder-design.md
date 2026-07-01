# M3 — Vision-Gated Pathfinder Parity + Router Region Hook

**Status:** Approved (user, 2026-07-01). Third and final checkpoint of the
server-authoritative movement decomposition
(`2026-06-25-server-authoritative-movement-design.md` §7). Branch:
`m10e-5-movement-animation` (continues M1/M2; not pushed/merged — push gate is
full M10).

## 1. Motivation

The M10e-5 buddy-check found P1: for sub-0.5-cell footprints, the client-side
preview pathfinder could accept a diagonal step that the server's move gate
then rejected, causing a rubber-band. The redirect to server-authoritative
movement (M1/M2) replaced client-side path preview with a server-owned grid
A* (M10e-6, `scene/pathfinding.rs`), but the root predicate mismatch that
caused P1 was never actually fixed — it just moved fully server-side, where
it is now a router/gate parity bug rather than a client/server trust
boundary bug. M3 closes it at the root, per the spec §7 M3 row, and adds the
router-side region-arrest hook so M10g has one hook shape to wire in two
places instead of discovering the router needs one later.

## 2. Root cause

Two places decide whether a step is legal against the vision mask, and they
use different predicates:

- **Gate** (`move_exec.rs::execute_move`, the true executor, M1): per step,
  computes `movement::supercover_cells(prev, next, cell)` — the full segment
  supercover, which for a diagonal step includes **both corner-flanker
  cells** (the cells adjacent to the shared corner the diagonal passes near)
  — and requires every cell in that set to be in the mask.
- **Router** (`pathfinding.rs::cell_enterable`, the A* preview used to plan
  and preview a route, M10e-6): per step, computes
  `footprint_cells(to, ctr, r_scene, cell)` — cells the footprint disc at the
  **destination** overlaps — and requires every cell in that set to be in
  the mask. It never calls `supercover_cells`.

For a small (sub-0.5-cell-radius) footprint, the disc at `to` does not reach
the diagonal flanker cells. The router can therefore approve a diagonal step
whose flanker cell is unseen, while the gate — walking that same step at
execute time — rejects it. The route the pathfinder found is not a subset of
what the gate will actually allow, which is the parity property M10e-6 was
supposed to guarantee ("route ⊆ gate-allowed by construction," per the
M10e-6 PLAN.md entry) but does not, for this case.

The wall check has the opposite (safe) asymmetry: `cell_enterable` already
checks footprint-disc-vs-wall clearance *and* center-line crossing, which is
strictly **more** conservative than the gate's plain center-line
`blocks_move` check. A router that is too conservative on walls only causes
false-negative previews (rejecting a route the gate would allow), never a
rubber-band. That asymmetry is intentional and out of scope for this fix.

## 3. Fix — router mask predicate becomes a superset of the gate's

In `cell_enterable`, add:

```rust
let step_cells = movement::supercover_cells(cell_center(from, grid.cell), ctr, grid.cell);
```

and require every cell in `footprint_cells(to, ctr, r_scene, grid.cell) ∪
step_cells` to be in `grid.mask` (when `grid.mask` is `Some`). If
`supercover_cells` returns `None` (degenerate/over-cap input), fail closed —
reject the step — mirroring the gate's `None ⇒ Forbidden` behavior in both
`move_exec.rs` and `Room::publish`.

This is additive only:

- `pathfinding.rs` gains a `use crate::scene::movement;` import (or
  `movement::supercover_cells` directly) — `movement` is already `pub` and
  used elsewhere for the same purpose (`Room::publish`, `move_exec.rs`).
- No change to `move_exec.rs`, `Room::publish`, or the wall-check portion of
  `cell_enterable` — the gate is the ground truth the router is being
  brought into parity with, not the other direction.
- No change to `PathGrid`'s public shape — `supercover_cells` needs only
  `grid.cell`, both cell centers, and the existing `mask`, all already
  available inside `cell_enterable`.

## 4. Router region hook

`pathfinding.rs` is deliberately pure and headless — callers pass parsed
inputs (walls, mask, cell size, rule, footprint); the module owns no I/O and
borrows no ECS (per its module doc comment). `move_exec.rs` already has a
region-arrest hook (`region_arrests(ecs, scene, cell) -> bool`, stubbed to
always return `false`, landed in M1, comment `// M3/M10g stub`). The router
has no equivalent today, meaning it can route through a cell that will
(once M10g lands) arrest movement on entry.

Add a same-shaped inert stub in `pathfinding.rs`:

```rust
/// Region-arrest hook stub (mirrors move_exec.rs::region_arrests). Returns true
/// when a region halts entry into `to`. Currently always false; the region
/// system (M10g) replaces this body. Pure/headless: unlike move_exec's version
/// this takes no ECS handle, consistent with this module's no-I/O contract.
fn region_arrests(_to: Cell) -> bool {
    false
}
```

Call it as a fourth check in `cell_enterable` (after walls, mask, and
center-line crossing), returning `false` from `cell_enterable` (step not
enterable) when `region_arrests` returns `true`. No new `PathGrid` field —
there is no region data model yet, so there is nothing to thread through.
When M10g lands, both stubs (this one and `move_exec.rs`'s) get real bodies;
whether they end up sharing a single implementation or stay parallel
per-module inert-until-real functions is an M10g decision, not this one.

## 5. Testing

- **P1 regression** (the primary test this checkpoint exists to add): a
  diagonal step with a small (sub-0.5-cell) footprint, a mask that contains
  the endpoint cells and all footprint-disc cells but *not* one diagonal
  flanker cell → `cell_enterable` must reject the step. Today it incorrectly
  accepts it; this is the test that proves the fix.
- **No large-footprint regression**: existing mask tests where the footprint
  disc already covers a large area (footprint cells were already a superset
  of the step's supercover) must still pass unchanged — the union should not
  introduce new false rejections when the footprint already dominates.
- **Degenerate-input fail-closed**: if `supercover_cells` returns `None` for
  the step (over-cap span or non-finite input), `cell_enterable` must reject
  the step, matching the gate's fail-closed behavior.
- **Router-region-hook stub**: `region_arrests` always returns `false`; a
  step that would otherwise be enterable stays enterable (i.e., the new
  fourth check is a true no-op today) — a small unit test asserting the stub
  doesn't change behavior guards against an accidental future default flip.

## 6. Out of scope

- The region system itself (data model, effects, cost/weighting) — M10g.
- Any change to `move_exec.rs`'s per-step logic, `Room::publish`'s legacy
  gate, or the wall-check portion of `cell_enterable`.
- Sharing one region-hook implementation across `move_exec.rs` and
  `pathfinding.rs` — deferred to M10g, when there is real region data to
  decide the shared shape around.

## 7. Completion

M3 is the last checkpoint of the M10e-5 server-authoritative-movement
redirect. On completion: `docs/PLAN.md`'s M10e status line updates to
"e-1 through e-6 + M1/M2/M3 all DONE," and the cross-cutting spec's §7 table
is marked complete. M10f (continuous/Polyanya pathfinding) and M10g
(weighted/impassable regions) remain, unstarted.
