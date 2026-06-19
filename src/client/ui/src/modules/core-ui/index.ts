import type { Module } from "@shadowcat/core";
import Settings from "./panels/Settings.svelte";
import StagePlaceholder from "./panels/StagePlaceholder.svelte";
import TopBar from "./panels/TopBar.svelte";
import StatusBar from "./panels/StatusBar.svelte";

/** First-party shell module: provides the region surfaces and contributes the
 * M7 default panels. Region content for M8+ tools / M11 chat / M12 browsers is
 * contributed by their own modules later. */
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
    ctx.contributions.contribute({ id: "core-ui:topbar", contract: "shadowcat.surface:topbar", component: TopBar });
    ctx.contributions.contribute({ id: "core-ui:stage", contract: "shadowcat.surface:stage", component: StagePlaceholder });
    ctx.contributions.contribute({ id: "core-ui:statusbar", contract: "shadowcat.surface:statusbar", component: StatusBar });
    ctx.contributions.contribute({ id: "core-ui:settings", contract: "shadowcat.surface:sidebar", order: 0, component: Settings });
  },
};
