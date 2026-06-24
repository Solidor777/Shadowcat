import type { Module } from "@shadowcat/core";
import Layout from "./Layout.svelte";

/** First-party layout module: owns the responsive region grid (Layout) and
 * declares the region surfaces. Contributes Layout into the singleton `root`
 * surface the shell hosts; each region's content is contributed by its own
 * per-element module (topbar / statusbar / stage / settings / assets / tools).
 * Replace this module to swap the whole layout. */
export const coreUi: Module = {
  manifest: {
    id: "core-ui",
    version: "0.1.0",
    dependencies: {},
    provides: [
      { contract: "shadowcat.surface:root", cardinality: "singleton" },
      { contract: "shadowcat.surface:topbar", cardinality: "singleton" },
      { contract: "shadowcat.surface:stage", cardinality: "singleton" },
      { contract: "shadowcat.surface:statusbar", cardinality: "singleton" },
      { contract: "shadowcat.surface:toolrail", cardinality: "multi" },
      { contract: "shadowcat.surface:sidebar", cardinality: "multi" },
    ],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "core-ui:root", contract: "shadowcat.surface:root", component: Layout });
  },
};
