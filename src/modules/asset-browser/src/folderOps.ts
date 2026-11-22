// Pure helpers over `asset_folder` documents: tree queries and the
// folder-doc builder. The server remains authoritative for every rule
// these mirror (placement, cycles, authz). The re-parenting op builder
// lives in `@shadowcat/core`'s `buildMoveOp` — shared with every other
// document tree that has a `parent_id` band.

import { appendSortKey, envelope, type SortSibling, type WireDocument } from "@shadowcat/core";

/** The `asset_folder` doc_type string. */
export const ASSET_FOLDER_DOC_TYPE = "asset_folder";

/** The drop-target id of the "All assets" row: an asset dropped on it leaves its folder. Never
 * collides with a folder id, which is a UUID. */
export const ROOT_DROP_ID = "all-assets";

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
 * @param nameCompare - The name tie-break comparator. Defaults to plain `String.localeCompare`
 * so this module stays independently testable with no `@shadowcat/ui-kit` dependency; the
 * production caller (`FolderTree.svelte`) passes ui-kit's locale-reactive `compare` — the ONE
 * collator every name sort in the client reads — so this pure module never constructs its own
 * `Intl.Collator`.
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
export function folderChildren(
  docs: WireDocument[],
  parentId: string | null,
  nameCompare: (a: string, b: string) => number = (a, b) => a.localeCompare(b),
): WireDocument[] {
  return docs
    .filter((d) => d.doc_type === ASSET_FOLDER_DOC_TYPE && d.parent_id === parentId)
    .sort((a, b) => sortOf(a) - sortOf(b) || nameCompare(a.name ?? "", b.name ?? ""));
}

/**
 * The folders directly under `parentId` as id + `sort` pairs in display order — the sibling list
 * `buildReorderOps` reorders.
 * @param docs - Candidate documents (non-folders are ignored).
 * @param parentId - The parent folder id, or `null` for the root.
 * @param nameCompare - The name tie-break comparator, as `folderChildren`.
 * @returns The ordered siblings.
 * @example
 * ```ts
 * import { folderSiblings } from "@shadowcat/module-asset-browser";
 * import type { WireDocument } from "@shadowcat/core";
 *
 * declare const docs: WireDocument[];
 * const siblings = folderSiblings(docs, null);
 * void siblings;
 * ```
 */
export function folderSiblings(
  docs: WireDocument[],
  parentId: string | null,
  nameCompare?: (a: string, b: string) => number,
): SortSibling[] {
  return folderChildren(docs, parentId, nameCompare).map((d) => ({ id: d.id, sort: sortOf(d) }));
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
 * The `sort` key a new folder appended under `parentId` should take — one density step past
 * the highest existing sibling's key, or `SORT_KEY_DENSITY` for the first child.
 * @param docs - Candidate documents (non-folders are ignored).
 * @param parentId - The parent folder id, or `null` for the root.
 * @returns The allocated append key.
 * @example
 * ```ts
 * import { appendFolderSortKey } from "@shadowcat/module-asset-browser";
 * import type { WireDocument } from "@shadowcat/core";
 *
 * declare const docs: WireDocument[];
 * appendFolderSortKey(docs, null);
 * ```
 */
export function appendFolderSortKey(docs: WireDocument[], parentId: string | null): number {
  const siblings = folderChildren(docs, parentId);
  return appendSortKey(siblings.map(sortOf));
}

/**
 * A fresh `asset_folder` document envelope for a Create op.
 * @param world - The owning world id.
 * @param name - The folder's display name.
 * @param parentId - The parent folder, or `null` for the root.
 * @param sort - The new folder's ordering key among its siblings; defaults to `0` for a caller
 * that has no sibling list to allocate an append key from (see `appendFolderSortKey`).
 * @returns The document for `dispatchIntent`'s Create.
 * @example
 * ```ts
 * import { buildFolderDoc } from "@shadowcat/module-asset-browser";
 *
 * const doc = buildFolderDoc("w1", "maps", null);
 * doc.doc_type; // "asset_folder"
 * ```
 */
export function buildFolderDoc(world: string, name: string, parentId: string | null, sort = 0): WireDocument {
  return envelope(world, ASSET_FOLDER_DOC_TYPE, parentId, {}, undefined, { sort }, name);
}
