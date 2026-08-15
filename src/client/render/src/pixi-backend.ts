import { Application, BlurFilter, Container, Graphics, RenderTexture, Sprite, AnimatedSprite, Texture, Rectangle, Text, Assets, type Filter } from "pixi.js";
import type { DisplayBackend, BackgroundSpec } from "./backend";
import type { LightingFrame } from "./lighting";
import type { LineSeg, CameraTransform, VisibilityInput, TokenNodeSpec, ShapeNodeSpec, Point, ResolvedAnimatedSource } from "./types";
import { computeAnimatedFrame } from "./token-animation";
import { fogBlendRtStale } from "./fog-blend";
import type { PingRing } from "./ping-view";

/** Initial renderer options for `createPixiBackend`. */
export interface PixiBackendOptions {
  /** The initial background clear color, packed `0xRRGGBB`. */
  background: number;
}

/** Per-token render state. `container` is the outer, non-rotating node (position = token
 * center; badges are its direct children, so they stay upright); `visualContainer` rotates with
 * the token and holds the art + border. `sourceKey` guards visual (re)creation against a
 * tweening token's ~60x/s re-push with an unchanged visual. `anim` is present only while `visual`
 * is an AnimatedSprite. */
interface TokenNode {
  /** Outer, non-rotating node — its position is the token center; `badges` are its direct
   * children so they stay upright regardless of `visualContainer`'s rotation. */
  container: Container;
  /** Inner node that rotates with the token (`.angle = tokenSpec.rotation`); holds `visual` +
   * `border`. */
  visualContainer: Container;
  /** The art sprite — a plain `Sprite` for an image visual, an `AnimatedSprite` while
   * `tokenSpec.visual.kind === "animated"`. */
  visual: Sprite | AnimatedSprite;
  /** Faction-border outline, redrawn by `updateTokenBorder`; cleared (no stroke) when
   * `tokenSpec.borderColor` is `null`. */
  border: Graphics;
  /** Condition-marker glyph chips, one `Text` per `tokenSpec.badges` entry, in order. */
  badges: Text[];
  /** `tokenSpec.badges.join("")`, memoized by `updateTokenBadges` to skip a full badge-set rebuild
   * when the badge list is unchanged. */
  badgeKey: string;
  /** `visualSourceKey(tokenSpec.visual)` of the last-applied visual, or `null` before the first
   * `setToken` call — an unchanged key short-circuits `updateTokenVisual`'s reload. */
  sourceKey: string | null;
  /** Tick-driven animation state, or `null` while `visual` is a plain (non-animated) `Sprite`. */
  anim: {
    /** Playback rate, in frames per second. */
    fps: number;
    /** `true` wraps at the end of the sequence; `false` holds the final frame. */
    loop: boolean;
    /** Total frame count in the resolved source; `1` while the async load is still pending. */
    frameCount: number;
    /** Accumulated elapsed time, in ms, since this visual was assigned — advanced by
     * `tickTokenAnimations`. */
    elapsedMs: number;
  } | null;
}

/** Identity key for a `TokenNodeSpec.visual` — equal specs must produce an equal key so a
 * tweening token's re-push (same visual, new transform) skips texture (re)loading.
 * @param v A token's resolved visual tokenSpec (image URL, or an animated source + fps/loop).
 * @returns A string key equal for equal specs; an `"image:"`- vs `"animated:"`-prefixed key never
 * collides across kinds.
 * @example
 * ```
 * // module-private helper; not exported from @shadowcat/render
 * visualSourceKey({ kind: "image", url: "https://example.test/token.png" });
 * ```
 */
function visualSourceKey(v: TokenNodeSpec["visual"]): string {
  return v.kind === "image" ? `image:${v.url}` : `animated:${JSON.stringify(v.source)}:${v.fps}:${v.loop}`;
}

/** The real DisplayBackend over pixi.js v8. The only GL-touching module (kept out
 * of unit tests; covered by Playwright). Layer containers parent under one `world`
 * container so a single camera transform pans/zooms the whole scene. */
export class PixiBackend implements DisplayBackend {
  /** Container every layer/the camera transform is parented under (`setCameraTransform` moves
   * this, not the stage). */
  private readonly world = new Container();
  /** Core layer id → its Container, populated by `ensureLayers`. */
  private readonly layers = new Map<string, Container>();
  /** The `grid`-layer line strokes, redrawn wholesale by `drawGrid`. */
  private readonly grid = new Graphics();
  /** Three-state fog: two stacked black sheets in the `mask` layer. `fogDark` (near-opaque)
   * shows only on UNEXPLORED area — inverse-masked by `exploredHoles` (explored ∪ visible).
   * `fogDim` (semi-transparent) shows on unexplored + explored — inverse-masked by `visibleHoles`.
   * Net: unexplored = both sheets (darkest), explored = dim only, visible = clear. */
  private readonly fogDark = new Graphics();
  /** Semi-transparent "explored memory" sheet — see `fogDark`'s doc for the two-sheet fog
   * model. */
  private readonly fogDim = new Graphics();
  /** Inverse-mask shapes (not rendered directly): explored∪visible cut from `fogDark`, visible
   * cut from `fogDim`. */
  private readonly exploredHoles = new Graphics();
  /** Visible-only inverse-mask shape cut from `fogDim` — see `exploredHoles`'s doc. */
  private readonly visibleHoles = new Graphics();
  /** Cross-fade sprites: stage-level (screen-space, untransformed) overlays showing two
   * rasterized fog snapshots at complementary alpha while a vision sweep is mid-interval.
   * Hidden (and `fogDark`/`fogDim` shown) outside a blend. */
  private readonly fogBlendFrom = new Sprite();
  /** The incoming (newer) cross-fade sprite — see `fogBlendFrom`'s doc. */
  private readonly fogBlendTo = new Sprite();
  /** `fogBlendFrom`'s captured texture, reused across calls until `fogBlendRtStale` finds it
   * mismatched (`null` before the first cross-fade, or right after a stale-size discard). */
  private fogBlendFromRT: RenderTexture | null = null;
  /** `fogBlendTo`'s captured texture — see `fogBlendFromRT`'s doc. */
  private fogBlendToRT: RenderTexture | null = null;
  /** The `overlays`-layer tool-preview Graphics, redrawn by `drawOverlay`/`clearOverlay`. */
  private readonly toolOverlay = new Graphics();
  /** The measurement segment stroke, redrawn by `drawMeasure`/`clearMeasure`. */
  private readonly measureGraphics = new Graphics();
  /** The measurement distance label, positioned at the segment midpoint by `drawMeasure`;
   * hidden (not destroyed) by `clearMeasure`. */
  private readonly measureText = new Text({ text: "", style: { fill: 0xffffff, fontSize: 14, fontFamily: "sans-serif" } });
  /** The ping-ring overlay, redrawn wholesale by `drawPings`. */
  private readonly pingGraphics = new Graphics();
  /** Per-cell darkening + tint quads for the lighting layer. Parented under the
   * `lighting` container, which carries a BlurFilter to soften band/edge boundaries. */
  private readonly lightingGraphics = new Graphics();
  /** Shape document id → its Graphics, populated by `setShape`. */
  private readonly shapes = new Map<string, Graphics>();
  /** Token document id → its render node, populated by `createTokenNode`. */
  private readonly tokens = new Map<string, TokenNode>();
  /** The current background sprite, or `null` before the first `setBackground` call/after a
   * clear. */
  private background: Sprite | null = null;
  /** `backgroundUrl` of the currently-displayed sprite — used to skip a redundant reload when
   * `setBackground` is called with an unchanged `url`. */
  private backgroundUrl: string | null = null;
  /** Monotonic counter disambiguating concurrent background loads. */
  private loadSeq = 0;

  /** Wire this backend to a running Pixi `Application`: parents the `world` container (which
   * every layer/camera transform lives under) onto the stage, and adds the cross-fade sprites
   * (`fogBlendFrom`/`fogBlendTo`) directly to the stage — not `world` — so they render in screen
   * space, unaffected by the camera transform `setCameraTransform` applies to `world`. Both
   * cross-fade sprites start hidden; `setVisibilityBlend` reveals them.
   * @param app A Pixi `Application` already initialized via `app.init(...)` — see
   * {@link createPixiBackend}, the normal construction path.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * // Derived from PixiBackend's own constructor rather than naming pixi.js's
   * // `Application` type directly, so this example stays correct even if the
   * // constructor's parameter type is ever narrowed/widened independently of
   * // pixi.js's own export surface.
   * declare const app: ConstructorParameters<typeof PixiBackend>[0];
   * const backend = new PixiBackend(app);
   * ```
   */
  constructor(private readonly app: Application) {
    this.app.stage.addChild(this.world);
    // Screen-space, added directly to the stage (not `world`) so the captured, already
    // camera-transformed fog snapshots display 1:1 without a second transform on top.
    this.fogBlendFrom.visible = false;
    this.fogBlendTo.visible = false;
    this.app.stage.addChild(this.fogBlendFrom, this.fogBlendTo);
  }

  /** `DisplayBackend.ensureLayers`: create any layer Containers named in `orderedIds` that don't
   * exist yet (parenting their fixed children — e.g. `grid`'s Graphics, `mask`'s fog sheets,
   * `lighting`'s BlurFilter — exactly once, on first creation), then re-parent EVERY container in
   * `orderedIds` order via `addChild` (which moves an already-parented child to the top) so the
   * final z-order matches `orderedIds` regardless of creation order or how many times this is
   * called — for a repeat call with the SAME id set (same-order re-append, a no-op in effect) or a
   * GROWING one (new ids appended on top). **A SHRINKING set does not remove the omitted layer —
   * it sinks to the bottom of the z-stack instead:** the reorder loop only re-parents ids present
   * in THIS call's `orderedIds`, so a Container created by an earlier call but omitted from a
   * later one is never removed from `this.layers`/`world`'s children; it simply isn't touched by
   * this call's re-append, and everything that IS re-appended moves above it. `MockBackend`'s
   * `ensureLayers` diverges here: it does `this.layers = [...orderedIds]`, a full replace, so an
   * omitted layer id is gone entirely from the recorded state, as if never created. No current
   * caller ever shrinks the set (the sole call site, `RenderEngine.start()`, passes a fixed
   * `this.layers.orderedIds()` once per engine instance) — a live gap, not a live bug.
   * @param orderedIds The z-order (bottom to top) of core layer ids to ensure exist.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.ensureLayers(["background", "grid", "tokens", "mask", "overlays"]);
   * ```
   */
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
        // TODO: replace with radial gradient fills when PixiJS gradient API stabilises.
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

  /** `DisplayBackend.setBackground`: set or clear the background-layer sprite. `null` invalidates
   * any in-flight load (bumping `loadSeq`) and destroys the current sprite. A non-null tokenSpec whose
   * `url` already matches the current background is a steady-state no-op (skips a redundant
   * reload). Otherwise loads the texture asynchronously (`Assets.load`) and swaps the sprite in
   * once the load resolves, guarded by a monotonic `loadSeq` token — not a URL comparison — so a
   * rapid set-X → set-Y → set-X sequence can't let the now-superseded first X load win the race
   * and flash a stale sprite after Y's more recent commit.
   * @param spec The background image `url` to load and display, or `null` to clear it.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.setBackground({ url: "https://example.test/map.png" });
   * backend.setBackground(null); // clear
   * ```
   */
  setBackground(spec: BackgroundSpec | null): void {
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

  /** `DisplayBackend.drawGrid`: replace the grid-layer line set. Clears the grid Graphics first
   * (so a shrinking line set doesn't leave stale strokes), then strokes each segment at 1px width,
   * 50% alpha. An empty `lines` array leaves the grid blank (e.g. a 0×0 viewport) rather than
   * erroring.
   * @param lines The grid line segments to stroke, in scene coordinates.
   * @param color The stroke color, packed `0xRRGGBB`.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.drawGrid([{ x1: 0, y1: 0, x2: 100, y2: 0 }], 0x3a3a4a);
   * ```
   */
  drawGrid(lines: LineSeg[], color: number): void {
    this.grid.clear();
    if (lines.length === 0) return; // nothing to stroke (e.g. a 0×0 viewport)
    for (const l of lines) this.grid.moveTo(l.x1, l.y1).lineTo(l.x2, l.y2);
    this.grid.stroke({ width: 1, color, alpha: 0.5 });
  }

  /** `DisplayBackend.setCameraTransform`: apply the camera transform to the `world` container —
   * translate then uniform scale, matching every layer parented under `world` (everything except
   * the stage-level fog cross-fade sprites, which are deliberately screen-space — see the
   * constructor doc).
   * @param t The camera transform: translation `(x,y)` plus uniform `scale`.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.setCameraTransform({ x: 0, y: 0, scale: 1 });
   * ```
   */
  setCameraTransform(t: CameraTransform): void {
    this.world.position.set(t.x, t.y);
    this.world.scale.set(t.scale);
  }

  /** `DisplayBackend.setVisibility`: apply the fog mask, ending any in-flight cross-fade
   * (`setVisibilityBlend`) — restores the normal `fogDark`/`fogDim` sheets, and eagerly destroys
   * the last cross-fade capture textures (`fogBlendFromRT`/`fogBlendToRT`) rather than leaving
   * them GPU-resident until the next blend call or backend teardown. `mode:"all"` clears both
   * sheets' masks (no fog); `mode:"masked"` repaints them via `paintFogSheets` — the same helper
   * `captureFog` uses for a cross-fade capture, so a plain apply and a captured snapshot of the
   * same input are pixel-identical.
   * @param input The visibility mask to apply (empty `visible`/`explored` under `mode:"masked"` ⇒
   * full fog).
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.setVisibility({ mode: "all", visible: [], explored: [] });
   * ```
   */
  setVisibility(input: VisibilityInput): void {
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
   * on every call (a sweep ticks ~60/s), but `fogBlendFromRT`/`fogBlendToRT` themselves are
   * reused across calls (`captureFog` renders fresh content into the same texture, which the
   * renderer clears before drawing) — only destroyed and recreated when `fogBlendRtStale` finds
   * the renderer's current size/resolution no longer matches (first call, a window resize, or a
   * DPR change), avoiding a GPU alloc/free pair on every one of the ~60 calls/sec a sweep makes.
   * @param from The outgoing sample's visibility mask.
   * @param to The incoming sample's visibility mask.
   * @param factor Blend position in `[0,1]`, clamped: 0 shows `from` fully opaque, 1 shows `to`
   * fully opaque.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.setVisibilityBlend(
   *   { mode: "masked", visible: [], explored: [] },
   *   { mode: "masked", visible: [], explored: [] },
   *   0.5,
   * );
   * ```
   */
  setVisibilityBlend(from: VisibilityInput, to: VisibilityInput, factor: number): void {
    const width = Math.max(1, this.app.screen.width);
    const height = Math.max(1, this.app.screen.height);
    const resolution = this.app.renderer.resolution;
    const current = this.fogBlendFromRT ? { width: this.fogBlendFromRT.width, height: this.fogBlendFromRT.height, resolution: this.fogBlendFromRT.source.resolution } : null;
    if (fogBlendRtStale(current, width, height, resolution)) {
      this.fogBlendFromRT?.destroy(true);
      this.fogBlendToRT?.destroy(true);
      this.fogBlendFromRT = null;
      this.fogBlendToRT = null;
    }
    this.fogBlendFromRT = this.captureFog(from, width, height, resolution, this.fogBlendFromRT);
    this.fogBlendToRT = this.captureFog(to, width, height, resolution, this.fogBlendToRT);
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
   * to the scratch capture container so the snapshot lines up with what is on screen right now.
   * Reuses `existing` (rendering into it clears+overwrites its prior content) when given, so a
   * caller can hold a `RenderTexture` across ticks; creates a fresh one when `existing` is
   * `null` (first call, or `setVisibilityBlend` just discarded a stale-sized pair).
   * @param input The visibility sample to rasterize (same shape `setVisibility` accepts).
   * @param width Capture width in CSS pixels (the current `app.screen.width`, clamped to `>=1`).
   * @param height Capture height in CSS pixels (the current `app.screen.height`, clamped to
   * `>=1`).
   * @param resolution Device-pixel-ratio resolution to render at (matches
   * `app.renderer.resolution` so the capture doesn't blur on HiDPI displays).
   * @param existing A `RenderTexture` to render into (cleared and overwritten in place), or `null`
   * to allocate a fresh one.
   * @returns The rasterized `RenderTexture` — `existing` if given, otherwise a newly allocated one.
   * @example
   * ```
   * // private method; not part of the public API
   * this.captureFog({ mode: "all", visible: [], explored: [] }, 800, 600, 1, null);
   * ```
   */
  private captureFog(input: VisibilityInput, width: number, height: number, resolution: number, existing: RenderTexture | null): RenderTexture {
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
    const texture = existing ?? RenderTexture.create({ width, height, resolution });
    this.app.renderer.render({ container: capture, target: texture });
    capture.destroy({ children: true });
    return texture;
  }

  /** `DisplayBackend.setToken`: upsert a token render node — create one (`createTokenNode`) if
   * `id` is new, then update its transform, visual (image or animated), border, and badges in
   * place. `visualContainer.angle` rotates the art + border only; `container`'s own position is
   * the token center and its badge children never rotate (see `TokenNode`'s field doc).
   * @param id The token document id.
   * @param tokenSpec The resolved token render tokenSpec (transform, size, visual, border, badges, shape).
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.setToken("00000000-0000-0000-0000-000000000001", {
   *   x: 0, y: 0, w: 70, h: 70, rotation: 0,
   *   visual: { kind: "image", url: "https://example.test/token.png" },
   *   borderColor: null, badges: [], shape: "square",
   * });
   * ```
   */
  setToken(id: string, tokenSpec: TokenNodeSpec): void {
    let node = this.tokens.get(id);
    if (!node) node = this.createTokenNode(id);
    node.container.position.set(tokenSpec.x, tokenSpec.y);
    node.visualContainer.angle = tokenSpec.rotation; // degrees; rotates art + border, not badges
    this.updateTokenVisual(id, node, tokenSpec);
    this.updateTokenBorder(node, tokenSpec);
    this.updateTokenBadges(node, tokenSpec);
  }

  /** Construct a new `TokenNode`: an outer non-rotating `container` (positioned at the token
   * center in `setToken`) holding an inner `visualContainer` (which rotates), which in turn holds
   * a placeholder `Sprite` visual (anchored at its center, `(0.5,0.5)`) plus an empty `border`
   * Graphics. Parents `container` into the `tokens` layer and registers the node in `this.tokens`
   * keyed by `id`.
   * @param id The token document id to register the node under.
   * @returns The newly created, registered `TokenNode`.
   * @example
   * ```
   * // private method; not part of the public API
   * this.createTokenNode("00000000-0000-0000-0000-000000000001");
   * ```
   */
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

  /** Resolve `tokenSpec.visual` onto `node.visual`, sized to `tokenSpec.w`×`tokenSpec.h` on both entry and exit
   * (so a size-only re-push still applies even when the visual itself is unchanged — see the
   * `sourceKey` short-circuit below). Guarded by `visualSourceKey(tokenSpec.visual)`: an unchanged key
   * is a tweening token's transform-only re-push and returns immediately without touching
   * `node.visual`/reloading anything. On a changed key: swaps `node.visual` to a plain `Sprite`
   * (image) or an `AnimatedSprite` (animated) via `replaceVisualChild` only when the CURRENT node
   * doesn't already have that kind of sprite (an image→image or animated→animated key change
   * reuses the existing sprite object, avoiding a needless destroy/recreate), sets `node.anim`
   * (`null` for image; a placeholder `{frameCount:1}` for animated, pending the async
   * texture/frame load), then kicks off the async load (`Assets.load` for an image,
   * `loadAnimatedTextures` for animated). Each load's completion callback re-checks object
   * identity — `this.tokens.get(id) === node`, `node.visual === sprite`, AND `node.sourceKey ===
   * key` — before writing the resolved texture/frames, so a stale in-flight load from a visual
   * that has since been replaced (or a token that has since been removed) can never write into an
   * already-destroyed or already-superseded Pixi object.
   * @param id The token document id (used to re-check `this.tokens.get(id) === node` in the async
   * completion callback).
   * @param node The token's render node, mutated in place.
   * @param tokenSpec The resolved token tokenSpec; only `.visual`/`.w`/`.h` are read here.
   * @example
   * ```
   * // private method; not part of the public API
   * declare const node: TokenNode;
   * declare const tokenSpec: TokenNodeSpec;
   * this.updateTokenVisual("00000000-0000-0000-0000-000000000001", node, tokenSpec);
   * ```
   */
  private updateTokenVisual(id: string, node: TokenNode, tokenSpec: TokenNodeSpec): void {
    const key = visualSourceKey(tokenSpec.visual);
    node.visual.width = tokenSpec.w;
    node.visual.height = tokenSpec.h;
    if (node.sourceKey === key) return; // unchanged visual: a tweening token's transform-only re-push
    node.sourceKey = key;
    if (tokenSpec.visual.kind === "image") {
      if (node.visual instanceof AnimatedSprite) this.replaceVisualChild(node, new Sprite());
      node.anim = null;
      const sprite = node.visual;
      const url = tokenSpec.visual.url;
      void Assets.load(url).then((texture) => {
        if (this.tokens.get(id) === node && node.visual === sprite && node.sourceKey === key) sprite.texture = texture;
      });
    } else {
      // Hoist the narrowed "animated" variant into its own binding: `tokenSpec.visual` re-read inside
      // an async closure loses the enclosing if/else narrowing (a fresh property read on a union),
      // so a plain `tokenSpec.visual.fps` there does not typecheck without this.
      const visual = tokenSpec.visual;
      if (!(node.visual instanceof AnimatedSprite)) this.replaceVisualChild(node, new AnimatedSprite([Texture.EMPTY]));
      const sprite = node.visual as AnimatedSprite;
      sprite.autoUpdate = false; // driven by tickTokenAnimations, not Pixi's shared ticker
      node.anim = { fps: visual.fps, loop: visual.loop, frameCount: 1, elapsedMs: 0 };
      const source = visual.source;
      void this.loadAnimatedTextures(source).then((textures) => {
        if (this.tokens.get(id) !== node || node.visual !== sprite || node.sourceKey !== key || textures.length === 0) return;
        sprite.textures = textures;
        sprite.gotoAndStop(0);
        node.anim = { fps: visual.fps, loop: visual.loop, frameCount: textures.length, elapsedMs: 0 };
      });
    }
    node.visual.width = tokenSpec.w;
    node.visual.height = tokenSpec.h;
  }

  /** Swap `node.visual` for `next`: re-anchors `next` to `(0.5,0.5)`, removes and destroys the old
   * `node.visual` from `visualContainer`, inserts `next` at the SAME child index (preserving
   * `visual`/`border` sibling order), and updates `node.visual` to point at it.
   * @param node The token render node whose visual to replace.
   * @param next The new visual object (a fresh `Sprite` or `AnimatedSprite`) to install.
   * @example
   * ```
   * // private method; not part of the public API
   * declare const node: TokenNode;
   * this.replaceVisualChild(node, new Sprite());
   * ```
   */
  private replaceVisualChild(node: TokenNode, next: Sprite | AnimatedSprite): void {
    next.anchor.set(0.5);
    const i = node.visualContainer.getChildIndex(node.visual);
    node.visualContainer.removeChild(node.visual);
    node.visual.destroy();
    node.visualContainer.addChildAt(next, i);
    node.visual = next;
  }

  /** Resolve an animated visual's source into an ordered `Texture` array: `"frames"` loads each
   * URL independently via `Assets.load`; `"sheet"` loads one sprite-sheet image and slices it into
   * `rows`×`cols` equal cells (row-major), capped at `source.count` when given (never exceeding
   * `rows*cols`). Fails closed to an empty array — never throws — on a `"frames"` source with no
   * URLs, or a `"sheet"` source with a non-positive/non-integer `rows`/`cols`; the caller
   * (`updateTokenVisual`) treats an empty result as "load produced nothing" and leaves `node.anim`
   * at its placeholder state.
   * @param source The resolved animated source (already asset-id-resolved to serve URLs).
   * @returns The ordered animation-frame textures, or `[]` on a degenerate/empty source.
   * @example
   * ```
   * // private method; not part of the public API
   * await this.loadAnimatedTextures({ type: "frames", urls: [] }); // []
   * ```
   */
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

  /** Redraw `node.border`: clears it first, then strokes an ellipse (`shape:"circle"`) or rect
   * (otherwise) sized to `tokenSpec.w`×`tokenSpec.h`, centered on the visual's own origin. `borderColor:
   * null` leaves the border cleared (no stroke) — the caller's way of expressing "no faction
   * border".
   * @param node The token render node whose border to redraw.
   * @param tokenSpec The resolved token tokenSpec; only `.w`/`.h`/`.shape`/`.borderColor` are read here.
   * @example
   * ```
   * // private method; not part of the public API
   * declare const node: TokenNode;
   * declare const tokenSpec: TokenNodeSpec;
   * this.updateTokenBorder(node, tokenSpec);
   * ```
   */
  private updateTokenBorder(node: TokenNode, tokenSpec: TokenNodeSpec): void {
    const hw = tokenSpec.w / 2;
    const hh = tokenSpec.h / 2;
    node.border.clear();
    if (tokenSpec.borderColor === null) return;
    if (tokenSpec.shape === "circle") node.border.ellipse(0, 0, hw, hh).stroke({ width: 3, color: tokenSpec.borderColor });
    else node.border.rect(-hw, -hh, tokenSpec.w, tokenSpec.h).stroke({ width: 3, color: tokenSpec.borderColor });
  }

  /** Redraw `node`'s condition-marker badges: emoji glyph chips laid out left-to-right along the
   * token's top edge, sized to `max(12, min(w,h)*0.28)`px and positioned relative to the
   * non-rotating outer `container`'s own origin (so they stay upright regardless of
   * `visualContainer`'s rotation — see `TokenNode`'s field doc). Guarded by a joined `badgeKey`:
   * an unchanged badge list only re-places the existing `Text` nodes (`place`); a changed one
   * destroys every existing badge and rebuilds the set from scratch.
   * @param node The token render node whose badges to redraw.
   * @param tokenSpec The resolved token tokenSpec; only `.w`/`.h`/`.badges` are read here.
   * @example
   * ```
   * // private method; not part of the public API
   * declare const node: TokenNode;
   * declare const tokenSpec: TokenNodeSpec;
   * this.updateTokenBadges(node, tokenSpec);
   * ```
   */
  private updateTokenBadges(node: TokenNode, tokenSpec: TokenNodeSpec): void {
    // Upright glyph chips along the token's top edge, relative to the (non-rotating) outer
    // container's own origin — badges are its direct children, so they stay upright automatically
    // when visualContainer (the sibling holding the rotating art+border) rotates.
    const size = Math.max(12, Math.min(tokenSpec.w, tokenSpec.h) * 0.28);
    const place = (txt: Text, i: number): void => {
      txt.position.set(-tokenSpec.w / 2 + size / 2 + i * (size + 2), -tokenSpec.h / 2 + size / 2);
    };
    const badgeKey = tokenSpec.badges.join("");
    if (node.badgeKey === badgeKey) {
      node.badges.forEach(place);
      return;
    }
    for (const b of node.badges) b.destroy();
    node.badgeKey = badgeKey;
    node.badges = tokenSpec.badges.map((glyph, i) => {
      const txt = new Text({ text: glyph, style: { fontSize: size, fontFamily: "sans-serif" } });
      txt.anchor.set(0.5);
      place(txt, i);
      node.container.addChild(txt);
      return txt;
    });
  }

  /** `DisplayBackend.removeToken`: destroy a token's render node (container + all children,
   * including `visualContainer`/`visual`/`border`/badges) and drop it from `this.tokens`. A no-op
   * for an unknown `id`.
   * @param id The token document id to remove.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.removeToken("00000000-0000-0000-0000-000000000001");
   * ```
   */
  removeToken(id: string): void {
    const node = this.tokens.get(id);
    if (!node) return;
    node.container.destroy({ children: true });
    this.tokens.delete(id);
  }

  /** `DisplayBackend.tickTokenAnimations`: advance every token's `AnimatedSprite` by `dtMs`. Skips
   * any token with no `node.anim` (an image-visual token, or an animated one whose texture load
   * hasn't resolved yet) or whose `node.visual` isn't currently an `AnimatedSprite`. Frame index is
   * computed by `computeAnimatedFrame` (pure, tick-driven — `autoUpdate` is `false`, so Pixi's own
   * shared ticker never advances these sprites); `gotoAndStop` is called only when the computed
   * frame differs from `currentFrame`, avoiding a redundant per-frame write. `MockBackend`'s
   * implementation of this same method is an intentional no-op — frame-advance state lives only in
   * these real `AnimatedSprite` objects, which the mock never creates.
   * @param dtMs Milliseconds elapsed since the previous tick.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.tickTokenAnimations(16);
   * ```
   */
  tickTokenAnimations(dtMs: number): void {
    for (const node of this.tokens.values()) {
      if (!node.anim || !(node.visual instanceof AnimatedSprite)) continue;
      node.anim.elapsedMs += dtMs;
      const frame = computeAnimatedFrame(node.anim.elapsedMs, node.anim.fps, node.anim.frameCount, node.anim.loop);
      if (node.visual.currentFrame !== frame) node.visual.gotoAndStop(frame);
    }
  }

  /** `DisplayBackend.setShape`: upsert a drawn shape node — creates a `Graphics` for a new `id`,
   * (re)parents it into `tokenSpec.layer` if it isn't already there (an id→layer change moves the node
   * rather than leaking the old parent), clears it, then paints `tokenSpec` via `paintShape`.
   * @param id The shape document id.
   * @param tokenSpec The resolved shape tokenSpec (target layer, points, fill/stroke).
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.setShape("00000000-0000-0000-0000-000000000001", {
   *   layer: "drawings", points: [0, 0, 10, 0, 10, 10, 0, 10], closed: true,
   *   stroke: { color: 0xffffff, width: 2 }, fill: null,
   * });
   * ```
   */
  setShape(id: string, tokenSpec: ShapeNodeSpec): void {
    let g = this.shapes.get(id);
    if (!g) {
      g = new Graphics();
      this.shapes.set(id, g);
    }
    // (Re)parent into the target layer. id→layer is stable for doc-backed shapes,
    // but addChild moves the node so a future layer-varying reconciler can't leak it.
    const layer = this.layers.get(tokenSpec.layer);
    if (layer && g.parent !== layer) layer.addChild(g);
    g.clear();
    paintShape(g, tokenSpec);
  }

  /** `DisplayBackend.removeShape`: destroy a drawn shape node and drop it from `this.shapes`. A
   * no-op for an unknown `id`.
   * @param id The shape document id to remove.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.removeShape("00000000-0000-0000-0000-000000000001");
   * ```
   */
  removeShape(id: string): void {
    const g = this.shapes.get(id);
    if (g) {
      g.destroy();
      this.shapes.delete(id);
    }
  }

  /** `DisplayBackend.drawOverlay`: replace the ephemeral tool-preview overlay. Clears the shared
   * `toolOverlay` Graphics, then paints every shape in order onto it (`paintShape` appends without
   * clearing, so multiple shapes share one Graphics object).
   * @param shapes The preview shapes to draw, in paint order.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.drawOverlay([{ points: [0, 0, 10, 0, 10, 10, 0, 10], closed: true, stroke: null, fill: null }]);
   * ```
   */
  drawOverlay(shapes: Omit<ShapeNodeSpec, "layer">[]): void {
    this.toolOverlay.clear();
    for (const s of shapes) paintShape(this.toolOverlay, s);
  }

  /** `DisplayBackend.clearOverlay`: clear the ephemeral tool-preview overlay.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.clearOverlay();
   * ```
   */
  clearOverlay(): void {
    this.toolOverlay.clear();
  }

  /** `DisplayBackend.drawMeasure`: draw the measurement overlay — a stroked segment `from`→`to`
   * (2px, `0xffd400`) plus a centered `Text` label shown at the segment's midpoint.
   * @param from The segment's start point (scene coords).
   * @param to The segment's end point (scene coords).
   * @param label The distance label text to center on the segment.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.drawMeasure({ x: 0, y: 0 }, { x: 10, y: 0 }, "2");
   * ```
   */
  drawMeasure(from: Point, to: Point, label: string): void {
    this.measureGraphics.clear();
    this.measureGraphics.moveTo(from.x, from.y).lineTo(to.x, to.y).stroke({ width: 2, color: 0xffd400 });
    this.measureText.text = label;
    this.measureText.position.set((from.x + to.x) / 2, (from.y + to.y) / 2);
    this.measureText.visible = true;
  }

  /** `DisplayBackend.clearMeasure`: clear the measurement segment and hide its label.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.clearMeasure();
   * ```
   */
  clearMeasure(): void {
    this.measureGraphics.clear();
    this.measureText.visible = false;
  }

  /** `DisplayBackend.drawPings`: redraw the ping-ring overlay — clears it, then strokes one circle
   * per ring (3px, `0xffd400`, at the ring's own `alpha`).
   * @param rings The current ping rings to draw (center, radius, alpha), in draw order.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.drawPings([{ x: 0, y: 0, radius: 20, alpha: 0.8 }]);
   * ```
   */
  drawPings(rings: PingRing[]): void {
    this.pingGraphics.clear();
    for (const r of rings) {
      this.pingGraphics.circle(r.x, r.y, r.radius).stroke({ width: 3, color: 0xffd400, alpha: r.alpha });
    }
  }

  /** `DisplayBackend.setLighting`: repaint the lighting overlay — clears `lightingGraphics`, then
   * for each cell draws up to three stacked fills over that cell's polygon (`c.corners`, already
   * resolved via the active grid — a square rect on a square grid, a hexagon on a hex grid; this
   * method paints whatever shape it is handed and performs no grid-kind math of its own): a black
   * darkening fill (`c.alpha`, skipped when `0`), a tinted fill (`c.tint` at `c.tintAlpha`, skipped
   * when `0`), and — when `c.desaturate` — a flat neutral-gray wash (`0x808080` at `0.18` alpha)
   * approximating desaturation for a darkvision-only cell. An empty `frame.cells` clears the
   * overlay entirely (no lighting effect); a cell with fewer than 3 corners is skipped (degenerate
   * geometry, nothing to fill).
   * @param frame The resolved per-cell lighting to paint.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.setLighting({ cell: 70, cells: [] });
   * ```
   */
  setLighting(frame: LightingFrame): void {
    this.lightingGraphics.clear();
    // empty cells = no lighting overlay (all-clear)
    for (const c of frame.cells) {
      if (c.corners.length < 3) continue; // degenerate geometry — nothing to fill
      const poly = c.corners.flatMap((p) => [p.x, p.y]);
      if (c.alpha > 0) this.lightingGraphics.poly(poly).fill({ color: 0x000000, alpha: c.alpha });
      if (c.tintAlpha > 0) this.lightingGraphics.poly(poly).fill({ color: c.tint, alpha: c.tintAlpha });
      // V1 desaturate approximation: a low-alpha neutral wash mutes color in darkvision-only cells.
      // TODO: replace with a masked ColorMatrixFilter over the scene layers for true desaturation.
      if (c.desaturate) this.lightingGraphics.poly(poly).fill({ color: 0x808080, alpha: 0.18 });
    }
  }

  /** `DisplayBackend.startTicker`: register the per-frame render ticker — forwards Pixi's own
   * `app.ticker` per-tick callback, translating its `deltaMS` into `cb`'s `dtMs` argument. Pixi
   * drives this ticker automatically (no manual pump needed), unlike `MockBackend`'s `startTicker`,
   * which only records `cb` for a test to invoke explicitly via `runTicker`. **Repeat calls
   * ACCUMULATE, never replace:** `app.ticker.add(...)` always inserts a fresh listener (no dedup
   * against an already-registered callback, even the identical `cb` reference — verified against
   * the vendored `Ticker._addListener` source) and this method never captures a handle to remove
   * one, so a second `startTicker` call on the same backend means BOTH callbacks fire every frame
   * for the backend's remaining lifetime. `MockBackend.startTicker` diverges here too: a second
   * call silently OVERWRITES `this.tick`, so only the latest ever fires via `runTicker`. No
   * current caller invokes this twice on one backend instance (`RenderEngine.start()` — the sole
   * caller — itself has no double-start guard, but every production caller,
   * `Stage`'s `$effect`, constructs a fresh `RenderEngine`/backend pair on each run and
   * tears the old one down first) — a contract caveat, not a live bug.
   * @param cb Called once per render frame with the elapsed milliseconds since the previous frame.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.startTicker((dtMs) => {});
   * ```
   */
  startTicker(cb: (dtMs: number) => void): void {
    this.app.ticker.add((ticker) => cb(ticker.deltaMS));
  }

  /** `DisplayBackend.addLayerFilter`: attach an opaque filter to a layer's Container. A no-op
   * (returns a no-op dispose) for an unknown `layerId` — unlike `MockBackend`'s implementation,
   * which records the registration unconditionally regardless of whether `layerId` names a real
   * layer. **Dispose is VALUE-based, not identity-of-registration-based:** the returned dispose
   * removes every entry `=== filter` from the layer's filter list, not just this specific
   * registration — registering the SAME filter object twice on one layer and calling EITHER
   * dispose strips BOTH entries. `MockBackend.addLayerFilter`'s dispose is stricter here: it
   * removes only its own `{layerId, filter}` entry (by object identity, via `indexOf`), so a
   * duplicate-registration test scenario would pass against the mock and fail against this
   * backend. No current caller registers the same filter twice on one layer (`registerLayerFilter`
   * is the sole forwarder) — a live gap, not a live bug.
   * @param layerId The target core-layer id (e.g. `"tokens"`).
   * @param filter A Pixi `Filter` (typed `unknown` at this seam — see `DisplayBackend`).
   * @returns A dispose function that removes every filter list entry `=== filter` — see the
   * value-based-removal note above.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * declare const filter: unknown; // e.g. a PixiJS `Filter`
   * const dispose = backend.addLayerFilter("tokens", filter);
   * dispose();
   * ```
   */
  addLayerFilter(layerId: string, filter: unknown): () => void {
    const c = this.layers.get(layerId);
    if (!c) return () => {};
    c.filters = [...(c.filters ?? []), filter as Filter];
    return () => {
      c.filters = (c.filters ?? []).filter((f) => f !== filter);
    };
  }

  /** `DisplayBackend.resize`: resize the Pixi renderer/viewport to CSS pixels; HiDPI scaling is
   * handled internally by the renderer's own `resolution`/`autoDensity` settings (set once at
   * `createPixiBackend` init time).
   * @param width The new viewport width, in CSS pixels.
   * @param height The new viewport height, in CSS pixels.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.resize(1280, 720);
   * ```
   */
  resize(width: number, height: number): void {
    this.app.renderer.resize(width, height);
  }

  /** `DisplayBackend.destroy`: release all GPU resources and detach the canvas. Bumps `loadSeq`
   * first so any in-flight background load resolving after destroy is a no-op; explicitly
   * destroys the cross-fade `RenderTexture`s (`fogBlendFromRT`/`fogBlendToRT`) before the cascade
   * below, since — unlike `Assets.load` results, which are shared/cached — they are owned solely
   * by this backend and would otherwise leak (NOT nulled afterward, so a second `destroy()` call
   * would re-invoke `.destroy(true)` on them too — moot per the next point); then calls
   * `app.destroy({removeView:true}, {children:true, texture:true})`, which recursively destroys
   * every remaining child and its textures and removes the `<canvas>` element from the DOM.
   * **SINGLE-USE — a second call THROWS, does not no-op:** Pixi's `Application.destroy` nulls
   * `this.stage`/`this.renderer` after destroying them (verified against the vendored
   * `Application.destroy` source, which also documents this outright: "After calling destroy, the
   * application instance should no longer be used… further operations will throw errors."), so a
   * second `destroy()` call on this backend hits `this.stage.destroy(...)` on `null` and throws
   * a `TypeError`. `MockBackend.destroy()` diverges here: it is trivially idempotent (`this.destroyed
   * = true` again, no error) and does NOT clear `this.tick`/`this.tokens`/other recorded state, so
   * a stale `runTicker()` call after a mock `destroy()` still fires — the opposite of the real
   * backend's crash-on-reuse contract. No current call site destroys twice
   * (`RenderEngine.destroy()`'s own doc already states "call once… not reusable afterward", but
   * that contract has no code-level guard at this layer) — a live gap, not a live bug.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.destroy();
   * ```
   */
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
 * Does not clear, so multiple shapes can share one Graphics (the overlay).
 * @param g The Graphics to paint onto (not cleared by this call).
 * @param tokenSpec The shape to paint: a polyline/polygon (flat scene-coord points) with optional
 * `closed`/`fill`/`stroke`.
 * @example
 * ```
 * // module-private helper; not exported from @shadowcat/render
 * import { Graphics } from "pixi.js";
 * const g = new Graphics();
 * paintShape(g, { points: [0, 0, 10, 0, 10, 10, 0, 10], closed: true, stroke: null, fill: null });
 * ```
 */
function paintShape(g: Graphics, tokenSpec: Omit<ShapeNodeSpec, "layer">): void {
  const p = tokenSpec.points;
  if (p.length < 4) return; // need at least two points
  g.moveTo(p[0], p[1]);
  for (let i = 2; i < p.length; i += 2) g.lineTo(p[i], p[i + 1]);
  if (tokenSpec.closed) g.closePath();
  if (tokenSpec.fill) g.fill({ color: tokenSpec.fill.color, alpha: tokenSpec.fill.alpha });
  if (tokenSpec.stroke) g.stroke({ width: tokenSpec.stroke.width, color: tokenSpec.stroke.color });
}

/** Paint the three-state fog (unexplored darkest / explored dimmed / visible clear) onto the
 * given sheet + hole Graphics: two opaque sheets over a large world region, cut by inverse
 * masks built from the input's `explored`/`visible` polygons. Shared by the live `mask` layer
 * (`setVisibility`) and the cross-fade capture (`captureFog`) so both draw IDENTICAL fog for
 * the same input — the cross-fade blends between two genuinely equivalent renders, not a
 * lookalike approximation. `mode:"all"` paints nothing (caller handles the no-fog case).
 * @param dark The near-opaque "unexplored" sheet to paint + mask.
 * @param dim The semi-transparent "explored memory" sheet to paint + mask.
 * @param exploredHoles Inverse-mask shape accumulator cut from `dark` (explored ∪ visible).
 * @param visibleHoles Inverse-mask shape accumulator cut from `dim` (visible only).
 * @param input The visibility sample to paint; `mode:"all"` is a no-op (caller handles no-fog).
 * @example
 * ```
 * // module-private helper; not exported from @shadowcat/render
 * import { Graphics } from "pixi.js";
 * const dark = new Graphics(), dim = new Graphics(), eh = new Graphics(), vh = new Graphics();
 * paintFogSheets(dark, dim, eh, vh, { mode: "all", visible: [], explored: [] });
 * ```
 */
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

/** Construct a PixiBackend over a canvas (async: v8 Application.init is async).
 * @param canvas The `<canvas>` element Pixi renders into.
 * @param opts Initial renderer options.
 * @returns A `PixiBackend` wrapping a fully-initialized Pixi `Application` (antialiased, WebGL
 * preferred, HiDPI-aware via `devicePixelRatio` resolution + `autoDensity`).
 * @example
 * ```ts
 * import { createPixiBackend } from "@shadowcat/render";
 *
 * declare const canvas: HTMLCanvasElement;
 * const backend = await createPixiBackend(canvas, { background: 0x1e1e2e });
 * ```
 */
export async function createPixiBackend(
  canvas: HTMLCanvasElement,
  opts: PixiBackendOptions,
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
