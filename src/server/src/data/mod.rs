//! The data layer.
//!
//! It holds the document model, per-recipient permissions/redaction, command application,
//! engine-band validation, search, assets, and SQLite persistence.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod asset;
pub mod command;
pub mod document;
pub mod engine;
pub mod membership;
pub mod migrate;
pub mod permission;
pub mod repository;
pub mod search;
pub mod snapshot;
pub mod sqlite;
pub mod validation;
pub mod world_bundle;
pub mod world_seed;

pub use asset::Asset;

use thiserror::Error;

/// All fallible operations in the data layer return this.
///
/// # Examples
///
/// ```
/// use shadowcat::data::DataError;
///
/// let err = DataError::BadPath("/tags/0".into());
/// assert_eq!(err.to_string(), "invalid field path: /tags/0");
/// ```
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
    /// A size-capped JSON payload (a document `system`/`engine`/`base` block,
    /// or the merged UI-state blob) exceeded its cap.
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
    /// A structurally-checked band failed shape validation: a `system`
    /// subtree against a declared tier-2 structural schema, or the
    /// server-owned `base` snapshot against the `MergeBase` shape
    /// (`validation::validate_engine_tree`'s base walk).
    #[error("schema violation at {pointer}: {reason}")]
    SchemaViolation {
        /// JSON pointer to the violating node.
        pointer: String,
        /// Player-presentable mismatch description.
        reason: String,
    },
    /// A sandboxed validator TECHNICALLY failed (trap/out-of-fuel/malformed module) — distinct
    /// from an authored refusal, which is `OpFailed`. `ws::conn`'s `reject_reason` maps this to
    /// `RejectReason::Invalid` like `OpFailed`; `Room::commit_ops_locked`'s error arm (never
    /// `data` itself — see `crate::sandbox`'s doc) additionally compares the carried
    /// `consecutive` count against `crate::sandbox::VALIDATOR_FAULT_LIMIT` before calling
    /// `disable_faulting_validator_locked`.
    #[error("validator '{module}' faulted", module = .0.module)]
    Validator(crate::sandbox::ValidatorFault),
}
