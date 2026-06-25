import type { Point, LineSeg } from "./types";

export type GridKind = "square" | "hex";

/** Cost rule for diagonal movement on square grids. Mirrors `DiagonalRule` in
 * `scene-docs.ts` and the server's `pathfinding.rs` `DiagonalRule` enum — the
 * distance metric must match the server's A* cost exactly. */
export type DiagonalRule = "chebyshev" | "manhattan" | "euclidean" | "alternating";

export interface GridSpec {
  /** "square": `size` = edge length. "hex": `size` = outer radius. */
  kind: GridKind;
  size: number;
  /** Square grids only. Diagonal cost rule for `distance()`. Defaults to `"chebyshev"`.
   * Source: the world-settings `pathfinding.diagonalRule` resolved via `resolveSceneSettings`. */
  diagonalRule?: DiagonalRule;
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

  /** Whole-cell distance between two scene points.
   * Hex: axial distance (`col`/`row` are axial q/r).
   * Square: selected by `spec.diagonalRule` (default `"chebyshev"`):
   *   - chebyshev  — max(dCol, dRow) (chessboard / 1-per-diagonal).
   *   - manhattan  — dCol + dRow (no diagonal shortcuts).
   *   - euclidean  — (dmax−dmin) + √2·dmin (true Euclidean cell distance).
   *   - alternating — (dmax−dmin) + dmin + floor(dmin/2) (5-10-5: diagonals cost 1,2,1,2…).
   * All four mirror the server's per-rule costs in `scene/pathfinding.rs`. */
  distance(a: Point, b: Point): number {
    const ca = this.cellOf(a);
    const cb = this.cellOf(b);
    const dCol = Math.abs(cb.col - ca.col);
    const dRow = Math.abs(cb.row - ca.row);
    if (this.spec.kind !== "square") {
      // Hex axial distance needs signed deltas for the cube-coordinate formula.
      const sCol = cb.col - ca.col;
      const sRow = cb.row - ca.row;
      return (Math.abs(sCol) + Math.abs(sRow) + Math.abs(sCol + sRow)) / 2;
    }
    const dmax = Math.max(dCol, dRow);
    const dmin = Math.min(dCol, dRow);
    switch (this.spec.diagonalRule ?? "chebyshev") {
      case "manhattan":   return dCol + dRow;
      case "euclidean":   return (dmax - dmin) + Math.SQRT2 * dmin;
      // Diagonals alternate cost 1, 2, 1, 2 … (5-10-5 rule). dmin diagonals cost
      // dmin + floor(dmin/2); the remainder (dmax−dmin) are orthogonal at cost 1 each.
      case "alternating": return (dmax - dmin) + dmin + Math.floor(dmin / 2);
      default:            return dmax; // chebyshev
    }
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
    // Integer cell indexing rather than float accumulation: exact under the
    // non-integer scene rects a panned/zoomed camera produces (screenToScene
    // divides by scale), so the edge line never flickers on/off from FP drift.
    const cxLo = Math.floor(rect.x / s);
    const cxHi = Math.ceil((rect.x + rect.w) / s);
    for (let i = cxLo; i <= cxHi; i++) {
      const x = i * s;
      out.push({ x1: x, y1: rect.y, x2: x, y2: rect.y + rect.h });
    }
    const cyLo = Math.floor(rect.y / s);
    const cyHi = Math.ceil((rect.y + rect.h) / s);
    for (let i = cyLo; i <= cyHi; i++) {
      const y = i * s;
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
