//! This module serves the rust-embedded client bundle (`dist/`) as the router's fallback.

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

/// True when `path` lives under the content-hashed `assets/` directory that the client build's
/// `rollupOptions.output.entryFileNames` setting writes every non-runtime build chunk to
/// (`assets/[name]-[hash].js`, and Vite's default asset naming for images/CSS matches the same
/// convention) — a changed file gets a brand-new URL there, so it alone is safe to cache
/// `immutable`. Root files (the page entry points and favicons) and the deliberately STABLE-named
/// `runtime/*.js` chunks (referenced by name from the page's import map at build time) are
/// excluded: their URL never changes when their content does.
fn is_hashed_asset_path(path: &str) -> bool {
    path.starts_with("assets/")
}

/// Serve an embedded asset by request path; `/` maps to `index.html`. Sets `Cache-Control`
/// per `is_hashed_asset_path`: `public, max-age=31536000, immutable` under `assets/`, `no-cache`
/// everywhere else so a client always revalidates a root file or runtime chunk with the server.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use axum::http::Uri;
/// use shadowcat::http::embed::static_handler;
///
/// // Reads the embedded/dist-relative bundle, so this is `no_run`: it needs
/// // the built `dist/` tree on disk (release) or embedded at compile time.
/// let uri: Uri = "/index.html".parse().unwrap();
/// let _response = static_handler(uri).await;
/// # }
/// ```
pub async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match StaticAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let cache_control = if is_hashed_asset_path(path) {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            (
                [
                    (header::CONTENT_TYPE, mime.as_ref()),
                    (header::CACHE_CONTROL, cache_control),
                ],
                content.data,
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests;
