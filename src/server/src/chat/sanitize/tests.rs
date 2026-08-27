use crate::chat::{sanitize, ChatContentPolicy, Segment};

fn off() -> ChatContentPolicy {
    ChatContentPolicy::default()
}

fn md() -> ChatContentPolicy {
    ChatContentPolicy {
        markdown: Some(true),
        ..off()
    }
}

fn html_on() -> ChatContentPolicy {
    ChatContentPolicy {
        html: Some(true),
        ..off()
    }
}

fn hyperlinks_on() -> ChatContentPolicy {
    ChatContentPolicy {
        html: Some(true),
        hyperlinks: Some(true),
        ..off()
    }
}

fn images_on() -> ChatContentPolicy {
    ChatContentPolicy {
        html: Some(true),
        images: Some(true),
        ..off()
    }
}

/// Concatenates a `&[Segment]` into a single string (Text verbatim, Html's
/// `sanitized_html`) so assertions can read uniformly regardless of which
/// segment variant `sanitize` produced.
fn render(segs: &[Segment]) -> String {
    segs.iter()
        .map(|s| match s {
            Segment::Text { text } => text.clone(),
            Segment::Html { sanitized_html } => sanitized_html.clone(),
            // `sanitize()` (the function under test) never produces a
            // roll, link-preview, oembed, or doc-link segment -- those are
            // `chat::rolls`'s, `chat::link_preview::enrich`'s, and
            // `chat::post_publish`'s own producers.
            Segment::RollEmbed { .. }
            | Segment::RollButton { .. }
            | Segment::LinkPreview { .. }
            | Segment::OEmbed(_)
            | Segment::DocLink { .. } => {
                unreachable!(
                    "sanitize() never produces roll, preview, oembed, or doc-link segments"
                )
            }
        })
        .collect()
}

/// The shortcode pre-pass runs identically on the plain-text early-return
/// path (the enriched path is exercised via `shortcodes`'s own tests).
#[test]
fn sanitize_replaces_shortcodes_in_plain_text_mode() {
    let policy = ChatContentPolicy::default(); // everything off
    let out = sanitize("gg :heart:", &policy);
    assert_eq!(
        out,
        vec![Segment::Text {
            text: "gg ❤️".into()
        }]
    );
}

#[test]
fn all_off_is_plain_text() {
    assert_eq!(
        sanitize("**bold** <b>x</b>", &off()),
        vec![Segment::Text {
            text: "**bold** <b>x</b>".into()
        }],
    );
}

#[test]
fn markdown_renders_to_sanitized_html_run() {
    let segs = sanitize("**bold**", &md());
    match segs.as_slice() {
        [Segment::Html { sanitized_html }] => {
            assert!(
                sanitized_html.contains("<strong>bold</strong>"),
                "got {sanitized_html}"
            );
        }
        other => panic!("expected one Html run, got {other:?}"),
    }
}

#[test]
fn script_tag_is_neutralized() {
    for policy in [md(), html_on()] {
        let out = render(&sanitize("<script>alert(1)</script>hi", &policy));
        assert!(
            !out.contains("<script"),
            "script survived under {policy:?}: {out}"
        );
        assert!(
            !out.to_lowercase().contains("alert(1)") || !out.contains("<script"),
            "script payload survived under {policy:?}: {out}"
        );
    }
}

#[test]
fn event_handler_and_js_url_stripped() {
    let out = render(&sanitize(
        r#"<a href="javascript:alert(1)" onclick="evil()">x</a>"#,
        &html_on(),
    ));
    assert!(!out.contains("javascript:"), "js url survived: {out}");
    assert!(!out.contains("onclick"), "event handler survived: {out}");
}

#[test]
fn css_is_always_stripped_even_when_html_on() {
    let out = render(&sanitize(
        r#"<b style="expression(x)">x</b><style>*{}</style>"#,
        &html_on(),
    ));
    assert!(!out.contains("style"), "style survived: {out}");
    assert!(!out.contains("expression"), "css survived: {out}");
}

#[test]
fn raw_html_in_markdown_escaped_when_html_off() {
    // markdown ON, html OFF: the author's raw <b> must NOT become a live tag.
    let out = render(&sanitize("hi <b>x</b>", &md()));
    assert!(
        !out.contains("<b>"),
        "raw html leaked through markdown-only: {out}"
    );
}

/// NOT a general-purpose HTML liveness parser. Sound only when `html` is
/// already ammonia-cleaned (or fully HTML-escaped) output, where every
/// surviving raw `<`/`>` genuinely delimits a real tag boundary; a
/// pre-sanitization string could contain a literal `&gt;` *inside* a
/// quoted attribute value and cause a false "not live" verdict. Use
/// direct substring/exact-output assertions instead when a needle could
/// legitimately appear inside quoted attribute text (see the
/// `surviving_*`/`protocol_relative_url_is_denied` tests above).
///
/// True if `needle` occurs inside a LIVE (unescaped) HTML tag in `html` —
/// i.e. after the nearest preceding raw `<` that has no `>` between it
/// and `needle`'s position. An occurrence that is only ever raw text
/// (e.g. inert display text produced by escaping `<` to `&lt;`, which
/// never leaves a bare `<` behind) returns `false`. This is the precise
/// liveness check the corpus needs: harmless *display* of an attack
/// string as literal chat text (the sanitizer's correct, lossless
/// behavior for raw markup a policy disallows) must not be confused with
/// the string surviving as an executable attribute of a real element.
fn appears_in_live_tag(html: &str, needle: &str) -> bool {
    let mut search_from = 0;
    while let Some(rel) = html[search_from..].find(needle) {
        let pos = search_from + rel;
        let before = &html[..pos];
        if let Some(lt) = before.rfind('<') {
            if !before[lt + 1..].contains('>') {
                return true; // `needle` sits inside an open, unescaped tag
            }
        }
        search_from = pos + needle.len().max(1);
    }
    false
}

/// Proves attribute-stripping / URL-scheme filtering, not just tag
/// removal: `hyperlinks: true` lets `<a>` SURVIVE ammonia's tag
/// whitelist, so this actually exercises the `javascript:` scheme
/// rejection and `onclick` attribute stripping rather than relying on
/// `rm_tags("a")` to remove the whole carrier tag.
#[test]
fn surviving_anchor_strips_js_scheme_and_event_handler() {
    let out = render(&sanitize(
        r#"<a href="javascript:alert(1)" onclick="evil()">x</a>"#,
        &hyperlinks_on(),
    ));
    assert!(out.contains("x"), "anchor text content lost: {out}");
    assert!(out.contains("<a"), "anchor tag did not survive: {out}");
    assert!(!out.contains("javascript:"), "js url survived: {out}");
    assert!(!out.contains("onclick"), "event handler survived: {out}");
}

/// Same proof for `<img>`: `images: true` lets the tag survive, so the
/// `javascript:`/`data:text/html` scheme rejection is what must do the
/// work, not tag removal.
#[test]
fn surviving_img_strips_dangerous_schemes() {
    for src in [
        "javascript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
    ] {
        let out = render(&sanitize(&format!(r#"<img src="{src}">"#), &images_on()));
        assert!(!out.contains("javascript:"), "js url survived: {out}");
        assert!(
            !out.contains("data:text/html"),
            "data:text/html url survived: {out}"
        );
    }
}

/// Same proof for a generic surviving tag's event-handler attribute
/// (not URL-scheme-specific): `<b>` is in ammonia's default whitelist
/// under `html_on()`, so it survives; `onclick` must still be stripped.
#[test]
fn surviving_bold_tag_strips_event_handler() {
    let out = render(&sanitize(r#"<b onclick="alert(1)">bold</b>"#, &html_on()));
    assert!(out.contains("bold"), "bold text content lost: {out}");
    assert!(!out.contains("onclick"), "event handler survived: {out}");
}

/// Protocol-relative URLs (`//host/path`) have no scheme at all, so
/// ammonia's `url_schemes` allowlist never sees them — they are governed
/// solely by `url_relative`, which defaults to `PassThrough` (lets them
/// through unchanged). `ammonia_for` must set `UrlRelative::Deny` or a
/// smuggled `<img src="//evil.example/pixel.gif">` in a whispered/
/// GM-only message becomes a tracking pixel that fires for every
/// recipient. Assert the relative URL does not survive (either the
/// attribute or the whole tag is dropped — assert what ammonia actually
/// does, not an assumption).
#[test]
fn protocol_relative_url_is_denied() {
    let out = render(&sanitize(
        r#"<img src="//evil.example/pixel.gif">"#,
        &images_on(),
    ));
    assert!(
        !out.contains("//evil.example"),
        "protocol-relative url survived: {out}"
    );

    let out = render(&sanitize(
        r#"<a href="//evil.example">x</a>"#,
        &hyperlinks_on(),
    ));
    assert!(
        !out.contains("//evil.example"),
        "protocol-relative url survived: {out}"
    );
}

/// `images: false` (the `md()` default) must strip `<img>` entirely, not
/// just leave a src-less tag — proves the tag-level `rm_tags` gate (not
/// the content-type filter, which only runs when `images` is true).
#[test]
fn images_off_strips_img() {
    let out = render(&sanitize("![a](https://x.example/a.png)", &md()));
    assert!(!out.contains("<img"), "img survived with images off: {out}");
}

/// Image `src` must be an allowlisted raster extension: an `https` `.png`
/// survives with its `src` intact, but a non-image extension (`.exe`) is
/// rejected — either the whole `<img>` is dropped or `src` is stripped,
/// so this asserts on the URL's absence rather than a specific shape.
#[test]
fn images_on_allows_https_png_only() {
    let p = ChatContentPolicy {
        markdown: Some(true),
        images: Some(true),
        ..Default::default()
    };
    let ok = render(&sanitize("![a](https://x.example/a.png)", &p));
    assert!(ok.contains("<img"), "png image dropped: {ok}");
    assert!(
        ok.contains("x.example/a.png"),
        "png src not preserved: {ok}"
    );
    let bad = render(&sanitize("![a](https://x.example/a.exe)", &p));
    assert!(
        !bad.contains("x.example/a.exe"),
        "non-image src survived: {bad}"
    );
}

/// Case variation in the extension must not bypass the filter — the
/// filter lowercases before matching.
#[test]
fn images_on_extension_match_is_case_insensitive() {
    let p = ChatContentPolicy {
        markdown: Some(true),
        images: Some(true),
        ..Default::default()
    };
    let ok = render(&sanitize("![a](https://x.example/a.PNG)", &p));
    assert!(ok.contains("<img"), "uppercase PNG dropped: {ok}");
    assert!(
        ok.contains("x.example/a.PNG"),
        "uppercase PNG src not preserved: {ok}"
    );
}

/// A query string or fragment appended after a disallowed extension must
/// not smuggle a fake image extension earlier in the URL past the
/// filter — the filter strips `?`/`#` suffixes before checking the
/// extension, so `a.exe?x=.png` is still rejected as `.exe`.
#[test]
fn images_on_rejects_extension_smuggled_via_query_string() {
    let p = ChatContentPolicy {
        markdown: Some(true),
        images: Some(true),
        ..Default::default()
    };
    let bad = render(&sanitize("![a](https://x.example/a.exe?x=.png)", &p));
    assert!(
        !bad.contains("x.example/a.exe"),
        "smuggled extension bypassed filter: {bad}"
    );
}

/// A URL with no extension at all (or no path) must be rejected, not
/// default-allowed.
#[test]
fn images_on_rejects_missing_extension() {
    let p = ChatContentPolicy {
        markdown: Some(true),
        images: Some(true),
        ..Default::default()
    };
    let bad = render(&sanitize("![a](https://x.example/a)", &p));
    assert!(
        !bad.contains("x.example/a\""),
        "extensionless src survived: {bad}"
    );
}

/// `hyperlinks: false` must unwrap the anchor to its inner text (content
/// preserved, tag gone) rather than leaving a dead `<a>` around.
#[test]
fn hyperlinks_off_unwraps_anchor_to_text() {
    let out = render(&sanitize("[label](https://x.example)", &md()));
    assert!(
        !out.contains("<a "),
        "anchor survived with hyperlinks off: {out}"
    );
    assert!(out.contains("label"), "anchor text lost: {out}");
}

/// `emails` toggle gates whether `mailto:` survives as a URL scheme,
/// independent of the `hyperlinks` toggle (already true in both cases).
#[test]
fn emails_toggle_gates_mailto() {
    let off = ChatContentPolicy {
        html: Some(true),
        hyperlinks: Some(true),
        ..Default::default()
    };
    assert!(!render(&sanitize(r#"<a href="mailto:a@b.example">m</a>"#, &off)).contains("mailto:"));
    let on = ChatContentPolicy {
        html: Some(true),
        hyperlinks: Some(true),
        emails: Some(true),
        ..Default::default()
    };
    assert!(render(&sanitize(r#"<a href="mailto:a@b.example">m</a>"#, &on)).contains("mailto:"));
}

/// Known XSS vectors, each asserted neutral under BOTH `md()` and
/// `html_on()`: no LIVE `on*` handler attribute, no live `<script`/
/// `<iframe` tag, and no live `javascript:`/`data:text/html` URL scheme.
/// "Live" (see `appears_in_live_tag`) — as opposed to a bare substring
/// check — because a policy that disallows raw HTML correctly renders an
/// attack string as inert, HTML-escaped DISPLAY TEXT rather than deleting
/// it; that inert text may still contain the attack string's characters
/// without being executable.
#[test]
fn xss_corpus_neutralized() {
    let vectors = [
        "<img src=x onerror=alert(1)>",
        "<svg/onload=alert(1)>",
        "<iframe src=\"javascript:alert(1)\"></iframe>",
        "<a href=\"data:text/html,<script>alert(1)</script>\">click</a>",
        "<body onload=alert(1)>",
        "<math><mtext></mtext></math>",
    ];
    for policy in [md(), html_on()] {
        for v in vectors {
            let out = render(&sanitize(v, &policy));
            let low = out.to_lowercase();
            for needle in ["onerror", "onload", "onclick"] {
                assert!(
                    !appears_in_live_tag(&low, needle),
                    "live {needle} handler survived ({policy:?}) for {v}: {out}"
                );
            }
            assert!(
                !low.contains("<script"),
                "<script survived ({policy:?}) for {v}: {out}"
            );
            assert!(
                !low.contains("<iframe"),
                "<iframe survived ({policy:?}) for {v}: {out}"
            );
            for scheme in ["javascript:", "data:text/html"] {
                assert!(
                    !appears_in_live_tag(&low, scheme),
                    "live {scheme} scheme survived ({policy:?}) for {v}: {out}"
                );
            }
        }
    }
}
