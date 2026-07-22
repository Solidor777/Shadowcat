/** Shared WAI-ARIA APG Menu Button keyboard behavior for a flat `role="menuitem"`
 * list: arrow keys move DOM focus directly (no separate "activate" step, since a
 * menuitem's own activation IS selecting it via Enter/Space/click). Framework- and
 * dockview-free by construction — pure keyboard-event logic only. */
export interface MenuKeyboard {
  focusItem(index: number): void;
  handleKeydown(event: KeyboardEvent, index: number): void;
}

export function createMenuKeyboard(
  getItemEls: () => HTMLElement[],
  onClose: (returnFocus?: boolean) => void,
): MenuKeyboard {
  function focusItem(index: number): void {
    const items = getItemEls();
    const n = items.length;
    if (n === 0) return;
    items[((index % n) + n) % n]?.focus();
  }

  function handleKeydown(event: KeyboardEvent, index: number): void {
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
        focusItem(getItemEls().length - 1);
        break;
      case "Escape":
        // Closes the owning menu popup only — the caller decides what "close"
        // means (e.g. a floating panel's own Escape-to-close is separate).
        event.preventDefault();
        event.stopPropagation();
        onClose();
        break;
      case "Tab":
        // WAI-ARIA APG Menu Button pattern: Tab closes the menu and lets focus
        // proceed natively to the next tabbable element — it does NOT bounce
        // focus back to the trigger/invoker (that is Escape's job) or suppress
        // the native Tab traversal.
        onClose(false);
        break;
    }
  }

  return { focusItem, handleKeydown };
}
