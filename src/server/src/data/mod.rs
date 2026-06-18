pub mod command;
pub mod document;
pub mod membership;
pub mod migrate;
pub mod permission;
pub mod repository;
pub mod sqlite;
pub mod validation;

use thiserror::Error;

/// All fallible operations in the data layer return this.
#[derive(Debug, Error)]
pub enum DataError {
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("invalid field path: {0}")]
    BadPath(String),
    #[error("system body too large: {0} bytes")]
    TooLarge(usize),
    #[error("not found")]
    NotFound,
    #[error("operation failed: {0}")]
    OpFailed(String),
}
