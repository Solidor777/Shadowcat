//! Response predicate for `tower_http::compression::CompressionLayer`, applied globally in
//! `crate::http::router`.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use axum::http::{Response, StatusCode};
use http_body::Body;
use tower_http::compression::predicate::{DefaultPredicate, NotForContentType, Predicate};

use crate::data::asset::process::audio::{OPUS_CONTENT_TYPE, WEBM_CONTENT_TYPE};

/// Response predicate: the tower-http default (`image/*`, gRPC, SSE, and bodies under 32 bytes
/// are excluded) PLUS the two asset-derivative media types the default does not know about —
/// `audio::OPUS_CONTENT_TYPE`/`audio::WEBM_CONTENT_TYPE` transcodes served by
/// `crate::http::assets::serve` are already compressed, so gzip/br would spend CPU for an
/// equal-or-larger output — PLUS any WebSocket upgrade response. A `101 Switching Protocols`
/// reply hands the raw TCP stream to the WebSocket protocol from that point on; wrapping it in
/// a gzip/br encoder would corrupt every frame sent after the upgrade, so it is refused by
/// status code alone (a status this specific never occurs on an ordinary REST/asset response).
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
        self.default.should_compress(response)
            && NotForContentType::const_new(OPUS_CONTENT_TYPE).should_compress(response)
            && NotForContentType::const_new(WEBM_CONTENT_TYPE).should_compress(response)
    }
}

#[cfg(test)]
mod tests;
