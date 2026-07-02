---
name: shadowcat-codebase-scene-rendering
description: "Use when touching Shadowcat scenes, the scene ECS, rendering, the PixiJS canvas/stage, vision raycasting, fog of war, lighting, the server visibility/lit mask, movement restriction (the Room::publish move gate, supercover, visible_cells), the grid A* pathfinder (scene/pathfinding.rs, SceneEcs::pathfind, Pathfind/PathResult frames, diagonal rules), streamed continuous vision (MoveStream, scene/move_stream.rs, player_vision_polygons_at, the per-recipient egress clip, client fog-sweep/cross-fade playback), regions (weighted/impassable/arrest zones, region docs, region-view.ts render layer), or scene-tools (place/select/move/draw/template/measure/ping/wall/region). Covers src/server/src/scene, src/client/render, src/modules/{stage,scene-tools}. Invoke shadowcat-codebase-core first."
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
  (f64,f64)` (M10f-0, `width,height` in grid units) via a fail-closed structural parse
  (`s.pointer("/bounds/width")`/`/height"`, `serde_json::Value::pointer` + `unwrap_or`, matching
  the file's existing sibling-field idiom — no ts-rs struct, the scene `system` body stays
  client-owned/server-structural); non-finite or ≤0-on-either-axis falls back to
  `DEFAULT_SCENE_BOUNDS_UNITS = (100.0, 100.0)`, which MUST numerically match the client's
  `DEFAULT_SCENE_BOUNDS` in `scene-docs.ts` (dual-language default-parity invariant — verify both
  when either changes). Per-scene only, no world-settings layer. **Movement gate (M10e-4):**
  `visible_cells(user, scene, lenient)` is the move-gate mask — under strict (center) sampling it
  EQUALS `player_lit_mask`'s cells (spec §13) because both share `cell_visible` / `lighting_inputs` /
  `source_los_poly` / `point_qualifies`; `lenient` adds the 4 corners (a superset, never a
  zero-overlap cell). `resolve_scene` also yields `movement_restriction`
  (`MovementRestriction::{Visible,Revealed,Unrestricted}`, scene-overridable, fail-closed to `Visible`)
  + `partial_cell_leniency` (world-only).
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
  restriction, visible, cell) -> Result<MoveOutcome, MoveReject>` (M1 server-authoritative movement):
  walks the path step by step — (1) wall gate (`blocks_move`, all modes incl. GM), (2) vision-mask
  gate (`supercover_cells` + `visible` membership, skipped for `Unrestricted`), (3) region gate
  (M10g — see below). Returns `stop` + `render_path` (legal prefix) + `truncated` + `cost`.
  `MAX_MOVE_PATH=256` DoS guard. `MoveReject` variants: `NotAToken`, `EmptyPath`, `TooLong`,
  `Degenerate` (non-finite coords / bad start / non-adjacent king-step).
  **Region gate (M10g, step 3):** always reads `ecs.region_field(scene, None)` — the AUTHORITATIVE
  field, computed once before the walk loop begins (never per-step, never filtered) — so a
  `gm_only` secret region "springs" on execution even for a mover whose own route preview couldn't
  see it. Center-cell only (`to_cell(next)`; no footprint check) — a documented asymmetry against
  the router's footprint-aware `cell_enterable` (router stricter, executor looser), but `route ⊆
  gate-allowed` still holds because the router's mask predicate is already a superset (see the
  pathfinder invariant below). Impassable stops BEFORE entry into the cell (like a wall — `stop`
  lands on the prior cell); arrest stops AT entry (the cell is entered, then the walk halts — a
  final-step arrest still sets `truncated: true` even though `stop_index == path.len()-1`).
  `MoveOutcome.cost` accumulates `regions.terrain_multiplier(region_cell)` per step (1.0 outside any
  terrain region) — center-cell-only, terrain-only; it does NOT apply the diagonal-rule step-cost
  factor (`sc` — 1.0/2.0/√2/alternating) that `pathfinding.rs`'s router applies. **Known, logged
  inconsistency (`docs/TODO.md`):** the two `cost` values are numerically comparable only under
  Chebyshev (where the diagonal step cost is 1.0); under any other diagonal rule they diverge. This
  is a deliberate v1 scoping decision (M10g Task 7), not a bug — nothing currently consumes or
  compares the two costs together. Resolve before any per-turn movement-budget system consumes
  either `MoveOutcome.cost` or `MoveStream.cost`.
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
  write, per connection, in three branches: the mover gets `samples` + `mover_vision` unchanged;
  a GM gets the FULL `samples` unclipped (GMs bypass position secrecy) but `mover_vision` forced
  to `None` (a GM has no fog to sweep); every other (non-GM, non-mover) recipient gets `samples`
  clipped to those whose `pos` falls inside the recipient's OWN authoritative vision polygons
  (`point_in_poly`, recomputed off the current ECS read — never a stale cache; the ECS guard drops
  before any await) with `mover_vision` also forced to `None`; a wholly-invisible move (empty clip)
  is **not sent at all** (suppressed, not an empty-`samples` frame — asserted by a dedicated test).
  `send_filtered` intentionally panics if a
  `MoveStream` reaches it — the clip MUST happen in the dedicated `egress_loop` branch, never the
  generic per-recipient filter path. `MoveError` stays mover-only via `etx`, generic (no path/vision
  geometry disclosed).
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
  only when `user` can see the visibility tier declared on its `/system` (defaults to `All` when
  undeclared), via the SAME `resolve_access`/`property_overrides["/system"]` mechanism that already
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
- `src/client/render/src/` — engine-owned PixiJS layer: `backend.ts` + `pixi-backend.ts`
  (renderer host), `engine.ts`, `reconciler.ts` (doc→scene reconcile), `compositor.ts`,
  `layers.ts` (CORE_LAYERS z-order; index 7 = `lighting`, between `templates` (6) and `mask` (8)),
  `camera.ts`, `grid.ts`, `token-view.ts` + `token-animator.ts` (tween),
  `wall-view.ts`, `drawing-view.ts`, `template-view.ts`, `ping-view.ts`. Modules draw through the
  render-layer API; the canvas host is not replaceable.
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
  `setRegionVisibility(doc, true)` (declares `/system` `gm_only` at construction — the create op
  never carries the geometry in the clear). Create-only, mirroring `makeWallTool`: no edit UI for
  an already-placed region's behavior/cost/visibility/`enabled` — a GM re-authors via
  delete+recreate, or the server's live `enabled` toggle (region_field already honors it) without
  a UI surface. `buildRegionDoc`/`setRegionVisibility`/`RegionSystem`/`RegionShape`/
  `RegionShapeKind`/`RegionBehavior` are exported from `@shadowcat/core`'s public `index.ts` (the
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

## Hard invariants

- **The canvas renders the OPTIMISTIC view** (`AppContext.documents` / `OptimisticClient`), NOT
  the authoritative `store` — the store is the rollback base; `appliedSeq` is identical so the
  derived watermark holds [[render-from-optimistic-view]].
- **Fog is the secrecy gate — fail closed.** A client-side visibility gate that is the SOLE thing
  hiding already-delivered data must hide-everything on a missing/garbled signal; container-local
  coords reused across containers must be tagged + filtered to the active container
  [[fog-is-the-secrecy-gate-fail-closed]].
- **Vision is server-authoritative, no client prediction** (ARCHITECTURE §2 invariant 3); movement that
  crosses a `blocksMove` wall is rejected server-side before the write — validate the **post-image**
  position, not just the pre-move one [[m9-progress]].
- **Movement restriction is server-authoritative at the same gate (M10e-4).** In `Room::publish`'s
  non-GM block, AFTER the M9a `blocks_move` wall check, a move is rejected (`DataError::Forbidden`,
  before `apply_intent` — no seq consumed; client rolls back) unless the **entire** move's supercover
  cells lie in the user's mask: `Visible` ⇒ `visible_cells`; `Revealed` ⇒ `visible_cells ∪
  get_explored` (explored is center-sampled by construction — the union only ever ENLARGES, so the
  asymmetry is fail-safe); `Unrestricted` ⇒ walls only. GM exempt. **The gate mask is the SAME mask as
  egress** (`visible_cells` strict ≡ `player_lit_mask`) — never fork the per-cell decision (spec §13).
  Fail-closed on empty mask / `supercover_cells`→None / `get_explored` Err. `get_explored` is on the
  `Repository` trait; the per-`(user,scene)` mask + explored blob are memoized within one publish, and
  the `get_explored().await` runs only AFTER the `scene.read()` guard drops (no lock across await).
  **By design: a dark scene under `Visible` freezes non-GM movement** — an empty lit mask rejects
  every move; a player who cannot see a cell must not move into it. The GM enables movement by
  lighting the scene or choosing `Revealed`/`Unrestricted`. Do NOT "fix" the freeze by softening the
  defaults — it is the correct fail-closed outcome.
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
  `polygons` + the post-lock `explored` are unchanged, GM stays `mode:"all"`. **Client lighting
  render is COSMETIC — fog stays the secrecy gate**; the per-cell `hint_idx` refines the visual
  (darkening + tint + desaturate) but never widens visibility or the secrecy mask. **Constraint:**
  environment light is a flat ambient, NOT edge-projected/occludable. Originally blocked on scenes
  being dimensionless; **M10f-0 added `scene.system.bounds` so a boundary now exists**, but
  edge-projected light is deliberately still homed to M12 (design review 2026-07-02) — the bound
  primitive alone does not implement the projection. Placed-light `blocksLight` occlusion IS
  implemented (see `docs/TODO.md`).

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
- **M1 executor per-cell parity (spec §13):** `execute_move` uses the SAME `blocks_move` +
  `supercover_cells` + `visible` membership as the M10e-4 `publish` move gate — per-cell decision
  parity, NO fork. A divergence between the executor and the gate equals a movement-into-fog leak.
  The executor is additionally STRICTER on path shape (requires king-step adjacency per consecutive
  waypoint pair; the legacy `publish` whole-segment gate does not enforce this). For `Revealed`, the
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
  `resolve_access`/`property_overrides["/system"]` mechanism as ordinary document egress — no new
  secrecy machinery was introduced for regions (spec §3).
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
  "none"` (NOT just a `/system` override), so `filter_command` drops a secret region's whole
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

## Pointers

- Rationale: `docs/design/ARCHITECTURE.md` §2 (invariants 3, 5, 6 + the M9 geometry exception)
  + §7 (rendering provenance); `docs/PLAN.md` (M8/M9/M10e/M2/M10g milestones);
  `docs/superpowers/specs/2026-06-25-m2-streamed-continuous-vision-design.md` (streamed vision);
  `docs/superpowers/specs/2026-07-01-m10g-regions-design.md` (regions design spec);
  `docs/superpowers/plans/2026-07-02-m10g-regions.md` (regions implementation plan);
  `docs/superpowers/specs/2026-07-02-m10f-continuous-navmesh-movement-design.md` (M10f continuous/
  navmesh movement design — bounds is §4.1/§5.1, checkpoint M10f-0 is §12);
  `docs/superpowers/plans/2026-07-02-m10f-0-scene-bounds.md` (M10f-0 implementation plan).
- Relationships:
  `graphify query "scene ECS derived read-model vision fog stage pixi render tokens regions"`.
- History/decisions: [[m8-brainstorm]], [[m8d-2-scene-tools]], [[m9-progress]],
  [[server-authoritative-movement-rule]], [[m10-pathfinding-architecture]].
