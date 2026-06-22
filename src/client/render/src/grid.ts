import type { Point, LineSeg } from "./types";

export type GridKind = "square" | "hex";

export interface GridSpec {
  /** "square": `size` = edge length. "hex": `size` = outer radius. */
  kind: GridKind;
  size: number;
}

interface SceneRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Engine-owned grid model + coordinate math (square + pointy-top hex). Pure: the
 * engine draws `lines(...)` into the grid layer and uses `snap`/`cellOf` for
 * placement (M8d). Hex uses axial coords (Red Blob Games). */
export class Grid {
  constructor(private readonly spec: GridSpec) {}

  snap(p: Point): Point {
    if (this.spec.kind === "square") {
      const { col, row } = this.cellOf(p);
      const s = this.spec.size;
      return { x: col * s + s / 2, y: row * s + s / 2 };
    }
    const { q, r } = this.axialRound(this.pixelToAxial(p));
    return this.axialToPixel(q, r);
  }

  cellOf(p: Point): { col: number; row: number } {
    if (this.spec.kind === "square") {
      return {
        col: Math.floor(p.x / this.spec.size),
        row: Math.floor(p.y / this.spec.size),
      };
    }
    const { q, r } = this.axialRound(this.pixelToAxial(p));
    return { col: q, row: r };
  }

  lines(rect: SceneRect): LineSeg[] {
    return this.spec.kind === "square"
      ? this.squareLines(rect)
      : this.hexLines(rect);
  }

  private squareLines(rect: SceneRect): LineSeg[] {
    const s = this.spec.size;
    const out: LineSeg[] = [];
    const x0 = Math.floor(rect.x / s) * s;
    const y0 = Math.floor(rect.y / s) * s;
    for (let x = x0; x <= rect.x + rect.w; x += s) {
      out.push({ x1: x, y1: rect.y, x2: x, y2: rect.y + rect.h });
    }
    for (let y = y0; y <= rect.y + rect.h; y += s) {
      out.push({ x1: rect.x, y1: y, x2: rect.x + rect.w, y2: y });
    }
    return out;
  }

  // --- pointy-top axial hex (Red Blob Games) ---
  // radius = size; width = sqrt(3)*size, height = 2*size; rows offset by height*3/4.
  private pixelToAxial(p: Point): { q: number; r: number } {
    const size = this.spec.size;
    const q = ((Math.sqrt(3) / 3) * p.x - (1 / 3) * p.y) / size;
    const r = ((2 / 3) * p.y) / size;
    return { q, r };
  }

  private axialToPixel(q: number, r: number): Point {
    const size = this.spec.size;
    return {
      x: size * (Math.sqrt(3) * q + (Math.sqrt(3) / 2) * r),
      y: size * (3 / 2) * r,
    };
  }

  private axialRound(a: { q: number; r: number }): { q: number; r: number } {
    // Round in cube space then fix the largest-drift component.
    let rx = Math.round(a.q);
    let ry = Math.round(-a.q - a.r);
    let rz = Math.round(a.r);
    const dx = Math.abs(rx - a.q);
    const dy = Math.abs(ry - (-a.q - a.r));
    const dz = Math.abs(rz - a.r);
    if (dx > dy && dx > dz) rx = -ry - rz;
    else if (dy > dz) ry = -rx - rz;
    else rz = -rx - ry;
    return { q: rx, r: rz };
  }

  private hexLines(rect: SceneRect): LineSeg[] {
    // Draw each hex outline whose center falls in (a margin around) the rect. The
    // overlap between adjacent hexes is acceptable for a grid overlay.
    const size = this.spec.size;
    const out: LineSeg[] = [];
    const margin = size * 2;
    const minA = this.pixelToAxial({ x: rect.x - margin, y: rect.y - margin });
    const maxA = this.pixelToAxial({ x: rect.x + rect.w + margin, y: rect.y + rect.h + margin });
    const qLo = Math.floor(Math.min(minA.q, maxA.q)) - 1;
    const qHi = Math.ceil(Math.max(minA.q, maxA.q)) + 1;
    const rLo = Math.floor(Math.min(minA.r, maxA.r)) - 1;
    const rHi = Math.ceil(Math.max(minA.r, maxA.r)) + 1;
    for (let r = rLo; r <= rHi; r++) {
      for (let q = qLo; q <= qHi; q++) {
        const c = this.axialToPixel(q, r);
        const pts: Point[] = [];
        for (let i = 0; i < 6; i++) {
          const ang = (Math.PI / 180) * (60 * i - 30); // pointy-top
          pts.push({ x: c.x + size * Math.cos(ang), y: c.y + size * Math.sin(ang) });
        }
        for (let i = 0; i < 6; i++) {
          const a = pts[i];
          const b = pts[(i + 1) % 6];
          out.push({ x1: a.x, y1: a.y, x2: b.x, y2: b.y });
        }
      }
    }
    return out;
  }
}
