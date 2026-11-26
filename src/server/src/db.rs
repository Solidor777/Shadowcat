//! This module opens the server's SQLite pools and resolves which file a database URL names.
//!
//! The read-write pool is single-connection (`connect_pool`, `connect_pool_with_options`), so one
//! writer serializes transactions; `open_read_only_pool` opens a small read-only pool over the same
//! database for hot read paths.

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

/// Resolves the on-disk file `url` names, or `None` when `url` addresses an
/// in-memory database with no filesystem location at all. The single decider
/// every caller that needs "which file is this database" (log/backup/module
/// sibling-directory placement, one-shot backup/restore) must route through,
/// rather than treating [`crate::config::Config::db`] as a bare path itself —
/// `Config.db` is any string [`parse_connect_options`] accepts: a bare path, a
/// `sqlite:`/`sqlite://` URL (relative or absolute), or one of two in-memory
/// spellings (`:memory:`/`sqlite::memory:`, or a `mode=memory` query
/// parameter on any URL).
///
/// Parses through [`parse_connect_options`] (never a second URL parser) and
/// inspects the result two ways. `:memory:` is detected via
/// [`SqliteConnectOptions::get_filename`]: sqlx's own `from_db_and_params`
/// marks an in-memory database by setting the filename to a generated
/// `file:sqlx-in-memory-<n>` placeholder rather than a real path, and that
/// marker is the only public signal of the in-memory flag sqlx exposes (the
/// flag itself is a private field with a builder setter but no getter).
/// `mode=memory` is a second, independent in-memory spelling sqlx's parser
/// reads into that same private flag without exposing it either, so this
/// function re-reads the query string itself with `url::form_urlencoded` —
/// the identical parser sqlx's own `from_db_and_params` uses internally,
/// already a workspace dependency, so no second query-string parser is
/// introduced.
///
/// # Examples
///
/// ```
/// use shadowcat::db::database_file;
///
/// // Bare filesystem path.
/// assert_eq!(
///     database_file("data/shadowcat.db").unwrap(),
///     Some(std::path::PathBuf::from("data/shadowcat.db")),
/// );
///
/// // A `sqlite://` URL is a real file too -- a bare-path check would miss it.
/// assert_eq!(
///     database_file("sqlite:///srv/shadowcat.db").unwrap(),
///     Some(std::path::PathBuf::from("/srv/shadowcat.db")),
/// );
///
/// // A relative `sqlite:` URL.
/// assert_eq!(
///     database_file("sqlite:data/shadowcat.db").unwrap(),
///     Some(std::path::PathBuf::from("data/shadowcat.db")),
/// );
///
/// // Both in-memory spellings resolve to `None`.
/// assert_eq!(database_file(":memory:").unwrap(), None);
/// assert_eq!(database_file("sqlite::memory:").unwrap(), None);
/// assert_eq!(database_file("sqlite://?mode=memory").unwrap(), None);
///
/// // A bare path containing the word "memory" is still a real file -- a
/// // substring check on `Config.db` would wrongly treat this as in-memory.
/// assert_eq!(
///     database_file("memory-lane/shadowcat.db").unwrap(),
///     Some(std::path::PathBuf::from("memory-lane/shadowcat.db")),
/// );
/// ```
pub fn database_file(url: &str) -> Result<Option<std::path::PathBuf>, sqlx::Error> {
    let options = parse_connect_options(url)?;
    let filename = options.get_filename();
    if filename
        .to_str()
        .is_some_and(|f| f.starts_with("file:sqlx-in-memory-"))
    {
        return Ok(None);
    }
    // `SqliteConnectOptions::from_str` strips a leading `sqlite://`/`sqlite:` scheme
    // before splitting on `?`; the query string starts at the same offset in `url`
    // itself regardless of which scheme prefix (or none) is present.
    if let Some((_, query)) = url.split_once('?') {
        let is_memory_mode = url::form_urlencoded::parse(query.as_bytes())
            .any(|(key, value)| key == "mode" && value == "memory");
        if is_memory_mode {
            return Ok(None);
        }
    }
    Ok(Some(filename.to_path_buf()))
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
        // An in-memory database exists only while a connection to it is open. sqlx reaps a pool
        // connection after `idle_timeout` (10 min) or `max_lifetime` (30 min) by default, which
        // would drop the whole database and let the next connection open an empty one ("no such
        // table"). Both are disabled so the single connection lives as long as the pool.
        .idle_timeout(None)
        .max_lifetime(None)
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
mod tests;
