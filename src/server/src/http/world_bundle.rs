//! `POST /api/worlds/{id}/export` and `POST /api/worlds/import` — per-world
//! bundle export/import
//! (`docs/superpowers/specs/2026-08-21-world-export-import-design.md`).
//! Export is world-GM-gated (`require_gm`, mirroring the existing GM-gated
//! asset routes). Import is server-admin-only: a bulk multi-table insert
//! that bypasses every capability/schema/OCC gate the live write paths
//! enforce (the same trusted-substrate posture `apply_command`'s replay path
//! already has) — a materially more privileged operation than ordinary world
//! CREATION (`POST /api/worlds`, open to any authenticated user) or GM-level
//! world management, so it needs the server's highest tier, not a match to
//! either of those.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use crate::auth::session::AuthUser;
use crate::http::error::AppError;
use crate::http::routes::require_gm;
use crate::http::AppState;
use crate::world_bundle::write_bundle;

/// `POST /api/worlds/{id}/export` — world-GM-gated (server admins resolve to
/// GM via `require_gm`). Streams the world's `.tar` bundle as the response
/// body.
pub async fn export_world(
    State(state): State<AppState>,
    user: AuthUser,
    Path(world): Path<Uuid>,
) -> Result<Response, AppError> {
    require_gm(&state, &user, world).await?;
    let data = state.repo.export_world_rows(world).await?;
    let assets_dir = state.config.assets_path();
    let bytes = tokio::task::spawn_blocking(move || write_bundle(&data, &assets_dir))
        .await
        .map_err(|e| {
            tracing::error!(?e, %world, "world export task panicked");
            AppError::Internal
        })?
        .map_err(|e| {
            tracing::error!(?e, %world, "world export failed");
            AppError::Internal
        })?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-tar".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"world-{world}.tar\""),
            ),
        ],
        Body::from(bytes),
    )
        .into_response())
}
