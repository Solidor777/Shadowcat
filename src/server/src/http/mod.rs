//! The HTTP surface: REST routes, asset serving, module serving, the
//! embedded SPA, error mapping, and auth throttles.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

/// Asset upload/serve routes + the upload rate limiter.
pub mod assets;
/// The rust-embedded client bundle (`dist/`) served as the SPA.
pub mod embed;
/// `AppError` and its status-code mapping.
pub mod error;
/// Installed-module discovery + path-guarded static serving.
pub mod module_routes;
/// All REST route handlers.
pub mod routes;
/// Login/invite sliding-window throttles.
pub mod throttle;
/// Per-world export/import bundle routes.
pub mod world_bundle;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::config::Config;
use crate::data::sqlite::SqliteRepository;

/// Shared handler state. `initialized` caches "an admin exists" so the init
/// gate avoids a DB hit per request; `setup_token`, when `Some`, is the value
/// `/api/setup` requires.
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
    /// the identical URL -- see `chat::post_publish::PreviewFetchLocks`.
    pub preview_fetch_locks: crate::chat::PreviewFetchLocks,
}

impl AppState {
    /// Resolve the token `/api/setup` will require. `None` = open window.
    pub fn resolve_setup_token(config: &Config) -> Option<String> {
        use crate::config::SetupTokenPolicy;
        match config.setup_token_policy() {
            SetupTokenPolicy::Open => {
                if !config.is_loopback_bind() {
                    tracing::warn!(
                        "setup token disabled on a non-loopback bind; /api/setup is unauthenticated until an admin exists"
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
/// ```text
/// let app = http::router(state).await; // axum::serve(listener, app...)
/// ```
pub async fn router(state: AppState) -> Router {
    use tower::ServiceBuilder;
    use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
    use tower_http::trace::TraceLayer;

    let sessions = crate::auth::session::session_layer(&state.repo, &state.config)
        .await
        .expect("session layer");

    Router::new()
        .route("/health", get(routes::health))
        .route("/api/config", get(routes::config))
        .route("/ws", get(crate::ws::conn::ws_handler))
        .route("/api/debug/rooms", get(routes::debug_rooms))
        .route("/api/admin/backup", post(routes::admin_backup))
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
                .get(assets::list)
                .layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/api/assets/{uuid}",
            get(assets::serve).delete(assets::delete),
        )
        .route(
            "/api/assets/{uuid}/replace",
            post(assets::replace).layer(DefaultBodyLimit::disable()),
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

#[cfg(test)]
pub(crate) mod tests;
