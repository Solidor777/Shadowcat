import { describe, it, expect } from "vitest";
import { createReorder } from "./reorder";

function rect(top: number, height = 40): DOMRect {
  return { top, height, bottom: top + height, left: 0, right: 0, width: 0, x: 0, y: top, toJSON: () => ({}) };
}

function ptr(clientY: number): PointerEvent {
  return { clientY } as PointerEvent;
}

describe("createReorder", () => {
  const rows = [rect(0), rect(40), rect(80), rect(120)]; // mids: 20, 60, 100, 140

  it("dragging from index 0 past the second row's own midpoint ends at { from: 0, to: 1 }", () => {
    const reorder = createReorder(() => rows);
    reorder.beginDrag(0, ptr(0));
    reorder.move(ptr(70)); // past mid(1)=60, before mid(2)=100
    expect(reorder.end(ptr(70))).toEqual({ from: 0, to: 1 });
  });

  it("dragging further, past the third row's midpoint, lands at to: 2", () => {
    const reorder = createReorder(() => rows);
    reorder.beginDrag(0, ptr(0));
    reorder.move(ptr(110));
    expect(reorder.end(ptr(110))).toEqual({ from: 0, to: 2 });
  });

  it("returns null when the target never moves off the start (a no-op drag)", () => {
    const reorder = createReorder(() => rows);
    reorder.beginDrag(0, ptr(0));
    reorder.move(ptr(10)); // still above every other row's midpoint
    expect(reorder.end(ptr(10))).toBeNull();
  });

  it("Escape (cancel) discards the drag; a subsequent end() returns null", () => {
    const reorder = createReorder(() => rows);
    reorder.beginDrag(0, ptr(0));
    reorder.move(ptr(70));
    reorder.cancel();
    expect(reorder.end(ptr(70))).toBeNull();
  });

  it("end() without a prior beginDrag returns null", () => {
    const reorder = createReorder(() => rows);
    expect(reorder.end(ptr(0))).toBeNull();
  });

  it("move() before beginDrag is a no-op (does not throw, no target armed)", () => {
    const reorder = createReorder(() => rows);
    reorder.move(ptr(70));
    expect(reorder.end(ptr(70))).toBeNull();
  });
});
