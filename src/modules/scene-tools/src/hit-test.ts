import type { WireDocument, ReadableDocuments, FootprintLookup } from "@shadowcat/core";
import { resolveTokenBox } from "@shadowcat/core";
import type { Point, ShapeNodeSpec } from "@shadowcat/render";
import { regionShapeSpec, drawingShapeSpec, templateShapeSpec } from "@shadowcat/render";

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

/** Scene-coord distance tolerance for picking an OPEN (unfilled) region/drawing/template
 * segment — matches `WALL_PICK_TOLERANCE`, the same forgiving-click radius convention. */
const SHAPE_PICK_TOLERANCE = 8;

/** Point-in-polygon test (even-odd / ray-casting rule) over a flat `[x0,y0,x1,y1,…]` point
 * ring — the standard Jordan-curve-theorem algorithm (public-domain; clean-room), used for
 * every CLOSED region/drawing/template shape (`ShapeNodeSpec.closed === true`). A point
 * exactly on an edge may resolve either way (the usual ray-casting caveat); picking is
 * forgiving by nature, so this is not worth a dedicated boundary case.
 * @param points Flat `[x0,y0,x1,y1,…]` polygon points (closed implicitly — the last point
 * connects back to the first).
 * @param p The point to test (scene coords).
 * @returns `true` when `p` is inside the polygon.
 * @example
 * ```
 * pointInPolygon([0, 0, 10, 0, 10, 10, 0, 10], { x: 5, y: 5 }); // true
 * ```
 */
function pointInPolygon(points: number[], p: Point): boolean {
  let inside = false;
  const n = points.length / 2;
  for (let i = 0, j = n - 1; i < n; j = i++) {
    const xi = points[i * 2];
    const yi = points[i * 2 + 1];
    const xj = points[j * 2];
    const yj = points[j * 2 + 1];
    if (yi > p.y !== yj > p.y && p.x < ((xj - xi) * (p.y - yi)) / (yj - yi) + xi) {
      inside = !inside;
    }
  }
  return inside;
}

/** Nearest distance from `p` to any consecutive-point segment of an OPEN flat
 * `[x0,y0,x1,y1,…]` polyline (no implicit closing edge), via `pointSegDistance`.
 * @param points Flat `[x0,y0,x1,y1,…]` polyline points.
 * @param p The point to measure from (scene coords).
 * @returns The smallest per-segment distance, or `Infinity` for a degenerate (fewer than 2
 * points) polyline.
 * @example
 * ```
 * distanceToPolyline([0, 0, 10, 0], { x: 5, y: 3 }); // 3
 * ```
 */
function distanceToPolyline(points: number[], p: Point): number {
  let best = Infinity;
  for (let i = 0; i + 3 < points.length; i += 2) {
    const a = { x: points[i], y: points[i + 1] };
    const b = { x: points[i + 2], y: points[i + 3] };
    best = Math.min(best, pointSegDistance(p, a, b));
  }
  return best;
}

/** Shared picking body for region/drawing/template docs: each doc converts to a
 * `ShapeNodeSpec` via `toSpec` (`regionShapeSpec`/`drawingShapeSpec`/`templateShapeSpec` — the
 * SAME tessellation the corresponding view draws from, never a forked copy of the shape math),
 * then is hit-tested by its OWN `closed` flag — a doc list can mix both (a drawing/template doc
 * can be an open polyline or a closed fill depending on its authored `shape.kind`). **Closed**
 * shapes use `pointInPolygon` and are picked "topmost wins": any later (higher z / more
 * recently created — the same iteration-order convention `topTokenAt` uses) containing doc
 * unconditionally overwrites an earlier hit, since a filled area is a genuine containment click.
 * **Open** shapes use `distanceToPolyline` and are picked "nearest wins within tolerance" (the
 * same convention `topWallAt`/`topLightAt` use), since a click near a bare line has no
 * meaningful z-stacking to prefer. A closed-shape hit always takes priority over an open-shape
 * hit found earlier in the same pass (an area click is a stronger signal than a near-miss on a
 * line) — but never over one found LATER, so a later closed doc still overwrites per its own
 * topmost-wins rule.
 * @param docs The candidate documents (typically `store.query(docType)`, scene-scoped).
 * @param p The point to test (scene coords).
 * @param toSpec Converts a doc to its `ShapeNodeSpec`, or `null` for an unrenderable doc.
 * @returns The picked doc's id, or `null` when none matches.
 * @example
 * ```
 * // module-private; not exported from @shadowcat/scene-tools
 * declare const docs: WireDocument[];
 * declare const p: Point;
 * topShapeAt(docs, p, regionShapeSpec);
 * ```
 */
function topShapeAt(docs: WireDocument[], p: Point, toSpec: (doc: WireDocument) => ShapeNodeSpec | null): string | null {
  let closedHit: string | null = null;
  let openHit: string | null = null;
  let openBest = SHAPE_PICK_TOLERANCE;
  for (const doc of docs) {
    const spec = toSpec(doc);
    if (!spec) continue;
    if (spec.closed) {
      if (pointInPolygon(spec.points, p)) closedHit = doc.id;
    } else {
      const d = distanceToPolyline(spec.points, p);
      if (d <= openBest) {
        openBest = d;
        openHit = doc.id;
      }
    }
  }
  return closedHit ?? openHit;
}

/** The id of the topmost region whose geometry contains `p`, or `null`. See `topShapeAt` for
 * the shared picking rule; region shapes (`rect`/`circle`/`polygon`) are always closed, so this
 * always resolves via `pointInPolygon`. Cull the candidate list to the viewed scene before calling (callers pass
 * `parent_id`-filtered docs).
 * @param regions The candidate region documents (typically `store.query("region")`, scene-scoped).
 * @param p The point to test (scene coords).
 * @returns The picked region's id, or `null` when none matches.
 * @example
 * ```
 * declare const store: ReadableDocuments;
 * declare const p: Point;
 * const id = topRegionAt(store.query("region"), p);
 * ```
 */
export function topRegionAt(regions: WireDocument[], p: Point): string | null {
  return topShapeAt(regions, p, regionShapeSpec);
}

/** The id of the topmost drawing whose geometry contains `p` (closed `rect`/`ellipse`/`polygon`),
 * or that passes within tolerance of `p` (open `freehand`/`line`), or `null`. See `topShapeAt`
 * for the shared picking rule. Cull the candidate list to the viewed scene before calling
 * (callers pass `parent_id`-filtered docs).
 * @param drawings The candidate drawing documents (typically `store.query("drawing")`, scene-scoped).
 * @param p The point to test (scene coords).
 * @returns The picked drawing's id, or `null` when none matches.
 * @example
 * ```
 * declare const store: ReadableDocuments;
 * declare const p: Point;
 * const id = topDrawingAt(store.query("drawing"), p);
 * ```
 */
export function topDrawingAt(drawings: WireDocument[], p: Point): string | null {
  return topShapeAt(drawings, p, drawingShapeSpec);
}

/** The id of the topmost template whose geometry contains `p` (closed `circle`/`cone`/`rect`),
 * or that passes within tolerance of `p` (open `line`), or `null`. See `topShapeAt` for the
 * shared picking rule. Cull the candidate list to the viewed scene before calling (callers pass
 * `parent_id`-filtered docs).
 * @param templates The candidate template documents (typically `store.query("template")`, scene-scoped).
 * @param p The point to test (scene coords).
 * @returns The picked template's id, or `null` when none matches.
 * @example
 * ```
 * declare const store: ReadableDocuments;
 * declare const p: Point;
 * const id = topTemplateAt(store.query("template"), p);
 * ```
 */
export function topTemplateAt(templates: WireDocument[], p: Point): string | null {
  return topShapeAt(templates, p, templateShapeSpec);
}
