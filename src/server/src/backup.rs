//! This module backs up and restores a whole server: a SQLite `VACUUM INTO` snapshot, a copy of the
//! assets directory and a `manifest.json` for round-trip sanity checks.
//!
//! It is pure file I/O plus one SQL statement, with no dependency on `AppState` or
//! `SqliteRepository`, so a fresh short-lived connection is opened directly against the
//! caller-resolved db path.
//!
//! INVARIANT: the DB snapshot (`VACUUM INTO`) always completes before the assets copy starts. Asset
//! uploads write bytes to disk BEFORE inserting the referencing DB row (`http::assets::upload`), and
//! asset files are never deleted except by explicit delete or by the rollback of an upload whose
//! row was never inserted, so db-snapshot-then-assets-copy guarantees every asset the snapshot's
//! rows reference is already present in the assets copy.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use crate::trash::Trash;

/// All fallible backup/restore operations return this.
///
/// # Examples
///
/// ```
/// use shadowcat::backup::BackupError;
///
/// let err = BackupError::DestinationNotEmpty("assets".to_string());
/// assert_eq!(
///     err.to_string(),
///     "refusing to write into non-empty directory assets without --force"
/// );
/// ```
#[derive(Debug, Error)]
pub enum BackupError {
    /// Filesystem operation failed (copy, rename, read_dir, ...).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// The `VACUUM INTO` snapshot connection or statement failed.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// `manifest.json` could not be serialized at backup-write time.
    #[error("manifest serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    /// Restore refused without `force`: the destination db file already
    /// exists, or the destination assets directory exists and is non-empty.
    #[error("refusing to write into non-empty directory {0} without --force")]
    DestinationNotEmpty(String),
    /// The source directory fails pre-restore validation (missing/malformed
    /// `manifest.json` or missing `world.db`) — nothing was touched.
    #[error("{0} is not a valid backup directory: {1}")]
    InvalidBackupDir(String, String),
}

/// Why a backup was taken. Recorded in `manifest.json` so the backup list can label it and a
/// later scheduler can filter its own runs.
///
/// # Examples
///
/// ```
/// use shadowcat::backup::BackupKind;
///
/// assert_eq!(serde_json::to_string(&BackupKind::PreRestore).unwrap(), "\"pre-restore\"");
/// assert_eq!(BackupKind::default(), BackupKind::Manual);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub enum BackupKind {
    /// An admin or CLI request.
    #[default]
    #[serde(rename = "manual")]
    #[ts(rename = "manual")]
    Manual,
    /// The safety backup taken just before a restore is staged.
    #[serde(rename = "pre-restore")]
    #[ts(rename = "pre-restore")]
    PreRestore,
    /// A timer-driven backup.
    #[serde(rename = "scheduled")]
    #[ts(rename = "scheduled")]
    Scheduled,
}

/// Written to `<out_dir>/manifest.json` by [`create_backup`] and validated by
/// `restore_backup` before any destination file is touched. `kind` and `note` default when
/// absent, so a manifest written before they existed still parses.
///
/// # Examples
///
/// ```
/// use shadowcat::backup::{BackupKind, BackupManifest};
///
/// let manifest = BackupManifest {
///     shadowcat_version: "0.1.0".to_string(),
///     created_at_unix_ms: 0,
///     source_db: "shadowcat.db".to_string(),
///     source_assets_dir: "assets".to_string(),
///     asset_file_count: 3,
///     db_bytes: 4096,
///     kind: BackupKind::Manual,
///     note: None,
/// };
/// let json = serde_json::to_string(&manifest).unwrap();
/// let round_tripped: BackupManifest = serde_json::from_str(&json).unwrap();
/// assert_eq!(round_tripped, manifest);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct BackupManifest {
    /// Server version that wrote the backup (shown when restoring later).
    pub shadowcat_version: String,
    /// Backup creation time, Unix epoch milliseconds.
    pub created_at_unix_ms: u64,
    /// The db path the snapshot was taken from (informational, not re-resolved).
    pub source_db: String,
    /// The assets root the copy was taken from (informational).
    pub source_assets_dir: String,
    /// Files copied into the backup's assets tree; echoed in the CLI summary line.
    pub asset_file_count: u64,
    /// Byte size of the `world.db` snapshot; echoed in the CLI summary line.
    pub db_bytes: u64,
    /// Why the backup was taken.
    #[serde(default)]
    pub kind: BackupKind,
    /// Free-text label the requester attached, if any.
    #[serde(default)]
    pub note: Option<String>,
}

/// True when `path` does not exist, or exists as an empty directory. A path
/// that exists as a non-directory (e.g. a file) is not "empty or absent" and
/// surfaces as an I/O error from the underlying `read_dir` call.
///
/// # Examples
///
/// ```
/// use shadowcat::backup::dir_is_empty_or_absent;
///
/// let missing = std::path::Path::new("no-such-dir-usr-test-001");
/// assert!(dir_is_empty_or_absent(missing).expect("absent is not an error"));
/// ```
pub fn dir_is_empty_or_absent(path: &Path) -> std::io::Result<bool> {
    match std::fs::read_dir(path) {
        Ok(mut entries) => Ok(entries.next().is_none()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e),
    }
}

/// True when `path` does not exist. Used to gate overwrite of the single-file
/// restore destination (`world.db`).
///
/// # Examples
///
/// ```text
/// file_absent(Path::new("shadowcat.db"))? == false  // live server dir
/// ```
fn file_absent(path: &Path) -> std::io::Result<bool> {
    match std::fs::metadata(path) {
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e),
    }
}

/// Recursively copy every file under `src` into `dst` (creating directories as
/// needed) and return the number of files copied. A missing `src` copies zero
/// files and still creates an empty `dst` — a fresh install may have no assets
/// directory yet. Symlinks are skipped: the assets tree is server-managed and
/// never contains one, so silently skipping avoids following into an
/// unexpected target rather than guessing at semantics.
///
/// # Examples
///
/// ```text
/// let copied = copy_dir_recursive(&assets_src, &out.join("assets")).await?; // -> file count
/// ```
fn copy_dir_recursive<'a>(
    src: &'a Path,
    dst: &'a Path,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u64, BackupError>> + Send + 'a>> {
    Box::pin(async move {
        tokio::fs::create_dir_all(dst).await?;
        let mut entries = match tokio::fs::read_dir(src).await {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.into()),
        };
        let mut count = 0u64;
        while let Some(entry) = entries.next_entry().await? {
            let file_type = entry.file_type().await?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if file_type.is_dir() {
                count += copy_dir_recursive(&src_path, &dst_path).await?;
            } else if file_type.is_file() {
                tokio::fs::copy(&src_path, &dst_path).await?;
                count += 1;
            }
        }
        Ok(count)
    })
}

/// Snapshot `db_path` (via `VACUUM INTO`, an atomic consistency-guaranteed copy
/// safe against a live writer) and recursively copy `assets_dir` into
/// `out_dir`, in that order, then write `out_dir/manifest.json` last. Creates
/// `out_dir` if absent. A leftover snapshot file in `out_dir` moves into `trash`
/// rather than being removed. Does not check `out_dir` for prior contents — callers
/// that need the refuse-non-empty-without-force gate call
/// [`dir_is_empty_or_absent`] first (the CLI layer owns that decision; see
/// `main::run_backup`). `kind` and `note` are recorded in the manifest.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// # async fn demo() -> Result<(), shadowcat::backup::BackupError> {
/// let manifest = shadowcat::backup::create_backup(
///     Path::new("shadowcat.db"),
///     Path::new("assets"),
///     Path::new("backups/2026-07-30"),
///     &shadowcat::trash::Trash::new(
///         std::path::PathBuf::from("trash"),
///         shadowcat::trash::TrashRetention::default(),
///     ),
///     shadowcat::backup::BackupKind::Manual,
///     None,
/// )
/// .await?;
/// println!("{} asset file(s), {} db bytes", manifest.asset_file_count, manifest.db_bytes);
/// # Ok(())
/// # }
/// ```
pub async fn create_backup(
    db_path: &Path,
    assets_dir: &Path,
    out_dir: &Path,
    trash: &Trash,
    kind: BackupKind,
    note: Option<String>,
) -> Result<BackupManifest, BackupError> {
    tokio::fs::create_dir_all(out_dir).await?;

    let db_out = out_dir.join("world.db");
    // VACUUM INTO refuses to write into an already-existing file.
    if tokio::fs::metadata(&db_out).await.is_ok() {
        trash.discard(&db_out).await?;
    }
    let db_url = format!("sqlite://{}", db_path.to_string_lossy());
    let pool = crate::db::connect_pool(&db_url).await?;
    // Standard SQL string-literal escaping (doubled single quotes) rather than
    // a bound parameter in the VACUUM INTO filename position — the safer,
    // universally-supported form across sqlite driver versions. The only
    // attacker-adjacent input here is a filesystem path derived from our own
    // `out_dir` joins, not untrusted SQL.
    let db_out_escaped = db_out.to_string_lossy().replace('\'', "''");
    // `AssertSqlSafe`: sqlx 0.9's `SqlSafeStr` bound rejects ad hoc `String`s at
    // the `query()` call site to force an explicit audit. This string is
    // manually audited above (doubled-quote escaping of a filesystem path we
    // constructed ourselves, not attacker-controlled SQL).
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "VACUUM INTO '{db_out_escaped}'"
    )))
    .execute(&pool)
    .await?;
    pool.close().await;
    let db_bytes = tokio::fs::metadata(&db_out).await?.len();

    let assets_out = out_dir.join("assets");
    let asset_file_count = copy_dir_recursive(assets_dir, &assets_out).await?;

    let manifest = BackupManifest {
        shadowcat_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at_unix_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        source_db: db_path.to_string_lossy().to_string(),
        source_assets_dir: assets_dir.to_string_lossy().to_string(),
        asset_file_count,
        db_bytes,
        kind,
        note,
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    tokio::fs::write(out_dir.join("manifest.json"), manifest_json).await?;

    Ok(manifest)
}

/// Restores a `backup_dir` matching the [`create_backup`] output layout:
/// validates `manifest.json` and `world.db` are present (fails closed on a
/// missing/malformed/foreign directory before touching any destination file),
/// then copies `backup_dir/world.db` to `db_path` and `backup_dir/assets/` to
/// `assets_dir`. Without `force`, refuses when `db_path` already exists or
/// `assets_dir` already exists and is non-empty. With `force`, the destination
/// assets directory is fully replaced rather than merged, so restore is a true
/// point-in-time reset, not an overlay. Never starts the server — callers own
/// that separation.
///
/// Stage-then-swap: every fallible copy lands in a sibling staging path first
/// — `<db_path>.restore-tmp` for the db, `<assets_dir>.restore-tmp` for the
/// assets tree — before any live path is touched, and the real destinations are
/// only ever touched by `rename`, never by an in-place write. A directory
/// rename is NOT a replace on any of the three OSes when the destination is
/// non-empty, so the assets swap is two renames: the old `assets_dir` moves to
/// `<assets_dir>.restore-old`, then the staged tree moves into `assets_dir`.
/// The db swap comes last: `std::fs::rename` atomically replaces an existing
/// FILE on all three target OSes (and fails, leaving the destination alone,
/// when it is a directory), so it is a single all-or-nothing step.
///
/// On return the live database and assets are either both the backup's or both
/// what they were: a failure in the copies leaves both untouched, and a failure
/// in the db rename moves the new assets tree aside and the old one back in. The
/// replaced assets tree is only moved into `trash` after the db rename
/// succeeded; every staging leftover also moves into `trash` rather than being
/// removed. The undo is two renames of directories the restore itself just
/// moved on the same filesystem, so it relies on nothing the forward swap did
/// not. Only a crash (not an error) between the final renames can leave
/// `<assets_dir>.restore-old` parked; the next restore attempt clears it, and
/// recovery is re-running `restore_backup` with `force`.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// # async fn demo() -> Result<(), shadowcat::backup::BackupError> {
/// // Overwrites an existing db/assets pair only because force = true.
/// shadowcat::backup::restore_backup(
///     Path::new("backups/2026-07-30"),
///     Path::new("shadowcat.db"),
///     Path::new("assets"),
///     true,
///     &shadowcat::trash::Trash::new(
///         std::path::PathBuf::from("trash"),
///         shadowcat::trash::TrashRetention::default(),
///     ),
/// )
/// .await?;
/// # Ok(())
/// # }
/// ```
pub async fn restore_backup(
    backup_dir: &Path,
    db_path: &Path,
    assets_dir: &Path,
    force: bool,
    trash: &Trash,
) -> Result<(), BackupError> {
    let manifest_path = backup_dir.join("manifest.json");
    let manifest_bytes = tokio::fs::read(&manifest_path).await.map_err(|_| {
        BackupError::InvalidBackupDir(
            backup_dir.display().to_string(),
            "missing manifest.json".to_string(),
        )
    })?;
    let _manifest: BackupManifest = serde_json::from_slice(&manifest_bytes).map_err(|e| {
        BackupError::InvalidBackupDir(
            backup_dir.display().to_string(),
            format!("malformed manifest.json: {e}"),
        )
    })?;

    let backup_db = backup_dir.join("world.db");
    if tokio::fs::metadata(&backup_db).await.is_err() {
        return Err(BackupError::InvalidBackupDir(
            backup_dir.display().to_string(),
            "missing world.db".to_string(),
        ));
    }

    if !force {
        if !file_absent(db_path)? {
            return Err(BackupError::DestinationNotEmpty(
                db_path.display().to_string(),
            ));
        }
        if !dir_is_empty_or_absent(assets_dir)? {
            return Err(BackupError::DestinationNotEmpty(
                assets_dir.display().to_string(),
            ));
        }
    }

    if let Some(parent) = db_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let db_stage = db_path.with_extension("restore-tmp");
    let assets_name = assets_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "assets".to_string());
    let assets_parent = assets_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let assets_stage = assets_parent.join(format!("{assets_name}.restore-tmp"));
    let assets_old = assets_parent.join(format!("{assets_name}.restore-old"));
    // Both staging paths must be clear before copying in: a prior interrupted
    // restore can leave either one behind, and `copy_dir_recursive` merges
    // into an existing directory rather than replacing it.
    if tokio::fs::metadata(&assets_stage).await.is_ok() {
        trash.discard(&assets_stage).await?;
    }
    if tokio::fs::metadata(&assets_old).await.is_ok() {
        trash.discard(&assets_old).await?;
    }
    if tokio::fs::metadata(&db_stage).await.is_ok() {
        trash.discard(&db_stage).await?;
    }

    // Every fallible copy happens before any live path is touched.
    let staged = stage_restore_copies(backup_dir, &backup_db, &db_stage, &assets_stage).await;
    if let Err(e) = staged {
        discard_staged(trash, &[&db_stage, &assets_stage]).await;
        return Err(e);
    }

    let had_assets = tokio::fs::metadata(assets_dir).await.is_ok();
    if had_assets {
        if let Err(e) = tokio::fs::rename(assets_dir, &assets_old).await {
            discard_staged(trash, &[&db_stage, &assets_stage]).await;
            return Err(e.into());
        }
    }
    if let Err(e) = tokio::fs::rename(&assets_stage, assets_dir).await {
        if had_assets {
            let _ = tokio::fs::rename(&assets_old, assets_dir).await;
        }
        discard_staged(trash, &[&db_stage, &assets_stage]).await;
        return Err(e.into());
    }
    // The database rename is the last, single, atomic step: if it fails the live database is
    // untouched, and the assets swap above is undone by moving the new tree back out of the way
    // and the old one back in.
    if let Err(e) = tokio::fs::rename(&db_stage, db_path).await {
        let _ = tokio::fs::rename(assets_dir, &assets_stage).await;
        if had_assets {
            let _ = tokio::fs::rename(&assets_old, assets_dir).await;
        }
        discard_staged(trash, &[&db_stage, &assets_stage]).await;
        return Err(e.into());
    }
    if had_assets {
        trash.discard(&assets_old).await?;
    }

    Ok(())
}

/// Copies the backup's database to `db_stage` and its assets tree to `assets_stage`; touches no
/// live path.
///
/// # Examples
///
/// ```text
/// stage_restore_copies(&backup_dir, &backup_dir.join("world.db"), &db_stage, &assets_stage).await?;
/// ```
async fn stage_restore_copies(
    backup_dir: &Path,
    backup_db: &Path,
    db_stage: &Path,
    assets_stage: &Path,
) -> Result<(), BackupError> {
    tokio::fs::copy(backup_db, db_stage).await?;
    copy_dir_recursive(&backup_dir.join("assets"), assets_stage).await?;
    Ok(())
}

/// Moves each existing staging path into `trash`; a path that is absent or cannot be moved is
/// skipped, because this runs on a restore that is already failing and must report that error.
///
/// # Examples
///
/// ```text
/// discard_staged(&trash, &[&db_stage, &assets_stage]).await;
/// ```
async fn discard_staged(trash: &Trash, paths: &[&Path]) {
    for path in paths {
        if tokio::fs::metadata(path).await.is_ok() {
            let _ = trash.discard(path).await;
        }
    }
}

/// The file name of the failed-restore marker, written beside the database file when a staged
/// restore could not be applied at startup.
pub const RESTORE_FAILED_FILE: &str = "restore-failed.json";

/// What a failed staged restore leaves for the admin list: the backup that was requested and why
/// it was not applied.
///
/// # Examples
///
/// ```
/// use shadowcat::backup::RestoreFailed;
///
/// let marker = RestoreFailed { id: "backup-1".into(), requested_at_ms: 5, error: "no such backup".into() };
/// let json = serde_json::to_string(&marker).unwrap();
/// assert_eq!(serde_json::from_str::<RestoreFailed>(&json).unwrap(), marker);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct RestoreFailed {
    /// The requested backup directory name; empty when the pending marker itself was unreadable.
    pub id: String,
    /// When the restore was requested, Unix epoch milliseconds; `0` when unreadable.
    pub requested_at_ms: u64,
    /// The error text that stopped the restore.
    pub error: String,
}

/// The result of [`apply_pending_restore`].
///
/// # Examples
///
/// ```
/// use shadowcat::backup::PendingRestore;
///
/// assert_eq!(PendingRestore::Absent, PendingRestore::Absent);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingRestore {
    /// No `restore-pending.json` marker exists; nothing was touched.
    Absent,
    /// The named backup was restored over the database and assets, and the marker was removed.
    Applied(String),
    /// The restore failed; `restore-failed.json` records why and the marker was removed.
    Failed(String),
}

/// Applies a restore the admin route staged: reads `restore-pending.json` beside `db_path`, and
/// when present restores that backup from `backups_root` with `force = true`, then moves the
/// marker into `trash`. A failure (unreadable marker, unknown id, invalid backup, copy error)
/// writes `restore-failed.json` and also moves the marker into `trash`, so the next start does
/// not retry. Must run before the database pool opens: a restore swaps the database file, which
/// a live connection cannot survive (Windows refuses the rename outright). A backup that fails
/// validation leaves the live database untouched, because `restore_backup` validates before it
/// writes a destination file.
///
/// # Errors
///
/// An unreadable marker file, or a failure to write the failed marker or to discard the pending
/// marker. The last case aborts startup rather than risk applying the same restore twice.
///
/// # Examples
///
/// ```
/// # #[tokio::main] async fn main() {
/// use shadowcat::backup::{apply_pending_restore, PendingRestore};
/// use shadowcat::trash::{Trash, TrashRetention};
///
/// let dir = tempfile::tempdir().unwrap();
/// let trash = Trash::new(dir.path().join("trash"), TrashRetention::default());
/// let outcome = apply_pending_restore(
///     &dir.path().join("shadowcat.db"),
///     &dir.path().join("assets"),
///     &dir.path().join("backups"),
///     &trash,
/// )
/// .await
/// .unwrap();
/// assert_eq!(outcome, PendingRestore::Absent);
/// # }
/// ```
pub async fn apply_pending_restore(
    db_path: &Path,
    assets_dir: &Path,
    backups_root: &Path,
    trash: &Trash,
) -> Result<PendingRestore, BackupError> {
    let marker_path = catalog::restore_pending_path(db_path);
    let bytes = match tokio::fs::read(&marker_path).await {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(PendingRestore::Absent),
        Err(e) => return Err(e.into()),
    };
    let failed_path = db_path.with_file_name(RESTORE_FAILED_FILE);
    let outcome: Result<String, RestoreFailed> =
        match serde_json::from_slice::<catalog::RestorePending>(&bytes) {
            Err(e) => Err(RestoreFailed {
                id: String::new(),
                requested_at_ms: 0,
                error: format!("unreadable {}: {e}", catalog::RESTORE_PENDING_FILE),
            }),
            Ok(pending) => {
                let attempt = match catalog::resolve_backup(backups_root, &pending.id) {
                    Ok(dir) => restore_backup(&dir, db_path, assets_dir, true, trash).await,
                    Err(e) => Err(e),
                };
                match attempt {
                    Ok(()) => Ok(pending.id),
                    Err(e) => Err(RestoreFailed {
                        id: pending.id,
                        requested_at_ms: pending.requested_at_ms,
                        error: e.to_string(),
                    }),
                }
            }
        };
    match outcome {
        Ok(id) => {
            // A restored server starts clean: a failure recorded by an earlier attempt no longer
            // describes the data.
            trash.discard(&failed_path).await?;
            trash.discard(&marker_path).await?;
            Ok(PendingRestore::Applied(id))
        }
        Err(failed) => {
            let staging = failed_path.with_extension("json.tmp");
            tokio::fs::write(&staging, serde_json::to_vec(&failed)?).await?;
            tokio::fs::rename(&staging, &failed_path).await?;
            trash.discard(&marker_path).await?;
            Ok(PendingRestore::Failed(failed.error))
        }
    }
}

pub mod catalog;

#[cfg(test)]
mod tests;
