// Shared re-parenting operation builder for any document tree with a `parent_id` band
// (asset folders, notes, tables, ...). Consumers own their own placement rules; this only
// shapes the wire frame.
import type { WireOperation } from "./wire";

/**
 * The Move operation re-parenting `docId`, carrying the TRUE current parent
 * as the OCC pre-image.
 * @param docId The document to move.
 * @param targetParentId The new parent (`null` = top level).
 * @param currentParentId The document's current parent (the pre-image).
 * @returns The wire operation for `dispatchIntent`.
 * @example
 * ```ts
 * import { buildMoveOp } from "@shadowcat/core";
 *
 * buildMoveOp("doc-1", "folder-2", null);
 * // { op: "move", doc_id: "doc-1", parent_id: "folder-2", old_parent_id: null }
 * ```
 */
export function buildMoveOp(
  docId: string,
  targetParentId: string | null,
  currentParentId: string | null,
): WireOperation {
  return {
    op: "move",
    doc_id: docId,
    parent_id: targetParentId,
    old_parent_id: currentParentId,
  };
}
