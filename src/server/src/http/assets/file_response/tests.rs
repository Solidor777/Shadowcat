//! These tests exercise `serve_file` and `not_modified` over real files.
//!
//! GET and HEAD agree on status and headers for every range outcome, bodies are byte-exact, and a
//! file that shrinks under a response aborts it. The tests also cover the release and refusal of
//! the streamed-file permit.

use super::*;
use axum::body::to_bytes;
use std::time::Duration;

/// The validator used by every case.
const ETAG: &str = "\"22222222-2222-2222-2222-222222222222-1\"";

/// A file of `len` bytes whose content varies with the offset, so a wrong slice is visible.
fn file_bytes(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

/// A temp directory holding `bytes` as `asset`.
fn stored(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset");
    std::fs::write(&path, bytes).unwrap();
    (dir, path)
}

/// A pool that never runs out inside one test.
fn ample() -> StreamedFilePermits {
    StreamedFilePermits::new(64, std::time::Duration::from_millis(50))
}

fn spec<'a>(path: &'a Path, content_type: &str, route: Route) -> FileResponse<'a> {
    FileResponse {
        id: Uuid::nil(),
        path,
        content_type: content_type.to_string(),
        content_disposition: "inline".to_string(),
        disposition: Disposition::Inline,
        route,
        etag: Validator::Version(ETAG.to_string()),
        missing: MissingFile::NotFound,
    }
}

fn request(range: Option<&str>, if_range: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(range) = range {
        headers.insert(header::RANGE, HeaderValue::from_str(range).unwrap());
    }
    if let Some(if_range) = if_range {
        headers.insert(header::IF_RANGE, HeaderValue::from_str(if_range).unwrap());
    }
    headers
}

async fn get(path: &Path, content_type: &str, route: Route, headers: &HeaderMap) -> Response {
    serve_file(
        spec(path, content_type, route),
        headers,
        &Method::GET,
        &ample(),
    )
    .await
    .unwrap()
}

async fn head(path: &Path, content_type: &str, route: Route, headers: &HeaderMap) -> Response {
    serve_file(
        spec(path, content_type, route),
        headers,
        &Method::HEAD,
        &ample(),
    )
    .await
    .unwrap()
}

fn text(response: &Response, name: header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .map(|v| v.to_str().unwrap().to_string())
}

#[tokio::test]
async fn full_response_streams_the_whole_file_with_an_exact_length() {
    let bytes = file_bytes(300_000);
    let (_dir, path) = stored(&bytes);
    let response = get(&path, "audio/ogg", Route::Embed, &request(None, None)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(text(&response, header::CONTENT_LENGTH).unwrap(), "300000");
    assert_eq!(text(&response, header::ACCEPT_RANGES).unwrap(), "bytes");
    assert_eq!(text(&response, header::ETAG).unwrap(), ETAG);
    assert_eq!(text(&response, header::CACHE_CONTROL).unwrap(), "no-cache");
    assert_eq!(text(&response, header::CONTENT_TYPE).unwrap(), "audio/ogg");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), bytes.as_slice());
}

#[tokio::test]
async fn an_empty_file_streams_an_empty_body() {
    let (_dir, path) = stored(b"");
    let response = get(&path, "audio/ogg", Route::Embed, &request(None, None)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(text(&response, header::CONTENT_LENGTH).unwrap(), "0");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(body.is_empty());
}

#[tokio::test]
async fn compressible_type_on_the_embed_route_is_sent_without_range_headers() {
    let bytes = file_bytes(1000);
    let (_dir, path) = stored(&bytes);
    let headers = request(Some("bytes=10-19"), None);
    let response = get(&path, "text/plain", Route::Embed, &headers).await;
    assert_eq!(response.status(), StatusCode::OK, "Range is ignored");
    assert!(response.headers().get(header::ACCEPT_RANGES).is_none());
    assert!(response.headers().get(header::CONTENT_RANGE).is_none());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), bytes.as_slice());
}

/// Every range outcome gives the same status and headers on both I/O edges, and the HEAD body is
/// empty.
#[tokio::test]
async fn get_and_head_agree_on_every_range_outcome() {
    let (_dir, path) = stored(&file_bytes(5 * 1024 * 1024));
    let rows: &[(&str, Route, Option<&str>, Option<&str>)] = &[
        ("audio/ogg", Route::Embed, None, None),
        ("audio/ogg", Route::Embed, Some("bytes=0-"), None),
        ("audio/ogg", Route::Embed, Some("bytes=10-19"), None),
        ("audio/ogg", Route::Embed, Some("bytes=-100"), None),
        ("audio/ogg", Route::Embed, Some("bytes=9999999999-"), None),
        ("audio/ogg", Route::Embed, Some("bytes=0-9,20-29"), None),
        // Over the media cap in total: answered in full.
        (
            "audio/ogg",
            Route::Embed,
            Some("bytes=0-1999999,3000000-4000000"),
            None,
        ),
        ("audio/ogg", Route::Embed, Some("items=0-9"), None),
        ("audio/ogg", Route::Embed, Some("bytes=10-19"), Some(ETAG)),
        (
            "audio/ogg",
            Route::Embed,
            Some("bytes=10-19"),
            Some("\"stale\""),
        ),
        ("text/plain", Route::Embed, Some("bytes=10-19"), None),
        ("text/plain", Route::Original, Some("bytes=3000000-"), None),
        (
            "application/zip",
            Route::Original,
            Some("bytes=0-9,100-199"),
            None,
        ),
    ];
    for (content_type, route, range, if_range) in rows {
        let headers = request(*range, *if_range);
        let got = get(&path, content_type, *route, &headers).await;
        let probed = head(&path, content_type, *route, &headers).await;
        let label = format!("{content_type} {route:?} {range:?} {if_range:?}");
        assert_eq!(got.status(), probed.status(), "{label}");
        // A multipart boundary is fresh per response; every other header is identical.
        let mut got_headers = got.headers().clone();
        let mut head_headers = probed.headers().clone();
        let multipart =
            text(&got, header::CONTENT_TYPE).is_some_and(|t| t.starts_with("multipart/byteranges"));
        if multipart {
            got_headers.remove(header::CONTENT_TYPE);
            head_headers.remove(header::CONTENT_TYPE);
            assert!(text(&probed, header::CONTENT_TYPE)
                .unwrap()
                .starts_with("multipart/byteranges; boundary="));
            // The length is the same because the boundary has a fixed length.
        }
        assert_eq!(got_headers, head_headers, "{label}");
        let body = to_bytes(probed.into_body(), usize::MAX).await.unwrap();
        assert!(body.is_empty(), "{label}: a HEAD has no body");
    }
}

#[tokio::test]
async fn head_of_a_missing_file_is_reported_like_a_get() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("absent");
    for method in [Method::GET, Method::HEAD] {
        let mut s = spec(&path, "audio/ogg", Route::Embed);
        let error = serve_file(s, &HeaderMap::new(), &method, &ample())
            .await
            .err()
            .unwrap();
        assert!(matches!(error, AppError::NotFound), "{method}");
        s = spec(&path, "audio/ogg", Route::Embed);
        s.missing = MissingFile::Internal;
        let error = serve_file(s, &HeaderMap::new(), &method, &ample())
            .await
            .err()
            .unwrap();
        assert!(matches!(error, AppError::Internal), "{method}");
    }
}

/// The length comes from the opened handle: a file replaced after the open still streams the bytes
/// the handle was opened over, and a file that shrinks under the response aborts it.
#[tokio::test]
async fn a_shrinking_file_aborts_the_response() {
    let bytes = file_bytes(1_000_000);
    let (_dir, path) = stored(&bytes);
    let response = get(&path, "audio/ogg", Route::Embed, &request(None, None)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(10).unwrap();
    drop(file);
    let result = to_bytes(response.into_body(), usize::MAX).await;
    assert!(
        result.is_err(),
        "the body ends in an error, not a short 200"
    );
}

#[tokio::test]
async fn a_rename_over_the_name_leaves_the_open_response_on_the_old_bytes() {
    let old = file_bytes(200_000);
    let (dir, path) = stored(&old);
    let response = get(&path, "audio/ogg", Route::Embed, &request(None, None)).await;
    let replacement = dir.path().join("replacement");
    std::fs::write(&replacement, file_bytes(50)).unwrap();
    std::fs::rename(&replacement, &path).unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), old.as_slice());
}

#[test]
fn not_modified_matches_any_listed_validator_and_carries_no_range_headers() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::IF_NONE_MATCH,
        HeaderValue::from_str(&format!("\"a\", {ETAG} ,\"b\"")).unwrap(),
    );
    let response = not_modified(&headers, ETAG).expect("listed validator");
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(text(&response, header::ETAG).unwrap(), ETAG);
    assert_eq!(text(&response, header::CACHE_CONTROL).unwrap(), "no-cache");
    assert!(response.headers().get(header::CONTENT_RANGE).is_none());
    assert!(response.headers().get(header::ACCEPT_RANGES).is_none());
    assert!(not_modified(&headers, "\"c\"").is_none());
    assert!(not_modified(&HeaderMap::new(), ETAG).is_none());
}

/// A pool of one permit, so a held permit is observable as `held() == 1` and a second GET is
/// refused.
fn single() -> StreamedFilePermits {
    StreamedFilePermits::new(1, Duration::from_millis(20))
}

/// Serves `path` as a GET with `headers` out of `permits`.
async fn get_with(
    permits: &StreamedFilePermits,
    path: &Path,
    headers: &HeaderMap,
) -> Result<Response, AppError> {
    serve_file(
        spec(path, "audio/ogg", Route::Embed),
        headers,
        &Method::GET,
        permits,
    )
    .await
}

/// The permit is returned the moment the last file byte has been read, before the body is polled
/// to its end, so a connection that still buffers the tail does not hold a descriptor.
#[tokio::test]
async fn the_permit_is_released_when_the_last_file_byte_is_read() {
    use futures_util::StreamExt;
    let bytes = file_bytes(FILE_READ_CHUNK * 4 + 100);
    let (_dir, path) = stored(&bytes);
    let permits = single();
    let response = get_with(&permits, &path, &request(None, None))
        .await
        .unwrap();
    assert_eq!(permits.held(), 1, "held while the response streams");
    let mut stream = response.into_body().into_data_stream();
    for read in 1..=4 {
        stream.next().await.unwrap().unwrap();
        assert_eq!(permits.held(), 1, "after chunk {read} of 5");
    }
    stream.next().await.unwrap().unwrap();
    assert_eq!(permits.held(), 0, "the last chunk is the last file byte");
    assert!(stream.next().await.is_none());
}

/// A multipart body still owes its terminator and close delimiter after the last file byte; the
/// permit does not wait for them.
#[tokio::test]
async fn a_multipart_body_releases_the_permit_after_its_last_file_piece() {
    use futures_util::StreamExt;
    let (_dir, path) = stored(&file_bytes(1000));
    let permits = single();
    let response = get_with(&permits, &path, &request(Some("bytes=0-9,20-29"), None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    let mut stream = response.into_body().into_data_stream();
    let mut chunks = 0;
    let mut released_at = None;
    while let Some(chunk) = stream.next().await {
        chunk.unwrap();
        chunks += 1;
        if released_at.is_none() && permits.held() == 0 {
            released_at = Some(chunks);
        }
    }
    let released_at = released_at.expect("released");
    assert!(
        released_at < chunks,
        "released at chunk {released_at} of {chunks}, before the close delimiter"
    );
}

/// Dropping the response (a reset, or a connection cut by the stall bounds) returns the permit
/// even though no byte was read.
#[tokio::test]
async fn dropping_the_response_releases_the_permit() {
    let (_dir, path) = stored(&file_bytes(FILE_READ_CHUNK * 3));
    let permits = single();
    let response = get_with(&permits, &path, &request(None, None))
        .await
        .unwrap();
    assert_eq!(permits.held(), 1);
    drop(response);
    assert_eq!(permits.held(), 0);
}

/// A 416 and an empty file carry no file body, so neither keeps the permit.
#[tokio::test]
async fn a_response_without_file_bytes_holds_no_permit() {
    let permits = single();
    let (_dir, path) = stored(&file_bytes(100));
    let refused = get_with(&permits, &path, &request(Some("bytes=500-"), None))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(permits.held(), 0, "416");
    let (_empty_dir, empty) = stored(&[]);
    let response = get_with(&permits, &empty, &request(None, None))
        .await
        .unwrap();
    assert_eq!(permits.held(), 0, "empty file");
    drop(response);
}

/// HEAD takes no permit and opens nothing, so it is answered while every permit is held.
#[tokio::test]
async fn head_is_answered_while_every_permit_is_held() {
    let (_dir, path) = stored(&file_bytes(100));
    let permits = std::sync::Arc::new(single());
    let _held = permits.acquire().await.unwrap();
    let response = serve_file(
        spec(&path, "audio/ogg", Route::Embed),
        &request(None, None),
        &Method::HEAD,
        &permits,
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(permits.held(), 1);
}

/// The permit is taken before the file is opened and before any header exists: with every permit
/// held, a GET is refused as unavailable even for a file that does not exist.
#[tokio::test]
async fn an_exhausted_pool_refuses_before_the_file_is_opened() {
    let dir = tempfile::tempdir().unwrap();
    let absent = dir.path().join("absent");
    let permits = std::sync::Arc::new(single());
    let _held = permits.acquire().await.unwrap();
    let error = get_with(&permits, &absent, &request(None, None))
        .await
        .err()
        .unwrap();
    assert!(
        matches!(
            error,
            AppError::Unavailable {
                retry_after_secs: 1
            }
        ),
        "{error:?}"
    );
}

/// A consumer that polls past the end of the body (tower-http's compression adapter reads its
/// source through a reader that is not fused) gets the end again rather than a panic.
#[tokio::test]
async fn polling_the_body_after_its_end_yields_the_end_again() {
    use futures_util::StreamExt;
    let (_dir, path) = stored(&file_bytes(1000));
    let response = get(&path, "audio/ogg", Route::Embed, &request(None, None)).await;
    let mut stream = response.into_body().into_data_stream();
    while stream.next().await.is_some() {}
    assert!(stream.next().await.is_none());
    assert!(stream.next().await.is_none());
}

/// An aborted body (the file shrank under it) is polled to its error and then past it.
#[tokio::test]
async fn polling_an_aborted_body_again_yields_the_end() {
    use futures_util::StreamExt;
    let (_dir, path) = stored(&file_bytes(1_000_000));
    let response = get(&path, "audio/ogg", Route::Embed, &request(None, None)).await;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(10)
        .unwrap();
    let mut stream = response.into_body().into_data_stream();
    let mut failed = false;
    while let Some(chunk) = stream.next().await {
        failed |= chunk.is_err();
    }
    assert!(failed, "the shrunk file aborts the body");
    assert!(stream.next().await.is_none());
}
