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

  /**
   * Opens the menu and resets `itemEls` so stale refs from a previous open
   * don't leak into the freshly-rendered list. Focus moves to the first item
   * only after Svelte finishes binding it (`queueMicrotask`, not
   * synchronously) — the item elements don't exist yet during this call.
   * @returns Nothing; opens the menu and schedules a focus move as side
   *   effects.
   * @example
   * ```
   * // private function; not part of the public API — wired to the trigger's onclick/onkeydown
   * openMenu();
   * ```
   */
  function openMenu(): void {
    itemEls = [];
    open = true;
    queueMicrotask(() => itemEls[0]?.focus());
  }
  /**
   * Closes the menu. `returnFocus` defaults to `true` (the WAI-ARIA APG Menu
   * Button pattern's expected dismiss behavior) — used by `activate()` after
   * selecting an item, and by the Escape branch below. Callers pass `false`
   * when focus either never left the trigger (the toggle-close branch below,
   * fired from the trigger's OWN keydown handler) or is moving elsewhere on
   * purpose (the backdrop's outside-pointer dismissal, which would otherwise
   * fight the click/tap that just happened outside the menu).
   * @param returnFocus Whether to move focus back to the trigger button.
   * @returns Nothing; closes the menu and optionally schedules a focus move.
   * @example
   * ```
   * // private function; not part of the public API — wired to Escape, outside-pointer
   * // dismissal, item activation, and the trigger's own open/close toggle
   * closeMenu();
   * ```
   */
  function closeMenu(returnFocus = true): void {
    open = false;
    if (returnFocus) queueMicrotask(() => triggerEl?.focus());
  }
  /**
   * Selects menu item `id`: toggles that panel via `AppContext.panels`
   * (`ctx.panels.toggle`) — the only way this module reaches the panel
   * manager; `topbar`'s own `package.json` declares no dependency on
   * `@shadowcat/module-panels` — then closes the menu, returning focus to
   * the trigger.
   * @param id The panel id to toggle (a `shadowcat.panel` contribution id).
   * @returns Nothing; toggles the panel and closes the menu as side effects.
   * @example
   * ```
   * // private function; not part of the public API — wired to each menu item's onclick
   * activate(panelId);
   * ```
   */
  function activate(id: string): void {
    ctx.panels.toggle(id);
    closeMenu();
  }
  const menuKeyboard = createMenuKeyboard(() => itemEls, closeMenu);
  /**
   * Delegates a menu item's keydown to `@shadowcat/ui-kit`'s
   * `createMenuKeyboard` — the arrow-key/Home/End navigation contract is
   * documented there, not re-described here, so this copy can't drift from
   * it.
   * @param event The keydown event from menu item `index`.
   * @param index The item's position in `itemEls`/`panels`.
   * @returns Nothing; may move focus among `itemEls` as a side effect.
   * @example
   * ```
   * // private function; not part of the public API — wired to each menu item's onkeydown
   * onItemKeydown(keyboardEvent, 0);
   * ```
   */
  function onItemKeydown(event: KeyboardEvent, index: number): void {
    menuKeyboard.handleKeydown(event, index);
  }
  /**
   * Handles the trigger button's own keydown. Escape closes an open menu.
   * ArrowDown/Enter/Space open a closed menu, or — on an ALREADY-open
   * trigger — close it rather than re-open it: a true toggle, so a menu with
   * no keyboard-reachable items (e.g. zero registered panels) can never trap
   * focus by re-opening itself on every keypress.
   * @param event The trigger button's keydown event.
   * @returns Nothing; opens/closes the menu as a side effect.
   * @example
   * ```
   * // private function; not part of the public API — wired to the trigger's onkeydown
   * onTriggerKeydown(keyboardEvent);
   * ```
   */
  function onTriggerKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape" && open) {
      event.preventDefault();
      closeMenu();
      return;
    }
    if (event.key === "ArrowDown" || event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      // Toggle, not open-only — see this function's JSDoc for why.
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
