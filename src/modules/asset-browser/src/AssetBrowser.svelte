<script lang="ts">
  import type { Asset } from "@shadowcat/types";
  import { getAppContext, type PickAssetOptions } from "@shadowcat/ui-kit";
  import { queryAssets, type AssetQuery } from "@shadowcat/core";
  import FilterBar from "./FilterBar.svelte";
  import type { FilterState } from "./filterState";
  import AssetGrid from "./AssetGrid.svelte";

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

  // The pick request's presets seed the filter ONCE — the browser owns it
  // afterward, so capturing the initial prop value is the intended semantics.
  // svelte-ignore state_referenced_locally
  let filter = $state<FilterState>({
    name: "",
    nameIsRegex: false,
    tags: initialFilters?.tags ?? [],
    kind: initialFilters?.kind,
    sort: "created",
  });
  let items = $state<Asset[]>([]);
  let nextCursor = $state<string | null>(null);
  let selected = $state<string[]>([]);
  let error = $state<string | null>(null);
  // Monotonic reload marker: a stale page resolving after a newer reload
  // started must not clobber the newer listing.
  let generation = 0;

  /** The `queryAssets` params for the current filter (`name` vs `name_regex`
   * exclusive on the toggle).
   * @returns The mapped query for the first page.
   * @example
   * ```
   * // private function; invoked by `reload`/`loadMore` below
   * void currentQuery();
   * ```
   */
  function currentQuery(): AssetQuery {
    return {
      name: filter.nameIsRegex ? undefined : filter.name || undefined,
      nameRegex: filter.nameIsRegex ? filter.name || undefined : undefined,
      tags: filter.tags.length > 0 ? filter.tags : undefined,
      kind: filter.kind,
      sort: filter.sort,
      limit: 200,
    };
  }

  /** Refetches the first page; the source of truth for the grid (assets are
   * REST resources, not store documents). `reconcile` self-heals any uuid
   * whose cache-bust state missed an `AssetChanged` frame.
   * @returns Resolves once `items` (or `error`) reflects the fresh page.
   * @example
   * ```
   * // private function; invoked from the mount/notice effect below
   * void reload();
   * ```
   */
  async function reload(): Promise<void> {
    const gen = ++generation;
    try {
      const page = await queryAssets(world, currentQuery());
      if (gen !== generation) return;
      items = page.items;
      nextCursor = page.next_cursor;
      resolver.reconcile(page.items);
      error = null;
    } catch (e) {
      if (gen !== generation) return;
      error = t("assetBrowser.error", { message: String(e) });
    }
  }

  /** Appends the next keyset page when the grid scrolls near its end.
   * @returns Resolves once the page (if any) is appended.
   * @example
   * ```
   * // private function; wired to AssetGrid's onNearEnd below
   * void loadMore();
   * ```
   */
  async function loadMore(): Promise<void> {
    if (!nextCursor) return;
    const cursor = nextCursor;
    nextCursor = null; // claims the cursor so overlapping onNearEnd fires no duplicate fetch
    const gen = generation;
    try {
      const page = await queryAssets(world, { ...currentQuery(), cursor });
      if (gen !== generation) return;
      items = [...items, ...page.items];
      nextCursor = page.next_cursor;
      resolver.reconcile(page.items);
    } catch (e) {
      if (gen !== generation) return;
      error = t("assetBrowser.error", { message: String(e) });
    }
  }

  // Leading-edge debounce for filter-driven reloads: fire immediately, absorb
  // the burst, run once more with the final state if anything changed inside
  // the window.
  let cooldown: ReturnType<typeof setTimeout> | null = null;
  let trailing = false;

  /** Applies a filter change and schedules the (debounced) refetch.
   * @param next - The full next filter state from the bar.
   * @example
   * ```
   * // private function; wired to FilterBar's onChange below
   * declare const next: FilterState;
   * onFilterChange(next);
   * ```
   */
  function onFilterChange(next: FilterState): void {
    filter = next;
    if (cooldown) {
      trailing = true;
      return;
    }
    void reload();
    cooldown = setTimeout(() => {
      cooldown = null;
      if (trailing) {
        trailing = false;
        void reload();
      }
    }, 250);
  }

  // Load on mount; reload when any asset notice lands (created/moved change
  // listings; deleted drops tiles; replaced re-renders thumbs at the bumped
  // version the resolver already reflects).
  $effect(() => {
    void reload();
    return onAssetChanged(() => void reload());
  });

</script>

<div class="asset-browser" data-testid="asset-browser">
  <aside class="tree" data-testid="asset-browser-tree"></aside>
  <div class="content">
    <FilterBar {filter} onChange={onFilterChange} />
    {#if error}
      <p class="error">{error}</p>
    {:else}
      <AssetGrid
        {items}
        {selected}
        onSelectionChange={(ids) => (selected = ids)}
        onOpen={(id) => {
          if (mode === "pick" && !initialFilters?.multiple) onConfirm?.([id]);
        }}
        onNearEnd={() => void loadMore()}
      />
    {/if}
    {#if mode === "pick"}
      <footer class="pick-bar" data-testid="pick-bar">
        <button type="button" data-testid="pick-cancel" onclick={() => onCancel?.()}>
          {t("assetBrowser.pickCancel")}
        </button>
        <button
          type="button"
          data-testid="pick-confirm"
          disabled={selected.length === 0}
          onclick={() => onConfirm?.(selected)}
        >
          {t("assetBrowser.pickConfirm")}
        </button>
      </footer>
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
  .preview {
    overflow-y: auto;
    border-left: 1px solid var(--border);
  }
  .error {
    color: var(--text-muted);
    padding: 1rem;
  }
  .pick-bar {
    flex: none;
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    padding: 0.375rem;
    border-top: 1px solid var(--border);
    button {
      min-height: 2.25rem;
      min-width: 44px;
    }
  }
</style>
