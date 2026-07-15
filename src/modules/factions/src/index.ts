import { PANEL_CONTRACT, type Module } from "@shadowcat/core";
import FactionsPanel from "./FactionsPanel.svelte";

/** World faction registry: seeds three defaults (GM, idempotent) and provides the GM editor.
 * Replaceable — a game-system module can supply its own seed/editor. Requires the
 * panel-manager's contract; contributes FactionsPanel minimized by default. */
export const factions: Module = {
  manifest: {
    id: "factions",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: [PANEL_CONTRACT],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "factions:panel",
      contract: PANEL_CONTRACT,
      order: 3,
      component: FactionsPanel,
      panel: { icon: "🚩", labelKey: "factions.tab", defaultPlacement: { kind: "minimized" } },
    });
  },
};
