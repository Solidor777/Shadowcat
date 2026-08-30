<script lang="ts">
  import type { Asset } from "@shadowcat/types";
  import { getAppContext, type PickAssetOptions } from "@shadowcat/ui-kit";
  import { queryAssets, type AssetQuery } from "@shadowcat/core";

  let {
    mode,
    initialFilters,
    onConfirm,
    onCancel,
  }: {
    /** `manage` = the GM panel (mutations shown); `pick` = modal pick mode
     * (mutations hidden, selection confirm bar shown). */
    mode: "manage" | "pick";
    /** Pick mode's filter/arity presets, from the `pickAsset` request. */
    initialFilters?: PickAssetOptions;
    /** Pick mode: called with the picked ids in pick order. */
    onConfirm?: (ids: string[]) => void;
    /** Pick mode: called when the pick is dismissed without a choice. */
    onCancel?: () => void;
  } = $props();

  const { world, assets: resolver, onAssetChanged, t } = getAppContext();

  let items = $state<Asset[]>([]);
  let error = $state<string | null>(null);

  /** The current query, from the mode's filter presets.
   * @returns The `queryAssets` parameters for the visible grid page.
   * @example
   * ```
   * // private function; invoked by `reload` below
   * void currentQuery();
   * ```
   */
  function currentQuery(): AssetQuery {
    return {
      kind: initialFilters?.kind,
      tags: initialFilters?.tags,
      sort: "created",
      limit: 200,
    };
  }

  /** Refetches the listing; the source of truth for the grid (assets are REST
   * resources, not store documents). `reconcile` self-heals any uuid whose
   * cache-bust state missed an `AssetChanged` frame.
   * @returns Resolves once `items` (or `error`) reflects the fresh page.
   * @example
   * ```
   * // private function; invoked from the mount/notice effect below
   * void reload();
   * ```
   */
  async function reload(): Promise<void> {
    try {
      const page = await queryAssets(world, currentQuery());
      items = page.items;
      resolver.reconcile(page.items);
      error = null;
    } catch (e) {
      error = t("assetBrowser.error", { message: String(e) });
    }
  }

  // Load on mount; reload when a broadcast invalidates listings.
  $effect(() => {
    void reload();
    return onAssetChanged((m) => {
      if (m.op === "created" || m.op === "moved" || m.op === "deleted") void reload();
    });
  });

  // Skeleton confirm/cancel wiring so pick-mode callers are exercisable; the
  // full selection model replaces this.
  void onConfirm;
  void onCancel;
  void mode;
</script>

<div class="asset-browser" data-testid="asset-browser">
  <aside class="tree" data-testid="asset-browser-tree"></aside>
  <div class="content">
    <header class="filters" data-testid="asset-browser-filters"></header>
    {#if error}
      <p class="error">{error}</p>
    {:else if items.length === 0}
      <p class="empty" data-testid="asset-browser-empty">{t("assetBrowser.empty")}</p>
    {:else}
      <div class="grid" data-testid="asset-browser-grid">
        {#each items as a (a.id)}
          <button type="button" class="tile" data-testid="asset-tile" title={a.original_name}>
            <img src={resolver.url(a.id, "thumb")} alt={a.original_name} />
          </button>
        {/each}
      </div>
    {/if}
  </div>
  <aside class="preview" data-testid="asset-browser-preview"></aside>
</div>

<style lang="scss">
  .asset-browser {
    display: grid;
    grid-template-columns: 12rem 1fr 16rem;
    gap: 0.5rem;
    height: 100%;
    min-height: 0;
  }
  .tree {
    overflow-y: auto;
    border-right: 1px solid var(--border);
  }
  .content {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .filters {
    flex: none;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(72px, 1fr));
    gap: 0.25rem;
    overflow-y: auto;
    min-height: 0;
  }
  .tile {
    min-width: 44px;
    min-height: 44px;
    padding: 0;
    border: 1px solid var(--border);
    background: var(--surface-raised);
    cursor: pointer;
    img {
      width: 100%;
      height: 100%;
      object-fit: cover;
      display: block;
    }
  }
  .preview {
    overflow-y: auto;
    border-left: 1px solid var(--border);
  }
  .empty,
  .error {
    color: var(--text-muted);
    padding: 1rem;
  }
</style>
