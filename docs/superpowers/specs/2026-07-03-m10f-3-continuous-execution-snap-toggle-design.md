# M10f-3 — Continuous execution + scene-level snap toggle (design)

Status: approved 2026-07-03. Checkpoint of the M10f continuous/navmesh movement
milestone (parent spec `2026-07-02-m10f-continuous-navmesh-movement-design.md`,
decomposition §12). Predecessor: M10f-2 (unified engine-agnostic executor, merged to
local `main` @ `3bec7c7`).

## 1. Goal

Give **continuous** (any-angle, gridless) scenes full server-authoritative gated
movement, and introduce a **scene-level snap toggle** as an independent authoring axis.

The checkpoint is **client-weighted**: M10f-2 already refactored `move_exec::execute_move`,
`move_stream::sample_path`, and the M2 per-recipient observer clip into a fully
**engine-agnostic** stack with **no `movementModel` branch anywhere** on the server move
path. The server already gates, executes, streams, and per-recipient-clips *any* polyline
(grid cell-centers or any-angle continuous vertices). The only thing preventing continuous
moves today is a **client-side refusal**. This checkpoint removes that refusal and adds a
snap toggle; net-new server engine code is **zero** (server work is verification + tests).

## 2. Scope — two composable parts

1. **Continuous execution wiring** — remove the client refusal so a navmesh route commits
   through the existing `moveRequest` → `execute_move` → `MoveStream` path.
2. **Scene-level snap toggle** — a new per-scene `snapToGrid` axis, enforced at one
   render-engine chokepoint so every tool inherits it, authored via a GM tool-rail toggle.

They compose: **snap-off + continuous** ⇒ free-form float placement + any-angle pathfound,
server-authoritative, streamed-vision execution. The two axes are otherwise independent
(a grid-stepped scene may have snap off; a continuous scene may have snap on).

## 3. Part 1 — Continuous execution

### 3.1 The one client change

`src/modules/scene-tools/src/controller.svelte.ts`, `commitRoute` (currently ~L422):

```ts
// REMOVE this early-return (the M10f-1 preview-only guard):
if (sceneDoc && resolveSceneSettings(sceneDoc, ctx.documents).movementModel === "continuous") {
  return;
}
```

With it gone, `commitRoute` proceeds identically for both engines:

- If `lastPreviewedPath` is cached (the M10f-1 navmesh preview already computed it for
  continuous scenes), it is sent verbatim via `moveRequest(scene.id, tokenId, path)`.
- Otherwise the fallback `ctx.pathfind(scene.id, start, [...waypoints, goal], fp)` runs;
  `SceneEcs::pathfind` already **dispatches on `movementModel`** (M10f-1) and returns the
  navmesh route for continuous scenes. Either way the committed polyline is the navmesh
  route.

No other client change is required for execution. The `start`, `waypoints`, and `goal`
fed into the route are float positions once snap is off (Part 2) — the free-form input the
continuous route needs.

### 3.2 Server — already engine-agnostic (no code change)

The full path — `handle_move_request` → `Room::execute_move` → `move_exec::execute_move`
(via `gate_walk`) → `move_stream::sample_path` → per-sample mover vision → the
`egress_loop` `MoveStream` per-recipient clip — has **no `movementModel` inspection**. It:

- accepts `path: Vec<(f64,f64)>` (any polyline) and gates it cell-by-cell over the dense
  `gate_walk` subdivision (M10f-2), so the **cell-sampled secrecy gate** applies to
  continuous routes with zero new secrecy code (parent §6.3);
- computes distance/duration Euclidean-ly (`sqrt(dx²+dy²)`), correct for any angle;
- samples any polyline and raycasts mover vision per sample;
- clips per observer against the recipient's own authoritative vision (M2 no-leak boundary).

The checkpoint therefore adds **tests only** on the server (§6).

## 4. Part 2 — Scene-level snap toggle

### 4.1 Data model (`src/client/core/src/scene-docs.ts`)

- Add `snapToGrid?: boolean` to `SceneSystem`. It rides the opaque scene `system` body
  exactly like `movementModel`/`bounds`: **no ts-rs type**, server-structural/client-owned
  (the server never reads it — it gates whatever float or grid-aligned position it
  receives).
- `resolveSceneSettings` resolves it **per-scene only** (no world-settings layer), with a
  **derived default**:

  ```
  snapToGrid: sys.snapToGrid ?? (resolvedMovementModel === "continuous" ? false : true)
  ```

  An explicit stored boolean overrides in either direction. Because `movementModel` is
  already resolved in the same function (world default < per-scene override), the default
  derives from the *resolved* model. This preserves parent §9's intent — a fresh continuous
  scene is free-form with zero GM action — while making snap a first-class independent axis
  (see §5, deviation).

### 4.2 The single chokepoint (`src/client/render/`)

The snap call chain is `tool → ctx.scene.snap → SceneInteractionBridge.snap →
RenderEngine.snap (engine.ts) → Grid.snap`. Enforce the toggle at `RenderEngine.snap`:

- `RenderEngine` gains a private `#snapEnabled = true`; `snap(p)` returns `p` (identity)
  when disabled, else `this.grid.snap(p)`.
- New host seam `setSnapEnabled(enabled: boolean)` on `SceneToolHost`
  (`render/src/types.ts`), forwarded through `SceneInteractionBridge` (`ui-kit`) — a no-op
  when the bridge is detached, mirroring the existing detached-host convention.

Every tool that calls `ctx.scene.snap` inherits the toggle automatically, including the
structural tools (wall/region/template/draw) — this is intentional ("applied globally to
all tools"). **Grid *rendering* is unaffected**: `setSnapEnabled` governs snapping only; a
snap-off scene may still display its reference grid (parent §9: grid display stays
orthogonal). The engine's grid-line drawing is a separate seam.

### 4.3 Stage wiring (`src/modules/stage/src/Stage.svelte`)

In the same reactive effect that already resolves and pushes grid size + `diagonalRule`,
resolve `snapToGrid` via `resolveSceneSettings` and push it via `setSnapEnabled`. The flow
is reactive and unidirectional: a toggle flip → scene-doc update → optimistic doc change →
effect re-resolves → `setSnapEnabled`.

### 4.4 GM tool-rail toggle (`src/modules/scene-tools/src/ToolRail.svelte`)

The tool-rail is already GM-gated (`{#if isGm}`). Add a persistent (non-tool-mode) toggle
button:

- reflects the **active scene's resolved `snapToGrid`** (source of truth: the resolved
  field, never local component state), rendered with `aria-pressed`;
- on click dispatches a scene-doc update
  `{op:"update", doc_id: sceneId, changes:[{path:"/system/snapToGrid", old, new: !current}]}`
  (optimistic; the server structurally accepts the opaque `system` write).

Only the GM authors it; every user (including players dragging their own tokens via
select-move) inherits the resolved value through the §4.2 chokepoint — so the toggle is
genuinely **scene-level and shared**, not per-client.

## 5. Deviation from parent spec §9 (recorded)

Parent §9 tied "no snap" to `movementModel` (continuous ⇒ no-snap, implicitly). This
checkpoint instead makes snap an **independent scene-level toggle** (user decision,
2026-07-03), which is strictly more expressive: any scene can be snap-on or snap-off
regardless of movement model. The **derived default** (§4.1) preserves §9's user-visible
intent — a fresh continuous scene is free-form out of the box — so no regression against
the parent spec's goal. Parent §9's mechanism is **superseded** by this toggle; the
`PLAN.md` M10f-3 entry records the supersession.

## 6. Testing

### Part 1 — continuous execution (server, net-new coverage)

- `Room::execute_move` on a **continuous** scene: an any-angle route executes and commits
  the stop atomically (mirror the existing grid `execute_move_commits_stop...` test).
- A continuous route whose subdivided cells leave the mover's `visible` set **truncates**
  at the last visible sample (`Ok` with a partial `stop`, matching the wall/region gates'
  existing mechanism — `Forbidden` is reserved for structural/degenerate input, never a
  per-cell gate stop) — proves the cell-sampled gate applies to any-angle paths, fail-closed.
- `MoveStream` samples an any-angle path correctly (arc-length monotonic `t_ms`; exact
  first/last vertex retained).
- The M2 observer no-leak assertion holds over an any-angle path (reuse the suite; a
  wholly-occluded any-angle move is suppressed, not sent empty).

### Part 1 — client

- Rewrite `measure-tool.test.ts` "commitRoute does nothing in a continuous-movement-model
  scene" — it asserts the removed refusal. It now asserts commit **fires** in a continuous
  scene (sends the navmesh `lastPreviewedPath` / dispatches a continuous `pathfind` then
  `moveRequest`). ([[tests-yield-to-correct-code]] — the code is now correct to commit.)

### Part 2 — snap toggle

- `resolveSceneSettings`: derived default (grid-stepped ⇒ true, continuous ⇒ false when
  unset) + explicit-override cases (both directions).
- `RenderEngine.snap` returns identity when `setSnapEnabled(false)`, snaps when true;
  `SceneInteractionBridge.setSnapEnabled` no-ops when detached.
- `ToolRail`: the toggle reflects the resolved field and, on click, dispatches the expected
  `/system/snapToGrid` update; button state follows the resolved value, not local state.

## 7. Out of scope (homes unchanged)

- **Regions on the navmesh** (terrain cost-layer / impassable obstacle / arrest truncation
  on continuous routes) → **M10f-4**.
- **Edge-projected environment light** → M12.
- **Per-actor / faction movement exemptions**, **trigger regions** → Phase 2.
- **Full scene-management UI** (resize handles, background-driven bounds, scene switching)
  → M12. Snap authoring here is the tool-rail toggle only.

## 8. Files touched (summary)

- `src/modules/scene-tools/src/controller.svelte.ts` — remove the `commitRoute` continuous
  refusal.
- `src/client/core/src/scene-docs.ts` — `SceneSystem.snapToGrid` + `resolveSceneSettings`
  derived default.
- `src/client/render/src/types.ts` — `SceneToolHost.setSnapEnabled`.
- `src/client/render/src/engine.ts` — `#snapEnabled` + gated `snap`.
- `src/client/ui-kit/src/sceneInteraction.ts` — bridge forward + detached no-op.
- `src/modules/stage/src/Stage.svelte` — resolve + push `snapToGrid`.
- `src/modules/scene-tools/src/ToolRail.svelte` — GM snap toggle button.
- Tests: `src/server/src/ws/room.rs`, `src/server/src/ws/conn.rs` (continuous +
  no-leak coverage), `src/modules/scene-tools/src/measure-tool.test.ts` (rewrite),
  new snap/resolve/engine/toolrail cases.

## 9. Cross-platform / bloat

No new dependencies (M10f-1 already pulled `polyanya`/`geo`/`glam`; M10f-2 added none;
M10f-3 adds none). Pure client seams + Rust tests. `snapToGrid` is opaque JSON — no path
handling, no `#[cfg]` code. Three-OS CI matrix unchanged.

## 10. Reviewed skill-update gate

On completion, update `shadowcat-codebase-scene-rendering` (the `snap` chokepoint + the
`snapToGrid` scene axis + continuous execution now wired end-to-end) and confirm accurate
via `shadowcat-spec-reviewer`, per CLAUDE.md's reviewed skill-update gate.
