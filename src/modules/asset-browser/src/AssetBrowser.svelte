<script lang="ts">
  import type { Asset } from "@shadowcat/types";
  import { getAppContext, sizeClass, type PickAssetOptions } from "@shadowcat/ui-kit";
  import { queryAssets, patchAsset, bulkPatchAssets, type AssetQuery } from "@shadowcat/core";
  import FilterBar from "./FilterBar.svelte";
  import type { FilterState } from "./filterState";
  import AssetGrid from "./AssetGrid.svelte";
  import FolderTree from "./FolderTree.svelte";
  import PreviewPane from "./PreviewPane.svelte";
  import BulkBar from "./BulkBar.svelte";
  import UploadQueue from "./UploadQueue.svelte";
  import { UploadQueue as UploadQueueModel } from "./uploadQueueModel.svelte";
  import PickConfirmBar from "./PickConfirmBar.svelte";

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
  /** The folder filtering the grid (`null` = all assets, recursive off). */
  let selectedFolder = $state<string | null>(null);
  /** Compact-mode drawer visibility for the folder tree. */
  let treeOpen = $state(false);
  const compact = $derived(sizeClass() === "compact");
  /** Mutation affordances render only in the managing panel, never pick mode. */
  const mutable = $derived(mode === "manage");
  /** The single selected asset shown in the preview pane, or null. */
  const previewAsset = $derived(
    selected.length === 1 ? (items.find((a) => a.id === selected[0]) ?? null) : null,
  );
  /** The browser's upload queue; refreshes the listing per created asset. */
  const uploads = new UploadQueueModel(world, () => void reload());
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
      folder: selectedFolder ?? undefined,
      recursive: selectedFolder ? true : undefined,
      name: filter.nameIsRegex ? undefined : filter.name || undefined,
      nameRegex: filter.nameIsRegex ? filter.name || undefined : undefined,
      tags: filter.tags.length > 0 ? filter.tags : undefined,
      kind: filter.kind,
      sort: filter.sort,
      limit: 200,
    };
  }

  /** Files dropped assets into `folderId` (single patch or one bulk call),
   * then refreshes the listing.
   * @param ids - The dragged asset ids.
   * @param folderId - The drop-target folder.
   * @example
   * ```
   * // private function; wired to FolderTree's onDropAssets below
   * void fileAssets(["a1"], "folder-1");
   * ```
   */
  async function fileAssets(ids: string[], folderId: string): Promise<void> {
    try {
      if (ids.length === 1) await patchAsset(ids[0], { folder_id: folderId });
      else await bulkPatchAssets(world, { ids, folder_id: folderId, add_tags: [], remove_tags: [] });
      await reload();
    } catch (e) {
      error = t("assetBrowser.error", { message: String(e) });
    }
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

<div class="asset-browser" class:compact data-testid="asset-browser">
  {#if !compact || treeOpen}
    <aside class="tree" data-testid="asset-browser-tree">
      <FolderTree
        {selectedFolder}
        {mutable}
        onSelectFolder={(id) => {
          selectedFolder = id;
          treeOpen = false;
          void reload();
        }}
        onDropAssets={(ids, folderId) => void fileAssets(ids, folderId)}
        onDropFiles={mutable ? (files, folderId) => uploads.enqueue(files, folderId) : undefined}
      />
    </aside>
  {/if}
  <div
    class="content"
    role="presentation"
    ondragover={(e) => {
      if (mutable && e.dataTransfer?.types.includes("Files")) e.preventDefault();
    }}
    ondrop={(e) => {
      if (!mutable) return;
      const files = Array.from(e.dataTransfer?.files ?? []);
      if (files.length > 0) {
        e.preventDefault();
        uploads.enqueue(files, selectedFolder);
      }
    }}
  >
    {#if compact}
      <button
        type="button"
        class="tree-toggle"
        data-testid="tree-toggle"
        aria-expanded={treeOpen}
        onclick={() => (treeOpen = !treeOpen)}
      >📁</button>
    {/if}
    <FilterBar {filter} onChange={onFilterChange} />
    {#if mutable}
      <label class="upload-fallback">
        {t("assetBrowser.upload")}
        <input
          type="file"
          multiple
          data-testid="asset-upload-input"
          onchange={(e) => {
            const files = Array.from(e.currentTarget.files ?? []);
            e.currentTarget.value = "";
            if (files.length > 0) uploads.enqueue(files, selectedFolder);
          }}
        />
      </label>
    {/if}
    {#if error}
      <p class="error">{error}</p>
    {:else}
      <AssetGrid
        {items}
        {selected}
        onSelectionChange={(ids) => (selected = ids)}
        appendOnClick={mode === "pick" && initialFilters?.multiple === true}
        onOpen={(id) => {
          if (mode === "pick" && !initialFilters?.multiple) onConfirm?.([id]);
        }}
        onNearEnd={() => void loadMore()}
      />
    {/if}
    <UploadQueue queue={uploads} />
    {#if mutable && selected.length > 1}
      <BulkBar {selected} onChanged={() => void reload()} />
    {/if}
    {#if mode === "pick"}
      <PickConfirmBar
        count={selected.length}
        onConfirm={() => onConfirm?.(selected)}
        onCancel={() => onCancel?.()}
      />
    {/if}
  </div>
  <aside class="preview" data-testid="asset-browser-preview">
    {#if previewAsset}
      <PreviewPane asset={previewAsset} {mutable} onChanged={() => void reload()} />
    {/if}
  </aside>
</div>

<style lang="scss">
  .asset-browser {
    display: grid;
    grid-template-columns: 12rem 1fr 16rem;
    gap: 0.5rem;
    height: 100%;
    min-height: 0;
    // Compact (<48rem): single column; the tree becomes a toggleable drawer
    // overlaying the content column, the preview stacks below.
    &.compact {
      grid-template-columns: 1fr;
      .tree {
        position: absolute;
        inset: 0 30% 0 0;
        z-index: 10;
        background: var(--surface-raised);
        border-right: 1px solid var(--border);
      }
      .preview {
        border-left: none;
        border-top: 1px solid var(--border);
      }
    }
    position: relative;
  }
  .tree-toggle {
    align-self: flex-start;
    min-width: 44px;
    min-height: 2rem;
    margin: 0.25rem;
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
</style>
