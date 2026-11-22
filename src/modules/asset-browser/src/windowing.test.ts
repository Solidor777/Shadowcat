// @vitest-environment node
// Exercises plain state and pure functions: no component render and no DOM API use, so
// the package-default jsdom environment would be constructed per file and never touched.
import { describe, it, expect } from "vitest";
import { computeGridWindow, computeListWindow } from "./windowing";

describe("computeListWindow", () => {
  // 100 rows of 40px → 4000px scroll height; 200px viewport at scrollTop 400 shows rows 10..15.
  it("windows rows around the scroll viewport with overscan", () => {
    expect(computeListWindow(400, 200, 4000, 100, 40, 1)).toEqual({ start: 9, end: 16 });
  });

  it("clamps at both list ends", () => {
    expect(computeListWindow(0, 200, 4000, 100, 40, 3).start).toBe(0);
    expect(computeListWindow(3800, 200, 4000, 100, 40, 3).end).toBe(100);
  });

  it("includes a row exactly at the viewport's bottom edge only through overscan", () => {
    // scrollTop 0, viewport exactly 5 rows: last visible row index is 4, so end = 5 + overscan.
    expect(computeListWindow(0, 200, 4000, 100, 40, 0)).toEqual({ start: 0, end: 5 });
    // One pixel more of viewport reveals row 5.
    expect(computeListWindow(0, 201, 4000, 100, 40, 0)).toEqual({ start: 0, end: 6 });
  });

  it("returns an empty window for an empty list", () => {
    expect(computeListWindow(0, 200, 0, 0, 40)).toEqual({ start: 0, end: 0 });
  });

  it("falls back to a small leading window when measurements are unusable", () => {
    const w = computeListWindow(0, 0, 0, 500, 40, 2);
    expect(w).toEqual({ start: 0, end: 5 });
  });
});

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
