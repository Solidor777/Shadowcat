import type { Point, LineSeg } from "./types";

/** The two supported grid geometries; a `Grid` instance is fixed to one for its lifetime. */
export type GridKind = "square" | "hex";

/** Cost rule for diagonal movement on square grids. Mirrors the client `DiagonalRule` type
 * and the server's `DiagonalRule` enum — see
 * {@link Grid.distance} for the exact per-rule formulas and the scope of the parity
 * they match (unweighted routes; terrain regions can raise the server's real cost
 * above what this client-side distance reports). */
export type DiagonalRule = "chebyshev" | "manhattan" | "euclidean" | "alternating";

/** A grid's fixed geometry: kind, cell size, and (square-only) diagonal-cost rule. */
export interface GridSpec {
  /** "square": `size` = edge length. "hex": `size` = outer radius. */
  kind: GridKind;
  /** Cell edge length (square) or outer radius/circumradius (hex), in scene (px) units. */
  size: number;
  /** Square grids only. Diagonal cost rule for `distance()`. Defaults to `"chebyshev"`.
   * Source: the world-settings `pathfinding.diagonalRule` resolved via `resolveSceneSettings`. */
  diagonalRule?: DiagonalRule;
}

/** A scene-coordinate rectangle (e.g. the visible viewport) to cover with grid lines. */
interface SceneRect {
  /** Left edge, scene x-coordinate. */
  x: number;
  /** Top edge, scene y-coordinate. */
  y: number;
  /** Width, in scene (px) units. */
  w: number;
  /** Height, in scene (px) units. */
  h: number;
}

/** Engine-owned grid model + coordinate math (square + pointy-top hex). Pure: the
 * engine draws `lines(...)` into the grid layer and uses `snap`/`cellOf` for
 * placement. Hex uses axial coords (Red Blob Games). */
export class Grid {
  /**
   * Constructs a grid from a fixed spec — the grid's kind, size, and diagonal rule
   * never change over the instance's lifetime; a new spec means a new `Grid`.
   * @param spec The grid's kind, cell size, and (square-only) diagonal-cost rule.
   * @example
   * ```ts
   * import { Grid } from "@shadowcat/render";
   *
   * const grid = new Grid({ kind: "square", size: 70 });
   * ```
   */
  constructor(private readonly spec: GridSpec) {}

  /**
   * Snaps a scene point to the active grid's nearest CELL CENTER — never a
   * vertex/corner, on either grid kind. Square: the containing cell's center. Hex: the
   * nearest hex's center, via `axialRound`-then-`axialToPixel` — the same
   * `axialToPixel` call `hexLines` uses as the origin it draws the six corners
   * around, so this is provably a center, not a vertex.
   * @param p A scene-coordinate point.
   * @returns `p` snapped to the nearest cell center.
   * @example
   * ```ts
   * import { Grid } from "@shadowcat/render";
   *
   * const grid = new Grid({ kind: "square", size: 70 });
   * grid.snap({ x: 12, y: 34 }); // { x: 35, y: 35 }
   * ```
   */
  snap(p: Point): Point {
    if (this.spec.kind === "square") {
      const { col, row } = this.cellOf(p);
      const s = this.spec.size;
      return { x: col * s + s / 2, y: row * s + s / 2 };
    }
    const { q, r } = this.axialRound(this.pixelToAxial(p));
    return this.axialToPixel(q, r);
  }

  /**
   * The world distance between two adjacent cell centres — `size` on square (the edge length IS
   * the center-to-center step), `size * sqrt(3)` on hex (every axial neighbour sits `sqrt(3) ·
   * size` away; {@link axialToPixel} confirms this for the six unit-axial offsets). This is NOT
   * the grid's indexing scale (`spec.size`, the square edge length or hex outer radius/
   * circumradius) — the two coincide on square grids and diverge by a factor of `sqrt(3)` on hex,
   * which is why a caller converting a travelled world distance into a cell count must use this
   * method, never `spec.size` directly. Mirrors the server's `GridShape::world_units_per_cell`
   * (same quantity, same name in the client's casing) so the two conventions are greppable as one
   * concept.
   * @returns The world distance spanned by one grid step, in scene (px) units.
   * @example
   * ```ts
   * import { Grid } from "@shadowcat/render";
   *
   * const grid = new Grid({ kind: "hex", size: 100 });
   * grid.worldUnitsPerCell(); // 173.20508075688772 (100 * sqrt(3))
   * ```
   */
  worldUnitsPerCell(): number {
    return this.spec.kind === "square" ? this.spec.size : this.spec.size * Math.sqrt(3);
  }

  /** Whole-cell distance between two scene points.
   * Hex: axial distance (`col`/`row` are axial q/r).
   * Square: selected by `spec.diagonalRule` (default `"chebyshev"`):
   *   - chebyshev  — max(dCol, dRow) (chessboard / 1-per-diagonal).
   *   - manhattan  — dCol + dRow (no diagonal shortcuts).
   *   - euclidean  — (dmax−dmin) + √2·dmin (true Euclidean cell distance).
   *   - alternating — (dmax−dmin) + dmin + floor(dmin/2) (5-10-5: diagonals cost 1,2,1,2…,
   *     starting at parity 0 — this closed form is the parity-0 case only; a hypothetical
   *     variant carrying parity across chained waypoint legs would need the general
   *     recurrence instead, but this method always measures a single fresh pair, so parity
   *     0 is the only case that applies here).
   * All four mirror the server's per-rule step costs: chebyshev/manhattan/euclidean
   * match `pathfinding::heuristic` exactly (an admissible AND tight bound for a
   * direct route with no obstacles AND no terrain weighting — that function's
   * ADMISSIBILITY WITH TERRAIN note records that it bounds only the UNWEIGHTED step
   * cost, so a terrain region's `terrain_multiplier > 1.0` can raise the server's real
   * A* cost above what this client-side `distance()` reports, even on an otherwise-direct,
   * wall-free route); alternating matches `grid_shape::step_cost`'s 1,2,1,2… parity-threaded
   * diagonal cost starting at parity 0, whose closed-form sum over `dmin` diagonal
   * steps is `dmin + floor(dmin/2)` (odd `dmin` diverges from this if the parity
   * sequence didn't start fresh; even `dmin` agrees regardless of starting parity).
   * @param a One scene-coordinate point.
   * @param b The other scene-coordinate point.
   * @returns The whole-cell distance between `a` and `b`, per the rule above.
   * @example
   * ```ts
   * import { Grid } from "@shadowcat/render";
   *
   * const grid = new Grid({ kind: "square", size: 70 });
   * grid.distance({ x: 0, y: 0 }, { x: 140, y: 70 }); // 2 (chebyshev)
   * ```
   */
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

  /**
   * The integer cell containing `p`. Square: `floor(x/size), floor(y/size)`. Hex: the
   * nearest hex's axial `(q,r)` (`col`/`row` alias `q`/`r`) — a hex "contains" `p` when
   * `p` is closer to that hex's center than to any other, so this is a rounded
   * nearest-center lookup, not a floor division.
   * @param p A scene-coordinate point.
   * @returns The containing cell as `{col, row}` (square indices, or hex axial `q`/`r`).
   * @example
   * ```ts
   * import { Grid } from "@shadowcat/render";
   *
   * const grid = new Grid({ kind: "square", size: 70 });
   * grid.cellOf({ x: 12, y: 34 }); // { col: 0, row: 0 }
   * ```
   */
  cellOf(p: Point): {
    /** Square column index, or hex axial q. */
    col: number;
    /** Square row index, or hex axial r. */
    row: number;
  } {
    if (this.spec.kind === "square") {
      return {
        col: Math.floor(p.x / this.spec.size),
        row: Math.floor(p.y / this.spec.size),
      };
    }
    const { q, r } = this.axialRound(this.pixelToAxial(p));
    return { col: q, row: r };
  }

  /**
   * The scene-coordinate CENTER of the cell at `(col, row)` — square column/row indices, or hex
   * axial `q`/`r` (mirrors {@link cellOf}'s return shape). Square: `col*size+size/2,
   * row*size+size/2`. Hex: {@link axialToPixel}, the same call `snap`'s hex branch and {@link
   * hexLines} both already use to locate a hex's center — this promotes that private call onto
   * the public surface rather than a second formula.
   * @param col Square column index, or hex axial q.
   * @param row Square row index, or hex axial r.
   * @returns The cell's center, in scene coordinates.
   * @example
   * ```ts
   * import { Grid } from "@shadowcat/render";
   *
   * const grid = new Grid({ kind: "square", size: 100 });
   * grid.cellCenter(2, 0); // { x: 250, y: 50 }
   * ```
   */
  cellCenter(col: number, row: number): Point {
    if (this.spec.kind === "square") {
      const s = this.spec.size;
      return { x: col * s + s / 2, y: row * s + s / 2 };
    }
    return this.axialToPixel(col, row);
  }

  /**
   * The scene-coordinate CORNERS of the cell at `(col, row)`, in draw order — square column/row
   * indices, or hex axial `q`/`r`. Square: the 4 axis-aligned corners of a `size`-edge rect
   * anchored at `(col*size, row*size)`. Hex: the 6 corners of the pointy-top hexagon centered on
   * {@link cellCenter}, using the SAME per-corner angle formula {@link hexLines} draws its
   * outlines with — {@link hexLines} calls this method rather than recomputing the corners a
   * second time, so there is exactly one hex-corner formula in this class.
   * @param col Square column index, or hex axial q.
   * @param row Square row index, or hex axial r.
   * @returns The cell's corner points, in draw order (a closed polygon; the last point does not
   * repeat the first).
   * @example
   * ```ts
   * import { Grid } from "@shadowcat/render";
   *
   * const grid = new Grid({ kind: "square", size: 100 });
   * grid.cellCorners(0, 0); // [{x:0,y:0},{x:100,y:0},{x:100,y:100},{x:0,y:100}]
   * ```
   */
  cellCorners(col: number, row: number): Point[] {
    if (this.spec.kind === "square") {
      const s = this.spec.size;
      const x0 = col * s, y0 = row * s;
      return [
        { x: x0, y: y0 },
        { x: x0 + s, y: y0 },
        { x: x0 + s, y: y0 + s },
        { x: x0, y: y0 + s },
      ];
    }
    const c = this.axialToPixel(col, row);
    const size = this.spec.size;
    const pts: Point[] = [];
    for (let i = 0; i < 6; i++) {
      const ang = (Math.PI / 180) * (60 * i - 30); // pointy-top
      pts.push({ x: c.x + size * Math.cos(ang), y: c.y + size * Math.sin(ang) });
    }
    return pts;
  }

  /**
   * Grid-overlay line segments covering `rect` (plus a margin), for the grid render
   * layer to draw. Dispatches on `spec.kind` — square and hex never share a code path.
   * @param rect The visible scene rectangle to cover.
   * @returns The grid line segments to draw.
   * @example
   * ```ts
   * import { Grid } from "@shadowcat/render";
   *
   * const grid = new Grid({ kind: "square", size: 70 });
   * grid.lines({ x: 0, y: 0, w: 700, h: 700 });
   * ```
   */
  lines(rect: SceneRect): LineSeg[] {
    return this.spec.kind === "square"
      ? this.squareLines(rect)
      : this.hexLines(rect);
  }

  /**
   * Square-grid line segments: one vertical line per column boundary, one horizontal
   * line per row boundary, spanning `rect`.
   * @param rect The visible scene rectangle to cover.
   * @returns The square grid's line segments.
   * @example
   * ```
   * // private — not constructible/callable outside Grid.
   * ```
   */
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
  /**
   * Pixel → fractional axial `(q,r)`, pointy-top orientation (Red Blob Games' hex grid
   * reference). Fractional: the caller rounds via {@link axialRound} when an integer
   * cell is needed — this function alone does not identify a specific hex.
   * @param p A scene-coordinate point.
   * @returns The point's fractional axial coordinates.
   * @example
   * ```
   * // private — not constructible/callable outside Grid.
   * ```
   */
  private pixelToAxial(p: Point): {
    /** Fractional axial q. */
    q: number;
    /** Fractional axial r. */
    r: number;
  } {
    const size = this.spec.size;
    const q = ((Math.sqrt(3) / 3) * p.x - (1 / 3) * p.y) / size;
    const r = ((2 / 3) * p.y) / size;
    return { q, r };
  }

  /**
   * Axial `(q,r)` → the CENTER pixel of that hex, pointy-top orientation. {@link
   * hexLines} calls this to get each hex's center, then generates its six corners at
   * `size` (the circumradius) around it — the same call this function makes proves
   * `snap`'s hex branch also returns a center, never a vertex.
   * @param q Axial q.
   * @param r Axial r.
   * @returns The hex's center, in scene coordinates.
   * @example
   * ```
   * // private — not constructible/callable outside Grid.
   * ```
   */
  private axialToPixel(q: number, r: number): Point {
    const size = this.spec.size;
    return {
      x: size * (Math.sqrt(3) * q + (Math.sqrt(3) / 2) * r),
      y: size * (3 / 2) * r,
    };
  }

  /**
   * Rounds fractional axial coordinates to the nearest integer hex (Red Blob Games'
   * cube-rounding algorithm): converts to cube coordinates `(x,y,z) = (q, -q-r, r)`,
   * rounds each independently, then recomputes whichever component drifted furthest
   * from its rounded value so the `x+y+z=0` cube invariant holds exactly.
   * @param a Fractional axial coordinates (as returned by {@link pixelToAxial}).
   * @param a.q Fractional axial q.
   * @param a.r Fractional axial r.
   * @returns The nearest integer axial `(q,r)`.
   * @example
   * ```
   * // private — not constructible/callable outside Grid.
   * ```
   */
  private axialRound(a: {
    /** Fractional axial q. */
    q: number;
    /** Fractional axial r. */
    r: number;
  }): {
    /** Nearest integer axial q. */
    q: number;
    /** Nearest integer axial r. */
    r: number;
  } {
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

  /**
   * Hex-grid line segments: draws the six-edge outline of every hex whose center falls
   * within `rect` plus a margin. Covers ALL FOUR corners of `rect` when computing the
   * axial `q`/`r` search bounds — not just two opposite corners — because
   * {@link pixelToAxial}'s `q = (√3/3·x − 1/3·y)/size` mixes x and y with OPPOSITE
   * signs, so q's extrema sit on the top-right/bottom-left diagonal while r (a
   * function of y alone) peaks on the other diagonal; sampling only one diagonal
   * understates q's true range and silently drops in-viewport hexes. Adjacent hex
   * outlines overlap (each edge is drawn twice,
   * once per flanking hex) — acceptable for a grid overlay, not deduplicated.
   * @param rect The visible scene rectangle to cover.
   * @returns The hex grid's line segments.
   * @example
   * ```
   * // private — not constructible/callable outside Grid.
   * ```
   */
  private hexLines(rect: SceneRect): LineSeg[] {
    // Draw each hex outline whose center falls in (a margin around) the rect. The
    // overlap between adjacent hexes is acceptable for a grid overlay.
    const size = this.spec.size;
    const out: LineSeg[] = [];
    const margin = size * 2;
    // Sample ALL FOUR corners: `pixelToAxial`'s q mixes x and y with OPPOSITE signs
    // (`(√3/3·x − 1/3·y)/size`), so q's extrema fall on the top-right/bottom-left
    // diagonal while r (a function of y alone) peaks on the other. Sampling one
    // diagonal only understates the q-range and leaves undrawn hexes inside the
    // viewport — worse the smaller `size` is relative to the rect.
    const x0 = rect.x - margin;
    const y0 = rect.y - margin;
    const x1 = rect.x + rect.w + margin;
    const y1 = rect.y + rect.h + margin;
    const corners = [
      this.pixelToAxial({ x: x0, y: y0 }),
      this.pixelToAxial({ x: x1, y: y0 }),
      this.pixelToAxial({ x: x0, y: y1 }),
      this.pixelToAxial({ x: x1, y: y1 }),
    ];
    const qs = corners.map((c) => c.q);
    const rs = corners.map((c) => c.r);
    const qLo = Math.floor(Math.min(...qs)) - 1;
    const qHi = Math.ceil(Math.max(...qs)) + 1;
    const rLo = Math.floor(Math.min(...rs)) - 1;
    const rHi = Math.ceil(Math.max(...rs)) + 1;
    for (let r = rLo; r <= rHi; r++) {
      for (let q = qLo; q <= qHi; q++) {
        const pts = this.cellCorners(q, r); // the single hex-corner formula (see cellCorners)
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
