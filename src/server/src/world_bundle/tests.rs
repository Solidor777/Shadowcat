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
    use crate::data::world_bundle::{BundleManifest, ExportedDocumentRow, BUNDLE_SCHEMA_VERSION};
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
    let manifest: crate::data::world_bundle::BundleManifest = serde_json::from_slice(&buf).unwrap();
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
            folder_id: None,
            tags: vec![],
            derived_tags: vec![],
            meta: crate::data::asset::AssetMeta::unprocessed("image/png", 7),
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
            folder_id: None,
            tags: vec![],
            derived_tags: vec![],
            meta: crate::data::asset::AssetMeta::unprocessed("image/png", 7),
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
            folder_id: None,
            tags: vec![],
            derived_tags: vec![],
            meta: crate::data::asset::AssetMeta::unprocessed("image/png", 7),
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

#[test]
fn bundle_round_trips_asset_siblings_and_rejects_an_unknown_suffix() {
    let tmp = tempfile::tempdir().unwrap();
    let world = Uuid::from_u128(7);
    let asset_id = Uuid::from_u128(100);
    let asset_dir = tmp.path().join(world.to_string());
    std::fs::create_dir_all(&asset_dir).unwrap();
    std::fs::write(asset_dir.join(asset_id.to_string()), b"WEBP").unwrap();
    std::fs::write(asset_dir.join(format!("{asset_id}.orig")), b"PNGORIG").unwrap();
    std::fs::write(asset_dir.join(format!("{asset_id}.thumb.webp")), b"THUMB").unwrap();
    // No preview on disk: only the siblings that exist travel.

    let mut data = sample_data(world);
    data.assets
        .push(crate::data::world_bundle::ExportedAssetRow {
            id: asset_id,
            original_name: "token.png".to_string(),
            content_type: "image/webp".to_string(),
            byte_size: 4,
            created_by_username: None,
            created_at: 0,
            version: 1,
            folder_id: None,
            tags: vec!["hero".into()],
            derived_tags: vec!["image".into(), "webp".into()],
            meta: crate::data::asset::AssetMeta {
                original_retained: true,
                original_content_type: "image/png".into(),
                original_byte_size: 7,
                ..crate::data::asset::AssetMeta::default()
            },
        });
    data.manifest.row_counts.insert("assets".to_string(), 1);
    let bytes = write_bundle(&data, tmp.path(), Vec::new()).unwrap();

    let mut archive = tar::Archive::new(bytes.as_slice());
    let names: Vec<String> = archive
        .entries()
        .unwrap()
        .map(|e| e.unwrap().path().unwrap().to_string_lossy().to_string())
        .filter(|n| n.starts_with("assets/"))
        .collect();
    assert_eq!(
        names,
        vec![
            format!("assets/{asset_id}"),
            format!("assets/{asset_id}.orig"),
            format!("assets/{asset_id}.thumb.webp"),
        ]
    );

    let tar_path = tmp.path().join("bundle.tar");
    std::fs::write(&tar_path, &bytes).unwrap();
    let import_root = tempfile::tempdir().unwrap();
    let imported = read_bundle(&tar_path, import_root.path()).unwrap();
    assert_eq!(imported.staged_assets.len(), 1);
    assert_eq!(imported.assets[0].tags, vec!["hero".to_string()]);
    assert!(imported.assets[0].meta.original_retained);
    let mut siblings: Vec<(String, Vec<u8>)> = imported
        .staged_siblings
        .iter()
        .map(|s| (s.suffix.clone(), std::fs::read(&s.staged).unwrap()))
        .collect();
    siblings.sort();
    assert_eq!(
        siblings,
        vec![
            (".orig".to_string(), b"PNGORIG".to_vec()),
            (".thumb.webp".to_string(), b"THUMB".to_vec()),
        ]
    );
    assert!(imported
        .staged_siblings
        .iter()
        .all(|s| s.asset_id == asset_id));

    // An `assets/<id>.<anything else>` entry is malformed, and every staged
    // file is cleaned up on the way out.
    let mut builder = tar::Builder::new(Vec::new());
    let manifest = serde_json::to_vec(&data.manifest).unwrap();
    let mut header = tar::Header::new_gnu();
    header.set_size(manifest.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, "manifest.json", manifest.as_slice())
        .unwrap();
    let mut header = tar::Header::new_gnu();
    header.set_size(3);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, format!("assets/{asset_id}.exe"), &b"bad"[..])
        .unwrap();
    let bad = builder.into_inner().unwrap();
    let bad_path = tmp.path().join("bad.tar");
    std::fs::write(&bad_path, &bad).unwrap();
    let bad_root = tempfile::tempdir().unwrap();
    let err = read_bundle(&bad_path, bad_root.path()).unwrap_err();
    assert!(
        matches!(err, WorldBundleError::Malformed(m) if m.contains("unknown asset sibling suffix"))
    );
    let leftovers = std::fs::read_dir(bad_root.path().join(world.to_string()))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0);
}
