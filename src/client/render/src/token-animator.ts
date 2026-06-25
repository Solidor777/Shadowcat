import type { TokenTransform } from "./types";
import { applyEasing, type EasingMode } from "./easing";

/** Below this distance (px) a component is treated as coincident. */
const EPSILON = 0.01;

/** Animation tuning resolved from `world-settings.animation` + the active grid. */
export interface AnimationConfig {
  speedCellsPerSec: number;
  easing: EasingMode;
  /** Pixels per grid cell (grid.size); converts pixel distance to cells for duration. */
  cellSize: number;
}

const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;

interface Anim {
  /** Polyline in scene px; `poly[0]` is the start captured at (re)target time. */
  poly: [number, number][];
  segLen: number[];
  total: number;
  elapsed: number;
  duration: number;
  startRot: number;
  finalRot: number;
  easing: EasingMode;
  /** True for an explicit route walk; gates the optimistic-vertex ignore rule. */
  pathDriven: boolean;
  /** Lowest vertex index the walk is still heading toward (monotonic). */
  segIndex: number;
}

/** Holds each token's rendered transform and advances it toward the document-authoritative
 * target along an eased polyline. Duration = pathDistanceCells / speedCellsPerSec.
 * New tokens snap; moves tween; a newer authoritative position retargets in place. */
export class TokenAnimator {
  private cur = new Map<string, TokenTransform>();
  private anim = new Map<string, Anim>();
  private cfg: AnimationConfig = { speedCellsPerSec: 6, easing: "easeInOut", cellSize: 100 };

  setConfig(cfg: AnimationConfig): void {
    this.cfg = cfg;
  }
  has(id: string): boolean {
    return this.cur.has(id);
  }
  get(id: string): TokenTransform | undefined {
    return this.cur.get(id);
  }
  remove(id: string): void {
    this.cur.delete(id);
    this.anim.delete(id);
  }

  setTarget(id: string, t: TokenTransform): void {
    const c = this.cur.get(id);
    if (!c) {
      this.cur.set(id, { ...t }); // brand-new → snap
      return;
    }
    const active = this.anim.get(id);
    if (active?.pathDriven) {
      // Expected optimistic progress: a target matching a path vertex at-or-ahead of the
      // current segment is the authoritative store catching up to where we already walk —
      // keep the smooth walk. Anything else (foreign mover / backward rollback) interrupts.
      for (let i = active.segIndex; i < active.poly.length; i++) {
        const v = active.poly[i];
        if (Math.abs(v[0] - t.x) < EPSILON && Math.abs(v[1] - t.y) < EPSILON) {
          active.finalRot = t.rotation; // adopt authoritative rotation as the settle value
          return;
        }
      }
    }
    this.startAnim(id, c, [[c.x, c.y], [t.x, t.y]], t.rotation, false);
  }

  animateAlongPath(id: string, path: [number, number][], rotation: number): void {
    const c = this.cur.get(id);
    if (!c) {
      const last = path[path.length - 1] ?? [0, 0];
      this.cur.set(id, { x: last[0], y: last[1], rotation }); // no prior render → snap
      return;
    }
    // Dedupe consecutive coincident points; anchor the walk at the live current position.
    const pts: [number, number][] = [[c.x, c.y]];
    for (const p of path) {
      const prev = pts[pts.length - 1];
      if (Math.abs(prev[0] - p[0]) >= EPSILON || Math.abs(prev[1] - p[1]) >= EPSILON) pts.push([p[0], p[1]]);
    }
    if (pts.length < 2) {
      this.setTarget(id, { x: pts[0][0], y: pts[0][1], rotation });
      return;
    }
    this.startAnim(id, c, pts, rotation, true);
  }

  private startAnim(id: string, c: TokenTransform, poly: [number, number][], finalRot: number, pathDriven: boolean): void {
    const segLen: number[] = [];
    let total = 0;
    for (let i = 1; i < poly.length; i++) {
      const dx = poly[i][0] - poly[i - 1][0];
      const dy = poly[i][1] - poly[i - 1][1];
      const len = Math.hypot(dx, dy);
      segLen.push(len);
      total += len;
    }
    const last = poly[poly.length - 1];
    if (total < EPSILON || this.cfg.cellSize <= 0 || this.cfg.speedCellsPerSec <= 0) {
      this.cur.set(id, { x: last[0], y: last[1], rotation: finalRot }); // degenerate → snap
      this.anim.delete(id);
      return;
    }
    const cells = total / this.cfg.cellSize;
    this.anim.set(id, {
      poly, segLen, total, elapsed: 0,
      duration: (cells / this.cfg.speedCellsPerSec) * 1000,
      startRot: c.rotation, finalRot, easing: this.cfg.easing,
      pathDriven, segIndex: 0,
    });
  }

  /** Advance all animations by `dtMs`; return ids whose transform changed. */
  tick(dtMs: number): string[] {
    const moved: string[] = [];
    for (const [id, a] of this.anim) {
      a.elapsed += dtMs;
      const tRaw = Math.min(1, a.elapsed / a.duration);
      const e = applyEasing(a.easing, tRaw);
      const target = e * a.total;
      // Walk segments to the eased distance.
      let acc = 0;
      let pos: [number, number] = a.poly[a.poly.length - 1];
      let idx = a.poly.length - 1;
      for (let i = 0; i < a.segLen.length; i++) {
        if (target <= acc + a.segLen[i] || i === a.segLen.length - 1) {
          const f = a.segLen[i] > 0 ? Math.min(1, (target - acc) / a.segLen[i]) : 1;
          pos = [lerp(a.poly[i][0], a.poly[i + 1][0], f), lerp(a.poly[i][1], a.poly[i + 1][1], f)];
          idx = f >= 1 ? i + 1 : i;
          break;
        }
        acc += a.segLen[i];
      }
      a.segIndex = idx; // monotonic-ish progress marker for the ignore rule
      const cur = this.cur.get(id)!;
      cur.x = pos[0];
      cur.y = pos[1];
      cur.rotation = lerp(a.startRot, a.finalRot, e);
      moved.push(id);
      if (tRaw >= 1) {
        const last = a.poly[a.poly.length - 1];
        cur.x = last[0];
        cur.y = last[1];
        cur.rotation = a.finalRot;
        this.anim.delete(id);
      }
    }
    return moved;
  }
}
