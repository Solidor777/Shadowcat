// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

/// Opens a SQLite connection pool with the server's single-writer connection
/// cap and foreign-key enforcement — the pool-open options every production
/// caller shares, stated once here rather than restated per call site.
/// Does not run migrations: [`crate::data::sqlite::SqliteRepository::connect`]
/// runs them itself immediately after opening; a caller that opens a
/// short-lived pool against an already-migrated database (e.g. one that only
/// runs `VACUUM INTO`) simply never calls migrate.
///
/// # Examples
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), sqlx::Error> {
/// let pool = shadowcat::db::connect_pool("sqlite::memory:").await?;
/// let row: (i64,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await?;
/// assert_eq!(row.0, 1);
/// # Ok(())
/// # }
/// ```
pub async fn connect_pool(url: &str) -> Result<SqlitePool, sqlx::Error> {
    SqlitePoolOptions::new()
        // Single writer connection serializes transactions against the pool,
        // avoiding SQLITE_BUSY contention — the same reasoning applies
        // equally to the long-lived server pool and any short-lived pool
        // opened against the same on-disk database.
        .max_connections(1)
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("PRAGMA foreign_keys = ON;")
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(url)
        .await
}

#[cfg(test)]
mod tests {
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
}
