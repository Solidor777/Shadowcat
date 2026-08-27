//! Whole-server backup/restore: SQLite `VACUUM INTO` snapshot + assets-directory
//! copy + a `manifest.json` for round-trip sanity checks. Pure file I/O plus one
//! SQL statement — no dependency on `AppState`/`SqliteRepository`, so a fresh
//! short-lived connection is opened directly against the caller-resolved db path.
//!
//! INVARIANT: the DB snapshot (`VACUUM INTO`) always completes before the assets
//! copy starts. Asset uploads write bytes to disk BEFORE inserting the
//! referencing DB row (`http::assets::upload`), and asset files are never deleted
//! except by explicit delete — so db-snapshot-then-assets-copy guarantees every
//! asset the snapshot's rows reference is already present in the assets copy.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// All fallible backup/restore operations return this.
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

/// Written to `<out_dir>/manifest.json` by [`create_backup`] and validated by
/// `restore_backup` before any destination file is touched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
/// `out_dir` if absent. Does not check `out_dir` for prior contents — callers
/// that need the refuse-non-empty-without-force gate call
/// [`dir_is_empty_or_absent`] first (the CLI layer owns that decision; see
/// `main::run_backup`).
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
) -> Result<BackupManifest, BackupError> {
    tokio::fs::create_dir_all(out_dir).await?;

    let db_out = out_dir.join("world.db");
    // VACUUM INTO refuses to write into an already-existing file.
    if tokio::fs::metadata(&db_out).await.is_ok() {
        tokio::fs::remove_file(&db_out).await?;
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
/// assets tree — and the real destination is only ever touched by `rename`,
/// never by an in-place write. `std::fs::rename` atomically replaces an
/// existing FILE on all three target OSes, so the db swap is a single rename.
/// A directory rename is NOT a replace on any of the three OSes when the
/// destination is non-empty, so the assets swap is two renames: the old
/// `assets_dir` is first renamed out of the way to `<assets_dir>.restore-old`,
/// then the staged tree is renamed into `assets_dir`, then the old tree is
/// removed. Net effect: a failure at any point leaves `assets_dir` either
/// fully pre-restore (old content, never touched) or fully post-restore
/// (staged content already swapped in) — never a partial mix. Worst case on a
/// crash between the two renames: the old tree is parked at
/// `<assets_dir>.restore-old` rather than deleted; the next restore attempt
/// clears it (see the pre-clear below), so no manual recovery step is
/// required, though the parked directory is recoverable by hand if needed.
///
/// The db swap and the assets swap are two INDEPENDENT atomic operations, not
/// one joint transaction across both artifacts: the db rename completes in
/// full before the assets copy/swap begins. A crash in that window leaves a
/// new db paired with the old (or, mid-assets-swap, momentarily absent)
/// assets directory; recovery is re-running `restore_backup` with `force`
/// (the db swap already completed, so `db_path` now exists and a
/// force-less retry would refuse), which re-copies the db and completes the
/// assets swap, leaving both artifacts consistent.
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
    tokio::fs::copy(&backup_db, &db_stage).await?;
    tokio::fs::rename(&db_stage, db_path).await?;

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
        tokio::fs::remove_dir_all(&assets_stage).await?;
    }
    if tokio::fs::metadata(&assets_old).await.is_ok() {
        tokio::fs::remove_dir_all(&assets_old).await?;
    }
    let backup_assets = backup_dir.join("assets");
    copy_dir_recursive(&backup_assets, &assets_stage).await?;
    if tokio::fs::metadata(assets_dir).await.is_ok() {
        tokio::fs::rename(assets_dir, &assets_old).await?;
    }
    tokio::fs::rename(&assets_stage, assets_dir).await?;
    if tokio::fs::metadata(&assets_old).await.is_ok() {
        tokio::fs::remove_dir_all(&assets_old).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
