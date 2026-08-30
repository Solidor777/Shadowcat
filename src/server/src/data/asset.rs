// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Pipeline-derived metadata recorded at commit (`data::asset::process`) and
/// rewritten on replace/reconvert. Flattened into `Asset` on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export, export_to = "../../types/generated/")]
pub struct AssetMeta {
    /// Canonical pixel width; `None` for a non-image or undecodable file.
    pub width: Option<u32>,
    /// Canonical pixel height; `None` for a non-image or undecodable file.
    pub height: Option<u32>,
    /// Whether the source carried an alpha channel (drives lossless encoding + the `transparent` tag).
    pub has_alpha: bool,
    /// Whether the source is an animation (served pass-through; never re-encoded).
    pub animated: bool,
    /// MIME type of the bytes that ARRIVED (vs `Asset.content_type`, the served canonical).
    pub original_content_type: String,
    /// Size of the bytes that arrived (vs `Asset.byte_size`, the served canonical).
    pub original_byte_size: i64,
    /// Whether `<uuid>.orig` exists on disk (`Config.retain_originals` AND the upload was converted).
    pub original_retained: bool,
    /// Why the upload was stored pass-through instead of converted, if it was.
    pub conversion_note: Option<String>,
}

impl AssetMeta {
    /// Metadata for bytes stored exactly as they arrived, with nothing
    /// decoded: no dimensions, no alpha/animation knowledge, no retained
    /// original. The pre-pipeline shape every commit path records until the
    /// conversion step fills the real values.
    pub fn unprocessed(content_type: &str, byte_size: i64) -> Self {
        Self {
            width: None,
            height: None,
            has_alpha: false,
            animated: false,
            original_content_type: content_type.to_string(),
            original_byte_size: byte_size,
            original_retained: false,
            conversion_note: None,
        }
    }
}

/// Who authored an asset — feeds the `uploaded` / `link-preview` derived tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// A GM upload (single-shot or chunked).
    Uploaded,
    /// A server-fetched link-preview/oEmbed image (`chat::post_publish`).
    LinkPreview,
}

/// Metadata for one stored asset. Bytes live on disk at `storage_key`
/// (relative to `assets_dir`); identity (`id`) is stable across rename/replace.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export, export_to = "../../types/generated/")]
pub struct Asset {
    /// Stable identity; survives rename and replace.
    pub id: Uuid,
    /// Owning world (assets are world-scoped).
    pub world_id: Uuid,
    /// `"<world_id>/<uuid>"`, relative to the configured assets_dir.
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
    /// Containing `asset_folder` document; `None` = world root.
    pub folder_id: Option<Uuid>,
    /// GM-set tags (client-writable via PATCH).
    pub tags: Vec<String>,
    /// Recomputed by `data::asset::tags::derive` on every commit/rename/move/reconvert; never client-writable.
    pub derived_tags: Vec<String>,
    /// Pipeline metadata (flattened onto the wire object).
    #[serde(flatten)]
    #[ts(flatten)]
    pub meta: AssetMeta,
}

/// Errors from the asset-commit path (`create_asset_from_bytes`/
/// `commit_staged_asset`): either the file-system write/rename failed (I/O)
/// or the row insert failed (`DataError`). Mirrors `http::assets::upload`'s
/// own two-stage failure surface, generalized for a caller with no
/// `AppError`/HTTP response to produce (the background image pipeline).
#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    /// Writing/renaming the asset bytes on disk failed.
    #[error("asset file write failed: {0}")]
    Io(#[from] std::io::Error),
    /// The metadata row insert failed.
    #[error("asset row insert failed: {0}")]
    Data(#[from] crate::data::DataError),
}

/// Renames an already-staged temp file into its final asset location, then
/// inserts `asset`'s metadata row — file-BEFORE-row (see
/// `create_asset_from_bytes`'s doc for why: a create has no prior bytes and
/// no existing ETag to strand, so the failure that matters is an orphan DB
/// row, not an orphan file) [[commit-db-row-before-swapping-file]]. Shared
/// commit step: `http::assets::upload` streams its OWN tmp file via
/// `store_streamed` (avoiding a second in-memory buffer for an arbitrarily
/// large GM upload) and calls this directly; `create_asset_from_bytes` stages
/// `bytes` itself first and then calls this — so both callers' resulting
/// `Asset` rows are committed through byte-for-byte the same ordering logic.
pub async fn commit_staged_asset(
    repo: &crate::data::sqlite::SqliteRepository,
    tmp_path: &std::path::Path,
    final_path: &std::path::Path,
    asset: Asset,
) -> Result<Asset, AssetError> {
    if let Err(e) = tokio::fs::rename(tmp_path, final_path).await {
        let _ = tokio::fs::remove_file(tmp_path).await;
        return Err(AssetError::Io(e));
    }
    if let Err(e) = repo.insert_asset(&asset).await {
        let _ = tokio::fs::remove_file(final_path).await;
        return Err(AssetError::Data(e));
    }
    Ok(asset)
}

/// Grouped byte-buffer/metadata parameters for `create_asset_from_bytes` —
/// grouped instead of five positional parameters (bringing the call to eight
/// total) to stay under `clippy::too_many_arguments` by restructuring the
/// signature, never by suppressing the lint (same pattern as `chat`'s
/// `RecalcRollRequestCtx`).
pub struct NewAssetBytes<'a> {
    /// The already-in-memory bytes to stage and commit.
    pub bytes: &'a [u8],
    /// MIME type to record on the row.
    pub content_type: &'a str,
    /// Display filename to record on the row (never a storage path).
    pub original_name: &'a str,
    /// `Asset.created_by` — `None` for a server-fetched asset, since the
    /// column carries a live `REFERENCES users(id)` foreign key and no real
    /// user account backs a server-initiated fetch.
    pub created_by: Option<uuid::Uuid>,
    /// Who authored the asset (drives the provenance derived tag).
    pub provenance: Provenance,
}

/// Creates an asset row from an already-in-memory byte buffer: allocates a
/// fresh `Uuid`/`storage_key`, writes `bytes` to a unique temp sibling of the
/// final path, then commits via `commit_staged_asset` (file-first-then-row,
/// unchanged ordering). For a SMALL buffer only (the link-preview/oEmbed
/// background image pipeline, capped at `chat::link_preview::MAX_IMAGE_BYTES`)
/// — `http::assets::upload`'s own arbitrarily-large GM uploads stream
/// straight to disk via `store_streamed` and call `commit_staged_asset`
/// directly instead, never buffering the whole body here.
pub async fn create_asset_from_bytes(
    repo: &crate::data::sqlite::SqliteRepository,
    assets_root: &std::path::Path,
    world_id: uuid::Uuid,
    new: NewAssetBytes<'_>,
    now: i64,
) -> Result<Asset, AssetError> {
    let NewAssetBytes {
        bytes,
        content_type,
        original_name,
        created_by,
        provenance,
    } = new;
    // Provenance reaches the derived-tag pass once the conversion step runs
    // on this path; until then the value is carried, not consumed.
    let _ = provenance;
    let id = uuid::Uuid::new_v4();
    let storage_key = format!("{world_id}/{id}");
    let final_path = assets_root.join(world_id.to_string()).join(id.to_string());
    let tmp_path = final_path.with_file_name(format!("{id}.{}.tmp", uuid::Uuid::new_v4()));
    if let Some(parent) = final_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&tmp_path, bytes).await?;
    let asset = Asset {
        id,
        world_id,
        storage_key,
        original_name: original_name.to_string(),
        content_type: content_type.to_string(),
        byte_size: bytes.len() as i64,
        created_by,
        created_at: now,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: AssetMeta::unprocessed(content_type, bytes.len() as i64),
    };
    commit_staged_asset(repo, &tmp_path, &final_path, asset).await
}

#[cfg(test)]
mod tests;
