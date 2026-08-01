import type { ReadableDocuments, WireDocument, TemplateEngine } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { ShapeNodeSpec } from "./types";
import { parseColor, circlePoints, conePoints, squarePoints } from "./geometry";
import { sceneScopedDocs } from "./scene-scope";

/** Templates render as translucent filled areas. */
const FILL_ALPHA = 0.25;
const STROKE_WIDTH = 2;

/** Reconciles `doc_type:"template"` documents into the `templates` layer. */
export class TemplateView {
  private readonly ids = new Set<string>();

  /**
   * Constructs a view bound to `store`/`backend`; call `reconcile()` once to populate it.
   * @param store The document store to read `template` docs from.
   * @param backend The display backend to push resolved shape nodes to.
   * @param viewedSceneId Resolves the currently-viewed scene id; `reconcile()` scopes its
   * query to this scene (falls back to unscoped — every `template` doc in the store —
   * when it resolves to `null`). Defaults to always-`null` (legacy/test callers that
   * never pass one).
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
  ) {}

  /**
   * Diffs the store's `template` docs (scoped to `viewedSceneId`) against the ids tracked
   * in `ids`: every current doc gets a fresh spec and an upsert via `backend.setShape`,
   * and every tracked id no longer present is torn down via `backend.removeShape`. A doc
   * whose `toSpec` resolves to `null` (a missing `engine.shape` or an unrecognized
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
    for (const doc of sceneScopedDocs(this.store, "template", this.viewedSceneId)) {
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
 * Tessellates a `template` doc's `engine.shape` into a flat-point `ShapeNodeSpec`. `x,y`
 * is always the anchor and `size` a radius/length (never a second corner): `circle` and
 * `cone` tessellate via `circlePoints`/`conePoints`; `rect` tessellates via `squarePoints`
 * — an axis-**rotated** square of side `2*size` centered at `(x,y)`, unlike
 * `drawing-view.ts`'s and `region-view.ts`'s `"rect"` kind, which is an axis-aligned bbox
 * between two authored corners (the `"rect"` string means different geometry in each
 * file). `line` builds a 2-point open segment from `(x,y)` at `direction` degrees, length
 * `size`, and is the only kind that returns `closed:false` (and therefore no `fill`).
 * Returns `null` for a missing `engine.shape`, an unrecognized `kind`, or a non-numeric
 * `x`/`y`/`size`/`direction`. What actually arrives is `null`, not `Infinity`: an oversized
 * magnitude survives `serde_json::from_value` as `f64::INFINITY`, but `normalize_engine`'s
 * round-trip re-serializes it and `serde_json`'s `From<f64>` maps any non-finite to
 * `Value::Null` — and that normalized value is what gets persisted and broadcast.
 * `WireDocument.engine` is `z.unknown()`, so nothing downstream re-checks it. Guarded on the
 * RAW authored scalars, before tessellation, matching `drawing-view.ts`, `region-view.ts`, and
 * `wall-view.ts` — see the guard's own comment for why placement matters.
 * @param doc The `template` document to convert.
 * @returns A `ShapeNodeSpec` for the `templates` layer, or `null` if it can't be rendered.
 * @example
 * ```
 * // not exported from @shadowcat/render's index.ts; internal to TemplateView.reconcile
 * const spec = toSpec(doc); // null if doc.engine.shape is absent or malformed
 * ```
 */
function toSpec(doc: WireDocument): ShapeNodeSpec | null {
  const s = doc.engine as TemplateEngine | undefined;
  if (!s?.shape) return null;
  const { kind, x, y, size, direction } = s.shape;
  // Checked on the RAW authored scalars, before tessellation — matching `region-view.ts` and
  // `wall-view.ts`. Post-tessellation would be too late: JS coerces `null` to 0 in arithmetic,
  // so a null `x` on a circle yields finite, plausible-looking geometry no later check can
  // distinguish from an authored shape. `direction` is included because `cone`/`rect`/`line`
  // route it through `cos`/`sin`.
  if (![x, y, size, direction].every((n) => Number.isFinite(n))) return null;
  let points: number[];
  let closed = true;
  switch (kind) {
    case "circle":
      points = circlePoints(x, y, size);
      break;
    case "cone":
      points = conePoints(x, y, size, direction);
      break;
    case "rect":
      points = squarePoints(x, y, size, direction);
      break;
    case "line": {
      const a = (direction * Math.PI) / 180;
      points = [x, y, x + size * Math.cos(a), y + size * Math.sin(a)];
      closed = false;
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
