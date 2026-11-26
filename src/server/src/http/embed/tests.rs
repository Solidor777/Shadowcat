use super::{is_hashed_asset_path, static_handler, strip_route_prefix, StaticAssets};
use crate::http::tests::initialized_state;
use crate::http::{router, AppState};
use axum::extract::{OriginalUri, State};
use axum::http::{header, Uri};
use std::sync::Arc;

/// An initialized state whose configuration mounts the server under `prefix`.
async fn state_with_prefix(prefix: &str) -> AppState {
    let mut state = initialized_state().await;
    state.config = Arc::new(crate::config::Config {
        route_prefix: Some(prefix.into()),
        ..crate::config::Config::default()
    });
    state
}

/// Runs `static_handler` for `path` against `state` and returns the response.
async fn serve_path(state: &AppState, path: &str) -> axum::response::Response {
    let uri: Uri = path.parse().unwrap();
    static_handler(State(state.clone()), OriginalUri(uri)).await
}

/// Reads a handler response body as text.
async fn body_text(res: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

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
    // The client build's `rollupOptions.output.entryFileNames` writes entry chunks outside
    // `runtime/` to `assets/[name]-[hash].js`; the content-hashed `assets/` directory is what the
    // predicate matches, and a changed file there gets a new URL, so those paths alone are safe to
    // cache as `immutable`. `runtime/*.js` chunk names are forced STABLE (no hash) so
    // `index.html`'s import map can reference them at build time.
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
    let state = initialized_state().await;
    let res = serve_path(&state, "/index.html").await;
    assert_eq!(
        res.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-cache"
    );

    let asset_name = StaticAssets::iter()
        .find(|p| is_hashed_asset_path(p))
        .expect("dist/ built with at least one hashed asset");
    let res = serve_path(&state, &format!("/{asset_name}")).await;
    assert_eq!(
        res.headers().get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=31536000, immutable"
    );
}

#[test]
fn strip_route_prefix_removes_only_a_whole_leading_segment() {
    assert_eq!(
        strip_route_prefix("/ttrpg/assets/a.js", "/ttrpg"),
        "/assets/a.js"
    );
    assert_eq!(strip_route_prefix("/ttrpg/", "/ttrpg"), "/");
    assert_eq!(strip_route_prefix("/ttrpg", "/ttrpg"), "");
    // A sibling path that merely shares the prefix's characters is not under it.
    assert_eq!(strip_route_prefix("/ttrpgx/a.js", "/ttrpg"), "/ttrpgx/a.js");
    // Without the prefix, or with an empty one, the path is returned unchanged.
    assert_eq!(strip_route_prefix("/assets/a.js", "/ttrpg"), "/assets/a.js");
    assert_eq!(strip_route_prefix("/assets/a.js", ""), "/assets/a.js");
    // Only the leading occurrence is removed: the remainder is not stripped again.
    assert_eq!(
        strip_route_prefix("/ttrpg/ttrpg/a.js", "/ttrpg"),
        "/ttrpg/a.js"
    );
}

#[tokio::test]
async fn hashed_assets_stay_immutable_under_a_route_prefix() {
    if !dist_built() {
        eprintln!("skipping: dist/ not built (run `pnpm --filter @shadowcat/shell build`)");
        return;
    }
    let state = state_with_prefix("/ttrpg").await;
    let asset_name = StaticAssets::iter()
        .find(|p| is_hashed_asset_path(p))
        .expect("dist/ built with at least one hashed asset");
    let res = serve_path(&state, &format!("/ttrpg/{asset_name}")).await;
    assert_eq!(res.status(), axum::http::StatusCode::OK);
    assert_eq!(
        res.headers().get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=31536000, immutable"
    );
    let index = serve_path(&state, "/ttrpg/index.html").await;
    assert_eq!(
        index.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-cache"
    );
}

#[tokio::test]
async fn index_html_carries_the_configured_prefix_not_the_placeholder() {
    if !dist_built() {
        eprintln!("skipping: dist/ not built (run `pnpm --filter @shadowcat/shell build`)");
        return;
    }
    let prefixed = body_text(serve_path(&state_with_prefix("/ttrpg").await, "/ttrpg/").await).await;
    assert!(!prefixed.contains(super::BASE_PLACEHOLDER));
    assert!(prefixed.contains("<base href=\"/ttrpg/\""));
    assert!(prefixed.contains("window.__SHADOWCAT_BASE__ = \"/ttrpg\";"));

    // The root mount substitutes the empty prefix: `<base href="/">` and an empty global.
    let root = body_text(serve_path(&initialized_state().await, "/").await).await;
    assert!(!root.contains(super::BASE_PLACEHOLDER));
    assert!(root.contains("<base href=\"/\""));
    assert!(root.contains("window.__SHADOWCAT_BASE__ = \"\";"));
}

#[tokio::test]
async fn only_index_html_is_rewritten() {
    if !dist_built() {
        eprintln!("skipping: dist/ not built (run `pnpm --filter @shadowcat/shell build`)");
        return;
    }
    let state = state_with_prefix("/ttrpg").await;
    let asset_name = StaticAssets::iter()
        .find(|p| is_hashed_asset_path(p))
        .expect("dist/ built with at least one hashed asset");
    let served = axum::body::to_bytes(
        serve_path(&state, &format!("/ttrpg/{asset_name}"))
            .await
            .into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    assert_eq!(
        served.as_ref(),
        StaticAssets::get(&asset_name).unwrap().data.as_ref()
    );
}

#[tokio::test]
async fn router_is_nested_under_the_route_prefix_and_not_dual_mounted() {
    if !dist_built() {
        eprintln!("skipping: dist/ not built (run `pnpm --filter @shadowcat/shell build`)");
        return;
    }
    let server =
        axum_test::TestServer::new(router(state_with_prefix("/ttrpg").await).await).unwrap();

    server.get("/ttrpg/api/config").await.assert_status_ok();
    server.get("/ttrpg/health").await.assert_status_ok();
    server.get("/api/config").await.assert_status_not_found();
    server.get("/health").await.assert_status_not_found();
    // The SPA is served at the prefix root, with and without the trailing slash, and its
    // sibling-looking path is outside the mount.
    let slash = server.get("/ttrpg/").await;
    slash.assert_status_ok();
    assert!(slash.text().contains("id=\"app\""));
    // The response layers wrap the nested router, so the bare-slash route and a miss outside the
    // mount carry the same headers as any other response.
    assert_eq!(slash.header(header::X_FRAME_OPTIONS), "DENY");
    let outside = server.get("/elsewhere").await;
    outside.assert_status_not_found();
    assert_eq!(outside.header(header::X_FRAME_OPTIONS), "DENY");
    server.get("/ttrpg").await.assert_status_ok();
    server.get("/ttrpgx/").await.assert_status_not_found();
    server.get("/").await.assert_status_not_found();
    // An unknown path inside the mount is the embedded handler's 404, not a routing miss.
    server
        .get("/ttrpg/does-not-exist")
        .await
        .assert_status_not_found();
    // A hashed asset resolves through the whole router under the prefix.
    let asset_name = StaticAssets::iter()
        .find(|p| is_hashed_asset_path(p))
        .expect("dist/ built with at least one hashed asset");
    let asset = server.get(&format!("/ttrpg/{asset_name}")).await;
    asset.assert_status_ok();
    assert_eq!(
        asset.header(header::CACHE_CONTROL),
        "public, max-age=31536000, immutable"
    );
}
