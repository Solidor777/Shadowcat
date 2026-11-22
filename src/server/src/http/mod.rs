//! The HTTP surface: REST routes, asset serving, module serving, the
//! embedded SPA, error mapping, and auth throttles.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod assets;
pub mod backups;
pub mod compression;
pub mod conn_serve;
pub mod embed;
pub mod error;
pub mod module_routes;
pub mod peer_slots;
pub mod routes;
pub mod scheme;
pub mod throttle;
pub mod trash;
pub mod world_bundle;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::config::Config;
use crate::data::sqlite::SqliteRepository;

pub use scheme::into_service;

/// Shared handler state. `initialized` caches "an admin exists" so the init
/// gate avoids a DB hit per request; `setup_token`, when `Some`, is the value
/// `/api/setup` requires.
///
/// # Examples
///
/// ```
/// use std::sync::atomic::AtomicBool;
/// use std::sync::Arc;
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::AppState;
///
/// # #[tokio::main] async fn main() {
/// let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
/// let state = AppState {
///     repo,
///     config: Arc::new(Config::default()),
///     setup_token: None,
///     initialized: Arc::new(AtomicBool::new(false)),
///     ws: shadowcat::ws::WsState::new(),
///     upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
///     uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
///     auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
///     write_barrier: Arc::new(tokio::sync::RwLock::new(())),
///     preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
///     file_permits: Arc::new(shadowcat::http::assets::permits::StreamedFilePermits::production()),
///     tls: shadowcat::tls::TlsRuntime::disabled(),
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(std::env::temp_dir(), shadowcat::trash::TrashRetention::Disabled)),
/// };
/// assert!(!state.initialized.load(std::sync::atomic::Ordering::Relaxed));
/// # }
/// ```
#[derive(Clone)]
pub struct AppState {
    /// The SQLite repository (single-writer pool).
    pub repo: Arc<SqliteRepository>,
    /// Effective layered configuration.
    pub config: Arc<Config>,
    /// The token `/api/setup` requires; `None` = open setup window.
    pub setup_token: Option<String>,
    /// Cached "an admin exists" bit (avoids a DB hit per request).
    pub initialized: Arc<AtomicBool>,
    /// Realtime rooms + per-user limiters.
    pub ws: crate::ws::WsState,
    /// Per-user upload budget.
    pub upload_rate: Arc<assets::UploadRateLimiter>,
    /// In-flight chunked upload sessions (`assets::uploads`).
    pub uploads: Arc<assets::uploads::UploadSessions>,
    /// Login/invite abuse throttles (per identity + per IP).
    pub auth_throttle: Arc<throttle::AuthThrottle>,
    /// Write-quiesce barrier for the in-server backup route: held in write mode
    /// across `create_backup`'s `VACUUM INTO` + assets copy so no asset
    /// commit+rename interleaves with the snapshot. Asset writers hold the read
    /// side (shared among themselves, blocked by the writer).
    pub write_barrier: Arc<tokio::sync::RwLock<()>>,
    /// Process-wide per-URL fetch-lock registry serializing concurrent
    /// `chat::post_publish` link-preview-image/oEmbed-thumbnail resolves for
    /// the identical URL in one world -- see `chat::post_publish::PreviewFetchLocks`.
    pub preview_fetch_locks: crate::chat::PreviewFetchLocks,
    /// Permits bounding the asset files held open while streaming to peers
    /// (`assets::permits`); its capacity is the descriptor term `fd_budget` reserves.
    pub file_permits: Arc<assets::permits::StreamedFilePermits>,
    /// The TLS subsystem: status published by `/api/config` and the admin route.
    pub tls: Arc<crate::tls::TlsRuntime>,
    /// The trash every production file removal goes through (`trash::Trash`).
    pub trash: Arc<crate::trash::Trash>,
}

impl AppState {
    /// Resolve the token `/api/setup` will require. `None` = open window.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::config::Config;
    /// use shadowcat::http::AppState;
    ///
    /// let cfg = Config {
    ///     setup_token: "the-token".into(),
    ///     ..Config::default()
    /// };
    /// assert_eq!(AppState::resolve_setup_token(&cfg), Some("the-token".to_string()));
    /// ```
    pub fn resolve_setup_token(config: &Config) -> Option<String> {
        use crate::config::SetupTokenPolicy;
        match config.setup_token_policy() {
            SetupTokenPolicy::Open => {
                if config.is_exposed() {
                    tracing::warn!(
                        "setup token disabled on an exposed deployment (non-loopback bind, or loopback behind a declared TLS proxy); /api/setup is unauthenticated until an admin exists"
                    );
                }
                None
            }
            SetupTokenPolicy::Required(Some(v)) => Some(v),
            SetupTokenPolicy::Required(None) => {
                let token = uuid::Uuid::new_v4().simple().to_string();
                tracing::info!(%token, "setup token required; enter it in the setup form the app shows on first run");
                Some(token)
            }
        }
    }
}

/// Build the complete axum router: sessions, request-id + trace layers, all
/// REST/WS/asset/module routes, and the embedded SPA fallback.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use std::sync::atomic::AtomicBool;
/// use std::sync::Arc;
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::{router, AppState};
///
/// let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
/// let state = AppState {
///     repo,
///     config: Arc::new(Config::default()),
///     setup_token: None,
///     initialized: Arc::new(AtomicBool::new(true)),
///     ws: shadowcat::ws::WsState::new(),
///     upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
///     uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
///     auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
///     write_barrier: Arc::new(tokio::sync::RwLock::new(())),
///     preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
///     file_permits: Arc::new(shadowcat::http::assets::permits::StreamedFilePermits::production()),
///     tls: shadowcat::tls::TlsRuntime::disabled(),
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(std::env::temp_dir(), shadowcat::trash::TrashRetention::Disabled)),
/// };
/// let app = router(state).await;
/// // `no_run`: binding a real listener needs a live socket.
/// let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
/// axum::serve(listener, app.into_make_service()).await.unwrap();
/// # }
/// ```
pub async fn router(state: AppState) -> Router {
    use axum::http::{header, HeaderValue};
    use tower::ServiceBuilder;
    use tower_http::compression::CompressionLayer;
    use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
    use tower_http::set_header::SetResponseHeaderLayer;
    use tower_http::trace::TraceLayer;

    let sessions = crate::auth::session::session_layer(&state.repo, &state.config)
        .await
        .expect("session layer");

    // Response headers every route gets. `X-Frame-Options`/`Referrer-Policy`/
    // `X-Content-Type-Options` are unconditional; a Content-Security-Policy is deliberately
    // NOT set here. `Strict-Transport-Security` is decided per request by
    // `scheme::hsts_max_age` (opt-in, client-facing HTTPS only, never for a connection served
    // with the self-signed fallback certificate): HSTS converts every future `http://host:*` in
    // the browser to HTTPS, so a static or scheme-blind header would end plain-HTTP play. These
    // layers are NOT the outermost wrapping: the request-id + trace layers below wrap them, so a
    // request-id is stamped and the trace span opened before these headers run, and they still
    // land on error responses.
    let security_headers = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ));
    let hsts_config = state.config.clone();
    let hsts_header = axum::middleware::from_fn(
        move |req: axum::extract::Request, next: axum::middleware::Next| {
            let max_age = scheme::hsts_max_age(req.extensions(), req.headers(), &hsts_config);
            async move {
                let mut res = next.run(req).await;
                if let Some(secs) = max_age {
                    if let Ok(v) = HeaderValue::from_str(&format!("max-age={secs}")) {
                        res.headers_mut()
                            .insert(header::STRICT_TRANSPORT_SECURITY, v);
                    }
                }
                res
            }
        },
    );

    Router::new()
        .route("/health", get(routes::health))
        .route("/api/config", get(routes::config))
        // GET for HTTP/1.1 upgrades, CONNECT for RFC 8441 WebSockets over HTTP/2 (the
        // listener advertises `h2` via ALPN, and `conn_serve` enables the extended CONNECT
        // protocol on its HTTP/2 connections).
        .route(
            "/ws",
            get(crate::ws::conn::ws_handler).connect(crate::ws::conn::ws_handler),
        )
        .route("/api/debug/rooms", get(routes::debug_rooms))
        .route("/api/admin/backup", post(backups::admin_backup))
        .route("/api/admin/backups", get(backups::list))
        .route("/api/admin/backups/prune", post(backups::prune_now))
        .route("/api/admin/backups/{id}", delete(backups::delete_backup))
        .route(
            "/api/admin/backups/{id}/restore",
            post(backups::restore_from_backup),
        )
        .route("/api/admin/tls", get(routes::admin_tls))
        .route("/api/admin/trash", get(trash::list_trash))
        .route("/api/admin/trash/empty", post(trash::empty_trash))
        .route(
            "/api/admin/trash/{entry_id}",
            delete(trash::delete_trash_entry),
        )
        .route("/api/me", get(routes::me))
        .route(
            "/api/me/ui-state",
            get(routes::get_ui_state).put(routes::put_ui_state),
        )
        .route("/api/login", post(routes::login))
        .route("/api/logout", post(routes::logout))
        .route("/api/setup", post(routes::setup))
        .route(
            "/api/users",
            post(routes::create_user).get(routes::list_users),
        )
        .route(
            "/api/worlds",
            post(routes::create_world).get(routes::list_worlds),
        )
        .route("/api/users/{id}", delete(routes::delete_user))
        .route(
            "/api/users/{id}/password",
            post(routes::reset_user_password),
        )
        .route("/api/worlds/{id}", delete(routes::delete_world))
        .route("/api/worlds/{id}/export", post(world_bundle::export_world))
        .route(
            "/api/worlds/import",
            post(world_bundle::import_world).layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/api/worlds/{id}/members",
            get(routes::list_members).post(routes::add_member),
        )
        .route(
            "/api/worlds/{id}/members/{user}",
            delete(routes::remove_member),
        )
        .route(
            "/api/worlds/{id}/invites",
            post(routes::create_invite).get(routes::list_invites),
        )
        .route(
            "/api/worlds/{id}/invites/{code_id}",
            delete(routes::revoke_invite),
        )
        // The code travels in the BODY: a path segment is recorded by the trace
        // span's `uri`, browser history, `Referer`, and proxy logs.
        .route("/api/invites/accept", post(routes::accept_invite))
        .route(
            "/api/worlds/{id}/capability-defaults",
            get(routes::get_world_capability_defaults).put(routes::set_world_capability_defaults),
        )
        .route(
            "/api/worlds/{id}/capability-requirements",
            get(routes::get_world_capability_requirements)
                .put(routes::set_world_capability_requirements),
        )
        .route(
            "/api/worlds/{id}/contracts",
            get(routes::get_world_contract_declarations)
                .put(routes::set_world_contract_declarations),
        )
        .route(
            "/api/worlds/{id}/schemas",
            get(routes::get_world_schema_declarations).put(routes::set_world_schema_declarations),
        )
        .route("/api/modules", get(module_routes::list_installed_modules))
        .route(
            "/modules/{id}/{*path}",
            get(module_routes::serve_module_file),
        )
        .route(
            "/api/worlds/{id}/enabled-modules",
            get(module_routes::get_world_enabled_modules)
                .put(module_routes::set_world_enabled_modules),
        )
        .route(
            "/api/worlds/{id}/documents",
            get(routes::list_documents).post(routes::create_document),
        )
        .route("/api/worlds/{id}/snapshot", get(routes::world_snapshot))
        .route(
            "/api/documents/{id}",
            get(routes::get_document)
                .patch(routes::patch_document)
                .delete(routes::delete_document),
        )
        .route(
            "/api/worlds/{world}/assets",
            post(assets::upload)
                .get(assets::query::list)
                .layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/api/assets/{uuid}",
            get(assets::serve)
                .delete(assets::delete)
                .patch(assets::mutate::patch),
        )
        .route("/api/assets/{uuid}/meta", get(assets::meta))
        .route(
            "/api/assets/{uuid}/replace",
            post(assets::replace).layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/api/worlds/{world}/assets/bulk",
            post(assets::mutate::bulk),
        )
        .route(
            "/api/asset-folders/{id}",
            delete(assets::mutate::delete_folder),
        )
        .route("/api/assets/{uuid}/original", get(assets::mutate::original))
        .route(
            "/api/assets/{uuid}/reconvert",
            post(assets::mutate::reconvert),
        )
        .route(
            "/api/worlds/{world}/assets/uploads",
            post(assets::uploads::create_session),
        )
        .route(
            "/api/assets/uploads/{id}/{offset}",
            put(assets::uploads::put_chunk)
                .layer(DefaultBodyLimit::max(assets::uploads::CHUNK_SIZE as usize)),
        )
        .route(
            "/api/assets/uploads/{id}/complete",
            post(assets::uploads::complete_session),
        )
        .route(
            "/api/assets/uploads/{id}",
            delete(assets::uploads::abort_session),
        )
        .fallback(embed::static_handler)
        .layer(sessions)
        .layer(CompressionLayer::new().compress_when(compression::AssetAndWsSafePredicate::new()))
        .layer(security_headers)
        .layer(hsts_header)
        .layer(
            // Last layer = outermost. Request id is stamped first, the trace span
            // wraps everything (including sessions and the gate), then the id is
            // propagated onto the response.
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(TraceLayer::new_for_http())
                .layer(PropagateRequestIdLayer::x_request_id()),
        )
        .with_state(state)
}

/// Bound on the shutdown drain: [`conn_serve::ConnLimits::drain_timeout`], measured from the moment
/// sessions are told to close. Connections still open past it are closed by an I/O error on their
/// IO and, after [`conn_serve::ABORT_WAIT`], aborted. A connection whose egress loop is merely
/// draining a large resync replay is expected to finish well inside this; one stuck past it must
/// not block process exit indefinitely.
pub(crate) const SHUTDOWN_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The least time WebSocket session tasks get to finish after the connection drain returns, even
/// when the drain deadline has already passed. A session blocked sending to a peer that has
/// stopped reading ends at its Close bound ([`crate::ws::bounded_sink::CLOSE_SEND_TIMEOUT`]) once
/// told to close, so the grace is that bound plus one second of scheduling slack. A refused join
/// obeys the same bound: its error and Close sends end at the Close bound, a failed error send
/// skips the Close, and its wait for the peer's inbound frames (`ws::conn`'s `drain_inbound`)
/// ends at once when sessions are told to close.
///
/// Every documented shutdown bound derives from this constant and the drain timeout: `serve_with`
/// returns within `drain_timeout + SESSION_GRACE`, or `drain_timeout + conn_serve::ABORT_WAIT +
/// SESSION_GRACE` when a connection task outlives its killed IO.
///
/// # Examples
///
/// ```
/// use shadowcat::ws::bounded_sink::CLOSE_SEND_TIMEOUT;
///
/// assert!(shadowcat::http::SESSION_GRACE > CLOSE_SEND_TIMEOUT);
/// ```
pub const SESSION_GRACE: std::time::Duration =
    std::time::Duration::from_secs(crate::ws::bounded_sink::CLOSE_SEND_TIMEOUT.as_secs() + 1);

/// How long `serve_with` waits for session tasks once the connection drain has returned at `now`:
/// whatever remains of the shared drain bound (`deadline`, from
/// [`conn_serve::ServeOutcome::drain_deadline`]), but never less than [`SESSION_GRACE`], so a
/// drain that consumed the whole bound still gives sessions time to send their Close frames.
pub(crate) fn session_wait(
    deadline: tokio::time::Instant,
    now: tokio::time::Instant,
) -> std::time::Duration {
    deadline.saturating_duration_since(now).max(SESSION_GRACE)
}

/// Serve `state`'s router over `source` until `shutdown_fut` resolves, then return — the
/// testable core of the `shadowcat` binary's normal (non-backup/restore) startup path. Runs on
/// [`conn_serve::ConnLimits::for_config`] of `state.config`, the same derivation the binary uses,
/// so the configured `egress_window_secs` applies; [`serve_with`] takes the limits.
///
/// # Errors
///
/// Returns an error when the serve loop ended because `source` ended, not because `shutdown_fut`
/// resolved: a server that can accept no more connections must exit non-zero so a supervisor
/// restarts it.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use std::sync::atomic::AtomicBool;
/// use std::sync::Arc;
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::AppState;
///
/// let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
/// let state = AppState {
///     repo,
///     config: Arc::new(Config::default()),
///     setup_token: None,
///     initialized: Arc::new(AtomicBool::new(false)),
///     ws: shadowcat::ws::WsState::new(),
///     upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
///     uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
///     auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
///     write_barrier: Arc::new(tokio::sync::RwLock::new(())),
///     preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
///     file_permits: Arc::new(shadowcat::http::assets::permits::StreamedFilePermits::production()),
///     tls: shadowcat::tls::TlsRuntime::disabled(),
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(std::env::temp_dir(), shadowcat::trash::TrashRetention::Disabled)),
/// };
/// let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
/// // A `shutdown_fut` that resolves immediately, for illustration only.
/// shadowcat::http::serve(listener, state, async {}).await.unwrap();
/// # }
/// ```
pub async fn serve<S>(
    source: S,
    state: AppState,
    shutdown_fut: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()>
where
    S: conn_serve::ConnSource,
{
    let limits = conn_serve::ConnLimits::for_config(&state.config);
    serve_with(source, state, shutdown_fut, limits).await
}

/// [`serve`] with explicit connection `limits`.
///
/// The serve loop also stops when `WsState::trigger_shutdown` is called from inside the server
/// (the admin restore route), exactly as when `shutdown_fut` resolves.
///
/// [`conn_serve::serve_router`] runs `WsState::trigger_shutdown` as its `on_drain` hook, before
/// it drains, for both causes (`shutdown_fut` resolving and `source` ending): an HTTP/2
/// WebSocket is a stream inside its connection, so a drain that waited for connections before
/// telling sessions to close would wait on them forever. `axum`'s `WebSocketUpgrade::on_upgrade`
/// also detaches an HTTP/1.1 session from the hyper connection that produced its 101, so after
/// the loop returns this function calls `WsState::wait_for_drain` (bounded by
/// `limits.drain_timeout`) so every session's `egress_loop` has a real chance to send its Close
/// frame and exit before the runtime is dropped. That wait shares the drain's bound (one
/// deadline, `limits.drain_timeout` after the hook ran) and is never shorter than
/// [`SESSION_GRACE`].
///
/// # Errors
///
/// Returns an error when the serve loop ended because `source` ended, not because `shutdown_fut`
/// resolved.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use std::sync::atomic::AtomicBool;
/// use std::sync::Arc;
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::conn_serve::ConnLimits;
/// use shadowcat::http::AppState;
///
/// let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
/// let state = AppState {
///     repo,
///     config: Arc::new(Config::default()),
///     setup_token: None,
///     initialized: Arc::new(AtomicBool::new(false)),
///     ws: shadowcat::ws::WsState::new(),
///     upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
///     uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
///     auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
///     write_barrier: Arc::new(tokio::sync::RwLock::new(())),
///     preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
///     file_permits: Arc::new(shadowcat::http::assets::permits::StreamedFilePermits::production()),
///     tls: shadowcat::tls::TlsRuntime::disabled(),
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(std::env::temp_dir(), shadowcat::trash::TrashRetention::Disabled)),
/// };
/// let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
/// let limits = ConnLimits { max_live_per_peer: 8, ..ConnLimits::default() };
/// shadowcat::http::serve_with(listener, state, async {}, limits).await.unwrap();
/// # }
/// ```
pub async fn serve_with<S>(
    source: S,
    state: AppState,
    shutdown_fut: impl std::future::Future<Output = ()> + Send + 'static,
    limits: conn_serve::ConnLimits,
) -> anyhow::Result<()>
where
    S: conn_serve::ConnSource,
{
    let ws_state = state.ws.clone();
    // A `WsState::trigger_shutdown` from inside the server (the admin restore route) stops the
    // serve loop the same way `shutdown_fut` does; both causes then run the one drain below.
    let mut stop_requested = ws_state.shutdown.clone();
    let shutdown_fut = async move {
        tokio::select! {
            () = shutdown_fut => {}
            _ = stop_requested.wait_for(|requested| *requested) => {}
        }
    };
    let app = router(state).await;
    let drain_ws = ws_state.clone();
    let outcome = conn_serve::serve_router(source, app, shutdown_fut, limits, move || {
        drain_ws.trigger_shutdown()
    })
    .await;
    let wait = session_wait(outcome.drain_deadline, tokio::time::Instant::now());
    if !ws_state.wait_for_drain(wait).await {
        tracing::warn!(
            live = ws_state.live_connection_count(),
            blocked_sends = ws_state.pending_sends(),
            "graceful shutdown timed out waiting for websocket connections to close"
        );
    }
    match outcome.end {
        conn_serve::ServeEnd::Shutdown => Ok(()),
        conn_serve::ServeEnd::SourceEnded => Err(anyhow::anyhow!(
            "the connection source ended without a shutdown request; the server can accept no more connections"
        )),
    }
}

#[cfg(test)]
pub(crate) mod tests;
