<script lang="ts">
  import { createMenuKeyboard } from "./MenuKeyboard";
  import { t } from "./i18n.svelte";
  import type { ContextMenuItem } from "./contextMenu";

  let {
    items,
    x,
    y,
    onClose,
  }: {
    /** The entries, in display order. */
    items: ContextMenuItem[];
    /** Viewport x of the opening gesture; the menu's left edge, shifted left to stay on screen. */
    x: number;
    /** Viewport y of the opening gesture; the menu's top edge, shifted up to stay on screen. */
    y: number;
    /** Called once when the menu should leave the page (item chosen, Escape, outside press, Tab). */
    onClose: () => void;
  } = $props();

  /** Gap kept between the menu and the viewport edge when it is shifted to fit. */
  const EDGE_PX = 4;

  let menuEl = $state<HTMLDivElement | null>(null);
  let itemEls: HTMLButtonElement[] = $state([]);
  /** The element focused when the menu opened; focus returns there on a keyboard dismissal. */
  let opener: Element | null = null;
  let left = $state(0);
  let top = $state(0);

  /**
   * Closes the menu, optionally returning focus to the element that held it on open.
   * @param returnFocus - Whether to restore focus; `false` when focus is moving on purpose.
   * @example
   * ```
   * // private function; wired to Escape, Tab, outside press and item choice
   * close(true);
   * ```
   */
  function close(returnFocus = true): void {
    menuKeyboard.deactivate();
    onClose();
    if (returnFocus && opener instanceof HTMLElement) opener.focus();
  }

  /**
   * The focusable items in order: a disabled button cannot take focus, so arrow navigation skips it.
   * @returns The enabled item buttons.
   * @example
   * ```
   * // private function; read by the keyboard handler on every keypress
   * enabledEls();
   * ```
   */
  function enabledEls(): HTMLButtonElement[] {
    return itemEls.filter((el) => el && !el.disabled);
  }

  const menuKeyboard = createMenuKeyboard(enabledEls, close);

  /**
   * Runs one item's action after closing the menu.
   * @param item - The chosen item.
   * @example
   * ```
   * // private function; wired to each item's onclick
   * declare const item: ContextMenuItem;
   * choose(item);
   * ```
   */
  function choose(item: ContextMenuItem): void {
    if (item.disabled) return;
    close();
    item.action();
  }

  // Lifetime: remember the opener, register Escape, focus the first enabled item. A change of
  // `items` identity does not re-run this; the menu is rebuilt per open.
  $effect(() => {
    opener = document.activeElement;
    menuKeyboard.activate();
    itemEls.find((el) => !el.disabled)?.focus();
    return () => menuKeyboard.deactivate();
  });

  // Shift the menu so its box stays inside the viewport once its size is known.
  $effect(() => {
    const rect = menuEl?.getBoundingClientRect();
    const width = rect?.width ?? 0;
    const height = rect?.height ?? 0;
    left = Math.max(EDGE_PX, Math.min(x, window.innerWidth - width - EDGE_PX));
    top = Math.max(EDGE_PX, Math.min(y, window.innerHeight - height - EDGE_PX));
  });
</script>

<!-- Outside-press dismissal; the menu itself sits one layer above this backdrop. -->
<div
  class="sc-context-backdrop"
  role="presentation"
  data-testid="context-menu-backdrop"
  onpointerdown={() => close(false)}
  oncontextmenu={(e) => {
    e.preventDefault();
    close(false);
  }}
></div>
<div
  class="sc-context-menu"
  role="menu"
  aria-label={t("contextMenu.label")}
  data-testid="context-menu"
  bind:this={menuEl}
  style:left="{left}px"
  style:top="{top}px"
>
  {#each items as item, i (item.id)}
    <button
      type="button"
      role="menuitem"
      class="sc-context-item"
      data-testid={"context-menu-item-" + item.id}
      disabled={item.disabled}
      bind:this={itemEls[i]}
      onclick={() => choose(item)}
      onkeydown={(e) => menuKeyboard.handleKeydown(e, enabledEls().indexOf(itemEls[i] as HTMLButtonElement))}
    >{t(item.labelKey)}</button>
  {/each}
</div>

<style lang="scss">
  .sc-context-backdrop {
    position: fixed;
    inset: 0;
    z-index: calc(var(--z-popover) - 1);
  }
  .sc-context-menu {
    position: fixed;
    z-index: var(--z-popover);
    display: flex;
    flex-direction: column;
    min-width: 10rem;
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-2);
    background: var(--surface-overlay);
    box-shadow: var(--shadow-elevated);
  }
  /* Touch-target floor (mobile invariant). */
  .sc-context-item {
    min-height: 44px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-1);
    background: transparent;
    color: var(--text-primary);
    text-align: left;
    cursor: pointer;
    &:hover:not(:disabled) {
      background: var(--surface-base);
    }
    &:disabled {
      opacity: 0.5;
      cursor: default;
    }
    &:focus-visible {
      outline: 2px solid var(--accent);
      outline-offset: -2px;
    }
  }
</style>
