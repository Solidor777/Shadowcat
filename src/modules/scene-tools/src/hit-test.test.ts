import { test, expect } from "vitest";
import { buildTokenDoc, type WireDocument } from "@shadowcat/core";
import { topTokenAt } from "./hit-test";

function tok(id: string, x: number, y: number, w = 100, h = 100): WireDocument {
  return buildTokenDoc("w1", "s1", { x, y, w, h, rotation: 0, visual: { kind: "image", asset: "a" } }, id);
}

test("picks the topmost (last in order) of overlapping tokens", () => {
  expect(topTokenAt([tok("a", 0, 0), tok("b", 0, 0)], { x: 0, y: 0 })).toBe("b");
});

test("returns null when the point is outside every token", () => {
  expect(topTokenAt([tok("a", 0, 0)], { x: 200, y: 0 })).toBeNull();
});

test("hit box is the center ± half-extent", () => {
  const a = [tok("a", 0, 0, 100, 100)];
  expect(topTokenAt(a, { x: 50, y: 50 })).toBe("a"); // on the corner → inside
  expect(topTokenAt(a, { x: 51, y: 0 })).toBeNull(); // just past the edge
});
