//! SSRF-guarded outbound HTTP fetcher for chat link previews — the server's
//! FIRST outbound HTTP surface, so every guard here is load-bearing. The
//! client never fetches link previews itself; ONLY the server fetches, behind
//! this module's address guard, and stores the result. See the M11d-3 design
//! doc §2 for the full rationale.
//!
//! Guard order (each a hard fail-closed reject): URL validation (scheme +
//! userinfo + host) -> address validation (`GuardedResolver`, DNS-rebind-safe
//! because reqwest connects to EXACTLY the IPs the resolver validated, no
//! second resolution) -> manual per-hop redirect re-validation -> connect/
//! total timeouts -> streamed size cap -> content-type check -> bounded
//! text extraction.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect::Policy;
use url::Url;

/// A server-fetched preview. Stored verbatim by the ingest stage (a later
/// checkpoint) as a `Segment::LinkPreview`; the client renders ONLY these
/// stored strings and never fetches `url` itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPreview {
    pub url: String,
    pub title: String,
    pub description: String,
}

/// Why `fetch_preview` failed. Every guard in this module maps to exactly one
/// variant; there is no panic path. `BadScheme` is the umbrella for every
/// URL-validation-stage rejection (bad scheme, userinfo present, missing/empty
/// host) — the spec's guard #1 is a single fail-closed step with one outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewError {
    BadScheme,
    BlockedAddress,
    Dns,
    Redirects,
    Timeout,
    TooLarge,
    NotHtml,
    NoContent,
    Http(u16),
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
const MAX_TITLE_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 400;
const USER_AGENT: &str = "shadowcat-linkpreview/1.0";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(5);

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

fn ipv4_in_cidr(ip: u32, network: u32, prefix: u32) -> bool {
    if prefix == 0 {
        return true;
    }
    let mask = u32::MAX << (32 - prefix);
    (ip & mask) == (network & mask)
}

fn is_blocked_ipv4(ip: Ipv4Addr) -> bool {
    let n = u32::from(ip);
    V4_BLOCKED
        .iter()
        .any(|&(network, prefix)| ipv4_in_cidr(n, u32::from(network), prefix))
}

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
// GuardedResolver: the single resolution point. reqwest connects to EXACTLY
// the addresses this returns, so validating them here closes the classic
// resolve-vs-connect DNS-rebind gap (no second, un-validated resolution).
// ---------------------------------------------------------------------------

/// Host -> IPs resolution seam. Production uses `tokio::net::lookup_host`;
/// tests inject a synthetic function so a test can supply an arbitrary IP mix
/// (public+private, a loopback stub target, etc.) without touching real DNS.
/// `Send + Sync + 'static` so `GuardedResolver` stays usable as
/// `Arc<dyn reqwest::dns::Resolve>`.
type ResolveFn = dyn Fn(&str) -> std::io::Result<Vec<IpAddr>> + Send + Sync;

pub struct GuardedResolver {
    /// Test-only escape hatch: when `true`, a loopback address (127.0.0.0/8,
    /// `::1`) is treated as allowed — every OTHER blocked range still applies
    /// unconditionally. The stub HTTP targets this module's tests run against
    /// bind `127.0.0.1`, so exercising the guard against them needs this.
    /// Production code MUST always construct with `allow_loopback: false`
    /// (the `build_client` entry point does).
    pub allow_loopback: bool,
    resolve_fn: Option<Arc<ResolveFn>>,
}

impl GuardedResolver {
    /// Production constructor: real DNS via `tokio::net::lookup_host`.
    pub fn new(allow_loopback: bool) -> Self {
        Self {
            allow_loopback,
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

/// Builds the shared preview-fetch client: `GuardedResolver`, no automatic
/// redirects (`fetch_preview` follows them manually so each hop re-validates),
/// bounded connect/total timeouts, no cookie store (a preview fetch must be
/// stateless/uncredentialed), a fixed User-Agent. `allow_loopback` must be
/// `false` in production; it exists only so tests can point the client at a
/// `127.0.0.1`-bound stub server.
pub fn build_client(allow_loopback: bool) -> reqwest::Client {
    build_client_with_timeouts(
        GuardedResolver::new(allow_loopback),
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
        .cookie_store(false)
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
            .unwrap_or("");
        if !is_html_content_type(content_type) {
            return Err(PreviewError::NotHtml);
        }

        // Fast-reject hint only; the streamed running total below is the real
        // enforcement (a server can omit or lie about Content-Length).
        if let Some(len) = response.content_length() {
            if len > MAX_PREVIEW_BYTES as u64 {
                return Err(PreviewError::TooLarge);
            }
        }

        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| classify_transport_error(&e))?;
            if body.len() + chunk.len() > MAX_PREVIEW_BYTES {
                return Err(PreviewError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }

        return match extract_preview(&body) {
            Some((title, description)) => Ok(LinkPreview {
                url: url.to_string(),
                title,
                description,
            }),
            None => Err(PreviewError::NoContent),
        };
    }
}

/// Guard #1: scheme MUST be exactly http/https; reject a URL carrying
/// `userinfo` (credential confusion); reject a missing/empty host. Run on the
/// initial URL AND on every redirect hop's resolved `Location`.
fn validate_url(url: &Url) -> Result<(), PreviewError> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(PreviewError::BadScheme);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(PreviewError::BadScheme);
    }
    match url.host_str() {
        Some(h) if !h.is_empty() => Ok(()),
        _ => Err(PreviewError::BadScheme),
    }
}

fn is_html_content_type(content_type: &str) -> bool {
    let base = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    base == "text/html" || base == "application/xhtml+xml"
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

/// Pulls `<title>`, then prefers OpenGraph `og:title`/`og:description`,
/// falling back to `<title>`/`<meta name="description">`. Whitespace-
/// collapsed, entity-decoded (small named + numeric set), length-capped
/// (title <= 200 chars, description <= 400). Returns `None` when both are
/// empty (no card). Never panics on malformed/truncated/binary input.
pub fn extract_preview(bytes: &[u8]) -> Option<(String, String)> {
    let html = String::from_utf8_lossy(bytes);
    let lower = html.to_ascii_lowercase();

    let title_tag = extract_tag_text(&html, &lower, "title");
    let meta_tags = extract_meta_tags(&html, &lower);

    let mut og_title = None;
    let mut og_description = None;
    let mut meta_description = None;
    for tag in &meta_tags {
        match tag.property.as_deref() {
            Some("og:title") if og_title.is_none() => og_title = tag.content.clone(),
            Some("og:description") if og_description.is_none() => {
                og_description = tag.content.clone()
            }
            _ => {}
        }
        if meta_description.is_none() && tag.name.as_deref() == Some("description") {
            meta_description = tag.content.clone();
        }
    }

    let title = clean_text(&og_title.or(title_tag).unwrap_or_default(), MAX_TITLE_CHARS);
    let description = clean_text(
        &og_description.or(meta_description).unwrap_or_default(),
        MAX_DESCRIPTION_CHARS,
    );

    if title.is_empty() && description.is_empty() {
        None
    } else {
        Some((title, description))
    }
}

struct MetaTag {
    property: Option<String>,
    name: Option<String>,
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
/// safe via `chars().take`, never a byte-index split).
fn clean_text(raw: &str, max_chars: usize) -> String {
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
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
        </head></html>"#;
        let (title, desc) = extract_preview(html).unwrap();
        assert_eq!(title, "OG Title");
        assert_eq!(desc, "OG Description");
    }

    #[test]
    fn falls_back_to_title_and_meta_description() {
        let html = br#"<html><head>
            <title>Just A Title</title>
            <meta name="description" content="Just a description">
        </head></html>"#;
        let (title, desc) = extract_preview(html).unwrap();
        assert_eq!(title, "Just A Title");
        assert_eq!(desc, "Just a description");
    }

    #[test]
    fn empty_document_yields_no_content() {
        assert_eq!(extract_preview(b"<html><body>hello</body></html>"), None);
        assert_eq!(extract_preview(b""), None);
    }

    #[test]
    fn decodes_common_entities() {
        let html =
            br#"<title>Fish &amp; Chips &lt;tasty&gt; &quot;deal&quot; &#39;now&#39;</title>"#;
        let (title, _) = extract_preview(html).unwrap();
        assert_eq!(title, "Fish & Chips <tasty> \"deal\" 'now'");
    }

    #[test]
    fn collapses_whitespace_and_caps_length() {
        let long_title = "A".repeat(500);
        let html = format!("<title>{long_title}</title><meta name=\"description\" content=\"line1\n\n  line2   line3\">");
        let (title, desc) = extract_preview(html.as_bytes()).unwrap();
        assert_eq!(title.chars().count(), MAX_TITLE_CHARS);
        assert_eq!(desc, "line1 line2 line3");
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
        let client = build_client(true);
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
        let client = build_client(true);
        let err = fetch_preview(&client, "http://user:pass@example.com/")
            .await
            .unwrap_err();
        assert_eq!(err, PreviewError::BadScheme);
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
        let router =
            Router::new().route("/", get(|| async { axum::response::Html("<html></html>") }));
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
}
