#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

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
use tower_sessions::session_store::{self, ExpiredDeletion, SessionStore};
use tower_sessions::{Expiry, Session, SessionManagerLayer};
use uuid::Uuid;

use crate::auth::role::ServerRole;
use crate::config::Config;
use crate::data::sqlite::SqliteRepository;
use crate::http::error::AppError;
use crate::http::AppState;

/// Session-record key the logged-in identity is stored under.
const SESSION_USER_KEY: &str = "user";
/// `settings`-table key of the DB-persisted cookie signing key — the reason
/// sessions survive a restart with `Config.session_key` unset
/// (`load_or_create_key`).
const SESSION_KEY_SETTING: &str = "session_key";

/// DB-backed session store over the data layer's sqlx 0.9 pool. A separate
/// `tower-sessions-sqlx-store` is not used: it pins sqlx 0.8, which would
/// duplicate the driver and require a second pool — breaking the single-writer
/// invariant. Sharing the existing pool keeps one writer and one sqlx version.
#[derive(Debug, Clone)]
pub struct SqlxSqliteStore {
    /// The shared single-writer pool — every write (`create`/`save`/
    /// `delete`/`delete_expired`/`migrate`) goes through this, never
    /// `read_pool`. `delete_user` (`data::sqlite::SqliteRepository`) relies
    /// on session deletion sharing this pool's transaction semantics.
    pool: SqlitePool,
    /// A dedicated read-only pool for the hot path (`load`/`id_exists`),
    /// which runs on every authenticated request and must not queue behind
    /// an in-flight app write on `pool`. See
    /// `data::sqlite::SqliteRepository::open_read_pool`'s doc for why this
    /// can't be built from a second, independent parse of the same URL.
    read_pool: SqlitePool,
}

impl SqlxSqliteStore {
    /// A store over the shared write pool and a dedicated read pool.
    ///
    /// # Examples
    ///
    /// ```text
    /// let store = SqlxSqliteStore::new(repo.pool().clone(), repo.open_read_pool().await?); // session_layer wires this
    /// ```
    pub fn new(pool: SqlitePool, read_pool: SqlitePool) -> Self {
        Self { pool, read_pool }
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

    /// Whether a session row with `id` exists (collision probe for `create`).
    ///
    /// # Examples
    ///
    /// ```text
    /// store.id_exists(&id).await? // true -> cycle a fresh id
    /// ```
    async fn id_exists(&self, id: &Id) -> session_store::Result<bool> {
        let row = sqlx::query("SELECT 1 FROM tower_sessions WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.read_pool)
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
            .fetch_optional(&self.read_pool)
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

#[async_trait]
impl ExpiredDeletion for SqlxSqliteStore {
    /// Delete rows past expiry. Mirrors `load`'s validity check (`expiry_date >
    /// now`): a row is expired — and unloadable — once `expiry_date <= now`.
    async fn delete_expired(&self) -> session_store::Result<()> {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        sqlx::query("DELETE FROM tower_sessions WHERE expiry_date <= ?")
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|e| session_store::Error::Backend(e.to_string()))?;
        Ok(())
    }
}

/// Interval between session-table sweeps. Sessions expire on 7-day inactivity,
/// so a daily sweep bounds table growth with ample margin.
const SESSION_SWEEP_PERIOD: std::time::Duration = std::time::Duration::from_secs(60 * 60 * 24);

/// Retention past `expires_at` before a spent invite row is deleted. Consumed
/// and revoked rows also age out through this: every row carries the 7-day
/// mint TTL, so all spent rows are gone within TTL + grace. 30 days keeps
/// recent redemption provenance (`consumed_by`) inspectable for a while
/// without unbounded growth.
const INVITE_GC_GRACE_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// Delete invite rows whose `expires_at` is more than the grace period past.
/// Correctness does not depend on this: expired rows are already unredeemable
/// (`consume_invite`'s guarded UPDATE); this bounds table growth.
pub(crate) async fn sweep_spent_invites(
    pool: &sqlx::SqlitePool,
    now_ms: i64,
) -> Result<u64, sqlx::Error> {
    let cutoff = now_ms - INVITE_GC_GRACE_MS;
    let res = sqlx::query("DELETE FROM world_invites WHERE expires_at <= ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// Spawn a background task that periodically deletes expired session rows
/// and spent world-invite rows. Housekeeping, not correctness: expired rows
/// are already unloadable/unredeemable; the sweep bounds unbounded table
/// growth. Sweeps once at startup, then every `SESSION_SWEEP_PERIOD`. A
/// failed sweep is logged and retried next tick — it never aborts the server.
pub fn spawn_session_sweep(repo: &SqliteRepository) {
    // Synchronous fn (no `open_read_pool().await` available here) and this
    // sweep only ever calls `delete_expired` (a write), never `load`/
    // `id_exists` — the write pool suffices for both arguments; this is the
    // one deliberate exception to "every caller passes a real read pool".
    let store = SqlxSqliteStore::new(repo.pool().clone(), repo.pool().clone());
    let pool = repo.pool().clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(SESSION_SWEEP_PERIOD);
        loop {
            interval.tick().await; // first tick completes immediately
            if let Err(e) = store.delete_expired().await {
                tracing::warn!(error = %e, "session sweep failed");
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            if let Err(e) = sweep_spent_invites(&pool, now).await {
                tracing::warn!(error = %e, "invite sweep failed");
            }
        }
    });
}

/// Identity persisted in the session store after login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionUser {
    /// Account id.
    pub id: Uuid,
    /// Account username at login time.
    pub username: String,
    /// Server tier at login time.
    pub role: ServerRole,
}

/// Any authenticated user.
#[derive(Debug, Clone)]
pub struct AuthUser {
    /// Account id.
    pub id: Uuid,
    /// Account username.
    pub username: String,
    /// Server tier (the `AdminUser` extractor additionally requires `Admin`).
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
        Ok(AuthUser {
            id: u.id,
            username: u.username,
            role: u.role,
        })
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
        return Key::try_from(raw.as_slice())
            .map_err(|e| anyhow::anyhow!("session key invalid (needs >= 64 bytes): {e}"));
    }
    if let Some(stored) = repo.get_setting(SESSION_KEY_SETTING).await? {
        let raw = base64::engine::general_purpose::STANDARD.decode(stored)?;
        return Key::try_from(raw.as_slice())
            .map_err(|e| anyhow::anyhow!("stored session key invalid (needs >= 64 bytes): {e}"));
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
    let read_pool = repo.open_read_pool().await?;
    let store = SqlxSqliteStore::new(repo.pool().clone(), read_pool);
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
                SessionUser {
                    id: Uuid::from_u128(1),
                    username: "a".into(),
                    role: ServerRole::Admin,
                },
            )
            .await
            .unwrap();
        "ok"
    }
    async fn login_as_user(session: Session) -> &'static str {
        session
            .insert(
                "user",
                SessionUser {
                    id: Uuid::from_u128(2),
                    username: "u".into(),
                    role: ServerRole::User,
                },
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
        (
            axum_test::TestServer::builder()
                .save_cookies()
                .build(app)
                .unwrap(),
            (),
        )
    }

    #[tokio::test]
    async fn auth_user_requires_session() {
        let (server, _) = harness().await;
        server
            .get("/t/me")
            .await
            .assert_status(axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn admin_extractor_rejects_non_admin() {
        let (server, _) = harness().await;
        server.post("/t/login-user").await.assert_status_ok();
        server.get("/t/me").await.assert_status_ok(); // any user passes AuthUser
        server
            .get("/t/admin")
            .await
            .assert_status(axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_extractor_accepts_admin() {
        let (server, _) = harness().await;
        server.post("/t/login-admin").await.assert_status_ok();
        server.get("/t/admin").await.assert_status_ok();
    }

    #[tokio::test]
    async fn delete_expired_removes_only_expired_rows() {
        let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:")
            .await
            .unwrap();
        let read_pool = repo.open_read_pool().await.unwrap();
        let store = SqlxSqliteStore::new(repo.pool().clone(), read_pool);
        store.migrate().await.unwrap();
        let now = OffsetDateTime::now_utc().unix_timestamp();

        // Boundary row (expiry == now) is non-loadable, so it must be swept too.
        for (id, expiry) in [
            ("expired", now - 100),
            ("boundary", now),
            ("live", now + 10_000),
        ] {
            sqlx::query("INSERT INTO tower_sessions (id, data, expiry_date) VALUES (?, '{}', ?)")
                .bind(id)
                .bind(expiry)
                .execute(repo.pool())
                .await
                .unwrap();
        }

        store.delete_expired().await.unwrap();

        let remaining: Vec<String> =
            sqlx::query_scalar("SELECT id FROM tower_sessions ORDER BY id")
                .fetch_all(repo.pool())
                .await
                .unwrap();
        assert_eq!(remaining, vec!["live".to_string()]);
    }

    #[tokio::test]
    async fn session_key_is_stable_across_loads() {
        let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:")
            .await
            .unwrap();
        let cfg = crate::config::Config::default();
        let k1 = load_or_create_key(&repo, &cfg).await.unwrap();
        let k2 = load_or_create_key(&repo, &cfg).await.unwrap();
        assert_eq!(k1.master(), k2.master(), "persisted key must be reused");
    }

    #[tokio::test]
    async fn sweep_deletes_only_rows_expired_past_grace() {
        use crate::data::sqlite::{NewInvite, SqliteRepository};
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo
            .create_user("gm", None, crate::auth::role::ServerRole::User, 0)
            .await
            .unwrap();
        let world = repo.create_world_owned("W", gm, 0).await.unwrap();
        let now: i64 = 100 * 24 * 60 * 60 * 1000; // day 100
        let mk = |id: u128, expires_at: i64| NewInvite {
            id: Uuid::from_u128(id),
            world: world.id,
            secret_hash: "x",
            role: crate::data::document::WorldRole::Player,
            created_by: gm,
            now: 0,
            expires_at,
        };
        // Expired 31 days ago -> swept. Expired 1 day ago -> kept (inside grace).
        repo.create_invite(mk(1, now - 31 * 24 * 60 * 60 * 1000), 64)
            .await
            .unwrap();
        repo.create_invite(mk(2, now - 24 * 60 * 60 * 1000), 64)
            .await
            .unwrap();
        let deleted = super::sweep_spent_invites(repo.pool(), now).await.unwrap();
        assert_eq!(deleted, 1);
        assert!(repo
            .invite_by_id(Uuid::from_u128(1))
            .await
            .unwrap()
            .is_none());
        assert!(repo
            .invite_by_id(Uuid::from_u128(2))
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn session_load_does_not_queue_behind_an_open_write_transaction() {
        let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:")
            .await
            .unwrap();
        let read_pool = repo.open_read_pool().await.unwrap();
        let store = SqlxSqliteStore::new(repo.pool().clone(), read_pool);
        store.migrate().await.unwrap();

        let mut record = Record {
            id: Id::default(),
            data: Default::default(),
            expiry_date: OffsetDateTime::now_utc() + Duration::days(1),
        };
        store.create(&mut record).await.unwrap();

        // Hold the write pool's single connection open in an uncommitted
        // transaction. Before this fix, `load()` shared this exact pool
        // (max_connections(1)), so it would have zero free connections to
        // acquire and block until this transaction ends.
        let tx = repo.pool().begin().await.unwrap();

        let loaded =
            tokio::time::timeout(std::time::Duration::from_secs(2), store.load(&record.id))
                .await
                .expect("load() must not queue behind an open write-pool transaction")
                .unwrap();
        assert_eq!(loaded.unwrap().id, record.id);

        tx.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn read_pool_shares_the_same_in_memory_database_as_the_write_pool() {
        let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:")
            .await
            .unwrap();
        let read_pool = repo.open_read_pool().await.unwrap();
        let store = SqlxSqliteStore::new(repo.pool().clone(), read_pool);
        store.migrate().await.unwrap();

        let mut record = Record {
            id: Id::default(),
            data: Default::default(),
            expiry_date: OffsetDateTime::now_utc() + Duration::days(1),
        };
        store.create(&mut record).await.unwrap();

        // If the read pool were built from a second, independent parse of
        // "sqlite::memory:" (the landmine this brief investigated), it would
        // point at a different, empty database and this would return None.
        let loaded = store.load(&record.id).await.unwrap();
        assert!(
            loaded.is_some(),
            "read pool must see data written via the write pool"
        );
    }

    #[tokio::test]
    async fn read_pool_rejects_a_write() {
        // File-backed, not "sqlite::memory:": a named shared-cache in-memory
        // database does not enforce `SQLITE_OPEN_READONLY` against a write
        // from a connection sharing that cache (measured — a raw sqlx probe
        // with no shadowcat code in the loop still lets the write through),
        // while a real on-disk database does. Production always connects to
        // a real file, so this exercises the shape that matters; the
        // in-memory URL this repo's other tests share would pass unconditionally
        // here regardless of whether `.read_only(true)` is even applied.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("read_only_probe.db");
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        let repo = crate::data::sqlite::SqliteRepository::connect(&url)
            .await
            .unwrap();
        let read_pool = repo.open_read_pool().await.unwrap();
        let result = sqlx::query("CREATE TABLE t (x INTEGER)")
            .execute(&read_pool)
            .await;
        assert!(result.is_err(), "a read-only pool must reject a write");
    }
}
