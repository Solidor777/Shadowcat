<script lang="ts">
  import { getAppContext } from "../../lib/appContext";
  import { ToolController, type ToolId } from "./controller.svelte";
  import AssetPicker from "./AssetPicker.svelte";

  const ctx = getAppContext();
  // The controller is fixed per ToolRail instance; capturing the context once is intended.
  // svelte-ignore state_referenced_locally
  const controller = new ToolController({
    scene: ctx.scene,
    dispatchIntent: ctx.dispatchIntent,
    documents: ctx.documents,
    assets: ctx.assets,
    world: ctx.world,
  });
  const t = ctx.t;
  // Authoring is GM-gated (the server is authoritative; this hides the controls).
  const isGm = ctx.role === "gm";

  const tools: { id: ToolId; label: string }[] = [
    { id: "select", label: t("tools.select") },
    { id: "place", label: t("tools.place") },
  ];
</script>

{#if isGm}
  <div class="tool-rail" role="toolbar" aria-label={t("tools.title")}>
    {#each tools as tool (tool.id)}
      <button
        type="button"
        class="tool"
        class:active={controller.active === tool.id}
        aria-pressed={controller.active === tool.id}
        data-testid="tool-{tool.id}"
        title={tool.label}
        onclick={() => controller.toggle(tool.id)}
      >
        {tool.label}
      </button>
    {/each}
    {#if controller.active === "place"}
      <AssetPicker {controller} />
    {/if}
  </div>
{/if}

<style lang="scss">
  .tool-rail {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
  }
  .tool {
    min-height: 44px; /* touch target (#10) */
    min-width: 44px;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
  }
  .tool.active {
    background: var(--accent);
    color: var(--on-accent);
    border-color: var(--accent);
  }
  .tool:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
