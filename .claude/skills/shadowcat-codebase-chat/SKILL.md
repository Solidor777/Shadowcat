---
name: shadowcat-codebase-chat
description: "Use when touching Shadowcat's chat core: the message Document model, SendMessage/EditMessage/DeleteMessage ingest, the ops_target_message ingress guard, the create-gate baseline-message exemption, the WriteOrigin-gated Update exemption, the content sanitizer (chat/sanitize.rs), the chat-settings content policy (chat/settings.rs), or the command parser (chat/commands.rs). Covers src/server/src/chat/. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Chat Core

Orientation for the server-authoritative chat system. **M11c-1 shipped**: messages are ordinary
sequenced `Document`s (`doc_type: "message"`) riding the existing Event/redaction/search
path — **no new transport or index code**. **M11c-2 shipped** (restricted-audience messaging —
whisper allowlist + a GM-only channel): a message's readership is now driven by an `Audience`
enum mapped onto the generic `PermissionSet`/`gm_role` mechanism, still with zero
message-specific redaction/search/broadcast code. **M11c-3 shipped** (sanitizer + command parser
+ a validated, sanitizing edit/delete path): a content-sanitization boundary (`chat::sanitize`,
`ammonia` + `pulldown-cmark`) replaces c-1's raw-text-only model; a leading-command parser
(`chat::parse_command`) derives `MessageKind`/a content-level `/w` whisper target; `EditMessage`/
`DeleteMessage` replace c-1's blanket Update rejection with a real, authorized, sanitizing edit
path and a soft-tombstone delete path, gated by a new `WriteOrigin` marker. c-4 (link-preview
fetcher) is the only remaining M11c checkpoint.

## Purpose

A chat message is a plain `Document` scoped to a world, authored ONLY by the server. The client
never constructs a stored message doc — it sends a `SendMessage` request frame; the server
validates, builds the `Document`, and publishes it through the normal authoritative
`Room::publish` path used by every other document write. Because a message is just a `Document`,
it automatically inherits per-recipient redaction, sequencing/resync, and the FTS5 search index
with zero message-specific plumbing in any of those subsystems.

## Key files & seams

- `src/server/src/chat/mod.rs` — the domain home:
  - `MESSAGE_DOC_TYPE = "message"`.
  - `ActorOwnerRef` (`Actor{actor_id}` | `TokenInstance{token_id}`) — the ONLY chat type with
    `#[derive(TS)]` (ts-rs export); carried on the `SendMessage` wire frame.
  - `MessageKind` (`Normal` default, `Emote`/`Roll` producible by `parse_command`; `System`
    reserved for future server-authored notices — NO parse path can ever produce it, proven by an
    exhaustive test) and `Segment` (tagged enum: `Text{text}` — verbatim, client renders as a DOM
    text node; `Html{sanitized_html}` — a run of already-`ammonia`-cleaned HTML, produced ONLY by
    `sanitize::sanitize`, client renders via `innerHTML`) — both serde-only, NO ts-rs; they live
    inside the opaque `system` JSON body, not the wire frame, so the client declares its own Zod
    mirror later (M11d). **Design note:** inline formatting/links/images do NOT get their own
    typed `Segment` variants — they stay INSIDE a `Segment::Html` run as ordinary sanitized markup
    (`<strong>`/`<a>`/`<img>`). A separate typed `Link`/`Image` segment would require re-parsing
    already-sanitized HTML to extract them, duplicating work `ammonia` already did; `Html` is the
    single content-bearing rich variant.
  - `plain_text_content(raw) -> Vec<Segment>` — the c-1 producer, wraps raw input verbatim as one
    `Segment::Text` (no sanitization yet; the client renders it as a text node, never
    `innerHTML`, so embedded markup is inert).
  - `Audience` (`Public`/`Whisper{recipients: Vec<Uuid>}`/`GmOnly`, `#[default] Public`, tagged
    enum, ts-rs exported same as `ActorOwnerRef`) — the intended readership of a message, carried
    on the `SendMessage` frame and stored verbatim in `MessageSystem`. This is the ONLY
    server-enforced visibility concept for chat; `channel` is a purely client-chosen label with
    ZERO server-enforced meaning — the server never validates or branches on it. A client module
    choosing to post to a "GM" channel is what sets `audience: GmOnly`; the server has no concept
    of a reserved channel name.
  - `MessageSystem{channel, user_owner, actor_owner, kind, audience, content, edited_at,
    deleted_at}` — the `system` body shape; `audience` rides the opaque body verbatim, same
    treatment as `kind`/`actor_owner`. `edited_at`/`deleted_at` (both `Option<i64>`, `#[serde(
    skip_serializing_if = "Option::is_none")]`) are the c-3 edit/delete markers — absent (not
    `null`) on an unedited/live message, so a stored c-1 message round-trips unchanged.
  - `build_message_doc(...) -> Document` — constructs the whole `Document`: `owner = Some(user)`;
    `audience` maps onto `PermissionSet{default, gm_role, users}` (see
    `shadowcat-codebase-documents-permissions` for what `gm_role` does at `resolve_access` time):

    | `Audience` | `default` | `gm_role` | `users` |
    |---|---|---|---|
    | `Public` | `Observer` | `None` | `{owner: Owner}` — c-1's original, unrestricted shape |
    | `Whisper{recipients}` | `None` | `Some(DocRole::None)` | `{owner: Owner, ...recipients: Observer}` |
    | `GmOnly` | `None` | `Some(DocRole::Observer)` | `{owner: Owner}` only |

    `owner` is inserted into `users` LAST in every branch, so a `Whisper` that redundantly names
    the sender as their own recipient can never downgrade them from `Owner` to `Observer` via
    map-insertion order. A `GmOnly` message names no GM in `users` at all — `gm_role =
    Some(Observer)` grants it to ANY current `WorldRole::Gm`, re-resolved on every `resolve_access`
    call (every broadcast recipient, every search hit, every page load), so GM-channel visibility
    tracks promotion/demotion dynamically rather than a frozen roster at send time. The SOLE
    construction site for a stored message doc.
  - `handle_send_message(room, repo, ctx, rate, channel, content, actor_owner, audience, now,
    budget_per_min) -> Result<Command, SendMessageError>` — validates (empty/`MAX_MESSAGE_CHARS =
    4096`/`MAX_CHANNEL_CHARS = 128`/per-user-per-minute flood budget via `PingRateLimiter`), then
    runs `parse_command(&content)` (c-3). If the parsed command carries `whisper_to` (a content-
    level `/w @user...`), its RAW name list is cap-checked against `MAX_WHISPER_RECIPIENTS = 128`
    BEFORE any username is resolved (resolving first would run one sequential
    `member_id_by_username` DB round-trip per `@name` ahead of the cap — the exact resource-
    amplification `MAX_WHISPER_RECIPIENTS` exists to prevent); resolved names build the
    EFFECTIVE `Audience::Whisper` — **content `/w` wins over the c-2 wire frame's `audience`
    field.** The effective audience is then re-validated (cap + `Repository::
    member_role(world_id, r).await?.is_some()` per recipient, fail-closed,
    `SendMessageError::UnknownRecipient`, nothing persisted) through the SAME chokepoint
    regardless of which front-door (frame field or content `/w`) produced it. A post-parse empty
    body (e.g. `/w @alice` with no trailing text) is rejected the same as raw-empty content. Only
    then does it resolve the world's `chat-settings` policy (`resolve_content_policy`), call
    `sanitize(&parsed.body, &policy)` to produce `content_segments`, then `build_message_doc`, then
    `room.publish(..., vec![Operation::Create { doc }], ..., WriteOrigin::Client)`. **The sole
    message-authoring entry point** — nothing else may produce a stored `message` doc. Posting
    rights are unchanged from c-1 (any world member may `SendMessage`); `audience` restricts only
    *readers*, never senders.
  - `ops_target_message(ops: &[Operation]) -> bool` — the ingress guard: `true` if any `Create`/
    `Delete` op targets a `message` doc_type. `Operation::Update` is always `false` here (an
    `Update` carries no `doc_type`, only `doc_id` + field changes) — Updates are guarded
    separately, see below.
  - `handle_edit_message(room, repo, ctx, rate, message_id, content, now, budget_per_min) ->
    Result<Command, SendMessageError>` — owner-or-GM authorized (`cur.owner == Some(ctx.user_id)
    || ctx.world_role == WorldRole::Gm`); rejects editing an already-tombstoned message (reuses
    `NotFound` — an edit must not resurrect cleared `content` on a soft-deleted doc); re-runs
    `parse_command` + `sanitize` on the new content (so `kind` MAY change, e.g. a plain message
    edited into `/me`); a `/w` in the edit content is rejected as `SendMessageError::
    AudienceLocked` rather than silently retargeting readership — `channel`/`user_owner`/
    `actor_owner`/`audience`/`deleted_at` are always copied verbatim from the STORED doc, never
    re-derived from the request. Publishes a single `Operation::Update` on `/system` under
    `WriteOrigin::ServerMessageRevision`. Rate-limited like `handle_send_message`.
  - `handle_delete_message(room, repo, ctx, rate, message_id, now, budget_per_min) ->
    Result<Command, SendMessageError>` — same owner-or-GM authorization; a pure SOFT tombstone (no
    command parsing/sanitization runs): clears `content` to `[]` and sets `deleted_at`, leaving
    `channel`/`user_owner`/`actor_owner`/`audience`/`kind`/`edited_at` untouched. Publishes an
    `Operation::Update` on `/system` (NOT a hard `Operation::Delete`) under `WriteOrigin::
    ServerMessageRevision` — the doc stays in the sequenced log at its original seq, so resync and
    per-recipient redaction continue to apply unmodified. Rate-limited (without a budget, a single
    owner/GM could repeatedly re-delete the same message, each call consuming a real seq number
    and re-writing the FTS index — an unbounded write/broadcast amplification from one frame).
- `src/server/src/chat/sanitize.rs` — `sanitize(raw: &str, policy: &ChatContentPolicy) ->
  Vec<Segment>`, the c-3 content-security boundary. `!policy.markdown && !policy.html` short-
  circuits to a single `Segment::Text` (identical to c-1's `plain_text_content`, the fail-closed
  baseline). Otherwise: `pulldown-cmark` renders Markdown to an HTML string (when `markdown` is
  on; when `html` is off, cmark's raw-HTML events are DOWNGRADED to escaped `Text` events rather
  than dropped, so an author's embedded tag becomes inert display text, e.g. `<b>` → `&lt;b&gt;`,
  never silently vanishing and never reaching `ammonia` as live markup) or is passed straight
  through (html-only), then the WHOLE string crosses `ammonia::Builder::clean()` exactly once —
  the single security boundary — producing one `Segment::Html`. `ammonia_for(policy)` narrows
  ammonia's already-safe default (which already strips `<script>`/`<style>`/the `style`
  attribute/`javascript:`/`data:` schemes) further per toggle: `images: false` removes `<img>`
  entirely; `images: true` adds a lexical `src` extension allowlist (`.png/.jpg/.jpeg/.webp/.gif`,
  checked after stripping `?query`/`#fragment` but NOT scheme/host — a filename-suffix heuristic,
  not real content-type verification; a genuine external host with an allowlisted-looking
  extension still passes, tracked as follow-up); `hyperlinks: false` removes `<a>`; `emails`
  gates whether `mailto:` is in the allowed URL scheme set (always `http`/`https`). CSS is
  ALWAYS stripped regardless of any toggle (belt-and-suspenders re-removal of `style` on every
  currently-whitelisted tag, not just reliance on ammonia's default). **`url_relative(Deny)` is
  load-bearing**: ammonia's own default (`PassThrough`) lets a schemeless, protocol-relative URL
  (`//evil.example/pixel.gif`) through unfiltered — invisible to the `url_schemes` allowlist,
  which only inspects URLs that HAVE a scheme — and would otherwise let a smuggled tracking pixel
  fire for every recipient of a whispered/GM-only message.
- `src/server/src/chat/settings.rs` — `ChatContentPolicy{markdown, html, images, hyperlinks,
  emails: bool}`, all `#[serde(default)]` = `false`, stored as the `system` body of the single
  per-world `chat-settings` config `Document` (`CHAT_SETTINGS_DOC_TYPE`). `resolve_content_policy
  (repo, world_id) -> ChatContentPolicy` is FAIL-CLOSED on every failure mode: a query error, an
  absent doc, or a `system` body that fails `serde_json::from_value` all yield
  `ChatContentPolicy::default()` (every toggle off, plain text) — never a partial/best-effort
  parse that could widen enrichment on malformed input. Every toggle can only WIDEN from that
  safe baseline, so degrading to `default()` is always the safe direction.
- `src/server/src/chat/commands.rs` — `parse_command(raw: &str) -> ParsedCommand{kind,
  whisper_to: Option<Vec<String>>, body}`, pure (no repo/async — the async caller resolves
  `whisper_to` usernames and re-validates). Only a LEADING token counts; the same text mid-message
  is literal. `/me `/`/em `/`/emote ` → `MessageKind::Emote`. `/roll `/`/r `, or bare `/NdM`
  shorthand (optionally `+K`/`-K`) → `MessageKind::Roll`, body stored VERBATIM/unexecuted (a
  future checkpoint runs it). `/w @user @user... rest` → `MessageKind::Normal` +
  `whisper_to: Some(raw_usernames)` — this is chat's SECOND `/w` front-door, independent of the
  c-2 `SendMessage` wire frame's `audience` field; `handle_send_message` reconciles the two,
  content taking precedence (see below). **`kind` can never be `MessageKind::System` from any
  parse path** — proven by an exhaustive test over every command token, not just the default
  fallthrough — `System` is reserved for a future server-authored-notice producer that does not
  go through this parser at all.
- `src/server/src/ws/protocol.rs` — `ClientMsg::SendMessage { channel, content, actor_owner:
  Option<ActorOwnerRef>, audience: Audience }` (ts-rs exported; `audience` is `#[serde(default)]`,
  so an omitted field parses as `Audience::Public`). `ClientMsg::EditMessage { message_id, content
  }` and `ClientMsg::DeleteMessage { message_id }` (both ts-rs exported, c-3) are the ONLY
  client-facing ways to mutate an existing stored message. None of the three carries an
  `intent_id`, so a rejection has nothing to correlate a `Reject` frame to and is logged only (no
  failure frame sent to the requester).
- `src/server/src/ws/conn.rs` — three chat dispatch points plus the `Intent` guard:
  - `ClientMsg::Intent { ops, .. }` arm: calls `chat::ops_target_message(&ops)` BEFORE
    `room.publish`; if true, sends `ServerMsg::Reject{reason: Forbidden}` and `continue`s without
    ever reaching `apply_intent`.
  - `ClientMsg::SendMessage { .. }` arm: calls `chat::handle_send_message`.
  - `ClientMsg::EditMessage { .. }` arm: calls `chat::handle_edit_message`.
  - `ClientMsg::DeleteMessage { .. }` arm: calls `chat::handle_delete_message`.
  - All three chat arms confirm success only by the broadcast echo of the authored `Event` (same
    pattern as `Intent`), not a direct reply; a failure is `tracing::debug!`-logged only.
- `src/server/src/http/routes.rs` (`write_ops`, around line 242) — mirrors the WS ingress guard:
  `if chat::ops_target_message(&ops) { return Err(AppError::Forbidden); }` before the room/repo
  write path. Both transports must independently apply this guard. (`EditMessage`/`DeleteMessage`
  have no HTTP equivalent — they are WS-only frames, same as `SendMessage`.)
- `src/server/src/data/sqlite.rs` (`apply_intent`) — takes a `WriteOrigin` (`Client` |
  `ServerMessageRevision`, `src/server/src/data/command.rs`) parameter, threaded from
  `Room::publish` through ~60+ call sites (every existing caller passes `WriteOrigin::Client`;
  ONLY `handle_edit_message`/`handle_delete_message` ever construct
  `WriteOrigin::ServerMessageRevision`, and only after their own owner-or-GM check has already
  passed). Now FOUR coupled chokepoints, not three:
  1. **Create-gate exemption** (`is_baseline_message = doc.doc_type == MESSAGE_DOC_TYPE &&
     ctx.world_role == WorldRole::Player && doc.owner == Some(ctx.user_id)`) — lets a Player
     create a `message` doc even though `core:create` is otherwise GM-only by world default.
  2. **Ingress guard** (`ops_target_message`, WS `Intent` + HTTP `write_ops`) — rejects any
     client-authored `message` Create/Delete before it ever reaches chokepoint 1.
  3. **Update blanket rejection**, now CONDITIONAL: `if cur.doc_type == MESSAGE_DOC_TYPE &&
     origin != WriteOrigin::ServerMessageRevision { return Err(DataError::Forbidden); }` — still
     rejects every ordinary client `Update` against a stored `message` doc (an owning Player's
     `DocRole::Owner` would otherwise satisfy WRITE_FIELDS and let them forge fields post-hoc), but
     now EXEMPTS the one write shape `handle_edit_message`/`handle_delete_message` produce.
  4. **`WriteOrigin::ServerMessageRevision` access grant**: when `cur.doc_type ==
     MESSAGE_DOC_TYPE && origin == WriteOrigin::ServerMessageRevision`, `apply_intent` does NOT
     call `resolve_access_world` (which would independently re-derive GM write authority from the
     MESSAGE'S OWN `gm_role`/`users` fields — and would incorrectly DENY a non-addressed/
     non-listed GM editing/deleting a `Whisper`/`GmOnly` message, since their capped role there
     has no `WRITE_FIELDS`). Instead it grants a narrowly SCOPED `Access { caps: {READ,
     WRITE_FIELDS}, all: false, ... }`, trusting that the calling handler has ALREADY completed
     its owner-or-GM check. This is proven correct for BOTH edit and delete, across all three
     `Audience` variants, for both the owner and a non-addressed GM. `all: false` (not `all:
     true`) is deliberate — it authorizes writing `/system` only, not `/permissions`/`/embedded`,
     even for this trusted origin. **Caveat:** this scoped grant does NOT auto-satisfy an
     additive `declared_caps_for_path` world/module requirement on a message `/system`
     (sub-)path — no first-party module declares one today (inert), but a future one would
     silently block a GM's already-vetted moderation edit/delete; re-review this chokepoint
     before adding such a requirement.

- **`SendMessage` is the SOLE message-authoring path.** A stored `message` doc can only be
  produced by `chat::handle_send_message` → `chat::build_message_doc` → `Room::publish`. No other
  code path may construct or persist one.
- **The seam is now a FOUR-part coupled surface — weakening any one part alone reopens forgery.**
  (1) create-gate exemption, (2) `ops_target_message` ingress guard (WS `Intent` + HTTP
  `write_ops`), (3) the Update blanket rejection (now conditional on `WriteOrigin`), and (4) the
  `WriteOrigin::ServerMessageRevision` scoped-access grant. (1) is sound only because (2) rejects
  a client-authored `message` Create/Delete before it ever reaches (1). (3) still blocks EVERY
  ordinary client Update, so a Player's own `DocRole::Owner` can never satisfy WRITE_FIELDS on
  their own message directly — the ONLY way through (3) is (4), and (4) is reachable ONLY via
  `handle_edit_message`/`handle_delete_message`, which run their own owner-or-GM check BEFORE
  setting `WriteOrigin::ServerMessageRevision`. `WriteOrigin` is not derivable from any wire frame
  — a client cannot request it, forge it, or otherwise reach (4) directly. Do not touch any one of
  the four without re-verifying the others.
- **GM edit/delete authority is audience-independent by design.** A GM may edit or delete ANY
  message (`Public`/`Whisper`/`GmOnly`) via `handle_edit_message`/`handle_delete_message`'s
  `ctx.world_role == WorldRole::Gm` check, REGARDLESS of whether that GM is individually listed in
  a `Whisper`'s `recipients` or would otherwise have read access to a `GmOnly`/`Whisper` message at
  all (moderation authority, not read authority — the two are deliberately decoupled). This is
  exactly why chokepoint (4) above cannot call `resolve_access_world` on the message's own
  `PermissionSet`: a non-addressed GM's capped role there has no `WRITE_FIELDS`, which would
  incorrectly deny a legitimate moderation edit/delete.
- **`/w` has two independent front-doors, and content wins.** A whisper audience can be set either
  via the c-2 `SendMessage` wire frame's `audience: Audience::Whisper{...}` field, or via a c-3
  content-level `/w @user...` command. `handle_send_message` reconciles both through the exact
  same cap+membership validation chokepoint; when BOTH are present, the parsed content `/w`
  overrides the frame's `audience` argument. An edit can never open either front-door — a `/w` in
  edited content is rejected outright (`AudienceLocked`), not silently applied.
- **An edit re-runs the FULL send pipeline (`parse_command` + `sanitize`) except audience, which is
  frozen.** `kind` MAY change on edit (a plain message can become `/me`), but `channel`/
  `user_owner`/`actor_owner`/`audience`/`deleted_at` are always copied verbatim from the STORED
  document, never re-derived from the edit request. A delete is a pure SOFT tombstone — `content`
  is cleared and `deleted_at` is set via `Operation::Update` on `/system`, NOT a hard
  `Operation::Delete`; the doc stays in the sequenced log at its original seq, so resync and
  per-recipient redaction keep applying to it unmodified. An edit on an already-tombstoned message
  is rejected (`NotFound`) — content can never be resurrected on a soft-deleted doc.
- **Content model is opaque and NOT ts-rs-exported** (`MessageKind`, `Segment`, `MessageSystem`)
  — only `ActorOwnerRef` and `Audience` (both on the wire `SendMessage` frame) are. The client
  mirrors the body shape independently in Zod later (M11d); a Rust-side shape change here needs a
  corresponding, manually-kept-in-sync client mirror, not a regenerated binding. `PermissionSet`
  itself IS a generic, already-mirrored envelope type — its new `gm_role` field is picked up by
  the existing drift guard, not a message-specific mirror.
- **A whisper hides from the GM by default; only `recipients` membership grants a GM access.**
  There is no automatic GM see-all for `Whisper`/`GmOnly` messages — a GM must be individually
  listed in `recipients` (for `Whisper`) or simply hold `WorldRole::Gm` at read time (for
  `GmOnly`, via `gm_role`); a GM not covered by either sees nothing, not even that the doc exists.
  This is a deliberate product decision (§0 of the design doc), not an oversight.
- **Recipient validation happens BEFORE document construction, fail-closed on the whole send.**
  `handle_send_message` checks every `Whisper` recipient against current world membership; a
  single bad uuid rejects the entire message (no partial send, nothing persisted) — do not move
  this check after `build_message_doc` or make it per-recipient-tolerant.
- **Messages ride the existing Event/redaction/search machinery with zero message-specific
  code in those subsystems** — a message's visibility, per-recipient redaction, sequencing, and
  FTS5 search hit are governed entirely by the generic `Document`/`PermissionSet` rules
  (`shadowcat-codebase-documents-permissions`) and the generic room broadcast/resync path
  (`shadowcat-codebase-realtime-sync`). Any change to those subsystems' redaction or indexing
  logic implicitly changes chat behavior too — there is no separate chat-specific override to
  audit, but also no chat-specific safety net.

## Gotchas

- **No `SendMessage`/`EditMessage`/`DeleteMessage` frame carries an `intent_id`.** A rejection
  from any of the three has no request to correlate a `Reject` to — it is logged server-side only,
  not surfaced to the sending client as a distinct failure frame. A future UX pass may need to add
  correlation (currently out of scope).
- **`Segment` now has `Html` alongside `Text`.** A pre-c-3 assumption that `content` is always
  literal, inert text is no longer valid — `sanitize()` produces `Segment::Html{sanitized_html}`
  whenever the world's `chat-settings` policy has `markdown` or `html` enabled, and the client is
  expected to render that variant via `innerHTML` (it is safe by construction ONLY because it
  passed through `ammonia`; never innerHTML-render a `Text` segment or a `Html` segment your code
  produced by any path other than `chat::sanitize`).
- **The Update blanket-rejection is no longer absolute — it is conditional on `WriteOrigin`.** Any
  code that reasons about `apply_intent`'s message-Update behavior must account for the
  `WriteOrigin::ServerMessageRevision` exemption (see Hard Invariants); treating the rejection as
  unconditional will misdiagnose why an edit/delete succeeds.
- **`chat-settings` fail-closed means a missing or malformed policy doc silently degrades to plain
  text**, not an error surfaced anywhere — a GM who intends to enable Markdown but leaves the
  `chat-settings` doc absent, or types a field with the wrong JSON type, gets ordinary c-1-style
  plain text with no diagnostic. This is deliberate (see `settings.rs`'s module doc) but easy to
  mistake for a bug when testing enrichment toggles.
- **`MAX_MESSAGE_CHARS = 4096` and the per-minute flood budget are enforced only inside
  `handle_send_message`/`handle_edit_message`/`handle_delete_message`** — they do not apply to any
  other document-write path (there isn't one for messages, per the invariants above, but this is a
  chat-specific limit, not a general `Document` size cap).
- **`Audience::Whisper.recipients` is capped at `MAX_WHISPER_RECIPIENTS = 128`**, checked in
  `handle_send_message` BEFORE the per-recipient `member_role` validation loop — an oversized list
  is rejected (`SendMessageError::TooLong`) without running any of those DB round-trips. Without
  this, one cheap `SendMessage` frame could force one sequential DB query per (attacker-supplied)
  recipient.
- **A message's sender always retains `DocRole::Owner` in `permissions.users`**, regardless of the
  message's `Audience` or any later `gm_role`/world-role change — e.g. a Player who posts to a
  `GmOnly` channel permanently keeps read/search access to their own message even if never
  promoted to GM. Anyone building an edit/delete path on top of this (c-3) must not assume `Owner`
  implies "currently privileged"; it means "originally authored."

## Pointers

- Design doc: `docs/superpowers/specs/2026-07-08-m11c-chat-core-design.md` (full M11c scope:
  c-1 message core, c-2 whisper allowlist, c-3 sanitizer/commands/edit, c-4 link previews).
- c-2 design doc: `docs/superpowers/specs/2026-07-08-m11c-2-whisper-allowlist-design.md` — the
  `Audience`→`PermissionSet` mapping table, the GM-only-channel scope addition, and the full
  testing strategy (per-egress-path proof, promotion/demotion dynamism, malformed-recipient
  fail-closed case).
- c-3 design doc: `docs/superpowers/specs/2026-07-09-m11c-3-sanitizer-commands-edit-design.md` —
  the sanitizer's ammonia/pulldown-cmark design, the `chat-settings` fail-closed policy model, the
  command-parser grammar, and the edit/delete authz-seam design (the `WriteOrigin` mechanism and
  its coupling to the pre-existing create-exemption/ingress-guard pair). New production deps:
  `ammonia`, `pulldown-cmark`.
- `shadowcat-codebase-documents-permissions` — the `Document`/`PermissionSet`/redaction/search
  machinery a message rides, including the `gm_role` field this checkpoint added (owned there,
  load-bearing here — see that skill's Hard Invariants for what `Some(role)` does to
  `resolve_access`'s GM branch).
- `shadowcat-codebase-realtime-sync` — `Room::publish`, WS `Intent`/`SendMessage` dispatch,
  broadcast/resync, and the HTTP `write_ops` mirror guard.
- graphify: `graphify explain "chat"` / `graphify query "how does SendMessage reach a stored
  document"` for the cross-file call graph.
- M11 milestone context (dice + chat, parallel to the M10 movement/vision track): memory
  `m11-dice-chat-resume` in the project's auto-memory.
