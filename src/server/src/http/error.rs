use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// Handler error mapped to a clean status code. 5xx detail is logged, never
/// returned in the body.
#[derive(Debug)]
pub enum AppError {
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict(String),
    BadRequest(String),
    Unprocessable(String),
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
            TooLarge(n) => AppError::Unprocessable(format!("system body too large: {n} bytes")),
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

#[derive(Serialize)]
struct ErrorBody {
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
            AppError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal error".to_string(),
            ),
        };
        (status, Json(ErrorBody { error: msg })).into_response()
    }
}
