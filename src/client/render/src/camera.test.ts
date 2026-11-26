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

const VIEW = { width: 800, height: 600 };
/** The scene point currently at the viewport center. */
const centerOf = (c: Camera) => c.screenToScene({ x: VIEW.width / 2, y: VIEW.height / 2 });

test("panTo_point_tweens_over_duration", () => {
  const c = new Camera();
  c.panTo({ x: 1000, y: 500 }, VIEW, { durationMs: 400, easing: "linear" });
  // The tween has not moved the camera until a tick advances it.
  expect(centerOf(c)).toEqual({ x: 400, y: 300 });
  expect(c.tick(200, VIEW)).toBe(true);
  expect(centerOf(c).x).toBeCloseTo(700);
  expect(centerOf(c).y).toBeCloseTo(400);
  expect(c.tick(200, VIEW)).toBe(true);
  expect(centerOf(c).x).toBeCloseTo(1000);
  expect(centerOf(c).y).toBeCloseTo(500);
  // Finished: further ticks neither move nor ask for a re-apply.
  expect(c.tick(16, VIEW)).toBe(false);
});

test("panTo applies the easing curve: easeInOut is behind linear at a quarter of the duration", () => {
  const linear = new Camera();
  linear.panTo({ x: 1000, y: 300 }, VIEW, { durationMs: 400, easing: "linear" });
  linear.tick(100, VIEW);
  const eased = new Camera();
  eased.panTo({ x: 1000, y: 300 }, VIEW, { durationMs: 400, easing: "easeInOut" });
  eased.tick(100, VIEW);
  expect(centerOf(eased).x).toBeLessThan(centerOf(linear).x);
});

test("panTo_token_tracks_live_position_each_frame", () => {
  const c = new Camera();
  let token = { x: 1000, y: 300 };
  c.panTo(() => token, VIEW, { durationMs: 400, easing: "linear" });
  c.tick(100, VIEW); // a quarter of the way toward the position at THIS frame
  expect(centerOf(c).x).toBeCloseTo(400 + (1000 - 400) * 0.25);
  token = { x: 2000, y: 300 }; // the subject moves mid-pan
  c.tick(300, VIEW); // the pan ends on where the subject is now, not where it was at the start
  expect(centerOf(c).x).toBeCloseTo(2000);
});

test("panTo_respects_reduced_motion_by_snapping_instantly", () => {
  const c = new Camera();
  c.panTo({ x: 1000, y: 500 }, VIEW, { durationMs: 400, reducedMotion: true });
  expect(centerOf(c)).toEqual({ x: 1000, y: 500 });
  expect(c.tick(16, VIEW)).toBe(false); // no tween was started
});

test("panTo centers at the current zoom without changing it", () => {
  const c = new Camera();
  c.zoomAt(2, 0, 0);
  c.panTo({ x: 100, y: 50 }, VIEW, { reducedMotion: true });
  expect(c.transform().scale).toBeCloseTo(2);
  expect(centerOf(c).x).toBeCloseTo(100);
  expect(centerOf(c).y).toBeCloseTo(50);
});

test("a manual panBy or zoomAt cancels an in-flight pan", () => {
  const byPan = new Camera();
  byPan.panTo({ x: 1000, y: 500 }, VIEW, { durationMs: 400 });
  byPan.panBy(5, 5);
  expect(byPan.tick(100, VIEW)).toBe(false);
  const byZoom = new Camera();
  byZoom.panTo({ x: 1000, y: 500 }, VIEW, { durationMs: 400 });
  byZoom.zoomAt(1.5, 10, 10);
  expect(byZoom.tick(100, VIEW)).toBe(false);
});

test("a provider that reports its subject gone drops the pan; an unresolvable target is a no-op", () => {
  const c = new Camera();
  let present: { x: number; y: number } | undefined = { x: 1000, y: 500 };
  c.panTo(() => present, VIEW, { durationMs: 400 });
  present = undefined;
  expect(c.tick(100, VIEW)).toBe(false);
  expect(c.tick(100, VIEW)).toBe(false); // dropped, not retried
  const unresolved = new Camera();
  unresolved.panTo(() => undefined, VIEW, { reducedMotion: true });
  expect(unresolved.transform()).toEqual({ x: 0, y: 0, scale: 1 });
});
