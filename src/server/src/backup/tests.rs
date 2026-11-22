use super::*;
use sqlx::Row;
use std::path::PathBuf;

/// A minimal seeded db: one table, one known row — deliberately independent
/// of the application schema/migrations, since `create_backup`'s `VACUUM INTO`
/// and `restore_backup` must work with any SQLite file content.
async fn seed_db(path: &Path) {
    let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
    let pool = crate::db::connect_pool(&url).await.unwrap();
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

/// A trash rooted inside the test's own temp directory, retaining entries for the default window.
fn test_trash(dir: &Path) -> Trash {
    Trash::new(dir.join("trash"), crate::trash::TrashRetention::default())
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
    let manifest = create_backup(
        &db_path,
        &assets_dir,
        &out_dir,
        &test_trash(tmp.path()),
        BackupKind::Manual,
        None,
    )
    .await
    .unwrap();

    assert_eq!(manifest.asset_file_count, 2);
    assert!(manifest.db_bytes > 0);
    assert_eq!(manifest.source_db, db_path.to_string_lossy());
    assert_eq!(manifest.source_assets_dir, assets_dir.to_string_lossy());
    assert_eq!(manifest.shadowcat_version, env!("CARGO_PKG_VERSION"));

    // world.db opens and round-trips the known row.
    let out_db_url = format!("sqlite://{}", out_dir.join("world.db").to_string_lossy());
    let pool = crate::db::connect_pool(&out_db_url).await.unwrap();
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

    let manifest = create_backup(
        &db_path,
        &assets_dir,
        &out_dir,
        &test_trash(tmp.path()),
        BackupKind::Manual,
        None,
    )
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
    let manifest = create_backup(
        &db_path,
        &assets_dir,
        &out_dir,
        &test_trash(tmp.path()),
        BackupKind::Manual,
        None,
    )
    .await
    .unwrap();
    assert!(manifest.db_bytes > 0);
    assert!(out_dir.join("world.db").is_file());
}

#[tokio::test]
async fn restore_backup_round_trips_content() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("shadowcat.db");
    seed_db(&db_path).await;
    let assets_dir = tmp.path().join("assets");
    tokio::fs::create_dir_all(&assets_dir).await.unwrap();
    tokio::fs::write(assets_dir.join("a.png"), b"AAA")
        .await
        .unwrap();

    let out_dir = tmp.path().join("out");
    create_backup(
        &db_path,
        &assets_dir,
        &out_dir,
        &test_trash(tmp.path()),
        BackupKind::Manual,
        None,
    )
    .await
    .unwrap();

    let restored_db = tmp.path().join("restored.db");
    let restored_assets = tmp.path().join("restored_assets");
    restore_backup(
        &out_dir,
        &restored_db,
        &restored_assets,
        false,
        &test_trash(tmp.path()),
    )
    .await
    .unwrap();

    let restored_url = format!("sqlite://{}", restored_db.to_string_lossy());
    let pool = crate::db::connect_pool(&restored_url).await.unwrap();
    let row = sqlx::query("SELECT val FROM t WHERE id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let val: String = row.get("val");
    assert_eq!(val, "hello");
    pool.close().await;

    assert_eq!(
        tokio::fs::read(restored_assets.join("a.png"))
            .await
            .unwrap(),
        b"AAA"
    );
}

#[tokio::test]
async fn restore_backup_refuses_nonempty_destination_without_force() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("shadowcat.db");
    seed_db(&db_path).await;
    let assets_dir = tmp.path().join("assets");
    tokio::fs::create_dir_all(&assets_dir).await.unwrap();
    tokio::fs::write(assets_dir.join("a.png"), b"AAA")
        .await
        .unwrap();
    let out_dir = tmp.path().join("out");
    create_backup(
        &db_path,
        &assets_dir,
        &out_dir,
        &test_trash(tmp.path()),
        BackupKind::Manual,
        None,
    )
    .await
    .unwrap();

    // An existing destination db file blocks a force-less restore.
    let restored_db = tmp.path().join("restored.db");
    tokio::fs::write(&restored_db, b"pre-existing bytes")
        .await
        .unwrap();
    let restored_assets = tmp.path().join("restored_assets");

    let err = restore_backup(
        &out_dir,
        &restored_db,
        &restored_assets,
        false,
        &test_trash(tmp.path()),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackupError::DestinationNotEmpty(_)));
    // The pre-existing file is untouched.
    assert_eq!(
        tokio::fs::read(&restored_db).await.unwrap(),
        b"pre-existing bytes"
    );

    // --force proceeds and overwrites it.
    restore_backup(
        &out_dir,
        &restored_db,
        &restored_assets,
        true,
        &test_trash(tmp.path()),
    )
    .await
    .unwrap();
    assert_eq!(
        tokio::fs::read(restored_assets.join("a.png"))
            .await
            .unwrap(),
        b"AAA"
    );
}

#[tokio::test]
async fn restore_leaves_no_partial_destination_and_no_staging_residue() {
    // After a successful force-restore over existing content: destination
    // matches the backup exactly and neither staging path
    // (assets.restore-tmp / assets.restore-old, db restore-tmp) survives.
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("shadowcat.db");
    seed_db(&db_path).await;
    let assets_dir = tmp.path().join("assets");
    tokio::fs::create_dir_all(&assets_dir).await.unwrap();
    tokio::fs::write(assets_dir.join("a.png"), b"AAA")
        .await
        .unwrap();
    let out_dir = tmp.path().join("out");
    create_backup(
        &db_path,
        &assets_dir,
        &out_dir,
        &test_trash(tmp.path()),
        BackupKind::Manual,
        None,
    )
    .await
    .unwrap();

    // Seed DIFFERENT pre-existing destination content that force-restore
    // must fully replace.
    let restored_db = tmp.path().join("restored.db");
    tokio::fs::write(&restored_db, b"stale pre-existing db bytes")
        .await
        .unwrap();
    let restored_assets = tmp.path().join("restored_assets");
    tokio::fs::create_dir_all(&restored_assets).await.unwrap();
    tokio::fs::write(restored_assets.join("stale.png"), b"STALE")
        .await
        .unwrap();

    restore_backup(
        &out_dir,
        &restored_db,
        &restored_assets,
        true,
        &test_trash(tmp.path()),
    )
    .await
    .unwrap();

    // Destination content equals backup content.
    assert_eq!(
        tokio::fs::read(restored_assets.join("a.png"))
            .await
            .unwrap(),
        b"AAA"
    );
    assert!(!restored_assets.join("stale.png").exists());
    let restored_url = format!("sqlite://{}", restored_db.to_string_lossy());
    let pool = crate::db::connect_pool(&restored_url).await.unwrap();
    let row = sqlx::query("SELECT val FROM t WHERE id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let val: String = row.get("val");
    assert_eq!(val, "hello");
    pool.close().await;

    // No staging residue.
    let parent = restored_assets.parent().unwrap();
    assert!(!parent.join("restored_assets.restore-tmp").exists());
    assert!(!parent.join("restored_assets.restore-old").exists());
    assert!(!restored_db.with_extension("restore-tmp").exists());
}

#[tokio::test]
async fn restore_backup_fails_closed_on_missing_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let empty_backup_dir = tmp.path().join("not_a_backup");
    tokio::fs::create_dir_all(&empty_backup_dir).await.unwrap();

    let err = restore_backup(
        &empty_backup_dir,
        &tmp.path().join("db.sqlite"),
        &tmp.path().join("assets"),
        false,
        &test_trash(tmp.path()),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackupError::InvalidBackupDir(_, _)));
}

#[tokio::test]
async fn restore_backup_fails_closed_on_malformed_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let backup_dir = tmp.path().join("bad_backup");
    tokio::fs::create_dir_all(&backup_dir).await.unwrap();
    tokio::fs::write(backup_dir.join("manifest.json"), b"{ not valid json")
        .await
        .unwrap();
    tokio::fs::write(backup_dir.join("world.db"), b"fake db bytes")
        .await
        .unwrap();

    let err = restore_backup(
        &backup_dir,
        &tmp.path().join("db.sqlite"),
        &tmp.path().join("assets"),
        false,
        &test_trash(tmp.path()),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackupError::InvalidBackupDir(_, _)));
}

#[tokio::test]
async fn backup_and_restore_round_trip_preserves_nested_directory_structure() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("shadowcat.db");
    seed_db(&db_path).await;

    // Multi-level nesting built exclusively via Path::join, never string
    // concatenation — the portability property this test exists to prove.
    let assets_dir = tmp.path().join("assets");
    let deep_dir = assets_dir.join("world1").join("scenes").join("battlemap");
    tokio::fs::create_dir_all(&deep_dir).await.unwrap();
    let deep_file = deep_dir.join("token.png");
    tokio::fs::write(&deep_file, b"DEEP").await.unwrap();

    let out_dir = tmp.path().join("out");
    let manifest = create_backup(
        &db_path,
        &assets_dir,
        &out_dir,
        &test_trash(tmp.path()),
        BackupKind::Manual,
        None,
    )
    .await
    .unwrap();
    assert_eq!(manifest.asset_file_count, 1);

    let backed_up_deep = out_dir
        .join("assets")
        .join("world1")
        .join("scenes")
        .join("battlemap")
        .join("token.png");
    assert_eq!(tokio::fs::read(&backed_up_deep).await.unwrap(), b"DEEP");

    // Round-trip through restore into a third location.
    let restored_db = tmp.path().join("restored.db");
    let restored_assets = tmp.path().join("restored_assets");
    restore_backup(
        &out_dir,
        &restored_db,
        &restored_assets,
        false,
        &test_trash(tmp.path()),
    )
    .await
    .unwrap();

    let restored_deep = restored_assets
        .join("world1")
        .join("scenes")
        .join("battlemap")
        .join("token.png");
    assert_eq!(tokio::fs::read(&restored_deep).await.unwrap(), b"DEEP");
}

/// A second backup into the same output directory moves the first snapshot's `world.db` into the
/// trash rather than destroying it. Fails if `create_backup` removes the file outright.
#[tokio::test]
async fn create_backup_trashes_a_previous_snapshot_database() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("shadowcat.db");
    seed_db(&db_path).await;
    let assets_dir = tmp.path().join("assets");
    let out_dir = tmp.path().join("out");
    let trash = test_trash(tmp.path());

    create_backup(
        &db_path,
        &assets_dir,
        &out_dir,
        &trash,
        BackupKind::Manual,
        None,
    )
    .await
    .unwrap();
    assert!(
        trash.list().unwrap().is_empty(),
        "a first backup trashes nothing"
    );
    create_backup(
        &db_path,
        &assets_dir,
        &out_dir,
        &trash,
        BackupKind::Manual,
        None,
    )
    .await
    .unwrap();

    let entries = trash.list().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].manifest.original_path.file_name().unwrap(),
        "world.db"
    );
    assert_eq!(entries[0].manifest.kind, crate::trash::TrashedKind::File);
}

/// A forced restore moves the assets directory it replaces into the trash. Fails if
/// `restore_backup` removes the previous directory outright, or leaves it out of the trash.
#[tokio::test]
async fn restore_backup_trashes_the_replaced_assets_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("shadowcat.db");
    seed_db(&db_path).await;
    let assets_dir = tmp.path().join("assets");
    tokio::fs::create_dir_all(&assets_dir).await.unwrap();
    tokio::fs::write(assets_dir.join("a.png"), b"AAA")
        .await
        .unwrap();
    let out_dir = tmp.path().join("out");
    let trash = test_trash(tmp.path());
    create_backup(
        &db_path,
        &assets_dir,
        &out_dir,
        &trash,
        BackupKind::Manual,
        None,
    )
    .await
    .unwrap();

    let restored_db = tmp.path().join("restored.db");
    let restored_assets = tmp.path().join("restored_assets");
    tokio::fs::create_dir_all(&restored_assets).await.unwrap();
    tokio::fs::write(restored_assets.join("stale.png"), b"STALE")
        .await
        .unwrap();
    restore_backup(&out_dir, &restored_db, &restored_assets, true, &trash)
        .await
        .unwrap();

    let entries = trash.list().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].manifest.kind, crate::trash::TrashedKind::Dir);
    assert_eq!(
        entries[0].size_bytes, 5,
        "the replaced directory's stale.png is retained"
    );
}

/// The `val` of row 1 in a `seed_db`-shaped file.
async fn read_val(path: &Path) -> String {
    let url = format!("sqlite://{}", path.to_string_lossy());
    let pool = crate::db::connect_pool(&url).await.unwrap();
    let row = sqlx::query("SELECT val FROM t WHERE id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let val: String = row.get("val");
    pool.close().await;
    val
}

/// Stages a restore of `id` the way the admin route does.
fn stage_restore(db_path: &Path, id: &str) {
    let marker = catalog::RestorePending {
        id: id.into(),
        requested_at_ms: 7,
    };
    std::fs::write(
        catalog::restore_pending_path(db_path),
        serde_json::to_vec(&marker).unwrap(),
    )
    .unwrap();
}

/// A staged restore replaces the live database and assets with the backup's, removes the marker
/// and reports the id. Fails if `apply_pending_restore` restores without `force` (the live db
/// exists, so the restore is refused) or leaves the marker behind.
#[tokio::test]
async fn apply_pending_restore_replaces_live_data_with_the_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("shadowcat.db");
    seed_db(&db_path).await;
    let assets_dir = tmp.path().join("assets");
    tokio::fs::create_dir_all(&assets_dir).await.unwrap();
    tokio::fs::write(assets_dir.join("old.png"), b"OLD")
        .await
        .unwrap();
    let trash = test_trash(tmp.path());
    let root = tmp.path().join("backups");
    create_backup(
        &db_path,
        &assets_dir,
        &root.join("backup-1"),
        &trash,
        BackupKind::Manual,
        None,
    )
    .await
    .unwrap();

    // Live state diverges after the backup.
    let url = format!("sqlite://{}", db_path.to_string_lossy());
    let pool = crate::db::connect_pool(&url).await.unwrap();
    sqlx::query("UPDATE t SET val = 'changed' WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    tokio::fs::write(assets_dir.join("new.png"), b"NEW")
        .await
        .unwrap();

    stage_restore(&db_path, "backup-1");
    let outcome = apply_pending_restore(&db_path, &assets_dir, &root, &trash)
        .await
        .unwrap();

    assert_eq!(outcome, PendingRestore::Applied("backup-1".into()));
    assert_eq!(read_val(&db_path).await, "hello");
    assert!(assets_dir.join("old.png").exists());
    assert!(!assets_dir.join("new.png").exists());
    assert!(!catalog::restore_pending_path(&db_path).exists());
    assert!(!db_path.with_file_name(RESTORE_FAILED_FILE).exists());
}

/// A backup that fails validation leaves the live database byte-identical, records the failure
/// and removes the pending marker so the next start does not retry. Fails if the failure path
/// leaves the marker in place or skips the failed marker.
#[tokio::test]
async fn apply_pending_restore_records_a_failure_and_keeps_live_data() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("shadowcat.db");
    seed_db(&db_path).await;
    let assets_dir = tmp.path().join("assets");
    let root = tmp.path().join("backups");
    let broken = root.join("backup-2");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("world.db"), b"not a database").unwrap();
    std::fs::write(broken.join("manifest.json"), b"{ not json").unwrap();
    let before = std::fs::read(&db_path).unwrap();

    stage_restore(&db_path, "backup-2");
    let outcome = apply_pending_restore(&db_path, &assets_dir, &root, &test_trash(tmp.path()))
        .await
        .unwrap();

    assert!(matches!(outcome, PendingRestore::Failed(_)));
    assert_eq!(std::fs::read(&db_path).unwrap(), before);
    assert!(!catalog::restore_pending_path(&db_path).exists());
    let failed: RestoreFailed = serde_json::from_slice(
        &std::fs::read(db_path.with_file_name(RESTORE_FAILED_FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(failed.id, "backup-2");
    assert_eq!(failed.requested_at_ms, 7);
    assert!(failed.error.contains("malformed manifest.json"));
}

/// An id that is not a direct child of the backups root, and an unreadable marker, both fail
/// closed with a failed marker and no destination write. Fails if the marker's id reaches the
/// filesystem without `resolve_backup`.
#[tokio::test]
async fn apply_pending_restore_rejects_a_traversal_id_and_a_garbled_marker() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("shadowcat.db");
    seed_db(&db_path).await;
    let root = tmp.path().join("backups");
    std::fs::create_dir_all(&root).unwrap();
    let trash = test_trash(tmp.path());
    let assets_dir = tmp.path().join("assets");
    let failed_path = db_path.with_file_name(RESTORE_FAILED_FILE);

    stage_restore(&db_path, "..");
    let outcome = apply_pending_restore(&db_path, &assets_dir, &root, &trash)
        .await
        .unwrap();
    assert!(matches!(outcome, PendingRestore::Failed(_)));
    assert!(failed_path.exists());

    std::fs::write(catalog::restore_pending_path(&db_path), b"garbage").unwrap();
    let outcome = apply_pending_restore(&db_path, &assets_dir, &root, &trash)
        .await
        .unwrap();
    assert!(matches!(outcome, PendingRestore::Failed(_)));
    let failed: RestoreFailed =
        serde_json::from_slice(&std::fs::read(&failed_path).unwrap()).unwrap();
    assert_eq!(failed.id, "");
    assert_eq!(read_val(&db_path).await, "hello");
}

/// A later successful restore clears the failed marker an earlier attempt left.
#[tokio::test]
async fn apply_pending_restore_success_clears_an_earlier_failed_marker() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("shadowcat.db");
    seed_db(&db_path).await;
    let assets_dir = tmp.path().join("assets");
    let trash = test_trash(tmp.path());
    let root = tmp.path().join("backups");
    create_backup(
        &db_path,
        &assets_dir,
        &root.join("backup-1"),
        &trash,
        BackupKind::Manual,
        None,
    )
    .await
    .unwrap();
    std::fs::write(db_path.with_file_name(RESTORE_FAILED_FILE), b"{}").unwrap();

    stage_restore(&db_path, "backup-1");
    apply_pending_restore(&db_path, &assets_dir, &root, &trash)
        .await
        .unwrap();

    assert!(!db_path.with_file_name(RESTORE_FAILED_FILE).exists());
}

/// A live db (`val = 'live'`) with live assets (`live.png`), and a backup of a db holding
/// `val = 'hello'` with assets (`backed.png`). Returns `(tmp, db_path, assets_dir, backup_dir)`.
async fn live_and_backup() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("shadowcat.db");
    seed_db(&db_path).await;
    let source_assets = tmp.path().join("source_assets");
    tokio::fs::create_dir_all(&source_assets).await.unwrap();
    tokio::fs::write(source_assets.join("backed.png"), b"BACKED")
        .await
        .unwrap();
    let backup_dir = tmp.path().join("backups").join("backup-1");
    create_backup(
        &db_path,
        &source_assets,
        &backup_dir,
        &test_trash(tmp.path()),
        BackupKind::Manual,
        None,
    )
    .await
    .unwrap();
    let url = format!("sqlite://{}", db_path.to_string_lossy());
    let pool = crate::db::connect_pool(&url).await.unwrap();
    sqlx::query("UPDATE t SET val = 'live' WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let assets_dir = tmp.path().join("assets");
    tokio::fs::create_dir_all(&assets_dir).await.unwrap();
    tokio::fs::write(assets_dir.join("live.png"), b"LIVE")
        .await
        .unwrap();
    (tmp, db_path, assets_dir, backup_dir)
}

/// A restore whose assets copy fails leaves the live database and assets exactly as they were.
/// The backup's `assets` entry is a file, so reading it as a directory fails. Fails if the
/// database swap runs before the assets copy has succeeded (the live db would read `hello`).
#[tokio::test]
async fn restore_failing_on_the_assets_copy_leaves_the_live_pair_untouched() {
    let (tmp, db_path, assets_dir, backup_dir) = live_and_backup().await;
    tokio::fs::rename(backup_dir.join("assets"), tmp.path().join("assets-aside"))
        .await
        .unwrap();
    tokio::fs::write(backup_dir.join("assets"), b"not a directory")
        .await
        .unwrap();

    let result = restore_backup(
        &backup_dir,
        &db_path,
        &assets_dir,
        true,
        &test_trash(tmp.path()),
    )
    .await;

    assert!(result.is_err());
    assert_eq!(read_val(&db_path).await, "live");
    assert!(assets_dir.join("live.png").exists());
    assert!(!assets_dir.join("backed.png").exists());
    assert!(!tmp.path().join("assets.restore-tmp").exists());
    assert!(!db_path.with_extension("restore-tmp").exists());
}

/// A staged restore whose assets copy fails still writes `restore-failed.json` with the error,
/// clears the pending marker and leaves the live database as it was. Fails if the failure path
/// skips the failed marker or the database was swapped before the copy failed.
#[tokio::test]
async fn staged_restore_failing_on_the_assets_copy_records_the_failure() {
    let (tmp, db_path, assets_dir, backup_dir) = live_and_backup().await;
    tokio::fs::rename(backup_dir.join("assets"), tmp.path().join("assets-aside"))
        .await
        .unwrap();
    tokio::fs::write(backup_dir.join("assets"), b"not a directory")
        .await
        .unwrap();
    stage_restore(&db_path, "backup-1");

    let outcome = apply_pending_restore(
        &db_path,
        &assets_dir,
        &tmp.path().join("backups"),
        &test_trash(tmp.path()),
    )
    .await
    .unwrap();

    assert!(matches!(outcome, PendingRestore::Failed(_)));
    assert_eq!(read_val(&db_path).await, "live");
    assert!(assets_dir.join("live.png").exists());
    assert!(!catalog::restore_pending_path(&db_path).exists());
    let failed: RestoreFailed = serde_json::from_slice(
        &std::fs::read(db_path.with_file_name(RESTORE_FAILED_FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(failed.id, "backup-1");
    assert!(!failed.error.is_empty());
}

/// A restore whose database swap fails after the assets were swapped puts the old assets back.
/// The live database path is a directory, so renaming the staged file over it fails. Fails if the
/// assets swap is not rolled back (the live assets would hold `backed.png`).
#[tokio::test]
async fn restore_failing_on_the_database_swap_puts_the_old_assets_back() {
    let (tmp, db_path, assets_dir, backup_dir) = live_and_backup().await;
    tokio::fs::rename(&db_path, tmp.path().join("shadowcat.db.aside"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(&db_path).await.unwrap();

    let result = restore_backup(
        &backup_dir,
        &db_path,
        &assets_dir,
        true,
        &test_trash(tmp.path()),
    )
    .await;

    assert!(result.is_err());
    assert!(db_path.is_dir());
    assert!(assets_dir.join("live.png").exists());
    assert!(!assets_dir.join("backed.png").exists());
    assert!(!tmp.path().join("assets.restore-tmp").exists());
    assert!(!tmp.path().join("assets.restore-old").exists());
    assert!(!db_path.with_extension("restore-tmp").exists());
}
