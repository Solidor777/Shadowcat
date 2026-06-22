import { test, expect } from "vitest";
import { Grid } from "./index";

test("square grid snaps to cell centers", () => {
  const g = new Grid({ kind: "square", size: 100 });
  expect(g.snap({ x: 140, y: 160 })).toEqual({ x: 150, y: 150 });
  expect(g.cellOf({ x: 250, y: 50 })).toEqual({ col: 2, row: 0 });
});

test("square grid lines cover the viewport rect", () => {
  const g = new Grid({ kind: "square", size: 100 });
  const lines = g.lines({ x: 0, y: 0, w: 300, h: 200 });
  // 4 verticals (x=0,100,200,300) + 3 horizontals (y=0,100,200).
  const verticals = lines.filter((l) => l.x1 === l.x2);
  const horizontals = lines.filter((l) => l.y1 === l.y2);
  expect(verticals.length).toBe(4);
  expect(horizontals.length).toBe(3);
});

test("hex snap round-trips: a snapped point snaps to itself", () => {
  const g = new Grid({ kind: "hex", size: 50 });
  const snapped = g.snap({ x: 137, y: 221 });
  const again = g.snap(snapped);
  expect(again.x).toBeCloseTo(snapped.x);
  expect(again.y).toBeCloseTo(snapped.y);
});

test("hex grid emits a non-empty line set over a viewport", () => {
  const g = new Grid({ kind: "hex", size: 50 });
  const lines = g.lines({ x: 0, y: 0, w: 400, h: 400 });
  expect(lines.length).toBeGreaterThan(0);
});
