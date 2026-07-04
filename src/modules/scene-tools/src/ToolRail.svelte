<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { resolveSceneSettings, type WireDocument } from "@shadowcat/core";
  import { ToolController, type ToolId, type DrawMode, type TemplateMode, type RegionShapeMode, type RegionBehaviorMode } from "./controller.svelte";
  import AssetPicker from "./AssetPicker.svelte";

  const ctx = getAppContext();
  // The controller is fixed per ToolRail instance; capturing the context once is intended.
  // svelte-ignore state_referenced_locally
  const controller = new ToolController({
    scene: ctx.scene,
    actorSelection: ctx.actorSelection,
    tokenSelection: ctx.tokenSelection,
    dispatchIntent: ctx.dispatchIntent,
    documents: ctx.documents,
    assets: ctx.assets,
    world: ctx.world,
    sendPing: ctx.sendPing,
    pathfind: ctx.pathfind,
  });
  const t = ctx.t;
  // Authoring is GM-gated (the server is authoritative; this hides the controls).
  const isGm = ctx.role === "gm";

  // Reactive subscription mirrors GameSettingsPanel's registry-seed pattern: calling
  // subscribe() inside each $derived.by registers a reactive dependency on the document
  // store so the snap toggle re-resolves as the active scene's doc changes (M10f-3 §4.4).
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const activeScene = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("scene")[0];
  });
  const snapToGrid = $derived.by((): boolean => {
    subscribe();
    return resolveSceneSettings(activeScene, ctx.documents).snapToGrid;
  });

  /** GM-authored scene-level snap toggle (M10f-3 §4.4): writes the opaque
   * `/system/snapToGrid` field on the active scene document (shared, not local UI state).
   * No-op with no active scene. */
  function toggleSnap(): void {
    const scene = activeScene;
    if (!scene) return;
    // Reads the RAW stored value (not the resolved/defaulted `snapToGrid`) for optimistic-
    // concurrency `old`: the server's field-level conflict check compares against the actual
    // stored value at this path, which is only `null` while the field is genuinely absent.
    // Mirrors controller.svelte.ts's sendMoves convention (`sys?.x ?? null`).
    const rawSnap = (scene.system as { snapToGrid?: boolean } | undefined)?.snapToGrid ?? null;
    ctx.dispatchIntent([
      { op: "update", doc_id: scene.id, changes: [{ path: "/system/snapToGrid", old: rawSnap, new: !snapToGrid }] },
    ]);
  }

  const tools: { id: ToolId; label: string }[] = [
    { id: "select", label: t("tools.select") },
    { id: "place", label: t("tools.place") },
    { id: "draw", label: t("tools.draw") },
    { id: "template", label: t("tools.template") },
    { id: "measure", label: t("tools.measure") },
    { id: "ping", label: t("tools.ping") },
    { id: "wall", label: t("tools.wall") },
    { id: "region", label: t("tools.region") },
  ];
  const drawModes: DrawMode[] = ["freehand", "rect", "ellipse", "line"];
  const templateModes: TemplateMode[] = ["circle", "cone", "rect", "line"];
  const regionShapeModes: RegionShapeMode[] = ["rect", "circle", "polygon"];
  const regionBehaviors: RegionBehaviorMode[] = ["terrain", "impassable", "arrest"];
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

    {#if activeScene}
      <button
        type="button"
        class="tool"
        aria-pressed={snapToGrid}
        data-testid="snap-toggle"
        title={t("tools.snap")}
        onclick={toggleSnap}
      >
        {t("tools.snap")}
      </button>
    {/if}

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
    {:else if controller.active === "region"}
      <div class="controls">
        <select data-testid="region-shape" aria-label={t("tools.shape")} bind:value={controller.regionShapeMode}>
          {#each regionShapeModes as m (m)}<option value={m}>{m}</option>{/each}
        </select>
        <select data-testid="region-behavior" aria-label={t("tools.behavior")} bind:value={controller.regionBehavior}>
          {#each regionBehaviors as b (b)}<option value={b}>{b}</option>{/each}
        </select>
        <input type="number" data-testid="region-cost" aria-label={t("tools.cost")} min="1" step="0.5" bind:value={controller.regionCost} disabled={controller.regionBehavior !== "terrain"} />
        <label>
          <input type="checkbox" data-testid="region-secret" bind:checked={controller.regionSecret} />
          {t("tools.secret")}
        </label>
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
