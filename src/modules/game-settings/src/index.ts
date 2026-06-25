import type { Module } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

/** GM game configuration: scene vision/lighting defaults + per-scene overrides,
 * light gradation, vision modes, pathfinding + movement + animation settings.
 * Requires core-ui's sidebar; contributes a GM configuration panel to the sidebar
 * after the actor/faction/condition panels (order 5). */
export const gameSettings: Module = {
  manifest: {
    id: "game-settings",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:sidebar"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "game-settings:sidebar", contract: "shadowcat.surface:sidebar", order: 5, component: GameSettingsPanel });
  },
};
