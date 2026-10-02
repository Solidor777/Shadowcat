//! `serve_file` and `not_modified` over real files: the two I/O edges (GET and HEAD) agree on
//! status and headers for every range outcome, bodies are byte-exact, and a file that shrinks
//! under a response aborts it.

use super::*;
use axum::body::to_bytes;

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

fn spec<'a>(path: &'a Path, content_type: &str, route: Route) -> FileResponse<'a> {
    FileResponse {
        id: Uuid::nil(),
        path,
        content_type: content_type.to_string(),
        content_disposition: "inline".to_string(),
        disposition: Disposition::Inline,
        route,
        etag: ETAG.to_string(),
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
    serve_file(spec(path, content_type, route), headers, &Method::GET)
        .await
        .unwrap()
}

async fn head(path: &Path, content_type: &str, route: Route, headers: &HeaderMap) -> Response {
    serve_file(spec(path, content_type, route), headers, &Method::HEAD)
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

#[tokio::test]
async fn single_range_is_a_byte_exact_206() {
    let bytes = file_bytes(300_000);
    let (_dir, path) = stored(&bytes);
    for (range, start, end) in [
        ("bytes=10-19", 10usize, 19usize),
        ("bytes=-5", 299_995, 299_999),
        ("bytes=299990-", 299_990, 299_999),
        // A slice spanning several read chunks.
        ("bytes=1000-200000", 1000, 200_000),
    ] {
        let response = get(
            &path,
            "audio/ogg",
            Route::Embed,
            &request(Some(range), None),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT, "{range}");
        assert_eq!(
            text(&response, header::CONTENT_RANGE).unwrap(),
            format!("bytes {start}-{end}/300000")
        );
        assert_eq!(
            text(&response, header::CONTENT_LENGTH).unwrap(),
            (end - start + 1).to_string()
        );
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body.as_ref(), &bytes[start..=end], "{range}");
    }
}

#[tokio::test]
async fn unsatisfiable_range_is_a_416_with_the_total() {
    let (_dir, path) = stored(&file_bytes(100));
    let response = get(
        &path,
        "audio/ogg",
        Route::Embed,
        &request(Some("bytes=100-"), None),
    )
    .await;
    assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        text(&response, header::CONTENT_RANGE).unwrap(),
        "bytes */100"
    );
    assert_eq!(text(&response, header::CONTENT_LENGTH).unwrap(), "0");
    assert_eq!(text(&response, header::ETAG).unwrap(), ETAG);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(body.is_empty());
}

#[tokio::test]
async fn multipart_response_has_an_exact_length_and_parses_back() {
    let bytes = file_bytes(300_000);
    let (_dir, path) = stored(&bytes);
    let response = get(
        &path,
        "application/zip",
        Route::Original,
        &request(Some("bytes=0-9,100000-100099,299990-"), None),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    let content_type = text(&response, header::CONTENT_TYPE).unwrap();
    let boundary = content_type
        .strip_prefix("multipart/byteranges; boundary=")
        .expect("multipart/byteranges content type")
        .to_string();
    let declared: usize = text(&response, header::CONTENT_LENGTH)
        .unwrap()
        .parse()
        .unwrap();
    assert!(response.headers().get(header::CONTENT_RANGE).is_none());
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.len(), declared, "Content-Length is exact");
    assert!(body.starts_with(format!("--{boundary}\r\n").as_bytes()));
    assert!(body.ends_with(format!("\r\n--{boundary}--\r\n").as_bytes()));
    for (start, end) in [(0usize, 9usize), (100_000, 100_099), (299_990, 299_999)] {
        let head = format!(
            "Content-Type: application/zip\r\nContent-Range: bytes {start}-{end}/300000\r\n\r\n"
        );
        let at = body
            .windows(head.len())
            .position(|w| w == head.as_bytes())
            .expect("part head present");
        let payload_start = at + head.len();
        assert_eq!(
            &body[payload_start..payload_start + (end - start + 1)],
            &bytes[start..=end]
        );
    }
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
        let error = serve_file(s, &HeaderMap::new(), &method)
            .await
            .err()
            .unwrap();
        assert!(matches!(error, AppError::NotFound), "{method}");
        s = spec(&path, "audio/ogg", Route::Embed);
        s.missing = MissingFile::Internal;
        let error = serve_file(s, &HeaderMap::new(), &method)
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
