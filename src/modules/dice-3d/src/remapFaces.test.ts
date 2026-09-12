// @vitest-environment node
import { describe, it, expect } from "vitest";
import { remapFaces } from "./remapFaces";

function isBijection(order: number[], faceCount: number): boolean {
  return new Set(order).size === faceCount && order.every((v) => v >= 0 && v < faceCount);
}

describe("remapFaces", () => {
  it.each([
    { faceCount: 4, up: 0, target: 2 },
    { faceCount: 6, up: 0, target: 3 },
    { faceCount: 8, up: 5, target: 0 },
    { faceCount: 10, up: 9, target: 4 },
    { faceCount: 12, up: 3, target: 11 },
    { faceCount: 20, up: 19, target: 0 },
  ])("standard shape d$faceCount: up=$up target=$target lands and stays bijective", ({ faceCount, up, target }) => {
    const order = remapFaces(up, faceCount, target);
    expect(order).toHaveLength(faceCount);
    expect(order[up]).toBe(target);
    expect(isBijection(order, faceCount)).toBe(true);
  });

  it("d6 truth table matches the documented example exactly", () => {
    expect(remapFaces(0, 6, 3)).toEqual([3, 4, 5, 0, 1, 2]);
  });

  it("d100 split: two independent d10 calls place the tens and ones digits", () => {
    const total = 47; // tens digit 4, ones digit 7
    const tens = remapFaces(2, 10, Math.floor(total / 10) % 10);
    const ones = remapFaces(6, 10, total % 10);
    expect(tens[2]).toBe(4);
    expect(ones[6]).toBe(7);
    expect(isBijection(tens, 10)).toBe(true);
    expect(isBijection(ones, 10)).toBe(true);
  });

  it("symbolic faces: the mechanism is face-index-generic, independent of what a face means", () => {
    const order = remapFaces(1, 3, 2);
    expect(order).toEqual([1, 2, 0]);
  });

  it("unused-face padding: a d3 rendered on a 6-face physical shape still lands within its own 3 real labels", () => {
    // The die's real labels are indices 0..2; the caller renders indices 3..5 blank. remapFaces
    // itself is agnostic to which indices are "real" — it only guarantees the target lands.
    const order = remapFaces(4, 6, 1);
    expect(order[4]).toBe(1);
    expect(isBijection(order, 6)).toBe(true);
  });
});
