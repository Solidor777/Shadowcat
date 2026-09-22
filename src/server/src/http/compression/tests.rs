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

#[test]
fn excludes_opus_and_webm_audio_derivatives() {
    let predicate = AssetAndWsSafePredicate::new();
    // The real content-type strings `crate::data::asset::process::audio` serves, including
    // their `codecs=` suffix.
    assert!(!predicate.should_compress(&response("audio/ogg; codecs=opus", StatusCode::OK)));
    assert!(!predicate.should_compress(&response("audio/webm; codecs=opus", StatusCode::OK)));
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
