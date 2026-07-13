import { PANEL_CONTRACT, type Module } from "@shadowcat/core";
import ConditionsPanel from "./ConditionsPanel.svelte";

/** World condition registry: seeds a generic emoji set (GM, idempotent) + a GM editor, and a
 * selection-driven toggle palette. Replaceable — a game-system module can supply its own
 * seed/editor. Requires the panel-manager's contract; contributes ConditionsPanel minimized
 * by default. */
export const conditions: Module = {
  manifest: {
    id: "conditions",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: [PANEL_CONTRACT],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "conditions:panel",
      contract: PANEL_CONTRACT,
      order: 4,
      component: ConditionsPanel,
      panel: { icon: "✨", labelKey: "conditions.tab", defaultPlacement: { kind: "minimized" } },
    });
  },
};
