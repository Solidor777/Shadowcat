import type { Point, CameraTransform } from "./types";

const MIN_SCALE = 0.1;
const MAX_SCALE = 10;

const clampScale = (s: number): number =>
  Math.min(MAX_SCALE, Math.max(MIN_SCALE, s));

/** Pure pan/zoom math: screen = scene * scale + offset. The engine applies
 * `transform()` to the Pixi world container and feeds it pointer gestures. */
export class Camera {
  private offset = { x: 0, y: 0 };
  private scale = 1;

  transform(): CameraTransform {
    return { x: this.offset.x, y: this.offset.y, scale: this.scale };
  }

  panBy(dxScreen: number, dyScreen: number): void {
    this.offset.x += dxScreen;
    this.offset.y += dyScreen;
  }

  /** Multiply scale by `factor`, holding the scene point under (screenX,screenY)
   * fixed. Derives the new offset so screenToScene(cursor) is invariant. */
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

  screenToScene(p: Point): Point {
    return {
      x: (p.x - this.offset.x) / this.scale,
      y: (p.y - this.offset.y) / this.scale,
    };
  }

  sceneToScreen(p: Point): Point {
    return {
      x: p.x * this.scale + this.offset.x,
      y: p.y * this.scale + this.offset.y,
    };
  }
}
