use super::*;
use axum::body::Body;
use axum::http::Response;

/// Builds a response with the given content type and a body large enough to clear the
/// tower-http default `SizeAbove` floor (32 bytes), so a `false` result below is attributable
/// to a content-type/status exclusion and never to the size floor.
fn response(content_type: &str, status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(axum::http::header::CONTENT_TYPE, content_type)
        .body(Body::from(vec![0u8; 100]))
        .unwrap()
}

#[test]
fn compresses_ordinary_html_and_json_responses() {
    let predicate = AssetAndWsSafePredicate::new();
    assert!(predicate.should_compress(&response("text/html", StatusCode::OK)));
    assert!(predicate.should_compress(&response("application/json", StatusCode::OK)));
}

#[test]
fn excludes_webp_images_via_the_inherited_default_predicate() {
    let predicate = AssetAndWsSafePredicate::new();
    assert!(!predicate.should_compress(&response("image/webp", StatusCode::OK)));
    assert!(!predicate.should_compress(&response("image/png", StatusCode::OK)));
}

/// A response with `Accept-Ranges: bytes`, as `serve_file` writes for every ranged class.
fn ranged(content_type: &str, status: StatusCode) -> Response<Body> {
    let mut response = response(content_type, status);
    response.headers_mut().insert(
        axum::http::header::ACCEPT_RANGES,
        axum::http::HeaderValue::from_static("bytes"),
    );
    response
}

#[test]
fn a_response_that_advertises_ranges_is_never_compressed() {
    let predicate = AssetAndWsSafePredicate::new();
    // A type the tower-http default would compress, a partial response, and the two Opus
    // derivative types: the header alone decides.
    assert!(predicate.should_compress(&response("audio/mpeg", StatusCode::OK)));
    assert!(!predicate.should_compress(&ranged("audio/mpeg", StatusCode::OK)));
    assert!(!predicate.should_compress(&ranged("audio/mpeg", StatusCode::PARTIAL_CONTENT)));
    assert!(!predicate.should_compress(&ranged("audio/ogg; codecs=opus", StatusCode::OK)));
    assert!(!predicate.should_compress(&ranged("audio/webm; codecs=opus", StatusCode::OK)));
}

#[test]
fn a_compressible_asset_type_without_ranges_is_compressed() {
    let predicate = AssetAndWsSafePredicate::new();
    assert!(predicate.should_compress(&response("image/svg+xml", StatusCode::OK)));
    assert!(predicate.should_compress(&response("text/plain", StatusCode::OK)));
}

#[test]
fn excludes_a_websocket_upgrade_response_regardless_of_content_type() {
    let predicate = AssetAndWsSafePredicate::new();
    let upgrade = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .body(Body::empty())
        .unwrap();
    assert!(!predicate.should_compress(&upgrade));
}

/// The RFC 8441 (HTTP/2) WebSocket reply is a `200` with an empty body, not a `101`, so the status
/// exclusion above does not cover it. It stays uncompressed only because tower-http's default
/// size predicate treats an exact-zero body as not compressible; this pins that reliance (the
/// `Accept-Encoding` header sits on the request and never reaches the predicate).
#[test]
fn a_200_empty_body_response_is_not_compressed() {
    let predicate = AssetAndWsSafePredicate::new();
    let h2_ws_accept = Response::builder()
        .status(StatusCode::OK)
        .body(Body::empty())
        .unwrap();
    assert!(!predicate.should_compress(&h2_ws_accept));
    let with_length = Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_LENGTH, "0")
        .body(Body::empty())
        .unwrap();
    assert!(!predicate.should_compress(&with_length));
}
