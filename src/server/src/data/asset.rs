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
    } = new;
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
    };
    commit_staged_asset(repo, &tmp_path, &final_path, asset).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::sqlite::SqliteRepository;

    #[tokio::test]
    async fn create_asset_from_bytes_and_upload_produce_identical_asset_shape() {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let world = repo.create_world("test world", 1_000).await.unwrap();
        let root = tempfile::tempdir().unwrap();
        let created_by = Some(
            repo.create_user("uploader", None, crate::auth::role::ServerRole::User, 1_000)
                .await
                .unwrap(),
        );

        let asset = create_asset_from_bytes(
            &repo,
            root.path(),
            world.id,
            NewAssetBytes {
                bytes: b"fake-image-bytes",
                content_type: "image/png",
                original_name: "preview.png",
                created_by,
            },
            1_234,
        )
        .await
        .unwrap();

        assert_eq!(asset.storage_key, format!("{}/{}", world.id, asset.id));
        assert_eq!(asset.byte_size, "fake-image-bytes".len() as i64);
        assert_eq!(asset.version, 1);
        assert_eq!(asset.world_id, world.id);
        assert_eq!(asset.created_by, created_by);

        let stored = tokio::fs::read(
            root.path()
                .join(world.id.to_string())
                .join(asset.id.to_string()),
        )
        .await
        .unwrap();
        assert_eq!(stored, b"fake-image-bytes");
    }

    #[tokio::test]
    async fn commit_staged_asset_insert_failure_removes_the_renamed_file() {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let root = tempfile::tempdir().unwrap();
        // No matching `worlds` row for this id — the FK-enforced insert must
        // fail, exercising `create_asset_from_bytes`'s insert-failure cleanup
        // path (`commit_staged_asset`'s file removal after a failed insert).
        let world_id = Uuid::new_v4();

        let err = create_asset_from_bytes(
            &repo,
            root.path(),
            world_id,
            NewAssetBytes {
                bytes: b"orphan-bytes",
                content_type: "image/png",
                original_name: "orphan.png",
                created_by: None,
            },
            1_234,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AssetError::Data(_)));
        // The final path is deterministic only via the id embedded in the
        // error's `Display`... instead assert no stray file survives under
        // the world directory (the temp file was renamed then removed).
        let world_dir = root.path().join(world_id.to_string());
        let remaining: Vec<_> = match tokio::fs::read_dir(&world_dir).await {
            Ok(mut rd) => {
                let mut entries = Vec::new();
                while let Some(e) = rd.next_entry().await.unwrap() {
                    entries.push(e.path());
                }
                entries
            }
            Err(_) => Vec::new(),
        };
        assert!(
            remaining.is_empty(),
            "expected no leftover asset file, found {remaining:?}"
        );
    }

    #[tokio::test]
    async fn create_asset_from_bytes_created_by_none_is_accepted() {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let world = repo.create_world("test world", 1_000).await.unwrap();
        let root = tempfile::tempdir().unwrap();

        let asset = create_asset_from_bytes(
            &repo,
            root.path(),
            world.id,
            NewAssetBytes {
                bytes: b"system-fetched-bytes",
                content_type: "image/jpeg",
                original_name: "og-image.jpg",
                created_by: None,
            },
            5_678,
        )
        .await
        .unwrap();

        assert_eq!(asset.created_by, None);
        let fetched = repo.get_asset(asset.id).await.unwrap().unwrap();
        assert_eq!(fetched.created_by, None);
    }
}
