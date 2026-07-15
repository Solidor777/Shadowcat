//! CLI-level integration tests for `--backup-to`/`--restore-from`, spawning the
//! actual compiled binary via Cargo's built-in `CARGO_BIN_EXE_<name>` env var
//! (no `assert_cmd` dependency needed — the package's default bin is named
//! `shadowcat`, matching `[package] name` in `Cargo.toml`).

use std::process::Command;

use shadowcat::auth::role::ServerRole;
use shadowcat::data::sqlite::SqliteRepository;

fn shadowcat_bin() -> &'static str {
    env!("CARGO_BIN_EXE_shadowcat")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backup_to_then_restore_from_round_trips_cli() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("source.db");
    let assets_dir = tmp.path().join("source_assets");
    tokio::fs::create_dir_all(&assets_dir).await.unwrap();
    tokio::fs::write(assets_dir.join("logo.png"), b"PNGDATA")
        .await
        .unwrap();

    // Seed via a real (migrated) SqliteRepository, matching production shape.
    {
        let url = format!("sqlite://{}?mode=rwc", db_path.to_string_lossy());
        let repo = SqliteRepository::connect(&url).await.unwrap();
        repo.create_user("ops", None, ServerRole::User, 0)
            .await
            .unwrap();
    }

    let backup_dir = tmp.path().join("backup");
    let status = Command::new(shadowcat_bin())
        .current_dir(tmp.path())
        .arg("--db")
        .arg(&db_path)
        .arg("--assets-dir")
        .arg(&assets_dir)
        .arg("--backup-to")
        .arg(&backup_dir)
        .status()
        .expect("run shadowcat --backup-to");
    assert!(status.success(), "backup-to exited non-zero");
    assert!(backup_dir.join("manifest.json").is_file());
    assert!(backup_dir.join("world.db").is_file());
    assert!(backup_dir.join("assets").join("logo.png").is_file());

    // Restore into fresh (nonexistent) destination paths.
    let restored_db = tmp.path().join("restored.db");
    let restored_assets = tmp.path().join("restored_assets");
    let status = Command::new(shadowcat_bin())
        .current_dir(tmp.path())
        .arg("--db")
        .arg(&restored_db)
        .arg("--assets-dir")
        .arg(&restored_assets)
        .arg("--restore-from")
        .arg(&backup_dir)
        .status()
        .expect("run shadowcat --restore-from");
    assert!(status.success(), "restore-from exited non-zero");

    let restored_url = format!("sqlite://{}", restored_db.to_string_lossy());
    let repo = SqliteRepository::connect(&restored_url).await.unwrap();
    assert!(repo.user_by_username("ops").await.unwrap().is_some());
    assert_eq!(
        tokio::fs::read(restored_assets.join("logo.png"))
            .await
            .unwrap(),
        b"PNGDATA"
    );
}

#[test]
fn backup_to_and_restore_from_together_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let status = Command::new(shadowcat_bin())
        .current_dir(tmp.path())
        .arg("--backup-to")
        .arg(tmp.path().join("a"))
        .arg("--restore-from")
        .arg(tmp.path().join("b"))
        .status()
        .expect("run shadowcat with both flags");
    assert!(!status.success());
}
