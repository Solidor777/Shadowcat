import type { ReadableDocuments, WireDocument, TemplateEngine } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { ShapeNodeSpec } from "./types";
import {
  parseColor,
  circlePoints,
  conePoints,
  squarePoints,
  linePoints,
  isDrawableConeAngle,
  DEFAULT_CONE_ANGLE_DEGREES,
} from "./geometry";
import { sceneScopedDocs } from "./scene-scope";

/** Templates render as translucent filled areas. */
const FILL_ALPHA = 0.25;
const STROKE_WIDTH = 2;

/** Reconciles `doc_type:"template"` documents into the `templates` layer. */
export class TemplateView {
  /** Document ids currently tracked in the backend, refreshed each `reconcile()`. */
  private readonly ids = new Set<string>();

  /**
   * Constructs a view bound to `store`/`backend`; call `reconcile()` once to populate it.
   * @param store The document store to read `template` docs from.
   * @param backend The display backend to push resolved shape nodes to.
   * @param viewedSceneId Resolves the currently-viewed scene id; `reconcile()` scopes its
   * query to this scene (falls back to unscoped — every `template` doc in the store —
   * when it resolves to `null`). Defaults to always-`null` (legacy/test callers that
   * never pass one).
   * @param viewedLevel Resolves the currently-viewed level id; `reconcile()` additionally
   * scopes its query to this level (see `sceneScopedDocs`). Defaults to always-`null` (every
   * level — the degenerate pre-levels case).
   * @example
   * ```ts
   * import { TemplateView, MockBackend } from "@shadowcat/render";
   * import type { ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new TemplateView(store, new MockBackend());
   * ```
   */
  constructor(
    private readonly store: ReadableDocuments,
    private readonly backend: DisplayBackend,
    private readonly viewedSceneId: () => string | null = () => null,
    private readonly viewedLevel: () => string | null = () => null,
  ) {}

  /**
   * Diffs the store's `template` docs (scoped to `viewedSceneId`) against the ids tracked
   * in `ids`: every current doc gets a fresh spec and an upsert via `backend.setShape`,
   * and every tracked id no longer present is torn down via `backend.removeShape`. A doc
   * whose `templateShapeSpec` resolves to `null` (a missing `engine.shape` or an unrecognized
   * `shape.kind`) is treated as absent — never added to `seen`, so it is torn down on
   * this same pass if it was tracked.
   * @example
   * ```ts
   * import { TemplateView, MockBackend } from "@shadowcat/render";
   * import type { ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new TemplateView(store, new MockBackend());
   * view.reconcile();
   * ```
   */
  reconcile(): void {
    const seen = new Set<string>();
    for (const doc of sceneScopedDocs(this.store, "template", this.viewedSceneId, this.viewedLevel)) {
      const spec = templateShapeSpec(doc);
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
 * Tessellates a `template` doc's `engine.shape` into a flat-point `ShapeNodeSpec`. `x,y`
 * is always the anchor and `size` a radius/length (never a second corner): `circle`
 * tessellates via `circlePoints`; `cone` via `conePoints`, its full aperture (the angle
 * BETWEEN the two base corners) `angle ?? {@link DEFAULT_CONE_ANGLE_DEGREES}`; `rect`
 * tessellates via `squarePoints` — an axis-**rotated** rectangle of sides `2*size` by
 * `2*(width ?? size)` centered at `(x,y)` (forced-square exactly when `width` is absent),
 * unlike `DrawingView.toSpec`'s and `RegionView.toSpec`'s `"rect"` kind, which is an
 * axis-aligned bbox between two authored corners (the `"rect"` string means different
 * geometry in each file). `line` builds a segment from `(x,y)` at `direction` degrees,
 * length `size`, via `linePoints`: a bare open 2-point segment when `width` is absent or
 * `0` (`closed:false`, no `fill`), or a
 * closed rectangle of thickness `width` otherwise. Returns `null` for a missing
 * `engine.shape`, an unrecognized `kind`, a non-numeric `x`/`y`/`size`/`direction`/authored
 * `angle`/`width`, or an `angle` that fails `isDrawableConeAngle` (non-finite, or outside
 * `(0.0, 180.0)`, matching `TemplateEngine::validate`'s own bound, since the optimistic store can
 * hold engine data the server has not yet validated). This is defense-in-depth, not a claim about any particular
 * upstream conversion: the render layer draws the OPTIMISTIC view (`AppContext.documents`), so
 * a scene-tool bug that builds a Create op with a missing or non-numeric coordinate reaches
 * `templateShapeSpec` on the authoring client before the server has validated anything. Guarded
 * on the RAW authored scalars, before tessellation, matching `drawingShapeSpec`, `regionShapeSpec`,
 * and `WallView.toSpec` — see the guard's own comment for why the placement, not just the
 * presence, is load-bearing. Exported so `scene-tools`' hit-test can reuse the SAME tessellation
 * this view draws from, rather than forking a second copy of the shape math.
 * @param doc The `template` document to convert.
 * @returns A `ShapeNodeSpec` for the `templates` layer, or `null` if it can't be rendered.
 * @example
 * ```ts
 * import { templateShapeSpec } from "@shadowcat/render";
 * import type { WireDocument } from "@shadowcat/core";
 *
 * declare const doc: WireDocument;
 * const spec = templateShapeSpec(doc); // null if doc.engine.shape is absent or malformed
 * ```
 */
export function templateShapeSpec(doc: WireDocument): ShapeNodeSpec | null {
  const s = doc.engine as TemplateEngine | undefined;
  if (!s?.shape) return null;
  const { kind, x, y, size, direction, angle, width } = s.shape;
  // Checked on the RAW authored scalars, before tessellation — matching `regionShapeSpec` and
  // `WallView.toSpec`. Post-tessellation would be too late: JS coerces `null` to 0 in arithmetic,
  // so a null `x` on a circle yields finite, plausible-looking geometry no later check can
  // distinguish from an authored shape. `direction` is included because `cone`/`rect`/`line`
  // route it through `cos`/`sin`. `angle`/`width` are optional (`number | null`) but must be
  // finite when authored, for the same reason.
  if (![x, y, size, direction].every((n) => Number.isFinite(n))) return null;
  // `angle`/`width` use loose null-checks: a genuine server document always carries the
  // key (Option<f64> serializes to `null`, never omitted), but a pre-migration/legacy
  // in-memory doc may omit it entirely (`undefined`) — both mean "absent, use the default".
  if (width != null && !Number.isFinite(width)) return null;
  // An out-of-`(0.0, 180.0)` cone angle is rejected the same as a non-finite one: the
  // optimistic store can hold engine data the server has not yet validated/rejected — the same
  // precedent `regionShapeSpec`'s own null-part guard documents. `isDrawableConeAngle` is the
  // guard `templateToRegionShape` reads too.
  if (!isDrawableConeAngle(angle)) return null;
  let points: number[];
  let closed = true;
  switch (kind) {
    case "circle":
      points = circlePoints(x, y, size);
      break;
    case "cone":
      points = conePoints(x, y, size, direction, angle ?? DEFAULT_CONE_ANGLE_DEGREES);
      break;
    case "rect":
      points = squarePoints(x, y, size, direction, width ?? size);
      break;
    case "line": {
      const w = width ?? 0;
      points = linePoints(x, y, size, direction, w);
      closed = w > 0;
      break;
    }
    default:
      return null;
  }
  const color = parseColor(s.color);
  return {
    layer: "templates",
    points,
    closed,
    stroke: { color, width: STROKE_WIDTH },
    fill: closed ? { color, alpha: FILL_ALPHA } : null,
  };
}
