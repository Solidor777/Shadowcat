import { test, expect } from "vitest";
import { computeAnimatedFrame } from "./token-animation";

test("advances one frame per 1000/fps ms", () => {
  expect(computeAnimatedFrame(0, 8, 10, true)).toBe(0);
  expect(computeAnimatedFrame(125, 8, 10, true)).toBe(1); // 1000/8 = 125ms/frame
  expect(computeAnimatedFrame(999, 8, 10, true)).toBe(7);
});

test("loops by wrapping past the frame count", () => {
  expect(computeAnimatedFrame(1250, 8, 10, true)).toBe(0); // frame 10 -> wraps to 0
  expect(computeAnimatedFrame(1375, 8, 10, true)).toBe(1);
});

test("a one-shot (loop:false) clamps to the last frame and holds", () => {
  expect(computeAnimatedFrame(1250, 8, 10, false)).toBe(9); // frame 10 clamps to index 9
  expect(computeAnimatedFrame(100_000, 8, 10, false)).toBe(9);
});

test("fails closed to frame 0 on degenerate input", () => {
  expect(computeAnimatedFrame(NaN, 8, 10, true)).toBe(0);
  expect(computeAnimatedFrame(100, NaN, 10, true)).toBe(0);
  expect(computeAnimatedFrame(100, 0, 10, true)).toBe(0);
  expect(computeAnimatedFrame(100, -1, 10, true)).toBe(0);
  expect(computeAnimatedFrame(100, 8, 0, true)).toBe(0);
  expect(computeAnimatedFrame(100, 8, -1, true)).toBe(0);
});
