import type { ReadableDocuments, WireDocument } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { ShapeNodeSpec } from "./types";
import { parseColor, rectPoints, ellipsePoints } from "./geometry";

/** Client-owned `drawing.system` (M8 §9). `points` are path vertices for
 * freehand/line/polygon, or bbox corners `[x0,y0,x1,y1]` for rect/ellipse. */
interface DrawingSystem {
  shape: { kind: string; points: number[] };
  stroke: { color: string; width: number } | null;
  fill: { color: string; alpha?: number } | null;
}

/** Reconciles `doc_type:"drawing"` documents into the `drawings` layer as shape nodes. */
export class DrawingView {
  private readonly ids = new Set<string>();

  constructor(
    private readonly store: ReadableDocuments,
    private readonly backend: DisplayBackend,
  ) {}

  reconcile(): void {
    const seen = new Set<string>();
    for (const doc of this.store.query("drawing")) {
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

function toSpec(doc: WireDocument): ShapeNodeSpec | null {
  const s = doc.system as DrawingSystem | undefined;
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
  return {
    layer: "drawings",
    points: pts,
    closed,
    stroke: s.stroke ? { color: parseColor(s.stroke.color), width: s.stroke.width } : null,
    fill: s.fill ? { color: parseColor(s.fill.color), alpha: s.fill.alpha ?? 1 } : null,
  };
}
