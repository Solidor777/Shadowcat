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
  ) {}

  reconcile(): void {
    // M8c-1 assumes a single active scene; `[0]` is insertion-order. Deterministic
    // active-scene selection among multiple scene docs is deferred to M8d (see
    // docs/TODO.md) when scene authoring can create more than one.
    const scene = this.store.query("scene")[0] as WireDocument | undefined;
    const bg = (scene?.system as SceneSystem | undefined)?.background;
    if (typeof bg === "string" && bg.length > 0) {
      this.backend.setBackground({ url: this.assets.url(bg) });
    } else {
      this.backend.setBackground(null);
    }
  }
}
