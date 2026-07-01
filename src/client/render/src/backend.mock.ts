import type { DisplayBackend } from "./backend";
import type { LineSeg, CameraTransform, VisibilityInput, TokenNodeSpec, ShapeNodeSpec, Point } from "./types";
import type { LightingFrame } from "./lighting";

/** A recording DisplayBackend for unit tests — never touches Pixi/GL. */
export class MockBackend implements DisplayBackend {
  layers: string[] = [];
  background: { url: string } | null = null;
  gridLineCount = 0;
  gridColor: number | null = null;
  camera: CameraTransform | null = null;
  visibility: VisibilityInput | null = null;
  /** Last `setVisibilityBlend` call recorded verbatim (from/to/factor), for asserting the
   * M2 §T7 cross-fade advances 0→1 across a sample interval. */
  visibilityBlend: { from: VisibilityInput; to: VisibilityInput; factor: number } | null = null;
  size: { width: number; height: number } | null = null;
  filters: Array<{ layerId: string; filter: unknown }> = [];
  tokens = new Map<string, TokenNodeSpec>();
  shapes = new Map<string, ShapeNodeSpec>();
  overlay: Omit<ShapeNodeSpec, "layer">[] = [];
  measure: { from: Point; to: Point; label: string } | null = null;
  pings: { x: number; y: number; radius: number; alpha: number }[] = [];
  lighting: LightingFrame | null = null;
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
    this.visibilityBlend = null;
  }
  setVisibilityBlend(from: VisibilityInput, to: VisibilityInput, factor: number): void {
    this.visibilityBlend = { from, to, factor };
    this.visibility = factor < 0.5 ? from : to;
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
  setShape(id: string, spec: ShapeNodeSpec): void {
    this.shapes.set(id, spec);
  }
  removeShape(id: string): void {
    this.shapes.delete(id);
  }
  drawOverlay(shapes: Omit<ShapeNodeSpec, "layer">[]): void {
    this.overlay = shapes;
  }
  clearOverlay(): void {
    this.overlay = [];
  }
  drawMeasure(from: Point, to: Point, label: string): void {
    this.measure = { from, to, label };
  }
  clearMeasure(): void {
    this.measure = null;
  }
  drawPings(rings: { x: number; y: number; radius: number; alpha: number }[]): void {
    this.pings = rings;
  }
  setLighting(frame: LightingFrame): void {
    this.lighting = frame;
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

  /** Test helper: drive the ticker by `ms` milliseconds in one shot. */
  runTicker(ms: number): void {
    this.tick!(ms);
  }

  /** Test helper: read the current rendered x of a token. */
  lastTokenX(id: string): number {
    return this.tokens.get(id)!.x;
  }
}
