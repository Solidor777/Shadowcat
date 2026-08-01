import type { ReadableDocuments, WireDocument, DrawingEngine } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { ShapeNodeSpec } from "./types";
import { parseColor, rectPoints, ellipsePoints } from "./geometry";
import { sceneScopedDocs } from "./scene-scope";

/** Reconciles `doc_type:"drawing"` documents into the `drawings` layer as shape nodes. */
export class DrawingView {
  private readonly ids = new Set<string>();

  /**
   * Constructs a view bound to `store`/`backend`; call `reconcile()` once to populate it.
   * @param store The document store to read `drawing` docs from.
   * @param backend The display backend to push resolved shape nodes to.
   * @param viewedSceneId Resolves the currently-viewed scene id; `reconcile()` scopes its
   * query to this scene (falls back to unscoped — every `drawing` doc in the store — when
   * it resolves to `null`). Defaults to always-`null` (legacy/test callers that never pass
   * one).
   * @example
   * ```ts
   * import { DrawingView, MockBackend } from "@shadowcat/render";
   * import type { ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new DrawingView(store, new MockBackend());
   * ```
   */
  constructor(
    private readonly store: ReadableDocuments,
    private readonly backend: DisplayBackend,
    private readonly viewedSceneId: () => string | null = () => null,
  ) {}

  /**
   * Diffs the store's `drawing` docs (scoped to `viewedSceneId`) against the ids tracked
   * in `ids`: every current doc gets a fresh spec and an upsert via `backend.setShape`
   * (one call handles both create and edit — the backend decides which), and every
   * tracked id no longer present is torn down via `backend.removeShape`. A doc whose
   * `toSpec` resolves to `null` (an unrecognized `shape.kind`, or a `rect`/`ellipse` with
   * fewer than 4 bbox coordinates) is treated as absent — never added to `seen`, so it is
   * torn down on this same pass if it was tracked.
   * @example
   * ```ts
   * import { DrawingView, MockBackend } from "@shadowcat/render";
   * import type { ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new DrawingView(store, new MockBackend());
   * view.reconcile();
   * ```
   */
  reconcile(): void {
    const seen = new Set<string>();
    for (const doc of sceneScopedDocs(this.store, "drawing", this.viewedSceneId)) {
      const spec = toSpec(doc);
      if (!spec) continue;
      seen.add(doc.id);
      this.ids.add(doc.id);
      this.backend.setShape(doc.id, spec); // upsert (handles create + edit)
    }
    for (const id of [...this.ids]) {
      if (seen.has(id)) continue;
      this.ids.delete(id);
      this.backend.removeShape(id);
    }
  }
}

/**
 * Tessellates a `drawing` doc's `engine.shape` into a flat-point `ShapeNodeSpec`.
 * `freehand`/`line`/`polygon` pass their authored `points` through unchanged (open for
 * the first two, closed for `polygon`); `rect`/`ellipse` read exactly 4 bbox-corner
 * coordinates and tessellate via `rectPoints`/`ellipsePoints`. Returns `null` for a
 * missing `engine.shape`, an unrecognized `kind`, or a `rect`/`ellipse` with fewer than 4
 * points — a malformed doc simply doesn't render. Non-finite coordinates also yield `null`:
 * JSON has no NaN/Infinity literal, but an oversized magnitude (`1e400`) parses to `Infinity`,
 * which would otherwise reach Pixi as NaN geometry. Checked on the TESSELLATED output, so a
 * non-finite input caught via `rectPoints`/`ellipsePoints` is rejected too. Matches
 * `region-view.ts` and `wall-view.ts`.
 * @param doc The `drawing` document to convert.
 * @returns A `ShapeNodeSpec` for the `drawings` layer, or `null` if it can't be rendered.
 * @example
 * ```
 * // not exported from @shadowcat/render's index.ts; internal to DrawingView.reconcile
 * const spec = toSpec(doc); // null if doc.engine.shape is absent or malformed
 * ```
 */
function toSpec(doc: WireDocument): ShapeNodeSpec | null {
  const s = doc.engine as DrawingEngine | undefined;
  if (!s?.shape) return null;
  const { kind, points } = s.shape;
  let pts = points;
  let closed = false;
  switch (kind) {
    case "freehand":
    case "line":
      break; // raw polyline
    case "polygon":
      closed = true;
      break;
    case "rect":
      if (points.length < 4) return null;
      pts = rectPoints(points[0], points[1], points[2], points[3]);
      closed = true;
      break;
    case "ellipse":
      if (points.length < 4) return null;
      pts = ellipsePoints(points[0], points[1], points[2], points[3]);
      closed = true;
      break;
    default:
      return null;
  }
  // Checked post-tessellation: a non-finite input propagates through rectPoints/ellipsePoints,
  // so the output covers both authored and derived coordinates in one guard.
  if (!pts.every((n) => Number.isFinite(n))) return null;
  return {
    layer: "drawings",
    points: pts,
    closed,
    stroke: s.stroke ? { color: parseColor(s.stroke.color), width: s.stroke.width } : null,
    fill: s.fill ? { color: parseColor(s.fill.color), alpha: s.fill.alpha ?? 1 } : null,
  };
}
