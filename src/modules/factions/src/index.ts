import type { Module } from "@shadowcat/core";
import FactionsPanel from "./FactionsPanel.svelte";

/** World faction registry: seeds three defaults (GM, idempotent) and provides the GM editor.
 * Replaceable — a game-system module can supply its own seed/editor. Requires core-ui's sidebar. */
export const factions: Module = {
  manifest: {
    id: "factions",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:sidebar"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "factions:sidebar", contract: "shadowcat.surface:sidebar", order: 3, component: FactionsPanel });
  },
};
