import { describe, it, expect } from "vitest";
import { applyEasing } from "./easing";

describe("applyEasing", () => {
  it("linear is the identity on [0,1]", () => {
    expect(applyEasing("linear", 0)).toBe(0);
    expect(applyEasing("linear", 0.25)).toBeCloseTo(0.25);
    expect(applyEasing("linear", 1)).toBe(1);
  });

  it("easeInOut pins endpoints and is symmetric about the midpoint", () => {
    expect(applyEasing("easeInOut", 0)).toBe(0);
    expect(applyEasing("easeInOut", 1)).toBe(1);
    expect(applyEasing("easeInOut", 0.5)).toBeCloseTo(0.5);
    // Symmetry: f(t) + f(1-t) === 1.
    expect(applyEasing("easeInOut", 0.3) + applyEasing("easeInOut", 0.7)).toBeCloseTo(1);
  });

  it("easeInOut starts slower than linear (ease-in) below the midpoint", () => {
    expect(applyEasing("easeInOut", 0.25)).toBeLessThan(0.25);
  });

  it("clamps out-of-range input", () => {
    expect(applyEasing("linear", -1)).toBe(0);
    expect(applyEasing("easeInOut", 5)).toBe(1);
  });
});
