#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

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
mod tests;
