<script lang="ts">
  import type { Asset } from "@shadowcat/types";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { listAssets } from "../../lib/api";
  import type { ToolController } from "./controller.svelte";

  let { controller }: { controller: ToolController } = $props();
  const { world, assets, t } = getAppContext();

  let items = $state<Asset[]>([]);
  let failed = $state(false);

  // Load the world's image assets (token art) once the picker is shown.
  $effect(() => {
    let alive = true;
    listAssets(world)
      .then((a) => {
        if (alive) items = a.filter((x) => x.content_type.startsWith("image/"));
      })
      .catch(() => {
        if (alive) failed = true;
      });
    return () => {
      alive = false;
    };
  });
</script>

<div class="asset-picker">
  <p class="hint">{t("tools.placeHint")}</p>
  {#if failed}
    <p class="error">{t("assets.error", { message: "" })}</p>
  {:else if items.length === 0}
    <p class="empty">{t("assets.empty")}</p>
  {:else}
    <div class="grid">
      {#each items as a (a.id)}
        <button
          type="button"
          class="tile"
          class:selected={controller.selectedAsset === a.id}
          data-testid="picker-asset"
          aria-pressed={controller.selectedAsset === a.id}
          title={a.original_name}
          onclick={() => (controller.selectedAsset = a.id)}
        >
          <img src={assets.url(a.id)} alt={a.original_name} />
        </button>
      {/each}
    </div>
  {/if}
</div>

<style lang="scss">
  .asset-picker {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
  }
  .hint {
    font-size: 0.85em;
    color: var(--text-muted);
    margin: 0;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(44px, 1fr));
    gap: var(--space-1);
  }
  .tile {
    padding: 0;
    border: 2px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    cursor: pointer;
    aspect-ratio: 1;
    overflow: hidden;
  }
  .tile.selected {
    border-color: var(--accent);
  }
  .tile img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .tile:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
</style>
