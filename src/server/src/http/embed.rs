//! This module serves the rust-embedded client bundle (`dist/`) as the router's fallback.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::borrow::Cow;

use axum::extract::{OriginalUri, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

use super::AppState;

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

/// The token the client build leaves in `index.html` (as the `<base href>` and the
/// `window.__SHADOWCAT_BASE__` value); `static_handler` replaces it with the configured route
/// prefix (`""` at the root mount) when it serves that one file.
pub const BASE_PLACEHOLDER: &str = "%%SHADOWCAT_BASE%%";

/// The request path relative to the bundle root: `path` without `prefix`, which is removed only
/// when it is a whole leading path segment (`/ttrpg` matches `/ttrpg` and `/ttrpg/x`, not
/// `/ttrpgx`). An empty `prefix`, or a `path` that does not start with it, comes back unchanged.
///
/// # Examples
///
/// ```
/// use shadowcat::http::embed::strip_route_prefix;
///
/// assert_eq!(strip_route_prefix("/ttrpg/assets/a.js", "/ttrpg"), "/assets/a.js");
/// assert_eq!(strip_route_prefix("/ttrpg", "/ttrpg"), "");
/// assert_eq!(strip_route_prefix("/ttrpgx/a.js", "/ttrpg"), "/ttrpgx/a.js");
/// assert_eq!(strip_route_prefix("/assets/a.js", ""), "/assets/a.js");
/// ```
pub fn strip_route_prefix<'a>(path: &'a str, prefix: &str) -> &'a str {
    match path.strip_prefix(prefix) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => rest,
        _ => path,
    }
}

/// Serve an embedded asset by request path; `/` maps to `index.html`. Sets `Cache-Control`
/// per `is_hashed_asset_path`: `public, max-age=31536000, immutable` under `assets/`, `no-cache`
/// everywhere else so a client always revalidates a root file or runtime chunk with the server.
///
/// The bundle is rooted at the configured route prefix: the handler reads the ORIGINAL request
/// path (`OriginalUri`, which a router `nest` leaves intact) and removes the prefix exactly once
/// with [`strip_route_prefix`], so the lookup key and `is_hashed_asset_path` always see a
/// bundle-relative path. `index.html` is the one file that is not served byte-for-byte: every
/// [`BASE_PLACEHOLDER`] in it is replaced with the prefix.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use axum::extract::{OriginalUri, State};
/// use axum::http::Uri;
/// use shadowcat::config::Config;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::http::embed::static_handler;
/// use shadowcat::http::AppState;
/// use std::sync::atomic::AtomicBool;
/// use std::sync::Arc;
///
/// // Reads the embedded/dist-relative bundle, so this is `no_run`: it needs
/// // the built `dist/` tree on disk (release) or embedded at compile time.
/// let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
/// let state = AppState {
///     repo,
///     config: Arc::new(Config::default()),
///     setup_token: None,
///     initialized: Arc::new(AtomicBool::new(true)),
///     ws: shadowcat::ws::WsState::new(),
///     upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
///     uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
///     auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
///     write_barrier: Arc::new(tokio::sync::RwLock::new(())),
///     preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
///     file_permits: Arc::new(shadowcat::http::assets::permits::StreamedFilePermits::production()),
///     tls: shadowcat::tls::TlsRuntime::disabled(),
///     portmap: shadowcat::portmap::PortMapRuntime::disabled(),
///     trash: std::sync::Arc::new(shadowcat::trash::Trash::new(std::env::temp_dir(), shadowcat::trash::TrashRetention::Disabled)),
/// };
/// let uri: Uri = "/index.html".parse().unwrap();
/// let _response = static_handler(State(state), OriginalUri(uri)).await;
/// # }
/// ```
pub async fn static_handler(
    State(state): State<AppState>,
    OriginalUri(uri): OriginalUri,
) -> Response {
    let prefix = state.config.base_path();
    let path = strip_route_prefix(uri.path(), prefix).trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match StaticAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let cache_control = if is_hashed_asset_path(path) {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            let body: Cow<'static, [u8]> = if path == "index.html" {
                Cow::Owned(
                    String::from_utf8_lossy(&content.data)
                        .replace(BASE_PLACEHOLDER, prefix)
                        .into_bytes(),
                )
            } else {
                content.data
            };
            (
                [
                    (header::CONTENT_TYPE, mime.as_ref()),
                    (header::CACHE_CONTROL, cache_control),
                ],
                body,
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests;
