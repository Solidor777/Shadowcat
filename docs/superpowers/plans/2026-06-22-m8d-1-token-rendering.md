# M8d-1 — Token Rendering Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: execute with **`mainline-plan-execution`**
> (inline enumerative per-task spec-compliance check + ONE dispatched final branch review;
> buddy-check that review). NOT subagent-driven / executing-plans. Steps use checkbox
> (`- [ ]`) syntax.

**Goal:** Render `doc_type:"token"` scene-entity documents as sprites on the M8c canvas,
tweening each toward its document-authoritative transform — render-only, no interaction UI
(tokens are created/moved via direct intents in tests; the place/move tools are M8d-2).

**Architecture:** A pure `TokenAnimator` holds per-token current transforms and tweens them
toward targets (exponential smoothing, headless-testable). A `TokenView` (engine-owned)
diffs `store.query("token")` → `DisplayBackend.setToken`/`removeToken`, sets animator
targets, and on each `tick(dt)` re-pushes moved tokens. A render **ticker** (added to the
backend; PixiJS `app.ticker`, no-op in the mock) drives `tick`. The render model stays
Pixi-free; `PixiBackend` is the only GL file and renders each token as a Container+Sprite
in the `tokens` layer.

**Tech Stack:** TypeScript (strict), `pixi.js ^8`, Vitest (node), Playwright, pnpm.

## Global Constraints

- **Testability invariant:** only `src/client/render/src/pixi-backend.ts` imports `pixi.js`
  (verify: `grep -rn "pixi.js" src/client/render/src` → only that file).
- **Token model (§4, confirmed §13):** a token is a top-level `Document`,
  `doc_type:"token"`, `parent_id` = scene id; `system` =
  `{ x, y, w, h, rotation, visual: { kind: "image", asset: <uuid> } }` where **`(x,y)` is
  the token CENTER** (scene units). `visual` is the forward-looking seam — only
  `kind:"image"` in M8d. Server stays structural-only (#6); the client interprets `system`.
- **Tween is ephemeral (#3/#5):** the document holds the authoritative target; the sprite
  tweens toward it; tween state is never persisted/ECS. A token's image resolves through
  the M8b `AssetResolver` (UUID→URL).
- **Render-only:** no interaction/tool code in M8d-1 (that's M8d-2). Tokens are placed/moved
  via direct document intents in tests + (M8d-2) the place/move tools.
- **No raw `console.log`; commit per task; do NOT push** (push is the M8d-milestone gate).

---

### Task 1: `TokenAnimator` (pure tween) + token value types

**Files:**
- Modify: `src/client/render/src/types.ts` (add `TokenTransform`, `TokenNodeSpec`)
- Create: `src/client/render/src/token-animator.ts`
- Modify: `src/client/render/src/index.ts` (export the types + `TokenAnimator`)
- Test: `src/client/render/src/token-animator.test.ts`

**Interfaces:**
- Produces:
  - `interface TokenTransform { x: number; y: number; rotation: number }`
  - `interface TokenNodeSpec { x: number; y: number; w: number; h: number; rotation: number; url: string }`
  - `class TokenAnimator`: `has(id): boolean`, `get(id): TokenTransform | undefined`,
    `setTarget(id, t: TokenTransform): void` (a brand-new id snaps current=target),
    `remove(id): void`, `tick(dtMs: number): string[]` (advances every tween toward its
    target by exponential smoothing; returns the ids whose current moved this tick;
    settles exactly when within ε).

- [ ] **Step 1: Write the failing test `token-animator.test.ts`**

```ts
import { test, expect } from "vitest";
import { TokenAnimator } from "./index";

test("a new token snaps to its target (appears in place, no tween)", () => {
  const a = new TokenAnimator();
  a.setTarget("t1", { x: 100, y: 50, rotation: 0 });
  expect(a.get("t1")).toEqual({ x: 100, y: 50, rotation: 0 });
  expect(a.tick(16)).toEqual([]); // already at target → nothing moves
});

test("a moved token tweens toward the new target and settles", () => {
  const a = new TokenAnimator();
  a.setTarget("t1", { x: 0, y: 0, rotation: 0 }); // initial (snap)
  a.setTarget("t1", { x: 100, y: 0, rotation: 0 }); // move
  const moved = a.tick(60); // partial advance
  expect(moved).toEqual(["t1"]);
  const mid = a.get("t1")!;
  expect(mid.x).toBeGreaterThan(0);
  expect(mid.x).toBeLessThan(100);
  a.tick(10_000); // a long tick fully settles
  expect(a.get("t1")).toEqual({ x: 100, y: 0, rotation: 0 });
});

test("remove drops the tween", () => {
  const a = new TokenAnimator();
  a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
  a.remove("t1");
  expect(a.has("t1")).toBe(false);
  expect(a.get("t1")).toBeUndefined();
});
```

- [ ] **Step 2: Run — verify fail** (`pnpm --filter @shadowcat/render test` → `TokenAnimator` not exported).

- [ ] **Step 3: Add types to `types.ts`**

```ts
/** A token's animatable transform (scene coords; `(x,y)` = center). */
export interface TokenTransform {
  x: number;
  y: number;
  rotation: number;
}

/** A resolved token render node: transform + size + resolved image URL. */
export interface TokenNodeSpec {
  x: number;
  y: number;
  w: number;
  h: number;
  rotation: number;
  url: string;
}
```

- [ ] **Step 4: Write `token-animator.ts`**

```ts
import type { TokenTransform } from "./types";

/** Exponential-smoothing factor: a tick of `SMOOTH_MS` (or more) fully settles. */
const SMOOTH_MS = 120;
/** Below this distance a component snaps exactly to target (kills float drift). */
const EPSILON = 0.01;

const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;
const near = (a: TokenTransform, b: TokenTransform): boolean =>
  Math.abs(a.x - b.x) < EPSILON && Math.abs(a.y - b.y) < EPSILON && Math.abs(a.rotation - b.rotation) < EPSILON;

/** Pure tween model: holds each token's current rendered transform and advances it toward
 * the document-authoritative target. New tokens snap (no tween); moves smooth in. */
export class TokenAnimator {
  private cur = new Map<string, TokenTransform>();
  private tgt = new Map<string, TokenTransform>();

  has(id: string): boolean {
    return this.cur.has(id);
  }
  get(id: string): TokenTransform | undefined {
    return this.cur.get(id);
  }
  setTarget(id: string, t: TokenTransform): void {
    if (!this.cur.has(id)) this.cur.set(id, { ...t }); // brand-new → snap into place
    this.tgt.set(id, { ...t });
  }
  remove(id: string): void {
    this.cur.delete(id);
    this.tgt.delete(id);
  }
  /** Advance all tweens by `dtMs`; return ids whose current transform changed. */
  tick(dtMs: number): string[] {
    const moved: string[] = [];
    const alpha = Math.min(1, dtMs / SMOOTH_MS);
    for (const [id, c] of this.cur) {
      const t = this.tgt.get(id);
      if (!t || near(c, t)) continue;
      c.x = lerp(c.x, t.x, alpha);
      c.y = lerp(c.y, t.y, alpha);
      c.rotation = lerp(c.rotation, t.rotation, alpha);
      if (near(c, t)) { c.x = t.x; c.y = t.y; c.rotation = t.rotation; } // settle exactly
      moved.push(id);
    }
    return moved;
  }
}
```

- [ ] **Step 5: Export from `index.ts`** (append)

```ts
export type { TokenTransform, TokenNodeSpec } from "./types";
export { TokenAnimator } from "./token-animator";
```

- [ ] **Step 6: Run — verify pass** (`pnpm --filter @shadowcat/render test`).

- [ ] **Step 7: Commit**

```bash
git add src/client/render/src
git commit -m "feat(m8d-1): TokenAnimator pure tween + token value types"
```

---

### Task 2: `DisplayBackend` token node API + ticker + `MockBackend`

**Files:**
- Modify: `src/client/render/src/backend.ts` (+ `setToken`, `removeToken`, `startTicker`)
- Modify: `src/client/render/src/backend.mock.ts` (record token nodes + ticker cb)
- Test: `src/client/render/src/backend.mock.test.ts` (new — the mock is now stateful enough to test)

**Interfaces:**
- Produces (on `DisplayBackend`):
  - `setToken(id: string, spec: TokenNodeSpec): void` — upsert a token node (create if new,
    update transform/size/texture if existing).
  - `removeToken(id: string): void`.
  - `startTicker(cb: (dtMs: number) => void): void` — register a per-frame callback (the
    render ticker). The mock stores it (tests invoke manually); PixiBackend hooks `app.ticker`.
  - `MockBackend.tokens: Map<string, TokenNodeSpec>` + `MockBackend.tick?: (dtMs) => void`.

- [ ] **Step 1: Write the failing test `backend.mock.test.ts`**

```ts
import { test, expect } from "vitest";
import { MockBackend } from "./index";

test("MockBackend records token upserts and removals", () => {
  const b = new MockBackend();
  b.setToken("t1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, url: "/a" });
  expect(b.tokens.get("t1")).toEqual({ x: 0, y: 0, w: 100, h: 100, rotation: 0, url: "/a" });
  b.setToken("t1", { x: 10, y: 0, w: 100, h: 100, rotation: 0, url: "/a" });
  expect(b.tokens.get("t1")!.x).toBe(10);
  b.removeToken("t1");
  expect(b.tokens.has("t1")).toBe(false);
});

test("MockBackend captures the ticker callback", () => {
  const b = new MockBackend();
  let dt = 0;
  b.startTicker((d) => { dt = d; });
  b.tick!(16);
  expect(dt).toBe(16);
});
```

- [ ] **Step 2: Run — verify fail.**

- [ ] **Step 3: Add to `DisplayBackend` (`backend.ts`)** — import `TokenNodeSpec`, add to the interface:

```ts
import type { LineSeg, CameraTransform, VisibilityInput, TokenNodeSpec } from "./types";
```
```ts
  /** Upsert a token render node (create if new; update transform/size/texture otherwise). */
  setToken(id: string, spec: TokenNodeSpec): void;
  /** Remove a token render node. */
  removeToken(id: string): void;
  /** Register the per-frame render ticker callback (drives tweens). */
  startTicker(cb: (dtMs: number) => void): void;
```

- [ ] **Step 4: Implement in `MockBackend` (`backend.mock.ts`)** — import `TokenNodeSpec`; add fields + methods:

```ts
  tokens = new Map<string, TokenNodeSpec>();
  tick: ((dtMs: number) => void) | undefined;
```
```ts
  setToken(id: string, spec: TokenNodeSpec): void {
    this.tokens.set(id, spec);
  }
  removeToken(id: string): void {
    this.tokens.delete(id);
  }
  startTicker(cb: (dtMs: number) => void): void {
    this.tick = cb;
  }
```

- [ ] **Step 5: Run — verify pass.** (Existing render tests still pass — `MockBackend` now
  satisfies the wider `DisplayBackend`.)

- [ ] **Step 6: Commit**

```bash
git add src/client/render/src
git commit -m "feat(m8d-1): DisplayBackend token node API + ticker + MockBackend"
```

---

### Task 3: `TokenView` (token reconcile + tick)

**Files:**
- Create: `src/client/render/src/token-view.ts`
- Modify: `src/client/render/src/index.ts` (export `TokenView`)
- Test: `src/client/render/src/token-view.test.ts`

**Interfaces:**
- Consumes: `DocumentStore`, `AssetResolver`, `WireDocument` (`@shadowcat/core`);
  `DisplayBackend` (Task 2); `TokenAnimator`, `TokenNodeSpec` (Task 1).
- Produces: `class TokenView` — `new TokenView(store, assets, backend)`,
  `reconcile(): void` (diff `store.query("token")` → setToken/removeToken + animator targets),
  `tick(dtMs): void` (advance + re-push moved tokens).

**Token system shape it reads** (client-owned interpretation): `system = { x, y, w, h,
rotation?, visual: { kind, asset } }`; `rotation` defaults to 0; only `kind:"image"` renders
in M8d-1.

- [ ] **Step 1: Write the failing test `token-view.test.ts`**

```ts
import { test, expect } from "vitest";
import { DocumentStore, AssetResolver } from "@shadowcat/core";
import { MockBackend, TokenView } from "./index";
import type { WireDocument } from "@shadowcat/core";

function tokenDoc(id: string, x: number, y: number, asset: string): WireDocument {
  return {
    id, scope: { kind: "world", world_id: "w1" }, doc_type: "token", schema_version: 1,
    source: null, owner: null,
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
    embedded: {}, system: { x, y, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset } },
    created_at: 0, updated_at: 0,
  };
}
const cmd = (seq: number, ops: object[]) => ({ seq, world_id: "w1", author: "a", ts: 0, ops });

test("reconcile creates a token node at its center transform with the resolved url", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: tokenDoc("t1", 100, 50, "img1") }]));
  new TokenView(store, assets, backend).reconcile();
  expect(backend.tokens.get("t1")).toEqual({ x: 100, y: 50, w: 100, h: 100, rotation: 0, url: assets.url("img1") });
});

test("a moved token tweens via tick toward the new position", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const view = new TokenView(store, new AssetResolver(), backend);
  store.applyCommand(cmd(1, [{ op: "create", doc: tokenDoc("t1", 0, 0, "img1") }]));
  view.reconcile();
  // Move the token doc.
  store.applyCommand(cmd(2, [{ op: "update", doc_id: "t1", changes: [{ path: "/system/x", old: 0, new: 100 }] }]));
  view.reconcile(); // sets the new target; current still ~0 (not snapped — existing token)
  expect(backend.tokens.get("t1")!.x).toBeLessThan(100);
  view.tick(10_000); // settle
  expect(backend.tokens.get("t1")!.x).toBe(100);
});

test("a deleted token doc removes its node", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const view = new TokenView(store, new AssetResolver(), backend);
  store.applyCommand(cmd(1, [{ op: "create", doc: tokenDoc("t1", 0, 0, "img1") }]));
  view.reconcile();
  store.applyCommand(cmd(2, [{ op: "delete", doc: tokenDoc("t1", 0, 0, "img1") }]));
  view.reconcile();
  expect(backend.tokens.has("t1")).toBe(false);
});
```

- [ ] **Step 2: Run — verify fail.**

- [ ] **Step 3: Write `token-view.ts`**

```ts
import type { DocumentStore, AssetResolver, WireDocument } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { TokenNodeSpec } from "./types";
import { TokenAnimator } from "./token-animator";

/** Engine-reserved token system fields (M8 §4.2; client-owned). `(x,y)` = center. */
interface TokenSystem {
  x: number;
  y: number;
  w: number;
  h: number;
  rotation?: number;
  visual: { kind: string; asset: string };
}

/** Renders `doc_type:"token"` docs as backend token nodes, tweening transforms via a
 * TokenAnimator. The visual (size + image) applies immediately; the transform tweens. */
export class TokenView {
  private readonly animator = new TokenAnimator();
  private readonly specs = new Map<string, TokenNodeSpec>();

  constructor(
    private readonly store: DocumentStore,
    private readonly assets: AssetResolver,
    private readonly backend: DisplayBackend,
  ) {}

  reconcile(): void {
    const seen = new Set<string>();
    for (const doc of this.store.query("token")) {
      const spec = this.toSpec(doc);
      if (!spec) continue;
      seen.add(doc.id);
      this.specs.set(doc.id, spec);
      this.animator.setTarget(doc.id, { x: spec.x, y: spec.y, rotation: spec.rotation });
      this.push(doc.id); // immediate: new tokens snapped, visual current
    }
    for (const id of [...this.specs.keys()]) {
      if (seen.has(id)) continue;
      this.specs.delete(id);
      this.animator.remove(id);
      this.backend.removeToken(id);
    }
  }

  tick(dtMs: number): void {
    for (const id of this.animator.tick(dtMs)) this.push(id);
  }

  /** Push a token to the backend with its latest visual + current (tweened) transform. */
  private push(id: string): void {
    const spec = this.specs.get(id);
    const t = this.animator.get(id);
    if (spec && t) this.backend.setToken(id, { ...spec, x: t.x, y: t.y, rotation: t.rotation });
  }

  private toSpec(doc: WireDocument): TokenNodeSpec | null {
    const s = doc.system as TokenSystem | undefined;
    if (!s || s.visual?.kind !== "image") return null; // only image tokens render in M8d-1
    return {
      x: s.x, y: s.y, w: s.w, h: s.h, rotation: s.rotation ?? 0,
      url: this.assets.url(s.visual.asset),
    };
  }
}
```

- [ ] **Step 4: Export from `index.ts`** (append)

```ts
export { TokenView } from "./token-view";
```

- [ ] **Step 5: Run — verify pass.**

- [ ] **Step 6: Commit**

```bash
git add src/client/render/src
git commit -m "feat(m8d-1): TokenView token reconcile + tween tick"
```

---

### Task 4: `RenderEngine` integration (token reconcile on store change + ticker)

**Files:**
- Modify: `src/client/render/src/engine.ts`
- Test: `src/client/render/src/engine.test.ts`

**Interfaces:**
- Consumes: `TokenView` (Task 3).
- The engine constructs a `TokenView`; `start()` reconciles tokens initially, on every store
  change, and registers the backend ticker to drive `tokenView.tick`. `destroy()` is
  unchanged (the backend's `destroy` stops its ticker).

- [ ] **Step 1: Add the failing tests** — append to `engine.test.ts`:

```ts
test("start renders existing token docs and re-reconciles on store change", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const engine = new RenderEngine({ store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 } });
  store.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id: "t1", scope: { kind: "world", world_id: "w1" }, doc_type: "token", schema_version: 1,
      source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, system: { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "i1" } }, created_at: 0, updated_at: 0,
    } }],
  });
  engine.start();
  expect(backend.tokens.has("t1")).toBe(true);
});

test("start registers the backend ticker", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const engine = new RenderEngine({ store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 } });
  engine.start();
  expect(backend.tick).toBeTypeOf("function"); // engine registered a ticker callback
});
```

- [ ] **Step 2: Run — verify fail.**

- [ ] **Step 3: Implement in `engine.ts`** — import + construct `TokenView`, wire reconcile + ticker.

Add the import:
```ts
import { TokenView } from "./token-view";
```
Add a field + construct it in the constructor (next to `this.compositor = ...`):
```ts
  private readonly tokens: TokenView;
```
```ts
    this.tokens = new TokenView(opts.store, opts.assets, opts.backend);
```
In `start()`, reconcile tokens initially, on store change, and start the ticker. Update the
existing body:
```ts
  start(): void {
    this.opts.backend.ensureLayers(this.layers.orderedIds());
    this.applyCamera();
    this.reconciler.reconcile();
    this.tokens.reconcile();
    this.unsubscribe = this.opts.store.subscribe(() => {
      this.reconciler.reconcile();
      this.tokens.reconcile();
      this.flushPendingDerived();
    });
    this.opts.backend.startTicker((dt) => this.tokens.tick(dt));
    if (this.opts.subscribeScene) {
      this.sceneSub = this.opts.subscribeScene("identity", (f) => this.onSceneFrame(f));
    }
  }
```

- [ ] **Step 4: Run — verify pass** (`pnpm --filter @shadowcat/render test`).

- [ ] **Step 5: Commit**

```bash
git add src/client/render/src/engine.ts src/client/render/src/engine.test.ts
git commit -m "feat(m8d-1): RenderEngine renders + ticks tokens"
```

---

### Task 5: `PixiBackend` token rendering + real ticker (GL)

**Files:**
- Modify: `src/client/render/src/pixi-backend.ts`

**Interfaces:**
- Implements `setToken`/`removeToken`/`startTicker` over pixi.js. Each token = a `Sprite`
  in the `tokens` layer, positioned at its center (`anchor` 0.5), sized to `w×h`, rotated;
  texture from `Assets.load(url)`. **No unit test** (GL; Playwright covers via M8d-2's place
  tool — the existing stage smoke must stay green here).

- [ ] **Step 1: Implement in `pixi-backend.ts`**

Add a field for token sprites + the ticker, and import `TokenNodeSpec`:
```ts
import type { LineSeg, CameraTransform, VisibilityInput, TokenNodeSpec } from "./types";
```
```ts
  private readonly tokens = new Map<string, Sprite>();
```
Add the methods (after `setVisibility`):
```ts
  setToken(id: string, spec: TokenNodeSpec): void {
    let sprite = this.tokens.get(id);
    if (!sprite) {
      sprite = new Sprite();
      sprite.anchor.set(0.5); // (x,y) is the token center
      this.tokens.set(id, sprite);
      this.layers.get("tokens")?.addChild(sprite);
    }
    sprite.position.set(spec.x, spec.y);
    sprite.width = spec.w;
    sprite.height = spec.h;
    sprite.angle = spec.rotation;
    const url = spec.url;
    void Assets.load(url).then((texture) => {
      // The sprite may have been removed or re-textured while loading.
      if (this.tokens.get(id) === sprite) sprite.texture = texture;
    });
  }

  removeToken(id: string): void {
    const sprite = this.tokens.get(id);
    if (sprite) { sprite.destroy(); this.tokens.delete(id); }
  }

  startTicker(cb: (dtMs: number) => void): void {
    this.app.ticker.add((ticker) => cb(ticker.deltaMS));
  }
```

- [ ] **Step 2: Typecheck** (`pnpm --filter @shadowcat/render typecheck`) — pass against pixi v8.

- [ ] **Step 3: Run the model tests** (`pnpm --filter @shadowcat/render test`) — still green;
  the backend file is not imported by them.

- [ ] **Step 4: Verify the testability invariant** — `grep -rn "pixi.js" src/client/render/src`
  → only `pixi-backend.ts`.

- [ ] **Step 5: Commit**

```bash
git add src/client/render/src/pixi-backend.ts
git commit -m "feat(m8d-1): PixiBackend token sprites + render ticker"
```

---

## Final verification (before the branch review)

- [ ] `pnpm -r typecheck` — all packages.
- [ ] `pnpm -r test` — core + render + ui unit suites green.
- [ ] `pnpm lint` — clean.
- [ ] `pnpm --filter @shadowcat/ui e2e` — the existing entry-flow + assets + stage smokes
  still pass (M8d-1 adds the ticker to the live stage; confirm it doesn't break mount/
  teardown). **No new e2e here** — token rendering is exercised in the browser once the
  M8d-2 place tool can author a token (the token-render + tween e2e rides with M8d-2, as the
  M8c-1 background-render e2e was deferred). Logged in `docs/TODO.md`.
- [ ] `grep -rn "pixi.js" src/client/render/src` → only `pixi-backend.ts`.

## Buddy-check directive

M8d-1 introduces the token render path + the tween/ticker loop that all token interaction
(M8d-2) and animation (M10) build on. Consistent with every prior M8 slice, **run a
buddy-check at the final branch review** (two blind reviewers + reconciliation debate),
focusing on: the animator tween/settle math, the TokenView reconcile diff (create/move/
destroy + the visual-vs-transform split), the engine ticker wiring + teardown, and the
PixiBackend sprite lifecycle (load race, anchor/center, destroy). Record the outcome in the
execution handoff.

## Deviation from the spec (surfaced)

- **Spec §12 lists a token-render Playwright in M8d-1.** Rendering a token in the browser
  needs a token document, which needs the M8d-2 **place tool** (no authoring UI in d-1).
  So the token reconcile/animator are **fully unit-tested in d-1**, and the browser
  token-render + tween e2e **rides with M8d-2** (alongside the deferred M8c-1
  background-render e2e). Decomposition, not descope — logged to `docs/TODO.md`.

## Spec coverage self-check

- §4 token document model (center origin, `visual` seam) → Task 1 (types) + Task 3 (`toSpec`).
- §5 reconciler + DisplayBackend node API → Tasks 2, 3, 5.
- §6 tween + render ticker → Tasks 1 (animator), 4 (engine ticker), 5 (Pixi ticker).
- §12 testability (headless model vs GL backend) → Tasks 1–4 (node) + Task 5 (Playwright via d-2).
- **Deferred to M8d-2 (correctly absent):** the interaction/tool API, place/move tools, the
  scene-tools module, drawing/template, measurement, pings.
