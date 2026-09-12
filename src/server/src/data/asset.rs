// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Pipeline-derived metadata recorded at commit (`data::asset::process`) and
/// rewritten on replace/reconvert. Flattened into `Asset` on the wire.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::AssetMeta;
///
/// let meta = AssetMeta {
///     width: Some(64),
///     ..AssetMeta::unprocessed("image/png", 10)
/// };
/// // Fields not named in the literal come from `unprocessed`, not the update syntax.
/// assert_eq!(meta.original_byte_size, 10);
/// assert!(meta.height.is_none());
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS, PartialEq)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(default)]
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
    /// Decoded audio duration, milliseconds; `None` for a non-audio asset or a decode
    /// failure. `f64`-adjacent precision is unnecessary here — this is metadata display, not
    /// a sync anchor (unlike `PlayingTrack.startedAt`), so a plain `i64` is fine: it is
    /// consumed only by `AssetResolver`/display code, never round-tripped through a
    /// client-authored write.
    pub duration_ms: Option<i64>,
    /// Decoded audio sample rate, Hz (the SOURCE rate, before the pipeline's 48 kHz Opus
    /// resample); `None` for a non-audio asset or a decode failure.
    pub sample_rate: Option<i64>,
}

impl AssetMeta {
    /// Metadata for bytes stored exactly as they arrived, with nothing
    /// decoded: no dimensions, no alpha/animation knowledge, no retained
    /// original. The pre-pipeline shape every commit path records until the
    /// conversion step fills the real values.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::asset::AssetMeta;
    ///
    /// let meta = AssetMeta::unprocessed("image/png", 2048);
    /// assert_eq!(meta.original_content_type, "image/png");
    /// assert!(meta.width.is_none());
    /// ```
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
            duration_ms: None,
            sample_rate: None,
        }
    }
}

/// Who authored an asset — feeds the `uploaded` / `link-preview` /
/// `chat-image` derived tag.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::Provenance;
/// use shadowcat::data::asset::tags::provenance_of;
///
/// // The provenance a stored derived-tag set encodes, recovered by `provenance_of`.
/// let provenance = provenance_of(&["chat-image".to_string()]);
/// assert_eq!(provenance, Provenance::ChatImage);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// A GM upload (single-shot or chunked).
    Uploaded,
    /// A server-fetched link-preview/oEmbed image (`chat::post_publish`).
    LinkPreview,
    /// A server-fetched external chat image, asset-ified by
    /// `chat::post_publish::resolve_inline_image` — distinct from
    /// `LinkPreview` (a link-card thumbnail) even though both are
    /// server-fetched, since a chat image is the message's own primary
    /// content rather than a preview of a linked page.
    ChatImage,
}

/// Metadata for one stored asset. Bytes live on disk at `storage_key`
/// (relative to `assets_dir`); identity (`id`) is stable across rename/replace.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::query::{sort_key_of, AssetSort};
/// use shadowcat::data::asset::{Asset, AssetMeta};
/// use uuid::Uuid;
///
/// let asset = Asset {
///     id: Uuid::new_v4(),
///     world_id: Uuid::new_v4(),
///     storage_key: "w/id".into(),
///     original_name: "Map.png".into(),
///     content_type: "image/webp".into(),
///     byte_size: 10,
///     created_by: None,
///     created_at: 42,
///     version: 1,
///     folder_id: None,
///     tags: vec![],
///     derived_tags: vec![],
///     meta: AssetMeta::unprocessed("image/png", 10),
/// };
/// // `sort_key_of` derives its output from the asset's fields; it is not a value
/// // the literal above assigns directly.
/// assert_eq!(sort_key_of(&asset, AssetSort::Name), "map.png");
/// assert_eq!(sort_key_of(&asset, AssetSort::Created), "42");
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::AssetError;
///
/// let err: AssetError = std::io::Error::new(std::io::ErrorKind::NotFound, "missing").into();
/// assert!(matches!(err, AssetError::Io(_)));
/// ```
#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    /// Writing/renaming the asset bytes on disk failed.
    #[error("asset file write failed: {0}")]
    Io(#[from] std::io::Error),
    /// The metadata row insert failed.
    #[error("asset row insert failed: {0}")]
    Data(#[from] crate::data::DataError),
}

/// Moves a processed upload from its staged stem to its final stem: the
/// canonical file, then every sibling artifact (`process::sibling_paths`) —
/// a sibling present at the staged stem replaces the one at the final stem,
/// and a sibling ABSENT at the staged stem removes any stale one at the
/// final stem (a pass-through replace has no `.orig`; an undecodable one has
/// no derivatives). A missing final sibling is not an error.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::move_asset_files;
/// use std::path::Path;
///
/// # #[tokio::main]
/// # async fn main() {
/// // Renaming a staged stem that was never written fails rather than panicking.
/// let err = move_asset_files(Path::new("no-such-staged"), Path::new("no-such-final"))
///     .await
///     .unwrap_err();
/// assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
/// # }
/// ```
pub async fn move_asset_files(
    staged: &std::path::Path,
    final_path: &std::path::Path,
) -> std::io::Result<()> {
    tokio::fs::rename(staged, final_path).await?;
    for (from, to) in process::sibling_paths(staged)
        .iter()
        .zip(process::sibling_paths(final_path).iter())
    {
        if tokio::fs::try_exists(from).await? {
            tokio::fs::rename(from, to).await?;
        } else {
            match tokio::fs::remove_file(to).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

/// Best-effort removal of a canonical and every sibling artifact; a file
/// that is already gone is not an error. Used on every rollback path and by
/// asset delete.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::process::sibling_paths;
/// use shadowcat::data::asset::remove_asset_files;
///
/// # #[tokio::main]
/// # async fn main() {
/// // A directory outside the repo tree — never written to source control.
/// let dir = tempfile::tempdir().unwrap();
/// let canonical = dir.path().join("uuid");
/// std::fs::write(&canonical, b"bytes").unwrap();
/// for sibling in sibling_paths(&canonical) {
///     std::fs::write(&sibling, b"bytes").unwrap();
/// }
/// remove_asset_files(&canonical).await;
/// assert!(!canonical.exists());
/// assert!(sibling_paths(&canonical).iter().all(|p| !p.exists()));
/// # }
/// ```
pub async fn remove_asset_files(canonical: &std::path::Path) {
    let _ = tokio::fs::remove_file(canonical).await;
    for p in process::sibling_paths(canonical) {
        let _ = tokio::fs::remove_file(p).await;
    }
}

/// Runs `process::process_staged` on the blocking pool (it is CPU-bound).
/// On any failure the staged file and its siblings are removed, so a caller
/// never has to reason about a half-processed stem.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::process_staged_blocking;
/// use std::path::PathBuf;
///
/// # #[tokio::main]
/// # async fn main() {
/// // A missing staged file fails to open rather than panicking.
/// let err = process_staged_blocking(PathBuf::from("no-such-staged"), "image/png".into(), 0, true)
///     .await
///     .unwrap_err();
/// assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
/// # }
/// ```
pub async fn process_staged_blocking(
    staged: std::path::PathBuf,
    original_content_type: String,
    original_byte_size: i64,
    retain_originals: bool,
) -> std::io::Result<process::Processed> {
    let path = staged.clone();
    let result = tokio::task::spawn_blocking(move || {
        process::process_staged(
            &path,
            &original_content_type,
            original_byte_size,
            retain_originals,
        )
    })
    .await
    .map_err(std::io::Error::other)
    .and_then(|r| r);
    if result.is_err() {
        remove_asset_files(&staged).await;
    }
    result
}

/// Moves an already-processed staged stem into its final location, then
/// inserts `asset`'s metadata row and its derived tags — file-BEFORE-row (see
/// `create_asset_from_bytes`'s doc for why: a create has no prior bytes and
/// no existing ETag to strand, so the failure that matters is an orphan DB
/// row, not an orphan file) [[commit-db-row-before-swapping-file]]. Shared
/// commit step: `http::assets::upload` streams its OWN tmp file via
/// `store_streamed` (avoiding a second in-memory buffer for an arbitrarily
/// large GM upload), processes it in place, and calls this directly;
/// `create_asset_from_bytes` stages + processes `bytes` itself first and then
/// calls this — so both callers' resulting `Asset` rows are committed through
/// byte-for-byte the same ordering logic. A row-insert failure removes every
/// file just moved; a tag-write failure after the row is in leaves the asset
/// untagged (`refresh_derived_tags` repairs it) rather than orphaning files.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::{commit_staged_asset, Asset, AssetError, AssetMeta};
/// use shadowcat::data::sqlite::SqliteRepository;
/// use std::path::Path;
/// use uuid::Uuid;
///
/// # #[tokio::main]
/// # async fn main() {
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let asset = Asset {
///     id: Uuid::new_v4(),
///     world_id: Uuid::new_v4(),
///     storage_key: "w/id".into(),
///     original_name: "map.png".into(),
///     content_type: "image/webp".into(),
///     byte_size: 10,
///     created_by: None,
///     created_at: 0,
///     version: 1,
///     folder_id: None,
///     tags: vec![],
///     derived_tags: vec![],
///     meta: AssetMeta::unprocessed("image/png", 10),
/// };
/// // No file was ever staged, so the move fails before the row insert.
/// let err = commit_staged_asset(
///     &repo,
///     Path::new("no-such-staged"),
///     Path::new("no-such-final"),
///     asset,
///     &[],
/// )
/// .await
/// .unwrap_err();
/// assert!(matches!(err, AssetError::Io(_)));
/// # }
/// ```
pub async fn commit_staged_asset(
    repo: &crate::data::sqlite::SqliteRepository,
    tmp_path: &std::path::Path,
    final_path: &std::path::Path,
    asset: Asset,
    derived_tags: &[String],
) -> Result<Asset, AssetError> {
    if let Err(e) = move_asset_files(tmp_path, final_path).await {
        remove_asset_files(tmp_path).await;
        remove_asset_files(final_path).await;
        return Err(AssetError::Io(e));
    }
    if let Err(e) = repo.insert_asset(&asset).await {
        remove_asset_files(final_path).await;
        return Err(AssetError::Data(e));
    }
    repo.set_asset_tags(asset.id, &asset.tags, derived_tags)
        .await?;
    Ok(Asset {
        derived_tags: derived_tags.to_vec(),
        ..asset
    })
}

/// Grouped byte-buffer/metadata parameters for `create_asset_from_bytes` —
/// grouped instead of five positional parameters (bringing the call to eight
/// total) to stay under `clippy::too_many_arguments` by restructuring the
/// signature, never by suppressing the lint (same pattern as `chat`'s
/// `RecalcRollRequestCtx`).
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::tags::{derive, DeriveInput};
/// use shadowcat::data::asset::{AssetMeta, NewAssetBytes, Provenance};
///
/// let bytes = [0u8; 4];
/// let new = NewAssetBytes {
///     bytes: &bytes,
///     content_type: "image/png",
///     original_name: "map.png",
///     created_by: None,
///     provenance: Provenance::Uploaded,
///     retain_originals: true,
/// };
/// // The `provenance` field drives the derived tag set `tags::derive` computes.
/// let tags = derive(DeriveInput {
///     content_type: new.content_type,
///     meta: &AssetMeta::unprocessed(new.content_type, new.bytes.len() as i64),
///     folder_names: &[],
///     provenance: new.provenance,
/// });
/// assert!(tags.contains(&"uploaded".to_string()));
/// ```
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
    /// `Config.retain_originals`: keep the arrived bytes as `.orig` when converted.
    pub retain_originals: bool,
}

/// Creates an asset from an already-in-memory byte buffer: allocates a fresh
/// `Uuid`/`storage_key`, writes `bytes` to a unique temp sibling of the final
/// path, runs the conversion pipeline on it, derives tags, then commits via
/// `commit_staged_asset` (file-first-then-row, unchanged ordering). For a
/// SMALL buffer only (the link-preview/oEmbed background image pipeline,
/// capped at `chat::link_preview::MAX_IMAGE_BYTES`) — `http::assets::upload`'s
/// own arbitrarily-large GM uploads stream straight to disk via
/// `store_streamed` and call `commit_staged_asset` directly instead, never
/// buffering the whole body here. The asset lands in the world root.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::{create_asset_from_bytes, NewAssetBytes, Provenance};
/// use shadowcat::data::sqlite::SqliteRepository;
/// use std::io::Cursor;
///
/// # #[tokio::main]
/// # async fn main() {
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let world = repo.create_world("w", 0).await.unwrap();
/// // A directory outside the repo tree — never written to source control.
/// let dir = tempfile::tempdir().unwrap();
/// let mut png = Vec::new();
/// image::RgbaImage::new(2, 2)
///     .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
///     .unwrap();
/// let asset = create_asset_from_bytes(
///     &repo,
///     dir.path(),
///     world.id,
///     NewAssetBytes {
///         bytes: &png,
///         content_type: "image/png",
///         original_name: "swatch.png",
///         created_by: None,
///         provenance: Provenance::Uploaded,
///         retain_originals: false,
///     },
///     0,
/// )
/// .await
/// .unwrap();
/// assert_eq!(asset.content_type, "image/webp");
/// # }
/// ```
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
        retain_originals,
    } = new;
    let id = uuid::Uuid::new_v4();
    let storage_key = format!("{world_id}/{id}");
    let final_path = assets_root.join(world_id.to_string()).join(id.to_string());
    let tmp_path = final_path.with_file_name(format!("{id}.{}.tmp", uuid::Uuid::new_v4()));
    if let Some(parent) = final_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&tmp_path, bytes).await?;
    let processed = process_staged_blocking(
        tmp_path.clone(),
        content_type.to_string(),
        bytes.len() as i64,
        retain_originals,
    )
    .await?;
    let derived = tags::derive(tags::DeriveInput {
        content_type: &processed.content_type,
        meta: &processed.meta,
        folder_names: &[],
        provenance,
    });
    let asset = Asset {
        id,
        world_id,
        storage_key,
        original_name: original_name.to_string(),
        content_type: processed.content_type,
        byte_size: processed.byte_size,
        created_by,
        created_at: now,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: processed.meta,
    };
    commit_staged_asset(repo, &tmp_path, &final_path, asset, &derived).await
}

pub mod process;
pub mod query;
pub mod tags;

#[cfg(test)]
mod tests;
