//! The data layer: the document model, per-recipient permissions/redaction,
//! command application, validation, search, assets, and SQLite persistence.

/// Uploaded binary assets: rows, content-addressed storage, ETag revalidation.
pub mod asset;
/// Commands/operations/field changes + the shared JSON-pointer mutation ops.
pub mod command;
/// The document envelope, permission types, and structural schema types.
pub mod document;
pub mod engine;
pub mod membership;
/// Schema migrations applied at pool open.
pub mod migrate;
/// The authz/redaction core: access resolution + per-recipient filtering.
pub mod permission;
/// The storage seam (`Repository` trait) the HTTP/WS layers program against.
pub mod repository;
pub mod search;
/// The SQLite `Repository` implementation + the authoritative apply loops.
pub mod sqlite;
/// Structural (never semantic) validation: size caps, paths, tier-2 shape.
pub mod validation;

pub use asset::Asset;

use thiserror::Error;

/// All fallible operations in the data layer return this.
#[derive(Debug, Error)]
pub enum DataError {
    /// The underlying SQLite operation failed.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// JSON (de)serialization failed.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    /// A JSON pointer was malformed or addressed an illegal target
    /// (e.g. array-index removal).
    #[error("invalid field path: {0}")]
    BadPath(String),
    /// A `system`/`engine`/`base` block exceeded the size cap.
    #[error("system body too large: {0} bytes")]
    TooLarge(usize),
    /// The addressed row/document does not exist (also used to hide existence).
    #[error("not found")]
    NotFound,
    /// A validated operation could not be applied.
    #[error("operation failed: {0}")]
    OpFailed(String),
    /// The actor lacks the capability/role the operation requires.
    #[error("forbidden")]
    Forbidden,
    /// An OCC pre-image mismatch or duplicate-singleton create.
    #[error("conflict: {0}")]
    Conflict(String),
    /// An `engine` band failed typed ingress validation.
    #[error("invalid engine body: {0}")]
    BadEngine(String),
    /// A `system` band violated a declared tier-2 structural schema.
    #[error("schema violation at {pointer}: {reason}")]
    SchemaViolation {
        /// JSON pointer to the violating node.
        pointer: String,
        /// Player-presentable mismatch description.
        reason: String,
    },
}
