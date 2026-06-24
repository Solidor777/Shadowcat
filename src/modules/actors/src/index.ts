import type { Module } from "@shadowcat/core";
import ActorsPanel from "./ActorsPanel.svelte";

/** Actor create/list/pick panel. Requires core-ui's sidebar region; contributes ActorsPanel. */
export const actors: Module = {
  manifest: {
    id: "actors",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:sidebar"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "actors:sidebar", contract: "shadowcat.surface:sidebar", order: 2, component: ActorsPanel });
  },
};
