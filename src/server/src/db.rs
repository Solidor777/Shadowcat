// Docs-ratchet: every item in this module ships documented (docs sweep 1).
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

/// Opens a SQLite connection pool. `"sqlite::memory:"` yields an ephemeral
/// in-process database — used here to prove the SQLite-only target wires up.
///
/// # Examples
///
/// ```no_run
/// # async fn demo() -> Result<(), sqlx::Error> {
/// let pool = shadowcat::db::open_pool("sqlite::memory:").await?;
/// let row: (i64,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await?;
/// assert_eq!(row.0, 1);
/// # Ok(())
/// # }
/// ```
pub async fn open_pool(url: &str) -> Result<SqlitePool, sqlx::Error> {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_pool_answers_select_one() {
        let pool = open_pool("sqlite::memory:").await.expect("open pool");
        let row: (i64,) = sqlx::query_as("SELECT 1")
            .fetch_one(&pool)
            .await
            .expect("query");
        assert_eq!(row.0, 1);
    }
}
