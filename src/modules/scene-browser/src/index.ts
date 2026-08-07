import { PANEL_CONTRACT, type Module } from "@shadowcat/core";
import SceneBrowserPanel from "./SceneBrowserPanel.svelte";

/** GM scene browser: scene list with background thumbnails, create, configure (deep-links
 * the game-settings per-scene section), local view (GM roam), and activate (sets the scene players
 * render). Requires the panel-manager's contract; launcher-closed by default, after game-settings. */
export const sceneBrowser: Module = {
  manifest: {
    id: "scene-browser",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: [PANEL_CONTRACT],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "scene-browser:panel",
      contract: PANEL_CONTRACT,
      order: 6,
      component: SceneBrowserPanel,
      panel: { icon: "🗺️", labelKey: "sceneBrowser.tab", gmOnly: true },
    });
  },
};
