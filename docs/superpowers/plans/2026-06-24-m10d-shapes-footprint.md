# M10d — Token Shapes + Footprint Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a token's **shape** (`square` | `circle`) and **size** (grid units, fractional → multi-cell) load-bearing across rendering, selection/hit-test, and a `footprintRadius` seam for the M10e+ pathfinder — resolved live from the actor through one read-through.

**Architecture:** `ActorSystem.size`/`shape` already exist (seeded M10a) but are inert. This checkpoint wires them through a single core chokepoint `resolveTokenBox(token, store) → {x,y,w,h,shape}` (scene pixels; size = `EffectiveActor.size × scene-grid cell px`, shape from `EffectiveActor.shape`, falling back to `token.system.w/h` + `square` for raw/actorless tokens). The renderer, hit-test, and selection ring all read this one function so they cannot diverge for multi-cell or circular tokens. Shape becomes a per-token override (extending the M10a name/visual/size whitelist). A separate pure `footprintRadius(eff) → number` (grid units) is the value seam the pathfinder (M10e–g) consumes; no pathfinding ships here.

**Tech Stack:** TypeScript (client monorepo: `@shadowcat/core`, `@shadowcat/render`, `@shadowcat/ui-kit`, `src/modules/*`), Svelte 5 runes, Vitest, PixiJS v8 (`Graphics`). No server/Rust changes — the server stays structural-only (#6); shape/size live in the opaque `system` body.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-06-24-m10-tokens-design.md` §9 (shapes + footprint), §4/§5 (actor/token model, override whitelist), §13.8 (footprint-aware pathfinding decision). This plan implements **§9 only**.
- **#5 render-from-store:** tokens render from the Zod `DocumentStore` via the M8c reconciler; no client ECS. Size/shape resolve through `resolveTokenActor`/`resolveTokenBox`, never from a parallel cache.
- **#6 server structural-only:** no Rust changes. The override whitelist is a **client-side** projection (the server stores `system`/`overrides` opaquely and does not enforce the whitelist).
- **Center origin:** a token's `(x,y)` is its CENTER (M8d §4). Size scales the half-extents symmetrically; snapping is unchanged (cell-center snap stays as-is — even-size multi-cell snap is **out of scope**, deferred to M10e movement).
- **Fail-closed defaults:** a raw/actorless or dangling-link token resolves to `shape:"square"` and `w/h` from `token.system` (or `0` if absent) — never a throw.
- **Linked-actor edits propagate live:** because size/shape resolve through the actor each reconcile, editing a linked actor's size/shape updates every linked token immediately (instanced copies stay frozen — provenance/merge deferred, [[document-inheritance-merge-model]]).
- **TDD + atomic commits:** each task is red→green→commit. Run the affected package's tests; the final task runs the full client suite + typecheck + eslint.
- **i18n:** every user-facing string goes through `t("…")` with a key added to `src/client/ui-kit/src/locales/en.ts` (the neutral I18n core).

---

## File map

- `src/client/core/src/scene-docs.ts` — add `shape?` to `TokenOverrides`.
- `src/client/core/src/actor.ts` — apply shape override in `project()`; add `resolveTokenBox` + `footprintRadius`; export both. (`SceneSystem` imported for the grid cell lookup.)
- `src/client/core/src/index.ts` — re-export `resolveTokenBox`, `footprintRadius`, `TokenBox`.
- `src/client/core/src/actor.test.ts` — unit tests for the override, `resolveTokenBox`, `footprintRadius`.
- `src/client/render/src/types.ts` — `TokenNodeSpec.shape`.
- `src/client/render/src/token-view.ts` — reconcile size/shape via `resolveTokenBox`.
- `src/client/render/src/pixi-backend.ts` — draw an ellipse border for circle shape (rect otherwise).
- `src/client/render/src/token-view.test.ts` — multi-cell + circle + raw-token spec assertions.
- `src/modules/scene-tools/src/hit-test.ts` — shape/size-aware `topTokenAt` (gains `store` param).
- `src/modules/scene-tools/src/hit-test.test.ts` — **new**: circle-corner miss, multi-cell box, raw token.
- `src/modules/scene-tools/src/controller.svelte.ts` — pass `store` to `topTokenAt`; selection ring uses `resolveTokenBox`.
- `src/modules/actors/src/ActorsPanel.svelte` — create-form shape select + size inputs; per-row GM shape/size edit.
- `src/modules/actors/src/ActorsPanel.test.ts` — create + edit dispatch shape/size.
- `src/client/ui-kit/src/locales/en.ts` — new `actors.*` keys.
- `.claude/skills/shadowcat-codebase-actors-tokens/SKILL.md`, `docs/PLAN.md` — sync (final task).

---

### Task 1: Core — shape override, `resolveTokenBox`, `footprintRadius`

**Files:**
- Modify: `src/client/core/src/scene-docs.ts` (TokenOverrides)
- Modify: `src/client/core/src/actor.ts`
- Modify: `src/client/core/src/index.ts` (re-exports)
- Test: `src/client/core/src/actor.test.ts`

**Interfaces:**
- Consumes: `resolveTokenActor`, `EffectiveActor` (existing, `actor.ts`); `ActorSystem`, `TokenOverrides`, `SceneSystem` (`scene-docs.ts`); `ReadableDocuments` (`store.ts`, has `.get(id)`/`.query(type)`); `WireDocument` (has `parent_id`).
- Produces:
  - `interface TokenBox { x: number; y: number; w: number; h: number; shape: "square" | "circle"; }`
  - `resolveTokenBox(token: WireDocument, store: ReadableDocuments): TokenBox` — scene-pixel footprint. Actor-backed: `w = eff.size.w × cell`, `h = eff.size.h × cell` where `cell` = the token's parent scene `system.grid.size` (default `100`); `shape = eff.shape`. Raw/dangling: `w/h` from `token.system` (default `0`), `shape:"square"`. `x/y` always from `token.system` (default `0`).
  - `footprintRadius(eff: Pick<EffectiveActor, "shape" | "size">): number` — bounding-disc radius in **grid units** for M10e+ clearance/inflation. `circle` → `Math.max(w,h)/2`; `square` → `Math.hypot(w,h)/2` (half-diagonal, conservative enclosure).

- [ ] **Step 1: Add `shape?` to `TokenOverrides`** in `scene-docs.ts` (the M10a whitelist comment already says name/visual/size — extend it):

```ts
/** The per-token override whitelist for a linked token (M10a; shape added M10d). */
export interface TokenOverrides {
  name?: string;
  visual?: ActorVisual;
  size?: { w: number; h: number };
  shape?: "square" | "circle";
}
```

- [ ] **Step 2: Write the failing tests** in `actor.test.ts` (append; mirror the existing fake-store style used by the conditions/resolveTokenActor tests in this file):

```ts
import { resolveTokenBox, footprintRadius } from "./actor";
import { buildActorDoc, buildSceneDoc, buildTokenFromActor, buildTokenDoc } from "./scene-docs";
import type { ReadableDocuments } from "./store";
import type { WireDocument } from "./wire";

// Minimal read-only store over a fixed doc set.
function fakeStore(docs: WireDocument[]): ReadableDocuments {
  return {
    get: (id) => docs.find((d) => d.id === id),
    query: (type) => docs.filter((d) => d.doc_type === type),
    subscribe: () => () => {},
  } as ReadableDocuments;
}

const actorSys = (over: Partial<import("./scene-docs").ActorSystem> = {}) => ({
  name: "Goblin", displayName: "Goblin", visual: { kind: "image" as const, asset: "a1" },
  size: { w: 1, h: 1 }, shape: "square" as const, faction: null, conditions: [], prototype: false, ...over,
});

test("resolveTokenBox derives multi-cell pixel size from actor.size × scene grid cell", () => {
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100 } }, "scene1");
  const actor = buildActorDoc("w1", actorSys({ size: { w: 2, h: 3 } }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 50, y: 60 }, 100, "tok1");
  const box = resolveTokenBox(token, fakeStore([scene, actor, token]));
  expect(box).toEqual({ x: 50, y: 60, w: 200, h: 300, shape: "square" });
});

test("resolveTokenBox reads shape from the actor and applies a per-token override", () => {
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100 } }, "scene1");
  const actor = buildActorDoc("w1", actorSys({ shape: "circle" }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
  expect(resolveTokenBox(token, fakeStore([scene, actor, token])).shape).toBe("circle");
  (token.system as { overrides: import("./scene-docs").TokenOverrides }).overrides = { shape: "square", size: { w: 4, h: 4 } };
  const box = resolveTokenBox(token, fakeStore([scene, actor, token]));
  expect(box.shape).toBe("square");
  expect(box.w).toBe(400);
});

test("resolveTokenBox falls back to token.system w/h + square for a raw (actorless) token", () => {
  const token = buildTokenDoc("w1", "scene1", { x: 10, y: 20, w: 64, h: 64, rotation: 0, visual: { kind: "image", asset: "a1" } }, "tok1");
  expect(resolveTokenBox(token, fakeStore([token]))).toEqual({ x: 10, y: 20, w: 64, h: 64, shape: "square" });
});

test("resolveTokenBox defaults the grid cell to 100 when the parent scene is absent", () => {
  const actor = buildActorDoc("w1", actorSys({ size: { w: 1, h: 1 } }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
  expect(resolveTokenBox(token, fakeStore([actor, token])).w).toBe(100);
});

test("footprintRadius: circle = max(w,h)/2, square = half-diagonal", () => {
  expect(footprintRadius({ shape: "circle", size: { w: 2, h: 4 } })).toBe(2);
  expect(footprintRadius({ shape: "square", size: { w: 2, h: 2 } })).toBeCloseTo(Math.SQRT2, 5);
});
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- actor.test.ts`
Expected: FAIL — `resolveTokenBox`/`footprintRadius` not exported.

- [ ] **Step 4: Implement in `actor.ts`** — apply the shape override in `project()`, add the scene-cell helper, `resolveTokenBox`, and `footprintRadius`:

```ts
// add to the imports at the top:
import type { ActorSystem, ActorVisual, TokenOverrides, ConditionRegistrySystem, SceneSystem } from "./scene-docs";
```

In `project()`, add the shape override (replace the `shape: base.shape,` line):

```ts
    shape: overrides?.shape ?? base.shape,
```

Append the new exports:

```ts
/** A token's resolved footprint in scene pixels + its shape — the single read-through the
 * renderer, hit-test, and selection ring share so they cannot diverge for multi-cell/circle
 * tokens. Actor-backed: size = EffectiveActor.size (grid units) × the parent scene's grid cell;
 * raw/dangling tokens fall back to their own transform + square. `(x,y)` is the token center. */
export interface TokenBox {
  x: number;
  y: number;
  w: number;
  h: number;
  shape: "square" | "circle";
}

/** Grid cell size (px) of the token's parent scene; 100 when the scene is absent/garbled. */
function sceneCellSize(token: WireDocument, store: ReadableDocuments): number {
  const scene = token.parent_id ? store.get(token.parent_id) : undefined;
  return (scene?.system as SceneSystem | undefined)?.grid?.size ?? 100;
}

export function resolveTokenBox(token: WireDocument, store: ReadableDocuments): TokenBox {
  const s = token.system as { x?: number; y?: number; w?: number; h?: number } | undefined;
  const x = s?.x ?? 0;
  const y = s?.y ?? 0;
  const eff = resolveTokenActor(token, store);
  if (eff) {
    const cell = sceneCellSize(token, store);
    return { x, y, w: eff.size.w * cell, h: eff.size.h * cell, shape: eff.shape };
  }
  return { x, y, w: s?.w ?? 0, h: s?.h ?? 0, shape: "square" };
}

/** Bounding-disc radius (grid units) consumed by the M10e+ pathfinder for clearance/inflation.
 * Conservative enclosure: a square uses its half-diagonal, a circle its radius. Per-engine
 * refinement (grid clearance vs navmesh inflation) is owned by M10e/M10f. */
export function footprintRadius(eff: Pick<EffectiveActor, "shape" | "size">): number {
  const { w, h } = eff.size;
  return eff.shape === "circle" ? Math.max(w, h) / 2 : Math.hypot(w, h) / 2;
}
```

- [ ] **Step 5: Re-export from `index.ts`** — add `resolveTokenBox`, `footprintRadius`, and the `TokenBox` type to the existing `actor.ts` re-export block (match the existing `export { … } from "./actor"` / `export type { … }` pattern).

- [ ] **Step 6: Run the tests to verify they pass**

Run: `pnpm --filter @shadowcat/core test -- actor.test.ts`
Expected: PASS (all five new tests + existing).

- [ ] **Step 7: Commit**

```bash
git add src/client/core/src/scene-docs.ts src/client/core/src/actor.ts src/client/core/src/index.ts src/client/core/src/actor.test.ts
git commit -m "feat(m10d): shape override + resolveTokenBox + footprintRadius in core"
```

---

### Task 2: Render — `TokenNodeSpec.shape`, reconciler size/shape, ellipse border

**Files:**
- Modify: `src/client/render/src/types.ts` (TokenNodeSpec)
- Modify: `src/client/render/src/token-view.ts`
- Modify: `src/client/render/src/pixi-backend.ts`
- Test: `src/client/render/src/token-view.test.ts`

**Interfaces:**
- Consumes: `resolveTokenBox` (Task 1), `resolveTokenActor`, `resolveConditions` (core).
- Produces: `TokenNodeSpec.shape: "square" | "circle"` — set by the reconciler, drawn by the backend (ellipse vs rect border).

- [ ] **Step 1: Add `shape` to `TokenNodeSpec`** in `types.ts`:

```ts
export interface TokenNodeSpec {
  x: number;
  y: number;
  w: number;
  h: number;
  rotation: number;
  url: string;
  /** Faction border color (0xRRGGBB), or null for no border. */
  borderColor: number | null;
  /** Condition marker glyphs (emoji), rendered as upright chips along the token's top edge. */
  badges: string[];
  /** Footprint shape: drives the border outline + hit-test (M10d). */
  shape: "square" | "circle";
}
```

- [ ] **Step 2: Write the failing tests** in `token-view.test.ts` (mirror the existing reconcile tests that build a store + `MockBackend` and assert `backend.tokens.get(id)`). Add a scene doc to the store so the cell lookup resolves:

```ts
test("reconciles a linked 2x2 actor token to 2-cell pixel size + circle shape", () => {
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100 } }, "scene1");
  const actor = buildActorDoc("w1", { name: "Ogre", displayName: "Ogre", visual: { kind: "image", asset: "a1" }, size: { w: 2, h: 2 }, shape: "circle", faction: null, conditions: [], prototype: false }, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
  const { view, backend } = setup([scene, actor, token]); // existing test harness factory
  view.reconcile();
  const spec = backend.tokens.get("tok1")!;
  expect(spec.w).toBe(200);
  expect(spec.h).toBe(200);
  expect(spec.shape).toBe("circle");
});

test("raw token keeps its own size + defaults to square", () => {
  const token = buildTokenDoc("w1", "scene1", { x: 0, y: 0, w: 80, h: 80, rotation: 0, visual: { kind: "image", asset: "a1" } }, "tok1");
  const { view, backend } = setup([token]);
  view.reconcile();
  const spec = backend.tokens.get("tok1")!;
  expect(spec.w).toBe(80);
  expect(spec.shape).toBe("square");
});
```

> If `token-view.test.ts` has no shared `setup(docs)` factory, build the store + `MockBackend` + `TokenView` inline exactly as the existing tests in that file do, and add the imports (`buildSceneDoc`, `buildActorDoc`, `buildTokenFromActor`, `buildTokenDoc` from `@shadowcat/core`).

- [ ] **Step 3: Run the tests to verify they fail**

Run: `pnpm --filter @shadowcat/render test -- token-view.test.ts`
Expected: FAIL — `spec.w` is 100 (old cell-seed) / `spec.shape` undefined.

- [ ] **Step 4: Update `token-view.ts` `toSpec`** to resolve the footprint via `resolveTokenBox` (keep visual/border/badge logic unchanged):

```ts
// add to the core import:
import { resolveTokenActor, resolveConditions, resolveTokenBox } from "@shadowcat/core";
```

Replace the `return { … }` at the end of `toSpec`:

```ts
    const badges = resolveConditions(doc, this.store).map((c) => c.icon);
    const box = resolveTokenBox(doc, this.store);
    return {
      x: box.x, y: box.y, w: box.w, h: box.h, rotation: s.rotation ?? 0,
      url: this.assets.url(visual.asset),
      borderColor,
      badges,
      shape: box.shape,
    };
```

- [ ] **Step 5: Update `pixi-backend.ts` `setToken`** — draw an ellipse border for circle shape (sprite stays rectangular; shape is conveyed by the outline + hit-test). Replace the border-draw block (the `border.rect(...)` line):

```ts
      const hw = spec.w / 2;
      const hh = spec.h / 2;
      border.clear();
      if (spec.shape === "circle") {
        border.ellipse(0, 0, hw, hh).stroke({ width: 3, color: spec.borderColor });
      } else {
        border.rect(-hw, -hh, spec.w, spec.h).stroke({ width: 3, color: spec.borderColor });
      }
      border.position.set(spec.x, spec.y);
      border.angle = spec.rotation; // degrees, like the sprite
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `pnpm --filter @shadowcat/render test`
Expected: PASS (new + existing; the `MockBackend` already records the full spec so `shape` flows through with no mock change — the pixi ellipse is verified by the Playwright stage smoke).

- [ ] **Step 7: Commit**

```bash
git add src/client/render/src/types.ts src/client/render/src/token-view.ts src/client/render/src/pixi-backend.ts src/client/render/src/token-view.test.ts
git commit -m "feat(m10d): render multi-cell size + circle border via resolveTokenBox"
```

---

### Task 3: Hit-test + selection ring — shape & size aware

**Files:**
- Modify: `src/modules/scene-tools/src/hit-test.ts`
- Create: `src/modules/scene-tools/src/hit-test.test.ts`
- Modify: `src/modules/scene-tools/src/controller.svelte.ts`

**Interfaces:**
- Consumes: `resolveTokenBox`, `ReadableDocuments` (core); `Point` (`@shadowcat/render`).
- Produces: `topTokenAt(tokens: WireDocument[], p: Point, store: ReadableDocuments): string | null` — gains the `store` param; picks via the resolved box (ellipse containment for circle, AABB for square).

- [ ] **Step 1: Write the failing tests** — create `hit-test.test.ts`:

```ts
import { expect, test } from "vitest";
import { topTokenAt } from "./hit-test";
import { buildSceneDoc, buildActorDoc, buildTokenFromActor, buildTokenDoc } from "@shadowcat/core";
import type { ReadableDocuments, WireDocument } from "@shadowcat/core";

function fakeStore(docs: WireDocument[]): ReadableDocuments {
  return { get: (id) => docs.find((d) => d.id === id), query: (type) => docs.filter((d) => d.doc_type === type), subscribe: () => () => {} } as ReadableDocuments;
}
const actorSys = (over = {}) => ({ name: "G", displayName: "G", visual: { kind: "image" as const, asset: "a1" }, size: { w: 1, h: 1 }, shape: "square" as const, faction: null, conditions: [], prototype: false, ...over });

test("circle token: a point in the corner of its bounding box misses", () => {
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100 } }, "scene1");
  const actor = buildActorDoc("w1", actorSys({ shape: "circle" }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
  const store = fakeStore([scene, actor, token]);
  expect(topTokenAt([token], { x: 0, y: 0 }, store)).toBe("tok1");   // center: hit
  expect(topTokenAt([token], { x: 48, y: 48 }, store)).toBeNull();   // corner of the 100px box: miss
});

test("multi-cell square token is picked across its full footprint", () => {
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100 } }, "scene1");
  const actor = buildActorDoc("w1", actorSys({ size: { w: 3, h: 3 } }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
  const store = fakeStore([scene, actor, token]);
  expect(topTokenAt([token], { x: 140, y: 0 }, store)).toBe("tok1"); // inside 300px box, outside a 1-cell box
});

test("raw token uses its own box; topmost (last) wins on overlap", () => {
  const a = buildTokenDoc("w1", "scene1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "x" } }, "a");
  const b = buildTokenDoc("w1", "scene1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "x" } }, "b");
  expect(topTokenAt([a, b], { x: 0, y: 0 }, fakeStore([a, b]))).toBe("b");
});
```

- [ ] **Step 2: Run to verify failure**

Run: `pnpm --filter @shadowcat/module-scene-tools test -- hit-test.test.ts`
Expected: FAIL — `topTokenAt` takes 2 args / ignores shape.

> The package filter name is the `name` in `src/modules/scene-tools/package.json` — confirm it (e.g. `@shadowcat/module-scene-tools`) and use it for all scene-tools test runs in this task.

- [ ] **Step 3: Rewrite `hit-test.ts`** to resolve the box and test per shape:

```ts
import type { WireDocument, ReadableDocuments } from "@shadowcat/core";
import { resolveTokenBox } from "@shadowcat/core";
import type { Point } from "@shadowcat/render";

/** The id of the topmost token whose footprint contains `p`, or null. "Topmost" is the last in
 * document order (render z-order). Footprint = the resolved box (M10d): a circle token uses
 * ellipse containment, a square the AABB. Rotation is ignored for picking. */
export function topTokenAt(tokens: WireDocument[], p: Point, store: ReadableDocuments): string | null {
  let hit: string | null = null;
  for (const t of tokens) {
    const box = resolveTokenBox(t, store);
    if (box.w <= 0 || box.h <= 0) continue;
    const dx = p.x - box.x;
    const dy = p.y - box.y;
    const hw = box.w / 2;
    const hh = box.h / 2;
    const inside =
      box.shape === "circle"
        ? (dx * dx) / (hw * hw) + (dy * dy) / (hh * hh) <= 1
        : Math.abs(dx) <= hw && Math.abs(dy) <= hh;
    if (inside) hit = t.id;
  }
  return hit;
}
```

- [ ] **Step 4: Update the caller in `controller.svelte.ts`** — pass the store and resolve the selection-ring size via `resolveTokenBox` so multi-cell rings are correct:

Add the import:

```ts
import { resolveTokenBox } from "@shadowcat/core";
```

In `makeSelectMoveTool`, replace `sizeOf` to use the resolved box:

```ts
  const sizeOf = (id: string): { w: number; h: number } => {
    const doc = ctx.documents.get(id);
    if (!doc) return { w: 100, h: 100 };
    const box = resolveTokenBox(doc, ctx.documents);
    return { w: box.w || 100, h: box.h || 100 };
  };
```

Update the `onPointerDown` pick call:

```ts
      const id = topTokenAt(ctx.documents.query("token"), p, ctx.documents);
```

> The selection ring stays a rectangle (cosmetic) but now uses the effective multi-cell size — a rect ring around a circular token is conventional and acceptable.

- [ ] **Step 5: Run to verify pass**

Run: `pnpm --filter @shadowcat/module-scene-tools test`
Expected: PASS (new hit-test tests + existing place/select/move/draw/template tests — the existing tests build tokens with `system.w/h` and no actor, so they resolve to the raw-token box and keep working).

- [ ] **Step 6: Commit**

```bash
git add src/modules/scene-tools/src/hit-test.ts src/modules/scene-tools/src/hit-test.test.ts src/modules/scene-tools/src/controller.svelte.ts
git commit -m "feat(m10d): shape/size-aware token hit-test + selection ring"
```

---

### Task 4: UI — shape + size editing in `module-actors`

**Files:**
- Modify: `src/modules/actors/src/ActorsPanel.svelte`
- Modify: `src/client/ui-kit/src/locales/en.ts`
- Test: `src/modules/actors/src/ActorsPanel.test.ts`

**Interfaces:**
- Consumes: `buildActorDoc`, `ActorSystem` (core); `getAppContext` (`dispatchIntent`, `role`, `t`).
- Produces: actors created/edited with non-default `shape` + `size`, exercising the live linked-token propagation from Tasks 1–3.

- [ ] **Step 1: Add i18n keys** to `en.ts` (after `"actors.faction"`):

```ts
  "actors.shape": "Shape",
  "actors.shapeSquare": "Square",
  "actors.shapeCircle": "Circle",
  "actors.size": "Size (cells)",
  "actors.width": "Width",
  "actors.height": "Height",
```

- [ ] **Step 2: Write the failing test** in `ActorsPanel.test.ts` (mirror the existing render/create test setup in that file — `render(ActorsPanel)` with a fake AppContext capturing `dispatchIntent`). If the file does not exist, create it modeled on `src/modules/conditions/src/ConditionsPanel.test.ts`:

```ts
test("create dispatches an actor with the chosen shape and size", async () => {
  const { ctx, dispatched } = setup(); // fake AppContext; dispatched: WireOperation[][]
  render(ActorsPanel, { context: appContextMap(ctx) });
  await fill(/Name/i, "Ogre");
  await pickFirstAsset();
  await selectOption(/Shape/i, "circle");
  await fill(/Width/i, "2");
  await fill(/Height/i, "2");
  await clickButton(/Create actor/i);
  const create = dispatched.at(-1)![0] as { op: "create"; doc: WireDocument };
  const sys = create.doc.system as ActorSystem;
  expect(sys.shape).toBe("circle");
  expect(sys.size).toEqual({ w: 2, h: 2 });
});
```

> Reuse the test helpers/fixtures already present in `src/modules/actors/src/ActorsPanel.test.ts` (or copy the conditions panel's). The assertion that matters: the dispatched create carries `system.shape` and `system.size` from the form.

- [ ] **Step 3: Run to verify failure**

Run: `pnpm --filter @shadowcat/module-actors test -- ActorsPanel.test.ts`
Expected: FAIL — no Shape/Width inputs; `sys.shape` is always `"square"`, size `{1,1}`.

- [ ] **Step 4: Add the form state + inputs + create wiring** to `ActorsPanel.svelte`.

Add state (after `let faction = …`):

```ts
  let shape = $state<"square" | "circle">("square");
  let sizeW = $state(1);
  let sizeH = $state(1);
```

Use them in `create()` (replace the hardcoded `size`/`shape` lines):

```ts
      size: { w: sizeW, h: sizeH },
      shape,
```

Reset them at the end of `create()` (with the other resets):

```ts
    shape = "square";
    sizeW = 1;
    sizeH = 1;
```

Add the form controls (after the faction `<label>` in the `<form>`):

```svelte
    <label>{t("actors.shape")}
      <select bind:value={shape}>
        <option value="square">{t("actors.shapeSquare")}</option>
        <option value="circle">{t("actors.shapeCircle")}</option>
      </select>
    </label>
    <label>{t("actors.size")}
      <input type="number" min="0.5" step="0.5" aria-label={t("actors.width")} bind:value={sizeW} />
      <input type="number" min="0.5" step="0.5" aria-label={t("actors.height")} bind:value={sizeH} />
    </label>
```

- [ ] **Step 5: Add per-row GM shape/size edit** (mirror the existing faction `<select>` in the `{#if ctx.role === "gm"}` block) so a GM can change an existing actor (live-propagating to linked tokens):

```svelte
          <select
            aria-label={t("actors.shape")}
            value={(a.system as { shape?: string }).shape ?? "square"}
            onchange={(e) => ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/system/shape", old: (a.system as { shape?: string }).shape ?? "square", new: e.currentTarget.value }] }])}
          >
            <option value="square">{t("actors.shapeSquare")}</option>
            <option value="circle">{t("actors.shapeCircle")}</option>
          </select>
          <input
            type="number" min="0.5" step="0.5" class="size-edit" aria-label={t("actors.width")}
            value={(a.system as { size?: { w: number } }).size?.w ?? 1}
            onchange={(e) => { const sz = (a.system as { size?: { w: number; h: number } }).size ?? { w: 1, h: 1 }; ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/system/size", old: sz, new: { w: Number(e.currentTarget.value), h: sz.h } }] }]); }}
          />
```

> Keep the row compact; the width input edits `size.w`, preserving `size.h`. (A matching height input is optional — add it the same way if it fits the row; the create form already sets both.)

- [ ] **Step 6: Run to verify pass**

Run: `pnpm --filter @shadowcat/module-actors test`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/modules/actors/src/ActorsPanel.svelte src/client/ui-kit/src/locales/en.ts src/modules/actors/src/ActorsPanel.test.ts
git commit -m "feat(m10d): shape + size editing in module-actors"
```

---

### Task 5: Verification, docs/skill/PLAN sync, graphify

**Files:**
- Modify: `.claude/skills/shadowcat-codebase-actors-tokens/SKILL.md`
- Modify: `docs/PLAN.md`
- (no code — full-suite gate + knowledge sync)

- [ ] **Step 1: Full client verification gate** (evidence before claiming done):

```bash
pnpm -r typecheck
pnpm -r lint
pnpm -r test
```

Expected: all green. Fix any failure at its root cause before proceeding (do not weaken a test that is correctly asserting M10d behavior — [[tests-yield-to-correct-code]]).

- [ ] **Step 2: Update the actors-tokens skill** — add the new seam to `SKILL.md` so it doesn't drift:
  - Under **Key files & seams** (`actor.ts`): note `resolveTokenBox(token, store) → TokenBox {x,y,w,h,shape}` (the scene-pixel footprint read-through: size = `EffectiveActor.size × parent-scene grid cell`; raw → `token.system` + square) and `footprintRadius(eff)` (grid-unit bounding-disc radius for the M10e+ pathfinder).
  - Under **Key files & seams** (`scene-docs.ts`): `TokenOverrides` whitelist now includes `shape`.
  - Add a **Hard invariant**: "Rendered token size, hit-test, and the selection ring all resolve through `resolveTokenBox` — never read `token.system.w/h` directly for an actor-backed token, or multi-cell/circle tokens diverge."
  - Note `module-actors` now edits shape + size.

- [ ] **Step 3: Update `docs/PLAN.md`** — mark **M10d (Shapes + footprint)** complete (shape `{square,circle}` + per-token override, fractional/multi-cell size via `resolveTokenBox`, `footprintRadius` seam for M10e+); next = M10e (Pathfinding — grid).

- [ ] **Step 4: Refresh graphify** (AST-only, no API cost):

```bash
graphify update .
```

- [ ] **Step 5: Commit the sync**

```bash
git add .claude/skills/shadowcat-codebase-actors-tokens/SKILL.md docs/PLAN.md graphify-out
git commit -m "docs(m10d): sync actors-tokens skill + PLAN; mark shapes+footprint complete"
```

---

## Buddy-check directives

Per the M10 execution directive (one buddy-check per checkpoint, before merge), after Task 5 dispatch the standing two-reviewer pair **in parallel** over the full M10d branch diff (`git diff main...HEAD`):

- `shadowcat-spec-reviewer` — does the implementation cover spec §9 fully (shape both square+circle in actor-data **and** per-token override; size fractional→multi-cell; footprint radius seam present), nothing downgraded or skipped?
- `shadowcat-code-reviewer` — bugs/logic/convention/reuse, with focus on: the single-chokepoint invariant (no direct `token.system.w/h` reads left for actor-backed tokens), fail-closed defaults (raw/dangling token, missing scene), ellipse-vs-rect border correctness, and the hit-test ellipse math.

Reconcile findings round-by-round; fix Critical/Important inline with regression tests; record non-defects with rationale. This checkpoint is **client-only rendering/geometry** — no secrecy/permission surface (unlike M10b name privacy), so standard (not heightened) review depth. After buddy-check converges and the skill diff is confirmed accurate, M10d merges `--no-ff` to LOCAL main (the full-M10 push gate still holds — do **not** push).

## Self-review notes

- **Spec §9 coverage:** shape `{square,circle}` in actor-data (existing) + per-token override (Task 1) + render (Task 2) + hit-test/footprint consumers (Task 3); size fractional→multi-cell via `resolveTokenBox` (Tasks 1–3); footprint radius (Task 1). UI to author shape/size (Task 4). ✅
- **Out of scope (spec-aligned):** pathfinding (M10e–g) — only the `footprintRadius` value seam ships; even-size multi-cell snapping (M10e); sprite image masking to circle (border-only conveys shape in M10d); server changes (structural-only). 
- **Type consistency:** `resolveTokenBox`/`TokenBox`/`footprintRadius` names are identical across Tasks 1–5; `TokenNodeSpec.shape` and `TokenOverrides.shape` both `"square" | "circle"`.
