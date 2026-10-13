//! This module provides the DB-backed session store, the per-scheme session layers, and the
//! `AuthUser`/`AdminUser` extractors.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{Request, Response};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tower::{Layer, Service};
use tower_sessions::cookie::time::{Duration, OffsetDateTime};
use tower_sessions::cookie::{Key, SameSite};
use tower_sessions::service::{SessionManager, SignedCookie};
use tower_sessions::session::{Id, Record};
use tower_sessions::session_store::{self, ExpiredDeletion, SessionStore};
use tower_sessions::{Expiry, Session, SessionManagerLayer};
use uuid::Uuid;

use crate::auth::role::ServerRole;
use crate::config::Config;
use crate::data::sqlite::SqliteRepository;
use crate::http::error::AppError;
use crate::http::scheme::{request_scheme, RequestScheme};
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
///
/// # Examples
///
/// ```
/// use shadowcat::auth::session::SqlxSqliteStore;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use tower_sessions::cookie::time::{Duration, OffsetDateTime};
/// use tower_sessions::session::Record;
/// use tower_sessions::session_store::SessionStore;
///
/// # #[tokio::main] async fn main() {
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let store = SqlxSqliteStore::new(repo.pool().clone(), repo.pool().clone());
/// store.migrate().await.unwrap();
///
/// let record = Record {
///     id: Default::default(),
///     data: Default::default(),
///     expiry_date: OffsetDateTime::now_utc() + Duration::days(1),
/// };
/// store.save(&record).await.unwrap();
/// assert!(store.load(&record.id).await.unwrap().is_some());
/// # }
/// ```
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
    /// ```
    /// use shadowcat::auth::session::SqlxSqliteStore;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use tower_sessions::cookie::time::{Duration, OffsetDateTime};
    /// use tower_sessions::session::Record;
    /// use tower_sessions::session_store::SessionStore;
    ///
    /// # #[tokio::main] async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let read_pool = repo.open_read_pool().await.unwrap(); // session_layer wires this
    /// let store = SqlxSqliteStore::new(repo.pool().clone(), read_pool);
    /// store.migrate().await.unwrap();
    ///
    /// let record = Record {
    ///     id: Default::default(),
    ///     data: Default::default(),
    ///     expiry_date: OffsetDateTime::now_utc() + Duration::days(1),
    /// };
    /// store.save(&record).await.unwrap();
    /// assert!(store.load(&record.id).await.unwrap().is_some());
    /// # }
    /// ```
    pub fn new(pool: SqlitePool, read_pool: SqlitePool) -> Self {
        Self { pool, read_pool }
    }

    /// Create the session table if absent. Run once at startup.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::auth::session::SqlxSqliteStore;
    /// use shadowcat::data::sqlite::SqliteRepository;
    ///
    /// # #[tokio::main] async fn main() {
    /// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    /// let store = SqlxSqliteStore::new(repo.pool().clone(), repo.pool().clone());
    /// assert!(store.migrate().await.is_ok());
    /// assert!(store.migrate().await.is_ok()); // idempotent: CREATE TABLE IF NOT EXISTS
    /// # }
    /// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::auth::session::spawn_session_sweep;
/// use shadowcat::data::sqlite::SqliteRepository;
///
/// # #[tokio::main] async fn main() {
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// spawn_session_sweep(&repo); // background task; does not block the caller
///
/// // The async runtime keeps serving other work while the sweep task is live.
/// repo.set_setting("probe", "ok").await.unwrap();
/// assert_eq!(repo.get_setting("probe").await.unwrap().as_deref(), Some("ok"));
/// # }
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::SessionUser;
/// use uuid::Uuid;
///
/// let user = SessionUser {
///     id: Uuid::new_v4(),
///     username: "alice".to_string(),
///     role: ServerRole::User,
/// };
/// let json = serde_json::to_string(&user).unwrap();
/// let round_tripped: SessionUser = serde_json::from_str(&json).unwrap();
/// assert_eq!(round_tripped.username, "alice");
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::AuthUser;
/// use uuid::Uuid;
///
/// let user = AuthUser {
///     id: Uuid::new_v4(),
///     username: "alice".to_string(),
///     role: ServerRole::User,
/// };
/// assert_eq!(user.role, ServerRole::User);
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::{AdminUser, AuthUser};
/// use uuid::Uuid;
///
/// let admin = AdminUser(AuthUser {
///     id: Uuid::new_v4(),
///     username: "root".to_string(),
///     role: ServerRole::Admin,
/// });
/// assert_eq!(admin.0.role, ServerRole::Admin);
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::auth::session::load_or_create_key;
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
///
/// # #[tokio::main] async fn main() {
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let config = Config::default();
/// let key = load_or_create_key(&repo, &config).await.unwrap();
/// // A second call reads the same persisted key back.
/// let again = load_or_create_key(&repo, &config).await.unwrap();
/// assert_eq!(key.master(), again.master());
/// # }
/// ```
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

/// Session cookie name on plain-HTTP requests.
pub const SESSION_COOKIE_PLAIN: &str = "sc_sid";
/// Session cookie name on HTTPS requests. The `__Host-` prefix makes a browser refuse to store
/// the cookie unless it is `Secure`, has `Path=/` and no `Domain`, and blocks a plain-HTTP network
/// attacker from planting a cookie into the HTTPS session.
pub const SESSION_COOKIE_TLS: &str = "__Host-sc_sid";

/// The two per-scheme session layers over one store and one signing key, dispatched per request on
/// `http::scheme::request_scheme`. `tower-sessions` fixes `Secure` once per layer and hands its
/// cookie controller no request, so the scheme decision cannot live inside it: a single static
/// `Secure` value would break either LAN plain-HTTP login (`Secure` on) or HTTPS session
/// protection (`Secure` off).
///
/// Distinct cookie names per scheme (`SESSION_COOKIE_PLAIN` / `SESSION_COOKIE_TLS`) are required
/// because cookies are not port-isolated: a non-`Secure` cookie cannot overwrite an existing
/// `Secure` cookie of the same name, so one shared name would let a plain-HTTP login fail to
/// replace an HTTPS session cookie on the same host. Consequence: one session per scheme.
///
/// # Examples
///
/// ```
/// use shadowcat::auth::session::session_layer;
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
///
/// # #[tokio::main] async fn main() {
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let layer = session_layer(&repo, &Config::default()).await.unwrap();
/// let _router = axum::Router::<()>::new().layer(layer);
/// # }
/// ```
#[derive(Clone)]
pub struct SchemeSessionLayer {
    /// Layer for plain-HTTP requests.
    http: SessionManagerLayer<SqlxSqliteStore, SignedCookie>,
    /// Layer for HTTPS requests.
    https: SessionManagerLayer<SqlxSqliteStore, SignedCookie>,
    /// Trusted-proxy list source for the scheme decider.
    config: Arc<Config>,
}

/// One of the two wrapped session services (each already includes its `CookieManager`).
type SessionService<S> =
    tower_cookies::CookieManager<SessionManager<S, SqlxSqliteStore, SignedCookie>>;

/// The service `SchemeSessionLayer` produces: it forwards each request to the plain-HTTP or the
/// HTTPS session service according to `http::scheme::request_scheme`.
///
/// # Examples
///
/// ```
/// use shadowcat::auth::session::{session_layer, SchemeSession};
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use tower::Layer;
///
/// # #[tokio::main] async fn main() {
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let layer = session_layer(&repo, &Config::default()).await.unwrap();
/// let inner = tower::service_fn(|_req: axum::extract::Request| async {
///     Ok::<_, std::convert::Infallible>(axum::response::Response::new(axum::body::Body::empty()))
/// });
/// let _service: SchemeSession<_> = layer.layer(inner);
/// # }
/// ```
#[derive(Clone)]
pub struct SchemeSession<S> {
    /// Plain-HTTP flavour.
    http: SessionService<S>,
    /// HTTPS flavour.
    https: SessionService<S>,
    /// Trusted-proxy list source for the scheme decider.
    config: Arc<Config>,
}

impl<S> Layer<S> for SchemeSessionLayer
where
    S: Clone,
{
    type Service = SchemeSession<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SchemeSession {
            http: self.http.layer(inner.clone()),
            https: self.https.layer(inner),
            config: self.config.clone(),
        }
    }
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for SchemeSession<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send,
    S::Error: Send,
    ReqBody: Send + 'static,
    ResBody: Default + Send + 'static,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self.http.poll_ready(cx) {
            Poll::Ready(Ok(())) => self.https.poll_ready(cx),
            other => other,
        }
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let scheme = request_scheme(req.extensions(), req.headers(), &self.config);
        match scheme {
            RequestScheme::Http => Box::pin(self.http.call(req)),
            RequestScheme::Https => {
                let fut = self.https.call(req);
                Box::pin(async move {
                    let mut res = fut.await?;
                    normalise_host_cookies(res.headers_mut());
                    Ok(res)
                })
            }
        }
    }
}

/// Rewrites every `Set-Cookie` naming a `__Host-` cookie to carry `Secure; Path=/; HttpOnly;
/// SameSite=Lax` and no `Domain`. `tower-sessions` builds its removal cookie from the cookie
/// PARSED FROM THE REQUEST (name and value only, then `Path`/`Domain` added), so a logout removal
/// would otherwise lack `Secure`, and a browser ignores a `__Host-` `Set-Cookie` without it: the
/// cookie would never be deleted. One normaliser over every HTTPS-flavour `Set-Cookie` (set and
/// removal alike) is the single mechanism guaranteeing the prefix's requirements.
///
/// # Examples
///
/// ```
/// use axum::http::{header, HeaderMap, HeaderValue};
/// use shadowcat::auth::session::normalise_host_cookies;
///
/// let mut h = HeaderMap::new();
/// h.append(header::SET_COOKIE, HeaderValue::from_static("__Host-sc_sid=; Max-Age=0; Path=/"));
/// normalise_host_cookies(&mut h);
/// let v = h.get(header::SET_COOKIE).unwrap().to_str().unwrap();
/// assert!(v.contains("Secure") && v.contains("HttpOnly") && v.contains("SameSite=Lax"));
/// ```
pub fn normalise_host_cookies(headers: &mut axum::http::HeaderMap) {
    use axum::http::header::SET_COOKIE;
    use axum::http::HeaderValue;
    let rewritten: Vec<HeaderValue> = headers
        .get_all(SET_COOKIE)
        .iter()
        .map(|raw| {
            let Ok(text) = raw.to_str() else {
                return raw.clone();
            };
            if !text.starts_with("__Host-") {
                return raw.clone();
            }
            let Ok(mut cookie) = tower_sessions::cookie::Cookie::parse(text.to_owned()) else {
                return raw.clone();
            };
            cookie.set_secure(true);
            cookie.set_http_only(true);
            cookie.set_same_site(SameSite::Lax);
            cookie.set_path("/");
            cookie.unset_domain();
            HeaderValue::from_str(&cookie.to_string()).unwrap_or_else(|_| raw.clone())
        })
        .collect();
    headers.remove(SET_COOKIE);
    for v in rewritten {
        headers.append(SET_COOKIE, v);
    }
}

/// Build the signed, DB-backed per-scheme session layer: plain-HTTP requests get a non-`Secure`
/// `sc_sid` cookie (browsers refuse to store a `Secure` cookie set from a non-secure,
/// non-loopback origin, which would make LAN login impossible), HTTPS requests get a
/// `Secure` `__Host-sc_sid` cookie. Which applies is decided per request by
/// `http::scheme::request_scheme`.
///
/// # Examples
///
/// ```
/// use shadowcat::auth::session::session_layer;
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
///
/// # #[tokio::main] async fn main() {
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let config = Config::default();
/// let layer = session_layer(&repo, &config).await;
/// assert!(layer.is_ok());
/// # }
/// ```
pub async fn session_layer(
    repo: &SqliteRepository,
    config: &Config,
) -> anyhow::Result<SchemeSessionLayer> {
    let read_pool = repo.open_read_pool().await?;
    let store = SqlxSqliteStore::new(repo.pool().clone(), read_pool);
    store.migrate().await?;
    let key = load_or_create_key(repo, config).await?;
    let base = |secure: bool, name: &'static str| {
        SessionManagerLayer::new(store.clone())
            .with_secure(secure)
            .with_name(name)
            .with_same_site(SameSite::Lax)
            .with_expiry(Expiry::OnInactivity(Duration::days(7)))
            .with_signed(key.clone())
    };
    Ok(SchemeSessionLayer {
        http: base(false, SESSION_COOKIE_PLAIN),
        https: base(true, SESSION_COOKIE_TLS),
        config: Arc::new(config.clone()),
    })
}

#[cfg(test)]
mod tests;
