import { PANEL_CONTRACT, type Module } from "@shadowcat/core";
import Settings from "./Settings.svelte";

/** Settings panel (role, locale switcher, leave-world, logout). Requires the
 * panel-manager's contract; contributes Settings at order 6 (after
 * game-settings' 5) so chat's order 0 stays the sole default docked panel,
 * launcher-closed by default. */
export const settings: Module = {
  manifest: {
    id: "settings",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: [PANEL_CONTRACT],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "settings:panel",
      contract: PANEL_CONTRACT,
      order: 6,
      component: Settings,
      panel: { icon: "🔧", labelKey: "settings.tab" },
    });
  },
};
