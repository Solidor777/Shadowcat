// Pure helpers over `asset_folder` documents: tree queries, the Move-op
// builder, and the folder-doc builder. The server remains authoritative for
// every rule these mirror (placement, cycles, authz).

import { envelope, type WireDocument, type WireOperation } from "@shadowcat/core";

/** The `asset_folder` doc_type string. */
export const ASSET_FOLDER_DOC_TYPE = "asset_folder";

/** The slice of the `asset_folder` engine band read client-side. */
interface FolderEngineShape {
  /** GM-set ordering key. */
  sort?: unknown;
}

/** A folder's `engine.sort`, or 0 when absent/malformed.
 * @param doc - The folder document.
 * @returns The numeric sort key.
 * @example
 * ```
 * // private helper; consumed by folderChildren's comparator below
 * declare const doc: WireDocument;
 * sortOf(doc);
 * ```
 */
function sortOf(doc: WireDocument): number {
  const sort = (doc.engine as FolderEngineShape | null | undefined)?.sort;
  return typeof sort === "number" ? sort : 0;
}

/**
 * The folder documents directly under `parentId` (`null` = world root),
 * ordered by `engine.sort`, then name.
 * @param docs - Candidate documents (non-folders are ignored).
 * @param parentId - The parent folder id, or `null` for the root.
 * @returns The ordered children.
 * @example
 * ```ts
 * import { folderChildren } from "@shadowcat/module-asset-browser";
 * import type { WireDocument } from "@shadowcat/core";
 *
 * declare const docs: WireDocument[];
 * const roots = folderChildren(docs, null);
 * void roots;
 * ```
 */
export function folderChildren(docs: WireDocument[], parentId: string | null): WireDocument[] {
  return docs
    .filter((d) => d.doc_type === ASSET_FOLDER_DOC_TYPE && d.parent_id === parentId)
    .sort((a, b) => sortOf(a) - sortOf(b) || (a.name ?? "").localeCompare(b.name ?? ""));
}

/**
 * The folder's ancestor-chain display names, root first, ending with its own.
 * @param docs - The folder documents to resolve through.
 * @param id - The folder to name.
 * @returns The path names; a dangling ancestor truncates the walk.
 * @example
 * ```ts
 * import { folderPathNames } from "@shadowcat/module-asset-browser";
 * import type { WireDocument } from "@shadowcat/core";
 *
 * declare const docs: WireDocument[];
 * const path = folderPathNames(docs, "some-folder-id");
 * void path;
 * ```
 */
export function folderPathNames(docs: WireDocument[], id: string): string[] {
  const byId = new Map(docs.map((d) => [d.id, d]));
  const names: string[] = [];
  let cursor: string | null = id;
  while (cursor) {
    const doc = byId.get(cursor);
    if (!doc) break;
    names.unshift(doc.name ?? "");
    cursor = doc.parent_id;
  }
  return names;
}

/**
 * The Move operation re-parenting `docId`, carrying the TRUE current parent
 * as the OCC pre-image.
 * @param docId - The document to move.
 * @param targetParentId - The new parent (`null` = top level).
 * @param currentParentId - The document's current parent (the pre-image).
 * @returns The wire operation for `dispatchIntent`.
 * @example
 * ```ts
 * import { buildMoveOp } from "@shadowcat/module-asset-browser";
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

/**
 * Whether `id` is `rootId` itself or sits anywhere in its subtree — the drag
 * guard that refuses dropping a folder into itself or its own descendants
 * (advisory; the server's cycle walk is authoritative).
 * @param docs - The folder documents to resolve through.
 * @param rootId - The candidate ancestor.
 * @param id - The node being tested.
 * @returns Whether `id` is inside `rootId`'s subtree (self included).
 * @example
 * ```ts
 * import { isDescendantOrSelf } from "@shadowcat/module-asset-browser";
 * import type { WireDocument } from "@shadowcat/core";
 *
 * declare const docs: WireDocument[];
 * isDescendantOrSelf(docs, "a", "a"); // true
 * ```
 */
export function isDescendantOrSelf(docs: WireDocument[], rootId: string, id: string): boolean {
  const byId = new Map(docs.map((d) => [d.id, d]));
  let cursor: string | null = id;
  while (cursor) {
    if (cursor === rootId) return true;
    cursor = byId.get(cursor)?.parent_id ?? null;
  }
  return false;
}

/**
 * A fresh `asset_folder` document envelope for a Create op.
 * @param world - The owning world id.
 * @param name - The folder's display name.
 * @param parentId - The parent folder, or `null` for the root.
 * @returns The document for `dispatchIntent`'s Create.
 * @example
 * ```ts
 * import { buildFolderDoc } from "@shadowcat/module-asset-browser";
 *
 * const doc = buildFolderDoc("w1", "maps", null);
 * doc.doc_type; // "asset_folder"
 * ```
 */
export function buildFolderDoc(world: string, name: string, parentId: string | null): WireDocument {
  return envelope(world, ASSET_FOLDER_DOC_TYPE, parentId, {}, undefined, { sort: 0 }, name);
}
