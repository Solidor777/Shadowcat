import type { Module } from "@shadowcat/core";
import StatusBar from "./StatusBar.svelte";

/** Status bar panel. Requires core-ui's statusbar region; contributes StatusBar.
 * Also declares the singleton `panel-dock` surface — the hosting module owns
 * the surface it renders into, per the `core-ui`/`panel-host` precedent — for
 * the panel-manager's minimized-panel chip strip (StatusBar renders it). */
export const statusBar: Module = {
  manifest: {
    id: "statusbar",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:statusbar"],
    provides: [{ contract: "shadowcat.surface:panel-dock", cardinality: "singleton" }],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "statusbar:statusbar", contract: "shadowcat.surface:statusbar", component: StatusBar });
  },
};
