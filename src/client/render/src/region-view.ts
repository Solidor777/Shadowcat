import type { ReadableDocuments, WireDocument, RegionEngine } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { ShapeNodeSpec } from "./types";
import { rectPoints, circlePoints } from "./geometry";
import { sceneScopedDocs } from "./scene-scope";

/** Fill tint per behavior — distinct from walls (red stroke) and drawings, so a GM can tell a
 * hazard's kind at a glance. Alpha kept low: regions must not visually dominate the token layer. */
const BEHAVIOR_FILL: Record<string, number> = {
  terrain: 0xd0a030,
  impassable: 0xd06060,
  arrest: 0x9040c0,
};
const FILL_ALPHA = 0.25;
const STROKE_WIDTH = 2;

/** Reconciles `doc_type:"region"` documents into the `regions` layer as tinted shapes. Only
 * regions the viewer is permitted to see ever reach `store` (server-side egress filtering, spec
 * §3) — there is no client-side hide check to get wrong. */
export class RegionView {
  private readonly ids = new Set<string>();

  /**
   * Constructs a view bound to `store`/`backend`; call `reconcile()` once to populate it.
   * @param store The document store to read `region` docs from.
   * @param backend The display backend to push resolved shape nodes to.
   * @param viewedSceneId Resolves the currently-viewed scene id; `reconcile()` scopes its
   * query to this scene (falls back to unscoped — every `region` doc in the store — when
   * it resolves to `null`). Defaults to always-`null` (legacy/test callers that never pass
   * one).
   * @example
   * ```ts
   * import { RegionView, MockBackend } from "@shadowcat/render";
   * import type { ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new RegionView(store, new MockBackend());
   * ```
   */
  constructor(
    private readonly store: ReadableDocuments,
    private readonly backend: DisplayBackend,
    private readonly viewedSceneId: () => string | null = () => null,
  ) {}

  /**
   * Diffs the store's `region` docs (scoped to `viewedSceneId`) against the ids tracked in
   * `ids`: every current doc gets a fresh spec and an upsert via `backend.setShape`, and
   * every tracked id no longer present is torn down via `backend.removeShape`. A doc whose
   * `toSpec` resolves to `null` (an unrecognized `shape.kind`, a non-finite coordinate, or
   * a malformed point count for that kind) is treated as absent — never added to `seen`,
   * so it is torn down on this same pass if it was tracked.
   * @example
   * ```ts
   * import { RegionView, MockBackend } from "@shadowcat/render";
   * import type { ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new RegionView(store, new MockBackend());
   * view.reconcile();
   * ```
   */
  reconcile(): void {
    const seen = new Set<string>();
    for (const doc of sceneScopedDocs(this.store, "region", this.viewedSceneId)) {
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
 * Tessellates a `region` doc's `engine.shape` into a flat-point `ShapeNodeSpec`, tinted by
 * `behavior` via `BEHAVIOR_FILL` (an unrecognized/absent `behavior` falls back to the
 * `terrain` color). `rect`/`circle`/`polygon` each require an exact-or-minimum `points`
 * count for that kind (bbox corners, center+radius, or `>=3` vertices respectively) and
 * tessellate via `rectPoints`/`circlePoints`, or pass authored polygon points through
 * unchanged. Returns `null` for a missing/malformed `engine.shape`, an unrecognized
 * `kind`, a wrong point count, or any non-finite coordinate — a malformed doc simply
 * doesn't render.
 * @param doc The `region` document to convert.
 * @returns A `ShapeNodeSpec` for the `regions` layer, or `null` if it can't be rendered.
 * @example
 * ```
 * // not exported from @shadowcat/render's index.ts; internal to RegionView.reconcile
 * const spec = toSpec(doc); // null if doc.engine.shape is absent or malformed
 * ```
 */
function toSpec(doc: WireDocument): ShapeNodeSpec | null {
  const s = doc.engine as RegionEngine | undefined;
  const shape = s?.shape;
  if (!shape?.kind || !Array.isArray(shape.points)) return null;
  const pts = shape.points;
  if (!pts.every((n) => Number.isFinite(n))) return null;

  let points: number[];
  switch (shape.kind) {
    case "rect":
      if (pts.length !== 4) return null;
      points = rectPoints(pts[0], pts[1], pts[2], pts[3]);
      break;
    case "circle":
      if (pts.length !== 3) return null;
      points = circlePoints(pts[0], pts[1], pts[2]);
      break;
    case "polygon":
      if (pts.length < 6 || pts.length % 2 !== 0) return null;
      points = pts;
      break;
    default:
      return null;
  }

  const color = BEHAVIOR_FILL[s?.behavior ?? ""] ?? BEHAVIOR_FILL.terrain;
  return {
    layer: "regions",
    points,
    closed: true,
    stroke: { color, width: STROKE_WIDTH },
    fill: { color, alpha: FILL_ALPHA },
  };
}
