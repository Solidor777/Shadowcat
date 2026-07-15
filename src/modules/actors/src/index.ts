import { PANEL_CONTRACT, type Module } from "@shadowcat/core";
import ActorsPanel from "./ActorsPanel.svelte";

/** Actor create/list/pick panel. Requires the panel-manager's contract;
 * contributes ActorsPanel launcher-closed by default. */
export const actors: Module = {
  manifest: {
    id: "actors",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: [PANEL_CONTRACT],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "actors:panel",
      contract: PANEL_CONTRACT,
      order: 2,
      component: ActorsPanel,
      panel: { icon: "👥", labelKey: "actors.tab" },
    });
  },
};
