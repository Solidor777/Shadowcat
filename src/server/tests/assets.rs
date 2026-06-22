mod common;
use common::{spawn, PNG_1X1};
use futures_util::StreamExt;
use shadowcat::data::repository::Repository;

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serve_returns_bytes_then_304_on_revalidation() {
    let h = spawn().await;
    let asset: serde_json::Value = h
        .upload("m.png", "image/png", PNG_1X1.to_vec())
        .await
        .json()
        .await
        .unwrap();
    let id = asset["id"].as_str().unwrap();
    let url = format!("http://{}/api/assets/{}", h.addr, id);

    let res = h.client.get(&url).send().await.unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["content-type"], "image/png");
    let etag = res.headers()["etag"].to_str().unwrap().to_string();
    assert_eq!(res.bytes().await.unwrap().as_ref(), PNG_1X1);

    // Conditional GET with the matching ETag → 304.
    let res2 = h
        .client
        .get(&url)
        .header("if-none-match", &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(res2.status(), 304);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serve_denies_non_member() {
    use shadowcat::data::asset::Asset;
    let h = spawn().await;
    // A world the seeded user is NOT a member of (create_world has no owner).
    let other = h.repo.create_world("B", 0).await.unwrap();
    let id = uuid::Uuid::from_u128(0xB0B);
    h.repo
        .insert_asset(&Asset {
            id,
            world_id: other.id,
            storage_key: format!("{}/{}", other.id, id),
            original_name: "x.png".into(),
            content_type: "image/png".into(),
            byte_size: 1,
            created_by: h.user,
            created_at: 0,
            version: 1,
        })
        .await
        .unwrap();
    let res = h
        .client
        .get(format!("http://{}/api/assets/{}", h.addr, id))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replace_swaps_bytes_bumps_version_and_broadcasts() {
    use common::drain_until_type;
    let h = spawn().await;
    let asset: serde_json::Value = h
        .upload("m.png", "image/png", PNG_1X1.to_vec())
        .await
        .json()
        .await
        .unwrap();
    let id = asset["id"].as_str().unwrap().to_string();

    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome

    let seq_before = h.repo.get_world(h.world).await.unwrap().unwrap().seq;
    // Replace with a GIF — content_type changes, version bumps to 2.
    let res = h
        .client
        .post(format!("http://{}/api/assets/{}/replace", h.addr, id))
        .multipart(
            reqwest::multipart::Form::new().part(
                "file",
                reqwest::multipart::Part::bytes(b"GIF89a\x01\x00\x01\x00\x00\x00\x00".to_vec())
                    .file_name("m.gif")
                    .mime_str("image/gif")
                    .unwrap(),
            ),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let updated: serde_json::Value = res.json().await.unwrap();
    assert_eq!(updated["version"], 2);
    assert_eq!(updated["content_type"], "image/gif");

    // Out-of-band: no world seq was consumed.
    assert_eq!(
        h.repo.get_world(h.world).await.unwrap().unwrap().seq,
        seq_before
    );

    // The room broadcast an asset_changed{replaced}.
    let frame = drain_until_type(&mut ws, "asset_changed").await;
    assert_eq!(frame["uuid"], id);
    assert_eq!(frame["op"], "replaced");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_removes_record_and_file_and_broadcasts() {
    use common::drain_until_type;
    let h = spawn().await;
    let asset: serde_json::Value = h
        .upload("m.png", "image/png", PNG_1X1.to_vec())
        .await
        .json()
        .await
        .unwrap();
    let id = asset["id"].as_str().unwrap().to_string();
    let uuid = uuid::Uuid::parse_str(&id).unwrap();

    let mut ws = h.connect().await;
    let _ = ws.next().await;

    let res = h
        .client
        .delete(format!("http://{}/api/assets/{}", h.addr, id))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
    assert!(h.repo.get_asset(uuid).await.unwrap().is_none());

    let frame = drain_until_type(&mut ws, "asset_changed").await;
    assert_eq!(frame["op"], "deleted");
}
