import type { Point, CameraTransform } from "./types";

const MIN_SCALE = 0.1;
const MAX_SCALE = 10;

const clampScale = (s: number): number =>
  Math.min(MAX_SCALE, Math.max(MIN_SCALE, s));

/** Pure pan/zoom math: `screen = scene * scale + offset`, identically on both axes —
 * neither axis is inverted (scene y and screen y both increase downward). The engine
 * applies `transform()` to the Pixi world container and feeds it pointer gestures. */
export class Camera {
  private offset = { x: 0, y: 0 };
  private scale = 1;

  /**
   * Current pan/zoom state, as applied to the Pixi world container.
   * @returns The current `{x, y, scale}` offset/scale pair.
   * @example
   * ```ts
   * import { Camera } from "@shadowcat/render";
   *
   * const camera = new Camera();
   * camera.transform(); // { x: 0, y: 0, scale: 1 }
   * ```
   */
  transform(): CameraTransform {
    return { x: this.offset.x, y: this.offset.y, scale: this.scale };
  }

  /**
   * Pans the camera by a screen-space delta — added directly to `offset`, unscaled by
   * zoom, so a fixed-pixel drag gesture pans the same screen distance at any zoom level.
   * @param dxScreen Screen-space x delta, in pixels.
   * @param dyScreen Screen-space y delta, in pixels.
   * @example
   * ```ts
   * import { Camera } from "@shadowcat/render";
   *
   * const camera = new Camera();
   * camera.panBy(10, -5);
   * ```
   */
  panBy(dxScreen: number, dyScreen: number): void {
    this.offset.x += dxScreen;
    this.offset.y += dyScreen;
  }

  /** Multiplies scale by `factor`, clamped to `[0.1, 10]`, holding the scene point
   * under `(screenX,screenY)` fixed — derives the new offset so
   * `screenToScene({x:screenX,y:screenY})` returns the same scene point before and
   * after the zoom.
   * @param factor Zoom multiplier (`>1` zooms in, `<1` zooms out).
   * @param screenX Screen-space x of the point to hold fixed (e.g. the cursor).
   * @param screenY Screen-space y of the point to hold fixed (e.g. the cursor).
   * @example
   * ```ts
   * import { Camera } from "@shadowcat/render";
   *
   * const camera = new Camera();
   * camera.zoomAt(1.5, 400, 300); // zoom in 1.5x, cursor at (400,300) stays put
   * ```
   */
  zoomAt(factor: number, screenX: number, screenY: number): void {
    const next = clampScale(this.scale * factor);
    // scene under cursor before: (screen - offset) / scale. Keep it constant:
    // offset' = screen - scene * scale'
    const sceneX = (screenX - this.offset.x) / this.scale;
    const sceneY = (screenY - this.offset.y) / this.scale;
    this.offset.x = screenX - sceneX * next;
    this.offset.y = screenY - sceneY * next;
    this.scale = next;
  }

  /**
   * Converts a screen-space point to scene coordinates — the inverse of {@link
   * sceneToScreen}: `scene = (screen - offset) / scale`, identically on both axes.
   * @param p A screen-space point.
   * @returns The equivalent scene-coordinate point.
   * @example
   * ```ts
   * import { Camera } from "@shadowcat/render";
   *
   * const camera = new Camera();
   * camera.screenToScene({ x: 0, y: 0 }); // { x: 0, y: 0 }
   * ```
   */
  screenToScene(p: Point): Point {
    return {
      x: (p.x - this.offset.x) / this.scale,
      y: (p.y - this.offset.y) / this.scale,
    };
  }

  /**
   * Converts a scene-coordinate point to screen space — see {@link transform}:
   * `screen = scene * scale + offset`, identically on both axes.
   * @param p A scene-coordinate point.
   * @returns The equivalent screen-space point.
   * @example
   * ```ts
   * import { Camera } from "@shadowcat/render";
   *
   * const camera = new Camera();
   * camera.sceneToScreen({ x: 0, y: 0 }); // { x: 0, y: 0 }
   * ```
   */
  sceneToScreen(p: Point): Point {
    return {
      x: p.x * this.scale + this.offset.x,
      y: p.y * this.scale + this.offset.y,
    };
  }
}
