import type { Module } from "@shadowcat/core";
import Settings from "./Settings.svelte";

/** Settings panel (role, locale switcher, leave-world, logout). Requires core-ui's
 * sidebar region; contributes Settings at order 6 (after game-settings' 5) so
 * chat's order 0 stays the sole default tab (TabbedSurface picks the first
 * visible contribution when no activeId matches). */
export const settings: Module = {
  manifest: {
    id: "settings",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:sidebar"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "settings:sidebar", contract: "shadowcat.surface:sidebar", order: 6, component: Settings, tab: { icon: "🔧", labelKey: "settings.tab" } });
  },
};
