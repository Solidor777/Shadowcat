import { STAGE_OVERLAY_CONTRACT, type Module } from "@shadowcat/core";

/** 3D dice overlay module. Full registration lands once `DiceOverlay.svelte` exists. */
export const dice3d: Module = {
  manifest: {
    id: "dice3d",
    version: "0.1.0",
    dependencies: {},
    requires: [],
    provides: [{ contract: STAGE_OVERLAY_CONTRACT, cardinality: "multi" }],
  },
  register() {
    // Filled out once DiceOverlay.svelte exists.
  },
};
