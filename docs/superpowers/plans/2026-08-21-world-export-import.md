# Per-World Export/Import — Implementation Plan

**REQUIRED SUB-SKILL: superpowers:subagent-driven-development**

## Goal

Let a world-scoped GM export a single world (every world-scoped row plus the asset bytes its
`assets` rows reference) into one portable `.tar` bundle, and let a server admin import that
bundle into a (possibly different) server to recreate the world — a narrower, world-scoped
sibling of the existing whole-server `backup`/`restore_backup` surface, which stays untouched.

## Architecture

Three-layer split mirroring the existing `backup` module's separation of "row/file data" from
"file format I/O" from "HTTP wiring":

1. **`data::world_bundle`** — pure DTOs: `BundleManifest`, one `Exported*Row` struct per exported
   table, `WorldExportData`/`WorldImportData`, `ImportSummary`. No I/O.
2. **`data::sqlite::SqliteRepository::export_world_rows`/`import_world`** — the DB-facing halves.
   `export_world_rows` reads every row `SqliteRepository::delete_world` already walks (read instead
   of deleted), resolving each `users(id)` reference to a portable username inline (one SQL
   `LEFT JOIN`/`JOIN` per table, no N+1 lookups). `import_world` is one transaction: reject a
   world-id collision before any row is written, insert `worlds` then every table in FK-safe order,
   resolving each row's username(s) back to a target-local id (or `NULL`/row-drop), then finalize
   staged asset files, then commit.
3. **`world_bundle`** (top-level, sibling to `backup`) — pure tar I/O, no `SqliteRepository`/
   `AppState` dependency: `write_bundle` builds the `.tar` from a `WorldExportData` + the assets
   directory; `read_bundle` extracts an uploaded `.tar` into a `WorldImportData`, staging asset
   bytes to disk and refusing (before returning) on a `schema_version` mismatch or a `rows/*.jsonl`
   line count that disagrees with the manifest's own promise.
4. **`http::world_bundle`** — the two routes: `POST /api/worlds/{id}/export` (world-GM-gated,
   streams the tar) and `POST /api/worlds/import` (server-admin-gated, multipart upload streamed to
   a temp file first).

World identity is preserved verbatim (never remapped) per the approved design's §3; a colliding
`world_id` on the target refuses the import cleanly. `world_events.command_json` is imported
byte-for-byte, never rewritten, per §4.

## Tech Stack

Existing crate stack (sqlx/SQLite, axum, tokio, serde, thiserror, uuid) plus one new dependency:
`tar = "0.4"` (synchronous, no_std-friendly tar archive reader/writer; the design's approved
bundle format is an uncompressed `.tar`, so no `flate2`/gzip dependency is needed). Every
`world_bundle` (top-level) call is synchronous/blocking and MUST run inside
`tokio::task::spawn_blocking` from async callers — this module has no async API, the same
constraint `backup::create_backup`'s `VACUUM INTO` statement does not have (that one's the async
`sqlx` driver; the `tar` crate's `Read`/`Write`-based API is genuinely synchronous).

## Spec

`docs/superpowers/specs/2026-08-21-world-export-import-design.md` — approved; every design fork it
resolves (world-id preservation/collision-reject, username-based owner remap reusing the
`delete_user` degradation path, verbatim `world_events.command_json` import, bundle format,
authorization tiers) is FINAL for this plan.

**Spec corrections applied in this plan** (see the final report accompanying this plan for full
detail — these are factual corrections to claims in the design doc, not re-litigation of its
decisions):

- §7's rationale for gating import server-admin-only ("matching how world CREATION is already
  gated... never independently") is factually wrong: `POST /api/worlds` (`http::routes::
  create_world`) is gated by plain `AuthUser` — ANY authenticated user may create a world. The
  DECISION (import is server-admin-only) is kept as approved, on its own independent merits
  (import is a privileged bulk multi-table insert that bypasses every capability/schema/OCC gate
  the live write paths enforce — categorically more privileged than an ordinary `create_world`
  call), but the doc comments this plan writes state that reasoning, not the spec's incorrect one.
- §9's ARCHITECTURE.md fix instruction claims the table row being corrected "originally meant"
  whole-server backup/restore, and that backup/restore "remains" a Phase 2 concern. Neither is
  true: `backup`/`restore_backup` is not mentioned anywhere in `ARCHITECTURE.md` today (confirmed
  by search), and it is already fully shipped, not deferred. The row's own "Seam in place" column
  (`document CRUD`) matches per-world export/import, not backup/restore. This plan's docs task
  removes the row from the "Deferred" table entirely (it is no longer deferred) rather than
  rewriting it to reference a nonexistent still-deferred backup/restore concern.
- §6's manifest shape (`schema_version, world_id, world_name, exported_at, row_counts`) is
  underspecified: it omits the `worlds.seq`/`created_at`/`updated_at` fields needed to
  reconstruct the `worlds` row faithfully. `worlds.seq` is load-bearing — it is the world's
  monotonic event-sequence watermark, and the bundle's `world_events` rows carry `seq` values up
  to it; importing with `seq` reset to 0 would let the next live command in the imported world
  collide with an already-imported historical event's sequence number. This plan extends
  `BundleManifest` with `world_seq`/`world_created_at`/`world_updated_at` fields to close this gap
  (§6 calls `worlds` "the bundle's root record," and the manifest is the natural single-JSON-object
  home for it — no separate `rows/worlds.jsonl` file is introduced).
- §4's username-based degradation ("Not found → the column is set `NULL`... every one of these
  columns is already designed around [`ON DELETE SET NULL`]") does not cover every `users(id)`
  reference in the exported table set: `world_members.user_id` is `NOT NULL` with `ON DELETE
  CASCADE` (not `SET NULL`), and `explored_fog.user_id` is `NOT NULL` with no FK at all. Neither
  column can hold `NULL`. This plan extends the degradation policy by necessity: an unresolved
  username on one of these two columns drops the row (never inserted) rather than attempting an
  illegal `NULL`, and `import_world`'s returned `ImportSummary` counts the drops so the importing
  admin sees exactly what was skipped, rather than a silent gap.

## Global Constraints

- Every new/changed file compiles under this crate's `#![deny(missing_docs)]` +
  `#![deny(clippy::missing_docs_in_private_items)]` ratchet (in force on `data`, `http`, and every
  module this plan adds beside `backup`) — every public AND private item needs a doc comment.
- No `#[allow(...)]`/`#[expect(...)]` suppression of any kind, per project `CLAUDE.md`. If a lint
  fires, fix the code.
- No lint suppressions, no dead code, no TODO placeholders. Every function body in this plan is
  complete, runnable code — copy it verbatim into the target file.
- Cross-platform: every path is built with `Path`/`PathBuf::join`, never string concatenation or a
  hardcoded separator (per project `CLAUDE.md`'s cross-platform mandate). No shell-out.
- `cargo fmt --check -p shadowcat` and `cargo clippy -p shadowcat --all-targets -- -D warnings` (or
  the project's equivalent lint gate) must pass after every task before moving to the next.
- Every task's tests run via `cargo test -p shadowcat <module path>::tests` (or the crate's usual
  test invocation) and must be green before the task is considered done.
- Doc-sync gate (project `CLAUDE.md`): Task 8 is not optional busywork — it is the completion gate.
  The plan is not done until it lands.

## Tasks

---

### Task 1 — `tar` dependency + `data::world_bundle` row/manifest DTOs

Pure data shapes, no I/O, no DB access. Establishes the vocabulary every later task builds on.

**Files:**
- `C:\Dev\Shadowcat\src\server\Cargo.toml` (edit)
- `C:\Dev\Shadowcat\src\server\src\data\world_bundle.rs` (new)
- `C:\Dev\Shadowcat\src\server\src\data\mod.rs` (edit)

**Step 1 — add the `tar` dependency.**

In `C:\Dev\Shadowcat\src\server\Cargo.toml`, after the line `subtle = "2"`, insert:

```toml
# Per-world export/import bundle format (`world_bundle`): synchronous, no_std-friendly
# tar archive reader/writer. No compression feature enabled — the approved bundle format
# is uncompressed `.tar` (see docs/superpowers/specs/2026-08-21-world-export-import-design.md §6).
tar = "0.4"
```

**Step 2 — write `data/world_bundle.rs`.**

```rust
//! Per-world export/import row DTOs — the on-disk-bundle-portable shape of the
//! six FK-scoped tables `SqliteRepository::delete_world` already walks (read
//! instead of deleted) plus the world's five keyed `settings` rows. A
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
    /// import — never remapped (see the design doc's world-identity fork).
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
/// a historical audit/replay payload, never rewritten (see the design doc's
/// user-identity-resolution fork).
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
/// (`ON DELETE CASCADE`, not `SET NULL`) — unlike the four `SET NULL`-
/// degraded columns the design doc names, an unresolvable username has no
/// `NULL` to degrade to, so `SqliteRepository::import_world` drops the row
/// entirely rather than seat a membership for nobody.
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
/// time (world id is unchanged by design, see the design doc §5).
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
/// since the world id is preserved verbatim on import (§3), the key is
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
    /// `import_world` renames each into place only after every DB row
    /// commits successfully.
    pub staged_assets: Vec<(Uuid, PathBuf)>,
}

/// The outcome of a successful `SqliteRepository::import_world` call. Rows
/// dropped because their `users(id)` reference did not resolve on the target
/// (`world_members`/`explored_fog` — the two `NOT NULL` user columns with no
/// `SET NULL` degradation, see `ExportedMemberRow`/`ExportedFogRow`) are
/// counted rather than silently absorbed, so the triggering admin can see
/// exactly what was dropped.
#[derive(Debug, Clone, Serialize)]
pub struct ImportSummary {
    /// The imported world's id (== the bundle's `manifest.world_id`).
    pub world_id: Uuid,
    /// `world_members` rows dropped because their username did not resolve.
    pub skipped_members: usize,
    /// `explored_fog` rows dropped because their username did not resolve.
    pub skipped_fog: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest() -> BundleManifest {
        let mut row_counts = BTreeMap::new();
        row_counts.insert("documents".to_string(), 1);
        BundleManifest {
            schema_version: BUNDLE_SCHEMA_VERSION,
            world_id: Uuid::from_u128(1),
            world_name: "MOCK_WORLD_A".to_string(),
            world_seq: 42,
            world_created_at: 1000,
            world_updated_at: 2000,
            exported_at_unix_ms: 3000,
            row_counts,
        }
    }

    #[test]
    fn manifest_round_trips_through_json() {
        let manifest = sample_manifest();
        let json = serde_json::to_string(&manifest).unwrap();
        let back: BundleManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest, back);
    }

    #[test]
    fn manifest_carries_world_seq_independent_of_row_counts() {
        // Regression guard for the manifest gap this plan closes: `world_seq`
        // must survive the round trip distinctly from any row count, so an
        // importer can restore `worlds.seq` without conflating it with (e.g.)
        // the document row count.
        let manifest = sample_manifest();
        assert_eq!(manifest.world_seq, 42);
        assert_ne!(
            manifest.world_seq as usize,
            *manifest.row_counts.get("documents").unwrap()
        );
    }
}
```

**Step 3 — register the module.**

In `C:\Dev\Shadowcat\src\server\src\data\mod.rs`, after the line `pub mod validation;`, insert:

```rust
/// Per-world export/import bundle row/manifest DTOs (see `crate::world_bundle`
/// for the tar file-format I/O built from them).
pub mod world_bundle;
```

**Step 4 — verify.** `cargo test -p shadowcat data::world_bundle::tests` passes; `cargo fmt --check
-p shadowcat` and `cargo clippy -p shadowcat --all-targets -- -D warnings` pass.

**Interfaces introduced:** `data::world_bundle::{BUNDLE_SCHEMA_VERSION, BundleManifest,
ExportedDocumentRow, ExportedEventRow, ExportedMemberRow, ExportedInviteRow, ExportedAssetRow,
ExportedFogRow, ExportedSettingRow, WorldExportData, WorldImportData, ImportSummary}`.

---

### Task 2 — `SqliteRepository::export_world_rows`

**Files:** `C:\Dev\Shadowcat\src\server\src\data\sqlite.rs` (edit)

**Step 1 — add the import.**

At the top of the file, in the existing multi-line `use crate::data::document::{...}` block (the
one currently reading `CapabilityRequirement, ContractDeclaration, Document, SchemaDeclaration,
Scope, World, WorldCapDefaults, WorldRole,`), no change is needed — `Document` and `WorldRole` are
already imported. Add a new `use` line directly below it:

```rust
use crate::data::world_bundle::{
    BundleManifest, ExportedAssetRow, ExportedDocumentRow, ExportedEventRow, ExportedFogRow,
    ExportedInviteRow, ExportedMemberRow, ExportedSettingRow, WorldExportData,
    BUNDLE_SCHEMA_VERSION,
};
```

**Step 2 — add the method.**

Insert directly after the existing `delete_world` method (which ends at the `Ok(())\n    }` closing
the function whose doc comment begins "Delete a world and every row keyed to it..."):

```rust
    /// Every world-scoped row `delete_world` would delete, read instead — the
    /// per-world export data source. `users(id)` references are resolved to
    /// portable usernames inline (one `LEFT JOIN`/`JOIN` per table, no N+1
    /// lookups) exactly as documented on each `data::world_bundle::Exported*Row`
    /// type. `NotFound` if `world` does not exist.
    pub async fn export_world_rows(&self, world: Uuid) -> Result<WorldExportData, DataError> {
        let world_row =
            sqlx::query("SELECT name, seq, created_at, updated_at FROM worlds WHERE id = ?")
                .bind(world.to_string())
                .fetch_optional(&self.pool)
                .await?
                .ok_or(DataError::NotFound)?;

        let doc_rows = sqlx::query(
            "SELECT documents.json AS json, documents.seq AS seq, \
             documents.created_seq AS created_seq, users.username AS owner_username \
             FROM documents LEFT JOIN users ON users.id = documents.owner_id \
             WHERE documents.world_id = ? ORDER BY documents.id",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut documents = Vec::with_capacity(doc_rows.len());
        for r in doc_rows {
            let mut document: Document = serde_json::from_str(&r.get::<String, _>("json"))?;
            document.owner = None;
            documents.push(ExportedDocumentRow {
                document,
                owner_username: r.get::<Option<String>, _>("owner_username"),
                seq: r.get("seq"),
                created_seq: r.get("created_seq"),
            });
        }

        let event_rows = sqlx::query(
            "SELECT world_events.seq AS seq, world_events.ts AS ts, \
             world_events.command_json AS command_json, users.username AS author_username \
             FROM world_events LEFT JOIN users ON users.id = world_events.author_id \
             WHERE world_events.world_id = ? ORDER BY world_events.seq",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let events = event_rows
            .into_iter()
            .map(|r| ExportedEventRow {
                seq: r.get("seq"),
                author_username: r.get::<Option<String>, _>("author_username"),
                ts: r.get("ts"),
                command_json: r.get("command_json"),
            })
            .collect();

        let member_rows = sqlx::query(
            "SELECT users.username AS username, world_members.role AS role \
             FROM world_members JOIN users ON users.id = world_members.user_id \
             WHERE world_members.world_id = ? ORDER BY users.username",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut members = Vec::with_capacity(member_rows.len());
        for r in member_rows {
            let role: WorldRole =
                serde_json::from_value(serde_json::Value::String(r.get::<String, _>("role")))?;
            members.push(ExportedMemberRow {
                username: r.get("username"),
                role,
            });
        }

        let invite_rows = sqlx::query(
            "SELECT world_invites.id AS id, world_invites.secret_hash AS secret_hash, \
             world_invites.role AS role, world_invites.created_at AS created_at, \
             world_invites.expires_at AS expires_at, world_invites.revoked_at AS revoked_at, \
             world_invites.consumed_at AS consumed_at, \
             creator.username AS created_by_username, consumer.username AS consumed_by_username \
             FROM world_invites \
             LEFT JOIN users creator ON creator.id = world_invites.created_by \
             LEFT JOIN users consumer ON consumer.id = world_invites.consumed_by \
             WHERE world_invites.world_id = ? ORDER BY world_invites.id",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut invites = Vec::with_capacity(invite_rows.len());
        for r in invite_rows {
            let role: WorldRole =
                serde_json::from_value(serde_json::Value::String(r.get::<String, _>("role")))?;
            invites.push(ExportedInviteRow {
                id: Uuid::parse_str(r.get::<String, _>("id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?,
                secret_hash: r.get("secret_hash"),
                role,
                created_by_username: r.get::<Option<String>, _>("created_by_username"),
                created_at: r.get("created_at"),
                expires_at: r.get("expires_at"),
                revoked_at: r.get::<Option<i64>, _>("revoked_at"),
                consumed_at: r.get::<Option<i64>, _>("consumed_at"),
                consumed_by_username: r.get::<Option<String>, _>("consumed_by_username"),
            });
        }

        let asset_rows = sqlx::query(
            "SELECT assets.id AS id, assets.original_name AS original_name, \
             assets.content_type AS content_type, assets.byte_size AS byte_size, \
             assets.created_at AS created_at, assets.version AS version, \
             users.username AS created_by_username \
             FROM assets LEFT JOIN users ON users.id = assets.created_by \
             WHERE assets.world_id = ? ORDER BY assets.id",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut assets = Vec::with_capacity(asset_rows.len());
        for r in asset_rows {
            assets.push(ExportedAssetRow {
                id: Uuid::parse_str(r.get::<String, _>("id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?,
                original_name: r.get("original_name"),
                content_type: r.get("content_type"),
                byte_size: r.get("byte_size"),
                created_by_username: r.get::<Option<String>, _>("created_by_username"),
                created_at: r.get("created_at"),
                version: r.get("version"),
            });
        }

        let fog_rows = sqlx::query(
            "SELECT explored_fog.scene_id AS scene_id, explored_fog.cells AS cells, \
             users.username AS username \
             FROM explored_fog JOIN users ON users.id = explored_fog.user_id \
             WHERE explored_fog.world_id = ? ORDER BY explored_fog.scene_id, users.username",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut fog = Vec::with_capacity(fog_rows.len());
        for r in fog_rows {
            fog.push(ExportedFogRow {
                scene_id: Uuid::parse_str(r.get::<String, _>("scene_id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?,
                username: r.get("username"),
                cells: r.get("cells"),
            });
        }

        let mut settings = Vec::new();
        for key in world_settings_keys(world) {
            let value: Option<String> =
                sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
                    .bind(&key)
                    .fetch_optional(&self.pool)
                    .await?;
            if let Some(value) = value {
                settings.push(ExportedSettingRow { key, value });
            }
        }

        let mut row_counts = std::collections::BTreeMap::new();
        row_counts.insert("documents".to_string(), documents.len());
        row_counts.insert("world_events".to_string(), events.len());
        row_counts.insert("world_members".to_string(), members.len());
        row_counts.insert("world_invites".to_string(), invites.len());
        row_counts.insert("assets".to_string(), assets.len());
        row_counts.insert("explored_fog".to_string(), fog.len());
        row_counts.insert("settings".to_string(), settings.len());

        let manifest = BundleManifest {
            schema_version: BUNDLE_SCHEMA_VERSION,
            world_id: world,
            world_name: world_row.get("name"),
            world_seq: world_row.get("seq"),
            world_created_at: world_row.get("created_at"),
            world_updated_at: world_row.get("updated_at"),
            exported_at_unix_ms: crate::ws::time::now_millis(),
            row_counts,
        };

        Ok(WorldExportData {
            manifest,
            documents,
            events,
            members,
            invites,
            assets,
            fog,
            settings,
        })
    }
```

**Step 3 — tests.**

Add to the existing `#[cfg(test)] mod tests { ... }` block in `sqlite.rs` (it already has `repo()`
and `world_doc()` helpers — reuse both):

```rust
    #[tokio::test]
    async fn export_world_rows_resolves_owner_username_and_nulls_owner_in_json() {
        let r = repo().await;
        let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
        let owner = r
            .create_user("owner-user", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();
        let mut doc = world_doc(1, w.id, serde_json::json!({}));
        doc.owner = Some(owner);
        let mut conn = r.pool().acquire().await.unwrap();
        SqliteRepository::upsert_document(&mut conn, &doc, 1).await.unwrap();
        drop(conn);

        let data = r.export_world_rows(w.id).await.unwrap();
        assert_eq!(data.documents.len(), 1);
        let exported = &data.documents[0];
        assert_eq!(exported.owner_username.as_deref(), Some("owner-user"));
        assert_eq!(exported.document.owner, None);
        assert_eq!(exported.seq, 1);
        assert_eq!(exported.created_seq, 1);
    }

    #[tokio::test]
    async fn export_world_rows_carries_manifest_watermark_and_row_counts() {
        let r = repo().await;
        let gm = r.create_user("gm2", None, ServerRole::User, 0).await.unwrap();
        let w = r.create_world_owned("Watermark World", gm, 0).await.unwrap();
        let doc = world_doc(2, w.id, serde_json::json!({}));
        let mut conn = r.pool().acquire().await.unwrap();
        SqliteRepository::upsert_document(&mut conn, &doc, 1).await.unwrap();
        drop(conn);

        let data = r.export_world_rows(w.id).await.unwrap();
        assert_eq!(data.manifest.world_id, w.id);
        assert_eq!(data.manifest.world_name, "Watermark World");
        assert_eq!(data.manifest.world_seq, w.seq);
        assert_eq!(data.manifest.world_created_at, w.created_at);
        assert_eq!(data.manifest.row_counts.get("documents"), Some(&1));
        // world_members always has at least the creating GM.
        assert_eq!(data.members.len(), 1);
        assert_eq!(data.members[0].username, "gm2");
    }

    #[tokio::test]
    async fn export_world_rows_not_found_for_unknown_world() {
        let r = repo().await;
        let err = r.export_world_rows(Uuid::from_u128(999)).await.unwrap_err();
        assert!(matches!(err, DataError::NotFound));
    }
```

**Step 4 — verify.** `cargo test -p shadowcat data::sqlite::tests::export_world_rows` (or the
crate's test filter equivalent) passes; `cargo fmt --check`/`clippy` pass.

**Interfaces introduced:** `data::sqlite::SqliteRepository::export_world_rows(&self, world: Uuid)
-> Result<WorldExportData, DataError>`.

---

### Task 3 — `world_bundle::write_bundle` (top-level tar writer)

**Files:**
- `C:\Dev\Shadowcat\src\server\src\world_bundle.rs` (new)
- `C:\Dev\Shadowcat\src\server\src\lib.rs` (edit)

**Step 1 — write `world_bundle.rs`.**

```rust
//! Per-world export/import bundle I/O: builds/reads the uncompressed `.tar`
//! format (`manifest.json` + `rows/<table>.jsonl` + `assets/<asset_id>`).
//! Pure file/tar I/O — no `SqliteRepository` dependency, mirroring `backup`'s
//! separation between row-fetching (data layer) and file format (this
//! module). Synchronous (the `tar` crate has no async API); every call in
//! this module is CPU/disk-bound and must run inside
//! `tokio::task::spawn_blocking` from async callers.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use thiserror::Error;

use crate::data::world_bundle::{
    BundleManifest, ExportedAssetRow, ExportedDocumentRow, ExportedEventRow, ExportedFogRow,
    ExportedInviteRow, ExportedMemberRow, ExportedSettingRow, WorldExportData, WorldImportData,
    BUNDLE_SCHEMA_VERSION,
};

/// All fallible bundle read/write operations return this.
#[derive(Debug, Error)]
pub enum WorldBundleError {
    /// A filesystem or tar-stream operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A `manifest.json`/`rows/*.jsonl` entry failed to (de)serialize.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    /// The archive is missing a required entry, or an entry's path/shape is
    /// not one `write_bundle` could have produced.
    #[error("malformed bundle: {0}")]
    Malformed(String),
    /// A `rows/<table>.jsonl` entry's line count did not match the
    /// manifest's promise for that table — extraction stopped before any row
    /// reached a transaction.
    #[error("row count mismatch for '{table}': manifest promised {expected}, extracted {actual}")]
    RowCountMismatch {
        /// The table whose count disagreed.
        table: String,
        /// `manifest.row_counts[table]`.
        expected: usize,
        /// The number of JSONL lines actually read.
        actual: usize,
    },
    /// `manifest.schema_version` is not one this build understands.
    #[error(
        "unsupported bundle schema_version {0} (this server supports {BUNDLE_SCHEMA_VERSION})"
    )]
    UnsupportedSchemaVersion(u32),
}

/// Append one in-memory JSON blob as a tar entry at `name`.
fn append_bytes(
    builder: &mut tar::Builder<Vec<u8>>,
    name: &str,
    bytes: &[u8],
) -> Result<(), WorldBundleError> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_cksum();
    builder.append_data(&mut header, name, bytes)?;
    Ok(())
}

/// Serialize `rows` as newline-delimited JSON (one object per line).
fn to_jsonl<T: serde::Serialize>(rows: &[T]) -> Result<Vec<u8>, WorldBundleError> {
    let mut out = Vec::new();
    for row in rows {
        serde_json::to_writer(&mut out, row)?;
        out.push(b'\n');
    }
    Ok(out)
}

/// Build the `.tar` bundle for `data`, streaming each `assets` row's bytes
/// directly from `assets_dir.join(<world_id>).join(<asset_id>)` (the
/// standard `storage_key` scheme) — `data.assets` carries no `storage_key`
/// field by design (see `data::world_bundle::ExportedAssetRow`'s doc).
/// `manifest.json` is written FIRST, always — `read_bundle` relies on this
/// ordering to resolve the asset extraction root before any `assets/*` entry
/// arrives.
///
/// # Examples
///
/// ```text
/// let bytes = write_bundle(&data, Path::new("/srv/shadowcat/assets"))?;
/// std::fs::write("world.tar", bytes)?;
/// ```
pub fn write_bundle(
    data: &WorldExportData,
    assets_dir: &Path,
) -> Result<Vec<u8>, WorldBundleError> {
    let mut builder = tar::Builder::new(Vec::new());

    let manifest_bytes = serde_json::to_vec(&data.manifest)?;
    append_bytes(&mut builder, "manifest.json", &manifest_bytes)?;

    append_bytes(
        &mut builder,
        "rows/documents.jsonl",
        &to_jsonl(&data.documents)?,
    )?;
    append_bytes(
        &mut builder,
        "rows/world_events.jsonl",
        &to_jsonl(&data.events)?,
    )?;
    append_bytes(
        &mut builder,
        "rows/world_members.jsonl",
        &to_jsonl(&data.members)?,
    )?;
    append_bytes(
        &mut builder,
        "rows/world_invites.jsonl",
        &to_jsonl(&data.invites)?,
    )?;
    append_bytes(&mut builder, "rows/assets.jsonl", &to_jsonl(&data.assets)?)?;
    append_bytes(
        &mut builder,
        "rows/explored_fog.jsonl",
        &to_jsonl(&data.fog)?,
    )?;
    append_bytes(
        &mut builder,
        "rows/settings.jsonl",
        &to_jsonl(&data.settings)?,
    )?;

    for asset in &data.assets {
        let src = assets_dir
            .join(data.manifest.world_id.to_string())
            .join(asset.id.to_string());
        let mut file = std::fs::File::open(&src)?;
        builder.append_file(format!("assets/{}", asset.id), &mut file)?;
    }

    builder.into_inner().map_err(WorldBundleError::Io)
}

/// Extract `tar_path` (a bundle previously staged to disk by the caller —
/// see `http::world_bundle::import_world`) into a `WorldImportData`.
/// `assets_dir` is the server's asset root (`Config::assets_path()`); each
/// `assets/<id>` entry is staged to
/// `assets_dir.join(<world_id>).join("<id>.<random>.import-tmp")` — the same
/// directory the finalized file will live in, so
/// `SqliteRepository::import_world`'s later rename is same-filesystem.
/// `manifest.json` MUST be the archive's first entry (the invariant
/// `write_bundle` establishes) — the asset extraction root cannot be
/// resolved before the manifest is read, so any entry preceding it is
/// `Malformed`. Refuses (before returning) on a `schema_version` mismatch or
/// a `rows/<table>.jsonl` line count that disagrees with the manifest's own
/// promise — a corrupt/truncated bundle is caught here, before
/// `SqliteRepository::import_world` ever opens a transaction.
///
/// # Examples
///
/// ```text
/// let data = read_bundle(Path::new("/tmp/upload.tar"), Path::new("/srv/shadowcat/assets"))?;
/// ```
pub fn read_bundle(
    tar_path: &Path,
    assets_dir: &Path,
) -> Result<WorldImportData, WorldBundleError> {
    let file = std::fs::File::open(tar_path)?;
    let mut archive = tar::Archive::new(file);
    let mut entries = archive.entries()?;

    let Some(first) = entries.next() else {
        return Err(WorldBundleError::Malformed("empty archive".into()));
    };
    let mut first = first?;
    let first_path = first.path()?.to_string_lossy().replace('\\', "/");
    if first_path != "manifest.json" {
        return Err(WorldBundleError::Malformed(
            "manifest.json must be the first archive entry".into(),
        ));
    }
    let mut manifest_bytes = Vec::new();
    first.read_to_end(&mut manifest_bytes)?;
    let manifest: BundleManifest = serde_json::from_slice(&manifest_bytes)?;
    if manifest.schema_version != BUNDLE_SCHEMA_VERSION {
        return Err(WorldBundleError::UnsupportedSchemaVersion(
            manifest.schema_version,
        ));
    }
    drop(first);

    let world_asset_dir = assets_dir.join(manifest.world_id.to_string());
    std::fs::create_dir_all(&world_asset_dir)?;

    let mut rows: HashMap<&'static str, Vec<u8>> = HashMap::new();
    let mut staged_assets: Vec<(uuid::Uuid, std::path::PathBuf)> = Vec::new();

    for entry in entries {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().replace('\\', "/");
        if let Some(id_str) = path.strip_prefix("assets/") {
            let id = uuid::Uuid::parse_str(id_str).map_err(|_| {
                WorldBundleError::Malformed(format!("non-UUID asset entry name: {id_str}"))
            })?;
            let staged =
                world_asset_dir.join(format!("{id}.{}.import-tmp", uuid::Uuid::new_v4()));
            let mut out = std::fs::File::create(&staged)?;
            std::io::copy(&mut entry, &mut out)?;
            staged_assets.push((id, staged));
            continue;
        }
        let table = match path.as_str() {
            "rows/documents.jsonl" => "documents",
            "rows/world_events.jsonl" => "world_events",
            "rows/world_members.jsonl" => "world_members",
            "rows/world_invites.jsonl" => "world_invites",
            "rows/assets.jsonl" => "assets",
            "rows/explored_fog.jsonl" => "explored_fog",
            "rows/settings.jsonl" => "settings",
            other => {
                return Err(WorldBundleError::Malformed(format!(
                    "unrecognized bundle entry: {other}"
                )))
            }
        };
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        rows.insert(table, bytes);
    }

    fn from_jsonl<T: serde::de::DeserializeOwned>(
        bytes: &[u8],
    ) -> Result<Vec<T>, WorldBundleError> {
        bytes
            .split(|&b| b == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).map_err(WorldBundleError::from))
            .collect()
    }

    fn take<'a>(
        rows: &'a HashMap<&'static str, Vec<u8>>,
        table: &'static str,
    ) -> Result<&'a [u8], WorldBundleError> {
        rows.get(table)
            .map(Vec::as_slice)
            .ok_or_else(|| WorldBundleError::Malformed(format!("missing rows/{table}.jsonl")))
    }

    let documents: Vec<ExportedDocumentRow> = from_jsonl(take(&rows, "documents")?)?;
    let events: Vec<ExportedEventRow> = from_jsonl(take(&rows, "world_events")?)?;
    let members: Vec<ExportedMemberRow> = from_jsonl(take(&rows, "world_members")?)?;
    let invites: Vec<ExportedInviteRow> = from_jsonl(take(&rows, "world_invites")?)?;
    let assets: Vec<ExportedAssetRow> = from_jsonl(take(&rows, "assets")?)?;
    let fog: Vec<ExportedFogRow> = from_jsonl(take(&rows, "explored_fog")?)?;
    let settings: Vec<ExportedSettingRow> = from_jsonl(take(&rows, "settings")?)?;

    for (table, actual) in [
        ("documents", documents.len()),
        ("world_events", events.len()),
        ("world_members", members.len()),
        ("world_invites", invites.len()),
        ("assets", assets.len()),
        ("explored_fog", fog.len()),
        ("settings", settings.len()),
    ] {
        let expected = *manifest.row_counts.get(table).ok_or_else(|| {
            WorldBundleError::Malformed(format!("manifest missing row_counts['{table}']"))
        })?;
        if expected != actual {
            return Err(WorldBundleError::RowCountMismatch {
                table: table.to_string(),
                expected,
                actual,
            });
        }
    }

    Ok(WorldImportData {
        manifest,
        documents,
        events,
        members,
        invites,
        assets,
        fog,
        settings,
        staged_assets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::document::{Document, Scope};
    use crate::data::world_bundle::WorldExportData;
    use uuid::Uuid;

    fn minimal_document() -> Document {
        Document {
            id: Uuid::from_u128(1),
            scope: Scope::World {
                world_id: Uuid::from_u128(9),
            },
            doc_type: "actor".into(),
            schema_version: 1,
            name: None,
            source: None,
            base: None,
            owner: None,
            permissions: Default::default(),
            embedded: Default::default(),
            parent_id: None,
            engine: crate::data::document::tests::default_test_engine("actor"),
            system: serde_json::json!({}),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn sample_data(world: Uuid) -> WorldExportData {
        use crate::data::world_bundle::{BundleManifest, ExportedDocumentRow, BUNDLE_SCHEMA_VERSION};
        let mut row_counts = std::collections::BTreeMap::new();
        row_counts.insert("documents".to_string(), 1);
        row_counts.insert("world_events".to_string(), 0);
        row_counts.insert("world_members".to_string(), 0);
        row_counts.insert("world_invites".to_string(), 0);
        row_counts.insert("assets".to_string(), 0);
        row_counts.insert("explored_fog".to_string(), 0);
        row_counts.insert("settings".to_string(), 0);
        WorldExportData {
            manifest: BundleManifest {
                schema_version: BUNDLE_SCHEMA_VERSION,
                world_id: world,
                world_name: "W".to_string(),
                world_seq: 5,
                world_created_at: 10,
                world_updated_at: 20,
                exported_at_unix_ms: 30,
                row_counts,
            },
            documents: vec![ExportedDocumentRow {
                document: minimal_document(),
                owner_username: None,
                seq: 1,
                created_seq: 1,
            }],
            events: vec![],
            members: vec![],
            invites: vec![],
            assets: vec![],
            fog: vec![],
            settings: vec![],
        }
    }

    #[test]
    fn write_bundle_with_no_assets_produces_a_readable_manifest_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let world = Uuid::from_u128(42);
        let data = sample_data(world);

        let bytes = write_bundle(&data, tmp.path()).unwrap();
        let mut archive = tar::Archive::new(bytes.as_slice());
        let mut entries = archive.entries().unwrap();
        let mut first = entries.next().unwrap().unwrap();
        assert_eq!(first.path().unwrap().to_string_lossy(), "manifest.json");
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut first, &mut buf).unwrap();
        let manifest: crate::data::world_bundle::BundleManifest =
            serde_json::from_slice(&buf).unwrap();
        assert_eq!(manifest.world_id, world);
        assert_eq!(manifest.world_seq, 5);
    }

    #[test]
    fn write_bundle_streams_asset_bytes_from_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let world = Uuid::from_u128(7);
        let asset_id = Uuid::from_u128(100);
        let asset_dir = tmp.path().join(world.to_string());
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join(asset_id.to_string()), b"PNGDATA").unwrap();

        let mut data = sample_data(world);
        data.assets.push(crate::data::world_bundle::ExportedAssetRow {
            id: asset_id,
            original_name: "token.png".to_string(),
            content_type: "image/png".to_string(),
            byte_size: 7,
            created_by_username: None,
            created_at: 0,
            version: 1,
        });
        data.manifest
            .row_counts
            .insert("assets".to_string(), 1);

        let bytes = write_bundle(&data, tmp.path()).unwrap();
        let mut archive = tar::Archive::new(bytes.as_slice());
        let found = archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap())
            .any(|e| e.path().unwrap().to_string_lossy() == format!("assets/{asset_id}"));
        assert!(found);
    }

    #[test]
    fn write_bundle_missing_asset_file_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let world = Uuid::from_u128(8);
        let mut data = sample_data(world);
        data.assets.push(crate::data::world_bundle::ExportedAssetRow {
            id: Uuid::from_u128(200),
            original_name: "missing.png".to_string(),
            content_type: "image/png".to_string(),
            byte_size: 1,
            created_by_username: None,
            created_at: 0,
            version: 1,
        });
        let err = write_bundle(&data, tmp.path()).unwrap_err();
        assert!(matches!(err, WorldBundleError::Io(_)));
    }
}
```

**Step 2 — register the module.**

In `C:\Dev\Shadowcat\src\server\src\lib.rs`, insert `pub mod world_bundle;` alphabetically between
`pub mod scene;` and `pub mod ws;`:

```rust
pub mod scene;
/// Per-world export/import: builds/reads the `.tar` bundle format (see
/// `data::world_bundle` for the row/manifest types, `http::world_bundle` for
/// the HTTP routes).
pub mod world_bundle;
pub mod ws;
```

**Step 3 — verify.** `cargo test -p shadowcat world_bundle::tests` passes (this task's tests need
the `tempfile`/`tar` dev-usage already available — `tempfile` is already a dev-dependency;
`tar` was added as a regular dependency in Task 1, so it is available in test builds too).
`cargo fmt --check`/`clippy` pass.

**Interfaces introduced:** `world_bundle::{WorldBundleError, write_bundle, read_bundle}` (note:
`read_bundle` is implemented and tested for real in Task 5; this task's file includes its full body
now since `write_bundle`'s own round-trip tests read back via the raw `tar` crate directly, not via
`read_bundle` — Task 5 adds `read_bundle`'s own dedicated test coverage).

---

### Task 4 — `POST /api/worlds/{id}/export` HTTP route

**Files:**
- `C:\Dev\Shadowcat\src\server\src\http\world_bundle.rs` (new)
- `C:\Dev\Shadowcat\src\server\src\http\mod.rs` (edit)

**Step 1 — write the route file (export half only; import is added in Task 7).**

```rust
//! `POST /api/worlds/{id}/export` and `POST /api/worlds/import` — per-world
//! bundle export/import
//! (`docs/superpowers/specs/2026-08-21-world-export-import-design.md`).
//! Export is world-GM-gated (`require_gm`, mirroring the existing GM-gated
//! asset routes). Import is server-admin-only: a bulk multi-table insert
//! that bypasses every capability/schema/OCC gate the live write paths
//! enforce (the same trusted-substrate posture `apply_command`'s replay path
//! already has) — a materially more privileged operation than ordinary world
//! CREATION (`POST /api/worlds`, open to any authenticated user) or GM-level
//! world management, so it needs the server's highest tier, not a match to
//! either of those.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use crate::auth::session::AuthUser;
use crate::http::error::AppError;
use crate::http::routes::require_gm;
use crate::http::AppState;
use crate::world_bundle::write_bundle;

/// `POST /api/worlds/{id}/export` — world-GM-gated (server admins resolve to
/// GM via `require_gm`). Streams the world's `.tar` bundle as the response
/// body.
pub async fn export_world(
    State(state): State<AppState>,
    user: AuthUser,
    Path(world): Path<Uuid>,
) -> Result<Response, AppError> {
    require_gm(&state, &user, world).await?;
    let data = state.repo.export_world_rows(world).await?;
    let assets_dir = state.config.assets_path();
    let bytes = tokio::task::spawn_blocking(move || write_bundle(&data, &assets_dir))
        .await
        .map_err(|e| {
            tracing::error!(?e, %world, "world export task panicked");
            AppError::Internal
        })?
        .map_err(|e| {
            tracing::error!(?e, %world, "world export failed");
            AppError::Internal
        })?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-tar".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"world-{world}.tar\""),
            ),
        ],
        Body::from(bytes),
    )
        .into_response())
}
```

**Step 2 — register the module + route.**

In `C:\Dev\Shadowcat\src\server\src\http\mod.rs`, add the module declaration after `pub mod
throttle;`:

```rust
/// Per-world export/import bundle routes.
pub mod world_bundle;
```

In the `router()` function, add a new route entry directly after the `/api/worlds/{id}` route
(so it reads):

```rust
        .route("/api/worlds/{id}", delete(routes::delete_world))
        .route(
            "/api/worlds/{id}/export",
            post(world_bundle::export_world),
        )
```

**Step 3 — tests.**

Add to `http/mod.rs`'s existing `#[cfg(test)] pub(crate) mod tests { ... }` block (it already has
`server_with_user`/`initialized_state` helpers):

```rust
    #[tokio::test]
    async fn export_world_requires_gm() {
        let server = server_with_user("gm-user", "pw-gm", ServerRole::User).await;
        server
            .post("/api/login")
            .json(&serde_json::json!({"username": "gm-user", "password": "pw-gm"}))
            .await
            .assert_status_success();
        let world: crate::data::document::World = server
            .post("/api/worlds")
            .json(&serde_json::json!({"name": "Exportable"}))
            .await
            .json();

        // A second, non-member user cannot export it.
        let outsider = server_with_user("outsider", "pw-out", ServerRole::User).await;
        outsider
            .post("/api/login")
            .json(&serde_json::json!({"username": "outsider", "password": "pw-out"}))
            .await
            .assert_status_success();
        outsider
            .post(&format!("/api/worlds/{}/export", world.id))
            .await
            .assert_status(axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn export_world_streams_a_tar_for_the_gm() {
        let server = server_with_user("gm-export", "pw-gm2", ServerRole::User).await;
        server
            .post("/api/login")
            .json(&serde_json::json!({"username": "gm-export", "password": "pw-gm2"}))
            .await
            .assert_status_success();
        let world: crate::data::document::World = server
            .post("/api/worlds")
            .json(&serde_json::json!({"name": "Exportable2"}))
            .await
            .json();

        let resp = server
            .post(&format!("/api/worlds/{}/export", world.id))
            .await;
        resp.assert_status_ok();
        let bytes = resp.into_bytes();
        // A valid, parseable tar whose first entry is manifest.json.
        let mut archive = tar::Archive::new(bytes.as_ref());
        let mut first = archive.entries().unwrap().next().unwrap().unwrap();
        assert_eq!(first.path().unwrap().to_string_lossy(), "manifest.json");
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut first, &mut buf).unwrap();
        let manifest: crate::data::world_bundle::BundleManifest =
            serde_json::from_slice(&buf).unwrap();
        assert_eq!(manifest.world_id, world.id);
    }
```

**Step 4 — verify.** `cargo test -p shadowcat http::tests::export_world` passes; a manual
`cargo test -p shadowcat --test '*'` full run stays green (no route-registration regressions);
`cargo fmt --check`/`clippy` pass.

**Interfaces introduced:** `http::world_bundle::export_world` handler; route `POST
/api/worlds/{id}/export`.

---

### Task 5 — `world_bundle::read_bundle` dedicated tests

`read_bundle`'s implementation already shipped in Task 3 (it is exercised transitively there only
by `write_bundle`'s own tests reading raw tar bytes back with the `tar` crate directly, never
through `read_bundle` itself). This task adds `read_bundle`'s own direct test coverage: the
write→read round trip, the manifest-must-be-first-entry contract, the row-count mismatch gate, and
the schema-version gate.

**Files:** `C:\Dev\Shadowcat\src\server\src\world_bundle.rs` (edit — append to the existing
`#[cfg(test)] mod tests` block)

**Step 1 — add tests.**

```rust
    #[test]
    fn read_bundle_round_trips_write_bundle_output() {
        let export_tmp = tempfile::tempdir().unwrap();
        let world = Uuid::from_u128(55);
        let asset_id = Uuid::from_u128(555);
        let asset_dir = export_tmp.path().join(world.to_string());
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join(asset_id.to_string()), b"BYTES").unwrap();

        let mut data = sample_data(world);
        data.assets.push(crate::data::world_bundle::ExportedAssetRow {
            id: asset_id,
            original_name: "a.png".to_string(),
            content_type: "image/png".to_string(),
            byte_size: 5,
            created_by_username: None,
            created_at: 0,
            version: 1,
        });
        data.manifest.row_counts.insert("assets".to_string(), 1);

        let bytes = write_bundle(&data, export_tmp.path()).unwrap();
        let tar_path = export_tmp.path().join("bundle.tar");
        std::fs::write(&tar_path, &bytes).unwrap();

        let import_tmp = tempfile::tempdir().unwrap();
        let imported = read_bundle(&tar_path, import_tmp.path()).unwrap();

        assert_eq!(imported.manifest.world_id, world);
        assert_eq!(imported.documents.len(), 1);
        assert_eq!(imported.staged_assets.len(), 1);
        let (staged_id, staged_path) = &imported.staged_assets[0];
        assert_eq!(*staged_id, asset_id);
        assert_eq!(std::fs::read(staged_path).unwrap(), b"BYTES");
        // Staged file lives beside where the final `<id>` path will live.
        assert_eq!(
            staged_path.parent().unwrap(),
            import_tmp.path().join(world.to_string())
        );
    }

    #[test]
    fn read_bundle_rejects_row_count_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let world = Uuid::from_u128(66);
        let mut data = sample_data(world);
        // Lie about the document count the manifest promises.
        data.manifest.row_counts.insert("documents".to_string(), 2);

        let bytes = write_bundle(&data, tmp.path()).unwrap();
        let tar_path = tmp.path().join("bad.tar");
        std::fs::write(&tar_path, &bytes).unwrap();

        let import_tmp = tempfile::tempdir().unwrap();
        let err = read_bundle(&tar_path, import_tmp.path()).unwrap_err();
        assert!(matches!(
            err,
            WorldBundleError::RowCountMismatch {
                table,
                expected: 2,
                actual: 1
            } if table == "documents"
        ));
    }

    #[test]
    fn read_bundle_rejects_unsupported_schema_version() {
        let tmp = tempfile::tempdir().unwrap();
        let world = Uuid::from_u128(77);
        let mut data = sample_data(world);
        data.manifest.schema_version = BUNDLE_SCHEMA_VERSION + 1;

        let bytes = write_bundle(&data, tmp.path()).unwrap();
        let tar_path = tmp.path().join("future.tar");
        std::fs::write(&tar_path, &bytes).unwrap();

        let import_tmp = tempfile::tempdir().unwrap();
        let err = read_bundle(&tar_path, import_tmp.path()).unwrap_err();
        assert!(matches!(
            err,
            WorldBundleError::UnsupportedSchemaVersion(v) if v == BUNDLE_SCHEMA_VERSION + 1
        ));
    }

    #[test]
    fn read_bundle_rejects_archive_not_starting_with_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, "rows/documents.jsonl", &b"{}\n"[..]).unwrap();
        let bytes = builder.into_inner().unwrap();
        let tar_path = tmp.path().join("wrong_order.tar");
        std::fs::write(&tar_path, &bytes).unwrap();

        let import_tmp = tempfile::tempdir().unwrap();
        let err = read_bundle(&tar_path, import_tmp.path()).unwrap_err();
        assert!(matches!(err, WorldBundleError::Malformed(_)));
    }
```

**Step 2 — verify.** `cargo test -p shadowcat world_bundle::tests` passes; `cargo fmt
--check`/`clippy` pass.

**Interfaces exercised (already introduced in Task 3):** `world_bundle::read_bundle`.

---

### Task 6 — `SqliteRepository::import_world`

**Files:** `C:\Dev\Shadowcat\src\server\src\data\sqlite.rs` (edit)

**Step 1 — add the import.**

Add to the existing `use crate::data::world_bundle::{...};` line from Task 2, extending it to also
bring in `ImportSummary` and `WorldImportData`:

```rust
use crate::data::world_bundle::{
    BundleManifest, ExportedAssetRow, ExportedDocumentRow, ExportedEventRow, ExportedFogRow,
    ExportedInviteRow, ExportedMemberRow, ExportedSettingRow, ImportSummary, WorldExportData,
    WorldImportData, BUNDLE_SCHEMA_VERSION,
};
```

**Step 2 — add the two private helpers + the public method.**

Insert directly after `export_world_rows` (added in Task 2):

```rust
    /// Resolve a portable username to a target-local user id inside `tx`, or
    /// `None` when `username` is `None` (no source owner) OR the username
    /// does not exist on this server — the degradation
    /// `documents.owner_id`/`world_events.author_id`/
    /// `world_invites.{created_by,consumed_by}` are already `ON DELETE SET
    /// NULL`-designed around.
    async fn resolve_username_tx(
        tx: &mut sqlx::SqliteConnection,
        username: Option<&str>,
    ) -> Result<Option<Uuid>, DataError> {
        let Some(username) = username else {
            return Ok(None);
        };
        let id: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(&mut *tx)
            .await?;
        id.map(|s| Uuid::parse_str(&s).map_err(|e| DataError::OpFailed(e.to_string())))
            .transpose()
    }

    /// Insert one imported document row with EXPLICIT `seq`/`created_seq`,
    /// independently preserved from the source server — unlike the live
    /// write path's `upsert_document`, where a fresh Create always sets
    /// `seq == created_seq`. Re-derives every column from `doc` the same way
    /// `upsert_document` does, and reindexes both FTS tables from
    /// `doc`'s content so search state is rebuilt rather than carried across
    /// servers (`documents_fts_public`/`documents_fts_gm` are never
    /// exported/imported directly — see the design doc §2). A plain `INSERT`
    /// (not `upsert_document`'s `ON CONFLICT(id) DO UPDATE`): a document id
    /// colliding with an existing row anywhere on the target server (a
    /// separate axis from the already-gated world-id collision) is a genuine
    /// data-integrity fault, and letting the `UNIQUE` constraint violation
    /// surface as an ordinary `DataError::Sqlx` — aborting and rolling back
    /// the whole import transaction — is exactly the "any row-insert failure
    /// mid-transaction rolls back the whole import" behavior the design doc
    /// §7 already specifies, not a case needing special handling.
    async fn insert_imported_document(
        conn: &mut sqlx::SqliteConnection,
        doc: &Document,
        seq: i64,
        created_seq: i64,
    ) -> Result<(), DataError> {
        let (scope_kind, world_id, pack) = match &doc.scope {
            Scope::Compendium { pack } => ("compendium", None, Some(pack.clone())),
            Scope::World { world_id } => ("world", Some(world_id.to_string()), None),
        };
        let (source_id, source_pack, source_version) = match &doc.source {
            Some(s) => (
                Some(s.id.to_string()),
                s.pack.clone(),
                Some(s.version as i64),
            ),
            None => (None, None, None),
        };
        let json = serde_json::to_string(doc)?;
        sqlx::query(
            "INSERT INTO documents (id, scope_kind, world_id, pack, doc_type, schema_version, \
             source_id, source_pack, source_version, owner_id, parent_id, seq, created_seq, json, \
             created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(doc.id.to_string())
        .bind(scope_kind)
        .bind(world_id.clone())
        .bind(pack)
        .bind(&doc.doc_type)
        .bind(doc.schema_version as i64)
        .bind(source_id)
        .bind(source_pack)
        .bind(source_version)
        .bind(doc.owner.map(|o| o.to_string()))
        .bind(doc.parent_id.map(|p| p.to_string()))
        .bind(seq)
        .bind(created_seq)
        .bind(json)
        .bind(doc.created_at)
        .bind(doc.updated_at)
        .execute(&mut *conn)
        .await?;
        sqlx::query("DELETE FROM documents_fts_public WHERE doc_id = ?")
            .bind(doc.id.to_string())
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM documents_fts_gm WHERE doc_id = ?")
            .bind(doc.id.to_string())
            .execute(&mut *conn)
            .await?;
        sqlx::query(
            "INSERT INTO documents_fts_public (content, doc_id, world_id) VALUES (?, ?, ?)",
        )
        .bind(crate::data::search::index_content_public(doc))
        .bind(doc.id.to_string())
        .bind(world_id.clone())
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "INSERT INTO documents_fts_gm (content_all, doc_id, world_id) VALUES (?, ?, ?)",
        )
        .bind(crate::data::search::index_content(doc))
        .bind(doc.id.to_string())
        .bind(world_id)
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    /// Import one `WorldImportData` bundle in a single transaction: reject a
    /// world-id collision with `worlds.id` before any row is written, insert
    /// `worlds` then every table in FK-safe order (`documents`/
    /// `world_events`/`world_members`/`world_invites`/`assets`, then the
    /// FK-less `explored_fog`/`settings`), resolving each row's portable
    /// username(s) against THIS server's `users` table, then finalize every
    /// staged asset file (rename into place beside itself — see
    /// `data::world_bundle::WorldImportData::staged_assets`) before
    /// committing — a failure at any point (including a rename) drops the
    /// transaction unrolled-back, so no partial world is ever visible.
    /// `world_members`/`explored_fog` rows whose username does not resolve
    /// are DROPPED (their `user_id` column is `NOT NULL`, so there is no
    /// `SET NULL` degradation to fall back to, unlike the four nullable
    /// owner/author/created_by/consumed_by columns) — counted in the
    /// returned `ImportSummary` rather than silently absorbed.
    pub async fn import_world(&self, data: WorldImportData) -> Result<ImportSummary, DataError> {
        let mut tx = self.pool.begin().await?;
        let world = data.manifest.world_id;

        let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM worlds WHERE id = ?")
            .bind(world.to_string())
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_some() {
            return Err(DataError::Conflict(format!(
                "world {world} already exists on this server"
            )));
        }

        sqlx::query(
            "INSERT INTO worlds (id, name, seq, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(world.to_string())
        .bind(&data.manifest.world_name)
        .bind(data.manifest.world_seq)
        .bind(data.manifest.world_created_at)
        .bind(data.manifest.world_updated_at)
        .execute(&mut *tx)
        .await?;

        for row in &data.documents {
            let owner = Self::resolve_username_tx(&mut tx, row.owner_username.as_deref()).await?;
            let mut document = row.document.clone();
            document.owner = owner;
            Self::insert_imported_document(&mut tx, &document, row.seq, row.created_seq).await?;
        }

        for row in &data.events {
            let author =
                Self::resolve_username_tx(&mut tx, row.author_username.as_deref()).await?;
            sqlx::query(
                "INSERT INTO world_events (world_id, seq, author_id, ts, command_json) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(world.to_string())
            .bind(row.seq)
            .bind(author.map(|u| u.to_string()))
            .bind(row.ts)
            .bind(&row.command_json)
            .execute(&mut *tx)
            .await?;
        }

        let mut skipped_members = 0usize;
        for row in &data.members {
            match Self::resolve_username_tx(&mut tx, Some(row.username.as_str())).await? {
                Some(user_id) => {
                    sqlx::query(
                        "INSERT INTO world_members (world_id, user_id, role) VALUES (?, ?, ?)",
                    )
                    .bind(world.to_string())
                    .bind(user_id.to_string())
                    .bind(
                        serde_json::to_value(row.role)?
                            .as_str()
                            .expect("WorldRole serializes as a string")
                            .to_string(),
                    )
                    .execute(&mut *tx)
                    .await?;
                }
                None => skipped_members += 1,
            }
        }

        for row in &data.invites {
            let created_by =
                Self::resolve_username_tx(&mut tx, row.created_by_username.as_deref()).await?;
            let consumed_by =
                Self::resolve_username_tx(&mut tx, row.consumed_by_username.as_deref()).await?;
            sqlx::query(
                "INSERT INTO world_invites \
                 (id, world_id, secret_hash, role, created_by, created_at, expires_at, \
                  revoked_at, consumed_at, consumed_by) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(row.id.to_string())
            .bind(world.to_string())
            .bind(&row.secret_hash)
            .bind(
                serde_json::to_value(row.role)?
                    .as_str()
                    .expect("WorldRole serializes as a string")
                    .to_string(),
            )
            .bind(created_by.map(|u| u.to_string()))
            .bind(row.created_at)
            .bind(row.expires_at)
            .bind(row.revoked_at)
            .bind(row.consumed_at)
            .bind(consumed_by.map(|u| u.to_string()))
            .execute(&mut *tx)
            .await?;
        }

        for row in &data.assets {
            let created_by =
                Self::resolve_username_tx(&mut tx, row.created_by_username.as_deref()).await?;
            let storage_key = format!("{world}/{}", row.id);
            sqlx::query(
                "INSERT INTO assets \
                 (id, world_id, storage_key, original_name, content_type, byte_size, created_by, \
                  created_at, version) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(row.id.to_string())
            .bind(world.to_string())
            .bind(storage_key)
            .bind(&row.original_name)
            .bind(&row.content_type)
            .bind(row.byte_size)
            .bind(created_by.map(|u| u.to_string()))
            .bind(row.created_at)
            .bind(row.version)
            .execute(&mut *tx)
            .await?;
        }

        let mut skipped_fog = 0usize;
        for row in &data.fog {
            match Self::resolve_username_tx(&mut tx, Some(row.username.as_str())).await? {
                Some(user_id) => {
                    sqlx::query(
                        "INSERT INTO explored_fog (world_id, scene_id, user_id, cells) \
                         VALUES (?, ?, ?, ?)",
                    )
                    .bind(world.to_string())
                    .bind(row.scene_id.to_string())
                    .bind(user_id.to_string())
                    .bind(row.cells.as_slice())
                    .execute(&mut *tx)
                    .await?;
                }
                None => skipped_fog += 1,
            }
        }

        for row in &data.settings {
            sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?)")
                .bind(&row.key)
                .bind(&row.value)
                .execute(&mut *tx)
                .await?;
        }

        // Finalize staged asset files: rename each staged temp file (already
        // living in the target world's asset directory, per
        // `world_bundle::read_bundle`) to its final `<id>` name in that same
        // directory, only after every row above has been accepted by the
        // transaction. A failure here still rolls the WHOLE transaction back
        // (the early `?` return drops `tx` unrolled-back), and best-effort
        // removes every staged/finalized file so a rolled-back import leaves
        // no orphan bytes behind.
        let mut finalized: Vec<std::path::PathBuf> = Vec::with_capacity(data.staged_assets.len());
        for (id, staged) in &data.staged_assets {
            let dest = staged
                .parent()
                .expect("staged asset path always has a parent directory")
                .join(id.to_string());
            if let Err(e) = tokio::fs::rename(staged, &dest).await {
                for done in &finalized {
                    let _ = tokio::fs::remove_file(done).await;
                }
                for (_, remaining) in &data.staged_assets {
                    let _ = tokio::fs::remove_file(remaining).await;
                }
                return Err(DataError::OpFailed(format!(
                    "failed to finalize imported asset {id}: {e}"
                )));
            }
            finalized.push(dest);
        }

        tx.commit().await?;

        Ok(ImportSummary {
            world_id: world,
            skipped_members,
            skipped_fog,
        })
    }
```

**Step 3 — tests.**

Add to the existing `#[cfg(test)] mod tests { ... }` block:

```rust
    #[tokio::test]
    async fn import_world_round_trips_every_table_through_a_real_tar_bundle() {
        let src = repo().await;
        let gm = src.create_user("gm3", None, ServerRole::User, 0).await.unwrap();
        let owner = src
            .create_user("actor-owner", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = src.create_world_owned("Round Trip World", gm, 0).await.unwrap();

        let mut doc = world_doc(10, w.id, serde_json::json!({"hp": 5}));
        doc.owner = Some(owner);
        let mut conn = src.pool().acquire().await.unwrap();
        SqliteRepository::upsert_document(&mut conn, &doc, 1).await.unwrap();
        drop(conn);

        sqlx::query(
            "INSERT INTO world_events (world_id, seq, author_id, ts, command_json) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(w.id.to_string())
        .bind(2i64)
        .bind(gm.to_string())
        .bind(0i64)
        .bind(r#"{"kind":"Noop","payload":{"embedded_ref":"deadbeef"}}"#)
        .execute(src.pool())
        .await
        .unwrap();

        let export_tmp = tempfile::tempdir().unwrap();
        let asset_id = Uuid::new_v4();
        let asset_dir = export_tmp.path().join(w.id.to_string());
        tokio::fs::create_dir_all(&asset_dir).await.unwrap();
        tokio::fs::write(asset_dir.join(asset_id.to_string()), b"ASSETBYTES")
            .await
            .unwrap();
        src.insert_asset(&crate::data::asset::Asset {
            id: asset_id,
            world_id: w.id,
            storage_key: format!("{}/{asset_id}", w.id),
            original_name: "token.png".to_string(),
            content_type: "image/png".to_string(),
            byte_size: 10,
            created_by: Some(owner),
            created_at: 0,
            version: 1,
        })
        .await
        .unwrap();
        src.set_explored(w.id, doc.id, owner, &[1, 2, 3]).await.unwrap();

        let export_data = src.export_world_rows(w.id).await.unwrap();
        let bytes = crate::world_bundle::write_bundle(&export_data, export_tmp.path()).unwrap();
        let tar_path = export_tmp.path().join("bundle.tar");
        tokio::fs::write(&tar_path, &bytes).await.unwrap();

        // Target server: same usernames, different underlying ids.
        let target = repo().await;
        let target_gm = target.create_user("gm3", None, ServerRole::User, 0).await.unwrap();
        let target_owner = target
            .create_user("actor-owner", None, ServerRole::User, 0)
            .await
            .unwrap();
        assert_ne!(target_gm, gm);
        assert_ne!(target_owner, owner);

        let import_tmp = tempfile::tempdir().unwrap();
        let import_data =
            crate::world_bundle::read_bundle(&tar_path, import_tmp.path()).unwrap();
        let summary = target.import_world(import_data).await.unwrap();

        assert_eq!(summary.world_id, w.id);
        assert_eq!(summary.skipped_members, 0);
        assert_eq!(summary.skipped_fog, 0);

        // worlds row: id preserved, seq/created_at/updated_at preserved.
        let target_world: (String, i64, i64, i64) =
            sqlx::query_as("SELECT id, seq, created_at, updated_at FROM worlds WHERE id = ?")
                .bind(w.id.to_string())
                .fetch_one(target.pool())
                .await
                .unwrap();
        assert_eq!(target_world.0, w.id.to_string());
        assert_eq!(target_world.1, w.seq);

        // documents: owner re-resolved to the TARGET user's id, both column
        // and JSON body in lockstep.
        let row: (Option<String>, String) =
            sqlx::query_as("SELECT owner_id, json FROM documents WHERE id = ?")
                .bind(doc.id.to_string())
                .fetch_one(target.pool())
                .await
                .unwrap();
        assert_eq!(row.0, Some(target_owner.to_string()));
        let json_doc: serde_json::Value = serde_json::from_str(&row.1).unwrap();
        assert_eq!(
            json_doc.get("owner").and_then(|v| v.as_str()),
            Some(target_owner.to_string().as_str())
        );

        // world_events: command_json byte-identical, author re-resolved.
        let event: (String, Option<String>) =
            sqlx::query_as("SELECT command_json, author_id FROM world_events WHERE world_id = ?")
                .bind(w.id.to_string())
                .fetch_one(target.pool())
                .await
                .unwrap();
        assert_eq!(
            event.0,
            r#"{"kind":"Noop","payload":{"embedded_ref":"deadbeef"}}"#
        );
        assert_eq!(event.1, Some(target_gm.to_string()));

        // assets: storage_key recomputed under the standard scheme, bytes
        // byte-identical after finalize.
        let asset_row = target.get_asset(asset_id).await.unwrap().unwrap();
        assert_eq!(asset_row.storage_key, format!("{}/{asset_id}", w.id));
        let final_path = import_tmp.path().join(w.id.to_string()).join(asset_id.to_string());
        assert_eq!(tokio::fs::read(&final_path).await.unwrap(), b"ASSETBYTES");

        // explored_fog: user re-resolved.
        let fog_user: String =
            sqlx::query_scalar("SELECT user_id FROM explored_fog WHERE scene_id = ?")
                .bind(doc.id.to_string())
                .fetch_one(target.pool())
                .await
                .unwrap();
        assert_eq!(fog_user, target_owner.to_string());
    }

    #[tokio::test]
    async fn import_world_nulls_owner_when_username_unresolvable() {
        let src = repo().await;
        let gm = src.create_user("gm4", None, ServerRole::User, 0).await.unwrap();
        let owner = src
            .create_user("owner-not-on-target", None, ServerRole::User, 0)
            .await
            .unwrap();
        let w = src.create_world_owned("W4", gm, 0).await.unwrap();
        let mut doc = world_doc(11, w.id, serde_json::json!({}));
        doc.owner = Some(owner);
        let mut conn = src.pool().acquire().await.unwrap();
        SqliteRepository::upsert_document(&mut conn, &doc, 1).await.unwrap();
        drop(conn);

        let export_data = src.export_world_rows(w.id).await.unwrap();

        // Target has neither `gm4` nor `owner-not-on-target` — but DOES have
        // a distinct user seated as the sole GM via `worlds` insert directly
        // (import_world does not require any pre-existing user).
        let target = repo().await;
        let import_data = crate::data::world_bundle::WorldImportData {
            manifest: export_data.manifest.clone(),
            documents: export_data.documents.clone(),
            events: export_data.events.clone(),
            members: export_data.members.clone(),
            invites: export_data.invites.clone(),
            assets: export_data.assets.clone(),
            fog: export_data.fog.clone(),
            settings: export_data.settings.clone(),
            staged_assets: Vec::new(),
        };
        let summary = target.import_world(import_data).await.unwrap();
        // `gm4` (the sole world_members row) also doesn't exist on target.
        assert_eq!(summary.skipped_members, 1);

        let row: (Option<String>, String) =
            sqlx::query_as("SELECT owner_id, json FROM documents WHERE id = ?")
                .bind(doc.id.to_string())
                .fetch_one(target.pool())
                .await
                .unwrap();
        assert_eq!(row.0, None);
        let json_doc: serde_json::Value = serde_json::from_str(&row.1).unwrap();
        assert!(json_doc.get("owner").unwrap().is_null());
    }

    #[tokio::test]
    async fn import_world_rejects_world_id_collision_before_writing_any_row() {
        let r = repo().await;
        let gm = r.create_user("gm5", None, ServerRole::User, 0).await.unwrap();
        let w = r.create_world_owned("Collider", gm, 0).await.unwrap();
        let doc = world_doc(12, w.id, serde_json::json!({}));
        let mut conn = r.pool().acquire().await.unwrap();
        SqliteRepository::upsert_document(&mut conn, &doc, 1).await.unwrap();
        drop(conn);

        let export_data = r.export_world_rows(w.id).await.unwrap();
        let import_data = crate::data::world_bundle::WorldImportData {
            manifest: export_data.manifest.clone(),
            documents: export_data.documents.clone(),
            events: export_data.events.clone(),
            members: export_data.members.clone(),
            invites: export_data.invites.clone(),
            assets: export_data.assets.clone(),
            fog: export_data.fog.clone(),
            settings: export_data.settings.clone(),
            staged_assets: Vec::new(),
        };

        let err = r.import_world(import_data).await.unwrap_err();
        assert!(matches!(err, DataError::Conflict(_)));

        // Zero partial state: still exactly the one original document, not two.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM documents WHERE world_id = ?")
            .bind(w.id.to_string())
            .fetch_one(r.pool())
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
```

**Step 4 — verify.** `cargo test -p shadowcat data::sqlite::tests::import_world` (and the sibling
`round_trips`/`nulls_owner`/`rejects_world_id_collision` test names) passes; `cargo fmt
--check`/`clippy` pass.

**Interfaces introduced:** `data::sqlite::SqliteRepository::import_world(&self, data:
WorldImportData) -> Result<ImportSummary, DataError>`.

---

### Task 7 — `POST /api/worlds/import` HTTP route

**Files:**
- `C:\Dev\Shadowcat\src\server\src\http\world_bundle.rs` (edit — add the import handler)
- `C:\Dev\Shadowcat\src\server\src\http\mod.rs` (edit — register the route)

**Step 1 — extend `http/world_bundle.rs`.**

Add these imports to the existing `use` block at the top of the file:

```rust
use axum::extract::Multipart;
use axum::Json;
use tokio::io::AsyncWriteExt;

use crate::auth::session::AdminUser;
use crate::data::world_bundle::ImportSummary;
use crate::world_bundle::{read_bundle, WorldBundleError};
```

Append the following to the file (after `export_world`):

```rust
/// Defensive cap on an uploaded bundle's total byte size — this endpoint has
/// no per-user rate limit (server-admin-only, not a hot path), so an
/// unbounded upload would still be an uncapped-disk-write vector even for a
/// trusted admin session. Generous: a world's assets can legitimately be
/// large.
const MAX_IMPORT_BUNDLE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Stream the multipart "file" field to `dest`, enforcing
/// `MAX_IMPORT_BUNDLE_BYTES` as bytes arrive (never buffering the whole
/// body). On any failure the partial file is removed.
async fn stream_bundle_upload(
    mut multipart: Multipart,
    dest: &std::path::Path,
) -> Result<(), AppError> {
    let mut field = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
        .ok_or_else(|| AppError::BadRequest("missing file field".into()))?;
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|_| AppError::Internal)?;
    let mut total: u64 = 0;
    loop {
        let chunk = match field.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                let _ = tokio::fs::remove_file(dest).await;
                return Err(AppError::BadRequest(format!("multipart error: {e}")));
            }
        };
        total += chunk.len() as u64;
        if total > MAX_IMPORT_BUNDLE_BYTES {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(AppError::PayloadTooLarge(format!(
                "bundle exceeds {MAX_IMPORT_BUNDLE_BYTES} bytes"
            )));
        }
        if file.write_all(&chunk).await.is_err() {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(AppError::Internal);
        }
    }
    file.flush().await.map_err(|_| AppError::Internal)?;
    Ok(())
}

/// `POST /api/worlds/import` — server-admin-only multipart upload of a
/// `.tar` bundle. Streams the upload to a local temp file first (never
/// buffers the whole body), extracts it (schema-version- and
/// row-count-checked before any DB row is touched), then inserts everything
/// in one transaction. See `SqliteRepository::import_world` for the
/// collision-reject/username-resolution/asset-finalize behavior.
pub async fn import_world(
    _admin: AdminUser,
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Json<ImportSummary>, AppError> {
    let tmp_tar = std::env::temp_dir().join(format!("shadowcat-import-{}.tar", Uuid::new_v4()));
    stream_bundle_upload(multipart, &tmp_tar).await?;

    let assets_dir = state.config.assets_path();
    let tar_path = tmp_tar.clone();
    let import_result = tokio::task::spawn_blocking(move || read_bundle(&tar_path, &assets_dir))
        .await
        .map_err(|e| {
            tracing::error!(?e, "world import extraction task panicked");
            AppError::Internal
        })?;
    let _ = tokio::fs::remove_file(&tmp_tar).await;

    let import_data = match import_result {
        Ok(d) => d,
        Err(e) => {
            return Err(match e {
                WorldBundleError::Malformed(m) => AppError::BadRequest(m),
                WorldBundleError::RowCountMismatch {
                    table,
                    expected,
                    actual,
                } => AppError::BadRequest(format!(
                    "row count mismatch for '{table}': expected {expected}, got {actual}"
                )),
                WorldBundleError::UnsupportedSchemaVersion(v) => {
                    AppError::BadRequest(format!("unsupported bundle schema_version {v}"))
                }
                WorldBundleError::Serde(e) => {
                    AppError::BadRequest(format!("malformed bundle content: {e}"))
                }
                WorldBundleError::Io(e) => {
                    tracing::error!(?e, "world import extraction I/O error");
                    AppError::BadRequest(format!("bundle read error: {e}"))
                }
            });
        }
    };

    let summary = state.repo.import_world(import_data).await?;
    Ok(Json(summary))
}
```

**Step 2 — register the route.**

In `C:\Dev\Shadowcat\src\server\src\http\mod.rs`'s `router()` function, add directly after the
`/api/worlds/{id}/export` route added in Task 4:

```rust
        .route(
            "/api/worlds/import",
            post(world_bundle::import_world).layer(DefaultBodyLimit::disable()),
        )
```

**Step 3 — tests.**

Add to `http/mod.rs`'s test module:

```rust
    #[tokio::test]
    async fn import_world_requires_server_admin() {
        let server = server_with_user("non-admin", "pw-na", ServerRole::User).await;
        server
            .post("/api/login")
            .json(&serde_json::json!({"username": "non-admin", "password": "pw-na"}))
            .await
            .assert_status_success();
        server
            .post("/api/worlds/import")
            .multipart(
                reqwest::multipart::Form::new()
                    .part("file", reqwest::multipart::Part::bytes(b"junk".to_vec())),
            )
            .await
            .assert_status(axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn import_world_round_trips_over_http() {
        let server = server_with_user("admin-io", "pw-admin-io", ServerRole::Admin).await;
        server
            .post("/api/login")
            .json(&serde_json::json!({"username": "admin-io", "password": "pw-admin-io"}))
            .await
            .assert_status_success();
        let world: crate::data::document::World = server
            .post("/api/worlds")
            .json(&serde_json::json!({"name": "HTTP Round Trip"}))
            .await
            .json();

        let export_resp = server
            .post(&format!("/api/worlds/{}/export", world.id))
            .await;
        export_resp.assert_status_ok();
        let bundle_bytes = export_resp.into_bytes();

        // Delete the world so import onto the SAME server does not collide.
        server
            .delete(&format!("/api/worlds/{}", world.id))
            .await
            .assert_status_success();

        let import_resp = server
            .post("/api/worlds/import")
            .multipart(
                reqwest::multipart::Form::new().part(
                    "file",
                    reqwest::multipart::Part::bytes(bundle_bytes.to_vec())
                        .file_name("world.tar"),
                ),
            )
            .await;
        import_resp.assert_status_ok();
        let summary: crate::data::world_bundle::ImportSummary = import_resp.json();
        assert_eq!(summary.world_id, world.id);
    }
```

**Step 4 — verify.** `cargo test -p shadowcat http::tests::import_world` passes; a full `cargo test
-p shadowcat` run stays green; `cargo fmt --check`/`clippy` pass.

**Interfaces introduced:** `http::world_bundle::import_world` handler; route `POST
/api/worlds/import`.

---

### Task 8 — Documentation sync (completion gate)

Per project `CLAUDE.md`, this is not optional cleanup — the plan is not complete until this lands,
reviewed by `shadowcat-spec-reviewer`.

**Files:**
- `C:\Dev\Shadowcat\docs\design\ARCHITECTURE.md` (edit)
- `C:\Dev\Shadowcat\.claude\skills\shadowcat-codebase-server-ops\SKILL.md` (edit)

**Step 1 — `ARCHITECTURE.md`: remove the now-false "deferred" row.**

In the "§4 Deferred behind abstractions" table, delete this line entirely (it no longer describes a
deferred feature — see this plan's "Spec corrections applied" section above for why the row is
removed rather than rewritten to reference a still-deferred concern):

```
| Bulk import/export (assets + documents) | document CRUD | Phase 2. |
```

**Step 2 — `shadowcat-codebase-server-ops` skill: correct the Gotchas entry + add a Key files
bullet.**

Replace the existing Gotchas bullet (currently reading):

```
- Per-world granular export/import is explicitly OUT of scope — the backup/restore surface ships
  whole-server snapshot/restore only (single shared `shadowcat.db` across all worlds); per-world
  would need to preserve referential integrity across cross-table FKs and shared asset
  references, real complexity not currently implemented.
```

with:

```
- Per-world export/import ships as a SEPARATE surface from `backup`/`restore_backup` — not
  whole-server snapshot/restore, and not gated the same way. `POST /api/worlds/{id}/export` is
  world-GM-gated (`require_gm`); `POST /api/worlds/import` is server-admin-only (a bulk multi-table
  insert bypassing every capability/schema/OCC gate the live write paths enforce — more privileged
  than ordinary world CREATION, which is open to any authenticated user, not admin-gated). World id
  is preserved verbatim on import; a colliding id refuses cleanly before any row is written.
  `users(id)` references export as portable usernames (the source server's `users` table itself is
  never exported) — resolved back to a target-local id, or `NULL`/row-drop for the two `NOT NULL`
  user columns with no `SET NULL` degradation (`world_members.user_id`/`explored_fog.user_id`),
  only at import time.
```

Add a new bullet to "Key files & seams", directly after the existing `backup` bullet:

```
- `world_bundle` (top-level, pure tar I/O, mirrors `backup`'s no-`AppState`-dependency separation)
  — `write_bundle`/`read_bundle` build/parse the `.tar` bundle format (`manifest.json` +
  `rows/<table>.jsonl` + `assets/<asset_id>`); `data::world_bundle` holds the row/manifest DTOs
  (`BundleManifest`, `Exported*Row`, `WorldExportData`/`WorldImportData`, `ImportSummary`) plus
  `BUNDLE_SCHEMA_VERSION`. `data::sqlite::SqliteRepository::export_world_rows`/`import_world` are
  the DB-facing halves — `import_world` rejects a world-id collision before any row is written,
  inserts `worlds` then every table `delete_world` already walks (read instead of deleted) in
  FK-safe order, and finalizes staged asset files only after every row is accepted.
  `http::world_bundle::export_world`/`import_world` are the two routes.
```

**Step 3 — reviewed-skill-update gate.** Dispatch `shadowcat-spec-reviewer` on the `ARCHITECTURE.md`
+ `shadowcat-codebase-server-ops` skill diff (per project `CLAUDE.md`'s "Reviewed Skill-Update
Gate") to confirm the diff accurately captures the shipped change with no omission or drift, before
merge.

**Step 4 — plugin refresh.** If `.claude/.claude-plugin/plugin.json` versions this plugin, bump
`version` per project `CLAUDE.md`'s "Plugin refresh" obligation, and re-run the marketplace update
in any consuming repository.

**Interfaces:** none (docs only).

---

## Self-Review Checklist (completed by the plan writer before handoff)

- **Spec coverage:** every numbered section of the design doc (§1 scope, §2 table scope, §3 world
  identity, §4 user-identity resolution, §5 asset bytes, §6 bundle format, §7 HTTP surface/authz,
  §8 testing, §9 documentation, §10 non-goals) is covered by at least one task above. §8's five
  named tests (round-trip, username-resolution, collision, asset round-trip, command_json verbatim)
  are all present in Task 6's test list.
- **Placeholder scan:** no task contains "TODO", "TBD", "..." as a stand-in for real code, or a step
  that describes an action without showing the exact code/text change.
- **Type consistency:** `WorldExportData`/`WorldImportData`/every `Exported*Row` type is defined
  once (Task 1) and used identically in every later task; `ImportSummary` is defined once and
  returned/consumed identically in Tasks 6 and 7; `WorldBundleError` is defined once (Task 3) and
  matched exhaustively in Task 7's HTTP mapping.
