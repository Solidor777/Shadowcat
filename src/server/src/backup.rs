//! Whole-server backup/restore: SQLite `VACUUM INTO` snapshot + assets-directory
//! copy + a `manifest.json` for round-trip sanity checks. Pure file I/O plus one
//! SQL statement — no dependency on `AppState`/`SqliteRepository`, so a fresh
//! short-lived connection is opened directly against the caller-resolved db path.
//!
//! INVARIANT: the DB snapshot (`VACUUM INTO`) always completes before the assets
//! copy starts. Asset uploads write bytes to disk BEFORE inserting the
//! referencing DB row (`http/assets.rs`), and asset files are never deleted
//! except by explicit delete — so db-snapshot-then-assets-copy guarantees every
//! asset the snapshot's rows reference is already present in the assets copy.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePoolOptions;
use thiserror::Error;

/// All fallible backup/restore operations return this.
#[derive(Debug, Error)]
pub enum BackupError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("manifest serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("refusing to write into non-empty directory {0} without --force")]
    DestinationNotEmpty(String),
    #[error("{0} is not a valid backup directory: {1}")]
    InvalidBackupDir(String, String),
}

/// Written to `<out_dir>/manifest.json` by [`create_backup`] and validated by
/// `restore_backup` before any destination file is touched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupManifest {
    pub shadowcat_version: String,
    pub created_at_unix_ms: u64,
    pub source_db: String,
    pub source_assets_dir: String,
    pub asset_file_count: u64,
    pub db_bytes: u64,
}

/// True when `path` does not exist, or exists as an empty directory. A path
/// that exists as a non-directory (e.g. a file) is not "empty or absent" and
/// surfaces as an I/O error from the underlying `read_dir` call.
pub fn dir_is_empty_or_absent(path: &Path) -> std::io::Result<bool> {
    match std::fs::read_dir(path) {
        Ok(mut entries) => Ok(entries.next().is_none()),
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
/// `main.rs::run_backup`).
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
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Row;

    /// A minimal seeded db: one table, one known row — deliberately independent
    /// of the application schema/migrations, since `backup.rs` must work with
    /// any SQLite file content.
    async fn seed_db(path: &Path) {
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t (id, val) VALUES (1, 'hello')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    #[test]
    fn dir_is_empty_or_absent_covers_absent_empty_and_nonempty() {
        let tmp = tempfile::tempdir().unwrap();
        let absent = tmp.path().join("nope");
        assert!(dir_is_empty_or_absent(&absent).unwrap());

        let empty = tmp.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(dir_is_empty_or_absent(&empty).unwrap());

        let nonempty = tmp.path().join("nonempty");
        std::fs::create_dir_all(&nonempty).unwrap();
        std::fs::write(nonempty.join("f.txt"), b"x").unwrap();
        assert!(!dir_is_empty_or_absent(&nonempty).unwrap());
    }

    #[tokio::test]
    async fn create_backup_snapshots_db_and_assets() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("shadowcat.db");
        seed_db(&db_path).await;

        let assets_dir = tmp.path().join("assets");
        tokio::fs::create_dir_all(assets_dir.join("world1"))
            .await
            .unwrap();
        tokio::fs::write(assets_dir.join("world1").join("a.png"), b"AAA")
            .await
            .unwrap();
        tokio::fs::write(assets_dir.join("b.png"), b"BBBB")
            .await
            .unwrap();

        let out_dir = tmp.path().join("out");
        let manifest = create_backup(&db_path, &assets_dir, &out_dir)
            .await
            .unwrap();

        assert_eq!(manifest.asset_file_count, 2);
        assert!(manifest.db_bytes > 0);
        assert_eq!(manifest.source_db, db_path.to_string_lossy());
        assert_eq!(manifest.source_assets_dir, assets_dir.to_string_lossy());
        assert_eq!(manifest.shadowcat_version, env!("CARGO_PKG_VERSION"));

        // world.db opens and round-trips the known row.
        let out_db_url = format!("sqlite://{}", out_dir.join("world.db").to_string_lossy());
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&out_db_url)
            .await
            .unwrap();
        let row = sqlx::query("SELECT val FROM t WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        let val: String = row.get("val");
        assert_eq!(val, "hello");
        pool.close().await;

        // assets/ contains the same files byte-for-byte.
        assert_eq!(
            tokio::fs::read(out_dir.join("assets").join("world1").join("a.png"))
                .await
                .unwrap(),
            b"AAA"
        );
        assert_eq!(
            tokio::fs::read(out_dir.join("assets").join("b.png"))
                .await
                .unwrap(),
            b"BBBB"
        );

        // manifest.json parses back to the same struct.
        let on_disk: BackupManifest = serde_json::from_slice(
            &tokio::fs::read(out_dir.join("manifest.json"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(on_disk, manifest);
    }

    #[tokio::test]
    async fn create_backup_with_no_assets_dir_yields_zero_files() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("shadowcat.db");
        seed_db(&db_path).await;
        let assets_dir = tmp.path().join("does_not_exist");
        let out_dir = tmp.path().join("out");

        let manifest = create_backup(&db_path, &assets_dir, &out_dir)
            .await
            .unwrap();
        assert_eq!(manifest.asset_file_count, 0);
        assert!(out_dir.join("assets").is_dir());
    }

    #[tokio::test]
    async fn create_backup_escapes_single_quote_in_output_path() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("shadowcat.db");
        seed_db(&db_path).await;
        let assets_dir = tmp.path().join("assets");
        tokio::fs::create_dir_all(&assets_dir).await.unwrap();

        // `'` is a legal filename character on Windows, Linux, and macOS; this
        // proves the VACUUM INTO literal-string escaping is correct.
        let out_dir = tmp.path().join("it's a backup dir");
        let manifest = create_backup(&db_path, &assets_dir, &out_dir)
            .await
            .unwrap();
        assert!(manifest.db_bytes > 0);
        assert!(out_dir.join("world.db").is_file());
    }
}
