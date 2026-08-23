//! oEmbed provider embeds: allowlisted provider HOSTS only, never
//! autodiscovery (`<link rel="alternate" type="application/json+oembed">`
//! against an arbitrary posted URL would reintroduce the arbitrary-host-
//! fetch risk this feature must avoid). Structured fields only — `title`,
//! `author_name`, `provider_name`, `thumbnail_asset_id` — never a
//! provider's raw `html` field, which is third-party-controlled markup and
//! a direct stored-XSS vector this server does not control.

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
/// never raw body-text substrings). No I/O: this check IS the entire SSRF
/// mitigation for oEmbed — a URL failing it falls through to the existing
/// generic `LinkPreview` scrape unchanged; there is no autodiscovery fetch
/// anywhere in this module.
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
    /// The URL as posted in the message.
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
mod tests {
    use super::*;

    #[test]
    fn match_provider_recognizes_every_allowlisted_youtube_host() {
        let hosts = [
            "youtube.com",
            "www.youtube.com",
            "m.youtube.com",
            "youtu.be",
        ];
        for host in hosts {
            let url = format!("https://{host}/watch?v=abc123");
            assert_eq!(
                match_provider(&url),
                Some(OEmbedProvider::YouTube),
                "expected {url} to match YouTube"
            );
        }
    }

    #[test]
    fn match_provider_recognizes_every_allowlisted_vimeo_host() {
        let hosts = ["vimeo.com", "www.vimeo.com", "player.vimeo.com"];
        for host in hosts {
            let url = format!("https://{host}/123456789");
            assert_eq!(
                match_provider(&url),
                Some(OEmbedProvider::Vimeo),
                "expected {url} to match Vimeo"
            );
        }
    }

    #[test]
    fn match_provider_rejects_non_allowlisted_hosts() {
        let cases = [
            "https://youtube.com.attacker.example/watch?v=abc",
            "https://notyoutube.com/watch?v=abc",
            "https://192.168.1.1/watch?v=abc",
            "https://example.com/",
        ];
        for url in cases {
            assert_eq!(match_provider(url), None, "expected {url} to not match");
        }
    }

    #[test]
    fn match_provider_rejects_non_http_schemes() {
        assert_eq!(match_provider("ftp://youtube.com/watch?v=abc"), None);
        assert_eq!(match_provider("javascript:alert(document.domain)"), None);
    }

    #[test]
    fn endpoint_carries_posted_url_as_query_param_on_the_fixed_provider_host() {
        let posted = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        let endpoint = OEmbedProvider::YouTube.endpoint(posted).unwrap();
        let parsed = url::Url::parse(&endpoint).unwrap();
        assert_eq!(parsed.host_str(), Some("www.youtube.com"));
        assert!(parsed.query_pairs().any(|(k, v)| k == "url" && v == posted));

        let endpoint = OEmbedProvider::Vimeo.endpoint(posted).unwrap();
        let parsed = url::Url::parse(&endpoint).unwrap();
        assert_eq!(parsed.host_str(), Some("vimeo.com"));
        assert!(parsed.query_pairs().any(|(k, v)| k == "url" && v == posted));
    }

    /// A `posted_url` crafted to LOOK like it carries its own `url=`/`host=`
    /// query keys must not smuggle a second query parameter, redirect the
    /// endpoint host, or otherwise escape the query VALUE it is percent-
    /// encoded into — the entire SSRF mitigation for this module rests on
    /// the endpoint host staying one of the two fixed base strings above,
    /// never anything derived from attacker-controlled input.
    #[test]
    fn endpoint_percent_encodes_a_posted_url_that_looks_like_extra_query_params() {
        let hostile = "https://x.example/?url=https://evil.example&host=attacker.example";
        let endpoint = OEmbedProvider::YouTube.endpoint(hostile).unwrap();
        let parsed = url::Url::parse(&endpoint).unwrap();
        assert_eq!(
            parsed.host_str(),
            Some("www.youtube.com"),
            "a crafted posted_url must never change the endpoint host"
        );
        let url_pairs: Vec<_> = parsed.query_pairs().filter(|(k, _)| k == "url").collect();
        assert_eq!(
            url_pairs.len(),
            1,
            "exactly one url= pair must exist, not a smuggled second one"
        );
        assert_eq!(url_pairs[0].1, hostile);
        assert!(
            parsed.query_pairs().all(|(k, _)| k != "host"),
            "a host= key embedded in posted_url must not appear as its own query pair"
        );
    }

    /// The security-critical test: a provider's raw `html` field (a direct
    /// stored-XSS vector) must never reach the deserialized `OEmbedResponse`
    /// in any observable way, and by construction can never reach the
    /// downstream `OEmbedSegment` either — proven here by round-tripping
    /// through BOTH types and asserting the markup substrings are absent
    /// from the final serialized output, not merely by enumerating fields.
    #[test]
    fn oembed_response_deserialize_drops_html_field_entirely() {
        let fixture = r#"{
            "title": "A Video",
            "author_name": "Someone",
            "thumbnail_url": "https://example.test/thumb.jpg",
            "html": "<script>alert(1)</script><iframe src=\"https://attacker.example/\"></iframe>",
            "provider_name": "SomeProvider",
            "width": 1920,
            "height": 1080
        }"#;
        let parsed: OEmbedResponse =
            serde_json::from_str(fixture).expect("unknown html field must not fail deserialize");
        assert_eq!(parsed.title.as_deref(), Some("A Video"));

        let segment = OEmbedSegment {
            url: "https://www.youtube.com/watch?v=abc".into(),
            provider_name: OEmbedProvider::YouTube.name().to_string(),
            title: parsed.title,
            author_name: parsed.author_name,
            thumbnail_asset_id: None,
        };
        let serialized = serde_json::to_string(&segment).unwrap();
        assert!(
            !serialized.contains("<script"),
            "serialized OEmbedSegment must never carry a provider's raw <script markup: {serialized}"
        );
        assert!(
            !serialized.contains("<iframe"),
            "serialized OEmbedSegment must never carry a provider's raw <iframe markup: {serialized}"
        );
    }
}
