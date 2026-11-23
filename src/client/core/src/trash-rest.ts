import { restError } from "./user-rest";
import { apiPath } from "./base-path";

// Admin trash REST, beside `user-rest`: the server's `/api/admin/trash` routes are gated on
// ServerRole::Admin (`http::trash`). Nothing here is a permission check; a non-admin caller gets
// a 403 the UI surfaces as an error.

/** One trashed item as the admin listing reports it. */
export interface TrashEntry {
  /** The entry id, `<trashed_at_ms>-<uuid>`; names the entry in `deleteTrashEntry` and is the
   * `before` cursor of the next `listTrash` page. */
  id: string;
  /** Absolute path the item had when it was discarded; where a manual restore puts it back. */
  original_path: string;
  /** Whether the item is a single file or a directory tree. */
  kind: "file" | "dir";
  /** Unix milliseconds at which the item was discarded. */
  trashed_at_ms: number;
  /** Bytes the item occupies. */
  size_bytes: number;
}

/** Paging options for `listTrash`. */
export interface ListTrashOptions {
  /** Page size; the server clamps it to its own maximum (200), and omitting it asks for that maximum. */
  limit?: number;
  /** The id of the last entry of the previous page; the page returned follows it. */
  before?: string;
}

/** What `emptyTrash` removed. */
export interface EmptyTrashResult {
  /** Number of entries permanently removed. */
  entries: number;
  /** Bytes those entries held. */
  bytes: number;
}

/** One page of the trash, newest first. Admin-only.
 * @param opts - Paging options; omit for the first page.
 * @returns The page of entries.
 * @example
 * ```ts
 * import { listTrash } from "@shadowcat/core";
 *
 * const page = await listTrash({ limit: 50 });
 * ```
 */
export async function listTrash(opts: ListTrashOptions = {}): Promise<TrashEntry[]> {
  const params = new URLSearchParams();
  if (opts.limit !== undefined) params.set("limit", String(opts.limit));
  if (opts.before !== undefined) params.set("before", opts.before);
  const query = params.toString();
  const res = await fetch(apiPath(`/api/admin/trash${query ? `?${query}` : ""}`), {
    headers: { accept: "application/json" },
  });
  if (!res.ok) throw new Error(await restError(res, "list trash failed"));
  return (await res.json()) as TrashEntry[];
}

/** Permanently remove every trash entry. Admin-only.
 * @returns How many entries and bytes were removed.
 * @example
 * ```ts
 * import { emptyTrash } from "@shadowcat/core";
 *
 * const { entries, bytes } = await emptyTrash();
 * ```
 */
export async function emptyTrash(): Promise<EmptyTrashResult> {
  const res = await fetch(apiPath("/api/admin/trash/empty"), { method: "POST" });
  if (!res.ok) throw new Error(await restError(res, "empty trash failed"));
  return (await res.json()) as EmptyTrashResult;
}

/** Thrown by `deleteTrashEntry` when the entry no longer exists (a 404): another admin, or the
 * retention sweep, removed it first. A caller treats it as "already gone", not a failure. */
export class TrashEntryGoneError extends Error {
  /**
   * Builds the error with the server's 404 message.
   * @param message - The server's message for the 404.
   * @example
   * ```ts
   * import { TrashEntryGoneError } from "@shadowcat/core";
   *
   * const err = new TrashEntryGoneError("trash entry not found");
   * ```
   */
  constructor(message: string) {
    super(message);
    this.name = "TrashEntryGoneError";
  }
}

/** Permanently remove one trash entry. Admin-only.
 * @param id - The entry id from `listTrash`.
 * @throws {TrashEntryGoneError} When the entry no longer exists.
 * @example
 * ```ts
 * import { deleteTrashEntry } from "@shadowcat/core";
 *
 * await deleteTrashEntry("1737490000123-3fa85f64-5717-4562-b3fc-2c963f66afa6");
 * ```
 */
export async function deleteTrashEntry(id: string): Promise<void> {
  const res = await fetch(apiPath(`/api/admin/trash/${encodeURIComponent(id)}`), { method: "DELETE" });
  if (res.status === 404) throw new TrashEntryGoneError(await restError(res, "trash entry not found"));
  if (!res.ok) throw new Error(await restError(res, "delete trash entry failed"));
}
