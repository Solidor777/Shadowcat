use super::*;
use sqlx::Row;

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
    create_backup(&db_path, &assets_dir, &out_dir)
        .await
        .unwrap();

    let restored_db = tmp.path().join("restored.db");
    let restored_assets = tmp.path().join("restored_assets");
    restore_backup(&out_dir, &restored_db, &restored_assets, false)
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
    create_backup(&db_path, &assets_dir, &out_dir)
        .await
        .unwrap();

    // An existing destination db file blocks a force-less restore.
    let restored_db = tmp.path().join("restored.db");
    tokio::fs::write(&restored_db, b"pre-existing bytes")
        .await
        .unwrap();
    let restored_assets = tmp.path().join("restored_assets");

    let err = restore_backup(&out_dir, &restored_db, &restored_assets, false)
        .await
        .unwrap_err();
    assert!(matches!(err, BackupError::DestinationNotEmpty(_)));
    // The pre-existing file is untouched.
    assert_eq!(
        tokio::fs::read(&restored_db).await.unwrap(),
        b"pre-existing bytes"
    );

    // --force proceeds and overwrites it.
    restore_backup(&out_dir, &restored_db, &restored_assets, true)
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
    create_backup(&db_path, &assets_dir, &out_dir)
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

    restore_backup(&out_dir, &restored_db, &restored_assets, true)
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
    let manifest = create_backup(&db_path, &assets_dir, &out_dir)
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
    restore_backup(&out_dir, &restored_db, &restored_assets, false)
        .await
        .unwrap();

    let restored_deep = restored_assets
        .join("world1")
        .join("scenes")
        .join("battlemap")
        .join("token.png");
    assert_eq!(tokio::fs::read(&restored_deep).await.unwrap(), b"DEEP");
}
