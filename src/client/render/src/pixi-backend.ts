import { Application, Container, Graphics, Sprite, Text, Assets, type Filter } from "pixi.js";
import type { DisplayBackend } from "./backend";
import type { LineSeg, CameraTransform, VisibilityInput, TokenNodeSpec, ShapeNodeSpec, Point } from "./types";

/** The real DisplayBackend over pixi.js v8. The only GL-touching module (kept out
 * of unit tests; covered by Playwright). Layer containers parent under one `world`
 * container so a single camera transform pans/zooms the whole scene. */
export class PixiBackend implements DisplayBackend {
  private readonly world = new Container();
  private readonly layers = new Map<string, Container>();
  private readonly grid = new Graphics();
  private readonly maskOverlay = new Graphics();
  private readonly toolOverlay = new Graphics();
  private readonly measureGraphics = new Graphics();
  private readonly measureText = new Text({ text: "", style: { fill: 0xffffff, fontSize: 14, fontFamily: "sans-serif" } });
  private readonly pingGraphics = new Graphics();
  private readonly shapes = new Map<string, Graphics>();
  private readonly tokens = new Map<string, Sprite>();
  /** Last-loaded image URL per token, so a tweening token doesn't reload each frame. */
  private readonly tokenUrls = new Map<string, string>();
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
      if (id === "overlays") {
        c.addChild(this.toolOverlay);
        c.addChild(this.measureGraphics);
        this.measureText.anchor.set(0.5);
        this.measureText.visible = false;
        c.addChild(this.measureText);
        c.addChild(this.pingGraphics);
      }
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

  setToken(id: string, spec: TokenNodeSpec): void {
    let sprite = this.tokens.get(id);
    if (!sprite) {
      sprite = new Sprite();
      sprite.anchor.set(0.5); // (x,y) is the token center
      this.tokens.set(id, sprite);
      this.layers.get("tokens")?.addChild(sprite);
    }
    sprite.position.set(spec.x, spec.y);
    sprite.width = spec.w;
    sprite.height = spec.h;
    sprite.angle = spec.rotation;
    // Only (re)load on a URL change — a tweening token re-pushes ~60×/s with the same url.
    if (this.tokenUrls.get(id) !== spec.url) {
      this.tokenUrls.set(id, spec.url);
      void Assets.load(spec.url).then((texture) => {
        // Bail if the sprite was removed or re-textured while loading.
        if (this.tokens.get(id) === sprite && this.tokenUrls.get(id) === spec.url) sprite.texture = texture;
      });
    }
  }

  removeToken(id: string): void {
    const sprite = this.tokens.get(id);
    if (sprite) {
      sprite.destroy();
      this.tokens.delete(id);
      this.tokenUrls.delete(id);
    }
  }

  setShape(id: string, spec: ShapeNodeSpec): void {
    let g = this.shapes.get(id);
    if (!g) {
      g = new Graphics();
      this.shapes.set(id, g);
    }
    // (Re)parent into the target layer. id→layer is stable for M8d's doc-backed shapes,
    // but addChild moves the node so a future layer-varying reconciler can't leak it.
    const layer = this.layers.get(spec.layer);
    if (layer && g.parent !== layer) layer.addChild(g);
    g.clear();
    paintShape(g, spec);
  }

  removeShape(id: string): void {
    const g = this.shapes.get(id);
    if (g) {
      g.destroy();
      this.shapes.delete(id);
    }
  }

  drawOverlay(shapes: Omit<ShapeNodeSpec, "layer">[]): void {
    this.toolOverlay.clear();
    for (const s of shapes) paintShape(this.toolOverlay, s);
  }

  clearOverlay(): void {
    this.toolOverlay.clear();
  }

  drawMeasure(from: Point, to: Point, label: string): void {
    this.measureGraphics.clear();
    this.measureGraphics.moveTo(from.x, from.y).lineTo(to.x, to.y).stroke({ width: 2, color: 0xffd400 });
    this.measureText.text = label;
    this.measureText.position.set((from.x + to.x) / 2, (from.y + to.y) / 2);
    this.measureText.visible = true;
  }

  clearMeasure(): void {
    this.measureGraphics.clear();
    this.measureText.visible = false;
  }

  drawPings(rings: { x: number; y: number; radius: number; alpha: number }[]): void {
    this.pingGraphics.clear();
    for (const r of rings) {
      this.pingGraphics.circle(r.x, r.y, r.radius).stroke({ width: 3, color: 0xffd400, alpha: r.alpha });
    }
  }

  startTicker(cb: (dtMs: number) => void): void {
    this.app.ticker.add((ticker) => cb(ticker.deltaMS));
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

/** Append one shape (a polyline/polygon subpath + its fill/stroke) onto a Graphics.
 * Does not clear, so multiple shapes can share one Graphics (the overlay). */
function paintShape(g: Graphics, spec: Omit<ShapeNodeSpec, "layer">): void {
  const p = spec.points;
  if (p.length < 4) return; // need at least two points
  g.moveTo(p[0], p[1]);
  for (let i = 2; i < p.length; i += 2) g.lineTo(p[i], p[i + 1]);
  if (spec.closed) g.closePath();
  if (spec.fill) g.fill({ color: spec.fill.color, alpha: spec.fill.alpha });
  if (spec.stroke) g.stroke({ width: spec.stroke.width, color: spec.stroke.color });
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
