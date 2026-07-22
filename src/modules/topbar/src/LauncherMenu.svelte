<script lang="ts">
  import { getAppContext, sizeClass, createMenuKeyboard } from "@shadowcat/ui-kit";

  const ctx = getAppContext();
  const t = ctx.t;
  const compact = $derived(sizeClass() === "compact");
  // Stable per-instance id (WAI-ARIA APG Menu Button pattern: the trigger's
  // `aria-controls` references the popup menu it owns).
  const menuId = $props.id();

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
  const menuKeyboard = createMenuKeyboard(() => itemEls, closeMenu);
  function onItemKeydown(event: KeyboardEvent, index: number): void {
    menuKeyboard.handleKeydown(event, index);
  }
  function onTriggerKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape" && open) {
      event.preventDefault();
      closeMenu();
      return;
    }
    if (event.key === "ArrowDown" || event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      // Enter/Space on an already-open trigger is a true toggle — closes
      // rather than re-opening, so an empty menu (no keyboard-reachable
      // items) is never a focus trap.
      if (open) closeMenu(false);
      else openMenu();
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
    aria-controls={open ? menuId : undefined}
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
      id={menuId}
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
    /* One layer below the menu itself so outside-pointer dismissal doesn't
       intercept clicks on the menu it belongs to. */
    z-index: calc(var(--z-popover) - 1);
  }
  .sc-launcher-menu {
    position: absolute;
    top: calc(100% + var(--space-1));
    left: 0;
    z-index: var(--z-popover);
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
