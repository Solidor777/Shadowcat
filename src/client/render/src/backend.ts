import type { LineSeg, CameraTransform, VisibilityInput } from "./types";

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
  /** Apply the camera transform to the world container. */
  setCameraTransform(t: CameraTransform): void;
  /** Resize the renderer/viewport to CSS pixels (HiDPI handled by the backend). */
  resize(width: number, height: number): void;
  /** Release all GPU resources and detach the canvas. */
  destroy(): void;
}
