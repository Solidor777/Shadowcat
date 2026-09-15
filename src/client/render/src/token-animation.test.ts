import { test, expect } from "vitest";
import { computeAnimatedFrame, computeVfxFrame } from "./token-animation";

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

test("computeVfxFrame walks per-frame durations", () => {
  expect(computeVfxFrame(0, [100, 200], 2, true)).toBe(0);
  expect(computeVfxFrame(99, [100, 200], 2, true)).toBe(0);
  expect(computeVfxFrame(100, [100, 200], 2, true)).toBe(1);
  expect(computeVfxFrame(299, [100, 200], 2, true)).toBe(1);
  // Loop wraps by the total duration (300ms): t=350 -> 50ms into frame 0.
  expect(computeVfxFrame(350, [100, 200], 2, true)).toBe(0);
  // A one-shot clamps to the last frame once the total elapses.
  expect(computeVfxFrame(10_000, [100, 200], 2, false)).toBe(1);
});

test("computeVfxFrame defaults a short frameMs tail to 100ms/frame", () => {
  // Three frames, only the first duration known: frames 1/2 advance at 100ms each.
  expect(computeVfxFrame(0, [500], 3, true)).toBe(0);
  expect(computeVfxFrame(500, [500], 3, true)).toBe(1);
  expect(computeVfxFrame(650, [500], 3, true)).toBe(2);
});

test("computeVfxFrame fails closed on degenerate input", () => {
  expect(computeVfxFrame(NaN, [100], 1, true)).toBe(0);
  expect(computeVfxFrame(100, [100], 0, true)).toBe(0);
  expect(computeVfxFrame(100, [0, 0], 2, true)).toBe(0); // zero total duration
});
