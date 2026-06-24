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

/** Visibility for the mask slot (scene coords). `mode:"all"` = no fog (GM / no occlusion).
 * `mode:"masked"` = three-state fog: **unexplored** (outside both sets) = darkest, **explored**
 * (in `explored`, not `visible`) = dimmed memory, **visible** = clear. Empty `visible` + empty
 * `explored` ⇒ full dark fog (see nothing), NOT "see everything". `explored` is the M9c persistent
 * memory layer (rect polygons rasterized from the server's per-(scene,player) explored cells);
 * `visible ⊆ explored` semantically (a visible cell is also explored). */
export interface VisibilityInput {
  mode: "all" | "masked";
  visible: Polygon[];
  explored: Polygon[];
}

/** A token's animatable transform (scene coords; `(x,y)` = center). */
export interface TokenTransform {
  x: number;
  y: number;
  rotation: number;
}

/** A resolved token render node: transform + size + resolved image URL + faction border. */
export interface TokenNodeSpec {
  x: number;
  y: number;
  w: number;
  h: number;
  rotation: number;
  url: string;
  /** Faction border color (0xRRGGBB), or null for no border. */
  borderColor: number | null;
}

/** A drawn shape node: a polyline/polygon (flat scene-coord points) with optional fill
 * and stroke, parented to `layer`. Drawings + templates reconcile to this; all shape
 * tessellation (cone/circle/…) happens in `geometry.ts` before reaching the backend. */
export interface ShapeNodeSpec {
  layer: string;
  points: number[];
  closed: boolean;
  stroke: { color: number; width: number } | null;
  fill: { color: number; alpha: number } | null;
}

/** A canvas tool. The engine routes pointer events (in scene coords) to the active
 * tool first; `onPointerDown` returning true claims the gesture (else camera pans). */
export interface SceneTool {
  onPointerDown(p: Point, ev: PointerEvent): boolean;
  onPointerMove(p: Point, ev: PointerEvent): void;
  onPointerUp(p: Point, ev: PointerEvent): void;
}

/** The engine surface tools drive (via the AppContext `scene` bridge). The
 * RenderEngine implements this; a detached bridge no-ops. */
export interface SceneToolHost {
  /** Set (or clear) the active tool; the no-tool case falls back to camera pan/zoom. */
  setActiveTool(tool: SceneTool | null): void;
  /** Snap a scene point to the active grid (cell/vertex). */
  snap(p: Point): Point;
  /** Mark a token as locally dragging so its sprite snaps to the authoritative
   * transform (no tween lag) while a remote move still tweens; null clears it. */
  setDraggingToken(id: string | null): void;
  /** Draw an ephemeral, non-document preview (tool in-progress shape) into the overlay. */
  previewOverlay(shapes: Omit<ShapeNodeSpec, "layer">[]): void;
  /** Clear the ephemeral preview overlay. */
  clearOverlay(): void;
  /** Whole-cell distance between two scene points via the active grid (measurement). */
  gridDistance(a: Point, b: Point): number;
  /** Draw the client-local measurement overlay (a segment + a distance label). */
  drawMeasure(from: Point, to: Point, label: string): void;
  /** Clear the measurement overlay. */
  clearMeasure(): void;
  /** Spawn a transient ping ring at scene `(x,y)` (from a received/own ping). */
  addPing(x: number, y: number): void;
}
