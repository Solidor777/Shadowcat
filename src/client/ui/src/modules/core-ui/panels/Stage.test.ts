import { test, expect, vi } from "vitest";
import { render } from "@testing-library/svelte";
import Stage from "./Stage.svelte";
import type { DisplayBackend } from "@shadowcat/render";
import { setAppContextForTest } from "../../../lib/__fixtures__/appContextTest";

function fakeBackend(): DisplayBackend & { destroyed: boolean } {
  return {
    destroyed: false,
    ensureLayers() {},
    setBackground() {},
    drawGrid() {},
    setCameraTransform() {},
    resize() {},
    destroy() { this.destroyed = true; },
  };
}

test("mounts a canvas container and tears the backend down on unmount", async () => {
  const backend = fakeBackend();
  const createBackend = vi.fn(async () => backend);
  const { container, unmount } = render(Stage, {
    props: { createBackend },
    context: setAppContextForTest(),
  });
  // The host renders a canvas element synchronously.
  expect(container.querySelector("[data-testid='stage-canvas']")).not.toBeNull();
  // The $effect's async init runs after mount; wait for the backend factory.
  await vi.waitFor(() => expect(createBackend).toHaveBeenCalledOnce());
  // Unmount tears the engine/backend down. When unmount races the still-pending
  // async init, the race-guard destroys the backend once init resumes — so the
  // teardown completes asynchronously either way.
  unmount();
  await vi.waitFor(() => expect(backend.destroyed).toBe(true));
});
