import type { ReadableDocuments, AssetResolver } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { VisibilityInput, LightingInput, LitCell, SceneTool, SceneToolHost, Point, ShapeNodeSpec, Polygon } from "./types";
import { Camera } from "./camera";
import { Compositor } from "./compositor";
import { Grid, type GridSpec } from "./grid";
import { LayerRegistry } from "./layers";
import { SceneReconciler } from "./reconciler";
import { TokenView } from "./token-view";
import { DrawingView } from "./drawing-view";
import { TemplateView } from "./template-view";
import { WallView } from "./wall-view";
import { PingView } from "./ping-view";

/** Rasterize a flat `[i,j,…]` explored-cell list into one square rect polygon per cell at
 * `size` world units. Cell `(i,j)` covers `[i*size,(i+1)*size) × [j*size,(j+1)*size)`. The fog
 * shader unions overlapping rects by overdraw, so per-cell rects (vs merged runs) are correct. */
function cellsToRects(cells: number[], size: number): Polygon[] {
  const rects: Polygon[] = [];
  for (let k = 0; k + 1 < cells.length; k += 2) {
    const x = cells[k] * size;
    const y = cells[k + 1] * size;
    rects.push({ points: [x, y, x + size, y, x + size, y + size, x, y + size] });
  }
  return rects;
}

/** Handle to a scene subscription (structurally matches @shadowcat/core's). */
export interface SceneSubscription {
  unsubscribe(): void;
}

/** Injected scene-subscribe function (no transport dependency in this package). `opts.asUser`
 * (GM-only see-as-player) views the channel as that user; the server gates + resolves it. */
export type SubscribeScene = (
  channel: string,
  onUpdate: (frame: { payload: unknown; computedAtSeq: number }) => void,
  opts?: { asUser?: string },
) => SceneSubscription;

export interface RenderEngineOpts {
  /** Document source to render. The host passes the optimistic view so predicted
   * (unconfirmed) creates/moves render immediately; the authoritative store is the
   * rollback base. */
  store: ReadableDocuments;
  assets: AssetResolver;
  backend: DisplayBackend;
  grid: GridSpec;
  /** Grid line color (0xRRGGBB) sampled from CSS tokens by the host; default slate. */
  gridColor?: number;
  /** Injected SceneDerived subscribe (from WorldSession via AppContext). */
  subscribeScene?: SubscribeScene;
  /** Called when a derived frame is applied (host observability hook); carries the applied
   * visibility so the host can surface the fog mode. */
  onDerivedApplied?: (input: VisibilityInput) => void;
}

/** Orchestrates the render model over a DisplayBackend: layers, camera, grid, and
 * the store-driven reconciler. Framework- and Pixi-free (the backend is injected). */
export class RenderEngine implements SceneToolHost {
  readonly camera = new Camera();
  readonly compositor: Compositor;
  private readonly layers = new LayerRegistry();
  /** Reassignable: the active scene's grid drives snapping + lines (M8d §15). */
  private grid: Grid;
  private readonly reconciler: SceneReconciler;
  private readonly tokens: TokenView;
  private readonly drawings: DrawingView;
  private readonly templates: TemplateView;
  private readonly walls: WallView;
  private readonly pings = new PingView();
  /** Whether ping rings were drawn last frame, so the ticker stops redrawing once idle. */
  private pingsActive = false;
  private readonly gridColor: number;
  private viewport = { width: 0, height: 0 };
  private unsubscribe: (() => void) | null = null;
  private sceneSub: SceneSubscription | null = null;
  /** Active canvas tool (null = camera owns all gestures). */
  private activeTool: SceneTool | null = null;
  /** Gesture ownership for the in-flight pointer drag: a tool claimed the down, or
   * the camera is panning. Both false between gestures. */
  private toolGesture = false;
  private panning = false;
  private lastPan: Point = { x: 0, y: 0 };
  /** The pointer that owns the in-flight gesture; events from other pointers
   * (multi-touch / pen+mouse) are ignored until it ends. Single-pointer by design. */
  private activePointerId: number | null = null;
  private pendingDerived: { input: VisibilityInput; seq: number } | null = null;
  /** Highest computed_at_seq applied to the mask; guards against regressing to an
   * older derived frame (latest-wins). */
  private lastAppliedSeq = -1;
  /** The last derived visibility, re-rendered when the GM fog preview toggles. */
  private lastInput: VisibilityInput = { mode: "all", visible: [], explored: [] };
  /** GM-only client preview: when true a no-fog (`mode:"all"`) frame renders as full fog, so the
   * GM can preview a vision-less player's view. Only ever ADDS fog to the GM's own view (D-V3,
   * no server path) — it can never reveal more than the frame already carries. */
  private fogPreview = false;
  /** GM see-as-player target (M9c-2): the user whose vision the `vision` subscription requests, or
   * null for the GM's own view. The server gates + resolves it (a non-GM is rejected). */
  private viewAsUser: string | null = null;

  constructor(private readonly opts: RenderEngineOpts) {
    this.grid = new Grid(opts.grid);
    this.gridColor = opts.gridColor ?? 0x3a3a4a;
    this.reconciler = new SceneReconciler(opts.store, opts.assets, opts.backend);
    this.tokens = new TokenView(opts.store, opts.assets, opts.backend);
    this.drawings = new DrawingView(opts.store, opts.backend);
    this.templates = new TemplateView(opts.store, opts.backend);
    this.walls = new WallView(opts.store, opts.backend);
    this.compositor = new Compositor(opts.backend);
  }

  start(): void {
    this.opts.backend.ensureLayers(this.layers.orderedIds());
    this.applyCamera();
    this.reconciler.reconcile();
    this.tokens.reconcile();
    this.drawings.reconcile();
    this.templates.reconcile();
    this.walls.reconcile();
    this.unsubscribe = this.opts.store.subscribe(() => {
      this.reconciler.reconcile();
      this.tokens.reconcile();
      this.drawings.reconcile();
      this.templates.reconcile();
      this.walls.reconcile();
      this.flushPendingDerived();
    });
    this.opts.backend.startTicker((dt) => {
      this.tokens.tick(dt);
      const rings = this.pings.tick(dt);
      // Redraw only while rings live (plus one final clear when they expire).
      if (rings.length > 0 || this.pingsActive) {
        this.opts.backend.drawPings(rings);
        this.pingsActive = rings.length > 0;
      }
    });
    this.subscribeVision();
  }

  /** (Re)establish the `vision` subscription for the current `viewAsUser` (per-recipient visibility
   * polygons, or `mode:"all"` for the GM's own view). */
  private subscribeVision(): void {
    if (!this.opts.subscribeScene) return;
    this.sceneSub?.unsubscribe();
    this.sceneSub = this.opts.subscribeScene(
      "vision",
      (f) => this.onSceneFrame(f),
      this.viewAsUser ? { asUser: this.viewAsUser } : undefined,
    );
  }

  /** GM see-as-player (M9c-2): re-subscribe the vision channel viewing as `userId` (null = the GM's
   * own view). Resets the mask watermark so the new view's first frame applies even at the same
   * world seq — a view switch is a fresh stream, not a regression of the same one. */
  setViewAsUser(userId: string | null): void {
    if (userId === this.viewAsUser) return;
    this.viewAsUser = userId;
    this.lastAppliedSeq = -1;
    this.pendingDerived = null;
    this.subscribeVision();
  }

  private onSceneFrame(frame: { payload: unknown; computedAtSeq: number }): void {
    // Per-channel frames are monotonic in computed_at_seq and latest wins. Drop any
    // frame already superseded by an applied or a pending one — never regress the
    // mask to an older derived state (defends the M9 consumer against reordering).
    if (frame.computedAtSeq <= this.lastAppliedSeq) return;
    if (this.pendingDerived && frame.computedAtSeq <= this.pendingDerived.seq) return;
    const input = this.toVisibility(frame.payload);
    if (this.opts.store.appliedSeq >= frame.computedAtSeq) {
      this.applyDerived(input, frame.computedAtSeq);
    } else {
      this.pendingDerived = { input, seq: frame.computedAtSeq }; // watermark: defer
    }
  }

  private flushPendingDerived(): void {
    const p = this.pendingDerived;
    if (p && this.opts.store.appliedSeq >= p.seq) {
      this.pendingDerived = null;
      this.applyDerived(p.input, p.seq);
    }
  }

  private applyDerived(input: VisibilityInput, seq: number): void {
    this.lastAppliedSeq = seq;
    this.lastInput = input;
    this.renderVisibility();
  }

  /** Apply the last derived visibility through the GM fog-preview override. */
  private renderVisibility(): void {
    const eff: VisibilityInput =
      this.fogPreview && this.lastInput.mode === "all"
        ? { mode: "masked", visible: [], explored: [] }
        : this.lastInput;
    this.compositor.setVisibility(eff);
    this.opts.onDerivedApplied?.(eff);
  }

  /** GM-only: toggle the client-side fog preview. `on` renders a no-fog frame as full fog so the
   * GM can preview the player view (see-as-player is M9c-2). Client-only (D-V3); only adds fog to
   * the GM's own view, so it cannot leak. */
  setFogPreview(on: boolean): void {
    this.fogPreview = on;
    this.renderVisibility();
  }

  /** Parse a `vision` payload into a VisibilityInput. Fail CLOSED: fog is the only client-side
   * secrecy gate (the document layer still delivers non-`gm_only` token/wall positions to
   * players), so ONLY an explicit `{mode:"all"}` clears it and ONLY an explicit `{mode:"masked"}`
   * reveals the supplied polygons. Any other payload — missing, garbled, or unknown-mode
   * (protocol skew) — yields full fog (`{mode:"masked", visible:[]}`), revealing nothing.
   * `{mode:"masked", polygons:[{scene,points:[x,y,…]},…]}` → fog outside those polygons; empty ⇒
   * full fog. Each polygon is filtered to the active scene so a polygon for a token in another
   * scene cannot punch a hole into this scene's fog (scene coordinates are scene-local and reused
   * across scenes). */
  private toVisibility(payload: unknown): VisibilityInput {
    const p = payload as
      | {
          mode?: string;
          polygons?: { scene?: string; points?: number[] }[];
          explored?: { scene?: string; cell?: number; cells?: number[] }[];
        }
      | null
      | undefined;
    if (p?.mode === "all") return { mode: "all", visible: [], explored: [] };
    // Garbled/missing/unknown mode → full fog. Only a well-formed `masked` payload reveals.
    if (p?.mode !== "masked") return { mode: "masked", visible: [], explored: [] };
    const activeScene = this.opts.store.query("scene")[0]?.id;
    const polygons = Array.isArray(p.polygons) ? p.polygons : [];
    const visible = polygons
      .filter(
        (g): g is { scene?: string; points: number[] } =>
          !!g && g.scene === activeScene && Array.isArray(g.points) && g.points.length >= 6,
      )
      .map((g) => ({ points: g.points }));
    // `explored` is the dimmed memory layer: scene-tagged cell sets rasterized to rect polygons,
    // filtered to the active scene (cross-scene guard, like `visible`). Missing/garbled → no
    // explored (fail-safe: more fog, never less).
    const exploredGroups = Array.isArray(p.explored) ? p.explored : [];
    const explored = exploredGroups
      .filter(
        (g): g is { scene?: string; cell: number; cells: number[] } =>
          !!g &&
          g.scene === activeScene &&
          typeof g.cell === "number" &&
          g.cell > 0 &&
          Array.isArray(g.cells),
      )
      .flatMap((g) => cellsToRects(g.cells, g.cell));
    return { mode: "masked", visible, explored };
  }

  // Parse-only; not yet called from onSceneFrame — TODO: wire into onSceneFrame + Lighting layer once the engine-integration task lands.
  /** Parse the `vision` payload's lighting dimension into a LightingInput for the ACTIVE scene, or
   * null. Lighting is COSMETIC — fog (toVisibility) is the secrecy gate — so any non-masked,
   * missing, or malformed input yields null (no overlay), never an over/under-reveal. Mirrors
   * toVisibility's active-scene filter so a lit set for a token in another scene cannot tint
   * this scene. */
  private toLighting(payload: unknown): LightingInput | null {
    const p = payload as {
      mode?: string;
      bands?: { name?: string; min?: number }[];
      renderHints?: string[];
      lit?: { scene?: string; cell?: number; cells?: number[] }[];
    } | null | undefined;
    if (p?.mode !== "masked" || !Array.isArray(p.lit)) return null;
    const activeScene = this.opts.store.query("scene")[0]?.id;
    const group = p.lit.find(
      (g): g is { scene?: string; cell: number; cells: number[] } =>
        !!g && g.scene === activeScene && typeof g.cell === "number" && g.cell > 0 && Array.isArray(g.cells),
    );
    if (!group) return null;
    const cells: LitCell[] = [];
    for (let k = 0; k + 4 < group.cells.length; k += 5) {
      cells.push({
        i: group.cells[k], j: group.cells[k + 1], band: group.cells[k + 2],
        tint: group.cells[k + 3], hint: group.cells[k + 4],
      });
    }
    const bands = Array.isArray(p.bands)
      ? p.bands
          .filter((b): b is { name: string; min: number } => !!b && typeof b.name === "string" && typeof b.min === "number")
          .map((b) => ({ name: b.name, min: b.min }))
      : [];
    const hints = Array.isArray(p.renderHints) ? p.renderHints.map(String) : [];
    return { cell: group.cell, bands, hints, cells };
  }

  /** Test seam: exposes toLighting for unit tests (cosmetic parse has no secrecy implication). */
  toLightingForTest(p: unknown): LightingInput | null { return this.toLighting(p); }

  /** Module-facing shader-filter seam (0.x). Forwards to the backend; no engine
   * consumer in M8 — the first consumers are token fx / Phase-3 VFX. */
  registerLayerFilter(layerId: string, filter: unknown): () => void {
    return this.opts.backend.addLayerFilter(layerId, filter);
  }

  // --- SceneToolHost: the canvas interaction seam (M8d §7). The host (Stage)
  // feeds DOM pointer events as screen points; the engine converts to scene coords
  // and routes to the active tool first, falling back to camera pan. ---

  setActiveTool(tool: SceneTool | null): void {
    this.activeTool = tool;
    // A tool swap cancels any in-flight gesture ownership and releases the dragging
    // latch, so an interrupted drag cannot strand a token in snap-no-tween mode.
    this.toolGesture = false;
    this.panning = false;
    this.activePointerId = null;
    this.tokens.setDragging(null);
    // Discard any in-progress tool ephemeral (preview shape + measure segment): the old
    // tool's pointerup won't fire after a mid-gesture swap, so it can't self-clean.
    this.clearOverlay();
    this.clearMeasure();
  }

  snap(p: Point): Point {
    return this.grid.snap(p);
  }

  setDraggingToken(id: string | null): void {
    this.tokens.setDragging(id);
  }

  previewOverlay(shapes: Omit<ShapeNodeSpec, "layer">[]): void {
    this.opts.backend.drawOverlay(shapes);
  }

  clearOverlay(): void {
    this.opts.backend.clearOverlay();
  }

  gridDistance(a: Point, b: Point): number {
    return this.grid.distance(a, b);
  }

  drawMeasure(from: Point, to: Point, label: string): void {
    this.opts.backend.drawMeasure(from, to, label);
  }

  clearMeasure(): void {
    this.opts.backend.clearMeasure();
  }

  addPing(x: number, y: number): void {
    this.pings.add(x, y);
  }

  /** Swap the active grid (from the active scene's `system.grid`) and redraw lines. */
  setGrid(spec: GridSpec): void {
    this.grid = new Grid(spec);
    this.redrawGrid();
  }

  dispatchPointerDown(screen: Point, ev: PointerEvent): void {
    if (this.toolGesture || this.panning) return; // a gesture already owns the canvas
    this.activePointerId = ev.pointerId;
    if (this.activeTool && this.activeTool.onPointerDown(this.camera.screenToScene(screen), ev)) {
      this.toolGesture = true; // the tool claimed this gesture
      return;
    }
    this.panning = true; // no tool / not handled → camera pans
    this.lastPan = screen;
  }

  dispatchPointerMove(screen: Point, ev: PointerEvent): void {
    if (ev.pointerId !== this.activePointerId) return; // a non-owning pointer
    if (this.toolGesture && this.activeTool) {
      this.activeTool.onPointerMove(this.camera.screenToScene(screen), ev);
      return;
    }
    if (this.panning) {
      this.camera.panBy(screen.x - this.lastPan.x, screen.y - this.lastPan.y);
      this.lastPan = screen;
      this.applyCamera();
    }
  }

  dispatchPointerUp(screen: Point, ev: PointerEvent): void {
    if (ev.pointerId !== this.activePointerId) return; // a non-owning pointer
    if (this.toolGesture && this.activeTool) {
      this.activeTool.onPointerUp(this.camera.screenToScene(screen), ev);
    }
    this.toolGesture = false;
    this.panning = false;
    this.activePointerId = null;
  }

  setViewport(width: number, height: number): void {
    this.viewport = { width, height };
    this.opts.backend.resize(width, height);
    this.redrawGrid();
  }

  /** Force a re-reconcile. Needed for out-of-band `AssetChanged` notices, which
   * mutate the `AssetResolver` (cache-bust / placeholder) without a document
   * mutation, so the `store.subscribe` reconcile never fires for them. */
  reconcileNow(): void {
    this.reconciler.reconcile();
    this.tokens.reconcile(); // re-resolve token images too (AssetChanged path)
  }

  /** Push the camera transform to the backend and redraw the grid for the new view. */
  applyCamera(): void {
    this.opts.backend.setCameraTransform(this.camera.transform());
    this.redrawGrid();
  }

  private redrawGrid(): void {
    const tl = this.camera.screenToScene({ x: 0, y: 0 });
    const br = this.camera.screenToScene({ x: this.viewport.width, y: this.viewport.height });
    const rect = { x: tl.x, y: tl.y, w: br.x - tl.x, h: br.y - tl.y };
    this.opts.backend.drawGrid(this.grid.lines(rect), this.gridColor);
  }

  destroy(): void {
    this.unsubscribe?.();
    this.unsubscribe = null;
    this.sceneSub?.unsubscribe();
    this.sceneSub = null;
    this.opts.backend.destroy();
  }
}
