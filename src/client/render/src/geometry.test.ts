import { test, expect } from "vitest";
import { parseColor, rectPoints, ellipsePoints, circlePoints, conePoints, squarePoints } from "./geometry";

const dist = (x: number, y: number): number => Math.hypot(x, y);

test("parseColor reads #rrggbb (with or without #); malformed → 0", () => {
  expect(parseColor("#ff8000")).toBe(0xff8000);
  expect(parseColor("ff8000")).toBe(0xff8000);
  expect(parseColor("#000000")).toBe(0);
  expect(parseColor("nope")).toBe(0);
  expect(parseColor("#fff")).toBe(0); // short form not supported
});

test("rectPoints returns the four corners from opposite corners", () => {
  expect(rectPoints(0, 0, 10, 20)).toEqual([0, 0, 10, 0, 10, 20, 0, 20]);
});

test("circlePoints yields `2*segments` numbers all at radius r", () => {
  const p = circlePoints(0, 0, 5, 8);
  expect(p).toHaveLength(16);
  for (let i = 0; i < p.length; i += 2) expect(dist(p[i], p[i + 1])).toBeCloseTo(5);
});

test("ellipsePoints inscribes the bounding box", () => {
  const p = ellipsePoints(-10, -4, 10, 4, 8); // rx 10, ry 4, centered origin
  expect(p).toHaveLength(16);
  // angle 0 → (rx, 0); angle 90° (i=2 of 8) → (0, ry)
  expect(p[0]).toBeCloseTo(10);
  expect(p[1]).toBeCloseTo(0);
});

test("conePoints is an isoceles triangle: apex + two corners at distance size", () => {
  const p = conePoints(0, 0, 10, 0, 60); // facing +x, 60° aperture
  expect(p.slice(0, 2)).toEqual([0, 0]); // apex
  expect(dist(p[2], p[3])).toBeCloseTo(10);
  expect(dist(p[4], p[5])).toBeCloseTo(10);
  // ±30° around +x: x ≈ 8.66, y = ∓5
  expect(p[2]).toBeCloseTo(Math.cos((-30 * Math.PI) / 180) * 10);
  expect(p[3]).toBeCloseTo(Math.sin((-30 * Math.PI) / 180) * 10);
});

test("squarePoints rotates a centered square", () => {
  expect(squarePoints(0, 0, 5, 0)).toEqual([-5, -5, 5, -5, 5, 5, -5, 5]);
  const r = squarePoints(0, 0, 5, 90); // 90° rotation maps (-5,-5)→(5,-5)
  expect(r[0]).toBeCloseTo(5);
  expect(r[1]).toBeCloseTo(-5);
});
