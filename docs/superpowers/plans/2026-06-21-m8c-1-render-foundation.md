# M8c-1 — Client Render Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: per the repo's user-scope guidance,
> execute this plan with **`mainline-plan-execution`** (inline enumerative
> spec-compliance check per task + one dispatched fresh-context branch review at the
> end) — NOT subagent-driven-development or executing-plans. Steps use checkbox
> (`- [ ]`) syntax for tracking.

**Goal:** Replace the M7 stage placeholder with a live PixiJS v8 canvas — a new
engine-owned `@shadowcat/render` package providing an ordered layer stack, a
pan/zoom camera, a square/hex grid, and a document-driven background reconciler —
mounted by a thin Svelte `Stage.svelte` host.

**Architecture:** A new framework-neutral `@shadowcat/render` workspace package
holds the render engine. Its **model** (LayerRegistry, Camera, Grid, reconciler,
RenderEngine orchestration) is pure TypeScript with NO `pixi.js` import, unit-tested
headless in node. A single `PixiBackend` file implements a narrow `DisplayBackend`
interface over `pixi.js` and is the only GL-touching code (validated by Playwright,
not unit tests). `Stage.svelte` (in `core-ui`, on the unchanged server-mirrored
`shadowcat.surface:stage` contract) owns the async init / resize / HiDPI / teardown
lifecycle and injects the backend, reading `store` + `assets` from `AppContext`.

**Tech Stack:** TypeScript (ES2022, bundler resolution, strict), `pixi.js ^8`,
Svelte 5 (runes), Vitest (node for the render model, jsdom for the Svelte host),
Playwright (real-GL e2e), pnpm workspaces.

## Global Constraints

- **Package layout:** new package at `src/client/render/`, name `@shadowcat/render`,
  `"type": "module"`, `"main": "src/index.ts"`, `"private": true`, `"version":
  "0.0.0"` — mirrors `@shadowcat/core`. tsconfig `extends "../../../tsconfig.base.json"`.
- **Dependency floor:** `pixi.js` `^8` (the only new runtime dep; lives in
  `@shadowcat/render` ONLY — never added to `@shadowcat/core` or `@shadowcat/ui`).
  `@shadowcat/core` consumed as `workspace:*`.
- **Testability invariant:** every file under `src/client/render/src/` EXCEPT
  `pixi-backend.ts` MUST NOT import `pixi.js` (keeps the model unit-testable in node
  jsdom-free). CI's `pnpm -r test` runs the render model in node; `pixi.js` is only
  loaded by Playwright and the ui build.
- **Cross-platform / mobile (#10):** all camera interaction via unified `pointer*`
  events (mouse/touch/pen) + `wheel` + two-pointer pinch; HiDPI via
  `resolution: devicePixelRatio` + `autoDensity: true`. No mouse-only or
  hover-only paths.
- **Engine-owned canvas (#7):** the PixiJS host is engine-owned; the layer stack is
  client-only (no server contract beyond the existing `shadowcat.surface:stage`
  mount point). Core layer ids are reserved; module-registered layers are 0.x.
- **No debug code in release (project rule):** client diagnostics go through the
  `@shadowcat/core` logger, never raw `console.log`; no `debugger;`.
- **Commit discipline:** commit each task's completed unit; do NOT push (push is the
  full-M8c milestone gate, after M8c-2).
- **Fixed core z-order (§6.1):** `background → grid → tiles → drawings → walls →
  tokens → templates → mask → overlays`. M8c-1 renders `background` + `grid`; the
  other layers are created empty (M8d/M8c-2 fill them).

---

### Task 1: `@shadowcat/render` package scaffold + value types

**Files:**
- Create: `src/client/render/package.json`
- Create: `src/client/render/tsconfig.json`
- Create: `src/client/render/vitest.config.ts`
- Create: `src/client/render/src/index.ts`
- Create: `src/client/render/src/types.ts`
- Test: `src/client/render/src/types.test.ts`

**Interfaces:**
- Produces: value types `Point = { x: number; y: number }`,
  `LineSeg = { x1: number; y1: number; x2: number; y2: number }`,
  `Polygon = { points: number[] }` (flat `[x0,y0,x1,y1,…]`, scene coords, D-V1),
  `CameraTransform = { x: number; y: number; scale: number }`, and the layer-id
  union `CoreLayerId` (defined in Task 2 but re-exported from `index.ts`).

- [ ] **Step 1: Write `package.json`**

```json
{
  "name": "@shadowcat/render",
  "version": "0.0.0",
  "private": true,
  "type": "module",
  "main": "src/index.ts",
  "dependencies": {
    "@shadowcat/core": "workspace:*",
    "pixi.js": "^8.0.0"
  },
  "scripts": {
    "typecheck": "tsc --noEmit",
    "test": "vitest run"
  }
}
```

- [ ] **Step 2: Write `tsconfig.json`**

```json
{
  "extends": "../../../tsconfig.base.json",
  "include": ["src/**/*.ts"]
}
```

- [ ] **Step 3: Write `vitest.config.ts`** (node env — the model is Pixi-free)

```ts
import { defineConfig } from "vitest/config";

// The render MODEL (layers/camera/grid/reconciler/engine) is framework- and
// Pixi-free, so it runs in node. The Pixi backend is GL and is covered by the ui
// Playwright suite, not here.
export default defineConfig({
  test: {
    // pixi.js must never be imported by a unit test; the model files don't import
    // it, so node is sufficient and fast.
  },
});
```

- [ ] **Step 4: Write `src/types.ts`**

```ts
/** A point in scene coordinates. */
export interface Point {
  x: number;
  y: number;
}

/** A line segment in scene coordinates (grid lines). */
export interface LineSeg {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}

/** Resolution-independent polygon geometry (D-V1), scene coords, flat
 * [x0,y0,x1,y1,…]. Consumed by the M8c-2 compositor; defined here so the public
 * value-type surface is one module. */
export interface Polygon {
  points: number[];
}

/** Camera transform applied to the world container: translate then uniform scale. */
export interface CameraTransform {
  x: number;
  y: number;
  scale: number;
}
```

- [ ] **Step 5: Write `src/index.ts`** (public surface; grows per task)

```ts
export type { Point, LineSeg, Polygon, CameraTransform } from "./types";
```

- [ ] **Step 6: Write the failing test `src/types.test.ts`**

```ts
import { test, expect } from "vitest";
import type { Polygon } from "./index";

test("Polygon stores flat scene-coordinate pairs", () => {
  const p: Polygon = { points: [0, 0, 10, 0, 10, 10] };
  expect(p.points.length % 2).toBe(0);
  expect(p.points.length / 2).toBe(3); // three vertices
});
```

- [ ] **Step 7: Install + run the test (verifies workspace wiring)**

Run: `pnpm install` (links the new package + fetches `pixi.js`), then
`pnpm --filter @shadowcat/render test`
Expected: 1 passed. (If `pnpm install` is needed for the lockfile, it updates
`pnpm-lock.yaml`.)

- [ ] **Step 8: Verify the whole workspace still typechecks**

Run: `pnpm -r typecheck`
Expected: all packages pass (the new package compiles; `pixi.js` types resolve).

- [ ] **Step 9: Commit**

```bash
git add src/client/render package.json pnpm-lock.yaml
git commit -m "feat(m8c-1): scaffold @shadowcat/render package + value types"
```

---

### Task 2: `LayerRegistry` + fixed core z-order

**Files:**
- Create: `src/client/render/src/layers.ts`
- Modify: `src/client/render/src/index.ts` (export `LayerRegistry`, `CORE_LAYERS`, `CoreLayerId`)
- Test: `src/client/render/src/layers.test.ts`

**Interfaces:**
- Produces:
  - `type CoreLayerId = "background" | "grid" | "tiles" | "drawings" | "walls" |
    "tokens" | "templates" | "mask" | "overlays"`
  - `const CORE_LAYERS: readonly CoreLayerId[]` (in §6.1 order)
  - `class LayerRegistry` with:
    - `orderedIds(): string[]` — core ids in z-order with module layers spliced by `order`
    - `register(id: string, order: number): () => void` — add a module layer (throws on a reserved core id or a duplicate id); returns a dispose that removes exactly it
- Consumes: nothing.

- [ ] **Step 1: Write the failing test `src/client/render/src/layers.test.ts`**

```ts
import { test, expect } from "vitest";
import { LayerRegistry, CORE_LAYERS } from "./index";

test("core layers are in the fixed §6.1 z-order", () => {
  const r = new LayerRegistry();
  expect(r.orderedIds()).toEqual([...CORE_LAYERS]);
  expect(CORE_LAYERS).toEqual([
    "background", "grid", "tiles", "drawings", "walls",
    "tokens", "templates", "mask", "overlays",
  ]);
});

test("a module layer is spliced by ascending order; dispose removes it", () => {
  const r = new LayerRegistry();
  const dispose = r.register("fx", 6.5); // between tokens(5) and templates(6)
  const ids = r.orderedIds();
  expect(ids.indexOf("fx")).toBeGreaterThan(ids.indexOf("tokens"));
  expect(ids.indexOf("fx")).toBeLessThan(ids.indexOf("mask"));
  dispose();
  expect(r.orderedIds()).not.toContain("fx");
});

test("registering a reserved core id or duplicate throws", () => {
  const r = new LayerRegistry();
  expect(() => r.register("tokens", 1)).toThrow();
  r.register("fx", 6.5);
  expect(() => r.register("fx", 7)).toThrow();
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/render test`
Expected: FAIL — `LayerRegistry`/`CORE_LAYERS` not exported.

- [ ] **Step 3: Write `src/client/render/src/layers.ts`**

```ts
/** The engine-owned canvas z-order (§6.1). Module layers splice between these by
 * fractional `order`; core ids are reserved. Index = the core order key. */
export type CoreLayerId =
  | "background" | "grid" | "tiles" | "drawings" | "walls"
  | "tokens" | "templates" | "mask" | "overlays";

export const CORE_LAYERS: readonly CoreLayerId[] = [
  "background", "grid", "tiles", "drawings", "walls",
  "tokens", "templates", "mask", "overlays",
] as const;

interface ModuleLayer {
  id: string;
  order: number;
}

/** Ordered named layer stack — client-only, engine-owned (#6/#7). Core layers are
 * fixed; modules add layers at a fractional `order` relative to core indices. */
export class LayerRegistry {
  private readonly core = new Map<string, number>(
    CORE_LAYERS.map((id, i) => [id, i]),
  );
  private modules: ModuleLayer[] = [];

  /** All layer ids in ascending z-order (core indices + module fractional orders). */
  orderedIds(): string[] {
    const all: { id: string; order: number }[] = [
      ...CORE_LAYERS.map((id, i) => ({ id, order: i })),
      ...this.modules,
    ];
    all.sort((a, b) => a.order - b.order);
    return all.map((l) => l.id);
  }

  /** Register a module layer; returns a dispose removing exactly it. */
  register(id: string, order: number): () => void {
    if (this.core.has(id)) {
      throw new Error(`layer id "${id}" is a reserved core layer`);
    }
    if (this.modules.some((m) => m.id === id)) {
      throw new Error(`layer id "${id}" is already registered`);
    }
    const layer: ModuleLayer = { id, order };
    this.modules.push(layer);
    return () => {
      const i = this.modules.indexOf(layer);
      if (i >= 0) this.modules.splice(i, 1);
    };
  }
}
```

- [ ] **Step 4: Export from `src/index.ts`** (append)

```ts
export { LayerRegistry, CORE_LAYERS, type CoreLayerId } from "./layers";
```

- [ ] **Step 5: Run to verify pass**

Run: `pnpm --filter @shadowcat/render test`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/client/render/src
git commit -m "feat(m8c-1): LayerRegistry with fixed core z-order"
```

---

### Task 3: `Camera` transform math

**Files:**
- Create: `src/client/render/src/camera.ts`
- Modify: `src/client/render/src/index.ts`
- Test: `src/client/render/src/camera.test.ts`

**Interfaces:**
- Produces: `class Camera` with:
  - `transform(): CameraTransform` — current `{ x, y, scale }`
  - `panBy(dxScreen: number, dyScreen: number): void` — pan by a screen-space delta
  - `zoomAt(factor: number, screenX: number, screenY: number): void` — multiply
    scale by `factor`, keeping the scene point under the cursor fixed (clamped scale)
  - `screenToScene(p: Point): Point`
  - `sceneToScreen(p: Point): Point`
- Consumes: `Point`, `CameraTransform` (Task 1).

**Math invariant:** `screen = scene * scale + offset`, where `offset = { x, y }`.
So `scene = (screen - offset) / scale`. `zoomAt` solves for the new offset that
holds `screenToScene(cursor)` invariant across the scale change.

- [ ] **Step 1: Write the failing test `src/client/render/src/camera.test.ts`**

```ts
import { test, expect } from "vitest";
import { Camera } from "./index";

test("default camera is identity", () => {
  const c = new Camera();
  expect(c.transform()).toEqual({ x: 0, y: 0, scale: 1 });
  expect(c.screenToScene({ x: 10, y: 20 })).toEqual({ x: 10, y: 20 });
});

test("panBy translates the offset in screen space", () => {
  const c = new Camera();
  c.panBy(15, -5);
  expect(c.transform()).toMatchObject({ x: 15, y: -5, scale: 1 });
  // A screen point now maps to a scene point shifted by the pan.
  expect(c.screenToScene({ x: 15, y: -5 })).toEqual({ x: 0, y: 0 });
});

test("zoomAt holds the scene point under the cursor fixed", () => {
  const c = new Camera();
  const cursor = { x: 100, y: 100 };
  const sceneBefore = c.screenToScene(cursor);
  c.zoomAt(2, cursor.x, cursor.y);
  const sceneAfter = c.screenToScene(cursor);
  expect(c.transform().scale).toBeCloseTo(2);
  expect(sceneAfter.x).toBeCloseTo(sceneBefore.x);
  expect(sceneAfter.y).toBeCloseTo(sceneBefore.y);
});

test("scale is clamped to the [0.1, 10] range", () => {
  const c = new Camera();
  c.zoomAt(1000, 0, 0);
  expect(c.transform().scale).toBeLessThanOrEqual(10);
  c.zoomAt(0.00001, 0, 0);
  expect(c.transform().scale).toBeGreaterThanOrEqual(0.1);
});

test("sceneToScreen is the inverse of screenToScene", () => {
  const c = new Camera();
  c.panBy(30, 40);
  c.zoomAt(1.5, 50, 50);
  const s = { x: 12, y: 34 };
  const round = c.sceneToScreen(c.screenToScene(s));
  expect(round.x).toBeCloseTo(s.x);
  expect(round.y).toBeCloseTo(s.y);
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/render test`
Expected: FAIL — `Camera` not exported.

- [ ] **Step 3: Write `src/client/render/src/camera.ts`**

```ts
import type { Point, CameraTransform } from "./types";

const MIN_SCALE = 0.1;
const MAX_SCALE = 10;

const clampScale = (s: number): number =>
  Math.min(MAX_SCALE, Math.max(MIN_SCALE, s));

/** Pure pan/zoom math: screen = scene * scale + offset. The engine applies
 * `transform()` to the Pixi world container and feeds it pointer gestures. */
export class Camera {
  private offset = { x: 0, y: 0 };
  private scale = 1;

  transform(): CameraTransform {
    return { x: this.offset.x, y: this.offset.y, scale: this.scale };
  }

  panBy(dxScreen: number, dyScreen: number): void {
    this.offset.x += dxScreen;
    this.offset.y += dyScreen;
  }

  /** Multiply scale by `factor`, holding the scene point under (screenX,screenY)
   * fixed. Derives the new offset so screenToScene(cursor) is invariant. */
  zoomAt(factor: number, screenX: number, screenY: number): void {
    const next = clampScale(this.scale * factor);
    // scene under cursor before: (screen - offset) / scale. Keep it constant:
    // offset' = screen - scene * scale'
    const sceneX = (screenX - this.offset.x) / this.scale;
    const sceneY = (screenY - this.offset.y) / this.scale;
    this.offset.x = screenX - sceneX * next;
    this.offset.y = screenY - sceneY * next;
    this.scale = next;
  }

  screenToScene(p: Point): Point {
    return {
      x: (p.x - this.offset.x) / this.scale,
      y: (p.y - this.offset.y) / this.scale,
    };
  }

  sceneToScreen(p: Point): Point {
    return {
      x: p.x * this.scale + this.offset.x,
      y: p.y * this.scale + this.offset.y,
    };
  }
}
```

- [ ] **Step 4: Export from `src/index.ts`** (append)

```ts
export { Camera } from "./camera";
```

- [ ] **Step 5: Run to verify pass**

Run: `pnpm --filter @shadowcat/render test`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/client/render/src
git commit -m "feat(m8c-1): Camera pan/zoom transform math"
```

---

### Task 4: `Grid` — square + hex coordinate math

**Files:**
- Create: `src/client/render/src/grid.ts`
- Modify: `src/client/render/src/index.ts`
- Test: `src/client/render/src/grid.test.ts`

**Interfaces:**
- Produces:
  - `type GridKind = "square" | "hex"`
  - `interface GridSpec { kind: GridKind; size: number }` (`size` = square edge /
    hex outer radius, scene units)
  - `class Grid` constructed `new Grid(spec: GridSpec)` with:
    - `snap(p: Point): Point` — nearest cell center (scene coords)
    - `cellOf(p: Point): { col: number; row: number }` — discrete cell index
    - `lines(viewportSceneRect: { x: number; y: number; w: number; h: number }):
      LineSeg[]` — grid lines covering the visible scene rect
- Consumes: `Point`, `LineSeg` (Task 1).

**Citations:** hex layout uses **pointy-top axial coordinates**
(Source: Red Blob Games, *Hexagonal Grids*, axial/round formulas). Chosen over
offset coords because axial round-trip (`pixel→hex→pixel`) and `snap` are exact and
branchless; offset coords need parity special-casing.

- [ ] **Step 1: Write the failing test `src/client/render/src/grid.test.ts`**

```ts
import { test, expect } from "vitest";
import { Grid } from "./index";

test("square grid snaps to cell centers", () => {
  const g = new Grid({ kind: "square", size: 100 });
  expect(g.snap({ x: 140, y: 160 })).toEqual({ x: 150, y: 150 });
  expect(g.cellOf({ x: 250, y: 50 })).toEqual({ col: 2, row: 0 });
});

test("square grid lines cover the viewport rect", () => {
  const g = new Grid({ kind: "square", size: 100 });
  const lines = g.lines({ x: 0, y: 0, w: 300, h: 200 });
  // 4 verticals (x=0,100,200,300) + 3 horizontals (y=0,100,200).
  const verticals = lines.filter((l) => l.x1 === l.x2);
  const horizontals = lines.filter((l) => l.y1 === l.y2);
  expect(verticals.length).toBe(4);
  expect(horizontals.length).toBe(3);
});

test("hex snap round-trips: a snapped point snaps to itself", () => {
  const g = new Grid({ kind: "hex", size: 50 });
  const snapped = g.snap({ x: 137, y: 221 });
  const again = g.snap(snapped);
  expect(again.x).toBeCloseTo(snapped.x);
  expect(again.y).toBeCloseTo(snapped.y);
});

test("hex grid emits a non-empty line set over a viewport", () => {
  const g = new Grid({ kind: "hex", size: 50 });
  const lines = g.lines({ x: 0, y: 0, w: 400, h: 400 });
  expect(lines.length).toBeGreaterThan(0);
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/render test`
Expected: FAIL — `Grid` not exported.

- [ ] **Step 3: Write `src/client/render/src/grid.ts`**

```ts
import type { Point, LineSeg } from "./types";

export type GridKind = "square" | "hex";

export interface GridSpec {
  /** "square": `size` = edge length. "hex": `size` = outer radius. */
  kind: GridKind;
  size: number;
}

interface SceneRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Engine-owned grid model + coordinate math (square + pointy-top hex). Pure: the
 * engine draws `lines(...)` into the grid layer and uses `snap`/`cellOf` for
 * placement (M8d). Hex uses axial coords (Red Blob Games). */
export class Grid {
  constructor(private readonly spec: GridSpec) {}

  snap(p: Point): Point {
    if (this.spec.kind === "square") {
      const { col, row } = this.cellOf(p);
      const s = this.spec.size;
      return { x: col * s + s / 2, y: row * s + s / 2 };
    }
    const { q, r } = this.axialRound(this.pixelToAxial(p));
    return this.axialToPixel(q, r);
  }

  cellOf(p: Point): { col: number; row: number } {
    if (this.spec.kind === "square") {
      return {
        col: Math.floor(p.x / this.spec.size),
        row: Math.floor(p.y / this.spec.size),
      };
    }
    const { q, r } = this.axialRound(this.pixelToAxial(p));
    return { col: q, row: r };
  }

  lines(rect: SceneRect): LineSeg[] {
    return this.spec.kind === "square"
      ? this.squareLines(rect)
      : this.hexLines(rect);
  }

  private squareLines(rect: SceneRect): LineSeg[] {
    const s = this.spec.size;
    const out: LineSeg[] = [];
    const x0 = Math.floor(rect.x / s) * s;
    const y0 = Math.floor(rect.y / s) * s;
    for (let x = x0; x <= rect.x + rect.w; x += s) {
      out.push({ x1: x, y1: rect.y, x2: x, y2: rect.y + rect.h });
    }
    for (let y = y0; y <= rect.y + rect.h; y += s) {
      out.push({ x1: rect.x, y1: y, x2: rect.x + rect.w, y2: y });
    }
    return out;
  }

  // --- pointy-top axial hex (Red Blob Games) ---
  // radius = size; width = sqrt(3)*size, height = 2*size; rows offset by height*3/4.
  private pixelToAxial(p: Point): { q: number; r: number } {
    const size = this.spec.size;
    const q = ((Math.sqrt(3) / 3) * p.x - (1 / 3) * p.y) / size;
    const r = ((2 / 3) * p.y) / size;
    return { q, r };
  }

  private axialToPixel(q: number, r: number): Point {
    const size = this.spec.size;
    return {
      x: size * (Math.sqrt(3) * q + (Math.sqrt(3) / 2) * r),
      y: size * (3 / 2) * r,
    };
  }

  private axialRound(a: { q: number; r: number }): { q: number; r: number } {
    // Round in cube space then fix the largest-drift component.
    let rx = Math.round(a.q);
    let ry = Math.round(-a.q - a.r);
    let rz = Math.round(a.r);
    const dx = Math.abs(rx - a.q);
    const dy = Math.abs(ry - (-a.q - a.r));
    const dz = Math.abs(rz - a.r);
    if (dx > dy && dx > dz) rx = -ry - rz;
    else if (dy > dz) ry = -rx - rz;
    else rz = -rx - ry;
    return { q: rx, r: rz };
  }

  private hexLines(rect: SceneRect): LineSeg[] {
    // Draw each hex outline whose center falls in (a margin around) the rect. The
    // overlap between adjacent hexes is acceptable for a grid overlay.
    const size = this.spec.size;
    const out: LineSeg[] = [];
    const margin = size * 2;
    const minA = this.pixelToAxial({ x: rect.x - margin, y: rect.y - margin });
    const maxA = this.pixelToAxial({ x: rect.x + rect.w + margin, y: rect.y + rect.h + margin });
    const qLo = Math.floor(Math.min(minA.q, maxA.q)) - 1;
    const qHi = Math.ceil(Math.max(minA.q, maxA.q)) + 1;
    const rLo = Math.floor(Math.min(minA.r, maxA.r)) - 1;
    const rHi = Math.ceil(Math.max(minA.r, maxA.r)) + 1;
    for (let r = rLo; r <= rHi; r++) {
      for (let q = qLo; q <= qHi; q++) {
        const c = this.axialToPixel(q, r);
        const pts: Point[] = [];
        for (let i = 0; i < 6; i++) {
          const ang = (Math.PI / 180) * (60 * i - 30); // pointy-top
          pts.push({ x: c.x + size * Math.cos(ang), y: c.y + size * Math.sin(ang) });
        }
        for (let i = 0; i < 6; i++) {
          const a = pts[i];
          const b = pts[(i + 1) % 6];
          out.push({ x1: a.x, y1: a.y, x2: b.x, y2: b.y });
        }
      }
    }
    return out;
  }
}
```

- [ ] **Step 4: Export from `src/index.ts`** (append)

```ts
export { Grid, type GridKind, type GridSpec } from "./grid";
```

- [ ] **Step 5: Run to verify pass**

Run: `pnpm --filter @shadowcat/render test`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/client/render/src
git commit -m "feat(m8c-1): square + hex grid coordinate math"
```

---

### Task 5: `DisplayBackend` interface, `MockBackend`, and the background reconciler

**Files:**
- Create: `src/client/render/src/backend.ts`
- Create: `src/client/render/src/backend.mock.ts`
- Create: `src/client/render/src/reconciler.ts`
- Modify: `src/client/render/src/index.ts`
- Test: `src/client/render/src/reconciler.test.ts`

**Interfaces:**
- Produces:
  - `interface DisplayBackend` (the narrow GL-abstraction the model drives):
    ```ts
    interface DisplayBackend {
      ensureLayers(orderedIds: string[]): void;
      setBackground(spec: { url: string } | null): void;
      drawGrid(lines: LineSeg[], color: number): void;
      setCameraTransform(t: CameraTransform): void;
      resize(width: number, height: number): void;
      destroy(): void;
    }
    ```
  - `class MockBackend implements DisplayBackend` — records calls
    (`background: { url } | null`, `layers: string[]`, `gridLineCount: number`,
    `camera: CameraTransform | null`, `destroyed: boolean`)
  - `class SceneReconciler` constructed `new SceneReconciler(store, assets, backend)`
    with `reconcile(): void` — reads the Scene doc, sets/clears the background
- Consumes: `LineSeg`, `CameraTransform` (Task 1); `DocumentStore`, `AssetResolver`,
  `WireDocument` from `@shadowcat/core`.

**Background rule:** the reconciler finds the single `doc_type === "scene"`
document; if its `system.background` is a non-empty string (asset UUID), it calls
`backend.setBackground({ url: assets.url(uuid) })`; otherwise `setBackground(null)`.
No Scene doc ⇒ `setBackground(null)`. (Token/wall/etc. reconcilers are M8d.)

- [ ] **Step 1: Write the failing test `src/client/render/src/reconciler.test.ts`**

```ts
import { test, expect } from "vitest";
import { DocumentStore, AssetResolver } from "@shadowcat/core";
import { MockBackend, SceneReconciler } from "./index";
import type { WireDocument } from "@shadowcat/core";

function sceneDoc(background: string | null): WireDocument {
  return {
    id: "scene-1",
    scope: { kind: "world", world_id: "w1" },
    doc_type: "scene",
    schema_version: 1,
    source: null,
    owner: null,
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
    embedded: {},
    system: background === null ? {} : { background },
    created_at: 0,
    updated_at: 0,
  };
}

test("reconcile resolves the scene background UUID to a URL", () => {
  const store = new DocumentStore();
  store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: sceneDoc("asset-uuid") }] });
  const assets = new AssetResolver();
  const backend = new MockBackend();
  new SceneReconciler(store, assets, backend).reconcile();
  expect(backend.background).toEqual({ url: assets.url("asset-uuid") });
});

test("reconcile clears the background when the scene has none", () => {
  const store = new DocumentStore();
  store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: sceneDoc(null) }] });
  const backend = new MockBackend();
  new SceneReconciler(store, new AssetResolver(), backend).reconcile();
  expect(backend.background).toBeNull();
});

test("reconcile with no scene doc clears the background", () => {
  const backend = new MockBackend();
  new SceneReconciler(new DocumentStore(), new AssetResolver(), backend).reconcile();
  expect(backend.background).toBeNull();
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/render test`
Expected: FAIL — `MockBackend`/`SceneReconciler` not exported.

- [ ] **Step 3: Write `src/client/render/src/backend.ts`**

```ts
import type { LineSeg, CameraTransform } from "./types";

/** The narrow GL abstraction the render model drives. The real implementation is
 * `pixi-backend.ts` (Playwright-covered); `MockBackend` covers it in unit tests.
 * Kept minimal for M8c-1 (background + grid + camera); M8d generalizes to a node
 * API for token/wall/etc. reconcilers. */
export interface DisplayBackend {
  /** Create/parent the core layer containers in the given z-order (idempotent). */
  ensureLayers(orderedIds: string[]): void;
  /** Set or clear the background-layer sprite. */
  setBackground(spec: { url: string } | null): void;
  /** Replace the grid-layer line set (scene coords) with the given color (0xRRGGBB). */
  drawGrid(lines: LineSeg[], color: number): void;
  /** Apply the camera transform to the world container. */
  setCameraTransform(t: CameraTransform): void;
  /** Resize the renderer/viewport to CSS pixels (HiDPI handled by the backend). */
  resize(width: number, height: number): void;
  /** Release all GPU resources and detach the canvas. */
  destroy(): void;
}
```

- [ ] **Step 4: Write `src/client/render/src/backend.mock.ts`**

```ts
import type { DisplayBackend } from "./backend";
import type { LineSeg, CameraTransform } from "./types";

/** A recording DisplayBackend for unit tests — never touches Pixi/GL. */
export class MockBackend implements DisplayBackend {
  layers: string[] = [];
  background: { url: string } | null = null;
  gridLineCount = 0;
  gridColor: number | null = null;
  camera: CameraTransform | null = null;
  size: { width: number; height: number } | null = null;
  destroyed = false;

  ensureLayers(orderedIds: string[]): void {
    this.layers = [...orderedIds];
  }
  setBackground(spec: { url: string } | null): void {
    this.background = spec;
  }
  drawGrid(lines: LineSeg[], color: number): void {
    this.gridLineCount = lines.length;
    this.gridColor = color;
  }
  setCameraTransform(t: CameraTransform): void {
    this.camera = t;
  }
  resize(width: number, height: number): void {
    this.size = { width, height };
  }
  destroy(): void {
    this.destroyed = true;
  }
}
```

- [ ] **Step 5: Write `src/client/render/src/reconciler.ts`**

```ts
import type { DocumentStore, AssetResolver, WireDocument } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";

/** The scene document's engine-reserved system fields (M8 §4.2: opaque to the
 * server, interpreted by the client). M8c-1 reads only `background`. */
interface SceneSystem {
  background?: string;
}

/** Maps scene-entity documents to display objects. M8c-1 handles the scene
 * background only; M8d adds per-doc_type handlers (token/wall/tile/…). */
export class SceneReconciler {
  constructor(
    private readonly store: DocumentStore,
    private readonly assets: AssetResolver,
    private readonly backend: DisplayBackend,
  ) {}

  reconcile(): void {
    const scene = this.store.query("scene")[0] as WireDocument | undefined;
    const bg = (scene?.system as SceneSystem | undefined)?.background;
    if (typeof bg === "string" && bg.length > 0) {
      this.backend.setBackground({ url: this.assets.url(bg) });
    } else {
      this.backend.setBackground(null);
    }
  }
}
```

- [ ] **Step 6: Export from `src/index.ts`** (append)

```ts
export type { DisplayBackend } from "./backend";
export { MockBackend } from "./backend.mock";
export { SceneReconciler } from "./reconciler";
```

- [ ] **Step 7: Run to verify pass**

Run: `pnpm --filter @shadowcat/render test`
Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add src/client/render/src
git commit -m "feat(m8c-1): DisplayBackend + MockBackend + background reconciler"
```

---

### Task 6: `RenderEngine` orchestration

**Files:**
- Create: `src/client/render/src/engine.ts`
- Modify: `src/client/render/src/index.ts`
- Test: `src/client/render/src/engine.test.ts`

**Interfaces:**
- Produces: `class RenderEngine` constructed with
  `new RenderEngine(opts: { store: DocumentStore; assets: AssetResolver; backend:
  DisplayBackend; grid: GridSpec })` and:
  - `start(): void` — ensure layers, draw the grid, initial reconcile, subscribe to
    the store (re-reconcile on change), apply the initial camera transform
  - `camera: Camera` — the engine's camera (public; the host wires gestures to it)
  - `applyCamera(): void` — push `camera.transform()` to the backend and redraw the
    grid for the new viewport (called by the host after a gesture)
  - `setViewport(width: number, height: number): void` — backend resize + grid redraw
  - `destroy(): void` — unsubscribe from the store + `backend.destroy()`
- Consumes: `DocumentStore`, `AssetResolver` (`@shadowcat/core`); `DisplayBackend`,
  `Camera`, `Grid`, `GridSpec`, `LayerRegistry`, `SceneReconciler` (this package).

**Viewport→scene rect:** the engine maps the current pixel viewport through the
camera (`screenToScene` of the four corners) to a scene rect for `grid.lines(...)`.
For M8c-1 the viewport defaults to `{ width: 0, height: 0 }` until `setViewport`.

- [ ] **Step 1: Write the failing test `src/client/render/src/engine.test.ts`**

```ts
import { test, expect } from "vitest";
import { DocumentStore, AssetResolver } from "@shadowcat/core";
import { RenderEngine, MockBackend } from "./index";

function makeEngine() {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const engine = new RenderEngine({ store, assets, backend, grid: { kind: "square", size: 100 } });
  return { store, backend, engine };
}

test("start ensures layers, draws the grid, and applies the camera", () => {
  const { backend, engine } = makeEngine();
  engine.setViewport(300, 200);
  engine.start();
  expect(backend.layers[0]).toBe("background");
  expect(backend.layers).toContain("mask");
  expect(backend.gridLineCount).toBeGreaterThan(0);
  expect(backend.camera).toEqual({ x: 0, y: 0, scale: 1 });
});

test("a store change triggers a re-reconcile", () => {
  const { store, backend, engine } = makeEngine();
  engine.start();
  expect(backend.background).toBeNull();
  store.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id: "s1", scope: { kind: "world", world_id: "w1" }, doc_type: "scene",
      schema_version: 1, source: null, owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, system: { background: "u1" }, created_at: 0, updated_at: 0,
    } }],
  });
  expect(backend.background).not.toBeNull();
});

test("destroy unsubscribes (no reconcile after destroy) and destroys the backend", () => {
  const { store, backend, engine } = makeEngine();
  engine.start();
  engine.destroy();
  expect(backend.destroyed).toBe(true);
  const before = backend.background;
  store.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id: "s1", scope: { kind: "world", world_id: "w1" }, doc_type: "scene",
      schema_version: 1, source: null, owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, system: { background: "u1" }, created_at: 0, updated_at: 0,
    } }],
  });
  expect(backend.background).toBe(before); // unchanged: listener was removed
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/render test`
Expected: FAIL — `RenderEngine` not exported.

- [ ] **Step 3: Write `src/client/render/src/engine.ts`**

```ts
import type { DocumentStore, AssetResolver } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import { Camera } from "./camera";
import { Grid, type GridSpec } from "./grid";
import { LayerRegistry } from "./layers";
import { SceneReconciler } from "./reconciler";

export interface RenderEngineOpts {
  store: DocumentStore;
  assets: AssetResolver;
  backend: DisplayBackend;
  grid: GridSpec;
  /** Grid line color (0xRRGGBB) sampled from CSS tokens by the host; default slate. */
  gridColor?: number;
}

/** Orchestrates the render model over a DisplayBackend: layers, camera, grid, and
 * the store-driven reconciler. Framework- and Pixi-free (the backend is injected). */
export class RenderEngine {
  readonly camera = new Camera();
  private readonly layers = new LayerRegistry();
  private readonly grid: Grid;
  private readonly reconciler: SceneReconciler;
  private readonly gridColor: number;
  private viewport = { width: 0, height: 0 };
  private unsubscribe: (() => void) | null = null;

  constructor(private readonly opts: RenderEngineOpts) {
    this.grid = new Grid(opts.grid);
    this.gridColor = opts.gridColor ?? 0x3a3a4a;
    this.reconciler = new SceneReconciler(opts.store, opts.assets, opts.backend);
  }

  start(): void {
    this.opts.backend.ensureLayers(this.layers.orderedIds());
    this.applyCamera();
    this.reconciler.reconcile();
    this.unsubscribe = this.opts.store.subscribe(() => this.reconciler.reconcile());
  }

  setViewport(width: number, height: number): void {
    this.viewport = { width, height };
    this.opts.backend.resize(width, height);
    this.redrawGrid();
  }

  /** Push the camera transform to the backend and redraw the grid for the new view. */
  applyCamera(): void {
    this.opts.backend.setCameraTransform(this.camera.transform());
    this.redrawGrid();
  }

  private redrawGrid(): void {
    const tl = this.camera.screenToScene({ x: 0, y: 0 });
    const br = this.camera.screenToScene({ x: this.viewport.width, y: this.viewport.height });
    const rect = { x: tl.x, y: tl.y, w: br.x - tl.x, h: br.y - tl.y };
    this.opts.backend.drawGrid(this.grid.lines(rect), this.gridColor);
  }

  destroy(): void {
    this.unsubscribe?.();
    this.unsubscribe = null;
    this.opts.backend.destroy();
  }
}
```

- [ ] **Step 4: Export from `src/index.ts`** (append)

```ts
export { RenderEngine, type RenderEngineOpts } from "./engine";
```

- [ ] **Step 5: Run to verify pass**

Run: `pnpm --filter @shadowcat/render test`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/client/render/src
git commit -m "feat(m8c-1): RenderEngine orchestration over the DisplayBackend"
```

---

### Task 7: `PixiBackend` — the real `pixi.js` v8 implementation

**Files:**
- Create: `src/client/render/src/pixi-backend.ts`
- Modify: `src/client/render/src/index.ts`

**Interfaces:**
- Produces: `class PixiBackend implements DisplayBackend`, and
  `createPixiBackend(canvas: HTMLCanvasElement, opts: { background: number }):
  Promise<PixiBackend>` — async because PixiJS v8 `Application.init` is async.
- Consumes: `DisplayBackend` (Task 5), `pixi.js`.

**No unit test:** this is the only GL file (Global Constraint); jsdom has no WebGL.
It is typechecked here and exercised by the Task 9 Playwright smoke. Correctness of
the *model* it serves is already covered by Tasks 2–6.

- [ ] **Step 1: Write `src/client/render/src/pixi-backend.ts`**

```ts
import { Application, Container, Graphics, Sprite, Assets } from "pixi.js";
import type { DisplayBackend } from "./backend";
import type { LineSeg, CameraTransform } from "./types";

/** The real DisplayBackend over pixi.js v8. The only GL-touching module (kept out
 * of unit tests; covered by Playwright). Layer containers parent under one `world`
 * container so a single camera transform pans/zooms the whole scene. */
export class PixiBackend implements DisplayBackend {
  private readonly world = new Container();
  private readonly layers = new Map<string, Container>();
  private readonly grid = new Graphics();
  private background: Sprite | null = null;
  private backgroundUrl: string | null = null;

  constructor(private readonly app: Application) {
    this.app.stage.addChild(this.world);
  }

  ensureLayers(orderedIds: string[]): void {
    for (const id of orderedIds) {
      if (this.layers.has(id)) continue;
      const c = new Container();
      c.label = id;
      this.layers.set(id, c);
      this.world.addChild(c);
      if (id === "grid") c.addChild(this.grid);
    }
    // Re-parent in z-order (addChild appends; order array is authoritative).
    for (const id of orderedIds) {
      const c = this.layers.get(id);
      if (c) this.world.addChild(c); // moving to top in order yields final stack
    }
  }

  setBackground(spec: { url: string } | null): void {
    if (spec === null) {
      this.background?.destroy();
      this.background = null;
      this.backgroundUrl = null;
      return;
    }
    if (spec.url === this.backgroundUrl) return; // unchanged
    this.backgroundUrl = spec.url;
    void Assets.load(spec.url).then((texture) => {
      // A teardown or a newer background may have raced ahead; bail if stale.
      if (this.backgroundUrl !== spec.url) return;
      this.background?.destroy();
      const sprite = new Sprite(texture);
      this.background = sprite;
      this.layers.get("background")?.addChild(sprite);
    });
  }

  drawGrid(lines: LineSeg[], color: number): void {
    this.grid.clear();
    for (const l of lines) this.grid.moveTo(l.x1, l.y1).lineTo(l.x2, l.y2);
    this.grid.stroke({ width: 1, color, alpha: 0.5 });
  }

  setCameraTransform(t: CameraTransform): void {
    this.world.position.set(t.x, t.y);
    this.world.scale.set(t.scale);
  }

  resize(width: number, height: number): void {
    this.app.renderer.resize(width, height);
  }

  destroy(): void {
    // Release GPU resources + remove the canvas; children/textures included.
    this.app.destroy({ removeView: true }, { children: true, texture: true });
  }
}

/** Construct a PixiBackend over a canvas (async: v8 Application.init is async). */
export async function createPixiBackend(
  canvas: HTMLCanvasElement,
  opts: { background: number },
): Promise<PixiBackend> {
  const app = new Application();
  await app.init({
    canvas,
    antialias: true,
    resolution: globalThis.devicePixelRatio || 1,
    autoDensity: true,
    background: opts.background,
    preference: "webgl",
  });
  return new PixiBackend(app);
}
```

- [ ] **Step 2: Export from `src/index.ts`** (append)

```ts
export { PixiBackend, createPixiBackend } from "./pixi-backend";
```

- [ ] **Step 3: Typecheck the package**

Run: `pnpm --filter @shadowcat/render typecheck`
Expected: pass (pixi.js v8 types resolve; no errors).

- [ ] **Step 4: Run the model tests (still green; backend not imported by them)**

Run: `pnpm --filter @shadowcat/render test`
Expected: all pass (no new tests; confirms the backend file didn't break exports).

- [ ] **Step 5: Commit**

```bash
git add src/client/render/src
git commit -m "feat(m8c-1): PixiBackend — pixi.js v8 DisplayBackend implementation"
```

---

### Task 8: `Stage.svelte` host + core-ui swap

**Files:**
- Create: `src/client/ui/src/modules/core-ui/panels/Stage.svelte`
- Modify: `src/client/ui/src/modules/core-ui/index.ts:4,26` (import + contribute Stage instead of StagePlaceholder)
- Delete: `src/client/ui/src/modules/core-ui/panels/StagePlaceholder.svelte`
- Modify: `src/client/ui/src/locales/en.ts:26` (repurpose the `stage.placeholder` key)
- Modify: `src/client/ui/package.json:6-9` (add `@shadowcat/render` dependency)
- Test: `src/client/ui/src/modules/core-ui/panels/Stage.test.ts`

**Interfaces:**
- Consumes: `RenderEngine`, `createPixiBackend`, `type DisplayBackend` from
  `@shadowcat/render`; `getAppContext` (`store`, `assets`, `t`).
- The component accepts an optional prop `createBackend?: (canvas:
  HTMLCanvasElement) => Promise<DisplayBackend>` defaulting to `createPixiBackend`,
  so the jsdom test injects a fake backend (real Pixi needs WebGL → Playwright only).

**Grid spec for M8c-1:** square, size 100 (a sensible default until scene-driven
grid config lands in M8d). Grid color sampled from the `--border` CSS token.

- [ ] **Step 1: Add the dependency to `src/client/ui/package.json`**

```json
  "dependencies": {
    "@shadowcat/core": "workspace:*",
    "@shadowcat/render": "workspace:*",
    "@shadowcat/types": "workspace:^"
  },
```

Run: `pnpm install`
Expected: `@shadowcat/render` linked into `@shadowcat/ui`.

- [ ] **Step 2: Write the failing test `Stage.test.ts`**

```ts
import { test, expect, vi } from "vitest";
import { render } from "@testing-library/svelte";
import Stage from "./Stage.svelte";
import type { DisplayBackend } from "@shadowcat/render";
import { setAppContextForTest } from "../../../lib/__fixtures__/appContextTest";

function fakeBackend(): DisplayBackend & { destroyed: boolean } {
  return {
    destroyed: false,
    ensureLayers() {},
    setBackground() {},
    drawGrid() {},
    setCameraTransform() {},
    resize() {},
    destroy() { this.destroyed = true; },
  };
}

test("mounts a canvas container and tears the backend down on unmount", async () => {
  const backend = fakeBackend();
  const createBackend = vi.fn(async () => backend);
  const { container, unmount } = render(Stage, {
    props: { createBackend },
    context: setAppContextForTest(),
  });
  // The host renders a canvas element synchronously.
  expect(container.querySelector("[data-testid='stage-canvas']")).not.toBeNull();
  // The $effect's async init runs after mount; wait for the backend factory.
  await vi.waitFor(() => expect(createBackend).toHaveBeenCalledOnce());
  // Unmount runs the $effect cleanup synchronously, destroying the backend.
  unmount();
  expect(backend.destroyed).toBe(true);
});
```

- [ ] **Step 3: Export the context key for tests — modify `src/client/ui/src/lib/appContext.ts`**

Replace the module-private `const KEY = Symbol("shadowcat.appContext");` with an
exported key so the test fixture can seed an AppContext via `@testing-library`'s
`context` Map. The two functions change only the identifier they reference:

```ts
/** Context key; exported only so test fixtures can seed an AppContext. */
export const __APP_CONTEXT_KEY__ = Symbol("shadowcat.appContext");

export function setAppContext(ctx: AppContext): void {
  setContext(__APP_CONTEXT_KEY__, ctx);
}

export function getAppContext(): AppContext {
  const ctx = getContext<AppContext | undefined>(__APP_CONTEXT_KEY__);
  if (!ctx) {
    throw new Error("AppContext is not set; render within a provider that calls setAppContext");
  }
  return ctx;
}
```

- [ ] **Step 3b: Create the test fixture `src/client/ui/src/lib/__fixtures__/appContextTest.ts`**

```ts
import type { AppContext } from "../appContext";
import { __APP_CONTEXT_KEY__ } from "../appContext";
import { DocumentStore, AssetResolver, ContributionRegistry } from "@shadowcat/core";

/** Build a Map for @testing-library/svelte's `context` option holding a minimal
 * AppContext (overridable per field), seeded under the real private key. */
export function setAppContextForTest(over: Partial<AppContext> = {}): Map<unknown, unknown> {
  const ctx: AppContext = {
    contributions: over.contributions ?? new ContributionRegistry(),
    store: over.store ?? new DocumentStore(),
    assets: over.assets ?? new AssetResolver(),
    world: over.world ?? "w1",
    role: over.role ?? "gm",
    t: over.t ?? ((k: string) => k),
    onAssetChanged: over.onAssetChanged ?? (() => () => {}),
    leaveWorld: over.leaveWorld ?? (() => {}),
  };
  return new Map([[__APP_CONTEXT_KEY__, ctx]]);
}
```

- [ ] **Step 4: Run to verify it fails**

Run: `pnpm --filter @shadowcat/ui test -- Stage`
Expected: FAIL — `Stage.svelte` does not exist.

- [ ] **Step 5: Write `src/client/ui/src/modules/core-ui/panels/Stage.svelte`**

```svelte
<script lang="ts">
  import { getAppContext } from "../../../lib/appContext";
  import {
    RenderEngine,
    createPixiBackend,
    type DisplayBackend,
  } from "@shadowcat/render";

  /** Backend factory; defaults to the real Pixi backend. Tests inject a fake
   * (jsdom has no WebGL — real GL is covered by Playwright). */
  let {
    createBackend = (canvas: HTMLCanvasElement): Promise<DisplayBackend> =>
      createPixiBackend(canvas, { background: readColor("--surface-base", 0x101014) }),
  }: {
    createBackend?: (canvas: HTMLCanvasElement) => Promise<DisplayBackend>;
  } = $props();

  const { store, assets } = getAppContext();

  let host: HTMLDivElement;
  let canvas: HTMLCanvasElement;

  /** Sample a CSS custom property as a 0xRRGGBB number (canvas chrome reads tokens). */
  function readColor(token: string, fallback: number): number {
    if (typeof getComputedStyle !== "function" || !host) return fallback;
    const raw = getComputedStyle(host).getPropertyValue(token).trim();
    const m = /^#([0-9a-f]{6})$/i.exec(raw);
    return m ? parseInt(m[1], 16) : fallback;
  }

  $effect(() => {
    let engine: RenderEngine | null = null;
    let disposed = false;
    let observer: ResizeObserver | null = null;

    void (async () => {
      const backend = await createBackend(canvas);
      if (disposed) { backend.destroy(); return; } // teardown raced the async init
      engine = new RenderEngine({
        store,
        assets,
        backend,
        grid: { kind: "square", size: 100 },
        gridColor: readColor("--border", 0x3a3a4a),
      });
      engine.setViewport(host.clientWidth, host.clientHeight);
      engine.start();
      wireCamera(engine);
      observer = new ResizeObserver(() => {
        if (engine) engine.setViewport(host.clientWidth, host.clientHeight);
      });
      observer.observe(host);
      host.dataset.renderReady = "true";
    })();

    return () => {
      disposed = true;
      observer?.disconnect();
      engine?.destroy();
    };
  });

  /** Pointer/wheel gestures → camera. Unified pointer events (#10). */
  function wireCamera(engine: RenderEngine): void {
    let dragging = false;
    let lastX = 0;
    let lastY = 0;
    canvas.addEventListener("pointerdown", (e) => {
      dragging = true; lastX = e.clientX; lastY = e.clientY;
      canvas.setPointerCapture(e.pointerId);
    });
    canvas.addEventListener("pointermove", (e) => {
      if (!dragging) return;
      engine.camera.panBy(e.clientX - lastX, e.clientY - lastY);
      lastX = e.clientX; lastY = e.clientY;
      engine.applyCamera();
    });
    const endDrag = (): void => { dragging = false; };
    canvas.addEventListener("pointerup", endDrag);
    canvas.addEventListener("pointercancel", endDrag);
    canvas.addEventListener("wheel", (e) => {
      e.preventDefault();
      const rect = canvas.getBoundingClientRect();
      const factor = e.deltaY < 0 ? 1.1 : 1 / 1.1;
      engine.camera.zoomAt(factor, e.clientX - rect.left, e.clientY - rect.top);
      engine.applyCamera();
    }, { passive: false });
  }
</script>

<div class="stage-host" bind:this={host}>
  <canvas bind:this={canvas} data-testid="stage-canvas"></canvas>
</div>

<style lang="scss">
  .stage-host {
    height: 100%;
    width: 100%;
    overflow: hidden;
    background: var(--surface-base);
    touch-action: none; /* let pointer gestures drive pan/zoom on touch (#10) */
  }
  canvas {
    display: block;
  }
</style>
```

- [ ] **Step 6: Swap the contribution — modify `src/client/ui/src/modules/core-ui/index.ts`**

Replace the `StagePlaceholder` import (line 4) and its `contribute` call (line 26):

```ts
import Stage from "./panels/Stage.svelte";
```
```ts
    ctx.contributions.contribute({ id: "core-ui:stage", contract: "shadowcat.surface:stage", component: Stage });
```

- [ ] **Step 7: Delete the placeholder + repurpose the i18n key**

Delete `src/client/ui/src/modules/core-ui/panels/StagePlaceholder.svelte`.

The `stage.placeholder` key is now unused by `Stage.svelte`. Remove the line
`"stage.placeholder": "Scene rendering arrives in M8.",` from
`src/client/ui/src/locales/en.ts` (and confirm no other reference remains —
`grep -rn "stage.placeholder" src/`).

- [ ] **Step 8: Run the Stage test + the full ui unit suite**

Run: `pnpm --filter @shadowcat/ui test`
Expected: `Stage.test.ts` passes; existing suites still pass. (If the coreUi test
references StagePlaceholder it does not — it only checks surface contracts.)

- [ ] **Step 9: Typecheck**

Run: `pnpm -r typecheck`
Expected: pass (the deleted component has no remaining importers; `@shadowcat/render`
resolves in ui).

- [ ] **Step 10: Commit**

```bash
git add src/client/ui src/client/render package.json pnpm-lock.yaml
git commit -m "feat(m8c-1): Stage.svelte Pixi host replaces the stage placeholder"
```

---

### Task 9: Playwright smoke + entry-flow fix

**Files:**
- Create: `src/client/ui/e2e/stage.spec.ts`
- Modify: `src/client/ui/e2e/entry-flow.spec.ts:18`

**Interfaces:**
- Consumes: the served binary (Playwright `webServer`), the in-world shell with the
  new `Stage` host.

**What the smoke proves (real GL, headless chromium with SwiftShader):** entering a
world mounts the Pixi canvas, the engine reaches first-frame readiness
(`data-render-ready` on the host), the canvas has non-zero size, a pointer drag is
accepted without error, and leaving the world removes the canvas (teardown). The
deep camera/grid/reconcile correctness is already unit-tested (Tasks 3–6); a
background-render e2e rides with scene authoring in M8d (the reconciler is unit-
tested now). See Deviations.

- [ ] **Step 1: Fix the existing entry-flow assertion — `entry-flow.spec.ts:18`**

Replace the placeholder-text assertion with the canvas-presence assertion:

```ts
  // Entering a world mounts the Pixi stage canvas.
  await expect(page.getByTestId("stage-canvas")).toBeVisible();
```

- [ ] **Step 2: Write `src/client/ui/e2e/stage.spec.ts`**

```ts
import { test, expect } from "@playwright/test";

// Drives the served binary: after entering a world the Pixi canvas mounts, the
// engine reaches first-frame readiness, accepts a pan gesture, and tears down on
// leave. Real WebGL via headless chromium (SwiftShader).
test("stage canvas mounts, renders, and tears down on leave", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();
  await page.getByLabel("New world name").fill("Render World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  const canvas = page.getByTestId("stage-canvas");
  await expect(canvas).toBeVisible();
  const box = await canvas.boundingBox();
  expect(box?.width ?? 0).toBeGreaterThan(0);
  expect(box?.height ?? 0).toBeGreaterThan(0);

  // A pan gesture must not throw (pointer events drive the camera).
  await canvas.hover();
  await page.mouse.down();
  await page.mouse.move((box!.x) + 50, (box!.y) + 50);
  await page.mouse.up();
  await expect(host).toHaveAttribute("data-render-ready", "true");

  // Leave-world tears the canvas down.
  await page.getByRole("button", { name: /leave world/i }).click();
  await expect(page.getByTestId("stage-canvas")).toHaveCount(0);
});
```

- [ ] **Step 3: Verify the leave-world control selector**

Run: `grep -rn "leave" src/client/ui/src/modules/core-ui/panels/` to confirm the
control's accessible name. If it differs from "Leave world", update the
`getByRole` name regex in Step 2 to match the actual button (the M7d leave-world
control). If the control lives elsewhere, target it by its real label.

- [ ] **Step 4: Build + run the e2e suite locally**

Run: `pnpm --filter @shadowcat/ui e2e`
Expected: `entry-flow.spec.ts` + `stage.spec.ts` pass against the built binary.
(The `e2e:build` step rebuilds dist/ — which now bundles `@shadowcat/render` — and
the binary; rust-embed picks up the new bundle.)

- [ ] **Step 5: Commit**

```bash
git add src/client/ui/e2e
git commit -m "test(m8c-1): Playwright smoke over the Pixi stage mount/teardown"
```

---

## Final verification (before handoff to review)

- [ ] `pnpm -r typecheck` — all packages pass.
- [ ] `pnpm -r test` — render model + core + ui unit suites pass.
- [ ] `pnpm lint` — clean (no raw `console.log`, no unused).
- [ ] `pnpm --filter @shadowcat/ui e2e` — entry-flow + stage smoke pass.
- [ ] `cargo build -p shadowcat --bin shadowcat` — server still embeds the new dist/.
- [ ] Confirm the testability invariant: `grep -rn "pixi.js" src/client/render/src`
  returns ONLY `pixi-backend.ts`.

## Buddy-check directives

This plan introduces a new workspace package, a new runtime dependency (`pixi.js`),
and the GL lifecycle/teardown boundary that **all** of M8c/M8d builds on — and the
async-init-vs-teardown race in `Stage.svelte` (Task 8) + `PixiBackend.setBackground`
(Task 7) is a subtle correctness hazard. Consistent with M8a and M8b-1 (both
buddy-checked), **offer a buddy-check for the final branch review** (two blind
reviewers + reconciliation debate) rather than a single review. The execution
handoff records the user's choice.

## Deviations from the M8c design spec (surfaced per project rule)

- **Spec §9 lists "background renders" in the c-1 Playwright smoke.** Rendering a
  background needs a Scene document with `system.background`, which needs
  scene-authoring UI that does not exist until **M8d**. Rather than fake a scene via
  a test-only window hook (a smell) or pull scene authoring forward (scope creep),
  the **background reconciler is fully unit-tested now** (Task 5) and the *browser*
  background-render assertion is deferred to M8d when scenes can be authored. The
  c-1 smoke instead proves canvas mount + first-frame readiness + pan + teardown.
  This is decomposition (full logic shipped + tested), not descope. **Logged to
  `docs/TODO.md` during execution.**

## Spec coverage self-check

- §4.1 new `@shadowcat/render` package + pixi.js dep → Task 1, Global Constraints.
- §4.2 RenderEngine headless orchestration → Task 6.
- §4.3 Stage.svelte host (async init, attach, resize, HiDPI, $effect teardown) → Task 8.
- §5.1 LayerRegistry + fixed core z-order → Task 2.
- §5.2 scene reconciler (background via AssetResolver) → Task 5.
- §6 Camera (pan/zoom, pointer/touch/pinch, screen↔scene) → Task 3 (math) + Task 8 (gestures).
- §6 Grid (square/hex + coordinate math) → Task 4.
- §10 testability split (headless model vs Pixi backend; Playwright for GL) → Global
  Constraints + Tasks 2–7 (node) + Task 9 (Playwright).
- **Deferred to M8c-2 (correctly absent here):** WsClient.subscribeScene, Compositor,
  vision-mask spike, shader-filter seam, module-facing API formalization, token re-audit.
