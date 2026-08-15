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

// Self-consistency pin. `TokenAnimator` must use the per-step distance on hex, not the outer
// radius — the two differ by a factor of √3. This measures the real
// distance between two adjacent hex centers via `snap` (which round-trips through the production
// `axialToPixel`, the same call `worldUnitsPerCell` itself must agree with) and derives its
// expectation from that measured geometry rather than restating the `size * sqrt(3)` formula — but
// the oracle is derived from `Grid`'s OWN geometry, so a formula change that updates
// `axialToPixel`/`worldUnitsPerCell` together stays self-consistent and passes this test even if it
// drifts from the server's convention. This test alone is NOT a cross-language parity pin; see the
// literal-value pin below for that.
test("hex per-step distance (worldUnitsPerCell) equals the measured distance between adjacent cell centers", () => {
  const g = new Grid({ kind: "hex", size: 50 });
  // Snap the origin to its hex's center, then snap a point known to fall inside the (1,0)
  // neighbor to that hex's center — both via the real production round-trip, not a restated
  // formula.
  const origin = g.snap({ x: 0, y: 0 });
  const neighbor = g.snap({ x: 90, y: 0 }); // inside the (1,0) hex for size 50
  const measured = Math.hypot(neighbor.x - origin.x, neighbor.y - origin.y);
  expect(g.worldUnitsPerCell()).toBeCloseTo(measured, 9);
});

// Cross-language parity pin. Neither side can share a symbol across the Rust/TS boundary, so both
// this test and the server's `hex_world_units_per_cell_matches_the_cross_language_pinned_literal`
// sibling state the SAME literal value (`50 * sqrt(3)`) independently, rather
// than each deriving its expectation from its own implementation as the self-consistency test
// above does. A literal is not a decision-maker two sites can disagree over — it is the ground
// truth both sides are checked against — so a formula change on EITHER side alone now fails that
// side's own comparison against this literal, where two self-derived oracles could update in
// lockstep and silently diverge from the other language.
test("hex per-step distance (worldUnitsPerCell) equals the cross-language pinned literal", () => {
  const g = new Grid({ kind: "hex", size: 50 });
  expect(g.worldUnitsPerCell()).toBeCloseTo(86.60254037844386, 9);
});

// `pixelToAxial`'s q mixes x and y with opposite signs, so its extrema sit on the
// top-right/bottom-left diagonal. Sampling only the other diagonal understated the
// q-range and left hexes CENTERED INSIDE the viewport undrawn (50 of them at
// 1920x1080/size=50). Asserting line COUNT alone cannot catch that — this walks the
// emitted outlines and checks every in-viewport hex center is actually covered.
test("hex grid covers every hex centered inside the viewport", () => {
  for (const [w, h, size] of [
    [1920, 1080, 50],
    [800, 600, 40],
    [1000, 1000, 50],
  ] as const) {
    const g = new Grid({ kind: "hex", size });
    const lines = g.lines({ x: 0, y: 0, w, h });
    // A drawn hex contributes six segments around its center; collect centers as the
    // centroid of each consecutive 6-segment group.
    const drawn = new Set<string>();
    for (let i = 0; i + 5 < lines.length; i += 6) {
      let cx = 0;
      let cy = 0;
      for (let k = 0; k < 6; k++) {
        cx += lines[i + k].x1;
        cy += lines[i + k].y1;
      }
      drawn.add(`${Math.round(cx / 6)},${Math.round(cy / 6)}`);
    }
    // Every hex whose center lies within the viewport must have been emitted.
    for (let q = -60; q <= 60; q++) {
      for (let r = -60; r <= 60; r++) {
        const cx = size * (Math.sqrt(3) * q + (Math.sqrt(3) / 2) * r);
        const cy = size * (1.5 * r);
        if (cx < 0 || cx > w || cy < 0 || cy > h) continue;
        expect(drawn.has(`${Math.round(cx)},${Math.round(cy)}`)).toBe(true);
      }
    }
  }
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

test("alternating (5-10-5) rule: two diagonal steps (dmin=2) cost 3, not 2", () => {
  const g = makeGrid({ kind: "square", size: 100, diagonalRule: "alternating" });
  // dmin=dmax=2 → (dmax-dmin) + dmin + floor(dmin/2) = 0 + 2 + 1 = 3.
  // Costing this as dmin*1=2 (ignoring the 1,2,1,2… alternation) would be wrong.
  expect(g.distance({ x: 50, y: 50 }, { x: 250, y: 250 })).toBe(3);
});

test("square cellCenter/cellCorners: axis-aligned rect anchored at (col*size, row*size)", () => {
  const g = new Grid({ kind: "square", size: 100 });
  expect(g.cellCenter(2, 0)).toEqual({ x: 250, y: 50 });
  expect(g.cellCorners(0, 0)).toEqual([
    { x: 0, y: 0 }, { x: 100, y: 0 }, { x: 100, y: 100 }, { x: 0, y: 100 },
  ]);
  expect(g.cellCorners(1, 0)).toEqual([
    { x: 100, y: 0 }, { x: 200, y: 0 }, { x: 200, y: 100 }, { x: 100, y: 100 },
  ]);
});

test("square cellCorners' centroid equals cellCenter for every cell", () => {
  const g = new Grid({ kind: "square", size: 70 });
  for (const [col, row] of [[0, 0], [3, -2], [-4, 5]] as const) {
    const corners = g.cellCorners(col, row);
    const cx = corners.reduce((s, p) => s + p.x, 0) / corners.length;
    const cy = corners.reduce((s, p) => s + p.y, 0) / corners.length;
    const center = g.cellCenter(col, row);
    expect(cx).toBeCloseTo(center.x, 9);
    expect(cy).toBeCloseTo(center.y, 9);
  }
});

// Hex cellCenter must equal the axial-to-pixel formula (the same math `snap`'s hex branch and
// `hexLines` already use) — this is the ground truth `cellsToRects`/`toLighting` now paint
// against instead of a square `i*size, j*size` position.
test("hex cellCenter matches the axial-to-pixel formula", () => {
  const g = new Grid({ kind: "hex", size: 100 });
  const c = g.cellCenter(1, 1);
  expect(c.x).toBeCloseTo(100 * (Math.sqrt(3) * 1 + (Math.sqrt(3) / 2) * 1), 9);
  expect(c.y).toBeCloseTo(100 * 1.5 * 1, 9);
});

test("hex cellCorners: 6 points, centroid equals cellCenter, every corner exactly `size` from center", () => {
  const g = new Grid({ kind: "hex", size: 50 });
  for (const [q, r] of [[0, 0], [1, 1], [-2, 3]] as const) {
    const corners = g.cellCorners(q, r);
    expect(corners.length).toBe(6);
    const center = g.cellCenter(q, r);
    const cx = corners.reduce((s, p) => s + p.x, 0) / 6;
    const cy = corners.reduce((s, p) => s + p.y, 0) / 6;
    expect(cx).toBeCloseTo(center.x, 9);
    expect(cy).toBeCloseTo(center.y, 9);
    for (const p of corners) {
      expect(Math.hypot(p.x - center.x, p.y - center.y)).toBeCloseTo(50, 9);
    }
  }
});

// `hexLines` must not carry a second hex-corner formula — pins that its emitted outline for a
// given hex is exactly `cellCorners`'s own output (in the same order), not merely
// geometrically equivalent.
test("hexLines' outline for a cell matches cellCorners exactly (single formula, not a duplicate)", () => {
  const g = new Grid({ kind: "hex", size: 50 });
  const corners = g.cellCorners(0, 0); // the (0,0) hex, centered inside a viewport starting at origin
  const lines = g.lines({ x: 0, y: 0, w: 10, h: 10 }); // tiny rect margined to still cover (0,0)
  // Collect the 6-segment outline whose first point matches corners[0].
  const outline = [];
  for (let i = 0; i + 5 < lines.length; i += 6) {
    if (Math.abs(lines[i].x1 - corners[0].x) < 1e-9 && Math.abs(lines[i].y1 - corners[0].y) < 1e-9) {
      for (let k = 0; k < 6; k++) outline.push({ x: lines[i + k].x1, y: lines[i + k].y1 });
      break;
    }
  }
  expect(outline).toEqual(corners);
});
