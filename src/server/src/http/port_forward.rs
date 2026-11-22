//! The admin routes for automatic router port forwarding.
//!
//! Both routes take [`AdminUser`]: the status carries the host's external address, which never
//! reaches a player-visible route. The toggle is one decision made by
//! `portmap::runtime::effective_enabled`; a `port_forward` configuration key locks it.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use ts_rs::TS;

use crate::auth::session::AdminUser;
use crate::http::error::AppError;
use crate::http::AppState;
use crate::portmap::runtime::write_stored_toggle;
use crate::portmap::status::PortMapStatus;

/// `PUT /api/admin/portmap` body.
///
/// # Examples
///
/// ```
/// use shadowcat::http::port_forward::SetPortForward;
/// let body: SetPortForward = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
/// assert!(body.enabled);
/// ```
#[derive(Debug, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct SetPortForward {
    /// The admin's choice, stored and applied at once.
    pub enabled: bool,
}

/// `GET /api/admin/portmap`: the port-forwarding status.
///
/// # Examples
///
/// ```
/// # #[tokio::main] async fn main() {
/// use axum::extract::State;
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::{AdminUser, AuthUser};
/// use shadowcat::http::port_forward::get_status;
/// use shadowcat::http::AppState;
/// use shadowcat::portmap::status::PortMapState;
///
/// let repo = std::sync::Arc::new(shadowcat::data::sqlite::SqliteRepository::connect("sqlite::memory:").await.unwrap());
/// let state = AppState {
///     repo,
///     config: std::sync::Arc::new(shadowcat::config::Config::default()),
///     setup_token: None,
///     initialized: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
///     ws: shadowcat::ws::WsState::new(),
///     upload_rate: std::sync::Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
///     uploads: std::sync::Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
///     auth_throttle: std::sync::Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
///     write_barrier: std::sync::Arc::new(tokio::sync::RwLock::new(())),
///     preview_fetch_locks: std::sync::Arc::new(dashmap::DashMap::new()),
///     file_permits: std::sync::Arc::new(shadowcat::http::assets::permits::StreamedFilePermits::production()),
///     tls: shadowcat::tls::TlsRuntime::disabled(),
///     portmap: shadowcat::portmap::PortMapRuntime::disabled(),
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(std::env::temp_dir(), shadowcat::trash::TrashRetention::Disabled)),
/// };
/// let admin = AdminUser(AuthUser { id: uuid::Uuid::new_v4(), username: "admin".into(), role: ServerRole::Admin });
/// let status = get_status(admin, State(state)).await;
/// assert_eq!(status.0.state, PortMapState::Disabled);
/// # }
/// ```
pub async fn get_status(_admin: AdminUser, State(state): State<AppState>) -> Json<PortMapStatus> {
    Json(state.portmap.status())
}

/// `PUT /api/admin/portmap`: stores the toggle and applies it at once.
///
/// # Errors
///
/// 409 when the `port_forward` configuration key fixes the toggle (the body names the key); 500
/// when the toggle cannot be stored.
///
/// # Examples
///
/// ```
/// # #[tokio::main] async fn main() {
/// use axum::extract::State;
/// use axum::Json;
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::{AdminUser, AuthUser};
/// use shadowcat::http::port_forward::{put_status, SetPortForward};
/// use shadowcat::http::AppState;
///
/// let repo = std::sync::Arc::new(shadowcat::data::sqlite::SqliteRepository::connect("sqlite::memory:").await.unwrap());
/// let state = AppState {
///     repo,
///     config: std::sync::Arc::new(shadowcat::config::Config::default()),
///     setup_token: None,
///     initialized: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
///     ws: shadowcat::ws::WsState::new(),
///     upload_rate: std::sync::Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
///     uploads: std::sync::Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
///     auth_throttle: std::sync::Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
///     write_barrier: std::sync::Arc::new(tokio::sync::RwLock::new(())),
///     preview_fetch_locks: std::sync::Arc::new(dashmap::DashMap::new()),
///     file_permits: std::sync::Arc::new(shadowcat::http::assets::permits::StreamedFilePermits::production()),
///     tls: shadowcat::tls::TlsRuntime::disabled(),
///     portmap: shadowcat::portmap::PortMapRuntime::disabled(),
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(std::env::temp_dir(), shadowcat::trash::TrashRetention::Disabled)),
/// };
/// let admin = AdminUser(AuthUser { id: uuid::Uuid::new_v4(), username: "admin".into(), role: ServerRole::Admin });
/// let status = put_status(admin, State(state), Json(SetPortForward { enabled: false })).await.unwrap();
/// assert!(!status.0.enabled);
/// # }
/// ```
pub async fn put_status(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(body): Json<SetPortForward>,
) -> Result<Json<PortMapStatus>, AppError> {
    if state.portmap.locked_by_config() {
        return Err(AppError::Conflict(
            "port_forward is set in the server configuration; change it there".to_owned(),
        ));
    }
    write_stored_toggle(&state.repo, body.enabled)
        .await
        .map_err(|error| {
            tracing::error!(%error, "port forwarding toggle could not be stored");
            AppError::Internal
        })?;
    state.portmap.apply(body.enabled).await;
    Ok(Json(state.portmap.status()))
}
