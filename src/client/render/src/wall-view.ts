import type { ReadableDocuments, WireDocument } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { ShapeNodeSpec } from "./types";

/** Client-owned `wall.system` (M9 §4): a segment + sight/movement flags. The server
 * also reads `seg` + `blocksMove` for its authoritative collision check (#6 exception). */
interface WallSystem {
  seg: { x1: number; y1: number; x2: number; y2: number };
  blocksSight?: boolean;
  blocksMove?: boolean;
}

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
  ) {}

  reconcile(): void {
    const seen = new Set<string>();
    for (const doc of this.store.query("wall")) {
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
  const s = doc.system as WallSystem | undefined;
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
