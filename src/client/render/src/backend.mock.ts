import type { DisplayBackend } from "./backend";
import type { LineSeg, CameraTransform, VisibilityInput, TokenNodeSpec } from "./types";

/** A recording DisplayBackend for unit tests — never touches Pixi/GL. */
export class MockBackend implements DisplayBackend {
  layers: string[] = [];
  background: { url: string } | null = null;
  gridLineCount = 0;
  gridColor: number | null = null;
  camera: CameraTransform | null = null;
  visibility: VisibilityInput | null = null;
  size: { width: number; height: number } | null = null;
  filters: Array<{ layerId: string; filter: unknown }> = [];
  tokens = new Map<string, TokenNodeSpec>();
  tick: ((dtMs: number) => void) | undefined;
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
  addLayerFilter(layerId: string, filter: unknown): () => void {
    const entry = { layerId, filter };
    this.filters.push(entry);
    return () => {
      const i = this.filters.indexOf(entry);
      if (i >= 0) this.filters.splice(i, 1);
    };
  }
  setToken(id: string, spec: TokenNodeSpec): void {
    this.tokens.set(id, spec);
  }
  removeToken(id: string): void {
    this.tokens.delete(id);
  }
  startTicker(cb: (dtMs: number) => void): void {
    this.tick = cb;
  }
  resize(width: number, height: number): void {
    this.size = { width, height };
  }
  destroy(): void {
    this.destroyed = true;
  }
}
