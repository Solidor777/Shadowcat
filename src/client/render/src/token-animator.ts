import type { TokenTransform } from "./types";

/** Exponential-smoothing factor: a tick of `SMOOTH_MS` (or more) fully settles. */
const SMOOTH_MS = 120;
/** Below this distance a component snaps exactly to target (kills float drift). */
const EPSILON = 0.01;

const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;
const near = (a: TokenTransform, b: TokenTransform): boolean =>
  Math.abs(a.x - b.x) < EPSILON && Math.abs(a.y - b.y) < EPSILON && Math.abs(a.rotation - b.rotation) < EPSILON;

/** Pure tween model: holds each token's current rendered transform and advances it toward
 * the document-authoritative target. New tokens snap (no tween); moves smooth in. */
export class TokenAnimator {
  private cur = new Map<string, TokenTransform>();
  private tgt = new Map<string, TokenTransform>();

  has(id: string): boolean {
    return this.cur.has(id);
  }
  get(id: string): TokenTransform | undefined {
    return this.cur.get(id);
  }
  setTarget(id: string, t: TokenTransform): void {
    if (!this.cur.has(id)) this.cur.set(id, { ...t }); // brand-new → snap into place
    this.tgt.set(id, { ...t });
  }
  remove(id: string): void {
    this.cur.delete(id);
    this.tgt.delete(id);
  }
  /** Advance all tweens by `dtMs`; return ids whose current transform changed. */
  tick(dtMs: number): string[] {
    const moved: string[] = [];
    const alpha = Math.min(1, dtMs / SMOOTH_MS);
    for (const [id, c] of this.cur) {
      const t = this.tgt.get(id);
      if (!t || near(c, t)) continue;
      c.x = lerp(c.x, t.x, alpha);
      c.y = lerp(c.y, t.y, alpha);
      c.rotation = lerp(c.rotation, t.rotation, alpha);
      if (near(c, t)) { c.x = t.x; c.y = t.y; c.rotation = t.rotation; } // settle exactly
      moved.push(id);
    }
    return moved;
  }
}
