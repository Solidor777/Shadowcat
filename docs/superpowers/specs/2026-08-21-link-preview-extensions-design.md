# Link-Preview Extensions — Design

**Status:** approved (self-directed design under the standing debt-burndown campaign authority).

**Spec for:** `docs/TODO.md` bucket-C sub-project 2, "Link-preview extensions — server-fetch-cache-
as-asset image pipeline + async post-publish enrichment (`WriteOrigin` path) + shared preview
cache + oEmbed provider embeds (user opted both edge items in; oEmbed carries SSRF/privacy surface
→ threat-model it)."

## 1. What already exists, unchanged by this spec

`chat::link_preview`/`chat::preview_cache` already ship a working, SSRF-guarded (`validate_url` +
`GuardedResolver`, no-redirect-follow, 5s timeout, 512KiB cap, `text/html`-only), synchronous,
title+description-only preview, run inside `handle_send_message`/`handle_edit_message`'s `enrich()`
step, capped at `MAX_PREVIEWS_PER_MESSAGE`, redacted per-recipient for free (it's ordinary message
content under the generic `PermissionSet` path — no message-specific redaction code, confirmed by
research). None of this changes. This spec adds three things on top of it.

## 2. Server-fetch-cache-as-asset image pipeline — async, via the `WriteOrigin` chokepoint

**Resolved design fork: only the NEW image-fetch step moves to async post-publish; the existing
synchronous title/description scrape is untouched.** Rewriting working, already-guarded
synchronous code to fit a new async shape would be unrequested scope growth — the minimal, correct
delta is: the synchronous scraper additionally extracts an `og:image` URL (or the page's declared
canonical image meta tag) when present, which it doesn't fetch or expose yet; then a background
task fetches and asset-ifies just that image, off the request path.

1. **`LinkPreview` gains `image_asset_id: Option<Uuid>`** (starts `None` at synchronous publish
   time).
2. **`data::asset` gains a reusable `create_asset_from_bytes(repo, world, bytes, content_type,
   created_by) -> Result<Asset, AssetError>` function**, extracted from the logic currently inlined
   in `http::assets::upload`'s multipart handler (file-first-then-row, per the existing
   `commit-db-row-before-swapping-file` invariant, unchanged) — both the existing GM upload route
   and this new caller use the same function. This is real refactoring, called out explicitly
   because the research flagged it as non-trivial: extracting shared logic out of an HTTP handler
   is expected work here, not scope creep on top of the spec.
3. **A background task, spawned after the synchronous message publish returns**, fetches the
   `og:image` URL through the SAME SSRF-guarded `reqwest::Client`/`GuardedResolver` machinery
   `link_preview.rs` already built (reused, not reimplemented), checks the persisted cache (§4)
   first, creates the asset via step 2 on a cache miss, then republishes the message via
   `Operation::Update` under `WriteOrigin::ServerMessageRevision` — the third caller of this
   chokepoint (recalc-from-chat, §3 of its own spec, is the second) — setting `image_asset_id` on
   the already-sent `LinkPreview` segment.
4. This server-initiated asset creation is **not** GM-gated (unlike the three existing
   `require_gm` asset routes) — it's a system action taken on behalf of whichever member posted the
   link, matching the existing preview fetch's own authority (any member who can post a URL can
   already trigger a preview fetch for it). `created_by` on the asset row is set to a sentinel
   system identity rather than a real user id, since no user account is the semantic author of a
   server-fetched image — see `docs/design/ARCHITECTURE.md`'s existing conventions for any
   comparable system-authored row before inventing a new sentinel shape.

## 3. Shared preview cache — resolved design fork: "shared" means persisted, not in-process

The existing `LinkPreviewCache` is already shared across worlds within one running process — so
the TODO's explicit call for a "shared preview cache" as separate, not-yet-built work must mean
**persisted across restarts**, the one dimension the current design genuinely lacks. New table,
added directly to `src/server/migrations/0001_init.sql` (single-baseline-edit convention, no
incremental migration file):

```sql
CREATE TABLE link_preview_cache (
    url TEXT PRIMARY KEY,
    title TEXT,
    description TEXT,
    image_asset_id TEXT REFERENCES assets(id) ON DELETE SET NULL,
    fetched_at TEXT NOT NULL
);
```

No `world_id` — this is the same process-global, URL-keyed scope the in-memory cache already has,
now durable. `fetched_at` drives the same max-age/staleness rule the in-memory cache already
enforces (re-fetch past a TTL), just checked against a persisted timestamp instead of an
`Instant`. The existing in-memory `Mutex<HashMap<..>>` becomes a fast-path cache in front of this
table (avoids a DB round-trip on every repeat-URL hit within one process's uptime) rather than
being replaced outright — same two-tier shape a persisted cache with a hot in-memory layer always
takes, not a new pattern.

## 4. oEmbed provider embeds — resolved design fork: allowlisted providers, structured fields only,
never render provider HTML

**Provider allowlist, not autodiscovery.** `<link rel="alternate" type="application/json+oembed">`
autodiscovery against an arbitrary user-posted URL reintroduces exactly the arbitrary-host-fetch
risk this feature must avoid — the user's own opt-in flagged "SSRF/privacy surface, threat-model
it," and an explicit host allowlist (checked synchronously and cheaply against the posted URL's
host, no network I/O, at `enrich()` time — no autodiscovery fetch ever happens) is the correct,
minimal-surface mitigation. The allowlist is a small, explicitly enumerated set of known oEmbed
provider hosts (e.g. YouTube, Vimeo — the exact set is an implementation choice for the plan, not
an architectural one); a URL matching no allowlisted host never attempts oEmbed and falls through
to the existing generic title/description preview (§1) unchanged.

**Never render a provider's raw `html` field.** An oEmbed "video"/"rich" response's `html` field is
third-party-controlled markup — rendering it client-side is a direct stored-XSS vector from a
source this server does not control. This design extracts and stores ONLY structured, non-markup
fields: `title`, `author_name`, `provider_name`, and a `thumbnail_url` (fetched and asset-ified
through the exact same §2 pipeline, never hotlinked, for the same client-IP-leak reason
`link_preview.rs`'s own doc comment already states for the generic preview's image). The original
posted URL remains the click-through target — the client renders a first-party-templated card
(provider name, title, thumbnail, "open on `<provider_name>`" link), never the provider's `html`.

```rust
pub struct OEmbedSegment {
    pub url: String,
    pub provider_name: String,
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub thumbnail_asset_id: Option<Uuid>,
}
```

Added as a new `Segment::OEmbed(OEmbedSegment)` variant (not folded into `LinkPreview`, whose shape
is generic-scrape-specific) — like the image pipeline, the oEmbed JSON fetch (allowlisted host,
same SSRF-guarded client) and thumbnail asset-ification run as a background task after synchronous
publish, republished via the same `WriteOrigin::ServerMessageRevision` chokepoint. A message whose
posted URL matches the oEmbed allowlist skips the generic `LinkPreview` scrape entirely (oEmbed's
provider-native metadata supersedes generic OG-tag scraping for that URL) rather than producing
both.

## 5. Secrecy — unchanged, confirmed sound

Every new field (`image_asset_id`, `OEmbedSegment`'s fields) lives inside `MessageEngine.content`,
redacted per-recipient by the same generic mechanism as every other segment — no new redaction
code. The persisted cache (§3) stores URL metadata treated as non-secret (an image/title
describing a public URL, not the message that posted it) — same reasoning already accepted for the
in-memory cache today, now explicitly re-confirmed as still correct once the cache survives
restarts and gains an asset row: a cached preview never carries which world/channel/message first
triggered its fetch, so persisting it introduces no new metadata-leak surface beyond what already
exists.

## 6. Testing

- `create_asset_from_bytes` unit tests (both callers — GM upload route, link-preview background
  task — produce identical `Asset` rows for identical bytes).
- Async image pipeline: publish → background task completes → `WriteOrigin::ServerMessageRevision`
  Update lands → recipients' next fetch shows `image_asset_id` populated; SSRF-guard reuse
  confirmed by re-running the existing `GuardedResolver` test suite against the new call site (not
  a new guard to test, a new caller of the existing one).
- Persisted cache: cold-start (empty in-memory cache) hits the DB table before hitting the network;
  TTL-expired row triggers a re-fetch; a fresh row updates both tiers.
- oEmbed: allowlisted-host URL produces `Segment::OEmbed`, not `LinkPreview`; non-allowlisted URL
  is unaffected (existing generic path); a fixture oEmbed JSON response's `html` field is asserted
  NEVER to reach any stored `OEmbedSegment` field or any rendered output.

## 7. Non-goals

- No autodiscovery-based oEmbed (§4).
- No rendering of provider `html` (§4) — ever, not even sanitized (structured fields only).
- No change to the existing synchronous title/description scrape's guard machinery — reused, not
  modified.
