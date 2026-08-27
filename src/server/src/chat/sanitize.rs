//! Message-content sanitizer: the single security boundary between raw user
//! input and a stored `Segment::Html` run. `sanitize` is the ONLY producer of
//! `Segment::Html` (see `Segment`'s doc comment) — every enrichment path
//! (Markdown rendering, raw HTML passthrough) funnels through one `ammonia`
//! `clean()` call here before anything is persisted or broadcast.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::chat::{ChatContentPolicy, Segment};
use pulldown_cmark::{html, Event, Options, Parser};

/// Enrich raw user input into a sanitized `Segment` list under `policy`.
/// INVARIANT: the ONLY producer of `Segment::Html`. `ammonia` is the single
/// security boundary, crossed exactly once here. All-off => one `Text`
/// segment (fail-closed baseline, identical to `plain_text_content`).
pub fn sanitize(raw: &str, policy: &ChatContentPolicy) -> Vec<Segment> {
    // `:shortcode:` -> unicode pre-pass, ahead of BOTH branches below, so
    // stored content is final and identical regardless of the markdown/html
    // policy toggles — always-on typing sugar, not policy-gated enrichment.
    let replaced = super::shortcodes::replace_shortcodes(raw);
    let raw: &str = &replaced;
    if !policy.markdown() && !policy.html() {
        return vec![Segment::Text {
            text: raw.to_string(),
        }];
    }
    // Produce an HTML string, then hand the WHOLE thing to ammonia once.
    let html_input = if policy.markdown() {
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
            if !policy.html() {
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
    if !policy.images() {
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
    if !policy.hyperlinks() {
        b.rm_tags(std::iter::once("a"));
    }
    let mut schemes: HashSet<&str> = HashSet::new();
    schemes.insert("http");
    schemes.insert("https");
    if policy.emails() {
        schemes.insert("mailto");
    }
    b.url_schemes(schemes);
    b
}

#[cfg(test)]
mod tests;
