import { PANEL_CONTRACT, type Module } from "@shadowcat/core";
import Assets from "./Assets.svelte";

/** Asset panel (upload / grid / replace / delete). Requires the panel-manager's
 * contract; contributes Assets at order 1 (after chat's order 0), launcher-closed
 * by default. */
export const assets: Module = {
  manifest: {
    id: "assets",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: [PANEL_CONTRACT],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "assets:panel",
      contract: PANEL_CONTRACT,
      order: 1,
      component: Assets,
      panel: { icon: "🖼️", labelKey: "assets.tab" },
    });
  },
};
