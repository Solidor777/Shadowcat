use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

/// Embedded client bundle. Embeds the Vite build output (`dist/` at the repo
/// root) into the binary. In debug, rust-embed reads from disk at runtime; a
/// release build embeds at compile time, so `dist/` must exist for `cargo build
/// --release` (CI builds the client first).
#[derive(rust_embed::RustEmbed)]
#[folder = "../../dist/"]
struct StaticAssets;

/// Serve an embedded asset by request path; `/` maps to `index.html`.
pub async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match StaticAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref())], content.data).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
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
}
