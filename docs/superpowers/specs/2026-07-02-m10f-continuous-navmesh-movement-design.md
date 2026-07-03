# M10f — Continuous (navmesh) Pathfinding + Movement — Design

> Checkpoint of the M10 Tokens milestone. Parent spec:
> `2026-06-24-m10-tokens-design.md` §10 (Pathfinding) + §10.5 (engine-choice
> rationale). Sibling: `2026-07-01-m10g-regions-design.md` (grid-engine regions;
> this spec adds the deferred **navmesh** region wiring). Builds on the
> server-authoritative movement model (`MoveRequest` / `move_exec` / `MoveStream`,
> M10e-5 → M1/M2/M3) and the grid A\* router (`scene/pathfinding.rs`, M10e-6).
>
> **Status: design approved (user, 2026-07-02).** Full continuous movement stack,
> decomposed into `M10f-0 … M10f-4`.

## 1. Goal

Give a scene a **continuous (any-angle, gridless) movement model** as a first-class
alternative to grid-stepped movement: tokens occupy arbitrary float positions,
distance is Euclidean, and routes are computed by the `vleue/polyanya` navmesh
engine — while every server-authoritative guarantee already built for grid
movement (the single per-cell visibility/secrecy gate, region cost/impassable/
arrest, streamed continuous vision) carries over **unchanged and unforked**.

## 2. Why this is bigger than the parent spec assumed (context)

The parent M10 spec (2026-06-24) framed M10f narrowly — "adopt `vleue/polyanya`;
walls→Triangulation adapter; gridless movement; movement-model dispatch." At that
time movement execution was the **optimistic client-intent** model. The subsequent
M10e-5 redirect (M1/M2/M3) rebuilt movement as **server-authoritative**, and that
entire stack is **grid-cell-based end to end**:

- `move_exec::execute_move` walks a **cell path step-by-step** with a **king-step
  adjacency guard** (`MoveFail::BadPath` on non-adjacent steps), gating each step
  via `movement::supercover_cells(prev, next, cell)` against the `visible` set
  (§13 per-cell parity with the `Room::publish` gate).
- Region secrecy (M10g) is a **per-cell** region field.
- `MoveStream` samples the executed path by arc-length (`move_stream::sample_path`,
  cap `MAX_VISION_SAMPLES = 96`) for client playback + fog-sweep.

Continuous movement has **no cell-step sequence**. So M10f is not "drop in a second
router" — it must decide how much of the movement stack goes continuous and how the
continuous path is gated. This spec resolves those decisions (§3) so the
decomposition (§12) is honest.

Also verified at design time: there is currently **no `movementModel` dispatch
anywhere** (`SceneEcs::pathfind` is grid-only; `polyanya`/`spade` are not in
`Cargo.toml`), and **scenes are dimensionless** (`docs/TODO.md`: "the scene model is
dimensionless — there is no boundary"; scene dimensions were homed to M12). Both are
prerequisites this spec pulls forward.

## 3. Locked decisions (user, 2026-07-02)

1. **Full continuous movement stack**, decomposed as needed — router + unified
   gated execution + streamed vision + navmesh regions. Decompose, do not descope.
2. **Cell-sampled gate — one mask, never forked.** The polyanya router emits an
   any-angle polyline; execution **arc-length-samples** it and gates each sample's
   **footprint cells** against the **same `visible_cells` cell mask** the grid path
   uses. Continuous changes *geometry* (any-angle) and *distance metric*
   (Euclidean) only; the secrecy decision stays cell-based and **single**. Preserves
   the §13 "never fork the per-cell visibility decision" + `fog-is-the-secrecy-gate`
   invariants.
   - **Documented invariant (accepted caveat):** the gate resolution equals the
     internal rasterization cell size, so a continuous move is admitted/denied at
     **cell granularity** even though the token moves at float precision. This is
     fail-closed and matches how fog already renders (cell/blur granularity) — no
     perceptible fidelity loss. Gridless scenes still carry an **internal
     rasterization cell size** (from `grid.size`, `unwrap_or(100.0)`), **decoupled
     from grid display**.
3. **One unified sampled executor.** `move_exec` is refactored from king-step-per-
   cell into a **sampled-polyline executor**: it takes a polyline (grid A\* emits
   cell-center vertices; polyanya emits any-angle vertices — both are polylines),
   arc-length-samples it via the existing `move_stream::sample_path` sampler, and
   runs the **same** `supercover_cells` → `visible` → region-field → arrest → commit
   per step. Grid becomes the special case where consecutive vertices are ≤1 cell
   apart. `supercover_cells` remains **the** gate primitive (no new gate); grid §13
   parity must be proven under buddy-check before continuous is layered on.
4. **Explicit scene bounds primitive.** A navmesh must triangulate a **bounded**
   walkable region; grid A\* never needed bounds (it expands cells within a search
   window). Add `scene.system.bounds = { width, height }` (grid units) now as the
   foundational primitive M10f-0. **Forward-looking hook (user):** when background
   authoring lands (M12), offer a "resize bounds to the background image" action —
   homed to M12, not built now (no background-authoring path exists yet).

## 4. Data model

### 4.1 Scene bounds (M10f-0)

```
scene.system.bounds = { width: number, height: number }   // grid units, > 0
```

- Resolved via the existing scene-settings resolver family; a scene without an
  explicit bound falls back to a sensible default (see §5.1). ts-rs → regenerate →
  mirror in the client Zod schema (drift guard).
- Mutating `bounds` triggers a **navmesh rebuild** (§5.2), at the M9 vision-re-derive
  cadence — never per-frame.

### 4.2 Movement model (M10f-1)

`movementModel: "grid-stepped" | "continuous"` — a **per-scene** axis with a
**world default**, resolved exactly like the existing scene axes
(`movement_restriction`, lighting master/mode). Authored in
`@shadowcat/module-game-settings` (the M10e-1 world/scene editor). Independent of
grid **display** and grid **kind** (a continuous scene MAY still show a reference
grid; snap is off — grid-metric distance rules do not apply).

- ts-rs → regenerate → Zod mirror.
- Changing `movementModel` does **not** rebuild the navmesh (the mesh depends on
  bounds + walls + impassable regions, not the model); it only changes dispatch.

### 4.3 Regions (M10f-4)

No new region data — reuses the M10g `region` doc_type (`{ shape, behavior, cost,
enabled }`) and the composed **region field**. The navmesh consumes the same
per-requester-visible vs authoritative field split (§7).

## 5. Server — navmesh (`scene/navmesh.rs`, new, pure)

### 5.1 Construction

- **Outer polygon** = the scene bounds rectangle `[0,0]..[width,height]` (grid
  units → scene pixels via `grid.size`). No explicit bound → a **fixed large finite
  default rectangle** (a fail-safe constant, always well-defined, never unbounded).
  The default is deliberately **not** derived from content AABB — content-derived
  bounds were rejected at design time (edge-drag re-mesh churn, ill-defined for open
  scenes); an authored `bounds` is the intended path, the default is only a
  finite-boundary backstop so navmesh construction never faces an unbounded plane.
- **Obstacles** = `blocksMove` wall segments (the same `move_walls(scene)` set the
  grid executor uses) + **impassable** region shapes (§7). Walls are segments, not
  closed polygons; the adapter builds obstacle constraints from them.
- **CDT** via `spade` (constrained Delaunay triangulation) → `polyanya::Mesh`.
- **Footprint** via **obstacle inflation** by `footprintRadius` (uniform across all
  token sizes, fractional included) — the navmesh analogue of the grid clearance
  test. `footprintRadius` comes from the request (client resolves `EffectiveActor`),
  as with grid A\*; the server needn't resolve actor-data for routing.

### 5.2 Lifecycle

- The `polyanya::Mesh` is **static after build** → held in a `SceneEcs` navmesh
  side-table (mirrors the light / region side-tables), rebuilt on **wall / impassable-
  region / bounds** mutation at the M9 vision-re-derive cadence. Degenerate inputs
  (zero/negative bounds, over-cap obstacle count) **fail closed** — the scene reports
  no navmesh, continuous routing returns `Unreachable`, never a silent all-pass.

### 5.3 Query

`navmesh_find(start, goal, waypoints[], footprintRadius, cost_field) → { polyline, cost }`

- Any-angle search over the inflated mesh; distance **Euclidean**.
- **cost_field** = the region cost-layers (§7); absent → uniform weight 1.
- Output is a **polyline** (scene coords) fed to the same `Pathfind`/`PathResult`
  path the grid router uses.

## 6. Server — dispatch + unified executor

### 6.1 Dispatch (M10f-1)

`SceneEcs::pathfind` gains a `movementModel` branch:

```
match resolved_movement_model(scene) {
    GridStepped => pathfinding::find(...),      // existing grid A*
    Continuous  => navmesh::navmesh_find(...),  // new
}
```

Both return the same `PathOutcome` shape (polyline + cost + `arrested`). The
per-requester visibility mask (grid: `visible_cells`; navmesh: the same
`visible_cells` set drives the **cell-sampled** post-filter — see §6.3) and the
per-requester region field are resolved identically to the grid path (the existing
`pathfind` body already computes both).

### 6.2 Unified executor (M10f-2)

`move_exec::execute_move` is refactored to a **sampled-polyline executor**:

- **Input:** a polyline `path: &[(f64,f64)]` (grid path = cell centers; continuous
  path = any-angle vertices).
- **Sampling:** arc-length-sample via `move_stream::sample_path` (or a shared
  sub-routine) so consecutive samples are ≤1 cell apart — this makes
  `supercover_cells(sample_prev, sample_next, cell)` well-defined and dense enough
  that the swept footprint is covered.
- **Per-sample:** the **same** step-2 vision gate (`supercover_cells` → `visible`),
  step-3 region field (impassable stop-before, arrest stop-at, terrain cost
  accumulation), and commit. `stopped_early` / `truncated` semantics unchanged.
- **Grid parity (§13):** the grid case (cell-center vertices, ≤1-cell steps) must
  produce **identical** admit/deny/arrest outcomes to the current king-step
  executor. This is the load-bearing parity proof of M10f-2 — proven under
  buddy-check with parity tests across env / global-illumination / darkvision / LOS+
  wall, mirroring the M10e-4 parity suite. The king-step adjacency **guard** relaxes
  to "each *sample* pair ≤1 cell" (samples, not authored waypoints).

### 6.3 Cell-sampled gate reused verbatim

The unified executor's gate is **engine-agnostic** — it never inspects the movement
model, only the polyline + cell mask + region field. Continuous execution therefore
inherits M1/M2/M3's secrecy guarantees with **zero new secrecy code**. The navmesh
router's own visibility handling is a **cell-sampled post-filter**: a continuous
route is admissible only where its sampled footprint cells lie in the requester's
`visible` (or `visible ∪ explored`) set — the same predicate `pathfinding::find`
applies per cell, so **route ⊆ gate-allowed** holds by construction across engines
(the M3 invariant, generalized).

## 7. Server — regions on the navmesh (M10f-4)

Reuses the M10g composed region field + the two-field secrecy split
(`region_field(scene, viewer)`; `viewer: None` = authoritative, `Some(user)` =
per-requester visible). Mapping to the navmesh:

- **terrain** (cost ≥ 1) → polyanya **cost-layer** (Split-Mesh boundary refraction —
  the Weighted Region Problem; approximate-but-cosmetic, visually fine for a
  human-previewed VTT path, per parent §10.5). Built from the **per-requester
  visible** field for the router/budget; a secret terrain region never inflates a
  player's cost.
- **impassable** → navmesh **obstacle** (mesh hole) when authoritative; for the
  per-requester router, an impassable region the requester cannot see is **absent**
  (routed straight through) and sprung at execution by the authoritative field —
  exactly the grid behavior.
- **arrest** → **not** a routing-geometry problem. Arrest is a **per-sample stop**,
  computed by the **same cell-sampled region field** as the grid: the player-facing
  router truncates the continuous route at the first **visible** arrest cell and sets
  `PathResult.arrested`; `move_exec` springs authoritative arrests (incl. secret
  ones) at execution. Identical across engines — arrest logic is shared, only the
  route *geometry* differs.
- **conditional layers:** `enabled` toggling of impassable/terrain uses the crate's
  conditional-layer support where it avoids a full re-mesh; otherwise a `SceneEcs`
  region-mutation rebuild (§5.2).
- **Parity invariant (carried from M10g):** router **visible field ⊆ authoritative
  field**, so a player-shown continuous route is always executable-or-arrested, never
  region-wall-blocked by a region the server knows and the player doesn't.

## 8. Protocol

No new frames. `Pathfind` / `PathResult` (incl. `arrested: bool` from M10g),
`MoveRequest` / `MoveExecuted` / `MoveStream` all already carry the polyline + per-
step outcome + samples. Additions are field-level only:

- `PathResult` / `MoveStream` polylines carry **any-angle** vertices for continuous
  scenes (already `(f64,f64)` — no shape change).
- `movementModel` + `bounds` ride the scene document (§4). ts-rs → Zod mirrors.

## 9. Client

- **Continuous scenes:** **no snap** on place/move (arbitrary float positions);
  **Euclidean** ruler + movement-budget readout (the M10e-6 measure-tool route mode
  already renders path + budget; continuous uses the Euclidean metric, no diagonal
  rule); polyanya route preview via the same route-mode overlay + `pendingSeq` epoch
  guard. Region render/secrecy unchanged — the server egress filter is the gate
  (`fog-is-the-secrecy-gate`).
- **Authoring:** `movementModel` (per-scene, world default) + `bounds`
  (`{width,height}`) editors in `@shadowcat/module-game-settings`, alongside the
  existing vision/lighting/movement-restriction axes. Bounds authored manually now;
  "resize to background image" homed to M12.
- **Grid display** stays orthogonal (a continuous scene may show a reference grid).

## 10. Cross-platform / bloat

- **Hard cargo-bloat gate:** `vleue/polyanya` pulls `spade` + `geo` (+ glam,
  hashbrown, smallvec, bvh2d). M9b deliberately avoided `geo`. The parent spec
  pre-adopted the dependency, but the **binary-size delta is measured in the M10f-1
  plan** and surfaced immediately if it blows the budget (fix-forward, not silent).
- Pure geometry in `scene/navmesh.rs` — no OS-specific code; `#[cfg]`-free; three-OS
  CI matrix proves portability. No files touched (navmesh is derived state), so no
  path handling.

## 11. Scope — explicit exclusions (deferred, homed in `PLAN.md`)

1. **Edge-projected environment light** — M10f-0's bounds primitive **unblocks** the
   M10e-2 flat-ambient deviation, but implementing edge-projected, `blocksLight`-
   occludable ambient light is **separate work homed to M12** (per the design
   review). M10f-0 ships bounds only.
2. **Full scene-management UI** (resize handles, background-driven sizing, scene
   switching, "resize bounds to background") → **M12**. M10f-0 ships the minimal
   bounds data primitive + a manual GM control.
3. **Boundary-exact (Snell / Weighted Region Problem-optimal) weighted continuous
   pathfinding** → out of scope permanently for v1; Split-Mesh cost-layer
   approximation is accepted (parent §10.5, §11 exclusions).
4. **Per-actor / faction movement exemptions**, **trigger regions** — remain homed to
   Phase 2 (unchanged from M10g §10).

## 12. Decomposition (approved 2026-07-02)

Linear; each independently shippable + buddy-checked (M8/M9/M10 cadence); `/clear`
between.

- **M10f-0 — Scene bounds primitive.** `scene.system.bounds{width,height}` +
  resolver + default fallback + minimal GM control (`module-game-settings`) + rebuild
  trigger seam. ts-rs + Zod. *Ships independently; navmesh consumes it in M10f-1.*
- **M10f-1 — `movementModel` axis + dispatch + polyanya router.** Add `polyanya` +
  `spade` + `geo` (**cargo-bloat check here**); `scene/navmesh.rs` adapter
  (walls+bounds → `Mesh`, obstacle inflation footprint, static+rebuild lifecycle);
  `movementModel` scene axis + world default; `SceneEcs::pathfind` dispatch; measure-
  tool preview + Euclidean budget for continuous. *Shippable: preview works,
  execution unchanged.*
- **M10f-2 — Unify the executor.** Refactor `move_exec` king-step → arc-length-
  sampled polyline gating; **prove grid §13 parity** (parity test suite) under heavy
  buddy-check. *Grid behavior unchanged; executor now polyline-shaped, engine-
  agnostic.*
- **M10f-3 — Continuous execution + streamed vision.** `MoveRequest` continuous
  polyline → unified executor → cell gate → `MoveStream` (already samples any
  polyline); client no-snap place/move in continuous scenes. *Continuous scenes gain
  full server-authoritative gated movement.*
- **M10f-4 — Regions on the navmesh.** Wire terrain (cost-layer), impassable
  (obstacle/conditional-layer), arrest (cell-sampled truncation, shared region field)
  into the continuous path; two-field secrecy split + `visible ⊆ authoritative`
  parity. *Completes the M10g navmesh deferral; lights up the last engine.*

**Rejected alternative:** folding M10f-2 into M10f-3 (4 checkpoints). Kept split —
the executor refactor touches working server-authoritative M1/M2/M3 code and must
prove grid parity **before** continuous is layered on top.

## 13. Testing

- `scene/navmesh.rs`: mesh construction from bounds+walls; obstacle inflation by
  footprint radius (fractional sizes); fail-closed on degenerate/over-cap/unbounded
  inputs; rebuild on wall/region/bounds mutation.
- Dispatch: `movementModel` selects the right engine; both return the same
  `PathOutcome` shape; continuous distance is Euclidean.
- **Executor parity (M10f-2, load-bearing):** the sampled executor reproduces the
  king-step executor's admit/deny/arrest outcomes on grid inputs across env / global-
  illumination / darkvision / LOS+wall (mirror the M10e-4 parity suite); `route ⊆
  gate-allowed` holds for continuous.
- Continuous execution: a continuous move into unseen space truncates at the cell
  gate (`stopped_early`/`truncated`, fail-closed — same mechanism as §6.2, not a
  request-level rejection); `MoveStream` samples an any-angle path correctly; observer
  clip leak-free (reuse the M2 no-leak suite, now over any-angle paths).
- Regions on navmesh: terrain reroutes/costs; impassable routes around; visible
  arrest truncates + sets `arrested`; **secret region absent from a player's field
  but sprung authoritatively**; `visible ⊆ authoritative` parity.
- Client: continuous scene has no snap; Euclidean budget; region render shows only
  permitted regions.
- **cargo-bloat**: binary-size delta within budget (or surfaced as a complication).

## 14. Execution

Per-checkpoint plan via `writing-plans` → SDD (per-task two-reviewer gate + whole-
branch buddy-check), matching the M10 cadence. **Reviewed skill-update gate:** update
`shadowcat-codebase-scene-rendering` (new `movementModel` axis + dispatch,
`scene/navmesh.rs` seam + navmesh lifecycle, the unified sampled executor replacing
the king-step walk, navmesh region cost-layers, scene bounds primitive) and confirm
via `shadowcat-spec-reviewer` before merge. M10f-2's executor refactor also touches
the `move_exec` seam documented there — mirror it.
