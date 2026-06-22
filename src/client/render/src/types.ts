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

/** Visibility for the mask slot (D-V1 polygons, scene coords). Empty `visible`
 * ⇒ identity (everything visible → transparent overlay). `explored` is M9 (D-V2). */
export interface VisibilityInput {
  visible: Polygon[];
  explored?: Polygon[];
}

/** A token's animatable transform (scene coords; `(x,y)` = center). */
export interface TokenTransform {
  x: number;
  y: number;
  rotation: number;
}

/** A resolved token render node: transform + size + resolved image URL. */
export interface TokenNodeSpec {
  x: number;
  y: number;
  w: number;
  h: number;
  rotation: number;
  url: string;
}
