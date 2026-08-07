import type { LineSeg, CameraTransform, VisibilityInput, TokenNodeSpec, ShapeNodeSpec, Point } from "./types";
import type { LightingFrame } from "./lighting";
import type { PingRing } from "./ping-view";

/** The background-layer sprite spec — the statement of record for this shape; every
 * implementation/recording site (`PixiBackend`, `MockBackend`) references this type rather
 * than restating its field. */
export interface BackgroundSpec {
  /** Background image serve URL. */
  url: string;
}

/** The narrow GL abstraction the render model drives. The real implementation is
 * `PixiBackend` (Playwright-covered); `MockBackend` covers it in unit tests. */
export interface DisplayBackend {
  /** Create/parent the core layer containers in the given z-order (idempotent).
   * @param orderedIds Layer ids, back-to-front — see the `layers` module's `CORE_LAYERS`. */
  ensureLayers(orderedIds: string[]): void;
  /** Set or clear the background-layer sprite.
   * @param spec The background image, or `null` to clear it. */
  setBackground(spec: BackgroundSpec | null): void;
  /** Replace the grid-layer line set (scene coords) with the given color (0xRRGGBB).
   * @param lines The grid line segments to draw, in scene coordinates.
   * @param color Line color, `0xRRGGBB`. */
  drawGrid(lines: LineSeg[], color: number): void;
  /** Apply the visibility mask (the mask slot). Empty `visible` = identity
   * (full visibility → transparent overlay).
   * @param input The fog/mask state to render — see `VisibilityInput`. */
  setVisibility(input: VisibilityInput): void;
  /** Cross-fade the visibility mask between two consecutive vision samples (M2 §T7 fog
   * cross-fade — a mover's own vision sweep interpolates between samples instead of
   * snapping). `factor` in `[0,1]`: 0 = fully `from`, 1 = fully `to`. Optional: a backend
   * without cross-fade support may omit it; `Compositor.setVisibilityBlend` falls back to a
   * plain `setVisibility` nearest-sample snap when absent.
   * @param from The outgoing (older) vision sample.
   * @param to The incoming (newer) vision sample.
   * @param factor Blend position in `[0,1]`; 0 = fully `from`, 1 = fully `to`. */
  setVisibilityBlend?(from: VisibilityInput, to: VisibilityInput, factor: number): void;
  /** Apply the camera transform to the world container.
   * @param t The camera transform to apply. */
  setCameraTransform(t: CameraTransform): void;
  /** Module-facing shader-filter seam: attach an opaque filter to a layer; returns a
   * dispose. No engine consumer currently forwards through this seam.
   * @param layerId Target core-layer id.
   * @param filter Backend-specific filter object (opaque to the render model).
   * @returns A dispose callback that removes the filter. */
  addLayerFilter(layerId: string, filter: unknown): () => void;
  /** Upsert a token render node (create if new; update transform/size/texture otherwise).
   * @param id The token document id.
   * @param spec The resolved render node to draw. */
  setToken(id: string, spec: TokenNodeSpec): void;
  /** Remove a token render node.
   * @param id The token document id to remove. */
  removeToken(id: string): void;
  /** Advance any tick-driven animated token visuals by `dtMs` (M10h). Called once per frame
   * alongside the `startTicker` callback; a no-op backend when nothing has an `animated` visual.
   * @param dtMs Elapsed time since the previous tick, in ms. */
  tickTokenAnimations(dtMs: number): void;
  /** Upsert a drawn shape node in `spec.layer` (drawings/templates reconcilers).
   * @param id The shape document id.
   * @param spec The shape to draw, including its target layer. */
  setShape(id: string, spec: ShapeNodeSpec): void;
  /** Remove a drawn shape node.
   * @param id The shape document id to remove. */
  removeShape(id: string): void;
  /** Replace the ephemeral overlay (in the `overlays` layer) with these shapes — the
   * tool preview / measurement; never document-backed.
   * @param shapes The overlay shapes to draw. */
  drawOverlay(shapes: Omit<ShapeNodeSpec, "layer">[]): void;
  /** Clear the ephemeral overlay. */
  clearOverlay(): void;
  /** Draw the measurement overlay: a segment `from`→`to` + a centered distance label.
   * @param from The segment's start point, in scene coordinates.
   * @param to The segment's end point, in scene coordinates.
   * @param label The distance label text to center on the segment. */
  drawMeasure(from: Point, to: Point, label: string): void;
  /** Clear the measurement overlay. */
  clearMeasure(): void;
  /** Redraw the transient ping rings (expanding/fading outline circles).
   * @param rings The ping rings to draw — see `PingRing`. */
  drawPings(rings: PingRing[]): void;
  /** Paint the lighting overlay (the `lighting` layer): per-cell darkening + tint + desaturate hint.
   * @param frame The resolved lighting overlay to paint. */
  setLighting(frame: LightingFrame): void;
  /** Register the per-frame render ticker callback (drives tweens).
   * @param cb Called once per frame with the elapsed time since the previous frame, in ms. */
  startTicker(cb: (dtMs: number) => void): void;
  /** Resize the renderer/viewport to CSS pixels (HiDPI handled by the backend).
   * @param width New viewport width, in CSS pixels.
   * @param height New viewport height, in CSS pixels. */
  resize(width: number, height: number): void;
  /** Release all GPU resources and detach the canvas. */
  destroy(): void;
}
