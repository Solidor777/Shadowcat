import type { ReadableDocuments, WireDocument } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { ShapeNodeSpec } from "./types";
import { rectPoints, circlePoints } from "./geometry";

/** Client-owned `region.system` (M10g spec §3): a vector shape + gameplay behavior. The server
 * also reads this (structural-only, #6 exception) to build its `RegionField`. */
interface RegionSystemLike {
  shape?: { kind?: string; points?: number[] };
  behavior?: string;
}

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

  constructor(
    private readonly store: ReadableDocuments,
    private readonly backend: DisplayBackend,
  ) {}

  reconcile(): void {
    const seen = new Set<string>();
    for (const doc of this.store.query("region")) {
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

function toSpec(doc: WireDocument): ShapeNodeSpec | null {
  const s = doc.system as RegionSystemLike | undefined;
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
