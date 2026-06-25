import { Application, BlurFilter, Container, Graphics, Sprite, Text, Assets, type Filter } from "pixi.js";
import type { DisplayBackend } from "./backend";
import type { LightingFrame } from "./lighting";
import type { LineSeg, CameraTransform, VisibilityInput, TokenNodeSpec, ShapeNodeSpec, Point } from "./types";

/** The real DisplayBackend over pixi.js v8. The only GL-touching module (kept out
 * of unit tests; covered by Playwright). Layer containers parent under one `world`
 * container so a single camera transform pans/zooms the whole scene. */
export class PixiBackend implements DisplayBackend {
  private readonly world = new Container();
  private readonly layers = new Map<string, Container>();
  private readonly grid = new Graphics();
  /** Three-state fog (M9c): two stacked black sheets in the `mask` layer. `fogDark` (near-opaque)
   * shows only on UNEXPLORED area — inverse-masked by `exploredHoles` (explored ∪ visible).
   * `fogDim` (semi-transparent) shows on unexplored + explored — inverse-masked by `visibleHoles`.
   * Net: unexplored = both sheets (darkest), explored = dim only, visible = clear. */
  private readonly fogDark = new Graphics();
  private readonly fogDim = new Graphics();
  /** Inverse-mask shapes (not rendered directly): explored∪visible cut from `fogDark`, visible
   * cut from `fogDim`. */
  private readonly exploredHoles = new Graphics();
  private readonly visibleHoles = new Graphics();
  private readonly toolOverlay = new Graphics();
  private readonly measureGraphics = new Graphics();
  private readonly measureText = new Text({ text: "", style: { fill: 0xffffff, fontSize: 14, fontFamily: "sans-serif" } });
  private readonly pingGraphics = new Graphics();
  /** Per-cell darkening + tint quads for the lighting layer (M10e-3). Parented under the
   * `lighting` container, which carries a BlurFilter to soften band/edge boundaries. */
  private readonly lightingGraphics = new Graphics();
  private readonly shapes = new Map<string, Graphics>();
  private readonly tokens = new Map<string, Sprite>();
  /** Last-loaded image URL per token, so a tweening token doesn't reload each frame. */
  private readonly tokenUrls = new Map<string, string>();
  /** Faction border outline per token (absent when the token has no faction color). */
  private readonly tokenBorders = new Map<string, Graphics>();
  /** Condition badge glyph nodes per token (upright; absent when the token has no conditions). */
  private readonly tokenBadges = new Map<string, Text[]>();
  /** Last-rendered badge glyph set per token, so a tweening token (re-pushed ~60×/s with the same
   * glyphs) repositions existing Text nodes instead of reallocating them each frame. */
  private readonly tokenBadgeKeys = new Map<string, string>();
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
      if (id === "lighting") {
        c.addChild(this.lightingGraphics);
        // BlurFilter softens cell-boundary stepping artifacts between gradation bands.
        // POST_WORK: replace with radial gradient fills when PixiJS gradient API stabilises.
        // NOTE: filter is attached directly (not via addLayerFilter); future filter swaps on
        // the "lighting" layer must account for this pre-existing BlurFilter in c.filters.
        c.filters = [new BlurFilter({ strength: 8 })];
      }
      if (id === "mask") {
        // Dim sheet under the dark sheet; the hole shapes are masks, not drawn directly.
        c.addChild(this.fogDim);
        c.addChild(this.fogDark);
        c.addChild(this.exploredHoles);
        c.addChild(this.visibleHoles);
      }
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
    this.fogDark.clear();
    this.fogDim.clear();
    this.exploredHoles.clear();
    this.visibleHoles.clear();
    if (input.mode === "all") {
      this.fogDark.mask = null; // no fog (GM / no occlusion)
      this.fogDim.mask = null;
      return;
    }
    // Three-state fog: two opaque sheets over a large world region, scene-locked in the camera-
    // transformed `mask` layer (holes sit at scene positions and pan/zoom with the map).
    // `fogDark` (near-opaque) is cut by `explored ∪ visible` → remains only on UNEXPLORED area.
    // `fogDim` (semi-transparent) is cut by `visible` → remains on unexplored + explored, so
    // explored shows dimmed and visible shows clear. Empty visible + empty explored → no holes →
    // full dark fog (see nothing).
    const R = 1_000_000; // world units; the viewport shows only a portion, so this covers it
    this.fogDark.rect(-R, -R, 2 * R, 2 * R).fill({ color: 0x000000, alpha: 0.92 });
    this.fogDim.rect(-R, -R, 2 * R, 2 * R).fill({ color: 0x000000, alpha: 0.5 });
    for (const poly of input.explored) {
      if (poly.points.length >= 6) this.exploredHoles.poly(poly.points).fill({ color: 0xffffff });
    }
    for (const poly of input.visible) {
      if (poly.points.length >= 6) {
        // Visible is clear in BOTH sheets, so it is cut from explored-holes too (visible ⊆ explored).
        this.exploredHoles.poly(poly.points).fill({ color: 0xffffff });
        this.visibleHoles.poly(poly.points).fill({ color: 0xffffff });
      }
    }
    this.fogDark.setMask({ mask: this.exploredHoles, inverse: true });
    this.fogDim.setMask({ mask: this.visibleHoles, inverse: true });
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
    // Faction outline: a stroked rect centered on the token, tracking its transform.
    let border = this.tokenBorders.get(id);
    if (spec.borderColor === null) {
      if (border) {
        border.destroy();
        this.tokenBorders.delete(id);
      }
    } else {
      if (!border) {
        border = new Graphics();
        this.tokenBorders.set(id, border);
        this.layers.get("tokens")?.addChild(border);
      }
      const hw = spec.w / 2;
      const hh = spec.h / 2;
      border.clear();
      if (spec.shape === "circle") {
        border.ellipse(0, 0, hw, hh).stroke({ width: 3, color: spec.borderColor });
      } else {
        border.rect(-hw, -hh, spec.w, spec.h).stroke({ width: 3, color: spec.borderColor });
      }
      border.position.set(spec.x, spec.y);
      border.angle = spec.rotation; // degrees, like the sprite
    }
    // Condition badges: upright glyph chips along the token's top edge, tracking its position
    // (not rotation — status markers stay upright). Glyph nodes are rebuilt only when the glyph
    // set changes; a transform-only re-push (tweening token, ~60×/s) just repositions them — the
    // same alloc-avoidance the URL guard gives the sprite.
    const size = Math.max(12, Math.min(spec.w, spec.h) * 0.28);
    const place = (txt: Text, i: number): void => {
      txt.position.set(spec.x - spec.w / 2 + size / 2 + i * (size + 2), spec.y - spec.h / 2 + size / 2);
    };
    const badgeKey = spec.badges.join("");
    const existing = this.tokenBadges.get(id);
    if (existing && this.tokenBadgeKeys.get(id) === badgeKey) {
      existing.forEach(place); // glyphs unchanged: reposition only
    } else {
      if (existing) for (const b of existing) b.destroy();
      if (spec.badges.length === 0) {
        this.tokenBadges.delete(id);
        this.tokenBadgeKeys.delete(id);
      } else {
        const nodes: Text[] = [];
        spec.badges.forEach((glyph, i) => {
          const txt = new Text({ text: glyph, style: { fontSize: size, fontFamily: "sans-serif" } });
          txt.anchor.set(0.5);
          place(txt, i);
          this.layers.get("tokens")?.addChild(txt);
          nodes.push(txt);
        });
        this.tokenBadges.set(id, nodes);
        this.tokenBadgeKeys.set(id, badgeKey);
      }
    }
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
    const border = this.tokenBorders.get(id);
    if (border) {
      border.destroy();
      this.tokenBorders.delete(id);
    }
    const badges = this.tokenBadges.get(id);
    if (badges) {
      for (const b of badges) b.destroy();
      this.tokenBadges.delete(id);
    }
    this.tokenBadgeKeys.delete(id);
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

  setLighting(frame: LightingFrame): void {
    this.lightingGraphics.clear();
    // empty cells = no lighting overlay (all-clear)
    const cellSize = frame.cell;
    for (const c of frame.cells) {
      const x = c.i * cellSize, y = c.j * cellSize;
      if (c.alpha > 0) this.lightingGraphics.rect(x, y, cellSize, cellSize).fill({ color: 0x000000, alpha: c.alpha });
      if (c.tintAlpha > 0) this.lightingGraphics.rect(x, y, cellSize, cellSize).fill({ color: c.tint, alpha: c.tintAlpha });
      // V1 desaturate approximation: a low-alpha neutral wash mutes color in darkvision-only cells.
      // POST_WORK: replace with a masked ColorMatrixFilter over the scene layers for true desaturation.
      if (c.desaturate) this.lightingGraphics.rect(x, y, cellSize, cellSize).fill({ color: 0x808080, alpha: 0.18 });
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
