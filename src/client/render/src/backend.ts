import type { LineSeg, CameraTransform, VisibilityInput, TokenNodeSpec, ShapeNodeSpec, Point } from "./types";
import type { LightingFrame } from "./lighting";

/** The narrow GL abstraction the render model drives. The real implementation is
 * `pixi-backend.ts` (Playwright-covered); `MockBackend` covers it in unit tests.
 * Kept minimal for M8c-1 (background + grid + camera); M8d generalizes to a node
 * API for token/wall/etc. reconcilers. */
export interface DisplayBackend {
  /** Create/parent the core layer containers in the given z-order (idempotent). */
  ensureLayers(orderedIds: string[]): void;
  /** Set or clear the background-layer sprite. */
  setBackground(spec: { url: string } | null): void;
  /** Replace the grid-layer line set (scene coords) with the given color (0xRRGGBB). */
  drawGrid(lines: LineSeg[], color: number): void;
  /** Apply the visibility mask (the mask slot). Empty `visible` = identity
   * (full visibility → transparent overlay). */
  setVisibility(input: VisibilityInput): void;
  /** Cross-fade the visibility mask between two consecutive vision samples (M2 §T7 fog
   * cross-fade — a mover's own vision sweep interpolates between samples instead of
   * snapping). `factor` in `[0,1]`: 0 = fully `from`, 1 = fully `to`. Optional: a backend
   * without cross-fade support may omit it; `Compositor.setVisibilityBlend` falls back to a
   * plain `setVisibility` nearest-sample snap when absent. */
  setVisibilityBlend?(from: VisibilityInput, to: VisibilityInput, factor: number): void;
  /** Apply the camera transform to the world container. */
  setCameraTransform(t: CameraTransform): void;
  /** Module-facing shader-filter seam: attach an opaque filter to a layer; returns a
   * dispose. No engine consumer in M8 (token fx / Phase-3 VFX are future consumers). */
  addLayerFilter(layerId: string, filter: unknown): () => void;
  /** Upsert a token render node (create if new; update transform/size/texture otherwise). */
  setToken(id: string, spec: TokenNodeSpec): void;
  /** Remove a token render node. */
  removeToken(id: string): void;
  /** Upsert a drawn shape node in `spec.layer` (drawings/templates reconcilers). */
  setShape(id: string, spec: ShapeNodeSpec): void;
  /** Remove a drawn shape node. */
  removeShape(id: string): void;
  /** Replace the ephemeral overlay (in the `overlays` layer) with these shapes — the
   * tool preview / measurement; never document-backed. */
  drawOverlay(shapes: Omit<ShapeNodeSpec, "layer">[]): void;
  /** Clear the ephemeral overlay. */
  clearOverlay(): void;
  /** Draw the measurement overlay: a segment `from`→`to` + a centered distance label. */
  drawMeasure(from: Point, to: Point, label: string): void;
  /** Clear the measurement overlay. */
  clearMeasure(): void;
  /** Redraw the transient ping rings (expanding/fading outline circles). */
  drawPings(rings: { x: number; y: number; radius: number; alpha: number }[]): void;
  /** Paint the lighting overlay (the `lighting` layer): per-cell darkening + tint + desaturate hint. */
  setLighting(frame: LightingFrame): void;
  /** Register the per-frame render ticker callback (drives tweens). */
  startTicker(cb: (dtMs: number) => void): void;
  /** Resize the renderer/viewport to CSS pixels (HiDPI handled by the backend). */
  resize(width: number, height: number): void;
  /** Release all GPU resources and detach the canvas. */
  destroy(): void;
}
