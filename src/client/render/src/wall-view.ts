import type { ReadableDocuments, WireDocument, WallEngine } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { ShapeNodeSpec } from "./types";
import { sceneScopedDocs } from "./scene-scope";

/** Walls render as a distinct stroked segment (GMs author + see them; per-recipient
 * hidden walls are a later permission refinement). */
const WALL_COLOR = 0xd06060;
const WALL_WIDTH = 4;

/** Reconciles `doc_type:"wall"` documents into the `walls` layer as line segments. */
export class WallView {
  /** Document ids currently tracked in the backend, refreshed each `reconcile()`. */
  private readonly ids = new Set<string>();

  /**
   * Constructs a view bound to `store`/`backend`; call `reconcile()` once to populate it.
   * @param store The document store to read `wall` docs from.
   * @param backend The display backend to push resolved shape nodes to.
   * @param viewedSceneId Resolves the currently-viewed scene id; `reconcile()` scopes its
   * query to this scene (falls back to unscoped — every `wall` doc in the store — when it
   * resolves to `null`). Defaults to always-`null` (legacy/test callers that never pass
   * one).
   * @example
   * ```ts
   * import { WallView, MockBackend } from "@shadowcat/render";
   * import type { ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new WallView(store, new MockBackend());
   * ```
   */
  constructor(
    private readonly store: ReadableDocuments,
    private readonly backend: DisplayBackend,
    private readonly viewedSceneId: () => string | null = () => null,
  ) {}

  /**
   * Diffs the store's `wall` docs (scoped to `viewedSceneId`) against the ids tracked in
   * `ids`: every current doc gets a fresh spec and an upsert via `backend.setShape`, and
   * every tracked id no longer present is torn down via `backend.removeShape`. A doc
   * whose `toSpec` resolves to `null` (a missing `engine.seg` or a non-finite endpoint)
   * is treated as absent — never added to `seen`, so it is torn down on this same pass if
   * it was tracked. The rendered segment is a single uniform color/width regardless of
   * `blocksSight`/`blocksMove`/`blocksLight` — this view does not visually distinguish
   * those flags.
   * @example
   * ```ts
   * import { WallView, MockBackend } from "@shadowcat/render";
   * import type { ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new WallView(store, new MockBackend());
   * view.reconcile();
   * ```
   */
  reconcile(): void {
    const seen = new Set<string>();
    for (const doc of sceneScopedDocs(this.store, "wall", this.viewedSceneId)) {
      const spec = toSpec(doc);
      if (!spec) continue;
      seen.add(doc.id);
      this.ids.add(doc.id);
      this.backend.setShape(doc.id, spec);
    }
    for (const id of [...this.ids]) {
      if (seen.has(id)) continue;
      this.ids.delete(id);
      this.backend.removeShape(id);
    }
  }
}

/**
 * Converts a `wall` doc's `engine.seg` into a 2-point `ShapeNodeSpec` line. Returns `null`
 * for a missing `engine.seg` or any non-finite endpoint.
 * @param doc The `wall` document to convert.
 * @returns A `ShapeNodeSpec` for the `walls` layer, or `null` if it can't be rendered.
 * @example
 * ```
 * // not exported from @shadowcat/render; internal to WallView.reconcile
 * const spec = toSpec(doc); // null if doc.engine.seg is absent or non-finite
 * ```
 */
function toSpec(doc: WireDocument): ShapeNodeSpec | null {
  const s = doc.engine as WallEngine | undefined;
  if (!s?.seg) return null;
  const { x1, y1, x2, y2 } = s.seg;
  // `WallEngine` is round-tripped through serde on ingress (`normalize_engine`) but never
  // passed through a `.validate()` call (unlike the
  // "token" doc_type), so a non-finite endpoint isn't ruled out server-side; guard here
  // instead (a malformed wall just doesn't render rather than pushing NaN into the
  // geometry).
  if (![x1, y1, x2, y2].every((n) => Number.isFinite(n))) return null;
  return {
    layer: "walls",
    points: [x1, y1, x2, y2],
    closed: false,
    stroke: { color: WALL_COLOR, width: WALL_WIDTH },
    fill: null,
  };
}
