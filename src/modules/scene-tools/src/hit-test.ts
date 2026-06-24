import type { WireDocument, ReadableDocuments } from "@shadowcat/core";
import { resolveTokenBox } from "@shadowcat/core";
import type { Point } from "@shadowcat/render";

/** The id of the topmost token whose footprint contains `p`, or null. "Topmost" is the last in
 * document order (render z-order). Footprint = the resolved box (M10d): a circle token uses
 * ellipse containment, a square the AABB. Rotation is ignored for picking. */
export function topTokenAt(tokens: WireDocument[], p: Point, store: ReadableDocuments): string | null {
  let hit: string | null = null;
  for (const t of tokens) {
    const box = resolveTokenBox(t, store);
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
