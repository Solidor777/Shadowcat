# M10f-4 — Regions on the Continuous (navmesh) Engine — Design

> **Final checkpoint of the M10f milestone** (continuous/navmesh movement).
> Parent spec: `2026-07-02-m10f-continuous-navmesh-movement-design.md` §7 (regions
> on the navmesh) + §12 (decomposition, this is M10f-4). Siblings:
> `2026-07-01-m10g-regions-design.md` (grid-engine regions — the weighting model
> this checkpoint reuses wholesale) and `2026-06-24-m10-tokens-design.md` §10.2/§10.3
> (the original two-cost-backend assumption this spec **corrects**). Builds on the
> unified engine-agnostic executor (`move_exec`/`gate_walk`, M10f-2), continuous
> execution (M10f-3), and the polyanya router (`scene/navmesh.rs`, M10f-1).
>
> **Status: design approved (user, 2026-07-03).** Completes the M10g navmesh
> deferral and lights up the last routing engine. Local-merge-only; push gate =
> full M10 (per the M10f milestone convention).

## 1. Goal

Wire the three M10g region behaviors — `terrain` (per-cell cost multiplier ≥ 1),
`impassable` (route around / entry refused), `arrest` (enter and stop) — into the
**continuous** movement model's router and preview, server-authoritatively, with
the same per-region secrecy the grid engine already enforces. Execution
(`move_exec`) is already engine-agnostic (M10f-2/3) and already springs regions on
any polyline; this checkpoint is about the **player-facing continuous route + budget**.

## 2. Locked decisions (user, 2026-07-03)

1. **The M10g per-requester cell region field is the single weighting authority for
   BOTH engines.** Terrain weighting is NOT delegated to polyanya. The
   `region_field(scene, viewer)` cell field (M10g §4) — `cell → { impassable |
   arrest | multiplier }`, precedence + MAX compose — is read identically by the
   grid router, the continuous router, and `move_exec`. This matches the M10
   architecture principle *"weighting = universal region overlay, not a scene mode."*
2. **Polyanya 0.16.1 cannot bias a route by graded terrain cost — verified against
   crate source, not the README.** The only cost-affecting knob is the
   `detailed-layers`-gated `Layer.scale`, a per-layer coordinate transform (applied
   as `root * scale` in the search, `instance.rs`), off in our
   `default-features = false` build and semantically wrong as a per-unit cost
   multiplier. The parent §7 / M10-tokens §10.3 "polyanya cost-layer (Split-Mesh)"
   is therefore **struck as infeasible** and replaced by decision 1. Polyanya's clean
   `path_on_layers(blocked_layers)` and obstacle mechanisms remain **available but
   deliberately unused** for regions in v1 (documented so a future maintainer does
   not re-adopt the stale plan).
3. **Continuous weighted routing reuses the shipped weighted grid A\* as the cost
   brain, then smooths.** When the per-requester region field carries `terrain`
   (mult > 1) or `impassable`, the cost-optimal route is computed by the existing
   `pathfinding::find` over the region field (M10g §5) — **forced to
   `DiagonalRule::Euclidean`** so the base metric stays continuous-Euclidean and only
   the terrain multiplier + cell topology come from the grid — then **line-of-sight
   smoothed** to restore any-angle geometry. A walls-only continuous route stays pure
   polyanya (M10f-1), unchanged.
4. **LOS smoothing ships in v1** (not deferred). Without it a continuous scene with
   any difficult terrain visibly routes like a grid scene — a regression in the exact
   any-angle feature continuous exists for. Decompose, don't descope.
5. **The cell grid is a cost-search discretization, never position snapping.**
   Continuous scenes stay snap-off (M10f-3); token positions remain any-angle floats.
   The scene's cell grid is used only to discretize the weighted search + rasterize
   regions (every scene already has a grid spec + rasterized region field).

## 3. Data model — no change

Reuses the M10g `region` doc_type verbatim (`system = { shape, behavior, cost,
enabled }`) and the M10g `region_field(scene, viewer)` two-value contract
(`viewer: None` = authoritative; `Some(user)` = per-requester visible). No new
fields, no new doc_type, no ts-rs/Zod change. Per-region secrecy rides the existing
envelope permission tier (`all` / `owner_or_gm` / `gm_only`); a secret region's
whole `Create` op is already dropped at egress (`permissions.default = "none"`,
M10g), and `region_field(scene, Some(user))` already omits it from a non-GM's field.

## 4. Server — continuous router dispatch (`SceneEcs::pathfind`)

The `Continuous` branch of `pathfind` (added M10f-1) gains region awareness. The
per-requester visibility mask and the per-requester `region_field` are **already
computed once** in the `pathfind` body and shared across engines (M10f-1 §13 — never
forked). This checkpoint threads the region field into the continuous branch.

```
// pathfind(), Continuous branch (per-requester field already in scope):
let field = region_field(scene, viewer);        // viewer = None for GM
let route = if field.has_terrain_or_impassable_in(search_window) {
    // Weighted route: cost-optimal over the cell field, then any-angle smoothed.
    let cells = pathfinding::find_weighted(
        start, waypoints, mask, field,
        DiagonalRule::Euclidean,                 // continuous base metric, NOT the world rule
    )?;                                          // terrain bends; impassable routes around;
                                                 // arrest truncates; cost = weighted sum
    los_smooth(cells, walls, field)             // §5
} else {
    navmesh_find(navmesh_for(scene, footprint), start, waypoints)?   // pure polyanya (M10f-1)
};
clip_to_visible_mask(route, mask, cell, footprint, walls)           // unchanged secrecy boundary
```

- **`find_weighted`** is not a new engine — it is the existing `pathfinding::find`
  with the diagonal rule pinned to `Euclidean` for this call (the `euclidean` rule
  already exists in `pathfinding.rs`). It already consumes the region field (M10g §5),
  already routes around impassable cells, already truncates at the first **visible**
  arrest cell and sets `arrested`, already returns the weighted `cost`. Reuse, not
  reimplementation.
- **Dispatch predicate** (`has_terrain_or_impassable_in`): the per-requester field
  contains at least one `terrain` (mult > 1) or `impassable` cell within the search
  window. Arrest-only (no terrain/impassable) does **not** force the weighted path —
  a pure-polyanya route with an arrest post-filter truncation is sufficient and keeps
  the any-angle geometry (arrest neither bends the route nor requires route-around).
- **GM requester** passes `viewer = None` (authoritative field), mirroring
  `visible_cells`'s GM-skips-the-mask convention (M10g `region_field` contract).

### 4.1 Why the grid is reused for the weighted search

Weighted route optimization requires a discretization — polyanya offers only a
single Euclidean-shortest route and cannot enumerate cheaper-weighted alternatives.
The cell field **is** the discretization every scene already carries. Forcing
`Euclidean` diagonal cost keeps the base metric faithful to continuous distance
(√2 diagonals), so the only thing the grid contributes is topology + the terrain
multiplier — exactly the "universal overlay" framing. The honest trade-off (locked,
approved): a *weighted* continuous route is grid-topology-derived-then-smoothed, not
a pure Euclidean navmesh route. For a human-previewed, one-path-per-drag VTT this is
the accepted approximation (parent §11.3 already deferred boundary-exact weighted
continuous pathfinding permanently).

## 5. Server — LOS smoothing (`scene/pathfinding.rs` or a pure sibling)

Pure, no-I/O. Input: the weighted cell-center polyline + `walls` (the `blocksMove`
set) + the region `field`. Standard line-of-sight string-pull with a **cost-guard**:

> Straighten a span `i → k` (drop intermediate vertices) **iff** the straight chord
> from vertex `i` to vertex `k` crosses **no** `blocksMove` wall, enters **no**
> `impassable` cell, and enters **no** cell whose terrain multiplier **exceeds the
> maximum multiplier along the original grid subpath `i..k`**.

- The cost-guard is what makes smoothing weight-**safe**: it can straighten through
  open or equal-cost terrain but never shortcuts *into* more-expensive terrain the
  weighted search deliberately routed around, so the smoothed route's weighted cost
  is `≤` the grid route's and never cheats the detour. (This sidesteps the Snell/
  Weighted-Region-Problem boundary-refraction issue by refusing, not by solving it —
  consistent with parent §11.3.)
- Chord-vs-cell tests reuse `movement::supercover_cells` (the same primitive the
  gate + `cell_enterable` use). Chord-vs-wall reuses `segments_cross`. No new geometry
  primitives.
- Fail-closed: a degenerate/over-cap span falls back to the un-smoothed subpath
  (grid-shaped but correct), never to an un-guarded straight line.
- `cost` after smoothing is **recomputed** as the Euclidean length of the smoothed
  polyline times the per-span terrain multipliers actually traversed (replay, never
  trusting the pre-smoothing sum) — mirroring `find`'s M10g arrest cost-replay
  discipline.

## 6. Server — arrest + impassable on the continuous route

- **Arrest** (both dispatch paths): truncate at the first **visible** arrest cell,
  set `PathResult.arrested`. On the weighted path this is already done inside `find`
  (M10g §5). On the pure-polyanya path it is a cell-sampled post-filter on the route
  (arc-length-sample via `move_stream::sample_path`, truncate at the first sample
  whose cell is a visible arrest cell) — the same sampling `clip_to_visible_mask`
  already performs, extended with the arrest-cell test.
- **Impassable**: handled by the weighted path's route-around (the dispatch predicate
  routes any impassable-bearing scene through `find`, which refuses impassable cells).
  Polyanya obstacles / `blocked_layers` are **not** used — one mechanism (the cell
  field), consistently, for all three behaviors.

## 7. Server — execution (already engine-agnostic; verify, do not rebuild)

`move_exec::execute_move` / `gate_walk` has cell-sampled the region field for
impassable (stop-before), arrest (stop-at), and terrain-cost accumulation on **any**
polyline since M10f-2, keyed on cell-entry transitions. The weighted+smoothed
continuous route (a polyline like any other) feeds the same executor unchanged.
Expected **zero** `move_exec` production changes — proven by tests, per the M10f-2/3
precedent (both shipped continuous execution with zero executor changes). The
authoritative field (`region_field(scene, None)`) still springs secret regions at
execution regardless of what the requester's route preview could see.

## 8. Secrecy — inherited, zero new machinery

- Router/preview/budget read `region_field(scene, Some(user))` (per-requester);
  `move_exec` reads `region_field(scene, None)` (authoritative). A secret region is
  absent from a player's continuous route + cost, and sprung at execution — identical
  to the grid engine (M10g §2.2).
- **`route ⊆ gate-allowed` holds on the continuous engine.** The weighted route is
  built from the per-requester field (a subset of authoritative), and
  `clip_to_visible_mask` applies the same `footprint_cells ∪ supercover_cells`
  predicate the executor uses. The M3 invariant generalizes to this engine exactly as
  M10f-1 generalized it for walls.
- **`MoveStream.cost` stays trusted-only** (`Some` for mover/GM, `None` for a clipped
  observer) — the whole-move-scalar invariant. A continuous move's cost may reflect a
  `gm_only` terrain region the observer's own vision would never reveal; it must not
  broadcast. (No new surface — this is the existing M10g invariant, now also reachable
  via the continuous path.)

## 9. Protocol — no change

`Pathfind` / `PathResult { path, cost, arrested }` and `MoveRequest` / `MoveExecuted`
/ `MoveStream` already carry any-angle `(f64,f64)` polylines + per-step outcome +
`cost` + `arrested`. Field-level only, no new frames (parent §8).

## 10. Client — no change beyond what M10f-1/3 shipped

The measure-tool route preview + Euclidean budget readout (M10f-1) and continuous
commit (M10f-3) already render whatever polyline + cost + `arrested` the server
returns. A weighted continuous route is just a polyline with a weighted `cost`; the
client draws it. Region render/secrecy is unchanged — the server egress filter is the
gate (`fog-is-the-secrecy-gate`), `RegionView` is a dumb per-frame reconciler (M10g).
No new client code is anticipated; verify the preview renders the smoothed weighted
route + weighted budget on a continuous scene.

## 11. Cross-platform / bloat

- **No new crate.** The weighting reuses the existing grid A\* + region field; the
  smoothing reuses `supercover_cells` / `segments_cross`. cargo-bloat budget
  untouched. (Polyanya is already in the tree from M10f-1; this checkpoint adds no
  dependency.)
- Pure geometry in `scene/pathfinding.rs` / the smoothing sibling — no OS-specific
  code, `#[cfg]`-free, three-OS CI matrix proves portability. Derived state only, no
  path/file handling.

## 12. Scope — explicit exclusions (deferred, homed in `PLAN.md`)

1. **Boundary-exact (Snell / Weighted-Region-Problem-optimal) weighted continuous
   pathfinding** — out of scope permanently for v1 (parent §11.3). The cost-guarded
   LOS smoothing is the accepted approximation.
2. **Polyanya native cost-layers / `detailed-layers` split-mesh** — rejected on
   merits (§2.2, source-verified infeasible/fragile in 0.16.1). Not a deferral; a
   permanent design decision for this crate version.
3. **Per-actor / faction movement exemptions**, **trigger/mechanical region effects**
   — remain homed to Phase 2 (unchanged from M10g §10).

## 13. Deviations recorded (update on merge)

- **`PLAN.md` M10f-4 entry** + the **parent spec §7** "terrain → polyanya cost-layer"
  sentence + **M10-tokens §10.2/§10.3** "cost-layers (Polyanya)" claim: struck and
  replaced by "shared cell field + Euclidean-weighted grid search + LOS smoothing;
  polyanya does not weight." Cite this spec §2.2 (crate-source verification) as the
  reason.
- The M10-tokens §10.5 engine-choice rationale's premise that Polyanya has "built-in
  cost layers (weighted regions)" is annotated as inaccurate for 0.16.1; the
  engine-choice conclusion (adopt polyanya for continuous any-angle geometry) stands —
  only the weighting mechanism changes.

## 14. Testing

Mirror the M10g §11 region suite over the **continuous** route:

- **Dispatch:** a continuous scene with terrain routes via the weighted path; a
  walls-only continuous scene routes via pure polyanya (assert the branch taken, e.g.
  via a route-shape or a `#[cfg(test)]` seam).
- **Terrain bends the route + cost:** a terrain region between start and goal shifts
  the continuous route toward cheaper cells and raises the reported `cost`; base
  metric is Euclidean (√2 diagonals, not the world diagonal rule).
- **Impassable routes around** on the continuous engine (not truncated-into).
- **Visible arrest truncates** + sets `arrested`; **secret arrest** is absent from a
  player's continuous route but **sprung authoritatively** by `move_exec`.
- **Secret terrain** does not inflate a player's continuous cost or bend their route,
  but is applied at execution.
- **LOS smoothing correctness:** the smoothed route crosses no wall, enters no
  impassable cell, and enters no cell exceeding the span-max multiplier; smoothed
  `cost ≤` grid-route `cost`; a degenerate span falls back to the un-smoothed subpath.
- **`route ⊆ gate-allowed`** on continuous: a smoothed weighted route never contains a
  sample outside the requester's mask (reuse the M2/M3 no-leak suite over weighted
  any-angle paths).
- **Execution parity:** `execute_move` on the weighted+smoothed continuous route
  produces the same admit/deny/arrest/cost outcomes the cell field dictates
  (engine-agnostic executor, zero production changes — the test is the proof).
- **`MoveStream.cost`** is `None` for a clipped observer of a continuous weighted move,
  `Some` for the mover/GM.

## 15. Execution

Per-checkpoint plan via `writing-plans` → `subagent-driven-development` (per-task
two-reviewer gate + whole-branch buddy-check), matching the M10 cadence. **Reviewed
skill-update gate:** update `shadowcat-codebase-scene-rendering` (the continuous
weighted-route dispatch, the `Euclidean`-forced grid reuse, the cost-guarded LOS
smoothing seam, the "polyanya does not weight — cell field is the universal overlay"
invariant, and the struck parent §7 claim) and confirm via `shadowcat-spec-reviewer`
before merge. Mirror any `pathfinding.rs` / `move_exec` seam changes documented there.
