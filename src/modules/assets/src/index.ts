import type { Module } from "@shadowcat/core";
import Assets from "./Assets.svelte";

/** Asset panel (upload / grid / replace / delete). Requires core-ui's sidebar
 * region; contributes Assets at order 1 (after settings). */
export const assets: Module = {
  manifest: {
    id: "assets",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:sidebar"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "assets:sidebar", contract: "shadowcat.surface:sidebar", order: 1, component: Assets });
  },
};
