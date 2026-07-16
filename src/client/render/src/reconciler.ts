import type { ReadableDocuments, AssetResolver, WireDocument, SceneEngine } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";

/** Maps scene-entity documents to display objects. M8c-1 handles the scene
 * background only; M8d adds per-doc_type handlers (token/wall/tile/…). */
export class SceneReconciler {
  constructor(
    private readonly store: ReadableDocuments,
    private readonly assets: AssetResolver,
    private readonly backend: DisplayBackend,
    private readonly viewedSceneId: () => string | null = () => null,
  ) {}

  reconcile(): void {
    // The viewed scene's background (M12d). `null` viewed id ⇒ the first scene (legacy
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
