//! Admin-only trash routes: list, empty, and permanently remove one entry.
//!
//! The trash itself (`crate::trash::Trash`) holds the filesystem logic; these handlers only gate
//! on `AdminUser`, validate the entry id at the HTTP boundary through `EntryId`, run the
//! synchronous `std::fs` work on the blocking pool, and shape the response. Nothing here is
//! reachable by a GM or a player: the trash holds raw server files, not documents, so no
//! per-recipient redaction question arises.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::session::AdminUser;
use crate::http::error::AppError;
use crate::http::AppState;
use crate::trash::{EntryId, TrashEntry, TrashedKind};

/// Default and maximum page size of `GET /api/admin/trash`. A `limit` above it is clamped, never
/// rejected: a long retention window (or `never`) lets the trash grow without bound, so the
/// listing is always bounded.
pub const TRASH_LIST_PAGE_SIZE: usize = 200;

/// Query string of `GET /api/admin/trash`.
///
/// # Examples
///
/// ```
/// use shadowcat::http::trash::TrashListQuery;
///
/// let q: TrashListQuery = serde_json::from_str(r#"{"limit":10}"#).unwrap();
/// assert_eq!(q.limit, Some(10));
/// assert!(q.before.is_none());
/// ```
#[derive(Debug, Deserialize)]
pub struct TrashListQuery {
    /// Page size, clamped to `1..=TRASH_LIST_PAGE_SIZE`; absent means the maximum.
    pub limit: Option<usize>,
    /// Cursor: the id of the last entry of the previous page. The page holds the entries listed
    /// after it, newest first. A well-formed id that no longer exists still works as a cursor.
    pub before: Option<String>,
}

/// One trash entry as the admin listing reports it.
///
/// # Examples
///
/// ```
/// use shadowcat::http::trash::TrashEntryView;
/// use shadowcat::trash::TrashedKind;
///
/// let view = TrashEntryView {
///     id: "1-00000000-0000-0000-0000-000000000000".into(),
///     original_path: "assets/a.png".into(),
///     kind: TrashedKind::File,
///     trashed_at_ms: 1,
///     size_bytes: 3,
/// };
/// assert_eq!(serde_json::to_value(&view).unwrap()["kind"], "file");
/// ```
#[derive(Debug, Serialize)]
pub struct TrashEntryView {
    /// The entry id, `<trashed_at_ms>-<uuid>`; the path segment of the delete route.
    pub id: String,
    /// Absolute path the item had when it was discarded.
    pub original_path: String,
    /// Whether the item is a file or a directory tree.
    pub kind: TrashedKind,
    /// Unix milliseconds at which the item was discarded.
    pub trashed_at_ms: u64,
    /// Bytes the item occupies.
    pub size_bytes: u64,
}

impl From<TrashEntry> for TrashEntryView {
    fn from(entry: TrashEntry) -> Self {
        Self {
            id: entry.id,
            original_path: entry.manifest.original_path.to_string_lossy().into_owned(),
            kind: entry.manifest.kind,
            trashed_at_ms: entry.manifest.trashed_at_ms,
            size_bytes: entry.size_bytes,
        }
    }
}

/// `POST /api/admin/trash/empty` response: what was permanently removed.
///
/// # Examples
///
/// ```
/// use shadowcat::http::trash::EmptyTrashResult;
///
/// let r = EmptyTrashResult { entries: 2, bytes: 10 };
/// assert_eq!(serde_json::to_value(&r).unwrap()["entries"], 2);
/// ```
#[derive(Debug, Serialize)]
pub struct EmptyTrashResult {
    /// Number of entries removed.
    pub entries: u64,
    /// Bytes those entries held.
    pub bytes: u64,
}

/// The sort key `Trash::list` orders by, newest first: the manifest time, then the id.
///
/// # Examples
///
/// ```
/// use shadowcat::http::trash::cursor_key;
///
/// assert_eq!(cursor_key("1737490000123-3fa85f64-5717-4562-b3fc-2c963f66afa6"), Some(1737490000123));
/// assert_eq!(cursor_key("nope"), None);
/// ```
pub fn cursor_key(id: &str) -> Option<u64> {
    id.split_once('-')?.0.parse().ok()
}

/// `GET /api/admin/trash` — admin only. Entries newest first, at most `TRASH_LIST_PAGE_SIZE` per
/// page; pass the last returned id as `before` for the next page. A malformed `before` is a 400.
///
/// # Examples
///
/// ```
/// # #[tokio::main] async fn main() {
/// use axum::extract::{Query, State};
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::{AdminUser, AuthUser};
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::trash::{list_trash, TrashListQuery};
/// use shadowcat::http::AppState;
///
/// let repo = std::sync::Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
/// let dir = tempfile::tempdir().unwrap();
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
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(dir.path().join("trash"), shadowcat::trash::TrashRetention::default())),
/// };
/// let admin = AdminUser(AuthUser { id: uuid::Uuid::new_v4(), username: "admin".into(), role: ServerRole::Admin });
/// let page = list_trash(admin, State(state), Query(TrashListQuery { limit: None, before: None }))
///     .await
///     .unwrap();
/// assert!(page.0.is_empty());
/// # }
/// ```
pub async fn list_trash(
    _admin: AdminUser,
    State(state): State<AppState>,
    Query(query): Query<TrashListQuery>,
) -> Result<Json<Vec<TrashEntryView>>, AppError> {
    let cursor = match query.before.as_deref() {
        None => None,
        Some(raw) => {
            let id: EntryId = raw
                .parse()
                .map_err(|_| AppError::BadRequest("malformed trash cursor".into()))?;
            let at = cursor_key(id.as_str())
                .ok_or_else(|| AppError::BadRequest("malformed trash cursor".into()))?;
            Some((at, id.as_str().to_string()))
        }
    };
    let limit = query
        .limit
        .unwrap_or(TRASH_LIST_PAGE_SIZE)
        .clamp(1, TRASH_LIST_PAGE_SIZE);
    let trash = state.trash.clone();
    let entries = tokio::task::spawn_blocking(move || trash.list())
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "trash listing task failed");
            AppError::Internal
        })?
        .map_err(|e| {
            tracing::error!(error = %e, "trash listing failed");
            AppError::Internal
        })?;
    let page = entries
        .into_iter()
        .filter(|e| match &cursor {
            Some((at, id)) => (e.manifest.trashed_at_ms, e.id.as_str()) < (*at, id.as_str()),
            None => true,
        })
        .take(limit)
        .map(TrashEntryView::from)
        .collect();
    Ok(Json(page))
}

/// `POST /api/admin/trash/empty` — admin only. Permanently removes every entry and reports the
/// count and bytes removed.
///
/// # Examples
///
/// ```
/// # #[tokio::main] async fn main() {
/// use axum::extract::State;
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::{AdminUser, AuthUser};
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::trash::empty_trash;
/// use shadowcat::http::AppState;
///
/// let repo = std::sync::Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
/// let dir = tempfile::tempdir().unwrap();
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
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(dir.path().join("trash"), shadowcat::trash::TrashRetention::default())),
/// };
/// let admin = AdminUser(AuthUser { id: uuid::Uuid::new_v4(), username: "admin".into(), role: ServerRole::Admin });
/// let result = empty_trash(admin, State(state)).await.unwrap();
/// assert_eq!(result.0.entries, 0);
/// # }
/// ```
pub async fn empty_trash(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<EmptyTrashResult>, AppError> {
    let trash = state.trash.clone();
    let (entries, bytes) = tokio::task::spawn_blocking(move || trash.empty())
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "trash empty task failed");
            AppError::Internal
        })?
        .map_err(|e| {
            tracing::error!(error = %e, "emptying the trash failed");
            AppError::Internal
        })?;
    Ok(Json(EmptyTrashResult { entries, bytes }))
}

/// `DELETE /api/admin/trash/{entry_id}` — admin only. Permanently removes exactly that entry:
/// 204 on success, 404 when no such entry exists (already removed), 400 when the id is not of the
/// `<timestamp>-<uuid>` shape.
///
/// # Examples
///
/// ```
/// # #[tokio::main] async fn main() {
/// use axum::extract::{Path, State};
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::{AdminUser, AuthUser};
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::trash::delete_trash_entry;
/// use shadowcat::http::AppState;
///
/// let repo = std::sync::Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
/// let dir = tempfile::tempdir().unwrap();
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
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(dir.path().join("trash"), shadowcat::trash::TrashRetention::default())),
/// };
/// let admin = AdminUser(AuthUser { id: uuid::Uuid::new_v4(), username: "admin".into(), role: ServerRole::Admin });
/// let missing = "1-00000000-0000-0000-0000-000000000000".to_string();
/// assert!(delete_trash_entry(admin, State(state), Path(missing)).await.is_err());
/// # }
/// ```
pub async fn delete_trash_entry(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> Result<StatusCode, AppError> {
    let id: EntryId = raw
        .parse()
        .map_err(|_| AppError::BadRequest("malformed trash entry id".into()))?;
    let trash = state.trash.clone();
    let removed = tokio::task::spawn_blocking(move || trash.empty_one(&id))
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "trash delete task failed");
            AppError::Internal
        })?
        .map_err(|e| {
            tracing::error!(error = %e, "removing a trash entry failed");
            AppError::Internal
        })?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound)
    }
}
