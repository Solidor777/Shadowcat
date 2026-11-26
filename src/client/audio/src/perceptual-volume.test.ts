import { describe, it, expect } from "vitest";
import { gainToSliderPosition, sliderPositionToGain } from "./perceptual-volume";

describe("perceptual volume curve", () => {
  it("fixes the ends of the travel", () => {
    expect(sliderPositionToGain(0)).toBe(0);
    expect(sliderPositionToGain(1)).toBe(1);
    expect(gainToSliderPosition(0)).toBe(0);
    expect(gainToSliderPosition(1)).toBe(1);
  });

  it("is applied: mid travel selects less than half gain", () => {
    expect(sliderPositionToGain(0.5)).toBeLessThan(0.5);
    expect(sliderPositionToGain(0.5)).toBeCloseTo(0.125, 10);
  });

  it("round-trips, so a stored gain lands the handle where it was set", () => {
    for (const x of [0.05, 0.25, 0.4, 0.5, 0.8, 0.95]) {
      expect(gainToSliderPosition(sliderPositionToGain(x))).toBeCloseTo(x, 10);
    }
  });

  it("clamps out-of-range input", () => {
    expect(sliderPositionToGain(-1)).toBe(0);
    expect(sliderPositionToGain(2)).toBe(1);
    expect(gainToSliderPosition(-1)).toBe(0);
    expect(gainToSliderPosition(2)).toBe(1);
  });
});
