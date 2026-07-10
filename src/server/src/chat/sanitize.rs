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
    if !policy.images {
        b.rm_tags(std::iter::once("img"));
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

    /// Concatenates a `&[Segment]` into a single string (Text verbatim, Html's
    /// `sanitized_html`) so assertions can read uniformly regardless of which
    /// segment variant `sanitize` produced.
    fn render(segs: &[Segment]) -> String {
        segs.iter()
            .map(|s| match s {
                Segment::Text { text } => text.clone(),
                Segment::Html { sanitized_html } => sanitized_html.clone(),
            })
            .collect()
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
