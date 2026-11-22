import { test, expect } from "vitest";
import { dashRing } from "./dash";

test("an open line splits into evenly spaced dashes", () => {
  expect(dashRing([0, 0, 10, 0], false, 2, 2)).toEqual([
    [0, 0, 2, 0],
    [4, 0, 6, 0],
    [8, 0, 10, 0],
  ]);
});

test("a dash keeps the vertex it spans", () => {
  const dashes = dashRing([0, 0, 3, 0, 3, 3], false, 4, 100);
  expect(dashes).toEqual([[0, 0, 3, 0, 3, 1]]);
});

test("a closed ring dashes its closing edge too", () => {
  const dashes = dashRing([0, 0, 4, 0, 4, 4, 0, 4], true, 1, 1);
  const last = dashes[dashes.length - 1];
  // Perimeter 16: dashes cover [0,1], [2,3] ... [14,15]; the last lies on the closing edge
  // (x = 0, descending from y = 2 to y = 1) and the final unit is the gap.
  expect(last).toEqual([0, 2, 0, 1]);
  expect(dashes.length).toBe(8);
});

test("a non-positive dash length returns the ring unbroken", () => {
  expect(dashRing([0, 0, 10, 0], false, 0, 2)).toEqual([[0, 0, 10, 0]]);
});

test("fewer than two points yield no dashes", () => {
  expect(dashRing([5, 5], false, 2, 2)).toEqual([]);
});
