import { test, expect, vi } from "vitest";
import { render } from "@testing-library/svelte";
import Stage from "./Stage.svelte";
import type { DisplayBackend } from "@shadowcat/render";
import { RenderEngine } from "@shadowcat/render";
import { DocumentStore, AssetResolver, buildSceneDoc, buildTokenDoc } from "@shadowcat/core";
import type { ReadableDocuments } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { __APP_CONTEXT_KEY__ } from "@shadowcat/ui-kit";

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

/** A documents view exposing a single scene doc with the given `system` body (M10f-3 snap
 * wiring: `resolveSceneSettings` reads it via `documents.query("scene")[0]`). */
function sceneDocs(engine: Record<string, unknown>): ReadableDocuments {
  return {
    query: (t: string) => (t === "scene" ? [{ id: "s1", doc_type: "scene", engine, system: {} }] : []),
    get: () => undefined,
    subscribe: () => () => {},
    snapshot: () => [],
    appliedSeq: 0,
  } as unknown as ReadableDocuments;
}

function fakeBackend(): DisplayBackend & { destroyed: boolean } {
  return {
    destroyed: false,
    ensureLayers() {},
    setBackground() {},
    drawGrid() {},
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

test("drives the initial reconcile from ctx.viewedSceneId (M12d)", async () => {
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
          { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null },
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

test("exposes the viewed scene's committed token positions as data-token-positions", async () => {
  const store = new DocumentStore();
  const token = (id: string, scene: string, x: number, y: number): unknown => ({
    op: "create",
    doc: buildTokenDoc(
      "w1",
      scene,
      { x, y, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null },
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

test("exposes the server's move-resolution outcome as data-last-move-outcome (M14b)", async () => {
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

test("the viewedSceneId-change watcher calls reapplyViewedScene exactly once per genuine change (M12d review)", async () => {
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
          { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null },
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
          { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null },
          "t2",
        ),
      },
    ],
  } as never);
  await new Promise((resolve) => setTimeout(resolve, 50));
  expect(spy).toHaveBeenCalledTimes(1);
  spy.mockRestore();
});
