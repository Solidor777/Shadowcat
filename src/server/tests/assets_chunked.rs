//! Chunked upload sessions over the live HTTP surface: a 16 MiB + 1 byte
//! pass-through blob pushed in three chunks, the offset/ownership/size gates
//! around it, completion into a real asset (folder + explicit tags echoed,
//! derived tags present), abort, and the rate-slot refund on abort.

use common::{spawn_with, Harness, PNG_1X1};
use shadowcat_test_support as common;
use uuid::Uuid;

const CHUNK: usize = 8 * 1024 * 1024;

async fn create_session(
    h: &Harness,
    name: &str,
    content_type: &str,
    byte_size: usize,
    folder_id: Option<Uuid>,
    tags: &[&str],
) -> reqwest::Response {
    h.client
        .post(format!(
            "http://{}/api/worlds/{}/assets/uploads",
            h.addr, h.world
        ))
        .json(&serde_json::json!({
            "name": name,
            "content_type": content_type,
            "byte_size": byte_size,
            "folder_id": folder_id,
            "tags": tags,
        }))
        .send()
        .await
        .unwrap()
}

async fn put_chunk(h: &Harness, id: &str, offset: usize, body: Vec<u8>) -> reqwest::StatusCode {
    h.client
        .put(format!(
            "http://{}/api/assets/uploads/{}/{}",
            h.addr, id, offset
        ))
        .body(body)
        .send()
        .await
        .unwrap()
        .status()
}

async fn complete(h: &Harness, id: &str) -> reqwest::Response {
    h.client
        .post(format!(
            "http://{}/api/assets/uploads/{}/complete",
            h.addr, id
        ))
        .send()
        .await
        .unwrap()
}

/// Creates an `asset_folder` document named `name` at the world root.
async fn create_folder(h: &Harness, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    let res = h
        .client
        .post(format!(
            "http://{}/api/worlds/{}/documents",
            h.addr, h.world
        ))
        .json(&serde_json::json!({
            "id": id,
            "scope": { "kind": "world", "world_id": h.world },
            "doc_type": "asset_folder",
            "schema_version": 1,
            "name": name,
            "engine": { "sort": 0 },
            "system": {},
            "permissions": { "default": "observer", "users": {}, "property_overrides": {} },
            "created_at": 0,
            "updated_at": 0,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "folder create: {:?}", res.text().await);
    id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_chunk_upload_completes_into_a_filed_tagged_asset() {
    let h = spawn_with(|c| c.upload_max_bytes_gm = Some(64 * 1024 * 1024)).await;
    let folder = create_folder(&h, "Handouts").await;
    let total = 2 * CHUNK + 1;

    let res = create_session(
        &h,
        "big.bin",
        "application/octet-stream",
        total,
        Some(folder),
        &["session-one", " hero "],
    )
    .await;
    assert_eq!(res.status(), 201, "{:?}", res.text().await);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["chunk_size"], CHUNK as u64);
    let id = body["upload_id"].as_str().unwrap().to_string();

    // Out-of-order first chunk is refused; a partial complete is refused.
    assert_eq!(put_chunk(&h, &id, CHUNK, vec![0u8; CHUNK]).await, 409);
    assert_eq!(complete(&h, &id).await.status(), 409);

    assert_eq!(put_chunk(&h, &id, 0, vec![0u8; CHUNK]).await, 204);
    // Re-sending an already-accepted chunk is not a resume.
    assert_eq!(put_chunk(&h, &id, 0, vec![0u8; CHUNK]).await, 409);
    assert_eq!(put_chunk(&h, &id, CHUNK, vec![0u8; CHUNK]).await, 204);
    assert_eq!(put_chunk(&h, &id, 2 * CHUNK, vec![7u8]).await, 204);

    let res = complete(&h, &id).await;
    assert_eq!(res.status(), 200, "{:?}", res.text().await);
    let asset: serde_json::Value = res.json().await.unwrap();
    assert_eq!(asset["content_type"], "application/octet-stream");
    assert_eq!(asset["byte_size"], total as u64);
    assert_eq!(asset["original_name"], "big.bin");
    assert_eq!(asset["folder_id"], folder.to_string());
    assert_eq!(
        asset["tags"],
        serde_json::json!(["session-one", "hero"]),
        "explicit tags trimmed, in order"
    );
    let derived = asset["derived_tags"].as_array().unwrap();
    for expected in ["other", "uploaded", "Handouts"] {
        assert!(
            derived.iter().any(|t| t == expected),
            "missing {expected} in {derived:?}"
        );
    }
    // The bytes are on disk under the stable id, exactly as sent.
    let asset_id = asset["id"].as_str().unwrap();
    let stored = std::fs::read(h.assets_dir.join(h.world.to_string()).join(asset_id)).unwrap();
    assert_eq!(stored.len(), total);
    assert_eq!(stored[total - 1], 7);
    assert!(stored[..total - 1].iter().all(|&b| b == 0));
    // Session is gone.
    assert_eq!(complete(&h, &id).await.status(), 404);
    // No staging residue.
    let residue = std::fs::read_dir(h.assets_dir.join(h.world.to_string()))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        })
        .count();
    assert_eq!(residue, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chunked_png_is_converted_like_a_single_shot_upload() {
    let h = spawn_with(|c| c.upload_max_bytes_gm = Some(64 * 1024 * 1024)).await;
    let res = create_session(&h, "tiny.png", "image/png", PNG_1X1.len(), None, &[]).await;
    assert_eq!(res.status(), 201);
    let body: serde_json::Value = res.json().await.unwrap();
    let id = body["upload_id"].as_str().unwrap().to_string();
    assert_eq!(put_chunk(&h, &id, 0, PNG_1X1.to_vec()).await, 204);
    let res = complete(&h, &id).await;
    assert_eq!(res.status(), 200, "{:?}", res.text().await);
    let asset: serde_json::Value = res.json().await.unwrap();
    assert_eq!(asset["content_type"], "image/webp");
    assert_eq!(asset["original_content_type"], "image/png");
    assert_eq!(asset["folder_id"], serde_json::Value::Null);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_gates_size_ownership_folder_and_overflow() {
    let h = spawn_with(|c| c.upload_max_bytes_gm = Some(1024)).await;

    // Over the GM cap at create time → 413, before any bytes.
    assert_eq!(
        create_session(&h, "x.bin", "application/octet-stream", 2048, None, &[])
            .await
            .status(),
        413
    );
    // A folder that is not an asset_folder → 422.
    assert_eq!(
        create_session(
            &h,
            "x.bin",
            "application/octet-stream",
            10,
            Some(Uuid::new_v4()),
            &[]
        )
        .await
        .status(),
        422
    );

    let res = create_session(&h, "x.bin", "application/octet-stream", 10, None, &[]).await;
    assert_eq!(res.status(), 201);
    let body: serde_json::Value = res.json().await.unwrap();
    let id = body["upload_id"].as_str().unwrap().to_string();

    // Another user cannot touch the GM's session.
    let (_player, cookie) = h.add_player("intruder").await;
    let other = reqwest::Client::new();
    let res = other
        .put(format!("http://{}/api/assets/uploads/{}/0", h.addr, id))
        .header("cookie", &cookie)
        .body(vec![1u8; 4])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
    let res = other
        .delete(format!("http://{}/api/assets/uploads/{}", h.addr, id))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);

    // A chunk past the declared size aborts the session.
    assert_eq!(put_chunk(&h, &id, 0, vec![1u8; 4]).await, 204);
    assert_eq!(put_chunk(&h, &id, 4, vec![1u8; 7]).await, 413);
    assert_eq!(put_chunk(&h, &id, 4, vec![1u8; 6]).await, 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abort_removes_the_staging_file_and_refunds_the_rate_slot() {
    // One upload per minute for the GM: the aborted session's slot must come
    // back for the real upload to fit.
    let h = spawn_with(|c| {
        c.upload_rate_per_min = 1;
        c.upload_rate_per_min_gm = Some(1);
    })
    .await;
    let res = create_session(&h, "x.bin", "application/octet-stream", 10, None, &[]).await;
    assert_eq!(res.status(), 201);
    let body: serde_json::Value = res.json().await.unwrap();
    let id = body["upload_id"].as_str().unwrap().to_string();
    assert_eq!(put_chunk(&h, &id, 0, vec![1u8; 4]).await, 204);
    // Second session while the first holds the only slot → 429.
    assert_eq!(
        create_session(&h, "y.bin", "application/octet-stream", 10, None, &[])
            .await
            .status(),
        429
    );

    let res = h
        .client
        .delete(format!("http://{}/api/assets/uploads/{}", h.addr, id))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
    assert_eq!(complete(&h, &id).await.status(), 404);
    let residue = std::fs::read_dir(h.assets_dir.join(h.world.to_string()))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(residue, 0, "staging file removed");

    // The slot is back: a single-shot upload succeeds.
    let res = h.upload("ok.png", "image/png", PNG_1X1.to_vec()).await;
    assert_eq!(res.status(), 200);
}
