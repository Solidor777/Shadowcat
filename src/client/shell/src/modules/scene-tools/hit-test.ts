import type { WireDocument } from "@shadowcat/core";
import type { Point } from "@shadowcat/render";

/** Token bounds the hit-test uses: an axis-aligned box centered on `(x,y)` (the token
 * center, M8d §4) with half-extents `w/2`,`h/2`. Rotation is ignored for picking in M8d. */
interface TokenBox {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** The id of the topmost token whose box contains `p`, or null. "Topmost" is the last in
 * document order (render z-order), so overlapping tokens pick the one drawn on top. */
export function topTokenAt(tokens: WireDocument[], p: Point): string | null {
  let hit: string | null = null;
  for (const t of tokens) {
    const s = t.system as Partial<TokenBox> | undefined;
    if (!s || s.x == null || s.y == null || s.w == null || s.h == null) continue;
    if (Math.abs(p.x - s.x) <= s.w / 2 && Math.abs(p.y - s.y) <= s.h / 2) hit = t.id;
  }
  return hit;
}
