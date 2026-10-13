use super::{is_hashed_asset_path, static_handler, StaticAssets};
use crate::http::router;
use crate::http::tests::initialized_state;
use axum::http::header;

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

#[test]
fn hashed_asset_path_predicate_matches_only_the_content_hashed_assets_dir() {
    // The client build's `rollupOptions.output.entryFileNames` writes every non-runtime chunk
    // to `assets/[name]-[hash].js` — a changed asset therefore gets a new URL, so those paths
    // alone are safe to cache as `immutable`. `runtime/*.js` chunk names are forced STABLE
    // (no hash) so `index.html`'s import map can reference them at build time, and root files
    // (`index.html`, `favicon.ico`) never carry a hash either.
    assert!(is_hashed_asset_path("assets/main-DSZtRtmZ.css"));
    assert!(is_hashed_asset_path("assets/index-client-BdH11yDY.js"));
    assert!(!is_hashed_asset_path("index.html"));
    assert!(!is_hashed_asset_path("popout.html"));
    assert!(!is_hashed_asset_path("favicon.ico"));
    assert!(!is_hashed_asset_path("runtime/shadowcat-core.js"));
}

#[tokio::test]
async fn spa_html_is_gzip_compressed_image_assets_are_not() {
    if !dist_built() {
        eprintln!("skipping: dist/ not built (run `pnpm --filter @shadowcat/shell build`)");
        return;
    }
    let server = axum_test::TestServer::new(router(initialized_state().await).await).unwrap();

    let html = server
        .get("/index.html")
        .add_header(header::ACCEPT_ENCODING, "gzip")
        .await;
    html.assert_status_ok();
    assert_eq!(html.header(header::CONTENT_ENCODING), "gzip");

    let icon = server
        .get("/icon-192.png")
        .add_header(header::ACCEPT_ENCODING, "gzip")
        .await;
    icon.assert_status_ok();
    assert!(
        icon.maybe_header(header::CONTENT_ENCODING).is_none(),
        "an already-compressed PNG must not be re-compressed"
    );
}

#[tokio::test]
async fn hashed_assets_get_immutable_cache_control_root_files_get_no_cache() {
    if !dist_built() {
        eprintln!("skipping: dist/ not built (run `pnpm --filter @shadowcat/shell build`)");
        return;
    }
    let index: axum::http::Uri = "/index.html".parse().unwrap();
    let res = static_handler(index).await;
    assert_eq!(
        res.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-cache"
    );

    let asset_name = StaticAssets::iter()
        .find(|p| is_hashed_asset_path(p))
        .expect("dist/ built with at least one hashed asset");
    let uri: axum::http::Uri = format!("/{asset_name}").parse().unwrap();
    let res = static_handler(uri).await;
    assert_eq!(
        res.headers().get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=31536000, immutable"
    );
}
