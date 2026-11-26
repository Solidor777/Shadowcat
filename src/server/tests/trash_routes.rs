//! Where the HTTP routes put what they discard: every file or directory a route removes moves into
//! the trash instead of being destroyed, so a mistaken delete can be recovered by hand.
//!
//! Each test gives the server a trash directory of its own and reads it back through
//! `Trash::list`; the harness default is a trash that removes outright.

use shadowcat::config::Config;
use shadowcat::trash::{Trash, TrashRetention, TrashedKind};
use shadowcat_test_support as common;
use shadowcat_test_support::Harness;

/// A pass-through (non-image) payload of exactly `len` bytes.
fn payload(len: usize) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7 ".to_vec();
    bytes.resize(len, 7);
    bytes
}

/// A server whose trash keeps what it receives, in a directory under `dir`, plus a handle that
/// reads the same trash.
async fn spawn_keeping_trash(
    dir: &tempfile::TempDir,
    tune: impl FnOnce(&mut Config),
) -> (Harness, Trash) {
    let trash_dir = dir.path().join("trash");
    let configured = trash_dir.to_string_lossy().into_owned();
    let h = common::spawn_with(move |c| {
        c.trash_retention = TrashRetention::default();
        c.trash_dir = Some(configured);
        tune(c);
    })
    .await;
    let trash = Trash::new(trash_dir, TrashRetention::default());
    (h, trash)
}

/// Deleting an asset moves its canonical file into the trash under its own name. Fails if the
/// route removes the file outright.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_an_asset_moves_its_file_to_the_trash() {
    let dir = tempfile::tempdir().unwrap();
    let (h, trash) = spawn_keeping_trash(&dir, |_| {}).await;
    let asset: serde_json::Value = h
        .upload("held.pdf", "application/pdf", payload(4096))
        .await
        .json()
        .await
        .unwrap();
    let id = asset["id"].as_str().unwrap().to_string();
    let path = h.assets_dir.join(h.world.to_string()).join(&id);
    assert!(path.exists());

    let res = h
        .client
        .delete(format!("http://{}/api/assets/{id}", h.addr))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
    assert!(!path.exists(), "the name is freed");

    let entries = trash.list().unwrap();
    assert_eq!(
        entries.len(),
        1,
        "one canonical, no siblings for a pass-through file"
    );
    assert_eq!(entries[0].manifest.kind, TrashedKind::File);
    assert_eq!(
        entries[0].manifest.original_path.file_name().unwrap(),
        id.as_str()
    );
    assert_eq!(entries[0].size_bytes, 4096);
}

/// An upload that exceeds the size cap moves its partial staging file into the trash and leaves
/// nothing beside the asset. Fails if the partial file is removed outright or left in place.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_oversized_upload_moves_its_partial_file_to_the_trash() {
    let dir = tempfile::tempdir().unwrap();
    let (h, trash) = spawn_keeping_trash(&dir, |c| {
        c.upload_max_bytes = 100;
        c.upload_max_bytes_gm = Some(100);
    })
    .await;

    let res = h.upload("big.pdf", "application/pdf", payload(5000)).await;
    assert_eq!(res.status(), 413);

    let world_dir = h.assets_dir.join(h.world.to_string());
    let left: Vec<_> = std::fs::read_dir(&world_dir)
        .map(|rd| rd.map(|e| e.unwrap().file_name()).collect())
        .unwrap_or_default();
    assert!(
        left.is_empty(),
        "no staging file stays in the world directory: {left:?}"
    );

    let entries = trash.list().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].manifest.kind, TrashedKind::File);
    assert_eq!(
        entries[0].manifest.original_path.extension().unwrap(),
        "tmp",
        "the trashed file is the upload's staging file"
    );
}

/// Deleting a world moves its whole asset directory into the trash. Fails if the route removes the
/// directory outright.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_a_world_moves_its_asset_directory_to_the_trash() {
    let dir = tempfile::tempdir().unwrap();
    let (h, trash) = spawn_keeping_trash(&dir, |_| {}).await;
    let res = h.upload("held.pdf", "application/pdf", payload(2048)).await;
    assert!(res.status().is_success());
    let world_dir = h.assets_dir.join(h.world.to_string());
    assert!(world_dir.exists());

    let res = h
        .client
        .delete(format!("http://{}/api/worlds/{}", h.addr, h.world))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
    assert!(!world_dir.exists());

    let entries = trash.list().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].manifest.kind, TrashedKind::Dir);
    assert_eq!(
        entries[0].manifest.original_path.file_name().unwrap(),
        h.world.to_string().as_str()
    );
    assert_eq!(entries[0].size_bytes, 2048);
}
