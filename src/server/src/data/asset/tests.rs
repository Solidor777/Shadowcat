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
            provenance: crate::data::asset::Provenance::Uploaded,
            retain_originals: true,
        },
        1_234,
        &test_trash(),
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
            provenance: crate::data::asset::Provenance::Uploaded,
            retain_originals: true,
        },
        1_234,
        &test_trash(),
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
            provenance: crate::data::asset::Provenance::Uploaded,
            retain_originals: true,
        },
        5_678,
        &test_trash(),
    )
    .await
    .unwrap();

    assert_eq!(asset.created_by, None);
    let fetched = repo.get_asset(asset.id).await.unwrap().unwrap();
    assert_eq!(fetched.created_by, None);
}

#[tokio::test]
async fn move_asset_files_removes_stale_sheet_siblings_absent_at_the_staged_stem() {
    // Replacing an animated asset with a non-animated one: processing yields no new sheet
    // (no `.sheet.*` files at the staged stem), so the stale pair at the final stem must be
    // removed, never left to be served or exported.
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    let final_path = dir.path().join("final");
    tokio::fs::write(&staged, b"new-still-bytes").await.unwrap();
    tokio::fs::write(&final_path, b"old-animated-bytes")
        .await
        .unwrap();
    for suffix in [".sheet.webp", ".sheet.json"] {
        let mut os = final_path.as_os_str().to_owned();
        os.push(suffix);
        tokio::fs::write(std::path::PathBuf::from(os), b"stale")
            .await
            .unwrap();
    }

    move_asset_files(&staged, &final_path, &test_trash())
        .await
        .unwrap();

    for p in crate::data::asset::process::sibling_paths(&final_path) {
        assert!(!p.exists(), "stale sibling left behind: {}", p.display());
    }
    assert!(final_path.exists());
}

/// `discard_asset_files` moves the canonical file and every `sibling_paths` derivative into the
/// trash, reports the canonical first, and leaves nothing behind. Replacing the move with an
/// outright removal leaves the trash empty; dropping the sibling loop leaves the derivatives on
/// disk and out of the trash; reordering reports a sibling before the canonical.
#[tokio::test]
async fn discard_asset_files_trashes_the_canonical_and_every_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let canonical = dir.path().join("asset-id");
    std::fs::write(&canonical, b"canonical").unwrap();
    let siblings = crate::data::asset::process::sibling_paths(&canonical);
    for sibling in &siblings {
        std::fs::write(sibling, b"derivative").unwrap();
    }
    let trash = crate::trash::Trash::new(
        dir.path().join("trash"),
        crate::trash::TrashRetention::default(),
    );

    let results = discard_asset_files(&trash, &canonical).await;

    assert_eq!(results.len(), 1 + siblings.len());
    assert_eq!(results[0].0, canonical);
    assert!(results.iter().all(|(_, outcome)| outcome.is_ok()));
    let mut expected: Vec<String> = std::iter::once(&canonical)
        .chain(siblings.iter())
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    expected.sort();
    let mut trashed: Vec<String> = trash
        .list()
        .unwrap()
        .iter()
        .map(|e| {
            e.manifest
                .original_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    trashed.sort();
    assert_eq!(trashed, expected);
    assert!(!canonical.exists());
    assert!(siblings.iter().all(|p| !p.exists()));
}

/// A trash that removes outright: these tests never inspect trashed files.
fn test_trash() -> crate::trash::Trash {
    crate::trash::Trash::new(
        std::env::temp_dir().join("shadowcat-test-trash"),
        crate::trash::TrashRetention::Disabled,
    )
}
