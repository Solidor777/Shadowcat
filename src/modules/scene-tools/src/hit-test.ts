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
