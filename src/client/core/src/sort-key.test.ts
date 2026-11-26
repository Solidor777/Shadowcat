// @vitest-environment node
// Exercises plain pure functions: no component render and no DOM API use, so the package-default
// jsdom environment would be constructed per file and never touched.
import { describe, it, expect } from "vitest";
import { allocateSortKey, appendSortKey, buildReorderOps, computeReorderUpdate, SORT_KEY_DENSITY } from "./sort-key";

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

describe("computeReorderUpdate", () => {
  const abc = [
    { id: "a", sort: 1000 },
    { id: "b", sort: 2000 },
    { id: "c", sort: 3000 },
  ];

  it("a move with room in the gap changes only the moved sibling, to the gap's midpoint", () => {
    expect(computeReorderUpdate(abc, 0, 1)).toEqual([{ docId: "a", sort: 2500, previous: 1000 }]);
    expect(computeReorderUpdate(abc, 2, 1)).toEqual([{ docId: "c", sort: 1500, previous: 3000 }]);
  });

  it("moving to the very start or end steps one density past the neighbour", () => {
    expect(computeReorderUpdate(abc, 2, 0)).toEqual([{ docId: "c", sort: 0, previous: 3000 }]);
    expect(computeReorderUpdate(abc, 0, 2)).toEqual([{ docId: "a", sort: 4000, previous: 1000 }]);
  });

  it("an exhausted gap renumbers every sibling whose key changes, in the post-move order", () => {
    const tight = [
      { id: "a", sort: 1000 },
      { id: "b", sort: 1001 },
      { id: "c", sort: 1002 },
    ];
    // c moves between a and b: no integer between 1000 and 1001.
    // a already holds the first fresh key (1000) and is left out.
    expect(computeReorderUpdate(tight, 2, 1)).toEqual([
      { docId: "c", sort: 2 * SORT_KEY_DENSITY, previous: 1002 },
      { docId: "b", sort: 3 * SORT_KEY_DENSITY, previous: 1001 },
    ]);
  });

  it("siblings that tie on sort force a renumber even when the move is one step", () => {
    const tied = [
      { id: "a", sort: 0 },
      { id: "b", sort: 0 },
      { id: "c", sort: 0 },
    ];
    expect(computeReorderUpdate(tied, 0, 1)).toEqual([
      { docId: "b", sort: SORT_KEY_DENSITY, previous: 0 },
      { docId: "a", sort: 2 * SORT_KEY_DENSITY, previous: 0 },
      { docId: "c", sort: 3 * SORT_KEY_DENSITY, previous: 0 },
    ]);
  });

  it("moving an item to its own position is a no-op", () => {
    expect(computeReorderUpdate(abc, 1, 1)).toBeNull();
  });

  it("rejects an out-of-range index", () => {
    expect(() => computeReorderUpdate(abc, 0, 3)).toThrow(RangeError);
    expect(() => computeReorderUpdate(abc, -1, 0)).toThrow(RangeError);
  });
});

describe("buildReorderOps", () => {
  it("builds one Update per changed sibling, carrying the stored key as the pre-image", () => {
    const ops = buildReorderOps(
      [
        { id: "a", sort: 1000 },
        { id: "b", sort: 2000 },
      ],
      1,
      0,
    );
    expect(ops).toEqual([{ op: "update", doc_id: "b", changes: [{ path: "/engine/sort", old: 2000, new: 0 }] }]);
  });

  it("is null for a no-op move", () => {
    expect(buildReorderOps([{ id: "a", sort: 1000 }], 0, 0)).toBeNull();
  });
});
