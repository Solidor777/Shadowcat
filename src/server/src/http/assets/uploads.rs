//! Resumable chunked upload sessions: a GM opens a session, appends chunks
//! at explicit offsets, then completes (or aborts) it. Sessions live in
//! memory only — they survive a dropped connection, not a server restart —
//! and idle ones are swept with their staging file.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::auth::session::AuthUser;
use crate::data::asset::tags::{derive, DeriveInput};
use crate::data::asset::{
    commit_staged_asset, process_staged_blocking, remove_asset_files, Asset, Provenance,
};
use crate::data::engine::ASSET_FOLDER_DOC_TYPE;
use crate::data::repository::Repository;
use crate::http::error::AppError;
use crate::http::{routes::require_gm, AppState};

use super::{detect_image_type, label_content_type, UploadRateLimiter};

/// Fixed chunk size the client must honor; the single-shot route covers
/// anything at or under one chunk.
pub const CHUNK_SIZE: u64 = 8 * 1024 * 1024;
/// A session untouched for this long is swept (file removed, rate slot refunded).
pub const SESSION_IDLE_MS: i64 = 30 * 60 * 1000;
/// Sweep cadence.
const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
/// Longest accepted display name for an upload.
const MAX_NAME_CHARS: usize = 255;
/// Longest accepted tag, in chars.
pub(crate) const MAX_TAG_CHARS: usize = 64;
/// Most tags one asset carries.
pub(crate) const MAX_TAGS: usize = 64;

/// Why `UploadSession::accept_chunk` refused a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkReject {
    /// Another chunk of this session is being written right now.
    Busy,
    /// `offset` is not the next byte the session expects (`expected`).
    OffsetMismatch {
        /// The offset the session would accept.
        expected: u64,
    },
    /// Appending the chunk would exceed the declared `byte_size`.
    Overflow,
}

/// One in-flight chunked upload.
#[derive(Debug, Clone)]
pub struct UploadSession {
    /// Session id (the `{id}` path segment).
    pub id: Uuid,
    /// World the asset will belong to.
    pub world: Uuid,
    /// The GM who opened the session; every later call must be theirs.
    pub user: Uuid,
    /// Display filename to record.
    pub name: String,
    /// The client's declared content type — a label candidate only; the
    /// sniffed leading bytes win at completion.
    pub content_type: String,
    /// Declared total size; `complete` requires exactly this many bytes.
    pub byte_size: u64,
    /// Bytes appended so far — also the only offset the next chunk may carry.
    pub received: u64,
    /// Destination folder (validated at create).
    pub folder_id: Option<Uuid>,
    /// Explicit tags to record at completion (validated at create).
    pub tags: Vec<String>,
    /// The staging file chunks append to.
    pub staged: PathBuf,
    /// `now_ms` the rate slot was taken at (refund key).
    pub rate_hit_ms: i64,
    /// Last create/put time; idle sweep compares against it.
    pub last_touch_ms: i64,
    /// Whether a chunk write is in progress (serializes concurrent PUTs).
    pub busy: bool,
}

impl UploadSession {
    /// Admit a chunk of `len` bytes at `offset`: it must be the next byte,
    /// nothing else may be writing, and it must fit the declared size. On
    /// success the session is marked busy; `finish_chunk` releases it.
    pub fn accept_chunk(&mut self, offset: u64, len: u64) -> Result<(), ChunkReject> {
        if self.busy {
            return Err(ChunkReject::Busy);
        }
        if offset != self.received {
            return Err(ChunkReject::OffsetMismatch {
                expected: self.received,
            });
        }
        if self.received + len > self.byte_size {
            return Err(ChunkReject::Overflow);
        }
        self.busy = true;
        Ok(())
    }

    /// Record a written chunk (or a failed write with `len == 0`) and release
    /// the busy mark.
    pub fn finish_chunk(&mut self, len: u64, now_ms: i64) {
        self.received += len;
        self.last_touch_ms = now_ms;
        self.busy = false;
    }

    /// Whether the session has gone idle past `SESSION_IDLE_MS` at `now_ms`.
    pub fn is_idle(&self, now_ms: i64) -> bool {
        !self.busy && now_ms - self.last_touch_ms > SESSION_IDLE_MS
    }
}

/// The in-memory session table (one per `AppState`).
pub struct UploadSessions {
    /// Sessions by id.
    inner: Mutex<HashMap<Uuid, UploadSession>>,
}

impl UploadSessions {
    /// An empty table.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Lock the table (a poisoned lock is recovered: the map holds only
    /// plain data, never a half-applied invariant).
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, UploadSession>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Register a fresh session.
    pub fn insert(&self, session: UploadSession) {
        self.lock().insert(session.id, session);
    }

    /// Run `f` against the session `id`, or `None` if there is none.
    pub fn with<R>(&self, id: Uuid, f: impl FnOnce(&mut UploadSession) -> R) -> Option<R> {
        self.lock().get_mut(&id).map(f)
    }

    /// Remove and return the session `id`.
    pub fn remove(&self, id: Uuid) -> Option<UploadSession> {
        self.lock().remove(&id)
    }

    /// Remove and return every session idle at `now_ms` (see
    /// `UploadSession::is_idle`); the caller removes their files and refunds
    /// their rate slots.
    pub fn sweep(&self, now_ms: i64) -> Vec<UploadSession> {
        let mut map = self.lock();
        let expired: Vec<Uuid> = map
            .iter()
            .filter(|(_, s)| s.is_idle(now_ms))
            .map(|(id, _)| *id)
            .collect();
        expired
            .into_iter()
            .filter_map(|id| map.remove(&id))
            .collect()
    }

    /// Number of live sessions.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether no session is live.
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }
}

impl Default for UploadSessions {
    fn default() -> Self {
        Self::new()
    }
}

/// Remove `session`'s staging file and give its rate slot back.
async fn discard(session: &UploadSession, rate: &UploadRateLimiter) {
    remove_asset_files(&session.staged).await;
    rate.refund(session.user, session.rate_hit_ms);
}

/// Spawn the idle-session sweeper: every `SWEEP_INTERVAL`, expired sessions
/// lose their staging file and get their rate slot refunded.
pub fn spawn_sweeper(uploads: Arc<UploadSessions>, rate: Arc<UploadRateLimiter>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            tick.tick().await;
            let now = crate::ws::time::now_millis();
            for session in uploads.sweep(now) {
                discard(&session, &rate).await;
            }
        }
    });
}

/// Validate explicit tags: trimmed, non-empty, at most `MAX_TAG_CHARS` each
/// and `MAX_TAGS` total; duplicates collapse. Shared by every route that
/// accepts GM tags.
pub(crate) fn validate_tags(tags: Vec<String>) -> Result<Vec<String>, AppError> {
    if tags.len() > MAX_TAGS {
        return Err(AppError::Unprocessable(format!("at most {MAX_TAGS} tags")));
    }
    let mut out: Vec<String> = Vec::with_capacity(tags.len());
    for raw in tags {
        let tag = raw.trim();
        if tag.is_empty() {
            return Err(AppError::Unprocessable("empty tag".into()));
        }
        if tag.chars().count() > MAX_TAG_CHARS {
            return Err(AppError::Unprocessable(format!(
                "tag longer than {MAX_TAG_CHARS} chars"
            )));
        }
        if !out.iter().any(|t| t == tag) {
            out.push(tag.to_string());
        }
    }
    Ok(out)
}

/// `folder_id` must name an `asset_folder` document of `world`, else 422.
pub(crate) async fn validate_folder(
    state: &AppState,
    world: Uuid,
    folder_id: Option<Uuid>,
) -> Result<Option<Uuid>, AppError> {
    let Some(fid) = folder_id else {
        return Ok(None);
    };
    let ok = state.repo.get_document(fid).await?.is_some_and(|d| {
        d.doc_type == ASSET_FOLDER_DOC_TYPE && crate::data::document::world_of(&d) == Some(world)
    });
    if !ok {
        return Err(AppError::Unprocessable(
            "folder_id must name an asset_folder in this world".into(),
        ));
    }
    Ok(Some(fid))
}

/// `POST /api/worlds/{world}/assets/uploads` body.
#[derive(Debug, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct CreateUploadRequest {
    /// Display filename.
    pub name: String,
    /// Declared MIME type (label only; the bytes decide for images).
    pub content_type: String,
    /// Total size the client will send.
    pub byte_size: u64,
    /// Destination folder; omitted/null = world root.
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    /// Explicit tags to record.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// `POST /api/worlds/{world}/assets/uploads` response.
#[derive(Debug, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct CreateUploadResponse {
    /// The session to `PUT` chunks into.
    pub upload_id: Uuid,
    /// Every chunk but the last must be exactly this many bytes.
    pub chunk_size: u64,
}

/// `POST /api/worlds/{world}/assets/uploads` — GM-gated (`require_gm`).
/// Takes the rate slot and checks the declared size against the GM cap up
/// front, so a session that could never complete is refused before any
/// bytes flow.
pub async fn create_session(
    State(state): State<AppState>,
    user: AuthUser,
    Path(world): Path<Uuid>,
    Json(body): Json<CreateUploadRequest>,
) -> Result<(StatusCode, Json<CreateUploadResponse>), AppError> {
    let ctx = require_gm(&state, &user, world).await?;
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(AppError::Unprocessable("name must be 1..=255 chars".into()));
    }
    if body.byte_size == 0 {
        return Err(AppError::Unprocessable("byte_size must be > 0".into()));
    }
    let max = state.config.effective_max_bytes(ctx.world_role);
    if body.byte_size > max {
        return Err(AppError::PayloadTooLarge(format!(
            "file exceeds {max} bytes"
        )));
    }
    let tags = validate_tags(body.tags)?;
    let folder_id = validate_folder(&state, world, body.folder_id).await?;

    let now = crate::ws::time::now_millis();
    if !state.upload_rate.check(
        user.id,
        now,
        state.config.effective_rate_per_min(ctx.world_role),
    ) {
        return Err(AppError::TooManyRequests(
            "upload rate limit exceeded".into(),
        ));
    }
    let id = Uuid::new_v4();
    let dir = state.config.assets_path().join(world.to_string());
    let staged = dir.join(format!("{id}.{}.tmp", Uuid::new_v4()));
    let created = async {
        tokio::fs::create_dir_all(&dir).await?;
        tokio::fs::File::create(&staged).await?;
        Ok::<(), std::io::Error>(())
    }
    .await;
    if let Err(e) = created {
        state.upload_rate.refund(user.id, now);
        tracing::error!(?e, "upload session staging file create failed");
        return Err(AppError::Internal);
    }
    state.uploads.insert(UploadSession {
        id,
        world,
        user: user.id,
        name: name.to_string(),
        content_type: body.content_type,
        byte_size: body.byte_size,
        received: 0,
        folder_id,
        tags,
        staged,
        rate_hit_ms: now,
        last_touch_ms: now,
        busy: false,
    });
    Ok((
        StatusCode::CREATED,
        Json(CreateUploadResponse {
            upload_id: id,
            chunk_size: CHUNK_SIZE,
        }),
    ))
}

/// Look up session `id` for `user`: 404 when absent, 403 when someone else's.
fn owned_session(state: &AppState, id: Uuid, user: Uuid) -> Result<UploadSession, AppError> {
    let session = state
        .uploads
        .with(id, |s| s.clone())
        .ok_or(AppError::NotFound)?;
    if session.user != user {
        return Err(AppError::Forbidden);
    }
    Ok(session)
}

/// `PUT /api/assets/uploads/{id}/{offset}` — append one chunk. The route's
/// `DefaultBodyLimit` caps the body at `CHUNK_SIZE`. `offset` must equal the
/// bytes received so far (409 otherwise — a retry of a LOST chunk carries
/// exactly that offset; a duplicate of an accepted one does not). A chunk
/// that would overflow the declared size aborts the session (413).
pub async fn put_chunk(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, offset)): Path<(Uuid, u64)>,
    body: axum::body::Bytes,
) -> Result<StatusCode, AppError> {
    owned_session(&state, id, user.id)?;
    let len = body.len() as u64;
    let admitted = state
        .uploads
        .with(id, |s| s.accept_chunk(offset, len))
        .ok_or(AppError::NotFound)?;
    match admitted {
        Ok(()) => {}
        Err(ChunkReject::Busy) => {
            return Err(AppError::Conflict(
                "a chunk is already being written".into(),
            ));
        }
        Err(ChunkReject::OffsetMismatch { expected }) => {
            return Err(AppError::Conflict(format!(
                "offset {offset} is not the next byte ({expected})"
            )));
        }
        Err(ChunkReject::Overflow) => {
            if let Some(session) = state.uploads.remove(id) {
                discard(&session, &state.upload_rate).await;
            }
            return Err(AppError::PayloadTooLarge(
                "chunk exceeds the declared byte_size".into(),
            ));
        }
    }
    let staged = state
        .uploads
        .with(id, |s| s.staged.clone())
        .ok_or(AppError::NotFound)?;
    let written = async {
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&staged)
            .await?;
        file.write_all(&body).await?;
        file.flush().await
    }
    .await;
    let now = crate::ws::time::now_millis();
    match written {
        Ok(()) => {
            state.uploads.with(id, |s| s.finish_chunk(len, now));
            Ok(StatusCode::NO_CONTENT)
        }
        Err(e) => {
            // A partial append leaves the file inconsistent with `received`;
            // the session cannot be resumed reliably, so it is aborted.
            tracing::error!(?e, %id, "chunk append failed");
            if let Some(session) = state.uploads.remove(id) {
                discard(&session, &state.upload_rate).await;
            }
            Err(AppError::Internal)
        }
    }
}

/// `POST /api/assets/uploads/{id}/complete` — finalize: the session must hold
/// exactly `byte_size` bytes (409 otherwise). Removes the session first so a
/// concurrent complete finds nothing, then sniffs, converts, derives tags and
/// commits through the shared `commit_staged_asset` path.
pub async fn complete_session(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Asset>, AppError> {
    let session = owned_session(&state, id, user.id)?;
    if session.busy {
        return Err(AppError::Conflict("a chunk is still being written".into()));
    }
    if session.received != session.byte_size {
        return Err(AppError::Conflict(format!(
            "{} of {} bytes received",
            session.received, session.byte_size
        )));
    }
    let Some(session) = state.uploads.remove(id) else {
        return Err(AppError::NotFound);
    };

    let outcome: Result<Asset, AppError> = async {
        let head = read_head(&session.staged)
            .await
            .map_err(|_| AppError::Internal)?;
        let content_type =
            label_content_type(detect_image_type(&head), Some(&session.content_type));
        let processed = process_staged_blocking(
            session.staged.clone(),
            content_type,
            session.byte_size as i64,
            state.config.retain_originals,
        )
        .await
        .map_err(|e| {
            tracing::error!(?e, %id, "chunked upload processing failed");
            AppError::Internal
        })?;
        let folder_names = state
            .repo
            .folder_ancestor_names_of(session.folder_id)
            .await?;
        let derived = derive(DeriveInput {
            content_type: &processed.content_type,
            meta: &processed.meta,
            folder_names: &folder_names,
            provenance: Provenance::Uploaded,
        });
        let asset_id = Uuid::new_v4();
        let final_path = state
            .config
            .assets_path()
            .join(session.world.to_string())
            .join(asset_id.to_string());
        let asset = Asset {
            id: asset_id,
            world_id: session.world,
            storage_key: format!("{}/{}", session.world, asset_id),
            original_name: session.name.clone(),
            content_type: processed.content_type,
            byte_size: processed.byte_size,
            created_by: Some(user.id),
            created_at: crate::ws::time::now_millis(),
            version: 1,
            folder_id: session.folder_id,
            tags: session.tags.clone(),
            derived_tags: vec![],
            meta: processed.meta,
        };
        // Same barrier discipline as the single-shot route: read side held
        // only around the move + row commit, never around the chunk stream.
        let _read_permit = state.write_barrier.read().await;
        commit_staged_asset(&state.repo, &session.staged, &final_path, asset, &derived)
            .await
            .map_err(AppError::from)
    }
    .await;

    match outcome {
        Ok(asset) => {
            if let Some(room) = state.ws.rooms.get(asset.world_id) {
                room.broadcast_aux(crate::ws::protocol::ServerMsg::AssetChanged {
                    uuid: asset.id,
                    op: crate::ws::protocol::AssetOp::Created,
                    version: asset.version,
                });
            }
            Ok(Json(asset))
        }
        Err(e) => {
            // The session is gone either way; the slot goes back because no
            // asset was produced. Files are already cleaned by the failing step.
            remove_asset_files(&session.staged).await;
            state.upload_rate.refund(session.user, session.rate_hit_ms);
            Err(e)
        }
    }
}

/// The first 12 bytes of `path` (fewer if the file is shorter).
async fn read_head(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await?;
    let mut buf = vec![0u8; 12];
    let mut filled = 0;
    while filled < buf.len() {
        let n = file.read(&mut buf[filled..]).await?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    buf.truncate(filled);
    Ok(buf)
}

/// `DELETE /api/assets/uploads/{id}` — abort: drop the session, remove the
/// staging file, refund the rate slot.
pub async fn abort_session(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    owned_session(&state, id, user.id)?;
    if let Some(session) = state.uploads.remove(id) {
        discard(&session, &state.upload_rate).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests;
