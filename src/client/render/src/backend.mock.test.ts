import { test, expect } from "vitest";
import { MockBackend } from "./index";

test("MockBackend records token upserts and removals", () => {
  const b = new MockBackend();
  b.setToken("t1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, url: "/a" });
  expect(b.tokens.get("t1")).toEqual({ x: 0, y: 0, w: 100, h: 100, rotation: 0, url: "/a" });
  b.setToken("t1", { x: 10, y: 0, w: 100, h: 100, rotation: 0, url: "/a" });
  expect(b.tokens.get("t1")!.x).toBe(10);
  b.removeToken("t1");
  expect(b.tokens.has("t1")).toBe(false);
});

test("MockBackend captures the ticker callback", () => {
  const b = new MockBackend();
  let dt = 0;
  b.startTicker((d) => { dt = d; });
  b.tick!(16);
  expect(dt).toBe(16);
});
