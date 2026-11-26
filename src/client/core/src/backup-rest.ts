import type {
  BackupEntry,
  BackupListing,
  BackupManifest,
  PruneResult,
  RestoreFailed,
  RestoreStaged,
} from "@shadowcat/types";
import { restError } from "./user-rest";
import { apiPath } from "./base-path";

// Admin backup REST, beside `trash-rest`: every `/api/admin/backup*` route is gated on
// ServerRole::Admin (`http::backups`). Nothing here is a permission check; a non-admin caller gets
// a 403 the UI surfaces as an error.
//
// The generated types declare every Rust `u64` as `bigint`, but the JSON carries plain numbers (no
// `BigInt` revival happens on this path), so each response is converted to the `*View` shapes
// below at this boundary and callers never see a `bigint`.

/** A `BackupManifest` whose `u64` fields are plain numbers, as the JSON delivers them. */
export type BackupManifestView = Omit<
  BackupManifest,
  "created_at_unix_ms" | "asset_file_count" | "db_bytes"
> & {
  /** Backup creation time, Unix epoch milliseconds. */
  created_at_unix_ms: number;
  /** Files copied into the backup's assets tree. */
  asset_file_count: number;
  /** Byte size of the database snapshot. */
  db_bytes: number;
};

/** A `BackupEntry` whose `u64` fields are plain numbers. */
export type BackupEntryView = Omit<BackupEntry, "manifest" | "total_bytes"> & {
  /** The parsed `manifest.json`. */
  manifest: BackupManifestView;
  /** Sum of every file's size under the backup directory. */
  total_bytes: number;
};

/** A `RestoreFailed` whose `u64` field is a plain number. */
export type RestoreFailedView = Omit<RestoreFailed, "requested_at_ms"> & {
  /** When the restore was requested, Unix epoch milliseconds; `0` when unreadable. */
  requested_at_ms: number;
};

/** A `BackupListing` whose `u64` fields are plain numbers. */
export type BackupListingView = Omit<BackupListing, "entries" | "restore_failed"> & {
  /** Every valid backup, newest first. */
  entries: BackupEntryView[];
  /** Why the last staged restore was not applied at startup, or `null`. */
  restore_failed: RestoreFailedView | null;
};

/**
 * Converts a manifest's `u64` fields from the declared `bigint` to the number the JSON carries.
 * @param m - The manifest as parsed from JSON.
 * @returns The same manifest with numeric `created_at_unix_ms`, `asset_file_count`, `db_bytes`.
 * @example
 * ```ts
 * // private function; not part of the public API — applied by every backup REST wrapper
 * declare const raw: import("@shadowcat/types").BackupManifest;
 * const view = toManifestView(raw);
 * ```
 */
function toManifestView(m: BackupManifest): BackupManifestView {
  return {
    ...m,
    created_at_unix_ms: Number(m.created_at_unix_ms),
    asset_file_count: Number(m.asset_file_count),
    db_bytes: Number(m.db_bytes),
  };
}

/** Takes a manual whole-server backup and waits for it to finish. Admin-only.
 * @returns The manifest of the backup just written, with numeric `u64` fields.
 * @example
 * ```ts
 * import { adminBackup } from "@shadowcat/core";
 *
 * const manifest = await adminBackup();
 * const bytes = manifest.db_bytes;
 * ```
 */
export async function adminBackup(): Promise<BackupManifestView> {
  const res = await fetch(apiPath("/api/admin/backup"), { method: "POST" });
  if (!res.ok) throw new Error(await restError(res, "backup failed"));
  return toManifestView((await res.json()) as BackupManifest);
}

/** Lists every backup under the server's backups root, newest first, with the retention policy.
 * Admin-only.
 * @returns The listing, with numeric `u64` fields.
 * @example
 * ```ts
 * import { listBackups } from "@shadowcat/core";
 *
 * const { entries, policy, root, restore_failed } = await listBackups();
 * ```
 */
export async function listBackups(): Promise<BackupListingView> {
  const res = await fetch(apiPath("/api/admin/backups"), { headers: { accept: "application/json" } });
  if (!res.ok) throw new Error(await restError(res, "list backups failed"));
  const raw = (await res.json()) as BackupListing;
  return {
    ...raw,
    entries: raw.entries.map((e) => ({
      ...e,
      manifest: toManifestView(e.manifest),
      total_bytes: Number(e.total_bytes),
    })),
    restore_failed: raw.restore_failed
      ? { ...raw.restore_failed, requested_at_ms: Number(raw.restore_failed.requested_at_ms) }
      : null,
  };
}

/** Moves one backup into the server's trash. Admin-only.
 * @param id - The backup id from `listBackups` (its directory name).
 * @example
 * ```ts
 * import { deleteBackup } from "@shadowcat/core";
 *
 * await deleteBackup("backup-1789000000000");
 * ```
 */
export async function deleteBackup(id: string): Promise<void> {
  const res = await fetch(apiPath(`/api/admin/backups/${encodeURIComponent(id)}`), { method: "DELETE" });
  if (!res.ok) throw new Error(await restError(res, "delete backup failed"));
}

/** Stages a restore of one backup and stops the server. The server takes a `pre-restore` backup of
 * its current state first, then applies the restore at its next start; it does not restart itself.
 * Admin-only.
 * @param id - The backup id from `listBackups`.
 * @returns The staged marker and the id of the safety backup.
 * @example
 * ```ts
 * import { restoreBackup } from "@shadowcat/core";
 *
 * const { pre_restore_id } = await restoreBackup("backup-1789000000000");
 * ```
 */
export async function restoreBackup(id: string): Promise<RestoreStaged> {
  const res = await fetch(apiPath(`/api/admin/backups/${encodeURIComponent(id)}/restore`), {
    method: "POST",
  });
  if (!res.ok) throw new Error(await restError(res, "restore backup failed"));
  return (await res.json()) as RestoreStaged;
}

/** Clears the failed-restore marker the server wrote at startup, moving it into the server's
 * trash. Succeeds when no marker exists. Admin-only.
 * @example
 * ```ts
 * import { dismissRestoreFailed } from "@shadowcat/core";
 *
 * await dismissRestoreFailed();
 * ```
 */
export async function dismissRestoreFailed(): Promise<void> {
  const res = await fetch(apiPath("/api/admin/backups/restore-failed"), { method: "DELETE" });
  if (!res.ok) throw new Error(await restError(res, "dismiss failed restore failed"));
}

/** Applies the server's retention policy now, moving the backups it no longer keeps into the
 * trash. Admin-only.
 * @returns The ids that were removed.
 * @example
 * ```ts
 * import { pruneBackups } from "@shadowcat/core";
 *
 * const { removed } = await pruneBackups();
 * ```
 */
export async function pruneBackups(): Promise<PruneResult> {
  const res = await fetch(apiPath("/api/admin/backups/prune"), { method: "POST" });
  if (!res.ok) throw new Error(await restError(res, "prune backups failed"));
  return (await res.json()) as PruneResult;
}
