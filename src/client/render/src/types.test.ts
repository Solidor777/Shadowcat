import { test, expect } from "vitest";
import type { Polygon } from "./index";

test("Polygon stores flat scene-coordinate pairs", () => {
  const p: Polygon = { points: [0, 0, 10, 0, 10, 10] };
  expect(p.points.length % 2).toBe(0);
  expect(p.points.length / 2).toBe(3); // three vertices
});
