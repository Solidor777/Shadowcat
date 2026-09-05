//! SSRF-guarded outbound HTTP fetcher for chat link previews — the server's
//! FIRST outbound HTTP surface, so every guard here is load-bearing. The
//! client never fetches link previews itself; ONLY the server fetches, behind
//! this module's address guard, and stores the result.
//!
//! Guard order (each a hard fail-closed reject): URL validation (scheme +
//! userinfo + host; a literal-IP host is checked against the blocked ranges
//! HERE because hyper's connector short-circuits DNS for IP literals, so the
//! resolver only ever sees `Host::Domain`) -> address validation for domain
//! hosts (`GuardedResolver`, DNS-rebind-safe because reqwest connects to
//! EXACTLY the IPs the resolver validated, no second resolution) -> manual
//! per-hop redirect re-validation -> a single deadline over the whole redirect
//! chain -> streamed size cap -> content-type check -> bounded text extraction.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect::Policy;
use url::{Host, Url};
use uuid::Uuid;

use super::preview_cache::{
    LinkPreviewCache, PreviewRateLimiter, NEGATIVE_TTL, POSITIVE_TTL, PREVIEW_FETCH_PER_MIN,
};
use super::sanitize::ImageSource;
use super::{PendingEnrichment, Segment};

/// A server-fetched preview. Stored verbatim by the ingest stage (a later
/// checkpoint) as a `Segment::LinkPreview`; the client renders ONLY these
/// stored strings and never fetches `url` itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPreview {
    /// The URL the fetch ended on — redirects substitute it (`url = next` per
    /// hop in `fetch_preview_inner`), so this is the post-redirect address,
    /// not necessarily what the author posted.
    pub url: String,
    /// Extracted page title, entity-decoded, capped at `MAX_TITLE_CHARS`.
    pub title: String,
    /// Extracted description, entity-decoded, capped at
    /// `MAX_DESCRIPTION_CHARS`; may be empty.
    pub description: String,
    /// A candidate `og:image`/canonical-image URL extracted (never fetched)
    /// by THIS fetch — `Some` only on a genuinely fresh scrape, always
    /// `None` on a cache-tier hit (see `image_asset_id`'s doc for why a
    /// cache hit carries the asset id instead, never the raw URL). Never
    /// persisted or serialized to any wire type — purely an in-process
    /// signal from `enrich` to its `PendingEnrichment::PreviewImage` queue.
    pub image_url: Option<String>,
    /// The already-known asset id for this URL's image, populated ONLY from
    /// a persisted-cache hit (`cached_or_fetch`) whose row already carries
    /// one — a fresh fetch never sets this (an image URL alone is not yet
    /// an asset). Mutually exclusive with `image_url` by construction.
    pub image_asset_id: Option<Uuid>,
}

/// Cap on distinct URLs previewed per message. First-seen
/// order, applied to the DEDUPED candidate list — a message pasting the same
/// link four times still counts it once toward this cap.
pub const MAX_PREVIEWS_PER_MESSAGE: usize = 3;

/// Cap on distinct inline chat images (Markdown/HTML image sources
/// `sanitize` collected) queued for background asset-ification per message,
/// independent of `MAX_PREVIEWS_PER_MESSAGE` — an inline image is the
/// message's own primary content, not a linked page's preview.
pub const MAX_INLINE_IMAGES: usize = 4;
/// Byte cap for one inline chat image fetch
/// (`post_publish::resolve_inline_image`): larger than `MAX_IMAGE_BYTES`
/// (a link-preview `og:image` thumbnail) since an inline chat image is
/// full-size message content, not a small thumbnail; smaller than
/// `MAX_PREVIEW_BYTES` (a page's whole HTML document).
pub const MAX_INLINE_IMAGE_BYTES: usize = 4 * 1024 * 1024;

/// Bounded scan for the `href` attribute of a genuine `<a ...>` tag opener
/// across an already ammonia-sanitized HTML run — NOT a raw `href="..."`
/// substring scan and NOT a full HTML parse (ammonia's output is
/// well-formed, so a bounded scan scoped to `<a` tag spans is safe and
/// avoids a second parser dependency).
///
/// SECURITY: scoping to an actual `<a ` tag's attribute list — rather than
/// searching the whole run for the bytes `href="..."` anywhere — is
/// load-bearing. Markdown BODY TEXT does not escape `"`/`'` (verified
/// against vendored `pulldown-cmark-escape`), so a member typing plain prose
/// like `see href="http://attacker.example/x" for details` (no markdown
/// link, no anchor) renders through `ammonia` unchanged; an unscoped scan
/// would match that literal substring and cause the server to fetch an
/// arbitrary attacker-chosen URL from inert, non-hyperlink text. Requiring
/// an `<a` tag OPEN (`<a` followed by whitespace or `>`, so `<article>` /
/// `<a-custom-element>` never match) before extracting `href` closes that
/// gap; when `html`/`hyperlinks` policy is on, raw user HTML also only
/// reaches this run through `ammonia::clean`, which normalizes/quotes
/// attributes, so every `<a` span found here is a well-formed tag ammonia
/// itself emitted.
///
/// `lower`/`html` stay byte-index-aligned the same way `extract_meta_tags`
/// relies on (ASCII-lowercasing never changes UTF-8 byte length). Capped at
/// 64 anchor tags scanned so a pathological anchor count cannot blow the
/// scan budget; the caller further caps the DEDUPED result at
/// `MAX_PREVIEWS_PER_MESSAGE`.
fn extract_href_urls(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut urls = Vec::new();
    let mut from = 0usize;
    while urls.len() < 64 {
        let Some(rel) = lower[from..].find("<a") else {
            break;
        };
        let start = from + rel;
        let after = lower[start + 2..].chars().next();
        let is_anchor_open = matches!(after, Some(c) if c.is_whitespace() || c == '>');
        let Some(gt_rel) = lower[start..].find('>') else {
            break;
        };
        let end = start + gt_rel;
        if is_anchor_open {
            let tag_orig = &html[start..end];
            let tag_lower = &lower[start..end];
            if let Some(href) = extract_attr(tag_lower, tag_orig, "href") {
                urls.push(href);
            }
        }
        from = end + 1;
    }
    urls
}

/// Borrow-bundle of the three link-preview dependencies threaded through
/// `handle_send_message`/`handle_edit_message`, replacing three positional
/// params with one. Not stored — constructed inline at each call site.
pub struct LinkPreviewDeps<'a> {
    /// The shared preview-fetch HTTP client (SSRF-guarded resolver).
    pub client: &'a reqwest::Client,
    /// Positive/negative preview outcome cache.
    pub cache: &'a LinkPreviewCache,
    /// Per-user fetch rate limiter (`PREVIEW_FETCH_PER_MIN`).
    pub rate: &'a PreviewRateLimiter,
}

/// Borrow-bundle of everything `enrich` needs beyond `segments`/`user`/the
/// two timestamps: the document repository (the persisted `link_preview_cache`
/// tier) alongside the existing fetch-dependency bundle. Keeps `enrich`'s own
/// parameter count from crossing the too-many-arguments threshold now that a
/// persisted-cache lookup needs a repository handle in addition to the
/// in-memory cache. Not stored — constructed inline at each call site.
pub struct EnrichDeps<'a> {
    /// The document repository, for the persisted `link_preview_cache` tier.
    pub repo: &'a dyn crate::data::repository::Repository,
    /// The shared preview-fetch HTTP client, in-memory cache, and rate limiter.
    pub fetch: LinkPreviewDeps<'a>,
}

/// Two-tier cache lookup for `url`: the in-memory `cache` (fast path, no
/// await beyond a mutex) first, then the persisted `link_preview_cache`
/// table (survives a restart) on a miss — checked BEFORE any network fetch
/// is attempted, so a cold-started process can reuse a still-fresh row
/// rather than re-fetching every URL seen since the process last started.
/// `None` return means BOTH tiers missed (or a persisted row expired past
/// its TTL) and the caller must actually fetch. A persisted hit backfills
/// the in-memory tier so a repeat within the same process's uptime skips
/// the DB entirely.
async fn cached_or_fetch(
    repo: &dyn crate::data::repository::Repository,
    cache: &LinkPreviewCache,
    url: &str,
    now: Instant,
    now_ms: i64,
) -> Option<Option<LinkPreview>> {
    if let Some(hit) = cache.get(url, now) {
        return Some(hit);
    }
    let row = repo.get_link_preview_cache(url).await.ok().flatten()?;
    let is_negative = row.title.is_none() && row.description.is_none();
    let ttl_ms = if is_negative {
        NEGATIVE_TTL.as_millis() as i64
    } else {
        POSITIVE_TTL.as_millis() as i64
    };
    let age_ms = now_ms.saturating_sub(row.fetched_at_ms);
    if age_ms >= ttl_ms {
        return None;
    }
    let outcome = if is_negative {
        None
    } else {
        Some(LinkPreview {
            url: url.to_string(),
            title: row.title.unwrap_or_default(),
            description: row.description.unwrap_or_default(),
            image_url: None,
            image_asset_id: row.image_asset_id,
        })
    };
    // Backfilled at the row's TRUE age (not a fresh `now` stamp), so the
    // in-memory tier's own TTL clock starts from the same origin the
    // persisted row's `fetched_at` already measured from — otherwise a
    // near-expiry persisted hit would silently re-arm a full fresh TTL
    // window in memory, up to doubling the effective staleness bound.
    let backfill_at = now
        .checked_sub(Duration::from_millis(age_ms.max(0) as u64))
        .unwrap_or(now);
    cache.insert(url.to_string(), outcome.clone(), backfill_at);
    Some(outcome)
}

/// Extracts candidate preview URLs from `segments`' `Html` runs — specifically
/// the `href` of an actual `<a>` tag in the sanitized output (see
/// `extract_href_urls`'s doc for why this must be scoped to a real anchor
/// tag, not any `href="..."` substring) — the authoritative "hyperlink-
/// enabled" set, since a URL the sanitizer stripped never reaches here, and
/// non-anchor body text can never yield a candidate. De-duplicated in
/// first-seen order and capped at `MAX_PREVIEWS_PER_MESSAGE`, then resolves
/// each through `cached_or_fetch` (a hit reuses the cached outcome; a miss is
/// rate-limit-gated then fetched). Misses are fetched CONCURRENTLY via a
/// `JoinSet` so the total added latency is one fetch's worth, not N serial
/// fetches. Each successful fetch APPENDS one `Segment::LinkPreview` to the
/// END of `segments` (existing segments are never reordered or removed); a
/// failure or a rate-limited URL degrades silently to no card (a failure is
/// still cached as a negative so a repeat within `NEGATIVE_TTL` doesn't
/// re-fetch; a rate-limited miss is NOT cached, so a later minute may still
/// succeed). `now_ms` drives the rate limiter's sliding window; `now` drives
/// the cache's TTL — both must be consistent with the caller's clock but are
/// deliberately separate types/precisions, matching each dependency's own
/// clock source.
///
/// Returns any `PendingEnrichment` jobs the caller must run AFTER its own
/// synchronous publish returns -- an extracted `og:image` candidate not yet
/// fetched, or an inline chat image (`image_urls`, from `Sanitized.image_urls`)
/// not yet fetched -- for the background image pipeline
/// (`chat::post_publish::run_pending_enrichments`), never run on this
/// request path. `image_urls` is queued independently of the href-preview
/// scan above, gated by the caller-supplied `scan_previews` (the world's
/// `previews_enabled()`) rather than a shared toggle: it carries Markdown/HTML
/// image sources `sanitize` already gated on `policy.images()`, so no further
/// policy check applies here -- only the same URL-validation and per-user
/// rate-limit guard every other outbound fetch candidate in this function
/// passes through. A world may enable images while previews are off (or vice
/// versa); `scan_previews` is what keeps the href/oEmbed scan from running
/// in the former case.
pub async fn enrich(
    segments: &mut Vec<Segment>,
    deps: EnrichDeps<'_>,
    user: Uuid,
    now_ms: i64,
    now: Instant,
    image_urls: &[ImageSource],
    scan_previews: bool,
) -> Vec<PendingEnrichment> {
    let EnrichDeps {
        repo,
        fetch: LinkPreviewDeps {
            client,
            cache,
            rate,
        },
    } = deps;
    let mut urls: Vec<String> = Vec::new();
    let mut pending: Vec<PendingEnrichment> = Vec::new();
    // Gated independently of `image_urls` below: a world can have hyperlink
    // previews off while images are on (or vice versa), and the two concerns
    // must not couple through one shared caller-side gate.
    if scan_previews {
        'outer: for seg in segments.iter() {
            if let Segment::Html { sanitized_html } = seg {
                for url in extract_href_urls(sanitized_html) {
                    if urls.contains(&url)
                        || pending.iter().any(
                            |p| matches!(p, PendingEnrichment::OEmbed { post_url, .. } if post_url == &url),
                        )
                    {
                        continue;
                    }
                    if let Some(provider) = crate::chat::match_oembed_provider(&url) {
                        pending.push(PendingEnrichment::OEmbed {
                            post_url: url,
                            provider,
                        });
                    } else {
                        urls.push(url);
                    }
                    if urls.len() + pending.len() >= MAX_PREVIEWS_PER_MESSAGE {
                        break 'outer;
                    }
                }
            }
        }
    }

    let mut previews: Vec<LinkPreview> = Vec::with_capacity(urls.len());
    let mut misses: Vec<String> = Vec::new();
    for url in urls {
        match cached_or_fetch(repo, cache, &url, now, now_ms).await {
            Some(Some(preview)) => previews.push(preview),
            Some(None) => {} // live cached negative: skip silently, no re-fetch
            None => misses.push(url),
        }
    }

    if !misses.is_empty() {
        let mut set = tokio::task::JoinSet::new();
        for url in misses {
            // Rate-limit-gated BEFORE spawning the fetch task — a rejected
            // URL never touches the network and is never cached (a fresh
            // minute may still succeed for it).
            if !rate.check(user, now_ms, PREVIEW_FETCH_PER_MIN) {
                continue;
            }
            let client = client.clone();
            set.spawn(async move {
                let result = fetch_preview(&client, &url).await;
                (url, result)
            });
        }
        while let Some(joined) = set.join_next().await {
            let Ok((url, result)) = joined else {
                continue; // a joined task panicking is not this stage's failure mode to surface
            };
            match result {
                Ok(preview) => {
                    cache.insert(url.clone(), Some(preview.clone()), now);
                    if let Err(e) = repo
                        .upsert_link_preview_cache(
                            &url,
                            Some(&preview.title),
                            Some(&preview.description),
                            now_ms,
                        )
                        .await
                    {
                        // In-memory tier is already populated above, so the
                        // fetch this request needed still succeeds — only
                        // the persisted tier's restart-survival guarantee is
                        // at risk, which is worth an operator-visible signal.
                        tracing::warn!(?e, %url, "link-preview cache persist failed");
                    }
                    previews.push(preview);
                }
                Err(_) => {
                    cache.insert(url.clone(), None, now);
                    if let Err(e) = repo
                        .upsert_link_preview_cache(&url, None, None, now_ms)
                        .await
                    {
                        tracing::warn!(?e, %url, "link-preview negative-cache persist failed");
                    }
                }
            }
        }
    }

    for preview in previews {
        if let Some(asset_id) = preview.image_asset_id {
            segments.push(Segment::LinkPreview {
                url: preview.url,
                title: preview.title,
                description: preview.description,
                image_asset_id: Some(asset_id),
            });
        } else {
            if let Some(image_url) = preview.image_url.clone() {
                pending.push(PendingEnrichment::PreviewImage {
                    preview_url: preview.url.clone(),
                    image_url,
                });
            }
            segments.push(Segment::LinkPreview {
                url: preview.url,
                title: preview.title,
                description: preview.description,
                image_asset_id: None,
            });
        }
    }

    // Inline chat images: `image_urls` already passed `policy.images()` inside
    // `sanitize` (only populated when the toggle is on), so the remaining
    // gates here are the same ones every other outbound fetch candidate in
    // this function passes through -- URL validation and the per-user rate
    // limit -- never a second policy check. Capped at `MAX_INLINE_IMAGES`
    // SUCCESSFULLY queued jobs, first-seen order; a rejected/rate-limited
    // candidate does not consume a slot.
    let mut queued = 0usize;
    for src in image_urls {
        if queued >= MAX_INLINE_IMAGES {
            break;
        }
        let Ok(parsed) = Url::parse(&src.url) else {
            continue;
        };
        if validate_url(&parsed).is_err() {
            continue;
        }
        if !rate.check(user, now_ms, PREVIEW_FETCH_PER_MIN) {
            continue;
        }
        pending.push(PendingEnrichment::InlineImage {
            image_url: src.url.clone(),
            alt: src.alt.clone(),
        });
        queued += 1;
    }

    pending
}

/// Why `fetch_preview` failed. Every guard in this module maps to exactly one
/// variant; there is no panic path. `BadScheme` is the umbrella for every
/// URL-validation-stage rejection (bad scheme, userinfo present, missing/empty
/// host) — `validate_url` is a single fail-closed function with one `Result`,
/// not a sequence of independently-observable checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewError {
    /// URL-validation reject: scheme, userinfo, or a missing/non-domain
    /// host. (A blocked literal-IP host is `BlockedAddress`, not this.)
    BadScheme,
    /// A resolved (or literal) address fell in an SSRF-blocked range.
    BlockedAddress,
    /// The host did not resolve, or resolution returned no addresses.
    Dns,
    /// More than `MAX_REDIRECTS` hops.
    Redirects,
    /// Connect or total deadline exceeded.
    Timeout,
    /// Body exceeded `MAX_PREVIEW_BYTES` (streamed count, not Content-Length).
    TooLarge,
    /// Response Content-Type did not match the family this guarded fetch
    /// expected -- HTML for a page preview, `image/*` for the background
    /// image pipeline.
    NotHtml,
    /// HTML fetched but no usable title/description found.
    NoContent,
    /// Non-success HTTP status.
    Http(u16),
    /// Any other transport-layer failure.
    Transport,
}

impl std::fmt::Display for PreviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreviewError::BadScheme => write!(f, "unsupported or malformed URL"),
            PreviewError::BlockedAddress => write!(f, "target address is not fetchable"),
            PreviewError::Dns => write!(f, "host did not resolve"),
            PreviewError::Redirects => write!(f, "too many redirects"),
            PreviewError::Timeout => write!(f, "request timed out"),
            PreviewError::TooLarge => write!(f, "response too large"),
            PreviewError::NotHtml => write!(f, "response was not HTML"),
            PreviewError::NoContent => write!(f, "no preview content found"),
            PreviewError::Http(status) => write!(f, "http status {status}"),
            PreviewError::Transport => write!(f, "transport error"),
        }
    }
}

/// Cap on redirect hops followed by `fetch_preview`'s manual loop (each hop is
/// re-validated: scheme/userinfo re-checked, host re-resolved through
/// `GuardedResolver`). Exceeding it fails closed as `PreviewError::Redirects`.
pub const MAX_REDIRECTS: u8 = 5;
/// Cap on the streamed, accumulated response body. `Content-Length` is only a
/// fast-reject hint (never trusted alone) — the running total during
/// `bytes_stream()` iteration is the real enforcement.
pub const MAX_PREVIEW_BYTES: usize = 512 * 1024;
/// Cap on a fetched image's bytes for the background asset-ification
/// pipeline (`og:image`/oEmbed thumbnail). Smaller than `MAX_PREVIEW_BYTES`
/// -- a page's declared preview image or a provider's thumbnail is a small
/// web graphic, not a page's full HTML.
pub const MAX_IMAGE_BYTES: usize = 256 * 1024;
/// Cap on a fetched oEmbed provider JSON response's bytes. An oEmbed
/// response is a small metadata object (title/author/thumbnail URL), never
/// a large payload.
pub const MAX_JSON_BYTES: usize = 64 * 1024;
/// Stored-title character cap (applied after entity decode + whitespace fold).
/// `pub(super)`: also the cap `post_publish::resolve_oembed` applies to a
/// provider's `title`/`author_name` via `clean_text` — the same untrusted-text
/// class as this module's own scraped title, capped the same way.
pub(super) const MAX_TITLE_CHARS: usize = 200;
/// Stored-description character cap.
const MAX_DESCRIPTION_CHARS: usize = 400;
/// Identifying User-Agent sent with every preview fetch.
const USER_AGENT: &str = "shadowcat-linkpreview/1.0";
/// TCP-connect budget per attempt.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Whole-fetch budget (all redirects + body streaming).
const TOTAL_TIMEOUT: Duration = Duration::from_secs(5);

/// Boxed error type the resolver seam returns (reqwest's dyn-error shape).
type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Sentinel error returned by `GuardedResolver::resolve` when a resolved
/// address is blocked. Downcast-matched out of reqwest's error `source()`
/// chain by `classify_transport_error` — hyper-util's `ConnectError::source()`
/// returns our boxed error directly (verified against the vendored
/// hyper-util/reqwest source), so a plain `source()` walk finds it without
/// needing to special-case `io::Error` wrapping.
#[derive(Debug)]
struct BlockedAddressError;
impl std::fmt::Display for BlockedAddressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("resolved address is in a blocked (SSRF-guard) range")
    }
}
impl std::error::Error for BlockedAddressError {}

/// Sentinel error for an unresolvable or empty-result host. See
/// `BlockedAddressError` doc for how this is recovered from the error chain.
#[derive(Debug)]
struct DnsFailureError;
impl std::fmt::Display for DnsFailureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("dns resolution failed")
    }
}
impl std::error::Error for DnsFailureError {}

// ---------------------------------------------------------------------------
// Address guard: explicit, clean-room, RFC-cited blocked ranges. Deliberately
// NOT `Ipv4Addr::is_global` (unstable, and its semantics have drifted across
// nightlies) — every range here is a named, cited constant, table-tested.
//
// INCLUSION RULE: an address is refused if it falls ANYWHERE in the IANA
// IPv4 or IPv6 Special-Purpose Address Registry, regardless of that entry's
// "Globally Reachable" value. "Globally routable" is a ROUTING property, and
// this guard's actual job is narrower and stricter: refuse anything that
// lets a requester reach a destination it could not otherwise reach, or that
// is not a real public web host. For several registry entries those two
// properties come apart — an anycast service address (PCP/TURN/DNS-SD-SRP,
// AMT, AS112 in either family, Direct Delegation AS112) is globally
// routable YET resolves to the NEAREST responder, typically a device on the
// requester's own
// network or its provider's edge: exactly the internal-reachability vector
// this guard exists to close. ORCHIDv2 and Drone Remote ID DETs are
// cryptographic IDENTIFIER space, not host addresses, and never serve web
// content either way. Nothing in special-purpose space is ever a legitimate
// link-preview target, so blocking the entire registry costs nothing, while
// allowing any routable-but-unsafe entry through is a live SSRF surface.
// Both tables below apply this rule to their registry in full: every
// registry row is present, and a row the registry marks "Globally
// Reachable: True" or "N/A" carries that fact in its own `reason` string
// alongside WHY it is refused anyway (anycast nearest-responder resolution,
// or identifier space rather than a host) — an entry blocked against its
// own registry flag needs that reasoning visible, or the next reader sees a
// contradiction and "corrects" it back. The IPv4 registry marks five rows
// True (PCP Anycast, TURN Anycast, AS112-v4, AMT, Direct Delegation AS112
// — all anycast) and one N/A (the deprecated 6to4 relay); the IPv6
// registry marks its anycast trio, AMT, both AS112 entries, ORCHIDv2 and
// Drone Remote ID True, and Teredo, ex-ORCHID and 6to4 N/A.
// `is_blocked_ipv4`/`is_blocked_ipv6` and their test suites each read their
// one table, so a future registry change is a visible row to add here,
// never a discrepancy between the guard and its own tests.
// ---------------------------------------------------------------------------

/// One entry of the IANA IPv4 Special-Purpose Address Registry, RFC-cited.
/// Every entry is blocked outright — the IPv4 registry has no translation
/// prefix embedding another address, so there is no IPv4 analogue of
/// `V6Disposition` and `is_blocked_ipv4`'s verdict is a plain any-match.
struct V4Range {
    /// Network address; bits beyond `prefix_len` are ignored and
    /// conventionally zero here.
    network: Ipv4Addr,
    /// CIDR prefix length, 0..=32.
    prefix_len: u32,
    /// RFC (or registry source) that reserves this range.
    rfc: &'static str,
    /// One-line reason, matched to the registry's own designation.
    reason: &'static str,
}

/// The IANA IPv4 Special-Purpose Address Registry, plus `224.0.0.0/4`
/// multicast. SOURCE: this table is a transcription of the IANA IPv4
/// Special-Purpose Address Registry as fetched and supplied for this guard's
/// construction — re-diff `V4_BLOCKED` against that registry directly rather
/// than re-deriving it from memory; the INCLUSION RULE comment above says
/// why every row is refused regardless of its reachability column. THE
/// REGISTRY NESTS here too (`192.0.0.0/24` contains six more specific rows,
/// `0.0.0.0/8` and `192.88.99.0/24` one each); the nested rows are
/// transcribed as their own entries so a re-diff is row-for-row, and since
/// every entry is blocked the nesting has no bearing on the verdict —
/// unlike `V6_RANGES`, whose `UnwrapV4` rows make most-specific matching
/// load-bearing. `224.0.0.0/4` is the one deliberate addition from outside
/// the special-purpose registry (it lives in the IANA IPv4 Multicast Address
/// Space Registry), mirroring `ff00::/8` in `V6_RANGES`.
const V4_BLOCKED: &[V4Range] = &[
    V4Range {
        network: Ipv4Addr::new(0, 0, 0, 0),
        prefix_len: 8,
        rfc: "RFC 791 §3.2",
        reason: "\"This network\"",
    },
    V4Range {
        network: Ipv4Addr::new(0, 0, 0, 0),
        prefix_len: 32,
        rfc: "RFC 1122 §3.2.1.3",
        reason: "\"This host on this network\"",
    },
    V4Range {
        network: Ipv4Addr::new(10, 0, 0, 0),
        prefix_len: 8,
        rfc: "RFC 1918",
        reason: "Private-Use",
    },
    V4Range {
        network: Ipv4Addr::new(100, 64, 0, 0),
        prefix_len: 10,
        rfc: "RFC 6598",
        reason: "Shared Address Space (CGNAT)",
    },
    V4Range {
        network: Ipv4Addr::new(127, 0, 0, 0),
        prefix_len: 8,
        rfc: "RFC 1122 §3.2.1.3",
        reason: "Loopback",
    },
    V4Range {
        network: Ipv4Addr::new(169, 254, 0, 0),
        prefix_len: 16,
        rfc: "RFC 3927",
        reason: "Link Local",
    },
    V4Range {
        network: Ipv4Addr::new(172, 16, 0, 0),
        prefix_len: 12,
        rfc: "RFC 1918",
        reason: "Private-Use",
    },
    V4Range {
        network: Ipv4Addr::new(192, 0, 0, 0),
        prefix_len: 24,
        rfc: "RFC 6890 §2.1",
        reason: "IETF Protocol Assignments — the parent pool the six more \
                  specific rows below carve out of; every one of those \
                  children is blocked in its own right, so this parent \
                  exists to catch whatever the registry has not yet carved \
                  a named entry out of",
    },
    V4Range {
        network: Ipv4Addr::new(192, 0, 0, 0),
        prefix_len: 29,
        rfc: "RFC 7335",
        reason: "IPv4 Service Continuity Prefix",
    },
    V4Range {
        network: Ipv4Addr::new(192, 0, 0, 8),
        prefix_len: 32,
        rfc: "RFC 7600",
        reason: "IPv4 dummy address",
    },
    V4Range {
        network: Ipv4Addr::new(192, 0, 0, 9),
        prefix_len: 32,
        rfc: "RFC 7723",
        reason: "Port Control Protocol Anycast. Globally Reachable: True \
                  per the registry, but an anycast address resolves to the \
                  NEAREST responder — typically a device on the requester's \
                  own network or its provider's edge, the exact \
                  internal-reachability vector this guard exists to close — \
                  so it is blocked despite the registry's reachability flag",
    },
    V4Range {
        network: Ipv4Addr::new(192, 0, 0, 10),
        prefix_len: 32,
        rfc: "RFC 8155",
        reason: "Traversal Using Relays around NAT Anycast. Globally \
                  Reachable: True per the registry, but see Port Control \
                  Protocol Anycast's reason above — anycast resolution to \
                  the nearest responder is the same guard-defeating \
                  property regardless of the protocol",
    },
    V4Range {
        network: Ipv4Addr::new(192, 0, 0, 170),
        prefix_len: 32,
        rfc: "RFC 8880, RFC 7050 §2.2",
        reason: "NAT64/DNS64 Discovery (one registry row lists both this \
                  address and 192.0.0.171)",
    },
    V4Range {
        network: Ipv4Addr::new(192, 0, 0, 171),
        prefix_len: 32,
        rfc: "RFC 8880, RFC 7050 §2.2",
        reason: "NAT64/DNS64 Discovery (one registry row lists both this \
                  address and 192.0.0.170)",
    },
    V4Range {
        network: Ipv4Addr::new(192, 0, 2, 0),
        prefix_len: 24,
        rfc: "RFC 5737",
        reason: "Documentation (TEST-NET-1)",
    },
    V4Range {
        network: Ipv4Addr::new(192, 31, 196, 0),
        prefix_len: 24,
        rfc: "RFC 7535",
        reason: "AS112-v4 anycast sink. Globally Reachable: True per the \
                  registry, but see Port Control Protocol Anycast's reason \
                  above — anycast resolution to the nearest responder is \
                  blocked here too",
    },
    V4Range {
        network: Ipv4Addr::new(192, 52, 193, 0),
        prefix_len: 24,
        rfc: "RFC 7450",
        reason: "AMT relay/gateway addressing. Globally Reachable: True per \
                  the registry, but AMT relays are anycast — see Port \
                  Control Protocol Anycast's reason above for why that is \
                  blocked anyway",
    },
    V4Range {
        network: Ipv4Addr::new(192, 88, 99, 0),
        prefix_len: 24,
        rfc: "RFC 7526",
        reason: "Deprecated (6to4 Relay Anycast). Globally Reachable: N/A, \
                  treated as non-routable per the INCLUSION RULE comment \
                  above",
    },
    V4Range {
        network: Ipv4Addr::new(192, 88, 99, 2),
        prefix_len: 32,
        rfc: "RFC 6751",
        reason: "6a44-relay anycast address",
    },
    V4Range {
        network: Ipv4Addr::new(192, 168, 0, 0),
        prefix_len: 16,
        rfc: "RFC 1918",
        reason: "Private-Use",
    },
    V4Range {
        network: Ipv4Addr::new(192, 175, 48, 0),
        prefix_len: 24,
        rfc: "RFC 7534",
        reason: "Direct Delegation AS112 Service. Globally Reachable: True \
                  per the registry, but see Port Control Protocol Anycast's \
                  reason above — an AS112 delegation is anycast, blocked \
                  here too",
    },
    V4Range {
        network: Ipv4Addr::new(198, 18, 0, 0),
        prefix_len: 15,
        rfc: "RFC 2544",
        reason: "Benchmarking",
    },
    V4Range {
        network: Ipv4Addr::new(198, 51, 100, 0),
        prefix_len: 24,
        rfc: "RFC 5737",
        reason: "Documentation (TEST-NET-2)",
    },
    V4Range {
        network: Ipv4Addr::new(203, 0, 113, 0),
        prefix_len: 24,
        rfc: "RFC 5737",
        reason: "Documentation (TEST-NET-3)",
    },
    V4Range {
        network: Ipv4Addr::new(224, 0, 0, 0),
        prefix_len: 4,
        rfc: "RFC 5771",
        reason: "multicast — tracked in the separate IANA IPv4 Multicast \
                  Address Space Registry, not the special-purpose one; \
                  included for the same reason ff00::/8 is in V6_RANGES",
    },
    V4Range {
        network: Ipv4Addr::new(240, 0, 0, 0),
        prefix_len: 4,
        rfc: "RFC 1112 §4",
        reason: "Reserved",
    },
    V4Range {
        network: Ipv4Addr::new(255, 255, 255, 255),
        prefix_len: 32,
        rfc: "RFC 8190, RFC 919 §7",
        reason: "Limited Broadcast",
    },
];

/// Whether `ip` falls inside `network/prefix` (`prefix == 0` matches all).
fn ipv4_in_cidr(ip: u32, network: u32, prefix: u32) -> bool {
    if prefix == 0 {
        return true;
    }
    let mask = u32::MAX << (32 - prefix);
    (ip & mask) == (network & mask)
}

/// Whether `ip` is in any `V4_BLOCKED` range. Every entry is blocked, so
/// the verdict is "any containing entry"; the most specific one is the
/// entry logged, so a hit inside a nested row (PCP Anycast rather than its
/// IETF Protocol Assignments parent) is auditable by its own name.
///
/// INVARIANT: the most-specific selection here is LOG-ONLY and cannot change
/// the verdict, which is why it may spell the rule `select_v6_range` also
/// spells rather than sharing it. Giving `V4Range` a disposition would make
/// this selection verdict-bearing and turn the two spellings into one
/// decision resolved in two places — unify them before adding one.
fn is_blocked_ipv4(ip: Ipv4Addr) -> bool {
    let n = u32::from(ip);
    let Some(r) = V4_BLOCKED
        .iter()
        .filter(|r| ipv4_in_cidr(n, u32::from(r.network), r.prefix_len))
        .max_by_key(|r| r.prefix_len)
    else {
        return false;
    };
    // Cites the matched registry entry so a rejected preview fetch is
    // auditable from logs alone, not just from this table.
    tracing::trace!(%ip, rfc = r.rfc, reason = r.reason, "matched V4_BLOCKED entry");
    true
}

/// How a `V6_RANGES` entry participates in `is_blocked_ipv6`. Every entry is
/// `Blocked` except the three `UnwrapV4` entries (IPv4-mapped `::ffff:0:0/96`,
/// IPv4-compatible `::/96`, NAT64 well-known `64:ff9b::/96`): see the
/// INCLUSION RULE comment above `V4_BLOCKED` for why NOTHING in
/// special-purpose space is ever excluded, regardless of its
/// registry-listed reachability.
#[derive(Clone, Copy, PartialEq, Eq)]
enum V6Disposition {
    /// The whole range is blocked outright.
    Blocked,
    /// Blocked by unwrapping the low 32 bits as an embedded IPv4 address and
    /// re-checking it through `V4_BLOCKED` — a mapped or translated
    /// destination must inherit the v4 guard, not bypass it.
    UnwrapV4,
}

/// One entry of the IANA IPv6 Special-Purpose Address Registry (RFC-cited)
/// relevant to the SSRF guard. See `V6Disposition` for what determines an
/// entry's `disposition`.
struct V6Range {
    /// Network address, in `Ipv6Addr::segments()` order; bits beyond
    /// `prefix_len` are ignored and conventionally zero here.
    network: [u16; 8],
    /// CIDR prefix length, 0..=128.
    prefix_len: u32,
    /// RFC (or registry source) that reserves this range.
    rfc: &'static str,
    /// One-line reason, matched to the registry's own designation.
    reason: &'static str,
    /// Whether a fetch target inside this range is blocked outright or
    /// unwrapped and re-checked as IPv4.
    disposition: V6Disposition,
}

/// The IANA IPv6 Special-Purpose Address Registry, plus `ff00::/8` multicast
/// (tracked in the separate IANA IPv6 Multicast Address Space Registry, not
/// the special-purpose one) and the historical IPv4-compatible `::/96` form
/// (RFC 4291 §2.5.5.1, which the live registry does not carry as its own row
/// but which Rust's `Ipv6Addr` parser still accepts and which is retained
/// here for defense-in-depth rather than dropped for the sake of a stricter
/// transcription). SOURCE: this table is a transcription of the IANA IPv6
/// Special-Purpose Address Registry as fetched and supplied for this guard's
/// construction — re-diff `V6_RANGES` against that registry directly rather
/// than re-deriving it from memory; the INCLUSION RULE comment above
/// `V4_BLOCKED` says why every row is refused regardless of its
/// reachability column. THE REGISTRY NESTS — `2001::/23` (IETF Protocol
/// Assignments) contains several more specific entries (the
/// PCP/TURN/DNS-SD-SRP anycast addresses, Benchmarking, AMT, AS112-v6, the
/// deprecated ex-ORCHID range, ORCHIDv2, Drone Remote ID) — so
/// `select_v6_range` matches MOST-SPECIFIC-FIRST (longest `prefix_len`
/// wins), the registry's own semantics, never first-match or any-match.
/// Every entry in this table is `Blocked` except the three
/// `V6Disposition::UnwrapV4` entries (IPv4-mapped `::ffff:0:0/96`,
/// IPv4-compatible `::/96`, NAT64 well-known `64:ff9b::/96`), which still
/// need most-specific-match to resolve to their OWN disposition rather than
/// a covering `Blocked` parent's — the embedded-address recheck only runs
/// if the more specific `UnwrapV4` entry, not the parent, wins the match.
const V6_RANGES: &[V6Range] = &[
    V6Range {
        network: [0, 0, 0, 0, 0, 0, 0, 1],
        prefix_len: 128,
        rfc: "RFC 4291",
        reason: "loopback",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0, 0, 0, 0, 0, 0, 0, 0],
        prefix_len: 128,
        rfc: "RFC 4291",
        reason: "unspecified address",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0, 0, 0, 0, 0, 0xffff, 0, 0],
        prefix_len: 96,
        rfc: "RFC 4291",
        reason: "IPv4-mapped",
        disposition: V6Disposition::UnwrapV4,
    },
    V6Range {
        network: [0, 0, 0, 0, 0, 0, 0, 0],
        prefix_len: 96,
        rfc: "RFC 4291 §2.5.5.1",
        reason: "IPv4-compatible (deprecated) — not its own row in the live \
                  registry; retained per the defense-in-depth note on \
                  V6_RANGES",
        disposition: V6Disposition::UnwrapV4,
    },
    V6Range {
        network: [0x0064, 0xff9b, 0, 0, 0, 0, 0, 0],
        prefix_len: 96,
        rfc: "RFC 6052",
        reason: "IPv4-IPv6 Translat. (well-known prefix) — the PREFIX is \
                  globally reachable as a translation mechanism, which is \
                  orthogonal to whether the v4 address it embeds is itself \
                  routable, so this stays UnwrapV4 to recheck the embedded \
                  address rather than being blocked wholesale",
        disposition: V6Disposition::UnwrapV4,
    },
    V6Range {
        network: [0x0064, 0xff9b, 1, 0, 0, 0, 0, 0],
        prefix_len: 48,
        rfc: "RFC 8215",
        reason: "IPv4-IPv6 Translat. (local-use prefix) — blocked wholesale \
                  rather than unwrapped: RFC 6052's embedding position \
                  shifts with the operator-chosen prefix length, so a \
                  partial decode here risks the same off-by-one class this \
                  guard exists to prevent, and the range is \
                  non-globally-routable regardless of what it carries",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x0100, 0, 0, 0, 0, 0, 0, 0],
        prefix_len: 64,
        rfc: "RFC 6666",
        reason: "Discard-Only",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x0100, 0, 0, 1, 0, 0, 0, 0],
        prefix_len: 64,
        rfc: "RFC 9780",
        reason: "Dummy IPv6 Prefix",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x2001, 0, 0, 0, 0, 0, 0, 0],
        prefix_len: 23,
        rfc: "RFC 2928",
        reason: "IETF Protocol Assignments — the parent pool several more \
                  specific, registry-globally-routable entries below carve \
                  out of; every one of those children is ALSO Blocked here \
                  (see each entry's own reason for why), so this parent \
                  exists to catch whatever the registry has not yet carved \
                  a named entry out of",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x2001, 0, 0, 0, 0, 0, 0, 0],
        prefix_len: 32,
        rfc: "RFC 4380, RFC 8190",
        reason: "TEREDO — an IPv4-in-IPv6 tunneling scheme; the embedded \
                  client address is NOT unwrapped and re-checked the way the \
                  IPv4-mapped/NAT64 entries above are, because a Teredo \
                  address XOR-obfuscates the sending NAT's public IPv4 \
                  against a fixed constant rather than embedding it plainly \
                  — blocking the whole prefix is the only sound option \
                  without a dedicated decoder. Globally Reachable: N/A, \
                  treated as non-routable per the INCLUSION RULE comment \
                  above V4_BLOCKED",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x2001, 1, 0, 0, 0, 0, 0, 1],
        prefix_len: 128,
        rfc: "RFC 7723",
        reason: "PCP Anycast. Globally Reachable: True per the registry, \
                  but an anycast address resolves to the NEAREST responder \
                  — typically a device on the requester's own network or \
                  its provider's edge, the exact internal-reachability \
                  vector this guard exists to close — so it is Blocked \
                  despite the registry's reachability flag",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x2001, 1, 0, 0, 0, 0, 0, 2],
        prefix_len: 128,
        rfc: "RFC 8155",
        reason: "TURN Anycast. Globally Reachable: True per the registry, \
                  but see PCP Anycast's reason above — anycast resolution \
                  to the nearest responder is the same guard-defeating \
                  property regardless of the protocol",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x2001, 1, 0, 0, 0, 0, 0, 3],
        prefix_len: 128,
        rfc: "RFC 9665",
        reason: "DNS-SD Service Registration Protocol Anycast. Globally \
                  Reachable: True per the registry; see PCP Anycast's \
                  reason above for why an anycast entry is Blocked anyway",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x2001, 2, 0, 0, 0, 0, 0, 0],
        prefix_len: 48,
        rfc: "RFC 5180",
        reason: "Benchmarking — the IPv6 analog of V4_BLOCKED's 198.18/15; \
                  routable in principle, never a legitimate fetch target",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x2001, 3, 0, 0, 0, 0, 0, 0],
        prefix_len: 32,
        rfc: "RFC 7450",
        reason: "AMT relay/gateway addressing. Globally Reachable: True per \
                  the registry, but AMT relays are anycast — see PCP \
                  Anycast's reason above for why that is Blocked anyway",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x2001, 4, 0x0112, 0, 0, 0, 0, 0],
        prefix_len: 48,
        rfc: "RFC 7535",
        reason: "AS112-v6 anycast sink. Globally Reachable: True per the \
                  registry, but see PCP Anycast's reason above — anycast \
                  resolution to the nearest responder is Blocked here too",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x2001, 0x0010, 0, 0, 0, 0, 0, 0],
        prefix_len: 28,
        rfc: "RFC 4843",
        reason: "Deprecated (previously ORCHID). Globally Reachable: N/A, \
                  treated as non-routable per the INCLUSION RULE comment \
                  above V4_BLOCKED",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x2001, 0x0020, 0, 0, 0, 0, 0, 0],
        prefix_len: 28,
        rfc: "RFC 7343",
        reason: "ORCHIDv2. Globally Reachable: True per the registry, but \
                  this is cryptographic IDENTIFIER space, not a host \
                  address — nothing here ever serves web content, so it is \
                  Blocked despite the registry's reachability flag",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x2001, 0x0030, 0, 0, 0, 0, 0, 0],
        prefix_len: 28,
        rfc: "RFC 9374",
        reason: "Drone Remote ID Protocol Entity Tags (DETs). Globally \
                  Reachable: True per the registry; see ORCHIDv2's reason \
                  above — identifier space, not a host, Blocked anyway",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x2001, 0x0db8, 0, 0, 0, 0, 0, 0],
        prefix_len: 32,
        rfc: "RFC 3849",
        reason: "Documentation",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x2002, 0, 0, 0, 0, 0, 0, 0],
        prefix_len: 16,
        rfc: "RFC 3056",
        reason: "6to4 — an arbitrary v4 is encapsulated in bits 16-48, so \
                  the whole prefix is blocked rather than unwrapped. \
                  Globally Reachable: N/A, treated as non-routable per the \
                  INCLUSION RULE comment above V4_BLOCKED",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x2620, 0x004f, 0x8000, 0, 0, 0, 0, 0],
        prefix_len: 48,
        rfc: "RFC 7534",
        reason: "Direct Delegation AS112 Service. Globally Reachable: True \
                  per the registry, but see PCP Anycast's reason above — \
                  an AS112 delegation is anycast, Blocked here too",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x3fff, 0, 0, 0, 0, 0, 0, 0],
        prefix_len: 20,
        rfc: "RFC 9637",
        reason: "Documentation",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0x5f00, 0, 0, 0, 0, 0, 0, 0],
        prefix_len: 16,
        rfc: "RFC 9602",
        reason: "Segment Routing (SRv6) SIDs",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0xfc00, 0, 0, 0, 0, 0, 0, 0],
        prefix_len: 7,
        rfc: "RFC 4193, RFC 8190",
        reason: "Unique-Local",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0xfe80, 0, 0, 0, 0, 0, 0, 0],
        prefix_len: 10,
        rfc: "RFC 4291",
        reason: "Link-Local Unicast",
        disposition: V6Disposition::Blocked,
    },
    V6Range {
        network: [0xff00, 0, 0, 0, 0, 0, 0, 0],
        prefix_len: 8,
        rfc: "RFC 4291",
        reason: "multicast — tracked in the separate IANA IPv6 Multicast \
                  Address Space Registry, not the special-purpose one; \
                  included here for the same reason the IPv4-compatible \
                  ::/96 form is (see the SOURCE note on V6_RANGES)",
        disposition: V6Disposition::Blocked,
    },
];

/// Whether `ip`'s segments fall inside `network`/`prefix_len` (0..=128),
/// comparing 16 bits at a time so a prefix that splits unevenly across a
/// segment boundary (e.g. a `/28`) still masks correctly.
fn ipv6_in_cidr(ip: [u16; 8], network: [u16; 8], prefix_len: u32) -> bool {
    let mut remaining = prefix_len;
    for i in 0..8 {
        if remaining == 0 {
            return true;
        }
        let seg_bits = remaining.min(16);
        let mask: u16 = if seg_bits == 16 {
            0xffff
        } else {
            !(0xffffu16 >> seg_bits)
        };
        if (ip[i] & mask) != (network[i] & mask) {
            return false;
        }
        remaining -= seg_bits;
    }
    true
}

/// Selects the `ranges` entry that governs `segments`: the MOST SPECIFIC
/// containing entry (the longest `prefix_len` among every entry that
/// contains the address), the registry's own semantics. The registry nests
/// (see the SOURCE note on `V6_RANGES`), so a first-match or any-match scan
/// could resolve a `V6Disposition::UnwrapV4` entry to its covering
/// `Blocked` parent's disposition instead of its own, skipping the
/// embedded-address recheck entirely — or, with the nesting the other way
/// round, resolve a narrow `Blocked` entry to its covering `UnwrapV4`
/// parent and let a public embedded address through. The table is a
/// parameter so this rule is testable against a fixture that nests the two
/// dispositions both ways, which the real registry never does: every
/// `V6_RANGES` nesting is `Blocked` inside `Blocked`, where any selection
/// order gives the same verdict.
fn select_v6_range(segments: [u16; 8], ranges: &[V6Range]) -> Option<&V6Range> {
    ranges
        .iter()
        .filter(|r| ipv6_in_cidr(segments, r.network, r.prefix_len))
        .max_by_key(|r| r.prefix_len)
}

/// Whether `ip` is refused under `ranges`, per the disposition of the entry
/// `select_v6_range` picks for it; an address in no entry is allowed.
fn is_blocked_ipv6_in(ip: Ipv6Addr, ranges: &[V6Range]) -> bool {
    let s = ip.segments();
    let Some(r) = select_v6_range(s, ranges) else {
        return false;
    };
    // Cites the matched registry entry so a rejected preview fetch is
    // auditable from logs alone, not just from this table.
    tracing::trace!(%ip, rfc = r.rfc, reason = r.reason, "matched V6_RANGES entry");
    match r.disposition {
        V6Disposition::Blocked => true,
        V6Disposition::UnwrapV4 => is_blocked_ipv4(embedded_v4(s)),
    }
}

/// Whether `ip` is in a `V6_RANGES` entry — `is_blocked_ipv6_in` over the
/// registry table. Every entry there is `Blocked` (see the INCLUSION RULE
/// comment above `V4_BLOCKED` for why nothing is ever excluded) except the
/// three `V6Disposition::UnwrapV4` translation entries.
fn is_blocked_ipv6(ip: Ipv6Addr) -> bool {
    is_blocked_ipv6_in(ip, V6_RANGES)
}

/// Extracts the embedded IPv4 address from the low 32 bits of a `/96`-mapped
/// IPv6 segment array (both `::ffff:0:0/96` and `64:ff9b::/96` carry it there).
fn embedded_v4(segments: [u16; 8]) -> Ipv4Addr {
    Ipv4Addr::new(
        (segments[6] >> 8) as u8,
        segments[6] as u8,
        (segments[7] >> 8) as u8,
        segments[7] as u8,
    )
}

/// True if `ip` must never be connected to by the preview fetcher. See the
/// module-level `V4_BLOCKED` and `V6_RANGES` tables for the exact, RFC-cited
/// ranges and the INCLUSION RULE comment above `V4_BLOCKED` for what
/// determines membership.
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_ipv4(v4),
        IpAddr::V6(v6) => is_blocked_ipv6(v6),
    }
}

// ---------------------------------------------------------------------------
// GuardedResolver: the single resolution point for DOMAIN hosts. reqwest
// connects to EXACTLY the addresses this returns, so validating them here
// closes the classic resolve-vs-connect DNS-rebind gap (no second, un-validated
// resolution). Literal-IP hosts never reach here — hyper's connector skips DNS
// for them, so `validate_url` checks IP-literal hosts against the blocked
// ranges directly.
// ---------------------------------------------------------------------------

/// Host -> IPs resolution seam. Production uses `tokio::net::lookup_host`;
/// tests inject a synthetic function so a test can supply an arbitrary IP mix
/// (public+private, a loopback stub target, etc.) without touching real DNS.
/// `Send + Sync + 'static` so `GuardedResolver` stays usable as
/// `Arc<dyn reqwest::dns::Resolve>`.
type ResolveFn = dyn Fn(&str) -> std::io::Result<Vec<IpAddr>> + Send + Sync;

/// The DOMAIN-host resolution point: resolves via `resolve_fn` (or real DNS)
/// then rejects any blocked address BEFORE reqwest connects — reqwest
/// connects to exactly the returned set, closing the resolve-vs-connect
/// rebind gap. Literal-IP hosts never reach it (`validate_url` handles them).
pub struct GuardedResolver {
    /// When `true`, a loopback address (127.0.0.0/8, `::1`) is treated as
    /// allowed — every OTHER blocked range still applies unconditionally. This
    /// is settable ONLY through the `#[cfg(test)]` constructors below (the field
    /// is private and no production constructor sets it), because the only
    /// legitimate use is pointing tests at a `127.0.0.1`-bound stub server;
    /// production code cannot reopen loopback.
    allow_loopback: bool,
    /// Test-injectable resolution seam; `None` = real DNS
    /// (`tokio::net::lookup_host`).
    resolve_fn: Option<Arc<ResolveFn>>,
}

impl GuardedResolver {
    /// Production constructor: real DNS via `tokio::net::lookup_host`, loopback
    /// always blocked. There is no production path that sets `allow_loopback`.
    pub fn new() -> Self {
        Self {
            allow_loopback: false,
            resolve_fn: None,
        }
    }

    /// Test constructor: real DNS, loopback permitted. See `allow_loopback` doc.
    #[cfg(test)]
    pub fn new_allow_loopback() -> Self {
        Self {
            allow_loopback: true,
            resolve_fn: None,
        }
    }

    /// Test constructor: replaces real DNS with `f`. See `ResolveFn` doc.
    #[cfg(test)]
    pub fn with_resolve_fn(
        allow_loopback: bool,
        f: impl Fn(&str) -> std::io::Result<Vec<IpAddr>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            allow_loopback,
            resolve_fn: Some(Arc::new(f)),
        }
    }
}

impl Default for GuardedResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        let allow_loopback = self.allow_loopback;
        let resolve_fn = self.resolve_fn.clone();
        Box::pin(async move {
            let ips: Vec<IpAddr> = match resolve_fn {
                Some(f) => f(&host).map_err(|_| Box::new(DnsFailureError) as BoxError)?,
                None => {
                    let addrs = tokio::net::lookup_host((host.as_str(), 0))
                        .await
                        .map_err(|_| Box::new(DnsFailureError) as BoxError)?;
                    addrs.map(|s| s.ip()).collect()
                }
            };
            if ips.is_empty() {
                return Err(Box::new(DnsFailureError) as BoxError);
            }
            // All-or-nothing: if ANY resolved address is blocked, the whole
            // resolution fails. An attacker whose host resolves to a mix of
            // public + private addresses must get zero connection attempt,
            // not a "lucky" connect to the public one.
            let any_blocked = ips
                .iter()
                .any(|ip| is_blocked_ip(*ip) && !(allow_loopback && ip.is_loopback()));
            if any_blocked {
                return Err(Box::new(BlockedAddressError) as BoxError);
            }
            let addrs: Addrs = Box::new(ips.into_iter().map(|ip| SocketAddr::new(ip, 0)));
            Ok(addrs)
        })
    }
}

/// Builds the shared preview-fetch client: `GuardedResolver` (loopback blocked),
/// no automatic redirects (`fetch_preview` follows them manually so each hop
/// re-validates), bounded connect/total timeouts, no cookie store (a preview
/// fetch must be stateless/uncredentialed), a fixed User-Agent. Takes NO
/// loopback flag: production cannot construct a loopback-permitting client.
pub fn build_client() -> reqwest::Client {
    build_client_with_timeouts(GuardedResolver::new(), CONNECT_TIMEOUT, TOTAL_TIMEOUT)
}

/// Test-only client whose resolver permits loopback, so tests can point it at a
/// `127.0.0.1`-bound stub server. No production counterpart exists. NOTE: a
/// literal-IP host (e.g. `http://127.0.0.1:PORT/`) is STILL rejected by
/// `validate_url` regardless of this flag (see that fn's doc) — this client
/// only helps a DOMAIN host that a resolver maps to loopback (see
/// `build_client_with_resolve_fn`).
#[cfg(test)]
pub fn build_client_allow_loopback() -> reqwest::Client {
    build_client_with_timeouts(
        GuardedResolver::new_allow_loopback(),
        CONNECT_TIMEOUT,
        TOTAL_TIMEOUT,
    )
}

/// Test-only client whose resolver is entirely replaced by `f` (loopback
/// permitted) — the seam ingest-stage tests use to point an ordinary DOMAIN
/// hostname (e.g. `stub.test`, never an IP literal, which `validate_url`
/// blocks unconditionally) at a real stub server's loopback address. Exposed
/// beyond `link_preview`'s own `#[cfg(test)] mod tests` so `chat::link_preview_ingest_tests`
/// can build an equivalent client without
/// duplicating `GuardedResolver`/`build_client_with_timeouts` wiring.
#[cfg(test)]
pub fn build_client_with_resolve_fn(
    f: impl Fn(&str) -> std::io::Result<Vec<IpAddr>> + Send + Sync + 'static,
) -> reqwest::Client {
    build_client_with_timeouts(
        GuardedResolver::with_resolve_fn(true, f),
        CONNECT_TIMEOUT,
        TOTAL_TIMEOUT,
    )
}

/// Shared construction path so `build_client` and the test-only timeout test
/// (which needs a much shorter total timeout than production's 5s to stay
/// fast) can never drift on the other settings (resolver wiring, redirect
/// policy, cookie store, User-Agent).
fn build_client_with_timeouts(
    resolver: GuardedResolver,
    connect_timeout: Duration,
    timeout: Duration,
) -> reqwest::Client {
    reqwest::Client::builder()
        .dns_resolver(Arc::new(resolver))
        .redirect(Policy::none())
        .connect_timeout(connect_timeout)
        .timeout(timeout)
        // No cookie store at all: the `cookies` feature is off in the release
        // build, so a preview fetch is inherently stateless/uncredentialed
        // (an explicit `.cookie_store(false)` would require that feature).
        .user_agent(USER_AGENT)
        .build()
        .expect("link-preview reqwest client configuration is always valid")
}

// ---------------------------------------------------------------------------
// fetch_preview
// ---------------------------------------------------------------------------

/// Fetch + parse a single URL's preview behind the full SSRF guard. Never
/// panics; every failure mode is a `PreviewError` variant. `client` is built
/// once (`build_client`) and injected/shared across calls.
pub async fn fetch_preview(
    client: &reqwest::Client,
    raw_url: &str,
) -> Result<LinkPreview, PreviewError> {
    fetch_preview_with_deadline(client, raw_url, TOTAL_TIMEOUT).await
}

/// Fetch behind a SINGLE `deadline` bounding the entire redirect chain, not each
/// hop. reqwest's per-request `timeout` is per `send()`, so a `MAX_REDIRECTS`
/// chain could otherwise burn `TOTAL_TIMEOUT * (MAX_REDIRECTS + 1)` wall-clock
/// per URL; this outer bound is the real limit (the per-request timeout stays as
/// defense in depth). Elapsing the deadline maps to `PreviewError::Timeout`.
/// `fetch_preview` passes `TOTAL_TIMEOUT`; tests inject a short deadline.
async fn fetch_preview_with_deadline(
    client: &reqwest::Client,
    raw_url: &str,
    deadline: Duration,
) -> Result<LinkPreview, PreviewError> {
    match tokio::time::timeout(deadline, fetch_preview_inner(client, raw_url)).await {
        Ok(result) => result,
        Err(_elapsed) => Err(PreviewError::Timeout),
    }
}

/// Which Content-Type family a guarded fetch must see to succeed -- shared by
/// the HTML preview fetch (`fetch_preview_inner`) and the background image
/// fetch (`fetch_image_bytes`), the guarded-GET consumers in this module.
enum ExpectedContentType {
    /// `text/html` or `application/xhtml+xml` (the existing preview gate).
    Html,
    /// Any `image/*` Content-Type (the background image-pipeline gate).
    Image,
    /// `application/json` or any `*+json` suffix (the oEmbed provider
    /// endpoint gate).
    Json,
}

impl ExpectedContentType {
    /// Whether `content_type`'s base (before `;charset=...`) matches this family.
    fn matches(&self, content_type: &str) -> bool {
        let base = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        match self {
            ExpectedContentType::Html => base == "text/html" || base == "application/xhtml+xml",
            ExpectedContentType::Image => base.starts_with("image/"),
            ExpectedContentType::Json => base == "application/json" || base.ends_with("+json"),
        }
    }
}

/// The shared guarded-GET pipeline: validate URL -> manual redirect loop
/// (each hop re-validated via `validate_url`, capped at `MAX_REDIRECTS`) ->
/// status/Content-Type gate (`expect`) -> streamed body capped at
/// `max_bytes`. Returns the final (post-redirect) `Url`, the raw Content-Type
/// header value, and the accumulated body. Every guarded fetch in this
/// module -- HTML preview, background image -- goes through this ONE
/// function, so the SSRF guard (literal-IP rejection in `validate_url`,
/// `GuardedResolver`, per-hop redirect re-validation, the size cap) is
/// written and tested exactly once.
async fn guarded_get(
    client: &reqwest::Client,
    raw_url: &str,
    expect: ExpectedContentType,
    max_bytes: usize,
) -> Result<(Url, String, Vec<u8>), PreviewError> {
    let mut url = Url::parse(raw_url).map_err(|_| PreviewError::BadScheme)?;
    validate_url(&url)?;
    let mut hop: u8 = 0;
    loop {
        let response = match client.get(url.clone()).send().await {
            Ok(r) => r,
            Err(e) => return Err(classify_transport_error(&e)),
        };
        let status = response.status();
        if status.is_redirection() {
            hop += 1;
            if hop > MAX_REDIRECTS {
                return Err(PreviewError::Redirects);
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or(PreviewError::Transport)?;
            let next = url.join(location).map_err(|_| PreviewError::BadScheme)?;
            validate_url(&next)?;
            url = next;
            continue;
        }
        if !status.is_success() {
            return Err(PreviewError::Http(status.as_u16()));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !expect.matches(&content_type) {
            return Err(PreviewError::NotHtml);
        }
        if let Some(len) = response.content_length() {
            if len > max_bytes as u64 {
                return Err(PreviewError::TooLarge);
            }
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| classify_transport_error(&e))?;
            if body.len() + chunk.len() > max_bytes {
                return Err(PreviewError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        return Ok((url, content_type, body));
    }
}

/// The undeadlined fetch pipeline: `guarded_get` gated on an HTML
/// Content-Type, then meta extraction. `fetch_preview` wraps it in the
/// total deadline.
async fn fetch_preview_inner(
    client: &reqwest::Client,
    raw_url: &str,
) -> Result<LinkPreview, PreviewError> {
    let (url, _content_type, body) = guarded_get(
        client,
        raw_url,
        ExpectedContentType::Html,
        MAX_PREVIEW_BYTES,
    )
    .await?;
    match extract_preview(&body) {
        Some(extract) => {
            let image_url = extract
                .image_url
                .and_then(|raw| url.join(&raw).ok())
                .map(|u| u.to_string());
            Ok(LinkPreview {
                url: url.to_string(),
                title: extract.title,
                description: extract.description,
                image_url,
                image_asset_id: None,
            })
        }
        None => Err(PreviewError::NoContent),
    }
}

/// Fetches `raw_url` through the SAME SSRF-guarded pipeline `fetch_preview`
/// uses (`guarded_get`), gated on an `image/*` Content-Type and `max_bytes`.
/// Used by the post-publish image background pipeline (`post_publish`) --
/// never on the synchronous send/edit request path. Callers pass
/// `MAX_IMAGE_BYTES` for a link-preview/oEmbed thumbnail or
/// `MAX_INLINE_IMAGE_BYTES` for a full-size inline chat image -- the two
/// pipelines share this one guarded fetch but differ on how large a "small
/// thumbnail" vs. "message's own content" is allowed to be.
pub async fn fetch_image_bytes(
    client: &reqwest::Client,
    raw_url: &str,
    deadline: Duration,
    max_bytes: usize,
) -> Result<(String, Vec<u8>), PreviewError> {
    match tokio::time::timeout(
        deadline,
        guarded_get(client, raw_url, ExpectedContentType::Image, max_bytes),
    )
    .await
    {
        Ok(Ok((_, content_type, body))) => Ok((content_type, body)),
        Ok(Err(e)) => Err(e),
        Err(_elapsed) => Err(PreviewError::Timeout),
    }
}

/// Fetches `raw_url` through the SAME SSRF-guarded pipeline `fetch_preview`
/// uses (`guarded_get`), gated on a `application/json`/`*+json`
/// Content-Type. Used by the post-publish oEmbed background pipeline
/// (`post_publish::resolve_oembed`) to query an allowlisted provider's
/// oEmbed endpoint -- never on the synchronous send/edit request path, and
/// never against an unvalidated host (the endpoint URL's host is always one
/// of `chat::oembed`'s fixed allowlisted hosts by the time this is called).
pub async fn fetch_json_bytes(
    client: &reqwest::Client,
    raw_url: &str,
    deadline: Duration,
) -> Result<Vec<u8>, PreviewError> {
    match tokio::time::timeout(
        deadline,
        guarded_get(client, raw_url, ExpectedContentType::Json, MAX_JSON_BYTES),
    )
    .await
    {
        Ok(Ok((_, _content_type, body))) => Ok(body),
        Ok(Err(e)) => Err(e),
        Err(_elapsed) => Err(PreviewError::Timeout),
    }
}

/// Guard #1: scheme MUST be exactly http/https; reject a URL carrying
/// `userinfo` (credential confusion); reject a missing/empty host; and — the
/// SSRF-critical part — validate any literal-IP host HERE, directly. A URL
/// whose host is an IP literal (incl. url-crate-normalized decimal/hex forms
/// like `2130706433`/`0x7f000001`, which parse to `Host::Ipv4`) never reaches
/// `GuardedResolver`: hyper-util's connector short-circuits DNS for IP-literal
/// hosts, so `is_blocked_ip` would otherwise never be consulted and reqwest
/// would connect straight to a blocked address. Only `Host::Domain` proceeds to
/// the resolver. Run on the initial URL AND on every redirect hop's resolved
/// `Location`, so this closes the literal-IP door on redirects too.
fn validate_url(url: &Url) -> Result<(), PreviewError> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(PreviewError::BadScheme);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(PreviewError::BadScheme);
    }
    match url.host() {
        Some(Host::Ipv4(a)) => {
            if is_blocked_ip(IpAddr::V4(a)) {
                return Err(PreviewError::BlockedAddress);
            }
            Ok(())
        }
        Some(Host::Ipv6(a)) => {
            if is_blocked_ip(IpAddr::V6(a)) {
                return Err(PreviewError::BlockedAddress);
            }
            Ok(())
        }
        Some(Host::Domain(h)) if !h.is_empty() => Ok(()),
        _ => Err(PreviewError::BadScheme),
    }
}

/// Recovers a `PreviewError` from a `reqwest::Error`. Timeouts are reported
/// directly by reqwest (`is_timeout`); a blocked/unresolvable address is
/// recovered by walking the `source()` chain for our sentinel error types —
/// hyper-util's `ConnectError::source()` returns the resolver's boxed error
/// directly (see `BlockedAddressError` doc), so no `io::Error`-unwrapping
/// special case is needed.
fn classify_transport_error(err: &reqwest::Error) -> PreviewError {
    if err.is_timeout() {
        return PreviewError::Timeout;
    }
    let mut cur: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(err);
    while let Some(e) = cur {
        if e.downcast_ref::<BlockedAddressError>().is_some() {
            return PreviewError::BlockedAddress;
        }
        if e.downcast_ref::<DnsFailureError>().is_some() {
            return PreviewError::Dns;
        }
        cur = e.source();
    }
    PreviewError::Transport
}

// ---------------------------------------------------------------------------
// extract_preview: pure, bounded, no general HTML parser (avoids a heavy dep
// and a parser-on-untrusted-input surface). Operates on the already
// size-capped body.
// ---------------------------------------------------------------------------

/// Everything `extract_preview` pulls from a fetched HTML document: title,
/// description, and an optional image candidate -- `og:image` preferred,
/// falling back to `<link rel="image_src">`. `image_url` may be RELATIVE
/// (resolved against the page's final URL by the caller, `fetch_preview_inner`,
/// since this pure function only sees bytes).
pub struct PreviewExtract {
    /// Extracted page title, entity-decoded, capped at `MAX_TITLE_CHARS`.
    pub title: String,
    /// Extracted description, entity-decoded, capped at `MAX_DESCRIPTION_CHARS`.
    pub description: String,
    /// Extracted `og:image`/`<link rel="image_src">` URL, RAW (not
    /// entity-decoded beyond attribute parsing, not length-capped, possibly
    /// relative) -- the caller resolves and validates it.
    pub image_url: Option<String>,
}

/// Pulls `<title>`, then prefers OpenGraph `og:title`/`og:description`,
/// falling back to `<title>`/`<meta name="description">`; also pulls an
/// image candidate (`og:image` preferred, falling back to
/// `<link rel="image_src">`). Whitespace-collapsed, entity-decoded (small
/// named + numeric set), length-capped (title <= 200 chars, description
/// <= 400). Returns `None` when both title and description are empty (no
/// card). Never panics on malformed/truncated/binary input.
pub fn extract_preview(bytes: &[u8]) -> Option<PreviewExtract> {
    let html = String::from_utf8_lossy(bytes);
    let lower = html.to_ascii_lowercase();

    let title_tag = extract_tag_text(&html, &lower, "title");
    let meta_tags = extract_meta_tags(&html, &lower);

    let mut og_title = None;
    let mut og_description = None;
    let mut og_image = None;
    let mut meta_description = None;
    for tag in &meta_tags {
        match tag.property.as_deref() {
            Some("og:title") if og_title.is_none() => og_title = tag.content.clone(),
            Some("og:description") if og_description.is_none() => {
                og_description = tag.content.clone()
            }
            Some("og:image") if og_image.is_none() => og_image = tag.content.clone(),
            _ => {}
        }
        if meta_description.is_none() && tag.name.as_deref() == Some("description") {
            meta_description = tag.content.clone();
        }
    }
    let image_url = og_image.or_else(|| extract_link_image_src(&html, &lower));

    let title = clean_text(&og_title.or(title_tag).unwrap_or_default(), MAX_TITLE_CHARS);
    let description = clean_text(
        &og_description.or(meta_description).unwrap_or_default(),
        MAX_DESCRIPTION_CHARS,
    );

    if title.is_empty() && description.is_empty() {
        None
    } else {
        Some(PreviewExtract {
            title,
            description,
            image_url,
        })
    }
}

/// Bounded scan for `<link rel="image_src" href="...">` -- the non-OpenGraph
/// canonical-image fallback some pages declare instead of `og:image`. Same
/// byte-index-aligned/64-tag-capped shape as `extract_meta_tags`.
fn extract_link_image_src(html: &str, lower: &str) -> Option<String> {
    let mut from = 0usize;
    let mut scanned = 0usize;
    while scanned < 64 {
        let Some(rel) = lower[from..].find("<link") else {
            break;
        };
        let start = from + rel;
        let Some(gt_rel) = lower[start..].find('>') else {
            break;
        };
        let end = start + gt_rel;
        let tag_orig = &html[start..end];
        let tag_lower = &lower[start..end];
        if extract_attr(tag_lower, tag_orig, "rel").as_deref() == Some("image_src") {
            return extract_attr(tag_lower, tag_orig, "href");
        }
        from = end + 1;
        scanned += 1;
    }
    None
}

/// One parsed `<meta>` tag's relevant attributes.
struct MetaTag {
    /// `property="..."` value (OpenGraph keys, e.g. `og:title`).
    property: Option<String>,
    /// `name="..."` value (e.g. `description`).
    name: Option<String>,
    /// `content="..."` value.
    content: Option<String>,
}

/// Bounded scan for `<meta ...>` tags: `lower`/`html` are byte-index-aligned
/// (ASCII-lowercasing never changes UTF-8 byte length), so offsets found in
/// `lower` slice `html` safely. Capped at 64 tags so a pathological document
/// (thousands of `<meta>` tags) cannot blow the extraction budget.
fn extract_meta_tags(html: &str, lower: &str) -> Vec<MetaTag> {
    let mut tags = Vec::new();
    let mut from = 0usize;
    while tags.len() < 64 {
        let Some(rel) = lower[from..].find("<meta") else {
            break;
        };
        let start = from + rel;
        let Some(gt_rel) = lower[start..].find('>') else {
            break;
        };
        let end = start + gt_rel;
        let tag_orig = &html[start..end];
        let tag_lower = &lower[start..end];
        tags.push(MetaTag {
            property: extract_attr(tag_lower, tag_orig, "property"),
            name: extract_attr(tag_lower, tag_orig, "name"),
            content: extract_attr(tag_lower, tag_orig, "content"),
        });
        from = end + 1;
    }
    tags
}

/// Finds `attr="value"` or `attr='value'` inside one already-isolated tag
/// (`tag_lower`/`tag_orig` are the `<...>` slice, byte-index-aligned). Only
/// matches an occurrence preceded by whitespace or the tag start, so
/// `data-property=` never satisfies `property=`.
fn extract_attr(tag_lower: &str, tag_orig: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=");
    let mut idx = 0;
    while idx <= tag_lower.len() {
        let rel = tag_lower.get(idx..)?.find(&needle)?;
        let abs = idx + rel;
        let preceded_ok = tag_lower[..abs]
            .chars()
            .next_back()
            .map(|c| c.is_whitespace())
            .unwrap_or(true);
        let val_start = abs + needle.len();
        if preceded_ok {
            if let Some(rest) = tag_orig.get(val_start..) {
                if let Some(quote) = rest.chars().next() {
                    if quote == '"' || quote == '\'' {
                        let after_quote = quote.len_utf8();
                        if let Some(end_rel) = rest[after_quote..].find(quote) {
                            return Some(rest[after_quote..after_quote + end_rel].to_string());
                        }
                    }
                }
            }
        }
        idx = val_start;
    }
    None
}

/// Bounded scan for `<tag>...</tag>` (case-insensitive). Requires the tag
/// name to be followed by whitespace or `>` (so `<titlefoo>` never matches
/// `title`). Returns `None` on any unterminated/malformed structure.
fn extract_tag_text(html: &str, lower: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}");
    let start = lower.find(&open)?;
    let after = lower[start + open.len()..].chars().next()?;
    if after != '>' && !after.is_whitespace() {
        return None;
    }
    let gt = start + lower[start..].find('>')?;
    let close_start = gt + lower[gt..].find(&close)?;
    if close_start <= gt + 1 {
        return None;
    }
    Some(html[gt + 1..close_start].to_string())
}

/// Strips any `<...>` runs (malformed-markup-safe: an unterminated `<` is
/// kept literal since `in_tag` never closes), decodes the small named/numeric
/// entity set, collapses whitespace, then caps to `max_chars` (char-boundary
/// safe via `chars().take`, never a byte-index split). `pub(super)`: also
/// applied by `post_publish::resolve_oembed` to a provider's `title`/
/// `author_name`, which are otherwise bounded only by the whole JSON
/// response's `MAX_JSON_BYTES` cap.
pub(super) fn clean_text(raw: &str, max_chars: usize) -> String {
    let stripped = strip_tags(raw);
    let decoded = decode_entities(&stripped);
    decoded
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect()
}

/// Drop everything between `<` and `>` (title text may contain markup).
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Decodes `&amp; &lt; &gt; &quot; &#39; &apos;` plus decimal (`&#NNN;`) and
/// hex (`&#xHHHH;`) numeric references. An entity search is bounded to 12
/// chars past `&` so a stray `&` in ordinary text never triggers a long scan.
fn decode_entities(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '&' {
            let scan_end = (i + 12).min(chars.len());
            let semicolon = (i + 1..scan_end).find(|&j| chars[j] == ';');
            if let Some(end) = semicolon {
                let entity: String = chars[i + 1..end].iter().collect();
                if let Some(decoded) = decode_one_entity(&entity) {
                    out.push(decoded);
                    i = end + 1;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Decode one HTML entity name/number to its character; `None` = unknown
/// (the caller then keeps the raw `&...;` text).
///
/// # Examples
///
/// ```text
/// decode_one_entity("amp") == Some('&')
/// decode_one_entity("#39") == Some('\'')
/// ```
fn decode_one_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" | "#39" => Some('\''),
        _ => {
            if let Some(hex) = entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
            {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else if let Some(dec) = entity.strip_prefix('#') {
                dec.parse::<u32>().ok().and_then(char::from_u32)
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests;
