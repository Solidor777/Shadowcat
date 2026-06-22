import { test, expect } from "vitest";
import { Camera } from "./index";

test("default camera is identity", () => {
  const c = new Camera();
  expect(c.transform()).toEqual({ x: 0, y: 0, scale: 1 });
  expect(c.screenToScene({ x: 10, y: 20 })).toEqual({ x: 10, y: 20 });
});

test("panBy translates the offset in screen space", () => {
  const c = new Camera();
  c.panBy(15, -5);
  expect(c.transform()).toMatchObject({ x: 15, y: -5, scale: 1 });
  // A screen point now maps to a scene point shifted by the pan.
  expect(c.screenToScene({ x: 15, y: -5 })).toEqual({ x: 0, y: 0 });
});

test("zoomAt holds the scene point under the cursor fixed", () => {
  const c = new Camera();
  const cursor = { x: 100, y: 100 };
  const sceneBefore = c.screenToScene(cursor);
  c.zoomAt(2, cursor.x, cursor.y);
  const sceneAfter = c.screenToScene(cursor);
  expect(c.transform().scale).toBeCloseTo(2);
  expect(sceneAfter.x).toBeCloseTo(sceneBefore.x);
  expect(sceneAfter.y).toBeCloseTo(sceneBefore.y);
});

test("scale is clamped to the [0.1, 10] range", () => {
  const c = new Camera();
  c.zoomAt(1000, 0, 0);
  expect(c.transform().scale).toBeLessThanOrEqual(10);
  c.zoomAt(0.00001, 0, 0);
  expect(c.transform().scale).toBeGreaterThanOrEqual(0.1);
});

test("sceneToScreen is the inverse of screenToScene", () => {
  const c = new Camera();
  c.panBy(30, 40);
  c.zoomAt(1.5, 50, 50);
  const s = { x: 12, y: 34 };
  const round = c.sceneToScreen(c.screenToScene(s));
  expect(round.x).toBeCloseTo(s.x);
  expect(round.y).toBeCloseTo(s.y);
});
