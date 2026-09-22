// @vitest-environment node
// Exercises plain pure functions: no component render and no DOM API use, so the package-default
// jsdom environment would be constructed per file and never touched.
import { describe, it, expect } from "vitest";
import { allocateSortKey, appendSortKey, SORT_KEY_DENSITY } from "./sort-key";

describe("allocateSortKey", () => {
  it("allocates the density constant for the first sibling of an empty list", () => {
    expect(allocateSortKey(null, null)).toEqual({ key: SORT_KEY_DENSITY, renumber: false });
  });

  it("appends by advancing one density step past the last sibling", () => {
    expect(allocateSortKey(1000, null)).toEqual({ key: 2000, renumber: false });
    expect(allocateSortKey(2000, null)).toEqual({ key: 3000, renumber: false });
  });

  it("prepends by receding one density step before the first sibling", () => {
    expect(allocateSortKey(null, 1000)).toEqual({ key: 0, renumber: false });
  });

  it("inserts between two siblings at the integer midpoint", () => {
    expect(allocateSortKey(1000, 2000)).toEqual({ key: 1500, renumber: false });
    expect(allocateSortKey(1000, 1004)).toEqual({ key: 1002, renumber: false });
  });

  it("signals renumber when no integer sits strictly between two adjacent keys", () => {
    expect(allocateSortKey(1000, 1001)).toEqual({ key: 1000, renumber: true });
    expect(allocateSortKey(1000, 1000)).toEqual({ key: 1000, renumber: true });
  });
});

describe("appendSortKey", () => {
  it("allocates the density constant for an empty sibling list", () => {
    expect(appendSortKey([])).toBe(SORT_KEY_DENSITY);
  });

  it("advances one density step past the highest sibling value, regardless of input order", () => {
    expect(appendSortKey([1000])).toBe(2000);
    expect(appendSortKey([1000, 2000])).toBe(3000);
    // Order-independent: the caller is not required to pre-sort — the highest wins either way.
    expect(appendSortKey([2000, 1000])).toBe(3000);
  });
});
