<script lang="ts">
  import type { PanelMeta } from "@shadowcat/core";
  import { getAppContext, sizeClass } from "@shadowcat/ui-kit";

  /** Bottom switcher for the compact (narrow-viewport) presentation: a full-
   * screen active view plus a tab strip listing every registered panel in
   * registry order. Adopts ONLY `activeView`'s slot into its content area;
   * every other `order` id stays in PanelHost's staging container. `release`
   * returns the currently adopted slot to staging whenever the active id
   * changes, this leaves compact presentation, or the component unmounts —
   * a launcher-only panel (absent from `expanded`) has no other reclaim
   * path, so ownership of releasing an adopted slot lives here. */
  let {
    order,
    activeView,
    meta,
    slotFor,
    release,
    onSwitch,
  }: {
    /** Panel ids in display order for the tab strip; see `CompactLayout.order`. */
    order: string[];
    /** The currently-shown panel id; see `CompactLayout.activeView`. */
    activeView: string | null;
    /** Per-panel display metadata (title, icon) keyed by panel id. */
    meta: ReadonlyMap<string, PanelMeta>;
    /** Resolves a panel id to its persistent, already-mounted slot element. */
    slotFor: (id: string) => HTMLElement;
    /** Returns an adopted slot element to PanelHost's staging container. */
    release: (el: HTMLElement) => void;
    /** Called when the user picks a different tab, with the newly-selected panel id. */
    onSwitch: (id: string) => void;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  let contentEl: HTMLElement;
  let adoptedId: string | null = null;
  let adoptedEl: HTMLElement | null = null;

  /** Returns the currently-adopted slot (if any) to `release`, and clears the
   * adopted-id/-element bookkeeping. A no-op when nothing is adopted.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the
   * // adoption effect below and this component's own unmount cleanup
   * releaseAdopted();
   * ```
   */
  function releaseAdopted(): void {
    if (adoptedEl) release(adoptedEl);
    adoptedEl = null;
    adoptedId = null;
  }

  $effect(() => {
    if (sizeClass() !== "compact" || !contentEl) {
      releaseAdopted();
      return;
    }
    if (activeView === adoptedId) return;
    releaseAdopted();
    if (activeView !== null) {
      const slot = slotFor(activeView);
      if (slot.parentElement !== contentEl) contentEl.appendChild(slot);
      slot.style.display = "";
      adoptedEl = slot;
      adoptedId = activeView;
    }
  });

  // No reactive dependencies read: runs once at mount, cleanup fires only on unmount.
  $effect(() => {
    return () => releaseAdopted();
  });

  /** Resolves a switcher tab's accessible/tooltip label.
   * @param id The panel id to label.
   * @returns The panel's translated `labelKey`, or `id` itself if it has no
   * registered `PanelMeta`.
   * @example
   * ```
   * // private function; not part of the public API — used only in this
   * // component's own switcher-tab template below
   * label("chat");
   * ```
   */
  function label(id: string): string {
    const m = meta.get(id);
    return m ? t(m.labelKey) : id;
  }
</script>

<div class="compact-switcher" hidden={sizeClass() !== "compact"}>
  <div class="active-view" bind:this={contentEl}></div>
  <div class="switcher-bar" role="tablist">
    {#each order as id (id)}
      {@const m = meta.get(id)}
      <button
        type="button"
        class="switcher-btn"
        role="tab"
        aria-selected={id === activeView}
        aria-label={label(id)}
        title={label(id)}
        data-testid="compact-switch-{id}"
        onclick={() => onSwitch(id)}
      >{m?.icon ?? id.slice(0, 1)}</button>
    {/each}
  </div>
</div>

<style lang="scss">
  .compact-switcher {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    // A plain `[hidden]` selector (the UA default `[hidden]{display:none}`)
    // has the same specificity as this scoped class selector; being the
    // scoped rule declared LATER in the cascade, `display: flex` above wins
    // over `[hidden]`'s `display: none` without this override — an expanded-
    // mode `.compact-switcher` would otherwise still lay out at full height,
    // stealing flex space and pointer events from `.engine-host`'s sibling
    // content (e.g. the stage canvas).
    &[hidden] {
      display: none;
    }
  }
  .active-view {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .switcher-bar {
    display: flex;
    flex-direction: row;
    gap: 0.25rem;
    padding: 0.25rem;
    border-top: 1px solid var(--border);
    background: var(--surface-overlay);
  }
  .switcher-btn {
    /* Touch target floor (mobile invariant). */
    min-width: 44px;
    min-height: 44px;
    border: none;
    border-radius: 0.375rem;
    background: transparent;
    color: var(--text-primary);
    font-size: 1.25rem;
    cursor: pointer;
  }
  .switcher-btn[aria-selected="true"] {
    background: var(--surface-base);
  }
</style>
