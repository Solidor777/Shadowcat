use super::*;
use axum::routing::get;
use axum::Router;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

// -- extract_href_urls: scoped to a genuine <a> tag span -----------------

#[test]
fn extracts_href_from_a_genuine_anchor_tag() {
    let html = r#"<p>check <a href="http://example.test/x" rel="noopener">this</a> out</p>"#;
    assert_eq!(extract_href_urls(html), vec!["http://example.test/x"]);
}

#[test]
fn extracts_multiple_anchors_in_first_seen_order() {
    let html = r#"<a href="http://a.test/">a</a><a href="http://b.test/">b</a>"#;
    assert_eq!(
        extract_href_urls(html),
        vec!["http://a.test/", "http://b.test/"]
    );
}

#[test]
fn ignores_href_substring_not_inside_an_anchor_tag() {
    // Body text containing a literal `href="..."` substring with no
    // preceding `<a` tag open — the exact inert-prose case the SSRF fix
    // closes. Must yield zero candidate URLs.
    let html = r#"<p>see href="http://attacker.example/x" for details</p>"#;
    assert!(extract_href_urls(html).is_empty());
}

#[test]
fn ignores_non_anchor_tags_whose_name_starts_with_a() {
    // `<article>`/`<a-custom-element>` share the `<a` prefix but are not
    // anchor tags — must not be mistaken for one.
    let html = r#"<article href="http://attacker.example/">x</article><a-widget href="http://attacker.example/2">y</a-widget>"#;
    assert!(extract_href_urls(html).is_empty());
}

#[test]
fn ignores_anchor_tag_with_no_href_attribute() {
    let html = r#"<a name="anchor">no href here</a>"#;
    assert!(extract_href_urls(html).is_empty());
}

// -- is_blocked_ip: table-driven, one representative per named range ----

#[test]
fn blocks_every_named_ipv4_range() {
    let cases: &[&str] = &[
        "0.1.2.3",         // 0.0.0.0/8
        "10.1.2.3",        // 10/8
        "100.64.1.1",      // 100.64/10 CGNAT
        "127.0.0.1",       // 127/8
        "169.254.1.1",     // 169.254/16
        "172.16.5.5",      // 172.16/12
        "192.0.0.5",       // 192.0.0/24
        "192.0.2.5",       // TEST-NET-1
        "192.88.99.5",     // 6to4 relay
        "192.168.1.1",     // 192.168/16
        "198.18.0.5",      // benchmark
        "198.51.100.5",    // TEST-NET-2
        "203.0.113.5",     // TEST-NET-3
        "224.0.0.1",       // multicast
        "240.0.0.1",       // reserved
        "255.255.255.255", // limited broadcast
    ];
    for &ip in cases {
        let addr: IpAddr = ip.parse().unwrap();
        assert!(is_blocked_ip(addr), "{ip} should be blocked");
    }
}

#[test]
fn blocks_every_named_ipv6_range() {
    let cases: &[&str] = &[
        "::",                // unspecified
        "::1",               // loopback
        "::ffff:10.0.0.1",   // IPv4-mapped private
        "64:ff9b::10.0.0.1", // NAT64-mapped private
        "::127.0.0.1",       // ::/96 IPv4-compatible embedding loopback
        "::7f00:1",          // ::/96 IPv4-compatible embedding loopback (packed form)
        "2002:c0a8:0101::1", // 2002::/16 6to4 encapsulating 192.168.1.1
        "2002::1",           // 2002::/16 6to4 (blocked wholesale)
        "100::1",            // discard
        "2001:db8::1",       // documentation
        "fc00::1",           // unique-local
        "fd12:3456::1",      // unique-local (fd00::/8 subset)
        "fe80::1",           // link-local
        "ff02::1",           // multicast
    ];
    for &ip in cases {
        let addr: IpAddr = ip.parse().unwrap();
        assert!(is_blocked_ip(addr), "{ip} should be blocked");
    }
}

#[test]
fn allows_known_public_addresses() {
    let v4: IpAddr = "93.184.216.34".parse().unwrap();
    assert!(!is_blocked_ip(v4));
    let v6: IpAddr = "2606:2800:220:1:248:1893:25c8:1946".parse().unwrap();
    assert!(!is_blocked_ip(v6));
    // A public IPv4-mapped v6 must also unwrap and be allowed.
    let mapped: IpAddr = "::ffff:93.184.216.34".parse().unwrap();
    assert!(!is_blocked_ip(mapped));
}

// -- extract_preview: pure unit tests -----------------------------------

#[test]
fn prefers_og_over_title_and_meta() {
    let html = br#"<html><head>
        <title>Fallback Title</title>
        <meta property="og:title" content="OG Title">
        <meta name="description" content="Meta Description">
        <meta property="og:description" content="OG Description">
        <meta property="og:image" content="https://example.test/og.png">
    </head></html>"#;
    let extract = extract_preview(html).unwrap();
    assert_eq!(extract.title, "OG Title");
    assert_eq!(extract.description, "OG Description");
    assert_eq!(
        extract.image_url.as_deref(),
        Some("https://example.test/og.png")
    );
}

#[test]
fn falls_back_to_title_and_meta_description() {
    let html = br#"<html><head>
        <title>Just A Title</title>
        <meta name="description" content="Just a description">
    </head></html>"#;
    let extract = extract_preview(html).unwrap();
    assert_eq!(extract.title, "Just A Title");
    assert_eq!(extract.description, "Just a description");
    assert_eq!(extract.image_url, None);
}

#[test]
fn falls_back_to_link_image_src_when_no_og_image() {
    let html = br#"<html><head>
        <title>Just A Title</title>
        <link rel="image_src" href="/canonical.png">
    </head></html>"#;
    let extract = extract_preview(html).unwrap();
    assert_eq!(extract.image_url.as_deref(), Some("/canonical.png"));
}

#[test]
fn empty_document_yields_no_content() {
    assert!(extract_preview(b"<html><body>hello</body></html>").is_none());
    assert!(extract_preview(b"").is_none());
}

#[test]
fn decodes_common_entities() {
    let html = br#"<title>Fish &amp; Chips &lt;tasty&gt; &quot;deal&quot; &#39;now&#39;</title>"#;
    let extract = extract_preview(html).unwrap();
    assert_eq!(extract.title, "Fish & Chips <tasty> \"deal\" 'now'");
}

#[test]
fn collapses_whitespace_and_caps_length() {
    let long_title = "A".repeat(500);
    let html = format!("<title>{long_title}</title><meta name=\"description\" content=\"line1\n\n  line2   line3\">");
    let extract = extract_preview(html.as_bytes()).unwrap();
    assert_eq!(extract.title.chars().count(), MAX_TITLE_CHARS);
    assert_eq!(extract.description, "line1 line2 line3");
}

#[test]
fn malformed_html_never_panics() {
    let cases: &[&[u8]] = &[
        b"<title>unterminated",
        b"<<<>>>><meta property=",
        b"not html at all \x00\x01\x02",
        &[0xff, 0xfe, 0x00, b'<', b't'],
        b"<meta property=\"og:title\" content=\"unterminated",
        b"<title></title><title>second</title>",
    ];
    for case in cases {
        let _ = extract_preview(case);
    }
}

// -- fetch_preview: URL-validation-stage (no network) -------------------

#[tokio::test]
async fn rejects_non_http_schemes() {
    let client = build_client_allow_loopback();
    for scheme_url in [
        "file:///etc/passwd",
        "ftp://example.com/x",
        "gopher://example.com/x",
        "data:text/html,hi",
    ] {
        let err = fetch_preview(&client, scheme_url).await.unwrap_err();
        assert_eq!(err, PreviewError::BadScheme, "{scheme_url}");
    }
}

#[tokio::test]
async fn rejects_userinfo() {
    let client = build_client_allow_loopback();
    let err = fetch_preview(&client, "http://user:pass@example.com/")
        .await
        .unwrap_err();
    assert_eq!(err, PreviewError::BadScheme);
}

// -- fetch_preview: literal-IP hosts blocked in validate_url (never reach
//    the resolver — hyper short-circuits DNS for IP literals) ------------

#[tokio::test]
async fn rejects_literal_blocked_ip_hosts() {
    // `build_client_allow_loopback` is irrelevant here: validate_url blocks
    // these before any resolver/connection, so loopback allowance can't save
    // an IP-literal host. Each fails closed as BlockedAddress (not a connect).
    let client = build_client_allow_loopback();
    for url in [
        "http://169.254.169.254/", // link-local cloud-metadata endpoint
        "http://127.0.0.1/",       // loopback
        "http://[::1]/",           // v6 loopback
        "http://10.0.0.1/",        // RFC 1918 private
    ] {
        let err = fetch_preview(&client, url).await.unwrap_err();
        assert_eq!(err, PreviewError::BlockedAddress, "{url}");
    }
}

#[test]
fn url_crate_normalizes_numeric_ipv4_literals() {
    // url 2.5.8 applies WHATWG IPv4 host parsing to special (http) schemes:
    // a bare decimal or hex host normalizes to a Host::Ipv4, so validate_url's
    // IP-literal arm catches it (it does NOT fall through as a Domain).
    let loopback = Ipv4Addr::new(127, 0, 0, 1);
    assert_eq!(
        Url::parse("http://2130706433/").unwrap().host(),
        Some(Host::Ipv4(loopback)),
        "decimal 2130706433 must normalize to 127.0.0.1"
    );
    assert_eq!(
        Url::parse("http://0x7f000001/").unwrap().host(),
        Some(Host::Ipv4(loopback)),
        "hex 0x7f000001 must normalize to 127.0.0.1"
    );
}

#[tokio::test]
async fn rejects_normalized_decimal_ip_literal() {
    // http://2130706433/ == http://127.0.0.1/ after url normalization.
    let client = build_client_allow_loopback();
    let err = fetch_preview(&client, "http://2130706433/")
        .await
        .unwrap_err();
    assert_eq!(err, PreviewError::BlockedAddress);
}

#[test]
fn validate_url_allows_public_ip_literal() {
    // A literal PUBLIC IP passes validate_url; whether a connection succeeds
    // is a separate concern (no resolver is consulted for an IP literal).
    let url = Url::parse("http://93.184.216.34/").unwrap();
    assert!(validate_url(&url).is_ok());
}

#[test]
fn validate_url_rejects_file_scheme_at_hop() {
    // Pins the scheme guard independent of the redirect stub: a file:// URL
    // (as a redirect Location would resolve to) is BadScheme.
    let url = Url::parse("file:///etc/passwd").unwrap();
    assert_eq!(validate_url(&url), Err(PreviewError::BadScheme));
}

// -- fetch_preview: address guard, via the injectable resolve_fn seam ---

fn client_with_hosts(hosts: HashMap<&'static str, Vec<IpAddr>>) -> reqwest::Client {
    let resolver = GuardedResolver::with_resolve_fn(true, move |host| {
        hosts
            .get(host)
            .cloned()
            .ok_or_else(|| std::io::Error::other("unknown test host"))
    });
    build_client_with_timeouts(resolver, Duration::from_secs(3), Duration::from_secs(5))
}

#[tokio::test]
async fn rejects_a_host_that_resolves_to_a_blocked_address() {
    let mut hosts = HashMap::new();
    hosts.insert("blocked.test", vec!["10.0.0.5".parse().unwrap()]);
    let client = client_with_hosts(hosts);
    let err = fetch_preview(&client, "http://blocked.test/")
        .await
        .unwrap_err();
    assert_eq!(err, PreviewError::BlockedAddress);
}

#[tokio::test]
async fn rejects_mixed_public_and_private_resolution_all_or_nothing() {
    let mut hosts = HashMap::new();
    hosts.insert(
        "mixed.test",
        vec![
            "93.184.216.34".parse().unwrap(),
            "10.0.0.5".parse().unwrap(),
        ],
    );
    let client = client_with_hosts(hosts);
    let err = fetch_preview(&client, "http://mixed.test/")
        .await
        .unwrap_err();
    assert_eq!(err, PreviewError::BlockedAddress);
}

// -- fetch_image_bytes: shared guarded_get pipeline ----------------------
// Proves the guard applies identically to every `guarded_get` consumer,
// not just the original HTML path -- both the Content-Type/size gates
// AND the SSRF guard itself (literal-IP + resolved-address rejection).

#[tokio::test]
async fn fetch_image_bytes_succeeds_with_correct_content_type() {
    let router = Router::new().route(
        "/",
        get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "image/png")],
                vec![1u8, 2, 3],
            )
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let client = stub_client("stub.test");
    let (content_type, bytes) = fetch_image_bytes(
        &client,
        &format!("http://stub.test:{port}/"),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(content_type, "image/png");
    assert_eq!(bytes, vec![1, 2, 3]);
}

#[tokio::test]
async fn fetch_image_bytes_rejects_wrong_content_type() {
    let router = Router::new().route(
        "/",
        get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "text/html")],
                "not an image",
            )
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let client = stub_client("stub.test");
    let err = fetch_image_bytes(
        &client,
        &format!("http://stub.test:{port}/"),
        Duration::from_secs(5),
    )
    .await
    .unwrap_err();
    assert_eq!(err, PreviewError::NotHtml);
}

#[tokio::test]
async fn fetch_image_bytes_rejects_oversized_body() {
    let router = Router::new().route(
        "/",
        get(|| async {
            let body = vec![0u8; MAX_IMAGE_BYTES + 1024];
            ([(axum::http::header::CONTENT_TYPE, "image/png")], body)
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let client = stub_client("stub.test");
    let err = fetch_image_bytes(
        &client,
        &format!("http://stub.test:{port}/"),
        Duration::from_secs(5),
    )
    .await
    .unwrap_err();
    assert_eq!(err, PreviewError::TooLarge);
}

#[tokio::test]
async fn fetch_image_bytes_rejects_literal_blocked_ip_hosts() {
    // Same SECURITY-CRITICAL case as `rejects_literal_blocked_ip_hosts`,
    // re-run against `fetch_image_bytes` directly: the SSRF guard applies
    // identically to every `guarded_get` consumer.
    let client = build_client_allow_loopback();
    let err = fetch_image_bytes(&client, "http://169.254.169.254/", Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(err, PreviewError::BlockedAddress);
}

#[tokio::test]
async fn fetch_image_bytes_rejects_a_host_that_resolves_to_a_blocked_address() {
    let mut hosts = HashMap::new();
    hosts.insert("blocked.test", vec!["10.0.0.5".parse().unwrap()]);
    let client = client_with_hosts(hosts);
    let err = fetch_image_bytes(&client, "http://blocked.test/", Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(err, PreviewError::BlockedAddress);
}

// -- fetch_preview: against a real stub axum server on 127.0.0.1 --------
//
// `GuardedResolver::with_resolve_fn` maps a fake hostname to the stub
// server's real loopback IP; `allow_loopback: true` (test-only) lets that
// connection through the guard the way it never would in production.

async fn spawn_stub(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (addr.to_string(), handle)
}

fn stub_client(fake_host: &'static str) -> reqwest::Client {
    let resolver =
        GuardedResolver::with_resolve_fn(true, |_host| Ok(vec!["127.0.0.1".parse().unwrap()]));
    let _ = fake_host;
    build_client_with_timeouts(resolver, Duration::from_secs(3), Duration::from_secs(5))
}

#[tokio::test]
async fn good_html_yields_correct_preview_og_preferred() {
    let router = Router::new().route(
        "/",
        get(|| async {
            axum::response::Html(
                r#"<html><head>
                    <title>Fallback</title>
                    <meta property="og:title" content="Real Title">
                    <meta property="og:description" content="Real Description">
                </head></html>"#,
            )
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let client = stub_client("stub.test");
    let preview = fetch_preview(&client, &format!("http://stub.test:{port}/"))
        .await
        .unwrap();
    assert_eq!(preview.title, "Real Title");
    assert_eq!(preview.description, "Real Description");
}

#[tokio::test]
async fn empty_page_yields_no_content() {
    let router = Router::new().route("/", get(|| async { axum::response::Html("<html></html>") }));
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let client = stub_client("stub.test");
    let err = fetch_preview(&client, &format!("http://stub.test:{port}/"))
        .await
        .unwrap_err();
    assert_eq!(err, PreviewError::NoContent);
}

#[tokio::test]
async fn non_html_content_type_is_rejected() {
    let router = Router::new().route(
        "/",
        get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
                "binary",
            )
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let client = stub_client("stub.test");
    let err = fetch_preview(&client, &format!("http://stub.test:{port}/"))
        .await
        .unwrap_err();
    assert_eq!(err, PreviewError::NotHtml);
}

#[tokio::test]
async fn oversized_body_is_rejected() {
    let router = Router::new().route(
        "/",
        get(|| async {
            let body = "x".repeat(MAX_PREVIEW_BYTES + 1024);
            ([(axum::http::header::CONTENT_TYPE, "text/html")], body)
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let client = stub_client("stub.test");
    let err = fetch_preview(&client, &format!("http://stub.test:{port}/"))
        .await
        .unwrap_err();
    assert_eq!(err, PreviewError::TooLarge);
}

#[tokio::test]
async fn http_error_status_is_reported() {
    let router = Router::new().route(
        "/",
        get(|| async { (axum::http::StatusCode::NOT_FOUND, "nope") }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let client = stub_client("stub.test");
    let err = fetch_preview(&client, &format!("http://stub.test:{port}/"))
        .await
        .unwrap_err();
    assert_eq!(err, PreviewError::Http(404));
}

#[tokio::test]
async fn follows_redirects_up_to_the_cap_then_succeeds() {
    // 3 redirects (under the cap of 5), landing on real content.
    let router = Router::new()
        .route(
            "/hop0",
            get(|| async { axum::response::Redirect::to("/hop1") }),
        )
        .route(
            "/hop1",
            get(|| async { axum::response::Redirect::to("/hop2") }),
        )
        .route(
            "/hop2",
            get(|| async { axum::response::Redirect::to("/final") }),
        )
        .route(
            "/final",
            get(|| async { axum::response::Html("<title>Landed</title>") }),
        );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let client = stub_client("stub.test");
    let preview = fetch_preview(&client, &format!("http://stub.test:{port}/hop0"))
        .await
        .unwrap();
    assert_eq!(preview.title, "Landed");
}

#[tokio::test]
async fn exceeding_max_redirects_is_rejected() {
    // An infinite self-redirect: always exceeds the cap.
    let router = Router::new().route(
        "/loop",
        get(|| async { axum::response::Redirect::to("/loop") }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let client = stub_client("stub.test");
    let err = fetch_preview(&client, &format!("http://stub.test:{port}/loop"))
        .await
        .unwrap_err();
    assert_eq!(err, PreviewError::Redirects);
}

#[tokio::test]
async fn redirect_to_a_blocked_host_is_rejected_at_the_hop() {
    // The first hop is the real stub server (allowed via allow_loopback);
    // its Location points at a fake hostname the resolver maps to a
    // private IP — the guard must reject at that hop, never connecting.
    let router = Router::new().route(
        "/",
        get(|| async { axum::response::Redirect::to("http://evil.test/secret") }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();

    let mut hosts = HashMap::new();
    hosts.insert("evil.test", vec!["10.1.2.3".parse::<IpAddr>().unwrap()]);
    let stub_ip: IpAddr = "127.0.0.1".parse().unwrap();
    let resolver = GuardedResolver::with_resolve_fn(true, move |host| {
        if host == "evil.test" {
            Ok(hosts.get("evil.test").cloned().unwrap())
        } else {
            Ok(vec![stub_ip])
        }
    });
    let client =
        build_client_with_timeouts(resolver, Duration::from_secs(3), Duration::from_secs(5));

    let err = fetch_preview(&client, &format!("http://stub.test:{port}/"))
        .await
        .unwrap_err();
    assert_eq!(err, PreviewError::BlockedAddress);
}

#[tokio::test]
async fn redirect_to_a_non_http_scheme_is_rejected_at_the_hop() {
    // The first hop is the real stub (allowed via allow_loopback); its
    // Location is a file:// URL. This pins the HOP-LEVEL scheme guard: if the
    // per-hop validate_url were removed, the resolver would NOT catch this
    // (a file:// URL never resolves a host), so only the hop-level scheme
    // check produces BadScheme here.
    let router = Router::new().route(
        "/",
        get(|| async { axum::response::Redirect::to("file:///etc/passwd") }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let client = stub_client("stub.test");
    let err = fetch_preview(&client, &format!("http://stub.test:{port}/"))
        .await
        .unwrap_err();
    assert_eq!(err, PreviewError::BadScheme);
}

#[tokio::test]
async fn redirect_chain_exceeding_the_total_deadline_times_out() {
    // Each hop delays; the per-request reqwest timeout is long (5s) so no
    // single hop times out, but the outer deadline bounds the WHOLE chain.
    // Proves the deadline is total, not per-hop: a chain that would burn
    // well past the deadline (yet stay under MAX_REDIRECTS pacing) yields
    // Timeout, not Redirects.
    let router = Router::new().route(
        "/slow",
        get(|| async {
            tokio::time::sleep(Duration::from_millis(60)).await;
            axum::response::Redirect::to("/slow")
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();

    // Long per-request timeout (no single hop times out); the injected outer
    // deadline (120ms) fires after ~2 hops, before MAX_REDIRECTS is reached.
    let resolver =
        GuardedResolver::with_resolve_fn(true, |_host| Ok(vec!["127.0.0.1".parse().unwrap()]));
    let client =
        build_client_with_timeouts(resolver, Duration::from_secs(3), Duration::from_secs(5));

    let err = fetch_preview_with_deadline(
        &client,
        &format!("http://stub.test:{port}/slow"),
        Duration::from_millis(120),
    )
    .await
    .unwrap_err();
    assert_eq!(err, PreviewError::Timeout);
}

#[tokio::test]
async fn slow_target_times_out() {
    let router = Router::new().route(
        "/",
        get(|| async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            axum::response::Html("<title>too slow</title>")
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();

    // A test-only client with a much shorter total timeout than
    // production's 5s, so this test stays fast while still proving the
    // total-timeout guard fires (Timeout, not a hang).
    let resolver =
        GuardedResolver::with_resolve_fn(true, |_host| Ok(vec!["127.0.0.1".parse().unwrap()]));
    let client =
        build_client_with_timeouts(resolver, Duration::from_secs(3), Duration::from_millis(50));

    let err = fetch_preview(&client, &format!("http://stub.test:{port}/"))
        .await
        .unwrap_err();
    assert_eq!(err, PreviewError::Timeout);
}

#[tokio::test]
async fn repeated_calls_do_not_share_hidden_state() {
    // Sanity check that the client/resolver has no shared mutable state
    // that would leak between concurrent fetches (the ingest stage fetches
    // up to 3 URLs concurrently via a JoinSet in the follow-up task).
    static HITS: AtomicUsize = AtomicUsize::new(0);
    let router = Router::new().route(
        "/",
        get(|| async {
            HITS.fetch_add(1, Ordering::SeqCst);
            axum::response::Html("<title>ok</title>")
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let client = stub_client("stub.test");
    let url = format!("http://stub.test:{port}/");
    let (a, b) = tokio::join!(fetch_preview(&client, &url), fetch_preview(&client, &url));
    assert!(a.is_ok());
    assert!(b.is_ok());
    assert_eq!(HITS.load(Ordering::SeqCst), 2);
}

// -- cached_or_fetch: two-tier cache lookup ------------------------------

#[tokio::test]
async fn cached_or_fetch_hits_in_memory_tier_without_touching_repo() {
    let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:")
        .await
        .unwrap();
    let cache = LinkPreviewCache::new();
    let now = Instant::now();
    // Never upserted into the DB table; the in-memory tier must still
    // resolve it, proving in-memory precedence over the persisted tier.
    let url = "https://mem-hit.example/";
    let preview = LinkPreview {
        url: url.to_string(),
        title: "t".to_string(),
        description: "d".to_string(),
        image_url: None,
        image_asset_id: None,
    };
    cache.insert(url.to_string(), Some(preview.clone()), now);

    let result = cached_or_fetch(&repo, &cache, url, now, 1_000).await;
    assert_eq!(result, Some(Some(preview)));
}

#[tokio::test]
async fn cached_or_fetch_cold_start_falls_through_to_persisted_row() {
    let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:")
        .await
        .unwrap();
    let cache = LinkPreviewCache::new();
    let now = Instant::now();
    let url = "https://cold.example/";
    repo.upsert_link_preview_cache(url, Some("T"), Some("D"), 1_000)
        .await
        .unwrap();

    let first = cached_or_fetch(&repo, &cache, url, now, 1_000).await;
    assert_eq!(
        first,
        Some(Some(LinkPreview {
            url: url.to_string(),
            title: "T".to_string(),
            description: "D".to_string(),
            image_url: None,
            image_asset_id: None,
        }))
    );

    // Mutate the underlying row directly; a second call must still
    // return the FIRST value, proving it now comes from the in-memory
    // tier the first call backfilled rather than re-reading the DB.
    repo.upsert_link_preview_cache(url, Some("CHANGED"), Some("CHANGED"), 1_000)
        .await
        .unwrap();
    let second = cached_or_fetch(&repo, &cache, url, now, 1_000).await;
    assert_eq!(second, first);
}

#[tokio::test]
async fn cached_or_fetch_ttl_expired_persisted_row_falls_through_to_miss() {
    let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:")
        .await
        .unwrap();
    let cache = LinkPreviewCache::new();
    let now = Instant::now();
    let url = "https://stale.example/";
    let fetched_at_ms = 1_000;
    repo.upsert_link_preview_cache(url, Some("T"), Some("D"), fetched_at_ms)
        .await
        .unwrap();

    let expired_now_ms = fetched_at_ms + POSITIVE_TTL.as_millis() as i64;
    let result = cached_or_fetch(&repo, &cache, url, now, expired_now_ms).await;
    assert_eq!(
        result, None,
        "caller must fetch on an expired persisted row"
    );
}

#[tokio::test]
async fn cached_or_fetch_negative_row_honors_negative_ttl() {
    let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:")
        .await
        .unwrap();
    let now = Instant::now();
    let url = "https://negative.example/";
    let fetched_at_ms = 1_000;
    repo.upsert_link_preview_cache(url, None, None, fetched_at_ms)
        .await
        .unwrap();

    let live = cached_or_fetch(&repo, &LinkPreviewCache::new(), url, now, fetched_at_ms).await;
    assert_eq!(live, Some(None));

    let expired_now_ms = fetched_at_ms + NEGATIVE_TTL.as_millis() as i64;
    let expired = cached_or_fetch(&repo, &LinkPreviewCache::new(), url, now, expired_now_ms).await;
    assert_eq!(expired, None);
}

// -- enrich: fresh fetch writes through both cache tiers -----------------

#[tokio::test]
async fn enrich_fresh_fetch_writes_through_both_tiers() {
    let router = Router::new().route(
        "/",
        get(|| async {
            axum::response::Html(
                r#"<html><head>
                    <title>Fallback</title>
                    <meta property="og:title" content="Fresh Title">
                    <meta property="og:description" content="Fresh Description">
                </head></html>"#,
            )
        }),
    );
    let (addr, _handle) = spawn_stub(router).await;
    let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();
    let client = stub_client("stub.test");
    let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:")
        .await
        .unwrap();
    let cache = LinkPreviewCache::new();
    let rate = PreviewRateLimiter::new();
    let url = format!("http://stub.test:{port}/");
    let mut segments = vec![Segment::Html {
        sanitized_html: format!(r#"<a href="{url}">link</a>"#),
    }];
    let now = Instant::now();

    enrich(
        &mut segments,
        EnrichDeps {
            repo: &repo,
            fetch: LinkPreviewDeps {
                client: &client,
                cache: &cache,
                rate: &rate,
            },
        },
        Uuid::new_v4(),
        1_000,
        now,
    )
    .await;

    assert!(cache.get(&url, now).is_some(), "in-memory tier not written");
    let row = repo
        .get_link_preview_cache(&url)
        .await
        .unwrap()
        .expect("persisted tier not written");
    assert_eq!(row.title.as_deref(), Some("Fresh Title"));
    assert_eq!(row.description.as_deref(), Some("Fresh Description"));
}

// -- link_preview_cache repository methods -------------------------------

#[tokio::test]
async fn upsert_link_preview_cache_preserves_existing_image_asset_id_on_conflict() {
    use crate::auth::role::ServerRole;
    use crate::data::asset::Asset;

    let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:")
        .await
        .unwrap();
    let owner = repo
        .create_user("u", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let asset_id = Uuid::new_v4();
    let asset = Asset {
        id: asset_id,
        world_id: world.id,
        storage_key: format!("{}/{asset_id}", world.id),
        original_name: "og.png".to_string(),
        content_type: "image/png".to_string(),
        byte_size: 10,
        created_by: Some(owner),
        created_at: 0,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: crate::data::asset::AssetMeta::unprocessed("image/png", 1),
    };
    repo.insert_asset(&asset).await.unwrap();

    let url = "https://image.example/";
    repo.upsert_link_preview_cache(url, Some("Title One"), Some("Desc One"), 1_000)
        .await
        .unwrap();
    repo.set_link_preview_cache_image(url, asset_id)
        .await
        .unwrap();
    repo.upsert_link_preview_cache(url, Some("Title Two"), Some("Desc Two"), 2_000)
        .await
        .unwrap();

    let row = repo.get_link_preview_cache(url).await.unwrap().unwrap();
    assert_eq!(row.image_asset_id, Some(asset_id));
    assert_eq!(row.title.as_deref(), Some("Title Two"));
    assert_eq!(row.description.as_deref(), Some("Desc Two"));
}

#[tokio::test]
async fn set_link_preview_cache_image_is_a_noop_on_absent_row() {
    let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:")
        .await
        .unwrap();
    let url = "https://never-upserted.example/";
    repo.set_link_preview_cache_image(url, Uuid::new_v4())
        .await
        .unwrap();
    assert_eq!(repo.get_link_preview_cache(url).await.unwrap(), None);
}
