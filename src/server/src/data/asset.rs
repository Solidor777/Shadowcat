// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Metadata for one stored asset. Bytes live on disk at `storage_key`
/// (relative to `assets_dir`); identity (`id`) is stable across rename/replace.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export, export_to = "../../types/generated/")]
pub struct Asset {
    /// Stable identity; survives rename and replace.
    pub id: Uuid,
    /// Owning world (assets are world-scoped).
    pub world_id: Uuid,
    /// "<world_id>/<uuid>", relative to the configured assets_dir.
    pub storage_key: String,
    /// Filename as uploaded (display only; never a storage path).
    pub original_name: String,
    /// MIME type recorded at upload.
    pub content_type: String,
    /// Size of the stored bytes.
    pub byte_size: i64,
    /// NULL when the uploading account has been deleted.
    pub created_by: Option<Uuid>,
    /// Upload time, Unix epoch milliseconds.
    pub created_at: i64,
    /// Bumped on every replace; backs the ETag and the resync source of truth.
    pub version: i64,
}
