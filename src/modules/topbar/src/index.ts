import type { Module } from "@shadowcat/core";
import TopBar from "./TopBar.svelte";

/** Top bar panel. Requires core-ui's topbar region; contributes TopBar into it. */
export const topBar: Module = {
  manifest: {
    id: "topbar",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:topbar"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "topbar:topbar", contract: "shadowcat.surface:topbar", component: TopBar });
  },
};
