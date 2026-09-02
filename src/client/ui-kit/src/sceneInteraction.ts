// The canvas interaction bridge. A stable handle owned by WorldSession and
// exposed on AppContext, so tool components reach the engine's tool API even though the
// RenderEngine is created lazily inside the Stage effect. Stage attaches the engine on
// mount; before/after attachment every call no-ops (snap is identity) so a tool
// component never crashes when no canvas is mounted. Render types are type-only imports
// (zero runtime dependency on @shadowcat/render here).
import type { SceneTool, SceneToolHost, Point, ShapeNodeSpec, MoveVisionSample, MoveLightSample } from "@shadowcat/render";

/** The host-facing seam plus late-attachment. */
export interface SceneInteraction extends SceneToolHost {
  /** Attach the live engine (a SceneToolHost); returns a detach that only clears the
   * host if it is still the current one (a stale detach after re-attach is a no-op).
   * @param host - The live engine implementing `SceneToolHost`.
   * @returns A detach function; safe to call multiple times or after superseded. */
  attach(host: SceneToolHost): () => void;
}

/**
 * Late-binding {@link SceneInteraction}: every method forwards to the attached host when
 * one is present, and falls back to a harmless default (identity for `snap`, `0` for
 * `gridDistance`, a no-op for everything else) before attach / after detach — so a tool
 * component can call these unconditionally without checking whether the Stage has mounted
 * a `RenderEngine` yet.
 */
export class SceneInteractionBridge implements SceneInteraction {
  /** The attached live engine, or `null` before attach / after detach. */
  #host: SceneToolHost | null = null;

  /** Attach `host` as the live engine. A later `attach` replaces the current host outright
   * (no stacking). The returned detach only clears `#host` if `host` is STILL the current
   * one — a stale detach captured before a later `attach` is a no-op, so an out-of-order
   * teardown can never clear a newer host.
   * @param host - The live engine implementing `SceneToolHost`.
   * @returns A detach function; safe to call multiple times or after superseded.
   * @example const detach = sceneInteraction.attach(renderEngine);
   */
  attach(host: SceneToolHost): () => void {
    this.#host = host;
    return () => {
      if (this.#host === host) this.#host = null;
    };
  }

  /** Forward to the attached host; a no-op when detached.
   * @param tool - The tool to activate, or `null` to fall back to camera pan/zoom.
   * @example sceneInteraction.setActiveTool(null);
   */
  setActiveTool(tool: SceneTool | null): void {
    this.#host?.setActiveTool(tool);
  }

  /** Forward to the attached host's grid snap; identity (`p` unchanged) when detached.
   * @param p - The scene point to snap.
   * @returns The snapped point, or `p` itself when no host is attached.
   * @example sceneInteraction.snap({ x: 12.3, y: 4.1 });
   */
  snap(p: Point): Point {
    return this.#host ? this.#host.snap(p) : p;
  }

  /** Forward to the attached host; a no-op when detached.
   * @param enabled - Whether grid snapping is active for the scene.
   * @example sceneInteraction.setSnapEnabled(false);
   */
  setSnapEnabled(enabled: boolean): void {
    this.#host?.setSnapEnabled(enabled);
  }

  /** Forward to the attached host; a no-op when detached.
   * @param id - The dragging token's id, or `null` to clear.
   * @example sceneInteraction.setDraggingToken("tok1");
   */
  setDraggingToken(id: string | null): void {
    this.#host?.setDraggingToken(id);
  }

  /** Forward to the attached host; a no-op when detached.
   * @param shapes - The ephemeral preview shapes to draw.
   * @example sceneInteraction.previewOverlay([]);
   */
  previewOverlay(shapes: Omit<ShapeNodeSpec, "layer">[]): void {
    this.#host?.previewOverlay(shapes);
  }

  /** Forward to the attached host; a no-op when detached.
   * @example sceneInteraction.clearOverlay();
   */
  clearOverlay(): void {
    this.#host?.clearOverlay();
  }

  /** Forward to the attached host's grid distance; `0` when detached.
   * @param a - The first scene point.
   * @param b - The second scene point.
   * @returns The whole-cell grid distance, or `0` when no host is attached.
   * @example sceneInteraction.gridDistance({ x: 0, y: 0 }, { x: 5, y: 0 });
   */
  gridDistance(a: Point, b: Point): number {
    return this.#host ? this.#host.gridDistance(a, b) : 0;
  }

  /** Forward to the attached host; a no-op when detached.
   * @param from - The measurement segment's start point.
   * @param to - The measurement segment's end point.
   * @param label - The distance label to render alongside the segment.
   * @example sceneInteraction.drawMeasure(a, b, "5 ft");
   */
  drawMeasure(from: Point, to: Point, label: string): void {
    this.#host?.drawMeasure(from, to, label);
  }

  /** Forward to the attached host; a no-op when detached.
   * @example sceneInteraction.clearMeasure();
   */
  clearMeasure(): void {
    this.#host?.clearMeasure();
  }

  /** Forward to the attached host; a no-op when detached.
   * @param x - The ping's scene x coordinate.
   * @param y - The ping's scene y coordinate.
   * @example sceneInteraction.addPing(10, 20);
   */
  addPing(x: number, y: number): void {
    this.#host?.addPing(x, y);
  }

  /** Forward to the attached host; a no-op when detached.
   * @param id - The token id to animate.
   * @param path - The scene-coord waypoints to walk through, in order.
   * @example sceneInteraction.animateAlongPath("tok1", [[0, 0], [5, 5]]);
   */
  animateAlongPath(id: string, path: [number, number][]): void {
    this.#host?.animateAlongPath(id, path);
  }

  /** Forward to the attached host; a no-op when detached.
   * @param id - The token id to animate.
   * @param samples - The server-broadcast position samples to interpolate between.
   * @param durationMs - The total playback duration in milliseconds.
   * @param startServerMs - The server clock time the move started, for catch-up elapsed.
   * @param serverNow - Optional current-server-time getter for catch-up; omitted means no catch-up.
   * @param moverVision - Mover-only progressive fog-sweep samples; `null`/omitted for observers.
   * @param moverLight - Carried-light sweep samples admitted to this viewer; `null`/omitted when none.
   * @example sceneInteraction.animateSamples("tok1", samples, 1000, startMs);
   */
  animateSamples(
    id: string,
    samples: {
      /** Milliseconds since the move started that `pos` applies at. */
      tMs: number;
      /** The scene-coord position at `tMs`. */
      pos: [number, number];
    }[],
    durationMs: number,
    startServerMs: number,
    serverNow?: () => number,
    moverVision?: MoveVisionSample[] | null,
    moverLight?: MoveLightSample[] | null,
  ): void {
    this.#host?.animateSamples(id, samples, durationMs, startServerMs, serverNow, moverVision, moverLight);
  }
}
