# M11c-3 Sanitizer + Commands + Edit/Delete — Design

**Status:** Approved design (brainstorm complete), pre-plan.
**Parent spec:** `2026-07-08-m11c-chat-core-design.md` §3 "M11c-3 · Sanitizer producer + commands" —
this document supersedes that section's implementation sketch with a codebase-grounded design and
extends it (validated edit path + soft-delete, per parent chat-system spec §7 "Delivery, edits &
updates").
**Depends on:** M11c-1 (message model + server-authoritative `SendMessage` ingest — DONE, branch
`m11c-1-message-model`) and M11c-2 (restricted-audience `Audience` frame field + validation — DONE,
branch `m11c-2-restricted-audience-messaging`).

## 0. Scope and decisions from brainstorming

M11c-3 is **one checkpoint** (one plan+execute+review cycle, per parent §3 "one plan+execute cycle
per checkpoint"), with internal task ordering **sanitizer → commands → edit/delete** (each depends
on the prior: the edit path re-runs the sanitizer + command pipeline).

It bundles the three pieces the parent grouped, plus the delete half of the parent chat-system
spec's "edits / deletes" pair:

1. **Sanitizer producer** — enriches c-1's `plain_text_content` into a real Markdown/HTML → segment
   pipeline (`ammonia` + `pulldown-cmark`), gated by per-world GM content-policy toggles.
2. **Command parser** — server-side, at ingest: `/me`·`/roll` set message **kind**; `/w` sets
   **audience** (a second front-door to c-2's typed `Audience`).
3. **Validated edit path** — a dedicated server-authoritative `EditMessage` frame replacing c-1's
   blanket rejection of client `Update`s to a stored message doc.
4. **Soft delete** — a dedicated server-authoritative `DeleteMessage` frame (tombstone).

Decisions locked during brainstorming (each is an architecture-consent point the user approved):

- **D1 — Edit/delete are dedicated server-authoritative frames, not a re-opened generic `Update`
  path.** `EditMessage`/`DeleteMessage` mirror `SendMessage`: the server validates, re-runs the
  ingest pipeline, and publishes the authoritative revision. Sanitization is unbypassable by
  construction, and c-1's "`SendMessage` is the sole message-authoring path" invariant generalizes
  cleanly to "server-authored frames are the sole message-write path." See §5, §6.
- **D2 — GM content-policy toggles live in a `chat-settings` config Document, read fail-closed.**
  Absent or malformed settings ⇒ all enrichment OFF ⇒ plain text (c-1's safe baseline). The toggles
  only ever *widen* from that safe baseline, so a missing/cleared/corrupt settings doc degrades
  *safe*, never unsafe. See §3.
- **D3 — Content is a typed `Segment` union with sanitized-HTML runs, trusting `ammonia` once.**
  Inline formatting collapses into an `Html { sanitized_html }` segment (rendered via `innerHTML`
  of `ammonia`-guaranteed-safe HTML — the sanitizer's intended use); rich items (`Link`, `Image`,
  and reserved `RollEmbed`/`PreviewCard`/`DocLink`) stay typed segments so they interleave and
  render specially. `ammonia` is the single security boundary, crossed exactly once at ingest. See
  §2.
- **D4 — The server parses ALL commands, including `/w`; the typed `Audience` frame field
  survives.** `/w @user…` is parsed server-side (usernames resolved against the world roster) and
  funneled through the **same** c-2 `Audience` validation + permission mapping. It coexists
  additively with the typed frame field: an explicit `/w` in content wins; otherwise the frame's
  `audience` applies. This makes whispers work for *any* client that sends raw text (first-class
  modularity) while keeping the typed contract for structured clients. See §4.
- **D5 — Kind is server-authoritative; audience is client-supplied-but-validated.** A client may
  NOT assert `kind` on the wire (that would let it forge `System` notices); `kind` is derived
  server-side from command tokens. A client MAY supply `audience` because c-2 validates it and
  lying about it is self-limiting. This asymmetry is *why* command parsing is server-side. See §4.
- **D6 — An edit re-runs the full ingest pipeline but freezes audience/channel/actor_owner.** An
  edit re-sanitizes and MAY change `kind` (it is the author's own message), but MUST NOT re-target
  or move the message: a `/w` inside an edit is rejected, because changing audience post-hoc would
  retroactively change who can read the backlog. See §5.
- **D7 — Delete is a soft tombstone, not a hard removal.** The doc stays in the sequenced log;
  content is cleared and `deleted_at` set. No sequence gap, clean resync, history preserved
  (immutable-history + synced-log discipline). See §5.

`/roll` is detected as a `kind` but **not executed** (parent §1): the raw dice expression is stored
verbatim, and the dice-engine invocation + `RollResult` embed is M11d. This keeps the chat core
decoupled from `dice/`.

New production dependencies land here: **`ammonia`** (HTML sanitization, strict tag/attr allowlist)
and **`pulldown-cmark`** (Markdown → events). Exact versions/features are fixed at plan time, with a
`cargo-bloat` check confirming the binary-size delta stays well under the 60 MiB CI budget (parent
§1). These are vetted audited libraries, not clean-room reimplementations — the IP clean-room rule
concerns copied code, and a security boundary must not be hand-rolled (forward-thinking
discipline).

## 1. Producer entry point today (what c-3 replaces)

`chat::handle_send_message` is the sole message-authoring entry point. Today it validates
(empty / `MAX_MESSAGE_CHARS` / channel length / whisper recipient allowlist / per-user flood
budget), calls `plain_text_content(raw)` — which wraps the raw string as a single
`Segment::Text { text }` — and then `build_message_doc(...)` → `room.publish(vec![Operation::Create
{ doc }], ...)`. `build_message_doc` hardcodes `kind: MessageKind::Normal` (`chat/mod.rs`).

The `Segment` enum currently has only the `Text` variant; `MessageKind` already declares
`Normal`/`Emote`/`Roll`/`System` (only `Normal` is produced). `MessageSystem` carries
`channel`, `user_owner`, `actor_owner`, `kind`, `audience`, `content`.

c-3 changes the middle of that flow — **parse commands → resolve policy → sanitize → build** — and
threads `kind` into `build_message_doc`. The validation floor, flood budget, and `Room::publish`
tail are unchanged. Nothing in the redaction / sequencing / search / resync machinery changes: a
message remains an ordinary `Document`.

## 2. Content model (`Segment` union)

`Segment` stays a `serde`-only tagged enum (`#[serde(tag = "kind", rename_all = "snake_case")]`),
**not** ts-rs-exported — the client declares its own Zod mirror in M11d. c-3 adds:

```rust
pub enum Segment {
    Text { text: String },                       // c-1: literal, rendered as a DOM text node
    Html { sanitized_html: String },             // c-3: ammonia-guaranteed-safe formatted run
    // reserved, produced later — declared as they land, not before:
    //   RollEmbed { .. }    (M11d — dice execution)
    //   PreviewCard { .. }  (c-4 — link previews)
    //   DocLink { .. }      (M11d — internal doc links, permission-gated at render)
}
```

**Why sanitized-HTML runs, and why links/images stay INSIDE them (plan-time refinement).** The
brainstorm framing listed typed `Link`/`Image` segments for c-3. Deriving those, however, means
DOM-walking `ammonia`'s sanitized HTML back into a typed AST — precisely the "re-parse sanitized
HTML" this design's D3 principle rejects. The consistent resolution: **c-3's only new `Segment`
variant is `Html`.** Hyperlinks (`<a>`), images (`<img>`), and autolinked emails (`mailto:`) remain
*inside* the `ammonia`-sanitized `Html` run — their scheme and image content-type/extension
constraints are enforced by `ammonia`'s URL-scheme allowlist and attribute filters, not by a
post-sanitize re-parse. This trusts the sanitizer exactly once, at the boundary. No functional loss:
rendering (lazy-load, click-to-expand, permission-gated doc-links) is M11d's concern, and the typed
`Link`/`Image`/`DocLink`/`PreviewCard` variants are reserved for when a later checkpoint genuinely
needs a representation that plain sanitized HTML *cannot* express (a dice widget, a fetched preview
card, an internal doc reference). Rendering `Html { sanitized_html }` via `innerHTML` is the
*intended* use of a sanitizer; the c-1 "never innerHTML" phrasing is about **untrusted** HTML, and
`ammonia` output is trusted by construction.

**Producer matrix (which segment a source yields), driven by the policy toggles:**
- All toggles off → `vec![Text { text: raw }]` (fail-closed baseline, identical to c-1 behavior).
- Markdown on → `pulldown-cmark` renders Markdown to an HTML string → `ammonia`-sanitized → one
  `Html` run. When the HTML toggle is OFF, raw HTML the author embedded in their Markdown is escaped
  (`pulldown-cmark`'s raw-HTML passthrough disabled) so only Markdown-generated tags survive.
- HTML on (Markdown off) → the raw input is `ammonia`-sanitized directly → one `Html` run.
- `images` / `hyperlinks` / `emails` toggles add `<img>` / `<a href=http(s)>` / `mailto:` to the
  `ammonia` allowlist respectively; when a toggle is off, that element/scheme is stripped (tag
  removed or unwrapped to its text). CSS is *always* stripped regardless of toggles.

## 3. `chat-settings` config Document (D2)

A GM-writable, per-world `Document` with `doc_type: "chat-settings"`, sibling to the
`faction-registry` / `condition-registry` / `chat-channel` config docs. Its `system` body holds a
`ChatContentPolicy`:

```rust
pub struct ChatContentPolicy {
    pub markdown: bool,
    pub html: bool,
    pub images: bool,
    pub hyperlinks: bool,
    pub emails: bool,
}
```

**`Default` = all `false`** (plain text). The sanitizer reads the policy at ingest via a
fail-closed resolver: query the world's `chat-settings` doc; if absent, or present-but-malformed
(deserialize error / wrong shape), return `ChatContentPolicy::default()` (all off). A non-GM cannot
edit it (ordinary permission model gates writes to GM); the server never trusts it for anything but
*widening* enrichment, so a hostile edit can only reduce what's rendered.

**Hard property:** the toggles are permission-to-*enrich*, never permission-to-*restrict*. Absence
⇒ maximum safety. There is therefore no seeding requirement for correctness — an M11d GM UI may
idempotently seed a doc so the toggles are visible/editable, but c-3 is correct with no settings
doc present.

**Layering within an enabled kind:** `html: true` still strips embedded CSS unconditionally (no
`style` attributes, no `<style>`, no CSS-bearing attributes) — `ammonia`'s allowlist excludes them
regardless of policy. `images`/`hyperlinks` gate whether `Image`/`Link` segments are emitted at
all; `emails` gates `mailto:`/autolinked-email handling. Scheme allowlist (`http`/`https` for
links/images; reject `javascript:` and unexpected `data:`) and image content-type/extension checks
(png/webp/jpg) always apply when the respective kind is enabled.

`chat-settings` reads add one small per-message document lookup; it is cache-friendly (a single doc
per world) and does not touch the hot redaction path.

## 4. Command parser (`chat/commands.rs`) (D4, D5)

A pure, server-side parse over the raw content, run at ingest inside `handle_send_message` (and the
edit path, §5), producing a `ParsedCommand`:

```rust
struct ParsedCommand {
    kind: MessageKind,               // Normal | Emote | Roll  (never System — see below)
    whisper_to: Option<Vec<String>>, // Some(raw @usernames) only when a /w command was present
    body: String,                    // content with the command token + recipient list stripped
}
```

`parse_command` is **pure** (no repo, no async): it returns the raw `@username` strings, not
resolved UUIDs. Username→UUID resolution and the `Audience::Whisper` construction happen in the
async caller (`handle_send_message` / `handle_edit_message`), which has the `repo`. This keeps the
parser trivially unit-testable and confines all roster I/O to one place.

Grammar (leading-token only; a command must start the message):
- `/me`, `/em`, `/emote` → `kind = Emote`; `body` = verbatim remainder (per-viewer name-prepend +
  italic reversal is render-time, M11d).
- `/roll`, `/r`, and shorthand `/NdM` (e.g. `/1d6`) → `kind = Roll`; `body` = the **verbatim dice
  expression**, stored unparsed and unexecuted (M11d owns dice). c-3 does not depend on `dice/`.
- `/w @user1 @user2 … message` → `whisper_to = Some(["user1", "user2"])`, `body` = the message
  after the recipient list. `@`-prefixed tokens disambiguate recipients from the body. The async
  caller then resolves each `@username` against the world roster by **username** (the unique login
  identity — no ambiguity) into `Audience::Whisper { recipients }`. Unknown username, or recipient
  count over `MAX_WHISPER_RECIPIENTS`, → the whole `SendMessage`/`EditMessage` is rejected
  (fail-closed, symmetric with c-2's frame-path recipient validation).
- No leading command → `kind = Normal`, `audience = None`, `body` = the input unchanged.

**`System` is never producible by a command** — it is reserved for server-authored notices, so no
client input path can set it (D5: kind is server-authoritative, forgery-proof).

**Audience reconciliation (D4):** `handle_send_message` computes the effective audience: if
`parsed.whisper_to` is `Some`, it resolves those usernames into an `Audience::Whisper` and uses it;
otherwise it uses the c-2 typed `audience` frame field. An explicit `/w` in content wins; otherwise
the frame applies. **Both inputs funnel through the identical c-2 validation** (recipients are real
world members, whisper cannot downgrade the sender's `Owner`, recipient cap) and the identical
`build_message_doc` permission mapping. Nothing in c-2's frame path is walked back; `/w` is purely
an additional front-door.

Server-side username resolution is the one new roster capability: a
`Repository::member_id_by_username(world, &str) -> Option<Uuid>` (or resolve against the existing
member-listing query, `SELECT m.user_id, u.username … FROM members`). It is used only by the `/w`
parser; the frame path continues to take pre-resolved UUIDs.

## 5. Edit + delete frames (D1, D6, D7)

Two new server-authoritative client frames (`ws/protocol.rs`), each ts-rs-exported like
`SendMessage`:

```rust
ClientMsg::EditMessage   { message_id: Uuid, content: String }
ClientMsg::DeleteMessage { message_id: Uuid }
```

Dispatched in `ws/conn.rs` with their own arms (mirroring the `SendMessage` arm; success is
confirmed by the broadcast echo of the authored `Event`, not a direct reply). Like `SendMessage`,
these are **WS-only frames with no HTTP counterpart** — there is no HTTP dispatch to add. The
existing HTTP `write_ops` guard (`ops_target_message` rejects client `Create`/`Delete` message ops)
and `apply_intent`'s blanket `Update` rejection already cover the generic HTTP op path against
client message writes and stay intact; the §6 authoritative-marker exemption fires only for the
server-authored revision, never a client HTTP op.

**`handle_edit_message(room, repo, ctx, message_id, raw, now)`:**
1. Load the stored doc; reject if it is not a `message` doc_type or does not exist.
2. **Authorize:** the requester must be the message owner **or** a `WorldRole::Gm`. (A GM edit of a
   player's message is allowed, per parent §7.)
3. **Re-run the full ingest pipeline** on `raw`: command parse (§4) + sanitize (§2) under the
   current `chat-settings` policy (§3).
4. **Freeze audience/channel/actor_owner (D6):** if the parse yields an audience-setting command
   (`/w`), **reject the edit** — an edit may correct words and may change `kind`, but may not
   re-target the message (that would retroactively change backlog visibility). `channel` and
   `actor_owner` are likewise unchanged.
5. Build the revised `system` (new `content`, possibly new `kind`, `edited_at = Some(now)`) and
   publish the authoritative revision (§6). The document's `PermissionSet` is untouched.

**`handle_delete_message(room, repo, ctx, message_id, now)`:**
1. Load; reject if not a `message` doc.
2. **Authorize:** owner or GM (same rule as edit).
3. **Soft tombstone (D7):** clear `content` to empty (`vec![]`), set `deleted_at = Some(now)`; the
   doc remains in the sequenced log with its permissions intact, so per-recipient redaction still
   applies to the (now-empty) tombstone. Publish the authoritative revision. No document is removed;
   no sequence gap; resync/snapshot are unaffected.

`MessageSystem` gains two fields (both `Option`, `#[serde(default, skip_serializing_if =
"Option::is_none")]` so existing stored messages deserialize unchanged):

```rust
pub edited_at:  Option<i64>,   // set by EditMessage; M11d renders "(edited)"
pub deleted_at: Option<i64>,   // set by DeleteMessage; M11d renders the tombstone
```

## 6. The coupled authz seam (highest risk — review as a unit)

c-1 protects message integrity with two coupled chokepoints (see `shadowcat-codebase-chat`): the
**ingress guard** (`ops_target_message` rejects client `Create`/`Delete` Intents at both the WS
`Intent` arm and HTTP `write_ops`) and the **blanket `Update` rejection** in `apply_intent`
(`sqlite.rs`, keyed on the stored `message` doc_type, rejecting every client `Update` to a message
doc regardless of the requester's `DocRole`).

`EditMessage`/`DeleteMessage` must publish an authoritative `Update` to a message doc — which the
blanket rejection would otherwise block. The rule:

> **The edit/delete path re-opens the message-`Update` path ONLY for the server-authored revision,
> distinguished from a client `Update` Intent by a server-internal authoritative marker that no
> wire frame can set. The client-facing blanket `Update` rejection remains fully intact for any
> `Update` lacking the marker.**

`apply_intent` cannot distinguish the server's own edit `Update` from a forged client `Update` by
`ctx` alone (both may arrive as `(WorldRole::Player, owns-doc)`), exactly as c-1's create-exemption
relies on the ingress guard having already rejected client `Create` Intents. The marker is the edit
path's analogue: `handle_edit_message`/`handle_delete_message` thread an internal
"authoritative-message-revision" signal through `Room::publish` into `apply_intent`; the exemption
to the blanket `Update` rejection fires **only** when that signal is present (after the ordinary
`WRITE_FIELDS` floor). Client `Intent` `Update`s never carry it, so the blanket rejection is
undiminished for them.

**This create-exemption / ingress-guard / Update-rejection / edit-marker set is one coupled
authorization surface. Weakening any one alone reopens forgery** (e.g. a marker settable from a wire
field would let a client forge an edit that changes `kind` or bypasses the sanitizer). It is
reviewed as a unit: **mandatory buddy-check + two blind security reviews**, verifying that no client
can author, edit, delete, re-target, or forge (`kind`/`user_owner`/`channel`/`audience`/`content`)
a message via any transport or op.

## 7. File-level change map

| File | Change |
|---|---|
| `src/server/Cargo.toml` | add `ammonia`, `pulldown-cmark` (pinned; features minimal) |
| `src/server/src/chat/sanitize.rs` | **new** — `(raw, policy) -> Vec<Segment>`; ammonia config, markdown render, scheme/content-type checks, CSS-strip |
| `src/server/src/chat/commands.rs` | **new** — `parse_command(raw) -> ParsedCommand`; `/me`·`/roll`·`/w` grammar |
| `src/server/src/chat/settings.rs` | **new** (or fold into `mod.rs`) — `ChatContentPolicy` + fail-closed resolver over the `chat-settings` doc |
| `src/server/src/chat/mod.rs` | `Segment` variants (Html/Link/Image); `MessageSystem.edited_at`/`deleted_at`; `kind` param into `build_message_doc`; `handle_send_message` uses parse→sanitize; **new** `handle_edit_message` / `handle_delete_message` |
| `src/server/src/ws/protocol.rs` | `ClientMsg::EditMessage` / `DeleteMessage` (ts-rs) |
| `src/server/src/ws/conn.rs` | dispatch arms for the two new frames |
| `src/server/src/data/sqlite.rs` | the `apply_intent` `Update`-rejection gains the authoritative-marker exemption (§6) |
| `src/server/src/data/repository.rs` | `member_id_by_username` (or reuse the member-listing query) for `/w` resolution |

## 8. Testing strategy

Per parent §4 (c-3) plus the edit/delete additions:

- **Sanitizer:** an XSS-payload corpus is neutralized (script tags, event handlers, `javascript:`
  URLs, `data:` HTML, SVG/CSS vectors); **embedded CSS is always stripped** regardless of policy;
  each GM toggle (Markdown / HTML / Images / Hyperlinks / Emails) independently on/off honored;
  scheme and image content-type rejection.
- **`chat-settings` fail-closed:** absent doc ⇒ plain text; malformed body ⇒ plain text; toggles
  only widen from the safe baseline.
- **Command parser:** each command → correct `kind`/`body`; `/roll` stores the verbatim expression
  unparsed; `/w @user` resolves usernames, funnels through c-2 validation; unknown username /
  over-cap → whole request rejected; `/w` precedence over the frame audience; no command → Normal.
- **`System` unforgeable:** no client input path yields `kind: System`.
- **Edit path:** owner and GM can edit; a non-owner non-GM is rejected; edit re-sanitizes; edit MAY
  change `kind`; an edit containing `/w` is rejected (audience frozen); `channel`/`actor_owner`
  unchanged; `edited_at` set; per-recipient redaction of the edited content on every egress path
  (broadcast / resync / search).
- **Delete path:** owner and GM can delete; soft tombstone clears content and sets `deleted_at`; the
  doc stays in the log (no sequence gap); a non-recipient still sees nothing.
- **Authz seam (§6):** a client `Intent` `Update`/`Create`/`Delete` to a message doc is still
  rejected on every transport; the authoritative marker cannot be set from any wire frame; no client
  can forge `kind`/`user_owner`/`channel`/`audience`/`content` via edit.

## 9. Out of scope (later checkpoints)

- **Dice execution + `RollResult` embed** — M11d (c-3 only detects `kind: Roll` and stores the raw
  expression).
- **Link-preview fetcher** (`PreviewCard` segment, SSRF-guarded outbound HTTP) — M11c-4.
- **Client composer / message card** (command affordances, render of the segment model, emote
  reversal, per-viewer name gating, doc-link permission checks) — M11d.
- **`chat-channel` config docs + default seeding** — M11d.
