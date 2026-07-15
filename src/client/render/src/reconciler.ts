import type { ReadableDocuments, AssetResolver, WireDocument } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";

/** The scene document's engine-reserved system fields (M8 §4.2: opaque to the
 * server, interpreted by the client). M8c-1 reads only `background`. */
interface SceneSystem {
  background?: string;
}

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
    const bg = (scene?.system as SceneSystem | undefined)?.background;
    if (typeof bg === "string" && bg.length > 0) {
      this.backend.setBackground({ url: this.assets.url(bg) });
    } else {
      this.backend.setBackground(null);
    }
  }
}
