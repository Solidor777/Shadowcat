/** A point in scene coordinates. */
export interface Point {
  x: number;
  y: number;
}

/** A line segment in scene coordinates (grid lines). */
export interface LineSeg {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}

/** Resolution-independent polygon geometry (D-V1), scene coords, flat
 * [x0,y0,x1,y1,…]. Consumed by the M8c-2 compositor; defined here so the public
 * value-type surface is one module. */
export interface Polygon {
  points: number[];
}

/** Camera transform applied to the world container: translate then uniform scale. */
export interface CameraTransform {
  x: number;
  y: number;
  scale: number;
}
