// Pure helpers over a folder document type (`asset_folder`, `table_folder`, ...): tree queries and
// the folder-doc builder, parametrized by the folder `doc_type`. The server stays authoritative for
// every rule these mirror (placement, cycles, authz). `DocFolderTree.svelte` and each folder
// browser's own op module read these, so every folder tree sorts, nests and appends identically.

import { appendSortKey, envelope, type SortSibling, type WireDocument } from "@shadowcat/core";

/** The i18n keys a `DocFolderTree` renders, one set per folder doc type. */
export interface FolderTreeLabels {
  /** The row that clears the folder filter. */
  all: string;
  /** The create-folder input's placeholder. */
  newFolder: string;
  /** The drag handle's accessible name. */
  dragFolder: string;
  /** The rename button's title and the context menu's Rename item. */
  rename: string;
  /** The move button's title and the context menu's Move item. */
  move: string;
  /** The delete button's title, the context menu's Delete item and the dialog's accessible name. */
  delete: string;
  /** The Move-to picker's root entry. */
  moveToRoot: string;
  /** The delete dialog's prompt; receives `{name}`. */
  deletePrompt: string;
  /** The delete dialog's dismiss button. */
  cancel: string;
  /** The notification shown when a delete fails; receives `{message}`. */
  error: string;
}

/** One button of a folder's delete dialog. */
export interface FolderDeleteChoice {
  /** The choice id passed to `onDelete`; also the button's test id suffix (`folder-delete-<id>`). */
  id: string;
  /** The button's i18n key. */
  labelKey: string;
  /** Whether the button renders in the danger color. */
  danger?: boolean;
}

/** The slice of a folder's engine band read client-side. */
interface FolderEngineShape {
  /** Sibling ordering key. */
  sort?: unknown;
}

/** A folder's `engine.sort`, or 0 when absent/malformed.
 * @param doc - The folder document.
 * @returns The numeric sort key.
 * @example
 * ```
 * // private helper; consumed by typedFolderChildren's comparator below
 * declare const doc: WireDocument;
 * sortOf(doc);
 * ```
 */
function sortOf(doc: WireDocument): number {
  const sort = (doc.engine as FolderEngineShape | null | undefined)?.sort;
  return typeof sort === "number" ? sort : 0;
}

/**
 * The folder documents of `folderDocType` directly under `parentId` (`null` = world root), ordered
 * by `engine.sort`, then name.
 * @param docs - Candidate documents (other doc types are ignored).
 * @param folderDocType - The folder `doc_type`.
 * @param parentId - The parent folder id, or `null` for the root.
 * @param nameCompare - The name tie-break comparator. Defaults to plain `String.localeCompare`;
 * a production caller passes ui-kit's locale-reactive `compare`, the one collator every name sort
 * in the client reads.
 * @returns The ordered children.
 * @example
 * ```ts
 * import { typedFolderChildren } from "@shadowcat/ui-kit";
 * import type { WireDocument } from "@shadowcat/core";
 *
 * declare const docs: WireDocument[];
 * typedFolderChildren(docs, "table_folder", null);
 * ```
 */
export function typedFolderChildren(
  docs: WireDocument[],
  folderDocType: string,
  parentId: string | null,
  nameCompare: (a: string, b: string) => number = (a, b) => a.localeCompare(b),
): WireDocument[] {
  return docs
    .filter((d) => d.doc_type === folderDocType && d.parent_id === parentId)
    .sort((a, b) => sortOf(a) - sortOf(b) || nameCompare(a.name ?? "", b.name ?? ""));
}

/**
 * The folders of `folderDocType` directly under `parentId` as id + `sort` pairs in display order:
 * the sibling list `buildReorderOps` reorders.
 * @param docs - Candidate documents (other doc types are ignored).
 * @param folderDocType - The folder `doc_type`.
 * @param parentId - The parent folder id, or `null` for the root.
 * @param nameCompare - The name tie-break comparator, as `typedFolderChildren`.
 * @returns The ordered siblings.
 * @example
 * ```ts
 * import { typedFolderSiblings } from "@shadowcat/ui-kit";
 * import type { WireDocument } from "@shadowcat/core";
 *
 * declare const docs: WireDocument[];
 * typedFolderSiblings(docs, "table_folder", null);
 * ```
 */
export function typedFolderSiblings(
  docs: WireDocument[],
  folderDocType: string,
  parentId: string | null,
  nameCompare?: (a: string, b: string) => number,
): SortSibling[] {
  return typedFolderChildren(docs, folderDocType, parentId, nameCompare).map((d) => ({ id: d.id, sort: sortOf(d) }));
}

/**
 * Whether `id` is `rootId` itself or sits anywhere in its subtree: the drag guard that refuses
 * dropping a folder into itself or its own descendants (advisory; the server's cycle walk is
 * authoritative).
 * @param docs - The folder documents to resolve through.
 * @param rootId - The candidate ancestor.
 * @param id - The node being tested.
 * @returns Whether `id` is inside `rootId`'s subtree (self included).
 * @example
 * ```ts
 * import { isDescendantOrSelf } from "@shadowcat/ui-kit";
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
 * The `sort` key a new folder appended under `parentId` should take: one density step past the
 * highest existing sibling's key, or `SORT_KEY_DENSITY` for the first child.
 * @param docs - Candidate documents (other doc types are ignored).
 * @param folderDocType - The folder `doc_type`.
 * @param parentId - The parent folder id, or `null` for the root.
 * @returns The allocated append key.
 * @example
 * ```ts
 * import { appendTypedFolderSortKey } from "@shadowcat/ui-kit";
 * import type { WireDocument } from "@shadowcat/core";
 *
 * declare const docs: WireDocument[];
 * appendTypedFolderSortKey(docs, "table_folder", null);
 * ```
 */
export function appendTypedFolderSortKey(docs: WireDocument[], folderDocType: string, parentId: string | null): number {
  return appendSortKey(typedFolderChildren(docs, folderDocType, parentId).map(sortOf));
}

/**
 * A fresh folder document envelope of `folderDocType` for a Create op.
 * @param world - The owning world id.
 * @param folderDocType - The folder `doc_type`.
 * @param name - The folder's display name.
 * @param parentId - The parent folder, or `null` for the root.
 * @param sort - The new folder's ordering key among its siblings; `0` for a caller with no sibling
 * list to allocate an append key from.
 * @returns The document for `dispatchIntent`'s Create.
 * @example
 * ```ts
 * import { buildTypedFolderDoc } from "@shadowcat/ui-kit";
 *
 * const doc = buildTypedFolderDoc("w1", "table_folder", "Loot", null);
 * doc.doc_type; // "table_folder"
 * ```
 */
export function buildTypedFolderDoc(
  world: string,
  folderDocType: string,
  name: string,
  parentId: string | null,
  sort = 0,
): WireDocument {
  return envelope(world, folderDocType, parentId, {}, undefined, { sort }, name);
}
