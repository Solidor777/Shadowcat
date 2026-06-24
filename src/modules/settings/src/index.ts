import type { Module } from "@shadowcat/core";
import Settings from "./Settings.svelte";

/** Settings panel (role, locale switcher, leave-world, logout). Requires core-ui's
 * sidebar region; contributes Settings at order 0. */
export const settings: Module = {
  manifest: {
    id: "settings",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:sidebar"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "settings:sidebar", contract: "shadowcat.surface:sidebar", order: 0, component: Settings });
  },
};
