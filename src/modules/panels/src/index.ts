// Public entry point for @shadowcat/module-panels. Re-exports the pure layout
// tree surface plus the panel-host runtime (engine seam + FakeEngine + host
// components); a Module registration wiring these into contribution surfaces
// composes on top of this barrel.
export * from "./layout/tree";
export * from "./layout/persist";
export type { EngineAdapter } from "./engine/adapter";
export { FakeEngine } from "./engine/fake";
export { DockviewEngine } from "./engine/dockview";
export { classifyDrop, STAGE_ID, type DropSite, type ClassifyResult } from "./engine/policy";
export { default as PanelHost } from "./PanelHost.svelte";
export { default as CompactSwitcher } from "./CompactSwitcher.svelte";
export { default as DockChips } from "./DockChips.svelte";
