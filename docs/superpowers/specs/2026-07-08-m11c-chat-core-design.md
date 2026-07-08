# M11c Chat Core (headless) — Checkpoint Design

**Status:** Approved design (brainstorm complete), pre-plan.
**Parent spec:** `2026-07-03-m11-chat-system-design.md` (cross-cutting M11c+M11d decisions — locked, not
re-litigated here). This document refines **M11c** (the headless chat core) into four ordered
sub-checkpoints and records the checkpoint-level resolutions the parent spec left implicit — including
the codebase-grounded correction that the message model is **server-constructed Rust**, not a client
document builder (see §1).
**Scope boundary:** M11c is the **headless chat core** — server-side (Rust) plus the minimal headless
client-core wire mirror — and is **dice-independent** (parent spec §1, §8). It establishes
**server-authoritative message ingest**: the client sends raw content via a `SendMessage` intent and
the **server constructs** the safe, stored message `Document` (never the client). Svelte display
modules (composer, message card) and roll-at-ingest wiring are **M11d**.

## 1. Boundary & approved choices

- **Server-authoritative ingest, not "server-only."** The server owns
  ingest → sanitize → **construct** → persist → broadcast, the message document model (Rust structs
  serialized into the opaque `system` body), and the whisper redaction tier. This is a codebase-grounded
  correction to an earlier framing: because the server *constructs* the message doc, the model is
  **server Rust** — *not* a client `scene-docs.ts` builder like other doc types. The server today is
  structural-only and stores client-authored docs verbatim; a `message` must instead be built by the
  server from a `SendMessage` intent, and ordinary client `Create` of a `message` doc is **rejected**
  server-side (only `SendMessage` may author a message). The headless client core gains only the **Zod
  mirror** of the new `SendMessage` / `ActorOwnerRef` wire types (`wire.ts`); the composer/card that
  emit and render them are M11d.
- **Channels stay client-built + M11d-seeded.** `chat-channel` config docs are GM config, not user
  content needing sanitization, so they follow the existing `faction-registry` / `condition-registry`
  client pattern (a `scene-docs.ts` builder + an M11d module that idempotently seeds the defaults).
  A c-1 message carries a `channel` string; the config-doc type + seeding is M11d.
- **Dice-independent.** `/roll` is detected as a `kind` at ingest (in c-3's command parser) but **not
  executed** in M11c; the dice-engine invocation + `RollResult` embed is M11d (keeps M11c parallel-able
  with dice per parent §1, and avoids coupling the chat core to `dice/`).
- **Sanitizer = vetted crates, not clean-room.** HTML sanitization via **`ammonia`** (strict
  tag/attr allowlist) and Markdown via **`pulldown-cmark`**. A security boundary is not hand-rolled
  (forward-thinking discipline); IP clean-room rules concern copied code, not reimplementing audited
  standard libraries. Both are new production dependencies introduced in **M11c-3** (architecture-consent;
  exact versions/features fixed at plan time). Binary-size delta is expected well under the 60 MiB CI
  budget; confirmed at c-3 plan time via cargo-bloat.

## 2. Cross-cutting resolutions (checkpoint-level)

These refine the parent spec; they are design decisions, not open questions.

### 2.1 Content-model boundary (c-1 defines the type + server ingest; c-3 enriches the producer)
The safe content model is a `Segment` list, defined **server-side (Rust `serde`, no `TS` derive)** and
serialized into the message's opaque `system` body. Under the layered decomposition:
- **c-1 defines the `Segment` enum with only the `Text` variant it produces** (extensible — each later
  checkpoint adds the variants it produces: c-3 mark/link/image, c-4 preview-card, M11d roll-embed;
  speculative fields for unbuilt producers are YAGNI) **and the server-authoritative ingest spine**
  (`SendMessage` intent → server builds the message `Document`) with a **trivial plain-text producer**
  (raw → single text-run; safety is a *rendering* property — M11d renders `Text` as a DOM text node,
  never `innerHTML` — so the producer does not escape), so c-1 round-trips a real, server-constructed
  message end-to-end through the Event/redaction/search path.
- **c-3 enriches the producer** — the `ammonia` / `pulldown-cmark` sanitizer, GM content-policy
  toggles, and command parser feeding the **same** c-1 ingest path (no new frame, no client-create path
  to remove).

Building the ingest spine in c-1 rather than a throwaway client-create path honors forward-thinking
discipline; c-3 is purely additive to it.

### 2.2 Wire-type strategy
The content model rides inside the message document's `system` body as **opaque serde JSON**, exactly
like every other document's system body (scene, actor) — **no ts-rs bindings for the content model**;
M11d declares whatever client Zod mirror it needs to render. XSS-safety is a property of **M11d
rendering via components (never `innerHTML`)**, not of the wire schema.

The new ts-rs surface is the **`SendMessage` intent** — a new `ClientMsg` variant in
`ws/protocol.rs` (`#[serde(tag = "type", rename_all = "snake_case")]`) carrying `channel`, raw
`content`, and `actor_owner: Option<ActorOwnerRef>` — plus **`ActorOwnerRef`** itself (its own
`#[serde(tag = "kind", rename_all = "snake_case")]` ts-rs enum, mirroring `Scope` / `Operation`,
variants `Actor { actor_id: Uuid }` and `TokenInstance { token_id: Uuid }`; there are no ID newtypes —
bare `Uuid`). Both land in **c-1** and regenerate under `src/types/generated/` (ts-rs emits on `cargo
test`; CI `git diff --exit-code`s the result). Their Zod mirrors (`z.discriminatedUnion`) are added to
`src/client/core/src/wire.ts`.

### 2.3 Channel model
A message carries `channel: String`. The `chat-channel` config-doc *type* + the six seeded defaults
(All / Combat / Whispers / Rolls / Emotes / System) + the implicit-"All" fallback are a **client chat
module → M11d**, mirroring `module-factions` / `module-conditions`. The server does not validate a
message's `channel` against channel docs (structural-only philosophy) — it records the key verbatim.

### 2.4 Reuse, not new infrastructure
Per-user flood limiting reuses the `PingRateLimiter` sliding-window pattern verbatim
(`ws/mod.rs:19-41`: `Mutex<HashMap<Uuid, Vec<i64>>>` + `check(user, now_ms, per_min)`), stored on
`WsState` behind `Arc`. The generic 256 KB `validate_system_size` cap (`validation.rs:11`) already runs
on every create; a tighter chat-specific length cap is applied in the `SendMessage` handler before
construction.

### 2.5 `roll` field redundancy (micro-decision)
The parent spec §2 lists both a top-level `roll: Option<RollResult>` field **and** a roll-embed
`Segment`. This is redundant. **Resolution: roll data lives only in the roll-embed `Segment`**; there
is no separate top-level `roll` field. The segment variant is defined (unpopulated) in c-1 and filled
by the dice-engine wiring in M11d.

### 2.6 HTTP client (deferred to the c-4 plan)
The outbound HTTP client for the preview fetcher (reqwest-with-custom-resolver vs. `hyper` directly —
DNS-rebind protection requires validating the *connected* IP, which favors lower-level control) is a
c-4-local architecture-consent decision, settled in the c-4 brainstorm/plan, not locked here.

## 3. Decomposition

Strict order **c-1 → c-2 → c-3 → c-4**; each depends on the prior. One plan+execute cycle per
checkpoint (`/clear` between), following the M11b cadence. Each checkpoint updates
`shadowcat-codebase-chat` under the reviewed skill-update gate.

### M11c-1 · Message model + server ingest + delivery — *risk: med*
- New server chat module (e.g. `src/server/src/chat/mod.rs`): the message `system`-body Rust structs —
  `MessageSystem` (`channel`, `user_owner`, `actor_owner: Option<ActorOwnerRef>`, `kind`, `content:
  Vec<Segment>`; `recipients` reserved for c-2), `MessageKind` (`Normal`|`Emote`|`Roll`|`System`),
  the `Segment` taxonomy, and `ActorOwnerRef` (the one ts-rs-exported chat type). Plain-text producer
  (escape → single text-run) + a `build_message_doc(...)` server constructor.
- **`SendMessage`** `ClientMsg` variant (ts-rs) + a `conn.rs` handler: flood-limit → build the message
  `Document` server-side (server sets `user_owner` = authenticated user, `owner`, timestamps;
  `kind = Normal` in c-1) → publish via the existing authoritative path as an `Operation::Create`.
- **Server-authored authorization:** posting a message is a **baseline world-member right**, not the
  GM-only `core:create` gate. The `SendMessage` path authorizes accordingly (exact seam — a
  trusted/system-authored publish vs. a seeded member grant — set by the c-1 authz mapping; the M1
  `execute_move` server-authoritative-write precedent is the reference).
- **Reject client-authored messages:** an ordinary client `Create` (or `Update`) of a `message`
  doc_type is refused, so `SendMessage` is the sole authoring path (this is what makes ingest
  server-authoritative).
- **Delivery = prove the generic path carries it** — create → sequence → ring/broadcast → egress →
  resync → search-index — with **no new transport/index code** (all doc_type-generic per the codebase
  map). Deliverable is the end-to-end proof + the `SendMessage` spine.
- Client core: add the `SendMessage` / `ActorOwnerRef` Zod mirrors to `wire.ts`.
- **Creates the `shadowcat-codebase-chat` skill** (parent §10); adds its globs to the activation hook.
- Buddy-check: not pre-authorized (the server-authored authz seam is the one spot warranting a careful
  standard two-reviewer pass); escalate to buddy-check only if that seam proves subtle.

### M11c-2 · Whisper recipient allowlist — *risk: HIGH (architecture-consent)*
- Add `recipients: Option<Vec<Uuid>>` to `SendMessage` + the message body; the server enforces a
  **fail-closed whole-document READ suppression** for non-recipients: a whisper is visible only to its
  `user_owner`, any listed recipient, and (per world policy, default-on) the GM — everyone else never
  receives the doc (not sent-then-hidden). Implemented at the single `resolve_access` / `can_see`
  chokepoint (`permission.rs`), which already receives the full `&Document` (so it can read
  `recipients` from `system`) and is shared by every egress route.
- **Whole-doc suppression, not property-strip** — because the FTS index is a **binary GM/non-GM
  partition** (`content` vs `content_all`) that cannot express per-recipient visibility, a whisper must
  drop out of the READ gate entirely (`sqlite.rs` per-hit `resolve_access_world` + READ check) so it
  never surfaces in a non-recipient's search, load, broadcast, or resync.
- **Mandatory buddy-check + two blind security reviews.** Per-recipient test: a whisper reaches no
  non-recipient on *every* egress path, including search and resync.
- Depends on c-1 (`SendMessage` + message model exist).

### M11c-3 · Sanitizer producer + commands — *risk: HIGH (XSS core)*
- **Enrich the c-1 producer** (same `SendMessage` path — no new frame): `pulldown-cmark`
  (Markdown → segments), `ammonia` (HTML → sanitized-HTML segment, strict allowlist). **Embedded CSS
  always stripped** (no `style` attrs, `<style>`, CSS-bearing attributes) regardless of settings.
  Scheme allowlist for links/images (`http`/`https` only; reject `javascript:` / unexpected `data:`);
  image content-type/extension checks (png/webp/jpg). Markdown / HTML / Images / Hyperlinks / Emails
  each gated by their own GM toggle (read from the world/GM chat-settings doc).
- **Command parser:** `/me` `/em` `/emote` → `Emote`; `/w` → `recipients` (uses c-2); `/roll` `/1d6`
  → `Roll` **kind only, detected not executed** (§1, dice wiring is M11d). Strips the command token,
  sets structured fields.
- **Emote** stores the verbatim remainder; per-viewer name-prepend + italic reversal is render-time
  (M11d).
- New production deps land here (`ammonia`, `pulldown-cmark`; §1).
- **Mandatory buddy-check + security review**: an XSS payload corpus is neutralized, CSS is always
  stripped, and each GM toggle is honored.
- Depends on c-1 (ingest spine + content taxonomy) + c-2 (`/w` allowlist).

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

## 4. Testing strategy (M11c)

Per parent §8:
- **c-1:** a `SendMessage` builds a server-constructed message doc that round-trips create → broadcast
  → resync → search; client `Create` of a `message` doc is rejected; flood limiter; `ActorOwnerRef`
  (both variants) serde round-trip; `SendMessage`/`ActorOwnerRef` ts-rs↔Zod parity.
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
