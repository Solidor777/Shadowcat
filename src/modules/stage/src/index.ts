import type { Module } from "@shadowcat/core";
import Stage from "./Stage.svelte";

/** Canvas stage panel: hosts the engine-owned PixiJS render surface. Requires
 * core-ui's stage region; contributes Stage into it. */
export const stage: Module = {
  manifest: {
    id: "stage",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:stage"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "stage:stage", contract: "shadowcat.surface:stage", component: Stage });
  },
};
