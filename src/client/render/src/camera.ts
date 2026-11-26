import { applyEasing, type EasingMode } from "./easing";
import type { Point, CameraTransform } from "./types";

/** Default {@link Camera.panTo} duration, in milliseconds. */
const DEFAULT_PAN_MS = 400;

/** The screen viewport a {@link Camera.panTo} centers within, in screen pixels. */
export interface PanViewport {
  /** Viewport width, screen px. */
  width: number;
  /** Viewport height, screen px. */
  height: number;
}

/** What {@link Camera.panTo} centers on: a fixed scene point, or a provider re-read every frame
 * so the camera tracks a moving subject (e.g. a token mid-tween). A provider returning
 * `undefined` means the subject is gone: the pan is dropped. */
export type PanTarget = Point | (() => Point | undefined);

/** Per-call options for {@link Camera.panTo}. */
export interface CameraPanOptions {
  /** Tween length in milliseconds; defaults to 400. A value `<= 0` snaps like `reducedMotion`. */
  durationMs?: number;
  /** The easing curve over the tween; defaults to `"easeInOut"`. */
  easing?: EasingMode;
  /** Snap straight to the target instead of tweening. */
  reducedMotion?: boolean;
}

/** An in-flight {@link Camera.panTo} tween. */
interface PanState {
  /** The scene point at the viewport center when the pan began. */
  startCenter: Point;
  /** The subject being centered on. */
  target: PanTarget;
  /** Tween length, ms (> 0). */
  durationMs: number;
  /** The easing curve applied to the elapsed fraction. */
  easing: EasingMode;
  /** Milliseconds advanced so far. */
  elapsedMs: number;
}

/** Resolves a {@link PanTarget} to its current scene point.
 * @param target A fixed point or a live provider.
 * @returns The point, or `undefined` when a provider reports its subject gone.
 * @example
 * ```
 * // module-private helper; not exported from @shadowcat/render
 * resolvePanTarget({ x: 3, y: 4 }); // { x: 3, y: 4 }
 * ```
 */
const resolvePanTarget = (target: PanTarget): Point | undefined =>
  typeof target === "function" ? target() : target;

/** Minimum allowed zoom scale — `zoomAt` clamps to this floor. */
const MIN_SCALE = 0.1;
/** Maximum allowed zoom scale — `zoomAt` clamps to this ceiling. */
const MAX_SCALE = 10;

/** Clamp a scale value to `[MIN_SCALE, MAX_SCALE]`.
 * @param s The raw (unclamped) scale value.
 * @returns `s` clamped to `[MIN_SCALE, MAX_SCALE]`.
 * @example
 * ```
 * // module-private helper; not exported from @shadowcat/render
 * clampScale(50); // 10 — clamped to MAX_SCALE
 * ```
 */
const clampScale = (s: number): number =>
  Math.min(MAX_SCALE, Math.max(MIN_SCALE, s));

/** Pure pan/zoom math: `screen = scene * scale + offset`, identically on both axes —
 * neither axis is inverted (scene y and screen y both increase downward). The engine
 * applies `transform()` to the Pixi world container and feeds it pointer gestures. */
export class Camera {
  /** World-container translation, screen-space px — see `transform`. */
  private offset = { x: 0, y: 0 };
  /** Uniform zoom scale, clamped to `[MIN_SCALE, MAX_SCALE]` — see `transform`. */
  private scale = 1;
  /** The in-flight {@link panTo} tween, or `null`. Cancelled by any manual {@link panBy} or
   * {@link zoomAt}, so a user gesture always wins over an animation. */
  private pan: PanState | null = null;

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
    this.pan = null;
    this.offset.x += dxScreen;
    this.offset.y += dyScreen;
  }

  /**
   * Centers the viewport on `target`, tweening the pan offset (zoom is untouched) over
   * `opts.durationMs` through `opts.easing`; {@link tick} advances the tween. A provider target
   * is re-read every {@link tick}, so the camera tracks a subject that moves while the pan runs.
   * `opts.reducedMotion` (or a non-positive duration) centers immediately and starts no tween.
   * A target that resolves to nothing is a no-op. Replaces any pan already in flight; a later
   * {@link panBy} or {@link zoomAt} cancels it. The caller applies the resulting
   * {@link transform} (immediately after a snap, after each {@link tick} that returns `true`).
   * @param target A scene point, or a provider of one (see {@link PanTarget}).
   * @param viewport The screen viewport to center within.
   * @param opts Duration, easing and reduced-motion behavior.
   * @example
   * ```ts
   * import { Camera } from "@shadowcat/render";
   *
   * const camera = new Camera();
   * camera.panTo({ x: 500, y: 300 }, { width: 800, height: 600 }, { durationMs: 300 });
   * ```
   */
  panTo(target: PanTarget, viewport: PanViewport, opts: CameraPanOptions = {}): void {
    this.pan = null;
    const first = resolvePanTarget(target);
    if (!first) return;
    const durationMs = opts.durationMs ?? DEFAULT_PAN_MS;
    if (opts.reducedMotion || durationMs <= 0) {
      this.centerOn(first, viewport);
      return;
    }
    this.pan = {
      startCenter: this.screenToScene({ x: viewport.width / 2, y: viewport.height / 2 }),
      target,
      durationMs,
      easing: opts.easing ?? "easeInOut",
      elapsedMs: 0,
    };
  }

  /**
   * Advances the in-flight {@link panTo} tween by `dtMs`: the viewport center moves from where
   * the pan began toward the target's CURRENT position by the eased elapsed fraction, landing
   * exactly on it at the end. A provider target that reports its subject gone drops the pan.
   * @param dtMs Milliseconds since the previous tick.
   * @param viewport The screen viewport to center within (read per tick so a resize mid-pan holds).
   * @returns Whether the camera moved, i.e. whether the caller must re-apply {@link transform}.
   * @example
   * ```ts
   * import { Camera } from "@shadowcat/render";
   *
   * const camera = new Camera();
   * camera.panTo({ x: 500, y: 300 }, { width: 800, height: 600 });
   * camera.tick(16, { width: 800, height: 600 }); // true
   * ```
   */
  tick(dtMs: number, viewport: PanViewport): boolean {
    const pan = this.pan;
    if (!pan) return false;
    const live = resolvePanTarget(pan.target);
    if (!live) {
      this.pan = null;
      return false;
    }
    pan.elapsedMs += dtMs;
    const raw = pan.elapsedMs / pan.durationMs;
    const k = applyEasing(pan.easing, raw);
    this.centerOn(
      {
        x: pan.startCenter.x + (live.x - pan.startCenter.x) * k,
        y: pan.startCenter.y + (live.y - pan.startCenter.y) * k,
      },
      viewport,
    );
    if (raw >= 1) this.pan = null;
    return true;
  }

  /**
   * Sets the offset so scene point `p` sits at the viewport center, at the current scale.
   * @param p The scene point to center.
   * @param viewport The screen viewport to center within.
   * @example
   * ```
   * // private method; not part of the public API
   * this.centerOn({ x: 500, y: 300 }, { width: 800, height: 600 });
   * ```
   */
  private centerOn(p: Point, viewport: PanViewport): void {
    this.offset.x = viewport.width / 2 - p.x * this.scale;
    this.offset.y = viewport.height / 2 - p.y * this.scale;
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
    this.pan = null;
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
