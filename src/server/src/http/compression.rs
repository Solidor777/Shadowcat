//! The `CompressionLayer` response predicate `router` installs globally.
//!
//! Response predicate for `tower_http::compression::CompressionLayer`, applied globally in
//! `crate::http::router`.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use axum::http::header::ACCEPT_RANGES;
use axum::http::{Response, StatusCode};
use http_body::Body;
use tower_http::compression::predicate::{DefaultPredicate, Predicate};

/// Response predicate: the tower-http default (`image/*`, gRPC, SSE, and bodies under 32 bytes
/// are excluded) PLUS any response that carries `Accept-Ranges` PLUS any WebSocket upgrade
/// response.
///
/// `Accept-Ranges` is the carrier of `http::assets::range::classify`'s decision: `serve_file`
/// writes it exactly when a response is ranged, and a ranged response cannot also be compressed
/// (a range addresses bytes of the selected representation, so compressing the whole body or
/// each part would break `Content-Range`, `If-Range` and a strong `ETag`). Reading the header
/// here, rather than keeping a second list of content types, makes the ranged-versus-compressed
/// decision a single one: the compression layer follows whatever the asset handlers decided and
/// the two cannot disagree. No other route sets `Accept-Ranges`.
///
/// A `101 Switching Protocols` reply hands the raw TCP stream to the WebSocket protocol from that
/// point on; wrapping it in a gzip/br encoder would corrupt every frame sent after the upgrade, so
/// it is refused by status code alone (a status this specific never occurs on an ordinary
/// REST/asset response).
///
/// # Examples
///
/// ```
/// use axum::http::{Response, StatusCode};
/// use shadowcat::http::compression::AssetAndWsSafePredicate;
/// use tower_http::compression::predicate::Predicate;
///
/// let predicate = AssetAndWsSafePredicate::new();
/// let switching_protocols = Response::builder()
///     .status(StatusCode::SWITCHING_PROTOCOLS)
///     .body(axum::body::Body::empty())
///     .unwrap();
/// assert!(!predicate.should_compress(&switching_protocols));
///
/// let ranged = Response::builder()
///     .header(axum::http::header::CONTENT_TYPE, "audio/mpeg")
///     .header(axum::http::header::ACCEPT_RANGES, "bytes")
///     .body(axum::body::Body::from(vec![0u8; 100]))
///     .unwrap();
/// assert!(!predicate.should_compress(&ranged));
/// ```
#[derive(Clone, Default)]
pub struct AssetAndWsSafePredicate {
    /// The tower-http built-in default predicate this composes with.
    default: DefaultPredicate,
}

impl AssetAndWsSafePredicate {
    /// Build the composed predicate.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::http::compression::AssetAndWsSafePredicate;
    ///
    /// let _predicate = AssetAndWsSafePredicate::new();
    /// ```
    pub fn new() -> Self {
        Self::default()
    }
}

impl Predicate for AssetAndWsSafePredicate {
    fn should_compress<B>(&self, response: &Response<B>) -> bool
    where
        B: Body,
    {
        if response.status() == StatusCode::SWITCHING_PROTOCOLS {
            return false;
        }
        !response.headers().contains_key(ACCEPT_RANGES) && self.default.should_compress(response)
    }
}

#[cfg(test)]
mod tests;
