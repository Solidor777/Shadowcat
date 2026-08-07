import type { ReadableDocuments, AssetResolver, WireDocument, SceneEngine } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";

/**
 * Reconciles the viewed scene's background image only. Every other scene-entity
 * doc_type (token/wall/drawing/template/region) has its own dedicated view class
 * (`TokenView`, `WallView`, …); this class does not dispatch to them — it exists solely
 * to set or clear `DisplayBackend`'s background layer from the viewed `scene` doc.
 */
export class SceneReconciler {
  /**
   * Constructs a reconciler bound to `store`/`assets`/`backend`; call `reconcile()` once
   * to populate it.
   * @param store The document store to read the viewed `scene` doc from.
   * @param assets Resolves the scene's `background` asset id to a serve URL.
   * @param backend The display backend to push the background to.
   * @param viewedSceneId Resolves the currently-viewed scene id; `reconcile()` reads that
   * scene's background. Falls back to the store's first `scene` doc (insertion order) when
   * it resolves to `null` (legacy single-scene case). Defaults to always-`null`
   * (legacy/test callers that never pass one).
   * @example
   * ```ts
   * import { SceneReconciler, MockBackend } from "@shadowcat/render";
   * import { AssetResolver, type ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const reconciler = new SceneReconciler(store, new AssetResolver(), new MockBackend());
   * ```
   */
  constructor(
    private readonly store: ReadableDocuments,
    private readonly assets: AssetResolver,
    private readonly backend: DisplayBackend,
    private readonly viewedSceneId: () => string | null = () => null,
  ) {}

  /**
   * Resolves the viewed scene's `engine.background` field and pushes it to the backend: a
   * non-empty string resolves through `assets.url` and calls `backend.setBackground` with
   * it; an absent, empty, or non-string value calls `backend.setBackground(null)` to clear
   * the background layer.
   * @example
   * ```ts
   * import { SceneReconciler, MockBackend } from "@shadowcat/render";
   * import { AssetResolver, type ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const reconciler = new SceneReconciler(store, new AssetResolver(), new MockBackend());
   * reconciler.reconcile();
   * ```
   */
  reconcile(): void {
    // The viewed scene's background. `null` viewed id ⇒ the first scene (legacy
    // single-scene behavior; `[0]` is insertion-order).
    const vsid = this.viewedSceneId();
    const scene = (vsid ? this.store.get(vsid) : this.store.query("scene")[0]) as WireDocument | undefined;
    const bg = (scene?.engine as SceneEngine | undefined)?.background;
    if (typeof bg === "string" && bg.length > 0) {
      this.backend.setBackground({ url: this.assets.url(bg) });
    } else {
      this.backend.setBackground(null);
    }
  }
}
