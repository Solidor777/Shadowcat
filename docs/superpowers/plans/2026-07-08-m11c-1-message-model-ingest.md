# M11c-1 · Message Model + Server-Authoritative Ingest + Delivery — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish server-authoritative chat ingest — a `SendMessage` intent whose handler makes the *server* construct, persist, and broadcast a `message` document — and prove it rides the existing Event/redaction/search path with no new transport code.

**Architecture:** A new `src/server/src/chat/` module owns the message `system`-body types (`MessageSystem`, `MessageKind`, `Segment`, `ActorOwnerRef`), a trivial plain-text producer, and the `build_message_doc` constructor. A new `ClientMsg::SendMessage` frame is handled in `conn.rs` by an extracted `handle_send_message` function that flood-limits, builds the doc, and publishes it via the existing `Room::publish` → `apply_intent` path. Server authority is enforced by two coupled guards: an **ingress guard** rejecting any client-authored `message` op (so `SendMessage` is the sole authoring path) and a narrow **create-gate exemption** (Seam B) letting a Player's server-built message create pass the otherwise-GM-only `core:create` gate.

**Tech Stack:** Rust (server), `serde` / `serde_json`, `ts-rs` (bindings emitted on `cargo test`), `tokio`, `axum`/WS, `sqlx`+SQLite. **No new crates** in this checkpoint (`ammonia`/`pulldown-cmark` are c-3; the HTTP client is c-4). Client: Zod mirrors in `src/client/core/src/wire.ts`.

## Global Constraints

- **No new production dependencies.** c-1 adds zero crates. (Copied from design §1: `ammonia`/`pulldown-cmark` land in c-3.)
- **Cross-platform.** Build paths with `std::path`; no OS-specific code. Server must compile/test on ubuntu/macos/windows (CI matrix).
- **ts-rs bindings are CI-checked.** ts-rs writes `.ts` files as a side effect of `cargo test`; CI runs `git diff --exit-code src/types/generated` (Linux). Every task that changes a `#[derive(TS)]` type MUST `cargo test` and commit the regenerated `src/types/generated/*.ts`.
- **Server is structural-only, except message authoring.** The only doc-type-specific server logic this checkpoint adds is the message ingress guard + create-gate exemption; everything else stays doc_type-generic.
- **Message ingest is server-authoritative.** The client never builds the stored message doc; the server does. A `message` doc reaches `apply_intent` ONLY via `SendMessage`.
- **IDs are bare `uuid::Uuid`** (no newtypes) server-side, `string` in TS, `z.string()` in Zod.
- **Commit after every task** once its tests pass (`cargo test` green, `cargo clippy --all-targets -- -D warnings` clean for touched crates).

## Model/Effort directives

- **Plan authored mainline** on Opus 4.8 / effort high (user directive: "You write the plan"), not dispatched to `sdd-plan-writer-*`.
- **Execution:** subagent-driven-development. Implementation via `shadowcat-coder` (sonnet, effort medium); each task reviewed by the `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` two-reviewer gate (effort high). Escalate a blocked task to `shadowcat-coder-opus`; escalate a shallow/uncertain review to the `-opus` reviewer twin.

## Buddy-check directives

- **No pre-authorized buddy-check for c-1** (it is the low-risk plumbing checkpoint; the HIGH-risk units — whisper allowlist, sanitizer, SSRF fetcher — are c-2/c-3/c-4).
- **Watch the authz seam (Tasks 5 + 6).** The create-gate exemption and the ingress guard are *coupled*: the exemption is only safe because the ingress guard makes `SendMessage` the sole path a `message` create reaches `apply_intent`. The two-reviewer gate on Tasks 5 and 6 must explicitly confirm both halves exist and that no client path (WS `Intent`, HTTP `write_ops`) can author a `message`. **Escalate to a full `buddy-checking` run only if that review reads as shallow or uncertain.**

## Flagged decision (surface to the user at spec-review)

**Seam B, Player-baseline posting.** Posting chat is treated as a hardwired baseline right for `WorldRole::Player` (and GM), implemented by exempting the `message` doc_type from the GM-only `core:create` role gate while still requiring the write-floor and recording the real user as `owner`. Spectators remain gated (a GM may grant them via the existing `WorldCapDefaults`). Rationale: "chat always works" without per-world GM config, and it works for already-created worlds (no seed/migration). Alternative (Seam A: seed a GM-revocable `core:create` grant at world creation) was rejected as fragile for existing worlds. Revisit if Spectators should post by default, or if Player posting should be GM-revocable.

---

## File Structure

**Created:**
- `src/server/src/chat/mod.rs` — chat domain: `MESSAGE_DOC_TYPE`, `MessageKind`, `Segment`, `ActorOwnerRef` (ts-rs), `MessageSystem`, `plain_text_content`, `build_message_doc`, `ops_target_message`, `handle_send_message`, `SendMessageError`.
- `src/types/generated/ActorOwnerRef.ts` — ts-rs output (generated, committed).
- `.claude/skills/shadowcat-codebase-chat/SKILL.md` — new codebase skill (git-ignored; created for the reviewed-skill gate).

**Modified:**
- `src/server/src/lib.rs` — add `pub mod chat;`.
- `src/server/src/ws/protocol.rs` — add `ClientMsg::SendMessage` variant.
- `src/server/src/ws/conn.rs` — handle `SendMessage`; add ingress guard to the `Intent` arm.
- `src/server/src/ws/mod.rs` — add `message_rate` to `WsState`.
- `src/server/src/http/routes.rs` — ingress guard on the HTTP `write_ops` path.
- `src/server/src/data/sqlite.rs` — Seam-B create-gate exemption in `apply_intent`.
- `src/types/generated/ClientMsg.ts` — ts-rs output (regenerated, committed).
- `src/client/core/src/wire.ts` — `ActorOwnerRef` + `SendMessage` Zod mirrors.
- `docs/PLAN.md` — M11c-1 status (final task).

---

### Task 1: Chat module scaffold + `ActorOwnerRef` + `MessageKind`

**Files:**
- Create: `src/server/src/chat/mod.rs`
- Modify: `src/server/src/lib.rs` (add `pub mod chat;` to the module list)
- Test: inline `#[cfg(test)]` in `src/server/src/chat/mod.rs`

**Interfaces:**
- Produces: `pub const MESSAGE_DOC_TYPE: &str = "message";`
- Produces: `pub enum ActorOwnerRef { Actor { actor_id: Uuid }, TokenInstance { token_id: Uuid } }` (serde `#[serde(tag = "kind", rename_all = "snake_case")]`, derives `TS`, exported to `../../types/generated/`).
- Produces: `pub enum MessageKind { Normal, Emote, Roll, System }` (serde `rename_all = "snake_case"`, `Default = Normal`; **no** `TS` derive — it rides the opaque body).

- [ ] **Step 1: Write the failing test**

In a new `src/server/src/chat/mod.rs`, add at the bottom:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn actor_owner_ref_tagged_roundtrip() {
        let a = ActorOwnerRef::Actor { actor_id: Uuid::from_u128(1) };
        let j = serde_json::to_value(&a).unwrap();
        assert_eq!(j["kind"], "actor");
        assert_eq!(a, serde_json::from_value(j).unwrap());

        let t = ActorOwnerRef::TokenInstance { token_id: Uuid::from_u128(2) };
        let j = serde_json::to_value(&t).unwrap();
        assert_eq!(j["kind"], "token_instance");
        assert_eq!(t, serde_json::from_value(j).unwrap());
    }

    #[test]
    fn message_kind_defaults_normal_snake_case() {
        assert_eq!(MessageKind::default(), MessageKind::Normal);
        assert_eq!(serde_json::to_value(MessageKind::System).unwrap(), serde_json::json!("system"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat chat::tests`
Expected: FAIL — `cannot find type ActorOwnerRef` / module `chat` not found.

- [ ] **Step 3: Write minimal implementation**

Top of `src/server/src/chat/mod.rs`:
```rust
//! Chat domain: the server-authoritative message model and ingest.
//!
//! Messages are ordinary sequenced `Document`s with an opaque `system` body
//! (this module's `MessageSystem`), authored ONLY by the server from a
//! `SendMessage` intent — never built by a client. INVARIANT: a `message`
//! doc_type reaches `apply_intent` only via `handle_send_message`; the ingress
//! guard (`ops_target_message`) rejects any client-authored message op.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Top-level doc_type for chat messages.
pub const MESSAGE_DOC_TYPE: &str = "message";

/// Attribution of a message to an actor: a linked canonical `Actor` document,
/// or an instanced actor resolved through its token. Carried on the
/// `SendMessage` frame and stored in `MessageSystem`. No ID newtypes exist —
/// identifiers are bare `Uuid` (rendered `string` in TS).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActorOwnerRef {
    Actor { actor_id: Uuid },
    TokenInstance { token_id: Uuid },
}

/// Message subtype, orthogonal to channel. Rides the opaque body (no ts-rs).
/// c-1 only ever produces `Normal`; `Emote`/`Roll` are set by c-3's command
/// parser, `System` by server-authored notices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    #[default]
    Normal,
    Emote,
    Roll,
    System,
}
```

In `src/server/src/lib.rs`, add to the `pub mod` list (alphabetical with the others, e.g. after `pub mod auth;`):
```rust
pub mod chat;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p shadowcat chat::tests`
Expected: PASS (2 tests).

- [ ] **Step 5: Verify ts-rs binding emitted**

Run: `cargo test -p shadowcat` then `git status src/types/generated/`
Expected: a new untracked file `src/types/generated/ActorOwnerRef.ts` containing
`export type ActorOwnerRef = { "kind": "actor", actor_id: string, } | { "kind": "token_instance", token_id: string, };`

- [ ] **Step 6: Commit**

```bash
git add src/server/src/chat/mod.rs src/server/src/lib.rs src/types/generated/ActorOwnerRef.ts
git commit -m "feat(chat/m11c-1): chat module scaffold + ActorOwnerRef + MessageKind"
```

---

### Task 2: `Segment` taxonomy + plain-text producer

**Files:**
- Modify: `src/server/src/chat/mod.rs`
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Consumes: nothing new.
- Produces: `pub enum Segment { Text { text: String } }` (serde `#[serde(tag = "kind", rename_all = "snake_case")]`, serde-only — no `TS`; extensible: c-3 adds `Mark`/`Link`/`Image`/`DocLink`, c-4 adds `PreviewCard`, M11d wires `RollEmbed`).
- Produces: `pub fn plain_text_content(raw: &str) -> Vec<Segment>` — the trivial c-1 producer.

> **Design note (refines design §2.1):** c-1 defines `Segment` with only the `Text` variant it actually produces; each later checkpoint adds the variants it produces. This is YAGNI-correct — speculative link/image/preview fields are c-3/c-4 concerns. Plain-text safety is a *rendering* property (M11d renders `Text` as a DOM text node, never `innerHTML`); the producer does not HTML-escape.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/server/src/chat/mod.rs`:
```rust
#[test]
fn plain_text_produces_single_text_segment() {
    let segs = plain_text_content("hello <b>world</b>");
    assert_eq!(segs, vec![Segment::Text { text: "hello <b>world</b>".into() }]);
    // Producer stores raw text verbatim; markup is inert data, rendered as text (M11d).
    let j = serde_json::to_value(&segs[0]).unwrap();
    assert_eq!(j["kind"], "text");
    assert_eq!(j["text"], "hello <b>world</b>");
}

#[test]
fn plain_text_empty_is_empty_segment() {
    assert_eq!(plain_text_content(""), vec![Segment::Text { text: String::new() }]);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat chat::tests::plain_text`
Expected: FAIL — `cannot find type Segment` / `plain_text_content` not found.

- [ ] **Step 3: Write minimal implementation**

Add to `src/server/src/chat/mod.rs` (after `MessageKind`):
```rust
/// One piece of a message's sanitized content model. Serialized into the
/// message's opaque `system` body (no ts-rs — M11d declares its own Zod mirror).
/// Extensible: later checkpoints add the variants they produce (c-3 marks/links/
/// images, c-4 preview cards, M11d roll embeds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Segment {
    /// Literal text. Rendered as a DOM text node by the client (never innerHTML),
    /// so any markup it contains is inert.
    Text { text: String },
}

/// The c-1 producer: wrap raw input as a single literal-text segment. Rich
/// producers (markdown/HTML) are added in c-3, feeding this same content model.
pub fn plain_text_content(raw: &str) -> Vec<Segment> {
    vec![Segment::Text { text: raw.to_string() }]
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p shadowcat chat::tests`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/chat/mod.rs
git commit -m "feat(chat/m11c-1): Segment content model (Text) + plain-text producer"
```

---

### Task 3: `MessageSystem` + `build_message_doc`

**Files:**
- Modify: `src/server/src/chat/mod.rs`
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Consumes: `Segment`, `MessageKind`, `ActorOwnerRef`, `MESSAGE_DOC_TYPE` (Task 1-2); `Document`, `PermissionSet`, `DocRole`, `Scope` from `crate::data::document`.
- Produces: `pub struct MessageSystem { channel: String, user_owner: Uuid, actor_owner: Option<ActorOwnerRef>, kind: MessageKind, content: Vec<Segment> }` (serde, no `TS`; `recipients` is intentionally absent — added in c-2).
- Produces: `pub fn build_message_doc(world_id: Uuid, user: Uuid, channel: String, actor_owner: Option<ActorOwnerRef>, content: Vec<Segment>, now: i64) -> Document`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module:
```rust
use crate::data::document::{DocRole, Scope};

#[test]
fn build_message_doc_is_server_owned_message() {
    let world = Uuid::from_u128(10);
    let user = Uuid::from_u128(20);
    let doc = build_message_doc(
        world, user, "all".into(), None,
        plain_text_content("hi"), 1234,
    );
    assert_eq!(doc.doc_type, MESSAGE_DOC_TYPE);
    assert_eq!(doc.owner, Some(user));
    assert_eq!(doc.scope, Scope::World { world_id: world });
    assert_eq!(doc.created_at, 1234);
    // Author gets the Owner floor so the create WRITE_FIELDS check passes;
    // default Observer so every world member can read it.
    assert_eq!(doc.permissions.default, DocRole::Observer);
    assert_eq!(doc.permissions.users.get(&user), Some(&DocRole::Owner));
    // Body round-trips back to a MessageSystem with server-set user_owner.
    let sys: MessageSystem = serde_json::from_value(doc.system.clone()).unwrap();
    assert_eq!(sys.user_owner, user);
    assert_eq!(sys.channel, "all");
    assert_eq!(sys.kind, MessageKind::Normal);
    assert_eq!(sys.content, vec![Segment::Text { text: "hi".into() }]);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat chat::tests::build_message_doc`
Expected: FAIL — `build_message_doc`/`MessageSystem` not found.

- [ ] **Step 3: Write minimal implementation**

Add to `src/server/src/chat/mod.rs`:
```rust
use crate::data::document::{DocRole, Document, PermissionSet, Scope};
use std::collections::BTreeMap;

/// The message document's `system` body. Opaque on the wire (no ts-rs); the
/// client declares its own Zod mirror in M11d. `recipients` (whispers) is added
/// in c-2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSystem {
    pub channel: String,
    /// The owning user; server-set to the authenticated poster (== `Document.owner`).
    pub user_owner: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_owner: Option<ActorOwnerRef>,
    pub kind: MessageKind,
    pub content: Vec<Segment>,
}

/// Server-construct a message `Document`. INVARIANT: only the server calls this
/// (via `handle_send_message`); clients never build message docs. Sets the
/// author as `owner` + `Owner` permission (satisfies the create WRITE_FIELDS
/// floor) with `default = Observer` so all world members may read it.
pub fn build_message_doc(
    world_id: Uuid,
    user: Uuid,
    channel: String,
    actor_owner: Option<ActorOwnerRef>,
    content: Vec<Segment>,
    now: i64,
) -> Document {
    let mut users = BTreeMap::new();
    users.insert(user, DocRole::Owner);
    let system = MessageSystem {
        channel,
        user_owner: user,
        actor_owner,
        kind: MessageKind::Normal,
        content,
    };
    Document {
        id: Uuid::new_v4(),
        scope: Scope::World { world_id },
        doc_type: MESSAGE_DOC_TYPE.to_string(),
        schema_version: 1,
        source: None,
        owner: Some(user),
        permissions: PermissionSet {
            default: DocRole::Observer,
            users,
            ..Default::default()
        },
        embedded: BTreeMap::new(),
        parent_id: None,
        system: serde_json::to_value(system).expect("MessageSystem serializes"),
        created_at: now,
        updated_at: now,
    }
}
```
> If the `Document` struct field set differs from the above (e.g. extra fields), read `src/server/src/data/document.rs` and match it exactly — do not trust this snippet over the real struct.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p shadowcat chat::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/chat/mod.rs
git commit -m "feat(chat/m11c-1): MessageSystem body + server build_message_doc"
```

---

### Task 4: `SendMessage` frame + Zod mirror

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (add `ClientMsg::SendMessage`)
- Modify: `src/client/core/src/wire.ts` (add `ActorOwnerRef` + `SendMessage` Zod)
- Modify: `src/types/generated/ClientMsg.ts` (regenerated)
- Test: inline serde test in `protocol.rs`; Zod parse test in `src/client/core/src/wire.test.ts`

**Interfaces:**
- Consumes: `crate::chat::ActorOwnerRef`.
- Produces: `ClientMsg::SendMessage { channel: String, content: String, actor_owner: Option<ActorOwnerRef> }` — serde tag `"send_message"`.

- [ ] **Step 1: Write the failing test (server)**

In `src/server/src/ws/protocol.rs` `#[cfg(test)]` module, add:
```rust
#[test]
fn send_message_frame_parses() {
    let raw = r#"{"type":"send_message","channel":"all","content":"hi","actor_owner":null}"#;
    let msg: ClientMsg = serde_json::from_str(raw).unwrap();
    match msg {
        ClientMsg::SendMessage { channel, content, actor_owner } => {
            assert_eq!(channel, "all");
            assert_eq!(content, "hi");
            assert!(actor_owner.is_none());
        }
        _ => panic!("wrong variant"),
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat ws::protocol`
Expected: FAIL — no `SendMessage` variant.

- [ ] **Step 3: Write minimal implementation**

In `src/server/src/ws/protocol.rs`, add a variant to `ClientMsg` (match the existing `#[serde(tag = "type", rename_all = "snake_case")]` enum), and `use crate::chat::ActorOwnerRef;` at the top:
```rust
    /// Author a chat message. The server sanitizes `content` and CONSTRUCTS the
    /// stored message doc (server-authoritative ingest). The sole message-
    /// authoring path — a client `Create` of a `message` doc is rejected.
    SendMessage {
        channel: String,
        content: String,
        #[serde(default)]
        actor_owner: Option<ActorOwnerRef>,
    },
```

- [ ] **Step 4: Run tests + regenerate bindings**

Run: `cargo test -p shadowcat ws::protocol`
Expected: PASS.
Run: `git status src/types/generated/ClientMsg.ts`
Expected: modified — now includes `{ "type": "send_message", channel: string, content: string, actor_owner: ActorOwnerRef | null, }`.

- [ ] **Step 5: Add the Zod mirror + parity test (client)**

In `src/client/core/src/wire.ts`, add near the other tagged unions:
```ts
export const ActorOwnerRefSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("actor"), actor_id: z.string() }),
  z.object({ kind: z.literal("token_instance"), token_id: z.string() }),
]);
```
and add the `send_message` member to the `ClientMsg` union schema if one exists (search `wire.ts` for `ClientMsg`); if the client does not validate outgoing `ClientMsg`, export a standalone schema instead:
```ts
export const SendMessageSchema = z.object({
  type: z.literal("send_message"),
  channel: z.string(),
  content: z.string(),
  actor_owner: ActorOwnerRefSchema.nullable(),
});
```
In `src/client/core/src/wire.test.ts`, add:
```ts
it("parses a send_message frame + actor_owner ref", () => {
  expect(SendMessageSchema.parse({
    type: "send_message", channel: "all", content: "hi",
    actor_owner: { kind: "actor", actor_id: "00000000-0000-0000-0000-000000000001" },
  }).actor_owner?.kind).toBe("actor");
});
```

- [ ] **Step 6: Run client tests + typecheck**

Run: `pnpm --filter @shadowcat/core test` and `pnpm --filter @shadowcat/core exec tsc --noEmit`
Expected: PASS (vitest strips types — the explicit `tsc --noEmit` is what actually typechecks; see [[vitest-skips-typecheck-in-sdd]]).

- [ ] **Step 7: Commit**

```bash
git add src/server/src/ws/protocol.rs src/types/generated/ClientMsg.ts src/client/core/src/wire.ts src/client/core/src/wire.test.ts
git commit -m "feat(chat/m11c-1): SendMessage ClientMsg frame + ActorOwnerRef Zod mirror"
```

---

### Task 5: Seam-B create-gate exemption (Player baseline message posting)

**Files:**
- Modify: `src/server/src/data/sqlite.rs` (the Create branch of `apply_intent`, ~lines 934-942)
- Test: inline `#[cfg(test)]` in `sqlite.rs`

**Interfaces:**
- Consumes: `crate::chat::MESSAGE_DOC_TYPE`.
- Produces: no new public API; behavior — a `WorldRole::Player` may create a `message` doc; a Player still may NOT create other doc types by default; a `Spectator` may NOT create a `message` doc by default.

> **Read first:** open `src/server/src/data/sqlite.rs` around the Create branch (`apply_intent`, the `core:create` gate near line 934) and match the real code. The exemption is a narrow addition to the existing role-gate condition; do not restructure it.

- [ ] **Step 1: Write the failing test**

In the `sqlite.rs` tests module, add (adapt to the local `repo()`/`tests_doc` helpers):
```rust
#[tokio::test]
async fn player_may_create_message_but_not_other_types() {
    let r = repo().await;
    let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    let player = r.create_user("pl", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();
    let pl_ctx = PermissionContext { user_id: player, world_role: WorldRole::Player };

    // A server-shaped message doc (author owns it) — Player create allowed.
    let msg = crate::chat::build_message_doc(
        w.id, player, "all".into(), None,
        crate::chat::plain_text_content("hi"), 1,
    );
    r.apply_intent(&pl_ctx, w.id, vec![Operation::Create { doc: msg }], 1)
        .await
        .expect("player may post a message");

    // A non-message doc the player owns — still denied (core:create GM-only).
    let mut other = crate::chat::build_message_doc(
        w.id, player, "all".into(), None, vec![], 2,
    );
    other.doc_type = "note".into();
    let err = r.apply_intent(&pl_ctx, w.id, vec![Operation::Create { doc: other }], 2).await;
    assert!(matches!(err, Err(DataError::Forbidden)), "non-message create must stay GM-gated");
}

#[tokio::test]
async fn spectator_may_not_create_message() {
    let r = repo().await;
    let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    let spec = r.create_user("sp", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, spec, WorldRole::Spectator).await.unwrap();
    let sp_ctx = PermissionContext { user_id: spec, world_role: WorldRole::Spectator };
    let msg = crate::chat::build_message_doc(w.id, spec, "all".into(), None, vec![], 1);
    let err = r.apply_intent(&sp_ctx, w.id, vec![Operation::Create { doc: msg }], 1).await;
    assert!(matches!(err, Err(DataError::Forbidden)));
}
```
> Use whatever `PermissionContext`/`WorldRole`/`DataError` imports the surrounding test module already uses.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat player_may_create_message`
Expected: FAIL — `Forbidden` on the player message create (gate is GM-only today).

- [ ] **Step 3: Write minimal implementation**

In the Create branch's `core:create` gate, add the exemption. Change the existing condition (currently roughly):
```rust
if ctx.world_role != WorldRole::Gm
    && !world_defaults.role_has(ctx.world_role, &doc.doc_type, cap::CREATE)
{
    return Err(DataError::Forbidden);
}
```
to:
```rust
// Baseline chat-posting right (design §M11c-1, Seam B): a Player may author a
// `message`, exempt from the otherwise-GM-only core:create gate. The write-floor
// check above still applies (author must own the doc), and `doc.owner` records
// the real poster. Server authority is preserved by the ingress guard
// (chat::ops_target_message), which is the ONLY reason a message reaches here:
// clients cannot author messages; only the SendMessage handler can.
let is_baseline_message =
    doc.doc_type == crate::chat::MESSAGE_DOC_TYPE && ctx.world_role == WorldRole::Player;
if ctx.world_role != WorldRole::Gm
    && !is_baseline_message
    && !world_defaults.role_has(ctx.world_role, &doc.doc_type, cap::CREATE)
{
    return Err(DataError::Forbidden);
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat player_may_create_message spectator_may_not_create_message`
Expected: PASS. Then `cargo test -p shadowcat` (full suite) to confirm no regression in existing capability tests.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/data/sqlite.rs
git commit -m "feat(chat/m11c-1): exempt Player message-create from the GM-only core:create gate (Seam B)"
```

---

### Task 6: Ingress guard — reject client-authored message ops

**Files:**
- Modify: `src/server/src/chat/mod.rs` (add `ops_target_message`)
- Modify: `src/server/src/ws/conn.rs` (guard the `ClientMsg::Intent` arm)
- Modify: `src/server/src/http/routes.rs` (guard the HTTP `write_ops` path)
- Test: inline unit test for `ops_target_message`; a WS or HTTP rejection test

**Interfaces:**
- Consumes: `Operation` from `crate::data::command`, `MESSAGE_DOC_TYPE`.
- Produces: `pub fn ops_target_message(ops: &[Operation]) -> bool` — true if any op Creates, Updates, or Deletes a `message` doc.

> This is the security half of server-authoritative ingest: with this guard in place, a `message` doc can reach `apply_intent` ONLY via `SendMessage` (Task 7). Review Tasks 5+6 together (see Buddy-check directives).

- [ ] **Step 1: Write the failing test (unit)**

Add to `chat/mod.rs` tests:
```rust
use crate::data::command::Operation;

#[test]
fn ops_target_message_detects_message_create_and_update() {
    let msg = build_message_doc(Uuid::from_u128(1), Uuid::from_u128(2), "all".into(), None, vec![], 0);
    assert!(ops_target_message(&[Operation::Create { doc: msg.clone() }]));
    assert!(ops_target_message(&[Operation::Delete { doc: msg }]));

    let mut note = build_message_doc(Uuid::from_u128(1), Uuid::from_u128(2), "all".into(), None, vec![], 0);
    note.doc_type = "note".into();
    assert!(!ops_target_message(&[Operation::Create { doc: note }]));
}
```
> For the `Update` case, `Operation::Update { doc_id, .. }` carries no doc_type — `ops_target_message` cannot classify an Update by doc_type alone. Decision: the guard only needs to block **Create** (and defensively **Delete**, which carries the doc); message **edits** are out of c-1 scope entirely (deferred to the c-3+ sanitizing edit path), so the client Intent arm additionally rejects any `Update`/`Delete` whose target is a stored `message` by loading its doc_type — but for c-1 simplicity, block only `Create { doc.doc_type == message }` and `Delete { doc.doc_type == message }` here, and document that message Updates are rejected at a higher layer once edits exist. Keep the test to Create + Delete as above.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat ops_target_message`
Expected: FAIL — function not found.

- [ ] **Step 3: Write minimal implementation**

Add to `chat/mod.rs`:
```rust
use crate::data::command::Operation;

/// True if any op authors a `message` doc via the generic document path.
/// Clients must NOT author messages (only `handle_send_message` may); the WS
/// `Intent` and HTTP write paths reject ops for which this is true, keeping
/// message ingest server-authoritative.
pub fn ops_target_message(ops: &[Operation]) -> bool {
    ops.iter().any(|op| match op {
        Operation::Create { doc } | Operation::Delete { doc } => doc.doc_type == MESSAGE_DOC_TYPE,
        Operation::Update { .. } => false,
    })
}
```
> Match the real `Operation` variants in `src/server/src/data/command.rs`; adjust the arms if they differ.

In `src/server/src/ws/conn.rs`, in the `ClientMsg::Intent { intent_id, ops }` arm (before `room.publish`), add:
```rust
if crate::chat::ops_target_message(&ops) {
    // Messages are server-authored via SendMessage only.
    let _ = etx.send(ServerMsg::Reject {
        intent_id,
        reason: RejectReason::Forbidden, // use the existing reject reason variant
    });
    continue; // or the arm's existing early-return shape
}
```
> Read the arm first: match the real reject/response mechanism (`ServerMsg::Reject { intent_id, reason }` shape and the `RejectReason` variant used elsewhere for `Forbidden`).

In `src/server/src/http/routes.rs`, in the `write_ops` handler (after building `ctx`, before applying), add:
```rust
if crate::chat::ops_target_message(&ops) {
    return Err(AppError::Forbidden); // match the existing HTTP error type used for authz denials
}
```

- [ ] **Step 4: Write the rejection integration test**

Prefer the Tier-A/repo path if a direct handler seam exists; otherwise add a Tier-B WS test in `src/server/tests/` (mirror `ws_convergence.rs`'s `spawn`/`connect`/`create_intent`). Minimal WS test:
```rust
// tests/chat_ingress.rs — a client Intent creating a `message` doc is rejected.
// Build a create-op intent with doc_type "message" and assert a `reject` frame,
// and that no `event` is broadcast (authoritative_seqs stays empty).
```
> Reuse `ws_convergence.rs`'s helpers (`spawn`, `connect`, `create_op` with `doc_type="message"`, `intent_msg`). Assert the received frame `type == "reject"` and `h.authoritative_seqs().await` is empty.

- [ ] **Step 5: Run tests**

Run: `cargo test -p shadowcat ops_target_message` and `cargo test -p shadowcat --test chat_ingress`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/chat/mod.rs src/server/src/ws/conn.rs src/server/src/http/routes.rs src/server/tests/chat_ingress.rs
git commit -m "feat(chat/m11c-1): reject client-authored message ops (ingress guard)"
```

---

### Task 7: `SendMessage` handler + flood limiter

**Files:**
- Modify: `src/server/src/ws/mod.rs` (add `message_rate` to `WsState`)
- Modify: `src/server/src/chat/mod.rs` (add `handle_send_message` + `SendMessageError` + `MAX_MESSAGE_CHARS`)
- Modify: `src/server/src/ws/conn.rs` (dispatch `ClientMsg::SendMessage`)
- Test: inline Tier-A test (`RoomRegistry` + repo) for `handle_send_message`

**Interfaces:**
- Consumes: `build_message_doc`, `plain_text_content`, `ActorOwnerRef`; `Room`, `Repository`, `PermissionContext`, `PingRateLimiter`.
- Produces: `pub const MAX_MESSAGE_CHARS: usize = 4096;`
- Produces: `pub enum SendMessageError { Empty, TooLong, RateLimited, Data(DataError) }`
- Produces: `pub async fn handle_send_message(room: &Room, repo: &dyn Repository, ctx: &PermissionContext, rate: &PingRateLimiter, channel: String, content: String, actor_owner: Option<ActorOwnerRef>, now: i64, budget_per_min: usize) -> Result<Command, SendMessageError>`.

- [ ] **Step 1: Write the failing test**

Add a Tier-A test in `chat/mod.rs` tests (mirror `room.rs`'s `repo_with_world` + `RoomRegistry`):
```rust
#[tokio::test]
async fn handle_send_message_publishes_and_broadcasts() {
    use crate::ws::room::RoomRegistry;
    use crate::ws::mod_ping::PingRateLimiter; // adjust to the real path of PingRateLimiter

    let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    let player = repo.create_user("pl", None, ServerRole::User, 0).await.unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player).await.unwrap();
    let ctx = PermissionContext { user_id: player, world_role: WorldRole::Player };

    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let (mut rx, _current) = room.subscribe();
    let rate = PingRateLimiter::new();

    let cmd = handle_send_message(&room, &repo, &ctx, &rate, "all".into(), "hello".into(), None, 100, 30)
        .await
        .unwrap();
    assert_eq!(cmd.seq, 1);
    let got = rx.recv().await.unwrap();
    assert_eq!(got.event_seq(), Some(1));

    // Rate limit: exhaust the budget then expect RateLimited.
    let rate2 = PingRateLimiter::new();
    for _ in 0..2 { let _ = handle_send_message(&room, &repo, &ctx, &rate2, "all".into(), "x".into(), None, 100, 2).await; }
    let err = handle_send_message(&room, &repo, &ctx, &rate2, "all".into(), "x".into(), None, 100, 2).await;
    assert!(matches!(err, Err(SendMessageError::RateLimited)));

    // Empty + too-long rejected before any publish.
    assert!(matches!(handle_send_message(&room, &repo, &ctx, &rate, "all".into(), "".into(), None, 100, 30).await, Err(SendMessageError::Empty)));
    let long = "a".repeat(MAX_MESSAGE_CHARS + 1);
    assert!(matches!(handle_send_message(&room, &repo, &ctx, &rate, "all".into(), long, None, 100, 30).await, Err(SendMessageError::TooLong)));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat handle_send_message_publishes`
Expected: FAIL — `handle_send_message` not found.

- [ ] **Step 3: Write minimal implementation**

Add to `chat/mod.rs` (`use` the real `Room`, `Repository`, `Command`, `DataError`, `PingRateLimiter` paths):
```rust
use crate::data::command::Command;
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::sqlite::DataError; // or wherever DataError lives
use crate::ws::room::Room;
use crate::ws::PingRateLimiter; // adjust to the real export

/// Max characters accepted for a single message's raw content (pre-producer).
pub const MAX_MESSAGE_CHARS: usize = 4096;

#[derive(Debug)]
pub enum SendMessageError {
    Empty,
    TooLong,
    RateLimited,
    Data(DataError),
}

/// Server-authoritative message ingest: flood-limit, validate, CONSTRUCT the
/// message doc, and publish it via the authoritative path. The sole message-
/// authoring entry point.
pub async fn handle_send_message(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    rate: &PingRateLimiter,
    channel: String,
    content: String,
    actor_owner: Option<ActorOwnerRef>,
    now: i64,
    budget_per_min: usize,
) -> Result<Command, SendMessageError> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(SendMessageError::Empty);
    }
    if content.chars().count() > MAX_MESSAGE_CHARS {
        return Err(SendMessageError::TooLong);
    }
    if !rate.check(ctx.user_id, now, budget_per_min) {
        return Err(SendMessageError::RateLimited);
    }
    let doc = build_message_doc(
        room_world_id(room), ctx.user_id, channel, actor_owner,
        plain_text_content(&content), now,
    );
    room.publish(repo, ctx, vec![Operation::Create { doc }], now)
        .await
        .map_err(SendMessageError::Data)
}
```
> `room_world_id(room)` — use the real accessor on `Room` for its world id (read `room.rs`; it is `self.world_id`). If no public accessor exists, add a small `pub fn world_id(&self) -> Uuid` to `Room`. Match the real `PingRateLimiter::check` signature (`check(user, now_ms, per_min)`).

In `src/server/src/ws/mod.rs`, add a field to `WsState` mirroring `ping_rate`:
```rust
pub message_rate: Arc<PingRateLimiter>,
```
and initialize it wherever `ping_rate` is initialized (same `Arc::new(PingRateLimiter::new())` / `Default`), so external construction sites (e.g. `tests/ws_convergence.rs`) that build `WsState` via its constructor need no change. If `WsState` is built with an explicit struct literal anywhere, add the field there too.

In `src/server/src/ws/conn.rs`, add the dispatch arm:
```rust
Ok(ClientMsg::SendMessage { channel, content, actor_owner }) => {
    match crate::chat::handle_send_message(
        &room, repo.as_ref(), &ctx, &message_rate,
        channel, content, actor_owner, now_millis(), MESSAGE_RATE_PER_MIN,
    ).await {
        Ok(_cmd) => {} // success confirmed by broadcast echo, like Intent
        Err(_e) => { /* optional: send a ServerMsg::Reject with an appropriate reason */ }
    }
}
```
> Bind `message_rate` from `ws_state.message_rate.clone()` alongside where `ping_rate` is bound in the connection setup. Define `const MESSAGE_RATE_PER_MIN: usize = 30;` (or route via `config.rs` like `effective_rate_per_min` if a per-role budget is wanted — a constant is sufficient for c-1).

- [ ] **Step 4: Run tests**

Run: `cargo test -p shadowcat handle_send_message_publishes`
Expected: PASS. Then `cargo test -p shadowcat` (full) to confirm `WsState` changes didn't break existing WS tests.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/chat/mod.rs src/server/src/ws/mod.rs src/server/src/ws/conn.rs
git commit -m "feat(chat/m11c-1): SendMessage handler + per-user flood limiter"
```

---

### Task 8: End-to-end delivery + search proof

**Files:**
- Create/Modify: `src/server/tests/chat_delivery.rs` (Tier-B WS) + a repo-level search test (inline in `chat/mod.rs` tests or a new tests file)

**Interfaces:**
- Consumes: everything above. No new production code — this task is the checkpoint's *proof* that a server-authored message rides create → sequence → broadcast → resync → search with no new transport/index code.

- [ ] **Step 1: Write the delivery test (Tier-B WS)**

In `src/server/tests/chat_delivery.rs`, mirror `ws_convergence.rs`'s harness (`spawn` with two members via `add_member`/login, `connect`). Send a `send_message` frame from a Player connection and assert a *second* connection receives a broadcast `event` whose created doc is a `message`:
```rust
// 1. spawn server; add a Player member + cookie (mirror ws_convergence spawn/login).
// 2. connect player WS (ws_p) and observer WS (ws_o); drain each Welcome.
// 3. ws_p.send(json!({"type":"send_message","channel":"all","content":"hello","actor_owner":null}).to_string()).
// 4. On ws_o, drain until a frame with type=="event"; assert
//    evt["command"]["ops"][0]["op"] == "create"
//    evt["command"]["ops"][0]["doc"]["doc_type"] == "message"
//    evt["command"]["ops"][0]["doc"]["system"]["content"][0]["text"] == "hello".
```
> If wiring a full two-client WS test is heavy, the Tier-A `handle_send_message_publishes_and_broadcasts` test (Task 7) already proves publish→broadcast; this task then only needs the *resync* + *search* proofs below. Include the WS test if the harness reuse is straightforward; otherwise document that publish/broadcast is covered by Task 7 and focus this task on search + a resync assertion via `repo.events_since`.

- [ ] **Step 2: Write the search proof (repo-level)**

Add a test (mirror `sqlite.rs`'s `search_ranks_and_filters_by_read_access`): a message posted by a Player is found by another member's `search`, and its body text appears in the snippet.
```rust
#[tokio::test]
async fn posted_message_is_searchable_by_members() {
    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    let player = r.create_user("pl", None, ServerRole::User, 0).await.unwrap();
    let other = r.create_user("ot", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();
    r.add_member(w.id, other, WorldRole::Player).await.unwrap();
    let pl_ctx = PermissionContext { user_id: player, world_role: WorldRole::Player };
    let ot_ctx = PermissionContext { user_id: other,  world_role: WorldRole::Player };

    let doc = crate::chat::build_message_doc(
        w.id, player, "all".into(), None,
        crate::chat::plain_text_content("banshee wail"), 1,
    );
    r.apply_intent(&pl_ctx, w.id, vec![Operation::Create { doc }], 1).await.unwrap();

    let page = r.search(&ot_ctx, w.id, "banshee", 10, None).await.unwrap();
    assert_eq!(page.hits.len(), 1, "another member finds the message");
    assert!(page.hits[0].snippet.to_lowercase().contains("banshee"));
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p shadowcat posted_message_is_searchable` and (if added) `cargo test -p shadowcat --test chat_delivery`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/server/tests/chat_delivery.rs src/server/src/chat/mod.rs
git commit -m "test(chat/m11c-1): prove server-authored message rides delivery + search"
```

---

### Task 9: `shadowcat-codebase-chat` skill (reviewed skill-update gate)

**Files:**
- Create: `.claude/skills/shadowcat-codebase-chat/SKILL.md`
- Modify: the codebase-skill activation hook (add `src/server/src/chat/**` globs) — locate it alongside the other `shadowcat-codebase-*` skill registrations.

> `.claude/` is largely git-ignored; this is a working-environment + process-gate deliverable, not a committed one. Per CLAUDE.md, dispatch `shadowcat-spec-reviewer` on the skill diff to confirm it accurately captures the c-1 surface before the checkpoint is considered done.

- [ ] **Step 1: Author the skill** using the fixed shape of the sibling `shadowcat-codebase-*` skills (Purpose / Key files / Hard invariants / Gotchas / Pointers). Content must capture: messages are server-constructed documents (never client-built); the `SendMessage`-only authoring invariant + the ingress-guard/create-exemption coupling; `chat/mod.rs` as the domain home; the opaque-body/no-ts-rs content model (only `ActorOwnerRef` is ts-rs); and pointers into `docs/superpowers/specs/2026-07-08-m11c-chat-core-design.md`, graphify, and `documents-permissions`/`realtime-sync`.

- [ ] **Step 2: Register globs** in the activation hook so edits under `src/server/src/chat/` remind the agent to invoke the skill (mirror an existing subsystem's entry).

- [ ] **Step 3: Reviewed skill-update gate.** Dispatch `shadowcat-spec-reviewer` on the skill; fix any inaccuracy it finds; record the PASS. (No git commit — `.claude/` is ignored.)

---

### Task 10: Whole-checkpoint verification + docs sync

**Files:**
- Modify: `docs/PLAN.md` (M11c-1 status)

- [ ] **Step 1: Full verification.**
Run: `cargo test -p shadowcat` (all server tests) — Expected: PASS.
Run: `cargo clippy --all-targets -- -D warnings` — Expected: clean **except** the pre-existing unrelated `scene/move_exec.rs:759 region_doc too_many_arguments` warning already tracked in `docs/TODO.md` (do not fix it here). If a NEW warning appears in chat code, fix it.
Run: `git diff --exit-code src/types/generated` — Expected: no diff (all regenerated bindings committed).
Run: `pnpm --filter @shadowcat/core test && pnpm --filter @shadowcat/core exec tsc --noEmit` — Expected: PASS.

- [ ] **Step 2: Update `docs/PLAN.md`** — record M11c-1 DONE under the M11c section (message model + server-authoritative ingest + delivery), noting: server-constructed messages, `SendMessage`-only authoring, Seam-B Player-baseline posting, and that c-2 (whisper allowlist) is next.

- [ ] **Step 3: Commit.**
```bash
git add docs/PLAN.md
git commit -m "docs(chat/m11c-1): record M11c-1 complete in PLAN.md"
```

---

## Self-Review

**Spec coverage (design §3 M11c-1 + §4):**
- Message model (`MessageSystem`, `MessageKind`, `Segment`, `ActorOwnerRef`) → Tasks 1-3. ✓
- Plain-text producer → Task 2. ✓
- `SendMessage` intent + server construction → Tasks 4, 7. ✓
- Server-authored authz (Seam B) → Task 5. ✓
- Reject client-authored messages → Task 6. ✓
- Delivery proof (create→broadcast→resync→search, generic) → Tasks 7-8. ✓
- Flood limiter → Task 7. ✓
- `ActorOwnerRef` ts-rs + Zod → Tasks 1, 4. ✓
- `shadowcat-codebase-chat` skill → Task 9. ✓
- Testing §4 c-1 bullets (SendMessage round-trip, client Create rejected, flood, ActorOwnerRef serde, ts-rs↔Zod parity) → Tasks 4-8. ✓
- Channels: intentionally NOT in c-1 (design §2.3 — `chat-channel` config docs + seeding are M11d; a c-1 message just carries a `channel` string). Noted, not a gap.

**Placeholder scan:** Snippets that say "match the real code / read the file first" are deliberate — they guard against the plan-vs-code drift lesson (M11a) where later tasks trusted stale signatures. Every code step shows concrete code; the "read first" notes are adaptation instructions, not deferred work.

**Type consistency:** `ActorOwnerRef`, `MessageSystem`, `Segment`, `MessageKind`, `build_message_doc`, `plain_text_content`, `ops_target_message`, `handle_send_message`, `MESSAGE_DOC_TYPE`, `MAX_MESSAGE_CHARS`, `SendMessageError` are used consistently across Tasks 1-10. `handle_send_message` consumes `build_message_doc`/`plain_text_content` (Tasks 2-3) as defined.

**Open adaptation risks flagged for the implementer:** the exact `Document` field set (Task 3), the `Operation` variants (Task 6), the `ServerMsg::Reject`/`RejectReason` and `AppError` shapes (Task 6), the `PingRateLimiter` export path and `Room::world_id` accessor (Task 7), and the `WsState` construction sites (Task 7) must each be matched against the real source, not the plan's illustrative snippet.
