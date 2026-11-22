import { test, expect } from "vitest";
import { rectPoints, ellipsePoints, circlePoints, conePoints, squarePoints, linePoints, DEFAULT_CONE_ANGLE_DEGREES } from "./geometry";

const dist = (x: number, y: number): number => Math.hypot(x, y);

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

test("conePoints_default_angle_matches_existing_sixty_degree_output", () => {
  expect(DEFAULT_CONE_ANGLE_DEGREES).toBe(60);
  // Independently derived expected geometry (not by calling conePoints a second time — a
  // consistently-wrong triangle formula would satisfy a self-comparison, which is why this pin
  // uses literal coordinates instead): apex (0,0), size 10, facing +x, 60° full aperture puts
  // the base corners at ±30° from +x. cos(30°) = √3/2, sin(30°) = 1/2, so the corners land at
  // (10·√3/2, -10·1/2) = (5√3, -5) and (5√3, 5).
  const expected = [0, 0, 5 * Math.sqrt(3), -5, 5 * Math.sqrt(3), 5];
  const p = conePoints(0, 0, 10, 0); // omitted 5th argument: must use the default aperture
  expect(p).toHaveLength(6);
  for (let i = 0; i < p.length; i++) expect(p[i]).toBeCloseTo(expected[i]);
  // Also pins that omitting the argument is equivalent to passing the named constant/its
  // literal value explicitly — a fact independent of whether the geometry itself is correct.
  expect(p).toEqual(conePoints(0, 0, 10, 0, DEFAULT_CONE_ANGLE_DEGREES));
  expect(p).toEqual(conePoints(0, 0, 10, 0, 60));
});

test("conePoints_custom_angle_widens_or_narrows_the_cone", () => {
  const narrow = conePoints(0, 0, 10, 0, 20);
  const wide = conePoints(0, 0, 10, 0, 120);
  // A wider aperture pushes the base corners further apart (larger |y| at facing 0°).
  expect(Math.abs(wide[3])).toBeGreaterThan(Math.abs(narrow[3]));
  expect(Math.abs(wide[5])).toBeGreaterThan(Math.abs(narrow[5]));
});

test("line_tessellation_zero_width_matches_existing_output", () => {
  const legacy = [0, 0, 10, 0];
  expect(linePoints(0, 0, 10, 0)).toEqual(legacy);
  expect(linePoints(0, 0, 10, 0, 0)).toEqual(legacy);
});

test("line_tessellation_nonzero_width_produces_a_rectangle", () => {
  const p = linePoints(0, 0, 10, 0, 4);
  expect(p).toHaveLength(8); // 4 corners
  // Facing +x with thickness 4: the two long edges sit at y = ±2.
  expect(p[1]).toBeCloseTo(2);
  expect(p[3]).toBeCloseTo(2);
  expect(p[5]).toBeCloseTo(-2);
  expect(p[7]).toBeCloseTo(-2);
});

test("rect_tessellation_width_equal_to_size_matches_existing_square_output", () => {
  expect(squarePoints(0, 0, 5, 0, 5)).toEqual(squarePoints(0, 0, 5, 0));
  expect(squarePoints(0, 0, 5, 0, 5)).toEqual([-5, -5, 5, -5, 5, 5, -5, 5]);
});

test("rect_tessellation_distinct_width_produces_a_non_square_rectangle", () => {
  const r = squarePoints(0, 0, 5, 0, 2);
  expect(r).toEqual([-5, -2, 5, -2, 5, 2, -5, 2]);
  expect(r).not.toEqual(squarePoints(0, 0, 5, 0));
});
