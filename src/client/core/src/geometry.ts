// Pure shape tessellation: scene-coordinate conversion of template/drawing/region shapes into
// flat point arrays [x0,y0,x1,y1,…]. Lives in `@shadowcat/core` so both the render layer (which
// draws the points) and the template-to-region conversion (which authors a region from the same
// points) read ONE tessellation; a second copy would let a region cover cells the template never
// showed. Angles are degrees; 0° = +x, positive toward +y (scene y is down).

/** Converts degrees to radians, per the shared angle convention (0° = +x, positive toward +y).
 * @param d An angle in degrees.
 * @returns `d` in radians.
 * @example
 * ```
 * // module-private helper; not exported from @shadowcat/core
 * deg2rad(180); // Math.PI
 * ```
 */
const deg2rad = (d: number): number => (d * Math.PI) / 180;

/** {@link conePoints}'s default full aperture when a template's `shape.angle` is absent. */
export const DEFAULT_CONE_ANGLE_DEGREES = 60;

/**
 * Whether a template's authored cone aperture can be drawn: absent (the default applies), or a
 * finite number strictly between 0 and 180 degrees. `conePoints`' three-point wedge is collinear
 * at 180 and opens away from the facing past it, so neither endpoint is drawable. The bound
 * matches the server's template validation; the render layer and the template-to-region
 * conversion both read this one guard so a template is drawn exactly when it can be converted.
 * @param angle The authored `shape.angle` (`null` or `undefined` when absent).
 * @returns `true` when the angle is absent or inside `(0, 180)`.
 * @example
 * ```ts
 * import { isDrawableConeAngle } from "@shadowcat/core";
 *
 * isDrawableConeAngle(60); // true
 * isDrawableConeAngle(180); // false
 * ```
 */
export function isDrawableConeAngle(angle: number | null | undefined): boolean {
  return angle == null || (Number.isFinite(angle) && angle > 0 && angle < 180);
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
 * import { rectPoints } from "@shadowcat/core";
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
 * import { ellipsePoints } from "@shadowcat/core";
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
 * import { circlePoints } from "@shadowcat/core";
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
 * `directionDeg ± apertureDeg/2`. Degrees follow the shared convention: 0° = +x,
 * positive toward +y (scene y is down).
 * @param apexX Apex x.
 * @param apexY Apex y.
 * @param size Distance from the apex to each base corner.
 * @param directionDeg Cone facing direction, in degrees (0° = +x).
 * @param apertureDeg Full angle between the two base corners. Defaults to
 * {@link DEFAULT_CONE_ANGLE_DEGREES}.
 * @returns A flat `[apexX,apexY, x1,y1, x2,y2]` 3-point array.
 * @example
 * ```ts
 * import { conePoints } from "@shadowcat/core";
 *
 * conePoints(0, 0, 10, 0, 60).length; // 6 (3 points × 2 coords)
 * ```
 */
export function conePoints(
  apexX: number,
  apexY: number,
  size: number,
  directionDeg: number,
  apertureDeg: number = DEFAULT_CONE_ANGLE_DEGREES,
): number[] {
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
 * Four corners of a rectangle (sides `2*half` by `2*halfWidth`) centered at `(cx,cy)`,
 * rotated `directionDeg` about its center. Degrees follow the shared convention: 0° = +x,
 * positive toward +y (scene y is down). `halfWidth` defaults to `half`, so omitting it gives a
 * rotated square — a template's `"rect"` kind is forced-square exactly when its `width` is
 * absent (`width ?? size`, the caller's job).
 * @param cx Center x.
 * @param cy Center y.
 * @param half Half the first-axis side length (so the full side is `2*half`).
 * @param directionDeg Rotation, in degrees (0° = axis-aligned).
 * @param halfWidth Half the second-axis side length. Defaults to `half` (a square).
 * @returns A flat `[x0,y0, x1,y1, x2,y2, x3,y3]` 4-point array.
 * @example
 * ```ts
 * import { squarePoints } from "@shadowcat/core";
 *
 * squarePoints(0, 0, 5, 0); // [-5,-5, 5,-5, 5,5, -5,5]
 * ```
 */
export function squarePoints(cx: number, cy: number, half: number, directionDeg: number, halfWidth: number = half): number[] {
  const a = deg2rad(directionDeg);
  const c = Math.cos(a);
  const s = Math.sin(a);
  const corners: [number, number][] = [
    [-half, -halfWidth],
    [half, -halfWidth],
    [half, halfWidth],
    [-half, halfWidth],
  ];
  const out: number[] = [];
  for (const [x, y] of corners) out.push(cx + x * c - y * s, cy + x * s + y * c);
  return out;
}

/**
 * A line segment from `(x0,y0)`, length `length`, facing `directionDeg` — a bare 2-point
 * open segment when `width` is `0` or omitted (the zero-width template line), or a closed
 * 4-point rectangle of thickness `width` centered on the segment's axis otherwise. Degrees
 * follow the shared convention: 0° = +x, positive toward +y (scene y is down).
 * @param x0 Start x.
 * @param y0 Start y.
 * @param length Segment length.
 * @param directionDeg Facing direction, in degrees (0° = +x).
 * @param width Perpendicular thickness, scene units. Defaults to `0` (a bare segment).
 * @returns A flat 2-point `[x0,y0, x1,y1]` array at `width <= 0`, else a flat 4-point
 * rectangle-corner array.
 * @example
 * ```ts
 * import { linePoints } from "@shadowcat/core";
 *
 * linePoints(0, 0, 10, 0); // [0,0, 10,0] — zero-width, a bare segment
 * linePoints(0, 0, 10, 0, 4).length; // 8 (4 points × 2 coords) — a thick line
 * ```
 */
export function linePoints(x0: number, y0: number, length: number, directionDeg: number, width = 0): number[] {
  const a = deg2rad(directionDeg);
  const dx = Math.cos(a);
  const dy = Math.sin(a);
  const x1 = x0 + length * dx;
  const y1 = y0 + length * dy;
  if (!(width > 0)) {
    return [x0, y0, x1, y1];
  }
  const half = width / 2;
  // Perpendicular unit vector (rotate the direction vector 90°).
  const px = -dy * half;
  const py = dx * half;
  return [x0 + px, y0 + py, x1 + px, y1 + py, x1 - px, y1 - py, x0 - px, y0 - py];
}
