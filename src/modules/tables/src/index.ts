import { PANEL_CONTRACT, type Module } from "@shadowcat/core";
import TablesPanel from "./TablesPanel.svelte";

/** Rollable-table list + quick-draw panel. Requires the panel-manager's contract;
 * contributes TablesPanel launcher-closed by default. Not `gmOnly` — a player draws from a
 * table they can read. */
export const tables: Module = {
  manifest: {
    id: "tables",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: [PANEL_CONTRACT],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "tables:panel",
      contract: PANEL_CONTRACT,
      order: 3,
      component: TablesPanel,
      panel: { icon: "📋", labelKey: "tables.tab" },
    });
  },
};
