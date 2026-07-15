import { PANEL_CONTRACT, type Module } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

/** GM game configuration: scene vision/lighting defaults + per-scene overrides,
 * light gradation, vision modes, pathfinding + movement + animation settings.
 * Requires the panel-manager's contract; contributes a GM-only configuration
 * panel after the actor/faction/condition panels (order 5), minimized by default. */
export const gameSettings: Module = {
  manifest: {
    id: "game-settings",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: [PANEL_CONTRACT],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "game-settings:panel",
      contract: PANEL_CONTRACT,
      order: 5,
      component: GameSettingsPanel,
      panel: { icon: "⚙️", labelKey: "gameSettings.tab", gmOnly: true, defaultPlacement: { kind: "minimized" } },
    });
  },
};
