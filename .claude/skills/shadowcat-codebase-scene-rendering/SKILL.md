---
name: shadowcat-codebase-scene-rendering
description: "Use when touching Shadowcat scenes, the scene ECS, rendering, the PixiJS canvas/stage, vision raycasting, fog of war, lighting, the server visibility/lit mask, movement restriction (the Room::publish move gate, supercover, visible_cells), the grid A* pathfinder (scene/pathfinding.rs, SceneEcs::pathfind, Pathfind/PathResult frames, diagonal rules), or scene-tools (place/select/move/draw/template/measure/ping). Covers src/server/src/scene, src/client/render, src/modules/{stage,scene-tools}. Invoke shadowcat-codebase-core first."
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
  precedence) plus `scene_lights`/`light_walls` accessors. **Movement gate (M10e-4):**
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
  gate (`supercover_cells` + `visible` membership, skipped for `Unrestricted`), (3) region-arrest
  hook (`region_arrests` — inert stub, always `false`, until M10g). Returns `stop` + `render_path`
  (legal prefix) + `truncated`. `MAX_MOVE_PATH=256` DoS guard. `MoveReject` variants: `NotAToken`,
  `EmptyPath`, `TooLong`, `Degenerate` (non-finite coords / bad start / non-adjacent king-step).
  `region_arrests` and `cost_field` are both inert stubs until M10g — do not implement region
  behavior there.
- `src/server/src/scene/mod.rs` — adds `SceneEcs::token_position(token) -> Option<(f64,f64)>` and
  `SceneEcs::resolved_animation_speed() -> f64` (`pub(crate)` seams; the latter sits alongside
  `resolved_diagonal_rule`, sources `world_settings.animation`, defaults to 6 cells/sec).
- `src/server/src/scene/explored.rs` — `ExploredSet` fog memory: `mark_polygons(polys, cell_size)`,
  `to_bytes`/`from_bytes` (persistence), cell-based.
- `src/server/src/scene/pathfinding.rs` — pure, headless grid A* (no I/O; clean-room):
  `DiagonalRule` (`chebyshev`|`manhattan`|`euclidean`|`alternating`) + `resolved_diagonal_rule`
  (world-only — no per-scene override; mirrors `resolveSceneSettings` precedence); `PathGrid` (wall-
  segment lookup built from `move_walls`); `cell_enterable(cell, grid, footprint_radius, user_mask)`
  — full geometric footprint-disc clearance: the token's bounding disc must clear ALL `blocksMove`
  segments (checked via `point_segment_distance`) **and** ALL footprint cells must be in the non-GM
  mask **and** the cell center must pass the mask test; `astar_leg` — king-move A*, 4 diagonal
  rules, 5-10-5 parity tracked in the `(cell, parity)` node and carried across waypoint legs (cost
  1,2,1,2…, never reset per leg), admissible+consistent heuristics per rule, stale-pop skip,
  `MAX_PATH_NODES`/`MAX_WAYPOINTS`/`MAX_FOOTPRINT_CELLS` fail-closed bounds; `find` — validates
  request, computes search window (AABB{start∪waypoints∪wall-endpoints}+8-cell margin), threads
  end-parity of each leg into the next, sums cost, returns ordered cell-center scene coords.
  `SceneEcs::pathfind` — reuses the SAME `visible_cells` mask as the M10e-4 movement gate (**§13
  invariant: never fork the per-cell visibility decision** — the route cannot thread the unknown nor
  leak hidden geometry); unions `explored` (`ExploredSet::iter`) for `revealed`; GM unconstrained
  (no mask); empty non-GM mask ⇒ `PathError::Unreachable` (fail-closed). New `move_walls(scene)`
  accessor returns the `blocksMove` segment list (mirrors the M9 `blocks_move` filter). Wire
  frames `Pathfind`/`PathResult`/`PathError` — one-shot to the requesting connection only (never
  broadcast); `get_explored` fetched off the scene read lock (no lock across await).
  Client: `ToolContext.pathfind?` seam + `SceneTool.onDeactivate?()` hook in scene-tools (clears
  route overlay on tool swap); ruler `Grid.distance()` gains the `alternating` (5-10-5) rule wired
  from `resolveSceneSettings(...).diagonalRule` into the Stage `GridSpec`.
- `src/client/render/src/` — engine-owned PixiJS layer: `backend.ts` + `pixi-backend.ts`
  (renderer host), `engine.ts`, `reconciler.ts` (doc→scene reconcile), `compositor.ts`,
  `layers.ts` (CORE_LAYERS z-order; index 7 = `lighting`, between `templates` (6) and `mask` (8)),
  `camera.ts`, `grid.ts`, `token-view.ts` + `token-animator.ts` (tween),
  `wall-view.ts`, `drawing-view.ts`, `template-view.ts`, `ping-view.ts`. Modules draw through the
  render-layer API; the canvas host is not replaceable.
- `src/client/render/src/lighting.ts` — `Lighting` class (M10e-3, GL-free, unit-tested):
  resolves gradation band→darkening alpha + tint color, applies `renderHint` (e.g. `"darkvision"`
  → gray-wash desaturation overlay), and interpolates day/night fades. Called by `PixiBackend`
  `setLighting` which renders per-cell darkening/tint sprites + a `BlurFilter` for soft band edges.
  Plan: `docs/superpowers/plans/2026-06-25-m10e-3-client-lighting-render.md`.
- `src/modules/stage/Stage.svelte` — mounts the render engine over a `ReadableDocuments` view.
- `src/modules/scene-tools/` — `controller.svelte.ts`, `hit-test.ts`, tools (place/select/move/
  draw/template/measure/ping) dispatching intents. Wall tool writes a **three-flag** segment:
  `blocksSight` + `blocksMove` + `blocksLight`.
- `src/client/core/src/scene-docs.ts` — **vision/lighting/movement data model (M10e-1 client model;
  the M10e-2 server mask now consumes these shapes; no client lighting render yet — M10e-3)**:
  world-scoped config-docs `world-settings`/`light-gradation`/`vision-modes`
  (builders + deep-frozen defaults `DEFAULT_WORLD_SETTINGS`/`DEFAULT_GRADATION`/`SEED_VISION_MODES`;
  builders `structuredClone` the frozen default), per-scene `SceneSystem.vision?`/`lighting?`
  overrides + `grid.distance?`, the scene-parented `light` doc_type (`LightSystem` +
  `buildLightDoc`), and the fail-closed resolvers `resolveSceneSettings`/`resolveGradation`/
  `resolveVisionModes`. Authored by `src/modules/game-settings/` (see
  `shadowcat-codebase-client-shell`).

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
  environment light is a flat ambient (NOT edge-projected/occludable) until scenes gain dimensions —
  placed-light `blocksLight` occlusion IS implemented (see `docs/TODO.md`).

- **The pathfinder route is footprint-STRICTER than the center-based authoritative gate, but shares
  the SAME mask (never weaker).** `cell_enterable` enforces full geometric footprint-disc clearance;
  the M9/M10e-4 authoritative gate stays center-based (parent spec §14). A wide token can be dragged
  (gate allows the center path) along a corridor the router refuses (footprint doesn't fit). This
  asymmetry is intentional: route ⊆ gate-allowed keeps the preview from suggesting a move the router
  would reject. The route never threads cells the gate would block (`visible_cells` mask is shared —
  spec §13). Never make the pathfinder mask test weaker than the gate mask.
- **M1 executor per-cell parity (spec §13):** `execute_move` uses the SAME `blocks_move` +
  `supercover_cells` + `visible` membership as the M10e-4 `publish` move gate — per-cell decision
  parity, NO fork. A divergence between the executor and the gate equals a movement-into-fog leak.
  The executor is additionally STRICTER on path shape (requires king-step adjacency per consecutive
  waypoint pair; the legacy `publish` whole-segment gate does not enforce this). For `Revealed`, the
  caller MUST pass `visible_cells ∪ explored` as the `visible` argument (not raw `visible_cells`
  alone) — same union `publish` uses. Do NOT re-grant GM wall-bypass in `execute_move`: GMs are
  folded to `Unrestricted` (mask-skip) but `blocks_move` is still enforced for GMs. This
  intentionally diverges from `publish`'s legacy GM wall-bypass (to be retired).

## Gotchas

- **Scene auto-creates on GM entry** (scene system schema `{grid, background}`); Stage reads the
  grid [[scene-lifecycle-gap]].
- **Clear tool overlays/previews on a mid-gesture tool swap** (draw preview, measure overlay) or
  stale geometry persists.
- **`resolved_diagonal_rule` is world-only** — there is intentionally no per-scene `diagonalRule`
  override in the pathfinder; the same rule applies across all scenes in a world. Matches the client
  `resolveSceneSettings` precedence (the setting lives in `world-settings`, not per-scene).

## Pointers

- Rationale: `docs/design/ARCHITECTURE.md` §2 (invariants 3, 5, 6 + the M9 geometry exception)
  + §7 (rendering provenance); `docs/PLAN.md` (M8/M9 milestones).
- Relationships:
  `graphify query "scene ECS derived read-model vision fog stage pixi render tokens"`.
- History/decisions: [[m8-brainstorm]], [[m8d-2-scene-tools]], [[m9-progress]].
