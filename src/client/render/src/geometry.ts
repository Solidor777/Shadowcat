// Pure shape geometry: scene-coordinate tessellation of template/drawing shapes into
// flat point arrays [x0,y0,x1,y1,…], plus color parsing. The backend draws whatever
// points it is given, so all shape math (cone/circle/ellipse/square) lives here and is
// headless-testable. Angles are degrees; 0° = +x, positive toward +y (scene y is down).

const deg2rad = (d: number): number => (d * Math.PI) / 180;

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
 * Four corners of the axis-aligned rectangle spanning the two given corners, in the
 * order `(x0,y0)`, `(x1,y0)`, `(x1,y1)`, `(x0,y1)`.
 * @param x0 One corner's x.
 * @param y0 One corner's y.
 * @param x1 The opposite corner's x.
 * @param y1 The opposite corner's y.
 * @returns A flat `[x0,y0, x1,y0, x1,y1, x0,y1]` point array (`PIXI.Graphics.poly`-ready).
 * @example
 * ```ts
 * import { rectPoints } from "@shadowcat/render";
 *
 * rectPoints(0, 0, 10, 20); // [0,0, 10,0, 10,20, 0,20]
 * ```
 */
export function rectPoints(x0: number, y0: number, x1: number, y1: number): number[] {
  return [x0, y0, x1, y0, x1, y1, x0, y1];
}

/**
 * Points around the ellipse inscribed in the bounding box `(x0,y0)`–`(x1,y1)`, evenly
 * spaced by angle (not arc length), starting at the box's right edge.
 * @param x0 One bbox corner's x.
 * @param y0 One bbox corner's y.
 * @param x1 The opposite bbox corner's x.
 * @param y1 The opposite bbox corner's y.
 * @param segments Number of points to generate. Defaults to 32.
 * @returns A flat `[x0,y0, x1,y1, …]` point array, `segments` points long.
 * @example
 * ```ts
 * import { ellipsePoints } from "@shadowcat/render";
 *
 * ellipsePoints(0, 0, 20, 10, 4).length; // 8 (4 points × 2 coords)
 * ```
 */
export function ellipsePoints(x0: number, y0: number, x1: number, y1: number, segments = 32): number[] {
  const cx = (x0 + x1) / 2;
  const cy = (y0 + y1) / 2;
  const rx = Math.abs(x1 - x0) / 2;
  const ry = Math.abs(y1 - y0) / 2;
  const out: number[] = [];
  for (let i = 0; i < segments; i++) {
    const a = (i / segments) * 2 * Math.PI;
    out.push(cx + rx * Math.cos(a), cy + ry * Math.sin(a));
  }
  return out;
}

/**
 * Points around a circle of radius `r` centered at `(cx,cy)` — delegates to
 * {@link ellipsePoints} with a square bounding box.
 * @param cx Center x.
 * @param cy Center y.
 * @param r Radius.
 * @param segments Number of points to generate. Defaults to 32.
 * @returns A flat `[x0,y0, x1,y1, …]` point array, `segments` points long.
 * @example
 * ```ts
 * import { circlePoints } from "@shadowcat/render";
 *
 * circlePoints(0, 0, 5, 4).length; // 8 (4 points × 2 coords)
 * ```
 */
export function circlePoints(cx: number, cy: number, r: number, segments = 32): number[] {
  return ellipsePoints(cx - r, cy - r, cx + r, cy + r, segments);
}

/**
 * Isoceles cone AoE as a 3-point wedge (apex + two straight base corners, not an arc)
 * — apex at `(apexX,apexY)`, base corners at distance `size` along
 * `directionDeg ± apertureDeg/2`. Degrees follow this file's convention: 0° = +x,
 * positive toward +y (scene y is down).
 * @param apexX Apex x.
 * @param apexY Apex y.
 * @param size Distance from the apex to each base corner.
 * @param directionDeg Cone facing direction, in degrees (0° = +x).
 * @param apertureDeg Full angle between the two base corners. Defaults to 60°.
 * @returns A flat `[apexX,apexY, x1,y1, x2,y2]` 3-point array.
 * @example
 * ```ts
 * import { conePoints } from "@shadowcat/render";
 *
 * conePoints(0, 0, 10, 0, 60).length; // 6 (3 points × 2 coords)
 * ```
 */
export function conePoints(apexX: number, apexY: number, size: number, directionDeg: number, apertureDeg = 60): number[] {
  const a = deg2rad(directionDeg);
  const half = deg2rad(apertureDeg / 2);
  return [
    apexX,
    apexY,
    apexX + size * Math.cos(a - half),
    apexY + size * Math.sin(a - half),
    apexX + size * Math.cos(a + half),
    apexY + size * Math.sin(a + half),
  ];
}

/**
 * Four corners of a square (side `2*half`) centered at `(cx,cy)`, rotated
 * `directionDeg` about its center. Degrees follow this file's convention: 0° = +x,
 * positive toward +y (scene y is down).
 * @param cx Center x.
 * @param cy Center y.
 * @param half Half the side length (so the full side is `2*half`).
 * @param directionDeg Rotation, in degrees (0° = axis-aligned).
 * @returns A flat `[x0,y0, x1,y1, x2,y2, x3,y3]` 4-point array.
 * @example
 * ```ts
 * import { squarePoints } from "@shadowcat/render";
 *
 * squarePoints(0, 0, 5, 0); // [-5,-5, 5,-5, 5,5, -5,5]
 * ```
 */
export function squarePoints(cx: number, cy: number, half: number, directionDeg: number): number[] {
  const a = deg2rad(directionDeg);
  const c = Math.cos(a);
  const s = Math.sin(a);
  const corners: [number, number][] = [
    [-half, -half],
    [half, -half],
    [half, half],
    [-half, half],
  ];
  const out: number[] = [];
  for (const [x, y] of corners) out.push(cx + x * c - y * s, cy + x * s + y * c);
  return out;
}
