<script lang="ts">
  import type { Asset } from "@shadowcat/types";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { createDragController, fmtBytes, type DragTarget, type DropResult } from "@shadowcat/core";
  import { computeListWindow } from "./windowing";
  import { applySelectionGesture } from "./selectionGesture";

  /** The fixed height of one row in px; also the touch-target minimum. */
  const ROW_HEIGHT = 44;

  let {
    items,
    selected,
    favorites = [],
    onToggleFavorite,
    onSelectionChange,
    onOpen,
    onNearEnd,
    appendOnClick = false,
    getDropTargets,
    onDropAssets,
    onDragHover,
  }: {
    /** The filtered listing, in server sort order. */
    items: Asset[];
    /** Selected asset ids (owned by the browser). */
    selected: string[];
    /** The account's favorite asset ids, drawn as a filled star on their rows. */
    favorites?: string[];
    /** Called with an asset id when its star is pressed. Omitted = no star is drawn. */
    onToggleFavorite?: (id: string) => void;
    /** Called with the full next selection on any selection gesture. */
    onSelectionChange: (ids: string[]) => void;
    /** Double-click / keyboard activation of one row. */
    onOpen?: (id: string) => void;
    /** Fired when the viewport scrolls near the last row (load-more). */
    onNearEnd?: () => void;
    /** Ordered multi-pick mode: a plain click APPENDS/toggles instead of replacing the selection. */
    appendOnClick?: boolean;
    /** The drop targets a row drag can land on. Omitted = rows are not draggable. */
    getDropTargets?: () => DragTarget[];
    /** Called with the dragged asset ids (the selection when the dragged row is selected, else
     * just that row) and the target id the pointer was released over. */
    onDropAssets?: (ids: string[], targetId: string) => void;
    /** Reports the live drop resolution while a row is dragged, `null` when the drag ends. */
    onDragHover?: (result: DropResult | null) => void;
  } = $props();

  const { t } = getAppContext();

  const drag = createDragController({
    axis: "vertical",
    getTargets: () => getDropTargets?.() ?? [],
    overIsContainer: () => true,
    onHover: (result) => onDragHover?.(result),
    onDrop: (result) => {
      if (!result.over) return;
      onDropAssets?.(selected.includes(result.dragged) ? selected : [result.dragged], result.over);
    },
  });

  let container = $state<HTMLDivElement | null>(null);
  let scrollTop = $state(0);
  let clientHeight = $state(0);
  let scrollHeight = $state(0);

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
  }

  $effect(() => {
    syncScrollState();
    if (!container || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => syncScrollState());
    ro.observe(container);
    return () => ro.disconnect();
  });

  const windowed = $derived.by(() =>
    computeListWindow(scrollTop, clientHeight, scrollHeight, items.length, ROW_HEIGHT),
  );
  const windowedItems = $derived.by(() => items.slice(windowed.start, windowed.end));
  const topSpacer = $derived(windowed.start * ROW_HEIGHT);
  const bottomSpacer = $derived(Math.max(0, items.length - windowed.end) * ROW_HEIGHT);

  // Shift-range anchor: the last plainly-clicked index.
  let anchor = $state<number | null>(null);

  /** Applies the click gesture's selection semantics for the row at `idx`.
   * @param idx - The clicked row's index into `items`.
   * @param ev - The click event carrying the modifier keys.
   * @example
   * ```
   * // private function; wired to each row's onclick below
   * declare const ev: MouseEvent;
   * select(0, ev);
   * ```
   */
  function select(idx: number, ev: MouseEvent): void {
    const next = applySelectionGesture(
      items.map((a) => a.id),
      idx,
      ev,
      anchor,
      selected,
      appendOnClick,
    );
    anchor = next.anchor;
    onSelectionChange(next.selection);
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
    if (onNearEnd && scrollHeight > 0 && scrollTop + clientHeight >= scrollHeight - 2 * ROW_HEIGHT) {
      onNearEnd();
    }
  }

  /** The kind glyph for an asset, from its stored content type.
   * @param a - The asset.
   * @returns An icon character for image, audio or other files.
   * @example
   * ```
   * // private function; used by each row below
   * declare const a: Asset;
   * kindIcon(a);
   * ```
   */
  function kindIcon(a: Asset): string {
    if (a.content_type.startsWith("image/")) return "🖼️";
    if (a.content_type.startsWith("audio/")) return "🔊";
    return "📄";
  }
</script>

<div
  class="list-viewport"
  data-testid="asset-browser-list"
  bind:this={container}
  onscroll={onScroll}
>
  {#if items.length === 0}
    <p class="empty" data-testid="asset-browser-empty">{t("assetBrowser.empty")}</p>
  {:else}
    <div style:height="{topSpacer}px"></div>
    {#each windowedItems as a, i (a.id)}
      <div class="entry" style:height="{ROW_HEIGHT}px">
        <button
          type="button"
          class="row"
          class:selected={selected.includes(a.id)}
          data-testid="asset-row"
          aria-pressed={selected.includes(a.id)}
          title={a.original_name}
          onpointerdown={(e) => {
            if (getDropTargets) drag.beginDrag(a.id, e);
          }}
          onclick={(e) => select(windowed.start + i, e)}
          ondblclick={() => onOpen?.(a.id)}
        >
          {#if appendOnClick && selected.includes(a.id)}
            <span class="order-badge">{selected.indexOf(a.id) + 1}</span>
          {/if}
          <span class="kind" aria-hidden="true">{kindIcon(a)}</span>
          <span class="name">{a.original_name}</span>
          <span class="size">{fmtBytes(a.byte_size)}</span>
          <span class="tags">{[...a.tags, ...a.derived_tags].join(", ")}</span>
        </button>
        {#if onToggleFavorite}
          <button
            type="button"
            class="star"
            class:on={favorites.includes(a.id)}
            data-testid="asset-favorite"
            aria-pressed={favorites.includes(a.id)}
            aria-label={favorites.includes(a.id) ? t("assetBrowser.unfavorite") : t("assetBrowser.favorite")}
            onclick={() => onToggleFavorite(a.id)}
          >{favorites.includes(a.id) ? "★" : "☆"}</button>
        {/if}
      </div>
    {/each}
    <div style:height="{bottomSpacer}px"></div>
  {/if}
</div>

<style lang="scss">
  .list-viewport {
    overflow-y: auto;
    min-height: 0;
    flex: 1;
  }
  .entry {
    display: flex;
    align-items: stretch;
    box-sizing: border-box;
    border-bottom: 1px solid var(--border);
  }
  .row {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0 0.5rem;
    border: none;
    background: var(--surface-raised);
    color: var(--text);
    text-align: left;
    cursor: pointer;
    user-select: none;
    -webkit-touch-callout: none;
    &.selected {
      outline: 2px solid var(--accent);
      outline-offset: -2px;
    }
    .name {
      flex: 1;
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .size {
      color: var(--text-muted);
      white-space: nowrap;
    }
    .tags {
      flex: 1;
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      color: var(--text-muted);
    }
    .order-badge {
      min-width: 1.1rem;
      border-radius: 0.55rem;
      background: var(--accent);
      color: var(--on-accent);
      font-size: 0.7rem;
      text-align: center;
    }
  }
  .star {
    min-width: 44px;
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-size: 1.1rem;
    cursor: pointer;
    &.on {
      color: var(--accent);
    }
  }
  .empty {
    color: var(--text-muted);
    padding: 1rem;
  }
</style>
