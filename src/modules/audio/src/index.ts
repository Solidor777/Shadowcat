import { PANEL_CONTRACT, type Module } from "@shadowcat/core";
import AudioPanel from "./AudioPanel.svelte";

/** The Audio panel: channel sliders/mutes, now-playing transport (GM), and the playlists
 * list. Panel `order` after `tables:panel` — grouped with the other content panels, before
 * the sheet-contributing modules in `App.svelte`'s registration order. */
export const audio: Module = {
  manifest: {
    id: "audio",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: [PANEL_CONTRACT],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "audio:panel",
      contract: PANEL_CONTRACT,
      order: 4,
      component: AudioPanel,
      panel: { icon: "🔊", labelKey: "audio.tab" },
    });
  },
};
