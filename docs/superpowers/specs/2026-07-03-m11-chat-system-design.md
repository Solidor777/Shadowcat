# M11 Chat System — Design

**Status:** Approved design (brainstorm complete), pre-plan.
**Scope:** M11c (headless chat core) + M11d (default display modules) as one spec.
**Depends on:** the dice engine (M11a/b, separate spec `2026-07-03-m11-dice-engine-design.md`) for
roll integration in M11d. Chat *core* (M11c) does not depend on dice.

## 1. Context & decomposition

Part 2 of M11. Companion to the dice engine spec; both are sub-milestones of M11 (see the dice spec
§1 for the full M11 decomposition). This document refines the roadmap: **M12's "chat panel" line is
superseded** — the baseline display modules land in M11d.

| Sub | Title | Depends on |
|-----|-------|-----------|
| **M11c** | Chat core (headless) | — (parallel-able with dice) |
| **M11d** | Default display modules | M11a/b (roll integration), M11c |

### Decided architecture (foundational, not re-litigated below)

1. **Messages are ordinary sequenced `Document`s** — not aux broadcast frames. Edits,
   roll-updates, and gap-resync all ride the normal authoritative Event path and the existing
   per-recipient redaction chokepoint (`can_see` / `filter_properties`). See
   `shadowcat-codebase-documents-permissions` and `-realtime-sync`.
2. **Sanitization + content-policy enforcement is server-authoritative at ingest.** The stored
   message is already-safe canonical content; the client only renders it. A malicious client
   cannot inject disallowed HTML/CSS/scripts, because the server — not the client — enforces the
   world's content policy.
3. **Display is UI-as-modules.** The default display is a small set of independently-replaceable
   contribution modules over Surfaces, consistent with the M7 contribution architecture.
4. **Link previews:** a server-side, SSRF-guarded fetcher, default-ON (§6).

## 2. Message document model & channels

**Message = a top-level `Document`** (`type: "message"`), world-scoped (no scene parent). Rides the
same storage, validation, sequencing, and redaction as every other document.

`system` body:

| Field | Meaning |
|-------|---------|
| `channel` | channel key/DocId the message belongs to |
| `user_owner` | owning user (= the doc's `owner_id`); always present |
| `actor_owner: Option<ActorOwnerRef>` | optional actor attribution (origin tracking); tagged ref, linked **or** instanced (§2.1) |
| `kind` | `Normal` \| `Emote` \| `Roll` \| `System` — subtype, orthogonal to `channel` |
| `content` | the **sanitized canonical content model** (§4): text runs, allowed marks, links, images, roll-embeds, doc-links, preview-cards |
| `roll: Option<RollResult>` | embedded dice result for roll messages / precomputed embeds |
| `recipients: Option<[UserId]>` | whisper allowlist; `None` = visible per the channel's tier |

### 2.1 Actor-owner reference (linked vs instanced)

A chat message's actor owner may be a **linked** actor (a shared world-scoped `Actor` document) or
an **instanced/unlinked** actor (a deep-cloned copy embedded on a token, with no standalone doc —
see `shadowcat-codebase-actors-tokens`). One bare DocId cannot express both, so `actor_owner` is a
tagged reference:

```
enum ActorOwnerRef {
    Actor        { actor_id: DocId },   // linked: canonical Actor document
    TokenInstance { token_id: DocId },  // instanced (or any token-borne) actor: resolve through the token
}
```

- **Resolution** is via the existing read-through: `Actor` → the actor doc; `TokenInstance` →
  `resolveTokenActor(token)` on the token's embedded copy. Both yield an `EffectiveActor` whose
  display name comes from `actorDisplayName(a, fallback)` — so **name privacy (`setNameHidden` →
  `/system/name` = `OwnerOrGm`) is honored per-viewer** for either kind (§4).
- **Provenance / dangling:** an instanced actor's identity is inseparable from its token; if that
  token is later deleted, a `TokenInstance` ref dangles. Resolution then **fails closed to the
  redaction fallback name** — the private name is *never* snapshotted into the stored message (that
  would bypass per-viewer redaction). A linked `Actor` ref survives independent of any token.

**Permissions:**
- Normal messages: `permissions.default = All` (owner + GM may edit/delete).
- **Whispers:** set `recipients` → enforced by a **new fail-closed per-doc recipient allowlist**
  added to `resolve_access` / `Access::can_see` (§3). Visible to sender + listed recipients + GM.
  GM-sees-whispers is a world policy (default on, for moderation), layered on top of the allowlist,
  not baked into it.

**Channels are world config documents** (`type: "chat-channel"`), **seeded by a default chat
module, not hardcoded.** Each carries `key`, `label`, `order`, and a visibility/post policy (e.g.
System = GM-post-only). Defaults seeded: **All, Combat, Whispers, Rolls, Emotes, System**. With
**no channel docs present, an implicit "All" channel** is the fallback so chat always works.

The core stores `channel` + `kind` per message; the **seeding module decides routing policy** (e.g.
whether "Rolls"/"Emotes" are real routing targets or kind-filtered views over All), keeping the
core model minimal and un-opinionated.

## 3. Whisper recipient allowlist (redaction extension)

The current visibility model (`shadowcat-codebase-documents-permissions`) has tiers `All`,
`GmOnly`, `OwnerOrGm` and a single `can_see(v)` chokepoint — but **no "visible to a specific set of
users" concept**, which whispers require. This spec adds one, at the same chokepoint, fail-closed:

- A message with `recipients: Some([UserIds])` is visible only to: its `user_owner`, any listed
  recipient, and (per world policy) the GM. Everyone else is denied **whole-document** (the message
  never enters their egress stream — not sent-then-hidden).
- Implemented as an extension to `resolve_access` (the allowlist is read from the message being
  filtered) and enforced through the one `can_see`/`filter_properties` path, so every egress route
  (initial load, broadcast, resync, snapshot, search) inherits it. **Search indexing** must treat
  whispered content as non-public (visibility-partitioned index invariant).
- **This is an architecture-consent item** — it widens the core permission model. It is scoped
  narrowly (whole-doc recipient gate, additive to the existing tiers) and must be reviewed as such.

## 4. Input → sanitization pipeline (server-authoritative)

**Flow: raw input → server sanitize → persist → broadcast.** The client sends a `SendMessage`
intent with **raw** content + target channel; it may parse commands locally for preview UX but
never for trust. All authority is server-side, at the ingest chokepoint before persist/broadcast:

1. **Command parse** — leading `/me` `/em` `/emote` → `kind: Emote`; `/roll` / `/1d6` →
   `kind: Roll`; `/w` / whisper → `recipients`. Sets structured fields; strips the command token.
2. **Content-policy enforcement** (reads the world/GM chat settings doc — the enrichment toggles).
   The server produces a **structured, sanitized content model** — an AST/segment list (text runs,
   allowed marks, links, images, roll-embeds, doc-links, preview-cards) — **not a raw HTML blob.**
   The client renders that model to DOM via components (no `innerHTML` of untrusted strings) →
   XSS-safe by construction. Where the GM *allows* raw HTML, it is sanitized server-side
   (ammonia-style strict tag/attr allowlist) into a dedicated sanitized-HTML segment. **Embedded CSS
   is always stripped** (no `style` attrs, no `<style>`, no CSS-bearing attributes) regardless of
   settings. Markdown is parsed server-side (pulldown-cmark-style) into the same safe model.
   Markdown / HTML / Images / Hyperlinks / Emails are each gated by their own GM toggle.
3. **Malicious-data safeguards** — message size cap; per-user rate/flood limiting; scheme allowlist
   for links/images (`http`/`https` only; reject `javascript:` and unexpected `data:`); image
   content-type/extension checks (png/webp/jpg); reuse `validation.rs` structural validation
   (`deny_unknown_fields`, field-path caps).
4. **Roll handling** — `/roll` and precomputed embeds invoke the **dice engine authoritatively** at
   ingest → embed a `RollResult`. Roll **buttons** (deferred rolls) store only the parsed formula
   ref; no execution until clicked.

**Per-viewer resolution (never baked into the stored message):**
- **Actor name** — the message stores the `actor_owner` **ref** (§2.1), not a name. Whether a
  viewer sees the actor's *name* is resolved at egress/render via the read-through
  (`resolveTokenActor` / `actorDisplayName`) and the `OwnerOrGm` name-privacy tier — dispatching on
  the ref variant (canonical actor doc vs the token's embedded copy). One stored message shows the
  name to permitted viewers and the redaction fallback to others; a dangling `TokenInstance` ref
  also falls back.
- **Whisper** — the `recipients` allowlist (§3); non-recipients never receive the message.

**Emote transform** — `kind: Emote` stores the verbatim remainder ("sticks out their tongue"); the
display prepends the per-viewer-resolved name (user, or actor name when `actor_owner` is set) and
renders italic, with explicit italic markup **reversed** (non-italic) inside emotes. Pronoun
rewriting (his→their) is out of scope — text kept verbatim.

## 5. Default display modules (M11d)

Independently-replaceable contribution modules over Surfaces:

1. **Chat panel host** — declares the panel region: channel tabs/selector, the (virtualized)
   scrolling message list, and the mount Surfaces below. Owns layout + active channel + ordering.
2. **Message bar (composer) module** → contributes into a `chat.composer` Surface: input field,
   command affordances, send; emits `SendMessage` intents with **raw** content. Replaceable alone.
3. **Message card (renderer) module** → contributes into a per-message `chat.message` Surface,
   invoked with the sanitized content model; renders header + body + embeds. Replaceable alone.

A system builder swaps the composer, the card, or both, without touching the other or the host.

**Default message-card rendering:**
- **Header** — user owner always; actor owner shown when present *and* the viewer may see the
  actor's name (per-viewer, §4). When shown, the actor name links to its sheet — the canonical
  actor sheet for an `Actor` ref, the token's instanced actor sheet for a `TokenInstance` ref —
  permission-gated like any internal doc link below.
- **Body** — renders the sanitized content model via components (no untrusted `innerHTML`); emotes
  italic + reversed-italic.
- **Emoji** — universal unicode + `:shortcode:` → unicode.
- **Images** (GM on/off) — forced to a new line, sized to the card, click → original in a new tab;
  png/webp/jpg (gif deferred).
- **Hyperlinks** — `Enabled(Remove Aliases)` default / `Enabled(Allow Aliases)` / `Disabled`;
  enriched anchors + a **preview card** (title + short description) at the bottom (§6).
- **Emails** — `Enabled(Remove Aliases)` default / `Enabled(Allow Aliases)` / `Disabled`; enriched
  mailto.
- **Roll embeds** (depend on dice M11a/b):
  - **Result message** (`/roll 1d6`) — total/successes + individual dice + modifiers.
  - **Precomputed inline** — highlighted, bordered element; tooltip = individual dice + modifiers
    (rolled at ingest).
  - **Roll button** — in-message formula → clickable → sends a Roll intent → produces a *new* roll
    message. Supports **aliasing** (`Roll 1d6` displayed as "Roll attack" — label vs formula).
- **Internal doc links** — link a doc (e.g. an actor) → clickable button; on click opens that doc's
  sheet **if the clicker is permitted** (data already server-filtered; button greys/no-ops
  otherwise).

## 6. Link preview fetcher (server-side, SSRF-guarded)

Approved: server-side guarded fetcher, **default-ON**.

- Fetch triggered at ingest for URLs in an enabled-hyperlink message; result cached by URL so
  repeats don't re-fetch.
- **SSRF/abuse guards:** resolve host and **block private / loopback / link-local / unique-local
  ranges** (IPv4 and IPv6); DNS-rebinding protection (validate the *connected* IP, not just the
  resolved-at-check IP); **`http`/`https` only**; **re-validate on every redirect hop** (cap hop
  count); connect + total **timeout**; **response-size cap** (stream, abort past cap);
  content-type check (HTML only for preview parse).
- Stores `title` + short `description` (and optionally an image URL, itself scheme/size-checked) as
  a **preview-card segment** on the message content model. Clients render the stored card — **no
  client-side outbound fetch** (never leaks a viewer's IP).
- GM toggle can disable previews per world; failures degrade gracefully to a plain enriched link.

## 7. Delivery, edits & updates

- Messages persist and broadcast as authoritative seq'd Events; late joiners and gap-resync get
  them via the normal `RingBuffer` / snapshot path.
- **Edits / deletes** are document updates (owner + GM), redacted per recipient like any doc.
- **Roll updates** (the recalculation use case, e.g. reroll-failures from the dice spec) are edits
  to the message's embedded `RollResult` — the message doc mutates, the update broadcasts, all
  viewers re-render. This is *why* messages are documents.

## 8. Testing strategy

- **Server (M11c):** unit tests for the sanitizer (XSS payloads neutralized; CSS always stripped;
  each GM toggle honored), the command parser, the whisper allowlist (non-recipients get nothing on
  every egress path incl. search/resync), rate/size caps, and the preview fetcher's SSRF guards
  (private-IP / redirect-to-private / oversized / non-HTML all rejected) using a stub HTTP target —
  no real network.
- **Client (M11d):** component tests for the composer (command → intent), the message card
  (each enrichment kind, emote reversal, per-viewer name gating, roll embeds, doc-link permission
  behavior), and module-replaceability (swap composer/card via contribution and re-render).
- **Cross-cutting:** a per-recipient redaction test proving a whisper and a hidden actor-name never
  reach an unauthorized client — for **both** a linked (`Actor`) and an instanced (`TokenInstance`)
  actor owner, plus a dangling-ref (token deleted) case that fails closed to the fallback name.

## 9. Deferred (noted, not built)

- GIF / animated images.
- Pronoun rewriting in emotes.
- Rich message components beyond the listed set (arbitrary interactive widgets) — systems extend via
  replacing the card module.

## 10. Codebase-skill gate

M11 chat opens a subsystem no existing `shadowcat-codebase-*` skill covers. Per CLAUDE.md, a new
**`shadowcat-codebase-chat`** skill must be created (fixed shape; globs added to the activation
hook) and reviewed as part of the skill-update gate before merge. (The dice spec separately requires
`shadowcat-codebase-dice`.)
