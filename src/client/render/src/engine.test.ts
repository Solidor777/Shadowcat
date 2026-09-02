import { test, expect, describe, it } from "vitest";
import { DocumentStore, OptimisticClient, AssetResolver, buildSceneDoc, buildTokenDoc } from "@shadowcat/core";
import { RenderEngine, MockBackend } from "./index";
import type { SceneTool } from "./index";
import type { FootprintLookup } from "@shadowcat/core";

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
      id, scope: { kind: "world", world_id: "w1" }, doc_type: "token", schema_version: 1, name: null,
      source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
      embedded: {}, parent_id: "s1", engine: { x, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "i1" }, actor_id: null, overrides: null, face: null }, system: {}, created_at: 0, updated_at: 0,
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
      schema_version: 1, name: null, source: null, owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
      embedded: {}, parent_id: null, engine: { grid: { kind: "square", size: 100, distance: null }, background: "u1", bounds: null, snapToGrid: null, vision: null, lighting: null }, system: {}, created_at: 0, updated_at: 0,
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
      schema_version: 1, name: null, source: null, owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
      embedded: {}, parent_id: null, engine: { grid: { kind: "square", size: 100, distance: null }, background: "u1", bounds: null, snapToGrid: null, vision: null, lighting: null }, system: {}, created_at: 0, updated_at: 0,
    } }],
  });
  engine.start();
  const first = backend.background?.url;
  expect(first).toBe(assets.url("u1"));
  // An out-of-band AssetChanged(replaced) bumps the resolver rev (no store change);
  // reconcileNow must re-resolve to the cache-busted URL.
  assets.onAssetChanged({ uuid: "u1", op: "replaced", version: 1 });
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
      schema_version: 1, name: null, source: null, owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
      embedded: {}, parent_id: null, engine: { grid: { kind: "square", size: 100, distance: null }, background: "u1", bounds: null, snapToGrid: null, vision: null, lighting: null }, system: {}, created_at: 0, updated_at: 0,
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
      id, scope: { kind: "world", world_id: "w1" }, doc_type: "scene", schema_version: 1, name: null,
      source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
      embedded: {}, parent_id: null, engine: { grid: { kind: "square", size: 100, distance: null }, background: null, bounds: null, snapToGrid: null, vision: null, lighting: null }, system: {}, created_at: 0, updated_at: 0,
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

// Regression: on a hex scene the server sends explored cells as axial (q,r) — `HexGrid`'s
// `axial_to_pixel` places cell (1,1) at x=150√3≈259.81, y=150, NOT the square position
// x=1*100=100, y=1*100=100 a naive `x=i*size, y=j*size` rasterization would paint. Pins the
// axial rasterization at `cellsToRects`'s own wiring site (via `toVisibility`), not just
// `Grid.cellVertices` in isolation — this test fails if `cellsToRects` indexes by `size` alone.
test("a masked frame rasterizes a hex scene's explored cells at their true axial position, not a square index", () => {
  const store = new DocumentStore();
  store.applyCommand(sceneCmd(1, "s1"));
  const backend = new MockBackend();
  let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
  const engine = new RenderEngine({
    store, assets: new AssetResolver(), backend, grid: { kind: "hex", size: 100 },
    subscribeScene: (_c, cb) => { onUpdate = cb; return { unsubscribe: () => {} }; },
  });
  engine.start();
  onUpdate({
    payload: {
      mode: "masked",
      polygons: [],
      explored: [{ scene: "s1", cell: 100, cells: [1, 1] }], // hex cell axial (q=1, r=1)
    },
    computedAtSeq: 1,
  });
  const rects = backend.visibility!.explored;
  expect(rects.length).toBe(1);
  // Ground truth: the true hex (1,1) center, computed independently from the axial formula
  // (`axialToPixel`: x = size*(√3q + √3/2 r), y = size*1.5r), NOT restated from `Grid`.
  const expectedCenterX = 100 * (Math.sqrt(3) * 1 + (Math.sqrt(3) / 2) * 1);
  const expectedCenterY = 100 * 1.5 * 1;
  const pts = rects[0].points;
  expect(pts.length).toBe(12); // 6 hex corners
  let cx = 0, cy = 0;
  for (let k = 0; k + 1 < pts.length; k += 2) { cx += pts[k]; cy += pts[k + 1]; }
  cx /= 6; cy /= 6;
  expect(cx).toBeCloseTo(expectedCenterX, 6);
  expect(cy).toBeCloseTo(expectedCenterY, 6);
  // Witness the bug directly: the square-position formula would have centered this at (100,100).
  expect(cx).not.toBeCloseTo(100, 1);
  expect(cy).not.toBeCloseTo(100, 1);
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
      id: "s1", scope: { kind: "world", world_id: "w1" }, doc_type: "scene", schema_version: 1, name: null,
      source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
      embedded: {}, parent_id: null, engine: { grid: { kind: "square", size: 100, distance: null }, background: null, bounds: null, snapToGrid: null, vision: null, lighting: null }, system: {}, created_at: 0, updated_at: 0,
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
      id: `d${seq}`, scope: { kind: "world" as const, world_id: "w1" }, doc_type: "scene", schema_version: 1, name: null,
      source: null, owner: null, permissions: { default: "observer" as const, users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
      embedded: {}, parent_id: null, engine: { grid: { kind: "square", size: 100, distance: null }, background: null, bounds: null, snapToGrid: null, vision: null, lighting: null }, system: {}, created_at: 0, updated_at: 0,
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
      id: "t1", scope: { kind: "world", world_id: "w1" }, doc_type: "token", schema_version: 1, name: null,
      source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
      embedded: {}, parent_id: null, engine: { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "i1" }, actor_id: null, overrides: null, face: null }, system: {}, created_at: 0, updated_at: 0,
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
      id: "t1", scope: { kind: "world", world_id: "w1" }, doc_type: "token", schema_version: 1, name: null,
      source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
      embedded: {}, parent_id: null, engine: { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "i1" }, actor_id: null, overrides: null, face: null }, system: {}, created_at: 0, updated_at: 0,
    } }],
  });
  engine.start();
  const before = backend.tokens.get("t1")!.visual;
  assets.onAssetChanged({ uuid: "i1", op: "replaced", version: 1 }); // cache-bust, no store change
  engine.reconcileNow();
  expect(backend.tokens.get("t1")!.visual).not.toEqual(before);
  expect((backend.tokens.get("t1")!.visual as { kind: "image"; url: string }).url).toBe(assets.url("i1"));
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

test("addEmote anchors above the token at fire time, driven by the ticker; unknown ids drop", () => {
  const { store, backend, engine } = makeEngine();
  store.applyCommand(tokenCmd(1, "t1", 50)); // center (50, 0), extent 100×100
  engine.start();
  engine.addEmote("t1", "😀");
  backend.tick!(100); // drive one frame
  expect(backend.emotes).toHaveLength(1);
  expect(backend.emotes[0].emote).toBe("😀");
  // Top-center of the token box: center x, top edge y (0 − 100/2 = −50), already rising.
  expect(backend.emotes[0].x).toBe(50);
  expect(backend.emotes[0].y).toBeLessThan(-50);
  expect(backend.emotes[0].alpha).toBeLessThan(1); // fading

  engine.addEmote("unknown-token", "🔥"); // no overlay for an id this viewer cannot resolve
  backend.tick!(100);
  expect(backend.emotes).toHaveLength(1);
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

test("setSnapEnabled(false) makes snap identity; true restores the active grid's snap", () => {
  const { engine } = makeEngine(); // square / 100
  expect(engine.snap({ x: 140, y: 160 })).toEqual({ x: 150, y: 150 }); // default: enabled
  engine.setSnapEnabled(false);
  expect(engine.snap({ x: 140, y: 160 })).toEqual({ x: 140, y: 160 }); // identity
  engine.setSnapEnabled(true);
  expect(engine.snap({ x: 140, y: 160 })).toEqual({ x: 150, y: 150 }); // restored
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
  store.applyCommand({ seq: 2, world_id: "w1", author: "a", ts: 0, ops: [{ op: "update", doc_id: "t1", changes: [{ path: "/engine/x", old: 0, new: 100 }] }] });
  expect(backend.tokens.get("t1")!.x).toBeLessThan(100);
});

test("setDraggingToken makes a moved token snap instead of tween", () => {
  const { store, backend, engine } = makeEngine();
  engine.start();
  store.applyCommand(tokenCmd(1, "t1", 0));
  engine.setDraggingToken("t1");
  store.applyCommand({ seq: 2, world_id: "w1", author: "a", ts: 0, ops: [{ op: "update", doc_id: "t1", changes: [{ path: "/engine/x", old: 0, new: 100 }] }] });
  expect(backend.tokens.get("t1")!.x).toBe(100); // snapped, no tween lag
});

test("renders documents from an optimistic source (predicted, unconfirmed)", () => {
  const oc = new OptimisticClient("u1");
  const backend = new MockBackend();
  const engine = new RenderEngine({ store: oc, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 } });
  engine.start();
  // A predicted create with no authoritative command behind it must still render.
  oc.applyIntent("i1", [{ op: "create", doc: {
    id: "t1", scope: { kind: "world", world_id: "w1" }, doc_type: "token", schema_version: 1, name: null,
    source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: {}, parent_id: "s1", engine: { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "i1" }, actor_id: null, overrides: null, face: null }, system: {}, created_at: 0, updated_at: 0,
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
      { op: "create", doc: { id: "d1", scope: { kind: "world", world_id: "w1" }, doc_type: "drawing", schema_version: 1, name: null, source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null }, embedded: {}, parent_id: "s1", engine: { shape: { kind: "freehand", points: [0, 0, 5, 5] }, stroke: { color: "#fff", width: 1 }, fill: null }, system: {}, created_at: 0, updated_at: 0 } },
      { op: "create", doc: { id: "tm1", scope: { kind: "world", world_id: "w1" }, doc_type: "template", schema_version: 1, name: null, source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null }, embedded: {}, parent_id: "s1", engine: { shape: { kind: "circle", x: 0, y: 0, size: 10, direction: 0 }, color: "#3388ff" }, system: {}, created_at: 0, updated_at: 0 } },
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
    ops: [{ op: "create", doc: { id: "wl1", scope: { kind: "world", world_id: "w1" }, doc_type: "wall", schema_version: 1, name: null, source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null }, embedded: {}, parent_id: "s1", engine: { seg: { x1: 0, y1: 0, x2: 50, y2: 50 }, blocksSight: true, blocksMove: true, blocksLight: true }, system: {}, created_at: 0, updated_at: 0 } }],
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
      id: "s1", scope: { kind: "world", world_id: "w1" }, doc_type: "scene", schema_version: 1, name: null,
      source: null, owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
      embedded: {}, parent_id: null, engine: { grid: { kind: "square", size: 100, distance: null }, background: null, bounds: null, snapToGrid: null, vision: null, lighting: null }, system: {}, created_at: 0, updated_at: 0,
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
      id: "s1", scope: { kind: "world", world_id: "w1" }, doc_type: "scene", schema_version: 1, name: null,
      source: null, owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
      embedded: {}, parent_id: null, engine: { grid: { kind: "square", size: 100, distance: null }, background: null, bounds: null, snapToGrid: null, vision: null, lighting: null }, system: {}, created_at: 0, updated_at: 0,
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
      id: "d5", scope: { kind: "world", world_id: "w1" }, doc_type: "scene", schema_version: 1, name: null,
      source: null, owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
      embedded: {}, parent_id: null, engine: { grid: { kind: "square", size: 100, distance: null }, background: null, bounds: null, snapToGrid: null, vision: null, lighting: null }, system: {}, created_at: 0, updated_at: 0,
    } }],
  });
  // Fog is now flushed.
  expect(backend.visibility).not.toBeNull();
  // Lighting cells must be unchanged: flush must NOT have called setTarget again (no fade restart).
  expect(backend.lighting!.cells.length).toBe(1);
  expect(backend.lighting!.cells[0]).toMatchObject({ i: 3, j: 4 });
});

function makeEngineWithToken(id: string, pos: { x: number; y: number }) {
  const { store, backend, engine } = makeEngine();
  store.applyCommand(tokenCmd(1, id, pos.x));
  return { store, backend, engine };
}

test("animateAlongPath forwards to the token view (SceneToolHost seam)", () => {
  const { engine, backend } = makeEngineWithToken("tok1", { x: 0, y: 0 }); // mirror existing engine test setup
  engine.setGrid({ kind: "square", size: 100 });
  engine.setAnimation({ speedCellsPerSec: 6, easing: "linear" });
  engine.start();
  engine.animateAlongPath("tok1", [[0, 0], [300, 0]]);
  backend.runTicker(500); // advance the injected ticker by 500ms
  expect(backend.lastTokenX("tok1")).toBeCloseTo(300, 0);
});

// Invariant: a hex `RenderEngine` must time a token tween against `Grid.worldUnitsPerCell()`
// (the per-step distance, `size * sqrt(3)`), not `GridSpec.size` (the indexing scale/outer
// radius) — the two coincide on square grids, so a square-fixture tween test cannot distinguish
// them however it is worded. This exercises the constructor's own
// `this.tokens.setWorldUnitsPerCell(this.grid.worldUnitsPerCell())` call, the site where the grid
// kind is consumed to resolve which scale reaches the animator (an isolated `Grid`/`TokenAnimator`
// unit test proves neither unit's own arithmetic — only whether the correct value reaches the
// animator through this wiring).
test("a hex RenderEngine times a token tween against worldUnitsPerCell, not size", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const engine = new RenderEngine({ store, assets, backend, grid: { kind: "hex", size: 100 } });
  store.applyCommand(tokenCmd(1, "tok1", 0));
  engine.start();
  const worldUnitsPerCell = 100 * Math.sqrt(3); // the true per-step distance for a hex of size 100
  engine.animateAlongPath("tok1", [[0, 0], [worldUnitsPerCell, 0]]); // exactly one cell step
  // Correct duration: 1 cell / 6 cells-per-sec * 1000 ≈ 166.67ms, so the tween is complete by
  // 200ms. Dividing by `size` (100) instead computes ~1.732 cells ≈ 288.7ms duration — still
  // short of the target at 200ms — which is what distinguishes the two.
  backend.runTicker(200);
  expect(backend.lastTokenX("tok1")).toBeCloseTo(worldUnitsPerCell, 5);
});

test("animateSamples' moverVision progressively sweeps the fog, reverting to derived vision on completion", () => {
  const store = new DocumentStore();
  store.applyCommand(sceneCmd(1, "s1"));
  const backend = new MockBackend();
  let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
  const engine = new RenderEngine({
    store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
    subscribeScene: (_c, cb) => { onUpdate = cb; return { unsubscribe: () => {} }; },
  });
  engine.start();
  // Baseline derived (subscription) vision: a small polygon.
  onUpdate({ payload: { mode: "masked", polygons: [{ scene: "s1", points: [0, 0, 10, 0, 10, 10] }] }, computedAtSeq: 1 });
  expect(backend.visibility).toEqual({ mode: "masked", visible: [{ points: [0, 0, 10, 0, 10, 10] }], explored: [] });

  // Mover's own MoveStream: two moverVision samples, small polygon → larger polygon.
  engine.animateSamples(
    "tok1",
    [{ tMs: 0, pos: [0, 0] }, { tMs: 500, pos: [100, 0] }],
    1000,
    0,
    () => 0,
    [
      { tMs: 0, polygons: [[[0, 0], [20, 0], [20, 20]]] },
      { tMs: 500, polygons: [[[0, 0], [50, 0], [50, 50]]] },
    ],
  );
  // At clock 0, the first (small) moverVision sample feeds the fog.
  expect(backend.visibility).toEqual({ mode: "masked", visible: [{ points: [0, 0, 20, 0, 20, 20] }], explored: [] });

  // Advance the clock past the second sample's tMs (500 < 1000, animation still in flight).
  backend.runTicker(500);
  expect(backend.visibility).toEqual({ mode: "masked", visible: [{ points: [0, 0, 50, 0, 50, 50] }], explored: [] });

  // Animation completes (elapsed reaches durationMs): fog reverts to the last derived vision.
  backend.runTicker(500);
  expect(backend.visibility).toEqual({ mode: "masked", visible: [{ points: [0, 0, 10, 0, 10, 10] }], explored: [] });
});

test("animateSamples with no moverVision (observer) leaves the fog untouched", () => {
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
  const before = backend.visibility;
  engine.animateSamples("tok1", [{ tMs: 0, pos: [0, 0] }, { tMs: 500, pos: [100, 0] }], 1000, 0, () => 0, null);
  expect(backend.visibility).toEqual(before);
  backend.runTicker(1000);
  expect(backend.visibility).toEqual(before);
});

test("a single in-flight sweep cross-fades the fog between consecutive samples (no snap)", () => {
  // Strictly between two sample tMs (not at a boundary), the compositor blends rather than
  // snapping: the mock backend records the (from, to, factor) triple, factor advancing 0→1
  // as the clock moves across the [0, 500) interval.
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

  engine.animateSamples(
    "tok1",
    [{ tMs: 0, pos: [0, 0] }, { tMs: 500, pos: [100, 0] }],
    1000,
    0,
    () => 0,
    [
      { tMs: 0, polygons: [[[0, 0], [20, 0], [20, 20]]] },
      { tMs: 500, polygons: [[[0, 0], [50, 0], [50, 50]]] },
    ],
  );
  // Immediately (elapsed 0, exactly tCur): factor 0, fully the first sample.
  expect(backend.visibilityBlend).toEqual({
    from: { mode: "masked", visible: [{ points: [0, 0, 20, 0, 20, 20] }], explored: [] },
    to: { mode: "masked", visible: [{ points: [0, 0, 50, 0, 50, 50] }], explored: [] },
    factor: 0,
  });

  // Advance 200ms into the [0,500) interval: factor 0.4, still blending toward the next sample.
  backend.runTicker(200);
  expect(backend.visibilityBlend?.factor).toBeCloseTo(0.4);
  expect(backend.visibilityBlend?.from.visible).toEqual([{ points: [0, 0, 20, 0, 20, 20] }]);
  expect(backend.visibilityBlend?.to.visible).toEqual([{ points: [0, 0, 50, 0, 50, 50] }]);

  // At exactly tMs=500 there is no further "next" sample (only two total): the sweep falls
  // back to the plain snap, clearing any stale blend record.
  backend.runTicker(300);
  expect(backend.visibility).toEqual({ mode: "masked", visible: [{ points: [0, 0, 50, 0, 50, 50] }], explored: [] });
});

test("a concurrent derived frame does not clobber an in-flight vision-sweep (no flicker)", () => {
  // A move commit's own server-recomputed vision broadcast (or any other scene update) arriving
  // WHILE the sweep animation plays must not clobber the sweep's progressive polygon with the
  // derived one — that would flicker until the next tick reasserts the sweep.
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

  engine.animateSamples(
    "tok1", [{ tMs: 0, pos: [0, 0] }, { tMs: 500, pos: [100, 0] }], 1000, 0, () => 0,
    [{ tMs: 0, polygons: [[[0, 0], [20, 0], [20, 20]]] }],
  );
  expect(backend.visibility).toEqual({ mode: "masked", visible: [{ points: [0, 0, 20, 0, 20, 20] }], explored: [] });

  // Bump the store to seq 2 so the seq-2 derived frame below applies IMMEDIATELY (not deferred
  // behind the appliedSeq watermark) — isolates the sweep-suppression behavior under test.
  store.applyCommand(tokenCmd(2, "tokX", 0));
  // A new derived (subscription) frame arrives mid-sweep, at a higher seq — normally this would
  // re-render immediately, but a sweep is in flight so the compositor must not be touched yet.
  onUpdate({ payload: { mode: "masked", polygons: [{ scene: "s1", points: [0, 0, 99, 0, 99, 99] }] }, computedAtSeq: 2 });
  expect(backend.visibility).toEqual({ mode: "masked", visible: [{ points: [0, 0, 20, 0, 20, 20] }], explored: [] }); // unchanged: still the sweep

  // The sweep completes: reverts to the LATEST derived vision (the seq=2 frame), not a stale one.
  backend.runTicker(1000);
  expect(backend.visibility).toEqual({ mode: "masked", visible: [{ points: [0, 0, 99, 0, 99, 99] }], explored: [] });
});

test("animateSamples' moverVision seeds a server-aligned catch-up mid-sample (startServerMs in the past)", () => {
  // Mirrors the "catch-up: jumps to the server-aligned position when startServerMs is in the
  // past" test — exercises the highest-risk untested path: initialElapsed
  // computed from a non-zero serverNow()-startServerMs delta, not always 0.
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

  // startServerMs=1000, serverNow()=1500 → initialElapsed=500 → the sweep must start already
  // seeded at the tMs=500 sample, NOT at tMs=0 (which `initialElapsed: 0` tests never exercise).
  engine.animateSamples(
    "tok1",
    [{ tMs: 0, pos: [0, 0] }, { tMs: 500, pos: [100, 0] }],
    1000,
    1000,
    () => 1500,
    [
      { tMs: 0, polygons: [[[0, 0], [20, 0], [20, 20]]] },
      { tMs: 500, polygons: [[[0, 0], [50, 0], [50, 50]]] },
    ],
  );
  // Applied immediately at call time (before any tick): the mid-sample (tMs=500), not tMs=0.
  expect(backend.visibility).toEqual({ mode: "masked", visible: [{ points: [0, 0, 50, 0, 50, 50] }], explored: [] });
});

test("concurrent sweeps for different tokens do not clobber each other; each settles independently", () => {
  // Guards the keyed (Map<string, …>) sweep state: a second token's sweep must not replace the
  // first's while both are in flight, and each must revert to derived vision only once ITS OWN
  // duration elapses (not when the other's does).
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

  engine.animateSamples(
    "tokA", [{ tMs: 0, pos: [0, 0] }, { tMs: 500, pos: [100, 0] }], 500, 0, () => 0,
    [{ tMs: 0, polygons: [[[0, 0], [20, 0], [20, 20]]] }],
  );
  // Second token's sweep starts while tokA's is still in flight: both polygons must be present
  // (union), proving tokA's entry was not replaced.
  engine.animateSamples(
    "tokB", [{ tMs: 0, pos: [0, 0] }, { tMs: 1000, pos: [100, 0] }], 1000, 0, () => 0,
    [{ tMs: 0, polygons: [[[100, 100], [120, 100], [120, 120]]] }],
  );
  expect(backend.visibility).toEqual({
    mode: "masked",
    visible: [{ points: [0, 0, 20, 0, 20, 20] }, { points: [100, 100, 120, 100, 120, 120] }],
    explored: [],
  });

  // tokA's 500ms sweep completes; tokB's 1000ms sweep is still in flight — the fog must NOT
  // revert to derived vision yet (tokB is still sweeping), only tokA's polygon drops out.
  backend.runTicker(500);
  expect(backend.visibility).toEqual({
    mode: "masked",
    visible: [{ points: [100, 100, 120, 100, 120, 120] }],
    explored: [],
  });

  // tokB's sweep also completes: NOW the fog reverts to the last derived vision.
  backend.runTicker(500);
  expect(backend.visibility).toEqual({ mode: "masked", visible: [{ points: [0, 0, 10, 0, 10, 10] }], explored: [] });
});

test("toLighting parses lit cells for the active scene and fails safe", () => {
  const { store, engine } = makeEngine();
  engine.start();
  // Seed an active scene "s1" (mirror the scene-create command in the fog tests).
  store.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id: "s1", scope: { kind: "world", world_id: "w1" }, doc_type: "scene", schema_version: 1, name: null,
      source: null, owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
      embedded: {}, parent_id: null, engine: { grid: { kind: "square", size: 100, distance: null }, background: null, bounds: null, snapToGrid: null, vision: null, lighting: null }, system: {}, created_at: 0, updated_at: 0,
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
  expect(li!.cells).toEqual([
    { i: 0, j: 0, band: 2, tint: 0, hint: 0, corners: [{ x: 0, y: 0 }, { x: 100, y: 0 }, { x: 100, y: 100 }, { x: 0, y: 100 }] },
  ]);
  expect(li!.hints).toEqual(["desaturate"]);
  expect(li!.bands).toEqual([{ name: "bright", min: 0.67 }, { name: "dim", min: 0.34 }, { name: "dark", min: 0 }]);
  // GM / garbled → null (cosmetic, no overlay).
  expect(engine.toLightingForTest({ mode: "all" })).toBeNull();
  expect(engine.toLightingForTest({ mode: "masked", lit: "garbage" })).toBeNull();
  expect(engine.toLightingForTest(null)).toBeNull();
});

// Regression: on a hex scene the lighting overlay's `lit` cells are also axial (q,r) — this
// pins the axial rasterization at the RenderEngine wiring site (`toLighting` →
// `Lighting.setTarget/apply` → `backend.setLighting`). It builds the engine with `MockBackend`,
// which records the frame but never invokes `PixiBackend`'s paint math, so replacing
// `this.grid.cellVertices(i, j)` in `toLighting` with square indexing fails this test, while
// `PixiBackend.setLighting`'s own paint math is pinned separately by that class's own tests.
test("a masked frame paints a hex scene's lit cell at its true axial position, not a square index", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
  const engine = new RenderEngine({
    store, assets: new AssetResolver(), backend, grid: { kind: "hex", size: 100 },
    subscribeScene: (_c, cb) => { onUpdate = cb; return { unsubscribe: () => {} }; },
  });
  engine.start();
  store.applyCommand(sceneCmd(1, "s1"));
  onUpdate({
    payload: {
      mode: "masked", polygons: [], bands: [{ name: "bright", min: 0.67 }, { name: "dark", min: 0 }],
      renderHints: [], lit: [{ scene: "s1", cell: 100, cells: [1, 1, 1, 0, -1] }], // hex axial (q=1, r=1)
    },
    computedAtSeq: 1,
  });
  backend.tick?.(1000); // settle the fade
  const cells = backend.lighting!.cells;
  expect(cells.length).toBe(1);
  const expectedCenterX = 100 * (Math.sqrt(3) * 1 + (Math.sqrt(3) / 2) * 1);
  const expectedCenterY = 100 * 1.5 * 1;
  const corners = cells[0].corners;
  expect(corners.length).toBe(6);
  let cx = 0, cy = 0;
  for (const p of corners) { cx += p.x; cy += p.y; }
  cx /= 6; cy /= 6;
  expect(cx).toBeCloseTo(expectedCenterX, 6);
  expect(cy).toBeCloseTo(expectedCenterY, 6);
  // Witness the bug directly: the square-position formula would have centered this at (100,100).
  expect(cx).not.toBeCloseTo(100, 1);
  expect(cy).not.toBeCloseTo(100, 1);
});

describe("multi-scene render filtering", () => {
  function seed() {
    const store = new DocumentStore();
    store.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [
      { op: "create", doc: buildSceneDoc("w1", { background: "bgA" }, "sA") },
      { op: "create", doc: buildSceneDoc("w1", { background: "bgB" }, "sB") },
      { op: "create", doc: buildTokenDoc("w1", "sA", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null }, "t-a") },
      { op: "create", doc: buildTokenDoc("w1", "sB", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null }, "t-b") },
    ] });
    return store;
  }

  it("renders only the viewed scene's tokens + background, and re-projects on switch", () => {
    const store = seed();
    let viewed = "sA";
    const backend = new MockBackend();
    const engine = new RenderEngine({ store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 }, viewedSceneId: () => viewed });
    engine.start();
    expect([...backend.tokens.keys()]).toEqual(["t-a"]);
    expect(backend.background?.url).toContain("bgA");

    viewed = "sB";
    engine.reapplyViewedScene();
    expect([...backend.tokens.keys()]).toEqual(["t-b"]);
    expect(backend.background?.url).toContain("bgB");
    engine.destroy();
  });

  it("a deferred scene-A vision frame flushing after a switch to scene B renders scene B's fog, not A's", () => {
    // Fog secrecy across the pendingDerived watermark: a vision frame deferred while viewing scene
    // A must NOT paint scene A's fog holes onto scene B once the store catches up and it flushes.
    // pendingDerived caches the RAW payload and re-filters at flush time against the CURRENT scene.
    const store = seed(); // scenes sA, sB seeded at seq 1 → appliedSeq 1
    let viewed = "sA";
    const backend = new MockBackend();
    let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
    const engine = new RenderEngine({
      store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
      viewedSceneId: () => viewed,
      subscribeScene: (_c, cb) => { onUpdate = cb; return { unsubscribe: () => {} }; },
    });
    engine.start();

    // A masked vision frame for scene sA at seq 5 (store at 1 → deferred into pendingDerived).
    onUpdate({ payload: { mode: "masked", polygons: [{ scene: "sA", points: [0, 0, 10, 0, 10, 10] }] }, computedAtSeq: 5 });
    expect(backend.visibility).toBeNull(); // deferred, not yet applied

    // Switch to scene sB (client-local, no new server frame): reapply re-filters the cached raw
    // payload to sB → the sA polygon is dropped → full fog (fail-closed, no cross-scene hole).
    viewed = "sB";
    engine.reapplyViewedScene();
    expect(backend.visibility).toEqual({ mode: "masked", visible: [], explored: [] });

    // Store advances past seq 5 → the deferred frame flushes. It must re-filter against the CURRENT
    // scene (sB), NOT replay the stale sA-filtered input. Pins: scene B's fog still stands (no sA
    // hole leaking through) once the deferred frame is applied.
    store.applyCommand({
      seq: 5, world_id: "w1", author: "u", ts: 0,
      ops: [{ op: "create", doc: buildSceneDoc("w1", {}, "sC") }],
    });
    expect(backend.visibility).toEqual({ mode: "masked", visible: [], explored: [] });
    engine.destroy();
  });

  it("a stale deferred frame superseded by a later immediate-apply frame is discarded, not re-applied, on flush (no lastAppliedSeq regression)", () => {
    // Frame-ordering monotonicity hole: seq 5 defers behind the watermark, then seq 7 arrives and
    // takes the IMMEDIATE-apply branch (appliedSeq already caught up) without touching the still-set
    // pendingDerived(5) entry. A later flush must discard that stale entry, never regress the mask
    // back to seq 5's payload. `store.appliedSeq` is mutated directly (a plain field on
    // DocumentStore) to isolate RenderEngine's own onSceneFrame/flushPendingDerived contract from
    // DocumentStore's incidental commit-triggers-flush coupling — the engine's watermark logic must
    // hold for any ReadableDocuments whose appliedSeq can advance independently of a flush trigger.
    const store = new DocumentStore();
    const backend = new MockBackend();
    let onUpdate!: (f: { payload: unknown; computedAtSeq: number }) => void;
    const engine = new RenderEngine({
      store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
      subscribeScene: (_c, cb) => { onUpdate = cb; return { unsubscribe: () => {} }; },
    });
    engine.start();

    // (a) seq 5 arrives while appliedSeq (0) is behind → deferred into pendingDerived.
    onUpdate({ payload: { mode: "masked", polygons: [{ scene: null, points: [0, 0, 10, 0, 10, 10] }] }, computedAtSeq: 5 });
    expect(backend.visibility).toBeNull(); // deferred, not yet applied

    // (b) appliedSeq catches up to 7 (bypassing store.subscribe's flush trigger — see rationale
    // above); seq 7 then arrives and takes the immediate-apply branch, advancing lastAppliedSeq to 7.
    store.appliedSeq = 7;
    onUpdate({ payload: { mode: "all" }, computedAtSeq: 7 });
    expect(backend.visibility).toEqual({ mode: "all", visible: [], explored: [] });

    // (c) A later store commit (any commit — flushPendingDerived runs on every one) re-checks the
    // still-set pendingDerived(5). It must be discarded (5 <= lastAppliedSeq 7), never applied: the
    // mask must stay at the seq-7 payload, and lastAppliedSeq must not regress to 5.
    store.applyCommand({
      seq: 8, world_id: "w1", author: "a", ts: 0,
      ops: [{ op: "create", doc: buildSceneDoc("w1", {}, "s1") }],
    });
    expect(backend.visibility).toEqual({ mode: "all", visible: [], explored: [] }); // unchanged, not the stale masked payload

    // A genuinely-newer frame at seq 8 must still apply normally afterward (monotonic forward
    // progress is unaffected by the discard).
    onUpdate({ payload: { mode: "masked", polygons: [{ scene: "s1", points: [1, 1, 11, 1, 11, 11] }] }, computedAtSeq: 8 });
    expect(backend.visibility).toEqual({ mode: "masked", visible: [{ points: [1, 1, 11, 1, 11, 11] }], explored: [] });
    engine.destroy();
  });
});

test("the engine renders a token at the footprint lookup it was constructed with, refreshed by reapplyFootprints", () => {
  // The wiring under test is the engine passing its `footprints` accessor down to `TokenView`.
  // The accessor is read per reconcile, so replacing the lookup and calling `reapplyFootprints`
  // repaints without any document change.
  const store = new DocumentStore();
  const backend = new MockBackend();
  let footprints: FootprintLookup = { token: () => ({ w: 173.2, h: 200 }), unit: () => null };
  const engine = new RenderEngine({
    store,
    assets: new AssetResolver(),
    backend,
    grid: { kind: "hex", size: 100 },
    footprints: () => footprints,
  });
  engine.start();
  store.applyCommand(tokenCmd(1, "t1", 0));
  expect(backend.tokens.get("t1")!.w).toBe(173.2);
  expect(backend.tokens.get("t1")!.h).toBe(200);

  footprints = { token: () => ({ w: 346.4, h: 400 }), unit: () => null };
  engine.reapplyFootprints();
  expect(backend.tokens.get("t1")!.w).toBe(346.4);
  expect(backend.tokens.get("t1")!.h).toBe(400);
  engine.destroy();
});

test("setThemeColors routes the background to the backend clear color and redraws the grid in the new color", () => {
  const { backend, engine } = makeEngine();
  engine.setViewport(300, 200);
  engine.start();
  expect(backend.clearColor).toBeNull();
  expect(backend.gridColor).toBe(0x3a3a4a); // the default slate

  engine.setThemeColors({ background: 0x112233, gridColor: 0x445566 });

  expect(backend.clearColor).toBe(0x112233);
  expect(backend.gridColor).toBe(0x445566);
});

test("setThemeColors with only a background leaves the grid untouched (no redraw, same color)", () => {
  const { backend, engine } = makeEngine();
  engine.setViewport(300, 200);
  engine.start();
  backend.gridLineCount = -1; // sentinel: any redraw overwrites this

  engine.setThemeColors({ background: 0x000001 });

  expect(backend.clearColor).toBe(0x000001);
  expect(backend.gridLineCount).toBe(-1);
  expect(backend.gridColor).toBe(0x3a3a4a);
});

test("the engine highlights selected tokens via the selectedTokens accessor, refreshed by reapplyTokenSelection", () => {
  // The wiring under test is the engine passing its `selectedTokens` accessor down to
  // `TokenView`. The accessor is read per reconcile, so changing the selection and calling
  // `reapplyTokenSelection` repaints without any document change.
  const store = new DocumentStore();
  const backend = new MockBackend();
  let selected: ReadonlySet<string> = new Set();
  const engine = new RenderEngine({
    store,
    assets: new AssetResolver(),
    backend,
    grid: { kind: "square", size: 100 },
    selectedTokens: () => selected,
  });
  engine.start();
  store.applyCommand(tokenCmd(1, "t1", 0));
  expect(backend.tokens.get("t1")!.fx).toBeUndefined();

  selected = new Set(["t1"]);
  engine.reapplyTokenSelection();
  expect(backend.tokens.get("t1")!.fx).toEqual([{ kind: "highlight", color: 0xffd400, strength: 0.4 }]);

  selected = new Set();
  engine.reapplyTokenSelection();
  expect(backend.tokens.get("t1")!.fx).toBeUndefined();
  engine.destroy();
});
