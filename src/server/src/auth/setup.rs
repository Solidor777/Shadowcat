#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use crate::auth::password::hash_password_async;
use crate::config::Config;
use crate::data::sqlite::SqliteRepository;
use crate::http::error::AppError;

/// Wall-clock milliseconds since the epoch. Used for `users.created_at`.
///
/// # Examples
///
/// ```
/// use shadowcat::auth::setup::now_millis;
///
/// assert!(now_millis() > 0);
/// ```
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Single audited path that hashes a password and writes the first admin user.
/// Returns the new id, or `None` when an admin already exists (the insert is
/// guarded so concurrent first-run callers cannot both create an admin).
///
/// The username goes through the same `validate_username` policy as an
/// admin-created account: both entry points to this function (`/api/setup` and
/// the headless `bootstrap_admin`) would otherwise insert an unvalidated name,
/// breaking the ASCII invariant `create_user_unique` relies on for its
/// `NOCASE` uniqueness guard.
///
/// # Examples
///
/// ```
/// use shadowcat::auth::setup::create_admin;
/// use shadowcat::data::sqlite::SqliteRepository;
///
/// # #[tokio::main] async fn main() {
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let created = create_admin(&repo, "admin", "correct horse battery staple", 0).await.unwrap();
/// assert!(created.is_some());
/// // A second attempt finds an admin already exists.
/// let second = create_admin(&repo, "admin2", "correct horse battery staple", 0).await.unwrap();
/// assert!(second.is_none());
/// # }
/// ```
pub async fn create_admin(
    repo: &SqliteRepository,
    username: &str,
    password: &str,
    now: i64,
) -> Result<Option<Uuid>, AppError> {
    let username = crate::http::routes::validate_username(username)?;
    let hash = hash_password_async(password.to_owned())
        .await
        .map_err(|_| AppError::Internal)?;
    repo.create_admin_if_none(&username, &hash, now)
        .await
        .map_err(|_| AppError::Internal)
}

/// Seed the admin from config when one is configured and none exists. Returns
/// whether it created an account. The remote-hosting path.
///
/// # Examples
///
/// ```
/// use shadowcat::auth::setup::bootstrap_admin;
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
///
/// # #[tokio::main] async fn main() {
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let mut config = Config::default();
/// assert!(!bootstrap_admin(&repo, &config).await.unwrap()); // nothing configured
///
/// config.admin_user = Some("admin".to_string());
/// config.admin_password = Some("correct horse battery staple".to_string());
/// assert!(bootstrap_admin(&repo, &config).await.unwrap()); // seeded from config
/// # }
/// ```
pub async fn bootstrap_admin(repo: &SqliteRepository, config: &Config) -> anyhow::Result<bool> {
    if let (Some(u), Some(p)) = (&config.admin_user, &config.admin_password) {
        // A configured username that fails the account policy is a startup
        // failure, not a silently-skipped seed: the operator asked for an admin
        // and must not be left believing one exists. The policy hint is
        // attached ONLY to the validation variant — a database fault must not
        // be reported as a malformed username.
        let created = create_admin(repo, u, p, now_millis())
            .await
            .map_err(|e| match e {
                AppError::Unprocessable(m) => {
                    anyhow::anyhow!("bootstrap admin creation failed: {m}")
                }
                other => anyhow::anyhow!("bootstrap admin creation failed ({other:?})"),
            })?;
        if created.is_some() {
            tracing::info!(username = %u, "bootstrapped admin from config");
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests;
