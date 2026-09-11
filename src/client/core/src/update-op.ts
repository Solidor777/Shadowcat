// Shared field-level Update operation builder for any document editor (sheets, combat, scene
// tools, ...). Consumers own their own field paths and OCC pre-images; this only shapes the wire
// frame, mirroring `buildMoveOp`'s role for the `move` operation.
import type { WireOperation } from "./wire";

/**
 * One field's change within a {@link buildUpdate} batch.
 */
export interface FieldEdit {
  /** The field's JSON-pointer path within the document. */
  path: string;
  /** The real current stored value at `path` (OCC pre-image); `undefined` for a genuinely
   * absent field. */
  old?: unknown;
  /** The new value to write. Ignored when {@link remove} is `true`. */
  value?: unknown;
  /** `true` to remove the object key at `path` instead of writing {@link value}, making it
   * GENUINELY ABSENT (`null` != absent). */
  remove?: boolean;
}

/**
 * Builds ONE atomic Update carrying MULTIPLE `FieldChange`s — either all apply or none does. The
 * single builder of the `{op:"update", doc_id, changes}` envelope every field-path edit in the
 * client goes through.
 * INVARIANT (OCC): each edit's `old` must be the RAW current stored value at its `path`; the
 * server's `apply_intent` enforces field-level optimistic concurrency (`actual != change.old` →
 * Conflict), so a hardcoded or defaulted `old` is accepted only once and rejected+rolled-back on
 * every subsequent edit. `old ?? null` collapses ONLY a genuinely absent (`undefined`) pre-image
 * to the wire's `null` — a falsy real value (`0`/`false`/`""`) is preserved verbatim.
 * @param docId - The target document's id.
 * @param edits - The fields to change, all as one Update.
 * @returns The wire operation for `dispatchIntent`.
 * @example
 * ```ts
 * import { buildUpdate } from "@shadowcat/core";
 *
 * buildUpdate("doc-1", [{ path: "/engine/draw", old: undefined, value: "weighted" }]);
 * // { op: "update", doc_id: "doc-1", changes: [{ path: "/engine/draw", old: null, new: "weighted" }] }
 * ```
 */
export function buildUpdate(docId: string, edits: FieldEdit[]): WireOperation {
  return {
    op: "update",
    doc_id: docId,
    changes: edits.map(({ path, old, value, remove }) =>
      remove ? { path, old: old ?? null, new: null, remove: true } : { path, old: old ?? null, new: value },
    ),
  };
}
