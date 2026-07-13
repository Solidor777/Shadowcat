<script lang="ts">
  import { getAppContext, TabbedSurface } from "@shadowcat/ui-kit";
  const ctx = getAppContext();
  // Restore once at mount; persistence is write-through on change.
  let active = $state<string | null>(ctx.uiState.getActiveTab());
  function change(id: string): void {
    active = id;
    ctx.uiState.setActiveTab(id);
  }
</script>

<TabbedSurface contract="shadowcat.surface:sidebar" activeId={active} onTabChange={change} />
