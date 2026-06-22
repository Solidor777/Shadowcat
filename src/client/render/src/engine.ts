import type { DocumentStore, AssetResolver } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { VisibilityInput } from "./types";
import { Camera } from "./camera";
import { Compositor } from "./compositor";
import { Grid, type GridSpec } from "./grid";
import { LayerRegistry } from "./layers";
import { SceneReconciler } from "./reconciler";

/** Handle to a scene subscription (structurally matches @shadowcat/core's). */
export interface SceneSubscription {
  unsubscribe(): void;
}

/** Injected scene-subscribe function (no transport dependency in this package). */
export type SubscribeScene = (
  channel: string,
  onUpdate: (frame: { payload: unknown; computedAtSeq: number }) => void,
) => SceneSubscription;

export interface RenderEngineOpts {
  store: DocumentStore;
  assets: AssetResolver;
  backend: DisplayBackend;
  grid: GridSpec;
  /** Grid line color (0xRRGGBB) sampled from CSS tokens by the host; default slate. */
  gridColor?: number;
  /** Injected SceneDerived subscribe (from WorldSession via AppContext). */
  subscribeScene?: SubscribeScene;
  /** Called when a derived frame is applied (host observability hook). */
  onDerivedApplied?: () => void;
}

/** Orchestrates the render model over a DisplayBackend: layers, camera, grid, and
 * the store-driven reconciler. Framework- and Pixi-free (the backend is injected). */
export class RenderEngine {
  readonly camera = new Camera();
  readonly compositor: Compositor;
  private readonly layers = new LayerRegistry();
  private readonly grid: Grid;
  private readonly reconciler: SceneReconciler;
  private readonly gridColor: number;
  private viewport = { width: 0, height: 0 };
  private unsubscribe: (() => void) | null = null;
  private sceneSub: SceneSubscription | null = null;
  private pendingDerived: { input: VisibilityInput; seq: number } | null = null;
  /** Highest computed_at_seq applied to the mask; guards against regressing to an
   * older derived frame (latest-wins). */
  private lastAppliedSeq = -1;

  constructor(private readonly opts: RenderEngineOpts) {
    this.grid = new Grid(opts.grid);
    this.gridColor = opts.gridColor ?? 0x3a3a4a;
    this.reconciler = new SceneReconciler(opts.store, opts.assets, opts.backend);
    this.compositor = new Compositor(opts.backend);
  }

  start(): void {
    this.opts.backend.ensureLayers(this.layers.orderedIds());
    this.applyCamera();
    this.reconciler.reconcile();
    this.unsubscribe = this.opts.store.subscribe(() => {
      this.reconciler.reconcile();
      this.flushPendingDerived();
    });
    if (this.opts.subscribeScene) {
      // M8a's debug channel; M9 swaps a real vision channel (polygon payload).
      this.sceneSub = this.opts.subscribeScene("identity", (f) => this.onSceneFrame(f));
    }
  }

  private onSceneFrame(frame: { payload: unknown; computedAtSeq: number }): void {
    // Per-channel frames are monotonic in computed_at_seq and latest wins. Drop any
    // frame already superseded by an applied or a pending one — never regress the
    // mask to an older derived state (defends the M9 consumer against reordering).
    if (frame.computedAtSeq <= this.lastAppliedSeq) return;
    if (this.pendingDerived && frame.computedAtSeq <= this.pendingDerived.seq) return;
    const input = this.toVisibility(); // M9: parse frame.payload polygons
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
    this.compositor.setVisibility(input);
    this.opts.onDerivedApplied?.();
  }

  /** M8 identity: full visibility regardless of payload. M9 will take the frame
   * payload and parse polygon geometry into `visible`. */
  private toVisibility(): VisibilityInput {
    return { visible: [] };
  }

  /** Module-facing shader-filter seam (0.x). Forwards to the backend; no engine
   * consumer in M8 — the first consumers are token fx / Phase-3 VFX. */
  registerLayerFilter(layerId: string, filter: unknown): () => void {
    return this.opts.backend.addLayerFilter(layerId, filter);
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
