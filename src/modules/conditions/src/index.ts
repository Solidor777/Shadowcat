import type { Module } from "@shadowcat/core";
import ConditionsPanel from "./ConditionsPanel.svelte";

/** World condition registry: seeds a generic emoji set (GM, idempotent) + a GM editor, and a
 * selection-driven toggle palette. Replaceable — a game-system module can supply its own
 * seed/editor. Requires core-ui's sidebar. */
export const conditions: Module = {
  manifest: {
    id: "conditions",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:sidebar"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "conditions:sidebar", contract: "shadowcat.surface:sidebar", order: 4, component: ConditionsPanel });
  },
};
