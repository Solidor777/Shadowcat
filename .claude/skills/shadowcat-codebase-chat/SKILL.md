---
name: shadowcat-codebase-chat
description: "Use when touching Shadowcat's chat core: the message Document model, SendMessage ingest, the ops_target_message ingress guard, the create-gate baseline-message exemption, or the message-Update blanket rejection. Covers src/server/src/chat/. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Chat Core

Orientation for the server-authoritative chat system. **M11c-1 shipped**: messages are ordinary
sequenced `Document`s (`doc_type: "message"`) riding the existing Event/redaction/search
path — **no new transport or index code**. **M11c-2 shipped** (restricted-audience messaging —
whisper allowlist + a GM-only channel): a message's readership is now driven by an `Audience`
enum mapped onto the generic `PermissionSet`/`gm_role` mechanism, still with zero
message-specific redaction/search/broadcast code. c-3 (sanitizer + command parser + a validated
edit path) and c-4 (link-preview fetcher) build on this later.

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
  - `MessageKind` (`Normal` default, `Emote`/`Roll`/`System` reserved for c-3+) and `Segment`
    (`Text{text}` only in c-1, tagged enum, extensible) — both serde-only, NO ts-rs; they live
    inside the opaque `system` JSON body, not the wire frame, so the client declares its own Zod
    mirror later (M11d).
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
  - `MessageSystem{channel, user_owner, actor_owner, kind, audience, content}` — the `system` body
    shape; `audience` rides the opaque body verbatim, same treatment as `kind`/`actor_owner`.
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
    4096`/`MAX_CHANNEL_CHARS = 128`/per-user-per-minute flood budget via `PingRateLimiter`),
    then for `Audience::Whisper` first rejects an oversized `recipients` list
    (`MAX_WHISPER_RECIPIENTS = 128`, `SendMessageError::TooLong`) BEFORE any DB query, then
    fail-closed-validates EVERY remaining recipient uuid via
    `Repository::member_role(world_id, r).await?.is_some()` — an unknown/foreign recipient rejects
    the WHOLE send (`SendMessageError::UnknownRecipient`, nothing persisted) BEFORE
    `build_message_doc` is ever called. Only after all validation passes does it call
    `build_message_doc`, then `room.publish(..., vec![Operation::Create { doc }], ...)`. **The sole
    message-authoring entry point** — nothing else may produce a stored `message` doc. Posting
    rights are unchanged from c-1 (any world member may `SendMessage`); `audience` restricts only
    *readers*, never senders.
  - `ops_target_message(ops: &[Operation]) -> bool` — the ingress guard: `true` if any `Create`/
    `Delete` op targets a `message` doc_type. `Operation::Update` is always `false` here (an
    `Update` carries no `doc_type`, only `doc_id` + field changes) — Updates are guarded
    separately, see below.
- `src/server/src/ws/protocol.rs` — `ClientMsg::SendMessage { channel, content, actor_owner:
  Option<ActorOwnerRef>, audience: Audience }` (ts-rs exported; `audience` is `#[serde(default)]`,
  so an omitted field parses as `Audience::Public`). The only client-facing way to author a
  message; there is no `intent_id`, so a rejection has nothing to correlate a `Reject` frame to
  and is logged only (no failure frame sent to the requester).
- `src/server/src/ws/conn.rs` — two dispatch points:
  - `ClientMsg::Intent { ops, .. }` arm: calls `chat::ops_target_message(&ops)` BEFORE
    `room.publish`; if true, sends `ServerMsg::Reject{reason: Forbidden}` and `continue`s without
    ever reaching `apply_intent`.
  - `ClientMsg::SendMessage { .. }` arm: calls `chat::handle_send_message`; success is confirmed
    only by the broadcast echo of the authored `Event` (same pattern as `Intent`), not a direct
    reply.
- `src/server/src/http/routes.rs` (`write_ops`, around line 242) — mirrors the WS ingress guard:
  `if chat::ops_target_message(&ops) { return Err(AppError::Forbidden); }` before the room/repo
  write path. Both transports must independently apply this guard.
- `src/server/src/data/sqlite.rs` (`apply_intent`) — two coupled chokepoints:
  - **Create-gate exemption** (~line 948): `is_baseline_message = doc.doc_type ==
    MESSAGE_DOC_TYPE && ctx.world_role == WorldRole::Player && doc.owner == Some(ctx.user_id)` —
    lets a Player create a `message` doc even though `core:create` is otherwise GM-only by world
    default. Still passes through the ordinary WRITE_FIELDS floor.
  - **Update blanket rejection** (~line 1019): `if cur.doc_type == MESSAGE_DOC_TYPE { return
    Err(DataError::Forbidden); }` — rejects EVERY client `Update` against a stored `message` doc,
    keyed on the STORED (authoritative) `doc_type`, unconditionally (even the owning Player's own
    message, even though their `DocRole::Owner` would otherwise satisfy WRITE_FIELDS). c-3
    replaces this blanket rejection with a validated, sanitizing edit path.

## Hard invariants

- **`SendMessage` is the SOLE message-authoring path.** A stored `message` doc can only be
  produced by `chat::handle_send_message` → `chat::build_message_doc` → `Room::publish`. No other
  code path may construct or persist one.
- **The create-gate exemption and the ingress guard are a COUPLED pair — weakening either one
  alone reopens forgery.** The exemption in `apply_intent` (sqlite.rs) that lets a Player create a
  self-owned `message` via the generic document path is sound ONLY because `ops_target_message`
  rejects a client-authored `message` Create/Delete at BOTH the WS `Intent` and HTTP `write_ops`
  boundaries first. If the ingress guard were ever removed or narrowed, a Player could `Intent`
  a raw `message` Create with an arbitrary/forged `actor_owner` or `kind`, bypassing
  `handle_send_message`'s validation/flood-limit entirely. Do not touch one without re-verifying
  the other.
- **`apply_intent`'s Update branch blanket-rejects ALL client Updates to a stored `message` doc**,
  regardless of the requester's own `DocRole` on that doc — an owning Player's `Owner` role would
  otherwise satisfy the ordinary WRITE_FIELDS check and let them forge `kind`/`user_owner`/
  `channel`/`content` post-hoc. This is classified against the STORED doc_type (Updates carry no
  doc_type of their own), so `ops_target_message` cannot and does not cover this case — it is a
  second, independent chokepoint, not redundant with the ingress guard.
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

- **A `SendMessage` frame carries no `intent_id`.** Unlike `Intent`, a `handle_send_message`
  failure has no request to correlate a `Reject` to — it is logged server-side only, not
  surfaced to the sending client as a distinct failure frame. A future UX pass may need to add
  correlation (currently out of scope).
- **c-1's content model is intentionally minimal.** `Segment` has only `Text`; no sanitization
  beyond storing raw text verbatim (safe today only because the client renders it as a DOM text
  node, never `innerHTML`). c-3 introduces an actual sanitizer/command parser — do not assume
  `content` is safe to render as markup before that lands.
- **The Update blanket-rejection is a placeholder, not the final edit model.** c-3 is expected to
  replace it with a validated, sanitizing edit path scoped to the message's own owner — until
  then, a message can never be edited or corrected by anyone once posted (including its author).
- **`MAX_MESSAGE_CHARS = 4096` and the per-minute flood budget are enforced only inside
  `handle_send_message`** — they do not apply to any other document-write path (there isn't one
  for messages, per the invariants above, but this is a chat-specific limit, not a general
  `Document` size cap).
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
