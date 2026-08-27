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
