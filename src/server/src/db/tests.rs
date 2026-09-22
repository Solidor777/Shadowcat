use super::*;

#[tokio::test]
async fn opens_a_single_connection_pool_with_foreign_keys_enabled() {
    let pool = connect_pool("sqlite::memory:").await.expect("open pool");
    assert_eq!(pool.options().get_max_connections(), 1);
    let row: (i64,) = sqlx::query_as("PRAGMA foreign_keys;")
        .fetch_one(&pool)
        .await
        .expect("query pragma");
    assert_eq!(row.0, 1, "foreign_keys pragma must be ON");
}

#[tokio::test]
async fn in_memory_pool_answers_select_one() {
    let pool = connect_pool("sqlite::memory:").await.expect("open pool");
    let row: (i64,) = sqlx::query_as("SELECT 1")
        .fetch_one(&pool)
        .await
        .expect("query");
    assert_eq!(row.0, 1);
}

#[test]
fn database_file_resolves_a_bare_path() {
    assert_eq!(
        database_file("shadowcat.db").unwrap(),
        Some(std::path::PathBuf::from("shadowcat.db")),
    );
}

#[test]
fn database_file_resolves_an_absolute_sqlite_url() {
    assert_eq!(
        database_file("sqlite:///srv/shadowcat.db").unwrap(),
        Some(std::path::PathBuf::from("/srv/shadowcat.db")),
    );
}

#[test]
fn database_file_resolves_a_relative_sqlite_url() {
    assert_eq!(
        database_file("sqlite:data/shadowcat.db").unwrap(),
        Some(std::path::PathBuf::from("data/shadowcat.db")),
    );
}

#[test]
fn database_file_is_none_for_colon_memory_colon() {
    assert_eq!(database_file(":memory:").unwrap(), None);
}

#[test]
fn database_file_is_none_for_sqlite_colon_colon_memory_colon() {
    assert_eq!(database_file("sqlite::memory:").unwrap(), None);
}

#[test]
fn database_file_is_none_for_mode_equals_memory_query_param() {
    assert_eq!(database_file("sqlite://?mode=memory").unwrap(), None);
}

#[test]
fn database_file_is_none_for_mode_equals_memory_on_a_bare_database_url() {
    assert_eq!(database_file("sqlite://a.db?mode=memory").unwrap(), None);
}

/// A path CONTAINING the substring "memory" is still a real on-disk file --
/// `database_file` resolves it via `SqliteConnectOptions`, never a substring
/// check on the raw `Config.db` string.
#[test]
fn database_file_resolves_a_path_containing_the_word_memory() {
    assert_eq!(
        database_file("memory-lane/shadowcat.db").unwrap(),
        Some(std::path::PathBuf::from("memory-lane/shadowcat.db")),
    );
}

#[test]
fn database_file_errors_on_an_unknown_query_parameter() {
    assert!(database_file("sqlite://a.db?bogus=1").is_err());
}
