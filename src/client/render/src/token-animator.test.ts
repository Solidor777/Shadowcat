import { test, expect } from "vitest";
import { TokenAnimator } from "./index";

test("a new token snaps to its target (appears in place, no tween)", () => {
  const a = new TokenAnimator();
  a.setTarget("t1", { x: 100, y: 50, rotation: 0 });
  expect(a.get("t1")).toEqual({ x: 100, y: 50, rotation: 0 });
  expect(a.tick(16)).toEqual([]); // already at target → nothing moves
});

test("a moved token tweens toward the new target and settles", () => {
  const a = new TokenAnimator();
  a.setTarget("t1", { x: 0, y: 0, rotation: 0 }); // initial (snap)
  a.setTarget("t1", { x: 100, y: 0, rotation: 0 }); // move
  const moved = a.tick(60); // partial advance
  expect(moved).toEqual(["t1"]);
  const mid = a.get("t1")!;
  expect(mid.x).toBeGreaterThan(0);
  expect(mid.x).toBeLessThan(100);
  a.tick(10_000); // a long tick fully settles
  expect(a.get("t1")).toEqual({ x: 100, y: 0, rotation: 0 });
});

test("remove drops the tween", () => {
  const a = new TokenAnimator();
  a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
  a.remove("t1");
  expect(a.has("t1")).toBe(false);
  expect(a.get("t1")).toBeUndefined();
});
