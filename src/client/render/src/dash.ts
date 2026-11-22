/**
 * Splits a ring into dash polylines: walks the ring's perimeter and emits one flat-point polyline
 * per "on" run, with "off" runs skipped. A dash that spans a vertex keeps the vertex, so corners
 * stay stroked. Pixi's Graphics has no native dash style, so the backend strokes these polylines
 * as separate subpaths. A non-positive `on`, or an `off` below zero, returns the ring unbroken.
 * @param points The ring's flat `[x0, y0, x1, y1, ...]` points.
 * @param closed Whether the ring closes back to its first point (the closing edge is dashed too).
 * @param on Dash length, in scene units.
 * @param off Gap length, in scene units.
 * @returns Flat-point polylines, one per dash; an empty list for fewer than two points.
 * @example
 * ```ts
 * import { dashRing } from "@shadowcat/render";
 *
 * dashRing([0, 0, 10, 0], false, 2, 2); // [[0,0, 2,0], [4,0, 6,0], [8,0, 10,0]]
 * ```
 */
export function dashRing(points: number[], closed: boolean, on: number, off: number): number[][] {
  const n = Math.floor(points.length / 2);
  if (n < 2) return [];
  const verts: number[] = points.slice(0, n * 2);
  if (closed) verts.push(points[0], points[1]);
  if (!(on > 0) || !(off >= 0)) return [verts];
  const dashes: number[][] = [];
  let current: number[] | null = [verts[0], verts[1]];
  let remaining = on; // length left in the current run
  let drawing = true;
  for (let i = 0; i + 3 < verts.length; i += 2) {
    const x0 = verts[i];
    const y0 = verts[i + 1];
    const x1 = verts[i + 2];
    const y1 = verts[i + 3];
    const len = Math.hypot(x1 - x0, y1 - y0);
    if (len === 0) continue;
    let pos = 0;
    while (len - pos > remaining) {
      pos += remaining;
      const t = pos / len;
      const x = x0 + (x1 - x0) * t;
      const y = y0 + (y1 - y0) * t;
      if (drawing) {
        current!.push(x, y);
        dashes.push(current!);
        current = null;
        remaining = off;
      } else {
        current = [x, y];
        remaining = on;
      }
      drawing = !drawing;
    }
    remaining -= len - pos;
    if (drawing) current!.push(x1, y1);
  }
  if (drawing && current && current.length >= 4) dashes.push(current);
  return dashes;
}
