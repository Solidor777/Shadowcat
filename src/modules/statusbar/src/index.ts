import type { Module } from "@shadowcat/core";
import StatusBar from "./StatusBar.svelte";

/** Status bar panel. Requires core-ui's statusbar region; contributes StatusBar. */
export const statusBar: Module = {
  manifest: {
    id: "statusbar",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:statusbar"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "statusbar:statusbar", contract: "shadowcat.surface:statusbar", component: StatusBar });
  },
};
