import type { ReadableDocuments, WireDocument, LightEngine } from "@shadowcat/core";
import { circlePoints, parseColor } from "./geometry";
import type { DisplayBackend } from "./backend";
import type { ShapeNodeSpec } from "./types";
import { sceneScopedDocs } from "./scene-scope";

/** Light-marker pick/display radius in scene units. A point marker (not the light's reach —
 * radius RINGS belong to the editing overlay the tool draws, so the resting canvas stays
 * uncluttered). */
const MARKER_RADIUS = 8;
/** Marker outline color (contrast against the light's own fill color). */
const MARKER_STROKE = 0x222222;

/** Reconciles `doc_type:"light"` documents into the `walls` layer as point markers. Like
 * `WallView`, this renders every light the recipient's store holds — document egress is
 * permission-based, not doc-type-based, so players see the markers too; the EDITING affordance
 * is GM-gated at the tool layer (`ToolRail`'s per-tool visibility), not here. A dumb per-frame
 * reconciler with no client-side secrecy logic. */
export class LightView {
  /** Document ids currently tracked in the backend, refreshed each `reconcile()`. */
  private readonly ids = new Set<string>();

  /**
   * Constructs a view bound to `store`/`backend`; call `reconcile()` once to populate it.
   * @param store The document store to read `light` docs from.
   * @param backend The display backend to push resolved shape nodes to.
   * @param viewedSceneId Resolves the currently-viewed scene id; `reconcile()` scopes its
   * query to this scene (falls back to unscoped when it resolves to `null`). Defaults to
   * always-`null` (legacy/test callers that never pass one).
   * @example
   * ```ts
   * import { LightView, MockBackend } from "@shadowcat/render";
   * import type { ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new LightView(store, new MockBackend());
   * ```
   */
  constructor(
    private readonly store: ReadableDocuments,
    private readonly backend: DisplayBackend,
    private readonly viewedSceneId: () => string | null = () => null,
  ) {}

  /**
   * Diffs the store's `light` docs (scoped to `viewedSceneId`) against the ids tracked in
   * `ids`: every current doc gets a fresh marker spec and an upsert via `backend.setShape`, and
   * every tracked id no longer present is torn down via `backend.removeShape`. A doc whose
   * `toSpec` resolves to `null` (missing `engine` or a non-finite position) is treated as
   * absent — never added to `seen`, so it is torn down on this same pass if it was tracked.
   * @example
   * ```ts
   * import { LightView, MockBackend } from "@shadowcat/render";
   * import type { ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new LightView(store, new MockBackend());
   * view.reconcile();
   * ```
   */
  reconcile(): void {
    const seen = new Set<string>();
    for (const doc of sceneScopedDocs(this.store, "light", this.viewedSceneId)) {
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
 * Converts a `light` doc into its marker `ShapeNodeSpec`: a small filled disc at the light's
 * position, filled with the emission color (dimmed when the emission is disabled). Returns
 * `null` for a missing `engine` or a non-finite position — a malformed light just doesn't
 * render rather than pushing NaN into the geometry (the same guard shape `WallView.toSpec`
 * carries).
 * @param doc The `light` document to convert.
 * @returns A `ShapeNodeSpec` for the `walls` layer, or `null` if it can't be rendered.
 * @example
 * ```
 * // not exported from @shadowcat/render; internal to LightView.reconcile
 * declare const doc: WireDocument;
 * const spec = toSpec(doc); // null if doc.engine is absent or its position is non-finite
 * ```
 */
function toSpec(doc: WireDocument): ShapeNodeSpec | null {
  const eng = doc.engine as LightEngine | undefined;
  if (!eng) return null;
  const { x, y } = eng;
  if (!Number.isFinite(x) || !Number.isFinite(y)) return null;
  if (!eng.emission) return null;
  return {
    layer: "walls",
    points: circlePoints(x, y, MARKER_RADIUS),
    closed: true,
    stroke: { color: MARKER_STROKE, width: 1 },
    fill: { color: parseColor(eng.emission.color), alpha: eng.emission.enabled ? 0.9 : 0.25 },
  };
}
