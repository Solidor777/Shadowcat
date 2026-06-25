import { test, expect } from "vitest";
import { Grid, type GridSpec } from "./index";

/** Convenience factory so tests can pass a partial spec without repeating defaults. */
function makeGrid(spec: GridSpec): Grid {
  return new Grid(spec);
}

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

test("square grid lines are deterministic for non-integer (panned/zoomed) rects", () => {
  const g = new Grid({ kind: "square", size: 100 });
  // A rect with fractional origin/extent, as screenToScene produces under pan/zoom.
  const lines = g.lines({ x: 10.3, y: 5.7, w: 280.4, h: 190.2 });
  const verticals = lines.filter((l) => l.x1 === l.x2).map((l) => l.x1).sort((a, b) => a - b);
  const horizontals = lines.filter((l) => l.y1 === l.y2).map((l) => l.y1).sort((a, b) => a - b);
  // floor(10.3/100)=0 .. ceil(290.7/100)=3 → x at 0,100,200,300 (no FP-dropped edge).
  expect(verticals).toEqual([0, 100, 200, 300]);
  // floor(5.7/100)=0 .. ceil(195.9/100)=2 → y at 0,100,200.
  expect(horizontals).toEqual([0, 100, 200]);
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

test("square distance is Chebyshev in whole cells", () => {
  const g = new Grid({ kind: "square", size: 100 });
  expect(g.distance({ x: 0, y: 0 }, { x: 250, y: 40 })).toBe(2); // cols 0→2, rows 0→0
  expect(g.distance({ x: 0, y: 0 }, { x: 250, y: 250 })).toBe(2); // diagonal → max(2,2)
  expect(g.distance({ x: 10, y: 10 }, { x: 10, y: 10 })).toBe(0);
});

test("hex distance is axial distance in whole cells", () => {
  const g = new Grid({ kind: "hex", size: 10 });
  const neighbor = { x: Math.sqrt(3) * 10, y: 0 }; // center of axial (1,0)
  expect(g.distance({ x: 0, y: 0 }, neighbor)).toBe(1);
  expect(g.distance({ x: 0, y: 0 }, { x: 0, y: 0 })).toBe(0);
});

test("alternating (5-10-5) costs diagonals 1,2,1,2 for square grids", () => {
  const g = makeGrid({ kind: "square", size: 100, diagonalRule: "alternating" });
  // 3 diagonal steps from origin: 1 + 2 + 1 = 4.
  expect(g.distance({ x: 50, y: 50 }, { x: 350, y: 350 })).toBe(4);
  // 1 diagonal + 1 orthogonal: diagonal(1) + orth(1) = 2.
  expect(g.distance({ x: 50, y: 50 }, { x: 250, y: 150 })).toBe(2);
});

test("chebyshev remains 1-per-diagonal (default)", () => {
  const g = makeGrid({ kind: "square", size: 100, diagonalRule: "chebyshev" });
  expect(g.distance({ x: 50, y: 50 }, { x: 350, y: 350 })).toBe(3);
});
