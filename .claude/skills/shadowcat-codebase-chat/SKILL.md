---
name: shadowcat-codebase-chat
description: "Use when touching Shadowcat's chat core: the message Document model, SendMessage ingest, the ops_target_message ingress guard, the create-gate baseline-message exemption, or the message-Update blanket rejection. Covers src/server/src/chat/. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Chat Core

Orientation for the server-authoritative chat system. **M11c-1 shipped**: messages are ordinary
sequenced `Document`s (`doc_type: "message"`) riding the existing Event/redaction/search
path — **no new transport or index code**. M11c-2 (whisper allowlist), c-3 (sanitizer + command
parser + a validated edit path), and c-4 (link-preview fetcher) build on this later.

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
  - `MessageSystem{channel, user_owner, actor_owner, kind, content}` — the `system` body shape.
  - `build_message_doc(...) -> Document` — constructs the whole `Document`: `owner = Some(user)`,
    `permissions.users[user] = DocRole::Owner`, `permissions.default = DocRole::Observer` (every
    world member can read). The SOLE construction site for a stored message doc.
  - `handle_send_message(room, repo, ctx, rate, channel, content, actor_owner, now, budget_per_min)
    -> Result<Command, SendMessageError>` — validates (empty/`MAX_MESSAGE_CHARS = 4096`/
    per-user-per-minute flood budget via `PingRateLimiter`), calls `build_message_doc`, then
    `room.publish(..., vec![Operation::Create { doc }], ...)`. **The sole message-authoring entry
    point** — nothing else may produce a stored `message` doc.
  - `ops_target_message(ops: &[Operation]) -> bool` — the ingress guard: `true` if any `Create`/
    `Delete` op targets a `message` doc_type. `Operation::Update` is always `false` here (an
    `Update` carries no `doc_type`, only `doc_id` + field changes) — Updates are guarded
    separately, see below.
- `src/server/src/ws/protocol.rs` — `ClientMsg::SendMessage { channel, content, actor_owner:
  Option<ActorOwnerRef> }` (ts-rs exported). The only client-facing way to author a message;
  there is no `intent_id`, so a rejection has nothing to correlate a `Reject` frame to and is
  logged only (no failure frame sent to the requester).
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
  — only `ActorOwnerRef` (on the wire frame) is. The client mirrors the body shape independently
  in Zod later (M11d); a Rust-side shape change here needs a corresponding, manually-kept-in-sync
  client mirror, not a regenerated binding.
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

## Pointers

- Design doc: `docs/superpowers/specs/2026-07-08-m11c-chat-core-design.md` (full M11c scope:
  c-1 message core, c-2 whisper allowlist, c-3 sanitizer/commands/edit, c-4 link previews).
- `shadowcat-codebase-documents-permissions` — the `Document`/`PermissionSet`/redaction/search
  machinery a message rides unmodified.
- `shadowcat-codebase-realtime-sync` — `Room::publish`, WS `Intent`/`SendMessage` dispatch,
  broadcast/resync, and the HTTP `write_ops` mirror guard.
- graphify: `graphify explain "chat"` / `graphify query "how does SendMessage reach a stored
  document"` for the cross-file call graph.
- M11 milestone context (dice + chat, parallel to the M10 movement/vision track): memory
  `m11-dice-chat-resume` in the project's auto-memory.
