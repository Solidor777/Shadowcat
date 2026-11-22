//! This module holds the admin routes that create, list, delete, prune and restore the in-app
//! backups under the backups root.
//!
//! Every route takes [`AdminUser`]: a backup holds every user's hidden data and the password
//! hashes, so it never reaches a non-admin. An id from a path reaches the filesystem only through
//! `backup::catalog::resolve_backup`.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use ts_rs::TS;

use crate::auth::session::AdminUser;
use crate::auth::setup::now_millis;
use crate::backup::catalog::{
    list_backups, prune, resolve_backup, restore_pending_path, BackupEntry, BackupRetention,
    RestorePending,
};
use crate::backup::{BackupError, BackupKind, BackupManifest, RestoreFailed, RESTORE_FAILED_FILE};
use crate::http::error::AppError;
use crate::http::AppState;

/// `GET /api/admin/backups` response.
///
/// # Examples
///
/// ```
/// use shadowcat::backup::catalog::BackupRetention;
/// use shadowcat::http::backups::BackupListing;
///
/// let listing = BackupListing {
///     entries: Vec::new(),
///     policy: BackupRetention::default(),
///     root: "backups".into(),
///     restore_failed: None,
/// };
/// assert_eq!(serde_json::to_value(&listing).unwrap()["root"], "backups");
/// ```
#[derive(Debug, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct BackupListing {
    /// Every valid backup, newest first.
    pub entries: Vec<BackupEntry>,
    /// The retention policy the server runs after each backup.
    pub policy: BackupRetention,
    /// The directory the backups live in, so the admin sees where the files are.
    pub root: String,
    /// Why the last staged restore was not applied at startup; `None` when no failure marker
    /// exists (it is removed by the next successful restore).
    pub restore_failed: Option<RestoreFailed>,
}

/// `POST /api/admin/backups/prune` response.
///
/// # Examples
///
/// ```
/// use shadowcat::http::backups::PruneResult;
///
/// let result = PruneResult { removed: vec!["backup-1".into()] };
/// assert_eq!(serde_json::to_value(&result).unwrap()["removed"][0], "backup-1");
/// ```
#[derive(Debug, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct PruneResult {
    /// The ids moved into the trash.
    pub removed: Vec<String>,
}

/// `POST /api/admin/backups/{id}/restore` response.
///
/// # Examples
///
/// ```
/// use shadowcat::http::backups::RestoreStaged;
///
/// let staged = RestoreStaged { staged: true, pre_restore_id: "backup-2".into() };
/// assert_eq!(serde_json::to_value(&staged).unwrap()["pre_restore_id"], "backup-2");
/// ```
#[derive(Debug, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct RestoreStaged {
    /// Always `true`: the restore applies at the next server start.
    pub staged: bool,
    /// The safety backup taken of the current state just before staging.
    pub pre_restore_id: String,
}

/// The current time as unsigned Unix epoch milliseconds, the unit manifests and the retention
/// planner use.
///
/// # Examples
///
/// ```text
/// let now = now_ms(); // -> milliseconds since the epoch
/// ```
fn now_ms() -> u64 {
    u64::try_from(now_millis()).unwrap_or(0)
}

/// The database file and backups root `state.config` resolves.
///
/// # Errors
///
/// 422 when the database is in-memory (nothing to back up); 500 when `config.db` cannot be
/// parsed.
///
/// # Examples
///
/// ```text
/// let (db_path, root) = backup_paths(&state)?;
/// ```
fn backup_paths(state: &AppState) -> Result<(std::path::PathBuf, std::path::PathBuf), AppError> {
    // Resolved through `db::database_file` rather than treated as a bare path: `config.db` may be
    // a `sqlite:`-scheme URL, which a literal `Path::new` would target incorrectly. `Ok(None)`
    // (in-memory) is a caller-facing configuration mistake, distinct from `Err(e)` (a malformed
    // `config.db` string).
    match crate::db::database_file(&state.config.db) {
        Ok(Some(db_path)) => {
            let root = state.config.backups_path_for(&db_path);
            Ok((db_path, root))
        }
        Ok(None) => Err(AppError::Unprocessable(format!(
            "backup requires a file-backed database; {} is in-memory",
            state.config.db
        ))),
        Err(e) => {
            tracing::error!(error = %e, "backup failed to resolve the database file");
            Err(AppError::Internal)
        }
    }
}

/// Maps a backup failure to a response: a bad or unknown id is 404, anything else an opaque 500.
///
/// # Examples
///
/// ```text
/// resolve_backup(&root, &id).map_err(backup_error)?
/// ```
fn backup_error(e: BackupError) -> AppError {
    match e {
        BackupError::InvalidBackupDir(..) => AppError::NotFound,
        other => {
            tracing::error!(error = %other, "backup operation failed");
            AppError::Internal
        }
    }
}

/// Takes one backup of the live server into the backups root and applies the retention policy.
/// The one function the admin route, the pre-restore step and a scheduler call. Holds the write
/// barrier in write mode across the snapshot and the prune, so no asset commit interleaves with
/// the `VACUUM INTO` + assets copy: the backup's database metadata and file bytes are mutually
/// consistent. Database writers need no gating, `VACUUM INTO` is transactionally consistent
/// against a live writer by itself. A prune failure after a successful backup is logged, not
/// returned: the backup exists.
///
/// # Errors
///
/// 422 for an in-memory database, 500 when the snapshot or the path resolution fails.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use shadowcat::backup::BackupKind;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::backups::create_in_app_backup;
/// use shadowcat::http::AppState;
///
/// // `no_run`: writes a real `VACUUM INTO` snapshot and asset copy to disk.
/// let repo = std::sync::Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
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
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(std::env::temp_dir(), shadowcat::trash::TrashRetention::Disabled)),
/// };
/// let (id, manifest) = create_in_app_backup(&state, BackupKind::Manual, None).await.unwrap();
/// assert!(id.starts_with("backup-") && manifest.db_bytes > 0);
/// # }
/// ```
pub async fn create_in_app_backup(
    state: &AppState,
    kind: BackupKind,
    note: Option<String>,
) -> Result<(String, BackupManifest), AppError> {
    let (db_path, root) = backup_paths(state)?;
    let _quiesce = state.write_barrier.write().await;
    let id = format!("backup-{}", now_millis());
    // `create_backup` opens its own single-connection pool against the same SQLite file; a
    // `VACUUM INTO` "database is locked" from racing the live pool surfaces as an ordinary
    // Internal error for the admin to retry, with no busy-loop.
    let manifest = crate::backup::create_backup(
        &db_path,
        &state.config.assets_path(),
        &root.join(&id),
        &state.trash,
        kind,
        note,
    )
    .await
    .map_err(backup_error)?;
    let policy = BackupRetention::from_config(&state.config);
    if let Err(e) = prune(&root, &policy, now_ms(), &state.trash).await {
        tracing::warn!(error = %e, "backup retention prune failed");
    }
    Ok((id, manifest))
}

/// `POST /api/admin/backup`: an in-server whole-server backup (admin only).
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use axum::extract::State;
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::{AdminUser, AuthUser};
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::backups::admin_backup;
/// use shadowcat::http::AppState;
///
/// // `no_run`: writes a real `VACUUM INTO` snapshot and asset copy to disk.
/// let repo = std::sync::Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
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
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(std::env::temp_dir(), shadowcat::trash::TrashRetention::Disabled)),
/// };
/// let admin = AdminUser(AuthUser { id: uuid::Uuid::new_v4(), username: "admin".into(), role: ServerRole::Admin });
/// let manifest = admin_backup(admin, State(state)).await.unwrap();
/// assert!(manifest.0.db_bytes > 0);
/// # }
/// ```
pub async fn admin_backup(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<BackupManifest>, AppError> {
    let (_, manifest) = create_in_app_backup(&state, BackupKind::Manual, None).await?;
    Ok(Json(manifest))
}

/// `GET /api/admin/backups`: every backup under the root, newest first, with the retention
/// policy and the root path.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use axum::extract::State;
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::{AdminUser, AuthUser};
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::backups::list;
/// use shadowcat::http::AppState;
///
/// // `no_run`: reads the backups directory of the default configuration.
/// let repo = std::sync::Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
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
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(std::env::temp_dir(), shadowcat::trash::TrashRetention::Disabled)),
/// };
/// let admin = AdminUser(AuthUser { id: uuid::Uuid::new_v4(), username: "admin".into(), role: ServerRole::Admin });
/// let listing = list(admin, State(state)).await.unwrap();
/// assert!(listing.0.entries.is_empty());
/// # }
/// ```
pub async fn list(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<BackupListing>, AppError> {
    let (db_path, root) = backup_paths(&state)?;
    let listing_root = root.clone();
    let entries = tokio::task::spawn_blocking(move || list_backups(&listing_root))
        .await
        .map_err(|_| AppError::Internal)?;
    // The marker is written by `apply_pending_restore` beside the database file. An absent or
    // unparsable marker reads as no failure: the list must still render.
    let restore_failed = tokio::fs::read(db_path.with_file_name(RESTORE_FAILED_FILE))
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice::<RestoreFailed>(&bytes).ok());
    Ok(Json(BackupListing {
        entries,
        policy: BackupRetention::from_config(&state.config),
        root: root.to_string_lossy().into_owned(),
        restore_failed,
    }))
}

/// `DELETE /api/admin/backups/{id}`: moves one backup into the trash. 404 for an unknown or
/// malformed id.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use axum::extract::{Path, State};
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::{AdminUser, AuthUser};
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::backups::delete_backup;
/// use shadowcat::http::AppState;
///
/// // `no_run`: moves a backup directory into the trash.
/// let repo = std::sync::Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
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
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(std::env::temp_dir(), shadowcat::trash::TrashRetention::Disabled)),
/// };
/// let admin = AdminUser(AuthUser { id: uuid::Uuid::new_v4(), username: "admin".into(), role: ServerRole::Admin });
/// let outcome = delete_backup(admin, State(state), Path("backup-1".to_string())).await;
/// assert!(outcome.is_err());
/// # }
/// ```
pub async fn delete_backup(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let (_, root) = backup_paths(&state)?;
    let dir = resolve_backup(&root, &id).map_err(backup_error)?;
    state
        .trash
        .discard(&dir)
        .await
        .map_err(|e| backup_error(e.into()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/admin/backups/restore-failed`: moves the failed-restore marker beside the database
/// into the trash, so `list` reports `restore_failed: null` afterwards. A missing marker is `Ok`
/// (`Trash::discard` treats an absent path as success), so the route is idempotent. The static
/// path segment takes precedence over the `{id}` route of `delete_backup`.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use axum::extract::State;
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::{AdminUser, AuthUser};
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::backups::dismiss_restore_failed;
/// use shadowcat::http::AppState;
///
/// // `no_run`: moves a marker file into the trash.
/// let repo = std::sync::Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
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
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(std::env::temp_dir(), shadowcat::trash::TrashRetention::Disabled)),
/// };
/// let admin = AdminUser(AuthUser { id: uuid::Uuid::new_v4(), username: "admin".into(), role: ServerRole::Admin });
/// let outcome = dismiss_restore_failed(admin, State(state)).await;
/// assert!(outcome.is_err());
/// # }
/// ```
pub async fn dismiss_restore_failed(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<StatusCode, AppError> {
    let (db_path, _) = backup_paths(&state)?;
    state
        .trash
        .discard(&db_path.with_file_name(RESTORE_FAILED_FILE))
        .await
        .map_err(|e| backup_error(e.into()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/admin/backups/prune`: applies the configured retention policy now.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use axum::extract::State;
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::{AdminUser, AuthUser};
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::backups::prune_now;
/// use shadowcat::http::AppState;
///
/// // `no_run`: moves pruned backups into the trash.
/// let repo = std::sync::Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
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
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(std::env::temp_dir(), shadowcat::trash::TrashRetention::Disabled)),
/// };
/// let admin = AdminUser(AuthUser { id: uuid::Uuid::new_v4(), username: "admin".into(), role: ServerRole::Admin });
/// let result = prune_now(admin, State(state)).await.unwrap();
/// assert!(result.0.removed.is_empty());
/// # }
/// ```
pub async fn prune_now(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<PruneResult>, AppError> {
    let (_, root) = backup_paths(&state)?;
    let policy = BackupRetention::from_config(&state.config);
    // The write side keeps a prune from racing the prune a concurrent backup runs.
    let _quiesce = state.write_barrier.write().await;
    let removed = prune(&root, &policy, now_ms(), &state.trash)
        .await
        .map_err(backup_error)?;
    Ok(Json(PruneResult { removed }))
}

/// `POST /api/admin/backups/{id}/restore`: stages a restore and stops the server. Takes a
/// `pre-restore` backup of the current state first (so a bad restore is itself undoable from the
/// list), writes `restore-pending.json` beside the database, then calls
/// `WsState::trigger_shutdown`, which `http::serve_with` turns into a graceful stop. The restore
/// applies at the next start, before the database opens; the server does not restart itself.
/// 404 for an unknown or malformed id, and nothing is written for it.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use axum::extract::{Path, State};
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::{AdminUser, AuthUser};
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::backups::restore_from_backup;
/// use shadowcat::http::AppState;
///
/// // `no_run`: writes a safety backup and the restore marker, then stops the server.
/// let repo = std::sync::Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
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
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(std::env::temp_dir(), shadowcat::trash::TrashRetention::Disabled)),
/// };
/// let admin = AdminUser(AuthUser { id: uuid::Uuid::new_v4(), username: "admin".into(), role: ServerRole::Admin });
/// let outcome = restore_from_backup(admin, State(state), Path("backup-1".to_string())).await;
/// assert!(outcome.is_err());
/// # }
/// ```
pub async fn restore_from_backup(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RestoreStaged>, AppError> {
    let (db_path, root) = backup_paths(&state)?;
    // Validated before the safety backup, so an unknown id changes nothing.
    resolve_backup(&root, &id).map_err(backup_error)?;
    let (pre_restore_id, _) = create_in_app_backup(
        &state,
        BackupKind::PreRestore,
        Some(format!("before restoring {id}")),
    )
    .await?;
    let marker = RestorePending {
        id,
        requested_at_ms: now_ms(),
    };
    let bytes = serde_json::to_vec(&marker).map_err(|_| AppError::Internal)?;
    let marker_path = restore_pending_path(&db_path);
    // Written beside the final name and renamed in, so startup never reads a half-written marker.
    let staging = marker_path.with_extension("json.tmp");
    tokio::fs::write(&staging, bytes)
        .await
        .map_err(|e| backup_error(e.into()))?;
    tokio::fs::rename(&staging, &marker_path)
        .await
        .map_err(|e| backup_error(e.into()))?;
    state.ws.trigger_shutdown();
    Ok(Json(RestoreStaged {
        staged: true,
        pre_restore_id,
    }))
}
