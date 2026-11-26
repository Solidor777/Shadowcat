// @vitest-environment node
import { describe, it, expect } from "vitest";
import { pointInPolygon, isTokenRevealed } from "./reveal";
import type { VisibilityInput } from "./types";

const SQUARE = [0, 0, 10, 0, 10, 10, 0, 10];
const masked = (over: Partial<VisibilityInput>): VisibilityInput => ({ mode: "masked", visible: [], explored: [], perceived: [], ...over });

describe("pointInPolygon", () => {
  it("is true inside and false outside a square", () => {
    expect(pointInPolygon({ x: 5, y: 5 }, SQUARE)).toBe(true);
    expect(pointInPolygon({ x: 15, y: 5 }, SQUARE)).toBe(false);
    expect(pointInPolygon({ x: 5, y: -1 }, SQUARE)).toBe(false);
  });

  it("handles a concave polygon's notch", () => {
    // A "C" shape opening to +x: the notch interior (8, 5) is outside.
    const c = [0, 0, 10, 0, 10, 3, 4, 3, 4, 7, 10, 7, 10, 10, 0, 10];
    expect(pointInPolygon({ x: 2, y: 5 }, c)).toBe(true);
    expect(pointInPolygon({ x: 8, y: 5 }, c)).toBe(false);
  });

  it("treats a polygon with fewer than three vertices as empty", () => {
    expect(pointInPolygon({ x: 0, y: 0 }, [0, 0, 5, 5])).toBe(false);
    expect(pointInPolygon({ x: 0, y: 0 }, [])).toBe(false);
  });
});

describe("isTokenRevealed", () => {
  const at = { x: 5, y: 5 };

  it("reveals everything when there is no fog", () => {
    expect(isTokenRevealed({ mode: "all", visible: [], explored: [], perceived: [] }, "t", at)).toBe(true);
  });

  it("hides a token under full fog (masked, nothing visible)", () => {
    expect(isTokenRevealed(masked({}), "t", at)).toBe(false);
  });

  it("reveals a token inside a visible polygon and hides one outside it", () => {
    const input = masked({ visible: [{ points: SQUARE }] });
    expect(isTokenRevealed(input, "t", at)).toBe(true);
    expect(isTokenRevealed(input, "t", { x: 50, y: 50 })).toBe(false);
  });

  it("does not reveal a token that sits only in the explored (memory) region", () => {
    expect(isTokenRevealed(masked({ explored: [{ points: SQUARE }] }), "t", at)).toBe(false);
  });

  it("reveals a creature-sense perceived token wherever it is", () => {
    expect(isTokenRevealed(masked({ perceived: ["t"] }), "t", { x: 999, y: 999 })).toBe(true);
    expect(isTokenRevealed(masked({ perceived: ["other"] }), "t", { x: 999, y: 999 })).toBe(false);
  });
});
