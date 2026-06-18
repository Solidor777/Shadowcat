use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tower_sessions::cookie::time::{Duration, OffsetDateTime};
use tower_sessions::cookie::{Key, SameSite};
use tower_sessions::service::SignedCookie;
use tower_sessions::session::{Id, Record};
use tower_sessions::session_store::{self, SessionStore};
use tower_sessions::{Expiry, Session, SessionManagerLayer};
use uuid::Uuid;

use crate::auth::role::ServerRole;
use crate::config::Config;
use crate::data::sqlite::SqliteRepository;
use crate::http::error::AppError;
use crate::http::AppState;

const SESSION_USER_KEY: &str = "user";
const SESSION_KEY_SETTING: &str = "session_key";

/// DB-backed session store over the data layer's sqlx 0.9 pool. A separate
/// `tower-sessions-sqlx-store` is not used: it pins sqlx 0.8, which would
/// duplicate the driver and require a second pool — breaking the single-writer
/// invariant. Sharing the existing pool keeps one writer and one sqlx version.
#[derive(Debug, Clone)]
pub struct SqlxSqliteStore {
    pool: SqlitePool,
}

impl SqlxSqliteStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Create the session table if absent. Run once at startup.
    pub async fn migrate(&self) -> Result<(), sqlx::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tower_sessions (\
             id TEXT PRIMARY KEY, data TEXT NOT NULL, expiry_date INTEGER NOT NULL)",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn id_exists(&self, id: &Id) -> session_store::Result<bool> {
        let row = sqlx::query("SELECT 1 FROM tower_sessions WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| session_store::Error::Backend(e.to_string()))?;
        Ok(row.is_some())
    }
}

#[async_trait]
impl SessionStore for SqlxSqliteStore {
    async fn create(&self, record: &mut Record) -> session_store::Result<()> {
        // Regenerate on the astronomically-unlikely id collision before insert.
        while self.id_exists(&record.id).await? {
            record.id = Id::default();
        }
        self.save(record).await
    }

    async fn save(&self, record: &Record) -> session_store::Result<()> {
        let data = serde_json::to_string(record)
            .map_err(|e| session_store::Error::Encode(e.to_string()))?;
        sqlx::query(
            "INSERT INTO tower_sessions (id, data, expiry_date) VALUES (?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET data = excluded.data, expiry_date = excluded.expiry_date",
        )
        .bind(record.id.to_string())
        .bind(data)
        .bind(record.expiry_date.unix_timestamp())
        .execute(&self.pool)
        .await
        .map_err(|e| session_store::Error::Backend(e.to_string()))?;
        Ok(())
    }

    async fn load(&self, id: &Id) -> session_store::Result<Option<Record>> {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let row = sqlx::query("SELECT data FROM tower_sessions WHERE id = ? AND expiry_date > ?")
            .bind(id.to_string())
            .bind(now)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| session_store::Error::Backend(e.to_string()))?;
        match row {
            Some(r) => {
                let data: String = r.get("data");
                let record: Record = serde_json::from_str(&data)
                    .map_err(|e| session_store::Error::Decode(e.to_string()))?;
                Ok(Some(record))
            }
            None => Ok(None),
        }
    }

    async fn delete(&self, id: &Id) -> session_store::Result<()> {
        sqlx::query("DELETE FROM tower_sessions WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| session_store::Error::Backend(e.to_string()))?;
        Ok(())
    }
}

/// Identity persisted in the session store after login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionUser {
    pub id: Uuid,
    pub username: String,
    pub role: ServerRole,
}

/// Any authenticated user.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: Uuid,
    pub username: String,
    pub role: ServerRole,
}

/// An authenticated user whose server role is Admin.
pub struct AdminUser(pub AuthUser);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let session = Session::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::Unauthorized)?;
        let user: Option<SessionUser> = session
            .get(SESSION_USER_KEY)
            .await
            .map_err(|_| AppError::Internal)?;
        let u = user.ok_or(AppError::Unauthorized)?;
        Ok(AuthUser { id: u.id, username: u.username, role: u.role })
    }
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if user.role == ServerRole::Admin {
            Ok(AdminUser(user))
        } else {
            Err(AppError::Forbidden)
        }
    }
}

/// Load the persisted session signing key, or generate + persist one. An
/// explicit `config.session_key` (base64) overrides storage.
pub async fn load_or_create_key(repo: &SqliteRepository, config: &Config) -> anyhow::Result<Key> {
    if let Some(explicit) = &config.session_key {
        let raw = base64::engine::general_purpose::STANDARD.decode(explicit)?;
        return Ok(Key::from(&raw));
    }
    if let Some(stored) = repo.get_setting(SESSION_KEY_SETTING).await? {
        let raw = base64::engine::general_purpose::STANDARD.decode(stored)?;
        return Ok(Key::from(&raw));
    }
    let key = Key::generate();
    let encoded = base64::engine::general_purpose::STANDARD.encode(key.master());
    repo.set_setting(SESSION_KEY_SETTING, &encoded).await?;
    Ok(key)
}

/// Build the signed, DB-backed session layer. Cookie is `Secure` only on a
/// non-loopback bind (so loopback dev over http still works).
pub async fn session_layer(
    repo: &SqliteRepository,
    config: &Config,
) -> anyhow::Result<SessionManagerLayer<SqlxSqliteStore, SignedCookie>> {
    let store = SqlxSqliteStore::new(repo.pool().clone());
    store.migrate().await?;
    let key = load_or_create_key(repo, config).await?;
    Ok(SessionManagerLayer::new(store)
        .with_secure(!config.is_loopback_bind())
        .with_same_site(SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(Duration::days(7)))
        .with_signed(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::role::ServerRole;
    use axum::routing::{get, post};
    use axum::Router;
    use tower_sessions::Session;
    use uuid::Uuid;

    // Test-only routes that exercise the extractors without the prod surface.
    async fn login_as_admin(session: Session) -> &'static str {
        session
            .insert(
                "user",
                SessionUser { id: Uuid::from_u128(1), username: "a".into(), role: ServerRole::Admin },
            )
            .await
            .unwrap();
        "ok"
    }
    async fn login_as_user(session: Session) -> &'static str {
        session
            .insert(
                "user",
                SessionUser { id: Uuid::from_u128(2), username: "u".into(), role: ServerRole::User },
            )
            .await
            .unwrap();
        "ok"
    }
    async fn whoami(user: AuthUser) -> String {
        user.username
    }
    async fn admin_only(_admin: AdminUser) -> &'static str {
        "admin"
    }

    async fn harness() -> (axum_test::TestServer, ()) {
        let state = crate::http::tests::test_state().await;
        let layer = session_layer(&state.repo, &state.config).await.unwrap();
        let app = Router::new()
            .route("/t/login-admin", post(login_as_admin))
            .route("/t/login-user", post(login_as_user))
            .route("/t/me", get(whoami))
            .route("/t/admin", get(admin_only))
            .layer(layer)
            .with_state(state);
        (axum_test::TestServer::builder().save_cookies().build(app).unwrap(), ())
    }

    #[tokio::test]
    async fn auth_user_requires_session() {
        let (server, _) = harness().await;
        server.get("/t/me").await.assert_status(axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn admin_extractor_rejects_non_admin() {
        let (server, _) = harness().await;
        server.post("/t/login-user").await.assert_status_ok();
        server.get("/t/me").await.assert_status_ok(); // any user passes AuthUser
        server.get("/t/admin").await.assert_status(axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_extractor_accepts_admin() {
        let (server, _) = harness().await;
        server.post("/t/login-admin").await.assert_status_ok();
        server.get("/t/admin").await.assert_status_ok();
    }

    #[tokio::test]
    async fn session_key_is_stable_across_loads() {
        let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let cfg = crate::config::Config::default();
        let k1 = load_or_create_key(&repo, &cfg).await.unwrap();
        let k2 = load_or_create_key(&repo, &cfg).await.unwrap();
        assert_eq!(k1.master(), k2.master(), "persisted key must be reused");
    }
}
