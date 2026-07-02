# M10g — Weighted / Impassable / Hazard Regions (grid engine) — Design

> Checkpoint of the M10 Tokens milestone. Parent spec:
> `2026-06-24-m10-tokens-design.md` §10.3. Grid engine only; the continuous
> (Polyanya) cost-layer wiring is **M10f**. Builds directly on the planted inert
> hooks from the M10e-5 M3 work (`region_arrests()` in both `scene/move_exec.rs`
> and `scene/pathfinding.rs`) and the server-authoritative movement model
> (`MoveRequest`/`move_exec`/`MoveStream`).

## 1. Goal

Let a GM author **regions** on a scene — vector-shaped zones that make grid
movement cost more (difficult terrain), route entirely around them (impassable),
or halt a token that steps into them (hazard-arrest) — enforced
server-authoritatively in both the grid A\* router and the `move_exec` executor,
and rendered/previewed on the client only to the players allowed to see them.

## 2. Locked decisions (user, 2026-07-01)

1. **Three behaviors ship in v1:** `terrain` (per-cell cost multiplier),
   `impassable` (routed around / entry refused), `arrest` (enter and stop).
2. **Per-region secrecy** via the existing permission tiers on the doc envelope
   (`all` = visible, `gm_only` = secret trap, `owner_or_gm` available). Secret
   regions are stripped from a player's egress AND absent from that player's
   pathfinder/budget field (routed straight through), then sprung by `move_exec`.
   Visible regions appear in render + route + budget.
3. **Authoring = vector shapes** (rect / circle / polygon), rasterized to grid
   cells server-side. Reuses the M8d-3a drawing/template geometry + preview
   overlay.
4. **Overlap compose = precedence + MAX.** Behavior precedence
   `impassable > arrest > terrain`; overlapping `terrain` costs take the **MAX**
   multiplier (difficulty is not cumulative). Deterministic, order-independent.
   Mirrors the per-cell lighting max-compose.
5. **Arrest is honest in preview:** the player-facing router truncates the route
   at the first **visible** arrest cell and flags it (`PathResult.arrested`).

## 3. Data model

New **`region` doc_type**, `parent_id = scene` (same shape as `wall` / `light`).

```
system = {
  shape:    Rect | Circle | Polygon,   // align with M8d-3a drawing shape geometry
  behavior: "terrain" | "impassable" | "arrest",
  cost:     number,                    // multiplier ≥ 1; only meaningful for "terrain"
  enabled:  bool,                      // GM live-toggle without deleting the doc
}
```

- **Visibility** rides the envelope permission tier — no new secrecy machinery;
  reuses per-recipient egress stripping (`documents-permissions`).
- `cost` is validated `≥ 1` (a multiplier below 1 would break heuristic
  admissibility, §5). `behavior != "terrain"` ignores `cost`.
- ts-rs type → regenerate → mirror in the client Zod schema (drift guard).
- `enabled == false` → the region contributes nothing to any field (§4); the doc
  persists so a GM can flip it back on with a single field-path Update.

## 4. Server ECS + rasterization (`scene/regions.rs`, new, pure)

- New **`SceneEcs` region side-table**, hydrated at room cold-start and rebuilt on
  region-doc mutation (mirrors the light / config-doc side-tables). Filter:
  `doc_type == "region" && parent_id == Some(scene) && system.enabled`.
- **Rasterize** each region's vector shape → the set of grid cells it covers, then
  **compose per cell** by precedence (`impassable > arrest > terrain`) + MAX cost,
  producing a **region field**: `cell -> { impassable | arrest | multiplier }`.
- **Membership rule:** a step into a cell qualifies if **any of the token's
  footprint-disc cells** (the same set `cell_enterable` already computes) intersect
  the region shape — conservative / fail-closed, consistent with the footprint
  clearance test. Degenerate / over-cap shapes fail closed (treated as no region,
  never as a silent all-pass — bound the raster like the existing
  `MAX_FOOTPRINT_CELLS` / supercover caps).
- **Two fields, mirroring the existing `visible_cells` vs authoritative split:**
  - **Authoritative field** — all enabled regions. Consumed by `move_exec` (the
    server springs everything, including secret regions).
  - **Per-requester visible field** — only regions that requester may see (GM's =
    authoritative). Consumed by the player-facing router + budget, so a secret
    region never leaks via a detour or an inflated cost.

## 5. Grid A\* (`scene/pathfinding.rs`)

Uses the **per-requester visible** field.

- `astar_leg` step cost becomes `base_diagonal_cost × entered_cell_multiplier`
  (multiplier = 1 outside terrain). Heuristic stays **admissible + consistent**
  because the minimum multiplier is 1 (§3 validation).
- **impassable** cell → not enterable, via the existing `cell_enterable` gate
  (routes around). This gives `region_arrests()`'s pathfinder twin a real body for
  the impassable case.
- **arrest** cell → the route is **truncated** at the first visible arrest cell and
  `PathResult` reports `arrested: true`. Mirrors `move_exec`'s `stopped_early`.
- Secret regions are absent from the field → routed straight through, full budget;
  `move_exec` springs them at execution.
- `cost` in `PathResult` is the real weighted sum (client still multiplies by
  `grid.distance.perCell` for the display readout).

## 6. `move_exec.rs` (authoritative execution)

Uses the **authoritative** field. Per step, after the existing wall + vision-mask
checks:

- **impassable** → stop before the cell (defense-in-depth; a well-formed path
  already avoids it). Treated like a wall for `stopped_early`.
- **arrest** → stop **at** the cell (`stopped_early`, including on the final step).
  This is exactly the planted `region_arrests()` hook's current stop semantics —
  the stub gets its real body here.
- **terrain** → accumulate the weighted cost into the move's cost accounting.
- **Animation pacing stays distance-based** (`resolved_animation_speed`
  unchanged): difficult terrain costs movement budget, it does not visually slow
  the token. Revisitable later.

Both planted `region_arrests()` stubs (this file + `pathfinding.rs`) light up
together — the M3 work planted them as a matched pair for exactly this checkpoint.
The §13 per-cell mask-parity invariant between `move_exec` and the router is
extended: **the router's visible field ⊆ the authoritative field**, so a route the
player is shown is always executable-or-arrested, never wall-blocked by a region
the server knows and the player doesn't.

## 7. Protocol

- `PathResult` gains **`arrested: bool`** (ts-rs + Zod mirror). No other frame
  changes: `MoveRequest`/`MoveExecuted`/`MoveStream` already carry per-step
  outcome via the existing `stopped_early` path, so arrest-at-execute needs no new
  field.

## 8. Client

- **Render:** regions draw as tinted / hatched shapes on a **region layer** in
  `@shadowcat/render` (stage). Only visible regions reach the client via egress
  filtering, so secrecy is automatic — no client-side hide logic to get wrong
  (`fog-is-the-secrecy-gate` discipline: the server, not the client, is the gate).
- **Authoring:** a GM **region tool** in `@shadowcat/module-scene-tools` (alongside
  draw / template / wall): place rect / circle / polygon; pick behavior + cost +
  visibility tier + enabled; existing preview-overlay tooling. Region editing
  (change cost / behavior / toggle enabled) via field-path Updates like the other
  scene entities.
- **Budget / preview:** the measure-tool route mode already renders path + budget;
  it now reflects terrain cost and shows arrest truncation (`PathResult.arrested`).
  No new panel.

## 9. Cross-platform / bloat

- **No new crate** (grid engine only; Polyanya deferred to M10f) → the
  cargo-bloat budget is untouched. This is the concrete reason M10g-before-M10f
  works cleanly.
- Pure geometry in `scene/regions.rs` (no OS-specific code); all paths via
  `std::path` where files are touched (none are — regions are docs).

## 10. Scope — explicit exclusions (deferred, homed in `PLAN.md`)

These are **tracked at the roadmap level so they are not lost** (user directive):

1. **Polyanya / navmesh cost-layers** → **M10f**. M10g wires cost/impassable/arrest
   into the **grid** engine only. M10f's PLAN.md entry gains an explicit
   region-cost-layer line item.
2. **Per-actor / faction movement exemptions** (flying / incorporeal ignore
   terrain) → **Phase 2, vision/lighting/movement completion** (grouped with the
   other movement-type work — darkvision / tremorsense / height). Needs
   movement-type tags on actors that do not exist yet; all tokens are affected
   equally in v1.
3. **Mechanical / trigger effects on arrest** (damage, condition application,
   scripted triggers) → **Phase 2, "trigger regions"** (token enrichment already
   names them). M10g ships the region **primitive**; trigger regions build on it.
   Arrest in v1 only *stops* the token.

## 11. Testing

- `scene/regions.rs` unit tests: rasterization of each shape kind; overlap compose
  (precedence + MAX); footprint-intersect membership; fail-closed degenerate/
  over-cap shapes; `enabled == false` contributes nothing.
- Router tests: terrain weighting changes the chosen route + cost; impassable
  routes around; visible arrest truncates + sets `arrested`; **secret region is
  absent from a player's field** (route straight, no cost bump) but present in the
  GM's.
- `move_exec` tests: impassable stop-before, arrest stop-at (incl. final step),
  terrain cost accumulation; **authoritative field springs a secret arrest** a
  player was routed straight through.
- Parity test: router visible field ⊆ authoritative field (no player-shown route is
  region-wall-blocked at execute).
- Client: region-layer render only shows permitted regions; measure-tool budget
  reflects terrain cost + arrest truncation.

## 12. Execution

Per-checkpoint plan via `writing-plans` → SDD (per-task two-reviewer gate +
whole-branch buddy-check), matching the M10 cadence. Reviewed skill-update gate:
update `shadowcat-codebase-scene-rendering` (new region doc_type, `scene/regions.rs`
seam, two-field split, router/executor region wiring) and confirm via
`shadowcat-spec-reviewer` before merge.
