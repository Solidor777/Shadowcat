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
    hits: Mutex<HashMap<Uuid, Vec<i64>>>,
}

impl UploadRateLimiter {
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
}

impl Default for UploadRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

use crate::auth::session::AuthUser;
use crate::data::asset::Asset;
use crate::http::error::AppError;
use crate::http::{routes::require_gm, AppState};
use crate::ws::protocol::{AssetOp, ServerMsg};
use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use tokio::io::AsyncWriteExt;

/// Stream a multipart "file" field to `dest`, enforcing `max_bytes` as bytes
/// arrive (never buffering the whole body), and validating the leading bytes are
/// a supported image. Returns (detected_content_type, byte_size, original_name).
/// On any failure the partial file is removed.
async fn store_streamed(
    mut multipart: Multipart,
    dest: &std::path::Path,
    max_bytes: u64,
) -> Result<(&'static str, i64, String), AppError> {
    let field = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
        .ok_or_else(|| AppError::BadRequest("missing file field".into()))?;
    let original_name = field.file_name().unwrap_or("upload").to_string();

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|_| AppError::Internal)?;
    }
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|_| AppError::Internal)?;
    let mut head: Vec<u8> = Vec::with_capacity(16);
    let mut total: u64 = 0;
    let mut detected: Option<&'static str> = None;

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
        if detected.is_none() {
            head.extend_from_slice(&chunk);
            if head.len() >= 12 {
                match detect_image_type(&head) {
                    Some(ct) => detected = Some(ct),
                    None => {
                        let _ = tokio::fs::remove_file(dest).await;
                        return Err(AppError::BadRequest("unsupported or non-image file".into()));
                    }
                }
            }
        }
        if file.write_all(&chunk).await.is_err() {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(AppError::Internal);
        }
    }
    file.flush().await.map_err(|_| AppError::Internal)?;

    // Files shorter than 12 bytes never reached the detector; decide on the head.
    let ct = match detected.or_else(|| detect_image_type(&head)) {
        Some(ct) => ct,
        None => {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(AppError::BadRequest("unsupported or non-image file".into()));
        }
    };
    Ok((ct, total as i64, original_name))
}

/// `POST /api/worlds/{world}/assets` — GM/owner-gated multipart image upload.
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
        return Err(AppError::TooManyRequests("upload rate limit exceeded".into()));
    }
    let id = uuid::Uuid::new_v4();
    let storage_key = format!("{world}/{id}");
    let dest = state
        .config
        .assets_path()
        .join(world.to_string())
        .join(id.to_string());
    let max = state.config.effective_max_bytes(ctx.world_role);
    let (content_type, byte_size, original_name) = store_streamed(multipart, &dest, max).await?;

    let asset = Asset {
        id,
        world_id: world,
        storage_key,
        original_name,
        content_type: content_type.to_string(),
        byte_size,
        created_by: user.id,
        created_at: now,
        version: 1,
    };
    if let Err(e) = state.repo.insert_asset(&asset).await {
        let _ = tokio::fs::remove_file(&dest).await; // keep disk and DB consistent
        return Err(e.into());
    }
    Ok(Json(asset))
}

/// `GET /api/assets/{uuid}` — read-gated by world membership; ETag-revalidated.
pub async fn serve(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<uuid::Uuid>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let asset = state.repo.get_asset(id).await?.ok_or(AppError::NotFound)?;
    // Read-gate: any member of the asset's world may read. permission_context
    // returns Forbidden for non-members.
    state
        .repo
        .permission_context(asset.world_id, user.id, user.role)
        .await?;

    let etag = format!("\"{}-{}\"", id, asset.version);
    if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        return Ok((StatusCode::NOT_MODIFIED).into_response());
    }

    let path = state.config.assets_path().join(&asset.storage_key);
    let bytes = tokio::fs::read(&path).await.map_err(|e| {
        tracing::error!(?e, %id, "asset file missing for existing record");
        AppError::Internal
    })?;
    Ok((
        [
            (header::CONTENT_TYPE, asset.content_type),
            (header::CONTENT_DISPOSITION, "inline".to_string()),
            (header::ETAG, etag),
        ],
        Body::from(bytes),
    )
        .into_response())
}

/// `POST /api/assets/{uuid}/replace` — GM/owner-gated byte-swap behind a stable
/// id. Undo-exempt: no world seq, no event-log entry.
pub async fn replace(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<uuid::Uuid>,
    multipart: Multipart,
) -> Result<Json<Asset>, AppError> {
    let existing = state.repo.get_asset(id).await?.ok_or(AppError::NotFound)?;
    let ctx = require_gm(&state, &user, existing.world_id).await?;

    // Write the new bytes to a temp sibling, then atomically replace.
    let final_path = state.config.assets_path().join(&existing.storage_key);
    let tmp_path = final_path.with_extension("tmp");
    let max = state.config.effective_max_bytes(ctx.world_role);
    let (content_type, byte_size, _name) = store_streamed(multipart, &tmp_path, max).await?;
    if let Err(e) = tokio::fs::rename(&tmp_path, &final_path).await {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        tracing::error!(?e, %id, "asset replace rename failed");
        return Err(AppError::Internal);
    }

    let version = state
        .repo
        .replace_asset_bytes(id, &existing.storage_key, content_type, byte_size)
        .await?;

    if let Some(room) = state.ws.rooms.get(existing.world_id) {
        room.broadcast_aux(ServerMsg::AssetChanged {
            uuid: id,
            op: AssetOp::Replaced,
        });
    }

    Ok(Json(Asset {
        content_type: content_type.to_string(),
        byte_size,
        version,
        ..existing
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn detects_supported_image_signatures_and_rejects_others() {
        assert_eq!(
            detect_image_type(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
            Some("image/png")
        );
        assert_eq!(detect_image_type(&[0xFF, 0xD8, 0xFF, 0x00]), Some("image/jpeg"));
        assert_eq!(detect_image_type(b"GIF89a..."), Some("image/gif"));
        assert_eq!(detect_image_type(b"RIFF\0\0\0\0WEBPxxxx"), Some("image/webp"));
        assert_eq!(detect_image_type(b"%PDF-1.7"), None);
        assert_eq!(detect_image_type(b"<svg"), None); // SVG excluded in M8
        assert_eq!(detect_image_type(&[0x89]), None); // too short to decide
    }

    #[test]
    fn rate_limiter_trips_after_per_min_then_window_slides() {
        let rl = UploadRateLimiter::new();
        let u = Uuid::from_u128(1);
        assert!(rl.check(u, 1_000, 2));
        assert!(rl.check(u, 1_500, 2));
        assert!(!rl.check(u, 1_800, 2)); // 3rd within the window → rejected
        // 61s later the earlier hits have aged out.
        assert!(rl.check(u, 62_001, 2));
    }
}
