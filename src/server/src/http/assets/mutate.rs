//! GM-only asset mutation routes beyond upload/replace/delete: downloading
//! the retained original and re-running the conversion from it.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Deserializer};
use ts_rs::TS;
use uuid::Uuid;

use crate::auth::session::AuthUser;
use crate::data::asset::process::original_path;
use crate::data::asset::{process_staged_blocking, Asset};
use crate::data::command::{Command, Operation};
use crate::data::engine::ASSET_FOLDER_DOC_TYPE;
use crate::data::repository::Repository;
use crate::http::error::AppError;
use crate::http::{routes::require_gm, routes::write_ops, AppState};
use crate::ws::protocol::{AssetOp, ServerMsg};

use super::uploads::{validate_folder, validate_tags};
use super::{commit_replacement, delete_asset_files_and_row, detect_audio_type};

/// Tri-state deserializer: a missing key is `None` (leave unchanged), an
/// explicit `null` is `Some(None)` (set to root), a value is `Some(Some(v))`.
fn double_option<'de, D>(de: D) -> Result<Option<Option<Uuid>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<Uuid>::deserialize(de).map(Some)
}

/// Load `id` and require the caller to be GM of its world (404 for an
/// unknown id, 403 for a non-GM).
async fn gm_asset(state: &AppState, user: &AuthUser, id: Uuid) -> Result<Asset, AppError> {
    let asset = state.repo.get_asset(id).await?.ok_or(AppError::NotFound)?;
    require_gm(state, user, asset.world_id).await?;
    Ok(asset)
}

/// `GET /api/assets/{uuid}/original` — GM-gated download of the retained
/// original bytes (`<uuid>.orig`), served under the arrived content type as
/// an attachment named after the upload. 404 when the original was not
/// retained (pass-through upload, or `retain_originals = false`).
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::AuthUser;
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::AppState;
/// use std::sync::{atomic::AtomicBool, Arc};
/// use uuid::Uuid;
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
/// };
/// let user = AuthUser { id: Uuid::new_v4(), username: "gm-example".into(), role: ServerRole::User };
/// let _ = shadowcat::http::assets::mutate::original(
///     axum::extract::State(state),
///     user,
///     axum::extract::Path(Uuid::new_v4()),
/// )
/// .await;
/// # }
/// ```
pub async fn original(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Response, AppError> {
    let asset = gm_asset(&state, &user, id).await?;
    if !asset.meta.original_retained {
        return Err(AppError::NotFound);
    }
    let canonical = state.config.assets_path().join(&asset.storage_key);
    let bytes = tokio::fs::read(original_path(&canonical))
        .await
        .map_err(|e| {
            tracing::error!(?e, %id, "retained original missing for existing record");
            AppError::Internal
        })?;
    Ok((
        [
            (header::CONTENT_TYPE, asset.meta.original_content_type),
            (
                header::CONTENT_DISPOSITION,
                attachment_disposition(&asset.original_name),
            ),
        ],
        Body::from(bytes),
    )
        .into_response())
}

/// `Content-Disposition: attachment; filename="<name>"`. The display name is
/// quoted; a quote, backslash or line break inside it would break the header
/// grammar (or inject a second header), so those are stripped, not escaped.
fn attachment_disposition(original_name: &str) -> String {
    let filename: String = original_name
        .chars()
        .filter(|c| !matches!(c, '"' | '\\' | '\r' | '\n'))
        .collect();
    format!("attachment; filename=\"{filename}\"")
}

/// `POST /api/assets/{uuid}/reconvert` — GM-gated: re-runs the conversion
/// pipeline on a copy of the retained original (404 when not retained) and
/// commits the result exactly like a replace (row-first, sibling swap under
/// the barrier, derived-tag refresh, `Replaced` broadcast). Whether the
/// original survives follows the CURRENT `retain_originals` setting.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::AuthUser;
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::AppState;
/// use std::sync::{atomic::AtomicBool, Arc};
/// use uuid::Uuid;
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
/// };
/// let user = AuthUser { id: Uuid::new_v4(), username: "gm-example".into(), role: ServerRole::User };
/// let _ = shadowcat::http::assets::mutate::reconvert(
///     axum::extract::State(state),
///     user,
///     axum::extract::Path(Uuid::new_v4()),
/// )
/// .await;
/// # }
/// ```
pub async fn reconvert(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Asset>, AppError> {
    let existing = gm_asset(&state, &user, id).await?;
    let final_path = state.config.assets_path().join(&existing.storage_key);
    let tmp_path = final_path.with_file_name(format!("{id}.{}.tmp", Uuid::new_v4()));

    if existing.content_type.starts_with("audio/") {
        // Audio's `original_retained` precondition holds by construction: the canonical is
        // NEVER converted (only sibling derivatives are emitted), so the "original" a retry
        // re-processes is the canonical file itself. This is audio's only recovery from an
        // over-cap/failed transcode; a stale derivative of a previous success is removed by
        // `commit_replacement`'s sibling swap when the retry emits none.
        let copied = tokio::fs::copy(&final_path, &tmp_path).await;
        if let Err(e) = copied {
            tracing::error!(?e, %id, "audio canonical missing for existing record");
            return Err(AppError::Internal);
        }
        let containers = crate::data::asset::process::audio::effective_reencode_selection(
            crate::data::asset::process::audio::has_sibling(
                &final_path,
                crate::data::asset::process::audio::OPUS_SUFFIX,
            ),
            crate::data::asset::process::audio::has_sibling(
                &final_path,
                crate::data::asset::process::audio::WEBM_SUFFIX,
            ),
        );
        let processed = process_staged_blocking(
            tmp_path.clone(),
            existing.content_type.clone(),
            existing.meta.original_byte_size,
            state.config.retain_originals,
            containers,
        )
        .await
        .map_err(|e| {
            tracing::error!(?e, %id, "audio reconvert processing failed");
            AppError::Internal
        })?;
        let asset =
            commit_replacement(&state, &existing, &tmp_path, &final_path, processed).await?;
        return Ok(Json(asset));
    }

    if !existing.meta.original_retained {
        return Err(AppError::NotFound);
    }
    let copied = tokio::fs::copy(original_path(&final_path), &tmp_path).await;
    if let Err(e) = copied {
        tracing::error!(?e, %id, "retained original missing for existing record");
        return Err(AppError::Internal);
    }
    // An upload whose label missed the bytes (e.g. stored `application/octet-stream` on a
    // real WAV before audio sniffing existed) is probed here: a sniffed audio container
    // reclassifies the retry as an audio re-transcode instead of an image reconvert.
    let head = {
        use tokio::io::AsyncReadExt;
        let mut head = vec![0u8; 16];
        let mut f = tokio::fs::File::open(&tmp_path)
            .await
            .map_err(|_| AppError::Internal)?;
        let mut read = 0usize;
        while read < head.len() {
            match f.read(&mut head[read..]).await {
                Ok(0) => break,
                Ok(n) => read += n,
                Err(_) => return Err(AppError::Internal),
            }
        }
        head.truncate(read);
        head
    };
    let audio_sniff = detect_audio_type(&head);
    let processed = process_staged_blocking(
        tmp_path.clone(),
        audio_sniff
            .map(str::to_string)
            .unwrap_or_else(|| existing.meta.original_content_type.clone()),
        existing.meta.original_byte_size,
        state.config.retain_originals,
        Default::default(),
    )
    .await
    .map_err(|e| {
        tracing::error!(?e, %id, "asset reconvert processing failed");
        AppError::Internal
    })?;
    let asset = commit_replacement(&state, &existing, &tmp_path, &final_path, processed).await?;
    Ok(Json(asset))
}

/// `PATCH /api/assets/{uuid}` body. Every field optional; an absent field is
/// left unchanged. `folder_id: null` moves the asset to the world root.
///
/// # Examples
///
/// ```
/// use shadowcat::http::assets::mutate::PatchAssetRequest;
///
/// let body: PatchAssetRequest = serde_json::from_str(r#"{"name":"New Name"}"#).unwrap();
/// assert_eq!(body.name.as_deref(), Some("New Name"));
/// assert_eq!(body.folder_id, None); // absent field stays unchanged
/// ```
#[derive(Debug, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct PatchAssetRequest {
    /// New display name.
    #[serde(default)]
    #[ts(optional)]
    pub name: Option<String>,
    /// New folder (`null` = root); absent = unchanged.
    #[serde(default, deserialize_with = "double_option")]
    #[ts(optional, type = "string | null")]
    pub folder_id: Option<Option<Uuid>>,
    /// Replacement explicit tag set.
    #[serde(default)]
    #[ts(optional)]
    pub tags: Option<Vec<String>>,
}

/// `PATCH /api/assets/{uuid}` — GM-gated rename / move / retag in one
/// transaction; derived tags are recomputed and a `Moved` notice (version
/// unchanged) is broadcast so open listings refresh.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::AuthUser;
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::assets::mutate::PatchAssetRequest;
/// use shadowcat::http::AppState;
/// use std::sync::{atomic::AtomicBool, Arc};
/// use uuid::Uuid;
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
/// };
/// let user = AuthUser { id: Uuid::new_v4(), username: "gm-example".into(), role: ServerRole::User };
/// let body = axum::Json(PatchAssetRequest {
///     name: Some("New Name".into()),
///     folder_id: None,
///     tags: None,
/// });
/// let _ = shadowcat::http::assets::mutate::patch(
///     axum::extract::State(state),
///     user,
///     axum::extract::Path(Uuid::new_v4()),
///     body,
/// )
/// .await;
/// # }
/// ```
pub async fn patch(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchAssetRequest>,
) -> Result<Json<Asset>, AppError> {
    let existing = gm_asset(&state, &user, id).await?;
    let name = match body.name.as_deref().map(str::trim) {
        None => None,
        Some("") => return Err(AppError::Unprocessable("name must not be empty".into())),
        Some(n) if n.chars().count() > 255 => {
            return Err(AppError::Unprocessable(
                "name must be at most 255 chars".into(),
            ));
        }
        Some(n) => Some(n.to_string()),
    };
    let folder = match body.folder_id {
        None => None,
        Some(f) => Some(validate_folder(&state, existing.world_id, f).await?),
    };
    let tags = body.tags.map(validate_tags).transpose()?;
    let updated = state
        .repo
        .update_asset_placement(id, name.as_deref(), folder, tags.as_deref())
        .await?
        .ok_or(AppError::NotFound)?;
    if let Some(room) = state.ws.rooms.get(updated.world_id) {
        room.broadcast_aux(ServerMsg::AssetChanged {
            uuid: id,
            op: AssetOp::Moved,
            version: updated.version,
        });
    }
    Ok(Json(updated))
}

/// `POST /api/worlds/{world}/assets/bulk` body.
///
/// # Examples
///
/// ```
/// use shadowcat::http::assets::mutate::BulkAssetRequest;
///
/// let body: BulkAssetRequest = serde_json::from_str(
///     r#"{"ids":["3fa85f64-5717-4562-b3fc-2c963f66afa6"],"add_tags":["npc"]}"#,
/// )
/// .unwrap();
/// assert_eq!(body.add_tags, vec!["npc".to_string()]);
/// assert!(body.remove_tags.is_empty()); // omitted key defaults to empty
/// ```
#[derive(Debug, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct BulkAssetRequest {
    /// The assets to edit; every one must belong to the world.
    pub ids: Vec<Uuid>,
    /// Move them all here (`null` = root); absent = leave folders alone.
    #[serde(default, deserialize_with = "double_option")]
    #[ts(optional, type = "string | null")]
    pub folder_id: Option<Option<Uuid>>,
    /// Explicit tags to add to every asset.
    #[serde(default)]
    pub add_tags: Vec<String>,
    /// Explicit tags to remove from every asset.
    #[serde(default)]
    pub remove_tags: Vec<String>,
}

/// `POST /api/worlds/{world}/assets/bulk` — GM-gated multi-select move /
/// tag edit in one transaction (404 if any id is not this world's asset;
/// nothing is applied then). One `Moved` notice per asset.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::AuthUser;
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::assets::mutate::BulkAssetRequest;
/// use shadowcat::http::AppState;
/// use std::sync::{atomic::AtomicBool, Arc};
/// use uuid::Uuid;
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
/// };
/// let user = AuthUser { id: Uuid::new_v4(), username: "gm-example".into(), role: ServerRole::User };
/// let body = axum::Json(BulkAssetRequest {
///     ids: vec![Uuid::new_v4()],
///     folder_id: None,
///     add_tags: vec!["npc".into()],
///     remove_tags: vec![],
/// });
/// let _ = shadowcat::http::assets::mutate::bulk(
///     axum::extract::State(state),
///     user,
///     axum::extract::Path(Uuid::new_v4()),
///     body,
/// )
/// .await;
/// # }
/// ```
pub async fn bulk(
    State(state): State<AppState>,
    user: AuthUser,
    Path(world): Path<Uuid>,
    Json(body): Json<BulkAssetRequest>,
) -> Result<Json<Vec<Asset>>, AppError> {
    require_gm(&state, &user, world).await?;
    if body.ids.is_empty() {
        return Err(AppError::Unprocessable("ids must not be empty".into()));
    }
    let folder = match body.folder_id {
        None => None,
        Some(f) => Some(validate_folder(&state, world, f).await?),
    };
    let add = validate_tags(body.add_tags)?;
    let remove = validate_tags(body.remove_tags)?;
    let updated = state
        .repo
        .bulk_update_assets(world, &body.ids, folder, &add, &remove)
        .await?;
    if let Some(room) = state.ws.rooms.get(world) {
        for a in &updated {
            room.broadcast_aux(ServerMsg::AssetChanged {
                uuid: a.id,
                op: AssetOp::Moved,
                version: a.version,
            });
        }
    }
    Ok(Json(updated))
}

/// `?assets=` on `DELETE /api/asset-folders/{id}`.
///
/// # Examples
///
/// ```
/// use shadowcat::http::assets::mutate::FolderDeleteQuery;
///
/// let q = FolderDeleteQuery { assets: Some("delete".into()) };
/// assert_eq!(q.assets.as_deref(), Some("delete"));
/// ```
#[derive(Debug, Deserialize)]
pub struct FolderDeleteQuery {
    /// `reparent` (default): contained assets move to the folder's parent;
    /// `delete`: every asset in the subtree is deleted first.
    pub assets: Option<String>,
}

/// `DELETE /api/asset-folders/{id}[?assets=reparent|delete]` — GM-gated
/// folder delete. The folder document goes through the same `write_ops`
/// path `routes::delete_document` uses (so the sub-folder cascade and the
/// `delete_document_tx` reparent hook apply); with `assets=delete` every
/// asset in the subtree is first removed through the shared asset-delete
/// tail (files + `Deleted` broadcasts). A failure mid-purge leaves the
/// already-deleted assets gone and the folder intact — the client re-issues.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::auth::session::AuthUser;
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::assets::mutate::FolderDeleteQuery;
/// use shadowcat::http::AppState;
/// use std::sync::{atomic::AtomicBool, Arc};
/// use uuid::Uuid;
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
/// };
/// let user = AuthUser { id: Uuid::new_v4(), username: "gm-example".into(), role: ServerRole::User };
/// let _ = shadowcat::http::assets::mutate::delete_folder(
///     axum::extract::State(state),
///     user,
///     axum::extract::Path(Uuid::new_v4()),
///     axum::extract::Query(FolderDeleteQuery { assets: Some("reparent".into()) }),
/// )
/// .await;
/// # }
/// ```
pub async fn delete_folder(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Query(q): Query<FolderDeleteQuery>,
) -> Result<Json<Command>, AppError> {
    let purge = match q.assets.as_deref() {
        None | Some("reparent") => false,
        Some("delete") => true,
        Some(other) => {
            return Err(AppError::BadRequest(format!(
                "unknown assets mode '{other}'"
            )));
        }
    };
    let doc = state
        .repo
        .get_document(id)
        .await?
        .filter(|d| d.doc_type == ASSET_FOLDER_DOC_TYPE)
        .ok_or(AppError::NotFound)?;
    let world = crate::data::document::world_of(&doc).ok_or(AppError::NotFound)?;
    // by-id route: a non-GM is 404, not 403 (existence hiding, as the
    // document routes do).
    require_gm(&state, &user, world)
        .await
        .map_err(|_| AppError::NotFound)?;
    if purge {
        for asset_id in state.repo.assets_in_folder_subtree_of(id).await? {
            delete_asset_files_and_row(&state, asset_id).await?;
        }
    }
    write_ops(&state, &user, world, vec![Operation::Delete { doc }]).await
}

#[cfg(test)]
mod tests;
