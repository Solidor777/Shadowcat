<script lang="ts">
  // Wraps `DockChips` for the standalone `panels:chips` contribution into the
  // `shadowcat.surface:panel-dock` region (rendered by an entirely different
  // Surface/host than `PanelHost`'s own inline `DockChips`). Reads live
  // minimized/meta state through the shared `PanelsBridge` instance on
  // AppContext — the same bridge `PanelsController` binds itself into at
  // mount — so this reflects layout changes made after this contribution
  // itself mounted, not just a one-time snapshot taken at registration
  // (`register()` runs in the framework-neutral `ModuleContext`, which has no
  // AppContext to read a live controller from directly).
  import { getAppContext } from "@shadowcat/ui-kit";
  import DockChips from "./DockChips.svelte";

  const ctx = getAppContext();
  const minimized = $derived(ctx.panels.minimized);
  const meta = $derived(ctx.panels.metaMap);
</script>

<DockChips {minimized} {meta} onRestore={(id) => ctx.panels.restore(id)} />
