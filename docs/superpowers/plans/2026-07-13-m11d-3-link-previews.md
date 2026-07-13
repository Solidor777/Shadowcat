# M11d-3 · SSRF-Guarded Link Previews — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development (Sonnet
> `shadowcat-coder` implementers, this session as dispatcher). Read the REAL files first.

**Goal:** http/https link previews fetched server-side behind a complete SSRF guard, stored as
`Segment::LinkPreview` and rendered client-side (no client fetch), cached by URL, default-ON
within a hyperlink-enabled world, GM-toggleable.

**Spec:** `docs/superpowers/specs/2026-07-13-m11d-3-link-previews-design.md` (committed — every
constant, range, and rule is normative there).

## Model/Effort directives

Same as M11d-1/2 (recorded, do not re-ask): plan mainline; `shadowcat-coder` (sonnet/medium)
implementers as unnamed one-off dispatches; `shadowcat-spec-reviewer`+`shadowcat-code-reviewer`
(high) per task; `-opus` twins for the mandatory security buddy-check + the whole-branch final
pair; escalate via twins on BLOCKED/shallow.

## Buddy-check directives

- **Task 1 (`link_preview.rs` SSRF fetcher + `GuardedResolver`):** MANDATORY buddy-check +
  security review (two independent security lenses, PHASE = code) — the checkpoint's HIGH-risk
  core. It REPLACES the normal two single-reviewer stages for that task.
- All other tasks: standard two-reviewer gate.

## Global Constraints

- Gates per task: `cargo test`/`fmt`/`clippy --all-targets -D warnings` for server tasks;
  workspace pnpm gates for client tasks; `pnpm build` before any cargo build.
- **No real network in ANY test** — the fetcher is exercised only against a stub `axum` target
  bound to loopback, reached via a TEST-ONLY `allow_loopback` resolver flag never set in prod.
- The card's `{@html}` single-sink invariant is untouched — the preview card renders via escaped
  interpolation ONLY.
- Clean-room IP-range checks, RFC-cited (per the security/IP guidelines); no unstable
  `Ipv4Addr::is_global`.
- Constants named exactly as in the spec (`MAX_REDIRECTS=5`, `MAX_PREVIEW_BYTES=512*1024`,
  `MAX_PREVIEWS_PER_MESSAGE=3`, timeouts 3s/5s, `POSITIVE_TTL`/`NEGATIVE_TTL`).
- The dice crate stays untouched; the fetcher lives in `chat/`.

## File Structure

```
src/server/Cargo.toml                     [M] reqwest → [dependencies] + rustls-tls; add `url`, `futures`
src/server/src/chat/link_preview.rs       [C] GuardedResolver + IP blocklist + fetch_preview + extractor
src/server/src/chat/mod.rs                [M] Segment::LinkPreview; the ingest preview stage; edit re-derive
src/server/src/chat/settings.rs           [M] link_previews toggle + resolve default-ON-within-hyperlinks
src/server/src/ws/mod.rs                  [M] WsState: reqwest Client + LinkPreviewCache + PreviewRateLimiter
src/server/src/ws/conn.rs                 [M] thread them into the handle_send_message call
src/server/src/http/mod.rs                [M] AppState wiring if the client is built there
src/client/core/src/chat-docs.ts          [M] link_preview mirror + refine exclusion
src/modules/chat-card/src/MessageCard.svelte [M] preview-card rendering
src/modules/game-settings/src/GameSettingsPanel.svelte [M] Link-previews toggle (if a chat section exists; else where hyperlinks is authored)
src/client/ui-kit/src/locales/en.ts       [M] keys
```

---

### Task 1: The SSRF fetcher (`link_preview.rs`) — BUDDY-CHECK CORE

**Files:** Create `src/server/src/chat/link_preview.rs` (+ `mod link_preview;`); modify
`src/server/Cargo.toml`.

- `Cargo.toml`: move `reqwest` to `[dependencies]` with
  `default-features = false, features = ["rustls-tls", "stream"]` (drop json/cookies/multipart —
  the preview client needs neither; verify the dev-tests still compile, they use the higher-level
  API — if a test needs json/cookies, ALSO list reqwest under `[dev-dependencies]` with those, or
  add the features to the single dependency and note it). Add `url = "2"` and `futures = "0.3"`
  (or use `tokio::task::JoinSet`, already available — prefer JoinSet, skip `futures` if possible).
- **cargo-bloat gate:** after it compiles, run the project's cargo-bloat / binary-size check
  (see CI config / `docs/design/ARCHITECTURE.md`) and RECORD the release-binary delta in the task
  report. If it breaches the CI budget, STOP and report BLOCKED (the spec's fallback is hyper +
  hyper-rustls — a plan change needing the human).
- Implement per spec §2 exactly:
  - `LinkPreview` struct, `PreviewError` enum (the listed variants).
  - `GuardedResolver` (`reqwest::dns::Resolve`): resolves via `tokio::net::lookup_host`, validates
    EVERY `IpAddr` via `is_blocked_ip`; ANY blocked → the whole resolution errors (all-or-nothing);
    a test-only `allow_loopback: bool` field that, when true, treats loopback as allowed (ONLY
    loopback — every other range still blocked) so the stub-target tests work; never set in prod.
  - `is_blocked_ip(ip: IpAddr) -> bool`: the explicit RFC-cited v4 + v6 ranges from spec §2 step 2
    (IPv4-mapped and NAT64 v6 addresses unwrap to their embedded v4 and re-check). Each range a
    named `const` or a documented comparison; a table-driven test asserts a representative address
    in every blocked range is blocked and a known-public address (e.g. `93.184.216.34`,
    `2606:2800:220:1::/…` documentation-free public v6) is allowed.
  - `build_client(allow_loopback) -> reqwest::Client`: `dns_resolver(Arc::new(GuardedResolver))`,
    `redirect(Policy::none())`, `connect_timeout(3s)`, `timeout(5s)`, no cookie store, fixed UA.
  - `fetch_preview(client, raw_url)`: url parse + scheme/userinfo/host checks → request loop with
    manual redirect following (≤ MAX_REDIRECTS, each Location re-parsed + scheme-rechecked; the
    resolver re-validates the host) → on 2xx: content-type check → streamed size-capped body read
    → `extract_preview(&body) -> Option<(String,String)>` → `LinkPreview` or `NoContent`.
  - `extract_preview`: bounded scan for `<title>`, `og:title`, `og:description`,
    `<meta name=description>`; whitespace-collapse; decode a small named/numeric-entity set;
    length-cap (200/400). Pure fn, heavily unit-tested.
- **Tests (this IS the buddy-check subject — write them thorough):** a stub `axum` server bound to
  `127.0.0.1` (model on the existing integration-test server helpers in `tests/common/`), plus
  `is_blocked_ip`/`extract_preview` pure unit tests. Cover EVERY spec §8 fetcher case: blocked-IP
  host, mixed public+private resolution rejected, redirect-to-private rejected, MAX_REDIRECTS
  exceeded, non-http scheme, userinfo, oversized body, non-HTML content-type, good HTML → correct
  title/desc (og preferred), empty → NoContent, timeout. For the resolution tests that need a
  specific IP mix, make `GuardedResolver` accept an injectable resolution function (a
  `dyn Fn(&str) -> Vec<IpAddr>` seam) so tests supply IP sets without real DNS — document this
  seam. The loopback stub tests set `allow_loopback: true`.
- Gates: `cargo test`, fmt, clippy. Commit: `feat(chat/m11d-3): SSRF-guarded link-preview fetcher`

> **After Task 1: run the MANDATORY security buddy-check (two opus security lenses) over the
> fetcher diff; converge; fix; only then proceed.**

---

### Task 2: Cache + per-user fetch rate limiter

**Files:** `src/server/src/chat/link_preview.rs` (or a sibling) — `LinkPreviewCache`; reuse or
extend `PingRateLimiter` for `PreviewRateLimiter`.

- `LinkPreviewCache`: `Mutex<HashMap<String, (Instant, Option<LinkPreview>)>>` with
  `get(url) -> Option<Option<LinkPreview>>` (outer None = miss/expired; inner mirrors success vs
  cached-negative), `insert(url, Option<LinkPreview>)` stamping `Instant::now()` + applying the
  right TTL on read (positive vs negative), and an opportunistic evict-oldest past
  `MAX_CACHE_ENTRIES`. `Instant::now()` is available server-side (not the workflow-script
  restriction). Unit-test hit/miss/expiry/negative-cache/eviction with an injectable clock
  (`now: Instant` param) so tests don't sleep.
- `PreviewRateLimiter`: if `PingRateLimiter` is generic enough, alias/reuse it; else a thin
  copy with the same `check(user, now_ms, per_min)` shape. `PREVIEW_FETCH_PER_MIN` const.
- Gates + commit: `feat(chat/m11d-3): link-preview cache + per-user fetch rate limiter`

---

### Task 3: Ingest integration

**Files:** `src/server/src/ws/mod.rs`, `src/server/src/http/mod.rs`,
`src/server/src/ws/conn.rs`, `src/server/src/chat/mod.rs`, `src/server/src/chat/settings.rs`.

- `WsState` gains `link_preview_client: Arc<reqwest::Client>` (built via `build_client(false)`),
  `link_preview_cache: Arc<LinkPreviewCache>`, `preview_rate: Arc<PreviewRateLimiter>` — mirror
  the `message_rate` field + `new()` init exactly. Thread into `AppState` if the client is
  constructed there.
- `settings.rs`: `ChatContentPolicy.link_previews: Option<bool>` (`#[serde(default)]`);
  `resolve_content_policy` unchanged shape, but add a resolved accessor `previews_enabled(&self)
  -> bool` = `self.hyperlinks && self.link_previews.unwrap_or(true)` (spec §6 — absent ⇒ ON only
  when hyperlinks on). Tests: hyperlinks off ⇒ false regardless; hyperlinks on + absent ⇒ true;
  hyperlinks on + `Some(false)` ⇒ false.
- `handle_send_message` signature gains `preview_client: &reqwest::Client, cache:
  &LinkPreviewCache, preview_rate: &PreviewRateLimiter` (thread from conn.rs, same as `rate`).
  After `sanitize` and `previews_enabled`: `link_preview::enrich(content_segments, client, cache,
  preview_rate, ctx.user_id, now)` — a new fn in `link_preview.rs` that extracts `<a href>` URLs
  from the `Segment::Html` runs (bounded regex/scan for `href="..."`), dedups, caps at
  `MAX_PREVIEWS_PER_MESSAGE`, checks cache then rate-limit-gated `fetch_preview` (concurrent via
  JoinSet), and appends `Segment::LinkPreview` for each success. Skip entirely for `kind == Roll`.
- `handle_edit_message`: run the same enrich stage on the edited content (a preview is derived,
  not authored — re-derive so the card reflects the edited links; the roll-immutability path
  already blocks kind==Roll edits, unaffected).
- Integration tests per spec §8 (stub target, `allow_loopback` client for tests — but the
  production `WsState` client can't reach loopback; the test constructs a `WsState`/handler with a
  loopback-allowing client, OR `handle_send_message` takes the client by reference so the test
  injects an `allow_loopback` one). Cover: URL → trailing LinkPreview; cache hit (one fetch for
  two identical sends, stub call-counter); cap; failing fetch degrades; suppressed when
  previews_enabled false; rate-limit burst rejected.
- Gates + commit: `feat(chat/m11d-3): fetch link previews at ingest (cached, gated, rate-limited)`

---

### Task 4: `Segment::LinkPreview` + client mirror

**Files:** `src/server/src/chat/mod.rs` (the enum variant — likely already added in Task 3;
if so this task is client-only), `src/client/core/src/chat-docs.ts` + test.

- Server `Segment::LinkPreview{url, title, description}` (kind-tagged snake_case) — ensure it's
  present + serde round-trips (a stored pre-M11d-3 message still parses).
- `chat-docs.ts`: `ChatSegmentSchema` gains `{kind:"link_preview", url, title, description}` (all
  `z.string()`); `UnknownSegmentSchema.refine` excludes `link_preview`; `isKnownSegment` widens.
  Tests: parses; malformed (missing title) fails the whole message; unknown still opaque.
- Gates: `pnpm --filter @shadowcat/core test` + `pnpm -r typecheck`. Commit:
  `feat(chat/m11d-3): client mirror for link-preview segments (fail-closed)`

---

### Task 5: Card rendering

**Files:** `src/modules/chat-card/src/MessageCard.svelte` + test, `locales/en.ts`.

- A `link_preview` segment renders `<a class="link-preview" href={s.url} target="_blank"
  rel="noopener noreferrer nofollow">` containing `{s.title}` (prominent), `{s.description}`
  (muted, `-webkit-line-clamp: 2`), and the host (`new URL(s.url).host`, guarded in a try/catch →
  fallback to the raw url) as a caption. All escaped interpolation, NO `{@html}`, NO `<img>`.
  Min 44px touch height; SCSS tokens.
- Tests: renders title/description/host as text; the anchor has the exact href + rel; no `<img>`;
  a malformed-url host falls back without throwing.
- Locale keys under `chat.linkPreview.*` if any labels are needed (likely none — pure data).
- Gates + commit: `feat(chat/m11d-3): card renders link-preview cards (no client fetch, no img)`

---

### Task 6: GM toggle

**Files:** `src/modules/game-settings/src/GameSettingsPanel.svelte` + test, locales.

- Add a "Link previews" tri-state control wherever the chat-settings `hyperlinks` policy is
  authored in the panel (read the panel — if M11c-3/M11d-1 added a chat-settings section, extend
  it; if the chat-settings doc has no editor yet, add a minimal one for `hyperlinks` +
  `link_previews`). Writes `/system/link_previews` as `true`/`false`/absent(inherit=default-on)
  using the panel's real-pre-image `set()` helper (post-M11d-2 all writes pass real `old`).
- Tests: the control dispatches the right update; reflects stored value.
- Gates + commit: `feat(chat/m11d-3): GM link-preview toggle`

---

### Task 7: Integration pass (dispatcher)

Full gates (pnpm -r test/typecheck/lint, pnpm build, cargo test/fmt/clippy), boot smoke (binary
serves; the fetcher is network-stubbed in tests so no live outbound in the smoke), no generated
drift. Record the cargo-bloat delta in the ledger.

### Task 8: Docs + skill gate + final review + merge

- PLAN.md M11d-3 entry (⇒ **M11 COMPLETE** — this is the last checkpoint); TODO.md spec §9
  deferrals (preview images, async enrichment, persistent cache, oEmbed).
- `shadowcat-codebase-chat` update (the outbound-HTTP surface: `link_preview.rs` fetcher +
  `GuardedResolver` invariant, ingest enrich stage, cache, toggle, `LinkPreview` segment + card);
  reviewed skill-update gate.
- Whole-branch final review: opus pair (with a security lens, given the new outbound surface).
  Fix wave if needed. Merge `--no-ff`. **NO push** (leave the full-M11 push decision to the user).

## Self-review

- Spec coverage: §1→T1, §2→T1, §3→T2+T3, §4→T3, §5→T4, §6→T3+T6, §7→T5, §8→per-task, §9→T8 TODO,
  §10→T8. No gaps.
- Types consistent: `LinkPreview`/`PreviewError`/`GuardedResolver`/`is_blocked_ip`/`fetch_preview`/
  `enrich`/`LinkPreviewCache`/`previews_enabled` used identically across tasks.
- No placeholders: the SSRF ranges, guard order, caps, and the default-ON resolution are all
  specified with exact values; established patterns (rate limiter, settings resolver, segment
  mirror, card render) reference the real in-tree precedent.
