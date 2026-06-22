# M8d — Scene Entities + Interaction Tools: Design Spec

> Status: **DRAFT for review.** Refines the M8d slice of the M8 cross-cutting design
> (`2026-06-19-m8-ecs-scene-rendering-design.md` §8 decomposition + §11 M8d open
> items) into implementable decisions. **Authored autonomously while the user is
> away** (per the "finish M8" directive) against the already-approved M8 parent
> spec + the user-set [token-architecture] and [UI-packaging] directions; it stops
> at the brainstorming review gate. **§13 lists the decisions made autonomously that
> need confirming before implementation.**

M8d completes M8: it puts **tokens on the map** (rendered from documents, moved
optimistically, tweened), adds the **measurement / template / drawing** tools and
**pings**, and introduces the **canvas interaction/tool API** plus the first
first-party **`src/modules/*` feature module** (per the UI-packaging target).

## 1. Goal

Turn the M8c render foundation into an interactive table: place and move token
images, draw/measure/template on the scene, and ping locations — all through the
engine-owned canvas and the document/optimistic pipeline, with the tool UI shipped
as a standalone module.

## 2. Constraints inherited (cited inline)

- **#1/#3** Server-authoritative; token/drawing moves are ordinary document intents
  with optimistic-apply + rollback (M5/M6).
- **#3 ephemerals** Pings + in-progress measure/template/draw previews are
  client-local or lightweight transient broadcasts — never documents, never ECS.
- **#5** Tokens/drawings render from the Zod `DocumentStore` via the M8c reconciler;
  no client ECS.
- **#6** Server stays structural-only — token/drawing/template shapes live in the
  opaque `system` body; the client interprets them. The ping frame carries no
  semantic validation.
- **#7** Canvas is engine-owned; tools drive it through a **public interaction API**,
  they do not own pointer handling directly.
- **#10** All interaction is pointer-event-based (mouse/touch/pen); tool targets are
  touch-sized; drag/pinch unified.

## 3. Decomposition

Mirrors the render-vs-interact seam (like M8c's foundation-vs-API split):

- **M8d-1 — Token rendering** *(render-only; no UI)*: the token document model
  (§4); the reconciler renders `doc_type:"token"` children as sprites via a
  generalized `DisplayBackend` node API (§5); a **render ticker** tweens each
  sprite toward its document-authoritative transform (§6). Tokens are created/moved
  in tests + the e2e via direct intents (as M8c seeded scenes); no tool UI yet.
- **M8d-2 — Interaction + tools + pings** *(the interactive layer)*: the canvas
  **interaction/tool API** (§7); the **`scene-tools` module** under `src/modules/*`
  contributing the tool rail (§8); token place/select/move, **drawing** + **template**
  entities (§9), **measurement** (§10), and **pings** (§11, a new transient
  broadcast frame — the only server work in M8d).

Dependency order: **M8d-1 → M8d-2**. Both depend on M8a (scene-entity docs) + M8c
(render foundation + reconciler).

## 4. Token document model (M8d-1)

Per M8 §4.2: a token is a top-level `Document`, `doc_type:"token"`,
`parent_id` = the scene's id, engine fields in the opaque `system` body. Concrete
shape (client-owned; server structural-only):

```jsonc
// token.system
{
  "x": 0, "y": 0,          // scene-coordinate position of the token CENTER (confirmed §13)
  "w": 100, "h": 100,      // size in scene units
  "rotation": 0,           // degrees
  "visual": { "kind": "image", "asset": "<uuid>" }
}
```

- **`visual` is the forward-looking seam** ([token-architecture]): `kind:"image"`
  (a static asset UUID) is the only kind in M8d; `faces` / `animated` / `generated`
  are added later as new `kind`s **without reshaping the token** — the transform
  fields stay flat (stable), only `visual` grows.
- Token scene-data is kept **separable from actor-data** (M8 §7.1) — M8d carries no
  actor; a token is transform + visual. M10 attaches `actor_id` / embedded actor.
- The image resolves through the M8b `AssetResolver` (UUID → URL, re-resolve on
  `AssetChanged`) — reusing the M8c background path.

## 5. Token reconciler + generalized `DisplayBackend` node API (M8d-1)

The M8c-1 reconciler handles only the background (`setBackground`). M8d-1
generalizes the backend to a **node API** and adds a token handler:

```ts
// DisplayBackend (additions). A NodeId is the document id.
createToken(id: string, spec: TokenNodeSpec): void;   // sprite in the tokens layer
updateToken(id: string, spec: TokenNodeSpec): void;   // transform/visual change
destroyToken(id: string): void;
// TokenNodeSpec = { x, y, w, h, rotation, url }  (url resolved from visual.asset)
```

- The reconciler diffs `store.query("token")` by id (create/update/destroy), mapping
  each token doc → `TokenNodeSpec` (resolving `visual.asset` via `AssetResolver`).
  Same reactive subscribe/snapshot path as the background.
- **MockBackend** records token nodes (headless-testable reconciler diff: a created
  token doc yields `createToken`; a moved token yields `updateToken`; a deleted doc
  yields `destroyToken`). **PixiBackend** renders each as a `Container`(sprite) in
  the `tokens` layer — the Container-based "token visual" from [token-architecture]
  (so overlays/fx/animatable transform attach later).

## 6. Token movement: tween + render ticker (M8d-1)

- The document holds the **authoritative target** transform; the render **tweens**
  the sprite toward it (ephemeral, never persisted — #3/#5). A **PixiJS `Ticker`**
  in the RenderEngine lerps each token's current→target transform per frame. M8c
  was event-driven; M8d-1 introduces the ticker (the [token-architecture] "ticker
  arrives with motion").
- `updateToken` sets a token's **target**; the ticker animates toward it (short
  lerp, e.g. ~120 ms ease). A locally-dragged token (M8d-2) snaps to the pointer
  (no tween for the dragger); remote clients tween to the authoritative position.
- Headless-testable: the tween math (lerp, arrival) is pure; the ticker drive +
  Pixi sprite update are GL (Playwright).

## 7. Canvas interaction / tool API (M8d-2)

The engine owns pointer events; tools plug in through a public seam (the §6.2
render-layer API gains an interaction surface):

```ts
interface SceneTool {
  onPointerDown(p: Point, ev: PointerEvent): boolean; // scene coords; true = handled
  onPointerMove(p: Point, ev: PointerEvent): void;
  onPointerUp(p: Point, ev: PointerEvent): void;
}
// RenderEngine:
setActiveTool(tool: SceneTool | null): void;
```

- The engine routes a canvas pointer event to the active tool first (in **scene
  coordinates** via `camera.screenToScene`); if no tool handles it (or none is
  active), it falls back to **camera pan/zoom**. This replaces M8c-1's direct
  `wireCamera` with a tool-aware dispatcher (camera becomes the default/no-tool
  behavior).
- Tools get scene coords + the engine's **`grid`** (snap) and an **ephemeral
  overlay** API for previews (draw transient graphics into the `templates`/
  `overlays` layer without creating documents): `engine.previewOverlay(draw)` /
  clear. Persisted results (token create, drawing) are document intents the tool
  issues via the module's `ctx.client` (optimistic) — not through the engine.

## 8. `scene-tools` module — first `src/modules/*` package (M8d-2)

Per [UI-packaging-target], new in-game UI ships as its own module under
`src/modules/*` (not piled into `core-ui`). `scene-tools` is the **first such
package** and the seam-discipline proof:

- Contributes tool buttons into the **`shadowcat.surface:toolrail`** surface
  (provided by core-ui). Owns the **active-tool state** and the `SceneTool`
  implementations: **select/move** (pick + drag tokens → coalesced position
  intents), **place** (click → create a token doc from a chosen asset), **measure**
  (drag → distance via `Grid`), **template** (place a cone/circle/rect/line area),
  **draw** (freehand/shape → drawing doc), **ping** (click → broadcast).
- Communicates only through public seams (contributions, the render interaction API,
  `ctx.client` for intents, the ping transport) — **never imports `core-ui`** — so it
  validates the contract-only discipline before the M8.5 decomposition.
- It needs the `RenderEngine` interaction API + `AssetResolver` + the ping send — so
  `AppContext` exposes the engine's tool API (or a thin `scene` service). **Wiring
  resolved in §16** (thin `scene` service + `dispatchIntent` seam).

## 9. Drawing + template entities (M8d-2)

Persisted scene entities (documents, optimistic, reconciled into their layers):

```jsonc
// drawing.system   (doc_type:"drawing", parent_id = scene; drawings layer)
{ "shape": { "kind": "freehand|rect|ellipse|line|polygon", "points": [x,y,...] },
  "stroke": { "color": "#rrggbb", "width": 2 }, "fill": null }

// template.system  (doc_type:"template", parent_id = scene; templates layer)
{ "shape": { "kind": "circle|cone|rect|line", "x": 0, "y": 0, "size": 0, "direction": 0 },
  "color": "#rrggbb" }
```

- Reconciled like tokens (new `doc_type` handlers → backend graphics nodes). Geometry
  math (template shapes, freehand simplification) is pure/headless-testable on the
  `Grid`/scene coords.

## 10. Measurement (M8d-2)

- **Client-local ephemeral** (no document, no broadcast in M8d): drag from A→B →
  the engine ephemeral-overlay draws the path + a distance label computed via the
  `Grid` (square: Chebyshev/Euclidean per grid config; hex: axial distance). Cleared
  on release. (Transient-broadcast-to-others is deferred.)

## 11. Pings (M8d-2) — the only server work

A new **transient broadcast** frame, modeled on the M8b out-of-band `AssetChanged`
(no seq, not in the event log, not a document — #3):

```
Client → Server:  Ping { scene: <id>, x, y }
Server → world:   Ping { scene: <id>, x, y, user: <id> }   (broadcast_aux, no seq)
```

- The server relays it to the world room (membership-gated; structural-only — no
  validation of coordinates). The client renders a transient expanding-ring
  animation at (x,y) in the `pings`/overlays layer, then fades (~2 s). Rate-limited
  per the cross-cutting WS rate-limit norm.
- ts-rs wire types + the Zod `ServerMsg`/`ClientMsg` additions (like `scene_*`).

## 12. Testability

- **Headless (node):** reconciler token/drawing/template diffs vs `MockBackend`;
  tween lerp math; grid distance + template geometry; the tool pointer-routing
  (active-tool-vs-camera) against a mock; ping frame parse.
- **Playwright (GL):** a token doc renders + moves/tweens; place/drag via the tool
  rail; a ping renders + fades. (Token placement is now authorable, so the M8c-1
  deferred **background-render e2e** rides here too — `docs/TODO.md`.)

## 13. Decisions — **CONFIRMED (user, 2026-06-22)**

1. **Token `system` schema:** flat transform (`x,y,w,h,rotation`) + nested
   `visual:{kind,asset}`. **Position origin = CENTER** — `(x,y)` is the token's
   center (aligns with `Grid.snap()` returning cell centers + center rotation pivot).
2. **The canvas interaction/tool API** (`setActiveTool` + `SceneTool` in scene
   coords; camera as the no-tool fallback) — **approved as designed**.
3. **`scene-tools` as the first `src/modules/*` package** now (contract-only; never
   imports `core-ui`) — **approved**; realizes the M8.5 discipline early.
4. **Ping = a new out-of-band server frame** (`broadcast_aux`, no seq), mirroring
   `AssetChanged` — **approved**.
5. **Measurement is client-local only** (no broadcast) in M8d — **approved**.
6. **d-1 / d-2 split** (render vs interact) — **approved**.

## 15. Scene lifecycle (added + user-approved 2026-06-22)

Discovered mid-M8d-2: nothing creates a `scene` document in the running app (only test
helpers), so the place tool had no active scene to parent tokens to (see
`scene-lifecycle-gap`). Approved minimal resolution (full scene management — multiple
scenes, browser, switching, dimensions — remains M12):

- **Scene `system` schema** (client-owned; server structural-only #6):
  `{ grid: { kind: "square"|"hex", size: number }, background: <uuid> | null }`.
  (Dimensions deferred — the canvas pans freely; M9 fog may add bounds later.)
- **Default scene on world entry:** on entry, after initial resync, if no `scene` doc
  exists **and** the actor is a GM, the client creates one (optimistic intent,
  `doc_type:"scene"`, world scope, default `grid: square/100`, `background: null`).
  Idempotent guard (create only if none). The rare multi-GM-simultaneous-first-entry
  double-create race is accepted (cosmetic; M12 scene management dedupes).
- **Active scene** = the single `scene` doc (`store.query("scene")[0]`) in M8d; tokens/
  drawings/templates parent to its id. Multiple scenes + selection → M12.
- **Stage reads the grid from the active scene** (`system.grid`) instead of hardcoding
  square/100; fallback to square/100 when no scene yet. Also unblocks the deferred M8c
  background-render + M8d-1 token-render e2e (a scene can now be authored).

This lands as the **scene-prelude tasks of M8d-2** (below).

## 14b. Decomposition refinement (M8d-2 split)

M8d-2 is split for tractability (each independently shippable + buddy-checked):
- **M8d-2** — scene lifecycle (§15) + the interaction/tool API (§7) + the `scene-tools`
  module (§8) + token **place / select / move** (the core interactive loop).
- **M8d-3** — **drawing** + **template** entities (§9) + **measurement** (§10) + **pings**
  (§11, the only server work). Builds on the M8d-2 interaction API + module.

## 14. Out of scope / deferred

- M9 vision/walls (walls as entities arrive with M9); real fog.
- Token actor-linking (M10); multi-face/animated/generated visuals, fx, emotes
  ([token-architecture] — M10+; M8d ships static-image tokens + the `visual` seam).
- Wall/light/sound/note entity kinds (later milestones; the pattern is proven by
  token/drawing/template).
- Measurement/template **broadcast** to other players; multi-level maps; post-fx.
- The full M8.5 UI decomposition (M8d only adds `scene-tools` as a new module; it
  does not split `core-ui` or extract the entry package).

## 16. Interaction wiring — **resolved autonomously 2026-06-22** (the §8 confirm item)

> Resolves the open "exact wiring" item §8 flagged. Decided on the merits while the
> user is away (per the overnight "continue through M8/M9" directive), following the
> established `subscribeScene` thin-seam convention. **The whole UI contribution API is
> pre-release/unfrozen (PLAN M7), so this is reversible internal surface — flagged for
> morning review.** Two facts forced the decision: (a) the `RenderEngine` is created
> lazily inside `Stage.svelte`'s effect and is not reachable from a module; (b)
> `ctx.client` (`OptimisticClient`) only *predicts* — nothing transmits a module's
> intent over the WS (`scene-tools` is the first feature to write from a module).

Two thin function-seams are added (mirroring how `subscribeScene` is exposed), not a
direct engine handle:

1. **`scene: SceneInteraction`** on `AppContext` (and the stable owner lives on
   `WorldSession`, so it survives Stage remount). Forwards to a late-attached
   `SceneToolHost` (the engine); no-ops / identity when detached:
   ```ts
   interface Point { x: number; y: number }
   interface SceneTool {
     onPointerDown(p: Point, ev: PointerEvent): boolean; // scene coords; true = handled
     onPointerMove(p: Point, ev: PointerEvent): void;
     onPointerUp(p: Point, ev: PointerEvent): void;
   }
   interface SceneToolHost {                 // implemented by RenderEngine
     setActiveTool(tool: SceneTool | null): void;
     snap(p: Point): Point;                  // engine owns the active scene's Grid
     previewOverlay(draw: (o: OverlayDraw) => void): void; // ephemeral, overlays layer
     clearOverlay(): void;
   }
   interface SceneInteraction extends SceneToolHost {
     attach(host: SceneToolHost): () => void; // Stage calls on engine mount; returns detach
   }
   ```
   - The engine keeps DOM out of itself (testability invariant): `Stage.svelte`'s
     pointer listeners call new engine methods `dispatchPointerDown/Move/Up(screen, ev)`;
     the engine converts screen→scene (`camera.screenToScene`), routes to the active
     tool first, and falls back to camera pan/zoom when no tool handles it. This is the
     §7 "tool-aware dispatcher replaces direct `wireCamera`" — dispatch logic in the
     engine, DOM binding stays in Stage.
   - Ephemeral previews draw into the existing `overlays` core layer via a new backend
     `drawOverlay(segs/shape)/clearOverlay` method (mock records; Pixi draws a Graphics).

2. **`dispatchIntent(ops: WireOperation[]): void`** on `AppContext` (and used internally
   by `WorldSession` for scene auto-create). Generates one `intent_id`, calls
   `optimistic.applyIntent(id, ops)` **and** `ws.send({ type:"intent", intent_id:id, ops })`
   — the missing predict-and-send seam. `ctx.client` stays the read/predict view (§7's
   "issues via `ctx.client`" is realized as predict-via-client + send-via-dispatchIntent,
   one correlated id). GM-gating is advisory client-side; the server remains authoritative.

**Module placement:** `scene-tools` lands at `src/client/ui/src/modules/scene-tools/`,
co-located with the only existing module (`core-ui`), importing only shared `lib/*`
(`appContext`, `api`) — never `core-ui` internals (the contract-only discipline). M8.5
relocates both to standalone packages; M8d does not invent a new package/build now (§14).
