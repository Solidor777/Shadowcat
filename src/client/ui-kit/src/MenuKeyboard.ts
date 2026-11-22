import { escapeCoordinator, ESCAPE_PRIORITY } from "./escapeCoordinator";

/** Shared WAI-ARIA APG Menu Button keyboard behavior for a flat `role="menuitem"`
 * list: arrow keys move DOM focus directly (no separate "activate" step, since a
 * menuitem's own activation IS selecting it via Enter/Space/click). Framework- and
 * dockview-free by construction — pure keyboard-event logic only. */
export interface MenuKeyboard {
  /** Move DOM focus to the item at `index`, wrapping in both directions.
   * @param index - Target index; wrapped modulo the current item count. */
  focusItem(index: number): void;
  /** Handle one keydown on the item at `index` per the WAI-ARIA APG Menu Button pattern.
   * @param event - The native keydown event.
   * @param index - The index of the item that currently has focus. */
  handleKeydown(event: KeyboardEvent, index: number): void;
  /** Registers the menu's Escape-to-close (calls `onClose()` with focus returned) with the shared
   * escape coordinator. Call when the menu opens; a second call while active is a no-op. */
  activate(): void;
  /** Removes the Escape registration. Call on every path that closes the menu; idempotent. */
  deactivate(): void;
}

/**
 * Build a {@link MenuKeyboard} bound to one flat item list. `getItemEls` is called
 * fresh on every keypress (not cached), so the caller may reorder/filter its menu
 * items between events without re-creating the returned object.
 * @param getItemEls - Returns the current focusable `role="menuitem"` elements, in order.
 * @param onClose - Invoked to close the owning menu; `returnFocus` (default `true`)
 * tells the caller whether to return focus to the menu's trigger.
 * @returns The `{ focusItem, handleKeydown, activate, deactivate }` handle: wire `handleKeydown`
 * onto the menu's keydown handler and bracket the menu's open lifetime with `activate`/`deactivate`.
 * @example
 * const menu = createMenuKeyboard(() => itemEls, (returnFocus) => close(returnFocus));
 */
export function createMenuKeyboard(
  getItemEls: () => HTMLElement[],
  onClose: (returnFocus?: boolean) => void,
): MenuKeyboard {
  /**
   * Move DOM focus to the item at `index`, wrapping in both directions. A no-op
   * when the item list is currently empty.
   * @param index - Target index; wrapped modulo the current item count.
   * @example focusItem(-1); // focuses the last item
   */
  function focusItem(index: number): void {
    const items = getItemEls();
    const n = items.length;
    if (n === 0) return;
    items[((index % n) + n) % n]?.focus();
  }

  /**
   * Handle one keydown on the menu's item at `index` per the WAI-ARIA APG Menu
   * Button pattern: Arrow/Home/End move focus, Tab closes without returning focus and lets
   * native Tab traversal proceed. Escape is not handled here: it reaches the menu through
   * `activate`'s coordinator registration.
   * Unhandled keys pass through untouched.
   * @param event - The native keydown event; preventDefault/stopPropagation are
   * called only for the keys this function handles.
   * @param index - The index of the item that currently has focus.
   * @example handleKeydown(event, currentIndex);
   */
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
      case "Tab":
        // WAI-ARIA APG Menu Button pattern: Tab closes the menu and lets focus
        // proceed natively to the next tabbable element — it does NOT bounce
        // focus back to the trigger/invoker (that is Escape's job) or suppress
        // the native Tab traversal.
        onClose(false);
        break;
    }
  }

  /** Disposer of the live Escape registration; `undefined` while the menu is inactive. */
  let disposeEscape: (() => void) | undefined;

  /**
   * Registers the menu's Escape-to-close with `escapeCoordinator` at menu priority. Calling it
   * while already active replaces nothing: the existing registration stays.
   * @example activate();
   */
  function activate(): void {
    if (disposeEscape !== undefined) return;
    disposeEscape = escapeCoordinator.register(ESCAPE_PRIORITY.MENU, () => onClose());
  }

  /**
   * Removes the Escape registration made by `activate`; a no-op when none is live.
   * @example deactivate();
   */
  function deactivate(): void {
    disposeEscape?.();
    disposeEscape = undefined;
  }

  return { focusItem, handleKeydown, activate, deactivate };
}
