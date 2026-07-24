---
name: shadowcat-codebase-scene-rendering
description: "Use when touching Shadowcat scenes, the scene ECS, rendering, the PixiJS canvas/stage, vision raycasting, fog of war, lighting, the server visibility/lit mask, movement restriction (the Room::publish move gate, supercover, visible_cells), the grid A* pathfinder (scene/pathfinding.rs, SceneEcs::pathfind, Pathfind/PathResult frames, diagonal rules), the continuous/navmesh router (movementModel axis, scene/navmesh.rs, polyanya, the navmesh cache, clip_to_visible_mask), streamed continuous vision (MoveStream, scene/move_stream.rs, player_vision_polygons_at, the per-recipient egress clip, client fog-sweep/cross-fade playback), regions (weighted/impassable/arrest zones, region docs, region-view.ts render layer), multi-scene viewing (viewedSceneId, resolveViewedScene, world-settings.activeScene, GM local roam, scene-scope.ts), or scene-tools (place/select/move/draw/template/measure/ping/wall/region). Covers src/server/src/scene, src/client/render, src/modules/{stage,scene-tools}. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Scene & Rendering

Orientation for the server scene ECS + vision/fog and the client PixiJS render layer + scene-tools.

## Purpose

Scene/runtime state is **derived** from documents into a per-world ECS (ephemeral). The server
runs engine-owned geometry (movement-collision, per-player vision); the client renders the
**optimistic** document view through an engine-owned PixiJS layer, with interactive tools.

## Key files & seams

- `src/server/src/scene/mod.rs` — `SceneEcs` (derived read-model, hydrated from documents + the
  M10e-2 config-doc/actor side-tables `world_settings`/`gradation`/`vision_modes`/`actors`, set via
  `set_world_config`/`set_actors` and maintained by `apply_op`), `compute_derived(...)` (builds
  derived frames; the `vision` masked payload is `{mode, polygons, bands, lit}`),
  `player_vision_polygons(user_id)`, `player_lit_mask(user_id)` (the M10e-2 lighting-aware mask →
  `LitScene` cells), and the fail-closed server resolvers `resolve_scene`/`resolved_bands`/
  `resolved_vision_modes`/`token_vision_floors` (mirror scene-docs.ts + actor.ts `resolveTokenActor`
  precedence) plus `scene_lights`/`light_walls` accessors. `resolve_scene` also yields `bounds:
  (f64,f64)` (M10f-0, `width,height` in grid units), read from the typed `eng::SceneEngine.bounds`
  (M13-0 re-root — the pre-M13-0 `s.pointer("/bounds/width")` structural walk is gone; every scene
  reader now goes through `engine_as::<T>(doc)`, the module-local typed accessor, see below); non-
  finite or ≤0-on-either-axis falls back to
  `DEFAULT_SCENE_BOUNDS_UNITS = (100.0, 100.0)`, which MUST numerically match the client's
  `DEFAULT_SCENE_BOUNDS` in `scene-docs.ts` (dual-language default-parity invariant — verify both
  when either changes). Per-scene only, no world-settings layer. **Movement gate (M10e-4):**
  `visible_cells(user, scene, lenient)` is the move-gate mask — under strict (center) sampling it
  EQUALS `player_lit_mask`'s cells (spec §13) because both share `cell_visible` / `lighting_inputs` /
  `source_los_poly` / `point_qualifies`; `lenient` adds the 4 corners (a superset, never a
  zero-overlap cell). `resolve_scene` also yields `movement_restriction`
  (`MovementRestriction::{Visible,Revealed,Unrestricted}`, scene-overridable, fail-closed to `Visible`)
  + `partial_cell_leniency` (world-only).
  **THE PARITY CHECKLIST — `Room::publish` (drag) and `move_exec`/`execute_move` (moveRequest) are
  two gates that MUST agree, and they have diverged on SIX independent axes. Adding a new gate
  input means adding a row here and pinning it.** Each row cost a defect, two of them Critical:
  (1) **per-cell decision** — same `blocks_move` + `GridShape::line_traversal` + `visible` mask;
  (2) **cell indexing** — the same resolved `GridShape`, never the free square functions;
  (3) **traversal completeness** — a supercover on both grid kinds, never a thin line;
  (4) **input admissibility** — both share `MAX_GATE_WALK_COORD`, checked before any traversal, in
  EVERY restriction mode (`Unrestricted` short-circuits later, so a check placed after it agrees in
  two modes of three and forks in the third);
  (5) **scene identity** — DERIVED from the token, never the frame (below);
  (6) **fail-open defaults** — an absent `scene_grid_sizes` entry means no scene document and must
  REFUSE, never synthesize a 100-unit grid.
  SCOPE caveat that applies to the whole comparison: `publish`'s gate block is non-GM-only, while
  `execute_move` and `gate_walk` bound unconditionally including GMs. Pin each axis with an
  anti-drift test exercising BOTH gates through the shared symbol — see `MAX_GATE_WALK_COORD`'s and
  the dangling-parent-scene one.
  **INVARIANT — a movement/routing gate's SCENE is DERIVED FROM THE TOKEN, never taken from the
  frame (Task 14j, `[sec]`, fixed a Critical).** `Room::execute_move` resolves the scene via
  `SceneEcs::token_move(token, &[])`, the same accessor `Room::publish` has always used — which is
  why the drag path was never vulnerable — and EVERY gate input (restriction, cell size,
  `visible_cells_cached`, `get_explored`, and `move_exec`'s walls/regions/grid shape) keys on that
  derived scene. `MoveExecution.scene` carries it out, and `MoveStream.scene` is stamped from it, so
  the per-recipient egress clip and the client's viewed-scene filter cannot key on a client value
  either. A request whose `scene_id` disagrees is additionally refused, but that is redundant
  defense-in-depth: the derivation is the mechanism. **Why:** `MoveRequest` previously trusted the
  client's `scene_id` while reading the token's position scene-agnostically, so a player owning a
  token in scene A could have the gate evaluated against scene B — B's walls, their own mask in B,
  B's regions — and teleport through fog in A. Authorization was intact (they owned the token); it
  was a total bypass of the wall + visibility gate. **A routing request that names NO token**
  (`Pathfind`) cannot derive, so a non-GM must instead prove PRESENCE: they must effectively own a
  token in the named scene, routed through the same effective-ownership rule (never a forked
  ownership check), failing with the generic `Unreachable` so it discloses nothing.
- **`/engine` re-root (M13-0).** Every scene/vision/movement/pathfinding document read in this
  subsystem now goes through the typed `engine` band, not a `/system` pointer walk. `mod.rs`'s
  private `engine_as::<T: DeserializeOwned>(doc: &Document) -> Option<T>` (`doc.engine.as_ref()
  .and_then(|v| serde_json::from_value(v.clone()).ok())`) is the module-local typed accessor every
  reader calls — a `None` result (absent `engine`, or a stored value that fails to parse) is
  treated identically to the pre-M13-0 pointer-walk's `None`, so every caller's OWN existing
  field-level fail-closed backstop (bounds → `DEFAULT_SCENE_BOUNDS_UNITS`, grid size default 100,
  etc.) is unchanged. `data/engine::{TokenEngine, SceneEngine, WallEngine, RegionEngine,
  WorldSettingsEngine, LightEngine, ...}` (re-exported here as `eng::*`) are the typed structs
  `engine_as` deserializes into. The DELETED pre-M13-0 `sys_f64`/raw pointer-walk helper no longer
  exists anywhere in this subsystem — do not reintroduce a pointer-walk reader; add a new typed
  field to the relevant `eng::*Engine` struct instead. `region_field`'s per-requester secrecy-tier
  lookup now reads `doc.permissions.property_overrides.get("/engine")` (was `"/system"` pre-M13-0)
  since a region's shape/behavior/cost live in `engine`; `setRegionVisibility`
  (`scene-docs.ts`) sets `property_overrides["/engine"] = "gm_only"` to match.
  `movementModel`/`snapToGrid` (below) are likewise now typed `SceneEngine` fields, ts-rs exported
  (`MovementModel`/`MovementRestriction` DO have ts-rs derives now — the pre-M13-0 "no ts-rs type,
  opaque `system`-body JSON" framing for these two fields is stale and must not be assumed).
  **`engine_as_cached` (Phase-1 A2 perf item, resolved):** the free function `engine_as` still
  fully re-`serde_json::from_value`-decodes on every call; `SceneEcs::engine_as_cached::<T>(&self,
  id: Uuid, doc: &Document) -> Option<T>` is the cached wrapper 18 of the ~19 hot-path call sites
  in `mod.rs` now go through (walls, tokens, scenes, regions, lights, the world-settings/gradation/
  vision-modes singletons, and actor-table lookups). Cache correctness is VALUE-COMPARISON based,
  not mutation-site invalidation: a cached entry (keyed on the document's own id) is reused only
  when its stored source `engine` `Value` still equals the document's current one — `apply_op` is
  NOT the only place a `Document` in this ECS gets mutated (`set_world_config`/`set_actors`, the
  room-hydration setters, assign fields directly, bypassing `apply_op` entirely), so an
  invalidate-on-every-mutation-site design would have been incomplete by construction (caught by
  two real test failures — `diagonal_rule_reads_world_settings_and_unknown_falls_back` and
  `token_vision_floors_resolve_through_actor_join` — when first tried). `apply_op` still removes
  the touched id's entry as a best-effort trim (bounds memory on delete), but this is NOT
  load-bearing for correctness. The ONE deliberately-uncached call site is `token_vision_floors`'s
  embedded-actor branch (`token.embedded.get("actor")...`): an embedded actor sub-document's own
  `id` differs from the owning token's `id`, so caching it under its own id would never be
  invalidated by a token-level mutation that changes `/embedded/actor/0/...` — this is the same
  failure shape as the two test bugs above, generalized to a case with no test coverage to catch
  it, so it stays on the direct, uncached `engine_as` path.
  **`visible_cells_cached` (Phase-1 perf item, resolved):** `SceneEcs::visible_cells_cached(user,
  scene, lenient) -> BTreeSet<(i32,i32)>` is a per-`(user, scene)` memoized wrapper around the same
  mask `visible_cells` computes for the M10e-4 movement gate — `visible_cells` itself and every
  other existing caller (pathfinder, §13 parity tests) are unchanged and still call the uncached
  primitive. Keyed on `VisibilityInputsSnapshot` (`{lenient, settings, cell, sources: Vec<(id, vp,
  floors)>, lights, light_walls, sight_walls}`) — a VALUE-COMPARISON cache like `engine_as_cached`,
  not mutation-site invalidation: a cached mask is reused only when a freshly rebuilt snapshot
  compares equal to the one stored alongside it, so correctness is independent of which code path
  mutated the underlying documents (`apply_op`, `set_world_config`/`set_actors`, or any other
  setter). `sources` is sorted by id before hashing so hecs' non-stable iteration order can't cause
  a spurious mismatch. The snapshot already covers everything `env_light_polys` occlusion depends
  on (`settings.bounds`, `cell`, `light_walls`), so the M13-0-era `env_polys` addition to
  `lighting_inputs_from` needed no cache-key change to stay correct.
- `src/server/src/scene/movement.rs` — pure `supercover_cells(a0, a1, cell) -> Option<BTreeSet<(i32,i32)>>`
  (M10e-4): every cell the move segment crosses (supercover, not a thin line — an exact corner crossing
  emits BOTH flanking cells so a diagonal can't thread an unseen cell). `None` ⇒ caller fails closed
  (`cell<=0.0` / non-finite endpoint / span > `MAX_MOVE_CELLS`). Clean-room (Amanatides–Woo extension);
  relative-epsilon corner test (over-include is the safe direction).
- `src/server/src/scene/vision.rs` — raycast `visibility_polygon(viewpoint, walls, bound)`,
  `bound_for(...)`, `Seg`/`Rect`/`P`, `point_in_poly` (shared). Public-source computational geometry only (ARCHITECTURE §7).
- `src/server/src/scene/lighting.rs` — pure illumination (M10e-2, no I/O — callers pass parsed
  structs): gradation `Band`s (`sorted_bands`/`band_index`/`floor_min`), `Light` radial falloff
  (`light_illumination`), `cell_illumination` (max-compose env + lights, `blocksLight` occlusion via
  `point_in_poly`). Clean-room. Non-finite/empty inputs fail closed (under-reveal).
- `src/server/src/scene/move_exec.rs` — pure, lock-free `execute_move(ecs, scene, token, path,
  restriction, visible, cell) -> Result<MoveOutcome, MoveReject>` (M1 server-authoritative
  movement; **engine-agnostic since M10f-2**): `path` may be ANY polyline — grid A* cell-center
  vertices ≤1 cell apart, or any-angle continuous vertices arbitrarily far apart. `gate_walk`
  (M10f-2, new pure primitive, same file) subdivides it into a DENSE walk where every consecutive
  sample is ≤1 cell apart (Chebyshev), preserving already-≤1-cell input segments EXACTLY —
  identity on grid input (cell-center vertices, ≤1 cell apart on every axis incl. diagonals). This
  identity property is what makes grid-parity a property of the code shape rather than something
  proven only by testing; empirically, a temporary `execute_move_kingstep_oracle` (a frozen,
  `#[cfg(test)]`-only verbatim copy of the pre-M10f-2 king-step executor) was added, used to
  prove parity across 10 scenarios, then DELETED once those cases were frozen as literal
  fixtures — a second permanent executor would reintroduce exactly the fork this refactor exists
  to avoid. The frozen fixture test,
  `frozen_parity_king_step_paths_match_previously_oracle_verified_outcomes`, is now the permanent
  regression proof. The per-step gate — (1) wall gate (`blocks_move`, all modes incl. GM), (2)
  vision-mask gate (`supercover_cells` + `visible` membership, skipped for `Unrestricted`), (3)
  region gate (M10g — see below) — runs over this DENSE walk, not the raw authored path; the
  coarse `render_path` returned to the caller is reconstructed as either the authored-vertex
  prefix (when the stop lands exactly on an authored vertex — always true for grid input) or the
  authored-prefix + the exact stop point (when the stop lands mid-subdivision — only possible for
  a genuinely long/any-angle continuous segment). **Guard relaxation (M10f-2):** the pre-M10f-2
  king-step adjacency guard (reject any >1-cell authored jump as `Degenerate`) is REMOVED — a
  >1-cell jump is now subdivided and gated per cell instead, exactly as if the client had sent the
  explicit intermediate waypoints (no new capability; security lives entirely in the per-cell
  gate, never the shape check). **DoS bound (M10f-2):** `MAX_GATE_WALK_SAMPLES=4096` (dense
  sample count, arc-length-based) + `MAX_GATE_WALK_COORD=1e9` (a coordinate-magnitude bound inside
  `gate_walk` itself, closing a false-identity failure mode where the identity-comparison's
  magnitude-scaled floating-point tolerance could otherwise grow large enough at extreme
  coordinates to silently misclassify a genuinely-multi-cell segment as identity — buddy-check
  caught this as a second-order defect introduced by the FIRST fix for a related zero-tolerance
  identity bug at non-round `cell` sizes) REPLACE the pre-M10f-2 `MAX_MOVE_PATH=256`
  authored-vertex-count cap; `MoveReject::TooLong` now reflects `gate_walk`'s `None` (either
  cap), not vertex count. `MoveReject` variants: `NotAToken`, `EmptyPath`, `TooLong` (as above),
  `Degenerate` (non-finite coords / bad start — no longer covers non-adjacent king-step, which is
  now subdivided-and-gated rather than rejected). **Region gate (M10g, step 3):** always reads
  `ecs.region_field(scene, None)` — the AUTHORITATIVE field, computed once before the walk loop
  begins (never per-step, never filtered) — so a `gm_only` secret region "springs" on execution
  even for a mover whose own route preview couldn't see it. Center-cell only (`to_cell(next)`; no
  footprint check) — a documented asymmetry against the router's footprint-aware
  `cell_enterable` (router stricter, executor looser), but `route ⊆ gate-allowed` still holds
  because the router's mask predicate is already a superset (see the pathfinder invariant below).
  **Keyed on CELL-ENTRY TRANSITIONS (M10f-2), not per dense sample:** a continuous path subdivided
  into several sub-cell samples within the same cell is evaluated exactly once for that cell,
  matching the pre-M10f-2 per-authored-step accrual count on grid input (where every step already
  crossed into a distinct new cell); a non-consecutive re-entry into a previously-visited cell (A
  → B → A) still re-evaluates correctly since the dedup only compares against the IMMEDIATELY
  prior cell, never a stale earlier value. Impassable stops BEFORE entry into the cell (like a
  wall — `stop` lands on the prior cell); arrest stops AT entry (the cell is entered, then the
  walk halts — a final-step arrest still sets `truncated: true` even though `stop_index ==
  path.len()-1`). `MoveOutcome.cost` accumulates `regions.terrain_multiplier(region_cell)` per
  cell-entry (1.0 outside any terrain region — this is a per-step-distance BASELINE, not merely
  additive terrain weighting; a plain grid move with no regions at all still accrues `1.0` per
  step) — center-cell-only, terrain-only; it does NOT apply the diagonal-rule step-cost factor
  (`sc` — 1.0/2.0/√2/alternating) that `pathfinding.rs`'s router applies. **Known, logged
  inconsistency (`docs/TODO.md`):** the two `cost` values are numerically comparable only under
  Chebyshev (where the diagonal step cost is 1.0); under any other diagonal rule they diverge. This
  is a deliberate v1 scoping decision (M10g Task 7), not a bug — nothing currently consumes or
  compares the two costs together. Resolve before any per-turn movement-budget system consumes
  either `MoveOutcome.cost` or `MoveStream.cost`. **RESOLVED (Phase-1 sweep):** `supercover_cells`'s
  lattice-corner-tie drift (a diagonal king-step whose leg endpoints both sit exactly on 4-way
  grid-line intersections could spuriously fail-closed) is fixed via a per-axis remaining-step
  budget gating the diagonal corner branch — see `docs/CLOSED_BUGS.md` for the root cause and fix.
- `src/server/src/scene/mod.rs` — adds `SceneEcs::token_position(token) -> Option<(f64,f64)>` and
  `SceneEcs::resolved_animation_speed() -> f64` (`pub(crate)` seams; the latter sits alongside
  `resolved_diagonal_rule`, sources `world_settings.animation`, defaults to 6 cells/sec).
  **M2 streamed-vision seam:** `SceneEcs::player_vision_inputs(user, scene, moving_token) ->
  VisionMoveInputs` hoists the per-move-invariant inputs (full `sight_walls` set + the user's
  OTHER owned tokens' static polygons) **once per move**; `VisionMoveInputs::polygons_at(viewpoint)`
  (also exposed as the convenience wrapper `SceneEcs::player_vision_polygons_at(user, scene,
  moving_token, viewpoint)`) is the cheap per-sample call — raycasts the moving token from
  `viewpoint` against the SAME full wall set (including `gm_only` sight walls) and unions it with
  the pre-hoisted static polygons. Empty when the user owns no token in the scene (fail-closed).
  Reused primitives, not a new vision model: identical `sight_walls` + `vision::visibility_polygon`
  as `player_vision_polygons`.
- `src/server/src/scene/move_stream.rs` (M2) — pure, no-I/O position/vision path sampler for the
  `MoveStream` broadcast: `sample_path(path, cell, duration_ms) -> Vec<PosSamplePt>` (arc-length
  parameterization; ~`SAMPLES_PER_CELL`=3 samples/cell; always includes the exact first/last
  vertex; strictly increasing `t_ms`, exact-equal consecutive dedup). `MAX_VISION_SAMPLES` (96) is
  the SHARED cap for both position samples and vision samples on one `MoveStream` frame — bounds
  the mover's per-move raycast count. `MAX_VISION_POLYGON_VERTS` (512) caps each `VisionSamplePt`
  polygon's vertex count (fail-closed truncation — under-reveal, never over-reveal).
  `Room::execute_move` calls `sample_path` then, for each sample, `player_vision_inputs` (once) +
  `VisionMoveInputs::polygons_at` (per sample) to fill `MoveExecution.mover_vision` (`None` for a
  GM mover — `Unrestricted` sees all, nothing to sweep).
- `src/server/src/ws/conn.rs` — **the M2 per-recipient egress clip is the secrecy boundary** for
  `MoveStream`. `handle_move_request` broadcasts the FULL (unclipped) `MoveStream` via
  `room.broadcast_aux` — the full trajectory lives only in-process. `egress_loop`'s dedicated
  `MoveStream` branch (`clip_move_stream` + `observer_vision_polys_for_scene`) runs BEFORE the sink
  write, per connection, in four branches: the mover gets `samples` + `mover_vision` unchanged
  (keyed on the REAL connection `user_id`, never a see-as target — a GM previewing as someone else
  is not "the mover" unless the GM's own token is what moves); a plain GM (no active see-as) gets
  the FULL `samples` unclipped (GMs bypass position secrecy) but `mover_vision` forced to `None` (a
  GM has no fog to sweep); a GM with an active see-as (`SceneSubscribe`-set `scene_subs` target)
  gets `samples` clipped to the see-as TARGET's own authoritative vision
  (`observer_vision_polys_for_scene(target.user_id, scene, room)`) instead of the plain-GM full
  stream — an empty result (target has no vision source in this move's scene) falls back to the
  full GM stream rather than clipping to nothing, since the see-as doesn't apply there; every other
  (non-GM, non-mover) recipient gets `samples` clipped to those whose `pos` falls inside the
  recipient's OWN authoritative vision polygons (`point_in_poly`, recomputed off the current ECS
  read — never a stale cache; the ECS guard drops before any await) with `mover_vision` also forced
  to `None`; a wholly-invisible move (empty clip) is **not sent at all** (suppressed, not an
  empty-`samples` frame — asserted by a dedicated test). The see-as branch can only NARROW what a
  GM receives relative to the plain-GM fallthrough, never widen a non-GM recipient's own view (see-
  as is GM-only, gated by `SceneSubscribe`'s `as_user` handler). `send_filtered` intentionally
  panics if a `MoveStream` reaches it — the clip MUST happen in the dedicated `egress_loop` branch,
  never the generic per-recipient filter path. `MoveError` stays mover-only via `etx`, generic (no
  path/vision geometry disclosed).
- `src/server/src/scene/explored.rs` — `ExploredSet` fog memory: `mark_polygons(polys, cell_size)`,
  `to_bytes`/`from_bytes` (persistence), cell-based.
- `src/server/src/scene/regions.rs` (M10g) — pure region geometry, no ECS/I/O (mirrors
  `movement.rs`'s module invariant): `RegionShape` (`Rect`/`Circle`/`Polygon`), `RegionBehavior`
  (`Terrain`/`Impassable`/`Arrest`), `RegionEffect` (composed per-cell result), `rasterize(shape,
  cell)` (grid cells whose CENTER falls inside the shape — fails closed to `None` on a degenerate
  shape (non-finite coords, non-positive radius, `<3`-vertex polygon) or an over-cap AABB
  (`MAX_REGION_CELLS`=100_000), never a partial/silently-empty result), `compose(contributions)`
  (precedence `Impassable > Arrest > Terrain`; overlapping Terrain costs take the MAX, never
  summed), `RegionField`/`RegionFieldBuilder` (`.add` silently drops a fail-closed shape —
  contributes nothing, never all-passes; `.build` composes per-cell), and `parse_region_shape`
  (structural-only parse of a region doc's `/shape` body; any malformed field fails closed to
  `None`, dropping the whole region). **Historical bug (fixed):** an extreme-magnitude finite
  coordinate (e.g. `-1e300`) saturated the float→cell-index cast to `i64::MIN/MAX`, letting the
  subsequent `i1 - i0 + 1` cell-count arithmetic overflow BEFORE the `checked_mul` DoS-cap check
  ever ran — a bypass of the cap, not merely a wrong count. Fixed via an explicit `MAX_CELL_COORD`
  (`i32::MAX - 1.0`) bound checked on all four AABB edges before any i64 arithmetic. A
  large-magnitude-but-small-span coordinate (e.g. `1e13`) is a separate failure mode also caught by
  the same bound (would otherwise silently truncate/wrap under `as i32`, aliasing onto an unrelated
  real cell).
- `SceneEcs::region_field(scene, viewer)` (`mod.rs`, M10g) — the composed `RegionField` for a
  scene. **Two-value contract, never a third mode:** `viewer: None` is the AUTHORITATIVE view
  (every enabled region, unfiltered) — used by the GM and by `move_exec` (which always springs
  secret regions on execution regardless of what the mover's own route preview could see);
  `viewer: Some(user)` is the PER-REQUESTER view used by the grid A* router — a region is included
  only when `user` can see the visibility tier declared on its `/engine` (M13-0 re-root; defaults
  to `All` when undeclared), via the SAME `resolve_access`/`property_overrides["/engine"]`
  mechanism that already
  gates every other document's egress (spec §3: no new secrecy machinery). **Callers MUST pass
  `None` for a GM requester** — mirrors `visible_cells`'s GM-skips-the-mask convention; passing
  `Some(gm_user)` would incorrectly filter a GM's own field.
- `src/server/src/scene/pathfinding.rs` — pure, headless grid A* (no I/O; clean-room):
  `DiagonalRule` (`chebyshev`|`manhattan`|`euclidean`|`alternating`) + `resolved_diagonal_rule`
  (world-only — no per-scene override; mirrors `resolveSceneSettings` precedence); `PathGrid` (wall-
  segment lookup built from `move_walls`); `cell_enterable(grid, from, to)` — four checks, ALL must
  pass: (1) footprint-disc-vs-wall clearance (the token's bounding disc must clear ALL `blocksMove`
  segments, via `point_segment_distance`); (2) **mask** — every cell in `footprint_cells(to,...) ∪
  movement::supercover_cells(cell_center(from), cell_center(to), cell)` must be in the non-GM mask
  (M3: the union closes buddy-check P1 — footprint-disc-at-destination alone missed a diagonal
  step's corner-flanker cells for sub-0.5-cell footprints, letting the router approve a step the
  M1 executor then rejected; `None` from `supercover_cells` fails closed); (3) center-to-center
  step crosses no wall (`segments_cross`); (4) `region_arrests`/impassable check via `PathGrid.regions:
  Option<&RegionField>` (M10g — see below; a `None` grid field means "no region enforcement",
  distinct from an empty `RegionField`). `astar_leg` — king-move A*, 4 diagonal
  rules, 5-10-5 parity tracked in the `(cell, parity)` node and carried across waypoint legs (cost
  1,2,1,2…, never reset per leg), admissible+consistent heuristics per rule, stale-pop skip,
  `MAX_PATH_NODES`/`MAX_WAYPOINTS`/`MAX_FOOTPRINT_CELLS` fail-closed bounds; **M10g terrain
  weighting:** the step-cost function multiplies the diagonal-rule base cost (`sc`) by
  `grid.regions.map_or(1.0, |r| r.terrain_multiplier(next))`, so a terrain region raises (never
  lowers — multipliers are validated `>= 1.0` at `region_field` construction) the A* edge weight
  into that cell, honored by the admissible/consistent heuristic (which already lower-bounds the
  UNWEIGHTED cost, so remains admissible under any `>=1.0` weighting). `find` — validates
  request, computes search window (AABB{start∪waypoints∪wall-endpoints}+8-cell margin), threads
  end-parity of each leg into the next, sums cost, returns ordered cell-center scene coords, THEN
  (M10g) applies **arrest truncation**: cuts the assembled route at the first cell (after the
  start — a token already standing in a cell is not "entering" it) flagged `is_arrest` in the
  region field, sets `arrested: true`, and recomputes the truncated `cost` by REPLAYING
  `step_cost` over the surviving prefix from parity 0 (a cost-replay technique, not trusting
  `astar_leg`'s per-leg running total, because parity threading is purely sequential/order-
  dependent — replaying reproduces the exact cost the original per-leg accumulation would give for
  that same prefix). Returns `PathOutcome { path, cost, arrested }`. This truncation exists so a
  player-facing route preview is honest about a hazard it already knows about (spec §5: "arrest is
  honest in preview") — never shows a route running past an arrest cell the requester can see.
  `SceneEcs::pathfind` — reuses the SAME `visible_cells` mask as the M10e-4 movement gate (**§13
  invariant: never fork the per-cell visibility decision** — the route cannot thread the unknown nor
  leak hidden geometry); unions `explored` (`ExploredSet::iter`) for `revealed`; GM unconstrained
  (no mask); empty non-GM mask ⇒ `PathError::Unreachable` (fail-closed); passes
  `ecs.region_field(scene, if is_gm { None } else { Some(user) })` — the PER-REQUESTER field (a
  non-GM's route/budget silently omits a region they cannot see; the secret region only "springs"
  later, at `move_exec`, which always reads the authoritative field — see `region_field` above).
  New `move_walls(scene)` accessor returns the `blocksMove` segment list (mirrors the M9
  `blocks_move` filter). Wire frames `Pathfind`/`PathResult` (`{path, cost, arrested}` —
  `arrested` is always disclosed to the requester, no secrecy concern: it only tells them a route
  THEY could already see is truncating)/`PathError` — one-shot to the requesting connection only
  (never broadcast); `get_explored` fetched off the scene read lock (no lock across await).
  Client: `ToolContext.pathfind?` seam + `SceneTool.onDeactivate?()` hook in scene-tools (clears
  route overlay on tool swap); ruler `Grid.distance()` gains the `alternating` (5-10-5) rule wired
  from `resolveSceneSettings(...).diagonalRule` into the Stage `GridSpec`.
- **`movementModel` axis (M10f-1)**: a per-scene/world-default routing-engine choice
  (`MovementModel::{GridStepped,Continuous}` server-side, `MovementModel = "grid-stepped" |
  "continuous"` client-side), resolved by `resolve_scene`/`resolveSceneSettings` with the EXACT
  same shape as `movement_restriction`/`MovementRestriction` (world default in
  `WorldSettingsEngine.scene` at `/engine/scene`, a per-scene override in `SceneEngine.vision` at
  `/engine/vision`, fail-closed to `GridStepped` on unknown/absent — never silently promotes a
  scene to the newer engine). **M13-0 re-root:** both `MovementModel` and `MovementRestriction`
  are now typed `engine`-band fields, ts-rs exported (`data/engine/scene.rs`) — the pre-M13-0 "no
  ts-rs type, opaque `system`-body JSON" framing for this axis no longer holds; do not assume
  either enum is untyped. `SceneEcs::pathfind`
  dispatches on `resolve_scene(scene).movement_model`: `GridStepped` calls the unchanged
  `pathfinding::find`; `Continuous` calls `navmesh_for` → `navmesh::navmesh_find` →
  `navmesh::clip_to_visible_mask` (below). Both branches build the per-`(user,scene)` visibility
  mask ONCE, above the dispatch, and pass the SAME reference into whichever engine runs — never
  forked (mirrors the pathfinder's own §13 invariant, generalized to a second engine). Client:
  `movementModel` world-default + scene-override editor in `GameSettingsPanel.svelte` (mirrors the
  `movementRestriction` editor exactly). **M10f-1 shipped router + preview only**: the measure-tool's
  `commitRoute` (`controller.svelte.ts`) refused to send a `moveRequest` when the active scene's
  `movementModel` was `"continuous"`, by design for that checkpoint. **M10f-3 lifted that
  restriction** — `commitRoute` no longer branches on `movementModel` at all; committing a route
  proceeds identically for grid-stepped and continuous scenes. This is possible because the server
  move-execution path (`execute_move`/`gate_walk`/`sample_path`/the M2 egress clip) has been fully
  engine-agnostic since M10f-2 — no `movementModel` branch anywhere in that path, so there was
  nothing engine-specific left to gate at the client. `requestRoute` (the preview path) was always
  unaffected — no grid-snap fallback, silent no-op on double-click.
- **`snapToGrid` axis (M10f-3, `src/client/core/src/scene-docs.ts`)**: `SceneEngine.snapToGrid?:
  boolean` (M13-0 re-root: typed `engine`-band field, ts-rs exported — was opaque `system`-body
  JSON pre-M13-0, mirrors `movementModel`/`bounds`'s same re-root).
  `resolveSceneSettings` resolves a DERIVED DEFAULT keyed off the already-resolved
  `movementModel`: `sys?.snapToGrid ?? (movementModel === "continuous" ? false : true)` — an
  explicit stored boolean (including `false`) always overrides the derived default in either
  direction via nullish-coalescing, never a truthy check (`false` is meaningful and must
  persist). Independent of `movementModel` (a deliberate design choice — an independent toggle
  rather than tying no-snap directly to the movement model — though the derived default preserves
  the original intent that a fresh continuous scene is free-form by default). **`RenderEngine.snap`
  chokepoint (`src/client/render/src/engine.ts` + `types.ts`)**: `SceneToolHost.setSnapEnabled(enabled:
  boolean): void` interface member; `RenderEngine` carries a private `snapEnabled = true` field and
  gates `snap(p)`: `return this.snapEnabled ? this.grid.snap(p) : p`. This is the SINGLE
  enforcement point — every scene tool that calls `ctx.scene.snap` (place, select-move drag,
  measure-route waypoints, wall/region/template/draw tools) inherits the toggle automatically,
  since they all go through the same `AppContext.scene` bridge. Snap gating is independent of grid
  RENDERING — a snap-off scene may still display its reference grid; `setSnapEnabled` never
  touches `redrawGrid`/grid-line drawing. **Wiring:** `SceneInteractionBridge.setSnapEnabled`
  (`src/client/ui-kit/src/sceneInteraction.ts`) forwards to the host (no-op when detached,
  mirroring every other bridge method). `Stage.svelte` pushes the resolved `snapToGrid` into the
  engine unconditionally on every `onDocs` pass (`e.setSnapEnabled(settings.snapToGrid)`), placed
  OUTSIDE the `lastGridKey` change-detection gate that exists for `setGrid`'s more expensive
  Grid-object rebuild — a cheap flag write doesn't need that gate, and gating it behind
  `lastGridKey` would be a real bug since that key doesn't include `snapToGrid` and would silently
  freeze the pushed value. **Authoring:** a GM-only persistent toggle button in `ToolRail.svelte`
  (`data-testid="snap-toggle"`), reflecting the resolved `snapToGrid` via a reactive
  `createSubscriber`+`$derived.by` subscription to the document store (mirrors
  `FactionsPanel`/`GameSettingsPanel`'s pattern), dispatching a `/engine/snapToGrid` (M13-0
  re-root; was `/system/snapToGrid`) scene-doc update on click. **Load-bearing convention for any
  config-doc field-toggle editor:** the dispatched update's `old` field must read the RAW stored
  value (`scene.engine?.snapToGrid ?? null`), NOT the resolved/defaulted value — a hardcoded `old:
  null` breaks after the first
  successful write, since the server's field-level optimistic-concurrency check
  (`Repository::apply_intent`) rejects any subsequent `Update` whose `old` doesn't match the
  actual current stored value. This was a Critical bug caught and fixed during M10f-3 Task 5's
  review; the SAME pre-existing bug shape was found but NOT fixed (logged to `docs/TODO.md`
  instead, out of scope) in `GameSettingsPanel`/`FactionsPanel`/`ConditionsPanel` — always read the
  raw stored value for `old`, never the resolved/defaulted one, in any future editor of this shape.
- **Regions on the continuous engine (M10f-4, final M10f checkpoint).** `SceneEcs::pathfind`'s
  `Continuous` branch (`mod.rs`) computes the per-requester `region_field` once (same call the
  `GridStepped` branch already made — the `GridStepped` branch itself is completely untouched by
  M10f-4) and dispatches on `RegionField::has_terrain_or_impassable()` (`regions.rs`: true iff any
  cell is `impassable` or `terrain` with `multiplier > 1.0`; arrest-only fields do NOT trigger this
  — arrest needs only a post-filter, not route-bending). **Terrain/impassable present:** the
  existing `pathfinding::find` runs forced to `DiagonalRule::Euclidean` (continuous base metric —
  only cell topology + the terrain multiplier come from the grid, never the world's configured
  diagonal rule), its cost is converted from CELLS to SCENE UNITS (`× cell`, matching the polyanya
  path's unit contract — the two continuous sub-paths must report cost in the same unit regardless
  of which ran), then `navmesh::los_smooth` (new) restores any-angle geometry. The weighted sub-path
  does NOT call `clip_to_visible_mask` at all — its route⊆mask/wall safety comes entirely from
  `pathfinding::find`'s own per-cell mask gate (already fed `mask.as_ref()`) plus `los_smooth`'s
  own mask-checking `chord_ok` guard (every cell a straightened chord enters must still be in
  `mask`). **Otherwise:** the unchanged pure-polyanya route (M10f-1) runs `clip_to_visible_mask`
  FIRST, then `navmesh::truncate_at_arrest` (new) on the clipped result — clip-then-truncate, so a
  fog-truncated route can never carry a stale `arrested: true` flag past the point the fog itself
  should have cut it. `clip_to_visible_mask` is exclusive to the pure-polyanya sub-path — the two
  continuous sub-paths enforce the SAME mask invariant through different mechanisms, not through a
  shared call.
  - `navmesh::los_smooth(outcome, walls, mask, field, cell, footprint_radius_cells)` — cost-guarded
    LOS string-pull smoothing for the weighted continuous path. A span `path[i]..path[j]`
    straightens only when every cell its chord enters (`grid.footprint_cells ∪ grid.line_traversal`,
    the SAME union `cell_enterable`/`clip_to_visible_mask` use) is in `mask` (when `Some`), not
    impassable, not arrest, and not weighted terrain (`terrain_multiplier > 1.0`), and the chord
    crosses no `blocksMove` wall — so a straightened chord can never shortcut INTO terrain/
    impassable/arrest the weighted search deliberately routed around or truncated at. **The single
    grid step `path[i] -> path[i+1]` is ALWAYS kept unconditionally** (it already passed `find`'s
    per-cell gate), guaranteeing goal progress even when nothing else can straighten. Fail-closed on
    two levels: a whole-input short-circuit (`<3` vertices, degenerate `cell`/`footprint_radius_cells`)
    returns the input unchanged; a per-span fallback (an over-cap/degenerate `line_traversal` for
    one candidate chord) fails only that chord, leaving it at its single grid step while smoothing
    continues over the rest of the path. `cost`/`arrested` are carried through UNCHANGED (not
    recomputed) — the pre-smoothing weighted grid cost is a conservative (never-cheaper) budget for
    the straighter geometry, the same preview-vs-execution divergence class as the pre-existing
    `MoveOutcome.cost`/router-cost TODO (an exact per-span smoothed cost is deferred, `docs/TODO.md`).
  - `navmesh::truncate_at_arrest(outcome, field, cell)` — arrest post-filter for the pure-polyanya
    continuous path (which never runs through `find`, so needs its own arrest truncation, mirroring
    `find`'s M10g arrest logic for the walls-only route). Arc-length-samples the route
    (`move_stream::sample_path`) and cuts at the first sample whose cell **differs from the last
    distinct cell seen** and is `field.is_arrest(...)` — **cell-ENTRY-TRANSITION detection, not raw
    per-sample checking**: the start cell is never a trigger even while several samples still sit
    inside it (a token already standing somewhere is not "entering" it), matching `find`'s
    `.skip(1)`-over-cells convention. A route with no arrest transition is returned UNCHANGED (no
    resample, no cost recompute). On truncation, `cost` is recomputed as the Euclidean length of the
    surviving polyline and `arrested: true` is set.
  - **Both `los_smooth` and `truncate_at_arrest` are called with the PER-REQUESTER `region_field`**
    (`region_field(scene, if is_gm { None } else { Some(user) })`, computed once in `pathfind` and
    reused for the dispatch predicate too) — a secret region is absent from a non-GM's route/cost
    exactly as on the grid engine; `move_exec` alone reads the authoritative field and springs any
    secret region at execution. `move_exec`/`gate_walk` required **zero production changes** for
    M10f-4 — proven, not merely asserted, since M10f-2/3: it already cell-samples the region field
    for any polyline, grid or any-angle.
- `src/server/src/scene/navmesh.rs` (M10f-1, new) — pure headless adapter around the `polyanya`
  (any-angle navmesh) + `geo`/`spade` (CDT + Minkowski buffer) crates, engine-owned geometry
  (ARCHITECTURE §6 exception). Carries **walls only** in this checkpoint — impassable/terrain
  regions on the navmesh are a later checkpoint.
  - `build_navmesh(bounds, cell, walls, footprint_radius_cells) -> Option<NavMesh>` — triangulates
    the scene's bounds rectangle and inflates each `blocksMove` wall segment into a capsule
    obstacle (`geo::Buffer`) by the requester's footprint radius. **`MAX_NAVMESH_COORD` (1e15)**
    bounds EVERY value that reaches an `f64→f32` cast in this module (derived pixel bounds,
    raw wall-segment endpoints, AND `footprint_scene` — all three were found and fixed as separate
    Critical bugs across a multi-round buddy check: an unbounded-but-finite coordinate saturates to
    `Infinity` on cast, which `spade`'s triangulation rejects via an unhandled internal `.unwrap()`
    on `Err(InsertionError::TooLarge)` — a panic, not a fail-closed `None`). **Separately**, a
    non-degenerate wall whose `footprint_scene`-to-segment-length ratio exceeds ~4.9e8 makes
    `geo`/`i_overlay`'s internal fixed-point quantization collapse both endpoints to the same
    integer point, silently returning ZERO polygons from `.buffer()` — a distinct, more severe bug
    class (**silent fail-OPEN**: the wall obstacle vanishes from the mesh under inputs that pass
    every magnitude check, and a route can pass straight through where a wall should block).
    `build_navmesh` now treats an empty-buffer result for a non-degenerate wall as a hard
    whole-build failure (`None`), distinguishing it from a genuinely zero-length segment (a
    legitimate no-op, not a failure). `MAX_NAVMESH_OBSTACLE_SEGMENTS` (5000) caps wall count.
  - `navmesh_find(nav, start, waypoints) -> Result<PathOutcome, PathFail>` — any-angle multi-leg
    routing via `polyanya::Mesh::path`, Euclidean cost. **`polyanya::Path::path` EXCLUDES the query
    start vertex** (verified against the pinned crate source) — the leg-concatenation logic skips
    a returned vertex only if it coincides with the already-known `leg_start`, which is correct
    regardless of whether the crate includes or excludes it (don't "fix" this dedup logic assuming
    one behavior). Validates `waypoints.len() <= MAX_WAYPOINTS` and finiteness of `start`/every
    waypoint (parity with the grid router's own `Invalid` guard) — this specific magnitude bound is
    defense-in-depth/input-hygiene, NOT a proven panic-prevention fix (empirically verified: the
    query side, `Mesh::path`, is pure point-in-polygon containment and never touches `spade`'s
    triangulation, so it already fails closed to `None`/`Unreachable` without any guard — unlike
    `build_navmesh`'s construction-side guards, which DO close real reproduced panics).
  - `clip_to_visible_mask(outcome, mask, cell, footprint_radius_cells, walls) -> PathOutcome` — the
    **fog-safe + wall-safe preview post-filter**, THE security-critical function in this module
    (buddy-checked; every M9/M10e/M2/M10g/M3 milestone touching this invariant class has been).
    Arc-length-samples the route (`move_stream::sample_path`) and truncates at the first sample
    whose footprint cells (`grid.footprint_cells ∪ grid.line_traversal`, the SAME predicate
    `pathfinding::cell_enterable`'s mask check applies) leave `mask` — `mask: None` skips this
    check (GM/unrestricted). **Independently**, every chord (from the previous retained sample) is
    also tested against `walls` via `segments_cross`, ALWAYS (even when `mask: None`) — this is a
    router-FIDELITY guarantee, not a secrecy one (walls are public geometry): the true navmesh
    polyline can detour around a wall corner, but once downsampled to ≤`MAX_VISION_SAMPLES` (96)
    arc-length samples, an undersampled chord between two corner-straddling samples could otherwise
    visually cross the wall the true route avoided. Also validates `footprint_radius_cells` (against
    `MAX_FOOTPRINT_CELLS`), `cell`, and skips (not fails) any individual non-finite wall
    endpoint — mirroring `build_navmesh`'s defense-in-depth convention, since this function had been
    the one place in the module NOT following it (found + fixed in buddy check). **Two-checks
    dichotomy, never conflate:** the mask check is a genuine secrecy gate (route ⊆ gate-allowed);
    the wall check is a fidelity/correctness guarantee with no confidentiality stake — don't reuse
    one's severity framing for the other. Cost is recomputed as the Euclidean length of the
    truncated polyline, never the original route's cost.
  - `SceneEcs::navmesh_for(scene, footprint_radius_cells) -> Option<Arc<NavMesh>>` (`mod.rs`) —
    memoized per `(scene, quantized footprint radius)` (nearest 1/1000 cell — matches the design
    spec's explicit "cache stays bounded" requirement; a plan-level buddy check caught an earlier
    exact-f64-bits keying scheme as a departure from this). **Validates `footprint_radius_cells`
    BEFORE computing the quantized key or touching the cache** (a buddy-check finding: doing the
    validation after the cache lookup let `NaN`/small-negative inputs alias onto an already-cached
    LEGITIMATE mesh — e.g. `NaN` saturates to the same quantized key as `0.0` via `f64 as i64` —
    silently returning a valid-looking result instead of `build_navmesh`'s own fail-closed `None`).
    `navmesh_cache: std::sync::Mutex<HashMap<(Uuid,i64), Arc<NavMesh>>>` — `Mutex`+`Arc`, never
    `RefCell`/`Rc` (`SceneEcs` lives behind a `tokio::sync::RwLock` shared across connection tasks;
    the cache must stay `Sync`); never locked across an `.await`. Invalidated wholesale (all
    scenes, not just the touched one — over-invalidation is the safe direction) in `apply_op`
    whenever a `wall` or `scene` document is created/updated/deleted; the `Update` case resolves
    the existing entity's doc_type from the ECS `index`/`world` BEFORE the mutation runs (an
    `Update` never changes doc_type, so this pre-lookup is safe). A failed `build_navmesh` (`None`)
    is never cached.
  - **Grid/continuous engine parity, not just "shippable together":** the dispatch treats a route
    whose destination coincides with the start (any waypoint sequence that collapses to a
    zero-displacement request) as a legitimate zero-cost success on BOTH engines — the grid router
    has always had this via `astar_leg`'s explicit `start == goal` short-circuit; the continuous
    dispatch captures whether `navmesh_find`'s RAW (pre-clip) result was already length-`<2` before
    `clip_to_visible_mask` consumes it, and only maps `clipped.path.len() < 2` to `Unreachable`
    when the raw result was NOT already trivial — otherwise a "route to where you're already
    standing" request would succeed on grid-stepped scenes and spuriously fail on continuous ones.
  - Dependencies (Cargo.toml, `src/server/`): `polyanya = { version = "0.16", default-features =
    false }` (drops `async`/`recast` — blocking `Mesh::path()` only), `geo = "0.32"` (pinned to
    unify with polyanya's own dependency copy — one compiled `geo`, no duplicate), `glam = "0.30"`
    (used directly for `Vec2`; must be a DIRECT dependency even though polyanya also pulls it
    transitively — Rust requires a crate used by name to be declared directly). Binary-size delta
    measured at ~0.94 MiB against the 60 MiB CI ceiling — no bloat concern.
- `src/client/render/src/` — engine-owned PixiJS layer: `backend.ts` + `pixi-backend.ts`
  (renderer host), `engine.ts`, `reconciler.ts` (doc→scene reconcile), `compositor.ts`,
  `layers.ts` (CORE_LAYERS z-order; index 7 = `lighting`, between `templates` (6) and `mask` (8)),
  `camera.ts`, `grid.ts`, `token-view.ts` + `token-animator.ts` (tween),
  `wall-view.ts`, `drawing-view.ts`, `template-view.ts`, `ping-view.ts`. Modules draw through the
  render-layer API; the canvas host is not replaceable.
- **Token visual rendering (M10h — faces + animated token visuals).**
  `src/client/render/src/token-animation.ts` — `computeAnimatedFrame(elapsedMs, fps, frameCount,
  loop) -> number`, pure tick-driven frame-index math (extracted for the same reason as
  `fog-blend.ts`: `pixi-backend.ts` is Playwright-only, no jsdom GL context, so frame-selection logic
  needs to live somewhere unit-testable). `loop:true` wraps arbitrarily-large `elapsedMs`;
  `loop:false` clamps to the final frame (a one-shot animation holds, never re-wraps); degenerate
  input (`frameCount<=0`, non-finite `elapsedMs`/`fps`, `fps<=0`) fails closed to frame 0.
  `TokenNodeSpec.visual` (`types.ts`) is now a discriminated union: `{kind:"image", url} |
  {kind:"animated", source: ResolvedAnimatedSource, fps, loop}` (replaces the old flat `.url`
  field) — `ResolvedAnimatedSource = {type:"frames", urls:string[]} | {type:"sheet", url, rows,
  cols, count?}`, already asset-id-resolved to serve URLs by `AssetResolver` (the backend never
  resolves asset ids itself). `DisplayBackend.tickTokenAnimations(dtMs): void` — the new per-frame
  animation-advance seam, called once per frame alongside `startTicker`; `MockBackend`'s
  implementation is an intentional no-op (frame-advance state lives only in `PixiBackend`'s real
  `AnimatedSprite`s). `TokenView.tick(dtMs)` calls both `this.animator.tick(dtMs)` (transform tween,
  unchanged) AND `this.backend.tickTokenAnimations(dtMs)` (new). `TokenView.toSpec` resolves a
  token's visual via `resolveTokenVisual` (see `shadowcat-codebase-actors-tokens`) then a private
  `resolveSource` maps `AnimatedSource` → `ResolvedAnimatedSource` through `AssetResolver`.
  **`PixiBackend`'s Container-per-token structure** (`pixi-backend.ts`, migrated off a bare
  `Sprite`-per-token + three separately-tracked sibling Maps): one `TokenNode` per token —
  `container` (outer, does NOT rotate, positioned at the token center; `badges` are its DIRECT
  children so condition-marker glyphs stay upright regardless of token facing) →
  `visualContainer` (inner, rotates with the token via `.angle = spec.rotation`) → holds `visual`
  (a `Sprite` or `AnimatedSprite`) + `border` (`Graphics`) as siblings. `AnimatedSprite` playback is
  entirely tick-driven: `autoUpdate = false` (never Pixi's own shared ticker), frame index advanced
  in `tickTokenAnimations` via `computeAnimatedFrame`. `node.sourceKey` short-circuits a re-push
  with an unchanged visual (a tweening token's transform-only updates never touch the visual/sprite
  object). **Load-bearing invariant — guard async texture/frame-load completions on OBJECT
  IDENTITY, not just a string/key match:** `replaceVisualChild` can recreate a token's `visual`
  object (image↔animated kind-swap, or a rapid A→B→A visual-cycling sequence), so an in-flight
  texture/frame-load promise's completion callback MUST check `node.visual === sprite` (the exact
  object captured at load-start) in addition to `sourceKey`/`id` equality — a key-only check lets a
  stale promise write into an already-`.destroy()`'d Pixi object once the visual has been recreated
  more than once while the load was in flight. The animated branch's `replaceVisualChild` call is
  ALSO conditional (`if (!(node.visual instanceof AnimatedSprite))`), mirroring the image branch, to
  reduce how often the object gets recreated in the first place. Any future code touching this
  async-completion pattern (anywhere a display object can be replaced mid-flight) must follow the
  same object-identity-guard shape — a real bug of this exact kind was found and fixed during M10h.
- `src/client/render/src/engine.ts` (M2) — `visionSweeps: Map<tokenId, {samples, elapsed,
  durationMs}>` drives the mover's fog sweep during `MoveStream` playback (keyed per token — unions
  concurrent sweeps' visible sets rather than clobbering). `animateSamples(id, samples, durationMs,
  startServerMs, moverVision?)` starts a sweep only when `moverVision` is present (an observer never
  populates this — observers receive `moverVision: null` from the egress clip and simply tween
  position). While `visionSweeps.size > 0`, the engine feeds the SNAPPED (Task 6) or CROSS-FADED
  (Task 7) sweep polygon to the compositor instead of the last `vision` subscription payload;
  reverts to that payload the instant the sweep map empties (sweep end or catch-up completion).
- `src/client/render/src/fog-blend.ts` (M2 Task 7) — `computeFogBlendFactor(clock, tCur, tNext)`:
  pure, unit-testable blend-factor helper (0 at `tCur` → 1 at `tNext`, clamped `[0,1]`; a
  degenerate/non-finite span snaps to 1 — fail-safe toward the newer sample, never frozen on a
  stale one). Extracted from `pixi-backend.ts` because that file is WebGL-only (Playwright-covered,
  no jsdom GL context).
- `src/client/render/src/pixi-backend.ts` (M2 Task 7) — `setVisibilityBlend(from, to, factor)`
  rasterizes both the outgoing and incoming vision-sample fog into `RenderTexture`s via the shared
  `captureFog`/`paintFogSheets` helpers (the SAME paint path `setVisibility` uses — draws IDENTICAL
  fog for a given input) and alpha cross-fades between them; falls back to the Task 6 snap when a
  next sample is unavailable or more than one sweep is concurrently in flight. No polygon morphing
  — cross-fades rasterized textures only.
- `src/client/render/src/lighting.ts` — `Lighting` class (M10e-3, GL-free, unit-tested):
  resolves gradation band→darkening alpha + tint color, applies `renderHint` (e.g. `"darkvision"`
  → gray-wash desaturation overlay), and interpolates day/night fades. Called by `PixiBackend`
  `setLighting` which renders per-cell darkening/tint sprites + a `BlurFilter` for soft band edges.
  Plan: `docs/superpowers/plans/2026-06-25-m10e-3-client-lighting-render.md`.
- `src/modules/stage/Stage.svelte` — mounts the render engine over a `ReadableDocuments` view.
- `src/modules/scene-tools/` — `controller.svelte.ts`, `hit-test.ts`, tools (place/select/move/
  draw/template/measure/ping/wall/region) dispatching intents. Wall tool writes a **three-flag**
  segment: `blocksSight` + `blocksMove` + `blocksLight`. Region tool (`makeRegionTool`) drags out
  a rect/circle/polygon `region` doc (`ToolController.regionShapeMode`/`regionBehavior`/
  `regionCost`/`regionSecret` reactive fields) via `buildRegionDoc` +, when `regionSecret`,
  `setRegionVisibility(doc, true)` (declares `/engine` `gm_only` at construction — M13-0 re-root,
  was `/system`; the create op never carries the geometry in the clear). Create-only, mirroring
  `makeWallTool`: no edit UI for an already-placed region's behavior/cost/visibility/`enabled` — a
  GM re-authors via delete+recreate, or the server's live `enabled` toggle (region_field already
  honors it) without a UI surface. `buildRegionDoc`/`setRegionVisibility`/`RegionEngine`/
  `RegionShape`/`RegionShapeKind`/`RegionBehavior` are exported from `@shadowcat/core`'s public
  `index.ts` (M13-0 renamed the client type from `RegionSystem` to `RegionEngine` — no back-compat
  alias; the
  scene-docs.ts source predates the export; any future scene-docs.ts addition needs its own
  `index.ts` export line — it is not automatic).
- `src/client/core/src/scene-docs.ts` — **vision/lighting/movement data model (M10e-1 client model;
  the M10e-2 server mask now consumes these shapes; no client lighting render yet — M10e-3)**:
  world-scoped config-docs `world-settings`/`light-gradation`/`vision-modes`
  (builders + deep-frozen defaults `DEFAULT_WORLD_SETTINGS`/`DEFAULT_GRADATION`/`SEED_VISION_MODES`;
  builders `structuredClone` the frozen default), per-scene `SceneSystem.vision?`/`lighting?`
  overrides + `grid.distance?` + `bounds?` (M10f-0), the scene-parented `light` doc_type
  (`LightSystem` + `buildLightDoc`), and the fail-closed resolvers `resolveSceneSettings`/
  `resolveGradation`/`resolveVisionModes`. **`bounds` (M10f-0):** `SceneDimensions {width,
  height}` (grid units) — the M10f navmesh's future triangulation boundary; per-scene ONLY, no
  world-settings layer; `resolveBounds` (private helper called from `resolveSceneSettings`) falls
  back to the deep-frozen `DEFAULT_SCENE_BOUNDS = {width:100,height:100}` on absent OR malformed
  (non-finite/≤0-on-either-axis) input — never a degenerate rectangle, never throws. Authored by
  `src/modules/game-settings/` (see `shadowcat-codebase-client-shell`).
- **Multi-scene viewing / GM local roam (M12d).** `resolveViewedScene(store, {gmViewedScene?})`
  (`scene-docs.ts`) is the single client-side answer to "which scene does THIS client render/
  subscribe to". Resolution order: a resolvable `gmViewedScene` (GM-only local override) → a
  resolvable `world-settings.system.activeScene` (`WorldSettingsSystem.activeScene: string |
  null`, new field, deliberately EXCLUDED from `resolveSceneSettings`'s existing
  structural-completeness triple so a pre-M12d world-settings doc missing this key stays
  "complete") → the first scene (legacy single-scene fallback) → `null` only when no scene exists
  at all. Fail-closed by construction: an id naming a scene that no longer exists is treated as
  unresolvable and skipped to the next tier, never rendering nothing while scenes exist and never
  leaking a stale scene's channel. `WorldSession.viewedSceneId` (`worldSession.svelte.ts`) is the
  live getter (`resolveViewedScene(this.#optimistic, {gmViewedScene: role==="gm" ?
  this.#gmViewedScene : null})`); `setGmViewedScene(id)` sets the GM-only `#gmViewedScene` $state
  (warns + no-ops for a non-GM). `sendPing`, `onMoveStream`, and the `scene_ping` handler all
  resolve through `viewedSceneId` (not a fixed `query("scene")[0]`) and drop a frame whose `scene`
  doesn't match — closing a cross-scene leak (see Gotchas). `src/client/render/src/scene-scope.ts`
  — `sceneScopedDocs(store, docType, viewedSceneId)` filters a doc-type query to `d.parent_id ===
  viewedSceneId()`, or returns the unfiltered list when `viewedSceneId()` is `null` (the
  degenerate pre-scene case). `RenderEngine` (`engine.ts`) takes `RenderEngineOpts.viewedSceneId?:
  () => string | null` and exposes a private `viewedScene` resolver (falls back to
  `store.query("scene")[0]?.id` when unset — legacy/test callers unaffected); every render-layer
  view (`token-view.ts`, `wall-view.ts`, `drawing-view.ts`, `template-view.ts`, `region-view.ts`)
  and `reconciler.ts` take a trailing `viewedSceneId: () => string | null = () => null`
  constructor param and filter through `sceneScopedDocs`. `RenderEngine.reapplyViewedScene()` is
  the client-local-switch seam: a scene switch (`activeScene` flip or GM roam) carries no new
  server frame, so it re-runs every view's `reconcile()` (their `parent_id` filter target changed)
  and re-filters the last cached vision payload against the newly-current scene. `Stage.svelte`
  passes `viewedSceneId: () => ctx.viewedSceneId` into the engine and a `$effect` watches
  `ctx.viewedSceneId`, calling `engine.reapplyViewedScene()` exactly once per change.
  `scene-tools/controller.svelte.ts`'s private `activeScene(ctx)` helper resolves through
  `ToolContext.viewedSceneId?.()` before falling back to the first scene — every doc-creating tool
  (place/wall/region/drawing/template) stamps `parent_id` onto the viewed scene, not always the
  first one.

## Hard invariants

- **The canvas renders the OPTIMISTIC view** (`AppContext.documents` / `OptimisticClient`), NOT
  the authoritative `store` — the store is the rollback base; `appliedSeq` is identical so the
  derived watermark holds [[render-from-optimistic-view]].
- **Fog is the secrecy gate — fail closed.** A client-side visibility gate that is the SOLE thing
  hiding already-delivered data must hide-everything on a missing/garbled signal; container-local
  coords reused across containers must be tagged + filtered to the active container
  [[fog-is-the-secrecy-gate-fail-closed]].
- **A value cached across a client-local scene switch must never be pre-filtered against the
  scene that was active when it was cached — recompute the scene filter at the point of
  application, against whatever scene is current THEN (M12d).** This generalizes/complements the
  fog-fail-closed invariant above to a NEW failure axis: multi-scene viewing means the "active
  scene" a cached value should be filtered against can itself change before the cache is
  consumed. `RenderEngine.pendingDerived` (`engine.ts`) is the concrete instance: a `vision` frame
  arriving before `store.appliedSeq` catches up is held behind the watermark. The pre-M12d shape
  cached an ALREADY-FILTERED `VisibilityInput` (the result of `toVisibility`/`toLighting` run at
  frame-ARRIVAL time); once multi-scene viewing made a client-local scene switch
  (`reapplyViewedScene`) possible while a frame sat pending, that stale pre-filtered snapshot could
  be silently flushed and painted on top of the SWITCHED-TO scene once the watermark caught up —
  a fog hole computed for scene A rendered on scene B. Two independent buddy-check reviewers traced
  this to the same root cause. The fix: `pendingDerived` now caches the RAW `{payload, seq}` only;
  `flushPendingDerived()` re-runs `toVisibility(p.payload)`/`toLighting` at FLUSH time, which reads
  `viewedScene()` internally and therefore always filters against the scene that is current AT
  FLUSH, not at arrival. Any future engine cache that spans a client-local scene switch (not just
  vision/fog) must follow this same raw-payload-cache/filter-at-consumption shape — filtering
  eagerly and caching the filtered result is the bug pattern to avoid.
- **RESOLVED (`docs/CLOSED_BUGS.md`): `flushPendingDerived` no longer regresses `lastAppliedSeq` to
  a stale `pendingDerived` entry superseded by an immediately-applied newer frame.** `onSceneFrame`'s
  IMMEDIATE-apply branch (taken when a frame's `computedAtSeq` is not ahead of `store.appliedSeq` at
  arrival) never touched `pendingDerived` — a still-set OLDER deferred entry (e.g. seq 5) could
  survive past a newer frame's (seq 7) immediate apply advancing `lastAppliedSeq` to 7, then get
  wrongly re-applied by a LATER `flushPendingDerived` call (any subsequent store commit), regressing
  the mask back to seq 5. Fix: `flushPendingDerived` now applies a pending entry only when
  `p.seq > this.lastAppliedSeq` at flush time (checked AFTER the pre-existing `store.appliedSeq >=
  p.seq` watermark check, BEFORE the `toVisibility` re-filter) — otherwise discards it; the pending
  slot is unconditionally cleared as soon as the watermark condition is met, applied or not. This is
  a distinct guard from the M12d scene-refilter-at-flush fix directly above (that fix is about WHAT
  scene a cached payload filters against; this one is about WHETHER a superseded payload should
  apply at all) — both guards live in the same function and must both be preserved by any future
  edit to `flushPendingDerived`.
- **Vision is server-authoritative, no client prediction** (ARCHITECTURE §2 invariant 3); movement that
  crosses a `blocksMove` wall is rejected server-side before the write — validate the **post-image**
  position, not just the pre-move one [[m9-progress]].
- **Movement restriction is server-authoritative at the same gate (M10e-4).** In `Room::publish`'s
  non-GM block, AFTER the M9a `blocks_move` wall check, a move is rejected (`DataError::Forbidden`,
  before `apply_intent` — no seq consumed; client rolls back) unless the **entire** move's traversed
  cells lie in the user's mask: `Visible` ⇒ `visible_cells`; `Revealed` ⇒ `visible_cells ∪
  get_explored` (explored is center-sampled by construction — the union only ever ENLARGES, so the
  asymmetry is fail-safe); `Unrestricted` ⇒ walls only. GM exempt. **The gate mask is the SAME mask as
  egress** (`visible_cells` strict ≡ `player_lit_mask`) — never fork the per-cell decision (spec §13).
  **Hex-correct (Task 14d):** the traversed-cell set comes from `scene.resolve_grid_shape(scene_id,
  cell).line_traversal(a0, a1, cell)` — a SUPERCOVER on both grid kinds (square delegates to
  `movement::supercover_cells`; hex uses a ψ-crossing supercover, see the gotcha below) — the SAME
  primitive `move_exec::execute_move` gates against, so a
  select/move-tool drag (which writes `/engine/x,y` directly via this `publish` gate, not
  `moveRequest`) and an executed move agree on every cell on BOTH grid kinds. A prior version called
  the square-only `supercover_cells` free function directly here, testing square-indexed cells
  against `visible_cells_cached`'s hex-aware mask — two incompatible coordinate systems on a hex
  scene. Fail-closed on empty mask / `line_traversal`→None / `get_explored` Err. `get_explored` is on
  the `Repository` trait; the per-`(user,scene)` mask + explored blob are memoized within one
  publish, and the `get_explored().await` runs only AFTER the `scene.read()` guard drops (no lock
  across await). **By design: a dark scene under `Visible` freezes non-GM movement** — an empty lit
  mask rejects every move; a player who cannot see a cell must not move into it. The GM enables
  movement by lighting the scene or choosing `Revealed`/`Unrestricted`. Do NOT "fix" the freeze by
  softening the defaults — it is the correct fail-closed outcome.
- **Bound recursive walks over self-FK (parent_id) tables with a visited-set** [[m8a-execution-state]].
- **Scene-settings resolvers are fail-closed and inheritance-layered**: `resolveSceneSettings`
  resolves built-in default < `world-settings` doc < per-scene override, never throws (structural
  guard tolerates a partial `world-settings` wire doc), and a per-scene override of `null` means
  **inherit** (resolver `??` chains treat null and undefined identically). The deep-frozen
  `DEFAULT_*`/`SEED_*` constants are immutable-by-design; builders `structuredClone` them so no
  frozen/shared reference reaches a doc.
- **The server lit mask is the lighting-aware secrecy gate (M10e-2)**: `player_lit_mask(user)` =
  `LOS ∩ (lit ∨ darkvision)`, union over the user's vision sources (owned tokens ∪ observer-tier
  tokens when `observerVision`), emitted as per-recipient `lit` cells. Wire format (M10e-3 update):
  5-int `[i,j,band,tint,hint_idx]` (was 4-int `(i,j,band,tint)`) + a top-level `renderHints:[String]`
  table (index into the hint name, e.g. `"darkvision"`); `VisionMode` carries `render_hint`;
  `player_lit_mask` resolves a per-cell hint via the highest-floor admitting vision mode (`None` wins
  ties). Fail-closed (no source / dark scene ⇒ empty; cell scans bounded by
  `explored::MAX_CELLS_PER_POLYGON` with a `saturating_mul` span guard). Egress is ADDITIVE —
  `polygons` + the post-lock `explored` are unchanged, GM stays `mode:"all"`. **Environment light
  is now edge-projected and `blocksLight`-occludable SERVER-SIDE (`lighting::env_light_polys`,
  `mod.rs`'s `lighting_inputs_from`) — this is a genuine secrecy input, not cosmetic.** A cell's
  illumination (`lighting::cell_illumination`) composes the boundary-projected environment ambient
  with placed lights and feeds `point_qualifies`/`cell_visible`, which gates both `player_lit_mask`
  and the M10e-4 movement gate (`visible_cells`/`visible_cells_cached`) — so occluding the
  environment ambient behind a `blocksLight` wall genuinely narrows what a non-GM can see and move
  into, the same as a placed light's occlusion. `env_light_polys` samples the scene-bounds
  perimeter (`MAX_ENV_LIGHT_SAMPLES=256`, clamped `[4, 256]`) and reuses the SAME
  `vision::visibility_polygon` primitive + `light_walls` set placed lights already use — no forked
  occlusion computation. **Fail-closed/strictly-narrowing by construction:** the occluded
  environment base is `≤` the pre-occlusion flat-floor level everywhere (an empty `env_polys` set
  or a cell outside every boundary polygon contributes 0, never negative), so the composed
  illumination — and therefore the derived visibility mask — can only SHRINK relative to the prior
  flat-ambient behavior, never grow; this monotonicity is what makes the projection safe to ship as
  a secrecy input even independent of `env_light_polys`'s own occlusion-computation correctness.
  **The CLIENT-side render (`lighting.ts`) is UNCHANGED and remains purely COSMETIC** — it resolves
  band→darkening alpha + tint + `renderHint` desaturation for display only; fog stays the sole
  *rendered* secrecy gate on the client, and the client performs no occlusion computation of its
  own. Do not conflate the two: server-side environment occlusion is a load-bearing secrecy input,
  client-side lighting is display-only.

- **The pathfinder route is footprint-STRICTER than the center-based authoritative gate on WALLS,
  but its MASK predicate is now a superset of the gate's (M3, spec §3 of the M3 design doc).**
  `cell_enterable`'s wall check (footprint-disc clearance) is stricter than the M9/M10e-4
  authoritative gate's center-based wall check (parent spec §14) — a wide token can be dragged
  (gate allows the center path) along a corridor the router refuses (footprint doesn't fit); this
  wall asymmetry is intentional and safe (over-restrictive, never under). The MASK check requires
  `footprint_cells(to,...) ∪ supercover_cells(from,to,cell)` — the same `supercover_cells` primitive
  `move_exec.rs`/`publish` use per step — so the router's mask predicate is provably `≥` the gate's;
  **route ⊆ gate-allowed holds for every footprint size**, including the sub-0.5-cell diagonal case
  where the pre-M3 footprint-disc-only check let the router approve a step the gate rejected
  (buddy-check P1). Never make the pathfinder mask test weaker than `footprint_cells ∪
  supercover_cells` — that union IS the invariant, not merely a suggestion.
- **The `route ⊆ gate-allowed` invariant is engine-agnostic, not grid-specific (M10f-1).**
  `SceneEcs::pathfind` builds the per-`(user,scene)` visibility mask exactly once and passes the
  SAME reference into both the grid (`pathfinding::find`) and continuous (`navmesh::
  clip_to_visible_mask`) branches — never a forked mask computation. `clip_to_visible_mask` applies
  the identical `grid.footprint_cells ∪ grid.line_traversal` predicate `cell_enterable` uses, so a
  continuous-scene route preview is fog-safe by the same mechanism the grid router already proved.
  Any future third routing engine MUST reuse this same mask-passing shape, not recompute visibility.
  **Passing the same MASK is necessary but not sufficient — the same GRID SHAPE must travel with it
  (Task 14e-7, `[sec]`).** This invariant was written assuming a square grid on the continuous
  engine and was false on hex: `clip_to_visible_mask`, `los_smooth::chord_ok` and
  `truncate_at_arrest` indexed route samples with square `floor(p/cell)` while the `mask` and
  `RegionField` handed to them were hex-axial. All three now take `&dyn GridShape`, and the caller
  MUST pass the same `resolve_grid_shape`-derived shape those sets were built from — in the weighted
  branch that means `&*grid_shape`, NOT the Euclidean-ruled `euclid_shape` in scope (the diagonal
  rule feeds step cost and the heuristic, never cell identity: `rule` is a `SquareGrid`-only field
  read solely by `neighbors_with_cost`/`heuristic`). Shape identity IS the invariant; a shared mask
  indexed in two coordinate systems is not a shared mask.
- **M1 executor per-cell parity (spec §13):** `execute_move` uses the SAME `blocks_move` +
  `GridShape::line_traversal` (via `ecs.resolve_grid_shape`; a supercover on both kinds — square
  cell-walk, hex ψ-crossing) + `visible` membership as the M10e-4
  `publish` move gate — per-cell decision parity, NO fork, on BOTH grid kinds (Task 14d closed a
  square-only gap in `publish`'s call — see the movement-restriction bullet above). A divergence
  between the executor and the gate equals a movement-into-fog leak.
  **Admissibility parity is a SECOND, DISTINCT axis of the same never-fork rule (Task 14e-9):** both
  gates also share `move_exec::MAX_GATE_WALK_COORD` (1e9) as a coordinate-MAGNITUDE bound, checked
  before any traversal call and in every restriction mode (including `Unrestricted`, which
  short-circuits later). Per-cell agreement is not enough if the two gates disagree about which
  inputs are admissible at all. An anti-drift test exercises BOTH gates at the exact bound and at
  bound+1.0 through the shared symbol, so a value change or a `>`/`>=` flip on either side fails.
  **(M10f-2 revision)** The executor is no longer stricter on authored path shape — the pre-M10f-2
  king-step-adjacency requirement (reject any >1-cell authored jump) is REMOVED; `gate_walk`
  subdivides a >1-cell jump into dense ≤1-cell samples and gates each one, so a >1-cell authored
  jump is now admitted exactly when every crossed cell is wall-clear/visible (equivalent to the
  client having sent the explicit intermediate waypoints, which was always legal — no new
  capability). The `blocks_move`/`line_traversal`/`visible` per-cell decision parity itself is
  UNCHANGED and remains the load-bearing invariant. For `Revealed`, the
  caller MUST pass `visible_cells ∪ explored` as the `visible` argument (not raw `visible_cells`
  alone) — same union `publish` uses. Do NOT re-grant GM wall-bypass in `execute_move`: GMs are
  folded to `Unrestricted` (mask-skip) but `blocks_move` is still enforced for GMs. This
  intentionally diverges from `publish`'s legacy GM wall-bypass (to be retired).
- **M2 streamed vision is strictly leak-free — no fork of the secrecy decision, fail closed
  (`fog-is-the-secrecy-gate-fail-closed`).** The mover's swept vision trajectory raycasts the SAME
  `sight_walls` (full set, incl. `gm_only`) as `player_vision_polygons`; the observer egress clip
  filters against the recipient's OWN authoritative vision (never the mover's) — a `gm_only`-walled
  area is never streamed to a non-owner because the observer's own vision already excludes it. No
  render-path leak: the full trajectory is broadcast in-process only; `egress_loop`'s dedicated
  branch strips it per recipient before the sink write, same discipline as `Event`/`vision` egress.
  A wholly-occluded move is suppressed (zero frames), never sent as an empty-`samples` frame — an
  observer must not learn a move even happened if they can't see any of it. `mover_vision` is
  disclosed to the mover only (nulled for every other recipient, incl. a full-vision GM observer who
  has no fog to sweep anyway). **Design doc scope note:** "strictly leak-free" covers the IN-FLIGHT
  path only; RESTING token positions still ride the pre-existing position `Event` + client-side fog
  model (delivered to all scene readers, fogged client-side per `fog-is-the-secrecy-gate-fail-closed`)
  — M2 does not change that. **Known v1 limitation (by design, not a bug):** each move's
  per-recipient clip is computed once at ITS execute time against the recipient's then-current
  vision; two tokens moving simultaneously do not reveal each other mid-walk if a watcher's vision
  opens after the clip — reconciles at the stop + the next `vision` rebroadcast. Live
  cross-animation concurrency is deferred (`docs/TODO.md` — needs a per-move server loop). Client
  computes NO vision in any of this (ARCHITECTURE §2 invariant 3/4) — it renders only the streamed,
  already-clipped polygons. Design doc:
  `docs/superpowers/specs/2026-06-25-m2-streamed-continuous-vision-design.md`.
- **Region secrecy is a two-value contract on `region_field`, never a third mode (M10g).**
  `region_field(scene, None)` = authoritative (GM + `move_exec`); `region_field(scene, Some(user))`
  = per-requester (the router only). Callers must never pass `Some(gm_user)`. By construction the
  router's field is a SUBSET of the authoritative field (spec §6 parity) — a secret region can
  narrow a player's route/preview but can never appear to them where it wouldn't to the GM, and it
  always still applies at `move_exec` regardless of what the router showed. Reuses the EXACT same
  `resolve_access`/`property_overrides["/engine"]` mechanism as ordinary document egress (M13-0
  re-root; was `"/system"`) — no new secrecy machinery was introduced for regions (spec §3).
  **Fixture-construction precision (test/brief authoring convention):** the correct way to mark a
  region `gm_only` in a test fixture is
  `doc.permissions.property_overrides.insert("/engine".into(), Visibility::GmOnly)` — matching
  `region_field`'s actual read (`doc.permissions.property_overrides.get("/engine")`, default
  `Visibility::All`, `mod.rs`). Setting `permissions.default = Access::None` instead does NOT gate
  `region_field`'s per-requester filter at all (that field only reads the `/engine`
  `property_overrides` entry) — a brief/test author who reaches for `permissions.default` here will
  write a region that still weights a non-GM's route (M10f-4 Task 4 brief slip, caught before
  merge).
- **The continuous-engine dispatch predicate MUST read the PER-REQUESTER region field, never the
  authoritative one (M10f-4).** `has_terrain_or_impassable()` is evaluated against `region_field(
  scene, Some(user))` for a non-GM — this is the single mechanism preventing a secret
  terrain/impassable region from indirectly leaking its own existence via route-shape or reported
  cost even though its geometry is never disclosed. A future refactor that fed the authoritative
  field into ONLY the dispatch predicate, while still correctly routing/costing off the
  per-requester field, would silently reopen this leak (dispatching to the weighted path at all is
  itself a signal a secret region exists). Caught during Task 4's review — treat as load-bearing,
  not incidental.
- **Polyanya does not weight — the M10g cell `region_field` is the universal weighting overlay for
  BOTH engines (M10f-4).** Polyanya 0.16.1's only cost-affecting knob is the
  `detailed-layers`-gated `Layer.scale` (a per-layer coordinate transform, `instance.rs`) — off in
  this build's `default-features = false` config and semantically wrong as a per-unit cost
  multiplier even if enabled (crate-source-verified, not README-derived). A continuous route that
  needs weighting is therefore computed by the SAME `pathfinding::find` the grid engine uses
  (forced `DiagonalRule::Euclidean`), never by a polyanya cost-layer/`blocked_layers` mechanism —
  those polyanya features remain available but are deliberately UNUSED for regions in this
  codebase. Do not "improve" continuous weighting by reaching for polyanya's own layer API; the
  cell field is the one and only weighting authority for every routing engine this project has.
- **A whole-region-scalar disclosed on `MoveStream` must default to trusted-recipient-only, not
  broadcast-by-default.** `PathResult.arrested: bool` is always disclosed (no secrecy concern — it
  only tells the requester their OWN already-visible route is truncating). `MoveStream.cost:
  Option<f64>` is `Some` for the mover/GM (trusted, full information) and `None` for a clipped
  observer, mirroring `mover_vision`'s pre-existing null-for-observers pattern — because the
  authoritative cost may reflect `gm_only` secret-region terrain the observer's own vision would
  never reveal. This was a Critical finding caught and fixed during the M10g Task 9 review (an
  earlier draft broadcast `cost` unconditionally). **Load-bearing invariant, not a footnote:** any
  FUTURE whole-move scalar added to `MoveStream` must default to the same trusted-only disclosure
  pattern unless explicitly proven safe to broadcast to every recipient.
- **`RegionView` (`region-view.ts`) mirrors `WallView` exactly** — a dumb per-frame reconciler with
  NO client-side secrecy logic. The `"regions"` render layer sits between `"tiles"` and
  `"drawings"` in `layers.ts`'s `CORE_LAYERS`. Only regions the viewer is permitted to see ever
  reach `store` in the first place: `setRegionVisibility(doc, true)` sets `permissions.default =
  "none"` (NOT just a `/engine` override — M13-0 re-root; was `/system`), so `filter_command` drops a secret region's whole
  `Create` op for non-owner/non-GM recipients — the doc never arrives, not even redacted — while
  `region_field`'s per-requester view independently keeps a secret region out of a non-GM's
  pathfinder/budget field. There is no client-side hide check to get wrong, by design.

## Gotchas

- **Scene auto-creates on GM entry** (scene system schema `{grid, background}`); Stage reads the
  grid [[scene-lifecycle-gap]].
- **Clear tool overlays/previews on a mid-gesture tool swap** (draw preview, measure overlay) or
  stale geometry persists.
- **`resolved_diagonal_rule` is world-only** — there is intentionally no per-scene `diagonalRule`
  override in the pathfinder; the same rule applies across all scenes in a world. Matches the client
  `resolveSceneSettings` precedence (the setting lives in `world-settings`, not per-scene).
- **`region_arrests`/impassable checking is footprint-gated in the router (`cell_enterable`'s mask
  check, via `footprint_cells ∪ supercover_cells`) but CENTER-CELL-ONLY in `move_exec` — a
  deliberate asymmetry (route stricter, execution looser), not a bug (M10g).** `route ⊆
  gate-allowed` still holds because the router's predicate is already a documented superset of the
  executor's. Do not "fix" `move_exec` to match the router's footprint check without re-deriving
  the parity argument — the asymmetry is load-bearing, matching the pre-existing wall-check
  asymmetry the pathfinder invariant above already documents.
- **`find()`'s arrest truncation recomputes cost by REPLAYING `step_cost`, never by trusting
  `astar_leg`'s per-leg running total for the truncated prefix.** Parity threading is purely
  sequential/order-dependent (not leg-boundary-dependent), so a naive "sum the per-leg totals up to
  the truncation point" would be wrong whenever the truncation falls mid-leg; the cost-replay
  technique (walk the surviving cell sequence from parity 0, re-run `step_cost` per pair) is the
  only correct way to get an accurate truncated cost (M10g).
- **A whole-`/system` `GmOnly` override must NULL the field, not remove the key, in
  `filter_properties` (M10g discovery).** `Document::system` is a required serde field — dropping
  the `"system"` key from the redacted JSON before re-deserializing into `Document` panics.
  `filter_properties` special-cases the exact pointer `"/system"` (as opposed to a nested pointer
  like `"/system/name"`, which keeps the normal key-strip) to null the value instead. This branch
  was previously a latent, unexercised code path — no doc type before M10g declared a whole-`/system`
  visibility override (only nested-property overrides existed); secret regions were the first doc
  type to exercise it, and the panic-on-strip bug was caught before it shipped. Any future doc type
  that wants whole-body secrecy (vs. per-field) must go through this same branch, not a new one.
  **M13-0 generalized this exact same null-not-strip branch to `/engine` and `/name` too**
  (`filter_properties` now special-cases all three top-level pointers identically) — a secret
  region's declared override moved from `/system` to `/engine` as part of that same re-root (see
  `shadowcat-codebase-documents-permissions` for the generalized rule).
- **A fixed-count cube lerp is a THIN LINE, not a supercover (Task 14e-8, `[sec]`).** `HexGrid::
  line_traversal` originally sampled `n+1` points with `n = max cube-axis delta` (the standard
  Red Blob hex line-draw). Its sample spacing is one full hex PITCH — a hex's minimum width — so
  corner slivers fall between samples: it omitted a geometrically crossed hex on ~55% of random
  segments, and when `n` rounded to 0 it dropped the destination's own hex, breaking its own
  "both endpoint cells always included" contract. Because this is the hex movement gate's primitive
  in BOTH `Room::publish` and `move_exec`, every omitted hex was one a non-GM could move through
  unchecked against the visibility mask. It is now a **ψ-crossing supercover**: `cell_of` is
  nearest-center, so a hex is its center's Voronoi cell and every hex boundary lies on an integer
  level set of ψ₁=x−y, ψ₂=z−y, ψ₃=x−z (fractional cube coords) — enumerate every integer ψ crossing,
  sample each interval's midpoint, plus a perpendicular epsilon probe either side of each crossing
  and both endpoints (edge-riding / vertex / endpoint-on-boundary). Over-inclusion is the only
  failure mode and is safe HERE because this set feeds gates only, never a reveal write (the sole
  explored-set writer, `conn.rs`'s `enrich_vision_explored`, is fed by vision polygons via
  `mark_polygons`) — re-check that property before reusing it anywhere else.
- **"The square failure mode can't happen here" is not a safety argument (Task 14e-8; design
  decision H6/H5).** Hex genuinely has no analog of the square diagonal-corner-tie bug — 6 uniform
  neighbors, no orthogonal/diagonal split — and that true statement is what let the thin-line
  traversal ship unexamined: hex had its OWN omission class. Establishing that a known failure mode
  is absent says nothing about which failure modes the new geometry has of its own. Related and
  identical in shape: `navmesh.rs` was excused from the Task 14e hex audit as "the continuous-model
  router, orthogonal to grid kind" — but grid kind and movement model are INDEPENDENT axes, so they
  COMBINE (`hex` + `continuous` is a live scene) rather than exclude, and it was square-on-hex at
  three sites (Task 14e-7). Independence is a reason to CHECK a site, never to skip it.
- **RESOLVED (`docs/CLOSED_BUGS.md`): `supercover_cells`'s corner-crossing branch no longer drifts
  past the target on a diagonal king-step whose leg endpoints both sit exactly on 4-way grid-line
  intersections.** (M10f-2 discovered this via a Task 6 fixture-derivation error; a later fix
  closed it.) Root cause: the branch stepped BOTH axes on every `tMax` tie without checking
  whether an axis had already reached its target cell — a forced single-axis step early in the
  traversal (from an endpoint sitting exactly on a grid line) could put `t_max_i`/`t_max_j` into
  permanent lockstep, so every later tie re-stepped the already-arrived axis too, drifting past
  `(ei,ej)` until `MAX_MOVE_CELLS` aborted with `None`. Fix: the diagonal corner-step is now gated
  on a per-axis remaining-step budget (`remaining_i`/`remaining_j`) — it only fires when BOTH axes
  still owe a grid-line crossing; once either budget hits zero, only the other axis steps
  regardless of any tie. Convergence is now a property of the (bounded) step budget, not
  floating-point tie-breaking; the existing safe-over-include behavior for genuine mid-path corner
  crossings (both flankers emitted) is unchanged and covered by dedicated regression tests in
  `movement.rs`. `execute_move`'s frozen-fixture "diagonal 3-step king path, full visible" case
  (`move_exec.rs`) is updated to the now-correct non-truncated outcome.
- **Cross-scene `MoveStream`/`ScenePing` leak class (M12d) — a NEW divergence axis, not a
  pre-existing gap.** Before M12d every client rendered the SAME scene (`activeScene`, in
  lockstep) — there was no per-client "which scene am I looking at" state for a broadcast
  fan-out egress path to diverge against, so this leak class could not previously exist.
  `gmViewedScene` (GM local roam, M12d) is what FIRST introduces per-client scene divergence: a
  room-wide `MoveStream`/`ScenePing` broadcast now reaches connections that may be viewing
  DIFFERENT scenes than the event targets. `WorldSession` closes it client-side by dropping any
  frame whose `scene` doesn't equal `this.viewedSceneId` (`onMoveStream`/the `scene_ping` handler,
  `worldSession.svelte.ts`) — a GM roaming scene B must not animate/ping-render scene A's event, and
  vice versa. **Any future per-client "which scene am I looking at/subscribed to" feature must
  re-audit EVERY broadcast fan-out egress path for this same divergence class, not just the render
  layer** — `MoveStream`/`ScenePing` were the two found and fixed in M12d; a new room-wide
  broadcast type added later (chat, pings, future presence/cursor frames) inherits the same risk
  the instant any client can view something other than the room's single shared `activeScene`.

## Pointers

- Rationale: `docs/design/ARCHITECTURE.md` §2 (invariants 3, 5, 6 + the M9 geometry exception)
  + §7 (rendering provenance); `docs/PLAN.md` (M8/M9/M10e/M2/M10g milestones);
  `docs/superpowers/specs/2026-06-25-m2-streamed-continuous-vision-design.md` (streamed vision);
  `docs/superpowers/specs/2026-07-01-m10g-regions-design.md` (regions design spec);
  `docs/superpowers/plans/2026-07-02-m10g-regions.md` (regions implementation plan);
  `docs/superpowers/specs/2026-07-02-m10f-continuous-navmesh-movement-design.md` (M10f continuous/
  navmesh movement design — bounds is §4.1/§5.1, checkpoint M10f-0 is §12);
  `docs/superpowers/plans/2026-07-02-m10f-0-scene-bounds.md` (M10f-0 implementation plan);
  `docs/superpowers/specs/2026-07-02-m10f-1-movement-model-dispatch-polyanya-router-design.md`
  (M10f-1 checkpoint design — the polyanya/geo/glam crate facts, footprint-aware memoized-mesh
  decision, preview-only execution boundary); `docs/superpowers/plans/2026-07-02-m10f-1-movement-
  model-dispatch-polyanya-router.md` (M10f-1 implementation plan + its buddy-check history, incl.
  the plan-level cache-quantization finding);
  `docs/superpowers/specs/2026-07-02-m10f-2-unified-movement-executor-design.md` (M10f-2
  checkpoint design — the gate_walk subdivide-only/identity-on-grid decision, the engine-agnostic
  executor, the differential-oracle parity-proof-then-delete strategy);
  `docs/superpowers/plans/2026-07-02-m10f-2-unified-movement-executor.md` (M10f-2 implementation
  plan + its buddy-check history, incl. the gate_walk floating-point tolerance bugs and the
  Task-6 fixture-derivation error that surfaced the `supercover_cells` corner-drift gotcha above);
  `docs/superpowers/specs/2026-07-03-m10f-4-regions-on-navmesh-design.md` (M10f-4 checkpoint
  design — final M10f checkpoint; the polyanya-cannot-weight crate-source verification §2.2, the
  weighted-grid-reuse-then-smooth decision §4.1, the LOS-smoothing cost-guard §5);
  `docs/superpowers/plans/2026-07-03-m10f-4-regions-on-navmesh.md` (M10f-4 implementation plan);
  `docs/superpowers/specs/2026-07-03-m10h-faces-animated-design.md` (M10h faces + animated token
  visuals — client-only, the Container-per-token migration + `tickTokenAnimations` seam).
- Relationships:
  `graphify query "scene ECS derived read-model vision fog stage pixi render tokens regions faces animated"`.
- History/decisions: [[m8-brainstorm]], [[m8d-2-scene-tools]], [[m9-progress]],
  [[server-authoritative-movement-rule]], [[m10-pathfinding-architecture]].
