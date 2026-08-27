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
/// fetched -- for the background image pipeline
/// (`chat::post_publish::run_pending_enrichments`), never run on this
/// request path.
pub async fn enrich(
    segments: &mut Vec<Segment>,
    deps: EnrichDeps<'_>,
    user: Uuid,
    now_ms: i64,
    now: Instant,
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
// ---------------------------------------------------------------------------

/// `(network, prefix_len)` pairs, each cited to the RFC that reserves it.
const V4_BLOCKED: &[(Ipv4Addr, u32)] = &[
    (Ipv4Addr::new(0, 0, 0, 0), 8),          // RFC 791 "this network"
    (Ipv4Addr::new(10, 0, 0, 0), 8),         // RFC 1918 private-use
    (Ipv4Addr::new(100, 64, 0, 0), 10),      // RFC 6598 shared address space (CGNAT)
    (Ipv4Addr::new(127, 0, 0, 0), 8),        // RFC 1122 loopback
    (Ipv4Addr::new(169, 254, 0, 0), 16),     // RFC 3927 link-local
    (Ipv4Addr::new(172, 16, 0, 0), 12),      // RFC 1918 private-use
    (Ipv4Addr::new(192, 0, 0, 0), 24),       // RFC 6890 IETF protocol assignments
    (Ipv4Addr::new(192, 0, 2, 0), 24),       // RFC 5737 TEST-NET-1
    (Ipv4Addr::new(192, 88, 99, 0), 24),     // RFC 7526 6to4 relay anycast
    (Ipv4Addr::new(192, 168, 0, 0), 16),     // RFC 1918 private-use
    (Ipv4Addr::new(198, 18, 0, 0), 15),      // RFC 2544 benchmarking
    (Ipv4Addr::new(198, 51, 100, 0), 24),    // RFC 5737 TEST-NET-2
    (Ipv4Addr::new(203, 0, 113, 0), 24),     // RFC 5737 TEST-NET-3
    (Ipv4Addr::new(224, 0, 0, 0), 4),        // RFC 5771 multicast
    (Ipv4Addr::new(240, 0, 0, 0), 4),        // RFC 1112 reserved
    (Ipv4Addr::new(255, 255, 255, 255), 32), // RFC 919 limited broadcast
];

/// Whether `ip` falls inside `network/prefix` (`prefix == 0` matches all).
fn ipv4_in_cidr(ip: u32, network: u32, prefix: u32) -> bool {
    if prefix == 0 {
        return true;
    }
    let mask = u32::MAX << (32 - prefix);
    (ip & mask) == (network & mask)
}

/// Whether `ip` is in any `V4_BLOCKED` range.
fn is_blocked_ipv4(ip: Ipv4Addr) -> bool {
    let n = u32::from(ip);
    V4_BLOCKED
        .iter()
        .any(|&(network, prefix)| ipv4_in_cidr(n, u32::from(network), prefix))
}

/// Whether `ip` is in a blocked v6 range; v4-mapped and NAT64 forms are
/// unwrapped and re-checked through the v4 table.
fn is_blocked_ipv6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();

    // ::ffff:0:0/96 — IPv4-mapped (RFC 4291 §2.5.5.2). Unwrap and re-check
    // through the v4 rules; a mapped-private address must stay blocked.
    if s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0xffff {
        return is_blocked_ipv4(embedded_v4(s));
    }
    // 64:ff9b::/96 — NAT64 well-known prefix (RFC 6052). Same unwrap-and-recheck.
    if s[0] == 0x0064 && s[1] == 0xff9b && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0 {
        return is_blocked_ipv4(embedded_v4(s));
    }
    if ip.is_unspecified() {
        return true; // ::/128, RFC 4291
    }
    if ip.is_loopback() {
        return true; // ::1/128, RFC 4291
    }
    // ::/96 IPv4-compatible (deprecated, RFC 4291 §2.5.5.1). `::` and `::1` are
    // already returned above; any remaining address with an all-zero high 96
    // bits embeds a v4 in segments 6-7 — unwrap and re-check through the v4
    // rules, mirroring the ::ffff:0:0/96 mapped-address handling above (e.g.
    // ::127.0.0.1 must stay blocked).
    if s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0 {
        return is_blocked_ipv4(embedded_v4(s));
    }
    if s[0] == 0x2002 {
        return true; // 2002::/16 6to4 (deprecated, RFC 7526) — an arbitrary v4
                     // is encapsulated in bits 16-48, so block the prefix wholesale
    }
    if s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0 {
        return true; // 100::/64 discard-only, RFC 6666
    }
    if s[0] == 0x2001 && s[1] == 0x0db8 {
        return true; // 2001:db8::/32 documentation, RFC 3849
    }
    if (s[0] & 0xfe00) == 0xfc00 {
        return true; // fc00::/7 unique-local, RFC 4193
    }
    if (s[0] & 0xffc0) == 0xfe80 {
        return true; // fe80::/10 link-local, RFC 4291
    }
    if (s[0] & 0xff00) == 0xff00 {
        return true; // ff00::/8 multicast, RFC 4291
    }
    false
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
/// module-level `V4_BLOCKED` table and `is_blocked_ipv6` for the exact,
/// RFC-cited ranges.
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
/// uses (`guarded_get`), gated on an `image/*` Content-Type. Used by the
/// post-publish image background pipeline (`post_publish`) -- never on the
/// synchronous send/edit request path.
pub async fn fetch_image_bytes(
    client: &reqwest::Client,
    raw_url: &str,
    deadline: Duration,
) -> Result<(String, Vec<u8>), PreviewError> {
    match tokio::time::timeout(
        deadline,
        guarded_get(client, raw_url, ExpectedContentType::Image, MAX_IMAGE_BYTES),
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
