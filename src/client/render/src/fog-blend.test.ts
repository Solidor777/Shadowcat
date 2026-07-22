import { describe, expect, test } from "vitest";
import { computeFogBlendFactor, fogBlendRtStale } from "./fog-blend";

describe("computeFogBlendFactor", () => {
  test("is 0 at tCur and 1 at tNext", () => {
    expect(computeFogBlendFactor(0, 0, 500)).toBe(0);
    expect(computeFogBlendFactor(500, 0, 500)).toBe(1);
  });

  test("advances linearly 0→1 across the interval", () => {
    expect(computeFogBlendFactor(125, 0, 500)).toBeCloseTo(0.25);
    expect(computeFogBlendFactor(250, 0, 500)).toBeCloseTo(0.5);
    expect(computeFogBlendFactor(375, 0, 500)).toBeCloseTo(0.75);
  });

  test("clamps outside the interval", () => {
    expect(computeFogBlendFactor(-50, 0, 500)).toBe(0);
    expect(computeFogBlendFactor(600, 0, 500)).toBe(1);
  });

  test("snaps to 1 on a degenerate or inverted span (tNext <= tCur)", () => {
    expect(computeFogBlendFactor(100, 500, 500)).toBe(1);
    expect(computeFogBlendFactor(100, 500, 200)).toBe(1);
  });

  test("fails safe (snaps to 1) on non-finite input", () => {
    expect(computeFogBlendFactor(NaN, 0, 500)).toBe(1);
    expect(computeFogBlendFactor(100, NaN, 500)).toBe(1);
    expect(computeFogBlendFactor(100, 0, Infinity)).toBe(1);
  });
});

describe("fogBlendRtStale", () => {
  test("stales when there is no existing texture", () => {
    expect(fogBlendRtStale(null, 800, 600, 1)).toBe(true);
  });

  test("does not stale when width/height/resolution are unchanged", () => {
    expect(fogBlendRtStale({ width: 800, height: 600, resolution: 1 }, 800, 600, 1)).toBe(false);
  });

  test("stales on a width or height change", () => {
    expect(fogBlendRtStale({ width: 800, height: 600, resolution: 1 }, 1024, 600, 1)).toBe(true);
    expect(fogBlendRtStale({ width: 800, height: 600, resolution: 1 }, 800, 768, 1)).toBe(true);
  });

  test("stales on a resolution change", () => {
    expect(fogBlendRtStale({ width: 800, height: 600, resolution: 1 }, 800, 600, 2)).toBe(true);
  });
});
