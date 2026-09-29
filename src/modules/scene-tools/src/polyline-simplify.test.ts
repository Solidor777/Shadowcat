// @vitest-environment node
// Exercises pure functions: no component render and no DOM API use.
import { describe, it, expect } from "vitest";
import { thinPolyline, type Polyline } from "./polyline-simplify";
import { countHypot, WORK_COUNTER_INERT } from "./__fixtures__/work-counter";

/** The cap the thinning tests use: the size of one shared-ruler frame. */
const MAX_POINTS = 256;

describe("thinPolyline", () => {
  it("returns a copy when under the cap", () => {
    const short: Polyline = [[0, 0], [1, 1]];
    const copy = thinPolyline(short, 256);
    expect(copy).toEqual(short);
    expect(copy).not.toBe(short);
  });

  it("drops collinear vertices first: a long straight run keeps only its endpoints", () => {
    const long: Polyline = Array.from({ length: 1000 }, (_, i) => [i, 0]);
    expect(thinPolyline(long, 10)).toEqual([[0, 0], [999, 0]]);
  });

  it("keeps every corner of a long routed path whose straight runs are thinned away", () => {
    // A grid route: 400 vertices east, 400 south, 400 west: three runs, two corners, 1200 vertices.
    const route: Polyline = [];
    for (let x = 0; x < 400; x++) route.push([x, 0]);
    for (let y = 0; y < 400; y++) route.push([400, y]);
    for (let x = 400; x >= 0; x--) route.push([x, 400]);
    const capped = thinPolyline(route, MAX_POINTS);
    expect(capped.length).toBeLessThanOrEqual(MAX_POINTS);
    expect(capped).toEqual([[0, 0], [400, 0], [400, 400], [0, 400]]);
  });

  it("when corners alone exceed the cap, removes the flattest ones and keeps the sharp ones", () => {
    // 300 vertices: four sharp corners of a square, the rest a barely-deflected wiggle along the edges.
    const route: Polyline = [];
    for (let i = 0; i < 75; i++) route.push([i * 10, i % 2 === 0 ? 0 : 0.01]);
    for (let i = 0; i < 75; i++) route.push([750, i * 10 + (i % 2 === 0 ? 0 : 0.01)]);
    for (let i = 0; i < 75; i++) route.push([750 - i * 10, 750 + (i % 2 === 0 ? 0 : 0.01)]);
    for (let i = 0; i < 75; i++) route.push([i % 2 === 0 ? 0 : 0.01, 750 - i * 10]);
    const capped = thinPolyline(route, 40);
    expect(capped).toHaveLength(40);
    expect(capped[0]).toEqual(route[0]);
    expect(capped[39]).toEqual(route[route.length - 1]);
    for (const corner of [route[75], route[150], route[225]]) {
      expect(capped.some((p) => Math.abs(p[0] - corner[0]) < 1 && Math.abs(p[1] - corner[1]) < 1)).toBe(true);
    }
  });

  it("a large overshoot survives thinning under cap pressure while small zigzag corners are removed", () => {
    // 301 vertices over a cap of 256: an overshoot [0,0] -> [1000,0] -> [500,0] whose middle vertex
    // deviates 500 from its chord (triangle area 0), a corner at [500,0] and a zigzag of 297
    // vertices, each deviating 3 from its chord, e.g. (508,110),(505,120),(508,130). Area alone
    // would remove the overshoot first; it must outlive every zigzag vertex.
    const route: Polyline = [[0, 0], [1000, 0], [500, 0], [500, 100]];
    for (let i = 0; i < 297; i++) route.push([i % 2 === 0 ? 508 : 505, 110 + i * 10]);
    const capped = thinPolyline(route, 256);
    expect(capped).toHaveLength(256);
    expect(capped.slice(0, 3)).toEqual([[0, 0], [1000, 0], [500, 0]]);
    expect(capped.at(-1)).toEqual(route.at(-1));
  });

  it("a near-reversal outside the collinearity tolerance is ranked by its overshoot, not its tiny area", () => {
    // [0,0] -> [1000,0] -> [500,0.001]: triangle area 0.5, but the vertex deviates 500 from its
    // chord.
    const route: Polyline = [[0, 0], [1000, 0], [500, 0.001], [500, 100]];
    for (let i = 0; i < 297; i++) route.push([i % 2 === 0 ? 508 : 505, 110 + i * 10]);
    const capped = thinPolyline(route, 256);
    expect(capped).toHaveLength(256);
    expect(capped[1]).toEqual([1000, 0]);
  });

  it("a 100-unit overshoot outranks zigzag corners that deviate 30", () => {
    // Overshoot [0,0] -> [100,0] -> [50,0]: the far vertex deviates 50 from its chord. The zigzag
    // corners deviate 30 each: (80,110),(50,120),(80,130).
    const route: Polyline = [[0, 0], [100, 0], [50, 0], [50, 100]];
    for (let i = 0; i < 297; i++) route.push([i % 2 === 0 ? 80 : 50, 110 + i * 10]);
    const capped = thinPolyline(route, 256);
    expect(capped).toHaveLength(256);
    expect(capped[1]).toEqual([100, 0]);
  });

  it("a route that goes out and returns to the same point keeps its far vertex under cap pressure", () => {
    // [0,0] -> [100,0] -> [0,0]: the neighbours of the far vertex coincide, so the chord has no
    // length and the triangle area is 0; the deviation is 100.
    const route: Polyline = [[0, 0], [100, 0], [0, 0], [0, 100]];
    for (let i = 0; i < 297; i++) route.push([i % 2 === 0 ? 30 : 0, 110 + i * 10]);
    const capped = thinPolyline(route, 256);
    expect(capped).toHaveLength(256);
    expect(capped[1]).toEqual([100, 0]);
  });

  it("a 1000-unit needle over a unit chord outranks jitter corners that deviate 60", () => {
    // [0,0],[0.5,1000],[1,0]: the needle tip's triangle area is only 500, below the jitter's 600,
    // but removing it deletes a 1000-unit excursion. Jitter: 298 zigzag corners deviating 60.
    const route: Polyline = [[0, 0], [0.5, 1000], [1, 0]];
    for (let i = 0; i < 298; i++) route.push([11 + i * 10, i % 2 === 0 ? 60 : 0]);
    const capped = thinPolyline(route, 256);
    expect(capped).toHaveLength(256);
    expect(capped[1]).toEqual([0.5, 1000]);
  });

  it("a hairpin whose return vertex is a hair off the start ranks like the exact hairpin", () => {
    const zigzag = (route: Polyline): Polyline => {
      for (let i = 0; i < 298; i++) route.push([11 + i * 10, i % 2 === 0 ? 60 : 0]);
      return route;
    };
    const exact = thinPolyline(zigzag([[0, 0], [0, 1000], [1, 0]]), 256);
    const perturbed = thinPolyline(zigzag([[0, 0], [0, 1000], [1e-9, -1e-9]]), 256);
    expect(exact[1]).toEqual([0, 1000]);
    expect(perturbed[1]).toEqual([0, 1000]);
  });

  it("removes a 10-unit deviation over a long chord before a 60-unit deviation over a short one", () => {
    // Y: [0,0],[5,60],[10,0] deviates 60 (triangle area 300). X: [10,0],[510,10],[1010,0] deviates
    // 10 (triangle area 5000 over its 1000-unit chord). One removal is forced (6 vertices, cap 5),
    // and it must be X, the smaller loss; an area ranking removes Y first.
    const route: Polyline = [[0, 0], [5, 60], [10, 0], [510, 10], [1010, 0], [1010, 1000]];
    expect(thinPolyline(route, 5)).toEqual([[0, 0], [5, 60], [10, 0], [1010, 0], [1010, 1000]]);
  });

  it("a large right-angle corner outranks many half-unit reversal spikes", () => {
    // One 1000 x 1000 corner, then 299 exact reversals of half a unit along a line: 302 vertices
    // over a cap of 256. Each spike deviates 0.5; the corner deviates about 707.
    const route: Polyline = [[0, 0], [1000, 0], [1000, 1000]];
    let x = 1000;
    for (let i = 0; i < 149; i++) {
      route.push([x + 1, 1000], [x + 0.5, 1000]);
      x += 0.5;
    }
    route.push([x + 1, 1000]);
    const capped = thinPolyline(route, 256);
    expect(capped).toHaveLength(256);
    expect(capped).toContainEqual([1000, 0]);
    expect(capped.at(-1)).toEqual(route.at(-1));
  });

  it("keeps the cap and both endpoints when a sawtooth of reversals plus a corner exceeds it", () => {
    const route: Polyline = [[0, 0]];
    for (let i = 0; i < 10; i++) route.push([i * 5 + 10, 0], [i * 5 + 5, 0]);
    route.push([50, 20], [60, 20]);
    const capped = thinPolyline(route, 8);
    expect(capped).toHaveLength(8);
    expect(capped[0]).toEqual(route[0]);
    expect(capped.at(-1)).toEqual(route.at(-1));
  });

  it("returns only the endpoints when the cap is below 2", () => {
    const route: Polyline = [[0, 0], [10, 0], [5, 0], [5, 5]];
    expect(thinPolyline(route, 1)).toEqual([[0, 0], [5, 5]]);
  });

  it("a spike that doubles back is not collinear and survives", () => {
    // 6 vertices, cap 5: the two flat vertices after the backtrack are dropped, the spike stays.
    const spike: Polyline = [[0, 0], [10, 0], [5, 0], [6, 0], [7, 0], [20, 0]];
    expect(thinPolyline(spike, 5)).toEqual([[0, 0], [10, 0], [5, 0], [20, 0]]);
  });

  it("thins a large route with deviation evaluations linear in the input, not in input times removals", () => {
    // Pins the NUMBER OF DEVIATION AND COLLINEARITY EVALUATIONS, not time, so the result is
    // identical on every machine and load. `countHypot` counts two `Math.hypot` calls per
    // collinearity check (`isCollinear`) and one per deviation (`chordDeviation`). A heap-based
    // pass needs one check and one deviation per vertex plus one deviation per neighbour re-score
    // per removal, about 5n; a pass that re-evaluates every vertex per removal needs about
    // n * (n - 256), ~16x more for 4x the input. It does NOT catch a linear min-scan over cached
    // deviations or splice-style removal from an array: neither adds evaluations. The floor below
    // is the positive control: it requires both primitives to contribute (2n from the checks
    // alone would not reach 3n), so the count reads too low if either stops calling `Math.hypot`.
    const route = (n: number): Polyline =>
      Array.from({ length: n }, (_, i) => [i * 3 + Math.sin(i) * 7, Math.cos(i * 0.37) * 50 + (i % 7)]);
    const work = (n: number): { calls: number; length: number } => {
      const pts = route(n);
      const { result, calls } = countHypot(() => thinPolyline(pts, 256));
      return { calls, length: result.length };
    };
    const small = work(2000);
    const large = work(8000);
    expect(small.length).toBe(MAX_POINTS);
    expect(large.length).toBe(MAX_POINTS);
    // Every vertex is collinearity-checked (2 calls) and scored (1 call) at least once, so 3n is a
    // floor; below it the counted primitive is no longer what the algorithm evaluates and the
    // upper bounds would pass with too little work counted.
    expect(small.calls, WORK_COUNTER_INERT).toBeGreaterThanOrEqual(3 * 2000);
    expect(large.calls, WORK_COUNTER_INERT).toBeGreaterThanOrEqual(3 * 8000);
    expect(large.calls).toBeLessThanOrEqual(5 * small.calls); // 4x the input: linear-ish, never ~16x
    expect(large.calls).toBeLessThanOrEqual(8 * 8000); // and a small constant per vertex in absolute terms
  });
});
