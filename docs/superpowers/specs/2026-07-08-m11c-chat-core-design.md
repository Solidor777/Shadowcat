# M11c Chat Core (headless) — Checkpoint Design

**Status:** Approved design (brainstorm complete), pre-plan.
**Parent spec:** `2026-07-03-m11-chat-system-design.md` (cross-cutting M11c+M11d decisions — locked, not
re-litigated here). This document refines **M11c** (the server-side, headless half) into four ordered
sub-checkpoints and records the checkpoint-level resolutions the parent spec left implicit.
**Scope boundary:** M11c is **server-side (Rust) only** and **dice-independent** (parent spec §1, §8).
Client display modules and roll-at-ingest wiring are **M11d**.

## 1. Boundary & approved choices

- **Server-only.** M11c is the headless chat core: ingest → sanitize → persist → broadcast, plus the
  message document model, channel model, and whisper redaction tier. No Svelte, no display modules
  (M11d). The client-side composer/card and their Zod mirrors are M11d.
- **Dice-independent.** `/roll` is detected as a `kind` at ingest but **not executed** in M11c; the
  dice-engine invocation + `RollResult` embed is M11d (keeps M11c parallel-able with dice per parent
  §1, and avoids coupling the chat core to `dice/`).
- **Sanitizer = vetted crates, not clean-room.** HTML sanitization via **`ammonia`** (strict
  tag/attr allowlist) and Markdown via **`pulldown-cmark`**. A security boundary is not hand-rolled
  (forward-thinking discipline); IP clean-room rules concern copied code, not reimplementing audited
  standard libraries. Both are new production dependencies introduced in M11c-3 (architecture-consent;
  exact versions/features fixed at plan time). Binary-size delta is expected well under the 60 MiB CI
  budget; confirmed at c-3 plan time via cargo-bloat.

## 2. Cross-cutting resolutions (checkpoint-level)

These refine the parent spec; they are design decisions, not open questions.

### 2.1 Content-model boundary (c-1 defines the type; c-3 produces it)
The safe content model is a `Segment` list. Under the layered decomposition:
- **c-1 defines the `Segment` type taxonomy** as pure serde data (text-run, mark, link, image,
  roll-embed, doc-link, preview-card) plus a **trivial plain-text producer** (escape → single
  text-run) so c-1 can round-trip a real message end-to-end through the Event/redaction path.
- **c-3 owns the real producer** — the `ammonia`/`pulldown-cmark` sanitizer, GM content-policy
  toggles, and command parser that *populate* those segments from raw input.

Defining the type vs. producing safe instances is the model/producer seam that keeps c-1 testable
while honoring the layered-by-subsystem decomposition.

### 2.2 Wire-type strategy
The content model rides inside the message document's `system` body as **opaque serde JSON**, exactly
like every other document's system body (scene, actor). Therefore **no ts-rs bindings for the content
model**; M11d declares whatever client Zod mirror it needs. XSS-safety is a property of **M11d
rendering via components (never `innerHTML`)**, not of the wire schema. The single new ts-rs frame is
the **`SendMessage` intent** (raw content + target channel), defined and handled server-side in c-3 —
a plain document `Create` would persist unsanitized client content, so the dedicated intent is what
forces the raw → sanitize → persist ordering.

### 2.3 Channel seeding split
c-1 ships the `chat-channel` config-doc **type**, server-side channel resolution, and the **implicit
"All" fallback** (chat works with zero channel docs present). **Seeding the six defaults**
(All / Combat / Whispers / Rolls / Emotes / System) is a **client chat module → M11d**, mirroring how
`module-factions` / `module-conditions` idempotently seed their registries. c-1 is channel-*aware* but
seed-*agnostic*.

### 2.4 Reuse, not new infrastructure
Message size cap + per-user flood limiting reuse the existing rate-limiter pattern (the pre-M10
per-user ping limiter, the asset-replace rate limit) and `validation.rs` structural checks
(`deny_unknown_fields`, field-path caps). No new limiter machinery.

### 2.5 `roll` field redundancy (micro-decision)
The parent spec §2 lists both a top-level `roll: Option<RollResult>` field **and** a roll-embed
`Segment`. This is redundant. **Resolution: roll data lives only in the roll-embed `Segment`**; there
is no separate top-level `roll` field. The segment is defined (empty) in c-1 and populated by the
dice-engine wiring in M11d.

### 2.6 HTTP client (deferred to the c-4 plan)
The outbound HTTP client for the preview fetcher (reqwest-with-custom-resolver vs. `hyper` directly —
DNS-rebind protection requires validating the *connected* IP, which favors lower-level control) is a
c-4-local architecture-consent decision, settled in the c-4 brainstorm/plan, not locked here.

## 3. Decomposition

Strict order **c-1 → c-2 → c-3 → c-4**; each depends on the prior. One plan+execute cycle per
checkpoint (`/clear` between), following the M11b cadence. Each checkpoint updates
`shadowcat-codebase-chat` under the reviewed skill-update gate.

### M11c-1 · Message model + channels + delivery — *risk: low-med*
- Server `doc_type: "message"` (world-scoped, no scene parent). `system` body: `channel`,
  `user_owner` (= `owner_id`), `actor_owner: Option<ActorOwnerRef>` (`Actor{actor_id}` |
  `TokenInstance{token_id}`), `kind` (`Normal`|`Emote`|`Roll`|`System`), `content` (`Vec<Segment>`),
  `recipients: Option<Vec<UserId>>`.
- `Segment` taxonomy as pure serde data + trivial plain-text producer (§2.1).
- `chat-channel` config-doc type + server channel resolution + implicit-"All" fallback (§2.3).
- **Delivery = prove the existing Event/redaction path carries message docs** (create → broadcast →
  resync → search) + message-specific structural validation and permission defaults
  (`permissions.default = All`). No new transport code — the deliverable is the proof + the doc-type
  wiring.
- Size cap + per-user flood limiter (reuse, §2.4).
- **Creates the `shadowcat-codebase-chat` skill** (parent §10); adds its globs to the activation hook.
- Buddy-check: optional (plumbing + reuse); standard two-reviewer gate per task.

### M11c-2 · Whisper recipient allowlist — *risk: HIGH (architecture-consent)*
- Extend `resolve_access` / `Access::can_see` with a **fail-closed per-doc recipient allowlist** read
  from the message's `recipients`: visible only to `user_owner`, any listed recipient, and (per world
  policy, default-on, layered on top of the allowlist) the GM. Everyone else is denied
  **whole-document** — the message never enters their egress stream.
- Enforced through the **single** `can_see` / `filter_properties` chokepoint, so every egress route
  (initial load, broadcast, resync, snapshot, search) inherits it. Search indexing treats whispered
  content as non-public (visibility-partitioned-index invariant).
- **Mandatory buddy-check + two blind security reviews** (this widens the core permission model — the
  parent spec's one flagged architecture-consent item). Per-recipient test: a whisper reaches no
  non-recipient on *every* egress path, including search and resync.
- Depends on c-1 (`recipients` field exists).

### M11c-3 · Sanitizer pipeline + commands — *risk: HIGH (XSS core)*
- **Command parser:** `/me` `/em` `/emote` → `Emote`; `/w` / whisper → `recipients` (uses c-2);
  `/roll` `/1d6` → `Roll` **kind only, detected not executed** (§1). Strips the command token, sets
  structured fields.
- **Content-policy enforcement** reads the world/GM chat-settings doc (enrichment toggles). Producers:
  `pulldown-cmark` (Markdown → segments), `ammonia` (HTML → sanitized-HTML segment, strict
  tag/attr allowlist). **Embedded CSS always stripped** (no `style` attrs, `<style>`, or CSS-bearing
  attributes) regardless of settings. Scheme allowlist for links/images (`http`/`https` only; reject
  `javascript:` and unexpected `data:`); image content-type/extension checks (png/webp/jpg).
  Markdown / HTML / Images / Hyperlinks / Emails each gated by their own GM toggle.
- **`SendMessage` intent frame (ts-rs) + server handler**: raw → sanitize → persist → broadcast
  (§2.2).
- **Emote** stores the verbatim remainder; per-viewer name-prepend + italic reversal is render-time
  (M11d).
- New production deps land here (`ammonia`, `pulldown-cmark`; §1).
- **Mandatory buddy-check + security review**: an XSS payload corpus is neutralized, CSS is always
  stripped, and each GM toggle is honored.
- Depends on c-1 (content taxonomy) + c-2 (`/w` allowlist).

### M11c-4 · Link-preview fetcher — *risk: HIGH (SSRF, outbound HTTP)*
- Server-side guarded fetcher, **default-ON**, triggered at ingest for URLs in an enabled-hyperlink
  message; result cached by URL (repeats don't re-fetch).
- **SSRF/abuse guards:** block private / loopback / link-local / unique-local ranges (IPv4 and IPv6);
  **DNS-rebind protection** (validate the *connected* IP, not just the resolved-at-check IP);
  **`http`/`https` only**; **re-validate on every redirect hop** (cap hop count); connect + total
  **timeout**; streamed **response-size cap** (abort past cap); content-type check (HTML only).
- Stores `title` + short `description` (+ optional image URL, itself scheme/size-checked) as a
  **preview-card `Segment`**. Clients render the stored card — **no client-side outbound fetch**
  (never leaks a viewer's IP). GM per-world toggle; failures degrade to a plain enriched link.
- HTTP-client dep chosen at plan time (§2.6).
- **Mandatory buddy-check + security review** against a **stub HTTP target** (private-IP /
  redirect-to-private / oversized / non-HTML all rejected; no real network).
- Depends on c-3 (link segments + the ingest pipeline).

## 4. Testing strategy (server, M11c)

Per parent §8, scoped to the server half:
- **c-1:** message doc round-trips create → broadcast → resync → search; channel resolution +
  implicit-All fallback; size cap + flood limiter; `ActorOwnerRef` (both variants) serde.
- **c-2:** whisper allowlist — a non-recipient gets nothing on *every* egress path (load / broadcast /
  resync / snapshot / search); GM-policy layer on/off; fail-closed on a malformed `recipients`.
- **c-3:** sanitizer XSS-payload corpus neutralized; CSS always stripped; each GM toggle honored;
  command parser (each command → correct structured fields); scheme/content-type rejection.
- **c-4:** SSRF guards via a stub HTTP target (private-IP / redirect-to-private / oversized / non-HTML
  rejected); cache-by-URL; graceful degradation. No real network.
- **Cross-cutting (with M11d):** a per-recipient redaction test proving a whisper and a hidden
  actor-name never reach an unauthorized client — for both a linked (`Actor`) and an instanced
  (`TokenInstance`) owner, plus a dangling-ref case that fails closed to the fallback name. The
  server-observable half lands with M11c; the client half completes in M11d.

## 5. Codebase-skill gate

M11c opens a subsystem no existing `shadowcat-codebase-*` skill covers. A new
**`shadowcat-codebase-chat`** skill is created in c-1 (fixed shape; globs added to the activation
hook) and updated at each subsequent checkpoint, reviewed by `shadowcat-spec-reviewer` under the
reviewed skill-update gate before each merge.
