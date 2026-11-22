import { test, expect } from "vitest";
import { dashPath } from "./dash";

test("an open line splits into evenly spaced dashes", () => {
  expect(dashPath([0, 0, 10, 0], false, 2, 2)).toEqual([
    [0, 0, 2, 0],
    [4, 0, 6, 0],
    [8, 0, 10, 0],
  ]);
});

test("the last dash is clipped to the path end", () => {
  expect(dashPath([0, 0, 10, 0], false, 4, 2)).toEqual([[0, 0, 4, 0], [6, 0, 10, 0]]);
  expect(dashPath([0, 0, 9, 0], false, 4, 2)).toEqual([[0, 0, 4, 0], [6, 0, 9, 0]]);
  expect(dashPath([0, 0, 0, 5], false, 2, 1)).toEqual([[0, 0, 0, 2], [0, 3, 0, 5]]);
});

test("a dash keeps the vertex it spans", () => {
  const dashes = dashPath([0, 0, 3, 0, 3, 3], false, 4, 100);
  expect(dashes).toEqual([[0, 0, 3, 0, 3, 1]]);
});

test("a closed ring dashes its closing edge too", () => {
  const dashes = dashPath([0, 0, 4, 0, 4, 4, 0, 4], true, 1, 1);
  const last = dashes[dashes.length - 1];
  // Perimeter 16: dashes cover [0,1], [2,3] ... [14,15]; the last lies on the closing edge
  // (x = 0, descending from y = 2 to y = 1) and the final unit is the gap.
  expect(last).toEqual([0, 2, 0, 1]);
  expect(dashes.length).toBe(8);
});

test("a zero-length path yields no dashes", () => {
  expect(dashPath([1, 1, 1, 1], false, 4, 2)).toEqual([]);
});

test("a non-positive or non-finite pattern yields no dashes", () => {
  expect(dashPath([0, 0, 10, 0], false, 0, 2)).toEqual([]);
  expect(dashPath([0, 0, 10, 0], false, 4, 0)).toEqual([]);
  expect(dashPath([0, 0, 10, 0], false, Number.NaN, 2)).toEqual([]);
  expect(dashPath([0, 0, 10, 0], false, 4, Number.POSITIVE_INFINITY)).toEqual([]);
});

test("fewer than two points yield no dashes", () => {
  expect(dashPath([5, 5], false, 2, 2)).toEqual([]);
});
