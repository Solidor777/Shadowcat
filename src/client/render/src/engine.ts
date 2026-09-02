import { EMPTY_FOOTPRINTS } from "@shadowcat/core";
import type { ReadableDocuments, AssetResolver, FootprintLookup } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { VisibilityInput, LightingInput, LitCell, SceneTool, SceneToolHost, Point, ShapeNodeSpec, Polygon, MoveVisionSample, MoveLightSample } from "./types";
import type { TokenTweenConfig } from "./easing";
import { computeFogBlendFactor, chooseVisionSample } from "./fog-blend";
import { blendLightCells, lightSampleCells } from "./light-sweep";
import { unionLightingInputs } from "./lighting";
import { Camera } from "./camera";
import { Compositor } from "./compositor";
import { Grid, type GridSpec } from "./grid";
import { LayerRegistry } from "./layers";
import { Lighting, type LightingFrame, type LitDrawCell } from "./lighting";
import { SceneReconciler } from "./reconciler";
import { TokenView } from "./token-view";
import { DrawingView } from "./drawing-view";
import { TemplateView } from "./template-view";
import { WallView } from "./wall-view";
import { RegionView } from "./region-view";
import { LightView } from "./light-view";
import { PingView } from "./ping-view";

/** Rasterize a flat `[i,j,…]` explored-cell list into one shape polygon per cell, via the active
 * `grid`'s own corner geometry — square on a square grid, hexagon on a hex grid. The fog shader
 * unions overlapping polygons by overdraw, so per-cell shapes (vs merged runs) are correct.
 * Delegates to `Grid.cellVertices` rather than assuming a square (`x=i*size, y=j*size`) shape —
 * indexing a hex-axial cell that way paints the wrong scene position (skewed square positions
 * over correctly-drawn hexes).
 * @param cells Flat `[i0,j0,i1,j1,…]` cell-index pairs; a trailing unpaired value is dropped.
 * @param grid The active grid, whose kind decides whether each cell is a square or a hexagon.
 * @returns One polygon per `(i,j)` pair (4 points on square, 6 on hex), in input order.
 * @example
 * ```
 * // module-private helper; not exported from @shadowcat/render
 * cellsToRects([0, 0, 1, 0], new Grid({ kind: "square", size: 5 }));
 * // 2 rects: [0,0]-[5,5] and [5,0]-[10,5]
 * ```
 */
function cellsToRects(cells: number[], grid: Grid): Polygon[] {
  const rects: Polygon[] = [];
  for (let k = 0; k + 1 < cells.length; k += 2) {
    const corners = grid.cellVertices(cells[k], cells[k + 1]);
    rects.push({ points: corners.flatMap((p) => [p.x, p.y]) });
  }
  return rects;
}

/** Handle to a scene subscription (structurally matches @shadowcat/core's). */
export interface SceneSubscription {
  /** Cancel this subscription; no further frames are delivered. */
  unsubscribe(): void;
}

/** Injected scene-subscribe function (no transport dependency in this package). `opts.asUser`
 * (GM-only see-as-player) views the channel as that user; the server gates + resolves it. */
export type SubscribeScene = (
  channel: string,
  onUpdate: (frame: {
    /** The channel-specific decoded frame data. */
    payload: unknown;
    /** The server document-seq this frame is valid as of; monotonic per channel. */
    computedAtSeq: number;
  }) => void,
  opts?: {
    /** GM-only see-as-player: view the channel as this user id. */
    asUser?: string;
  },
) => SceneSubscription;

/** Constructor options for {@link RenderEngine}. */
export interface RenderEngineOpts {
  /** Document source to render. The host passes the optimistic view so predicted
   * (unconfirmed) creates/moves render immediately; the authoritative store is the
   * rollback base. */
  store: ReadableDocuments;
  /** Resolves asset ids to serve URLs for token visuals — see `ResolvedAnimatedSource`'s
   * interface doc for the never-resolved-in-the-backend invariant. */
  assets: AssetResolver;
  /** The injected display backend (PixiJS in production, a mock in tests) — the engine is
   * Pixi-free itself. */
  backend: DisplayBackend;
  /** The initial grid kind/size/diagonal-rule; swap the active grid via {@link
   * RenderEngine.setGrid}. */
  grid: GridSpec;
  /** Grid line color (0xRRGGBB) sampled from CSS tokens by the host; default slate. */
  gridColor?: number;
  /** Injected SceneDerived subscribe (from WorldSession via AppContext). */
  subscribeScene?: SubscribeScene;
  /** Called when a derived frame is applied (host observability hook); carries the applied
   * visibility so the host can surface the fog mode. */
  onDerivedApplied?: (input: VisibilityInput) => void;
  /** Called on every painted lighting frame (host observability hook) with the frame the
   * backend was handed and whether a carried-light sweep is in flight — so a host can surface
   * "the lighting overlay changed mid-walk" without reading WebGL pixels. */
  onLightingApplied?: (frame: LightingFrame, sweeping: boolean) => void;
  /** Which scene to render/scene-filter by. From the host (Stage → `ctx.viewedSceneId`).
   * Absent ⇒ the first scene, preserving single-scene behavior. */
  viewedSceneId?: () => string | null;
  /** The server's resolved token footprints (Stage → `ctx.footprints`). The engine computes no
   * footprint geometry: `TokenView` reads the extent from here. Absent ⇒ `EMPTY_FOOTPRINTS`, under
   * which every token draws at its document's own authored `w`/`h`. */
  footprints?: () => FootprintLookup;
}

/** Orchestrates the render model over a DisplayBackend: layers, camera, grid, and
 * the store-driven reconciler. Framework- and Pixi-free (the backend is injected). */
export class RenderEngine implements SceneToolHost {
  /** Screen↔scene transform driving pan/zoom; pushed to the backend by {@link applyCamera}. */
  readonly camera = new Camera();
  /** Owns the mask/fog compositing target the backend renders; driven by
   * `renderVisibility`/`applyVisionSweep`. */
  readonly compositor: Compositor;
  /** Resolves the `CORE_LAYERS` z-order the backend's containers are created in (`start`). */
  private readonly layers = new LayerRegistry();
  /** Cosmetic band/tint overlay driven by `onSceneFrame`'s `toLighting` parse; independent of
   * the fog secrecy gate (`compositor`). */
  private readonly lighting: Lighting;
  /** Reassignable: the active scene's grid drives snapping + lines. */
  private grid: Grid;
  /** Scene-level snap-to-grid toggle; default enabled. Disabled makes `snap`
   * identity — grid RENDERING is unaffected, only the snap call chain every tool inherits
   * via `ctx.scene.snap`. */
  private snapEnabled = true;
  /** Diffs the store's `scene`/background doc into backend nodes; one of the six views
   * {@link start}/{@link reapplyViewedScene} re-run per store commit or scene switch. */
  private readonly reconciler: SceneReconciler;
  /** Doc→sprite reconciler + tween driver for `token` docs; also backs the drag/animation
   * `SceneToolHost` methods (`setDraggingToken`, `animateAlongPath`, `animateSamples`). */
  private readonly tokens: TokenView;
  /** Doc→shape reconciler for `drawing` docs. */
  private readonly drawings: DrawingView;
  /** Doc→shape reconciler for `template` docs. */
  private readonly templates: TemplateView;
  /** Doc→shape reconciler for `wall` docs. */
  private readonly walls: WallView;
  /** Doc→shape reconciler for `region` docs. */
  private readonly regions: RegionView;
  /** Doc→shape reconciler for `light` docs (GM authoring markers). */
  private readonly lights: LightView;
  /** Transient ping-ring state, ticked each frame; drives `DisplayBackend.drawPings`. */
  private readonly pings = new PingView();
  /** Whether ping rings were drawn last frame, so the ticker stops redrawing once idle. */
  private pingsActive = false;
  /** Resolved grid line color (0xRRGGBB) — `opts.gridColor`, or the default slate. */
  private readonly gridColor: number;
  /** Current CSS-pixel viewport size, set by {@link setViewport}; feeds the visible-scene-rect
   * computation in {@link redrawGrid}. */
  private viewport = { width: 0, height: 0 };
  /** The store-subscription teardown installed by {@link start}; `null` before `start` runs or
   * after {@link destroy}. */
  private unsubscribe: (() => void) | null = null;
  /** The active `vision`-channel subscription handle, re-established by `subscribeVision` on
   * every `viewAsUser` change; `null` when unsubscribed. */
  private sceneSub: SceneSubscription | null = null;
  /** Active canvas tool (null = camera owns all gestures). */
  private activeTool: SceneTool | null = null;
  /** Gesture ownership for the in-flight pointer drag: a tool claimed the down, or
   * the camera is panning. Both false between gestures. */
  private toolGesture = false;
  /** Whether the camera currently owns the in-flight pointer gesture (mutually exclusive with
   * `toolGesture`). */
  private panning = false;
  /** The previous pointer position for the in-flight camera pan, in screen coordinates. */
  private lastPan: Point = { x: 0, y: 0 };
  /** The pointer that owns the in-flight gesture; events from other pointers
   * (multi-touch / pen+mouse) are ignored until it ends. Single-pointer by design. */
  private activePointerId: number | null = null;
  /** Deferred visibility frame held behind the appliedSeq watermark. Caches the RAW payload (not a
   * pre-filtered VisibilityInput): `toVisibility` is re-run at flush time against the THEN-current
   * viewed scene, so a scene switch between defer and flush cannot paint a stale scene's fog holes
   * onto the newly-viewed one (fog secrecy gate — fail closed). */
  private pendingDerived: {
    /** The deferred frame's raw, unparsed payload. */
    payload: unknown;
    /** The deferred frame's `computedAtSeq`. */
    seq: number;
  } | null = null;
  /** Highest computed_at_seq applied to the mask; guards against regressing to an
   * older derived frame (latest-wins). */
  private lastAppliedSeq = -1;
  /** The last derived visibility, re-rendered when the GM fog preview toggles. */
  private lastInput: VisibilityInput = { mode: "all", visible: [], explored: [], perceived: [] };
  /** Ids of tokens the latest applied `vision` frame reports as perceived ONLY through a
   * creature sense (tremorsense & kin), taken from the scene-filtered `VisibilityInput.perceived`.
   * Read by `TokenView.toSpec` through the constructor-injected lookup. Written only by
   * {@link applyDerived} and {@link reapplyViewedScene} — a vision sweep
   * (`applyVisionSweep`/`tickVisionSweep`) replaces the FOG input only and never touches this
   * set, so a perceived token stays raised above the fog for the whole sweep. */
  private perceived: ReadonlySet<string> = new Set();
  /** GM-only client preview: when true a no-fog (`mode:"all"`) frame renders as full fog, so the
   * GM can preview a vision-less player's view. Only ever ADDS fog to the GM's own view, client-only
   * with no server path — it can never reveal more than the frame already carries. */
  private fogPreview = false;
  /** GM see-as-player target: the user whose vision the `vision` subscription requests, or
   * null for the GM's own view. The server gates + resolves it (a non-GM is rejected). */
  private viewAsUser: string | null = null;
  /** Active mover vision-sweep overrides, keyed by token id: while a mover's own
   * MoveStream plays, the `moverVision` sample with the greatest `tMs <= elapsed` feeds the fog,
   * bypassing `lastInput`. Keyed (mirrors `TokenAnimator.samplesAnim`) so a second concurrent sweep
   * for a DIFFERENT token cannot clobber an in-flight one; when non-empty, ALL active sweeps'
   * chosen samples are unioned into the rendered visible set. Empty when no sweep is in flight
   * (observers never populate this — `animateSamples` only starts a sweep when `moverVision` is
   * non-empty, and only the mover's own MoveStream carries a non-null `moverVision`, because the
   * server's per-recipient egress clip omits it for observers). */
  private readonly visionSweeps = new Map<
    string,
    {
      /** The sweep's ordered vision samples. */
      samples: MoveVisionSample[];
      /** Milliseconds elapsed since the sweep started. */
      elapsed: number;
      /** The sweep's total duration, in ms (mirrors the MoveStream's `durationMs`). */
      durationMs: number;
    }
  >();
  /** Active carried-light sweeps, keyed by token id — the lighting twin of `visionSweeps`.
   * While non-empty, `Lighting` paints the held committed frame unioned with every sweep's
   * chosen sample (`applyLightSweep`), and a committed lighting frame arriving meanwhile is
   * parked in `lastLightingInput` rather than applied: the post-commit rebroadcast already
   * carries the light at its FINAL position, and applying it mid-walk would show the corridor's
   * far end lit before the torch gets there. Any recipient may populate this — the mover, a
   * GM, or an observer whose vision the glow reached (`MoveStream.mover_light`). */
  private readonly lightSweeps = new Map<
    string,
    {
      /** The sweep's ordered light samples. */
      samples: MoveLightSample[];
      /** Milliseconds elapsed since the sweep started. */
      elapsed: number;
      /** The sweep's total duration, in ms: the MoveStream's `durationMs`, extended to the last
       * admitted light sample's `tMs` — an observer's clipped duration can end before a still-
       * admitted glow does. */
      durationMs: number;
    }
  >();
  /** The last parsed lighting input for the viewed scene (`toLighting`), applied or parked —
   * see `lightSweeps`. `null` = no overlay (GM `mode:"all"`, or no frame yet); a light sweep
   * has no darkness model to lift then and paints nothing. */
  private lastLightingInput: LightingInput | null = null;
  /** The committed lighting in force when the viewer's own vision sweep began (`animateSamples`
   * with `moverVision`), unioned with every newer committed frame for as long as a vision
   * sweep plays (`applyCommittedLighting`): the post-move frame lights the cells seen from the
   * STOP, this one the cells seen from the START, and the sweeping line of sight runs between
   * them. Cleared when the last vision sweep ends. */
  private lightingBeforeSweep: LightingInput | null = null;
  /** Resolved viewed scene, falling back to the first scene so single-scene tests/hosts are
   * unaffected. The single definition every view + reconciler + fog filter reads.
   * @returns The viewed scene's document id, or `null` when no scene document exists yet.
   * @example
   * ```
   * // private field; not part of the public API
   * const sceneId = this.viewedScene();
   * ```
   */
  private readonly viewedScene = (): string | null =>
    this.opts.viewedSceneId?.() ?? this.opts.store.query("scene")[0]?.id ?? null;
  /** The last vision payload received, re-projected onto a new viewed scene by
   * `reapplyViewedScene` (a scene switch has no new server frame — `activeScene`/roam are
   * client-local). Undefined until the first frame. */
  private lastRawPayload: unknown = undefined;

  /** Builds the camera, grid, doc-type views (tokens/drawings/templates/walls/regions),
   * compositor, and lighting model from `opts` — every one of these is constructed here,
   * eagerly, so calling a `SceneToolHost` method (`snap`, `setGrid`, `dispatchPointerDown`,
   * …) before {@link start} never fails from a missing field. Does NOT touch `opts.backend`
   * beyond passing it through to the constructed sub-objects — no layer containers are
   * created and no reconcile pass runs yet; call `start()` once to make output visible.
   * @param opts Backend, document source, and grid to render; see {@link RenderEngineOpts}.
   * @example
   * ```ts
   * import { RenderEngine, type RenderEngineOpts } from "@shadowcat/render";
   *
   * declare const opts: RenderEngineOpts;
   * const engine = new RenderEngine(opts);
   * ```
   */
  constructor(private readonly opts: RenderEngineOpts) {
    this.grid = new Grid(opts.grid);
    this.gridColor = opts.gridColor ?? 0x3a3a4a;
    this.reconciler = new SceneReconciler(opts.store, opts.assets, opts.backend, this.viewedScene);
    this.tokens = new TokenView(opts.store, opts.assets, opts.backend, this.viewedScene, () => opts.footprints?.() ?? EMPTY_FOOTPRINTS, () => this.perceived);
    this.tokens.setWorldUnitsPerCell(this.grid.worldUnitsPerCell());
    this.drawings = new DrawingView(opts.store, opts.backend, this.viewedScene);
    this.templates = new TemplateView(opts.store, opts.backend, this.viewedScene);
    this.walls = new WallView(opts.store, opts.backend, this.viewedScene);
    this.regions = new RegionView(opts.store, opts.backend, this.viewedScene);
    this.lights = new LightView(opts.store, opts.backend, this.viewedScene);
    this.compositor = new Compositor(opts.backend);
    this.lighting = new Lighting(opts.backend, (frame) => opts.onLightingApplied?.(frame, this.lightSweeps.size > 0));
  }

  /** Wires the engine to its backend and store. In order: creates the backend's layer
   * containers in the {@link CORE_LAYERS} z-order (currently the only layers `start()` ever
   * requests — the private `layers` registry has no public passthrough on `RenderEngine` for
   * a module to splice in a fractional-order layer of its own yet); pushes the initial camera
   * transform and draws the grid; runs one reconcile pass over every doc-type view so the
   * current scene renders synchronously, without waiting for a store event; subscribes to
   * further store changes (each one re-reconciles every view, then flushes any deferred
   * derived-vision frame); starts the backend's per-frame ticker (token tween, lighting fade,
   * vision-sweep advance, ping rings); and opens the `vision` subscription. Call exactly once
   * per engine instance. Every field a `SceneToolHost` method reads is already constructed
   * before `start()` runs (see the constructor) — no `RenderEngine` method of its own depends on
   * `start()` having run — but no scene document has been reconciled into a render node yet, so
   * a method that would otherwise touch one (e.g. `animateAlongPath` on a token not yet
   * reconciled) has nothing to act on until the first reconcile pass above completes.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.start();
   * ```
   */
  start(): void {
    this.opts.backend.ensureLayers(this.layers.orderedIds());
    this.applyCamera();
    this.reconciler.reconcile();
    this.tokens.reconcile();
    this.drawings.reconcile();
    this.templates.reconcile();
    this.walls.reconcile();
    this.regions.reconcile();
    this.lights.reconcile();
    this.unsubscribe = this.opts.store.subscribe(() => {
      this.reconciler.reconcile();
      this.tokens.reconcile();
      this.drawings.reconcile();
      this.templates.reconcile();
      this.walls.reconcile();
      this.regions.reconcile();
      this.lights.reconcile();
      this.flushPendingDerived();
    });
    this.opts.backend.startTicker((dt) => {
      this.tokens.tick(dt);
      this.lighting.tick(dt);
      this.tickVisionSweep(dt);
      this.tickLightSweep(dt);
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
   * polygons, or `mode:"all"` for the GM's own view).
   * @example
   * ```
   * // private method; not part of the public API
   * this.subscribeVision();
   * ```
   */
  private subscribeVision(): void {
    if (!this.opts.subscribeScene) return;
    this.sceneSub?.unsubscribe();
    this.sceneSub = this.opts.subscribeScene(
      "vision",
      (f) => this.onSceneFrame(f),
      this.viewAsUser ? { asUser: this.viewAsUser } : undefined,
    );
  }

  /** GM see-as-player: re-subscribe the vision channel viewing as `userId` (null = the GM's
   * own view). Resets the mask watermark so the new view's first frame applies even at the same
   * world seq — a view switch is a fresh stream, not a regression of the same one.
   * @param userId The user whose vision to view as, or `null` to revert to the GM's own view.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.setViewAsUser("00000000-0000-0000-0000-000000000001"); // see-as
   * engine.setViewAsUser(null); // revert to the GM's own view
   * ```
   */
  setViewAsUser(userId: string | null): void {
    if (userId === this.viewAsUser) return;
    this.viewAsUser = userId;
    this.lastAppliedSeq = -1;
    this.pendingDerived = null;
    this.subscribeVision();
  }

  /** Handles one `vision` channel frame: drops it if superseded (monotonic `computedAtSeq`,
   * latest wins), applies lighting eagerly (cosmetic, unwatermarked), and either applies the
   * fog immediately or defers it behind `store.appliedSeq` — see the inline comments for the
   * per-branch reasoning; this is the fog-secrecy watermark gate `flushPendingDerived` later
   * resolves.
   * @param frame The subscription frame.
   * @param frame.payload The opaque `vision` payload — parsed by `toVisibility`/`toLighting`.
   * @param frame.computedAtSeq The server document-seq this frame is valid as of; monotonic
   * per channel.
   * @example
   * ```
   * // private method; not part of the public API — invoked only as the `subscribeScene`
   * // callback for the "vision" channel
   * declare const payload: unknown;
   * this.onSceneFrame({ payload, computedAtSeq: 7 });
   * ```
   */
  private onSceneFrame(frame: {
    /** The opaque `vision` payload — parsed by `toVisibility`/`toLighting`. */
    payload: unknown;
    /** The server document-seq this frame is valid as of; monotonic per channel. */
    computedAtSeq: number;
  }): void {
    // Per-channel frames are monotonic in computed_at_seq and latest wins. Drop any
    // frame already superseded by an applied or a pending one — never regress the
    // mask to an older derived state (defends against out-of-order frame delivery).
    if (frame.computedAtSeq <= this.lastAppliedSeq) return;
    if (this.pendingDerived && frame.computedAtSeq <= this.pendingDerived.seq) return;
    this.lastRawPayload = frame.payload;
    // Lighting is cosmetic — applied eagerly here (monotonic order already honored by the
    // guards above), NOT held behind the appliedSeq watermark that fog uses for document
    // consistency. Exactly one retarget per non-dropped frame (parked while a light sweep
    // plays — see `lightSweeps`).
    this.retargetLighting(this.toLighting(frame.payload));
    if (this.opts.store.appliedSeq >= frame.computedAtSeq) {
      // Immediate: filter against the CURRENT viewed scene now (toVisibility reads viewedScene()).
      this.applyDerived(this.toVisibility(frame.payload), frame.computedAtSeq);
    } else {
      // Watermark defer: cache the RAW payload, never a pre-filtered VisibilityInput. A scene
      // switch (reapplyViewedScene) can change the viewed scene before this flushes, so re-run
      // toVisibility at flush time against the then-current scene (fog secrecy gate).
      this.pendingDerived = { payload: frame.payload, seq: frame.computedAtSeq };
    }
  }

  /** Applies a `pendingDerived` frame once `store.appliedSeq` catches up to its seq, re-filtering
   * the RAW payload against the CURRENT viewed scene at flush time (never the scene viewed when it
   * was deferred — see `pendingDerived`'s field doc). Also guards against re-applying a pending
   * entry a later immediate-apply frame already superseded (see `lastAppliedSeq`'s field doc) — a
   * pending entry is always cleared once the watermark condition is met, whether or not it ends up
   * applied. Called from the `store.subscribe` listener installed in `start()`, once per store
   * commit. No-op when no frame is pending or the watermark hasn't caught up yet.
   * @example
   * ```
   * // private method; not part of the public API — invoked only from the store.subscribe
   * // listener installed in start()
   * this.flushPendingDerived();
   * ```
   */
  private flushPendingDerived(): void {
    const p = this.pendingDerived;
    if (p && this.opts.store.appliedSeq >= p.seq) {
      // Always clear the pending slot here — flushed or discarded, it must never linger past this
      // watermark check (a lingering entry is exactly the monotonicity hole this guard closes).
      this.pendingDerived = null;
      // Monotonicity guard: a newer frame may have taken the IMMEDIATE-apply branch in onSceneFrame
      // (its own computedAtSeq not ahead of appliedSeq at arrival) while this entry sat deferred,
      // advancing lastAppliedSeq PAST p.seq without touching pendingDerived. Applying this now would
      // regress the mask to a stale-but-superseded frame. Only a pending entry still newer than the
      // highest already-applied seq is live; anything else is discarded, never applied.
      if (p.seq > this.lastAppliedSeq) {
        // Re-filter the raw payload against the CURRENT viewed scene at flush time (not the scene
        // viewed when the frame was deferred): toVisibility reads this.viewedScene() internally, so a
        // deferred scene-A frame flushing after a switch to scene B yields scene B's fog, not A's.
        this.applyDerived(this.toVisibility(p.payload), p.seq);
      }
    }
  }

  /** Records a resolved `vision`-frame application and renders it. The sole write site for
   * `lastAppliedSeq`/`lastInput` reached from frame handling — both `onSceneFrame`'s immediate
   * branch and `flushPendingDerived`'s deferred branch funnel through here, so those two paths
   * can never disagree about which applied frame is current. Two other write sites exist
   * outside frame handling, neither a frame application: {@link setViewAsUser} resets
   * `lastAppliedSeq` to `-1` (a view-switch watermark reset) without touching `lastInput`, and
   * {@link reapplyViewedScene} writes `lastInput` directly (a scene-switch re-filter of the
   * already-cached raw payload) without touching `lastAppliedSeq`. Also re-projects the
   * creature-sense `perceived` id set into the token views: a derived frame carries no document
   * change, so without the reconcile here a perceived add/remove would wait for an unrelated
   * store commit to reach `PixiBackend.setToken`.
   * @param input The resolved `VisibilityInput` (already scene-filtered by `toVisibility`).
   * @param seq The frame's `computedAtSeq`, recorded as the new watermark.
   * @example
   * ```
   * // private method; not part of the public API
   * this.applyDerived({ mode: "masked", visible: [], explored: [], perceived: [] }, 7);
   * ```
   */
  private applyDerived(input: VisibilityInput, seq: number): void {
    this.lastAppliedSeq = seq;
    this.lastInput = input;
    this.perceived = new Set(input.perceived);
    this.tokens.reconcile();
    this.renderVisibility();
  }

  /** Apply the last derived visibility through the GM fog-preview override. Skips the actual
   * `compositor.setVisibility` call while a vision-sweep is in flight (`visionSweeps` non-empty):
   * a move commit's own server-recomputed vision broadcast (or any other scene update) would
   * otherwise clobber the sweep's progressive polygon until the next tick reasserts it — a visible
   * flicker. `lastInput` is still updated (and the host still notified) so the sweep's completion
   * (`tickVisionSweep`) reverts to the CURRENT derived vision, not a stale one.
   * @example
   * ```
   * // private method; not part of the public API
   * this.renderVisibility();
   * ```
   */
  private renderVisibility(): void {
    const eff: VisibilityInput =
      this.fogPreview && this.lastInput.mode === "all"
        ? { mode: "masked", visible: [], explored: [], perceived: [] }
        : this.lastInput;
    if (this.visionSweeps.size === 0) {
      this.compositor.setVisibility(eff);
      // The darkness overlay covers exactly the line of sight the fog clears: an in-sight cell
      // no lit-mask entry lights renders at the darkest band, never clear.
      this.lighting.setDarkness(eff.visible);
    }
    this.opts.onDerivedApplied?.(eff);
  }

  /** GM-only: toggle the client-side fog preview. `on` renders a no-fog frame as full fog so the
   * GM can preview the player view. Client-only, with no server path; only adds fog to
   * the GM's own view, so it cannot leak.
   * @param on `true` to force a `mode:"all"` frame to render as full fog; `false` to render it as
   * received (no fog).
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.setFogPreview(true);
   * ```
   */
  setFogPreview(on: boolean): void {
    this.fogPreview = on;
    this.renderVisibility();
  }

  /** Re-project the render onto the CURRENT viewed scene after a client-local scene switch
   * (`activeScene` flip or GM roam — neither carries a new server frame). Re-runs background + all
   * doc views (their `parent_id` filter changed) and re-filters the last vision payload to the new
   * scene. Fog secrecy across the switch: a cached frame is re-filtered to the new scene, so only
   * that scene's polygons punch holes (a polygon tagged for another scene is dropped) — never the
   * prior scene's. With no cached frame yet (`lastRawPayload === undefined`) visibility is left
   * untouched; the initial `lastInput` default (`{mode:"all"}` = NO fog, the pre-vision bootstrap
   * reveal) stands until the first frame arrives. A frame deferred in `pendingDerived` caches the
   * raw payload and is likewise re-filtered against the then-current scene at flush time.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.reapplyViewedScene();
   * ```
   */
  reapplyViewedScene(): void {
    // Re-filter the cached raw payload FIRST: `TokenView.toSpec` reads the `perceived` set
    // through the injected lookup, so it must reflect the newly-viewed scene before the token
    // reconcile below runs.
    // A light sweep belongs to a token of the scene being left; it ends here so the newly
    // viewed scene's committed lighting applies at once rather than after the walk.
    if (this.lightSweeps.size > 0) {
      this.lightSweeps.clear();
      this.lighting.setSweep(null);
    }
    this.lightingBeforeSweep = null;
    if (this.lastRawPayload !== undefined) {
      this.retargetLighting(this.toLighting(this.lastRawPayload));
      this.lastInput = this.toVisibility(this.lastRawPayload);
      this.perceived = new Set(this.lastInput.perceived);
    }
    this.reconciler.reconcile();
    this.tokens.reconcile();
    this.drawings.reconcile();
    this.templates.reconcile();
    this.walls.reconcile();
    this.regions.reconcile();
    this.lights.reconcile();
    if (this.lastRawPayload !== undefined) {
      this.renderVisibility();
    }
  }

  /** Re-project every token after a new `"footprints"` frame. A footprints frame carries no
   * document change, so the store commit that would otherwise drive a reconcile has already been
   * and gone (the server recomputes derived channels on a debounce AFTER the event that changed
   * them). Without this the canvas would keep drawing the previous extents until the next
   * unrelated store commit. Tokens only — no other view reads a footprint.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.reapplyFootprints();
   * ```
   */
  reapplyFootprints(): void {
    this.tokens.reconcile();
  }

  /** Parse a `vision` payload into a VisibilityInput. Fail CLOSED: fog is the only client-side
   * secrecy gate (the document layer still delivers non-`gm_only` token/wall positions to
   * players), so ONLY an explicit `{mode:"all"}` clears it and ONLY an explicit `{mode:"masked"}`
   * reveals the supplied polygons. Any other payload — missing, garbled, or unknown-mode
   * (protocol skew) — yields full fog (`{mode:"masked", visible:[]}`), revealing nothing.
   * `{mode:"masked", polygons:[{scene,points:[x,y,…]},…]}` → fog outside those polygons; empty ⇒
   * full fog. Each polygon is filtered to the active scene so a polygon for a token in another
   * scene cannot punch a hole into this scene's fog (scene coordinates are scene-local and reused
   * across scenes). The `perceived` key (`[{scene, tokens:[id,…]}]`, creature-sense-only token
   * ids) gets the same active-scene filter and the same fail-closed default: missing or garbled
   * ⇒ an empty list, never a widened render.
   * @param payload The raw `vision` channel payload — untyped: this layer trusts nothing about
   * its shape and validates every field structurally below.
   * @returns A `VisibilityInput` scene-filtered against {@link viewedScene}; full fog
   * (`{mode:"masked", visible:[], explored:[], perceived:[]}`) on any missing, garbled, or
   * unknown-mode input.
   * @example
   * ```
   * // private method; not part of the public API
   * const input = this.toVisibility({ mode: "all" }); // {mode:"all", visible:[], explored:[], perceived:[]}
   * ```
   */
  private toVisibility(payload: unknown): VisibilityInput {
    const p = payload as
      | {
          /** `"all"` or `"masked"` — see the method doc for the fail-closed default. */
          mode?: string;
          /** Per-token visible-polygon groups (only meaningful when `mode:"masked"`). */
          polygons?: {
            /** The scene id this polygon group belongs to — filtered against the active scene. */
            scene?: string;
            /** Flat `[x0,y0,x1,y1,…]` polygon points. */
            points?: number[];
          }[];
          /** Per-token explored-cell groups (the persistent memory layer). */
          explored?: {
            /** The scene id this cell group belongs to — filtered against the active scene. */
            scene?: string;
            /** World-unit length of one cell's edge — see `cellsToRects`. */
            cell?: number;
            /** Flat `[i0,j0,i1,j1,…]` cell-index pairs — see `cellsToRects`. */
            cells?: number[];
          }[];
          /** Per-scene creature-sense (tremorsense & kin) token-id groups — see the method doc. */
          perceived?: {
            /** The scene id this id group belongs to — filtered against the active scene. */
            scene?: string;
            /** Token ids perceived only through a creature sense in that scene. */
            tokens?: unknown[];
          }[];
        }
      | null
      | undefined;
    if (p?.mode === "all") return { mode: "all", visible: [], explored: [], perceived: [] };
    // Garbled/missing/unknown mode → full fog. Only a well-formed `masked` payload reveals.
    if (p?.mode !== "masked") return { mode: "masked", visible: [], explored: [], perceived: [] };
    const activeScene = this.viewedScene();
    const polygons = Array.isArray(p.polygons) ? p.polygons : [];
    const visible = polygons
      .filter(
        (g): g is {
          /** The scene id this polygon group belongs to. */
          scene?: string;
          /** Flat `[x0,y0,x1,y1,…]` polygon points (validated `>= 6`, i.e. ≥3 vertices). */
          points: number[];
        } => !!g && g.scene === activeScene && Array.isArray(g.points) && g.points.length >= 6,
      )
      .map((g) => ({ points: g.points }));
    // `explored` is the dimmed memory layer: scene-tagged cell sets rasterized to rect polygons,
    // filtered to the active scene (cross-scene guard, like `visible`). Missing/garbled → no
    // explored (fail-safe: more fog, never less).
    const exploredGroups = Array.isArray(p.explored) ? p.explored : [];
    const explored = exploredGroups
      .filter(
        (g): g is {
          /** The scene id this cell group belongs to. */
          scene?: string;
          /** World-unit length of one cell's edge. */
          cell: number;
          /** Flat `[i0,j0,i1,j1,…]` cell-index pairs. */
          cells: number[];
        } =>
          !!g &&
          g.scene === activeScene &&
          typeof g.cell === "number" &&
          g.cell > 0 &&
          Array.isArray(g.cells),
      )
      .flatMap((g) => cellsToRects(g.cells, this.grid));
    // `perceived` ids get the same active-scene filter as `visible`/`explored`; missing/garbled
    // groups → no perceived ids (fail-safe: fewer reveals, never more). Non-string ids are
    // dropped individually rather than failing the whole group — either way the result only
    // ever under-collects.
    const perceivedGroups = Array.isArray(p.perceived) ? p.perceived : [];
    const perceived = perceivedGroups
      .filter(
        (g): g is {
          /** The scene id this id group belongs to. */
          scene?: string;
          /** Token ids perceived only through a creature sense in that scene. */
          tokens: unknown[];
        } => !!g && g.scene === activeScene && Array.isArray(g.tokens),
      )
      .flatMap((g) => g.tokens.filter((t): t is string => typeof t === "string"));
    return { mode: "masked", visible, explored, perceived };
  }

  /** Parse the `vision` payload's lighting dimension into a LightingInput for the ACTIVE scene, or
   * null. Lighting is COSMETIC — fog (toVisibility) is the secrecy gate — so any non-masked,
   * missing, or malformed input yields null (no overlay), never an over/under-reveal. Mirrors
   * toVisibility's active-scene filter so a lit set for a token in another scene cannot tint
   * this scene.
   * @param payload The raw `vision` channel payload — the same untyped value `toVisibility` reads.
   * @returns The active scene's `LightingInput`, or `null` when the payload is non-`masked`,
   * missing a `lit` array, or has no group tagged for the active scene.
   * @example
   * ```
   * // private method; not part of the public API — see toLightingForTest for the exposed seam
   * const lighting = this.toLighting({ mode: "all" }); // null (not "masked")
   * ```
   */
  private toLighting(payload: unknown): LightingInput | null {
    const p = payload as {
      /** `"masked"` required — anything else yields no overlay (see the method doc). */
      mode?: string;
      /** Gradation bands, brightest-first. */
      bands?: {
        /** Band name (e.g. `"bright"`, `"dim"`, `"dark"`). */
        name?: string;
        /** Band floor — the minimum [0,1] illumination value admitting this band; mirrors
         * `scene::lighting::Band.min_illumination`, wire-keyed `min` by `compute_derived`. */
        min?: number;
      }[];
      /** `renderHints` lookup table; indexed by a cell's packed `hint` field. */
      renderHints?: string[];
      /** Per-token lit-cell groups. */
      lit?: {
        /** The scene id this cell group belongs to. */
        scene?: string;
        /** World-unit length of one cell's edge. */
        cell?: number;
        /** Flat `[i,j,band,tint,hint,…]` 5-int cell tuples — see `LitCell`. */
        cells?: number[];
      }[];
    } | null | undefined;
    if (p?.mode !== "masked" || !Array.isArray(p.lit)) return null;
    const activeScene = this.viewedScene();
    const group = p.lit.find(
      (g): g is {
        /** The scene id this cell group belongs to. */
        scene?: string;
        /** World-unit length of one cell's edge. */
        cell: number;
        /** Flat `[i,j,band,tint,hint,…]` 5-int cell tuples. */
        cells: number[];
      } => !!g && g.scene === activeScene && typeof g.cell === "number" && g.cell > 0 && Array.isArray(g.cells),
    );
    if (!group) return null;
    const cells: LitCell[] = [];
    for (let k = 0; k + 4 < group.cells.length; k += 5) {
      const i = group.cells[k], j = group.cells[k + 1];
      cells.push({
        i, j, band: group.cells[k + 2],
        tint: group.cells[k + 3], hint: group.cells[k + 4],
        // Resolved via the active grid, not `x=i*cell, y=j*cell`: on a hex scene the wire
        // indices are axial, and a square rect there paints the wrong scene position.
        corners: this.grid.cellVertices(i, j),
      });
    }
    const bands = Array.isArray(p.bands)
      ? p.bands
          .filter(
            (b): b is {
              /** Band name. */
              name: string;
              /** Band floor. */
              min: number;
            } => !!b && typeof b.name === "string" && typeof b.min === "number",
          )
          .map((b) => ({ name: b.name, min: b.min }))
      : [];
    const hints = Array.isArray(p.renderHints) ? p.renderHints.map(String) : [];
    return { cell: group.cell, bands, hints, cells };
  }

  /** Records the viewed scene's parsed lighting and applies it through
   * `applyCommittedLighting`.
   * @param input The parsed lighting for the viewed scene, or `null` to clear the overlay.
   * @example
   * ```
   * // private method; not part of the public API
   * this.retargetLighting(null);
   * ```
   */
  private retargetLighting(input: LightingInput | null): void {
    this.lastLightingInput = input;
    this.applyCommittedLighting();
  }

  /** Apply the committed lighting the viewer should see now: nothing while a carried-light
   * sweep is in flight (`lastLightingInput` stays parked until `tickLightSweep` ends the last
   * sweep — see `lightSweeps` for why a mid-walk frame must not paint); the union of the
   * lighting held when the viewer's own vision sweep began and the newest frame while a vision
   * sweep plays (`lightingBeforeSweep`); the newest frame alone otherwise. THE one path that
   * retargets `Lighting` from a committed frame.
   * @example
   * ```
   * // private method; not part of the public API
   * this.applyCommittedLighting();
   * ```
   */
  private applyCommittedLighting(): void {
    if (this.lightSweeps.size > 0) return;
    const li = this.lastLightingInput;
    const before = this.visionSweeps.size > 0 ? this.lightingBeforeSweep : null;
    this.lighting.setTarget(before && li ? unionLightingInputs(before, li) : li);
  }

  /** Test seam: exposes toLighting for unit tests (cosmetic parse has no secrecy implication).
   * @param p A raw `vision` channel payload, exactly as `toLighting` receives it.
   * @returns The active scene's `LightingInput`, or `null` — see `toLighting`.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.toLightingForTest({ mode: "all" }); // null
   * ```
   */
  toLightingForTest(p: unknown): LightingInput | null { return this.toLighting(p); }

  /** The badge chips the token view last projected for `id` (delegates to
   * `TokenView.badgesForTest`), or `null` when the token is untracked. Read-only observability
   * seam for the host stage's debug surface and tests.
   * @param id The token document id to read badges for.
   * @returns The current badge chip texts, or `null` when the token is untracked.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.badgesForTest("token-1");
   * ```
   */
  badgesForTest(id: string): string[] | null { return this.tokens.badgesForTest(id); }

  /** Module-facing shader-filter seam. Forwards to the backend; no engine consumer
   * currently calls it.
   * @param layerId The target core-layer id (e.g. `"tokens"`); the backend defines what an
   * unknown id does (`DisplayBackend.addLayerFilter`).
   * @param filter An opaque, backend-specific filter object (e.g. a PixiJS `Filter`).
   * @returns A dispose function that removes exactly this filter.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * declare const filter: unknown;
   * const dispose = engine.registerLayerFilter("tokens", filter);
   * dispose();
   * ```
   */
  registerLayerFilter(layerId: string, filter: unknown): () => void {
    return this.opts.backend.addLayerFilter(layerId, filter);
  }

  // --- SceneToolHost: the canvas interaction seam. The host (Stage)
  // feeds DOM pointer events as screen points; the engine converts to scene coords
  // and routes to the active tool first, falling back to camera pan. ---

  /** `SceneToolHost.setActiveTool`: set (or clear) the active canvas tool. A swap cancels any
   * in-flight gesture ownership and releases the dragging latch (so an interrupted drag cannot
   * strand a token in snap-no-tween mode), and unconditionally clears the ephemeral preview
   * overlay + measurement overlay: after a mid-gesture swap the OLD tool's own `onPointerUp`
   * (where it would otherwise clear its own preview) never fires, so this call is the backstop
   * that discards it regardless. Distinct from `SceneTool.onDeactivate` — that hook, when the
   * tool declares one, is invoked by the host's `ToolController.toggle`, not by this method.
   * @param tool The tool to activate, or `null` so the camera owns all gestures (pan/zoom only).
   * @example
   * ```ts
   * import type { RenderEngine, SceneTool } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * declare const tool: SceneTool;
   * engine.setActiveTool(tool);
   * engine.setActiveTool(null); // camera-only
   * ```
   */
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

  /** `SceneToolHost.snap`: snap a scene point to the active grid's nearest CELL CENTER — for
   * square grids, the containing cell's center; for hex, the nearest hex's center via
   * axial-round-then-convert (`Grid.snap`; `Grid.axialToPixel` returns a hex center, the same
   * method `Grid.hexLines` calls to get the origin it draws the six corners around — so this is a
   * center, never a vertex, on either grid kind) — or the identity function when {@link
   * setSnapEnabled} has disabled snapping for the current scene.
   * @param p A scene-coordinate point.
   * @returns `p` snapped to the active grid, or `p` unchanged when snapping is disabled.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.snap({ x: 12, y: 34 });
   * ```
   */
  snap(p: Point): Point {
    return this.snapEnabled ? this.grid.snap(p) : p;
  }

  /** `SceneToolHost.setSnapEnabled`: toggle the scene-level snap-to-grid axis.
   * Disabled makes {@link snap} the identity function; grid RENDERING is unaffected (a snap-off
   * scene may still display its reference grid) — only the snap call chain every tool inherits
   * via `ctx.scene.snap` changes.
   * @param enabled `false` disables snapping (free-form placement/movement); `true` re-enables it.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.setSnapEnabled(false);
   * ```
   */
  setSnapEnabled(enabled: boolean): void {
    this.snapEnabled = enabled;
  }

  /** `SceneToolHost.setDraggingToken`: mark a token as locally dragging (forwards to
   * `TokenView.setDragging`) so its sprite snaps to the authoritative transform with no tween
   * lag while a remote move on a different token still tweens normally.
   * @param id The token document id to mark as dragging, or `null` to clear it.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.setDraggingToken("00000000-0000-0000-0000-000000000001");
   * engine.setDraggingToken(null);
   * ```
   */
  setDraggingToken(id: string | null): void {
    this.tokens.setDragging(id);
  }

  /** `SceneToolHost.previewOverlay`: draw an ephemeral, non-document tool-in-progress preview
   * into the backend's overlay layer. Forwards verbatim to `DisplayBackend.drawOverlay`.
   * @param shapes The preview shapes (no `layer` field — always the `overlays` layer).
   * @example
   * ```ts
   * import type { RenderEngine, ShapeNodeSpec } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * declare const shapes: Omit<ShapeNodeSpec, "layer">[];
   * engine.previewOverlay(shapes);
   * ```
   */
  previewOverlay(shapes: Omit<ShapeNodeSpec, "layer">[]): void {
    this.opts.backend.drawOverlay(shapes);
  }

  /** `SceneToolHost.clearOverlay`: clear the ephemeral preview overlay. Forwards verbatim to
   * `DisplayBackend.clearOverlay`.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.clearOverlay();
   * ```
   */
  clearOverlay(): void {
    this.opts.backend.clearOverlay();
  }

  /** `SceneToolHost.gridDistance`: whole-cell distance between two scene points via the active
   * grid ({@link Grid.distance} — hex axial distance, or square with the configured
   * `diagonalRule`). See that method's doc for the per-rule server-parity scope: the match is
   * exact only for an unweighted route, since a terrain region's `terrain_multiplier > 1.0`
   * raises the server's real A* cost above what this reports.
   * @param a The first scene point.
   * @param b The second scene point.
   * @returns The whole-cell distance between `a` and `b` on the active grid.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.gridDistance({ x: 0, y: 0 }, { x: 10, y: 10 });
   * ```
   */
  gridDistance(a: Point, b: Point): number {
    return this.grid.distance(a, b);
  }

  /** `SceneToolHost.drawMeasure`: draw the client-local measurement overlay (a segment `from`→`to`
   * plus a centered distance label). Forwards verbatim to `DisplayBackend.drawMeasure`.
   * @param from The segment's start point (scene coords).
   * @param to The segment's end point (scene coords).
   * @param label The distance label text to center on the segment.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.drawMeasure({ x: 0, y: 0 }, { x: 10, y: 0 }, "2");
   * ```
   */
  drawMeasure(from: Point, to: Point, label: string): void {
    this.opts.backend.drawMeasure(from, to, label);
  }

  /** `SceneToolHost.clearMeasure`: clear the measurement overlay. Forwards verbatim to
   * `DisplayBackend.clearMeasure`.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.clearMeasure();
   * ```
   */
  clearMeasure(): void {
    this.opts.backend.clearMeasure();
  }

  /** `SceneToolHost.addPing`: spawn a transient ping ring at scene `(x,y)` (a received or
   * locally-originated ping). Forwards to `PingView.add`; ring lifetime/fade is driven by the
   * ticker installed in `start()`.
   * @param x The ping's scene x-coordinate.
   * @param y The ping's scene y-coordinate.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.addPing(10, 20);
   * ```
   */
  addPing(x: number, y: number): void {
    this.pings.add(x, y);
  }

  /** Swap the active grid (from the active scene's `engine.grid`) and redraw lines.
   * Coupling: notifies the token animator so tween durations are recalculated against the new
   * grid's per-step world distance (`Grid.worldUnitsPerCell`) — not `spec.size` directly, which
   * is the indexing scale and diverges from the per-step distance by `sqrt(3)` on hex.
   * @param spec The new grid's kind/size/diagonal-rule.
   * @example
   * ```ts
   * import type { RenderEngine, GridSpec } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * const gridSpec: GridSpec = { kind: "square", size: 70 };
   * engine.setGrid(gridSpec);
   * ```
   */
  setGrid(spec: GridSpec): void {
    this.grid = new Grid(spec);
    this.tokens.setWorldUnitsPerCell(this.grid.worldUnitsPerCell());
    this.redrawGrid();
  }

  /** Push animation config (speed + easing) to the token animator. Separate from setGrid
   * so world-settings animation changes can be applied without rebuilding the grid.
   * @param cfg Animation config.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.setAnimation({ speedCellsPerSec: 6, easing: "linear" });
   * ```
   */
  setAnimation(cfg: TokenTweenConfig): void {
    this.tokens.setAnimationConfig(cfg);
  }

  /** Has no production caller today (`WorldSession` drives route playback exclusively through
   * `SceneInteraction.animateSamples`, per `commitRoute`'s own "Animation is
   * broadcast-driven via onMoveStream ... no local animation from the moveRequest resolve value"
   * comment); this `SceneToolHost` seam is exercised only by tests. The mechanism below is the
   * contract it honors if called.
   *
   * Drive a smooth local walk of token `id` along scene-coord waypoints. Forwards to TokenView
   * which anchors the walk at the live current position and ignores authoritative setTarget calls
   * for each waypoint as the commit burst arrives.
   * @param id The token document id to animate.
   * @param path Scene-coord `[x,y]` waypoints, in walk order.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.animateAlongPath("00000000-0000-0000-0000-000000000001", [[0, 0], [70, 0]]);
   * ```
   */
  animateAlongPath(id: string, path: [number, number][]): void {
    this.tokens.animateAlongPath(id, path);
  }

  /** Drive server-broadcast sample-based playback (SceneToolHost seam). Forwards the position
   * samples to TokenView; `serverNow` used once at call time for catch-up alignment. When
   * `moverVision` is present (non-empty — only the mover's own MoveStream carries it, because the
   * server's per-recipient egress clip omits it for observers), also starts a fog vision-sweep
   * keyed to the SAME clock: the override is applied immediately and advanced each tick by
   * `tickVisionSweep`.
   * @param id The token document id whose playback this drives.
   * @param samples Position samples, each a `{tMs, pos}` pair; interpolated between adjacent
   * samples by `tMs`.
   * @param durationMs Total playback duration in ms (also the vision sweep's duration, when one
   * starts).
   * @param startServerMs The server clock time (ms) at which this playback begins.
   * @param serverNow Optional server-clock accessor, used once at call time as
   * `Math.max(0, serverNow() - startServerMs)` to compute catch-up elapsed time for both the
   * token tween and any accompanying vision sweep; when absent, elapsed starts at `0` (no
   * catch-up assumed).
   * @param moverVision Mover-only per-sample vision polygons (`null`/absent for observers, because
   * the server's per-recipient egress clip omits it for them); presence starts a fog vision-sweep
   * alongside the position tween.
   * @param moverLight Per-sample carried-light polygons — present for any recipient the server
   * admitted at least one sample to; presence starts a lighting sweep (`lightSweeps`) whose
   * duration extends to the last admitted sample's `tMs`. A GLOW-ONLY frame (`samples` empty,
   * `moverLight` present: the light reached this viewer, the token never did) starts the light
   * sweep alone — no token tween, since there is no position to play.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.animateSamples(
   *   "00000000-0000-0000-0000-000000000001",
   *   [{ tMs: 0, pos: [0, 0] }, { tMs: 500, pos: [70, 0] }],
   *   500,
   *   Date.now(),
   * );
   * ```
   */
  animateSamples(
    id: string,
    samples: {
      /** Server-clock offset, in ms, this sample applies from. */
      tMs: number;
      /** Scene-coord `[x,y]` position at this sample. */
      pos: [number, number];
    }[],
    durationMs: number,
    startServerMs: number,
    serverNow?: () => number,
    moverVision?: MoveVisionSample[] | null,
    moverLight?: MoveLightSample[] | null,
  ): void {
    if (samples.length > 0) this.tokens.animateSamples(id, samples, durationMs, startServerMs, serverNow);
    const initialElapsed = serverNow ? Math.max(0, serverNow() - startServerMs) : 0;
    if (moverVision && moverVision.length > 0) {
      // The first sweep captures the lighting in force — the cells lit from the START — so the
      // post-move frame (lit from the STOP) unions with it for the sweep's duration.
      if (this.visionSweeps.size === 0) this.lightingBeforeSweep = this.lastLightingInput;
      this.visionSweeps.set(id, { samples: moverVision, elapsed: initialElapsed, durationMs });
      this.applyVisionSweep();
    }
    if (moverLight && moverLight.length > 0) {
      const lastTMs = moverLight[moverLight.length - 1].tMs;
      this.lightSweeps.set(id, { samples: moverLight, elapsed: initialElapsed, durationMs: Math.max(durationMs, lastTMs) });
      this.applyLightSweep();
    }
  }

  /** Advance every in-flight carried-light sweep by `dtMs` (backend ticker, beside
   * `tickVisionSweep`). Each sweep completes on its own `durationMs`; once ALL have completed
   * the overlay clears and the parked committed lighting (`lastLightingInput`, which by then
   * carries the light at its final position) applies through `applyCommittedLighting`.
   * @param dtMs Milliseconds elapsed since the previous tick.
   * @example
   * ```
   * // private method; not part of the public API — invoked from the startTicker callback
   * this.tickLightSweep(16);
   * ```
   */
  private tickLightSweep(dtMs: number): void {
    if (this.lightSweeps.size === 0) return;
    for (const [id, sweep] of this.lightSweeps) {
      sweep.elapsed += dtMs;
      if (sweep.elapsed >= sweep.durationMs) this.lightSweeps.delete(id);
    }
    if (this.lightSweeps.size === 0) {
      this.lighting.setSweep(null);
      this.applyCommittedLighting();
      return;
    }
    this.applyLightSweep();
  }

  /** The viewer's current line-of-sight polygons the light sweep intersects with: the union of
   * every in-flight vision sweep's chosen sample while one plays (the fog being shown), else the
   * last derived frame's `visible` set.
   * @returns The polygons currently clear of fog.
   * @example
   * ```
   * // private method; not part of the public API
   * this.currentLosPolygons();
   * ```
   */
  private currentLosPolygons(): Polygon[] {
    if (this.visionSweeps.size === 0) return this.lastInput.visible;
    const out: Polygon[] = [];
    for (const sweep of this.visionSweeps.values()) out.push(...this.samplePolygons(this.chosenSample(sweep)));
    return out;
  }

  /** Drive the lighting overlay from the active light sweep(s): with exactly one sweep and a
   * next sample to fade toward, cross-fades the two samples' cell sets (`blendLightCells` at the
   * `computeFogBlendFactor` factor — the same blend clock the fog sweep uses); otherwise unions
   * every sweep's chosen sample (a hard snap). Cells come from `lightSampleCells` against the
   * viewer's current line of sight and the active grid; with no darkness model
   * (`lastLightingInput === null`) the sweep paints nothing.
   * @example
   * ```
   * // private method; not part of the public API
   * this.applyLightSweep();
   * ```
   */
  private applyLightSweep(): void {
    if (this.lightSweeps.size === 0) return;
    const li = this.lastLightingInput;
    if (!li) {
      this.lighting.setSweep(null);
      return;
    }
    const los = this.currentLosPolygons();
    const bandCount = li.bands.length;
    const cellsOf = (s: MoveLightSample): LitDrawCell[] => lightSampleCells(s, this.grid, los, bandCount);
    if (this.lightSweeps.size === 1) {
      const [sweep] = this.lightSweeps.values();
      const cur = chooseVisionSample(sweep.samples, sweep.elapsed);
      let next: MoveLightSample | null = null;
      for (const s of sweep.samples) {
        if (s.tMs > cur.tMs && (next === null || s.tMs < next.tMs)) next = s;
      }
      if (next) {
        this.lighting.setSweep(blendLightCells(cellsOf(cur), cellsOf(next), computeFogBlendFactor(sweep.elapsed, cur.tMs, next.tMs)));
        return;
      }
    }
    const cells: LitDrawCell[] = [];
    for (const sweep of this.lightSweeps.values()) cells.push(...cellsOf(chooseVisionSample(sweep.samples, sweep.elapsed)));
    this.lighting.setSweep(cells);
  }

  /** Advance every in-flight vision-sweep by `dtMs` (called from the backend ticker alongside
   * token/lighting tick). A sweep completes (elapsed reaches its durationMs) independently of the
   * others — each token's own MoveStream duration governs its own sweep. Once ALL sweeps have
   * completed, the override clears and the last derived (subscription) vision re-applies via the
   * existing `lastInput`/`renderVisibility` path — never leaves the fog pinned to a stale sample.
   * @param dtMs Milliseconds elapsed since the previous tick.
   * @example
   * ```
   * // private method; not part of the public API — invoked from the startTicker callback
   * this.tickVisionSweep(16);
   * ```
   */
  private tickVisionSweep(dtMs: number): void {
    if (this.visionSweeps.size === 0) return;
    let anyCompleted = false;
    for (const [id, sweep] of this.visionSweeps) {
      sweep.elapsed += dtMs;
      if (sweep.elapsed >= sweep.durationMs) {
        this.visionSweeps.delete(id);
        anyCompleted = true;
      }
    }
    if (this.visionSweeps.size === 0) {
      if (anyCompleted) {
        this.lightingBeforeSweep = null;
        this.applyCommittedLighting(); // the newest committed lighting alone, from the STOP
        this.renderVisibility(); // revert to the last derived vision (+ fog-preview override)
      }
      return;
    }
    this.applyVisionSweep();
  }

  /** The sample with the greatest `tMs <= sweep.elapsed` (falls back to the first sample when
   * elapsed precedes it). Delegates to `chooseVisionSample`, which is also
   * asserted against the server's `chosen_vision_sample` on the shared parity fixture.
   * @param sweep The in-flight sweep state.
   * @param sweep.samples The sweep's ordered vision samples.
   * @param sweep.elapsed Milliseconds elapsed since the sweep started.
   * @returns The sample that should be showing at `sweep.elapsed`.
   * @example
   * ```
   * // private method; not part of the public API
   * this.chosenSample({ samples: [], elapsed: 0 });
   * ```
   */
  private chosenSample(sweep: {
    /** The sweep's ordered vision samples. */
    samples: MoveVisionSample[];
    /** Milliseconds elapsed since the sweep started. */
    elapsed: number;
  }): MoveVisionSample {
    return chooseVisionSample(sweep.samples, sweep.elapsed);
  }

  /** Converts one `MoveVisionSample`'s raw polygon point-groups into `Polygon`s, dropping any
   * group with fewer than 3 points (degenerate, cannot enclose an area).
   * @param s The sample whose `polygons` to convert.
   * @returns One `Polygon` per non-degenerate point-group, flattened to `[x0,y0,x1,y1,…]`.
   * @example
   * ```
   * // private method; not part of the public API
   * this.samplePolygons({ tMs: 0, polygons: [] });
   * ```
   */
  private samplePolygons(s: MoveVisionSample): Polygon[] {
    return s.polygons.filter((pts) => pts.length >= 3).map((pts) => ({ points: pts.flat() }));
  }

  /** For a SINGLE in-flight sweep, the `(from, to, factor)` cross-fade triple between the
   * chosen sample and the next one (smallest `tMs` strictly greater than the chosen sample's) —
   * or `null` when there is no next sample (elapsed at/after the last sample: nothing to fade
   * toward, caller snaps instead). `explored` and `perceived` are carried from the last derived
   * frame in both endpoints — only `visible` cross-fades, and the creature-sense id set is not
   * part of the sweep at all.
   * @param sweep The in-flight sweep state.
   * @param sweep.samples The sweep's ordered vision samples.
   * @param sweep.elapsed Milliseconds elapsed since the sweep started.
   * @returns The cross-fade `{from, to, factor}` triple, or `null` when there's no next sample.
   * @example
   * ```
   * // private method; not part of the public API
   * this.sweepBlendInputs({ samples: [], elapsed: 0 });
   * ```
   */
  private sweepBlendInputs(
    sweep: {
      /** The sweep's ordered vision samples. */
      samples: MoveVisionSample[];
      /** Milliseconds elapsed since the sweep started. */
      elapsed: number;
    },
  ): {
    /** The chosen (current) sample's fog input. */
    from: VisibilityInput;
    /** The next sample's fog input to fade toward. */
    to: VisibilityInput;
    /** The cross-fade blend factor, `[0,1]` — see `computeFogBlendFactor`. */
    factor: number;
  } | null {
    const cur = this.chosenSample(sweep);
    let next: MoveVisionSample | null = null;
    for (const s of sweep.samples) {
      if (s.tMs > cur.tMs && (next === null || s.tMs < next.tMs)) next = s;
    }
    if (!next) return null;
    return {
      from: { mode: "masked", visible: this.samplePolygons(cur), explored: this.lastInput.explored, perceived: this.lastInput.perceived },
      to: { mode: "masked", visible: this.samplePolygons(next), explored: this.lastInput.explored, perceived: this.lastInput.perceived },
      factor: computeFogBlendFactor(sweep.elapsed, cur.tMs, next.tMs),
    };
  }

  /** Drive the fog from the active vision-sweep(s) (bypasses `lastInput`/`renderVisibility` — the
   * sweep is a transient animation override, not a new derived frame). With exactly one active
   * sweep AND a next sample to fade toward, cross-fades between the current and next sample via
   * `Compositor.setVisibilityBlend`. Otherwise (no next sample yet to fade toward, or multiple
   * concurrent sweeps whose sample timelines aren't comparable to a single blend factor) falls
   * back to the union of every active sweep's chosen sample (a hard snap, no cross-fade).
   * @example
   * ```
   * // private method; not part of the public API
   * this.applyVisionSweep();
   * ```
   */
  private applyVisionSweep(): void {
    if (this.visionSweeps.size === 0) return;
    if (this.visionSweeps.size === 1) {
      const [sweep] = this.visionSweeps.values();
      const blend = this.sweepBlendInputs(sweep);
      if (blend) {
        this.compositor.setVisibilityBlend(blend.from, blend.to, blend.factor);
        // Both endpoints darken: a cell the cross-fade is clearing must never flash lit before
        // the darkness reaches it.
        this.lighting.setDarkness([...blend.from.visible, ...blend.to.visible]);
        return;
      }
    }
    const visible: Polygon[] = [];
    for (const sweep of this.visionSweeps.values()) {
      visible.push(...this.samplePolygons(this.chosenSample(sweep)));
    }
    this.compositor.setVisibility({ mode: "masked", visible, explored: this.lastInput.explored, perceived: this.lastInput.perceived });
    this.lighting.setDarkness(visible);
  }

  /** Routes a DOM pointerdown (screen coords) to the active tool first, falling back to camera
   * pan. Ignored while a gesture already owns the canvas (`toolGesture`/`panning`). Claims
   * `ev.pointerId` as the gesture owner — a second pointer's events are ignored by
   * {@link dispatchPointerMove}/{@link dispatchPointerUp} until this one ends (single-pointer by
   * design: no multi-touch/pen+mouse concurrency).
   * @param screen The pointer position in screen (DOM) coordinates.
   * @param ev The originating `PointerEvent` (forwarded to the tool; its `pointerId` becomes the
   * gesture owner).
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * declare const ev: PointerEvent;
   * engine.dispatchPointerDown({ x: 100, y: 200 }, ev);
   * ```
   */
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

  /** Routes a DOM pointermove to whichever gesture owns the in-flight drag (the active tool, or
   * the camera pan) — a no-op for any pointer that isn't the one {@link dispatchPointerDown}
   * claimed.
   * @param screen The pointer position in screen (DOM) coordinates.
   * @param ev The originating `PointerEvent`.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * declare const ev: PointerEvent;
   * engine.dispatchPointerMove({ x: 105, y: 200 }, ev);
   * ```
   */
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

  /** Routes a DOM pointerup to the owning tool (if any), then releases gesture ownership
   * (`toolGesture`/`panning`/`activePointerId` all reset) regardless of which gesture was active
   * — a no-op for any pointer that isn't the one {@link dispatchPointerDown} claimed.
   * @param screen The pointer position in screen (DOM) coordinates.
   * @param ev The originating `PointerEvent`.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * declare const ev: PointerEvent;
   * engine.dispatchPointerUp({ x: 105, y: 200 }, ev);
   * ```
   */
  dispatchPointerUp(screen: Point, ev: PointerEvent): void {
    if (ev.pointerId !== this.activePointerId) return; // a non-owning pointer
    if (this.toolGesture && this.activeTool) {
      this.activeTool.onPointerUp(this.camera.screenToScene(screen), ev);
    }
    this.toolGesture = false;
    this.panning = false;
    this.activePointerId = null;
  }

  /** Resizes the backend's renderer/viewport to CSS pixels and redraws the grid for the new
   * bounds.
   * @param width The new viewport width, in CSS pixels.
   * @param height The new viewport height, in CSS pixels.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.setViewport(1280, 720);
   * ```
   */
  setViewport(width: number, height: number): void {
    this.viewport = { width, height };
    this.opts.backend.resize(width, height);
    this.redrawGrid();
  }

  /** Force a re-reconcile. Needed for out-of-band `AssetChanged` notices, which
   * mutate the `AssetResolver` (cache-bust / placeholder) without a document
   * mutation, so the `store.subscribe` reconcile never fires for them.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.reconcileNow();
   * ```
   */
  reconcileNow(): void {
    this.reconciler.reconcile();
    this.tokens.reconcile(); // re-resolve token images too (AssetChanged path)
  }

  /** Push the camera transform to the backend and redraw the grid for the new view.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.applyCamera();
   * ```
   */
  applyCamera(): void {
    this.opts.backend.setCameraTransform(this.camera.transform());
    this.redrawGrid();
  }

  /** Recomputes the visible scene rect from the current camera + viewport, then redraws the
   * grid layer for it. Called after any camera or viewport change ({@link applyCamera},
   * {@link setViewport}, {@link setGrid}) — never on its own by a caller outside `RenderEngine`.
   * @example
   * ```
   * // private method; not part of the public API
   * this.redrawGrid();
   * ```
   */
  private redrawGrid(): void {
    const tl = this.camera.screenToScene({ x: 0, y: 0 });
    const br = this.camera.screenToScene({ x: this.viewport.width, y: this.viewport.height });
    const rect = { x: tl.x, y: tl.y, w: br.x - tl.x, h: br.y - tl.y };
    this.opts.backend.drawGrid(this.grid.lines(rect), this.gridColor);
  }

  /** Tears down this engine instance: unsubscribes from the document store and the `vision`
   * channel, then calls `DisplayBackend.destroy()` — which releases all GPU resources and
   * detaches the canvas. Does not individually tear down each doc-type
   * view (tokens/drawings/templates/walls/regions/pings); their render nodes are reclaimed as
   * part of the backend's own teardown, not by a per-view `destroy()` call here. Call once, when
   * the host unmounts the canvas; the instance is not reusable afterward.
   * @example
   * ```ts
   * import type { RenderEngine } from "@shadowcat/render";
   *
   * declare const engine: RenderEngine;
   * engine.destroy();
   * ```
   */
  destroy(): void {
    this.unsubscribe?.();
    this.unsubscribe = null;
    this.sceneSub?.unsubscribe();
    this.sceneSub = null;
    this.opts.backend.destroy();
  }
}
