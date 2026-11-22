// Pointer-Events drag controller shared by every first-party drag-and-drop surface (sibling
// reorder lists, folder trees, asset tiles). Pointer Events fire uniformly for mouse, pen and
// touch; the native HTML5 drag protocol has no touch path on Android/iOS browsers. DOM-light by
// design: geometry is read through the injected `getTargets` callback (fresh on every query,
// never cached) so jsdom tests stub rects without a real gesture. Targets are keyed by id, not
// array index, because a tree or a cross-component drop has no single flat sibling array.
// Reuses the midpoint-crossing technique of a linear reorder list: the pointer's half of the
// nearest target's rect on the drag axis decides before/after.

/** The pointer-type value that arms only after a long press. */
const TOUCH_POINTER_TYPE = "touch";

/** How long a touch pointer must be held before a drag arms, so an ordinary tap or scroll is
 * never hijacked. A mouse or pen pointer arms immediately. */
export const DEFAULT_LONG_PRESS_MS = 350;

/** The fraction of a container target's extent, centred on the drag axis, where a drop counts
 * as "into" the container; the outer quarters on each side drop before/after it. */
const INTO_BAND = 0.25;

/** One element a drag can land on. */
export interface DragTarget {
  /** Opaque id the controller neither reads nor interprets beyond equality. */
  readonly id: string;
  /** The target's current bounding rect, read fresh on every query.
   * @returns The bounding rect in viewport coordinates. */
  rect(): DOMRect;
}

/** Where, relative to the over target, a drop settled. */
export type DropPlacement = "before" | "after" | "into";

/** The outcome of a completed drag. */
export interface DropResult {
  /** The id passed to `beginDrag`. */
  dragged: string;
  /** The target the pointer settled over at drop time, or `null` when it sat over no target
   * (only possible without `snapToNearest`); a caller cancels. */
  over: string | null;
  /** `"into"` only when `overIsContainer(over)` is true and the pointer sits in the middle half
   * of the target on the drag axis; otherwise `"before"`/`"after"` by which half of the target's
   * extent the pointer is in. */
  placement: DropPlacement;
}

/** Construction options for `createDragController`. */
export interface DragControllerOptions {
  /** The drop targets, queried fresh on every pointer event. The dragged id is excluded. */
  getTargets: () => DragTarget[];
  /** The axis `before`/`after` is decided on: `"vertical"` by Y, `"horizontal"` and `"grid"` by
   * X (a grid reads left-to-right within a row). */
  axis: "vertical" | "horizontal" | "grid";
  /** Receives the result of a drag that ends over the page (not on `cancel`). */
  onDrop: (result: DropResult) => void;
  /** Called on every armed pointer move with the current drop resolution, and with `null` when
   * the drag ends or cancels; drives a caller's drop-indicator styling. */
  onHover?: (result: DropResult | null) => void;
  /** Whether a target accepts an `"into"` drop. Omitted = no target does. */
  overIsContainer?: (id: string) => boolean;
  /** When `true`, a pointer outside every target resolves to the nearest target on the drag
   * axis (a reorder list with gaps and ends); when `false` or omitted it resolves to no target
   * (a drop zone that must be hit). */
  snapToNearest?: boolean;
  /** Touch hold time before a drag arms. Defaults to `DEFAULT_LONG_PRESS_MS`. */
  longPressMs?: number;
}

/** A drag controller: one drag at a time, driven by pointer events on `window`. */
export interface DragController {
  /** Starts a drag of `id`. Attaches `pointermove`/`pointerup`/`pointercancel` listeners on
   * `window` until the drag ends. A touch pointer arms only after `longPressMs`.
   * @param id The dragged item's id.
   * @param ev The originating `pointerdown`. */
  beginDrag(id: string, ev: PointerEvent): void;
  /** Re-resolves the drop target for the pointer's current position and reports it through
   * `onHover` (no-op until armed).
   * @param ev The pointer event carrying the position. */
  move(ev: PointerEvent): void;
  /** Ends the drag and resolves it at the pointer's position.
   * @param ev The pointer event carrying the release position.
   * @returns The result, or `null` when no armed drag was in progress. */
  end(ev: PointerEvent): DropResult | null;
  /** Aborts the drag without a result. */
  cancel(): void;
  /** The id being dragged once the drag is armed, else `null`. */
  readonly dragging: string | null;
}

/**
 * Index in `order` the dragged id lands on after a before/after drop on `over`, in the
 * post-removal convention `moveInOrder` and `computeReorderUpdate` use.
 * @param order The sibling ids in display order.
 * @param dragged The dragged id.
 * @param over The id the pointer settled over.
 * @param placement `"before"` or `"after"` relative to `over`.
 * @returns The destination index, or `null` when either id is not in `order`.
 * @example
 * ```ts
 * import { dropIndex } from "@shadowcat/core";
 *
 * dropIndex(["a", "b", "c"], "a", "c", "after"); // 2
 * dropIndex(["a", "b", "c"], "c", "a", "before"); // 0
 * ```
 */
export function dropIndex(order: string[], dragged: string, over: string, placement: "before" | "after"): number | null {
  const from = order.indexOf(dragged);
  const overIdx = order.indexOf(over);
  if (from < 0 || overIdx < 0) return null;
  const slot = placement === "before" ? overIdx : overIdx + 1;
  return from < slot ? slot - 1 : slot;
}

/**
 * Creates a drag controller.
 * @param opts The targets, axis and drop callback.
 * @returns The controller.
 * @example
 * ```ts
 * import { createDragController } from "@shadowcat/core";
 *
 * const drag = createDragController({
 *   axis: "vertical",
 *   getTargets: () => [],
 *   onDrop: (result) => void result,
 * });
 * drag.dragging; // null
 * ```
 */
export function createDragController(opts: DragControllerOptions): DragController {
  const longPressMs = opts.longPressMs ?? DEFAULT_LONG_PRESS_MS;
  let dragged: string | null = null;
  let armed = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let detach: (() => void) | null = null;

  /** Cancels a touch drag's page scroll once armed; the finger held still through the long
   * press, so the first `touchmove` is still cancelable.
   * @param e The touch event.
   * @example
   * ```ts
   * import { createDragController } from "@shadowcat/core";
   *
   * const drag = createDragController({ axis: "vertical", getTargets: () => [], onDrop: (result) => void result });
   * declare const ev: PointerEvent;
   * drag.beginDrag("a", ev); // a touch pointer arms after the long press, then blocks page scroll
   * drag.cancel(); // removes the touchmove listener
   * ```
   */
  function blockScroll(e: Event): void {
    if (e.cancelable) e.preventDefault();
  }

  /** Clears all drag state and listeners.
   * @example
   * ```ts
   * import { createDragController } from "@shadowcat/core";
   *
   * const drag = createDragController({ axis: "vertical", getTargets: () => [], onDrop: (result) => void result });
   * declare const ev: PointerEvent;
   * drag.beginDrag("a", ev);
   * drag.cancel(); // clears the drag state and listeners
   * drag.dragging; // null
   * ```
   */
  function reset(): void {
    if (timer !== null) clearTimeout(timer);
    timer = null;
    detach?.();
    detach = null;
    dragged = null;
    armed = false;
  }

  /** Resolves the drop for a pointer position against the current targets.
   * @param x The pointer's `clientX`.
   * @param y The pointer's `clientY`.
   * @returns The result for the active drag.
   * @example
   * ```ts
   * import { createDragController } from "@shadowcat/core";
   *
   * const drag = createDragController({ axis: "vertical", getTargets: () => [], onDrop: (result) => void result });
   * declare const ev: PointerEvent;
   * drag.beginDrag("a", ev);
   * drag.end(ev); // resolves the drop at the pointer position
   * ```
   */
  function resolve(x: number, y: number): DropResult {
    const id = dragged as string;
    const vertical = opts.axis === "vertical";
    let best: DragTarget | null = null;
    let bestRect: DOMRect | null = null;
    let bestDist = Infinity;
    for (const t of opts.getTargets()) {
      if (t.id === id) continue;
      const r = t.rect();
      const dx = x < r.left ? r.left - x : x > r.right ? x - r.right : 0;
      const dy = y < r.top ? r.top - y : y > r.bottom ? y - r.bottom : 0;
      const inside = dx === 0 && dy === 0;
      if (!opts.snapToNearest && !inside) continue;
      // Snap mode measures along the drag axis only, so a pointer beside a list still picks a row.
      const dist = !opts.snapToNearest ? 0 : opts.axis === "vertical" ? dy : opts.axis === "horizontal" ? dx : dx * dx + dy * dy;
      if (dist < bestDist) {
        best = t;
        bestRect = r;
        bestDist = dist;
      }
    }
    if (!best || !bestRect) return { dragged: id, over: null, placement: "after" };
    const pos = vertical ? y : x;
    const start = vertical ? bestRect.top : bestRect.left;
    const size = vertical ? bestRect.height : bestRect.width;
    const frac = size > 0 ? (pos - start) / size : 0.5;
    let placement: DropPlacement = frac <= 0.5 ? "before" : "after";
    if (opts.overIsContainer?.(best.id) && frac >= INTO_BAND && frac <= 1 - INTO_BAND) placement = "into";
    return { dragged: id, over: best.id, placement };
  }

  /** Ends the drag at the event's position.
   * @param ev The release event.
   * @returns The result, or `null` when no armed drag was in progress.
   * @example
   * ```ts
   * import { createDragController } from "@shadowcat/core";
   *
   * const drag = createDragController({ axis: "vertical", getTargets: () => [], onDrop: (result) => void result });
   * declare const ev: PointerEvent;
   * drag.beginDrag("a", ev);
   * drag.end(ev); // a DropResult, or null when the drag never armed
   * ```
   */
  function finish(ev: PointerEvent): DropResult | null {
    const result = dragged !== null && armed ? resolve(ev.clientX ?? 0, ev.clientY ?? 0) : null;
    reset();
    return result;
  }

  /** Re-resolves the hover target and reports it.
   * @param ev The pointer event carrying the position.
   * @example
   * ```ts
   * import { createDragController } from "@shadowcat/core";
   *
   * const drag = createDragController({ axis: "vertical", getTargets: () => [], onDrop: (result) => void result });
   * declare const ev: PointerEvent;
   * drag.beginDrag("a", ev);
   * drag.move(ev); // reports the hovered target through onHover
   * ```
   */
  function move(ev: PointerEvent): void {
    if (dragged === null || !armed) return;
    opts.onHover?.(resolve(ev.clientX ?? 0, ev.clientY ?? 0));
  }

  /** Aborts the drag and clears the hover report.
   * @example
   * ```ts
   * import { createDragController } from "@shadowcat/core";
   *
   * const drag = createDragController({ axis: "vertical", getTargets: () => [], onDrop: (result) => void result });
   * declare const ev: PointerEvent;
   * drag.beginDrag("a", ev);
   * drag.cancel(); // aborts and reports onHover(null)
   * ```
   */
  function cancel(): void {
    reset();
    opts.onHover?.(null);
  }

  return {
    beginDrag(id, ev) {
      reset();
      dragged = id;
      const onUp = (e: PointerEvent): void => {
        const result = finish(e);
        opts.onHover?.(null);
        if (result) opts.onDrop(result);
      };
      const onMove = move;
      const onCancel = cancel;
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
      window.addEventListener("pointercancel", onCancel);
      detach = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        window.removeEventListener("pointercancel", onCancel);
        window.removeEventListener("touchmove", blockScroll);
      };
      if (ev.pointerType === TOUCH_POINTER_TYPE) {
        timer = setTimeout(() => {
          timer = null;
          armed = true;
          window.addEventListener("touchmove", blockScroll, { passive: false });
        }, longPressMs);
      } else {
        armed = true;
      }
    },
    move,
    end(ev) {
      const result = finish(ev);
      opts.onHover?.(null);
      return result;
    },
    cancel,
    get dragging() {
      return armed ? dragged : null;
    },
  };
}
