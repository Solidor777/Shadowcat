// Pure shape geometry: scene-coordinate tessellation of template/drawing shapes into
// flat point arrays [x0,y0,x1,y1,…], plus color parsing. The backend draws whatever
// points it is given, so all shape math (cone/circle/ellipse/square) lives here and is
// headless-testable. Angles are degrees; 0° = +x, positive toward +y (scene y is down).

const deg2rad = (d: number): number => (d * Math.PI) / 180;

/** `#rrggbb` (or `rrggbb`) → `0xRRGGBB`; `0x000000` on malformed input. */
export function parseColor(hex: string): number {
  const m = /^#?([0-9a-fA-F]{6})$/.exec(hex.trim());
  return m ? parseInt(m[1], 16) : 0;
}

/** Four corners of the axis-aligned rectangle spanning two opposite corners. */
export function rectPoints(x0: number, y0: number, x1: number, y1: number): number[] {
  return [x0, y0, x1, y0, x1, y1, x0, y1];
}

/** `segments` points of the ellipse inscribed in the bbox `(x0,y0)-(x1,y1)`. */
export function ellipsePoints(x0: number, y0: number, x1: number, y1: number, segments = 32): number[] {
  const cx = (x0 + x1) / 2;
  const cy = (y0 + y1) / 2;
  const rx = Math.abs(x1 - x0) / 2;
  const ry = Math.abs(y1 - y0) / 2;
  const out: number[] = [];
  for (let i = 0; i < segments; i++) {
    const a = (i / segments) * 2 * Math.PI;
    out.push(cx + rx * Math.cos(a), cy + ry * Math.sin(a));
  }
  return out;
}

/** `segments` points of a circle of radius `r` centered at `(cx,cy)`. */
export function circlePoints(cx: number, cy: number, r: number, segments = 32): number[] {
  return ellipsePoints(cx - r, cy - r, cx + r, cy + r, segments);
}

/** Isoceles cone: apex `(apexX,apexY)`, two base corners at distance `size` along
 * `directionDeg ± apertureDeg/2`. */
export function conePoints(apexX: number, apexY: number, size: number, directionDeg: number, apertureDeg = 60): number[] {
  const a = deg2rad(directionDeg);
  const half = deg2rad(apertureDeg / 2);
  return [
    apexX,
    apexY,
    apexX + size * Math.cos(a - half),
    apexY + size * Math.sin(a - half),
    apexX + size * Math.cos(a + half),
    apexY + size * Math.sin(a + half),
  ];
}

/** Four corners of a square (side `2*half`) centered at `(cx,cy)`, rotated `directionDeg`. */
export function squarePoints(cx: number, cy: number, half: number, directionDeg: number): number[] {
  const a = deg2rad(directionDeg);
  const c = Math.cos(a);
  const s = Math.sin(a);
  const corners: [number, number][] = [
    [-half, -half],
    [half, -half],
    [half, half],
    [-half, half],
  ];
  const out: number[] = [];
  for (const [x, y] of corners) out.push(cx + x * c - y * s, cy + x * s + y * c);
  return out;
}
