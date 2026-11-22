import type { ReadableDocuments, WireDocument, RegionEngine, RegionShape, CompoundShapePart } from "@shadowcat/core";
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
  ambient_light: 0x3060b0,
};
const FILL_ALPHA = 0.25;
const STROKE_WIDTH = 2;
/** Fill of a region that blocks sight: a pale haze, denser than a behavior tint so a fog bank
 * reads as fog. */
const SIGHT_HAZE_COLOR = 0xcfd8e0;
/** Opacity of the sight haze fill. */
const SIGHT_HAZE_ALPHA = 0.55;
/** Rim color of a region that blocks light. */
const LIGHT_RIM_COLOR = 0x101018;
/** Rim width of a region that blocks light, wider than the plain outline. */
const LIGHT_RIM_WIDTH = 5;
/** Dash pattern of the ring of a region that blocks sound, in scene units. */
const SOUND_DASH = { on: 12, off: 8 };

/** Reconciles `doc_type:"region"` documents into the `regions` layer as tinted shapes. Only
 * regions the viewer is permitted to see ever reach `store` (server-side egress filtering) —
 * there is no client-side hide check to get wrong. */
export class RegionView {
  /** Document ids currently tracked in the backend, refreshed each `reconcile()`. */
  private readonly ids = new Set<string>();

  /**
   * Constructs a view bound to `store`/`backend`; call `reconcile()` once to populate it.
   * @param store The document store to read `region` docs from.
   * @param backend The display backend to push resolved shape nodes to.
   * @param viewedSceneId Resolves the currently-viewed scene id; `reconcile()` scopes its
   * query to this scene (falls back to unscoped — every `region` doc in the store — when
   * it resolves to `null`). Defaults to always-`null` (legacy/test callers that never pass
   * one).
   * @param viewedLevel Resolves the currently-viewed level id; `reconcile()` additionally
   * scopes its query to this level (see `sceneScopedDocs`). Defaults to always-`null` (every
   * level — the degenerate pre-levels case).
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
    private readonly viewedLevel: () => string | null = () => null,
  ) {}

  /**
   * Diffs the store's `region` docs (scoped to `viewedSceneId`) against the ids tracked in
   * `ids`: every current doc gets a fresh spec and an upsert via `backend.setShape`, and
   * every tracked id no longer present is torn down via `backend.removeShape`. A doc whose
   * `regionShapeSpec` resolves to `null` (an unrecognized `shape.kind`, a non-finite coordinate, or
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
    for (const doc of sceneScopedDocs(this.store, "region", this.viewedSceneId, this.viewedLevel)) {
      const spec = regionShapeSpec(doc);
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
 * Tessellates ONE wire `RegionShape` (`{kind, points}`) into a flat-point closed ring:
 * `rect`/`circle`/`polygon` each require an exact-or-minimum `points` count for that kind
 * (bbox corners, center+radius, or `>=3` vertices respectively) and tessellate via
 * `rectPoints`/`circlePoints`, or pass authored polygon points through unchanged. Returns
 * `null` for a missing/malformed shape, an unrecognized `kind`, a wrong point count, or any
 * non-finite coordinate. Exported so `regionShapeSpec` (per region part) and scene-tools'
 * compound-authoring preview tessellate through the ONE outline rule, never a forked copy.
 * @param shape The wire shape to tessellate.
 * @returns The flat ring points, or `null` when the shape can't be outlined.
 * @example
 * ```ts
 * import { regionShapeOutline } from "@shadowcat/render";
 *
 * regionShapeOutline({ kind: "rect", points: [0, 0, 10, 10] }); // [0,0, 10,0, 10,10, 0,10]
 * regionShapeOutline({ kind: "hexagon", points: [] }); // null
 * ```
 */
export function regionShapeOutline(shape: RegionShape | undefined): number[] | null {
  if (!shape?.kind || !Array.isArray(shape.points)) return null;
  const pts = shape.points;
  if (!pts.every((n) => Number.isFinite(n))) return null;
  switch (shape.kind) {
    case "rect":
      return pts.length === 4 ? rectPoints(pts[0], pts[1], pts[2], pts[3]) : null;
    case "circle":
      return pts.length === 3 ? circlePoints(pts[0], pts[1], pts[2]) : null;
    case "polygon":
      return pts.length >= 6 && pts.length % 2 === 0 ? pts : null;
    default:
      return null;
  }
}

/**
 * Tessellates a `region` doc's `engine.shape` into a flat-point `ShapeNodeSpec`, tinted by
 * `behavior` via `BEHAVIOR_FILL` (an unrecognized/absent `behavior` falls back to the
 * `terrain` color). `engine.shape` is the untagged `RegionGeometry` union: a bare
 * `RegionShape` object (the legacy singular wire form — one additive ring) or an array of
 * `{shape, hole}` parts. Parts compose as the
 * union of additive rings minus the hole rings: additive rings fill/stroke as one node
 * (`points` = the first, `extraParts` = the rest), hole rings ride `holes` and are
 * subtracted at draw time by the backend's per-shape inverse mask (the fog sheets' own
 * `setMask({ inverse: true })` mechanism — NOT a winding-reversal, which Pixi's per-subpath
 * tessellation does not honor). One malformed part nulls the WHOLE spec (mirroring the
 * server's whole-region fail-closed parse), as does a compound with no additive part (a
 * holes-only region covers nothing, so nothing renders). Exported so `scene-tools`' hit-test
 * can reuse the SAME tessellation this view draws from, rather than forking a second copy
 * of the shape math.
 * @param doc The `region` document to convert.
 * @returns A `ShapeNodeSpec` for the `regions` layer, or `null` if it can't be rendered.
 * @example
 * ```ts
 * import { regionShapeSpec } from "@shadowcat/render";
 * import type { WireDocument } from "@shadowcat/core";
 *
 * declare const doc: WireDocument;
 * const spec = regionShapeSpec(doc); // null if doc.engine.shape is absent or malformed
 * ```
 */
export function regionShapeSpec(doc: WireDocument): ShapeNodeSpec | null {
  const s = doc.engine as RegionEngine | undefined;
  const geometry = s?.shape;
  if (!geometry) return null;
  // Elements typed nullable: the optimistic store can hold unvalidated engine data
  // pre-rejection, so a `null` array element must fail the whole spec closed rather than
  // throw inside the reconciler or the hit-test — the same "one malformed part drops the
  // whole spec" rule as any other structural defect.
  const parts: (CompoundShapePart | null)[] = Array.isArray(geometry)
    ? geometry
    : [{ shape: geometry, hole: false }];
  const additive: number[][] = [];
  const holes: number[][] = [];
  for (const part of parts) {
    const outline = regionShapeOutline(part?.shape);
    if (!outline) return null;
    if (part?.hole === true) holes.push(outline);
    else additive.push(outline);
  }
  if (additive.length === 0) return null;

  const color = BEHAVIOR_FILL[s?.behavior ?? ""] ?? BEHAVIOR_FILL.terrain;
  // Each blocked sense owns one visual channel so a GM reads them independently: sight swaps the
  // fill for a pale haze, light darkens and thickens the rim, sound dashes the ring.
  const occ = s?.occludes;
  return {
    layer: "regions",
    points: additive[0],
    extraParts: additive.length > 1 ? additive.slice(1) : undefined,
    holes: holes.length > 0 ? holes : undefined,
    closed: true,
    stroke: {
      color: occ?.light ? LIGHT_RIM_COLOR : color,
      width: occ?.light ? LIGHT_RIM_WIDTH : STROKE_WIDTH,
      ...(occ?.sound ? { dash: SOUND_DASH } : {}),
    },
    fill: occ?.sight ? { color: SIGHT_HAZE_COLOR, alpha: SIGHT_HAZE_ALPHA } : { color, alpha: FILL_ALPHA },
  };
}
