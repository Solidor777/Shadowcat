# M10e-6 — Grid A\* Pathfinder: Design Spec

**Date:** 2026-06-25
**Status:** Design approved (decisions §9); ready for the implementation plan.
**Parent:** `2026-06-24-m10e-vision-lighting-movement-design.md` §10 (P1 — Grid A\* pathfinding). This
doc distills §10 into an implementable per-checkpoint spec and pins the decisions §10 left open.
**Position:** the **last** of the six M10e sub-checkpoints (e-1…e-4 done; e-5 anytime). After this,
**M10f** (continuous/Polyanya) and **M10g** (weighted/impassable regions) resume per the M10 spec.

## 1. Goal

A server-authoritative, engine-owned **grid A\*** pathfinder, request/response like search: given a
start, ordered waypoints, a goal, and the moving token's footprint, return the cell-stepped route and
its movement cost. The route is **mask-bounded** for non-GM requesters (it consumes the same
per-`(user, scene)` visibility mask as the M10e-4 movement gate, so a previewed path can never thread
the unknown nor leak hidden geometry), and **footprint-clear** (the token's body fits the route). The
client extends the measure tool into a waypoint router with a path-preview overlay and a
movement-budget readout, and teaches the ruler the `alternating` (5-10-5) diagonal rule.

## 2. Constraints inherited (cited inline)

- **Server-authoritative, engine-owned geometry** (ARCHITECTURE §2 invariant 6 + the M9 exception):
  walls are GM-secret, so pathing is server-side. Pure computational geometry only (clean-room;
  ARCHITECTURE §7).
- **Same mask as the gate** (parent §13): the pathfinder's per-cell mask test is the **same**
  `visible_cells` (strict==`player_lit_mask`) the M10e-4 gate uses; never fork the per-cell decision.
- **Fail-closed** (`fog-is-the-secrecy-gate-fail-closed`): a degenerate request, an over-cap search,
  or an empty mask yields a `PathError`, never a partial or geometry-leaking answer.
- **Cross-platform** — pure Rust, `std::path` only, deterministic ordering into wire output
  (`BTreeMap`/sorted `Vec`, never `HashMap` iteration); the client renders desktop + mobile/touch.
- **ts-rs is the wire source of truth** — the new frames are Rust enums with
  `#[ts(export, export_to = "../../types/generated/")]`; the client Zod schema mirrors them (drift
  guard). Internally tagged on `type`, `snake_case`.
- **Optimistic move unchanged** (M10 §10.4): pathfinding produces the *route*, not the move. The move
  remains a standard optimistic document intent gated by the M9 (`blocks_move`) + M10e-4 server block.

## 3. The seam & frames

### 3.1 Pathfinder seam (engine-internal)

```
find(start, goal, waypoints[], footprint_radius, cost_field, movement_model) -> { path[], cost }
```

M10e-6 implements `movement_model = grid-stepped`; the seam dispatches by model (continuous/Polyanya
is M10f). `cost_field` is accepted but **inert** in M10e-6 (uniform per-cell weight = 1); the weighted
hook activates in M10g.

### 3.2 Wire frames (mirror `Search`/`SearchResult`/`SearchError`; one-shot to requester only)

```rust
// ClientMsg (protocol.rs) — request
Pathfind {
    request_id: Uuid,
    scene: Uuid,
    start: (f64, f64),                 // scene coords
    waypoints: Vec<(f64, f64)>,        // ordered intermediate points (may be empty); last = goal
    footprint_radius: f64,             // grid units (cells); bounding-disc radius (client footprintRadius)
}

// ServerMsg (protocol.rs) — response (to the requesting connection only)
PathResult { request_id: Uuid, path: Vec<(f64, f64)>, cost: f64 }   // cell-center scene coords; cost in cells
PathError  { request_id: Uuid, message: String }
```

- **Goal encoding:** the goal is the **last** element of `waypoints` (so `waypoints` is the full
  ordered leg list `start → wp₁ → … → goal`). An empty `waypoints` is an invalid request (no
  destination) → `PathError`. (This keeps the frame a single ordered list rather than a separate
  `goal` + `waypoints`; the seam signature's `goal` is `waypoints.last()`.)
- **Routing:** parsed in `conn.rs` ingress exactly like one-shot `Search` (no `subscribe`), correlated
  by `request_id`, dispatched to the requesting connection's egress sink — never broadcast.
- **`PathResult` vs `PathError`:** a found route → `PathResult` (path = ordered cell-center points
  including start and goal; `cost` = summed leg cost in cell-units). Unreachable-within-constraints,
  degenerate input, or over-cap search → `PathError { message }` (`"unreachable"`, `"invalid
  request"`, `"search exceeded"`). The client renders a "no route" state on `PathError`.
- **Units:** `cost` is in **cells**; the client multiplies `grid.distance.perCell` for the readout
  (parent §5.7). `path` points are **scene coordinates** (cell centers).

## 4. Server grid A\* (`scene/pathfinding.rs`, pure + headless)

Hand-rolled grid A\*; nodes = cells; **king-moves** (8-neighborhood). Clean-room (A\* — Hart, Nilsson
& Raphael 1968; the diagonal-cost rules are standard tabletop grid metrics). Pure: takes parsed
inputs (walls, mask, cell size, rule, footprint), returns the solution; no I/O, no ECS borrow.

### 4.1 Diagonal-cost rules (from `world-settings.pathfinding.diagonalRule`)

All four are the **same king-move graph**; they differ only in diagonal cost and the admissible
heuristic. `dmax = max(|Δi|,|Δj|)`, `dmin = min(|Δi|,|Δj|)`.

| rule | orth step | diagonal step | admissible + consistent heuristic |
|---|---|---|---|
| `chebyshev` (5e, 1-1) | 1 | 1 | Chebyshev `dmax` |
| `manhattan` | 1 | 2 | Manhattan `|Δi|+|Δj|` |
| `euclidean` | 1 | √2 | Octile `(dmax − dmin) + √2·dmin` |
| `alternating` (PF1e/3.5, 5-10-5) | 1 | 1, 2, 1, 2 … (parity-tracked) | Chebyshev `dmax` (optimistic: every diagonal cheap) |

Unknown/missing rule resolves to `chebyshev` (server mirrors the client `DEFAULT_WORLD_SETTINGS`;
verify against `scene-docs.ts`, not this table). `manhattan` keeps diagonals **allowed at cost 2**
(diagonal-cost rule, not 4-connectivity) — a diagonal and its L-detour are cost-equal.

### 4.2 Alternating parity (5-10-5) — node state

For `alternating`, the cost of a diagonal depends on how many diagonals the path has already taken:
the k-th diagonal costs `1` if k is odd, `2` if k is even (5,10,5,10 ft). The A\* node therefore
carries a **parity bit** `p = (diagonals_taken) mod 2`:

- Taking a diagonal from a node with parity `p`: cost = `1` if `p == 0` (next diagonal is
  odd-indexed) else `2`; the successor's parity is `1 − p`.
- Orthogonal steps cost `1` and leave parity unchanged.
- **Node = `(cell, parity)`** uniformly. For the other three rules diagonal cost is parity-independent,
  so the search never branches on parity (parity stays `0`). **Start parity = `0`.**
- **Goal test:** any node whose `cell == goal` (parity irrelevant at the goal).
- **Waypoint legs:** parity **carries** from the end of one leg into the next leg's start (a route
  through waypoints is one continuous move; resetting parity per leg would misprice 5-10-5). Only
  `alternating` observes this; for the others it is a no-op.

### 4.3 Passability — full geometric footprint clearance (decision §9.1)

The footprint is a **disc** of radius `R = footprint_radius · cell` (scene units) centered at a cell
center (the client's `footprintRadius` returns a bounding-disc radius for both square and circle
shapes — parent §9). A candidate cell `c` is **enterable** iff **all** hold:

1. **Footprint-wall clearance (full geometric):** no `blocksMove` wall segment comes within `R` of
   `c`'s center — `dist(segment, center) ≥ R` for every wall. This is what makes the route
   footprint-aware: a token wider than a gap cannot route through it (a 1-cell gap between two walls
   rejects any disc of radius ≳ ½ cell). Source: point-to-segment distance (clean-room).
2. **Mask (non-GM only):** every cell **overlapped by the footprint disc** is in the requester's
   mask (the M10e-4 `visible`/`revealed` set under the scene's `partialCellLeniency`). The token's
   whole body stays within seen/revealed area — a secrecy concern (a big body must not extend into
   unseen cells). GM is unconstrained by the mask.
3. **Step-wall clearance:** the predecessor→`c` center-to-center segment crosses no `blocksMove`
   wall (reuse the M9 `segments_cross` predicate behind `blocks_move`).

The **start** cell is the origin (always in the path, never re-tested for entry). The **goal** cell
must be enterable. `cost_field` (M10g) multiplies a per-cell weight into the step cost; M10e-6 uses
weight `1`.

> **Asymmetry with the authoritative gate is intentional and safe.** Authoritative
> movement-blocking stays **center-based** (M9/M10e-4; parent §14 defers footprint-aware *blocking*).
> The route is *stricter* on footprint than the gate, so a routed path is always gate-passable
> (stricter ⊆ allowed) — the preview never suggests a move the gate would reject — while the gate
> never needs the footprint. The shared **mask** test (§2) guarantees the route can't leak geometry
> the gate forbids.

### 4.4 Search bounds (fail-closed DoS guards)

- **Waypoints:** `waypoints.len()` capped at `MAX_WAYPOINTS` (32); over-cap → `PathError`.
- **Footprint:** `footprint_radius` finite and `0 ≤ r ≤ MAX_FOOTPRINT_CELLS` (64); else `PathError`.
- **Coordinates:** all of `start`/`waypoints` finite; `cell > 0`; else `PathError`.
- **Search window:** A\* is bounded to the AABB of `{start} ∪ waypoints ∪ blocksMove-wall extent`,
  expanded by a margin; cells outside are non-enterable. This bounds a GM search (no mask) when the
  goal is unreachable; for non-GM the finite mask is the tighter bound.
- **Node cap backstop:** total expanded nodes capped at `MAX_PATH_NODES` (e.g. 200_000); exceeding it
  → `PathError { "search exceeded" }` (never a truncated path).
- **Determinism:** the open set breaks f-score ties deterministically (e.g. by `(cell, parity)` order)
  so identical requests yield identical routes across runs/OSes.

### 4.5 ECS assembly (`SceneEcs::pathfind`)

A thin method on `SceneEcs` assembles the pure search's inputs and is called from the handler:

```
pathfind(user, scene, start, waypoints, footprint_radius, is_gm, explored) -> Result<(Vec<(f64,f64)>, f64), PathErrorKind>
```

- Resolves the scene's `diagonalRule` (new resolver mirroring the client) and
  `movementRestriction` + `partialCellLeniency` (existing `resolve_scene`).
- **Mask:** for non-GM, `visible_cells(user, scene, lenient)`, unioned with the passed `explored`
  set when `movementRestriction == Revealed`. GM → `None` (unconstrained). `Unrestricted` →
  walls-only (no mask). Empty mask for a non-GM `Visible`/`Revealed` scene ⇒ no enterable cell ⇒
  `PathError` (fail-closed — consistent with the dark-scene movement freeze, parent §13).
- **Walls:** a new `move_walls(scene) -> Vec<Seg>` accessor (the `blocksMove` segments) feeds both
  the footprint and step tests.
- Runs `pathfinding::find` per leg (`start→wp₁`, …, `wpₙ₋₁→goal`), threading parity, summing cost,
  concatenating points (de-duplicating shared leg endpoints). Any leg with no route ⇒ `PathError`.

### 4.6 Handler (`conn.rs`)

Mirrors the one-shot `Search` dispatch. On `ClientMsg::Pathfind`: resolve the requester's GM status
(world role) and, when `movementRestriction == Revealed` and non-GM, `repo.get_explored(scene,
user).await` (after dropping the scene read guard — no lock across await, mirroring the M10e-4 gate).
Call `SceneEcs::pathfind`; wrap the result in `PathResult`/`PathError`; send to the requesting
connection's egress sink only.

## 5. Client — measure tool route mode (decision §9.2)

The **measure tool** gains a waypoint/route mode (it already does anchor→point with a live distance
label — the natural host; matches parent §10.4 "the move/measure tool gains waypoints").

- **Waypoint placement:** click-to-add ordered waypoints (the draw/template multi-point pattern in
  `scene-tools/controller.svelte.ts`); the moving token's center is the start; the final click /
  commit is the goal. Snap to grid via `ctx.scene.snap()`.
- **Pathfind call:** a new `WsClient.pathfind(start, waypoints, footprint_radius, opts)` mirrors
  `WsClient.search` (UUID `request_id`, `pending` map, timeout, resolve on `pathfind_result`/reject on
  `pathfind_error`), exposed through a new `AppContext.pathfind(...)` seam (wired in `Table.svelte`
  via `WorldSession`). The tool resolves `footprint_radius` from the selected token's
  `EffectiveActor` (`footprintRadius(eff)`), and re-requests on waypoint change (debounced/coalesced
  like the move tool's coalesced intents).
- **Path-preview overlay:** the returned route renders as an ephemeral polyline (+ footprint disc
  hint) via `ctx.scene.previewOverlay([...])`; cleared on tool swap / release / new request
  (`clearOverlay`) — the mid-gesture-clear gotcha (scene-rendering skill).
- **Movement-budget readout:** `cost × grid.distance.perCell + unit` (e.g. "45 ft"), read from the
  scene's `grid.distance` (`{ perCell, unit }`, default `{5,"ft"}`). Actor-stat movement allowances
  are M12; M10e-6 shows distance only (an optional manual cap is a later add).
- **Commit:** unchanged — the move is a standard optimistic intent gated by the M9 + M10e-4 server
  block; the token animates along the route once M10e-5 lands (not required by this checkpoint).

### 5.1 Ruler `alternating`

`src/client/render/src/grid.ts` `distance()` gains the `alternating` (5-10-5) rule alongside the
current `chebyshev` for square grids: `dmin` diagonals + `(dmax − dmin)` orthogonals, with the
diagonals costed 1,2,1,2… (`dmin` diagonals → `dmin + floor(dmin/2)` cells). The rule comes from the
scene's resolved `pathfinding.diagonalRule`; unknown ⇒ `chebyshev` (current behavior). Hex distance
is unchanged.

## 6. Security considerations

- The mask is the **only** thing gating per-player geometry; the pathfinder consumes the **same**
  `visible_cells` mask as the M10e-4 gate (parent §13), so the preview cannot leak a wall/region the
  gate would hide, and the gate cannot forbid a cell the preview routed through.
- `start`/`waypoints` are **client-claimed**, but for non-GM **every** cell on the route (including the
  start cell and the whole footprint disc) must be in the requester's mask — a claimed start inside
  hidden geometry yields `PathError`, never a leak. No token id is trusted: footprint comes as a
  scalar radius (client resolves `EffectiveActor`); the server resolves only geometry.
- Fail-closed on degenerate/over-cap/empty-mask (§4.4) — `PathError`, never a partial route.
- The pathfinder is **read-only** (no document write, no seq); it cannot mutate authoritative state.

## 7. Out of scope / deferred

- **Weighted/impassable regions** feeding per-cell A\* cost — **M10g** (`cost_field` ships inert).
- **Continuous/gridless** (`vleue/polyanya`) and movement-model dispatch beyond grid-stepped — **M10f**.
- **Footprint-aware authoritative blocking** — stays center-based (parent §14); only the *route* is
  footprint-clear here.
- **Actor-stat movement budgets** (enforced speed) — M12; M10e-6 shows distance only.
- **Animating along the route** — M10e-5 (independent; not required by this checkpoint).
- **Hex pathfinding** — square grids only in M10e-6 (the ruler's hex distance is untouched).

## 8. Decomposition (implementation plan tasks)

See `docs/superpowers/plans/2026-06-25-m10e-6-grid-pathfinder.md`. Server first (pure A\* → ECS
assembly → frames + handler), then client (wire + `pathfind` call + AppContext seam → measure-tool
route mode + preview + budget → ruler `alternating`), then docs + skill sync. Buddy-checked on the
M8/M9/M10 cadence (security-sensitive: the mask now also bounds the route).

## 9. Decisions — CONFIRMED (user, 2026-06-25)

1. **Footprint clearance = full geometric** (§4.3): the route's footprint disc must clear `blocksMove`
   walls (token body can't pass a gap narrower than the footprint) **and** every footprint cell must be
   in the non-GM mask. Authoritative blocking stays center-based (parent §14); the route is the
   stricter, footprint-aware layer.
2. **Route UI = extend the measure tool** (§5) with a waypoint/route mode + preview + budget readout —
   not a new tool, not the move tool.
3. (Merits) Frames mirror `Search`: `Pathfind`/`PathResult`/`PathError`, one-shot to requester,
   ts-rs-exported; goal = `waypoints.last()`; `cost` in cells; `path` in scene coords (§3).
4. (Merits) `alternating` 5-10-5 tracked via a **parity bit in the A\* node** carried across waypoint
   legs; per-rule **admissible + consistent** heuristics (§4.1–4.2).
5. (Merits) `cost_field` accepted but **inert** (uniform weight 1) until M10g.
6. (Merits) Fail-closed search bounds: waypoint/footprint/coordinate validation, a search window, and a
   node-cap backstop, all → `PathError` (§4.4).
