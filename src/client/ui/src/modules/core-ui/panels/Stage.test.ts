import { test, expect, vi } from "vitest";
import { render } from "@testing-library/svelte";
import Stage from "./Stage.svelte";
import type { DisplayBackend } from "@shadowcat/render";
import type { ReadableDocuments } from "@shadowcat/core";
import { setAppContextForTest } from "../../../lib/__fixtures__/appContextTest";

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
    setShape() {},
    removeShape() {},
    drawOverlay() {},
    clearOverlay() {},
    drawMeasure() {},
    clearMeasure() {},
    drawPings() {},
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
