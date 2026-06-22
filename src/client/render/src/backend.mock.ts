import type { DisplayBackend } from "./backend";
import type { LineSeg, CameraTransform, VisibilityInput } from "./types";

/** A recording DisplayBackend for unit tests — never touches Pixi/GL. */
export class MockBackend implements DisplayBackend {
  layers: string[] = [];
  background: { url: string } | null = null;
  gridLineCount = 0;
  gridColor: number | null = null;
  camera: CameraTransform | null = null;
  visibility: VisibilityInput | null = null;
  size: { width: number; height: number } | null = null;
  destroyed = false;

  ensureLayers(orderedIds: string[]): void {
    this.layers = [...orderedIds];
  }
  setBackground(spec: { url: string } | null): void {
    this.background = spec;
  }
  drawGrid(lines: LineSeg[], color: number): void {
    this.gridLineCount = lines.length;
    this.gridColor = color;
  }
  setCameraTransform(t: CameraTransform): void {
    this.camera = t;
  }
  setVisibility(input: VisibilityInput): void {
    this.visibility = input;
  }
  resize(width: number, height: number): void {
    this.size = { width, height };
  }
  destroy(): void {
    this.destroyed = true;
  }
}
