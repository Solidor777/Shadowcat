use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

/// Embedded transitional auth bundle. Embeds `src/server/static/` into the
/// binary. SEAM: when the Vite client bundle exists, repoint `folder` at the
/// client `dist/` output — callers of `static_handler` do not change.
#[derive(rust_embed::RustEmbed)]
#[folder = "static/"]
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
    use crate::http::router;
    use crate::http::tests::initialized_state;

    #[tokio::test]
    async fn serves_index_at_root_and_named_assets() {
        let server = axum_test::TestServer::new(router(initialized_state().await).await).unwrap();

        let root = server.get("/").await;
        root.assert_status_ok();
        assert!(root.text().contains("Server is running"));

        let setup = server.get("/setup.html").await;
        setup.assert_status_ok();
        assert!(setup.text().contains("Create the admin account"));

        let missing = server.get("/does-not-exist").await;
        missing.assert_status_not_found();
    }
}
