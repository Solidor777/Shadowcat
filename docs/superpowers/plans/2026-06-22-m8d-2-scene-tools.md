# M8d-2 — Scene Lifecycle + Tool API + scene-tools + Place/Select/Move: Plan

> **For agentic workers:** REQUIRED SUB-SKILL: execute with **`mainline-plan-execution`**
> (inline enumerative per-task spec-compliance check + ONE dispatched final branch review;
> buddy-check that review). NOT subagent-driven / executing-plans. Steps use checkbox
> (`- [ ]`) syntax.

**Goal:** Turn the M8c/M8d-1 render foundation into an interactive table: a default scene
exists on GM entry, the engine exposes a tool API, a new `scene-tools` module contributes
the tool rail, and the GM can **place / select / move** token images — all through documents
+ the optimistic pipeline.

**Spec:** [`../specs/2026-06-22-m8d-scene-entities-tools-design.md`](../specs/2026-06-22-m8d-scene-entities-tools-design.md)
(§7 tool API, §8 module, §15 scene lifecycle, **§16 interaction wiring** — the autonomously
resolved confirm item). M8d-3 (drawing/template/measure/pings) is a separate plan.

**Architecture (§16):** Two thin function-seams added to `AppContext`, mirroring the existing
`subscribeScene` convention — no direct engine handle leaks to modules:
1. **`scene: SceneInteraction`** — a stable forwarder (owned by `WorldSession`) to a
   late-`attach`ed `SceneToolHost` (the `RenderEngine`). Tools set the active tool / snap /
   mark-dragging through it; no-ops when no engine is attached.
2. **`dispatchIntent(ops)`** — the missing predict-and-send seam: one `intent_id`,
   `optimistic.applyIntent(id, ops)` **and** `ws.send({type:"intent", …})`.

The engine keeps DOM out of itself (testability invariant): `Stage.svelte` pointer listeners
call `engine.dispatchPointer{Down,Move,Up}(screenPoint, ev)`; the engine converts screen→scene,
routes to the active `SceneTool` first, and falls back to camera pan/zoom (the §7 tool-aware
dispatcher replacing direct `wireCamera`).

**Tech stack:** TypeScript (strict), `@shadowcat/{core,render,ui}`, Svelte 5 runes, Vitest
(node + jsdom), Playwright, pnpm. Server: **untouched** (M8d-2 is client-only; the only M8d
server work — pings — is M8d-3).

## Global constraints

- **#1/#3:** token place/move are ordinary document intents (optimistic-apply + rollback). No
  new server logic.
- **#5/#6:** tokens/scenes render from the Zod store via reconcilers; scene/token `system`
  shapes are client-owned; server stays structural-only.
- **#7:** canvas engine-owned; tools drive it through the public seam, never owning pointer
  handling directly.
- **#10:** pointer-event-based (mouse/touch/pen); tool targets touch-sized.
- **Testability invariant:** only `pixi-backend.ts` imports `pixi.js`
  (`grep -rn "pixi.js" src/client/render/src` → only that file).
- **No raw `console.log`; commit per task; do NOT push** (push is the **M8-milestone** gate —
  after M8d-3 completes M8).

---

### Task 1: `parent_id` on the client wire model

The server `Document` has `parent_id: string | null` (M8a scene-entity link); the client
`WireDocument`/`DocumentSchema` omit it, so Zod **strips** it from every inbound `create`/echo
— scene entities lose their scene link over the wire. Add it.

**Files:**
- Modify: `src/client/core/src/wire.ts` (`WireDocument` type + `DocumentSchema`)
- Test: `src/client/core/src/wire.test.ts` (round-trip assertion)

**Interfaces:** `WireDocument` gains `parent_id: string | null`; `DocumentSchema` gains
`parent_id: z.string().nullable()`.

- [ ] **Step 1 (test):** add to `wire.test.ts` a case parsing a `welcome`-free `event` frame
  whose `create.doc` carries `parent_id: "scene-1"`, asserting the parsed command's doc keeps
  `parent_id` (today it is stripped → fails). Also assert a top-level doc parses `parent_id: null`.
- [ ] **Step 2:** run → fail.
- [ ] **Step 3:** add `parent_id: string | null;` to the `WireDocument` type (after `embedded`)
  and `parent_id: z.string().nullable(),` to the `DocumentSchema` object (the ts-rs drift guard
  in `wire.test.ts` may also need the field — keep it in sync).
- [ ] **Step 4:** run → pass. Confirm `applyOperation`/`store` need no change (they store the
  whole doc object; `parent_id` rides along).
- [ ] **Step 5 (commit):** `feat(m8d-2): round-trip parent_id on the client wire document`

---

### Task 2: scene/token system types + pure doc builders

A shared, framework-neutral home for the scene/token `system` shapes (§4, §15) + builders both
`WorldSession` (scene auto-create) and `scene-tools` (token create) use. Pure → unit-tested.

**Files:**
- Create: `src/client/core/src/scene-docs.ts`
- Modify: `src/client/core/src/index.ts` (export the types + builders)
- Test: `src/client/core/src/scene-docs.test.ts`

**Interfaces (produces):**
```ts
export interface SceneSystem { grid: { kind: "square" | "hex"; size: number }; background: string | null }
export interface TokenSystem { x: number; y: number; w: number; h: number; rotation: number;
                               visual: { kind: "image"; asset: string } }
// Builders return a fully-formed WireDocument (default permissions: visible-to-all `observer`;
// id = crypto.randomUUID() unless passed; world scope; correct doc_type + parent_id).
export function buildSceneDoc(worldId: string, system?: Partial<SceneSystem>, id?: string): WireDocument
export function buildTokenDoc(worldId: string, sceneId: string, system: TokenSystem, id?: string): WireDocument
```
- `buildSceneDoc` defaults: `grid: { kind:"square", size:100 }`, `background:null`,
  `parent_id:null`, `doc_type:"scene"`.
- `buildTokenDoc`: `parent_id = sceneId`, `doc_type:"token"`, `system` as given.
- Both: `scope:{kind:"world",world_id}`, `schema_version:1`, `source:null`, `owner:null`,
  `permissions:{default:"observer",users:{},property_overrides:{},capabilities:{by_role:{},by_user:{}}}`,
  `embedded:{}`, `created_at:Date.now()`, `updated_at:Date.now()`.

- [ ] **Step 1 (test):** `buildSceneDoc("w1")` → `doc_type:"scene"`, `parent_id:null`,
  `system.grid={kind:"square",size:100}`, world scope, a uuid id, an overridable id; a partial
  `{grid:{kind:"hex",size:50}}` overrides. `buildTokenDoc("w1","s1",{…})` → `doc_type:"token"`,
  `parent_id:"s1"`, system preserved.
- [ ] **Step 2:** fail. **Step 3:** implement `scene-docs.ts`. **Step 4:** export from `index.ts`.
  **Step 5:** pass.
- [ ] **Step 6 (commit):** `feat(m8d-2): scene/token system types + pure doc builders`

---

### Task 3: `dispatchIntent` predict-and-send seam

`OptimisticClient.applyIntent` only predicts; nothing transmits a module's intent. Add the
seam on `WorldSession` and expose it on `AppContext`.

**Files:**
- Modify: `src/client/ui/src/lib/worldSession.svelte.ts` (add `dispatchIntent`)
- Modify: `src/client/ui/src/lib/appContext.ts` (`dispatchIntent` field)
- Modify: the AppContext construction site (populate it) — **locate** via
  `grep -rn "setAppContext(" src/client/ui/src` (likely a `WorldView`/session view).
- Modify: `src/client/ui/src/lib/__fixtures__/appContextTest.ts` (+ default no-op)
- Test: `src/client/ui/src/lib/worldSession.test.ts` (or the existing session test file)

**Interfaces:**
```ts
// WorldSession:
dispatchIntent(ops: WireOperation[]): void  // genid → optimistic.applyIntent(id, ops) + ws.send({type:"intent",intent_id:id,ops})
// AppContext:
dispatchIntent: (ops: WireOperation[]) => void
```
- `dispatchIntent` reads from `this.#optimistic` (the same `ctx.client` view) so a caller sees
  its own prediction immediately; the `ws.send` is a no-op when disconnected (the send path
  already guards `transport?`).

- [ ] **Step 1 (test):** a `WorldSession` over a mock `connect` (mirror the existing session
  test harness): after `enter`, `dispatchIntent([create buildTokenDoc(...)])` → (a) the doc is
  visible in `session`'s optimistic view (assert via a module `ctx.client.get` or expose a test
  read), and (b) the mock transport received one `{type:"intent", intent_id, ops}` frame whose
  ops match. Use the existing mock-connect pattern (capture sent frames).
- [ ] **Step 2:** fail. **Step 3:** implement on `WorldSession`; add to `AppContext` + populate at
  the construction site + the test fixture default. **Step 4:** pass; `pnpm --filter @shadowcat/ui typecheck`.
- [ ] **Step 5 (commit):** `feat(m8d-2): dispatchIntent predict-and-send seam on WorldSession + AppContext`

---

### Task 4: default scene auto-create on GM world entry (§15)

In `#onWelcome`, after role is set + bootstrap: if the actor is a GM and no `scene` exists in
the optimistic view, dispatch a default-scene create. Idempotent (guards the reconnect re-fire
and a confirmed scene from another GM).

**Files:**
- Modify: `src/client/ui/src/lib/worldSession.svelte.ts`
- Test: same session test file.

**Logic:**
```ts
// inside #onWelcome, after reconcileTopology / scene-sub re-establish:
if (this.role === "gm" && this.#optimistic.query("scene").length === 0 && this.world) {
  this.dispatchIntent([{ op: "create", doc: buildSceneDoc(this.world) }]);
}
```
- Query the **optimistic view** (includes pending) so a second Welcome before the echo does not
  double-create. The rare multi-GM simultaneous-first-entry double-create is accepted (§15; M12
  dedupes).

- [ ] **Step 1 (test):** GM `Welcome`, no scenes → exactly one scene-create intent sent + a
  `scene` doc in the view. Player/spectator `Welcome` → none. A second `Welcome` (reconnect)
  after the scene is present → no new create. (Drive via the mock connect emitting Welcome
  frames; assert on captured intent frames.)
- [ ] **Step 2:** fail. **Step 3:** implement. **Step 4:** pass.
- [ ] **Step 5 (commit):** `feat(m8d-2): auto-create a default scene on GM world entry`

---

### Task 5: engine interaction API — `setActiveTool` + pointer dispatch + `snap` + `setGrid` + dragging

The engine gains the §7/§16 tool surface (the `SceneToolHost`), screen→scene pointer dispatch
with a tool-vs-camera fallback, grid snap, runtime grid swap (scene-driven), and a
dragging-token snap hint. **No DOM** in the engine.

**Files:**
- Modify: `src/client/render/src/types.ts` (`SceneTool`, `SceneToolHost` interfaces; `Point` exists)
- Modify: `src/client/render/src/engine.ts`
- Modify: `src/client/render/src/token-view.ts` (dragging snap)
- Modify: `src/client/render/src/index.ts` (export `SceneTool`, `SceneToolHost`)
- Test: `src/client/render/src/engine.test.ts`, `src/client/render/src/token-view.test.ts`

**Interfaces (produces):**
```ts
export interface SceneTool {
  onPointerDown(p: Point, ev: PointerEvent): boolean; // scene coords; true = handled
  onPointerMove(p: Point, ev: PointerEvent): void;
  onPointerUp(p: Point, ev: PointerEvent): void;
}
export interface SceneToolHost {
  setActiveTool(tool: SceneTool | null): void;
  snap(p: Point): Point;
  setDraggingToken(id: string | null): void;
}
// RenderEngine (additions), structurally implements SceneToolHost:
setActiveTool(tool: SceneTool | null): void
snap(p: Point): Point                                  // delegates to the active Grid
setDraggingToken(id: string | null): void              // forwards to TokenView
setGrid(spec: GridSpec): void                          // rebuild Grid + redrawGrid
dispatchPointerDown(screen: Point, ev: PointerEvent): void
dispatchPointerMove(screen: Point, ev: PointerEvent): void
dispatchPointerUp(screen: Point, ev: PointerEvent): void
```
- **Dispatch:** `dispatchPointerDown` → `p = camera.screenToScene(screen)`; if an active tool's
  `onPointerDown(p,ev)` returns `true`, mark "tool owns this gesture" and stop. Else begin a
  camera pan (record `screen`). `dispatchPointerMove`: if tool owns the gesture, forward
  `onPointerMove(scene,ev)`; else if panning, `camera.panBy(dxScreen,dyScreen)` + `applyCamera()`.
  `dispatchPointerUp`: forward `onPointerUp` / end pan. (Wheel zoom stays directly on the camera
  in Stage — tools don't intercept zoom in M8d-2.)
- **`setGrid`:** replace `this.grid` (it is currently `readonly`/constructed once → make it
  reassignable) and `redrawGrid()`. `snap` uses the current grid.
- **TokenView dragging:** add `setDragging(id: string | null)`; in `reconcile`, when a token's id
  === the dragging id, **snap** the animator (set current=target) instead of tweening, so the
  local dragger shows no tween lag while remote moves still tween.

- [ ] **Step 1 (tests):**
  - engine: active tool receives a **scene-coord** pointerdown (construct engine with a known
    camera transform; assert the tool got `screenToScene(screen)`); a tool returning `true`
    suppresses camera pan on the following move (mock backend camera unchanged); a tool returning
    `false` lets the move pan the camera; `snap` delegates to the grid; `setGrid` changes snap
    output; `setDraggingToken` forwards.
  - token-view: while `setDragging("t1")`, moving `t1`'s doc reconciles **snapped** (backend token
    x equals the new doc x immediately, no tween); after `setDragging(null)`, a move tweens.
- [ ] **Step 2:** fail. **Step 3:** implement (engine + token-view + types + exports). **Step 4:**
  pass; verify the testability invariant grep.
- [ ] **Step 5 (commit):** `feat(m8d-2): engine tool API — setActiveTool, pointer dispatch, snap, setGrid, dragging`

---

### Task 6: `SceneInteraction` bridge + `AppContext.scene`

A stable forwarder owned by `WorldSession`, attached to the engine by `Stage`, consumed by tool
components via `AppContext`. Pixi-free; render types imported **type-only**.

**Files:**
- Create: `src/client/ui/src/lib/sceneInteraction.ts`
- Modify: `src/client/ui/src/lib/appContext.ts` (`scene` field) + construction site + test fixture
- Modify: `src/client/ui/src/lib/worldSession.svelte.ts` (own a `readonly sceneInteraction`)
- Test: `src/client/ui/src/lib/sceneInteraction.test.ts`

**Interfaces:**
```ts
import type { SceneTool, SceneToolHost, Point } from "@shadowcat/render"; // type-only
export interface SceneInteraction extends SceneToolHost { attach(host: SceneToolHost): () => void }
export class SceneInteractionBridge implements SceneInteraction {
  // holds host: SceneToolHost | null; attach sets it + returns detach;
  // setActiveTool/setDraggingToken forward (no-op if detached); snap returns p unchanged if detached.
}
```

- [ ] **Step 1 (test):** before `attach`, `setActiveTool`/`setDraggingToken` are no-ops and
  `snap(p)` returns `p`; after `attach(fakeHost)`, calls forward to the host; the returned detach
  restores no-op behavior; a second `attach` replaces the host.
- [ ] **Step 2:** fail. **Step 3:** implement the bridge; `WorldSession` constructs one; add `scene`
  to `AppContext` + populate + fixture default (a fresh bridge). **Step 4:** pass; ui typecheck.
- [ ] **Step 5 (commit):** `feat(m8d-2): SceneInteraction bridge + AppContext.scene`

---

### Task 7: `scene-tools` module + tool rail + active-tool wiring

The first `src/client/ui/src/modules/scene-tools/` package (§8, §16): a module contributing one
`ToolRail.svelte` into `shadowcat.surface:toolrail`, owning active-tool state, wiring
`scene.setActiveTool`. Registered in the app composition root alongside `core-ui`. Imports only
shared `lib/*` — never `core-ui` internals.

**Files:**
- Create: `src/client/ui/src/modules/scene-tools/index.ts` (the `Module`)
- Create: `src/client/ui/src/modules/scene-tools/ToolRail.svelte`
- Create: `src/client/ui/src/modules/scene-tools/controller.svelte.ts` (active-tool + selected-asset runes; tool factory)
- Modify: composition root that builds `WorldSession` — pass `scene-tools` as a feature module
  (add `featureModules?: Module[]` to `WorldSessionOpts`, activated after `core-ui` in
  `#onWelcome`; locate the `new WorldSession({…})` site via grep).
- Test: `src/client/ui/src/modules/scene-tools/ToolRail.test.ts` (+ a harness mirroring `SurfaceHarness`)

**Behavior:** ToolRail reads `getAppContext()` (`scene`, `dispatchIntent`, `store`, `assets`,
`world`, `role`, `t`); renders touch-sized buttons (select/move, place); clicking a button
toggles the active tool (`scene.setActiveTool(toolImpl | null)`); GM-gates the buttons
(`role==="gm"`). Tool implementations are built in the component (Tasks 8–9) capturing the
context. Manifest:
```ts
{ id:"scene-tools", version:"0.1.0", dependencies:{ "core-ui":"^0.1.0" },
  requires:["shadowcat.surface:toolrail"], provides:[] }
```

- [ ] **Step 1 (test):** rendering `ToolRail` (GM context) into a toolrail harness shows the tool
  buttons; clicking "select/move" calls `scene.setActiveTool` with a non-null tool and marks the
  button active; clicking it again calls `setActiveTool(null)`; a non-GM context renders no
  buttons. (Use a fake `scene` bridge capturing `setActiveTool`.)
- [ ] **Step 2:** fail. **Step 3:** implement module + component + controller; add `featureModules`
  to `WorldSessionOpts` + activate in bootstrap; pass `[sceneTools]` at the composition root.
- [ ] **Step 4:** pass; ui typecheck. **Step 5 (commit):** `feat(m8d-2): scene-tools module + tool rail + active-tool wiring`

---

### Task 8: place tool + mini asset picker

**Files:**
- Modify: `src/client/ui/src/modules/scene-tools/controller.svelte.ts` (place `SceneTool` factory)
- Create: `src/client/ui/src/modules/scene-tools/AssetPicker.svelte` (lists assets via `lib/api` `listAssets`; selecting sets `controller.selectedAsset`)
- Modify: `ToolRail.svelte` (show the picker when place is active)
- Test: `src/client/ui/src/modules/scene-tools/place-tool.test.ts` (pure tool factory) + a picker render test

**Place `SceneTool`:** `onPointerDown(p)` → if a scene is active (`store.query("scene")[0]`) and an
asset is selected → `dispatchIntent([{op:"create", doc: buildTokenDoc(world, sceneId,
{ x: snap(p).x, y: snap(p).y, w:gridSize, h:gridSize, rotation:0, visual:{kind:"image",asset}})}])`;
return `true`. No scene or no asset → return `false` (let camera pan; optionally a `t()` hint).
`onPointerMove/Up` → no-op. (`gridSize` from the active scene's `system.grid.size`, default 100.)

- [ ] **Step 1 (test):** build the place tool with a fake ctx (a store seeded with one scene, a
  selected asset, a capturing `dispatchIntent`, an identity `snap`). `onPointerDown({x:140,y:160})`
  → one create intent whose doc is a token with `parent_id`=scene id, center=snap result,
  `visual.asset` = selected, `w/h` = scene grid size; returns `true`. With no selected asset →
  no intent, returns `false`. With no scene → no intent, returns `false`.
- [ ] **Step 2:** fail. **Step 3:** implement the place factory + `AssetPicker.svelte` + show-on-active.
- [ ] **Step 4:** pass; ui typecheck. **Step 5 (commit):** `feat(m8d-2): place tool + asset picker`

---

### Task 9: select/move tool + hit-test + drag

**Files:**
- Create: `src/client/ui/src/modules/scene-tools/hit-test.ts` (pure: topmost token under a point)
- Modify: `src/client/ui/src/modules/scene-tools/controller.svelte.ts` (select/move `SceneTool` factory)
- Test: `src/client/ui/src/modules/scene-tools/hit-test.test.ts`, `select-move-tool.test.ts`

**Hit-test (pure):** `topTokenAt(tokens: WireDocument[], p: Point): string | null` — AABB around
each token center (`x±w/2`, `y±h/2`; ignore rotation in M8d), return the **last** (topmost in
insertion/z order) containing `p`.

**Select/move `SceneTool`:**
- `onPointerDown(p)` → `id = topTokenAt(query("token"), p)`; if none, return `false` (camera pan).
  Else record `id` + the grab offset (`p - tokenCenter`), `scene.setDraggingToken(id)`, return `true`.
- `onPointerMove(p)` → if dragging: `target = snap(p - offset)`; **coalesced** dispatch (leading-edge
  throttle ~50ms via an injected clock; always remember the latest target) of
  `[{op:"update",doc_id:id,changes:[{path:"/system/x",old,new},{path:"/system/y",old,new}]}]`.
- `onPointerUp(p)` → send the final position update (the latest target, even if the throttle
  suppressed it), `scene.setDraggingToken(null)`, clear drag state.
- `old` values come from the current doc system; `new` from the snapped target. (The optimistic
  view tolerates approximate `old`; the server is authoritative.)

- [ ] **Step 1 (tests):**
  - hit-test: topmost-of-overlapping; point outside all → null; respects center±half-extent.
  - select-move: pointerdown on a token returns `true` + `setDraggingToken(id)`; a drag
    (down→move→up) dispatches coalesced `/system/x,y` update intents and a final on up;
    `setDraggingToken(null)` on up; pointerdown on empty space returns `false` + no drag. (Inject
    a fake clock to assert leading-edge coalescing.)
- [ ] **Step 2:** fail. **Step 3:** implement hit-test + the select/move factory; wire it into the
  ToolRail button. **Step 4:** pass; ui typecheck.
- [ ] **Step 5 (commit):** `feat(m8d-2): select/move tool — hit-test + coalesced drag intents`

---

### Task 10: Stage wiring (pointer routing + bridge attach + scene grid) + e2e

Route `Stage.svelte` pointer events through the engine dispatcher, attach the scene bridge, and
drive the grid from the active scene. Add the Playwright coverage deferred from M8c-1/M8d-1.

**Files:**
- Modify: `src/client/ui/src/modules/core-ui/panels/Stage.svelte`
- Create/Modify: `src/client/ui/e2e/stage.spec.ts` (or the existing stage smoke)

**Stage changes:**
- Replace `wireCamera`'s `pointerdown/move/up/cancel` bodies so they call
  `engine.dispatchPointerDown/Move/Up({x:clientX-rect.left, y:clientY-rect.top}, e)` (keep
  `setPointerCapture`; keep the wheel→`camera.zoomAt` handler as-is). The engine now owns
  tool-vs-camera.
- After `engine.start()`: `const detach = scene.attach(engine); ` (get `scene` from
  `getAppContext()`); call `detach()` in teardown.
- Drive the grid from the scene: read `store.query("scene")[0]?.system.grid`; call
  `engine.setGrid(grid ?? {kind:"square",size:100})` initially and on store change (a small
  subscription in the effect, disposed on teardown). Replaces the hardcoded
  `grid:{kind:"square",size:100}` construction default (keep the default as fallback).

- [ ] **Step 1 (e2e):** against the binary (Playwright), as GM: the stage renders; after entry a
  **scene background** path is exercisable and a placed **token** renders (place via the tool rail:
  activate place, pick an asset, click the canvas → a token sprite appears) and a **drag** moves
  it. Assert via the existing `data-render*` hooks + a token-count/DOM/screenshot signal. (This is
  the M8c-1 background-render + M8d-1 token-render e2e riding here per `docs/TODO.md`.)
- [ ] **Step 2:** implement Stage changes; run `pnpm --filter @shadowcat/ui e2e` (builds the client
  first — `rust-embed`/Playwright ordering per the embed-dist note).
- [ ] **Step 3:** keep existing entry/assets/stage smokes green.
- [ ] **Step 4 (commit):** `feat(m8d-2): Stage routes pointers through the engine + scene-driven grid + place/drag e2e`

---

## Final verification (before the branch review)

- [ ] `pnpm -r typecheck` — all packages.
- [ ] `pnpm -r test` — core + render + ui unit suites green.
- [ ] `pnpm lint` — clean (no `console.log`).
- [ ] `pnpm --filter @shadowcat/ui e2e` — entry + assets + stage (place/drag/render) green.
- [ ] `grep -rn "pixi.js" src/client/render/src` → only `pixi-backend.ts`.
- [ ] `grep -rn "core-ui" src/client/ui/src/modules/scene-tools` → no imports of core-ui internals.

## Buddy-check directive

M8d-2 introduces the **canvas interaction seam** every future tool (M8d-3 draw/template/measure,
M9 wall tool) and the **dispatchIntent** write path all build on, plus the scene-lifecycle
auto-create. Per every prior M8 slice, **run a buddy-check at the final branch review** (two blind
reviewers + reconciliation debate), focusing on: the tool-vs-camera dispatch (gesture ownership,
pan fallback, capture/teardown), the `dispatchIntent` correlation (one id predicts AND sends; no
double-send / orphaned pending), the scene auto-create idempotency (optimistic-view guard, reconnect
re-fire, multi-GM race), the drag coalescing + the dragging-snap (no pending-flood, no tween lag),
and the `parent_id` round-trip. Record the outcome in the execution handoff + auto-memory.

## §16 wiring — flagged for morning review

The AppContext extensions (`scene` + `dispatchIntent`) and the `SceneInteraction` bridge resolve
the §8 "exact wiring" confirm item **autonomously** (spec §16), following the `subscribeScene`
thin-seam convention. Pre-release/unfrozen surface → reversible. Surface this in the completion
report so the user can object early.

## Spec coverage self-check

- §15 scene lifecycle (schema, auto-create, scene-driven grid) → Tasks 2, 4, 10.
- §7 tool API (setActiveTool + SceneTool scene-coords + camera fallback) → Task 5 (+ §16 bridge Task 6).
- §8 scene-tools as first `src/modules/*` (contract-only) → Tasks 7–9.
- §4 token model (center origin, `visual` seam) → Task 2 + Task 8.
- place / select / move (the core interactive loop) → Tasks 8, 9, 10.
- §16 dispatchIntent predict-and-send + parent_id → Tasks 1, 3.
- §12 testability (headless model vs GL e2e) → Tasks 1–9 (node) + Task 10 (Playwright).
- **Deferred to M8d-3 (correctly absent):** drawing/template entities, measurement, pings (server),
  the ephemeral `previewOverlay` backend method (first needed by measure/template/draw).
