// Public entry point for @shadowcat/module-panels. Re-exports the pure layout
// tree surface plus the panel-host runtime (engine seam + FakeEngine + host
// components) and the `panels` Module registration below.
import { PANEL_CONTRACT, type Module } from "@shadowcat/core";
import PanelHost from "./PanelHost.svelte";
import DockChips from "./DockChips.svelte";

export * from "./layout/tree";
export * from "./layout/persist";
export type { EngineAdapter } from "./engine/adapter";
export { FakeEngine } from "./engine/fake";
export { DockviewEngine } from "./engine/dockview";
export { classifyDrop, STAGE_ID, type DropSite, type ClassifyResult } from "./engine/policy";
export { PanelsController, regsForRole, type PanelsBridgeLike, type PanelsControllerDeps } from "./controller.svelte";
export { default as PanelHost } from "./PanelHost.svelte";
export { default as CompactSwitcher } from "./CompactSwitcher.svelte";
export { default as DockChips } from "./DockChips.svelte";

/** Panel-manager module: hosts the dockable panel surface into core-ui's
 * singleton `panel-host` region and the minimized-chips strip into
 * statusbar's singleton `panel-dock` region, and in turn provides the multi
 * `shadowcat.panel` contract every panel module (chat, assets, actors, ...)
 * contributes into — the panel-contract mirror of how the sidebar module
 * owns the `sidebar`/`sidebar-host` contract pair.
 *
 * `register` runs in the framework-neutral `ModuleContext` (no AppContext:
 * no role, no `uiState`, no `PanelsBridge`), so it cannot construct the
 * `PanelsController` that owns persisted layout state — `PanelHost` builds
 * its own controller lazily at mount, from AppContext, once it has one.
 * TODO: give the `panels:chips` contribution live props (minimized ids +
 * restore callback) once the `panel-dock` surface is real; that needs a
 * cross-layer seam sharing ONE `PanelsController` between this contribution
 * and `PanelHost`'s, which doesn't exist yet. */
export const panels: Module = {
  manifest: {
    id: "panels",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:panel-host", "shadowcat.surface:panel-dock"],
    provides: [{ contract: PANEL_CONTRACT, cardinality: "multi" }],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "panels:host",
      contract: "shadowcat.surface:panel-host",
      component: PanelHost,
    });
    ctx.contributions.contribute({
      id: "panels:chips",
      contract: "shadowcat.surface:panel-dock",
      component: DockChips,
    });
  },
};
