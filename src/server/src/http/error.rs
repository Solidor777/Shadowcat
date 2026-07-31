#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// Handler error mapped to a clean status code. 5xx detail is logged, never
/// returned in the body.
#[derive(Debug)]
pub enum AppError {
    /// 401: no valid session.
    Unauthorized,
    /// 403: authenticated but not allowed.
    Forbidden,
    /// 404: absent — also used to HIDE existence where READ is refused.
    NotFound,
    /// 409: OCC mismatch / duplicate singleton / last-GM removal.
    Conflict(String),
    /// 400: malformed request.
    BadRequest(String),
    /// 422: well-formed but semantically rejected payload.
    Unprocessable(String),
    /// 413: body over the configured cap.
    PayloadTooLarge(String),
    /// 429: a throttle budget was exceeded.
    TooManyRequests(String),
    /// 500: server fault — detail logged, never echoed to the client.
    Internal,
}

/// Map a data-layer error to its client-facing status. `Sqlx`/`Serde` are
/// server faults (500, detail logged, not echoed); the rest are client-actionable.
impl From<crate::data::DataError> for AppError {
    fn from(e: crate::data::DataError) -> Self {
        use crate::data::DataError::*;
        match e {
            Forbidden => AppError::Forbidden,
            Conflict(m) => AppError::Conflict(m),
            NotFound => AppError::NotFound,
            BadPath(m) => AppError::Unprocessable(format!("invalid field path: {m}")),
            TooLarge(n) => AppError::Unprocessable(format!("payload too large: {n} bytes")),
            BadEngine(m) => AppError::Unprocessable(format!("invalid engine body: {m}")),
            SchemaViolation { pointer, reason } => {
                AppError::Unprocessable(format!("schema violation at {pointer}: {reason}"))
            }
            OpFailed(m) => AppError::BadRequest(m),
            Sqlx(e) => {
                tracing::error!(?e, "database error");
                AppError::Internal
            }
            Serde(e) => {
                tracing::error!(?e, "serialization error");
                AppError::Internal
            }
        }
    }
}

/// The uniform JSON error payload: `{ "error": "..." }`.
#[derive(Serialize)]
struct ErrorBody {
    /// Player-presentable message.
    error: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_string()),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "forbidden".to_string()),
            AppError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            AppError::Conflict(m) => (StatusCode::CONFLICT, m),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            AppError::Unprocessable(m) => (StatusCode::UNPROCESSABLE_ENTITY, m),
            AppError::PayloadTooLarge(m) => (StatusCode::PAYLOAD_TOO_LARGE, m),
            AppError::TooManyRequests(m) => (StatusCode::TOO_MANY_REQUESTS, m),
            AppError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal error".to_string(),
            ),
        };
        (status, Json(ErrorBody { error: msg })).into_response()
    }
}
