// Render-side shape geometry: the tessellation helpers are defined in `@shadowcat/core` (one
// tessellation shared with the template-to-region conversion) and re-exported here so the
// render layer keeps importing them from one local module; color parsing is render-only.

export {
  DEFAULT_CONE_ANGLE_DEGREES,
  rectPoints,
  ellipsePoints,
  circlePoints,
  conePoints,
  squarePoints,
  linePoints,
} from "@shadowcat/core";

/**
 * Parses a CSS-style hex color string into a packed 24-bit integer.
 * @param hex `#rrggbb` or `rrggbb` (case-insensitive); leading/trailing whitespace is trimmed.
 * @returns The packed `0xRRGGBB` integer, or `0x000000` when `hex` doesn't match a 6-digit hex string.
 * @example
 * ```ts
 * import { parseColor } from "@shadowcat/render";
 *
 * parseColor("#ff8800"); // 0xff8800
 * parseColor("not a color"); // 0x000000
 * ```
 */
export function parseColor(hex: string): number {
  const m = /^#?([0-9a-fA-F]{6})$/.exec(hex.trim());
  return m ? parseInt(m[1], 16) : 0;
}

/**
 * Splits one segment into dash sub-segments: `on` drawn, `off` skipped, starting with a dash
 * at `(x1, y1)`. A final partial dash is clipped to the segment end. Returns nothing for a
 * zero-length segment, or when `on` or `off` is not a finite positive number.
 * @param x1 Segment start x.
 * @param y1 Segment start y.
 * @param x2 Segment end x.
 * @param y2 Segment end y.
 * @param on Drawn length of each dash, in the same units as the coordinates.
 * @param off Gap length between dashes.
 * @returns Flat `[ax, ay, bx, by]` quadruples, one per dash.
 * @example
 * ```ts
 * import { dashSegments } from "@shadowcat/render";
 *
 * dashSegments(0, 0, 10, 0, 4, 2); // [[0, 0, 4, 0], [6, 0, 10, 0]]
 * ```
 */
export function dashSegments(x1: number, y1: number, x2: number, y2: number, on: number, off: number): number[][] {
  const len = Math.hypot(x2 - x1, y2 - y1);
  if (!(len > 0) || !(on > 0) || !(off > 0) || !Number.isFinite(on) || !Number.isFinite(off)) return [];
  const ux = (x2 - x1) / len;
  const uy = (y2 - y1) / len;
  const out: number[][] = [];
  for (let d = 0; d < len; d += on + off) {
    const end = Math.min(d + on, len);
    out.push([x1 + ux * d, y1 + uy * d, x1 + ux * end, y1 + uy * end]);
  }
  return out;
}
