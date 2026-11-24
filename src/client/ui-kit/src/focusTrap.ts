/** CSS selector matching the elements a Tab press can land on. Disabled controls and
 * negative-tabindex elements are excluded; hidden ones are filtered by layout in `focusables`. */
const FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled]):not([type='hidden'])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "[tabindex]:not([tabindex^='-'])",
].join(",");

/**
 * Lists the focusable descendants of `root` in DOM order. Queried on every call, never cached,
 * so a control that became disabled or enabled since the last keypress is picked up.
 * @param root - The container to search.
 * @returns The focusable elements, in document order.
 * @example
 * const items = focusables(dialogEl);
 */
function focusables(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));
}

/**
 * Records the element focused now and returns a function that gives it focus back. The restore
 * runs only when focus is still inside `root` or has fallen to the page body (a dialog that
 * unmounted took focus with it), and only when the recorded element is still in the document; a
 * deliberate focus move elsewhere is left alone. Call it before the dialog moves focus inward.
 * @param root - The overlay whose lifetime brackets the remembered focus.
 * @returns A function that restores focus to the recorded element.
 * @example
 * const restore = rememberFocus(dialogEl);
 * restore();
 */
export function rememberFocus(root: HTMLElement): () => void {
  const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  return () => {
    if (opener === null || !opener.isConnected || root.contains(opener)) return;
    const active = document.activeElement;
    if (active === null || active === document.body || root.contains(active)) opener.focus();
  };
}

/**
 * Keeps Tab and Shift+Tab inside `root`: Tab on the last focusable element moves focus to the
 * first, Shift+Tab on the first moves it to the last. With no focusable descendant, Tab is
 * cancelled so focus cannot leave the container. Tab presses between interior elements are left
 * to native traversal. Releasing the trap returns focus to the element that held it when the
 * trap was installed (see `rememberFocus`), so a dialog must install it before focusing inward.
 * @param root - The container that traps focus (a dialog element).
 * @returns A disposer that removes the trap and restores the opener's focus.
 * @example
 * const release = trapFocus(dialogEl);
 * release();
 */
export function trapFocus(root: HTMLElement): () => void {
  const restoreFocus = rememberFocus(root);
  /**
   * Wraps focus at the container's edges.
   * @param event - The keydown event on `root`.
   * @example
   * // private; installed as `root`'s keydown listener
   */
  function onKeydown(event: KeyboardEvent): void {
    if (event.key !== "Tab") return;
    const items = focusables(root);
    const first = items[0];
    const last = items[items.length - 1];
    if (first === undefined || last === undefined) {
      event.preventDefault();
      return;
    }
    const active = document.activeElement;
    if (event.shiftKey && (active === first || active === root)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  }
  root.addEventListener("keydown", onKeydown);
  return () => {
    root.removeEventListener("keydown", onKeydown);
    restoreFocus();
  };
}
