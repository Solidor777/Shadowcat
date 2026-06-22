# M8c-2 — Render-Layer API + Vision-Mask Spike Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: execute with **`mainline-plan-execution`**
> (inline enumerative per-task spec-compliance check + ONE dispatched fresh-context
> branch review at the end; offer a buddy-check at that review). NOT
> subagent-driven-development / executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Complete the M8c render-layer public API and prove the M9 vision path
end-to-end with an **identity** mask: wire the client half of M8a's `SceneDerived`
channel (`WsClient.subscribeScene` → `WorldSession` auto-resubscribe → `AppContext`),
feed it through a watermark-gated `Compositor` into the engine-owned mask slot, formalize
the module-facing render API (+ a typed shader-filter seam), and re-audit canvas chrome
tokens.

**Architecture:** `WsClient.subscribeScene` mirrors `subscribeSearch` (correlated by
`request_id`, resolves on first frame, dropped on disconnect). `WorldSession` owns scene
subscriptions and **re-establishes them on every `Welcome`** so derived state survives a
reconnect (the §12 open item, decided: auto-resubscribe). The `RenderEngine` consumes an
injected `subscribeScene`, subscribes to M8a's existing `"identity"` channel, and applies
each frame **only once `store.appliedSeq >= computed_at_seq`** (the §2 watermark), routing
it through a `Compositor` to `DisplayBackend.setVisibility`. M8 identity = empty `visible`
⇒ transparent overlay (the engine-owned fog shader + render target are M9). The render
model stays Pixi-free; `PixiBackend` is the only GL file.

**Tech Stack:** TypeScript (strict, bundler), `pixi.js ^8`, Svelte 5 (runes), Vitest
(node for the render model + core; jsdom for the Svelte host), Playwright, pnpm workspaces.

## Global Constraints

- **Testability invariant:** every file under `src/client/render/src/` EXCEPT
  `pixi-backend.ts` MUST NOT import `pixi.js`. Verify with
  `grep -rn "pixi.js" src/client/render/src` → only `pixi-backend.ts`.
- **No transport in `@shadowcat/render`:** the engine consumes an injected
  `subscribeScene` function (a plain signature); it never imports `WsClient`/core
  transport. Core types it may import: `DocumentStore`, `AssetResolver`, `WireDocument`.
- **Watermark (#2):** a derived frame is applied only when `store.appliedSeq >=
  computed_at_seq`; earlier frames are deferred and flushed when the store advances. A
  mask never precedes the document events it derives from.
- **Auto-resubscribe (decided §12):** `WorldSession` re-establishes every active scene
  subscription on each `Welcome` (the `WsClient` drops them on disconnect). Vision must
  survive a reconnect.
- **Identity only (no M9):** no raycasting, no fog shader, no persisted explored mask, no
  GM vision mode. `toVisibility(payload)` returns `{ visible: [] }` for M8; the GL overlay
  is transparent. The render-target + shader land in M9 behind the same `Compositor`/mask
  slot with zero API change.
- **Shader-filter seam:** a typed, module-facing extension point with **no M8 consumer**
  (token fx / Phase-3 VFX are the future consumers); opaque filter type at the public
  boundary.
- **No raw `console.log`** in client code; no debug code. Commit per task; do NOT push
  (push is the full-M8c gate — after this lands, M8c is complete).
- **0.x / unfrozen** render API, like M7.

---

### Task 1: `WsClient.subscribeScene` (client half of the SceneDerived channel)

**Files:**
- Modify: `src/client/core/src/ws-client.ts`
- Modify: `src/client/core/src/index.ts` (export `SceneFrame`, `SceneSubscription`)
- Test: `src/client/core/src/ws-client.test.ts`

**Interfaces:**
- Produces:
  - `interface SceneFrame { payload: unknown; computedAtSeq: number }`
  - `interface SceneSubscription { unsubscribe(): void }`
  - `WsClient.subscribeScene(channel: string, onUpdate: (frame: SceneFrame) => void,
    opts?: { timeoutMs?: number }): Promise<SceneSubscription>` — resolves on the first
    `scene_derived` for the request; fires `onUpdate` per frame; rejects on `scene_error`,
    timeout, or no transport; dropped on disconnect.
- Consumes: the `scene_subscribe`/`scene_unsubscribe` client frames + `scene_derived`/
  `scene_error` server frames already defined in `wire.ts`.

- [ ] **Step 1: Write the failing tests** — append to `ws-client.test.ts`:

```ts
it("subscribeScene fires onUpdate on each scene_derived; unsubscribe stops dispatch", async () => {
  const sent: string[] = [];
  let onMessage: (d: string) => void = () => {};
  const client = new WsClient({
    connect: (h) => { onMessage = h.onMessage; return Promise.resolve({ send: (d) => sent.push(d), close: () => {} }); },
    handlers: noop,
  });
  await client.start();
  const frames: Array<{ payload: unknown; computedAtSeq: number }> = [];
  const p = client.subscribeScene("identity", (f) => frames.push(f));
  const req = JSON.parse(sent.find((s) => JSON.parse(s).type === "scene_subscribe")!);
  expect(req.channel).toBe("identity");
  onMessage(JSON.stringify({ type: "scene_derived", request_id: req.request_id, channel: "identity", computed_at_seq: 3, payload: { entity_count: 0 } }));
  const handle = await p;
  expect(frames).toEqual([{ payload: { entity_count: 0 }, computedAtSeq: 3 }]);
  onMessage(JSON.stringify({ type: "scene_derived", request_id: req.request_id, channel: "identity", computed_at_seq: 4, payload: { entity_count: 2 } }));
  expect(frames).toHaveLength(2);
  handle.unsubscribe();
  expect(sent.some((s) => JSON.parse(s).type === "scene_unsubscribe")).toBe(true);
  onMessage(JSON.stringify({ type: "scene_derived", request_id: req.request_id, channel: "identity", computed_at_seq: 5, payload: {} }));
  expect(frames).toHaveLength(2); // no dispatch after unsubscribe
});

it("subscribeScene rejects on a scene_error frame", async () => {
  const sent: string[] = [];
  let onMessage: (d: string) => void = () => {};
  const client = new WsClient({
    connect: (h) => { onMessage = h.onMessage; return Promise.resolve({ send: (d) => sent.push(d), close: () => {} }); },
    handlers: noop,
  });
  await client.start();
  const p = client.subscribeScene("nope", () => {});
  const req = JSON.parse(sent.find((s) => JSON.parse(s).type === "scene_subscribe")!);
  onMessage(JSON.stringify({ type: "scene_error", request_id: req.request_id, message: "unknown channel" }));
  await expect(p).rejects.toThrow(/unknown channel/);
});

it("subscribeScene rejects immediately with no live transport", async () => {
  const client = new WsClient({ connect: () => Promise.resolve({ send: () => {}, close: () => {} }), handlers: noop });
  await expect(client.subscribeScene("identity", () => {}, { timeoutMs: 60_000 })).rejects.toThrow(/not connected/i);
});
```

- [ ] **Step 2: Run — verify they fail**

Run: `pnpm --filter @shadowcat/core test -- ws-client`
Expected: FAIL — `subscribeScene` is not a function.

- [ ] **Step 3: Add types + maps + handlers to `ws-client.ts`**

After the `SubscriptionHandle` interface, add:

```ts
/** A SceneDerived frame delivered to a scene subscription. */
export interface SceneFrame {
  payload: unknown;
  computedAtSeq: number;
}

/** Handle to an active SceneDerived subscription. */
export interface SceneSubscription {
  unsubscribe(): void;
}
```

In the class fields (next to `subscriptions`):

```ts
  /** Active scene subscriptions, keyed by request_id (ongoing onUpdate dispatch). */
  private sceneSubs = new Map<string, (frame: SceneFrame) => void>();
  /** In-flight scene-subscribe initial promises, keyed by request_id. */
  private scenePending = new Map<
    string,
    { resolve: (s: SceneSubscription) => void; reject: (e: Error) => void; timer: ReturnType<typeof setTimeout> }
  >();
```

In `failPending`, after the `subscriptions.clear()` line, add:

```ts
    for (const p of this.scenePending.values()) {
      clearTimeout(p.timer);
      p.reject(new Error(reason));
    }
    this.scenePending.clear();
    // Scene subscriptions are bound to this socket; a reconnect does not replay them,
    // so drop them (WorldSession re-subscribes after reconnect).
    this.sceneSubs.clear();
```

In `handleFrame`'s `switch`, after the `asset_changed` case, add:

```ts
      case "scene_derived": {
        const handler = this.sceneSubs.get(msg.request_id);
        if (handler) this.safeEmit(() => handler({ payload: msg.payload, computedAtSeq: msg.computed_at_seq }));
        const init = this.scenePending.get(msg.request_id);
        if (init) {
          clearTimeout(init.timer);
          this.scenePending.delete(msg.request_id);
          init.resolve({
            unsubscribe: () => {
              this.sceneSubs.delete(msg.request_id);
              this.send({ type: "scene_unsubscribe", request_id: msg.request_id });
            },
          });
        }
        break;
      }
      case "scene_error": {
        const init = this.scenePending.get(msg.request_id);
        if (init) {
          clearTimeout(init.timer);
          this.scenePending.delete(msg.request_id);
          init.reject(new Error(msg.message));
        }
        this.sceneSubs.delete(msg.request_id);
        break;
      }
```

- [ ] **Step 4: Add the `subscribeScene` method** (next to `subscribeSearch`):

```ts
  /**
   * Subscribe to a SceneDerived channel. Resolves once the first frame arrives;
   * `onUpdate` fires for every frame. Rejects on `scene_error`, timeout, or no
   * transport. Dropped on disconnect (WorldSession re-subscribes).
   */
  subscribeScene(
    channel: string,
    onUpdate: (frame: SceneFrame) => void,
    opts: { timeoutMs?: number } = {},
  ): Promise<SceneSubscription> {
    const request_id = crypto.randomUUID();
    const timeoutMs = opts.timeoutMs ?? 10_000;
    return new Promise<SceneSubscription>((resolve, reject) => {
      if (!this.transport) {
        reject(new Error("not connected"));
        return;
      }
      this.sceneSubs.set(request_id, onUpdate);
      const timer = setTimeout(() => {
        this.scenePending.delete(request_id);
        this.sceneSubs.delete(request_id);
        reject(new Error("scene subscribe timeout"));
      }, timeoutMs);
      this.scenePending.set(request_id, { resolve, reject, timer });
      this.send({ type: "scene_subscribe", request_id, channel });
    });
  }
```

- [ ] **Step 5: Export the types — `src/client/core/src/index.ts`** (extend the
  `from "./ws-client"` type export):

```ts
export type {
  WsClientOptions,
  WsClientHandlers,
  WireWelcome,
  SearchPage,
  SubscriptionHandle,
  SceneFrame,
  SceneSubscription,
} from "./ws-client";
```

- [ ] **Step 6: Run — verify pass + no core regressions**

Run: `pnpm --filter @shadowcat/core test`
Expected: all pass (new scene tests + existing).

- [ ] **Step 7: Commit**

```bash
git add src/client/core/src
git commit -m "feat(m8c-2): WsClient.subscribeScene (client SceneDerived channel)"
```

---

### Task 2: `WorldSession` scene subscriptions (auto-resubscribe) + `AppContext`

**Files:**
- Modify: `src/client/ui/src/lib/worldSession.svelte.ts`
- Modify: `src/client/ui/src/lib/appContext.ts` (add required `subscribeScene`)
- Modify: `src/client/ui/src/lib/Table.svelte` (the live provider — wire `subscribeScene`)
- Modify: `src/client/ui/src/lib/__fixtures__/appContextTest.ts` (default)
- Modify: `src/client/ui/src/modules/core-ui/panels/__fixtures__/AssetsHarness.svelte` (literal)
- Modify: `src/client/ui/src/lib/__fixtures__/SurfaceHarness.svelte` (literal)
- Test: `src/client/ui/src/lib/worldSession.test.ts`

> **Adding a required `AppContext` field breaks every literal constructor.** There are
> four: `Table.svelte` (live), `appContextTest.ts` (fixture), `AssetsHarness.svelte`,
> `SurfaceHarness.svelte`. All four are updated below; `pnpm -r typecheck` (Step 7) is the
> backstop that no constructor was missed.

**Interfaces:**
- Produces:
  - `WorldSession.subscribeScene(channel: string, onUpdate: (f: SceneFrame) => void):
    SceneSubscription` — **synchronous** handle; manages the async `WsClient` subscription
    and re-establishes it on every `Welcome`.
  - `AppContext.subscribeScene(channel, onUpdate): SceneSubscription`.
- Consumes: `WsClient.subscribeScene`, `SceneFrame`, `SceneSubscription` (Task 1).

- [ ] **Step 1: Write the failing test** — append to `worldSession.test.ts`:

```ts
test("subscribeScene sends scene_subscribe and re-establishes on a reconnect Welcome", async () => {
  let push!: (frame: unknown) => void;
  const sent: Array<Record<string, unknown>> = [];
  const connect: Connect = (handlers) => {
    push = (frame) => handlers.onMessage(JSON.stringify(frame));
    queueMicrotask(() => push(welcomeFrame));
    return Promise.resolve({ send: (d) => sent.push(JSON.parse(d)), close: () => handlers.onClose() });
  };
  const session = new WorldSession({ selfId: "u1", connect, coreUiModule: coreUiStub, logger: silentLogger });
  await session.enter("w1");
  await vi.waitFor(() => expect(session.role).toBe("player"));

  const frames: unknown[] = [];
  session.subscribeScene("identity", (f) => frames.push(f));
  await vi.waitFor(() => expect(sent.filter((m) => m.type === "scene_subscribe")).toHaveLength(1));
  const req = sent.find((m) => m.type === "scene_subscribe")!;
  // First frame resolves the underlying ws subscription + fires onUpdate.
  push({ type: "scene_derived", request_id: req.request_id, channel: "identity", computed_at_seq: 0, payload: {} });
  await vi.waitFor(() => expect(frames).toHaveLength(1));

  // A second Welcome (reconnect) must re-establish the subscription.
  push(welcomeFrame);
  await vi.waitFor(() => expect(sent.filter((m) => m.type === "scene_subscribe")).toHaveLength(2));
});
```

- [ ] **Step 2: Run — verify it fails**

Run: `pnpm --filter @shadowcat/ui test -- worldSession`
Expected: FAIL — `subscribeScene` not a function.

- [ ] **Step 3: Implement in `worldSession.svelte.ts`**

Add the import:

```ts
import {
  // ...existing imports...
  type SceneFrame,
  type SceneSubscription,
} from "@shadowcat/core";
```

Add a field (next to `#assetListeners`):

```ts
  #sceneSubs = new Map<
    string,
    { channel: string; onUpdate: (f: SceneFrame) => void; handle: SceneSubscription | null }
  >();
```

Add the methods (next to `onAssetChanged`):

```ts
  /** Subscribe to a SceneDerived channel. Returns a synchronous handle; the
   * underlying WS subscription is (re)established on every Welcome so derived state
   * survives a reconnect. */
  subscribeScene(channel: string, onUpdate: (f: SceneFrame) => void): SceneSubscription {
    const id = crypto.randomUUID();
    const rec = { channel, onUpdate, handle: null as SceneSubscription | null };
    this.#sceneSubs.set(id, rec);
    this.#establishScene(id, rec);
    return {
      unsubscribe: () => {
        this.#sceneSubs.delete(id);
        rec.handle?.unsubscribe();
        rec.handle = null;
      },
    };
  }

  #establishScene(
    id: string,
    rec: { channel: string; onUpdate: (f: SceneFrame) => void; handle: SceneSubscription | null },
  ): void {
    const ws = this.#ws;
    if (!ws) return;
    void ws
      .subscribeScene(rec.channel, rec.onUpdate)
      .then((h) => {
        // Still registered (and the same record)? keep the handle; else drop it.
        if (this.#sceneSubs.get(id) === rec) rec.handle = h;
        else h.unsubscribe();
      })
      .catch(() => {
        // Dropped (e.g. disconnect during connect); re-established on the next Welcome.
      });
  }
```

In `#onWelcome`, after the `reconcileTopology(...)` call, add the re-establish loop:

```ts
      // Scene subscriptions are dropped by the WS on disconnect; re-establish each
      // on every (re)connect so derived state (vision) survives a reconnect. No-op
      // on the first Welcome (none registered until the render engine subscribes).
      for (const [id, rec] of this.#sceneSubs) {
        rec.handle = null;
        this.#establishScene(id, rec);
      }
```

- [ ] **Step 4: Expose on `AppContext` — `appContext.ts`**

Add the import + field:

```ts
import type { ContributionRegistry, DocumentStore, AssetResolver, SceneFrame, SceneSubscription } from "@shadowcat/core";
```
```ts
  /** Subscribe to a SceneDerived channel; the session re-establishes it across
   * reconnects. Returns a synchronous unsubscribe handle. */
  subscribeScene(channel: string, onUpdate: (f: SceneFrame) => void): SceneSubscription;
```

- [ ] **Step 5: Wire the live provider + the literal constructors**

In `Table.svelte`'s `setAppContext({...})` object, add (alongside `onAssetChanged`):
```ts
    subscribeScene: (c, cb) => session.subscribeScene(c, cb),
```
In `appContextTest.ts` (the `ctx` literal), add:
```ts
    subscribeScene: over.subscribeScene ?? (() => ({ unsubscribe() {} })),
```
In `AssetsHarness.svelte` and `SurfaceHarness.svelte`'s `setAppContext({...})` literals, add:
```ts
    subscribeScene: () => ({ unsubscribe() {} }),
```

- [ ] **Step 6: (covered by Step 5 — the fixture default)**

- [ ] **Step 7: Run — verify pass**

Run: `pnpm --filter @shadowcat/ui test -- worldSession` then `pnpm -r typecheck`
Expected: the new test passes; all packages typecheck.

- [ ] **Step 8: Commit**

```bash
git add src/client/ui/src/lib
git commit -m "feat(m8c-2): WorldSession scene subscriptions (auto-resubscribe) + AppContext"
```

---

### Task 3: `VisibilityInput` + `Compositor` + `DisplayBackend.setVisibility`

**Files:**
- Modify: `src/client/render/src/types.ts` (add `VisibilityInput`)
- Modify: `src/client/render/src/backend.ts` (+ `setVisibility`)
- Modify: `src/client/render/src/backend.mock.ts` (+ record)
- Create: `src/client/render/src/compositor.ts`
- Modify: `src/client/render/src/index.ts` (export `Compositor`, `VisibilityInput`)
- Test: `src/client/render/src/compositor.test.ts`

**Interfaces:**
- Produces:
  - `interface VisibilityInput { visible: Polygon[]; explored?: Polygon[] }`
  - `DisplayBackend.setVisibility(input: VisibilityInput): void`
  - `class Compositor` — `new Compositor(backend)`, `setVisibility(input)`, `current()`.
  - `MockBackend.visibility: VisibilityInput | null`.

- [ ] **Step 1: Write the failing test `compositor.test.ts`**

```ts
import { test, expect } from "vitest";
import { Compositor, MockBackend } from "./index";

test("setVisibility forwards to the backend and is retrievable", () => {
  const backend = new MockBackend();
  const c = new Compositor(backend);
  c.setVisibility({ visible: [] }); // identity
  expect(backend.visibility).toEqual({ visible: [] });
  expect(c.current()).toEqual({ visible: [] });

  const poly = { visible: [{ points: [0, 0, 10, 0, 10, 10] }] };
  c.setVisibility(poly);
  expect(backend.visibility).toEqual(poly);
  expect(c.current()).toEqual(poly);
});
```

- [ ] **Step 2: Run — verify it fails**

Run: `pnpm --filter @shadowcat/render test`
Expected: FAIL — `Compositor` not exported.

- [ ] **Step 3: Add `VisibilityInput` to `types.ts`**

```ts
/** Visibility for the mask slot (D-V1 polygons, scene coords). Empty `visible`
 * ⇒ identity (everything visible → transparent overlay). `explored` is M9 (D-V2). */
export interface VisibilityInput {
  visible: Polygon[];
  explored?: Polygon[];
}
```

- [ ] **Step 4: Add `setVisibility` to `DisplayBackend` (`backend.ts`)**

```ts
import type { LineSeg, CameraTransform, VisibilityInput } from "./types";
```
Add to the interface (after `drawGrid`):
```ts
  /** Apply the visibility mask (the mask slot). Empty `visible` = identity
   * (full visibility → transparent overlay). */
  setVisibility(input: VisibilityInput): void;
```

- [ ] **Step 5: Record it in `MockBackend` (`backend.mock.ts`)**

```ts
import type { LineSeg, CameraTransform, VisibilityInput } from "./types";
```
Add the field + method:
```ts
  visibility: VisibilityInput | null = null;
```
```ts
  setVisibility(input: VisibilityInput): void {
    this.visibility = input;
  }
```

- [ ] **Step 6: Write `compositor.ts`**

```ts
import type { DisplayBackend } from "./backend";
import type { VisibilityInput } from "./types";

/** Owns the mask slot. M8 = identity (empty `visible` ⇒ transparent overlay). Feeds
 * VisibilityInput to the backend mask; M9 swaps an engine-owned fog shader + render
 * target behind this same surface with no API change. */
export class Compositor {
  private last: VisibilityInput = { visible: [] };

  constructor(private readonly backend: DisplayBackend) {}

  setVisibility(input: VisibilityInput): void {
    this.last = input;
    this.backend.setVisibility(input);
  }

  /** The last applied visibility (re-applied on resize in M9). */
  current(): VisibilityInput {
    return this.last;
  }
}
```

- [ ] **Step 7: Export — `index.ts`**

```ts
export type { Point, LineSeg, Polygon, CameraTransform, VisibilityInput } from "./types";
```
```ts
export { Compositor } from "./compositor";
```

- [ ] **Step 8: Run — verify pass** (`pnpm --filter @shadowcat/render test`). The
  `MockBackend` now satisfies `DisplayBackend` with `setVisibility`; existing tests pass.

- [ ] **Step 9: Commit**

```bash
git add src/client/render/src
git commit -m "feat(m8c-2): VisibilityInput + Compositor + DisplayBackend.setVisibility"
```

---

### Task 4: `RenderEngine` consumes `subscribeScene` (watermark-gated → Compositor)

**Files:**
- Modify: `src/client/render/src/engine.ts`
- Modify: `src/client/render/src/index.ts` (export `SubscribeScene`, `SceneSubscription`)
- Test: `src/client/render/src/engine.test.ts`

**Interfaces:**
- Produces:
  - `interface SceneSubscription { unsubscribe(): void }` (render-local; structurally
    matches core's)
  - `type SubscribeScene = (channel: string, onUpdate: (frame: { payload: unknown;
    computedAtSeq: number }) => void) => SceneSubscription`
  - `RenderEngineOpts.subscribeScene?: SubscribeScene`
  - `RenderEngineOpts.onDerivedApplied?: () => void` (host hook — sets an observable
    signal when a derived frame is applied)
  - `RenderEngine.compositor: Compositor` (public, readonly)
- Consumes: `Compositor`, `VisibilityInput` (Task 3); `DocumentStore.appliedSeq`.

**Watermark behavior:** on a frame, if `store.appliedSeq >= computedAtSeq` apply now; else
stash it and apply when the store next emits (the engine's existing store subscription).

- [ ] **Step 1: Add the failing tests** — append to `engine.test.ts`:

```ts
test("subscribeScene: an identity frame at/under the watermark applies immediately", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
  const sub = { unsubscribe: () => {} };
  let applied = 0;
  const engine = new RenderEngine({
    store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
    subscribeScene: (_c, cb) => { onUpdate = cb; return sub; },
    onDerivedApplied: () => { applied++; },
  });
  engine.start();
  onUpdate({ payload: { entity_count: 0 }, computedAtSeq: 0 }); // appliedSeq 0 >= 0
  expect(backend.visibility).toEqual({ visible: [] }); // identity
  expect(applied).toBe(1);
});

test("subscribeScene: a frame above the watermark defers until the store advances", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
  const engine = new RenderEngine({
    store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
    subscribeScene: (_c, cb) => { onUpdate = cb; return { unsubscribe: () => {} }; },
  });
  engine.start();
  onUpdate({ payload: {}, computedAtSeq: 5 }); // appliedSeq 0 < 5 → deferred
  expect(backend.visibility).toBeNull();
  // Advance the store to seq 5 → the deferred frame flushes.
  store.applyCommand({
    seq: 5, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id: "s1", scope: { kind: "world", world_id: "w1" }, doc_type: "scene", schema_version: 1,
      source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, system: {}, created_at: 0, updated_at: 0,
    } }],
  });
  expect(backend.visibility).toEqual({ visible: [] });
});

test("destroy unsubscribes the scene subscription", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  let unsubscribed = false;
  const engine = new RenderEngine({
    store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
    subscribeScene: () => ({ unsubscribe: () => { unsubscribed = true; } }),
  });
  engine.start();
  engine.destroy();
  expect(unsubscribed).toBe(true);
});
```

- [ ] **Step 2: Run — verify they fail** (`pnpm --filter @shadowcat/render test`).

- [ ] **Step 3: Implement in `engine.ts`**

Add imports + types:
```ts
import { Compositor } from "./compositor";
import type { VisibilityInput } from "./types";

/** Handle to a scene subscription (structurally matches @shadowcat/core's). */
export interface SceneSubscription {
  unsubscribe(): void;
}
/** Injected scene-subscribe function (no transport dependency in this package). */
export type SubscribeScene = (
  channel: string,
  onUpdate: (frame: { payload: unknown; computedAtSeq: number }) => void,
) => SceneSubscription;
```

Extend `RenderEngineOpts`:
```ts
  /** Injected SceneDerived subscribe (from WorldSession via AppContext). */
  subscribeScene?: SubscribeScene;
  /** Called when a derived frame is applied (host observability hook). */
  onDerivedApplied?: () => void;
```

Add fields:
```ts
  readonly compositor: Compositor;
  private sceneSub: SceneSubscription | null = null;
  private pendingDerived: { input: VisibilityInput; seq: number } | null = null;
```

In the constructor, after `this.reconciler = ...`:
```ts
    this.compositor = new Compositor(opts.backend);
```

Replace the `start()` store subscription + add the scene subscription:
```ts
  start(): void {
    this.opts.backend.ensureLayers(this.layers.orderedIds());
    this.applyCamera();
    this.reconciler.reconcile();
    this.unsubscribe = this.opts.store.subscribe(() => {
      this.reconciler.reconcile();
      this.flushPendingDerived();
    });
    if (this.opts.subscribeScene) {
      // M8a's debug channel; M9 swaps a real vision channel (polygon payload).
      this.sceneSub = this.opts.subscribeScene("identity", (f) => this.onSceneFrame(f));
    }
  }
```

Add the derived-frame methods:
```ts
  private onSceneFrame(frame: { payload: unknown; computedAtSeq: number }): void {
    const input = this.toVisibility(frame.payload);
    if (this.opts.store.appliedSeq >= frame.computedAtSeq) {
      this.applyDerived(input);
    } else {
      this.pendingDerived = { input, seq: frame.computedAtSeq }; // watermark: defer
    }
  }

  private flushPendingDerived(): void {
    const p = this.pendingDerived;
    if (p && this.opts.store.appliedSeq >= p.seq) {
      this.pendingDerived = null;
      this.applyDerived(p.input);
    }
  }

  private applyDerived(input: VisibilityInput): void {
    this.compositor.setVisibility(input);
    this.opts.onDerivedApplied?.();
  }

  /** M8 identity: any payload ⇒ full visibility. M9 parses polygon geometry. */
  private toVisibility(_payload: unknown): VisibilityInput {
    return { visible: [] };
  }
```

Update `destroy()`:
```ts
  destroy(): void {
    this.unsubscribe?.();
    this.unsubscribe = null;
    this.sceneSub?.unsubscribe();
    this.sceneSub = null;
    this.opts.backend.destroy();
  }
```

- [ ] **Step 4: Export the types — `index.ts`**

```ts
export { RenderEngine, type RenderEngineOpts, type SubscribeScene, type SceneSubscription } from "./engine";
```

- [ ] **Step 5: Run — verify pass** (`pnpm --filter @shadowcat/render test`).

- [ ] **Step 6: Commit**

```bash
git add src/client/render/src
git commit -m "feat(m8c-2): RenderEngine subscribeScene consumer (watermark-gated Compositor)"
```

---

### Task 5: `PixiBackend` mask slot + visibility overlay + typed filter seam + exports

**Files:**
- Modify: `src/client/render/src/pixi-backend.ts`
- Modify: `src/client/render/src/backend.ts` (+ `addLayerFilter`)
- Modify: `src/client/render/src/backend.mock.ts` (+ record)
- Modify: `src/client/render/src/engine.ts` (+ `registerLayerFilter`)
- Modify: `src/client/render/src/index.ts` (export the filter-seam type)
- Test: `src/client/render/src/engine.test.ts` (filter seam pass-through)

**Interfaces:**
- Produces:
  - `DisplayBackend.addLayerFilter(layerId: string, filter: unknown): () => void`
  - `RenderEngine.registerLayerFilter(layerId: string, filter: unknown): () => void`
    (the module-facing shader-filter seam; **no M8 consumer**)
  - `PixiBackend.setVisibility` — M8 identity renders a transparent (cleared) overlay in
    the `mask` layer.
- Consumes: `VisibilityInput` (Task 3), pixi.js.

**Scoping note:** M8 ships the **mask slot** (a `Graphics` in the `mask` layer) + the
identity (transparent) overlay + the data path. The viewport-sized **render target** and
the engine-owned **fog shader** are M9 (they plug into this same slot with no API change).
The filter seam is typed + functional but has no M8 caller.

- [ ] **Step 1: Add the filter-seam pass-through test** — append to `engine.test.ts`:

```ts
test("registerLayerFilter forwards to the backend and disposes", () => {
  const backend = new MockBackend();
  const engine = new RenderEngine({ store: new DocumentStore(), assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 } });
  const filter = {};
  const dispose = engine.registerLayerFilter("tokens", filter);
  expect(backend.filters).toEqual([{ layerId: "tokens", filter }]);
  dispose();
  expect(backend.filters).toEqual([]);
});
```

- [ ] **Step 2: Run — verify it fails** (`registerLayerFilter` undefined).

- [ ] **Step 3: `DisplayBackend.addLayerFilter` (`backend.ts`)** — add to the interface:

```ts
  /** Module-facing shader-filter seam: attach an opaque filter to a layer; returns a
   * dispose. No engine consumer in M8 (token fx / Phase-3 VFX are future consumers). */
  addLayerFilter(layerId: string, filter: unknown): () => void;
```

- [ ] **Step 4: `MockBackend` (`backend.mock.ts`)** — record:

```ts
  filters: Array<{ layerId: string; filter: unknown }> = [];
```
```ts
  addLayerFilter(layerId: string, filter: unknown): () => void {
    const entry = { layerId, filter };
    this.filters.push(entry);
    return () => {
      const i = this.filters.indexOf(entry);
      if (i >= 0) this.filters.splice(i, 1);
    };
  }
```

- [ ] **Step 5: `RenderEngine.registerLayerFilter` (`engine.ts`)**:

```ts
  /** Module-facing shader-filter seam (0.x). Forwards to the backend; no engine
   * consumer in M8 — the first consumers are token fx / Phase-3 VFX. */
  registerLayerFilter(layerId: string, filter: unknown): () => void {
    return this.opts.backend.addLayerFilter(layerId, filter);
  }
```

- [ ] **Step 6: `PixiBackend` (`pixi-backend.ts`)** — mask slot + setVisibility + filter:

Add a field + parent it in `ensureLayers` (next to the grid handling):
```ts
  private readonly maskOverlay = new Graphics();
```
In `ensureLayers`, in the create loop, after the grid line:
```ts
      if (id === "mask") c.addChild(this.maskOverlay);
```
Add `setVisibility` (after `drawGrid`):
```ts
  setVisibility(input: VisibilityInput): void {
    // M8 identity: empty `visible` ⇒ full visibility ⇒ transparent overlay (clear).
    // M9 draws fog occluding everything outside `visible` (+ explored), via an
    // engine-owned shader + a viewport render target plugged into this same slot.
    this.maskOverlay.clear();
    if (input.visible.length > 0) {
      // (M9) fog composition over the mask slot.
    }
  }
```
Add `addLayerFilter` (after `setVisibility`):
```ts
  addLayerFilter(layerId: string, filter: unknown): () => void {
    const c = this.layers.get(layerId);
    if (!c) return () => {};
    c.filters = [...(c.filters ?? []), filter as Filter];
    return () => {
      c.filters = (c.filters ?? []).filter((f) => f !== filter);
    };
  }
```
Add the imports:
```ts
import { Application, Container, Graphics, Sprite, Assets, type Filter } from "pixi.js";
import type { LineSeg, CameraTransform, VisibilityInput } from "./types";
```

- [ ] **Step 7: Run model tests + typecheck**

Run: `pnpm --filter @shadowcat/render test` then `pnpm --filter @shadowcat/render typecheck`
Expected: pass (filter-seam test green; PixiBackend typechecks against pixi v8).

- [ ] **Step 8: Verify the testability invariant**

Run: `grep -rn "pixi.js" src/client/render/src` → only `pixi-backend.ts`.

- [ ] **Step 9: Commit**

```bash
git add src/client/render/src
git commit -m "feat(m8c-2): PixiBackend mask slot + identity overlay + typed filter seam"
```

---

### Task 6: `Stage.svelte` — wire `subscribeScene` + observable derived signal

**Files:**
- Modify: `src/client/ui/src/modules/core-ui/panels/Stage.svelte`
- Test: `src/client/ui/src/modules/core-ui/panels/Stage.test.ts`

**Interfaces:**
- Consumes: `AppContext.subscribeScene` (Task 2); `RenderEngineOpts.subscribeScene` +
  `onDerivedApplied` (Task 4).

- [ ] **Step 1: Update the Stage test** — extend the fake backend with the new
  `DisplayBackend` methods and assert the engine subscribes. Replace the `fakeBackend`
  helper + the mount test body in `Stage.test.ts`:

```ts
function fakeBackend(): DisplayBackend & { destroyed: boolean } {
  return {
    destroyed: false,
    ensureLayers() {},
    setBackground() {},
    drawGrid() {},
    setCameraTransform() {},
    setVisibility() {},
    addLayerFilter() { return () => {}; },
    resize() {},
    destroy() { this.destroyed = true; },
  };
}

test("mounts a canvas, subscribes to the scene channel, and tears down on unmount", async () => {
  const backend = fakeBackend();
  const createBackend = vi.fn(async () => backend);
  const subscribeScene = vi.fn(() => ({ unsubscribe: () => {} }));
  const { container, unmount } = render(Stage, {
    props: { createBackend },
    context: setAppContextForTest({ subscribeScene }),
  });
  expect(container.querySelector("[data-testid='stage-canvas']")).not.toBeNull();
  await vi.waitFor(() => expect(createBackend).toHaveBeenCalledOnce());
  await vi.waitFor(() => expect(subscribeScene).toHaveBeenCalledWith("identity", expect.any(Function)));
  unmount();
  await vi.waitFor(() => expect(backend.destroyed).toBe(true));
});
```

- [ ] **Step 2: Run — verify it fails** (`pnpm --filter @shadowcat/ui test -- Stage`) —
  `subscribeScene` not yet passed to the engine, and the fake-backend type now requires
  the new methods.

- [ ] **Step 3: Wire it in `Stage.svelte`**

Add `subscribeScene` to the context destructure:
```ts
  const { store, assets, onAssetChanged, subscribeScene } = getAppContext();
```
In the `$effect`, pass both into the engine opts (extend the existing `new RenderEngine({...})`):
```ts
      engine = new RenderEngine({
        store,
        assets,
        backend,
        grid: { kind: "square", size: 100 },
        gridColor: readColor("--grid-line", 0x363645),
        subscribeScene,
        onDerivedApplied: () => { host.dataset.sceneDerived = "1"; },
      });
```
(The engine's `sceneSub` is disposed in `engine.destroy()`, already called in cleanup —
no extra teardown here.)

- [ ] **Step 4: Run — verify pass** (`pnpm --filter @shadowcat/ui test`) + `pnpm -r typecheck`.

- [ ] **Step 5: Commit**

```bash
git add src/client/ui/src/modules/core-ui/panels
git commit -m "feat(m8c-2): Stage wires subscribeScene + data-scene-derived signal"
```

---

### Task 7: §10 token re-audit — canvas chrome color resolution + `--grid-line`

**Files:**
- Modify: `src/client/ui/src/styles/_semantic.scss` (+ `--grid-line`)
- Modify: `src/client/ui/src/modules/core-ui/panels/Stage.svelte` (`readColor` resolution fix)
- Modify: `docs/POST_WORK_FINDINGS.md` (record the audit outcome)

**Audit outcome (the deliverable):** (1) canvas chrome gets a semantic `--grid-line`
token, decoupled from UI `--border`; (2) **fix** the latent M8c-1 bug where `readColor`
read the unresolved `var(...)` string (so the grid silently used the fallback, bypassing
the theme); (3) defer fog-state colors to M9 (no visible fog in identity mode) and the
caption/`--text-sm` size token to M12 (text-dense sheets — out of canvas-chrome scope).

- [ ] **Step 1: Add the semantic token — `_semantic.scss`** (after `--border`):

```scss
  --grid-line: var(--slate-700); // canvas grid overlay (decoupled from UI --border)
```

- [ ] **Step 2: Fix `readColor` to resolve the computed color — `Stage.svelte`**

Replace the `readColor` function. `getComputedStyle().getPropertyValue("--x")` returns the
unresolved `var(...)` for an aliased custom property; resolve it by reading a computed
`color` off a probe instead:

```ts
  /** Resolve a CSS custom property (which may be a `var()` alias) to a 0xRRGGBB
   * number by reading the computed `color` off a throwaway probe — getPropertyValue
   * returns the unresolved `var(...)` string for aliased custom properties. */
  function readColor(token: string, fallback: number): number {
    if (typeof getComputedStyle !== "function" || !host) return fallback;
    const probe = document.createElement("span");
    probe.style.color = `var(${token})`;
    probe.style.display = "none";
    host.appendChild(probe);
    const rgb = getComputedStyle(probe).color; // "rgb(r, g, b)" or ""
    host.removeChild(probe);
    const m = /^rgba?\((\d+),\s*(\d+),\s*(\d+)/.exec(rgb);
    if (!m) return fallback;
    return (Number(m[1]) << 16) | (Number(m[2]) << 8) | Number(m[3]);
  }
```

(The grid already reads `--grid-line` after Task 6 Step 3; the background reads
`--surface-base` in the default `createBackend`.)

- [ ] **Step 3: Record the audit — `POST_WORK_FINDINGS.md`**

Update the existing caption finding entry: note that the M8c-2 canvas-chrome re-audit
(§10) (a) added `--grid-line` + fixed canvas color resolution, (b) confirmed the
background uses `--surface-base`, (c) defers fog-state colors to M9 and the caption/
`--text-sm` size token to M12 (text-dense sheets), since canvas chrome renders no text.

- [ ] **Step 4: Run — verify no regressions**

Run: `pnpm --filter @shadowcat/ui test` then `pnpm -r typecheck` then `pnpm lint`
Expected: all green (no test asserts the exact color; the resolution fix is covered by the
Playwright canvas render).

- [ ] **Step 5: Commit**

```bash
git add src/client/ui/src/styles src/client/ui/src/modules/core-ui/panels/Stage.svelte docs/POST_WORK_FINDINGS.md
git commit -m "fix(m8c-2): canvas chrome color resolution + --grid-line token (§10 re-audit)"
```

---

### Task 8: Playwright smoke — the identity vision-mask spike

**Files:**
- Modify: `src/client/ui/e2e/stage.spec.ts`

**Interfaces:** Consumes the served binary; M8a's `"identity"` channel pushes an initial
frame on subscribe, so entering a world drives `subscribeScene → onDerivedApplied →
data-scene-derived`.

- [ ] **Step 1: Add the spike assertion** — append a test to `stage.spec.ts`:

```ts
test("the identity SceneDerived spike reaches the mask slot", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();
  await page.getByLabel("New world name").fill("Vision Spike World");
  await page.getByRole("button", { name: "Create world" }).click();

  // Entering a world subscribes to the "identity" channel; the server pushes an
  // initial frame, the engine applies it (watermark-gated) and sets the signal.
  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });
  await expect(host).toHaveAttribute("data-scene-derived", "1", { timeout: 30_000 });
});
```

- [ ] **Step 2: Build + run the e2e suite**

Run: `pnpm --filter @shadowcat/ui e2e`
Expected: `entry-flow`, `assets`, and both `stage` tests pass against the built binary.

- [ ] **Step 3: Commit**

```bash
git add src/client/ui/e2e
git commit -m "test(m8c-2): Playwright smoke over the identity vision-mask spike"
```

---

## Final verification (before the branch review)

- [ ] `pnpm -r typecheck` — all packages.
- [ ] `pnpm -r test` — core + render + ui unit suites.
- [ ] `pnpm lint` — clean.
- [ ] `pnpm --filter @shadowcat/ui e2e` — entry-flow + assets + both stage smokes.
- [ ] `grep -rn "pixi.js" src/client/render/src` → only `pixi-backend.ts`.
- [ ] `cargo build -p shadowcat --bin shadowcat` — binary embeds the new dist/.

## Buddy-check directive

M8c-2 touches the realtime watermark path (derived-frame ordering vs the document
stream), the reconnect/re-subscribe lifecycle, and the public render API surface — all
load-bearing for M9 vision. Per M8a/M8b/M8c-1 precedent, **offer a buddy-check at the
final branch review** (two blind reviewers + reconciliation debate); record the outcome in
the execution-handoff.

## Spec coverage self-check

- §7.1 `WsClient.subscribeScene` + `WorldSession`/`AppContext` + watermark → Tasks 1, 2, 4.
- §7.1 auto-resubscribe (decided §12 open item) → Task 2.
- §7.2 `Compositor` (mask slot + identity) → Tasks 3, 5.
- §7.3 identity vision-mask spike end-to-end → Tasks 4, 6, 8.
- §6.2 module-facing API formalization (public `LayerRegistry`/`Camera`/`Grid`/
  `Compositor` exports + typed shader-filter seam) → Tasks 3, 4, 5 (exports + seam).
- §8/§10 token re-audit (canvas chrome) → Task 7 (incl. the readColor resolution fix).
- **Deferred (correctly absent):** M9 fog shader + render target + persisted/explored
  mask + GM vision; real polygon payload parsing; token/wall/etc. reconcilers (M8d).
