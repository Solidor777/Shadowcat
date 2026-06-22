import { Application, Container, Graphics, Sprite, Assets, type Filter } from "pixi.js";
import type { DisplayBackend } from "./backend";
import type { LineSeg, CameraTransform, VisibilityInput } from "./types";

/** The real DisplayBackend over pixi.js v8. The only GL-touching module (kept out
 * of unit tests; covered by Playwright). Layer containers parent under one `world`
 * container so a single camera transform pans/zooms the whole scene. */
export class PixiBackend implements DisplayBackend {
  private readonly world = new Container();
  private readonly layers = new Map<string, Container>();
  private readonly grid = new Graphics();
  private readonly maskOverlay = new Graphics();
  private background: Sprite | null = null;
  private backgroundUrl: string | null = null;
  /** Monotonic counter disambiguating concurrent background loads. */
  private loadSeq = 0;

  constructor(private readonly app: Application) {
    this.app.stage.addChild(this.world);
  }

  ensureLayers(orderedIds: string[]): void {
    for (const id of orderedIds) {
      if (this.layers.has(id)) continue;
      const c = new Container();
      c.label = id;
      this.layers.set(id, c);
      this.world.addChild(c);
      if (id === "grid") c.addChild(this.grid);
      if (id === "mask") c.addChild(this.maskOverlay);
    }
    // Re-parent in z-order (addChild appends; order array is authoritative).
    for (const id of orderedIds) {
      const c = this.layers.get(id);
      if (c) this.world.addChild(c); // moving to top in order yields final stack
    }
  }

  setBackground(spec: { url: string } | null): void {
    if (spec === null) {
      this.loadSeq++; // invalidate any in-flight load
      this.background?.destroy();
      this.background = null;
      this.backgroundUrl = null;
      return;
    }
    if (spec.url === this.backgroundUrl) return; // steady-state no-op
    this.backgroundUrl = spec.url;
    // Guard on a monotonic load token, not URL equality: two in-flight loads of
    // the SAME url (set X → set Y → set X) would both pass a URL check and the
    // earlier one would flash a stale sprite. The token admits only the latest.
    const token = ++this.loadSeq;
    void Assets.load(spec.url).then((texture) => {
      if (token !== this.loadSeq) return; // superseded by a newer set/clear/destroy
      this.background?.destroy();
      const sprite = new Sprite(texture);
      this.background = sprite;
      this.layers.get("background")?.addChild(sprite);
    });
  }

  drawGrid(lines: LineSeg[], color: number): void {
    this.grid.clear();
    if (lines.length === 0) return; // nothing to stroke (e.g. a 0×0 viewport)
    for (const l of lines) this.grid.moveTo(l.x1, l.y1).lineTo(l.x2, l.y2);
    this.grid.stroke({ width: 1, color, alpha: 0.5 });
  }

  setCameraTransform(t: CameraTransform): void {
    this.world.position.set(t.x, t.y);
    this.world.scale.set(t.scale);
  }

  setVisibility(input: VisibilityInput): void {
    // M8 identity: empty `visible` ⇒ full visibility ⇒ transparent overlay (clear).
    // M9 draws fog occluding everything outside `visible` (+ explored), via an
    // engine-owned shader + a viewport render target plugged into this same slot.
    this.maskOverlay.clear();
    if (input.visible.length > 0) {
      // (M9) fog composition over the mask slot.
    }
  }

  addLayerFilter(layerId: string, filter: unknown): () => void {
    const c = this.layers.get(layerId);
    if (!c) return () => {};
    c.filters = [...(c.filters ?? []), filter as Filter];
    return () => {
      c.filters = (c.filters ?? []).filter((f) => f !== filter);
    };
  }

  resize(width: number, height: number): void {
    this.app.renderer.resize(width, height);
  }

  destroy(): void {
    this.loadSeq++; // invalidate any in-flight background load post-destroy
    // Release GPU resources + remove the canvas; children/textures included.
    this.app.destroy({ removeView: true }, { children: true, texture: true });
  }
}

/** Construct a PixiBackend over a canvas (async: v8 Application.init is async). */
export async function createPixiBackend(
  canvas: HTMLCanvasElement,
  opts: { background: number },
): Promise<PixiBackend> {
  const app = new Application();
  await app.init({
    canvas,
    antialias: true,
    resolution: globalThis.devicePixelRatio || 1,
    autoDensity: true,
    background: opts.background,
    preference: "webgl",
  });
  return new PixiBackend(app);
}
