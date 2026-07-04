import { Application, BlurFilter, Container, Graphics, RenderTexture, Sprite, AnimatedSprite, Texture, Rectangle, Text, Assets, type Filter } from "pixi.js";
import type { DisplayBackend } from "./backend";
import type { LightingFrame } from "./lighting";
import type { LineSeg, CameraTransform, VisibilityInput, TokenNodeSpec, ShapeNodeSpec, Point, ResolvedAnimatedSource } from "./types";
import { computeAnimatedFrame } from "./token-animation";

/** Per-token render state (M10h). `container` is the outer, non-rotating node (position = token
 * center; badges are its direct children, so they stay upright); `visualContainer` rotates with
 * the token and holds the art + border. `sourceKey` guards visual (re)creation against a
 * tweening token's ~60x/s re-push with an unchanged visual. `anim` is present only while `visual`
 * is an AnimatedSprite. */
interface TokenNode {
  container: Container;
  visualContainer: Container;
  visual: Sprite | AnimatedSprite;
  border: Graphics;
  badges: Text[];
  badgeKey: string;
  sourceKey: string | null;
  anim: { fps: number; loop: boolean; frameCount: number; elapsedMs: number } | null;
}

/** Identity key for a `TokenNodeSpec.visual` — equal specs must produce an equal key so a
 * tweening token's re-push (same visual, new transform) skips texture (re)loading. */
function visualSourceKey(v: TokenNodeSpec["visual"]): string {
  return v.kind === "image" ? `image:${v.url}` : `animated:${JSON.stringify(v.source)}:${v.fps}:${v.loop}`;
}

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
  /** Cross-fade sprites: stage-level (screen-space, untransformed) overlays showing two
   * rasterized fog snapshots at complementary alpha while a vision sweep is mid-interval.
   * Hidden (and `fogDark`/`fogDim` shown) outside a blend. */
  private readonly fogBlendFrom = new Sprite();
  private readonly fogBlendTo = new Sprite();
  private fogBlendFromRT: RenderTexture | null = null;
  private fogBlendToRT: RenderTexture | null = null;
  private readonly toolOverlay = new Graphics();
  private readonly measureGraphics = new Graphics();
  private readonly measureText = new Text({ text: "", style: { fill: 0xffffff, fontSize: 14, fontFamily: "sans-serif" } });
  private readonly pingGraphics = new Graphics();
  /** Per-cell darkening + tint quads for the lighting layer (M10e-3). Parented under the
   * `lighting` container, which carries a BlurFilter to soften band/edge boundaries. */
  private readonly lightingGraphics = new Graphics();
  private readonly shapes = new Map<string, Graphics>();
  private readonly tokens = new Map<string, TokenNode>();
  private background: Sprite | null = null;
  private backgroundUrl: string | null = null;
  /** Monotonic counter disambiguating concurrent background loads. */
  private loadSeq = 0;

  constructor(private readonly app: Application) {
    this.app.stage.addChild(this.world);
    // Screen-space, added directly to the stage (not `world`) so the captured, already
    // camera-transformed fog snapshots display 1:1 without a second transform on top.
    this.fogBlendFrom.visible = false;
    this.fogBlendTo.visible = false;
    this.app.stage.addChild(this.fogBlendFrom, this.fogBlendTo);
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
    // A plain (non-blended) apply ends any in-flight cross-fade — restore the normal sheets and
    // eagerly free the last sweep's capture textures rather than leaving them GPU-resident until
    // the next setVisibilityBlend call (or backend teardown).
    this.fogBlendFrom.visible = false;
    this.fogBlendTo.visible = false;
    this.fogBlendFromRT?.destroy(true);
    this.fogBlendToRT?.destroy(true);
    this.fogBlendFromRT = null;
    this.fogBlendToRT = null;
    this.fogDark.visible = true;
    this.fogDim.visible = true;
    this.fogDark.clear();
    this.fogDim.clear();
    this.exploredHoles.clear();
    this.visibleHoles.clear();
    if (input.mode === "all") {
      this.fogDark.mask = null; // no fog (GM / no occlusion)
      this.fogDim.mask = null;
      return;
    }
    paintFogSheets(this.fogDark, this.fogDim, this.exploredHoles, this.visibleHoles, input);
  }

  /** Cross-fade the mask between two consecutive vision samples: rasterize each sample's fog
   * into a screen-sized `RenderTexture` (a scratch capture of the SAME sheet+hole technique
   * `setVisibility` draws live, positioned/scaled to the current camera transform so the two
   * snapshots line up with what's on screen), then show both as complementary-alpha sprites — an
   * actual GPU alpha blend between two rasterized states, not a polygon-vertex morph. Recaptured
   * on every call (a sweep ticks ~60/s) — the previous textures are destroyed first so a
   * multi-second sweep doesn't leak GPU memory.
   * TODO: cache/reuse the RenderTextures across ticks (recreate only on resize or fog-input
   * change) instead of a full recapture every call. */
  setVisibilityBlend(from: VisibilityInput, to: VisibilityInput, factor: number): void {
    const width = Math.max(1, this.app.screen.width);
    const height = Math.max(1, this.app.screen.height);
    this.fogBlendFromRT?.destroy(true);
    this.fogBlendToRT?.destroy(true);
    this.fogBlendFromRT = this.captureFog(from, width, height);
    this.fogBlendToRT = this.captureFog(to, width, height);
    this.fogBlendFrom.texture = this.fogBlendFromRT;
    this.fogBlendTo.texture = this.fogBlendToRT;
    const f = Math.min(1, Math.max(0, factor));
    this.fogBlendFrom.alpha = 1 - f;
    this.fogBlendTo.alpha = f;
    this.fogBlendFrom.visible = true;
    this.fogBlendTo.visible = true;
    this.fogDark.visible = false;
    this.fogDim.visible = false;
  }

  /** Rasterize one visibility sample's fog (the same sheet+hole technique as `setVisibility`)
   * into a screen-sized RenderTexture, applying the world container's CURRENT camera transform
   * to the scratch capture container so the snapshot lines up with what is on screen right now. */
  private captureFog(input: VisibilityInput, width: number, height: number): RenderTexture {
    const dark = new Graphics();
    const dim = new Graphics();
    const exploredHoles = new Graphics();
    const visibleHoles = new Graphics();
    paintFogSheets(dark, dim, exploredHoles, visibleHoles, input);
    const capture = new Container();
    capture.addChild(dim, dark, exploredHoles, visibleHoles);
    capture.position.copyFrom(this.world.position);
    capture.scale.copyFrom(this.world.scale);
    // Match the renderer's device-pixel-ratio resolution (Application runs `autoDensity: true`
    // at `devicePixelRatio`) or the capture rasterizes at 1x and visibly blurs on HiDPI displays.
    const texture = RenderTexture.create({ width, height, resolution: this.app.renderer.resolution });
    this.app.renderer.render({ container: capture, target: texture });
    capture.destroy({ children: true });
    return texture;
  }

  setToken(id: string, spec: TokenNodeSpec): void {
    let node = this.tokens.get(id);
    if (!node) node = this.createTokenNode(id);
    node.container.position.set(spec.x, spec.y);
    node.visualContainer.angle = spec.rotation; // degrees; rotates art + border, not badges
    this.updateTokenVisual(id, node, spec);
    this.updateTokenBorder(node, spec);
    this.updateTokenBadges(node, spec);
  }

  private createTokenNode(id: string): TokenNode {
    const container = new Container();
    const visualContainer = new Container();
    const visual = new Sprite();
    visual.anchor.set(0.5); // (x,y) is the token center
    const border = new Graphics();
    visualContainer.addChild(visual, border);
    container.addChild(visualContainer);
    this.layers.get("tokens")?.addChild(container);
    const node: TokenNode = { container, visualContainer, visual, border, badges: [], badgeKey: "", sourceKey: null, anim: null };
    this.tokens.set(id, node);
    return node;
  }

  private updateTokenVisual(id: string, node: TokenNode, spec: TokenNodeSpec): void {
    const key = visualSourceKey(spec.visual);
    node.visual.width = spec.w;
    node.visual.height = spec.h;
    if (node.sourceKey === key) return; // unchanged visual: a tweening token's transform-only re-push
    node.sourceKey = key;
    if (spec.visual.kind === "image") {
      if (node.visual instanceof AnimatedSprite) this.replaceVisualChild(node, new Sprite());
      node.anim = null;
      const sprite = node.visual;
      const url = spec.visual.url;
      void Assets.load(url).then((texture) => {
        if (this.tokens.get(id) === node && node.sourceKey === key) sprite.texture = texture;
      });
    } else {
      this.replaceVisualChild(node, new AnimatedSprite([Texture.EMPTY]));
      const sprite = node.visual as AnimatedSprite;
      sprite.autoUpdate = false; // driven by tickTokenAnimations, not Pixi's shared ticker
      node.anim = { fps: spec.visual.fps, loop: spec.visual.loop, frameCount: 1, elapsedMs: 0 };
      const source = spec.visual.source;
      void this.loadAnimatedTextures(source).then((textures) => {
        if (this.tokens.get(id) !== node || node.sourceKey !== key || textures.length === 0) return;
        sprite.textures = textures;
        sprite.gotoAndStop(0);
        node.anim = { fps: spec.visual.kind === "animated" ? spec.visual.fps : 1, loop: spec.visual.kind === "animated" ? spec.visual.loop : false, frameCount: textures.length, elapsedMs: 0 };
      });
    }
    node.visual.width = spec.w;
    node.visual.height = spec.h;
  }

  private replaceVisualChild(node: TokenNode, next: Sprite | AnimatedSprite): void {
    next.anchor.set(0.5);
    const i = node.visualContainer.getChildIndex(node.visual);
    node.visualContainer.removeChild(node.visual);
    node.visual.destroy();
    node.visualContainer.addChildAt(next, i);
    node.visual = next;
  }

  private async loadAnimatedTextures(source: ResolvedAnimatedSource): Promise<Texture[]> {
    if (source.type === "frames") {
      if (source.urls.length === 0) return [];
      return Promise.all(source.urls.map((url) => Assets.load<Texture>(url)));
    }
    if (!Number.isInteger(source.rows) || source.rows <= 0 || !Number.isInteger(source.cols) || source.cols <= 0) return [];
    const sheet = await Assets.load<Texture>(source.url);
    const frameW = sheet.width / source.cols;
    const frameH = sheet.height / source.rows;
    const total = source.count !== undefined ? Math.min(source.count, source.rows * source.cols) : source.rows * source.cols;
    const frames: Texture[] = [];
    for (let i = 0; i < total; i++) {
      const col = i % source.cols;
      const row = Math.floor(i / source.cols);
      frames.push(new Texture({ source: sheet.source, frame: new Rectangle(col * frameW, row * frameH, frameW, frameH) }));
    }
    return frames;
  }

  private updateTokenBorder(node: TokenNode, spec: TokenNodeSpec): void {
    const hw = spec.w / 2;
    const hh = spec.h / 2;
    node.border.clear();
    if (spec.borderColor === null) return;
    if (spec.shape === "circle") node.border.ellipse(0, 0, hw, hh).stroke({ width: 3, color: spec.borderColor });
    else node.border.rect(-hw, -hh, spec.w, spec.h).stroke({ width: 3, color: spec.borderColor });
  }

  private updateTokenBadges(node: TokenNode, spec: TokenNodeSpec): void {
    // Upright glyph chips along the token's top edge, relative to the (non-rotating) outer
    // container's own origin — badges are its direct children, so they stay upright automatically
    // when visualContainer (the sibling holding the rotating art+border) rotates.
    const size = Math.max(12, Math.min(spec.w, spec.h) * 0.28);
    const place = (txt: Text, i: number): void => {
      txt.position.set(-spec.w / 2 + size / 2 + i * (size + 2), -spec.h / 2 + size / 2);
    };
    const badgeKey = spec.badges.join("");
    if (node.badgeKey === badgeKey) {
      node.badges.forEach(place);
      return;
    }
    for (const b of node.badges) b.destroy();
    node.badgeKey = badgeKey;
    node.badges = spec.badges.map((glyph, i) => {
      const txt = new Text({ text: glyph, style: { fontSize: size, fontFamily: "sans-serif" } });
      txt.anchor.set(0.5);
      place(txt, i);
      node.container.addChild(txt);
      return txt;
    });
  }

  removeToken(id: string): void {
    const node = this.tokens.get(id);
    if (!node) return;
    node.container.destroy({ children: true });
    this.tokens.delete(id);
  }

  tickTokenAnimations(dtMs: number): void {
    for (const node of this.tokens.values()) {
      if (!node.anim || !(node.visual instanceof AnimatedSprite)) continue;
      node.anim.elapsedMs += dtMs;
      const frame = computeAnimatedFrame(node.anim.elapsedMs, node.anim.fps, node.anim.frameCount, node.anim.loop);
      if (node.visual.currentFrame !== frame) node.visual.gotoAndStop(frame);
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
    // Destroy the blend RenderTextures explicitly first: they are NOT shared/cached (unlike
    // Assets.load results), so the stage-destroy cascade below must not be their only owner.
    this.fogBlendFromRT?.destroy(true);
    this.fogBlendToRT?.destroy(true);
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

/** Paint the three-state fog (unexplored darkest / explored dimmed / visible clear) onto the
 * given sheet + hole Graphics: two opaque sheets over a large world region, cut by inverse
 * masks built from the input's `explored`/`visible` polygons. Shared by the live `mask` layer
 * (`setVisibility`) and the cross-fade capture (`captureFog`) so both draw IDENTICAL fog for
 * the same input — the cross-fade blends between two genuinely equivalent renders, not a
 * lookalike approximation. `mode:"all"` paints nothing (caller handles the no-fog case). */
function paintFogSheets(dark: Graphics, dim: Graphics, exploredHoles: Graphics, visibleHoles: Graphics, input: VisibilityInput): void {
  if (input.mode !== "masked") return;
  const R = 1_000_000; // world units; the viewport shows only a portion, so this covers it
  dark.rect(-R, -R, 2 * R, 2 * R).fill({ color: 0x000000, alpha: 0.92 });
  dim.rect(-R, -R, 2 * R, 2 * R).fill({ color: 0x000000, alpha: 0.5 });
  for (const poly of input.explored) {
    if (poly.points.length >= 6) exploredHoles.poly(poly.points).fill({ color: 0xffffff });
  }
  for (const poly of input.visible) {
    if (poly.points.length >= 6) {
      // Visible is clear in BOTH sheets, so it is cut from explored-holes too (visible ⊆ explored).
      exploredHoles.poly(poly.points).fill({ color: 0xffffff });
      visibleHoles.poly(poly.points).fill({ color: 0xffffff });
    }
  }
  dark.setMask({ mask: exploredHoles, inverse: true });
  dim.setMask({ mask: visibleHoles, inverse: true });
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
