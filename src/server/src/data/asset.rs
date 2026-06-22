use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Metadata for one stored asset. Bytes live on disk at `storage_key`
/// (relative to `assets_dir`); identity (`id`) is stable across rename/replace.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export, export_to = "../../types/generated/")]
pub struct Asset {
    pub id: Uuid,
    pub world_id: Uuid,
    /// "<world_id>/<uuid>", relative to the configured assets_dir.
    pub storage_key: String,
    pub original_name: String,
    pub content_type: String,
    pub byte_size: i64,
    pub created_by: Uuid,
    pub created_at: i64,
    /// Bumped on every replace; backs the ETag and the resync source of truth.
    pub version: i64,
}
