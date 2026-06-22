import type { Module } from "@shadowcat/core";
import ToolRail from "./ToolRail.svelte";

/** First-party canvas tools module. Contributes the tool rail into core-ui's toolrail
 * surface; owns place/select/move tools. Depends on core-ui (the toolrail provider) and
 * communicates only through public seams (contributions, AppContext) — never imports
 * core-ui internals (the contract-only element boundary). */
export const sceneTools: Module = {
  manifest: {
    id: "scene-tools",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:toolrail"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "scene-tools:toolrail",
      contract: "shadowcat.surface:toolrail",
      component: ToolRail,
    });
  },
};
