import { test, expect } from "vitest";
import { DocumentStore, OptimisticClient, AssetResolver } from "@shadowcat/core";
import { RenderEngine, MockBackend } from "./index";
import type { SceneTool } from "./index";

const noopTool = (over: Partial<SceneTool> = {}): SceneTool => ({
  onPointerDown: () => false,
  onPointerMove: () => {},
  onPointerUp: () => {},
  ...over,
});
const ev = {} as PointerEvent;

function tokenCmd(seq: number, id: string, x: number): { seq: number; world_id: string; author: string; ts: number; ops: { op: "create"; doc: import("@shadowcat/core").WireDocument }[] } {
  return {
    seq, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id, scope: { kind: "world", world_id: "w1" }, doc_type: "token", schema_version: 1,
      source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, parent_id: "s1", system: { x, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "i1" } }, created_at: 0, updated_at: 0,
    } }],
  };
}

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
      embedded: {}, parent_id: null, system: { background: "u1" }, created_at: 0, updated_at: 0,
    } }],
  });
  expect(backend.background).not.toBeNull();
});

test("reconcileNow re-resolves the background after an asset rev bump", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const engine = new RenderEngine({ store, assets, backend, grid: { kind: "square", size: 100 } });
  store.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id: "s1", scope: { kind: "world", world_id: "w1" }, doc_type: "scene",
      schema_version: 1, source: null, owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, parent_id: null, system: { background: "u1" }, created_at: 0, updated_at: 0,
    } }],
  });
  engine.start();
  const first = backend.background?.url;
  expect(first).toBe(assets.url("u1"));
  // An out-of-band AssetChanged(replaced) bumps the resolver rev (no store change);
  // reconcileNow must re-resolve to the cache-busted URL.
  assets.onAssetChanged({ uuid: "u1", op: "replaced" });
  engine.reconcileNow();
  expect(backend.background?.url).not.toBe(first);
  expect(backend.background?.url).toBe(assets.url("u1"));
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
      embedded: {}, parent_id: null, system: { background: "u1" }, created_at: 0, updated_at: 0,
    } }],
  });
  expect(backend.background).toBe(before); // unchanged: listener was removed
});

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
  store.applyCommand({
    seq: 5, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id: "s1", scope: { kind: "world", world_id: "w1" }, doc_type: "scene", schema_version: 1,
      source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, parent_id: null, system: {}, created_at: 0, updated_at: 0,
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

test("a lower-seq derived frame never supersedes a higher-seq pending one (latest wins)", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
  const engine = new RenderEngine({
    store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
    subscribeScene: (_c, cb) => { onUpdate = cb; return { unsubscribe: () => {} }; },
  });
  engine.start();
  onUpdate({ payload: {}, computedAtSeq: 5 }); // defer (appliedSeq 0 < 5)
  onUpdate({ payload: {}, computedAtSeq: 3 }); // lower seq → ignored, does not replace seq 5
  const create = (seq: number) => ({
    seq, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create" as const, doc: {
      id: `d${seq}`, scope: { kind: "world" as const, world_id: "w1" }, doc_type: "scene", schema_version: 1,
      source: null, owner: null, permissions: { default: "observer" as const, users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, parent_id: null, system: {}, created_at: 0, updated_at: 0,
    } }],
  });
  store.applyCommand(create(3)); // appliedSeq 3 < pending 5 → no flush
  expect(backend.visibility).toBeNull();
  store.applyCommand(create(5)); // appliedSeq 5 >= 5 → the seq-5 frame flushes
  expect(backend.visibility).toEqual({ visible: [] });
});

test("a frame at/below the last-applied seq is ignored (no regression)", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
  let applied = 0;
  const engine = new RenderEngine({
    store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
    subscribeScene: (_c, cb) => { onUpdate = cb; return { unsubscribe: () => {} }; },
    onDerivedApplied: () => { applied++; },
  });
  engine.start();
  onUpdate({ payload: {}, computedAtSeq: 0 }); // appliedSeq 0 >= 0 → apply, lastApplied=0
  expect(applied).toBe(1);
  onUpdate({ payload: {}, computedAtSeq: 0 }); // <= lastApplied → ignored
  expect(applied).toBe(1);
});

test("start renders existing token docs and re-reconciles on store change", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const engine = new RenderEngine({ store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 } });
  store.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id: "t1", scope: { kind: "world", world_id: "w1" }, doc_type: "token", schema_version: 1,
      source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, parent_id: null, system: { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "i1" } }, created_at: 0, updated_at: 0,
    } }],
  });
  engine.start();
  expect(backend.tokens.has("t1")).toBe(true);
});

test("reconcileNow re-resolves token images (AssetChanged path)", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const engine = new RenderEngine({ store, assets, backend, grid: { kind: "square", size: 100 } });
  store.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id: "t1", scope: { kind: "world", world_id: "w1" }, doc_type: "token", schema_version: 1,
      source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, parent_id: null, system: { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "i1" } }, created_at: 0, updated_at: 0,
    } }],
  });
  engine.start();
  const before = backend.tokens.get("t1")!.url;
  assets.onAssetChanged({ uuid: "i1", op: "replaced" }); // cache-bust, no store change
  engine.reconcileNow();
  expect(backend.tokens.get("t1")!.url).not.toBe(before);
  expect(backend.tokens.get("t1")!.url).toBe(assets.url("i1"));
});

test("start registers the backend ticker", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const engine = new RenderEngine({ store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 } });
  engine.start();
  expect(backend.tick).toBeTypeOf("function"); // engine registered a ticker callback
});

test("setActiveTool routes a scene-coord pointerdown to the tool; handled suppresses pan", () => {
  const { backend, engine } = makeEngine();
  engine.setViewport(300, 200);
  engine.start(); // identity camera: screen == scene
  const seen: Array<{ x: number; y: number }> = [];
  engine.setActiveTool(noopTool({ onPointerDown: (p) => { seen.push(p); return true; } }));
  const cam = backend.camera;
  engine.dispatchPointerDown({ x: 50, y: 60 }, ev);
  engine.dispatchPointerMove({ x: 90, y: 60 }, ev);
  expect(seen[0]).toEqual({ x: 50, y: 60 }); // scene coords
  expect(backend.camera).toBe(cam); // tool owned the gesture → camera untouched
});

test("a tool that does not handle pointerdown falls back to camera pan", () => {
  const { backend, engine } = makeEngine();
  engine.setViewport(300, 200);
  engine.start();
  engine.setActiveTool(noopTool()); // onPointerDown returns false
  engine.dispatchPointerDown({ x: 0, y: 0 }, ev);
  engine.dispatchPointerMove({ x: 40, y: 0 }, ev);
  expect(backend.camera!.x).toBe(40); // panned by the screen delta
});

test("snap delegates to the active grid; setGrid changes it", () => {
  const { engine } = makeEngine(); // square / 100
  expect(engine.snap({ x: 140, y: 160 })).toEqual({ x: 150, y: 150 });
  engine.setGrid({ kind: "square", size: 50 });
  expect(engine.snap({ x: 140, y: 160 })).toEqual({ x: 125, y: 175 });
});

test("setDraggingToken makes a moved token snap instead of tween", () => {
  const { store, backend, engine } = makeEngine();
  engine.start();
  store.applyCommand(tokenCmd(1, "t1", 0));
  engine.setDraggingToken("t1");
  store.applyCommand({ seq: 2, world_id: "w1", author: "a", ts: 0, ops: [{ op: "update", doc_id: "t1", changes: [{ path: "/system/x", old: 0, new: 100 }] }] });
  expect(backend.tokens.get("t1")!.x).toBe(100); // snapped, no tween lag
});

test("renders documents from an optimistic source (predicted, unconfirmed)", () => {
  const oc = new OptimisticClient("u1");
  const backend = new MockBackend();
  const engine = new RenderEngine({ store: oc, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 } });
  engine.start();
  // A predicted create with no authoritative command behind it must still render.
  oc.applyIntent("i1", [{ op: "create", doc: {
    id: "t1", scope: { kind: "world", world_id: "w1" }, doc_type: "token", schema_version: 1,
    source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
    embedded: {}, parent_id: "s1", system: { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "i1" } }, created_at: 0, updated_at: 0,
  } }]);
  expect(backend.tokens.has("t1")).toBe(true);
});

test("registerLayerFilter forwards to the backend and disposes", () => {
  const backend = new MockBackend();
  const engine = new RenderEngine({ store: new DocumentStore(), assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 } });
  const filter = {};
  const dispose = engine.registerLayerFilter("tokens", filter);
  expect(backend.filters).toEqual([{ layerId: "tokens", filter }]);
  dispose();
  expect(backend.filters).toEqual([]);
});
