import type { Point, VisibilityInput } from "./types";

/**
 * Whether a point lies inside a flat polygon (even-odd rule, ray cast along +x).
 * Algorithm: crossing-number point-in-polygon test, the standard ray-casting formulation
 * (Shimrat 1962, "Algorithm 112: Position of point relative to polygon"); even-odd keeps a
 * self-overlapping visibility polygon from flipping to "outside" at an overlap.
 * @param p The scene-coordinate point to test.
 * @param points The polygon's flat `[x0,y0,x1,y1,…]` vertices.
 * @returns `true` when `p` is inside the polygon; a degenerate polygon (under three vertices) is outside.
 * @example
 * ```ts
 * import { pointInPolygon } from "@shadowcat/render";
 *
 * pointInPolygon({ x: 5, y: 5 }, [0, 0, 10, 0, 10, 10, 0, 10]); // true
 * ```
 */
export function pointInPolygon(p: Point, points: readonly number[]): boolean {
  const n = points.length / 2;
  if (n < 3) return false;
  let inside = false;
  for (let i = 0, j = n - 1; i < n; j = i++) {
    const xi = points[2 * i];
    const yi = points[2 * i + 1];
    const xj = points[2 * j];
    const yj = points[2 * j + 1];
    if (yi > p.y !== yj > p.y && p.x < ((xj - xi) * (p.y - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

/**
 * Whether a token at `at` is revealed by the fog `input`: no fog (`mode: "all"`), perceived
 * through a creature sense (those tokens render above the fog), or its point inside a currently
 * visible polygon. Fail-closed: a `masked` input with no visible polygons reveals nothing, and
 * the explored (dimmed-memory) region never reveals a token — it shows terrain only.
 * @param input The visibility mask currently applied.
 * @param tokenId The token's document id.
 * @param at The token's scene-coordinate position.
 * @returns `true` when a viewer sees the token's position.
 * @example
 * ```ts
 * import { isTokenRevealed } from "@shadowcat/render";
 *
 * isTokenRevealed({ mode: "masked", visible: [], explored: [], perceived: [] }, "t1", { x: 0, y: 0 }); // false
 * ```
 */
export function isTokenRevealed(input: VisibilityInput, tokenId: string, at: Point): boolean {
  if (input.mode === "all") return true;
  if (input.perceived.includes(tokenId)) return true;
  return input.visible.some((poly) => pointInPolygon(at, poly.points));
}
