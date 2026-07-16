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
  private readonly ids = new Set<string>();

  constructor(
    private readonly store: ReadableDocuments,
    private readonly backend: DisplayBackend,
    private readonly viewedSceneId: () => string | null = () => null,
  ) {}

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

function toSpec(doc: WireDocument): ShapeNodeSpec | null {
  const s = doc.engine as WallEngine | undefined;
  if (!s?.seg) return null;
  const { x1, y1, x2, y2 } = s.seg;
  // The opaque `system` is server-structural-only, so guard the coords (a malformed
  // wall just doesn't render rather than pushing NaN into the geometry).
  if (![x1, y1, x2, y2].every((n) => Number.isFinite(n))) return null;
  return {
    layer: "walls",
    points: [x1, y1, x2, y2],
    closed: false,
    stroke: { color: WALL_COLOR, width: WALL_WIDTH },
    fill: null,
  };
}
