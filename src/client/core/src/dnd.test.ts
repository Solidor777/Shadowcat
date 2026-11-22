// @vitest-environment jsdom
// The controller attaches its pointer listeners to `window`.
import { afterEach, describe, expect, it, vi } from "vitest";
import { createDragController, dropIndex, DEFAULT_LONG_PRESS_MS, type DragTarget, type DropResult } from "./dnd";

/** A target with a fixed rect. */
function target(id: string, left: number, top: number, width: number, height: number): DragTarget {
  const rect = { left, top, width, height, right: left + width, bottom: top + height, x: left, y: top, toJSON: () => ({}) } as DOMRect;
  return { id, rect: () => rect };
}

/** A pointer-event stand-in carrying only what the controller reads. */
function ptr(x: number, y: number, pointerType = "mouse"): PointerEvent {
  return { clientX: x, clientY: y, pointerType } as PointerEvent;
}

/** Four 40px-tall stacked rows: mids 20, 60, 100, 140. */
const ROWS = [target("a", 0, 0, 100, 40), target("b", 0, 40, 100, 40), target("c", 0, 80, 100, 40), target("d", 0, 120, 100, 40)];

afterEach(() => {
  vi.useRealTimers();
});

describe("createDragController placement", () => {
  it("vertical: the pointer's half of the nearest other row decides before/after", () => {
    const drag = createDragController({ axis: "vertical", getTargets: () => ROWS, onDrop: () => {}, snapToNearest: true });
    drag.beginDrag("a", ptr(0, 0));
    expect(drag.end(ptr(10, 50))).toEqual({ dragged: "a", over: "b", placement: "before" }); // above mid(b)=60
    drag.beginDrag("a", ptr(0, 0));
    expect(drag.end(ptr(10, 70))).toEqual({ dragged: "a", over: "b", placement: "after" }); // below mid(b)
  });

  it("vertical: crossing exactly the midpoint is still before (matches a midpoint-crossing count)", () => {
    const drag = createDragController({ axis: "vertical", getTargets: () => ROWS, onDrop: () => {}, snapToNearest: true });
    drag.beginDrag("a", ptr(0, 0));
    expect(drag.end(ptr(10, 60))?.placement).toBe("before");
  });

  it("horizontal and grid read X within the target", () => {
    const cols = [target("a", 0, 0, 40, 40), target("b", 40, 0, 40, 40)];
    for (const axis of ["horizontal", "grid"] as const) {
      const drag = createDragController({ axis, getTargets: () => cols, onDrop: () => {}, snapToNearest: true });
      drag.beginDrag("a", ptr(0, 0));
      expect(drag.end(ptr(50, 20))).toEqual({ dragged: "a", over: "b", placement: "before" });
      drag.beginDrag("a", ptr(0, 0));
      expect(drag.end(ptr(70, 20))).toEqual({ dragged: "a", over: "b", placement: "after" });
    }
  });

  it("excludes the dragged target from the candidates", () => {
    const drag = createDragController({ axis: "vertical", getTargets: () => ROWS, onDrop: () => {}, snapToNearest: true });
    drag.beginDrag("b", ptr(0, 0));
    // Pointer inside b's own rect resolves to a neighbour, never to b.
    expect(drag.end(ptr(10, 70))?.over).not.toBe("b");
  });

  it("snapToNearest resolves a pointer in a gap or past either end to the nearest row", () => {
    const gapped = [target("a", 0, 0, 100, 40), target("b", 0, 44, 100, 40)];
    const drag = createDragController({ axis: "vertical", getTargets: () => gapped, onDrop: () => {}, snapToNearest: true });
    drag.beginDrag("a", ptr(0, 0));
    expect(drag.end(ptr(10, 500))).toEqual({ dragged: "a", over: "b", placement: "after" });
    drag.beginDrag("b", ptr(0, 0));
    expect(drag.end(ptr(10, -50))).toEqual({ dragged: "b", over: "a", placement: "before" });
  });

  it("without snapToNearest a pointer outside every target resolves to no target", () => {
    const drag = createDragController({ axis: "vertical", getTargets: () => ROWS, onDrop: () => {} });
    drag.beginDrag("a", ptr(0, 0));
    expect(drag.end(ptr(500, 70))?.over).toBeNull(); // right of every row
    drag.beginDrag("a", ptr(0, 0));
    expect(drag.end(ptr(50, 70))?.over).toBe("b");
  });

  it("into fires only for a container target in the middle half of its extent", () => {
    const drag = createDragController({
      axis: "vertical",
      getTargets: () => ROWS,
      onDrop: () => {},
      overIsContainer: (id) => id === "b",
    });
    drag.beginDrag("a", ptr(0, 0));
    expect(drag.end(ptr(10, 60))?.placement).toBe("into");
    drag.beginDrag("a", ptr(0, 0));
    expect(drag.end(ptr(10, 42))?.placement).toBe("before"); // outer quarter of b
    drag.beginDrag("a", ptr(0, 0));
    expect(drag.end(ptr(10, 78))?.placement).toBe("after");
    drag.beginDrag("a", ptr(0, 0));
    // c is not a container: its middle is before/after, never into.
    expect(drag.end(ptr(10, 100))?.placement).toBe("before");
  });
});

describe("createDragController lifecycle", () => {
  it("end() without a drag returns null", () => {
    const drag = createDragController({ axis: "vertical", getTargets: () => ROWS, onDrop: () => {} });
    expect(drag.end(ptr(0, 0))).toBeNull();
  });

  it("cancel() discards the drag: end() returns null and a later window pointerup drops nothing", () => {
    const onDrop = vi.fn();
    const drag = createDragController({ axis: "vertical", getTargets: () => ROWS, onDrop, snapToNearest: true });
    drag.beginDrag("a", ptr(0, 0));
    drag.cancel();
    window.dispatchEvent(new MouseEvent("pointerup", { clientY: 70 }));
    expect(drag.end(ptr(0, 70))).toBeNull();
    expect(onDrop).not.toHaveBeenCalled();
  });

  it("a window pointerup delivers the result to onDrop once and detaches", () => {
    const onDrop = vi.fn();
    const drag = createDragController({ axis: "vertical", getTargets: () => ROWS, onDrop, snapToNearest: true });
    drag.beginDrag("a", ptr(0, 0));
    window.dispatchEvent(new MouseEvent("pointerup", { clientY: 70 }));
    window.dispatchEvent(new MouseEvent("pointerup", { clientY: 70 }));
    expect(onDrop).toHaveBeenCalledTimes(1);
    expect(onDrop).toHaveBeenCalledWith({ dragged: "a", over: "b", placement: "after" });
  });

  it("a window pointercancel aborts the drag", () => {
    const onDrop = vi.fn();
    const drag = createDragController({ axis: "vertical", getTargets: () => ROWS, onDrop, snapToNearest: true });
    drag.beginDrag("a", ptr(0, 0));
    window.dispatchEvent(new Event("pointercancel"));
    window.dispatchEvent(new MouseEvent("pointerup", { clientY: 70 }));
    expect(onDrop).not.toHaveBeenCalled();
  });

  it("onHover reports the live resolution on window pointermove and clears on release", () => {
    const seen: (DropResult | null)[] = [];
    const drag = createDragController({
      axis: "vertical",
      getTargets: () => ROWS,
      onDrop: () => {},
      snapToNearest: true,
      onHover: (r) => seen.push(r),
    });
    drag.beginDrag("a", ptr(0, 0));
    window.dispatchEvent(new MouseEvent("pointermove", { clientY: 110 }));
    window.dispatchEvent(new MouseEvent("pointerup", { clientY: 110 }));
    expect(seen).toEqual([{ dragged: "a", over: "c", placement: "after" }, null]);
  });

  it("dragging is the dragged id once armed, else null", () => {
    const drag = createDragController({ axis: "vertical", getTargets: () => ROWS, onDrop: () => {} });
    expect(drag.dragging).toBeNull();
    drag.beginDrag("a", ptr(0, 0));
    expect(drag.dragging).toBe("a");
    drag.cancel();
    expect(drag.dragging).toBeNull();
  });
});

describe("createDragController touch long press", () => {
  it("a touch pointer does not arm before the hold time, so a quick release is a tap with no drop", () => {
    vi.useFakeTimers();
    const drag = createDragController({ axis: "vertical", getTargets: () => ROWS, onDrop: () => {}, snapToNearest: true });
    drag.beginDrag("a", ptr(0, 0, "touch"));
    vi.advanceTimersByTime(DEFAULT_LONG_PRESS_MS - 1);
    expect(drag.dragging).toBeNull();
    expect(drag.end(ptr(10, 70, "touch"))).toBeNull();
  });

  it("a touch pointer held for the hold time arms and resolves a drop", () => {
    vi.useFakeTimers();
    const drag = createDragController({ axis: "vertical", getTargets: () => ROWS, onDrop: () => {}, snapToNearest: true });
    drag.beginDrag("a", ptr(0, 0, "touch"));
    vi.advanceTimersByTime(DEFAULT_LONG_PRESS_MS);
    expect(drag.dragging).toBe("a");
    expect(drag.end(ptr(10, 70, "touch"))).toEqual({ dragged: "a", over: "b", placement: "after" });
  });

  it("longPressMs overrides the default hold", () => {
    vi.useFakeTimers();
    const drag = createDragController({ axis: "vertical", getTargets: () => ROWS, onDrop: () => {}, longPressMs: 50 });
    drag.beginDrag("a", ptr(0, 0, "touch"));
    vi.advanceTimersByTime(50);
    expect(drag.dragging).toBe("a");
    drag.cancel();
  });

  it("a mouse or pen pointer arms immediately", () => {
    const drag = createDragController({ axis: "vertical", getTargets: () => ROWS, onDrop: () => {} });
    drag.beginDrag("a", ptr(0, 0, "pen"));
    expect(drag.dragging).toBe("a");
    drag.cancel();
  });

  it("a touchmove is cancelled only once the touch drag is armed, and again released after the drag", () => {
    vi.useFakeTimers();
    const drag = createDragController({ axis: "vertical", getTargets: () => ROWS, onDrop: () => {} });
    drag.beginDrag("a", ptr(0, 0, "touch"));
    const early = new Event("touchmove", { cancelable: true });
    window.dispatchEvent(early);
    expect(early.defaultPrevented).toBe(false);
    vi.advanceTimersByTime(DEFAULT_LONG_PRESS_MS);
    const armed = new Event("touchmove", { cancelable: true });
    window.dispatchEvent(armed);
    expect(armed.defaultPrevented).toBe(true);
    drag.cancel();
    const after = new Event("touchmove", { cancelable: true });
    window.dispatchEvent(after);
    expect(after.defaultPrevented).toBe(false);
  });
});

describe("dropIndex", () => {
  const order = ["a", "b", "c", "d"];

  it("moving down lands one before the slot because the dragged item leaves the list first", () => {
    expect(dropIndex(order, "a", "c", "after")).toBe(2);
    expect(dropIndex(order, "a", "c", "before")).toBe(1);
  });

  it("moving up lands on the slot itself", () => {
    expect(dropIndex(order, "d", "b", "before")).toBe(1);
    expect(dropIndex(order, "d", "b", "after")).toBe(2);
  });

  it("dropping next to the current position is the identity index", () => {
    expect(dropIndex(order, "b", "a", "after")).toBe(1);
    expect(dropIndex(order, "b", "c", "before")).toBe(1);
  });

  it("is null for an id outside the order", () => {
    expect(dropIndex(order, "x", "a", "before")).toBeNull();
    expect(dropIndex(order, "a", "x", "before")).toBeNull();
  });
});
