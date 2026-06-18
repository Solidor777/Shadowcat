use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use crate::auth::password::hash_password;
use crate::config::Config;
use crate::data::sqlite::SqliteRepository;
use crate::http::error::AppError;

/// Wall-clock milliseconds since the epoch. Used for `users.created_at`.
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Single audited path that hashes a password and writes the first admin user.
/// Returns the new id, or `None` when an admin already exists (the insert is
/// guarded so concurrent first-run callers cannot both create an admin).
pub async fn create_admin(
    repo: &SqliteRepository,
    username: &str,
    password: &str,
    now: i64,
) -> Result<Option<Uuid>, AppError> {
    let hash = hash_password(password).map_err(|_| AppError::Internal)?;
    repo.create_admin_if_none(username, &hash, now)
        .await
        .map_err(|_| AppError::Internal)
}

/// Seed the admin from config when one is configured and none exists. Returns
/// whether it created an account. The remote-hosting path.
pub async fn bootstrap_admin(repo: &SqliteRepository, config: &Config) -> anyhow::Result<bool> {
    if let (Some(u), Some(p)) = (&config.admin_user, &config.admin_password) {
        let created = create_admin(repo, u, p, now_millis())
            .await
            .map_err(|_| anyhow::anyhow!("bootstrap admin creation failed"))?;
        if created.is_some() {
            tracing::info!(username = %u, "bootstrapped admin from config");
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bootstrap_seeds_admin_once_then_is_idempotent() {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let cfg = Config {
            admin_user: Some("ops".into()),
            admin_password: Some("pw-bootstrap".into()),
            ..Config::default()
        };

        assert!(bootstrap_admin(&repo, &cfg).await.unwrap());
        assert!(repo.admin_exists().await.unwrap());
        // Second call: admin already exists → no-op.
        assert!(!bootstrap_admin(&repo, &cfg).await.unwrap());
    }

    #[tokio::test]
    async fn bootstrap_noop_without_config_creds() {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let cfg = Config::default();
        assert!(!bootstrap_admin(&repo, &cfg).await.unwrap());
        assert!(!repo.admin_exists().await.unwrap());
    }
}
