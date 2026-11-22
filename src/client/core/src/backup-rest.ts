import type { BackupManifest } from "@shadowcat/types";
import { restError } from "./user-rest";

// Admin backup REST, beside `trash-rest`: `POST /api/admin/backup` is gated on ServerRole::Admin
// (`http::backups::admin_backup`). Nothing here is a permission check; a non-admin caller gets a
// 403 the UI surfaces as an error.

/** Takes a manual whole-server backup and waits for it to finish. Admin-only.
 *
 * The generated `BackupManifest` types its `u64` fields as `bigint`, but the JSON carries plain
 * numbers (no `BigInt` revival happens on this path), so a caller reads `created_at_unix_ms`,
 * `asset_file_count` and `db_bytes` through `Number(...)`, which accepts both shapes.
 * @returns The manifest of the backup just written.
 * @example
 * ```ts
 * import { adminBackup } from "@shadowcat/core";
 *
 * const manifest = await adminBackup();
 * const bytes = Number(manifest.db_bytes);
 * ```
 */
export async function adminBackup(): Promise<BackupManifest> {
  const res = await fetch("/api/admin/backup", { method: "POST" });
  if (!res.ok) throw new Error(await restError(res, "backup failed"));
  return (await res.json()) as BackupManifest;
}
