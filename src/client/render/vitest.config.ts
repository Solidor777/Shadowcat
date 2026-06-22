import { defineConfig } from "vitest/config";

// The render MODEL (layers/camera/grid/reconciler/engine) is framework- and
// Pixi-free, so it runs in node. The Pixi backend is GL and is covered by the ui
// Playwright suite, not here.
export default defineConfig({
  test: {
    // pixi.js must never be imported by a unit test; the model files don't import
    // it, so node is sufficient and fast.
  },
});
