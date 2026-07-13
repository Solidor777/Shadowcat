import type { Module } from "@shadowcat/core";
import SidebarHost from "./SidebarHost.svelte";

/** Sidebar presentation module: hosts the tabbed rail into core-ui's singleton
 * `sidebar-host` region and, in turn, provides the multi `sidebar` contract every
 * panel module (chat, factions, conditions, game-settings, ...) contributes into.
 * Owns per-world active-tab persistence via the AppContext `uiState` seam.
 * Replaceable — swap this module to change the sidebar's presentation entirely. */
export const sidebar: Module = {
  manifest: {
    id: "sidebar",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:sidebar-host"],
    provides: [{ contract: "shadowcat.surface:sidebar", cardinality: "multi" }],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "sidebar:host",
      contract: "shadowcat.surface:sidebar-host",
      component: SidebarHost,
    });
  },
};
