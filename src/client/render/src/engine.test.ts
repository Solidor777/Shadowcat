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
      embedded: {}, system: { background: "u1" }, created_at: 0, updated_at: 0,
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
      embedded: {}, system: { background: "u1" }, created_at: 0, updated_at: 0,
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
