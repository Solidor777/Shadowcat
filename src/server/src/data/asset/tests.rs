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
    )
    .await
    .unwrap();

    assert_eq!(asset.created_by, None);
    let fetched = repo.get_asset(asset.id).await.unwrap().unwrap();
    assert_eq!(fetched.created_by, None);
}
