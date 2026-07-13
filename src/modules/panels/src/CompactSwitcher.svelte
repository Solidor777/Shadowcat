<script lang="ts">
  import type { PanelMeta } from "@shadowcat/core";
  import { getAppContext, sizeClass } from "@shadowcat/ui-kit";

  /** Bottom switcher for the compact (narrow-viewport) presentation: a full-
   * screen active view plus a tab strip listing every registered panel in
   * registry order. Adopts slot elements the same way the engine does — every
   * `order` id is adopted into its content area (display toggled to the
   * active one) whenever this is the active presentation, and released back
   * on the next flip (the engine's own `apply` reclaims them on flip-back). */
  let {
    order,
    activeView,
    meta,
    slotFor,
    onSwitch,
  }: {
    order: string[];
    activeView: string | null;
    meta: ReadonlyMap<string, PanelMeta>;
    slotFor: (id: string) => HTMLElement;
    onSwitch: (id: string) => void;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  let contentEl: HTMLElement;

  $effect(() => {
    if (sizeClass() !== "compact") return;
    if (!contentEl) return;
    for (const id of order) {
      const slot = slotFor(id);
      if (slot.parentElement !== contentEl) contentEl.appendChild(slot);
      slot.style.display = id === activeView ? "" : "none";
    }
  });

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
