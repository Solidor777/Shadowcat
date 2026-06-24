<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { ToolController, type ToolId, type DrawMode, type TemplateMode } from "./controller.svelte";
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
    sendPing: ctx.sendPing,
  });
  const t = ctx.t;
  // Authoring is GM-gated (the server is authoritative; this hides the controls).
  const isGm = ctx.role === "gm";

  const tools: { id: ToolId; label: string }[] = [
    { id: "select", label: t("tools.select") },
    { id: "place", label: t("tools.place") },
    { id: "draw", label: t("tools.draw") },
    { id: "template", label: t("tools.template") },
    { id: "measure", label: t("tools.measure") },
    { id: "ping", label: t("tools.ping") },
    { id: "wall", label: t("tools.wall") },
  ];
  const drawModes: DrawMode[] = ["freehand", "rect", "ellipse", "line"];
  const templateModes: TemplateMode[] = ["circle", "cone", "rect", "line"];
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
    {:else if controller.active === "draw"}
      <div class="controls">
        <select data-testid="draw-mode" aria-label={t("tools.shape")} bind:value={controller.drawMode}>
          {#each drawModes as m (m)}<option value={m}>{m}</option>{/each}
        </select>
        <input type="color" data-testid="draw-color" aria-label={t("tools.color")} bind:value={controller.strokeColor} />
      </div>
    {:else if controller.active === "template"}
      <div class="controls">
        <select data-testid="template-mode" aria-label={t("tools.shape")} bind:value={controller.templateMode}>
          {#each templateModes as m (m)}<option value={m}>{m}</option>{/each}
        </select>
        <input type="color" data-testid="template-color" aria-label={t("tools.color")} bind:value={controller.templateColor} />
      </div>
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
  .controls {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .controls select,
  .controls input {
    min-height: 32px;
  }
</style>
