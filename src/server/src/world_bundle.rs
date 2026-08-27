//! Per-world export/import bundle I/O: builds/reads the uncompressed `.tar`
//! format (`manifest.json` + `rows/<table>.jsonl` + `assets/<asset_id>`).
//! Pure file/tar I/O — no `SqliteRepository` dependency, mirroring `backup`'s
//! separation between row-fetching (data layer) and file format (this
//! module). Synchronous (the `tar` crate has no async API); every call in
//! this module is CPU/disk-bound and must run inside
//! `tokio::task::spawn_blocking` from async callers.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use thiserror::Error;

use crate::data::world_bundle::{
    BundleManifest, ExportedAssetRow, ExportedDocumentRow, ExportedEventRow, ExportedFogRow,
    ExportedInviteRow, ExportedMemberRow, ExportedSettingRow, WorldExportData, WorldImportData,
    BUNDLE_SCHEMA_VERSION,
};

/// All fallible bundle read/write operations return this.
#[derive(Debug, Error)]
pub enum WorldBundleError {
    /// A filesystem or tar-stream operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A `manifest.json`/`rows/*.jsonl` entry failed to (de)serialize.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    /// The archive is missing a required entry, or an entry's path/shape is
    /// not one `write_bundle` could have produced.
    #[error("malformed bundle: {0}")]
    Malformed(String),
    /// A `rows/<table>.jsonl` entry's line count did not match the
    /// manifest's promise for that table — extraction stopped before any row
    /// reached a transaction.
    #[error("row count mismatch for '{table}': manifest promised {expected}, extracted {actual}")]
    RowCountMismatch {
        /// The table whose count disagreed.
        table: String,
        /// `manifest.row_counts[table]`.
        expected: usize,
        /// The number of JSONL lines actually read.
        actual: usize,
    },
    /// `manifest.schema_version` is not one this build understands.
    #[error(
        "unsupported bundle schema_version {0} (this server supports {BUNDLE_SCHEMA_VERSION})"
    )]
    UnsupportedSchemaVersion(u32),
}

/// Append one in-memory JSON blob as a tar entry at `name`.
fn append_bytes<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    name: &str,
    bytes: &[u8],
) -> Result<(), WorldBundleError> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_cksum();
    builder.append_data(&mut header, name, bytes)?;
    Ok(())
}

/// Serialize `rows` as newline-delimited JSON (one object per line).
fn to_jsonl<T: serde::Serialize>(rows: &[T]) -> Result<Vec<u8>, WorldBundleError> {
    let mut out = Vec::new();
    for row in rows {
        serde_json::to_writer(&mut out, row)?;
        out.push(b'\n');
    }
    Ok(out)
}

/// Build the `.tar` bundle for `data` into `dest`, streaming each `assets`
/// row's bytes directly from `assets_dir.join(<world_id>).join(<asset_id>)`
/// (the standard `storage_key` scheme) — `data.assets` carries no
/// `storage_key` field by design (see
/// `data::world_bundle::ExportedAssetRow`'s doc). `manifest.json` is written
/// FIRST, always — `read_bundle` relies on this ordering to resolve the
/// asset extraction root before any `assets/*` entry arrives.
///
/// Generic over the destination `Write` so a caller can hand this an owned
/// `Vec<u8>` (tests, or a future local-file export), a `File`, or a
/// channel-backed adapter that forwards each tar-internal `write` call
/// straight to an HTTP response body as it happens — `write_bundle` itself
/// never accumulates the whole tar in memory; each asset's bytes flow
/// through `dest` via `std::io::copy`'s bounded internal buffer rather than
/// being read fully into a `Vec` first.
///
/// # Examples
///
/// ```text
/// let bytes = write_bundle(&data, Path::new("/srv/shadowcat/assets"), Vec::new())?;
/// std::fs::write("world.tar", bytes)?;
/// ```
pub fn write_bundle<W: std::io::Write>(
    data: &WorldExportData,
    assets_dir: &Path,
    dest: W,
) -> Result<W, WorldBundleError> {
    let mut builder = tar::Builder::new(dest);

    let manifest_bytes = serde_json::to_vec(&data.manifest)?;
    append_bytes(&mut builder, "manifest.json", &manifest_bytes)?;

    append_bytes(
        &mut builder,
        "rows/documents.jsonl",
        &to_jsonl(&data.documents)?,
    )?;
    append_bytes(
        &mut builder,
        "rows/world_events.jsonl",
        &to_jsonl(&data.events)?,
    )?;
    append_bytes(
        &mut builder,
        "rows/world_members.jsonl",
        &to_jsonl(&data.members)?,
    )?;
    append_bytes(
        &mut builder,
        "rows/world_invites.jsonl",
        &to_jsonl(&data.invites)?,
    )?;
    append_bytes(&mut builder, "rows/assets.jsonl", &to_jsonl(&data.assets)?)?;
    append_bytes(
        &mut builder,
        "rows/explored_fog.jsonl",
        &to_jsonl(&data.fog)?,
    )?;
    append_bytes(
        &mut builder,
        "rows/settings.jsonl",
        &to_jsonl(&data.settings)?,
    )?;

    for asset in &data.assets {
        let src = assets_dir
            .join(data.manifest.world_id.to_string())
            .join(asset.id.to_string());
        let mut file = std::fs::File::open(&src)?;
        builder.append_file(format!("assets/{}", asset.id), &mut file)?;
    }

    builder.into_inner().map_err(WorldBundleError::Io)
}

/// Extract `tar_path` (a bundle the caller has already staged to disk — see
/// `http::world_bundle::import_world`) into a `WorldImportData`.
/// `assets_dir` is the server's asset root (`Config::assets_path()`); each
/// `assets/<id>` entry is staged to
/// `assets_dir.join(<world_id>).join("<id>.<random>.import-tmp")` — the same
/// directory the finalized file will live in, so
/// `SqliteRepository::import_world`'s later rename is same-filesystem.
/// `manifest.json` MUST be the archive's first entry (the invariant
/// `write_bundle` establishes) — the asset extraction root cannot be
/// resolved before the manifest is read, so any entry preceding it is
/// `Malformed`. Refuses (before returning) on a `schema_version` mismatch or
/// a `rows/<table>.jsonl` line count that disagrees with the manifest's own
/// promise — a corrupt/truncated bundle is caught here, before
/// `SqliteRepository::import_world` ever opens a transaction.
///
/// # Examples
///
/// ```text
/// let data = read_bundle(Path::new("/tmp/upload.tar"), Path::new("/srv/shadowcat/assets"))?;
/// ```
pub fn read_bundle(
    tar_path: &Path,
    assets_dir: &Path,
) -> Result<WorldImportData, WorldBundleError> {
    let file = std::fs::File::open(tar_path)?;
    let mut archive = tar::Archive::new(file);
    let mut entries = archive.entries()?;

    let Some(first) = entries.next() else {
        return Err(WorldBundleError::Malformed("empty archive".into()));
    };
    let mut first = first?;
    let first_path = first.path()?.to_string_lossy().replace('\\', "/");
    if first_path != "manifest.json" {
        return Err(WorldBundleError::Malformed(
            "manifest.json must be the first archive entry".into(),
        ));
    }
    let mut manifest_bytes = Vec::new();
    first.read_to_end(&mut manifest_bytes)?;
    let manifest: BundleManifest = serde_json::from_slice(&manifest_bytes)?;
    if manifest.schema_version != BUNDLE_SCHEMA_VERSION {
        return Err(WorldBundleError::UnsupportedSchemaVersion(
            manifest.schema_version,
        ));
    }
    drop(first);

    let world_asset_dir = assets_dir.join(manifest.world_id.to_string());
    std::fs::create_dir_all(&world_asset_dir)?;

    let mut rows: HashMap<&'static str, Vec<u8>> = HashMap::new();
    let mut staged_assets: Vec<(uuid::Uuid, std::path::PathBuf)> = Vec::new();
    // Guards against two `assets/<id>` entries sharing an id: without this,
    // the second entry's staged file would silently overwrite the first in
    // `staged_assets` bookkeeping while its own file leaks as an orphan temp
    // file (never referenced by any `staged_assets` entry, so never cleaned
    // up on the success path either).
    let mut staged_ids: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();

    // Runs the per-entry loop behind ONE fallible closure so every early
    // return inside it — not just the duplicate-asset-id rejection, but also
    // a malformed tar entry, a non-UUID asset name, an unrecognized
    // `rows/*.jsonl` path, or any I/O error from `entry?`/`read_to_end`/
    // `std::io::copy` — reaches the SAME cleanup below. A rejected/failed
    // import must leave no orphan temp files behind, regardless of which
    // check inside the loop is what actually failed.
    let loop_result: Result<(), WorldBundleError> = (|| {
        for entry in entries {
            let mut entry = entry?;
            let path = entry.path()?.to_string_lossy().replace('\\', "/");
            if let Some(id_str) = path.strip_prefix("assets/") {
                let id = uuid::Uuid::parse_str(id_str).map_err(|_| {
                    WorldBundleError::Malformed(format!("non-UUID asset entry name: {id_str}"))
                })?;
                if !staged_ids.insert(id) {
                    return Err(WorldBundleError::Malformed(format!(
                        "duplicate asset entry in bundle: {id}"
                    )));
                }
                let staged =
                    world_asset_dir.join(format!("{id}.{}.import-tmp", uuid::Uuid::new_v4()));
                let mut out = std::fs::File::create(&staged)?;
                std::io::copy(&mut entry, &mut out)?;
                staged_assets.push((id, staged));
                continue;
            }
            let table = match path.as_str() {
                "rows/documents.jsonl" => "documents",
                "rows/world_events.jsonl" => "world_events",
                "rows/world_members.jsonl" => "world_members",
                "rows/world_invites.jsonl" => "world_invites",
                "rows/assets.jsonl" => "assets",
                "rows/explored_fog.jsonl" => "explored_fog",
                "rows/settings.jsonl" => "settings",
                other => {
                    return Err(WorldBundleError::Malformed(format!(
                        "unrecognized bundle entry: {other}"
                    )))
                }
            };
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            rows.insert(table, bytes);
        }
        Ok(())
    })();
    if let Err(e) = loop_result {
        for (_, staged) in &staged_assets {
            let _ = std::fs::remove_file(staged);
        }
        return Err(e);
    }

    fn from_jsonl<T: serde::de::DeserializeOwned>(
        bytes: &[u8],
    ) -> Result<Vec<T>, WorldBundleError> {
        bytes
            .split(|&b| b == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).map_err(WorldBundleError::from))
            .collect()
    }

    fn take<'a>(
        rows: &'a HashMap<&'static str, Vec<u8>>,
        table: &'static str,
    ) -> Result<&'a [u8], WorldBundleError> {
        rows.get(table)
            .map(Vec::as_slice)
            .ok_or_else(|| WorldBundleError::Malformed(format!("missing rows/{table}.jsonl")))
    }

    let documents: Vec<ExportedDocumentRow> = from_jsonl(take(&rows, "documents")?)?;
    let events: Vec<ExportedEventRow> = from_jsonl(take(&rows, "world_events")?)?;
    let members: Vec<ExportedMemberRow> = from_jsonl(take(&rows, "world_members")?)?;
    let invites: Vec<ExportedInviteRow> = from_jsonl(take(&rows, "world_invites")?)?;
    let assets: Vec<ExportedAssetRow> = from_jsonl(take(&rows, "assets")?)?;
    let fog: Vec<ExportedFogRow> = from_jsonl(take(&rows, "explored_fog")?)?;
    let settings: Vec<ExportedSettingRow> = from_jsonl(take(&rows, "settings")?)?;

    for (table, actual) in [
        ("documents", documents.len()),
        ("world_events", events.len()),
        ("world_members", members.len()),
        ("world_invites", invites.len()),
        ("assets", assets.len()),
        ("explored_fog", fog.len()),
        ("settings", settings.len()),
    ] {
        let expected = *manifest.row_counts.get(table).ok_or_else(|| {
            WorldBundleError::Malformed(format!("manifest missing row_counts['{table}']"))
        })?;
        if expected != actual {
            return Err(WorldBundleError::RowCountMismatch {
                table: table.to_string(),
                expected,
                actual,
            });
        }
    }

    Ok(WorldImportData {
        manifest,
        documents,
        events,
        members,
        invites,
        assets,
        fog,
        settings,
        staged_assets,
    })
}

#[cfg(test)]
mod tests;
