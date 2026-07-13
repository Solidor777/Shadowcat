# M11d-3 · SSRF-Guarded Link Previews — Checkpoint Design

> The final M11 checkpoint. Parent: `2026-07-03-m11-chat-system-design.md` §6 + the M11c core
> design's c-4 entry (this IS the deferred M11c-4, folded into the M11d cycle). Depends on
> M11c-3 (sanitized `<a>` link segments) and M11d-1 (the card that renders segments). Adds the
> server's FIRST outbound HTTP — the highest-risk surface in the chat system — so the fetcher is
> the mandatory buddy-check + security-review core.

## 0. Load-bearing shape (decided)

A link preview is a stored `Segment::LinkPreview{url, title, description}` appended to the
message content at ingest, for http/https URLs in a hyperlink-enabled message. The server does
ALL fetching behind an SSRF guard; the client renders only the stored card and NEVER fetches
anything (a client fetch would leak the viewer's IP — the invariant this whole feature exists to
avoid). Cached by URL so repeats don't re-fetch. Default-ON, GM per-world toggle. Failures
degrade silently to the plain enriched link (no card, no error).

## 1. HTTP client dependency (architecture-consent)

**Decision: promote `reqwest` from a dev-dependency to a production dependency**, add the
`rustls-tls` feature (pure-Rust TLS — no OpenSSL system dependency, satisfies the cross-platform
invariant on all three CI targets; `reqwest` is already vendored for tests, so this adds TLS +
the client to the release binary, not a wholly new crate family). Rationale for `reqwest` over
alternatives: the SSRF requirement "validate the *connected* IP, not the resolved-at-check IP"
(DNS-rebind protection) needs the fetcher to control DNS resolution and connection targeting.
`reqwest` supports a **custom `dns_resolver`** (the single resolution point → reqwest connects to
exactly the IPs the resolver returns → no TOCTOU second resolution) and `redirect(Policy::none())`
(manual per-hop re-validation). Hand-rolling this on raw `hyper` would mean re-implementing
connection pooling + TLS wiring for no security gain. **Plan gate: measure the cargo-bloat binary
delta and record it; if it breaches the CI budget, fall back to `hyper` + `hyper-rustls` with a
hand-written connector.** This is the one deviation the overnight autonomous build makes that the
user should explicitly veto-check; every other choice follows from the parent spec.

## 2. The SSRF fetcher — `src/server/src/chat/link_preview.rs` (BUDDY-CHECK CORE)

Pure-ish, fully testable against a stub `axum` HTTP target (no real network). Public surface:

```rust
pub struct LinkPreview { pub url: String, pub title: String, pub description: String }
pub enum PreviewError { BadScheme, BlockedAddress, Dns, Redirects, Timeout, TooLarge,
    NotHtml, NoContent, Http(u16), Transport }
/// Fetch + parse a single URL's preview behind the full SSRF guard. Never panics;
/// every failure mode is a variant. The `Client` is built once (§4) and injected.
pub async fn fetch_preview(client: &reqwest::Client, raw_url: &str) -> Result<LinkPreview, PreviewError>;
```

Guards, in order (each a hard fail-closed reject):

1. **URL validation** — parse via `url::Url`; scheme MUST be exactly `http`/`https`; reject any
   URL carrying `userinfo` (`user:pass@` — credential confusion); reject a missing/empty host.
2. **Address validation (the DNS-rebind close)** — a custom `reqwest::dns::Resolve` impl
   (`GuardedResolver`) resolves the host (`tokio::net::lookup_host`) and validates **every**
   returned `IpAddr`; if **any** resolved address is in a blocked range, the whole resolution
   FAILS (an attacker returning one public + one private IP must not get a public-IP connection
   attempt). Only all-clear resolutions return their (validated) addrs to reqwest, which then
   connects to exactly those — closing the resolve-vs-connect gap. Blocked ranges (explicit,
   clean-room, RFC-cited — NOT the unstable `Ipv4Addr::is_global`):
   - **IPv4:** `0.0.0.0/8`, `10/8`, `100.64/10` (CGNAT, RFC 6598), `127/8`, `169.254/16`,
     `172.16/12`, `192.0.0/24`, `192.0.2/24` (TEST-NET-1), `192.88.99/24`, `192.168/16`,
     `198.18/15` (benchmark), `198.51.100/24`, `203.0.113/24`, `224/4` (multicast), `240/4`
     (reserved), `255.255.255.255/32`.
   - **IPv6:** `::/128` (unspecified), `::1/128` (loopback), `::ffff:0:0/96` (IPv4-mapped —
     validate the embedded v4 through the SAME v4 rules), `64:ff9b::/96` (NAT64 → embedded v4),
     `100::/64` (discard), `2001:db8::/32` (documentation), `fc00::/7` (unique-local),
     `fe80::/10` (link-local), `ff00::/8` (multicast).
   - Fail-closed on an unresolvable host (`PreviewError::Dns`).
3. **Redirects** — `redirect(Policy::none())`; on a 3xx with a `Location`, resolve it against the
   current URL, re-run step 1 (scheme/userinfo), and re-request (the `GuardedResolver` re-runs
   step 2 automatically for the new host). Cap at `MAX_REDIRECTS = 5`; exceeding → `Redirects`.
4. **Timeouts** — `connect_timeout(3s)` + total `timeout(5s)` on the client; a slow target →
   `Timeout`, never a hang.
5. **Response size cap** — stream `bytes_stream()`, accumulate up to `MAX_PREVIEW_BYTES = 512 KiB`,
   abort past it (`TooLarge`). `Content-Length` is a fast-reject hint only, never trusted.
6. **Content-type** — require `text/html` / `application/xhtml+xml` (ignoring params); else
   `NotHtml` (no preview — an image/binary URL never parses).
7. **Extraction** — over the size-capped, decoded-as-UTF-8-lossy body, a bounded manual scan
   pulls `<title>`, then prefers OpenGraph `og:title`/`og:description`, falling back to
   `<meta name="description">`. Whitespace-collapsed, entity-decoded for the common named/numeric
   entities only, and length-capped (`title` ≤ 200 chars, `description` ≤ 400). No general HTML
   parser (avoids a heavy dep + a parser-on-untrusted-input surface); the extractor operates on a
   bounded byte budget. Empty title + empty description → `NoContent` (no card).

**v1 scope decision: no preview image.** The spec's "optional image URL" is DEFERRED — storing a
remote image URL and rendering `<img src>` would make the client fetch it, violating the
"no client-side outbound fetch / never leaks a viewer's IP" invariant; serving it safely means
fetching + caching it as an asset (a separate pipeline). Title + description only in v1; image
logged as a follow-up (server-fetched-and-cached-as-asset). This keeps the invariant strict.

## 3. Ingest integration — synchronous, cached (`chat/mod.rs` + a cache on `AppState`)

**Synchronous fetch before publish, not async post-hoc enrichment.** Chosen deliberately for this
security-critical overnight checkpoint: no spawned task, no post-publish `Update`, no new
`WriteOrigin`, no message-deleted-mid-fetch race — the preview is part of the message's initial
content, fetched during `handle_send_message`. The only cost is up to the total timeout of added
latency on a message containing a *never-before-seen* link (cached links are instant). Async
enrichment (post immediately, enrich moments later) is a logged UX upgrade, not a v1 requirement.

Flow, after `sanitize` produces `content_segments`, gated on `policy.hyperlinks && link_previews_on`:
- Extract candidate URLs from the sanitized `Segment::Html` runs' `<a href>` values (the
  authoritative "enabled-hyperlink" set — a URL the sanitizer stripped never previews),
  de-duplicated, capped at `MAX_PREVIEWS_PER_MESSAGE = 3` (first-seen order).
- For each: consult the **cache** (below); on miss, `fetch_preview`. Fetch the ≤3 concurrently
  (`JoinSet`) so total added latency is one timeout, not three.
- Each `Ok(preview)` appends ONE `Segment::LinkPreview` to `content_segments` (at the end — "card
  at the bottom", spec §5). Each `Err` is dropped silently (degrade to the plain link) but is
  cached as a negative result.
- Roll messages (`kind == Roll`, content = one `RollEmbed`) and System notices are skipped — no
  hyperlink anchors, no previews. The edit path (`handle_edit_message`) re-runs the same preview
  stage on the new content (a preview is derived, not user-authored, so re-deriving on edit is
  correct and keeps no stale card).

**Cache** (`AppState.link_preview_cache`): an in-memory `Mutex<HashMap<String, (Instant, Option<LinkPreview>)>>`
keyed by the exact fetched URL — `Some` = a hit to reuse, `None` = a cached negative (don't
re-fetch a known-bad URL for the TTL). `POSITIVE_TTL = 1 h`, `NEGATIVE_TTL = 5 min`; opportunistic
prune on insert past a `MAX_CACHE_ENTRIES` bound (evict oldest). In-memory is correct — previews
are ephemeral and re-fetchable, nothing needs persistence. The cache is shared across
connections/worlds (a URL's preview is world-independent).

**DoS bounds:** the flood limiter already caps message rate per user; `MAX_PREVIEWS_PER_MESSAGE`
caps fetches per message; the cache collapses repeats; each fetch is timeout- and size-bounded.
A `PreviewRateLimiter` (per-user outbound-fetch budget, reusing the `PingRateLimiter` shape) caps
distinct-URL fetch attempts per user per minute so a user can't drive unbounded outbound requests
by rotating fresh URLs — checked before a cache-miss fetch, not on a cache hit.

## 4. Client construction

One `reqwest::Client` built at server start (in `AppState`), wired with the `GuardedResolver`,
`Policy::none()`, both timeouts, `rustls-tls`, a fixed `User-Agent` (`shadowcat-linkpreview/1.0`),
and NO cookie store (a preview fetch must be stateless/uncredentialed). `https_only` is NOT set
(http is allowed per spec, but the scheme allowlist already enforces http/https).

## 5. Content model + client mirror

Server `Segment` gains (serde kind-tagged snake_case):
```rust
/// A server-fetched, SSRF-guarded preview of a link in the message. Rendered by
/// the client from STORED data only — the client never fetches `url` or any
/// remote resource (that would leak the viewer's IP).
LinkPreview { url: String, title: String, description: String },
```
Client `chat-docs.ts`: `ChatSegmentSchema` gains `link_preview` (`url`/`title`/`description`
strings); `UnknownSegmentSchema.refine` and `isKnownSegment` extend to it (the fail-closed
pattern — a malformed link_preview fails the whole message parse). Drift note updated.

## 6. GM toggle

`ChatContentPolicy` (chat/settings.rs) gains `link_previews: Option<bool>` (`#[serde(default)]` =
`None`). Every other toggle is a bare `bool` defaulting OFF (fail-closed to plain text); link
previews are spec'd **default-ON**, a *wider* behavior. Resolve cleanly with a three-state field:
`Some(false)` = GM disabled; `Some(true)` = GM enabled; `None` (absent) = the default, which
`resolve_content_policy` maps to ON **only when `hyperlinks` is enabled** (previews are meaningless
without links). So: enabling hyperlinks brings previews along unless the GM writes
`link_previews: false`; a fail-closed empty policy still yields no previews (hyperlinks off). The
game-settings chat editor gets a "Link previews" checkbox (tri-state via a nullable stored value,
matching the panel's inherit=null idiom). **This default-ON-within-hyperlinks-on point is the one
place the spec's "default-ON" meets M11c's "everything fail-closed off"; flagged for review.**

## 7. Card rendering (`module-chat-card`)

A `link_preview` segment renders a bordered card at the bottom of the message: `title`
(prominent; the whole card is a link to `url` — `<a href={url} target="_blank" rel="noopener
noreferrer nofollow">`), `description` (muted, clamped to ~2 lines), and the `url`'s host as a
small caption. All fields are plain interpolation (Svelte-escaped — NO `{@html}`; the single-sink
invariant is untouched). No image. 44px-tall min touch target on the card link.

## 8. Testing strategy

- **Server / fetcher (buddy-check core, `link_preview.rs` + a stub axum target — NO real
  network):** private-IP host rejected; a host resolving to a mix of public+private rejected
  (the all-or-nothing rule); redirect-to-private rejected at the hop; `MAX_REDIRECTS` exceeded
  rejected; non-http scheme (`file:`/`ftp:`/`gopher:`/`data:`) rejected; `userinfo` rejected;
  oversized body aborted (`TooLarge`); non-HTML content-type → `NotHtml`; a well-formed HTML page
  → correct `{title, description}` (og: preferred over `<title>`/meta); empty page → `NoContent`;
  timeout honored (a stub that delays past the total timeout). The stub target binds `127.0.0.1`,
  so the tests inject an override making loopback allowed IN TESTS ONLY (a test-only
  `allow_loopback` flag on the resolver, never set in production) — document this seam loudly.
- **Server / integration (`chat/mod.rs`):** a `/`-free Normal message with an enabled-hyperlink
  URL gets a trailing `LinkPreview` segment (stub target); the same URL twice hits the cache
  (one fetch, asserted via a call counter on the stub); `MAX_PREVIEWS_PER_MESSAGE` honored; a
  failing fetch degrades to no card (message still posts); previews suppressed when
  `hyperlinks`/`link_previews` off; the per-user fetch rate limit rejects a burst of distinct URLs.
- **Client:** `chat-docs` mirror (link_preview parses; malformed fails the whole message; unknown
  still opaque); card renders title/description/host as escaped text, the card is an
  `href`-to-url anchor with `rel="noopener noreferrer nofollow"`, no image, no `{@html}`.

## 9. Explicitly out of scope (logged)

- Preview images (server-fetch-and-cache-as-asset) — the invariant-preserving path is real work;
  v1 is title+description only.
- Async post-publish enrichment (post instantly, enrich moments later) — the synchronous+cache
  design is the v1; async is a UX upgrade needing a spawned-task/`WriteOrigin` path.
- Persistent/shared preview cache (in-memory only; a multi-process deployment re-fetches per
  process — fine, re-fetchable).
- oEmbed / provider-specific rich embeds; `<meta http-equiv="refresh">` following.

## 10. Codebase-skill gate

`shadowcat-codebase-chat` (the new outbound-HTTP surface: `link_preview.rs` SSRF fetcher, the
ingest integration + cache + toggle, the `LinkPreview` segment + client mirror + card). The
`GuardedResolver` + IP-range blocklist is the load-bearing new invariant. Reviewed per the gate.

## Buddy-check directives

- **`link_preview.rs` fetcher + `GuardedResolver` (the SSRF core):** MANDATORY buddy-check +
  security review over the fetcher diff, against the stub target (PHASE = code). This is the
  checkpoint's entire reason for the HIGH risk rating — two independent security lenses on the
  IP-range blocklist completeness, the all-or-nothing resolution rule, the per-hop redirect
  re-validation, and the size/timeout/content-type bounds.
- Ingest integration + cache + client card: standard two-reviewer gate.
