import { describe, it, expect } from "vitest";
import { collinearRuns } from "./path-runs";

describe("collinearRuns", () => {
  it("collapses a straight horizontal run to its endpoints", () => {
    expect(collinearRuns([[0, 0], [100, 0], [200, 0], [300, 0]])).toEqual([[0, 0], [300, 0]]);
  });

  it("keeps the corner of an L-route", () => {
    expect(collinearRuns([[0, 0], [100, 0], [200, 0], [200, 100], [200, 200]])).toEqual([
      [0, 0], [200, 0], [200, 200],
    ]);
  });

  it("keeps a diagonal run as one segment and its turn", () => {
    expect(collinearRuns([[0, 0], [100, 100], [200, 200], [300, 200]])).toEqual([
      [0, 0], [200, 200], [300, 200],
    ]);
  });

  it("passes through trivial paths unchanged", () => {
    expect(collinearRuns([[5, 5]])).toEqual([[5, 5]]);
    expect(collinearRuns([[0, 0], [50, 0]])).toEqual([[0, 0], [50, 0]]);
  });
});
