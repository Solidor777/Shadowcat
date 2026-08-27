//! Per-world export/import row DTOs — the on-disk-bundle-portable shape of the
//! five FK-scoped tables `SqliteRepository::delete_world` already walks (read
//! instead of deleted), the FK-less `explored_fog` table it purges via an
//! explicit `DELETE`, plus the world's five keyed `settings` rows. A
//! `users(id)` reference is exported as a portable username string, never a
//! raw id (source and target servers do not share a `users` table) — resolved
//! back to a target-local id (or degraded to `NULL`/row-drop when
//! unresolved) only at import time, in `SqliteRepository::import_world`.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::data::document::{Document, WorldRole};

/// Bundle format version. Bumped whenever `BundleManifest`'s or a row type's
/// shape changes in a way `world_bundle::read_bundle` cannot tolerate;
/// import refuses cleanly on a mismatch rather than attempting a migration —
/// no data-migration machinery exists pre-customers.
pub const BUNDLE_SCHEMA_VERSION: u32 = 1;

/// `manifest.json`'s root: the exported world's identity/watermark plus
/// per-table row counts `world_bundle::read_bundle` cross-checks against what
/// it actually extracted before any row reaches a transaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BundleManifest {
    /// Bundle format version; import refuses cleanly on a mismatch.
    pub schema_version: u32,
    /// The exported world's id, preserved verbatim into `worlds.id` on
    /// import — never remapped.
    pub world_id: Uuid,
    /// The exported world's display name.
    pub world_name: String,
    /// The exported world's monotonic event-sequence watermark
    /// (`worlds.seq`). Preserved verbatim: the bundle's `world_events` rows
    /// carry `seq` values up to this watermark, and a reset-to-zero import
    /// would let the next live command in the imported world collide with an
    /// already-imported historical event's sequence number.
    pub world_seq: i64,
    /// The exported world's `created_at` (Unix epoch milliseconds).
    pub world_created_at: i64,
    /// The exported world's `updated_at` (Unix epoch milliseconds).
    pub world_updated_at: i64,
    /// Export time, Unix epoch milliseconds.
    pub exported_at_unix_ms: i64,
    /// Row count per exported table, keyed by the same table-name strings
    /// used for each `rows/<table>.jsonl` bundle entry ("documents",
    /// "world_events", "world_members", "world_invites", "assets",
    /// "explored_fog", "settings").
    pub row_counts: BTreeMap<String, usize>,
}

/// One exported `documents` row. `document.owner` is ALWAYS `None` in the
/// exported copy — the source owner (if any) is carried instead as
/// `owner_username`, a portable string a target server resolves
/// independently (see `SqliteRepository::import_world`). `seq`/`created_seq`
/// are the DB-only columns `Document` itself does not carry; both must be
/// preserved independently (unlike the live write path's `upsert_document`,
/// where a fresh Create always sets `seq == created_seq`), or a later
/// `created_seq_at_commit` redaction-generation check would see the wrong
/// generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedDocumentRow {
    /// The document envelope, with `owner` nulled (see `owner_username`).
    pub document: Document,
    /// The source server's owner's username, or `None` if the document had
    /// no owner. Resolved to a target-local id (or `NULL` if unresolvable)
    /// at import time, in the same lockstep the column and the JSON body's
    /// `owner` field are always kept in.
    pub owner_username: Option<String>,
    /// `documents.seq` at export time.
    pub seq: i64,
    /// `documents.created_seq` at export time.
    pub created_seq: i64,
}

/// One exported `world_events` row. `command_json` is carried byte-for-byte —
/// a historical audit/replay payload, never rewritten.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedEventRow {
    /// `world_events.seq`.
    pub seq: i64,
    /// The source server's author's username, or `None` for a system/
    /// already-unattributed event.
    pub author_username: Option<String>,
    /// `world_events.ts`.
    pub ts: i64,
    /// `world_events.command_json`, verbatim.
    pub command_json: String,
}

/// One exported `world_members` row. `world_members.user_id` is `NOT NULL`
/// (`ON DELETE CASCADE`, not `SET NULL`), so `username` here is a plain
/// `String`, not the `Option<String>` shape used by columns that CAN degrade
/// to `NULL` on the target (`owner_username`, `author_username`,
/// `created_by_username`, `consumed_by_username`): an unresolvable username
/// here has no `NULL` to degrade to, so `SqliteRepository::import_world`
/// drops the row entirely rather than seat a membership for nobody — the
/// same row-drop behavior `ExportedFogRow.username` uses for the identical
/// reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedMemberRow {
    /// The member's username.
    pub username: String,
    /// The member's role in the exported world.
    pub role: WorldRole,
}

/// One exported `world_invites` row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedInviteRow {
    /// Invite id, preserved verbatim (selector half of the invite code).
    pub id: Uuid,
    /// PHC hash over the code's verifier half; the code itself was never
    /// stored.
    pub secret_hash: String,
    /// Role granted on redemption.
    pub role: WorldRole,
    /// The minting GM's username, or `None`.
    pub created_by_username: Option<String>,
    /// Mint time, Unix epoch milliseconds.
    pub created_at: i64,
    /// Expiry, Unix epoch milliseconds.
    pub expires_at: i64,
    /// Revocation time, if revoked.
    pub revoked_at: Option<i64>,
    /// Redemption time, if redeemed.
    pub consumed_at: Option<i64>,
    /// The redeeming user's username, or `None`.
    pub consumed_by_username: Option<String>,
}

/// One exported `assets` row. `storage_key` is NOT preserved — import
/// recomputes the standard `"{world_id}/{asset_id}"` scheme at extraction
/// time (the world id is preserved verbatim across export/import).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedAssetRow {
    /// Asset id (stable across rename/replace on the source; preserved
    /// verbatim).
    pub id: Uuid,
    /// Filename as uploaded (display only).
    pub original_name: String,
    /// MIME type recorded at upload.
    pub content_type: String,
    /// Size of the stored bytes.
    pub byte_size: i64,
    /// The uploading user's username, or `None` if the source account was
    /// already deleted at export time.
    pub created_by_username: Option<String>,
    /// Upload time, Unix epoch milliseconds.
    pub created_at: i64,
    /// Replace-count watermark; preserved verbatim into the new row's ETag
    /// basis.
    pub version: i64,
}

/// One exported `explored_fog` row. `user_id` has no FK but is `NOT NULL`; an
/// unresolvable username degrades the same way `world_members` does (row
/// dropped) — `delete_user` already purges a deleted account's
/// `explored_fog` rows outright, so a fog row's user always resolves to a
/// LIVE account on the source server, meaning `username` here is never
/// itself absent at export time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedFogRow {
    /// The scene the explored-cell memory belongs to.
    pub scene_id: Uuid,
    /// The remembering user's username.
    pub username: String,
    /// The serialized explored-cell blob, verbatim.
    pub cells: Vec<u8>,
}

/// One exported per-world `settings` row (keyed via `world_settings_keys`).
/// The key already embeds the world id (e.g. `"world_caps:{world_id}"`);
/// since the world id is preserved verbatim on import, the key is
/// reinserted unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedSettingRow {
    /// The settings key, e.g. `"world_schemas:<world_id>"`.
    pub key: String,
    /// The raw JSON value string stored under `key`.
    pub value: String,
}

/// Every row `SqliteRepository::export_world_rows` reads for one world, ready
/// for `world_bundle::write_bundle` to serialize. Asset BYTES are not
/// included here — the writer streams them directly from
/// `Config::assets_path()` via each row's `id`, so a large world's asset
/// bytes are never buffered twice.
#[derive(Debug, Clone)]
pub struct WorldExportData {
    /// The manifest, with `row_counts` already filled from the vectors
    /// below.
    pub manifest: BundleManifest,
    /// Exported `documents` rows.
    pub documents: Vec<ExportedDocumentRow>,
    /// Exported `world_events` rows.
    pub events: Vec<ExportedEventRow>,
    /// Exported `world_members` rows.
    pub members: Vec<ExportedMemberRow>,
    /// Exported `world_invites` rows.
    pub invites: Vec<ExportedInviteRow>,
    /// Exported `assets` rows (metadata only; bytes are streamed
    /// separately).
    pub assets: Vec<ExportedAssetRow>,
    /// Exported `explored_fog` rows.
    pub fog: Vec<ExportedFogRow>,
    /// Exported per-world `settings` rows.
    pub settings: Vec<ExportedSettingRow>,
}

/// Every row `world_bundle::read_bundle` extracted from an uploaded `.tar`,
/// ready for `SqliteRepository::import_world` to insert in one transaction.
#[derive(Debug, Clone)]
pub struct WorldImportData {
    /// The bundle's manifest (already schema-version- and row-count-checked).
    pub manifest: BundleManifest,
    /// Extracted `documents` rows.
    pub documents: Vec<ExportedDocumentRow>,
    /// Extracted `world_events` rows.
    pub events: Vec<ExportedEventRow>,
    /// Extracted `world_members` rows.
    pub members: Vec<ExportedMemberRow>,
    /// Extracted `world_invites` rows.
    pub invites: Vec<ExportedInviteRow>,
    /// Extracted `assets` rows (metadata; bytes are staged on disk, see
    /// `staged_assets`).
    pub assets: Vec<ExportedAssetRow>,
    /// Extracted `explored_fog` rows.
    pub fog: Vec<ExportedFogRow>,
    /// Extracted per-world `settings` rows.
    pub settings: Vec<ExportedSettingRow>,
    /// One `(asset_id, staged_tmp_path)` pair per extracted `assets/<id>` tar
    /// entry — bytes already on disk, in the SAME directory their final path
    /// will live in (so `SqliteRepository::import_world`'s finalize step is
    /// a same-filesystem rename), named `"<asset_id>.<random>.import-tmp"`.
    /// `import_world` renames each into place after every DB row has been
    /// accepted by the transaction but BEFORE that transaction commits — the
    /// safer of the two orders, since it guarantees a committed/visible
    /// world's `assets` rows never reference a not-yet-finalized file; a
    /// rename failure at that point still aborts and rolls back the whole
    /// transaction.
    pub staged_assets: Vec<(Uuid, PathBuf)>,
}

/// The outcome of a successful `SqliteRepository::import_world` call. Rows
/// dropped because their `users(id)` reference did not resolve on the target
/// (`world_members`/`explored_fog` — the two `NOT NULL` user columns with no
/// `SET NULL` degradation, see `ExportedMemberRow`/`ExportedFogRow`) are
/// counted rather than silently absorbed, so the triggering admin can see
/// exactly what was dropped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSummary {
    /// The imported world's id (== the bundle's `manifest.world_id`).
    pub world_id: Uuid,
    /// `world_members` rows dropped because their username did not resolve.
    pub skipped_members: usize,
    /// `explored_fog` rows dropped because their username did not resolve.
    pub skipped_fog: usize,
}

#[cfg(test)]
mod tests;
