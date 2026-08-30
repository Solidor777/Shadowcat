import { PANEL_CONTRACT, type Module } from "@shadowcat/core";
import AssetBrowserPanel from "./AssetBrowserPanel.svelte";

/** GM asset browser (folder tree / filter bar / thumbnail grid / preview
 * pane, uploads, bulk operations, and the pick-mode overlay). Requires the
 * panel-manager's contract; contributes the browser panel at order 1
 * (after chat's order 0), launcher-closed and GM-only. */
export const assetBrowser: Module = {
  manifest: {
    id: "asset-browser",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: [PANEL_CONTRACT],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "asset-browser:panel",
      contract: PANEL_CONTRACT,
      order: 1,
      component: AssetBrowserPanel,
      panel: { icon: "🖼️", labelKey: "assetBrowser.tab", gmOnly: true },
    });
  },
};
