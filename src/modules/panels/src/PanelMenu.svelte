<script lang="ts">
  import { t } from "@shadowcat/ui-kit";
  import type { MenuCommand } from "./engine/policy";

  /** The per-tab/floating-header command menu. Framework-only: no dockview
   * import here — `dockview.ts` mounts this component imperatively via
   * Svelte's `mount()` and translates its `onCommand` callback into a
   * `LayoutOp` via `opForMenuCommand` on its own side, so this component
   * never needs to know an engine exists at all. */
  let {
    onCommand,
    onClose,
  }: {
    onCommand: (cmd: MenuCommand) => void;
    onClose: () => void;
  } = $props();

  const items: { cmd: MenuCommand; labelKey: string }[] = [
    { cmd: "dockRight", labelKey: "panels.dockRight" },
    { cmd: "dockBottom", labelKey: "panels.dockBottom" },
    { cmd: "dockLeft", labelKey: "panels.dockLeft" },
    { cmd: "float", labelKey: "panels.float" },
    { cmd: "minimize", labelKey: "panels.minimize" },
    { cmd: "close", labelKey: "panels.close" },
  ];

  let itemEls: HTMLButtonElement[] = [];

  /** Menu items are a flat list per the WAI-ARIA Menu pattern: arrow keys
   * move DOM focus directly — no separate "activate" step, since a
   * menuitem's own activation IS selecting it via Enter/Space/click. */
  function focusItem(index: number): void {
    const n = itemEls.length;
    itemEls[((index % n) + n) % n]?.focus();
  }

  function onKeydown(event: KeyboardEvent, index: number): void {
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
        // Closes the MENU POPUP only — distinct from a floating PANEL's own
        // Escape-to-close (wired in `dockview.ts` against the floating
        // dialog element, unrelated to this popup).
        event.preventDefault();
        event.stopPropagation();
        onClose();
        break;
      case "Tab":
        // A menu is a closed focus loop while open (WAI-ARIA Menu pattern) —
        // Tab closes it rather than escaping into the surrounding page.
        event.preventDefault();
        onClose();
        break;
    }
  }
</script>

<div class="sc-panel-menu" role="menu">
  {#each items as item, i (item.cmd)}
    <button
      type="button"
      role="menuitem"
      class="sc-panel-menu-item"
      data-testid="panel-menu-{item.cmd}"
      bind:this={itemEls[i]}
      onclick={() => onCommand(item.cmd)}
      onkeydown={(event) => onKeydown(event, i)}
    >{t(item.labelKey)}</button>
  {/each}
</div>

<style lang="scss">
  .sc-panel-menu {
    display: flex;
    flex-direction: column;
    min-width: 9rem;
    padding: 0.25rem;
    border: 1px solid var(--border);
    border-radius: 0.375rem;
    background: var(--surface-overlay);
    box-shadow: var(--shadow-elevated);
  }
  .sc-panel-menu-item {
    /* Touch target floor (mobile invariant); comfortably above the 24px a11y floor too. */
    min-height: 36px;
    padding: 0 0.75rem;
    border: none;
    border-radius: 0.25rem;
    background: transparent;
    color: var(--text-primary);
    font-size: 0.85rem;
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
</style>
