use super::StaticAssets;
use crate::http::router;
use crate::http::tests::initialized_state;

/// The SPA bundle is a build artifact; when `dist/` has not been built these
/// tests self-skip so local `cargo test` (no client build) still passes. CI
/// builds the client first, so they run there.
fn dist_built() -> bool {
    StaticAssets::get("index.html").is_some()
}

#[tokio::test]
async fn serves_the_spa_index_and_assets() {
    if !dist_built() {
        eprintln!("skipping: dist/ not built (run `pnpm --filter @shadowcat/shell build`)");
        return;
    }
    let server = axum_test::TestServer::new(router(initialized_state().await).await).unwrap();

    let root = server.get("/").await;
    root.assert_status_ok();
    // The Vite SPA index mounts into #app and loads a module script.
    assert!(root.text().contains("id=\"app\""));

    // A known public asset is served from dist/.
    server.get("/favicon.ico").await.assert_status_ok();

    let missing = server.get("/does-not-exist").await;
    missing.assert_status_not_found();
}
