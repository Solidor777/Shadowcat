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
mod tests {
    use super::*;
    use crate::data::document::{Document, Scope};
    use crate::data::world_bundle::WorldExportData;
    use uuid::Uuid;

    fn minimal_document() -> Document {
        Document {
            id: Uuid::from_u128(1),
            scope: Scope::World {
                world_id: Uuid::from_u128(9),
            },
            doc_type: "actor".into(),
            schema_version: 1,
            name: None,
            source: None,
            base: None,
            owner: None,
            permissions: Default::default(),
            embedded: Default::default(),
            parent_id: None,
            engine: crate::data::document::tests::default_test_engine("actor"),
            system: serde_json::json!({}),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn sample_data(world: Uuid) -> WorldExportData {
        use crate::data::world_bundle::{
            BundleManifest, ExportedDocumentRow, BUNDLE_SCHEMA_VERSION,
        };
        let mut row_counts = std::collections::BTreeMap::new();
        row_counts.insert("documents".to_string(), 1);
        row_counts.insert("world_events".to_string(), 0);
        row_counts.insert("world_members".to_string(), 0);
        row_counts.insert("world_invites".to_string(), 0);
        row_counts.insert("assets".to_string(), 0);
        row_counts.insert("explored_fog".to_string(), 0);
        row_counts.insert("settings".to_string(), 0);
        WorldExportData {
            manifest: BundleManifest {
                schema_version: BUNDLE_SCHEMA_VERSION,
                world_id: world,
                world_name: "W".to_string(),
                world_seq: 5,
                world_created_at: 10,
                world_updated_at: 20,
                exported_at_unix_ms: 30,
                row_counts,
            },
            documents: vec![ExportedDocumentRow {
                document: minimal_document(),
                owner_username: None,
                seq: 1,
                created_seq: 1,
            }],
            events: vec![],
            members: vec![],
            invites: vec![],
            assets: vec![],
            fog: vec![],
            settings: vec![],
        }
    }

    #[test]
    fn write_bundle_with_no_assets_produces_a_readable_manifest_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let world = Uuid::from_u128(42);
        let data = sample_data(world);

        let bytes = write_bundle(&data, tmp.path(), Vec::new()).unwrap();
        let mut archive = tar::Archive::new(bytes.as_slice());
        let mut entries = archive.entries().unwrap();
        let mut first = entries.next().unwrap().unwrap();
        assert_eq!(first.path().unwrap().to_string_lossy(), "manifest.json");
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut first, &mut buf).unwrap();
        let manifest: crate::data::world_bundle::BundleManifest =
            serde_json::from_slice(&buf).unwrap();
        assert_eq!(manifest.world_id, world);
        assert_eq!(manifest.world_seq, 5);
    }

    #[test]
    fn write_bundle_streams_asset_bytes_from_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let world = Uuid::from_u128(7);
        let asset_id = Uuid::from_u128(100);
        let asset_dir = tmp.path().join(world.to_string());
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join(asset_id.to_string()), b"PNGDATA").unwrap();

        let mut data = sample_data(world);
        data.assets
            .push(crate::data::world_bundle::ExportedAssetRow {
                id: asset_id,
                original_name: "token.png".to_string(),
                content_type: "image/png".to_string(),
                byte_size: 7,
                created_by_username: None,
                created_at: 0,
                version: 1,
            });
        data.manifest.row_counts.insert("assets".to_string(), 1);

        let bytes = write_bundle(&data, tmp.path(), Vec::new()).unwrap();
        let mut archive = tar::Archive::new(bytes.as_slice());
        let found = archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap())
            .any(|e| e.path().unwrap().to_string_lossy() == format!("assets/{asset_id}"));
        assert!(found);
    }

    #[test]
    fn write_bundle_missing_asset_file_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let world = Uuid::from_u128(8);
        let mut data = sample_data(world);
        data.assets
            .push(crate::data::world_bundle::ExportedAssetRow {
                id: Uuid::from_u128(200),
                original_name: "missing.png".to_string(),
                content_type: "image/png".to_string(),
                byte_size: 1,
                created_by_username: None,
                created_at: 0,
                version: 1,
            });
        let err = write_bundle(&data, tmp.path(), Vec::new()).unwrap_err();
        assert!(matches!(err, WorldBundleError::Io(_)));
    }

    #[test]
    fn read_bundle_round_trips_write_bundle_output() {
        let export_tmp = tempfile::tempdir().unwrap();
        let world = Uuid::from_u128(55);
        let asset_id = Uuid::from_u128(555);
        let asset_dir = export_tmp.path().join(world.to_string());
        std::fs::create_dir_all(&asset_dir).unwrap();
        std::fs::write(asset_dir.join(asset_id.to_string()), b"BYTES").unwrap();

        let mut data = sample_data(world);
        data.assets
            .push(crate::data::world_bundle::ExportedAssetRow {
                id: asset_id,
                original_name: "a.png".to_string(),
                content_type: "image/png".to_string(),
                byte_size: 5,
                created_by_username: None,
                created_at: 0,
                version: 1,
            });
        data.manifest.row_counts.insert("assets".to_string(), 1);

        let bytes = write_bundle(&data, export_tmp.path(), Vec::new()).unwrap();
        let tar_path = export_tmp.path().join("bundle.tar");
        std::fs::write(&tar_path, &bytes).unwrap();

        let import_tmp = tempfile::tempdir().unwrap();
        let imported = read_bundle(&tar_path, import_tmp.path()).unwrap();

        assert_eq!(imported.manifest.world_id, world);
        assert_eq!(imported.documents.len(), 1);
        assert_eq!(imported.staged_assets.len(), 1);
        let (staged_id, staged_path) = &imported.staged_assets[0];
        assert_eq!(*staged_id, asset_id);
        assert_eq!(std::fs::read(staged_path).unwrap(), b"BYTES");
        // Staged file lives beside where the final `<id>` path will live.
        assert_eq!(
            staged_path.parent().unwrap(),
            import_tmp.path().join(world.to_string())
        );
    }

    #[test]
    fn read_bundle_rejects_row_count_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let world = Uuid::from_u128(66);
        let mut data = sample_data(world);
        // Lie about the document count the manifest promises.
        data.manifest.row_counts.insert("documents".to_string(), 2);

        let bytes = write_bundle(&data, tmp.path(), Vec::new()).unwrap();
        let tar_path = tmp.path().join("bad.tar");
        std::fs::write(&tar_path, &bytes).unwrap();

        let import_tmp = tempfile::tempdir().unwrap();
        let err = read_bundle(&tar_path, import_tmp.path()).unwrap_err();
        assert!(matches!(
            err,
            WorldBundleError::RowCountMismatch {
                table,
                expected: 2,
                actual: 1
            } if table == "documents"
        ));
    }

    #[test]
    fn read_bundle_rejects_unsupported_schema_version() {
        let tmp = tempfile::tempdir().unwrap();
        let world = Uuid::from_u128(77);
        let mut data = sample_data(world);
        data.manifest.schema_version = BUNDLE_SCHEMA_VERSION + 1;

        let bytes = write_bundle(&data, tmp.path(), Vec::new()).unwrap();
        let tar_path = tmp.path().join("future.tar");
        std::fs::write(&tar_path, &bytes).unwrap();

        let import_tmp = tempfile::tempdir().unwrap();
        let err = read_bundle(&tar_path, import_tmp.path()).unwrap_err();
        assert!(matches!(
            err,
            WorldBundleError::UnsupportedSchemaVersion(v) if v == BUNDLE_SCHEMA_VERSION + 1
        ));
    }

    #[test]
    fn read_bundle_rejects_archive_not_starting_with_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "rows/documents.jsonl", &b"{}\n"[..])
            .unwrap();
        let bytes = builder.into_inner().unwrap();
        let tar_path = tmp.path().join("wrong_order.tar");
        std::fs::write(&tar_path, &bytes).unwrap();

        let import_tmp = tempfile::tempdir().unwrap();
        let err = read_bundle(&tar_path, import_tmp.path()).unwrap_err();
        assert!(matches!(err, WorldBundleError::Malformed(_)));
    }

    #[test]
    fn read_bundle_rejects_duplicate_asset_entry_and_cleans_up_staged_files() {
        let tmp = tempfile::tempdir().unwrap();
        let world = Uuid::from_u128(88);
        let asset_id = Uuid::from_u128(888);

        let mut row_counts = std::collections::BTreeMap::new();
        for table in [
            "documents",
            "world_events",
            "world_members",
            "world_invites",
            "explored_fog",
            "settings",
        ] {
            row_counts.insert(table.to_string(), 0);
        }
        row_counts.insert("assets".to_string(), 1);
        let manifest = crate::data::world_bundle::BundleManifest {
            schema_version: BUNDLE_SCHEMA_VERSION,
            world_id: world,
            world_name: "Dup".to_string(),
            world_seq: 0,
            world_created_at: 0,
            world_updated_at: 0,
            exported_at_unix_ms: 0,
            row_counts,
        };

        let mut builder = tar::Builder::new(Vec::new());
        append_bytes(
            &mut builder,
            "manifest.json",
            &serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        for table in [
            "documents",
            "world_events",
            "world_members",
            "world_invites",
            "assets",
            "explored_fog",
            "settings",
        ] {
            append_bytes(&mut builder, &format!("rows/{table}.jsonl"), b"").unwrap();
        }
        // Two entries sharing the same asset id.
        append_bytes(&mut builder, &format!("assets/{asset_id}"), b"FIRST").unwrap();
        append_bytes(&mut builder, &format!("assets/{asset_id}"), b"SECOND").unwrap();
        let bytes = builder.into_inner().unwrap();
        let tar_path = tmp.path().join("dup.tar");
        std::fs::write(&tar_path, &bytes).unwrap();

        let import_tmp = tempfile::tempdir().unwrap();
        let err = read_bundle(&tar_path, import_tmp.path()).unwrap_err();
        assert!(
            matches!(&err, WorldBundleError::Malformed(m) if m.contains("duplicate asset entry")),
            "unexpected error: {err:?}"
        );

        // The first entry's staged file must be cleaned up, not orphaned.
        let world_asset_dir = import_tmp.path().join(world.to_string());
        let leftover: Vec<_> = std::fs::read_dir(&world_asset_dir).unwrap().collect();
        assert!(
            leftover.is_empty(),
            "duplicate-id rejection must remove any already-staged file"
        );
    }
}
