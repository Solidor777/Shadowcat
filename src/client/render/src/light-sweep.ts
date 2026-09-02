// Pure cell math for the carried-light sweep: while a `moverLight` timeline plays, each
// sample's glow is rasterized onto the cells it reaches and unioned over the held lighting
// frame. Extracted from `RenderEngine`/`Lighting` for the same reason as `fog-blend`: the
// backend is GL-only, so the sweep's cell selection and cross-fade must be unit-testable.
// A `//` header, not a `/** */` block: a doc block preceding another doc block rather than a
// declaration binds to nothing, since every consumer takes the NEAREST one.

import type { Grid } from "./grid";
import { bandAlpha, MAX_DARK_ALPHA, TINT_ALPHA, type LitDrawCell } from "./lighting";
import type { MoveLightSample, Polygon } from "./types";

/** Upper bound on the candidate cells one light sample may enumerate. A sample whose dim
 * reach covers more than this many cells contributes nothing (fail-closed: the glow is a
 * cosmetic preview, and an unbounded enumeration per tick is a DoS surface). */
export const MAX_LIGHT_SWEEP_CELLS = 4096;

/** Even-odd point-in-polygon over a flat `[x0,y0,x1,y1,…]` ring (the `Polygon` encoding).
 * Clean-room crossing-number test; a ring with fewer than 3 vertices contains nothing.
 * @param x The probe's scene x.
 * @param y The probe's scene y.
 * @param pts The ring's flat coordinate pairs.
 * @returns Whether `(x, y)` lies inside the ring.
 * @example
 * ```
 * // not exported from @shadowcat/render; internal to the light sweep
 * pointInFlatRing(5, 5, [0, 0, 10, 0, 10, 10, 0, 10]); // true
 * ```
 */
function pointInFlatRing(x: number, y: number, pts: number[]): boolean {
  const n = pts.length >> 1;
  if (n < 3) return false;
  let inside = false;
  for (let i = 0, j = n - 1; i < n; j = i++) {
    const xi = pts[2 * i], yi = pts[2 * i + 1];
    const xj = pts[2 * j], yj = pts[2 * j + 1];
    const crosses = (yi > y) !== (yj > y);
    if (crosses && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

/** Whether `(x, y)` lies inside any of the sample's `[x, y][]` rings.
 * @param x The probe's scene x.
 * @param y The probe's scene y.
 * @param rings The rings to test, each a list of `[x, y]` vertices.
 * @returns `true` when some ring contains the point.
 * @example
 * ```
 * // not exported from @shadowcat/render; internal to the light sweep
 * pointInRings(5, 5, [[[0, 0], [10, 0], [10, 10], [0, 10]]]); // true
 * ```
 */
function pointInRings(x: number, y: number, rings: [number, number][][]): boolean {
  return rings.some((ring) => pointInFlatRing(x, y, ring.flat()));
}

/**
 * The lit cells one carried-light sample contributes: every cell of `grid` whose center lies
 * within `dim` of `sample.pos`, inside one of the sample's occlusion polygons, AND inside one
 * of the viewer's own line-of-sight polygons (`los`, the fog's `visible` set — the server
 * never clips the glow to the recipient's sight, so the client intersects here). A cell within
 * `bright` resolves to the brightest band; the rest of the disc to band 1 (the second band,
 * "dim" under the seed gradation) — a cosmetic approximation of the server's falloff, never
 * a second illumination rule; the committed frame that follows the move is the truth at rest.
 * Every contributed cell is tinted with the light's color. Candidate cells come from the axial
 * bounding box of the disc's pixel bounding box through `Grid.cellOf` (the affine pixel→axial
 * map sends a rectangle to a parallelogram, whose axial bounds contain every hex centered in
 * the rectangle — so one enumeration serves both grid kinds); a degenerate sample
 * (non-finite position or reach) or one exceeding `MAX_LIGHT_SWEEP_CELLS` contributes nothing.
 * @param sample The light sample to rasterize.
 * @param grid The active grid (cell geometry + indexing).
 * @param los The viewer's current visible polygons (scene coords, flat encoding).
 * @param bandCount The active gradation's band count (drives the dim-band darkening alpha).
 * @returns The contributed cells, one per grid cell, in enumeration order.
 * @example
 * ```ts
 * import { Grid } from "@shadowcat/render";
 *
 * const grid = new Grid({ kind: "square", size: 100 });
 * // not exported from @shadowcat/render; internal to RenderEngine's light sweep
 * lightSampleCells(
 *   { tMs: 0, pos: [50, 50], bright: 100, dim: 250, color: 0xffcc66, intensity: 1, falloff: "linear", polygons: [[[-500, -500], [500, -500], [500, 500], [-500, 500]]] },
 *   grid,
 *   [{ points: [-500, -500, 500, -500, 500, 500, -500, 500] }],
 *   3,
 * ).length; // 21 — the cells within 250 units of (50, 50)
 * ```
 */
export function lightSampleCells(
  sample: MoveLightSample,
  grid: Grid,
  los: Polygon[],
  bandCount: number,
): LitDrawCell[] {
  const [px, py] = sample.pos;
  const dim = sample.dim;
  if (!Number.isFinite(px) || !Number.isFinite(py) || !Number.isFinite(dim) || dim <= 0) return [];
  const bright = Number.isFinite(sample.bright) ? Math.max(0, sample.bright) : 0;
  const corners = [
    grid.cellOf({ x: px - dim, y: py - dim }),
    grid.cellOf({ x: px + dim, y: py - dim }),
    grid.cellOf({ x: px + dim, y: py + dim }),
    grid.cellOf({ x: px - dim, y: py + dim }),
  ];
  const c0 = Math.min(...corners.map((c) => c.col)) - 1;
  const c1 = Math.max(...corners.map((c) => c.col)) + 1;
  const r0 = Math.min(...corners.map((c) => c.row)) - 1;
  const r1 = Math.max(...corners.map((c) => c.row)) + 1;
  if ((c1 - c0 + 1) * (r1 - r0 + 1) > MAX_LIGHT_SWEEP_CELLS) return [];
  const losRings = los.map((p) => p.points);
  const out: LitDrawCell[] = [];
  for (let col = c0; col <= c1; col++) {
    for (let row = r0; row <= r1; row++) {
      const center = grid.cellCenter(col, row);
      const d = Math.hypot(center.x - px, center.y - py);
      if (d > dim) continue;
      if (!losRings.some((ring) => pointInFlatRing(center.x, center.y, ring))) continue;
      if (!pointInRings(center.x, center.y, sample.polygons)) continue;
      const band = d <= bright ? 0 : Math.min(1, Math.max(0, bandCount - 1));
      out.push({
        i: col,
        j: row,
        alpha: bandAlpha(band, bandCount),
        tint: sample.color,
        tintAlpha: TINT_ALPHA,
        desaturate: false,
        corners: grid.cellVertices(col, row),
      });
    }
  }
  return out;
}

/** Cell identity key shared with `Lighting`'s fade — `"i,j"`.
 * @param c A cell's grid coordinates.
 * @param c.i Grid column index.
 * @param c.j Grid row index.
 * @returns `"i,j"`.
 * @example
 * ```
 * // module-private helper; not exported from @shadowcat/render
 * cellKey({ i: 0, j: 0 }); // "0,0"
 * ```
 */
const cellKey = (c: {
  /** Grid column index. */
  i: number;
  /** Grid row index. */
  j: number;
}): string => `${c.i},${c.j}`;

/**
 * Cross-fade between two consecutive light samples' cell sets: `factor` 0 is fully `from`,
 * 1 fully `to`. Cells in both lerp `alpha` and `tintAlpha` and keep `to`'s tint; a cell in only
 * one set fades its `tintAlpha` from/to 0 and its darkening from/to `MAX_DARK_ALPHA` — an
 * ABSENT cell is fully dark, so that is the value a one-sided cell fades toward — and is
 * dropped outright at zero weight (a `to`-only cell at factor 0, a `from`-only cell at 1: a
 * cell the sweep does not light must not exist in the overlay, or it would count as lit). The
 * glow therefore slides between positions rather than snapping. `desaturate` is never set by a
 * sweep cell. Deterministic order: `from`'s cells first, then `to`-only cells.
 * @param from The outgoing sample's cells (`lightSampleCells`).
 * @param to The incoming sample's cells.
 * @param factor Blend position in `[0,1]` (`computeFogBlendFactor`).
 * @returns The blended cell set.
 * @example
 * ```
 * // not exported from @shadowcat/render; internal to RenderEngine's light sweep
 * blendLightCells([], [{ i: 0, j: 0, alpha: 0, tint: 0xffffff, tintAlpha: 0.25, desaturate: false, corners: [] }], 0.5)[0].tintAlpha; // 0.125
 * ```
 */
export function blendLightCells(from: LitDrawCell[], to: LitDrawCell[], factor: number): LitDrawCell[] {
  const t = Number.isFinite(factor) ? Math.min(1, Math.max(0, factor)) : 1;
  const toByKey = new Map(to.map((c) => [cellKey(c), c]));
  const seen = new Set<string>();
  const out: LitDrawCell[] = [];
  for (const f of from) {
    const k = cellKey(f);
    seen.add(k);
    const n = toByKey.get(k);
    if (n) {
      out.push({ ...n, alpha: f.alpha + (n.alpha - f.alpha) * t, tintAlpha: f.tintAlpha + (n.tintAlpha - f.tintAlpha) * t });
    } else if (t < 1) {
      out.push({ ...f, alpha: f.alpha + (MAX_DARK_ALPHA - f.alpha) * t, tintAlpha: f.tintAlpha * (1 - t) });
    }
  }
  if (t > 0) {
    for (const n of to) {
      if (seen.has(cellKey(n))) continue;
      out.push({ ...n, alpha: MAX_DARK_ALPHA + (n.alpha - MAX_DARK_ALPHA) * t, tintAlpha: n.tintAlpha * t });
    }
  }
  return out;
}
