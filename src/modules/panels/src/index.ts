// Public entry point for @shadowcat/module-panels. Re-exports the pure layout
// tree surface plus the panel-host runtime (engine seam + FakeEngine + host
// components) and the `panels` Module registration below.
import { consoleLogger, PANEL_CONTRACT, type Module } from "@shadowcat/core";
import PanelHost from "./PanelHost.svelte";
import DockChipsContribution from "./DockChipsContribution.svelte";
import { DockviewEngine } from "./engine/dockview";

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
export { default as DockChipsContribution } from "./DockChipsContribution.svelte";

/** Panel-manager module: hosts the dockable panel surface into core-ui's
 * singleton `panel-host` region and the minimized-chips strip into
 * statusbar's singleton `panel-dock` region, and in turn provides the multi
 * `shadowcat.panel` contract every panel module (chat, assets, actors, ...)
 * contributes into.
 *
 * `register` runs in the framework-neutral `ModuleContext` (no AppContext: no
 * role, no `uiState`, no `PanelsBridge`), so it cannot construct the
 * `PanelsController` that owns persisted layout state itself. `PanelHost`
 * builds its own controller lazily at mount, from AppContext, and binds it
 * into the shell's shared `PanelsBridge` (`ctx.panels`); `DockChipsContribution`
 * reads the SAME bridge reactively, so the `panel-dock` chip strip stays live
 * without needing its own controller instance. */
export const panels: Module = {
  manifest: {
    id: "panels",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:panel-host", "shadowcat.surface:panel-dock"],
    provides: [{ contract: PANEL_CONTRACT, cardinality: "multi" }],
  },
  register(ctx) {
    // Composition root for the production docking engine: `register()` runs
    // exactly once per module install, so this constructs exactly one
    // `DockviewEngine` instance for the app's one `panel-host` surface.
    // `FakeEngine` remains the default only where a caller constructs
    // `PanelHost` directly WITHOUT an `engine` prop (tests, and the
    // bespoke-fallback seam the `EngineAdapter` doc comment describes) — this
    // contribution always supplies the real engine, so that default path is
    // never reached in production.
    ctx.contributions.contribute({
      id: "panels:host",
      contract: "shadowcat.surface:panel-host",
      component: PanelHost,
      props: { engine: new DockviewEngine(consoleLogger()) },
    });
    ctx.contributions.contribute({
      id: "panels:chips",
      contract: "shadowcat.surface:panel-dock",
      component: DockChipsContribution,
    });
  },
};
