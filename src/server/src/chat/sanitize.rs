//! Message-content sanitizer: the single security boundary between raw user
//! input and a stored `Segment::Html` run. `sanitize` is the ONLY producer of
//! `Segment::Html` (see `Segment`'s doc comment) — every enrichment path
//! (Markdown rendering, raw HTML passthrough) funnels through one `ammonia`
//! `clean()` call here before anything is persisted or broadcast.

use crate::chat::{ChatContentPolicy, Segment};
use pulldown_cmark::{html, Event, Options, Parser};

/// Enrich raw user input into a sanitized `Segment` list under `policy`.
/// INVARIANT: the ONLY producer of `Segment::Html`. `ammonia` is the single
/// security boundary, crossed exactly once here. All-off => one `Text`
/// segment (fail-closed baseline, identical to c-1's `plain_text_content`).
pub fn sanitize(raw: &str, policy: &ChatContentPolicy) -> Vec<Segment> {
    // `:shortcode:` -> unicode pre-pass, ahead of BOTH branches below, so
    // stored content is final and identical regardless of the markdown/html
    // policy toggles — always-on typing sugar, not policy-gated enrichment.
    let replaced = super::shortcodes::replace_shortcodes(raw);
    let raw: &str = &replaced;
    if !policy.markdown && !policy.html {
        return vec![Segment::Text {
            text: raw.to_string(),
        }];
    }
    // Produce an HTML string, then hand the WHOLE thing to ammonia once.
    let html_input = if policy.markdown {
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_TABLES);
        let parser = Parser::new_ext(raw, opts).map(|ev| {
            // When raw HTML is not allowed, DOWNGRADE cmark's raw-HTML events to
            // plain Text carrying the same literal characters, rather than
            // dropping them. `html::push_html` HTML-escapes every Text event
            // (`<`/`>`/`&`/quotes), so the author's embedded tag becomes inert,
            // visible, escaped text (e.g. `<b>` -> `&lt;b&gt;`) instead of
            // silently vanishing — content is never lost, and no raw `<`
            // character from user input can ever reach ammonia as live markup.
            if !policy.html {
                match ev {
                    Event::Html(s) | Event::InlineHtml(s) => Event::Text(s),
                    other => other,
                }
            } else {
                ev
            }
        });
        let mut s = String::new();
        html::push_html(&mut s, parser);
        s
    } else {
        // html-only: feed the raw input straight to ammonia.
        raw.to_string()
    };
    let cleaned = ammonia_for(policy).clean(&html_input).to_string();
    vec![Segment::Html {
        sanitized_html: cleaned,
    }]
}

/// Build the ammonia sanitizer for `policy`. ammonia's DEFAULT already strips
/// `<script>`/`<style>` (`clean_content_tags`), the `style` attribute (never
/// whitelisted on any tag or as a generic attribute), and non-allowlisted URL
/// schemes (`javascript:`/`data:` are absent from the default scheme set).
/// This only NARROWS the default further per toggle — it never widens beyond
/// http/https(/mailto).
fn ammonia_for(policy: &ChatContentPolicy) -> ammonia::Builder<'static> {
    use std::collections::HashSet;
    let mut b = ammonia::Builder::default();
    // CSS is never permitted. Belt-and-suspenders over ammonia's default:
    // explicitly re-remove the `style` tag/attribute so a future change to
    // the tag or generic-attribute whitelist above cannot silently
    // reintroduce it. `rm_tag_attributes` is scoped per-tag (there is no "*"
    // wildcard in this API), so it is applied to every currently-whitelisted
    // tag.
    b.rm_tags(std::iter::once("style"));
    b.rm_generic_attributes(std::iter::once("style"));
    for tag in b.clone_tags() {
        b.rm_tag_attributes(tag, std::iter::once("style"));
    }
    // ammonia's default `UrlRelative::PassThrough` lets a schemeless,
    // protocol-relative URL (`//evil.example/pixel.gif`) through unfiltered
    // -- `url_schemes` below never sees it (there is no scheme to check).
    // Against Shadowcat's whispered/GM-only messages this is a live privacy
    // leak: a smuggled tracking pixel fires for every restricted-audience
    // recipient. Deny relative URLs outright; only the http/https(/mailto)
    // absolute schemes below are ever permitted.
    b.url_relative(ammonia::UrlRelative::Deny);
    if !policy.images {
        b.rm_tags(std::iter::once("img"));
    } else {
        // Image `src` must resolve to an allowlisted raster extension
        // (png/jpg/jpeg/webp/gif). The check runs over the whole
        // query/fragment-stripped `src` string (`lower.split(['?', '#'])`
        // only removes the trailing `?query`/`#fragment`; scheme and host
        // are NOT stripped first), so `a.exe?x=.png` cannot smuggle a fake
        // extension past the check, but a host that itself ends in an
        // allowlisted extension (e.g. `https://evil.png`) would also pass —
        // low real exploitability since image-extension strings are not
        // delegated public TLDs, so such a host is not publicly resolvable
        // in practice, but the filter is lexical over the string, not a
        // parsed-URL path check. `url_schemes`/`url_relative` above already
        // constrain `src` to an absolute http(s) URL before this callback
        // ever runs; only the raster-extension narrowing happens here. An
        // unrecognized `src` has its ATTRIBUTE dropped (returns `None`);
        // ammonia has no required-attribute enforcement, so this leaves a
        // retained `<img>` with no `src` — inert (no request ever fires),
        // not removed.
        //
        // CAVEAT: this is a lexical filename-suffix heuristic, not real
        // content-type verification. It does not close the tracking-pixel
        // threat that motivates `url_relative(Deny)` above: a genuine
        // external URL with an allowlisted extension (e.g.
        // `https://evil.example/pixel.png`) still passes and beacons on
        // every restricted-audience recipient's render. Full closure would
        // need image-proxying or server-side content-type enforcement —
        // out of scope for this filter; tracked as follow-up work.
        b.attribute_filter(|element, attribute, value| {
            if element == "img" && attribute == "src" {
                let lower = value.to_ascii_lowercase();
                let path = lower.split(['?', '#']).next().unwrap_or("");
                let ok = [".png", ".jpg", ".jpeg", ".webp", ".gif"]
                    .iter()
                    .any(|ext| path.ends_with(ext));
                return if ok { Some(value.into()) } else { None };
            }
            Some(value.into())
        });
    }
    if !policy.hyperlinks {
        b.rm_tags(std::iter::once("a"));
    }
    let mut schemes: HashSet<&str> = HashSet::new();
    schemes.insert("http");
    schemes.insert("https");
    if policy.emails {
        schemes.insert("mailto");
    }
    b.url_schemes(schemes);
    b
}

#[cfg(test)]
mod tests {
    use crate::chat::{sanitize, ChatContentPolicy, Segment};

    fn off() -> ChatContentPolicy {
        ChatContentPolicy::default()
    }

    fn md() -> ChatContentPolicy {
        ChatContentPolicy {
            markdown: true,
            ..off()
        }
    }

    fn html_on() -> ChatContentPolicy {
        ChatContentPolicy {
            html: true,
            ..off()
        }
    }

    fn hyperlinks_on() -> ChatContentPolicy {
        ChatContentPolicy {
            html: true,
            hyperlinks: true,
            ..off()
        }
    }

    fn images_on() -> ChatContentPolicy {
        ChatContentPolicy {
            html: true,
            images: true,
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
                // roll segment -- those are `chat::rolls`'s own producers.
                Segment::RollEmbed { .. } | Segment::RollButton { .. } => {
                    unreachable!("sanitize() never produces roll segments")
                }
            })
            .collect()
    }

    /// The shortcode pre-pass runs identically on the plain-text early-return
    /// path (the enriched path is exercised via `shortcodes.rs`'s own tests).
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
            markdown: true,
            images: true,
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
            markdown: true,
            images: true,
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
            markdown: true,
            images: true,
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
            markdown: true,
            images: true,
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
            html: true,
            hyperlinks: true,
            ..Default::default()
        };
        assert!(
            !render(&sanitize(r#"<a href="mailto:a@b.example">m</a>"#, &off)).contains("mailto:")
        );
        let on = ChatContentPolicy {
            html: true,
            hyperlinks: true,
            emails: true,
            ..Default::default()
        };
        assert!(
            render(&sanitize(r#"<a href="mailto:a@b.example">m</a>"#, &on)).contains("mailto:")
        );
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
}
