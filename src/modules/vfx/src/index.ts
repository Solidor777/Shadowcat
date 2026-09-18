import type { Module } from "@shadowcat/core";
import { PANEL_CONTRACT } from "@shadowcat/core";
import FxToolPanel from "./FxToolPanel.svelte";

/** VFX config panel + scene-tool registration (the scene-tool contribution itself is
 * registered by `FxToolPanel`'s own `$effect`, not here — see the panel's own doc). Depends
 * on core-ui (the panel provider) and communicates only through public seams (contributions,
 * AppContext). */
export const vfx: Module = {
  manifest: {
    id: "vfx",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: [PANEL_CONTRACT],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "vfx:panel",
      contract: PANEL_CONTRACT,
      order: 3,
      component: FxToolPanel,
      panel: { icon: "✨", labelKey: "vfx.tab" },
    });
  },
};
