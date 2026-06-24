import { test, expect } from "vitest";
import { MockBackend } from "./index";

test("MockBackend records token upserts and removals", () => {
  const b = new MockBackend();
  b.setToken("t1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, url: "/a", borderColor: null });
  expect(b.tokens.get("t1")).toEqual({ x: 0, y: 0, w: 100, h: 100, rotation: 0, url: "/a", borderColor: null });
  b.setToken("t1", { x: 10, y: 0, w: 100, h: 100, rotation: 0, url: "/a", borderColor: null });
  expect(b.tokens.get("t1")!.x).toBe(10);
  b.removeToken("t1");
  expect(b.tokens.has("t1")).toBe(false);
});

test("MockBackend records shape upserts/removals and the ephemeral overlay", () => {
  const b = new MockBackend();
  const spec = { layer: "drawings", points: [0, 0, 10, 0], closed: false, stroke: { color: 0xff0000, width: 2 }, fill: null };
  b.setShape("s1", spec);
  expect(b.shapes.get("s1")).toEqual(spec);
  b.setShape("s1", { ...spec, points: [0, 0, 20, 0] });
  expect(b.shapes.get("s1")!.points).toEqual([0, 0, 20, 0]);
  b.removeShape("s1");
  expect(b.shapes.has("s1")).toBe(false);

  b.drawOverlay([{ points: [0, 0, 5, 5], closed: false, stroke: { color: 0, width: 1 }, fill: null }]);
  expect(b.overlay).toHaveLength(1);
  b.clearOverlay();
  expect(b.overlay).toHaveLength(0);
});

test("MockBackend records the measurement overlay", () => {
  const b = new MockBackend();
  b.drawMeasure({ x: 0, y: 0 }, { x: 10, y: 0 }, "1");
  expect(b.measure).toEqual({ from: { x: 0, y: 0 }, to: { x: 10, y: 0 }, label: "1" });
  b.clearMeasure();
  expect(b.measure).toBeNull();
});

test("MockBackend captures the ticker callback", () => {
  const b = new MockBackend();
  let dt = 0;
  b.startTicker((d) => { dt = d; });
  b.tick!(16);
  expect(dt).toBe(16);
});
