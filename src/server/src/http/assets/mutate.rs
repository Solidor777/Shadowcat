//! GM-only asset mutation routes beyond upload/replace/delete: downloading
//! the retained original and re-running the conversion from it.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use uuid::Uuid;

use crate::auth::session::AuthUser;
use crate::data::asset::process::original_path;
use crate::data::asset::{process_staged_blocking, Asset};
use crate::http::error::AppError;
use crate::http::{routes::require_gm, AppState};

use super::commit_replacement;

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
pub async fn reconvert(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Asset>, AppError> {
    let existing = gm_asset(&state, &user, id).await?;
    if !existing.meta.original_retained {
        return Err(AppError::NotFound);
    }
    let final_path = state.config.assets_path().join(&existing.storage_key);
    let tmp_path = final_path.with_file_name(format!("{id}.{}.tmp", Uuid::new_v4()));
    let copied = tokio::fs::copy(original_path(&final_path), &tmp_path).await;
    if let Err(e) = copied {
        tracing::error!(?e, %id, "retained original missing for existing record");
        return Err(AppError::Internal);
    }
    let processed = process_staged_blocking(
        tmp_path.clone(),
        existing.meta.original_content_type.clone(),
        existing.meta.original_byte_size,
        state.config.retain_originals,
    )
    .await
    .map_err(|e| {
        tracing::error!(?e, %id, "asset reconvert processing failed");
        AppError::Internal
    })?;
    let asset = commit_replacement(&state, &existing, &tmp_path, &final_path, processed).await?;
    Ok(Json(asset))
}

#[cfg(test)]
mod tests;
