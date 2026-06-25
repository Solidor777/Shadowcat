# M10e — Scene Vision, Lighting & Movement: Design Spec

**Date:** 2026-06-24
**Status:** Design approved; ready for per-checkpoint plans.
**Supersedes/expands:** the single `M10e — Pathfinding (grid)` checkpoint in
`2026-06-24-m10-tokens-design.md` §10/§12. The grid A* pathfinder is now the *last* of six
sub-checkpoints; it is gated on a vision/lighting/movement foundation that did not previously exist.
`M10f` (continuous pathfinding) and `M10g` (weighted/impassable regions) resume unchanged after.

## 1. Goal

Redefine *what a player can see* from pure line-of-sight to a **lighting-aware grid visibility
mask**, then build server-authoritative **movement restriction**, **movement animation**, and the
**grid A\* pathfinder** on top of that one abstraction. The user-facing driver: a player must not be
able to accidentally drag their token into unseen map and reveal it; secondarily, a routed path must
never thread the unknown.

## 2. Constraints inherited (cited inline)

- **Secrecy gate, fail-closed** — the server ships a player only what is inside their mask; a
  missing/garbled visibility signal hides everything (`fog-is-the-secrecy-gate-fail-closed`). Walls,
  lights, and illumination outside the mask are never sent.
- **Server-authoritative geometry** — engine-owned, the ARCHITECTURE §6 exception already taken by
  M9 movement-blocking and vision. Pathfinding/lighting/restriction are all server-side because the
  geometry is GM-secret.
- **Config as world-scoped documents** (M10 decision #10) — registries/settings are world-scoped
  config docs (faction/condition pattern).
- **Decide on merits, not Foundry** (`decide-on-merits-not-foundry`).
- **Cross-platform** — pure-Rust server, no OS-specific paths; client renders on desktop + mobile.

## 3. The five scene axes

A scene's visibility is governed by independent knobs (each world-default + per-scene override):

1. **LOS restriction** on/off — do `blocksSight` walls limit sight to a vision polygon, or is the
   whole scene geometrically visible?
2. **Lighting enabled** on/off (master) — is darkness a concept at all? Off ⇒ everything is lit, no
   tint. (Distinct from global illumination, which keeps the lighting system *active*.)
3. **Light mode** — `globalIllumination` (whole scene lit, but tint/intensity still apply) vs
   `environmentLight` (edge-projected light occluded by `blocksLight`, plus placed lights).
4. **Fog of war** on/off — is per-player explored memory tracked and dimmed?
5. **Vision modes** (per actor/token) — darkvision et al. lower the illumination a creature needs to
   perceive a cell within range.

## 4. Core architecture — one per-`(user, scene)` grid visibility mask

Everything downstream consumes one authoritative server-computed thing:

```
mask(user, scene) =
    ( LOS-restriction ON ? vision_cells(user)              : all_cells )
  ∩ ( lighting        ON ? (lit_cells ∨ darkvision_cells(user)) : all_cells )

vision_sources(user) = owned tokens ∪ (observerVision ON ? observer-tier tokens : ∅)
vision_cells(user)   = rasterize( ⋃ visibility_polygon(t, blocksSight) for t in vision_sources(user) )
lit_cells            = global-illum ? all_cells
                        : rasterize( ⋃ light_polygon(L, blocksLight) for L in lights, + environment edge light )
                          thresholded at the gradation floor
darkvision_cells(u)  = cells within a vision-source token's darkvision range (floor lowered)
```

A cell is **visible to `user`** iff ∃ vision-source `t`:
`cell ∈ vision_polygon(t)` **AND** `illumination(cell) ≥ floor(t, cell)`, where `floor(t,cell)` is
the lowest floor among `t`'s vision modes whose range covers the cell (darkvision-in-range ⇒ floor =
`dark`; otherwise normal ⇒ `dim`). Union over sources. **GM mask = `all_cells`** at full
illumination.

This grid mask is the **secrecy gate**: the server ships a player only entities/areas inside it,
accumulates explored fog from it, gates movement against it, and bounds pathfinding to it.

### 4.1 Computation approach — hybrid (chosen)

Reuse M9's `visibility_polygon` raycast as the geometry primitive for **both** vision and each light
(a light is a radius-bounded viewpoint emitting a polygon, occluded by `blocksLight`). **Rasterize
all polygons onto the existing fog grid and compose visibility per cell** (AND/OR). The grid is the
shared composition + secrecy + movement + pathfinding substrate (already the fog resolution);
polygons are reused primitives, not new boolean math.

- Rejected — **pure polygon booleans** (composing N light radii + falloff + gradation + darkvision in
  vector space is the most error-prone path).
- Rejected — **pure grid field** (discards the crisp raycast LOS we already have).

Tradeoff: lit/visible **determination** is at grid-cell resolution (same as today's fog). Crisp
*appearance* (smooth gradients, soft edges) is the client's job in V3 — it renders prettily *within*
what the mask permits; the server mask stays coarse, authoritative, secure.

## 5. V1 — Data model (M10e-1)

### 5.1 World config-documents (world-scoped; faction/condition-registry pattern; modular/replaceable)

- **`world-settings`** — the defaults every scene inherits:
  ```jsonc
  {
    "scene": {
      "losRestriction": true, "fog": true,
      "lightingEnabled": true, "lightMode": "environmentLight",
      "environment": { "color": "#0a0e1a", "intensity": 0.0 },
      "observerVision": false,
      "movementRestriction": "visible",   // "visible" | "revealed" | "unrestricted"
      "partialCellLeniency": true
    },
    "pathfinding": { "diagonalRule": "chebyshev" }, // | "alternating" | "euclidean" | "manhattan"
    "animation":   { "speedCellsPerSec": 6, "easing": "easeInOut" } // | "linear"
  }
  ```
- **`light-gradation`** — `{ "bands": [{ "name": "bright", "minIllumination": 0.67 },
  { "name": "dim", "minIllumination": 0.34 }, { "name": "dark", "minIllumination": 0.0 }] }`
  (redefinable; the named bands the illumination axis is sliced into).
- **`vision-modes`** — registry seeded with `normal` (floor = `dim`) and `darkvision`
  (floor = `dark`, perceives unlit within range). Each:
  `{ "id", "name", "illuminationFloor": "<band-name>", "defaultRange": <cells>, "renderHint"? }`.
  Custom modes (blindsight/truesight) slot in later.

### 5.2 Scene document (`scene.system`) — per-scene overrides; absent field ⇒ inherit world default

```jsonc
{
  "vision":   { "losRestriction"?: bool, "fog"?: bool, "observerVision"?: bool,
                "movementRestriction"?: "visible"|"revealed"|"unrestricted" },
  "lighting": { "enabled"?: bool, "mode"?: "globalIllumination"|"environmentLight",
                "environment"?: { "color": "#rrggbb", "intensity": 0.0..1.0 } },
  "grid":     { /* existing kind, size */, "distance"?: { "perCell": 5, "unit": "ft" } }
}
```
`environment` is a normal document field updated via the standard intent API, so a module can drive
an automated day/night cycle; the client interpolates smoothly on change (§7.2). `grid.distance` is
labels-only (ruler/budget/range readouts); lighting/vision math uses cells/pixels.

### 5.3 Wall (`wall.system`) — add `blocksLight`

A boolean alongside `blocksMove`/`blocksSight`. New walls default `blocksLight = blocksSight`;
legacy/missing ⇒ `false` (safe — light can only *brighten* within the LOS mask, never reveal beyond
it).

### 5.4 Light — new `light` doc_type (`parent_id = scene`; GM-authored like walls)

```jsonc
{ "x": f64, "y": f64, "color": "#rrggbb", "intensity": 0.0..1.0,
  "brightRadius": <cells>, "dimRadius": <cells>,
  "falloff"?: { "curve": "linear"|"quadratic"|"none" }, "enabled": bool }
```
Emits a radius-bounded visibility polygon occluded by `blocksLight` (reuses the M9 raycast).
`brightRadius`/`dimRadius` feed the gradation by default; `falloff` enables smooth gradients.

### 5.5 Actor / Token vision modes

- **Actor** (M10a) gains `system.vision: [{ "mode": "<vision-mode-id>", "range": <cells> }]`
  (`[]` ⇒ normal only).
- **Per-token override** via the existing `TokenOverrides` (`overrides.vision?`).
- Resolved through `resolveTokenActor` → **`EffectiveActor.visionModes`** (joins the existing
  size/shape/faction/conditions resolution).

### 5.6 Observer designation

Reuses the **token document permission tier** — a user holding the observer/view tier on a token is
a vision source when `observerVision` is on. No parallel list. (Confirm the exact tier name against
the existing `PermissionSet`/M10b tiers during the V1 plan; add an `Observer`/`View` tier only if one
does not already convey this.)

### 5.7 Units

Light radii, darkvision ranges, footprints, A* costs stay in **grid units (cells)**.
`grid.distance.perCell` is multiplied in only for human-readable labels.

## 6. V2 — Server lighting-aware vision (M10e-2)

Extends the M9 vision pipeline in `src/server/src/scene/` (`player_vision_polygons`, `vision.rs`,
`explored.rs`). Per dispatch, two stages:

1. **Illumination field (scene-shared, computed once):** `lightingEnabled` off ⇒ all-bright; else
   `globalIllumination` ⇒ all bright (tinted by `environment`); else `environmentLight` ⇒ raycast
   each enabled `light` (radius-bounded, occluded by `blocksLight`) **+** edge-projected environment
   light → rasterize to a **per-cell illumination + tint** field. Multiple contributors combine by
   **max** (no over-brightening).
2. **Per-player mask:** compute `vision_sources(user)`; rasterize their `visibility_polygon`s to
   `vision_cells` (or all-cells if `losRestriction` off); a cell is visible iff some source's polygon
   covers it AND `illumination ≥ floor(source, cell)` (darkvision lowers the floor within range).
   GM ⇒ all cells.

**Egress (secrecy-safe):** the per-player vision frame carries only visible cells, each with its
illumination band + tint, plus accumulated explored cells; entities are filtered to the mask. This
**generalizes the current polygon vision frame to a grid mask + illumination** — the notable M9
rework — reusing the existing raycast and the polygon→cell rasterizer that already builds explored
fog. The illumination field is player-independent (compute once/scene/dispatch); only the vision
intersection is per-player (same raycast cost class as M9 today, plus one raycast per light).

## 7. V3 — Client lighting render (M10e-3)

Renders via the M8c render-layer / mask-compositor seam (the seam vision was pulled forward to drive).

### 7.1 Lighting layer
Composites the server's per-cell illumination + tint over the scene: bright/dim/dark treatment per
the gradation bands, colored by `environment`/light tint. Edges render smooth (gradient compositing)
despite the per-cell server mask.

### 7.2 Smooth transitions
`environment {color, intensity}` changes interpolate over a short duration (the day/night fade),
driven off the field update.

### 7.3 Vision modes & fog
Darkvision rendered per the vision-mode `renderHint` (e.g. desaturated within darkvision-only area).
Fog integrates the light dimension: explored-but-not-currently-visible cells render as dimmed memory;
never-seen cells stay dark.

## 8. M1 — Movement restriction (M10e-4)

Extends the M9 `Room::publish` gate (`src/server/src/ws/room.rs:164-214`). For a **non-GM** move of a
token by user `U`, after the existing `blocks_move` wall check:

- **`unrestricted`** → walls only.
- **`visible`** → every cell the move segment `a0→a1` passes through (supercover rasterization) must
  be in `U`'s **current visible mask** (V2, over `U`'s vision sources).
- **`revealed`** → same, against `U`'s **explored set** (`get_explored`, already server-persisted) ∪
  current-visible.

**Partial-cell leniency** (default true) selects the rasterization rule for the movement *and*
pathfinding mask: lenient ⇒ a cell counts if the vision/explored polygon overlaps it at all; strict ⇒
the polygon must cover the cell *center*.

**Entire-move-in-mask** = all supercover cells of `a0→a1` pass (not just the endpoint). A violating
non-GM move is rejected with `DataError::Forbidden` *before* the write (consumes no seq; client rolls
back) — identical to the M9 wall gate. GM exempt.

*Implementation note:* the gate reuses the V2 mask function for `(U, scene)`; moves are human-paced,
so computing it on demand (or reusing the last egress-computed mask via a small per-`(user,scene)`
cache) is acceptable — settle caching in the plan.

## 9. M2 — Movement animation (M10e-5)

Client, extends the M8d token tween + render ticker. On an authoritative position change the token
**tweens** instead of snapping. Config from `world-settings.animation`: **`speedCellsPerSec`**
(duration = distance ÷ speed) and **`easing`** — default **`easeInOut`**, option **`linear`**.
Drives both drag commits and P1 routed moves (token animates **along the path waypoints**).
**Interruptible:** a newer authoritative position retargets the in-flight tween. Per-user override of
speed/easing is a later option; world-level governs game feel now.

## 10. P1 — Grid A* pathfinding (M10e-6)

Server-side, engine-owned, request/response like search; the original M10e, now mask-aware.

**Seam:** `find(start, goal, waypoints[], footprintRadius, costField, movementModel) → { path[], cost }`.
M10e implements `movementModel = grid-stepped`; the seam dispatches by model (continuous/Polyanya is
M10f).

**Frames** (mirror search in `protocol.rs`/`conn.rs`; ts-rs exported; one-shot to requester only):
```
ClientMsg::Pathfind   { request_id, scene, start, waypoints[], footprint_radius }
ServerMsg::PathResult { request_id, path:[Point], cost:f64 } | PathError { request_id, message }
```

**Grid A\*** (hand-rolled; nodes = cells; king-moves). A cell is **passable** iff: the step between
cell centers doesn't cross a `blocksMove` wall (reuse M9 `blocks_move`); **AND** for a non-GM
requester the cell is in *their* mask (same `visible`/`revealed` + leniency mask as M1 — preview and
authoritative move agree, route never threads the unknown nor leaks hidden geometry); **AND** the
token's **clearance footprint** (from `footprintRadius`, multi-cell aware) is clear. GM unconstrained
by the mask.

- **Diagonal rule** from `world-settings.pathfinding.diagonalRule`: `chebyshev` (1‑1) |
  `alternating` (5‑10‑5, parity-tracked) | `euclidean` (√2) | `manhattan`.
- **`costField`/per-cell weights:** the seam accepts them; M10e passes **uniform** cost — the
  weighted-region hook stays inert until M10g.
- **Waypoints:** path = A* legs `start→wp₁→…→goal`, each under all constraints; costs summed. Output
  = cell-center points (scene coords) + total cost.

**Client** (scene-tools + render):
- **Waypoint tool** — the move/measure tool gains click-to-add waypoints (draw-tool multi-point
  pattern); sends a `Pathfind` request (mirrors `Core.search`) on change.
- **Path-preview overlay** — ephemeral routed polyline + footprint via `previewOverlay`.
- **Movement-budget readout** — path cost × `grid.distance.perCell` + unit (e.g. "45 ft"). The
  token's actual movement allowance comes from actor stats (deferred to M12); M10e shows distance,
  with an optional manual cap as a later add.
- **Commit** — the move stays a standard optimistic document intent gated by the M9 + M1 server
  block; the token animates along the path (M2). Pathfinding produces the route, not the move.
- **Ruler** — `src/client/render/src/grid.ts` `distance()` gains the `alternating` rule alongside
  chebyshev.

## 11. Decomposition (6 checkpoints)

| # | Unit | Deliverable |
|---|---|---|
| **M10e-1** | V1 | Vision/lighting data model + config: world-settings/light-gradation/vision-modes config-docs, scene overrides, `blocksLight` wall flag, `light` doc_type, actor/token vision modes, `grid.distance`. |
| **M10e-2** | V2 | Server lighting-aware vision: illumination field + per-player grid mask + secrecy-safe egress (generalizes the M9 polygon vision frame). |
| **M10e-3** | V3 | Client lighting render: lighting layer, smooth transitions, darkvision/fog integration. |
| **M10e-4** | M1 | Movement restriction at the `Room::publish` gate (visible/revealed/unrestricted, entire-move + leniency, GM exempt). |
| **M10e-5** | M2 | Movement animation: configurable speed + easing tween along moves/paths. |
| **M10e-6** | P1 | Grid A* pathfinder + seam + frames + waypoint tool + preview + budget readout + ruler `alternating`. |

**Dependency order:** `e-1 → e-2 → {e-3, e-4} → e-6`; **e-5** anytime. Each is independently
shippable + buddy-checked (M8/M9 cadence); `/clear` between checkpoints. After M10e-6: **M10f**
(continuous pathfinding) and **M10g** (regions) resume per the M10 spec.

## 12. Decisions — CONFIRMED (user, 2026-06-24)

1. Split the original M10e: movement restriction first (priority), pathfinder last; six checkpoints.
2. Reachability/lighting knobs: **world default + per-scene override**.
3. Drag gate tests the **entire move** stays in mask (not just endpoint), with **partial-cell
   leniency default true** (a partially-revealed cell counts as revealed).
4. Five scene axes: LOS restriction, lighting enabled (master), light mode (global illumination vs
   environment light), fog, per-actor vision modes (darkvision).
5. **Environment light** = edge-projected, occludable by `blocksLight`, with color + intensity;
   global illumination = whole-scene lit with the lighting system still active. Lighting can be
   disabled entirely (no darkness/tint concept).
6. Light levels: **continuous illumination + configurable gradation scheme** (bands redefinable);
   vision modes data-driven (range + illumination floor).
7. Light source authoring: **bright + dim radii AND an optional falloff curve** (+ color, intensity).
8. Vision is **per-user**: owned tokens by default, optionally observer-tier tokens when
   `observerVision` is on; observer = a token **permission tier** (single source of truth).
9. Day/night: **GM-adjustable `environment` color/intensity with smooth transitions**, exposed as a
   normal field so a module can automate it (no built-in clock now).
10. Computation: **hybrid** — reuse the M9 raycast for vision + light polygons, rasterize to the fog
    grid, compose per cell.
11. Movement animation: configurable speed + easing, default `easeInOut`, option `linear`.

## 13. Security considerations

- The grid mask is the **only** thing gating per-player data; it fails closed (empty mask ⇒ nothing
  shipped). Walls, lights, and illumination outside the mask are never serialized to that player.
- Movement restriction and pathfinding use the **same** mask, so the path preview cannot leak
  geometry the authoritative gate would forbid, and vice-versa.
- The authoritative move start is the committed ECS position, never the client's claimed pre-image
  (M9a invariant preserved); the mask is tested against the post-image path.
- `blocksLight` defaulting to `false` for legacy walls is safe: light only brightens *within* the LOS
  mask and can never extend visibility past it.

## 14. Out of scope / deferred

- **Weighted/impassable regions** feeding per-cell A* cost — **M10g** (the `costField` hook ships
  inert in M10e-6).
- **Continuous/gridless pathfinding** (`vleue/polyanya`) — **M10f**.
- **Mechanical effects** of light level (e.g. disadvantage in dim light) — needs the rules engine
  (combat milestones).
- **Actor-stat movement budgets** (enforced speed) — M12; M10e-6 shows distance only.
- **Automated day/night clock** — a module can drive `environment`; no built-in time system.
- **Per-user animation overrides** — world-level only for now.
- **Sub-cell light resolution** for the authoritative mask — client renders smooth; server stays
  cell-granular.
