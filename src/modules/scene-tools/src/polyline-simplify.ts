// Pure polyline thinning: greedy minimum-deviation vertex removal. No DOM, no framework state, no
// dependency on the shared-ruler protocol.

/** One vertex list in scene coordinates, start through goal. */
export type Polyline = [number, number][];

/** Whether `b` lies on the segment interior of `a`-`c` within a relative tolerance: the cross
 * product of the two legs is negligible against their lengths and the legs point the same way
 * (a backtrack `a`-`b`-`a` is a real spike, not a redundant vertex).
 * @param a The previous kept vertex.
 * @param b The candidate vertex.
 * @param c The next vertex.
 * @returns `true` when dropping `b` leaves the drawn polyline unchanged, up to the 1e-9 relative
 * tolerance.
 * @example
 * ```ts
 * isCollinear([0, 0], [1, 0], [2, 0]); // true
 * ```
 */
function isCollinear(a: [number, number], b: [number, number], c: [number, number]): boolean {
  const abx = b[0] - a[0];
  const aby = b[1] - a[1];
  const bcx = c[0] - b[0];
  const bcy = c[1] - b[1];
  const cross = abx * bcy - aby * bcx;
  const dot = abx * bcx + aby * bcy;
  const scale = Math.hypot(abx, aby) * Math.hypot(bcx, bcy);
  return dot > 0 && Math.abs(cross) <= 1e-9 * scale;
}

/** The drawn deviation removing vertex `b` causes between its neighbours `a` and `c`: the
 * distance from `b` to the SEGMENT `a`-`c` (the perpendicular height when the foot of `b` lies
 * on the segment, else the distance to the nearer endpoint). It is the Hausdorff distance
 * between the polyline `a`-`b`-`c` and the chord `a`-`c`: the farthest any drawn point moves when
 * `b` is dropped (distance to a segment is convex along each leg, so the maximum sits at `b`).
 * INVARIANT: the value is a continuous function of the positions of `a`, `b` and `c`
 * everywhere (including `a` equal to `c` and where the foot crosses an endpoint), nondecreasing
 * as `b` moves away from the segment, and comparable across vertices: a vertex with a larger
 * value moves the drawn line more than one with a smaller value, whatever the lengths of their
 * chords. Decision: the measure is the Hausdorff error of Imai and Iri ("Polygonal approximations
 * of a curve: formulations and algorithms", 1988, who give optimal algorithms for it), applied
 * in the greedy vertex-elimination order of Visvalingam and Whyatt (1993) in place of their
 * triangle area: the least-deviation vertex goes first. Douglas and Peucker (1973) split
 * top-down against the ORIGINAL segment instead and are not used. Chosen over the triangle area
 * because area is half of height times chord length: a 10-unit deviation over a 1000-unit chord
 * (area 5000) would outrank a 60-unit deviation over a 10-unit chord (area 300), so a larger
 * real loss would be removed before a smaller one. Each score is against the CURRENT surviving
 * neighbours, so removals compound: the result carries no bound on its deviation from the
 * original polyline, only that every single removal was the cheapest available at its time.
 * PRECONDITION: coordinates are finite and small enough (magnitude below about 1e150) that the
 * squared chord length is finite.
 * @param a The previous surviving vertex.
 * @param b The vertex being scored.
 * @param c The next surviving vertex.
 * @returns The deviation, in scene units; lower is removed first.
 * @example
 * ```ts
 * import { chordDeviation } from "./polyline-simplify";
 *
 * chordDeviation([0, 0], [1, 1], [2, 0]); // 1
 * chordDeviation([0, 0], [10, 0], [5, 0]); // 5: the overshoot past the chord's end
 * ```
 */
export function chordDeviation(a: [number, number], b: [number, number], c: [number, number]): number {
  const acx = c[0] - a[0];
  const acy = c[1] - a[1];
  const len2 = acx * acx + acy * acy;
  const along = len2 === 0 ? 0 : ((b[0] - a[0]) * acx + (b[1] - a[1]) * acy) / len2;
  const foot = Math.min(1, Math.max(0, along));
  return Math.hypot(b[0] - a[0] - foot * acx, b[1] - a[1] - foot * acy);
}

/** Removes the least significant interior vertices of `points` until at most `limit` remain,
 * always removing the vertex whose `chordDeviation` between its surviving neighbours is
 * smallest. A doubly linked list gives O(1) neighbour lookup and a binary min-heap with lazy
 * invalidation gives O(log n) per removal, so the whole pass is O(n log n); scanning every vertex
 * per removal is O(n * k). Ties remove the lower index first, so the result is deterministic.
 * The first and last vertex are never removed.
 * @param points The vertices, at least 2.
 * @param limit The most vertices to keep, at least 2.
 * @returns A new list of the surviving vertices in route order.
 * @example
 * ```ts
 * simplifyByDeviation([[0, 0], [5, 0.1], [10, 0], [10, 10]], 3); // [[0, 0], [10, 0], [10, 10]]
 * ```
 */
function simplifyByDeviation(points: Polyline, limit: number): Polyline {
  const n = points.length;
  const prev = new Int32Array(n);
  const next = new Int32Array(n);
  const version = new Uint32Array(n);
  const removed = new Uint8Array(n);
  for (let i = 0; i < n; i++) {
    prev[i] = i - 1;
    next[i] = i + 1;
  }
  // Heap entries as parallel arrays: deviation, vertex index, that vertex's version when pushed.
  const hDev: number[] = [];
  const hIdx: number[] = [];
  const hVer: number[] = [];
  const less = (x: number, y: number): boolean =>
    hDev[x] < hDev[y] || (hDev[x] === hDev[y] && hIdx[x] < hIdx[y]);
  const swap = (x: number, y: number): void => {
    [hDev[x], hDev[y]] = [hDev[y], hDev[x]];
    [hIdx[x], hIdx[y]] = [hIdx[y], hIdx[x]];
    [hVer[x], hVer[y]] = [hVer[y], hVer[x]];
  };
  const push = (i: number): void => {
    hDev.push(chordDeviation(points[prev[i]], points[i], points[next[i]]));
    hIdx.push(i);
    hVer.push(version[i]);
    let c = hDev.length - 1;
    while (c > 0) {
      const parent = (c - 1) >> 1;
      if (!less(c, parent)) break;
      swap(c, parent);
      c = parent;
    }
  };
  const popIndex = (): number => {
    const top = hIdx[0];
    const topVer = hVer[0];
    const last = hDev.length - 1;
    swap(0, last);
    hDev.pop();
    hIdx.pop();
    hVer.pop();
    let c = 0;
    for (;;) {
      const l = 2 * c + 1;
      const r = l + 1;
      let m = c;
      if (l < hDev.length && less(l, m)) m = l;
      if (r < hDev.length && less(r, m)) m = r;
      if (m === c) break;
      swap(c, m);
      c = m;
    }
    return removed[top] === 0 && version[top] === topVer ? top : -1;
  };
  for (let i = 1; i < n - 1; i++) push(i);
  let remaining = n;
  while (remaining > limit && hDev.length > 0) {
    const i = popIndex();
    if (i < 0) continue;
    removed[i] = 1;
    remaining--;
    const p = prev[i];
    const q = next[i];
    next[p] = q;
    prev[q] = p;
    for (const j of [p, q]) {
      if (j > 0 && j < n - 1) {
        version[j]++;
        push(j);
      }
    }
  }
  const out: Polyline = [];
  for (let i = 0; i < n; i++) if (removed[i] === 0) out.push([points[i][0], points[i][1]]);
  return out;
}

/** Reduces `points` to at most `max` vertices (never fewer than the 2 endpoints, so `max < 2`
 * returns the first and last vertex) while keeping the polyline's shape: first every collinear
 * interior vertex is dropped (lossless up to `isCollinear`'s 1e-9 relative tolerance), and only
 * if more than `max` vertices remain are the least significant corners removed
 * (`simplifyByDeviation`, O(n log n)). The first and last vertex always survive, so the
 * endpoints stay exact. A polyline of at most `max` vertices is returned as a copy. Chosen over
 * even index spacing because viewers draw straight chords between survivors, and index spacing
 * cuts the corners of a long routed path while deviation-based removal drops the flattest
 * vertices first.
 * PRECONDITION: every coordinate is finite and within the magnitude `chordDeviation` accepts; a
 * NaN makes heap comparisons false and misorders the removals.
 * @param points The polyline's vertices.
 * @param max The most vertices to keep.
 * @returns A copy of `points`, or a shape-preserving subset of at most `max` vertices (2 at the
 * least) when it exceeds `max`.
 * @example
 * ```ts
 * import { thinPolyline } from "./polyline-simplify";
 *
 * const long: [number, number][] = Array.from({ length: 1000 }, (_, i) => [i, 0]);
 * thinPolyline(long, 10); // [[0, 0], [999, 0]]: a straight run needs only its endpoints
 * ```
 */
export function thinPolyline(points: Polyline, max: number): Polyline {
  const copy: Polyline = points.map((p) => [p[0], p[1]]);
  if (copy.length <= max) return copy;
  const kept: Polyline = [];
  for (let i = 0; i < copy.length; i++) {
    const last = kept[kept.length - 1];
    const next = copy[i + 1];
    if (i > 0 && i < copy.length - 1 && isCollinear(last, copy[i], next)) continue;
    kept.push(copy[i]);
  }
  const limit = Math.max(2, max);
  return kept.length > limit ? simplifyByDeviation(kept, limit) : kept;
}
