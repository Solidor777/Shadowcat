#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod mutate;
pub mod query;
pub mod uploads;

use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// Detect a supported image content-type from leading bytes, else `None`.
/// The bytes are the validation boundary — the client-declared content-type is
/// never trusted. Needs ≥12 bytes to rule on WebP. Source: file-format magic
/// numbers (PNG/JFIF/GIF/RIFF specs).
pub fn detect_image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// Per-user sliding-window upload limiter (trailing 60s). In-memory; resets on
/// restart, which is acceptable for an abuse backstop.
pub struct UploadRateLimiter {
    /// Per-user hit timestamps within the trailing window.
    hits: Mutex<HashMap<Uuid, Vec<i64>>>,
}

impl UploadRateLimiter {
    /// An empty limiter (one per `AppState`).
    ///
    /// # Examples
    ///
    /// ```text
    /// state.upload_rate.check(user, now_ms, per_min) // role-tiered per_min from Config
    /// ```
    pub fn new() -> Self {
        Self {
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// Record an upload at `now_ms` and report whether it is within `per_min`.
    /// Prunes entries older than the 60s window first.
    pub fn check(&self, user: Uuid, now_ms: i64, per_min: u32) -> bool {
        let mut map = self.hits.lock().expect("rate-limiter mutex poisoned");
        let v = map.entry(user).or_default();
        let cutoff = now_ms - 60_000;
        v.retain(|&t| t > cutoff);
        if v.len() as u32 >= per_min {
            return false;
        }
        v.push(now_ms);
        true
    }

    /// Return a hit recorded by `check` (matched by `now_ms`) to the user's
    /// budget — called when the gated upload subsequently fails, so a rejected
    /// request (bad bytes, over-cap, I/O error) does not consume quota. The
    /// `check`-before-stream order still bounds in-flight concurrency.
    pub fn refund(&self, user: Uuid, now_ms: i64) {
        let mut map = self.hits.lock().expect("rate-limiter mutex poisoned");
        if let Some(v) = map.get_mut(&user) {
            if let Some(pos) = v.iter().rposition(|&t| t == now_ms) {
                v.remove(pos);
            }
        }
    }
}

impl Default for UploadRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

use crate::auth::session::AuthUser;
use crate::data::asset::process::{derivative_path, sibling_paths, write_derivatives, Variant};
use crate::data::asset::tags::{derive, DeriveInput};
use crate::data::asset::{
    commit_staged_asset, move_asset_files, process_staged_blocking, remove_asset_files, Asset,
    Provenance,
};
use crate::http::error::AppError;
use crate::http::{routes::require_gm, AppState};
use crate::ws::protocol::{AssetOp, ServerMsg};
use axum::body::Body;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use tokio::io::AsyncWriteExt;

/// Stream a multipart "file" field to `dest`, enforcing `max_bytes` as bytes
/// arrive (never buffering the whole body). Returns
/// `(content_type, byte_size, original_name)`, where `content_type` is the
/// type SNIFFED from the leading bytes when they are a supported image; when
/// they are not, the client's declared type is used as a plain label —
/// unless it CLAIMS `image/*`, which the bytes just disproved, in which case
/// the label is `application/octet-stream`. The bytes are the validation
/// boundary; a client's image claim is never trusted. On any failure the
/// partial file is removed.
async fn store_streamed(
    mut multipart: Multipart,
    dest: &std::path::Path,
    max_bytes: u64,
) -> Result<(String, i64, String), AppError> {
    let field = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
        .ok_or_else(|| AppError::BadRequest("missing file field".into()))?;
    let original_name = field.file_name().unwrap_or("upload").to_string();
    let declared = field.content_type().map(str::to_string);

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|_| AppError::Internal)?;
    }
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|_| AppError::Internal)?;
    // Leading bytes for the sniff; `detect_image_type` needs at most 12.
    let mut head: Vec<u8> = Vec::with_capacity(16);
    let mut total: u64 = 0;

    let mut field = field;
    loop {
        let chunk = match field.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                let _ = tokio::fs::remove_file(dest).await;
                return Err(AppError::BadRequest(format!("multipart error: {e}")));
            }
        };
        total += chunk.len() as u64;
        if total > max_bytes {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(AppError::PayloadTooLarge(format!(
                "file exceeds {max_bytes} bytes"
            )));
        }
        if head.len() < 12 {
            let want = 12 - head.len();
            head.extend_from_slice(&chunk[..chunk.len().min(want)]);
        }
        if file.write_all(&chunk).await.is_err() {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(AppError::Internal);
        }
    }
    file.flush().await.map_err(|_| AppError::Internal)?;

    let content_type = label_content_type(detect_image_type(&head), declared.as_deref());
    Ok((content_type, total as i64, original_name))
}

/// The content type recorded for an upload: the sniffed image type when the
/// bytes are a supported image; otherwise the declared type as a label,
/// except that a declared `image/*` the bytes disproved becomes
/// `application/octet-stream`.
pub(super) fn label_content_type(sniffed: Option<&'static str>, declared: Option<&str>) -> String {
    if let Some(ct) = sniffed {
        return ct.to_string();
    }
    match declared {
        Some(d) if !d.starts_with("image/") && !d.is_empty() => d.to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

/// `POST /api/worlds/{world}/assets` — GM-gated single-shot multipart upload
/// (`require_gm`; server admins resolve to GM). There is no owner exception.
/// Images are converted through `data::asset::process`; anything else is
/// stored pass-through under its declared type. Lands in the world root.
pub async fn upload(
    State(state): State<AppState>,
    user: AuthUser,
    Path(world): Path<uuid::Uuid>,
    multipart: Multipart,
) -> Result<Json<Asset>, AppError> {
    let ctx = require_gm(&state, &user, world).await?;
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
    let id = uuid::Uuid::new_v4();
    let storage_key = format!("{world}/{id}");
    let final_path = state
        .config
        .assets_path()
        .join(world.to_string())
        .join(id.to_string());
    // Unique temp sibling in the same directory: stream network bytes to disk
    // BEFORE acquiring the backup quiesce barrier below. These routes disable
    // `DefaultBodyLimit`, so a slow multipart upload has no timeout — holding
    // a write-preferring `tokio::sync::RwLock`'s read side across that wait
    // would queue an admin `write()` (i.e. `/api/admin/backup`) behind it.
    let tmp_path = final_path.with_file_name(format!("{id}.{}.tmp", uuid::Uuid::new_v4()));
    let max = state.config.effective_max_bytes(ctx.world_role);

    // Do the fallible work in one block so a failure at any step refunds the
    // rate-limit hit `check` recorded — a rejected upload must not burn quota.
    let retain = state.config.retain_originals;
    let outcome: Result<Asset, AppError> = async {
        let (arrived_type, arrived_size, original_name) =
            store_streamed(multipart, &tmp_path, max).await?;
        // CPU-bound conversion, off the async runtime and BEFORE the barrier.
        let processed =
            process_staged_blocking(tmp_path.clone(), arrived_type, arrived_size, retain)
                .await
                .map_err(|e| {
                    tracing::error!(?e, %id, "asset processing failed");
                    AppError::Internal
                })?;
        // Single-shot uploads land in the world root: no folder segments.
        let derived = derive(DeriveInput {
            content_type: &processed.content_type,
            meta: &processed.meta,
            folder_names: &[],
            provenance: Provenance::Uploaded,
        });
        let asset = Asset {
            id,
            world_id: world,
            storage_key,
            original_name,
            content_type: processed.content_type,
            byte_size: processed.byte_size,
            created_by: Some(user.id),
            created_at: now,
            version: 1,
            folder_id: None,
            tags: vec![],
            derived_tags: vec![],
            meta: processed.meta,
        };
        // Read-side of the backup quiesce barrier, acquired only around the
        // rename+DB-commit pair below — the one critical section the quiesce
        // exists to keep non-interleaving with an in-server backup's VACUUM +
        // assets copy. Concurrent asset writes share the read side freely;
        // this serializes nothing between uploads.
        let _read_permit = state.write_barrier.read().await;
        commit_staged_asset(&state.repo, &tmp_path, &final_path, asset, &derived)
            .await
            .map_err(AppError::from)
    }
    .await;

    match outcome {
        Ok(asset) => Ok(Json(asset)),
        Err(e) => {
            state.upload_rate.refund(user.id, now);
            Err(e)
        }
    }
}

/// `?variant=` on `GET /api/assets/{uuid}`.
#[derive(Debug, serde::Deserialize)]
pub struct ServeQuery {
    /// `thumb` | `preview`; absent = the canonical file.
    pub variant: Option<String>,
}

/// `GET /api/assets/{uuid}[?variant=thumb|preview]` — read-gated by world
/// membership; ETag-revalidated. A derivative shares the canonical's ETag
/// (`"{id}-{version}"`): it is regenerated whenever the canonical's version
/// changes, so the version keys it. A missing derivative is regenerated on
/// demand; if the canonical does not decode, the canonical itself is served
/// in its place rather than a 404.
pub async fn serve(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<ServeQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let variant = match q.variant.as_deref() {
        None => None,
        Some("thumb") => Some(Variant::Thumb),
        Some("preview") => Some(Variant::Preview),
        Some(other) => {
            return Err(AppError::BadRequest(format!("unknown variant '{other}'")));
        }
    };
    let asset = state.repo.get_asset(id).await?.ok_or(AppError::NotFound)?;
    // Read-gate: any member of the asset's world may read. permission_context
    // returns Forbidden for non-members.
    state
        .repo
        .permission_context(asset.world_id, user.id, user.role)
        .await?;

    let etag = format!("\"{}-{}\"", id, asset.version);
    // `If-None-Match` is an RFC 7232 comma-separated list (browsers may send
    // several cached ETags); 304 if ours appears anywhere in it.
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if if_none_match.split(',').any(|t| t.trim() == etag) {
        return Ok((StatusCode::NOT_MODIFIED).into_response());
    }

    let canonical = state.config.assets_path().join(&asset.storage_key);
    let (path, content_type) = match variant {
        None => (canonical, asset.content_type),
        Some(v) => match ensure_derivative(&canonical, v).await {
            Ok(p) => (
                p,
                crate::data::asset::process::WEBP_CONTENT_TYPE.to_string(),
            ),
            Err(e) => {
                // Not decodable (pass-through non-image, corrupt file): the
                // canonical stands in for its own preview.
                tracing::debug!(?e, %id, "derivative unavailable; serving canonical");
                (canonical, asset.content_type)
            }
        },
    };
    let bytes = tokio::fs::read(&path).await.map_err(|e| {
        tracing::error!(?e, %id, "asset file missing for existing record");
        AppError::Internal
    })?;
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_DISPOSITION, "inline".to_string()),
            (header::ETAG, etag),
        ],
        Body::from(bytes),
    )
        .into_response())
}

/// Path of the `variant` derivative of `canonical`, regenerating both
/// derivatives (blocking pool) when it is missing.
async fn ensure_derivative(
    canonical: &std::path::Path,
    variant: Variant,
) -> std::io::Result<std::path::PathBuf> {
    let path = derivative_path(canonical, variant);
    if tokio::fs::try_exists(&path).await? {
        return Ok(path);
    }
    let src = canonical.to_path_buf();
    tokio::task::spawn_blocking(move || write_derivatives(&src))
        .await
        .map_err(std::io::Error::other)??;
    Ok(path)
}

/// `POST /api/assets/{uuid}/replace` — GM-gated byte-swap behind a stable id
/// (`require_gm`; no owner exception). Undo-exempt: no world seq, no
/// event-log entry.
pub async fn replace(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<uuid::Uuid>,
    multipart: Multipart,
) -> Result<Json<Asset>, AppError> {
    let existing = state.repo.get_asset(id).await?.ok_or(AppError::NotFound)?;
    let ctx = require_gm(&state, &user, existing.world_id).await?;
    // Replace streams a full file like upload, so it shares the per-user tiered
    // rate limit — the cap is on total write volume, not per-endpoint.
    let now = crate::ws::time::now_millis();
    if !state.upload_rate.check(
        user.id,
        now,
        state.config.effective_rate_per_min(ctx.world_role),
    ) {
        return Err(AppError::TooManyRequests(
            "replace rate limit exceeded".into(),
        ));
    }

    // Stream the new bytes to a per-request temp file. A unique name (not a
    // fixed `<uuid>.tmp`) keeps two concurrent replaces of the same asset from
    // clobbering each other's partial writes.
    let final_path = state.config.assets_path().join(&existing.storage_key);
    let tmp_path = final_path.with_file_name(format!("{id}.{}.tmp", uuid::Uuid::new_v4()));
    let max = state.config.effective_max_bytes(ctx.world_role);

    // Fallible work in one block so any failure refunds the rate-limit hit `check`
    // recorded — a rejected replace must not burn quota.
    let retain = state.config.retain_originals;
    let outcome: Result<Asset, AppError> = async {
        let (arrived_type, arrived_size, _name) = store_streamed(multipart, &tmp_path, max).await?;
        let processed =
            process_staged_blocking(tmp_path.clone(), arrived_type, arrived_size, retain)
                .await
                .map_err(|e| {
                    tracing::error!(?e, %id, "asset processing failed");
                    AppError::Internal
                })?;
        commit_replacement(&state, &existing, &tmp_path, &final_path, processed).await
    }
    .await;

    match outcome {
        Ok(asset) => Ok(Json(asset)),
        Err(e) => {
            state.upload_rate.refund(user.id, now);
            Err(e)
        }
    }
}

/// The shared tail of every byte-swap behind a stable id (`replace`,
/// `mutate::reconvert`): row-first commit, then canonical + sibling swap,
/// then derived-tag refresh and the `Replaced` broadcast. `processed` is the
/// pipeline's verdict on the bytes staged at `tmp_path`.
///
/// Read-side of the backup quiesce barrier is acquired only around the
/// DB-commit + rename pair — the one critical section the quiesce exists to
/// keep non-interleaving with an in-server backup's VACUUM + assets copy —
/// never across the caller's network-bound stream or CPU-bound conversion:
/// a slow uploader holding a write-preferring `tokio::sync::RwLock`'s read
/// side open would queue an admin `write()` behind it indefinitely.
///
/// Commits to the DB BEFORE swapping the live file. If the DB write fails the
/// live bytes are untouched and the record stays consistent (tmp is removed).
/// If the rename later fails, the DB is one version ahead of unchanged bytes
/// — clients re-fetch (ETag changed) and the next replace lands correctly;
/// the inverse order would strand new bytes under a stale ETag (broken 304)
/// [[commit-db-row-before-swapping-file]].
pub(super) async fn commit_replacement(
    state: &AppState,
    existing: &Asset,
    tmp_path: &std::path::Path,
    final_path: &std::path::Path,
    processed: crate::data::asset::process::Processed,
) -> Result<Asset, AppError> {
    let id = existing.id;
    let _read_permit = state.write_barrier.read().await;
    let version = match state
        .repo
        .replace_asset_bytes(
            id,
            &existing.storage_key,
            &processed.content_type,
            processed.byte_size,
            &processed.meta,
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            remove_asset_files(tmp_path).await;
            return Err(e.into());
        }
    };
    // Canonical + every sibling (a stale `.orig`/derivative of the old
    // bytes is removed when the new upload has none).
    if let Err(e) = move_asset_files(tmp_path, final_path).await {
        remove_asset_files(tmp_path).await;
        tracing::error!(?e, %id, "asset replace rename failed after DB commit");
        return Err(AppError::Internal);
    }
    // Kind/dimension/alpha tags follow the new bytes.
    state.repo.refresh_derived_tags(id).await?;

    if let Some(room) = state.ws.rooms.get(existing.world_id) {
        room.broadcast_aux(ServerMsg::AssetChanged {
            uuid: id,
            op: AssetOp::Replaced,
            version,
        });
    }

    state
        .repo
        .get_asset(id)
        .await?
        .ok_or(AppError::NotFound)
        .map(|a| Asset { version, ..a })
}

/// `DELETE /api/assets/{uuid}` — GM-gated (`require_gm`; no owner exception).
/// Undo-exempt.
///
/// `existing` (the pre-delete read) backs only `require_gm`'s authorization and the initial
/// `NotFound` check: `write_barrier`'s read side excludes a backup's write side, not a racing
/// `replace` on the same id, so `existing.version` can be stale by the time the row is actually
/// removed below. Every post-delete use (file path, broadcast) instead reads `deleted` — the row
/// `delete_asset`'s `DELETE ... RETURNING *` actually removed — so the broadcast always carries
/// the version of the row that was truly deleted, never an earlier snapshot.
pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<uuid::Uuid>,
) -> Result<StatusCode, AppError> {
    let existing = state.repo.get_asset(id).await?.ok_or(AppError::NotFound)?;
    require_gm(&state, &user, existing.world_id).await?;

    // Read-side of the backup quiesce barrier, held across the row-removal +
    // unlink pair below — without it a backup could capture the row gone but
    // the file still present (or vice versa), and in the row-gone/file-still-
    // present ordering the backup's manifest would reference a file the DB no
    // longer knows about, worse than replace's stale-bytes race.
    let _read_permit = state.write_barrier.read().await;
    let Some(deleted) = state.repo.delete_asset(id).await? else {
        // A racing delete on the same id already removed the row (and already
        // broadcast the correct notice) between the existence check above and
        // this DELETE — nothing left here to unlink or broadcast.
        return Ok(StatusCode::NO_CONTENT);
    };
    let path = state.config.assets_path().join(&deleted.storage_key);
    if let Err(e) = tokio::fs::remove_file(&path).await {
        // Record is gone; a missing file is not fatal (it becomes a no-op).
        tracing::warn!(?e, %id, "asset file remove failed after record delete");
    }
    // Siblings (`.orig`, derivatives) exist only for some assets; absence is
    // the ordinary case and not worth a warning.
    for sibling in sibling_paths(&path) {
        match tokio::fs::remove_file(&sibling).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(?e, %id, "asset sibling remove failed after record delete"),
        }
    }
    if let Some(room) = state.ws.rooms.get(deleted.world_id) {
        room.broadcast_aux(ServerMsg::AssetChanged {
            uuid: id,
            op: AssetOp::Deleted,
            version: deleted.version,
        });
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests;
