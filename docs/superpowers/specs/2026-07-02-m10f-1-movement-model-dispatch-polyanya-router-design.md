# M10f-1 — `movementModel` axis + dispatch + polyanya router — Design

> Checkpoint of M10f (continuous/navmesh movement). Parent spec:
> `2026-07-02-m10f-continuous-navmesh-movement-design.md` (§4.2, §5, §6.1, §10, §12).
> Builds on **M10f-0** (`scene.system.bounds`, shipped) and the M10e-6 grid A\* router
> (`scene/pathfinding.rs`). This doc records the **checkpoint-level decisions** the
> parent spec left open (the polyanya/spade adapter, the footprint/mesh-caching model,
> the dependency/bloat policy, and the exact "execution unchanged" boundary). It does
> **not** restate the parent spec's locked decisions — read that first.
>
> **Status: design approved (user, 2026-07-02).**

## 1. Goal (this checkpoint)

Add the `movementModel` scene axis + world default, dispatch `SceneEcs::pathfind` to a
new **polyanya navmesh router** for continuous scenes, and render an **honest, fog-safe
route preview** with a Euclidean budget. **Grid movement is byte-for-byte unchanged;
continuous move *execution* is not built here** (it lands in M10f-2/3). Shippable: a GM
can flip a scene to `continuous`, preview any-angle routes, and see a Euclidean budget;
committing a continuous move is cleanly disabled until the unified executor exists.

## 2. Crate facts (verified 2026-07-02, feed the adapter design)

- **polyanya `0.16.1`** ships a **headless CDT mesh builder** (spade internal — no Bevy,
  no `vleue_navigator`, no hand-rolled spade):
  `Triangulation::from_outer_edges(&[Vec2])` + `add_obstacle(s)(rings)` →
  `.as_navmesh() -> Mesh`. Query: `Mesh::path(from, to) -> Option<Path>` where
  `Path { path: Vec<Vec2>, length: f32 }` (destination is the last vertex). All geometry
  is `glam::Vec2` (f32).
- **`geo` is already a polyanya dependency** and exposes a native **`Buffer` trait**
  (`geo 0.32`): buffering a `Line`/`LineString` yields a capsule (pill) polygon — the
  exact **open-segment → closed-obstacle** inflation the navmesh needs. No new offset
  crate.
- **Weighted per-region *cost* is uncertain** in polyanya (the `Layer` system reads as
  block/allow, not graded weights). **Forward risk for M10f-4 — verify against source
  there.** Not an M10f-1 concern (M10f-1 mesh carries no regions).

## 3. Dependencies + binary-size policy

```toml
polyanya = { version = "0.16", default-features = false }  # drop `async` + `recast`/`rerecast`
geo      = "0.32"                                          # pin to unify with polyanya's copy
```

- We use the **blocking** `Mesh::path()` and never import Recast meshes, so the `async`
  and `recast` default features are dropped. `spade` / `glam` / `bvh2d` / `i_overlay`
  ride transitively. Pinning `geo = "0.32"` shares polyanya's compiled copy (no
  duplicate geo).
- **The "cargo-bloat gate" is the existing whole-binary guardrail** (`ci.yml` "Binary
  size budget": `wc -c` release binary `< 62914560` = 60 MiB). No per-crate bloat
  tooling exists. **The plan measures the release-binary delta** from these deps.
- **If the delta approaches the ceiling:** feature-trim is already applied; then
  **surface a `bump-the-ceiling` vs `optimize` decision as a complication** (a core
  navmesh engine justifies a documented ceiling raise, but that is a user decision —
  surfaced, never silent). Fix-forward, per the parent §10 hard gate.

## 4. `movementModel` scene axis + dispatch

### 4.1 Data model

`movementModel: "grid-stepped" | "continuous"` — **per-scene override + world default**,
resolved **exactly** like `movement_restriction`:

- **Server** (`scene/mod.rs`): new `MovementModel` enum + `parse_movement_model(&str)`
  (unknown/absent → `GridStepped`, fail-safe to the unchanged engine) + a
  `movement_model` field on `ResolvedScene`, set in `resolve_scene` from the resolved
  settings string. Mirrors the `MovementRestriction` shape line-for-line.
- **Client** (`scene-docs.ts`): `MovementModel` type + resolution in
  `resolveSceneSettings` (built-in default `grid-stepped` < `world-settings` <
  per-scene override; `null` = inherit, per the fail-closed resolver family invariant).
- ts-rs → regenerate → Zod mirror (drift guard).
- Authored in `@shadowcat/module-game-settings` alongside the existing vision / lighting
  / movement-restriction axes (world default + per-scene override).
- **Independent of grid display/kind** — a continuous scene MAY still show a reference
  grid; the axis only changes routing/dispatch, never the grid overlay.

### 4.2 Dispatch

`SceneEcs::pathfind` gains a branch on `resolve_scene(scene).movement_model`:

```
GridStepped => pathfinding::find(...)        // existing grid A*, untouched
Continuous  => navmesh::navmesh_find(...)    // new
```

Both return the same `PathOutcome { path, cost, arrested }`, so every downstream caller
(`conn.rs` `Pathfind` handler, measure-tool preview) is **engine-blind**. The
per-requester visibility mask and (M10f-4) region field are resolved identically to the
grid path — the existing `pathfind` body already computes the mask.

## 5. Navmesh adapter — `scene/navmesh.rs` (new, pure, headless)

### 5.1 Construction

- **Outer rectangle** = `resolve_scene(scene).bounds` (grid units) × `grid.size`
  (pixels/cell) → `[0,0]..[width·cell, height·cell]` → `Triangulation::from_outer_edges`.
- **Obstacles (M10f-1 = walls only)** = `move_walls(scene)` (`blocksMove` segments, the
  same set the grid executor uses), each **buffered by `footprintRadius`** via geo
  `Buffer` → capsule ring(s) → `add_obstacles`. (`move_walls` are zero-width segments —
  there is no wall-thickness field; inflating a segment by the agent disc radius is the
  correct Minkowski obstacle for a disc agent. If walls ever gain authored thickness, the
  buffer becomes `wall_half + footprintRadius`.) **Impassable/terrain regions are
  M10f-4** — not added to the mesh here.
- `.as_navmesh() -> Mesh`. Coordinates: f64 scene-pixel → f32 `Vec2` for the mesh, cast
  back to f64 on output (scene pixel magnitudes fit f32 precision comfortably).

### 5.2 Footprint / caching model (the crux — decided for durability)

The mesh is **footprint-*aware*** (inflated by the request's `footprintRadius`), giving
continuous routing **parity with the already-footprint-aware grid A\*** (`cell_enterable`
does footprint-disc clearance) — *not* a footprint-agnostic mesh, which would regress
route quality between engines.

- **Memoized per `(scene, quantized footprintRadius)`** in a new `SceneEcs` navmesh
  side-table (token sizes are a small discrete set — the cache stays bounded).
- **No navmesh state exists before M10f-1** (M10f-0 shipped only the `bounds` primitive +
  resolver; there is no dirty flag or rebuild seam). M10f-1 **introduces** the side-table
  **and its invalidation**, hung off the ECS's existing document-mutation path
  (`apply_op` / the config-doc side-table maintenance point) — invalidated on **bounds /
  `blocksMove`-wall** mutation. This reconciles parent §5.1 (inflate by footprintRadius)
  with §5.2 (static after build): **static per radius bucket, rebuilt on geometry
  change**, never per-frame, never per-query once cached.
- **Fail closed:** degenerate bounds (non-finite / ≤0 axis — already guarded by the
  M10f-0 resolver default), over-cap obstacle count, or a `Mesh::new`/triangulation
  failure ⇒ the scene reports **no navmesh** ⇒ `navmesh_find` returns `Unreachable`
  (never a silent all-pass). Mirrors the grid router's fail-closed discipline.

### 5.3 Query

`navmesh_find(scene, start, waypoints[], footprintRadius, mask...) -> Result<PathOutcome, PathFail>`:

1. Resolve/build the footprint-inflated `Mesh` for `(scene, footprintRadius)` (memoized).
2. `Mesh::path` **per leg** (`start→wp1`, `wp1→wp2`, …), concatenating polylines
   (dedupe the shared vertex) and summing the Euclidean `length`. Any leg returning
   `None` ⇒ `Unreachable`.
3. Apply the **cell-sampled visibility post-filter** (§6).
4. Return `PathOutcome { path, cost: Euclidean length, arrested: false }` (arrest is
   M10f-4; the M10f-1 navmesh carries no region field).

## 6. Cell-sampled visibility post-filter (fog-safe preview, route ⊆ gate-allowed)

The any-angle route is **arc-length-sampled** (`move_stream::sample_path`); for each
sample, `footprint_cells ∪ movement::supercover_cells(prev, next, cell)` is tested
against the **same** per-requester `visible` mask the grid router uses (`∪ explored` for
`revealed`), truncating the route at the first sample that leaves the mask. This is
**parent §6.3 verbatim**:

- The preview is **honest / fog-safe** — no route line is drawn through unseen space
  (`fog-is-the-secrecy-gate` applies to previews too; a preview through fog leaks
  geometry).
- **`route ⊆ gate-allowed` holds across both engines by construction** — the same cell
  predicate, the same primitives (`supercover_cells` / `footprint_cells` / the `visible`
  mask), **no forked visibility decision, no new secrecy code** (the M3 invariant,
  generalized to any-angle).
- The rasterization cell size for a gridless scene is the internal
  `scene_grid_sizes().unwrap_or(100.0)` (parent §3 decision 2), decoupled from grid
  display.

## 7. Client

- Continuous route **preview** via the existing measure-tool route mode + `pendingSeq`
  epoch guard (the same overlay grid uses), with a **Euclidean** budget readout (no
  diagonal rule).
- **Commit disabled** in continuous scenes — a clear guard (no grid-snap fallback).
- **No client no-snap place/move yet** (homed to M10f-3, since nothing executes here).
- Region render/secrecy unchanged — the server egress filter remains the gate.

## 8. Protocol

**No new frames.** `Pathfind` / `PathResult` (`{path, cost, arrested}`) already carry an
any-angle `(f64,f64)` polyline — continuous routes ride them unchanged. `movementModel`
rides the scene document (§4.1); ts-rs → Zod mirror.

## 9. Testing

- **Adapter:** mesh construction from bounds+walls; segment→capsule inflation by
  footprint radius (incl. fractional); fail-closed on degenerate / over-cap / unbounded /
  triangulation-failure inputs.
- **Dispatch:** `movementModel` selects the right engine; both return an identical
  `PathOutcome` shape; continuous distance is Euclidean.
- **Fog-safe preview / `route ⊆ gate-allowed`:** a continuous route that would cross
  unseen space **truncates** at the mask boundary; a goal outside the mask ⇒
  `Unreachable`; GM (no mask) routes freely.
- **Caching:** memoization returns a cached mesh; **bounds / wall mutation invalidates**
  it; distinct footprint radii get distinct meshes.
- **Bloat:** release-binary delta measured against the 60 MiB ceiling (surfaced as a
  complication if near).
- **Client:** continuous scene shows a preview + Euclidean budget; commit is disabled.

## 10. Scope — explicit exclusions (homed elsewhere)

1. **Continuous move execution + streamed vision** → M10f-3 (needs the M10f-2 unified
   executor first).
2. **Unified sampled executor / grid §13 parity proof** → M10f-2.
3. **Regions on the navmesh** (terrain cost-layer, impassable obstacle, arrest
   truncation) → M10f-4. The M10f-1 mesh carries **walls only**; `arrested` is always
   `false`.
4. **Client no-snap place/move** in continuous scenes → M10f-3.
5. **Graded per-region cost feasibility in polyanya** → verified in M10f-4 (§2 forward
   risk).

## 11. Forward risks recorded

- Graded per-region cost may not be first-class in polyanya (block/allow layers only) —
  M10f-4 blocker to verify, not M10f-1.
- Binary-size headroom is unknown until measured — the plan measures it.

## 12. Execution + gates

Per-checkpoint plan via `writing-plans` → SDD (per-task two-reviewer gate + whole-branch
buddy-check), M10 cadence, `/clear` between. **Reviewed skill-update gate:** update
`shadowcat-codebase-scene-rendering` (new `movementModel` axis + `pathfind` dispatch,
`scene/navmesh.rs` seam + memoized footprint-mesh lifecycle + fail-closed contract, the
cell-sampled preview post-filter) and confirm via `shadowcat-spec-reviewer` before merge.
