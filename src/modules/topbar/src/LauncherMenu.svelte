<script lang="ts">
  import { getAppContext, sizeClass } from "@shadowcat/ui-kit";

  const ctx = getAppContext();
  const t = ctx.t;
  const compact = $derived(sizeClass() === "compact");

  // Registered panels in metaMap order — already gmOnly-filtered by the bound
  // PanelsController (the host is the one place role filtering happens). `$state`-
  // backed on the bridge, so this unfreezes once the panel host binds and tracks
  // module install/uninstall.
  const panels = $derived([...ctx.panels.metaMap.entries()].map(([id, meta]) => ({ id, meta })));

  let open = $state(false);
  let triggerEl: HTMLButtonElement;
  let itemEls: HTMLButtonElement[] = $state([]);

  function openMenu(): void {
    itemEls = [];
    open = true;
    // Focus the first item after Svelte binds the freshly-rendered menu.
    queueMicrotask(() => itemEls[0]?.focus());
  }
  function closeMenu(returnFocus = true): void {
    open = false;
    if (returnFocus) queueMicrotask(() => triggerEl?.focus());
  }
  function activate(id: string): void {
    ctx.panels.toggle(id);
    closeMenu();
  }
  function focusItem(index: number): void {
    const n = itemEls.length;
    if (n === 0) return;
    itemEls[((index % n) + n) % n]?.focus();
  }
  function onItemKeydown(event: KeyboardEvent, index: number): void {
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        focusItem(index + 1);
        break;
      case "ArrowUp":
        event.preventDefault();
        focusItem(index - 1);
        break;
      case "Home":
        event.preventDefault();
        focusItem(0);
        break;
      case "End":
        event.preventDefault();
        focusItem(itemEls.length - 1);
        break;
      case "Escape":
        event.preventDefault();
        event.stopPropagation();
        closeMenu();
        break;
      case "Tab":
        // A menu is a closed focus loop while open (WAI-ARIA Menu pattern).
        event.preventDefault();
        closeMenu();
        break;
    }
  }
  function onTriggerKeydown(event: KeyboardEvent): void {
    if (event.key === "ArrowDown" || event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      openMenu();
    }
  }
</script>

<div class="sc-launcher" class:compact>
  <button
    type="button"
    class="sc-launcher-trigger"
    bind:this={triggerEl}
    aria-haspopup="menu"
    aria-expanded={open}
    aria-label={t("topbar.launcher")}
    data-testid="launcher-trigger"
    onclick={() => (open ? closeMenu() : openMenu())}
    onkeydown={onTriggerKeydown}
  >
    <span class="sc-launcher-glyph" aria-hidden="true">☰</span>
    <span class="sc-launcher-label">{t("topbar.launcher")}</span>
  </button>

  {#if open}
    <!-- Outside-pointer dismissal; the menu itself is above this backdrop. -->
    <div
      class="sc-launcher-backdrop"
      aria-hidden="true"
      onpointerdown={() => closeMenu(false)}
    ></div>
    <div
      class="sc-launcher-menu"
      role="menu"
      aria-label={t("topbar.launcher")}
      data-testid="launcher-menu"
    >
      {#each panels as p, i (p.id)}
        <button
          type="button"
          role="menuitem"
          class="sc-launcher-item"
          data-testid="launcher-item-{p.id}"
          bind:this={itemEls[i]}
          onclick={() => activate(p.id)}
          onkeydown={(e) => onItemKeydown(e, i)}
        >
          <span class="sc-launcher-icon" aria-hidden="true">{p.meta.icon}</span>
          <span>{t(p.meta.labelKey)}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style lang="scss">
  .sc-launcher {
    position: relative;
    display: flex;
    align-items: center;
  }
  .sc-launcher-trigger {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 44px; /* touch target (mobile invariant); >=24px a11y floor */
    padding: 0 var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
    &:focus-visible {
      outline: 2px solid var(--accent);
      outline-offset: 2px;
    }
  }
  .sc-launcher.compact .sc-launcher-label {
    /* Compact: icon-only trigger to reclaim topbar width — the single axis. */
    display: none;
  }
  .sc-launcher-backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
  }
  .sc-launcher-menu {
    position: absolute;
    top: calc(100% + var(--space-1));
    left: 0;
    z-index: 41;
    display: flex;
    flex-direction: column;
    min-width: 12rem;
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-2);
    background: var(--surface-overlay);
    box-shadow: var(--shadow-elevated);
  }
  .sc-launcher-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 36px; /* comfortably above the 24px a11y floor */
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-1);
    background: transparent;
    color: var(--text-primary);
    font-size: 0.9rem;
    text-align: left;
    cursor: pointer;
    &:hover {
      background: var(--surface-base);
    }
    &:focus-visible {
      outline: 2px solid var(--accent);
      outline-offset: -2px;
    }
  }
  .sc-launcher-icon {
    width: 1.25rem;
    text-align: center;
  }
</style>
