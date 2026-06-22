import type { ReadableDocuments, WireDocument } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { ShapeNodeSpec } from "./types";
import { parseColor, circlePoints, conePoints, squarePoints } from "./geometry";

/** Client-owned `template.system` (M8 §9): an area anchored at `(x,y)` with a `size`
 * and `direction` (degrees), tessellated per `kind`. */
interface TemplateSystem {
  shape: { kind: string; x: number; y: number; size: number; direction: number };
  color: string;
}

/** Templates render as translucent filled areas. */
const FILL_ALPHA = 0.25;
const STROKE_WIDTH = 2;

/** Reconciles `doc_type:"template"` documents into the `templates` layer. */
export class TemplateView {
  private readonly ids = new Set<string>();

  constructor(
    private readonly store: ReadableDocuments,
    private readonly backend: DisplayBackend,
  ) {}

  reconcile(): void {
    const seen = new Set<string>();
    for (const doc of this.store.query("template")) {
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
  const s = doc.system as TemplateSystem | undefined;
  if (!s?.shape) return null;
  const { kind, x, y, size, direction } = s.shape;
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
