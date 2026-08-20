// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

/// Parses `url` into connect options once. A second pool sharing the same
/// database (e.g. [`open_read_only_pool`]) must be built by cloning this
/// result, never by re-parsing `url` a second time — an in-memory URL's
/// shared-cache database is assigned a unique generated name at PARSE time,
/// so two independent parses of the identical string open two different,
/// unrelated empty databases.
///
/// # Examples
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), sqlx::Error> {
/// let opts = shadowcat::db::parse_connect_options("sqlite::memory:")?;
/// let pool = shadowcat::db::connect_pool_with_options(opts).await?;
/// # let _ = pool;
/// # Ok(())
/// # }
/// ```
pub fn parse_connect_options(url: &str) -> Result<SqliteConnectOptions, sqlx::Error> {
    SqliteConnectOptions::from_str(url)
}

/// Opens a SQLite connection pool from already-parsed `options`, with the
/// server's single-writer connection cap and foreign-key enforcement.
/// [`connect_pool`] is the URL-string convenience over this for callers with
/// no need to share `options` with a second pool. Does not run migrations:
/// [`crate::data::sqlite::SqliteRepository::connect`] runs them itself
/// immediately after opening; a caller that opens a short-lived pool against
/// an already-migrated database (e.g. one that only runs `VACUUM INTO`)
/// simply never calls migrate.
///
/// # Examples
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), sqlx::Error> {
/// let opts = shadowcat::db::parse_connect_options("sqlite::memory:")?;
/// let pool = shadowcat::db::connect_pool_with_options(opts).await?;
/// let row: (i64,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await?;
/// assert_eq!(row.0, 1);
/// # Ok(())
/// # }
/// ```
pub async fn connect_pool_with_options(
    options: SqliteConnectOptions,
) -> Result<SqlitePool, sqlx::Error> {
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
        .connect_with(options)
        .await
}

/// Opens a small, read-only pool against the same database `options` points
/// at — `options` must be a clone of whatever [`parse_connect_options`]
/// produced for the paired write pool, never a fresh parse of the same URL
/// string (see [`parse_connect_options`]'s doc). For a hot read path (e.g.
/// tower-sessions' per-request session load) that must not queue behind the
/// single writer: read-only connections never contend for a write lock, so a
/// small concurrent cap is safe.
///
/// # Examples
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), sqlx::Error> {
/// let opts = shadowcat::db::parse_connect_options("sqlite::memory:")?;
/// let write_pool = shadowcat::db::connect_pool_with_options(opts.clone()).await?;
/// let read_pool = shadowcat::db::open_read_only_pool(opts).await?;
/// let row: (i64,) = sqlx::query_as("SELECT 1").fetch_one(&read_pool).await?;
/// assert_eq!(row.0, 1);
/// # let _ = write_pool;
/// # Ok(())
/// # }
/// ```
pub async fn open_read_only_pool(options: SqliteConnectOptions) -> Result<SqlitePool, sqlx::Error> {
    SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options.read_only(true))
        .await
}

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
    connect_pool_with_options(parse_connect_options(url)?).await
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
