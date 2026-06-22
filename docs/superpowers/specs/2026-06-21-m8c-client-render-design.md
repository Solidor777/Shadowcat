# M8c — Client Render Foundation + Render-Layer API + Vision-Mask Spike: Design Spec

> Status: **DRAFT for review.** This refines the M8c slice of the M8 cross-cutting
> architecture pass (`2026-06-19-m8-ecs-scene-rendering-design.md`, §6 render-layer
> API, §8 decomposition, §10 token re-audit, §11 M8c open items) into the
> load-bearing decisions for implementation. Each sub-milestone (**M8c-1**,
> **M8c-2**) gets its own `writing-plans` pass against the decisions fixed here.
>
> M8c is the **first real pixels**: it turns the M7 stage placeholder into a live
> PixiJS canvas with an ordered layer stack, a document-driven scene reconciler, a
> pan/zoom camera, a square/hex grid, and the engine-owned render-layer public API
> — proven end-to-end by an **identity** vision-mask spike that exercises the M9
> path with zero server change.

## 1. Goal

Replace `StagePlaceholder.svelte` (mounted on the `shadowcat.surface:stage`
contract) with a PixiJS v8 canvas host and an engine-owned render API, and prove
the M8a `SceneDerived` → mask-slot → fog-compositor path with an identity mask.
M8c renders the **scene background + grid**; placing tokens/walls/tiles/etc. is
**M8d** (those scene-entity documents do not exist until M8d creates them), so the
reconciler ships its pattern proven on the background and is extended per-`doc_type`
in M8d.

## 2. Constraints inherited (cited inline below)

From `ARCHITECTURE.md` / the M8 parent spec:

- **#5** Documents are the source of truth; the client renders directly from the
  Zod `DocumentStore` — **no client-side ECS** (PixiJS's container tree *is* the
  client scene graph).
- **#6** Server is relay + persistence + structural validation only — it has no
  stake in client z-order or display objects.
- **#7** The public module API is framework-neutral; the UI is
  extendable/replaceable, **but the PixiJS canvas host is engine-owned** — modules
  draw through the render-layer API, they cannot replace the renderer.
- **#10** Cross-platform / mobile from day one — **pointer-events from the start**
  (unified mouse/touch/pen, pinch-zoom, drag-pan), HiDPI-correct, responsive
  canvas.
- **#2 / D-V1** Ordered, recoverable realtime; derived state carries a
  `computed_at_seq` watermark; the vision dispatch payload is **polygon geometry,
  not a rasterized mask** (resolution-independent).

## 3. Decisions resolved (M8c open items §11 + decomposition)

| # | Decision | Choice |
|---|---|---|
| 1 | Where the PixiJS engine lives + the dependency | **New `@shadowcat/render` workspace package** depending on `pixi.js ^8`; the Svelte `Stage.svelte` is a thin host. (§4) |
| 2 | Render-layer registration: M7b server-mirrored contracts vs client-only | **Client-only, engine-owned `LayerRegistry`** in `@shadowcat/render`. The Pixi host itself still mounts via the existing **server-mirrored `shadowcat.surface:stage` contract**; only layer registration *within* the host is client-only. (§5) |
| 3 | The mask/render-target compositor's concrete API | **`Compositor` over a viewport render target + mask-slot layer**, fed `VisibilityInput` (D-V1 polygon geometry; empty = identity). Engine-owned fog shader is M9. (§7) |
| 4 | M8c decomposition | **M8c-1 foundation + M8c-2 API/spike** (mirrors M6c/M8b). (§9) |
| 5 | §10 token re-audit | Lands in **M8c-2** alongside the real rendered canvas chrome. (§8) |

## 4. Package & host architecture (decision 1)

### 4.1 `@shadowcat/render` (new package)

- Location `src/client/render/` (auto-included by the `src/client/*` workspace
  glob); `package.json` mirrors `@shadowcat/core` (`"type": "module"`,
  `"main": "src/index.ts"`, `"private": true`, `typecheck`/`test` scripts).
- **Dependencies:** `pixi.js ^8`, `@shadowcat/core` (`workspace:*`, for
  `DocumentStore`/`AssetResolver` types). **No Svelte, no protocol/transport.**
- **Exports** the engine-owned, framework-neutral render API: `RenderEngine`,
  `LayerRegistry`, `Camera`, `Grid`, `Compositor`, and the supporting value types
  (`Polygon`, `VisibilityInput`, layer ids). This is the §6.2 "full API now"
  surface (layer contracts + camera + grid + mask/compositor), **0.x / unfrozen**
  like M7 — hardens through internal use before any freeze.
- The **shader-filter registration** path ships as a **typed extension seam with
  no consumer** (§6.2): the engine fog shader is engine-owned (M9), the first real
  module consumer is Phase-3 VFX. Designing it against a real consumer is deferred;
  the seam only reserves the shape.

### 4.2 `RenderEngine` — the headless-orchestrated core

`RenderEngine` owns the lifecycle and wires the model pieces to a **display
backend** (§6 testability). It is constructed with a `DocumentStore`, an
`AssetResolver`, and a display backend; it does **not** itself import Svelte. The
PixiJS `Application` is created/destroyed by the host (§4.3) and handed in, so the
engine's logic is exercisable against a mock backend in jsdom.

### 4.3 `Stage.svelte` — the thin host (decision 1)

A new `core-ui` panel replacing `StagePlaceholder` on `shadowcat.surface:stage`
(the `coreUi` `provides` entry and the `contribute(...)` call are updated; the
contract string and its `singleton` cardinality are unchanged — no server/topology
change). Responsibilities, all in `$effect` with a returned cleanup:

- **Mount:** `await new Application().init({ resizeTo / canvas, antialias,
  resolution: devicePixelRatio, autoDensity: true, ... })` (PixiJS v8 init is
  **async**); attach `app.canvas` to a container `<div>`; construct the
  `RenderEngine` over the app + `AppContext` `store` + `assets`.
- **Resize:** a `ResizeObserver` on the container drives `app.renderer.resize(...)`
  and notifies the camera/compositor of the new viewport (the render target is
  viewport-sized).
- **HiDPI:** `resolution: devicePixelRatio` + `autoDensity` so the canvas is
  crisp on retina/mobile; the camera works in CSS/scene units, not device pixels.
- **Teardown ($effect cleanup):** `engine.destroy()` then `app.destroy(...)`
  (release GL context + textures), `observer.disconnect()`, remove listeners. This
  is the M7c reconnect/teardown discipline applied to GL resources; the async init
  must guard against a teardown that races before `init()` resolves.

## 5. Layer stack + scene reconciler (decision 2)

### 5.1 `LayerRegistry` (client-only, engine-owned)

An ordered named stack — the canvas analog of M7's `ui.surfaces`, structurally
like `ContributionRegistry` but canvas-native and **client-only** (the server has
no stake — #6, #7). The **fixed core z-order** (§6.1):

```
background → grid → tiles → drawings → walls → tokens → templates → [mask] → pings/overlays
```

Each named layer is a PixiJS `Container` parented under the camera root (the mask
slot is the `Compositor`'s, §7). Modules may register **additional** named layers
(0.x/unfrozen) with an order relative to core layers; core layer ids are reserved.
Registration returns a dispose that removes exactly that layer (module-unload
teardown, mirroring `ContributionRegistry.contribute`/`removeModule`).

### 5.2 Scene-graph reconciler

Subscribes to the `DocumentStore` (the same `subscribe`/`snapshot` reactivity the
`<Surface>` adapter uses) and maps each scene-entity `doc_type` → display objects
in its layer, reconciling create/update/destroy as document Events arrive (#5). It
keys display objects by document id and diffs against the prior snapshot.

- **M8c scope:** the reconciler proves the pattern on the **scene background** —
  the Scene document's `system.background` asset UUID → a background sprite
  resolved through `AssetResolver.url(uuid)` and re-resolved on `AssetChanged`
  (M8b path). Grid is engine-drawn (§6), not document-reconciled.
- **M8d scope:** per-`doc_type` reconcilers for token/wall/tile/region/light/
  drawing/template — adding a kind is a new reconciler entry, no new machinery.
- The reconciler interface is written so its diff logic is testable against a mock
  display backend (§6) without real Pixi.

## 6. Camera & grid

- **`Camera`** — pan/zoom as a transform on the root container. **Pointer-events
  from the start** (#10): drag-pan, wheel-zoom, two-pointer pinch-zoom, all via
  unified `pointer*` events (mouse/touch/pen). Public API: `pan`, `zoomAt`, and
  **`screenToScene` / `sceneToScreen`** transforms (the basis for hit-testing,
  snapping, and the compositor's viewport mapping). Camera math is pure and
  headless-testable.
- **`Grid`** — engine-owned **square + hex** model with coordinate math:
  `sceneToCell` / `cellToScene`, `snap(point)`, cell traversal. Drawn into the
  `grid` layer; shared later by M8d snapping + measurement/templates. Coordinate
  math is pure and headless-testable (the hard correctness surface — hex axial/
  offset math especially).

## 7. SceneDerived client plumbing + Compositor API (decision 3)

### 7.1 Client `SceneDerived` subscription (missing half of M8a)

M8a defined the wire frames (`scene_subscribe`/`scene_unsubscribe` →
`scene_derived`/`scene_error`, with `channel` + `computed_at_seq` + opaque
`payload`) but **`WsClient` has no scene-subscription method**. M8c-2 adds it,
modeled on the existing `subscribeSearch`:

```ts
interface SceneSubscription { unsubscribe(): void; }
// On WsClient:
subscribeScene(
  channel: string,
  onUpdate: (frame: { payload: unknown; computedAtSeq: number }) => void,
): Promise<SceneSubscription>;
```

Same lifecycle as `subscribeSearch`: correlated by `request_id`, resolves on the
first frame, fires `onUpdate` for each push, dropped on disconnect (the caller
re-subscribes after reconnect), `scene_error` rejects/drops it. `WorldSession`
owns the subscription and exposes it through `AppContext` (alongside `store` /
`assets`) so the render engine can consume it.

**Watermark (#2):** `onUpdate` carries `computedAtSeq`; the consumer applies a
derived frame only once `store.appliedSeq >= computedAtSeq`, so a mask never
precedes the document events it derives from. In M8's identity mode this is a
correctness invariant carried through the plumbing even though the identity payload
is trivial.

### 7.2 `Compositor` API (the §6.2 hard surface, §6.3-driven)

The compositor owns the **mask slot** layer and a **viewport-sized render target**;
it composites the three fog states into one overlay above the fog-affected layers.

```ts
/** D-V1: resolution-independent polygon geometry in scene coordinates. */
type Polygon = { points: number[] }; // [x0,y0,x1,y1,...]
interface VisibilityInput {
  /** Live "visible" region. Empty ⇒ identity (everything visible). */
  visible: Polygon[];
  /** Persisted "explored" region (M9 D-V2); unused in M8. */
  explored?: Polygon[];
}
interface Compositor {
  /** Fed from a SceneDerived payload; recomputes the overlay. */
  setVisibility(v: VisibilityInput): void;
  /** Viewport changed (resize/camera) — re-sizes/re-maps the render target. */
  resize(width: number, height: number): void;
}
```

- **States (§6.3):** unexplored = black default, explored-not-visible = dimmed,
  visible = clear. The **fog shader is engine-owned and M9** — M8 ships a
  placeholder overlay (no shader; identity ⇒ fully transparent), proving render-
  target ownership + mask composition across layers while they are cheap to change.
- **M9 swap-in (zero structural change):** a real vision channel emits polygon
  `payload`; the client maps it to `VisibilityInput.visible`, the engine fog shader
  replaces the placeholder, `explored` activates (D-V2). No API change.

### 7.3 Vision-mask spike (M8 = identity, **no server change**)

End-to-end wiring that is the M8c-2 acceptance target:

1. The engine `subscribeScene("identity", …)` — M8a's existing debug channel
   (`{ entity_count }` payload).
2. Each frame → `compositor.setVisibility({ visible: [] })` (identity: payload is
   opaque to the mask; emptiness ⇒ full visibility), gated on the
   `computedAtSeq` watermark.
3. The compositor renders the identity overlay (transparent) into the mask slot.

This proves *SceneDerived → mask slot → compositor* over the real WS server with no
new server channel; M9 supplies the vision channel + shader later.

## 8. Token re-audit (§10, decision 5 — M8c-2)

When the first themed canvas chrome lands (M8c-2: stage background color, grid line
color, fog dim-level placeholder), re-audit the M7d 3-tier SCSS token set against
real rendered output. Add the missing caption/`--text-sm` token noted in
`POST_WORK_FINDINGS.md` if canvas chrome needs it. Recorded against the M7d token
system, **not** treated as a new theme. (PixiJS draws with numeric colors, so the
audit also defines how engine-drawn chrome reads token values — resolved CSS custom
properties sampled at host mount and passed to the engine, re-read on theme change.)

## 9. Decomposition (decision 4)

### M8c-1 — Render foundation (*first pixels*)
- `@shadowcat/render` package scaffold (package.json/tsconfig/vitest, workspace
  wiring), `pixi.js ^8` dependency.
- `Stage.svelte` host on `shadowcat.surface:stage` (async init, attach, resize,
  HiDPI, `$effect` teardown) replacing `StagePlaceholder`; `RenderEngine` lifecycle.
- `LayerRegistry` with the fixed core z-order; mock-backend headless tests.
- Scene reconciler proven on the **background** (Scene `system.background` →
  sprite via `AssetResolver`, re-resolve on `AssetChanged`).
- `Camera` (pan/zoom, pointer/touch/pinch, screen↔scene) + `Grid` (square/hex +
  coordinate math), both headless-tested.
- Playwright smoke: canvas mounts, background renders, pan/zoom works, teardown
  on leave-world releases GL.

### M8c-2 — Render-layer API + vision-mask spike
- `WsClient.subscribeScene` + `WorldSession`/`AppContext` exposure
  (watermark-gated).
- `Compositor` API (mask slot + viewport render target + identity overlay).
- Identity vision-mask spike end-to-end (§7.3).
- Module-facing API formalization: `LayerRegistry`/`Camera`/`Grid`/`Compositor`
  public surface + the typed **shader-filter extension seam** (no consumer).
- §10 token re-audit for canvas chrome.
- Playwright smoke over the spike (subscribe → identity overlay present).

Dependency order: **M8c-1 → M8c-2** (the spike needs the foundation). Both depend
on M8a (`SceneDerived`) + M8b (assets), which are merged.

## 10. Testability strategy (design constraint)

PixiJS WebGL cannot run in vitest/jsdom. `@shadowcat/render` therefore separates a
**headless-testable model** from a **thin Pixi display backend**:

- **Unit-tested headless:** `LayerRegistry` ordering/dispose, `Grid` coordinate
  math (square + hex), `Camera` transform math, reconciler diff against a mock
  display backend, compositor identity logic, the `subscribeScene` correlation
  lifecycle (already pattern-proven by the search subscription tests).
- **Playwright (real browser GL):** canvas mount, background render, camera
  interaction, teardown, and the identity spike — as M7c-2 / M8b-2 do.

The display backend is a narrow interface (create/parent/destroy containers,
sprites, graphics, render targets) so the model never imports Pixi types directly
and the mock backend is small.

## 11. Out of scope / deferred

- **All M9 vision:** raycasting, real fog shader, persistent explored mask (D-V2),
  GM vision mode (D-V3). M8c ships only the identity seam + placeholder overlay.
- **Module-facing shader-filter registration** (Phase-3 VFX) — typed seam only.
- **Token/wall/tile/region/light/drawing/template reconcilers + placement +
  interaction tools + pings** — **M8d**.
- **Layer-API freeze** — stays 0.x/unfrozen until N internal systems exercise it.
- Multi-level maps, portals, post-processing (PLAN M8 exclusions).

## 12. Open items for the c-1 / c-2 plans

- **M8c-1:** PixiJS v8 vitest setup (does any render-model test need a canvas shim,
  or is the model fully Pixi-free?); the display-backend interface boundary
  (exact methods); how engine-drawn chrome reads resolved CSS token values at mount;
  the async-init-vs-teardown race guard; whether `RenderEngine` subscribes to the
  store directly or the reconciler does.
- **M8c-2:** the `subscribeScene` reconnect/re-subscribe policy (search drops on
  disconnect — does the scene subscription auto-resubscribe in `WorldSession`?);
  the compositor render-target sizing on HiDPI (device vs scene pixels); the exact
  shader-filter seam type; the identity-spike Playwright assertion (how to observe
  the overlay without a real shader).
