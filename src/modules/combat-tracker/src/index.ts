import { PANEL_CONTRACT, type Module, type CoreHooks } from "@shadowcat/core";
import CombatTrackerPanel from "./CombatTrackerPanel.svelte";
import { TurnBadge } from "./turnBadge";

/** Default combat tracker panel: combats for the viewed scene, ordered rows, clock controls,
 * add/remove/hide/reorder, initiative rolls, resource editing, and a "your turn" notice. Pure
 * presentation over `AppContext.combat`/`ctx.documents`/`ctx.chat`/`ctx.panels`/`ctx.hooks` —
 * replaceable by any system or community module contributing its own `shadowcat.panel`. */
export const combatTracker: Module = {
  manifest: {
    id: "combat-tracker",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: [PANEL_CONTRACT],
    provides: [],
  },
  register(ctx) {
    const badge = new TurnBadge();
    ctx.contributions.contribute({
      id: "combat-tracker:panel",
      contract: PANEL_CONTRACT,
      order: 2,
      component: CombatTrackerPanel,
      props: { badge },
      panel: { icon: "⚔️", labelKey: "combatTracker.tab", badge },
    });
    ctx.hooks.on("combat:turn-start", (p) => badge.onTurnStart(p as CoreHooks["combat:turn-start"]), { requires: "^1.0.0" });
    ctx.hooks.on("combat:turn-end", (p) => badge.onTurnEnd(p as CoreHooks["combat:turn-end"]));
    ctx.hooks.on("combat:end", () => badge.clear());
  },
};
