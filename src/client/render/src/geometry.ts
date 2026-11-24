// Render-side shape geometry: the tessellation helpers are defined in `@shadowcat/core` (one
// tessellation shared with the template-to-region conversion) and re-exported here so the
// render layer keeps importing them from one local module; color parsing is render-only.

export {
  DEFAULT_CONE_ANGLE_DEGREES,
  isDrawableConeAngle,
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

