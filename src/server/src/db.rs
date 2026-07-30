// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

/// Opens a SQLite connection pool. `"sqlite::memory:"` yields an ephemeral
/// in-process database — used here to prove the SQLite-only target wires up.
///
/// # Examples
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), sqlx::Error> {
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
