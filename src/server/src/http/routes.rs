use std::sync::atomic::Ordering;
use std::sync::OnceLock;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use tower_sessions::Session;

use crate::auth::password::{hash_password, verify_password};
use crate::auth::role::ServerRole;
use crate::auth::session::{AuthUser, SessionUser};
use crate::auth::setup::{create_admin, now_millis};
use crate::health::HealthStatus;
use crate::http::error::AppError;
use crate::http::AppState;

/// Liveness + DB connectivity probe.
pub async fn health(State(state): State<AppState>) -> Json<HealthStatus> {
    let connected = sqlx::query("SELECT 1")
        .fetch_one(state.repo.pool())
        .await
        .is_ok();
    Json(HealthStatus::ok(connected))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct MeResponse {
    pub id: uuid::Uuid,
    pub username: String,
    pub server_role: ServerRole,
}

/// Current session identity, or 401.
pub async fn me(user: AuthUser) -> Json<MeResponse> {
    Json(MeResponse { id: user.id, username: user.username, server_role: user.role })
}

/// A real Argon2id hash of a throwaway password, computed once. The unknown-user
/// login path verifies against it so it costs the same as a wrong-password path,
/// removing a timing oracle that would otherwise reveal which usernames exist.
fn anti_enumeration_phc() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY
        .get_or_init(|| hash_password("anti-enumeration-unused").expect("hash dummy"))
        .as_str()
}

/// Verify credentials and establish a session. Uniform 401 on unknown user or
/// wrong password — no enumeration. Always runs a verify to keep timing flat.
pub async fn login(
    State(state): State<AppState>,
    session: Session,
    Json(body): Json<LoginRequest>,
) -> Result<axum::http::StatusCode, AppError> {
    let record = state
        .repo
        .user_by_username(&body.username)
        .await
        .map_err(|_| AppError::Internal)?;

    let ok = match &record {
        Some(u) => u
            .password_hash
            .as_deref()
            .map(|h| verify_password(&body.password, h))
            .unwrap_or(false),
        None => {
            let _ = verify_password(&body.password, anti_enumeration_phc());
            false
        }
    };
    if !ok {
        return Err(AppError::Unauthorized);
    }
    let u = record.expect("ok implies record present");
    session
        .insert("user", SessionUser { id: u.id, username: u.username, role: u.server_role })
        .await
        .map_err(|_| AppError::Internal)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Destroy the session.
pub async fn logout(session: Session) -> axum::http::StatusCode {
    let _ = session.flush().await;
    axum::http::StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
    pub token: Option<String>,
}

/// First-run admin creation. Gated: 409 once initialized; 403 on token mismatch
/// when a token is required. Flips `initialized` so the gate opens.
pub async fn setup(
    State(state): State<AppState>,
    Json(body): Json<SetupRequest>,
) -> Result<axum::http::StatusCode, AppError> {
    if state.initialized.load(Ordering::Relaxed)
        || state.repo.admin_exists().await.map_err(|_| AppError::Internal)?
    {
        return Err(AppError::Conflict("server already initialized".into()));
    }
    if let Some(expected) = &state.setup_token {
        if body.token.as_deref() != Some(expected.as_str()) {
            return Err(AppError::Forbidden);
        }
    }
    create_admin(&state.repo, &body.username, &body.password, now_millis()).await?;
    state.initialized.store(true, Ordering::Relaxed);
    Ok(axum::http::StatusCode::NO_CONTENT)
}
