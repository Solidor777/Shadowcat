//! This module holds the oEmbed provider allowlist and the structured-fields-only oEmbed types.
//!
//! `match_provider` admits allowlisted provider HOSTS only, and `OEmbedProvider::endpoint` builds
//! the request URL from a fixed per-provider base, passing the posted URL only as the `url` query
//! value, so no endpoint is ever discovered. Autodiscovery (reading a `<link rel="alternate"
//! type="application/json+oembed">` tag from an arbitrary posted URL) would reintroduce the
//! arbitrary-host-fetch risk this feature must avoid. The fetch itself runs in
//! `chat::post_publish`.
//!
//! The accepted input, `OEmbedResponse`, carries `title`, `author_name` and `thumbnail_url` only,
//! and never a provider's raw `html` field, which is third-party-controlled markup and a direct
//! stored-XSS vector this server does not control. The output, `OEmbedSegment`, carries the posted
//! `url`, this server's fixed `OEmbedProvider::name` as `provider_name` (the provider's own
//! self-reported name is never used), `title`, `author_name` and `thumbnail_asset_id`.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::Deserialize;
use url::Host;
use uuid::Uuid;

/// A known oEmbed provider this server will query. Host allowlist ONLY —
/// see this module's doc for why autodiscovery is never attempted. Extending
/// the allowlist means adding a variant here plus a `match_provider`/
/// `endpoint`/`name` arm; the set itself is an implementation choice, not an
/// architectural one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OEmbedProvider {
    /// youtube.com / www.youtube.com / m.youtube.com / youtu.be
    YouTube,
    /// vimeo.com / www.vimeo.com / player.vimeo.com
    Vimeo,
}

impl OEmbedProvider {
    /// This server's OWN fixed display name for the card's "open on
    /// `<provider_name>`" link and `OEmbedSegment.provider_name` — NEVER the
    /// provider's self-reported `provider_name` JSON field (still
    /// third-party-controlled text; this fixed string cannot be spoofed and
    /// needs no sanitization).
    pub fn name(self) -> &'static str {
        match self {
            OEmbedProvider::YouTube => "YouTube",
            OEmbedProvider::Vimeo => "Vimeo",
        }
    }

    /// The provider's oEmbed JSON endpoint URL, carrying `posted_url` (the
    /// URL the author posted, NOT the endpoint) as the `url` query
    /// parameter. The endpoint HOST itself is always one of this module's
    /// fixed allowlisted hosts — never derived from `posted_url`. `None`
    /// only on an internal `Url` construction failure (the base strings are
    /// fixed and always valid; this is defensive, not expected to fire).
    pub fn endpoint(self, posted_url: &str) -> Option<String> {
        let (base, extra): (&str, &[(&str, &str)]) = match self {
            OEmbedProvider::YouTube => ("https://www.youtube.com/oembed", &[("format", "json")]),
            OEmbedProvider::Vimeo => ("https://vimeo.com/api/oembed.json", &[]),
        };
        let mut u = url::Url::parse(base).ok()?;
        {
            let mut qp = u.query_pairs_mut();
            qp.append_pair("url", posted_url);
            for (k, v) in extra {
                qp.append_pair(k, v);
            }
        }
        Some(u.to_string())
    }
}

/// Synchronous, zero-network host check against the allowlist — matched at
/// `link_preview::enrich` time against the SAME candidate URLs the generic
/// preview scraper extracts (genuine `<a href>` targets from sanitized HTML,
/// never raw body-text substrings). No I/O: this check admits allowlisted provider hosts only,
/// and a URL failing it falls through to the existing generic `LinkPreview` scrape unchanged.
/// The request for an admitted URL then goes through `link_preview::fetch_json_bytes`, which
/// calls `guarded_get`, so it passes the same URL validation and redirect re-validation as every
/// other fetch routed through that function. There is no autodiscovery fetch anywhere in this
/// module.
pub fn match_provider(raw_url: &str) -> Option<OEmbedProvider> {
    let url = url::Url::parse(raw_url).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    let host = match url.host() {
        Some(Host::Domain(h)) => h.to_ascii_lowercase(),
        _ => return None, // an IP-literal host never matches a provider domain
    };
    match host.as_str() {
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "youtu.be" => {
            Some(OEmbedProvider::YouTube)
        }
        "vimeo.com" | "www.vimeo.com" | "player.vimeo.com" => Some(OEmbedProvider::Vimeo),
        _ => None,
    }
}

/// The subset of a provider's oEmbed JSON response this server ever reads.
/// STRUCTURAL guarantee, not a runtime filter: this type has NO `html`
/// field, so `serde_json::from_slice` cannot populate one no matter what a
/// provider's JSON contains — the provider's raw markup is dropped by
/// ordinary serde unknown-field behavior. Deliberately does NOT set
/// `#[serde(deny_unknown_fields)]` (the opposite of every engine-defined
/// doc_type's ingress gate elsewhere in this codebase): a provider's `html`
/// field, or any other field this server doesn't read, must be silently
/// ignored, never turn a legitimate oEmbed fetch into a hard failure.
#[derive(Debug, Clone, Deserialize)]
pub struct OEmbedResponse {
    /// Provider-supplied title, if present.
    pub title: Option<String>,
    /// Provider-supplied author/channel name, if present.
    pub author_name: Option<String>,
    /// Provider-supplied thumbnail image URL — fetched and asset-ified
    /// separately (`post_publish::resolve_thumbnail_asset`), never hotlinked.
    pub thumbnail_url: Option<String>,
}

/// The client-visible, structured-fields-only oEmbed segment payload. NO
/// `html` field exists on this type by construction — see `OEmbedResponse`'s
/// doc for the same guarantee at the deserialization boundary one layer
/// earlier. The posted `url` remains the click-through target; the client
/// renders a first-party-templated card (provider name, title, thumbnail,
/// "open on `<provider_name>`" link), never any provider markup.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct OEmbedSegment {
    /// The candidate URL `enrich` extracted from the message.
    pub url: String,
    /// This server's own fixed provider display name (`OEmbedProvider::name`).
    pub provider_name: String,
    /// Provider-supplied title, if any.
    pub title: Option<String>,
    /// Provider-supplied author/channel name, if any.
    pub author_name: Option<String>,
    /// The asset-ified thumbnail, once the post-publish background pipeline
    /// resolves one. Always `None` when this segment is first appended.
    #[serde(default)]
    pub thumbnail_asset_id: Option<Uuid>,
}

#[cfg(test)]
mod tests;
