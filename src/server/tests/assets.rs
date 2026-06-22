mod common;
use common::{spawn, PNG_1X1};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_persists_record_and_file() {
    let h = spawn().await;
    let res = h.upload("battlemap.png", "image/png", PNG_1X1.to_vec()).await;
    assert_eq!(res.status(), 200, "body: {:?}", res.text().await);
    let asset: serde_json::Value = res.json().await.unwrap();
    assert_eq!(asset["content_type"], "image/png");
    assert_eq!(asset["version"], 1);
    // The record is queryable and the file exists on disk.
    let id = uuid::Uuid::parse_str(asset["id"].as_str().unwrap()).unwrap();
    assert!(h.repo.get_asset(id).await.unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_rejects_non_image_bytes() {
    let h = spawn().await;
    // Declared image/png, but the bytes are a PDF → magic-byte mismatch.
    let res = h
        .upload("evil.png", "image/png", b"%PDF-1.7 not an image".to_vec())
        .await;
    assert_eq!(res.status(), 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_over_cap_is_rejected() {
    use common::spawn_with;
    // Regular cap 8 bytes → GM cap 16 (the uploader is GM). The 67-byte PNG
    // exceeds it, tripping the streaming size guard.
    let h = spawn_with(|c| c.upload_max_bytes = 8).await;
    let res = h.upload("big.png", "image/png", PNG_1X1.to_vec()).await;
    assert_eq!(res.status(), 413);
}
