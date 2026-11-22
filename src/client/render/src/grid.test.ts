import { readFileSync } from "node:fs";
import { test, expect } from "vitest";
import { Grid, type DiagonalRule, type GridSpec } from "./index";

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

test("approximate costs a diagonal at 1.5x an orthogonal step", () => {
  const g = makeGrid({ kind: "square", size: 100, diagonalRule: "approximate" });
  // Pure diagonal: 2 steps at 1.5 → 3.
  expect(g.distance({ x: 50, y: 50 }, { x: 250, y: 250 })).toBe(3);
  // Mixed (dCol=4, dRow=2): 2 diagonals (3) + 2 orthogonals (2) → 5.
  expect(g.distance({ x: 50, y: 50 }, { x: 450, y: 250 })).toBe(5);
  // Adjacency is unchanged from the king-move rules — unlike orthogonal below.
  expect(g.distance({ x: 50, y: 50 }, { x: 150, y: 150 })).toBe(1.5);
});

test("orthogonal prices a diagonal displacement as the Manhattan figure", () => {
  const g = makeGrid({ kind: "square", size: 100, diagonalRule: "orthogonal" });
  // Diagonal steps are illegal under this rule, so (2,2) costs 4 (two dog-leg
  // pairs), not any diagonal shortcut — and this figure is EXACT for the
  // 4-directional graph, not a bound.
  expect(g.distance({ x: 50, y: 50 }, { x: 250, y: 250 })).toBe(4);
  expect(g.distance({ x: 50, y: 50 }, { x: 450, y: 250 })).toBe(6);
});

// Cross-language parity pin. Both this corpus read and the server's
// `diagonal_cost_matches_the_shared_client_corpus` assert EVERY case of the same file —
// `Grid.distance` here, `astar_leg` there — so a per-rule cost drift on either side fails
// its own comparison against the corpus. The corpus covers all six rules (not only the two
// newest) so an edit to an EXISTING rule's pricing is caught the same way.
test("Grid.distance matches the shared diagonal-cost corpus", () => {
  const raw = JSON.parse(
    readFileSync(new URL("./__fixtures__/diagonal-cost-conformance.json", import.meta.url), "utf8"),
  ) as { cases: { name: string; rule: DiagonalRule; dCol: number; dRow: number; expect: number }[] };
  expect(raw.cases.length).toBe(6 * 8);
  const seen = new Set<string>();
  for (const c of raw.cases) {
    expect(seen.has(c.name), `duplicate case name ${c.name}`).toBe(false);
    seen.add(c.name);
    const g = makeGrid({ kind: "square", size: 100, diagonalRule: c.rule });
    // Cell centers of (0,0) and (dCol,dRow), so `cellOf` assigns the offset exactly.
    const got = g.distance({ x: 50, y: 50 }, { x: 50 + c.dCol * 100, y: 50 + c.dRow * 100 });
    expect(got, `case "${c.name}" (${c.rule} +${c.dCol},+${c.dRow})`).toBeCloseTo(c.expect, 9);
  }
});

test("square cellCenter/cellVertices: axis-aligned rect anchored at (col*size, row*size)", () => {
  const g = new Grid({ kind: "square", size: 100 });
  expect(g.cellCenter(2, 0)).toEqual({ x: 250, y: 50 });
  expect(g.cellVertices(0, 0)).toEqual([
    { x: 0, y: 0 }, { x: 100, y: 0 }, { x: 100, y: 100 }, { x: 0, y: 100 },
  ]);
  expect(g.cellVertices(1, 0)).toEqual([
    { x: 100, y: 0 }, { x: 200, y: 0 }, { x: 200, y: 100 }, { x: 100, y: 100 },
  ]);
});

test("square cellVertices' centroid equals cellCenter for every cell", () => {
  const g = new Grid({ kind: "square", size: 70 });
  for (const [col, row] of [[0, 0], [3, -2], [-4, 5]] as const) {
    const corners = g.cellVertices(col, row);
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

test("hex cellVertices: 6 points, centroid equals cellCenter, every corner exactly `size` from center", () => {
  const g = new Grid({ kind: "hex", size: 50 });
  for (const [q, r] of [[0, 0], [1, 1], [-2, 3]] as const) {
    const corners = g.cellVertices(q, r);
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
// given hex is exactly `cellVertices`'s own output (in the same order), not merely
// geometrically equivalent.
test("hexLines' outline for a cell matches cellVertices exactly (single formula, not a duplicate)", () => {
  const g = new Grid({ kind: "hex", size: 50 });
  const corners = g.cellVertices(0, 0); // the (0,0) hex, centered inside a viewport starting at origin
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

test("snapVertex resolves to the nearest square vertex across a cell boundary", () => {
  const g = new Grid({ kind: "square", size: 100 });
  // (140,160) falls in cell (1,1); its nearest vertex (100,200) is shared with cells
  // (0,1)/(0,2)/(1,2) — the corner four cells meet at, not a corner unique to the
  // containing cell. `snap` on the same point resolves the cell CENTER (150,150) instead.
  expect(g.snapVertex({ x: 140, y: 160 })).toEqual({ x: 100, y: 200 });
  expect(g.snap({ x: 140, y: 160 })).toEqual({ x: 150, y: 150 });
  // Just across the boundary in the other direction: cell (0,0)'s side of the same corner.
  expect(g.snapVertex({ x: 95, y: 95 })).toEqual({ x: 100, y: 100 });
});

test("snapVertex resolves to the nearest hex vertex", () => {
  const g = new Grid({ kind: "hex", size: 100 });
  // Hex (0,0) is centered on the origin; its pointy-top vertices sit at angles 60i−30°
  // around it, so (0,-100) is the top vertex and (86.6025…,-50) the top-right one.
  const top = g.snapVertex({ x: 5, y: -95 });
  expect(top.x).toBeCloseTo(0, 9);
  expect(top.y).toBeCloseTo(-100, 9);
  const topRight = g.snapVertex({ x: 80, y: -45 });
  expect(topRight.x).toBeCloseTo(100 * Math.cos(-Math.PI / 6), 9);
  expect(topRight.y).toBeCloseTo(100 * Math.sin(-Math.PI / 6), 9);
  // The result is `cellVertices`'s own corner up to float dust — the same single corner
  // formula `hexLines` draws with, never a second vertex computation. Not bitwise: a shared
  // corner is recomputed per adjoining cell (`center + size·cos/sin`), and those copies can
  // differ in the last ulp.
  const corner = g.cellVertices(0, 0)[0];
  expect(Math.abs(topRight.x - corner.x)).toBeLessThan(1e-9);
  expect(Math.abs(topRight.y - corner.y)).toBeLessThan(1e-9);
});

test("snapVertex breaks exact ties deterministically (lowest x, then lowest y)", () => {
  const g = new Grid({ kind: "square", size: 100 });
  // The exact center of cell (0,0) is equidistant to all four corners (the squared
  // distances are bitwise equal: 50²+50² exactly), so the outcome must come from the
  // documented tie-break — lowest x, then lowest y — never from candidate enumeration order.
  expect(g.snapVertex({ x: 50, y: 50 })).toEqual({ x: 0, y: 0 });
  // A point on the shared edge midpoint ties between the two corners of that edge.
  expect(g.snapVertex({ x: 100, y: 50 })).toEqual({ x: 100, y: 0 });
});

test("snapEdge resolves to the nearest point on a cell edge, mid-span rather than at a corner", () => {
  const g = new Grid({ kind: "square", size: 100 });
  // Near the top edge's mid-span: the edge point directly above, where snapVertex resolves a corner.
  expect(g.snapEdge({ x: 30, y: 4 })).toEqual({ x: 30, y: 0 });
  expect(g.snapVertex({ x: 30, y: 4 })).toEqual({ x: 0, y: 0 });
  // Nearer the lower edge (30) than the left edge (40): the edge point below, not a corner.
  expect(g.snapEdge({ x: 140, y: 170 })).toEqual({ x: 140, y: 200 });
});

test("snapEdge resolves a corner point to that corner and a boundary point across cells", () => {
  const g = new Grid({ kind: "square", size: 100 });
  // A point exactly on a corner resolves to that corner (both flanking edges clamp to it).
  expect(g.snapEdge({ x: 100, y: 100 })).toEqual({ x: 100, y: 100 });
  // (95,50) is in cell (0,0); the nearest edge is the x=100 edge it shares with cell (1,0).
  expect(g.snapEdge({ x: 95, y: 50 })).toEqual({ x: 100, y: 50 });
  // A point in a negative-coordinate cell resolves on that cell's own edge.
  expect(g.snapEdge({ x: -30, y: -40 })).toEqual({ x: 0, y: -40 });
});

test("snapEdge breaks exact ties deterministically (lowest x, then lowest y)", () => {
  const g = new Grid({ kind: "square", size: 100 });
  // A cell's exact center is equidistant (50) to all four edges: the tie-break picks the
  // left edge's point (0,50), lowest x, never candidate enumeration order.
  expect(g.snapEdge({ x: 50, y: 50 })).toEqual({ x: 0, y: 50 });
});

test("snapEdge resolves to the nearest hex edge point", () => {
  const g = new Grid({ kind: "hex", size: 100 });
  // Hex (0,0)'s top-right edge runs (0,-100) to (86.6,-50); its midpoint is (43.3,-75) and its
  // outward normal points at -60 degrees. A point 10 units inside the midpoint snaps back to it.
  const mid = { x: (100 * Math.cos(-Math.PI / 6)) / 2, y: -75 };
  const inside = { x: mid.x - 10 * Math.cos(-Math.PI / 3), y: mid.y - 10 * Math.sin(-Math.PI / 3) };
  const snapped = g.snapEdge(inside);
  expect(snapped.x).toBeCloseTo(mid.x, 9);
  expect(snapped.y).toBeCloseTo(mid.y, 9);
});
