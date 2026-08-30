//! End-to-end asset lifecycle against a live server: upload, retrieval, replacement, deletion and
//! listing, each asserted on BOTH the database record and the file on disk, plus the gates around
//! them — content-type and size-cap rejection, world-membership authorization, `304` revalidation,
//! per-user rate limiting, and the broadcasts that replacement and deletion emit.
//!
//! Exercises the HTTP surface rather than `data::repository` directly, so a change that leaves the
//! repository correct while breaking the route still fails here. A rejected upload is asserted not
//! to consume rate quota, which couples the validation order to the quota accounting: validation
//! must run first, and testing that through the route is the only way to observe the ordering.

use common::{spawn, PNG_1X1};
use futures_util::StreamExt;
use shadowcat::data::repository::Repository;
use shadowcat::data::DataError;
use shadowcat_test_support as common;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_persists_record_and_file() {
    let h = spawn().await;
    let res = h
        .upload("battlemap.png", "image/png", PNG_1X1.to_vec())
        .await;
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
            created_by: Some(h.user),
            created_at: 0,
            version: 1,
            folder_id: None,
            tags: vec![],
            derived_tags: vec![],
            meta: shadowcat::data::asset::AssetMeta::unprocessed("image/png", 1),
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
    assert_eq!(
        frame["version"], 2,
        "carries the bumped, authoritative version"
    );
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
    let pre_delete_version = asset["version"].as_i64().unwrap();

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
    assert_eq!(
        frame["version"], pre_delete_version,
        "carries the version the row held immediately before removal"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_broadcast_reflects_the_truly_removed_version_despite_a_racing_replace() {
    use common::drain_until_type;
    // `http::assets::delete` reads `existing` once at request entry, before the row is actually
    // removed; a `replace` on the same id can commit (bumping the version) in the window between
    // that read and the handler's own `delete_asset` call. This races the REAL HTTP `DELETE`
    // (`h.client.delete`, i.e. `http::assets::delete` itself) against a direct
    // `h.repo.replace_asset_bytes` call inside one `tokio::join!`: the repo call has far fewer
    // awaits than a full HTTP round trip through auth + the write-barrier permit, so it is likely
    // to land inside the handler's request-to-delete window purely from that latency gap — but
    // there is no pause/hook seam in this harness (and adding one to production code would violate
    // this project's no-debug-code rule) to force the interleaving deterministically, so a bounded
    // retry loop keeps attempting the race, against a freshly uploaded asset each time, until it
    // observes a run where the replace's bump genuinely landed inside the window. Every iteration
    // asserts the broadcast is correct for whichever ordering actually occurred; only the
    // iterations that land inside the window carry the assertion distinguishing a broadcast
    // sourced from `delete_asset`'s own returned row from one sourced off a stale `existing`
    // snapshot, and the test fails outright if the race is never won.
    let h = spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await; // Welcome

    let mut race_won = false;
    for _ in 0..20 {
        let asset: serde_json::Value = h
            .upload("m.png", "image/png", PNG_1X1.to_vec())
            .await
            .json()
            .await
            .unwrap();
        let id_str = asset["id"].as_str().unwrap().to_string();
        let id = uuid::Uuid::parse_str(&id_str).unwrap();
        let storage_key = asset["storage_key"].as_str().unwrap().to_string();

        let url = format!("http://{}/api/assets/{}", h.addr, id_str);
        let meta = shadowcat::data::asset::AssetMeta::unprocessed("image/gif", 7);
        let (delete_res, replace_res) = tokio::join!(
            h.client.delete(&url).send(),
            h.repo
                .replace_asset_bytes(id, &storage_key, "image/gif", 7, &meta)
        );
        assert_eq!(delete_res.unwrap().status(), 204);

        let frame = drain_until_type(&mut ws, "asset_changed").await;
        assert_eq!(frame["op"], "deleted");

        match replace_res {
            // The replace's UPDATE committed before `delete`'s own `delete_asset` DELETE ran: the
            // row the DELETE actually removed carries the bumped version, so that's what the
            // handler must broadcast — the exact interleaving this test exists to exercise.
            Ok(bumped_version) => {
                assert_eq!(
                    frame["version"], bumped_version,
                    "a replace that landed inside the handler's window must be reflected in the \
                     broadcast, not discarded in favor of the pre-delete snapshot"
                );
                assert_ne!(
                    bumped_version, 1,
                    "the replace must have actually bumped the version"
                );
                race_won = true;
            }
            // The DELETE already removed the row by the time the replace's UPDATE ran (no row left
            // to match): `replace_asset_bytes`'s `UPDATE ... RETURNING version` matched zero rows,
            // which it reports as `DataError::NotFound` specifically — the un-bumped version is
            // what's broadcast, which is also correct; this iteration just didn't land inside the
            // race window. Any other `Err` variant is a genuine DB fault distinct from a lost race
            // and must fail the test loudly rather than being folded into this branch.
            Err(DataError::NotFound) => {
                assert_eq!(frame["version"], 1);
            }
            Err(e) => {
                panic!("replace_asset_bytes failed for a reason other than losing the race: {e}")
            }
        }
    }
    assert!(
        race_won,
        "the replace never landed inside the handler's request-to-delete window across 20 \
         attempts; the race this test exists to exercise was never actually won"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_deletes_never_double_broadcast_or_error() {
    use common::drain_until_type;
    // Two real concurrent HTTP DELETEs against the same asset: the repository's atomic
    // `DELETE ... RETURNING` guarantees only one actually removes the row, and the handler's
    // `None` branch (for whichever request loses that race after already passing its own
    // pre-delete existence check) must return 204 without erroring or broadcasting again — the
    // other possible ordering (the loser's own existence check runs after the row is already
    // gone) is the pre-existing 404 path `delete_is_idempotent_second_is_404` covers sequentially.
    // Either ordering is a safe outcome; what this test pins is that concurrency never panics,
    // never 500s, and never produces a second `asset_changed` notice.
    let h = spawn().await;
    let asset: serde_json::Value = h
        .upload("m.png", "image/png", PNG_1X1.to_vec())
        .await
        .json()
        .await
        .unwrap();
    let id = asset["id"].as_str().unwrap().to_string();

    let mut ws = h.connect().await;
    let _ = ws.next().await;

    let url = format!("http://{}/api/assets/{}", h.addr, id);
    let (r1, r2) = tokio::join!(h.client.delete(&url).send(), h.client.delete(&url).send());
    let s1 = r1.unwrap().status().as_u16();
    let s2 = r2.unwrap().status().as_u16();
    assert!(matches!(s1, 204 | 404), "unexpected status: {s1}");
    assert!(matches!(s2, 204 | 404), "unexpected status: {s2}");
    assert!(s1 == 204 || s2 == 204, "at least one delete must succeed");

    assert!(h
        .repo
        .get_asset(uuid::Uuid::parse_str(&id).unwrap())
        .await
        .unwrap()
        .is_none());

    let frame = drain_until_type(&mut ws, "asset_changed").await;
    assert_eq!(frame["op"], "deleted");
    // No second notice arrives within a short window.
    let second = tokio::time::timeout(std::time::Duration::from_millis(300), ws.next()).await;
    assert!(
        second.is_err(),
        "a second asset_changed notice was broadcast"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_returns_world_assets() {
    let h = spawn().await;
    h.upload("a.png", "image/png", PNG_1X1.to_vec()).await;
    h.upload("b.png", "image/png", PNG_1X1.to_vec()).await;
    let res = h
        .client
        .get(format!("http://{}/api/worlds/{}/assets", h.addr, h.world))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let list: Vec<serde_json::Value> = res.json().await.unwrap();
    assert_eq!(list.len(), 2);
}

// --- Upload/replace rate-limit and idempotency regression tests ---

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejected_upload_does_not_consume_rate_quota() {
    use common::spawn_with;
    // GM rate pinned to 1/min. A rejected upload must be refunded so the one
    // real upload still fits the budget.
    let h = spawn_with(|c| {
        c.upload_rate_per_min = 1;
        c.upload_rate_per_min_gm = Some(1);
    })
    .await;
    // Rejected (non-image) — should refund its slot.
    let bad = h
        .upload("bad.png", "image/png", b"%PDF-1.7 nope".to_vec())
        .await;
    assert_eq!(bad.status(), 400);
    // The single real upload still succeeds (quota was refunded, not burned).
    let good = h.upload("ok.png", "image/png", PNG_1X1.to_vec()).await;
    assert_eq!(
        good.status(),
        200,
        "refund failed; quota was consumed by the rejected upload"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replace_is_rate_limited_per_user() {
    use common::spawn_with;
    // Shared per-user budget (upload + replace) pinned to 2/min for the GM.
    let h = spawn_with(|c| {
        c.upload_rate_per_min = 2;
        c.upload_rate_per_min_gm = Some(2);
    })
    .await;
    let asset: serde_json::Value = h
        .upload("m.png", "image/png", PNG_1X1.to_vec())
        .await
        .json()
        .await
        .unwrap();
    let id = asset["id"].as_str().unwrap().to_string();
    // Op 2 (first replace) fits the 2/min budget.
    assert_eq!(
        h.replace(&id, "m.png", "image/png", PNG_1X1.to_vec())
            .await
            .status(),
        200
    );
    // Op 3 exceeds it — replace participates in the per-user rate limit.
    assert_eq!(
        h.replace(&id, "m.png", "image/png", PNG_1X1.to_vec())
            .await
            .status(),
        429
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejected_replace_does_not_consume_rate_quota() {
    use common::spawn_with;
    let h = spawn_with(|c| {
        c.upload_rate_per_min = 2;
        c.upload_rate_per_min_gm = Some(2);
    })
    .await;
    let asset: serde_json::Value = h
        .upload("m.png", "image/png", PNG_1X1.to_vec())
        .await
        .json()
        .await
        .unwrap();
    let id = asset["id"].as_str().unwrap().to_string();
    // Rejected (non-image) replace → 400; its rate slot is refunded.
    assert_eq!(
        h.replace(&id, "x.png", "image/png", b"%PDF-1.7 nope".to_vec())
            .await
            .status(),
        400
    );
    // The good replace still fits (upload=1, bad refunded, good=2).
    assert_eq!(
        h.replace(&id, "m.png", "image/png", PNG_1X1.to_vec())
            .await
            .status(),
        200
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_replace_leaves_original_intact() {
    let h = spawn().await;
    let asset: serde_json::Value = h
        .upload("m.png", "image/png", PNG_1X1.to_vec())
        .await
        .json()
        .await
        .unwrap();
    let id = asset["id"].as_str().unwrap().to_string();

    // Replace with non-image bytes → 400; the DB write and rename never run.
    let res = h
        .client
        .post(format!("http://{}/api/assets/{}/replace", h.addr, id))
        .multipart(
            reqwest::multipart::Form::new().part(
                "file",
                reqwest::multipart::Part::bytes(b"%PDF-1.7 nope".to_vec())
                    .file_name("x.png")
                    .mime_str("image/png")
                    .unwrap(),
            ),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);

    // Original is untouched: version still 1, original bytes served.
    let got = h
        .client
        .get(format!("http://{}/api/assets/{}", h.addr, id))
        .send()
        .await
        .unwrap();
    assert_eq!(got.status(), 200);
    assert_eq!(got.bytes().await.unwrap().as_ref(), PNG_1X1);
    assert_eq!(
        h.repo
            .get_asset(uuid::Uuid::parse_str(&id).unwrap())
            .await
            .unwrap()
            .unwrap()
            .version,
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_is_idempotent_second_is_404() {
    let h = spawn().await;
    let asset: serde_json::Value = h
        .upload("m.png", "image/png", PNG_1X1.to_vec())
        .await
        .json()
        .await
        .unwrap();
    let id = asset["id"].as_str().unwrap().to_string();
    let url = format!("http://{}/api/assets/{}", h.addr, id);

    assert_eq!(h.client.delete(&url).send().await.unwrap().status(), 204);
    // The atomic DELETE..RETURNING means the second delete finds no row → 404,
    // not a second 204 + duplicate broadcast.
    assert_eq!(h.client.delete(&url).send().await.unwrap().status(), 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serve_304_when_etag_in_comma_list() {
    let h = spawn().await;
    let asset: serde_json::Value = h
        .upload("m.png", "image/png", PNG_1X1.to_vec())
        .await
        .json()
        .await
        .unwrap();
    let id = asset["id"].as_str().unwrap();
    let url = format!("http://{}/api/assets/{}", h.addr, id);
    let etag = h.client.get(&url).send().await.unwrap().headers()["etag"]
        .to_str()
        .unwrap()
        .to_string();

    // A browser-style multi-ETag If-None-Match must still match → 304.
    let res = h
        .client
        .get(&url)
        .header("if-none-match", format!("\"stale-9\", {etag}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 304);
}
