//! `Validator::OfFile`: the validator is derived from the opened file's bytes.
//!
//! A validator hit on that arm is therefore answered after the open, under a permit, and a rewrite
//! of the file changes the validator.

use std::sync::Arc;
use std::time::Duration;

use super::*;

/// The unquoted stem every case uses.
const STEM: &str = "33333333-3333-3333-3333-333333333333-1-opus";

/// A pool of one permit, so a held permit is observable as `held() == 1` and a second request
/// waits out the bounded wait and is refused.
fn single() -> Arc<StreamedFilePermits> {
    Arc::new(StreamedFilePermits::new(1, Duration::from_millis(20)))
}

/// Bytes whose content varies with the offset.
fn bytes(len: usize, salt: u8) -> Vec<u8> {
    (0..len)
        .map(|i| ((i % 251) as u8).wrapping_add(salt))
        .collect()
}

/// A request with the given headers.
fn headers(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.insert(name.clone(), HeaderValue::from_str(value).unwrap());
    }
    map
}

/// A byte-validated `audio/ogg` response spec for `path`.
fn spec(path: &Path) -> FileResponse<'_> {
    FileResponse {
        id: Uuid::nil(),
        path,
        content_type: "audio/ogg".to_string(),
        content_disposition: "inline".to_string(),
        disposition: Disposition::Inline,
        route: Route::Embed,
        etag: Validator::OfFile {
            stem: STEM.to_string(),
        },
        missing: MissingFile::NotFound,
    }
}

/// Serves `path` with `method` and `request` out of `permits`.
async fn serve(
    permits: &StreamedFilePermits,
    path: &Path,
    method: Method,
    request: &HeaderMap,
) -> Result<Response, AppError> {
    serve_file(spec(path), request, &method, permits).await
}

/// The `ETag` a plain GET of `path` carries.
async fn etag_of(path: &Path) -> String {
    let permits = StreamedFilePermits::new(8, Duration::from_millis(20));
    let response = serve(&permits, path, Method::GET, &HeaderMap::new())
        .await
        .unwrap();
    response
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
}

/// Each component of the validator changes it alone.
#[test]
fn each_component_changes_the_validator_alone() {
    let head = bytes(4096, 0);
    let base = byte_derived_etag(STEM, 100, 1_000, &head);
    assert_eq!(
        base,
        byte_derived_etag(STEM, 100, 1_000, &head),
        "deterministic"
    );
    let mut other_head = head.clone();
    other_head[17] ^= 1;
    for (label, changed) in [
        ("length", byte_derived_etag(STEM, 101, 1_000, &head)),
        ("mtime", byte_derived_etag(STEM, 100, 1_001, &head)),
        ("head", byte_derived_etag(STEM, 100, 1_000, &other_head)),
        ("stem", byte_derived_etag("other", 100, 1_000, &head)),
    ] {
        assert_ne!(changed, base, "{label} alone");
    }
}

/// The validator names the stem, is a strong (unprefixed) quoted tag and carries no `W/`.
#[test]
fn the_validator_is_a_strong_quoted_tag_that_begins_with_the_stem() {
    let etag = byte_derived_etag(STEM, 100, 1_000, b"head");
    assert!(etag.starts_with(&format!("\"{STEM}-")), "{etag}");
    assert!(etag.ends_with('"'));
}

/// A GET and a HEAD of one file name the same validator, and a rewrite that keeps the length but
/// changes the head changes it.
#[tokio::test]
async fn a_get_and_a_head_agree_and_a_same_length_rewrite_changes_the_validator() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("derivative");
    std::fs::write(&path, bytes(10_000, 0)).unwrap();
    let permits = StreamedFilePermits::new(8, Duration::from_millis(20));
    let get = etag_of(&path).await;
    let head = serve(&permits, &path, Method::HEAD, &HeaderMap::new())
        .await
        .unwrap();
    assert_eq!(
        head.headers().get(header::ETAG).unwrap().to_str().unwrap(),
        get
    );
    std::fs::write(&path, bytes(10_000, 1)).unwrap();
    assert_ne!(etag_of(&path).await, get, "same length, different bytes");
}

/// A rewrite that keeps the length AND the modification time (a coarse-clock tick, or a restored
/// timestamp) still changes the validator, because the head bytes are hashed: only the head hash
/// distinguishes the two files.
#[tokio::test]
async fn a_rewrite_with_the_same_length_and_mtime_changes_the_validator_through_the_head() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("derivative");
    std::fs::write(&path, bytes(10_000, 0)).unwrap();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    let before = etag_of(&path).await;
    std::fs::write(&path, bytes(10_000, 1)).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified
    );
    assert_ne!(etag_of(&path).await, before);
}

/// A hit answers 304 with the validator and no range headers, after the open: a hit with a `Range`
/// header is still a 304.
#[tokio::test]
async fn a_hit_is_a_304_even_with_a_range_header() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("derivative");
    std::fs::write(&path, bytes(10_000, 0)).unwrap();
    let etag = etag_of(&path).await;
    let permits = StreamedFilePermits::new(8, Duration::from_millis(20));
    let response = serve(
        &permits,
        &path,
        Method::GET,
        &headers(&[(header::IF_NONE_MATCH, &etag), (header::RANGE, "bytes=0-9")]),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert!(response.headers().get(header::CONTENT_RANGE).is_none());
    assert_eq!(
        response
            .headers()
            .get(header::ETAG)
            .unwrap()
            .to_str()
            .unwrap(),
        etag
    );
    assert_eq!(permits.held(), 0, "the 304 returned its permit");
}

/// The 304 takes a permit: with every permit held, a validator hit waits out the bounded wait and
/// is refused as unavailable instead of being answered from the record, and once a permit frees
/// the same request is a 304.
#[tokio::test]
async fn a_validator_hit_needs_a_permit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("derivative");
    std::fs::write(&path, bytes(10_000, 0)).unwrap();
    let etag = etag_of(&path).await;
    let request = headers(&[(header::IF_NONE_MATCH, &etag)]);
    let permits = single();
    let held = permits.acquire().await.unwrap();
    let refused = serve(&permits, &path, Method::GET, &request)
        .await
        .err()
        .unwrap();
    assert!(
        matches!(
            refused,
            AppError::Unavailable {
                retry_after_secs: 1
            }
        ),
        "{refused:?}"
    );
    assert_eq!(permits.held(), 1);
    drop(held);
    let answered = serve(&permits, &path, Method::GET, &request).await.unwrap();
    assert_eq!(answered.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(permits.held(), 0, "the 304 returned its permit");
}

/// A HEAD on this arm takes no permit: it is answered while every permit is held, 304 included.
#[tokio::test]
async fn head_takes_no_permit_on_the_byte_validated_arm() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("derivative");
    std::fs::write(&path, bytes(10_000, 0)).unwrap();
    let etag = etag_of(&path).await;
    let permits = single();
    let _held = permits.acquire().await.unwrap();
    let plain = serve(&permits, &path, Method::HEAD, &HeaderMap::new())
        .await
        .unwrap();
    assert_eq!(plain.status(), StatusCode::OK);
    let hit = serve(
        &permits,
        &path,
        Method::HEAD,
        &headers(&[(header::IF_NONE_MATCH, &etag)]),
    )
    .await
    .unwrap();
    assert_eq!(hit.status(), StatusCode::NOT_MODIFIED);
}

/// After a rewrite, the old validator is not a 304, and an `If-Range` carrying it yields the whole
/// new file (a 200), never a slice of the new bytes: a stream resumed across the swap restarts.
#[tokio::test]
async fn the_old_validator_after_a_rewrite_is_neither_a_304_nor_a_valid_if_range() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("derivative");
    std::fs::write(&path, bytes(10_000, 0)).unwrap();
    let old = etag_of(&path).await;
    std::fs::write(&path, bytes(12_000, 3)).unwrap();
    let permits = StreamedFilePermits::new(8, Duration::from_millis(20));

    let revalidated = serve(
        &permits,
        &path,
        Method::GET,
        &headers(&[(header::IF_NONE_MATCH, &old)]),
    )
    .await
    .unwrap();
    assert_eq!(revalidated.status(), StatusCode::OK);

    let resumed = serve(
        &permits,
        &path,
        Method::GET,
        &headers(&[(header::RANGE, "bytes=100-"), (header::IF_RANGE, &old)]),
    )
    .await
    .unwrap();
    assert_eq!(resumed.status(), StatusCode::OK);
    assert_eq!(
        resumed.headers().get(header::CONTENT_LENGTH).unwrap(),
        "12000"
    );

    let fresh = etag_of(&path).await;
    let ranged = serve(
        &permits,
        &path,
        Method::GET,
        &headers(&[(header::RANGE, "bytes=100-199"), (header::IF_RANGE, &fresh)]),
    )
    .await
    .unwrap();
    assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);
}

/// A missing file on the byte-validated arm is the ordinary 404, with or without a validator.
#[tokio::test]
async fn a_missing_file_is_a_404_even_with_a_validator() {
    let dir = tempfile::tempdir().unwrap();
    let absent = dir.path().join("absent");
    let permits = StreamedFilePermits::new(8, Duration::from_millis(20));
    for method in [Method::GET, Method::HEAD] {
        let error = serve(
            &permits,
            &absent,
            method,
            &headers(&[(header::IF_NONE_MATCH, "\"x\"")]),
        )
        .await
        .err()
        .unwrap();
        assert!(matches!(error, AppError::NotFound), "{error:?}");
    }
}
