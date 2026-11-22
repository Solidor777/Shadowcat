// The click-selection semantics shared by the thumbnail grid and the list view, so both views
// resolve a gesture identically.

/** The outcome of one click gesture. */
export interface SelectionOutcome {
  /** The full next selection, in selection order. */
  selection: string[];
  /** The next shift-range anchor index. */
  anchor: number;
}

/**
 * Applies a click gesture's selection semantics for the item at `idx`: plain = only that item,
 * ctrl/meta (or `append`) = toggle, shift = the contiguous range from the anchor.
 * @param ids - The displayed item ids, in display order.
 * @param idx - The clicked item's index into `ids`.
 * @param ev - The click's modifier keys.
 * @param anchor - The last plainly-clicked index, or `null`.
 * @param selected - The current selection.
 * @param append - Ordered multi-pick mode: a plain click toggles instead of replacing.
 * @returns The next selection and anchor.
 * @example
 * ```ts
 * import { applySelectionGesture } from "./selectionGesture";
 *
 * applySelectionGesture(["a", "b"], 1, { shiftKey: false, ctrlKey: false, metaKey: false }, null, [], false);
 * // { selection: ["b"], anchor: 1 }
 * ```
 */
export function applySelectionGesture(
  ids: string[],
  idx: number,
  ev: Pick<MouseEvent, "shiftKey" | "ctrlKey" | "metaKey">,
  anchor: number | null,
  selected: string[],
  append: boolean,
): SelectionOutcome {
  const id = ids[idx];
  if (ev.shiftKey && anchor !== null) {
    const [lo, hi] = anchor <= idx ? [anchor, idx] : [idx, anchor];
    return { selection: ids.slice(lo, hi + 1), anchor };
  }
  if (ev.ctrlKey || ev.metaKey || append) {
    return {
      selection: selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id],
      anchor: idx,
    };
  }
  return { selection: [id], anchor: idx };
}
