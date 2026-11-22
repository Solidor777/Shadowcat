//! What an asset replace and an asset delete do to a file that a reader holds open.
//!
//! A response that streams an asset keeps the canonical file open for as long as its body is
//! unfinished (up to the egress window plus one sampler tick when the peer stops reading), so the
//! replace and delete routes must neither fail nor block on that handle, and a delete must free
//! the name at once. Rename and unlink semantics over an open handle are operating-system
//! behaviour, so the outcome is measured on each leg: the test asserts the contract and records
//! the observed values with `measure::record`, which reaches the runner's step summary.
//!
//! The held handle is a `tokio::fs::File` from `File::open`, the open the streamed serve path
//! uses, and `replace_and_delete_succeed_while_a_response_streams_the_canonical` repeats both
//! controls with a real streaming response as the open handle. The delete controls call the
//! production route, whose removal is a move into the trash (`Trash::discard`); the tests
//! themselves perform no delete.

use std::time::{Duration, Instant};

use shadowcat_test_support as common;
use shadowcat_test_support::measure::record;
use tokio::io::AsyncReadExt;

/// Upper bound on a replace or delete round trip against an idle local server. A hold on the
/// file that blocked the route would run into the tokio test's own timeout instead; this bound
/// turns a stall into a named failure.
const ROUTE_BUDGET: Duration = Duration::from_secs(10);

/// A pass-through (non-image) payload large enough to span several reads.
fn payload(seed: u8) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7 ".to_vec();
    bytes.extend((0..200_000u32).map(|i| (i as u8).wrapping_add(seed)));
    bytes
}

/// OS leg label for the record lines.
fn leg() -> &'static str {
    std::env::consts::OS
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replace_succeeds_while_a_reader_holds_the_canonical_open() {
    let h = common::spawn().await;
    let old = payload(1);
    let asset: serde_json::Value = h
        .upload("held.pdf", "application/pdf", old.clone())
        .await
        .json()
        .await
        .unwrap();
    let id = asset["id"].as_str().unwrap().to_string();
    let path = h.assets_dir.join(h.world.to_string()).join(&id);
    let mut held = tokio::fs::File::open(&path).await.unwrap();

    let new = payload(9);
    let started = Instant::now();
    let res = tokio::time::timeout(
        ROUTE_BUDGET,
        h.replace(&id, "held2.pdf", "application/pdf", new.clone()),
    )
    .await
    .expect("replace blocked on the held handle");
    let elapsed = started.elapsed();
    let status = res.status();
    record(&format!(
        "asset-open-handle replace os={} status={status} elapsed_ms={}",
        leg(),
        elapsed.as_millis()
    ));
    assert_eq!(
        status,
        200,
        "replace over a held canonical: {:?}",
        res.text().await
    );

    let mut seen = Vec::new();
    held.read_to_end(&mut seen).await.unwrap();
    let held_old = seen == old;
    let held_new = seen == new;
    record(&format!(
        "asset-open-handle replace os={} held_handle_reads_old_bytes={held_old} reads_new_bytes={held_new}",
        leg()
    ));
    assert!(
        held_old,
        "the open handle must yield the bytes it was opened over, not a mixture or the new file"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        new,
        "the name serves the new bytes"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_frees_the_name_while_a_reader_holds_the_canonical_open() {
    let h = common::spawn().await;
    let old = payload(3);
    let asset: serde_json::Value = h
        .upload("held.pdf", "application/pdf", old.clone())
        .await
        .json()
        .await
        .unwrap();
    let id = asset["id"].as_str().unwrap().to_string();
    let uuid = uuid::Uuid::parse_str(&id).unwrap();
    let path = h.assets_dir.join(h.world.to_string()).join(&id);
    let mut held = tokio::fs::File::open(&path).await.unwrap();

    let started = Instant::now();
    let res = tokio::time::timeout(
        ROUTE_BUDGET,
        h.client
            .delete(format!("http://{}/api/assets/{id}", h.addr))
            .send(),
    )
    .await
    .expect("delete blocked on the held handle")
    .unwrap();
    let elapsed = started.elapsed();
    let status = res.status();
    let name_listed = path.exists();
    let recreated = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map(|_| ())
        .map_err(|e| format!("{:?}: {e}", e.kind()));
    record(&format!(
        "asset-open-handle delete os={} status={status} elapsed_ms={} name_exists_after={name_listed} recreate_name={recreated:?}",
        leg(),
        elapsed.as_millis()
    ));
    assert_eq!(status, 204);
    assert!(h.repo.get_asset(uuid).await.unwrap().is_none());
    assert!(
        !name_listed,
        "the deleted name is still listed while a reader holds it"
    );
    assert!(
        recreated.is_ok(),
        "the deleted name cannot be re-created while a reader holds it: {recreated:?}"
    );

    let mut seen = Vec::new();
    held.read_to_end(&mut seen).await.unwrap();
    record(&format!(
        "asset-open-handle delete os={} held_handle_reads_old_bytes={}",
        leg(),
        seen == old
    ));
    assert_eq!(
        seen, old,
        "the open handle keeps yielding the deleted file's bytes"
    );
}

/// A payload several times larger than any loopback socket buffer pair, so a client that reads
/// nothing leaves the server's body unfinished and its file open.
fn large_payload(seed: u8) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7 ".to_vec();
    bytes.extend((0..40 * 1024 * 1024u32).map(|i| (i as u8).wrapping_add(seed)));
    bytes
}

/// The response-level form of the controls above: the open handle is a streaming response whose
/// client has read nothing past the headers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replace_and_delete_succeed_while_a_response_streams_the_canonical() {
    let h = common::spawn().await;
    let old = large_payload(5);
    let asset: serde_json::Value = h
        .upload("streamed.pdf", "application/pdf", old.clone())
        .await
        .json()
        .await
        .unwrap();
    let id = asset["id"].as_str().unwrap().to_string();
    let path = h.assets_dir.join(h.world.to_string()).join(&id);
    let streaming = h
        .client
        .get(format!("http://{}/api/assets/{id}", h.addr))
        .send()
        .await
        .unwrap();
    assert_eq!(streaming.status(), 200);

    let new = payload(9);
    let replaced = tokio::time::timeout(
        ROUTE_BUDGET,
        h.replace(&id, "streamed2.pdf", "application/pdf", new.clone()),
    )
    .await
    .expect("replace blocked on the streaming response");
    assert_eq!(replaced.status(), 200, "{:?}", replaced.text().await);
    assert_eq!(std::fs::read(&path).unwrap(), new);

    let deleted = tokio::time::timeout(
        ROUTE_BUDGET,
        h.client
            .delete(format!("http://{}/api/assets/{id}", h.addr))
            .send(),
    )
    .await
    .expect("delete blocked on the streaming response")
    .unwrap();
    assert_eq!(deleted.status(), 204);
    let name_listed = path.exists();
    record(&format!(
        "asset-open-handle response os={} replace_status=200 delete_status=204 name_exists_after_delete={name_listed}",
        leg()
    ));
    assert!(
        !name_listed,
        "the deleted name is still listed while a response streams it"
    );

    let body = streaming.bytes().await.unwrap();
    assert_eq!(
        body.len(),
        old.len(),
        "the streaming response completes over the replaced and deleted file"
    );
    assert!(
        body == old,
        "the response yields the bytes it was opened over"
    );
}
