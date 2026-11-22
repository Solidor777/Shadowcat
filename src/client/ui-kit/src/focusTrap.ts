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
 * Keeps Tab and Shift+Tab inside `root`: Tab on the last focusable element moves focus to the
 * first, Shift+Tab on the first moves it to the last. With no focusable descendant, Tab is
 * cancelled so focus cannot leave the container. Tab presses between interior elements are left
 * to native traversal.
 * @param root - The container that traps focus (a dialog element).
 * @returns A disposer that removes the trap.
 * @example
 * const release = trapFocus(dialogEl);
 * release();
 */
export function trapFocus(root: HTMLElement): () => void {
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
  return () => root.removeEventListener("keydown", onKeydown);
}
