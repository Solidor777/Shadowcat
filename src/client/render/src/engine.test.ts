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
  onUpdate({ payload: { mode: "all" }, computedAtSeq: 0 }); // appliedSeq 0 >= 0
  expect(backend.visibility).toEqual({ mode: "all", visible: [], explored: [] }); // GM no-fog
  expect(applied).toBe(1);
});

function sceneCmd(seq: number, id: string): { seq: number; world_id: string; author: string; ts: number; ops: { op: "create"; doc: import("@shadowcat/core").WireDocument }[] } {
  return {
    seq, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id, scope: { kind: "world", world_id: "w1" }, doc_type: "scene", schema_version: 1,
      source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, parent_id: null, system: {}, created_at: 0, updated_at: 0,
    } }],
  };
}

test("a masked vision frame parses the active scene's polygons into the VisibilityInput", () => {
  const store = new DocumentStore();
  store.applyCommand(sceneCmd(1, "s1"));
  const backend = new MockBackend();
  let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
  const engine = new RenderEngine({
    store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
    subscribeScene: (_c, cb) => { onUpdate = cb; return { unsubscribe: () => {} }; },
  });
  engine.start();
  onUpdate({ payload: { mode: "masked", polygons: [{ scene: "s1", points: [0, 0, 10, 0, 10, 10] }] }, computedAtSeq: 1 });
  expect(backend.visibility).toEqual({ mode: "masked", visible: [{ points: [0, 0, 10, 0, 10, 10] }], explored: [] });
});

test("a polygon for another scene is filtered out (no cross-scene fog hole)", () => {
  const store = new DocumentStore();
  store.applyCommand(sceneCmd(1, "s1")); // active scene is s1
  const backend = new MockBackend();
  let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
  const engine = new RenderEngine({
    store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
    subscribeScene: (_c, cb) => { onUpdate = cb; return { unsubscribe: () => {} }; },
  });
  engine.start();
  // A polygon tagged for scene s2 (a token the player owns elsewhere) must not cut s1's fog.
  onUpdate({ payload: { mode: "masked", polygons: [{ scene: "s2", points: [0, 0, 10, 0, 10, 10] }] }, computedAtSeq: 1 });
  expect(backend.visibility).toEqual({ mode: "masked", visible: [], explored: [] }); // full fog, no hole
});

test("a garbled/unknown-mode vision payload fails CLOSED to full fog (not see-all)", () => {
  const store = new DocumentStore();
  store.applyCommand(sceneCmd(1, "s1"));
  const backend = new MockBackend();
  let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
  const engine = new RenderEngine({
    store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
    subscribeScene: (_c, cb) => { onUpdate = cb; return { unsubscribe: () => {} }; },
  });
  engine.start();
  // Only an explicit mode:"all" may clear fog; an unknown/garbled mode must mask everything.
  onUpdate({ payload: { mode: "wat", polygons: [{ scene: "s1", points: [0, 0, 10, 0, 10, 10] }] }, computedAtSeq: 1 });
  expect(backend.visibility).toEqual({ mode: "masked", visible: [], explored: [] });
  // A null payload likewise → full fog, never see-all.
  onUpdate({ payload: null, computedAtSeq: 2 });
  expect(backend.visibility).toEqual({ mode: "masked", visible: [], explored: [] });
});

test("a masked frame rasterizes the active scene's explored cells into dimmed-memory rects", () => {
  const store = new DocumentStore();
  store.applyCommand(sceneCmd(1, "s1")); // active scene
  const backend = new MockBackend();
  let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
  const engine = new RenderEngine({
    store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
    subscribeScene: (_c, cb) => { onUpdate = cb; return { unsubscribe: () => {} }; },
  });
  engine.start();
  onUpdate({
    payload: {
      mode: "masked",
      polygons: [{ scene: "s1", points: [0, 0, 100, 0, 100, 100, 0, 100] }],
      explored: [
        { scene: "s1", cell: 100, cells: [0, 0, 1, 0] }, // cells (0,0) and (1,0)
        { scene: "s2", cell: 100, cells: [5, 5] }, // another scene → filtered out
        { scene: "s1", cells: [9, 9] }, // missing `cell` → fail-safe, dropped
      ],
    },
    computedAtSeq: 1,
  });
  expect(backend.visibility).toEqual({
    mode: "masked",
    visible: [{ points: [0, 0, 100, 0, 100, 100, 0, 100] }],
    explored: [
      { points: [0, 0, 100, 0, 100, 100, 0, 100] }, // cell (0,0)
      { points: [100, 0, 200, 0, 200, 100, 100, 100] }, // cell (1,0)
    ],
  });
});

test("setFogPreview renders a GM no-fog frame as full fog and restores on toggle off", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
  const modes: string[] = [];
  const engine = new RenderEngine({
    store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
    subscribeScene: (_c, cb) => { onUpdate = cb; return { unsubscribe: () => {} }; },
    onDerivedApplied: (i) => { modes.push(i.mode); },
  });
  engine.start();
  // A GM frame: no fog.
  onUpdate({ payload: { mode: "all" }, computedAtSeq: 0 });
  expect(backend.visibility).toEqual({ mode: "all", visible: [], explored: [] });
  // Preview on → the same frame renders as full fog (masked, empty) without a new derived frame.
  engine.setFogPreview(true);
  expect(backend.visibility).toEqual({ mode: "masked", visible: [], explored: [] });
  // Preview off → restores no-fog.
  engine.setFogPreview(false);
  expect(backend.visibility).toEqual({ mode: "all", visible: [], explored: [] });
  expect(modes).toEqual(["all", "masked", "all"]);
});

test("setViewAsUser re-subscribes vision with as_user and resets the watermark", () => {
  const store = new DocumentStore();
  store.applyCommand(sceneCmd(1, "s1"));
  const backend = new MockBackend();
  const opts: ({ asUser?: string } | undefined)[] = [];
  let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
  let unsubs = 0;
  const engine = new RenderEngine({
    store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
    subscribeScene: (_c, cb, o) => { opts.push(o); onUpdate = cb; return { unsubscribe: () => { unsubs++; } }; },
  });
  engine.start();
  expect(opts[0]).toBeUndefined(); // own view (no as_user)
  onUpdate({ payload: { mode: "all" }, computedAtSeq: 1 });
  expect(backend.visibility).toEqual({ mode: "all", visible: [], explored: [] });

  // View as a player → the old subscription is torn down and a new one carries as_user.
  engine.setViewAsUser("u1");
  expect(unsubs).toBe(1);
  expect(opts[1]).toEqual({ asUser: "u1" });
  // The new view's first frame applies even at the SAME seq (watermark reset — a view switch is a
  // fresh stream, not a regression of the prior one).
  onUpdate({ payload: { mode: "masked", polygons: [{ scene: "s1", points: [0, 0, 10, 0, 10, 10] }] }, computedAtSeq: 1 });
  expect(backend.visibility).toEqual({ mode: "masked", visible: [{ points: [0, 0, 10, 0, 10, 10] }], explored: [] });

  // Back to "see all" (null) → re-subscribe without as_user.
  engine.setViewAsUser(null);
  expect(opts[2]).toBeUndefined();
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
  onUpdate({ payload: { mode: "all" }, computedAtSeq: 5 }); // appliedSeq 0 < 5 → deferred
  expect(backend.visibility).toBeNull();
  store.applyCommand({
    seq: 5, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id: "s1", scope: { kind: "world", world_id: "w1" }, doc_type: "scene", schema_version: 1,
      source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, parent_id: null, system: {}, created_at: 0, updated_at: 0,
    } }],
  });
  expect(backend.visibility).toEqual({ mode: "all", visible: [], explored: [] });
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
  onUpdate({ payload: { mode: "all" }, computedAtSeq: 5 }); // defer (appliedSeq 0 < 5)
  onUpdate({ payload: { mode: "all" }, computedAtSeq: 3 }); // lower seq → ignored, does not replace seq 5
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
  expect(backend.visibility).toEqual({ mode: "all", visible: [], explored: [] });
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

test("addPing renders an expanding ring driven by the ticker", () => {
  const { backend, engine } = makeEngine();
  engine.start();
  engine.addPing(5, 5);
  backend.tick!(100); // drive one frame
  expect(backend.pings).toHaveLength(1);
  expect(backend.pings[0]).toMatchObject({ x: 5, y: 5 });
  expect(backend.pings[0].alpha).toBeLessThan(1); // fading
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

test("a second pointer mid-gesture is ignored (single-pointer dispatch)", () => {
  const { backend, engine } = makeEngine();
  engine.setViewport(300, 200);
  engine.start();
  // Pointer 1 starts a camera pan.
  engine.dispatchPointerDown({ x: 0, y: 0 }, { pointerId: 1 } as PointerEvent);
  // Pointer 2 (a second finger) must not hijack the gesture or pan the camera.
  engine.dispatchPointerDown({ x: 100, y: 0 }, { pointerId: 2 } as PointerEvent);
  engine.dispatchPointerMove({ x: 200, y: 0 }, { pointerId: 2 } as PointerEvent);
  expect(backend.camera!.x).toBe(0); // only pointer 1 owns the gesture; no pan from p2
  // Pointer 1 still drives the pan.
  engine.dispatchPointerMove({ x: 40, y: 0 }, { pointerId: 1 } as PointerEvent);
  expect(backend.camera!.x).toBe(40);
});

test("switching the active tool releases the dragging latch", () => {
  const { store, backend, engine } = makeEngine();
  engine.start();
  store.applyCommand(tokenCmd(1, "t1", 0));
  engine.setDraggingToken("t1");
  engine.setActiveTool(null); // a tool swap must clear the latch
  // With dragging cleared, a move now tweens (does not snap to the new position).
  store.applyCommand({ seq: 2, world_id: "w1", author: "a", ts: 0, ops: [{ op: "update", doc_id: "t1", changes: [{ path: "/system/x", old: 0, new: 100 }] }] });
  expect(backend.tokens.get("t1")!.x).toBeLessThan(100);
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

test("start renders existing drawing and template docs", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const engine = new RenderEngine({ store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 } });
  store.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      { op: "create", doc: { id: "d1", scope: { kind: "world", world_id: "w1" }, doc_type: "drawing", schema_version: 1, source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } }, embedded: {}, parent_id: "s1", system: { shape: { kind: "freehand", points: [0, 0, 5, 5] }, stroke: { color: "#fff", width: 1 }, fill: null }, created_at: 0, updated_at: 0 } },
      { op: "create", doc: { id: "tm1", scope: { kind: "world", world_id: "w1" }, doc_type: "template", schema_version: 1, source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } }, embedded: {}, parent_id: "s1", system: { shape: { kind: "circle", x: 0, y: 0, size: 10, direction: 0 }, color: "#3388ff" }, created_at: 0, updated_at: 0 } },
    ],
  });
  engine.start();
  expect(backend.shapes.has("d1")).toBe(true);
  expect(backend.shapes.has("tm1")).toBe(true);
});

test("start renders existing wall docs into the walls layer", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const engine = new RenderEngine({ store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 } });
  store.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: { id: "wl1", scope: { kind: "world", world_id: "w1" }, doc_type: "wall", schema_version: 1, source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } }, embedded: {}, parent_id: "s1", system: { seg: { x1: 0, y1: 0, x2: 50, y2: 50 }, blocksSight: true, blocksMove: true }, created_at: 0, updated_at: 0 } }],
  });
  engine.start();
  expect(backend.shapes.get("wl1")?.layer).toBe("walls");
});

test("previewOverlay / clearOverlay forward to the backend", () => {
  const { backend, engine } = makeEngine();
  engine.previewOverlay([{ points: [0, 0, 5, 5], closed: false, stroke: { color: 0, width: 1 }, fill: null }]);
  expect(backend.overlay).toHaveLength(1);
  engine.clearOverlay();
  expect(backend.overlay).toHaveLength(0);
});

test("gridDistance delegates to the grid; drawMeasure/clearMeasure forward", () => {
  const { backend, engine } = makeEngine(); // square / 100
  expect(engine.gridDistance({ x: 0, y: 0 }, { x: 250, y: 0 })).toBe(2);
  engine.drawMeasure({ x: 0, y: 0 }, { x: 10, y: 0 }, "1");
  expect(backend.measure).toEqual({ from: { x: 0, y: 0 }, to: { x: 10, y: 0 }, label: "1" });
  engine.clearMeasure();
  expect(backend.measure).toBeNull();
});

test("setActiveTool discards an in-progress preview overlay (mid-gesture tool swap)", () => {
  const { backend, engine } = makeEngine();
  engine.previewOverlay([{ points: [0, 0, 5, 5], closed: false, stroke: null, fill: null }]);
  expect(backend.overlay).toHaveLength(1);
  engine.setActiveTool(null);
  expect(backend.overlay).toHaveLength(0);
});

test("setActiveTool also clears a stranded measure overlay", () => {
  const { backend, engine } = makeEngine();
  engine.drawMeasure({ x: 0, y: 0 }, { x: 10, y: 0 }, "1");
  expect(backend.measure).not.toBeNull();
  engine.setActiveTool(null);
  expect(backend.measure).toBeNull();
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

test("the lighting layer is in the core z-order between templates and mask", () => {
  const { backend, engine } = makeEngine();
  engine.start();
  const li = backend.layers.indexOf("lighting");
  expect(li).toBeGreaterThan(backend.layers.indexOf("templates"));
  expect(li).toBeLessThan(backend.layers.indexOf("mask"));
});

test("applying a derived frame drives the lighting overlay; GM clears it", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
  const engine = new RenderEngine({
    store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
    subscribeScene: (_c, cb) => { onUpdate = cb; return { unsubscribe: () => {} }; },
  });
  engine.start();
  store.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id: "s1", scope: { kind: "world", world_id: "w1" }, doc_type: "scene", schema_version: 1,
      source: null, owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, parent_id: null, system: { grid: { kind: "square", size: 100 } }, created_at: 0, updated_at: 0,
    } }],
  });
  onUpdate({ payload: {
    mode: "masked", polygons: [], bands: [{ name: "bright", min: 0.67 }, { name: "dim", min: 0.34 }, { name: "dark", min: 0 }],
    renderHints: ["desaturate"], lit: [{ scene: "s1", cell: 100, cells: [0, 0, 2, 0, 0] }],
  }, computedAtSeq: 1 });
  backend.tick?.(1000); // settle the fade
  expect(backend.lighting!.cells.length).toBe(1);
  expect(backend.lighting!.cells[0].desaturate).toBe(true);

  onUpdate({ payload: { mode: "all" }, computedAtSeq: 2 });
  backend.tick?.(1000);
  expect(backend.lighting!.cells).toEqual([]); // GM → no overlay
});

test("lighting is applied eagerly on a deferred fog frame; fog flush does not restart the fade", () => {
  // Guards the eager-once design: lighting (cosmetic) must not wait behind the fog watermark.
  // When fog is deferred (computedAtSeq > store.appliedSeq), lighting must already be applied.
  // When the store advances and fog flushes, lighting must NOT receive a second setTarget call
  // (which would reset prev+elapsed and cause a visible stutter).
  const store = new DocumentStore();
  const backend = new MockBackend();
  let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
  const engine = new RenderEngine({
    store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
    subscribeScene: (_c, cb) => { onUpdate = cb; return { unsubscribe: () => {} }; },
  });
  engine.start();
  store.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id: "s1", scope: { kind: "world", world_id: "w1" }, doc_type: "scene", schema_version: 1,
      source: null, owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, parent_id: null, system: { grid: { kind: "square", size: 100 } }, created_at: 0, updated_at: 0,
    } }],
  });
  // Send a masked frame at seq=5 (store is at seq=1 → fog deferred, computedAtSeq 5 > 1).
  onUpdate({ payload: {
    mode: "masked", polygons: [], bands: [{ name: "bright", min: 0.67 }],
    renderHints: [],
    lit: [{ scene: "s1", cell: 100, cells: [3, 4, 0, 0, 0] }],
  }, computedAtSeq: 5 });
  // Lighting must already be applied (eager), even though fog is deferred.
  backend.tick?.(1000); // settle the lighting fade
  expect(backend.lighting!.cells.length).toBe(1); // lighting overlay present
  expect(backend.lighting!.cells[0]).toMatchObject({ i: 3, j: 4 });
  // Fog is still deferred — visibility not yet applied.
  expect(backend.visibility).toBeNull();
  // Advance the store past seq=5 so the fog flushes.
  store.applyCommand({
    seq: 5, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id: "d5", scope: { kind: "world", world_id: "w1" }, doc_type: "scene", schema_version: 1,
      source: null, owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, parent_id: null, system: {}, created_at: 0, updated_at: 0,
    } }],
  });
  // Fog is now flushed.
  expect(backend.visibility).not.toBeNull();
  // Lighting cells must be unchanged: flush must NOT have called setTarget again (no fade restart).
  expect(backend.lighting!.cells.length).toBe(1);
  expect(backend.lighting!.cells[0]).toMatchObject({ i: 3, j: 4 });
});

test("toLighting parses lit cells for the active scene and fails safe", () => {
  const { store, engine } = makeEngine();
  engine.start();
  // Seed an active scene "s1" (mirror the scene-create command in the fog tests).
  store.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id: "s1", scope: { kind: "world", world_id: "w1" }, doc_type: "scene", schema_version: 1,
      source: null, owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, parent_id: null, system: { grid: { kind: "square", size: 100 } }, created_at: 0, updated_at: 0,
    } }],
  });
  const li = engine.toLightingForTest({
    mode: "masked", bands: [{ name: "bright", min: 0.67 }, { name: "dim", min: 0.34 }, { name: "dark", min: 0 }],
    renderHints: ["desaturate"],
    lit: [
      { scene: "s1", cell: 100, cells: [0, 0, 2, 0, 0] },      // active: dark band, hint "desaturate"
      { scene: "other", cell: 100, cells: [9, 9, 0, 0, -1] },  // other scene: dropped
    ],
  });
  expect(li).not.toBeNull();
  expect(li!.cell).toBe(100);
  expect(li!.cells).toEqual([{ i: 0, j: 0, band: 2, tint: 0, hint: 0 }]);
  expect(li!.hints).toEqual(["desaturate"]);
  expect(li!.bands).toEqual([{ name: "bright", min: 0.67 }, { name: "dim", min: 0.34 }, { name: "dark", min: 0 }]);
  // GM / garbled → null (cosmetic, no overlay).
  expect(engine.toLightingForTest({ mode: "all" })).toBeNull();
  expect(engine.toLightingForTest({ mode: "masked", lit: "garbage" })).toBeNull();
  expect(engine.toLightingForTest(null)).toBeNull();
});
