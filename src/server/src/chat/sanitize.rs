//! Message-content sanitizer: the single security boundary between raw user
//! input and a stored `Segment::Html` run. `sanitize` is the ONLY producer of
//! `Segment::Html` (see `Segment`'s doc comment) — every enrichment path
//! (Markdown rendering, raw HTML passthrough) funnels through one `ammonia`
//! `clean()` call here before anything is persisted or broadcast.
//!
//! INVARIANT: an `<img>` carrying a `src` never survives `sanitize` — chat
//! images travel exclusively as structured `Segment::Image` references
//! (`[[asset:...]]` spans, or a server-fetched external URL asset-ified by
//! `chat::post_publish`), never as raw hotlinked markup. `image_urls` on the
//! returned `Sanitized` is how a Markdown `![alt](url)`/raw `<img src=url>`
//! source is captured for the post-publish inline-image pipeline; the `<img>`
//! element itself is always removed from the returned segments.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::sync::{Arc, Mutex};

use crate::chat::{ChatContentPolicy, Segment};
use pulldown_cmark::{html, Event, Options, Parser, Tag, TagEnd};

/// One image source `sanitize` extracted from raw input: a Markdown
/// `![alt](url)` span's `dest_url` + accumulated alt text, or a raw HTML
/// `<img src=...>` tag's `src` (whose `alt` is NOT captured — see
/// `ammonia_for`'s attribute_filter doc for why only the Markdown-syntax
/// path can correlate `src` and `alt` from the same element).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSource {
    /// The image URL as authored (relative to the message, not yet fetched).
    pub url: String,
    /// Alt text, or empty when none was captured.
    pub alt: String,
}

/// The result of sanitizing one message body: the segment(s) to store, plus
/// every image source the input carried (Markdown `![alt](url)` syntax or a
/// raw HTML `<img src>`) — collected ONLY when `policy.images()` is on, empty
/// otherwise. Never carries a rendered `<img>` element (see this module's
/// INVARIANT doc); `chat::body::compose_message`/`chat::link_preview::enrich`
/// read `image_urls` to queue `PendingEnrichment::InlineImage` jobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sanitized {
    /// The segments to store (identical to the pre-M19a `sanitize` return
    /// shape).
    pub segments: Vec<Segment>,
    /// Deduped (first-seen order), uncapped image sources this body carried.
    pub image_urls: Vec<ImageSource>,
}

/// Enrich raw user input into a sanitized `Sanitized` under `policy`.
/// INVARIANT: the ONLY producer of `Segment::Html`. `ammonia` is the single
/// security boundary, crossed exactly once here. All-off => one `Text`
/// segment (fail-closed baseline, identical to `plain_text_content`).
pub fn sanitize(raw: &str, policy: &ChatContentPolicy) -> Sanitized {
    // `:shortcode:` -> unicode pre-pass, ahead of BOTH branches below, so
    // stored content is final and identical regardless of the markdown/html
    // policy toggles — always-on typing sugar, not policy-gated enrichment.
    let replaced = super::shortcodes::replace_shortcodes(raw);
    let raw: &str = &replaced;
    if !policy.markdown() && !policy.html() {
        return Sanitized {
            segments: vec![Segment::Text {
                text: raw.to_string(),
            }],
            image_urls: Vec::new(),
        };
    }
    let mut image_urls: Vec<ImageSource> = Vec::new();
    // Produce an HTML string, then hand the WHOLE thing to ammonia once.
    let html_input = if policy.markdown() {
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_TABLES);
        let parser = Parser::new_ext(raw, opts);
        let events = rewrite_markdown_images(parser, policy, &mut image_urls);
        let mut s = String::new();
        html::push_html(&mut s, events.into_iter());
        s
    } else {
        // html-only: feed the raw input straight to ammonia.
        raw.to_string()
    };
    let collected: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let cleaned = ammonia_for(policy, Arc::clone(&collected))
        .clean(&html_input)
        .to_string();
    if policy.images() {
        let raw_html_srcs = Arc::try_unwrap(collected)
            .expect("no other Arc clone outlives clean()")
            .into_inner()
            .expect("attribute_filter never panics while holding the lock");
        for url in raw_html_srcs {
            if !image_urls.iter().any(|s| s.url == url) {
                // Raw HTML `<img>` syntax has no correlated alt text (see
                // `ImageSource`'s doc) -- empty, not guessed.
                image_urls.push(ImageSource {
                    url,
                    alt: String::new(),
                });
            }
        }
    }
    // Every literal `<img>` element (markdown-sourced ones were already
    // rewritten to alt text above; this catches a raw HTML `<img>` written
    // directly in the input) is removed from the FINAL, already-cleaned
    // output. Safe unconditionally: `strip_img_tags` only ever shrinks a
    // string ammonia has already escaped every literal `<` in.
    let cleaned = strip_img_tags(&cleaned);
    Sanitized {
        segments: vec![Segment::Html {
            sanitized_html: cleaned,
        }],
        image_urls,
    }
}

/// Rewrites a Markdown event stream so that every image span (`Tag::Image`..
/// `TagEnd::Image`) is replaced by a single `Text` event carrying its
/// accumulated inner alt text, and — when `policy.images()` is on — records
/// the image's `dest_url` into `image_urls` (first-seen order, deduped).
/// This runs BEFORE the event stream ever reaches `html::push_html`, so no
/// markdown-sourced `<img>` element is ever constructed at all (unlike a raw
/// HTML `<img>`, which reaches `ammonia_for`'s attribute_filter instead).
/// Also carries the existing raw-HTML downgrade: when `policy.html()` is
/// off, an `Event::Html`/`Event::InlineHtml` becomes escaped display `Text`
/// rather than live markup.
fn rewrite_markdown_images<'a>(
    parser: Parser<'a>,
    policy: &ChatContentPolicy,
    image_urls: &mut Vec<ImageSource>,
) -> Vec<Event<'a>> {
    let mut out = Vec::new();
    // `(dest_url, accumulated alt text)` while inside an image span.
    let mut in_image: Option<(String, String)> = None;
    for ev in parser {
        if let Some((_, alt)) = in_image.as_mut() {
            match ev {
                Event::End(TagEnd::Image) => {
                    let (dest_url, alt) = in_image.take().expect("in_image checked Some above");
                    if policy.images() && !image_urls.iter().any(|s| s.url == dest_url) {
                        image_urls.push(ImageSource {
                            url: dest_url,
                            alt: alt.clone(),
                        });
                    }
                    out.push(Event::Text(alt.into()));
                }
                Event::Text(t) | Event::Code(t) => alt.push_str(&t),
                // Any other nested event inside an image span (e.g. a
                // SoftBreak in a multi-line alt) contributes no text.
                _ => {}
            }
            continue;
        }
        match ev {
            Event::Start(Tag::Image { dest_url, .. }) => {
                in_image = Some((dest_url.to_string(), String::new()));
            }
            Event::Html(s) | Event::InlineHtml(s) if !policy.html() => {
                out.push(Event::Text(s));
            }
            other => out.push(other),
        }
    }
    out
}

/// Removes every `<img ...>` element from an already-`ammonia`-cleaned HTML
/// string. Sound ONLY because ammonia has escaped every literal `<` present
/// in user-supplied text (`clean()`'s output can contain a bare `<` ONLY as
/// a real tag delimiter) — so removing one whole element here can never
/// unbalance or otherwise corrupt the surrounding markup; it can only ever
/// shrink the string. Scans byte-wise but always advances by a full UTF-8
/// character when copying non-tag content, so a multi-byte character
/// straddling the scan is never split.
fn strip_img_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while i < html.len() {
        if html[i..].starts_with("<img") {
            let bytes = html.as_bytes();
            let mut j = i + 4;
            let mut in_quote: Option<u8> = None;
            while j < bytes.len() {
                let b = bytes[j];
                match in_quote {
                    Some(q) if b == q => in_quote = None,
                    Some(_) => {}
                    None if b == b'"' || b == b'\'' => in_quote = Some(b),
                    None if b == b'>' => {
                        j += 1;
                        break;
                    }
                    None => {}
                }
                j += 1;
            }
            i = j;
        } else {
            let ch_len = html[i..].chars().next().map(char::len_utf8).unwrap_or(1);
            out.push_str(&html[i..i + ch_len]);
            i += ch_len;
        }
    }
    out
}

/// Build the ammonia sanitizer for `policy`. ammonia's DEFAULT already strips
/// `<script>`/`<style>` (`clean_content_tags`), the `style` attribute (never
/// whitelisted on any tag or as a generic attribute), and non-allowlisted URL
/// schemes (`javascript:`/`data:` are absent from the default scheme set).
/// This only NARROWS the default further per toggle — it never widens beyond
/// http/https(/mailto). `img_srcs` collects every `<img src>` value ammonia's
/// own scheme/relative-url gates already let through (a `javascript:`/
/// protocol-relative src never reaches this closure at all — see the
/// `url_relative`/`url_schemes` calls below); the caller only reads it back
/// when `policy.images()` is on.
fn ammonia_for(
    policy: &ChatContentPolicy,
    img_srcs: Arc<Mutex<Vec<String>>>,
) -> ammonia::Builder<'static> {
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
        // Record every `src` ammonia's own scheme/relative-url gates already
        // approved (see this function's doc), then ALWAYS strip the
        // attribute -- the element itself is also removed wholesale from the
        // final output by `strip_img_tags`, but dropping `src` here too
        // means no code path can ever observe a rendered `<img src>`, even
        // transiently.
        b.attribute_filter(move |element, attribute, value| {
            if element == "img" && attribute == "src" {
                img_srcs
                    .lock()
                    .expect("attribute_filter never panics while holding the lock")
                    .push(value.to_string());
                return None;
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
