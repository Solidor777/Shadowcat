// @vitest-environment node
import { describe, it, expect } from "vitest";
import { shapeFor, realFaceCountOf } from "./shapes";

describe("shapeFor", () => {
  it.each([
    [1, "d4", 4],
    [3, "d4", 4],
    [4, "d4", 4],
    [5, "d6", 6],
    [6, "d6", 6],
    [7, "d8", 8],
    [8, "d8", 8],
    [9, "d10", 10],
    [10, "d10", 10],
    [11, "d12", 12],
    [12, "d12", 12],
    [13, "d20", 20],
    [20, "d20", 20],
  ])("realFaceCount %i resolves to %s with %i physical faces", (real, shape, physical) => {
    const resolved = shapeFor(real as number);
    expect(resolved.shape).toBe(shape);
    expect(resolved.physicalFaceCount).toBe(physical);
    expect(resolved.realFaceCount).toBe(real);
    expect(resolved.sameLabel).toBe(false);
  });

  it("renders an over-large kind as a d20 value chip", () => {
    expect(shapeFor(37)).toEqual({ shape: "d20", physicalFaceCount: 20, realFaceCount: 37, sameLabel: true });
    expect(shapeFor(20).sameLabel).toBe(false);
  });
});

describe("realFaceCountOf", () => {
  it("counts a Numeric die's inclusive range", () => {
    expect(realFaceCountOf({ Numeric: { min: 1, max: 20 } })).toBe(20);
  });

  it("counts a Faces die's explicit list length", () => {
    expect(realFaceCountOf({ Faces: { faces: [{ symbols: ["a"] }, { symbols: ["b"] }, { symbols: ["c"] }] } })).toBe(3);
  });
});
