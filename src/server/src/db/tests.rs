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
