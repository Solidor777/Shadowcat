// Pointer-drag reorder state machine. DOM-light by design: geometry is read through an injected
// `rects()` callback (one DOMRect per visible row, in order) rather than touching the DOM
// directly, so jsdom tests can stub geometry without simulating a real pointer gesture.

/** The completed drag's result: move the row at `from` to `to`, in `moveInOrder`'s index
 * convention (`to` indexes the array AFTER `from` has been spliced out). */
export interface ReorderMove {
  /** The dragged row's starting index. */
  from: number;
  /** The destination index, in the post-removal array. */
  to: number;
}

/** One in-progress (or idle) drag's state machine.
 * @param rects Returns each visible row's current bounding rect, top-to-bottom in row order —
 * called fresh on every `move()` so a reflow between calls is picked up.
 * @returns The drag controller: `beginDrag`/`move`/`end`/`cancel`.
 * @example
 * ```ts
 * import { createReorder } from "./reorder";
 *
 * declare const rowRects: () => DOMRect[];
 * const reorder = createReorder(rowRects);
 * ```
 */
export function createReorder(rects: () => DOMRect[]): {
  /** Arms a drag starting at `index`.
   * @param index The dragged row's starting index.
   * @param ev The originating pointer event (unused by the geometry calculation; kept for a
   * future gesture refinement and API symmetry with `move`/`end`). */
  beginDrag: (index: number, ev: PointerEvent) => void;
  /** Updates the drag's current target index from the pointer's Y position.
   * @param ev The pointer event carrying the current `clientY`. */
  move: (ev: PointerEvent) => void;
  /** Ends the drag.
   * @param ev The originating pointer event (unused; kept for API symmetry).
   * @returns The move to apply, or `null` when no drag was in progress or the target never
   * moved off the start (a no-op drag). */
  end: (ev: PointerEvent) => ReorderMove | null;
  /** Aborts the in-progress drag without producing a move (e.g. on Escape). */
  cancel: () => void;
} {
  let from: number | null = null;
  let to: number | null = null;

  /** The target insertion index for `y`: the count of every OTHER row (excluding `from`) whose
   * midpoint sits above `y` — the index `moveInOrder`'s post-removal `to` convention expects, so
   * crossing exactly one row's midpoint moves the target by exactly one step.
   * @param y The pointer's current `clientY`.
   * @returns The target index.
   */
  function targetFor(y: number): number {
    const rs = rects();
    let count = 0;
    for (let i = 0; i < rs.length; i++) {
      if (i === from) continue;
      const mid = rs[i].top + rs[i].height / 2;
      if (mid < y) count++;
    }
    return count;
  }

  return {
    beginDrag(index) {
      from = index;
      to = index;
    },
    move(ev) {
      if (from === null) return;
      to = targetFor(ev.clientY);
    },
    end() {
      if (from === null || to === null) return null;
      const result = to !== from ? { from, to } : null;
      from = null;
      to = null;
      return result;
    },
    cancel() {
      from = null;
      to = null;
    },
  };
}
