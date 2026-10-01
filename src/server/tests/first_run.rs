//! First-run behaviour of the storage bootstrap: server startup creates a missing database file,
//! while one-shot tooling that opens a pool directly never does.

use shadowcat::data::sqlite::SqliteRepository;

#[tokio::test]
async fn connect_creates_a_missing_database_file_for_a_bare_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fresh.db");
    assert!(!path.exists());
    let repo = SqliteRepository::connect(path.to_str().unwrap())
        .await
        .unwrap();
    assert!(path.is_file(), "server startup creates the database file");
    assert!(repo.admin_exists().await.is_ok());
}

#[tokio::test]
async fn direct_pool_opening_never_creates_a_missing_database_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("typo.db");
    let opts = shadowcat::db::parse_connect_options(path.to_str().unwrap()).unwrap();
    assert!(shadowcat::db::connect_pool_with_options(opts)
        .await
        .is_err());
    assert!(
        !path.exists(),
        "a mistyped backup/restore path must stay an error"
    );
}

#[tokio::test]
async fn creating_a_missing_database_file_is_logged_with_its_path_and_only_then() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fresh.db");
    let (guard, buf) = shadowcat_test_support::logs::capture_current_thread();
    SqliteRepository::connect(path.to_str().unwrap())
        .await
        .unwrap();
    let first = buf.text();
    assert!(
        first.contains("does not exist; creating") && first.contains("fresh.db"),
        "a new file is announced with its path: {first}"
    );
    SqliteRepository::connect(path.to_str().unwrap())
        .await
        .unwrap();
    drop(guard);
    assert_eq!(
        buf.text().matches("does not exist; creating").count(),
        1,
        "an existing file is opened silently"
    );
}
