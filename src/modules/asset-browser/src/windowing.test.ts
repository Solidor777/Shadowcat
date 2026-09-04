import { describe, it, expect } from "vitest";
import { computeGridWindow } from "./windowing";

describe("computeGridWindow", () => {
  // 100 items, 4 columns → 25 rows; container shows 200px of a 1000px scroll
  // height → 40px rows, 5 visible rows.
  it("windows whole rows around the scroll viewport with overscan", () => {
    const w = computeGridWindow(400, 200, 1000, 100, 4, 1);
    // Visible rows 10..15, overscan 1 row each side → rows 9..16.
    expect(w.start).toBe(9 * 4);
    expect(w.end).toBe(Math.min(100, 16 * 4));
  });

  it("clamps the overscan at both list ends", () => {
    const top = computeGridWindow(0, 200, 1000, 100, 4, 3);
    expect(top.start).toBe(0);
    const bottom = computeGridWindow(800, 200, 1000, 100, 4, 3);
    expect(bottom.end).toBe(100);
  });

  it("returns an empty window for an empty list", () => {
    expect(computeGridWindow(0, 200, 1000, 0, 4)).toEqual({ start: 0, end: 0 });
  });

  it("falls back to a small leading window when measurements are unusable", () => {
    // A hidden tab reads 0 geometry; render a bounded leading window rather
    // than everything (or nothing).
    const w = computeGridWindow(0, 0, 0, 500, 4, 2);
    expect(w.start).toBe(0);
    expect(w.end).toBeGreaterThan(0);
    expect(w.end).toBeLessThan(100);
  });
});
