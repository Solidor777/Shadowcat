import { test, expect } from "vitest";
import { PingView } from "./index";

test("a ping expands and fades over its lifetime, then drops", () => {
  const v = new PingView();
  v.add(10, 20);
  let rings = v.tick(0);
  expect(rings).toHaveLength(1);
  expect(rings[0]).toMatchObject({ x: 10, y: 20 });
  expect(rings[0].alpha).toBeCloseTo(1);
  expect(rings[0].radius).toBeCloseTo(0);

  rings = v.tick(1000); // half life
  expect(rings[0].alpha).toBeCloseTo(0.5);
  expect(rings[0].radius).toBeGreaterThan(0);

  rings = v.tick(2000); // total age 3000 > lifetime → dropped
  expect(rings).toHaveLength(0);
});

test("multiple pings animate independently", () => {
  const v = new PingView();
  v.add(0, 0);
  v.tick(1500);
  v.add(5, 5); // a fresh ping while the first is mid-fade
  const rings = v.tick(100);
  expect(rings).toHaveLength(2);
});
