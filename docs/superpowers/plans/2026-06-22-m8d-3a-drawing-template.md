# M8d-3a — Drawing + Template Entities: Plan

> **For agentic workers:** execute with **`mainline-plan-execution`** (inline enumerative
> per-task spec-compliance check + ONE dispatched final branch review; buddy-check it).
> Steps use checkbox (`- [ ]`) syntax.

**Goal:** Persisted **drawing** + **template** scene entities (documents, optimistic,
reconciled into their layers) plus the **draw** and **template** tools, on the M8d-2
interaction API. Adds a generic shape backend node + the ephemeral **preview-overlay** API
(the draw tool needs live feedback). Client-only; pings/measurement are M8d-3b.

**Spec:** [`../specs/2026-06-22-m8d-scene-entities-tools-design.md`](../specs/2026-06-22-m8d-scene-entities-tools-design.md)
§9 (drawing/template entities), §7 (overlay preview API), §14b (the 3a/3b split).

**Architecture:** Shape geometry is **pure** (cone/circle/rect/ellipse → flat point arrays) so
the backend stays dumb (draw a polyline/polygon with stroke+fill). `DrawingView`/`TemplateView`
reconcile `doc_type:"drawing"/"template"` from the **optimistic** view (`ReadableDocuments`,
[[render-from-optimistic-view]]) → `setShape`/`removeShape`. The draw/template tools author docs
via `dispatchIntent` and show a live `previewOverlay` while dragging.

## Global constraints
- **#5/#6:** render from the Zod store via reconcilers; shapes live in the opaque `system`
  body (client-interpreted); server structural-only (no server work in 3a).
- **#3 ephemerals:** the in-progress draw/template preview is a client-local overlay — never a
  document, never persisted.
- **#7:** tools drive the canvas through the public interaction API (`SceneTool` +
  `previewOverlay`); persisted results are `dispatchIntent` creates.
- **Testability invariant:** only `pixi-backend.ts` imports `pixi.js`.
- **No raw `console.log`; commit per task; do NOT push** (the push gate is M8d-3b → M8).

---

### Task 1: shape geometry helpers (pure) + color parse

**Files:** Create `src/client/render/src/geometry.ts`; export from `index.ts`; test `geometry.test.ts`.

**Produces (all pure, scene coords, flat `[x0,y0,x1,y1,…]`):**
- `parseColor(hex: string): number` — `"#rrggbb"` → `0xRRGGBB` (fallback `0x000000` on malformed).
- `rectPoints(x0, y0, x1, y1): number[]` — 4 corners from opposite corners.
- `ellipsePoints(x0, y0, x1, y1, segments = 32): number[]` — ellipse inscribed in the bbox.
- `circlePoints(cx, cy, r, segments = 32): number[]`.
- `conePoints(apexX, apexY, size, directionDeg, apertureDeg = 60): number[]` — isoceles triangle
  apex + two base corners at distance `size`, at `direction ± apertureDeg/2`.
- `squarePoints(cx, cy, half, directionDeg): number[]` — square centered at `(cx,cy)`, side `2*half`, rotated.

- [ ] Test each: `parseColor("#ff8000")===0xff8000`, malformed→0; `rectPoints` → 8 numbers, correct
  corners; `circlePoints` count `2*segments`, all at radius `r±ε`; `conePoints` → apex + 2 corners
  at distance `size`; angle math (a cone facing 0° points +x). Implement. Pass.
- [ ] **Commit:** `feat(m8d-3a): pure shape geometry + color parse`

---

### Task 2: backend shape node + ephemeral overlay API

**Files:** `backend.ts` (interface), `backend.mock.ts`, `pixi-backend.ts`; test `backend.mock.test.ts`.

**Produces (on `DisplayBackend`):**
```ts
interface ShapeNodeSpec { layer: string; points: number[]; closed: boolean;
  stroke: { color: number; width: number } | null; fill: { color: number; alpha: number } | null; }
setShape(id: string, spec: ShapeNodeSpec): void;   // upsert a Graphics node in spec.layer
removeShape(id: string): void;
drawOverlay(shapes: Omit<ShapeNodeSpec, "layer">[]): void; // replace the overlays-layer content
clearOverlay(): void;
```
- **Mock:** `shapes: Map<string, ShapeNodeSpec>` (+ upsert/delete); `overlay: …[]` (last drawOverlay) / cleared.
- **Pixi:** `setShape` upserts a `Graphics` in `layers.get(spec.layer)`; `clear()`, `poly(points, closed)`
  (or moveTo/lineTo + `closePath` when closed), `fill(...)` then `stroke(...)`. `drawOverlay` clears a
  dedicated overlay `Graphics` (in the `overlays` layer) and draws each shape; `clearOverlay` clears it.
- [ ] Test the mock: setShape upsert/update/remove; drawOverlay records; clearOverlay empties. Implement
  (mock + interface + pixi). Pass. **Verify** `grep pixi.js` → only pixi-backend.
- [ ] **Commit:** `feat(m8d-3a): backend shape node + ephemeral overlay API`

---

### Task 3: `DrawingView` reconciler

**Files:** Create `drawing-view.ts`; export; test `drawing-view.test.ts`.

**Reads** (`drawing.system`, client-owned): `{ shape: { kind: "freehand"|"line"|"polygon"|"rect"|"ellipse", points: number[] }, stroke: { color: string, width: number } | null, fill: { color: string, alpha?: number } | null }`.
- `new DrawingView(documents: ReadableDocuments, backend)`, `reconcile()`: diff `query("drawing")` →
  `setShape(id, spec)` / `removeShape`. Map: freehand/line → open polyline; polygon → closed; rect →
  `rectPoints(points[0..3])` closed; ellipse → `ellipsePoints(points[0..3])` closed. `stroke`/`fill`
  colors via `parseColor`; `fill.alpha ?? 1`. Layer `"drawings"`.
- [ ] Test: a freehand doc → open polyline shape with parsed stroke; a rect doc → closed rect points; a
  deleted doc → removeShape. Implement. Pass.
- [ ] **Commit:** `feat(m8d-3a): DrawingView reconciler`

---

### Task 4: `TemplateView` reconciler

**Files:** Create `template-view.ts`; export; test `template-view.test.ts`.

**Reads** (`template.system`): `{ shape: { kind: "circle"|"cone"|"rect"|"line", x: number, y: number, size: number, direction: number }, color: string }`.
- `reconcile()`: diff `query("template")` → `setShape(id, spec)`. Map via geometry: circle →
  `circlePoints(x,y,size)`; cone → `conePoints(x,y,size,direction)`; rect → `squarePoints(x,y,size,direction)`;
  line → `[x, y, x+size·cos(dir), y+size·sin(dir)]` (open). Templates are translucent areas: `fill =
  { color: parseColor(color), alpha: 0.25 }`, `stroke = { color: parseColor(color), width: 2 }`. Layer
  `"templates"`. circle/cone/rect closed; line open.
- [ ] Test: a circle template → closed circle points + translucent fill; a cone → triangle; a line → open
  2-point segment; delete → removeShape. Implement. Pass.
- [ ] **Commit:** `feat(m8d-3a): TemplateView reconciler`

---

### Task 5: engine wiring + `previewOverlay` on the tool host

**Files:** `engine.ts`, `types.ts` (`SceneToolHost` += preview), `index.ts`; `src/client/ui/src/lib/sceneInteraction.ts` (bridge += preview); test `engine.test.ts`, `sceneInteraction.test.ts`.

- Engine constructs `DrawingView` + `TemplateView`; `start()` reconciles both initially + on every
  store change (alongside the scene/token reconcile); `destroy()` unchanged.
- `SceneToolHost` gains `previewOverlay(shapes: Omit<ShapeNodeSpec,"layer">[]): void` + `clearOverlay(): void`;
  `RenderEngine` forwards to `backend.drawOverlay`/`clearOverlay`. Export `ShapeNodeSpec`. The
  `SceneInteractionBridge` forwards both (no-op when detached).
- [ ] Test: engine renders an existing `drawing` doc + a `template` doc on start; `previewOverlay` forwards
  to the backend (mock records); bridge forwards preview to the host. Implement. Pass.
- [ ] **Commit:** `feat(m8d-3a): engine reconciles drawings/templates + previewOverlay tool-host API`

---

### Task 6: draw tool (freehand + rect/ellipse/line) + live preview

**Files:** `src/client/ui/src/modules/scene-tools/draw-tool.ts` (pure factory), `controller.svelte.ts`
(register the tool + draw mode/color state), `ToolRail.svelte` (draw button + mode/color controls);
test `draw-tool.test.ts`.

**`makeDrawTool(ctx, controller)`** (`controller` carries `drawMode: "freehand"|"rect"|"ellipse"|"line"`,
`strokeColor: string`):
- freehand: down → start points `[p]`; move → append `snap?`-free raw `p` (freehand is unsnapped),
  `previewOverlay([{points, closed:false, stroke}])`; up → `dispatchIntent(create drawing { shape:{kind:"freehand", points}, stroke })`, `clearOverlay`.
- rect/ellipse/line: down → anchor `a`; move → preview the shape from `a`→`p` (via geometry); up →
  create the drawing doc (`points=[a.x,a.y,p.x,p.y]`, kind=mode), `clearOverlay`.
- Returns `true` on down (claims the gesture). `stroke = { color: strokeColor, width: 2 }`.
- [ ] Test (fake ctx + capturing previewOverlay/dispatchIntent): a freehand down-move-move-up →
  ≥1 preview call + one create intent whose doc is `doc_type:"drawing"`, `parent_id`=scene, kind freehand,
  points include the path; a rect drag → create with kind rect + corner points; up clears the overlay.
- [ ] Implement. Pass. **Commit:** `feat(m8d-3a): draw tool (freehand/rect/ellipse/line) + live preview`

---

### Task 7: template tool (circle/cone/rect/line) + live preview

**Files:** `template-tool.ts`, `controller.svelte.ts` (template mode/color), `ToolRail.svelte` (template
button + mode); test `template-tool.test.ts`.

**`makeTemplateTool(ctx, controller)`** (`templateMode: "circle"|"cone"|"rect"|"line"`, `templateColor`):
- down → anchor `a = snap(p)`; move → `size = dist(a, p)`, `direction = angle(a→p)`; preview via geometry;
  up → `dispatchIntent(create template { shape:{kind:mode, x:a.x, y:a.y, size, direction}, color })`, clear.
- A zero-drag (size ~0) still creates a default-size template (e.g. one grid cell) so a click works.
- [ ] Test: a circle drag (anchor→radius) → preview + create template doc with kind circle, x/y=anchor,
  size=radius, parent_id=scene; a cone drag sets direction; up clears overlay.
- [ ] Implement. Pass. **Commit:** `feat(m8d-3a): template tool (circle/cone/rect/line) + live preview`

---

### Task 8: tool-rail controls + e2e

**Files:** `ToolRail.svelte` (draw + template buttons; mode + color pickers shown when active),
`Stage.svelte` (`data-shape-count` = drawings+templates, mirroring `data-token-count`), `en.ts`
(labels); `e2e/stage.spec.ts` (draw a stroke → a shape renders); tests for the rail.

- ToolRail: add `draw` + `template` to the tool set; when active, show a small mode selector + a color
  input (draw stroke / template color). Tool toggling routes through `scene.setActiveTool` as for place/select.
- Stage: `host.dataset.shapeCount = String(documents.query("drawing").length + documents.query("template").length)`.
- e2e: as GM, activate Draw, drag on the canvas, assert `data-shape-count` becomes `"1"`.
- [ ] Implement; ToolRail test (draw/template buttons activate the tools; non-GM hidden). Pass; `pnpm -r typecheck`/`test`/`lint`; `pnpm --filter @shadowcat/ui e2e`.
- [ ] **Commit:** `feat(m8d-3a): tool-rail draw/template controls + shape-render e2e`

---

## Final verification (before the branch review)
- [ ] `pnpm -r typecheck`; `pnpm -r test`; `pnpm lint` (no console.log); `pnpm --filter @shadowcat/ui e2e`.
- [ ] `grep -rn "pixi.js" src/client/render/src` → only `pixi-backend.ts`.
- [ ] `grep -rn "core-ui" src/client/ui/src/modules/scene-tools` → no core-ui internal imports.

## Buddy-check directive
Drawing/template are the second + third reconciled scene-entity kinds (the pattern M9 walls + later
light/sound/note reuse), and the shape/overlay backend API + geometry are load-bearing. **Run a
buddy-check at the final branch review** (two blind reviewers + reconciliation), focusing on: the
geometry correctness (cone/circle/ellipse tessellation, color parse), the reconciler diff (create/update/
delete; open-vs-closed; optimistic source), the overlay-vs-document separation (no preview leaks into a
doc), the draw/template tool gesture lifecycle (preview cleared on up/cancel, claim/own), and `parent_id`.

## Spec coverage self-check
- §9 drawing/template entity models (all kinds) → Tasks 1,3,4.
- §7 ephemeral overlay preview API → Tasks 2,5.
- draw + template tools → Tasks 6,7,8.
- §12 testability (pure geometry + headless reconciler vs GL e2e) → Tasks 1–7 (node) + 8 (Playwright).
- **Deferred to M8d-3b (correctly absent):** measurement (reuses this overlay API), pings (the server work).
