import type { AmbientLightMode } from "@shadowcat/core";

/** The largest wall proximity distance, in cells, the server accepts: the client copy of the
 * server's `MAX_PROXIMITY_CELLS`. `__fixtures__/server-bounds.json` pins it, and a server test
 * reads the same fixture, so a change on either side fails the other. */
export const MAX_PROXIMITY_CELLS = 1000;

/** `true` when `within` is a proximity distance `WallEngine::validate` accepts: finite, positive
 * and at most `MAX_PROXIMITY_CELLS`.
 * @param within The authored distance, in cells.
 * @returns Whether the server accepts the distance.
 * @example
 * ```
 * isValidProximityWithin(2);    // true
 * isValidProximityWithin(0);    // false
 * isValidProximityWithin(1001); // false
 * ```
 */
export function isValidProximityWithin(within: number): boolean {
  return Number.isFinite(within) && within > 0 && within <= MAX_PROXIMITY_CELLS;
}

/** `true` when `angle` is a value `TemplateEngine::validate` accepts for a cone's `shape.angle`:
 * `null` (no authored override; the render layer's default aperture applies) or a finite value
 * in `(0, 180)`, exclusive on both ends. `conePoints`' three-point wedge is collinear (zero area)
 * at exactly 180 degrees and opens away from `direction` past it, so neither end is a
 * representable cone. Pinned to the server by `__fixtures__/server-bounds.json`.
 * @param angle The authored `templateAngle`, or `null`.
 * @returns Whether a cone template may be dispatched with this angle.
 * @example
 * ```
 * isValidTemplateAngle(null); // true: no override
 * isValidTemplateAngle(60);   // true
 * isValidTemplateAngle(180);  // false: collinear wedge
 * isValidTemplateAngle(200);  // false: opens away from `direction`
 * ```
 */
export function isValidTemplateAngle(angle: number | null): boolean {
  return angle === null || (Number.isFinite(angle) && angle > 0 && angle < 180);
}

/** `true` when `width` is a value `TemplateEngine::validate` accepts for `shape.width`: `null`, or
 * finite and `>= 0` (a rect or line width has no upper bound, unlike the cone angle's wedge
 * geometry). Pinned to the server by `__fixtures__/server-bounds.json`.
 * @param width The authored `templateWidth`, or `null`.
 * @returns Whether a line or rect template may be dispatched with this width.
 * @example
 * ```
 * isValidTemplateWidth(null); // true: no override
 * isValidTemplateWidth(4);    // true
 * isValidTemplateWidth(-5);   // false: the server requires >= 0
 * ```
 */
export function isValidTemplateWidth(width: number | null): boolean {
  return width === null || (Number.isFinite(width) && width >= 0);
}

/** Clamps an authored ambient-light level into what `RegionEngine::validate` accepts for the
 * mode: an absolute illumination in `[0, 1]` for `"override"`, a non-negative delta for
 * `"darken"` and `"brighten"`. A non-finite value (`NaN` is what a cleared number input binds
 * as, and serializes to JSON `null`) becomes `0`, the lower bound of every mode. Pinned to the
 * server by `__fixtures__/server-bounds.json`.
 * @param mode The ambient-light mode the level is authored under.
 * @param level The authored level, possibly non-finite or out of range.
 * @returns A level the server accepts for `mode`.
 * @example
 * ```
 * clampAmbientLevel("override", 2);  // 1
 * clampAmbientLevel("darken", -1);   // 0
 * clampAmbientLevel("brighten", NaN); // 0
 * ```
 */
export function clampAmbientLevel(mode: AmbientLightMode, level: number): number {
  if (!Number.isFinite(level)) return 0;
  const floored = Math.max(0, level);
  return mode === "override" ? Math.min(1, floored) : floored;
}
