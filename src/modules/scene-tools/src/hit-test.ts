import type { WireDocument, ReadableDocuments, FootprintLookup } from "@shadowcat/core";
import { resolveTokenBox } from "@shadowcat/core";
import type { Point } from "@shadowcat/render";

/** The id of the topmost token whose footprint contains `p`, or `null`. "Topmost" is the LAST
 * matching entry in `tokens`' own iteration order, which is `ReadableDocuments.query`'s Map-
 * insertion order (`DocumentStore.query`,
 * `OptimisticClient.query` — a `Map` never reorders an existing key on
 * update) — the same order `TokenView.reconcile` walks, and the render layer's `tokens` layer
 * only ever APPENDS a new token's container the first time it is seen
 * (`PixiBackend.createTokenNode`), so this "last in iteration order" tie-break
 * genuinely matches render z-order, not merely by convention. Footprint = the resolved box
 * (`resolveTokenBox`): a circle token uses ellipse containment, a square the AABB. A
 * degenerate box (`w <= 0 || h <= 0`) is skipped entirely (never hit-testable). Rotation is
 * ignored for picking.
 * @param tokens The candidate token documents (typically `store.query("token")`).
 * @param p The point to test (scene coords).
 * @param store Passed through to `resolveTokenBox` for actor-linked shape resolution.
 * @param footprints The server's resolved footprints, passed through to `resolveTokenBox` — the
 * picked area is the authoritative extent, so a hex token picks over the hexes it occupies.
 * @returns The topmost hit token's id, or `null` when none contains `p`.
 * @example
 * ```
 * declare const store: ReadableDocuments;
 * declare const footprints: FootprintLookup;
 * declare const p: Point;
 * const id = topTokenAt(store.query("token"), p, store, footprints);
 * ```
 */
export function topTokenAt(tokens: WireDocument[], p: Point, store: ReadableDocuments, footprints: FootprintLookup): string | null {
  let hit: string | null = null;
  for (const t of tokens) {
    const box = resolveTokenBox(t, store, footprints);
    if (box.w <= 0 || box.h <= 0) continue;
    const dx = p.x - box.x;
    const dy = p.y - box.y;
    const hw = box.w / 2;
    const hh = box.h / 2;
    const inside =
      box.shape === "circle"
        ? (dx * dx) / (hw * hw) + (dy * dy) / (hh * hh) <= 1
        : Math.abs(dx) <= hw && Math.abs(dy) <= hh;
    if (inside) hit = t.id;
  }
  return hit;
}

/** Scene-coord tolerance (from a light's position) within which a click picks the light's
 * marker — deliberately a little wider than `LightView`'s drawn marker so the pick target is
 * forgiving. */
const LIGHT_PICK_TOLERANCE = 12;

/** The id of the nearest light whose marker contains `p`, or `null`. "Nearest" (not document
 * order): markers are point targets, so z-order is meaningless and the closest one is the only
 * defensible pick. A light with a missing/non-finite position is never pickable (it renders no
 * marker either — `LightView` skips the same docs). Cull the candidate list to the viewed scene
 * before calling (callers pass `parent_id`-filtered docs).
 * @param lights The candidate light documents (typically `store.query("light")`, scene-scoped).
 * @param p The point to test (scene coords).
 * @returns The nearest hit light's id, or `null` when none is within tolerance.
 * @example
 * ```
 * declare const store: ReadableDocuments;
 * declare const p: Point;
 * const id = topLightAt(store.query("light"), p);
 * ```
 */
export function topLightAt(lights: WireDocument[], p: Point): string | null {
  let hit: string | null = null;
  let best = LIGHT_PICK_TOLERANCE;
  for (const l of lights) {
    const e = l.engine as {
      /** Light position x, scene units; absent ⇒ unpickable. */
      x?: number;
      /** Light position y, scene units; absent ⇒ unpickable. */
      y?: number;
    } | undefined;
    if (typeof e?.x !== "number" || typeof e.y !== "number") continue;
    if (!Number.isFinite(e.x) || !Number.isFinite(e.y)) continue;
    const d = Math.hypot(p.x - e.x, p.y - e.y);
    if (d <= best) {
      best = d;
      hit = l.id;
    }
  }
  return hit;
}

/** Scene-coord distance tolerance for picking a wall segment (a little wider than the drawn
 * `WALL_WIDTH` stroke so the pick target is forgiving). */
const WALL_PICK_TOLERANCE = 8;

/** Distance from point `p` to the segment `a`–`b` (scene coords).
 * @param p The point to measure from.
 * @param a The segment's first endpoint.
 * @param b The segment's second endpoint.
 * @returns The perpendicular (or endpoint) distance, in scene units.
 * @example
 * ```
 * pointSegDistance({ x: 5, y: 5 }, { x: 0, y: 0 }, { x: 10, y: 0 }); // 5
 * ```
 */
function pointSegDistance(p: Point, a: Point, b: Point): number {
  const abx = b.x - a.x;
  const aby = b.y - a.y;
  const len2 = abx * abx + aby * aby;
  // Degenerate (zero-length) segment: the projection parameter is meaningless, measure to `a`.
  const t = len2 > 0 ? Math.max(0, Math.min(1, ((p.x - a.x) * abx + (p.y - a.y) * aby) / len2)) : 0;
  return Math.hypot(p.x - (a.x + t * abx), p.y - (a.y + t * aby));
}

/** The id of the nearest wall whose segment passes within tolerance of `p`, or `null`. A wall
 * with a missing/non-finite endpoint is never pickable (it renders no segment either —
 * `WallView.toSpec` rejects the same docs). Cull the candidate list to the viewed scene before
 * calling (callers pass `parent_id`-filtered docs).
 * @param walls The candidate wall documents (typically `store.query("wall")`, scene-scoped).
 * @param p The point to test (scene coords).
 * @returns The nearest hit wall's id, or `null` when none is within tolerance.
 * @example
 * ```
 * declare const store: ReadableDocuments;
 * declare const p: Point;
 * const id = topWallAt(store.query("wall"), p);
 * ```
 */
export function topWallAt(walls: WireDocument[], p: Point): string | null {
  let hit: string | null = null;
  let best = WALL_PICK_TOLERANCE;
  for (const w of walls) {
    const seg = (w.engine as {
      /** The wall's segment; absent ⇒ unpickable. */
      seg?: {
        /** First endpoint x. */
        x1: number;
        /** First endpoint y. */
        y1: number;
        /** Second endpoint x. */
        x2: number;
        /** Second endpoint y. */
        y2: number;
      };
    } | undefined)?.seg;
    if (!seg) continue;
    const { x1, y1, x2, y2 } = seg;
    if (![x1, y1, x2, y2].every((n) => Number.isFinite(n))) continue;
    const d = pointSegDistance(p, { x: x1, y: y1 }, { x: x2, y: y2 });
    if (d <= best) {
      best = d;
      hit = w.id;
    }
  }
  return hit;
}
