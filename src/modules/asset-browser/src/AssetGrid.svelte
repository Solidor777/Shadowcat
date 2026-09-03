<script lang="ts">
  import type { Asset } from "@shadowcat/types";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { computeGridWindow } from "./windowing";

  let {
    items,
    selected,
    onSelectionChange,
    onOpen,
    onNearEnd,
    appendOnClick = false,
  }: {
    /** The filtered listing, in server sort order. */
    items: Asset[];
    /** Selected asset ids (owned by the browser). */
    selected: string[];
    /** Called with the full next selection on any selection gesture. */
    onSelectionChange: (ids: string[]) => void;
    /** Double-click / keyboard activation of one tile. */
    onOpen?: (id: string) => void;
    /** Fired when the viewport scrolls near the last row (load-more). */
    onNearEnd?: () => void;
    /** Ordered multi-pick mode: a plain click APPENDS/toggles (preserving
     * pick order, shown as tile badges) instead of replacing the selection. */
    appendOnClick?: boolean;
  } = $props();

  const { assets: resolver, t } = getAppContext();

  // Scroll geometry backing the virtualized window — written only from real
  // measurement points (scroll handler, mount), never inferred.
  let container = $state<HTMLDivElement | null>(null);
  let scrollTop = $state(0);
  let clientHeight = $state(0);
  let scrollHeight = $state(0);
  let clientWidth = $state(0);

  /** Re-measures the container's geometry into the `$state` mirrors above.
   * @example
   * ```
   * // private function; invoked from the scroll handler and mount effect below
   * syncScrollState();
   * ```
   */
  function syncScrollState(): void {
    if (!container) return;
    scrollTop = container.scrollTop;
    clientHeight = container.clientHeight;
    scrollHeight = container.scrollHeight;
    clientWidth = container.clientWidth;
  }

  $effect(() => {
    syncScrollState();
    if (!container || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => syncScrollState());
    ro.observe(container);
    return () => ro.disconnect();
  });

  // Tile track is minmax(72px,1fr) with a 0.25rem gap → ~76px per column.
  const columns = $derived(Math.max(1, Math.floor(clientWidth / 76)));
  const windowed = $derived.by(() =>
    computeGridWindow(scrollTop, clientHeight, scrollHeight, items.length, columns),
  );
  const windowedItems = $derived.by(() => items.slice(windowed.start, windowed.end));
  // Spacer heights keep the scrollbar's proportion stable as the window moves;
  // derived from the container's own current layout like the window itself.
  const rowHeight = $derived.by(() => {
    const rows = Math.ceil(items.length / columns);
    return rows > 0 && scrollHeight > 0 ? scrollHeight / rows : 0;
  });
  const topSpacer = $derived(Math.floor(windowed.start / columns) * rowHeight);
  const bottomSpacer = $derived(
    Math.max(0, Math.ceil((items.length - windowed.end) / columns)) * rowHeight,
  );

  // Shift-range anchor: the last plainly-clicked index.
  let anchor = $state<number | null>(null);

  /** Applies the click gesture's selection semantics for the tile at `idx`:
   * plain = only, ctrl/meta = toggle, shift = contiguous range from the anchor.
   * @param idx - The clicked tile's index into `items`.
   * @param ev - The click event carrying the modifier keys.
   * @example
   * ```
   * // private function; wired to each tile's onclick below
   * declare const ev: MouseEvent;
   * select(0, ev);
   * ```
   */
  function select(idx: number, ev: MouseEvent): void {
    const id = items[idx].id;
    if (ev.shiftKey && anchor !== null) {
      const [lo, hi] = anchor <= idx ? [anchor, idx] : [idx, anchor];
      onSelectionChange(items.slice(lo, hi + 1).map((a) => a.id));
      return;
    }
    if (ev.ctrlKey || ev.metaKey || appendOnClick) {
      anchor = idx;
      onSelectionChange(
        selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id],
      );
      return;
    }
    anchor = idx;
    onSelectionChange([id]);
  }

  /** Scroll handler: refreshes geometry and fires load-more near the end.
   * @example
   * ```
   * // private function; wired to the viewport's onscroll below
   * onScroll();
   * ```
   */
  function onScroll(): void {
    syncScrollState();
    if (
      onNearEnd &&
      scrollHeight > 0 &&
      scrollTop + clientHeight >= scrollHeight - 2 * Math.max(rowHeight, 76)
    ) {
      onNearEnd();
    }
  }
</script>

<div
  class="grid-viewport"
  data-testid="asset-browser-grid"
  bind:this={container}
  onscroll={onScroll}
>
  {#if items.length === 0}
    <p class="empty" data-testid="asset-browser-empty">{t("assetBrowser.empty")}</p>
  {:else}
    <div style:height="{topSpacer}px"></div>
    <div class="grid">
      {#each windowedItems as a, i (a.id)}
        <button
          type="button"
          class="tile"
          class:selected={selected.includes(a.id)}
          data-testid="asset-tile"
          aria-pressed={selected.includes(a.id)}
          title={a.original_name}
          draggable="true"
          ondragstart={(e) =>
            e.dataTransfer?.setData(
              "application/x-shadowcat-assets",
              JSON.stringify(selected.includes(a.id) ? selected : [a.id]),
            )}
          onclick={(e) => select(windowed.start + i, e)}
          ondblclick={() => onOpen?.(a.id)}
        >
          <img src={resolver.url(a.id, "thumb")} alt={a.original_name} loading="lazy" />
          {#if appendOnClick && selected.includes(a.id)}
            <span class="order-badge">{selected.indexOf(a.id) + 1}</span>
          {/if}
        </button>
      {/each}
    </div>
    <div style:height="{bottomSpacer}px"></div>
  {/if}
</div>

<style lang="scss">
  .grid-viewport {
    overflow-y: auto;
    min-height: 0;
    flex: 1;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(72px, 1fr));
    gap: 0.25rem;
    padding: 0.25rem;
  }
  .tile {
    position: relative;
    min-width: 44px;
    min-height: 44px;
    aspect-ratio: 1;
    padding: 0;
    border: 1px solid var(--border);
    background: var(--surface-raised);
    cursor: pointer;
    &.selected {
      outline: 2px solid var(--accent);
      outline-offset: -2px;
    }
    img {
      width: 100%;
      height: 100%;
      object-fit: cover;
      display: block;
    }
    .order-badge {
      position: absolute;
      top: 2px;
      right: 2px;
      min-width: 1.1rem;
      border-radius: 0.55rem;
      background: var(--accent);
      color: var(--on-accent);
      font-size: 0.7rem;
      text-align: center;
    }
  }
  .empty {
    color: var(--text-muted);
    padding: 1rem;
  }
</style>
