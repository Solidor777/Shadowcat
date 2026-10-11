//! The bare hyper server's single-buffer variant serves the whole body.

use shadowcat_test_support::throughput::{bare_server, download, BareApp};

/// A download from the `BareApp::one_buffer` server completes only when every one of
/// `BODY_BYTES` arrives: `download` asserts the received length equals it.
#[tokio::test]
async fn the_one_buffer_variant_serves_the_whole_body() {
    let addr = bare_server(None, true, BareApp::one_buffer()).await;
    download(addr, "/").await;
}
