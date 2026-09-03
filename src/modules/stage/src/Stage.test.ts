import { test, expect, vi } from "vitest";
import { render } from "@testing-library/svelte";
import Stage from "./Stage.svelte";
import type { DisplayBackend, TokenNodeSpec } from "@shadowcat/render";
import { RenderEngine } from "@shadowcat/render";
import { DocumentStore, AssetResolver, buildSceneDoc, buildTokenDoc, EMPTY_FOOTPRINTS, silentLogger } from "@shadowcat/core";
import type { ReadableDocuments, FootprintLookup, Logger } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { __APP_CONTEXT_KEY__, theme, TokenSelection } from "@shadowcat/ui-kit";

const OWNER = "11111111-2222-3333-4444-555555555555";

/** A documents view exposing a single token owned by OWNER. */
function tokenDocs(): ReadableDocuments {
  return {
    query: (t: string) => (t === "token" ? [{ id: "tok", doc_type: "token", owner: OWNER }] : []),
    get: () => undefined,
    subscribe: () => () => {},
    snapshot: () => [],
    appliedSeq: 0,
  } as unknown as ReadableDocuments;
}

/** A documents view exposing a single scene doc with the given `engine` body, read by
 * `resolveSceneSettings` via `documents.query("scene")[0]`. */
function sceneDocs(engine: Record<string, unknown>): ReadableDocuments {
  return {
    query: (t: string) => (t === "scene" ? [{ id: "s1", doc_type: "scene", engine, system: {} }] : []),
    get: () => undefined,
    subscribe: () => () => {},
    snapshot: () => [],
    appliedSeq: 0,
  } as unknown as ReadableDocuments;
}

function fakeBackend(): DisplayBackend & { destroyed: boolean; clearColor: number | null; gridColor: number | null } {
  return {
    destroyed: false,
    clearColor: null,
    gridColor: null,
    ensureLayers() {},
    setBackground() {},
    setClearColor(color: number) { this.clearColor = color; },
    drawGrid(_lines: unknown, color: number) { this.gridColor = color; },
    setCameraTransform() {},
    setVisibility() {},
    addLayerFilter() { return () => {}; },
    setToken() {},
    removeToken() {},
    tickTokenAnimations() {},
    setShape() {},
    removeShape() {},
    drawOverlay() {},
    clearOverlay() {},
    drawMeasure() {},
    clearMeasure() {},
    drawPings() {},
    drawEmotes() {},
    setLighting() {},
    startTicker() {},
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
  // The host renders a canvas element synchronously.
  expect(container.querySelector("[data-testid='stage-canvas']")).not.toBeNull();
  // The $effect's async init runs after mount; wait for the backend factory.
  await vi.waitFor(() => expect(createBackend).toHaveBeenCalledOnce());
  await vi.waitFor(() => expect(subscribeScene).toHaveBeenCalledWith("vision", expect.any(Function), undefined));
  // Unmount tears the engine/backend down (async when unmount races the init).
  unmount();
  await vi.waitFor(() => expect(backend.destroyed).toBe(true));
});

test("see-as picker labels options by username from the members map", async () => {
  const createBackend = vi.fn(async () => fakeBackend());
  const { getByText } = render(Stage, {
    props: { createBackend },
    context: setAppContextForTest({
      role: "gm",
      documents: tokenDocs(),
      members: new Map([[OWNER, "Alice"]]),
      subscribeScene: () => ({ unsubscribe() {} }),
    }),
  });
  await vi.waitFor(() => expect(getByText("See as Alice")).toBeTruthy());
});

test("see-as picker falls back to the short id for an unknown owner", async () => {
  const createBackend = vi.fn(async () => fakeBackend());
  const { getByText } = render(Stage, {
    props: { createBackend },
    context: setAppContextForTest({
      role: "gm",
      documents: tokenDocs(),
      members: new Map(),
      subscribeScene: () => ({ unsubscribe() {} }),
    }),
  });
  await vi.waitFor(() => expect(getByText(`See as ${OWNER.slice(0, 8)}`)).toBeTruthy());
});

test("a backend-init failure logs through the injected logger, not silently", async () => {
  const failure = new Error("no webgl context");
  const createBackend = vi.fn(async () => {
    throw failure;
  });
  const errors: unknown[][] = [];
  const logger: Logger = { ...silentLogger, error: (...args) => errors.push(args) };
  const { container } = render(Stage, {
    props: { createBackend, logger },
    context: setAppContextForTest({ subscribeScene: () => ({ unsubscribe: () => {} }) }),
  });
  const host = container.querySelector(".stage-host") as HTMLElement | null;
  await vi.waitFor(() => {
    expect(host?.dataset.renderError).toBe("true");
  });
  expect(errors).toHaveLength(1);
  expect(errors[0][0]).toContain("Stage backend init failed");
});

test("pushes the resolved snapToGrid to the engine (grid-stepped scene: default true)", async () => {
  const createBackend = vi.fn(async () => fakeBackend());
  const spy = vi.spyOn(RenderEngine.prototype, "setSnapEnabled");
  render(Stage, {
    props: { createBackend },
    context: setAppContextForTest({
      documents: sceneDocs({ grid: { kind: "square", size: 100 } }),
      subscribeScene: () => ({ unsubscribe() {} }),
    }),
  });
  await vi.waitFor(() => expect(spy).toHaveBeenCalledWith(true));
  spy.mockRestore();
});

test("pushes the resolved snapToGrid to the engine (continuous scene: default false)", async () => {
  const createBackend = vi.fn(async () => fakeBackend());
  const spy = vi.spyOn(RenderEngine.prototype, "setSnapEnabled");
  render(Stage, {
    props: { createBackend },
    context: setAppContextForTest({
      documents: sceneDocs({ grid: { kind: "square", size: 100 }, vision: { movementModel: "continuous" } }),
      subscribeScene: () => ({ unsubscribe() {} }),
    }),
  });
  await vi.waitFor(() => expect(spy).toHaveBeenCalledWith(false));
  spy.mockRestore();
});

test("drives the initial reconcile from ctx.viewedSceneId", async () => {
  const store = new DocumentStore();
  store.applyCommand({
    seq: 1,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "sA") },
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 50, distance: null } }, "sB") },
      {
        op: "create",
        doc: buildTokenDoc(
          "w1",
          "sB",
          { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null },
          "t-b",
        ),
      },
    ],
  } as never);
  const createBackend = vi.fn(async () => fakeBackend());
  const { container } = render(Stage, {
    props: { createBackend },
    context: setAppContextForTest({
      documents: store,
      store,
      assets: new AssetResolver(),
      viewedSceneId: "sA",
      subscribeScene: () => ({ unsubscribe() {} }),
    }),
  });
  await vi.waitFor(() => expect(createBackend).toHaveBeenCalledOnce());
  const host = container.querySelector(".stage-host") as HTMLElement;
  await vi.waitFor(() => expect(host.dataset.renderReady).toBe("true"));
  // sA is viewed, has no tokens (t-b is parented to sB); the Stage must filter tokenCount
  // by the viewed scene, not report the whole store's token count.
  expect(host.dataset.tokenCount).toBe("0");
});

test("exposes the viewed scene's engine.background as data-background, empty when unset", async () => {
  const store = new DocumentStore();
  store.applyCommand({
    seq: 1,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null }, background: "asset-1" }, "sA") },
    ],
  } as never);
  const createBackend = vi.fn(async () => fakeBackend());
  const { container } = render(Stage, {
    props: { createBackend },
    context: setAppContextForTest({
      documents: store,
      store,
      assets: new AssetResolver(),
      viewedSceneId: "sA",
      subscribeScene: () => ({ unsubscribe() {} }),
    }),
  });
  const host = container.querySelector(".stage-host") as HTMLElement;
  await vi.waitFor(() => expect(host.dataset.renderReady).toBe("true"));
  expect(host.dataset.background).toBe("asset-1");
});

test("exposes the viewed scene's committed token positions as data-token-positions", async () => {
  const store = new DocumentStore();
  const token = (id: string, scene: string, x: number, y: number): unknown => ({
    op: "create",
    doc: buildTokenDoc(
      "w1",
      scene,
      { x, y, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null },
      id,
    ),
  });
  store.applyCommand({
    seq: 1,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "hex", size: 100, distance: null } }, "sA") },
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "hex", size: 100, distance: null } }, "sB") },
      token("t-b", "sA", 250, -125),
      token("t-a", "sA", 0, 0),
      token("t-other", "sB", 900, 900),
    ],
  } as never);
  const createBackend = vi.fn(async () => fakeBackend());
  const { container } = render(Stage, {
    props: { createBackend },
    context: setAppContextForTest({
      documents: store,
      store,
      assets: new AssetResolver(),
      viewedSceneId: "sA",
      subscribeScene: () => ({ unsubscribe() {} }),
    }),
  });
  await vi.waitFor(() => expect(createBackend).toHaveBeenCalledOnce());
  const host = container.querySelector(".stage-host") as HTMLElement;
  await vi.waitFor(() => expect(host.dataset.renderReady).toBe("true"));
  // Id-sorted, viewed-scene-only: a token parented to another scene never appears.
  expect(host.dataset.tokenPositions).toBe("t-a:0,0;t-b:250,-125");

  // A committed position change is reflected on the next store pass — this is the
  // signal a rollback is observed through (a rejected write reverts the string).
  store.applyCommand({
    seq: 2,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: [
      {
        op: "update",
        doc_id: "t-a",
        changes: [
          { path: "/engine/x", old: 0, new: 100 },
          { path: "/engine/y", old: 0, new: 50 },
        ],
      },
    ],
  } as never);
  await vi.waitFor(() => expect(host.dataset.tokenPositions).toBe("t-a:100,50;t-b:250,-125"));
});

test("writes the lighting observability attributes only when their values change", async () => {
  const store = new DocumentStore();
  store.applyCommand({
    seq: 1,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: [{ op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "s1") }],
  } as never);
  let onVision: ((f: { payload: unknown; computedAtSeq: number }) => void) | undefined;
  const createBackend = vi.fn(async () => fakeBackend());
  const { container } = render(Stage, {
    props: { createBackend },
    context: setAppContextForTest({
      documents: store,
      store,
      assets: new AssetResolver(),
      viewedSceneId: "s1",
      subscribeScene: (channel: string, cb: (f: { payload: unknown; computedAtSeq: number }) => void) => {
        if (channel === "vision") onVision = cb;
        return { unsubscribe() {} };
      },
    }),
  });
  await vi.waitFor(() => expect(createBackend).toHaveBeenCalledOnce());
  const host = container.querySelector(".stage-host") as HTMLElement;
  await vi.waitFor(() => expect(host.dataset.renderReady).toBe("true"));
  await vi.waitFor(() => expect(onVision).toBeDefined());
  /** A masked frame lighting `cells` (5-int tuples) in the viewed scene. */
  const lit = (cells: number[]): unknown => ({
    mode: "masked",
    polygons: [{ scene: "s1", points: [-1000, -1000, 1000, -1000, 1000, 1000, -1000, 1000] }],
    bands: [{ name: "bright", min: 0.67 }, { name: "dim", min: 0.34 }, { name: "dark", min: 0 }],
    renderHints: [],
    lit: [{ scene: "s1", cell: 100, cells }],
    perceived: [],
  });
  onVision!({ payload: lit([0, 0, 0, 0, -1]), computedAtSeq: 1 });
  expect(host.dataset.litCells).toBe("1");
  expect(host.dataset.lightSweep).toBe("0");
  expect(host.dataset.litBbox).toBe("0,0,0,0");
  const observer = new MutationObserver(() => {});
  observer.observe(host, { attributes: true, attributeFilter: ["data-lit-cells", "data-light-sweep", "data-lit-bbox"] });
  // An identical frame repaints the overlay (lighting applies eagerly per frame) but changes
  // no value: zero attribute mutations.
  onVision!({ payload: lit([0, 0, 0, 0, -1]), computedAtSeq: 2 });
  expect(observer.takeRecords()).toEqual([]);
  // A frame lighting one more cell changes the count and the bbox, not the sweep flag.
  onVision!({ payload: lit([0, 0, 0, 0, -1, 2, 3, 0, 0, -1]), computedAtSeq: 3 });
  expect(observer.takeRecords().map((r) => r.attributeName).sort()).toEqual(["data-lit-bbox", "data-lit-cells"]);
  expect(host.dataset.litCells).toBe("2");
  expect(host.dataset.litBbox).toBe("0,0,2,3");
  observer.disconnect();
});

test("exposes each viewed-scene token's resolved visual kind as data-token-visuals", async () => {
  const store = new DocumentStore();
  store.applyCommand({
    seq: 1,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "sA") },
      {
        op: "create",
        doc: buildTokenDoc(
          "w1",
          "sA",
          { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "generated", art: { kind: "image", asset: "p1" }, crop: "circle", border: { color: "#ff8800", width: 0.06 }, background: { color: "#102030" } }, elevation: null, actor_id: null, overrides: null, face: null },
          "t-gen",
        ),
      },
      {
        op: "create",
        doc: buildTokenDoc(
          "w1",
          "sA",
          { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, elevation: null, actor_id: null, overrides: null, face: null },
          "t-img",
        ),
      },
    ],
  } as never);
  const createBackend = vi.fn(async () => fakeBackend());
  const { container } = render(Stage, {
    props: { createBackend },
    context: setAppContextForTest({
      documents: store,
      store,
      assets: new AssetResolver(),
      viewedSceneId: "sA",
      subscribeScene: () => ({ unsubscribe() {} }),
    }),
  });
  const host = container.querySelector(".stage-host") as HTMLElement;
  await vi.waitFor(() => expect(host.dataset.renderReady).toBe("true"));
  // Id-sorted `id:kind` pairs, resolved through the same resolveTokenVisual the render layer
  // draws from: a generated visual reports its own kind, not its art's.
  expect(host.dataset.tokenVisuals).toBe("t-gen:generated;t-img:image");
});

test("re-projects tokens with the selection highlight fx when the token selection changes", async () => {
  const store = new DocumentStore();
  store.applyCommand({
    seq: 1,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "sA") },
      {
        op: "create",
        doc: buildTokenDoc(
          "w1",
          "sA",
          { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, elevation: null, actor_id: null, overrides: null, face: null },
          "t1",
        ),
      },
    ],
  } as never);
  /** The last `setToken` spec per token id, recorded verbatim (a MockBackend-shaped read of what
   * the engine pushed). */
  const specs = new Map<string, TokenNodeSpec>();
  const backend = { ...fakeBackend(), setToken(id: string, spec: TokenNodeSpec): void { specs.set(id, spec); } };
  const tokenSelection = new TokenSelection();
  const { container } = render(Stage, {
    props: { createBackend: async () => backend },
    context: setAppContextForTest({
      documents: store,
      store,
      assets: new AssetResolver(),
      viewedSceneId: "sA",
      subscribeScene: () => ({ unsubscribe() {} }),
      tokenSelection,
    }),
  });
  const host = container.querySelector(".stage-host") as HTMLElement;
  await vi.waitFor(() => expect(host.dataset.renderReady).toBe("true"));
  expect(specs.get("t1")!.fx).toBeUndefined();

  // A selection change carries no store commit; the Stage's watcher must re-project explicitly.
  tokenSelection.set(["t1"]);
  await vi.waitFor(() => expect(specs.get("t1")!.fx).toEqual([{ kind: "highlight", color: 0xffd400, strength: 0.4 }]));

  tokenSelection.clear();
  await vi.waitFor(() => expect(specs.get("t1")!.fx).toBeUndefined());
});

test("exposes the server's move-resolution outcome as data-last-move-outcome", async () => {
  const createBackend = vi.fn(async () => fakeBackend());
  let capturedCb: ((msg: { tokenId: string; outcome: "executed" | "truncated" | "rejected" }) => void) | null = null;
  const { container } = render(Stage, {
    props: { createBackend },
    context: setAppContextForTest({
      subscribeScene: () => ({ unsubscribe() {} }),
      onMoveOutcome: (cb) => {
        capturedCb = cb;
        return () => { capturedCb = null; };
      },
    }),
  });
  const host = container.querySelector(".stage-host") as HTMLElement;
  await vi.waitFor(() => expect(host.dataset.renderReady).toBe("true"));
  expect(host.dataset.lastMoveOutcome).toBeUndefined();
  capturedCb!({ tokenId: "tok1", outcome: "truncated" });
  expect(host.dataset.lastMoveOutcome).toBe("truncated");

  capturedCb!({ tokenId: "tok1", outcome: "executed" });
  expect(host.dataset.lastMoveOutcome).toBe("executed");
});

test("exposes each viewed-scene token's projected badge chips as data-token-badges", async () => {
  const store = new DocumentStore();
  store.applyCommand({
    seq: 1,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "sA") },
      {
        op: "create",
        doc: buildTokenDoc(
          "w1", "sA",
          { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: 3 },
          "t-a",
        ),
      },
      {
        op: "create",
        doc: buildTokenDoc(
          "w1", "sA",
          { x: 200, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null },
          "t-b",
        ),
      },
    ],
  } as never);
  const createBackend = vi.fn(async () => fakeBackend());
  const { container } = render(Stage, {
    props: { createBackend },
    context: setAppContextForTest({
      documents: store,
      store,
      assets: new AssetResolver(),
      viewedSceneId: "sA",
      subscribeScene: () => ({ unsubscribe() {} }),
    }),
  });
  const host = container.querySelector(".stage-host") as HTMLElement;
  await vi.waitFor(() => expect(host.dataset.renderReady).toBe("true"));
  // Id-sorted; the elevated token shows its chip, the grounded one an empty list.
  expect(host.dataset.tokenBadges).toBe("t-a:↑3;t-b:");

  // A committed elevation change re-projects on the next store pass.
  store.applyCommand({
    seq: 2, world_id: "w1", author: "u", ts: 0,
    ops: [{ op: "update", doc_id: "t-a", changes: [{ path: "/engine/elevation", old: 3, new: 0 }] }],
  } as never);
  await vi.waitFor(() => expect(host.dataset.tokenBadges).toBe("t-a:;t-b:"));
});

test("exposes the applied vision frame's creature-sense ids as data-perceived-tokens", async () => {
  const store = new DocumentStore();
  store.applyCommand({
    seq: 1,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "sA") },
      {
        op: "create",
        doc: buildTokenDoc(
          "w1", "sA",
          { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null },
          "t-a",
        ),
      },
    ],
  } as never);
  const createBackend = vi.fn(async () => fakeBackend());
  let frameHandler: ((f: { payload: unknown; computedAtSeq: number }) => void) | null = null;
  const { container } = render(Stage, {
    props: { createBackend },
    context: setAppContextForTest({
      documents: store,
      store,
      assets: new AssetResolver(),
      viewedSceneId: "sA",
      subscribeScene: (_channel: string, cb: typeof frameHandler) => {
        frameHandler = cb;
        return { unsubscribe() {} };
      },
    }),
  });
  const host = container.querySelector(".stage-host") as HTMLElement;
  await vi.waitFor(() => expect(host.dataset.renderReady).toBe("true"));
  expect(frameHandler).not.toBeNull();

  // appliedSeq (1) >= computedAtSeq (1): the frame applies immediately.
  frameHandler!({ payload: { mode: "masked", polygons: [], explored: [], perceived: [{ scene: "sA", tokens: ["t-a"] }] }, computedAtSeq: 1 });
  expect(host.dataset.perceivedTokens).toBe("t-a");

  // A later frame naming nothing clears the attribute; a frame for ANOTHER scene contributes
  // no ids (the active-scene filter). computedAtSeq 2 is ahead of the store's appliedSeq, so
  // the frame sits deferred behind the fog-secrecy watermark until a store commit catches up
  // (`flushPendingDerived`).
  frameHandler!({ payload: { mode: "masked", polygons: [], explored: [], perceived: [{ scene: "sB", tokens: ["t-x"] }] }, computedAtSeq: 2 });
  expect(host.dataset.perceivedTokens).toBe("t-a"); // still the last APPLIED frame
  store.applyCommand({
    seq: 2, world_id: "w1", author: "u", ts: 0,
    ops: [{ op: "update", doc_id: "t-a", changes: [{ path: "/engine/x", old: 0, new: 50 }] }],
  } as never);
  expect(host.dataset.perceivedTokens).toBe("");
});

test("the viewedSceneId-change watcher calls reapplyViewedScene exactly once per genuine change", async () => {
  const store = new DocumentStore();
  store.applyCommand({
    seq: 1,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "sA") },
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "sB") },
    ],
  } as never);
  const createBackend = vi.fn(async () => fakeBackend());
  const context = setAppContextForTest({
    documents: store,
    store,
    assets: new AssetResolver(),
    viewedSceneId: "sA",
    subscribeScene: () => ({ unsubscribe() {} }),
  });
  // Install a LIVE getter on the actual ctx object so the Stage's `ctx.viewedSceneId` reads
  // reflect this closure variable, mirroring how the real session's reactive `gmViewedScene`
  // $state is read live rather than snapshotted at context-creation time.
  let viewed = "sA";
  Object.defineProperty(context.get(__APP_CONTEXT_KEY__), "viewedSceneId", {
    get: () => viewed,
    configurable: true,
  });
  const spy = vi.spyOn(RenderEngine.prototype, "reapplyViewedScene");
  const { container } = render(Stage, { props: { createBackend }, context });
  const host = container.querySelector(".stage-host") as HTMLElement;
  await vi.waitFor(() => expect(host.dataset.renderReady).toBe("true"));
  expect(spy).not.toHaveBeenCalled();

  // Change the viewed scene, then drive a real document-store mutation (the thing
  // `vsSub()`'s `createSubscriber(documents.subscribe)` bridge actually observes) to fire
  // the watcher's $effect re-run.
  viewed = "sB";
  store.applyCommand({
    seq: 2,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: [
      {
        op: "create",
        doc: buildTokenDoc(
          "w1",
          "sA",
          { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null },
          "t1",
        ),
      },
    ],
  } as never);
  await vi.waitFor(() => expect(spy).toHaveBeenCalledTimes(1));

  // A second, unrelated doc mutation with `viewed` unchanged must NOT re-trigger the watcher
  // (the `if (now !== lastViewed)` guard).
  store.applyCommand({
    seq: 3,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: [
      {
        op: "create",
        doc: buildTokenDoc(
          "w1",
          "sA",
          { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null },
          "t2",
        ),
      },
    ],
  } as never);
  await new Promise((resolve) => setTimeout(resolve, 50));
  expect(spy).toHaveBeenCalledTimes(1);
  spy.mockRestore();
});

test("a new footprints lookup re-projects the tokens exactly once per genuine change", async () => {
  const store = new DocumentStore();
  store.applyCommand({
    seq: 1,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: [{ op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "sA") }],
  } as never);
  const createBackend = vi.fn(async () => fakeBackend());
  const context = setAppContextForTest({
    documents: store,
    store,
    assets: new AssetResolver(),
    viewedSceneId: "sA",
    subscribeScene: () => ({ unsubscribe() {} }),
  });
  // A live getter, mirroring the session's reactive `#footprints` $state: a frame replaces the
  // lookup wholesale, so the Stage watches the reference.
  let footprints: FootprintLookup = EMPTY_FOOTPRINTS;
  Object.defineProperty(context.get(__APP_CONTEXT_KEY__), "footprints", {
    get: () => footprints,
    configurable: true,
  });
  const spy = vi.spyOn(RenderEngine.prototype, "reapplyFootprints");
  const { container } = render(Stage, { props: { createBackend }, context });
  const host = container.querySelector(".stage-host") as HTMLElement;
  await vi.waitFor(() => expect(host.dataset.renderReady).toBe("true"));
  expect(spy).not.toHaveBeenCalled();

  footprints = { token: () => ({ w: 173.2, h: 200 }), unit: () => null };
  store.applyCommand({
    seq: 2,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: [
      {
        op: "create",
        doc: buildTokenDoc(
          "w1",
          "sA",
          { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null },
          "t1",
        ),
      },
    ],
  } as never);
  await vi.waitFor(() => expect(spy).toHaveBeenCalledTimes(1));

  // An unrelated doc mutation with the same lookup must not re-project.
  store.applyCommand({
    seq: 3,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: [
      {
        op: "create",
        doc: buildTokenDoc(
          "w1",
          "sA",
          { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null },
          "t2",
        ),
      },
    ],
  } as never);
  await new Promise((resolve) => setTimeout(resolve, 50));
  expect(spy).toHaveBeenCalledTimes(1);
  spy.mockRestore();
});

test("a theme change re-reads the color tokens and pushes them into the engine", async () => {
  // `readColor` resolves a token through a throwaway probe span's computed
  // `color`; jsdom never resolves `var(...)`, so stub `getComputedStyle` to
  // answer from this per-test map, keyed off the probe's inline `color`.
  const colors: Record<string, string> = {
    "--surface-base": "rgb(16, 16, 20)",
    "--grid-line": "rgb(54, 54, 69)",
  };
  vi.stubGlobal("getComputedStyle", (el: Element) => {
    const token = /var\((--[\w-]+)\)/.exec((el as HTMLElement).style?.color ?? "")?.[1] ?? "";
    return { color: colors[token] ?? "" } as CSSStyleDeclaration;
  });
  try {
    const backend = fakeBackend();
    render(Stage, {
      props: { createBackend: async () => backend },
      context: setAppContextForTest({ subscribeScene: () => ({ unsubscribe: () => {} }) }),
    });
    // The mount effect applies the current theme's colors once the engine exists.
    await vi.waitFor(() => expect(backend.clearColor).toBe(0x101014));
    expect(backend.gridColor).toBe(0x363645);

    // Simulate the swapped theme's token values, then swap the theme.
    colors["--surface-base"] = "rgb(240, 240, 244)";
    colors["--grid-line"] = "rgb(200, 200, 210)";
    theme.setActive("slate-light");

    await vi.waitFor(() => expect(backend.clearColor).toBe(0xf0f0f4));
    expect(backend.gridColor).toBe(0xc8c8d2);
  } finally {
    theme.setActive("slate-dark");
    vi.unstubAllGlobals();
  }
});
