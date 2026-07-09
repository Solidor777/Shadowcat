# M11c-2 · Restricted-Audience Messaging (Whisper + GM-Only Channel) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a chat message restrict its readership below the world-default ("every member reads") — either to a sender-picked list of recipients (a whisper, which also excludes the GM unless individually named), or to whoever currently holds `WorldRole::Gm` (a GM-only channel any member may post into) — enforced fail-closed on every egress path (broadcast, resync/load, search).

**Architecture:** One new field, `PermissionSet.gm_role: Option<DocRole>`, makes the GM's usual unconditional access conditional per-document. `resolve_access`'s GM short-circuit becomes: `None` (every pre-existing document) → unchanged unconditional access; `Some(role)` → the GM falls through to the *same* per-document role-floor resolution every other actor already uses, seeded with `role` as their fallback when not individually listed in `permissions.users`. Because `resolve_access`/`resolve_access_world` is the single chokepoint every egress path (`filter_command` broadcast, `search`'s per-hit filter, `query_documents`/`get_document` HTTP reads) already calls, this one field is automatically honored everywhere. A new `chat::Audience` enum (`Public` / `Whisper{recipients}` / `GmOnly`) on the `SendMessage` frame drives `build_message_doc`'s `PermissionSet` construction per a fixed table; `handle_send_message` validates whisper recipients against real world membership before constructing anything (fail-closed).

**Tech Stack:** Rust (server: `serde`/`serde_json`, `ts-rs` bindings emitted on `cargo test`, `tokio`, `axum`/WS, `sqlx`+SQLite). Client: Zod mirrors in `src/client/core/src/wire.ts` (`pnpm`/vitest). No new crates.

## Global Constraints

- **`gm_role: None` (the default, `#[serde(default)]`) must change behavior for zero existing documents.** Every current test (secret regions, actors, `OwnerOrGm` tier) relies on the GM's unconditional short-circuit; this checkpoint must not touch it for any doc that doesn't explicitly opt in.
- **A whisper hides from the GM by default.** The GM sees a whisper only if their own user id is among its `recipients` — this supersedes the parent M11c spec's "(per world policy, default-on) the GM" language (design doc §0).
- **The GM-only channel is dynamically resolved**, not a frozen roster: a co-GM promoted after a message was sent immediately sees it; one demoted immediately loses access.
- **Fail-closed on a malformed whisper.** Any `recipients` uuid that is not a real world member rejects the *entire* `SendMessage` — nothing is persisted.
- **A redundant self-recipient must never downgrade the owner.** `build_message_doc` inserts `owner: Owner` into `permissions.users` LAST.
- **`channel` stays a purely client-chosen, server-unvalidated label** (unchanged from c-1) — `Audience` is the only server-enforced visibility concept this checkpoint adds.
- **ts-rs bindings are CI-checked.** ts-rs writes `.ts` files as a side effect of `cargo test`; CI runs `git diff --exit-code src/types/generated` (Linux). Every task that changes a `#[derive(TS)]` type MUST `cargo test` and commit the regenerated files.
- **IDs are bare `uuid::Uuid`** (no newtypes) server-side, `string` in TS, `z.string()` in Zod.
- **Cross-platform.** No OS-specific code; server must compile/test on ubuntu/macos/windows (CI matrix).
- **Commit after every task** once its tests pass (`cargo test -p shadowcat` for touched areas green, `cargo clippy --all-targets -- -D warnings` clean for touched crates).
- **Mandatory buddy-check** on the permission-chokepoint change (Task 1) and the audience→permission mapping (Task 3) before this checkpoint is considered done — see Buddy-check directives.

## Model/Effort directives

- **Plan authored mainline** in this session (user directive: "You write the plan" / "like I said, you" — declined `sdd-plan-writer-*` dispatch), at the session's current model/effort (no `/model` or `/effort` switch was requested).
- **Dispatcher: this session runs the loop mainline** (user directive: "you are the dispatcher" — declined `sdd-dispatcher` sub-delegation).
- **Execution:** subagent-driven-development. Implementation via `shadowcat-coder` (sonnet, effort medium); each task reviewed by the `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` two-reviewer gate (effort high). Escalate a blocked task to `shadowcat-coder-opus`; escalate a shallow/uncertain review to the `-opus` reviewer twin.

## Buddy-check directives

- **This checkpoint is HIGH risk (architecture-consent)** per the parent M11c design's rating — it changes `resolve_access`, the one permission chokepoint every subsystem (broadcast, search, resync, HTTP reads) depends on being simple and universal.
- **Offer a full `buddy-checking` run (two independent blind reviewers + adversarial debate) on Task 1** (the `resolve_access`/`resolve_access_world` refactor in `permission.rs`) **and Task 3** (the `Audience` → `PermissionSet` mapping table in `build_message_doc`) specifically — these two are where a subtle mistake would silently leak a whisper or GM-only message. The per-task `shadowcat-spec-reviewer`/`shadowcat-code-reviewer` gate is the baseline; escalate to full `buddy-checking` if either review reads as shallow or uncertain, or proactively for Task 1 given its blast radius.
- **The integration tests in Tasks 7-8 are the checkpoint's actual proof** — per-recipient/per-GM exclusion on every egress path (broadcast, resync/load, search), plus the dynamic-promotion proof. Do not consider the checkpoint done without them green.

---

## File Structure

**Created:**
- `src/server/tests/chat_audience.rs` — integration proof: whisper + GM-only-channel visibility on every egress path, over real WS/HTTP connections.

**Modified:**
- `src/server/src/data/document.rs` — `PermissionSet.gm_role: Option<DocRole>`.
- `src/server/src/data/permission.rs` — `resolve_access`/`resolve_access_world` refactored around a shared `effective_role` helper that consults `gm_role`.
- `src/server/src/data/repository.rs` — new `Repository::member_role` trait method.
- `src/server/src/data/sqlite.rs` — `impl Repository for SqliteRepository::member_role` (delegates to the existing inherent method).
- `src/server/src/chat/mod.rs` — new `Audience` enum; `MessageSystem.audience`; `build_message_doc` and `handle_send_message` take `audience: Audience`; new `SendMessageError::UnknownRecipient`; existing call sites in this file's tests updated for the new parameter.
- `src/server/src/ws/protocol.rs` — `ClientMsg::SendMessage.audience: Audience` (`#[serde(default)]`).
- `src/server/src/ws/conn.rs` — pass `audience` through to `handle_send_message`.
- `src/types/generated/Audience.ts`, `src/types/generated/PermissionSet.ts`, `src/types/generated/ClientMsg.ts` — ts-rs output (regenerated, committed).
- `src/client/core/src/wire.ts` — `AudienceSchema`, `PermissionSetSchema.gm_role`, `SendMessageSchema.audience`, `ClientMsg` type union.
- `src/client/core/src/wire.test.ts` — drift guard + parse tests for the above.
- `.claude/skills/shadowcat-codebase-chat/SKILL.md`, `.claude/skills/shadowcat-codebase-documents-permissions/SKILL.md` — reviewed skill-update gate.
- `docs/PLAN.md` — M11c-2 status (final task).

---

### Task 1: `PermissionSet.gm_role` + DRY `resolve_access`/`resolve_access_world`

**Files:**
- Modify: `src/server/src/data/document.rs` (`PermissionSet` struct, ~line 179-188)
- Modify: `src/server/src/data/permission.rs` (`resolve_access` ~line 143-171, `resolve_access_world` ~line 177-200)
- Test: inline `#[cfg(test)]` in `src/server/src/data/permission.rs`

**Interfaces:**
- Produces: `PermissionSet.gm_role: Option<DocRole>` (`#[serde(default)]`, ts-rs exported).
- Produces (internal): `fn effective_role(user: Uuid, world_role: WorldRole, doc: &Document) -> Option<DocRole>` — `None` means the unconditional GM/admin all-access short-circuit; `Some(role)` is the role this actor effectively holds on this document.
- Behavior change: `resolve_access`/`resolve_access_world` are unchanged in every observable way for any document with `gm_role: None` (the default). For a document with `gm_role: Some(role)`, a `WorldRole::Gm` actor is capped like any other actor: they get `role` unless individually listed in `permissions.users`.

> **Read first:** open `src/server/src/data/permission.rs` and match the real, current `resolve_access`/`resolve_access_world`/`role_floor` against the code shown below — this plan was written against the current source, but re-verify line numbers before editing.

- [ ] **Step 1: Write the failing tests**

In `src/server/src/data/permission.rs`, inside the existing `#[cfg(test)] mod tests { ... }` block (it already has a `doc(perms, system)` helper and imports `PermissionSet`, `Scope`), add:

```rust
#[test]
fn gm_role_none_excludes_gm_unless_individually_granted() {
    let owner = Uuid::from_u128(1);
    let gm = Uuid::from_u128(2);
    let mut perms = PermissionSet {
        default: DocRole::None,
        gm_role: Some(DocRole::None),
        ..Default::default()
    };
    perms.users.insert(owner, DocRole::Owner);
    let d = doc(perms, serde_json::json!({}));

    // A GM not individually listed gets nothing — gm_role caps them like any other actor.
    let a_gm = resolve_access(gm, WorldRole::Gm, &d);
    assert!(
        !a_gm.has(cap::READ),
        "unlisted GM must not read a gm_role:None document"
    );
    assert!(
        !a_gm.all,
        "gm_role:Some(_) must not grant the unconditional short-circuit"
    );

    // The owner is unaffected.
    let a_owner = resolve_access(owner, WorldRole::Player, &d);
    assert!(a_owner.has(cap::READ));
}

#[test]
fn gm_role_none_admits_a_gm_individually_listed() {
    let owner = Uuid::from_u128(1);
    let gm = Uuid::from_u128(2);
    let mut perms = PermissionSet {
        default: DocRole::None,
        gm_role: Some(DocRole::None),
        ..Default::default()
    };
    perms.users.insert(owner, DocRole::Owner);
    perms.users.insert(gm, DocRole::Observer); // e.g. a whisper naming the GM
    let d = doc(perms, serde_json::json!({}));

    let a_gm = resolve_access(gm, WorldRole::Gm, &d);
    assert!(
        a_gm.has(cap::READ),
        "a GM individually listed in `users` must read despite gm_role:None"
    );
    assert!(
        !a_gm.all,
        "still not the unconditional short-circuit — just an ordinary Observer grant"
    );
}

#[test]
fn gm_role_observer_grants_any_gm_without_explicit_listing() {
    let owner = Uuid::from_u128(1);
    let gm = Uuid::from_u128(2);
    let stranger = Uuid::from_u128(3);
    let mut perms = PermissionSet {
        default: DocRole::None,
        gm_role: Some(DocRole::Observer),
        ..Default::default()
    };
    perms.users.insert(owner, DocRole::Owner);
    let d = doc(perms, serde_json::json!({}));

    // Any GM reads, even without being individually listed (dynamic resolution).
    let a_gm = resolve_access(gm, WorldRole::Gm, &d);
    assert!(a_gm.has(cap::READ));
    assert!(a_gm.see_gm_only, "still a GM for property-tier purposes");

    // A non-owner, non-GM Player reads nothing.
    let a_stranger = resolve_access(stranger, WorldRole::Player, &d);
    assert!(!a_stranger.has(cap::READ));
}

#[test]
fn resolve_access_world_layers_world_grants_using_the_gm_role_fallback() {
    use crate::data::document::CapabilityGrants;
    let owner = Uuid::from_u128(1);
    let gm = Uuid::from_u128(2);
    let mut perms = PermissionSet {
        default: DocRole::None,
        gm_role: Some(DocRole::Observer),
        ..Default::default()
    };
    perms.users.insert(owner, DocRole::Owner);
    let d = doc(perms, serde_json::json!({}));

    let mut world_grants = CapabilityGrants::default();
    world_grants
        .by_role
        .entry(DocRole::Observer)
        .or_default()
        .insert("dnd5e:extra".to_string());

    // A GM not individually listed still resolves via the gm_role (Observer)
    // fallback, so world-level Observer grants must layer on top of it too —
    // not just `doc.permissions.default` (None here, which carries no such
    // grant). Proves resolve_access_world uses the SAME effective role as
    // resolve_access rather than recomputing it independently.
    let a_gm = resolve_access_world(gm, WorldRole::Gm, &d, &world_grants);
    assert!(
        a_gm.has("dnd5e:extra"),
        "world grant for the gm_role fallback role must apply"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat --lib data::permission::tests::gm_role`
Expected: FAIL — `no field \`gm_role\` on type \`PermissionSet\``.

- [ ] **Step 3: Add the field**

In `src/server/src/data/document.rs`, modify the `PermissionSet` struct:

```rust
pub struct PermissionSet {
    pub default: DocRole,
    pub users: BTreeMap<Uuid, DocRole>,
    pub property_overrides: BTreeMap<String, Visibility>,
    #[serde(default)]
    pub capabilities: CapabilityGrants,
    /// When `Some(role)`, a `WorldRole::Gm` actor's access to THIS document is
    /// capped like any other actor's — resolved via the same per-document
    /// `users`/role-floor logic, seeded with `role` as their fallback instead
    /// of the unconditional GM short-circuit. `None` (the default for every
    /// document type that predates this field) preserves the GM's usual
    /// unconditional `all: true` access. Lets a document (e.g. a chat whisper
    /// or a GM-only channel message) restrict even the GM unless explicitly
    /// granted — see `permission::resolve_access`.
    #[serde(default)]
    pub gm_role: Option<DocRole>,
}
```

- [ ] **Step 4: Write minimal implementation**

In `src/server/src/data/permission.rs`, replace the existing `resolve_access` and `resolve_access_world` functions (and everything between/around them) with:

```rust
/// The document-level role this actor effectively holds, or `None` when they
/// hold the unconditional GM/admin all-access short-circuit — no single role
/// applies, because every capability is granted regardless. Shared by
/// `resolve_access` (which turns this into an `Access`) and
/// `resolve_access_world` (which needs the SAME effective role to layer
/// world-default grants consistently — recomputing it independently from
/// `doc.permissions.default` would silently diverge for a GM whose access is
/// capped via `gm_role`).
fn effective_role(user: Uuid, world_role: WorldRole, doc: &Document) -> Option<DocRole> {
    if world_role == WorldRole::Gm {
        let fallback = doc.permissions.gm_role?;
        return Some(doc.permissions.users.get(&user).copied().unwrap_or(fallback));
    }
    Some(
        doc.permissions
            .users
            .get(&user)
            .copied()
            .unwrap_or(doc.permissions.default),
    )
}

/// Resolve a user's effective capabilities on a document. A world GM (or
/// server admin, which resolves to GM) holds every capability UNLESS the
/// document's `gm_role` caps them to an ordinary per-document role (see
/// `effective_role`) — used by restricted-audience chat messages. Otherwise
/// the user's `DocRole` (per-user, else the document default) seeds a
/// built-in floor that the document's additive grants (`by_role`, `by_user`)
/// widen.
pub fn resolve_access(user: Uuid, world_role: WorldRole, doc: &Document) -> Access {
    let Some(role) = effective_role(user, world_role, doc) else {
        return Access {
            caps: BTreeSet::new(),
            all: true,
            see_gm_only: true,
            is_owner: true,
        };
    };
    let mut caps = role_floor(role);
    if let Some(extra) = doc.permissions.capabilities.by_role.get(&role) {
        caps.extend(extra.iter().cloned());
    }
    if let Some(extra) = doc.permissions.capabilities.by_user.get(&user) {
        caps.extend(extra.iter().cloned());
    }
    Access {
        caps,
        all: false,
        // A GM capped via `gm_role` remains the GM for property-tier
        // (`GmOnly`/`OwnerOrGm`) visibility purposes even though their
        // whole-document READ is now floor-gated like anyone else's.
        see_gm_only: world_role == WorldRole::Gm,
        is_owner: doc.owner == Some(user),
    }
}

/// `resolve_access` plus a world's default capability grants, layered
/// additively on top of the per-document resolution (unaffected when
/// `resolve_access` already returned the unconditional GM short-circuit).
/// World defaults let a deployment grant, e.g., every Owner in a world
/// `core:manage_embedded` without editing each document. Uses the SAME
/// `effective_role` as `resolve_access` — including a `gm_role`-capped GM's
/// fallback role — so a world-level grant for that role also applies to them.
pub fn resolve_access_world(
    user: Uuid,
    world_role: WorldRole,
    doc: &Document,
    world_grants: &CapabilityGrants,
) -> Access {
    let mut access = resolve_access(user, world_role, doc);
    if access.all {
        return access;
    }
    let role = effective_role(user, world_role, doc)
        .expect("access.all was false, so effective_role returned Some (see resolve_access)");
    if let Some(extra) = world_grants.by_role.get(&role) {
        access.caps.extend(extra.iter().cloned());
    }
    if let Some(extra) = world_grants.by_user.get(&user) {
        access.caps.extend(extra.iter().cloned());
    }
    access
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p shadowcat --lib data::permission::tests`
Expected: PASS — all new `gm_role_*` tests plus every pre-existing `permission.rs` test (regression: `owner_or_gm_visible_to_owner_and_gm_not_other_player`, `filter_command_create_drops_op_entirely_for_default_none_region`, `gm_holds_every_capability`, etc. — all use `gm_role: None` implicitly via `Default`, so must be byte-for-byte unaffected).
Run: `cargo test -p shadowcat` (full suite) — Expected: PASS, no regression anywhere else that constructs a `PermissionSet` literal without `gm_role` (they all use `..Default::default()` or the `doc()`/`perms_with()` test helpers, which pick up the new field's `Default::default()` automatically).

- [ ] **Step 6: Commit**

```bash
git add src/server/src/data/document.rs src/server/src/data/permission.rs
git commit -m "feat(chat/m11c-2): add PermissionSet.gm_role, DRY resolve_access around effective_role"
```

---

### Task 2: `Repository::member_role` trait method

**Files:**
- Modify: `src/server/src/data/repository.rs` (add trait method)
- Modify: `src/server/src/data/sqlite.rs` (implement it in `impl Repository for SqliteRepository`, delegating to the existing inherent `SqliteRepository::member_role`)
- Test: inline `#[cfg(test)]` in `src/server/src/data/sqlite.rs`

**Interfaces:**
- Produces: `async fn member_role(&self, world: Uuid, user: Uuid) -> Result<Option<WorldRole>, DataError>` on the `Repository` trait — lets `chat::handle_send_message` (which only has `repo: &dyn Repository`, a trait object) validate a whisper's recipient uuids against real world membership.

> `SqliteRepository` already has an **inherent** `pub async fn member_role(&self, world_id: Uuid, user_id: Uuid) -> Result<Option<WorldRole>, DataError>` at `src/server/src/data/sqlite.rs:608` (used internally by `permission_context`). This task exposes the SAME query through the `Repository` TRAIT so `dyn Repository` callers can reach it — it does not duplicate the SQL.

- [ ] **Step 1: Write the failing test**

In `src/server/src/data/sqlite.rs`'s `#[cfg(test)] mod tests { ... }` block, add:

```rust
#[tokio::test]
async fn repository_trait_member_role_matches_inherent_method() {
    use crate::auth::role::ServerRole;
    use crate::data::repository::Repository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    let player = r.create_user("pl", None, ServerRole::User, 0).await.unwrap();
    let stranger = r.create_user("st", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();

    let dyn_repo: &dyn Repository = &r;
    assert_eq!(
        dyn_repo.member_role(w.id, player).await.unwrap(),
        Some(WorldRole::Player)
    );
    assert_eq!(dyn_repo.member_role(w.id, stranger).await.unwrap(), None);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat repository_trait_member_role_matches_inherent_method`
Expected: FAIL — `no method named \`member_role\` found for reference \`&dyn Repository\``.

- [ ] **Step 3: Write minimal implementation**

In `src/server/src/data/repository.rs`, add `WorldRole` to the existing import and add the trait method (place it after `get_world`, before `world_cap_defaults`):

```rust
use crate::data::document::{
    CapabilityRequirement, ContractDeclaration, Document, World, WorldCapDefaults, WorldRole,
};
```

```rust
    /// Fetch a world row by id, or `None` if it does not exist.
    async fn get_world(&self, id: Uuid) -> Result<Option<World>, DataError>;

    /// A user's role within `world`, or `None` if they are not a member.
    /// Lets a `dyn Repository` caller (e.g. `chat::handle_send_message`)
    /// validate candidate uuids — a whisper's recipients — actually belong to
    /// the world before trusting them.
    async fn member_role(&self, world: Uuid, user: Uuid) -> Result<Option<WorldRole>, DataError>;

    /// A world's default capability grants (additive over the per-document
    /// `DocRole` floor). Empty when unset.
    async fn world_cap_defaults(&self, world: Uuid) -> Result<WorldCapDefaults, DataError>;
```

In `src/server/src/data/sqlite.rs`, inside `impl Repository for SqliteRepository { ... }` (starts at line 785), add — right after the existing `async fn get_world` implementation, before `async fn world_cap_defaults`:

```rust
    async fn member_role(&self, world: Uuid, user: Uuid) -> Result<Option<WorldRole>, DataError> {
        // Delegates to the inherent method of the same name (line ~608);
        // method resolution on a concrete `SqliteRepository` self prefers the
        // inherent impl, so this is not infinite recursion.
        SqliteRepository::member_role(self, world, user).await
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p shadowcat repository_trait_member_role_matches_inherent_method`
Expected: PASS.
Run: `cargo test -p shadowcat` (full) — Expected: PASS (adding a trait method only breaks other implementors of `Repository`; confirm there is exactly one, `SqliteRepository` — `grep -n "impl Repository for" src/server/src/data/sqlite.rs` should show only one match).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/data/repository.rs src/server/src/data/sqlite.rs
git commit -m "feat(chat/m11c-2): expose member_role on the Repository trait"
```

---

### Task 3: `chat::Audience` enum + `MessageSystem.audience` + `build_message_doc` mapping

**Files:**
- Modify: `src/server/src/chat/mod.rs` (add `Audience`; extend `MessageSystem`; rewrite `build_message_doc`; update every existing test call site in this file)
- Test: inline `#[cfg(test)]` in `src/server/src/chat/mod.rs`

**Interfaces:**
- Consumes: `crate::data::document::{DocRole, PermissionSet}` (already imported), `PermissionSet.gm_role` (Task 1).
- Produces: `pub enum Audience { Public, Whisper { recipients: Vec<Uuid> }, GmOnly }` (ts-rs exported, `Default = Public`).
- Produces: `MessageSystem.audience: Audience` (new required field on the opaque body).
- Produces: `pub fn build_message_doc(world_id: Uuid, user: Uuid, channel: String, actor_owner: Option<ActorOwnerRef>, audience: Audience, content: Vec<Segment>, now: i64) -> Document` — **signature changed**: `audience` inserted between `actor_owner` and `content`.

> **Read first:** re-open `src/server/src/chat/mod.rs` and confirm the current `MessageSystem`/`build_message_doc`/test bodies match what's shown below (this plan was written against the live c-1 source) before editing.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests { ... }` block in `src/server/src/chat/mod.rs`:

```rust
#[test]
fn audience_tagged_roundtrip_and_default() {
    let w = Audience::Whisper {
        recipients: vec![Uuid::from_u128(1)],
    };
    let j = serde_json::to_value(&w).unwrap();
    assert_eq!(j["kind"], "whisper");
    assert_eq!(w, serde_json::from_value(j).unwrap());
    assert_eq!(
        serde_json::to_value(Audience::GmOnly).unwrap()["kind"],
        "gm_only"
    );
    assert_eq!(Audience::default(), Audience::Public);
}

#[test]
fn build_message_doc_public_matches_c1_shape() {
    let owner = Uuid::from_u128(1);
    let doc = build_message_doc(
        Uuid::from_u128(9),
        owner,
        "all".into(),
        None,
        Audience::Public,
        plain_text_content("hi"),
        0,
    );
    assert_eq!(doc.permissions.default, DocRole::Observer);
    assert_eq!(doc.permissions.gm_role, None);
    assert_eq!(doc.permissions.users.get(&owner), Some(&DocRole::Owner));
}

#[test]
fn build_message_doc_whisper_restricts_default_and_gm() {
    let owner = Uuid::from_u128(1);
    let recipient = Uuid::from_u128(2);
    let doc = build_message_doc(
        Uuid::from_u128(9),
        owner,
        "whispers".into(),
        None,
        Audience::Whisper {
            recipients: vec![recipient],
        },
        plain_text_content("psst"),
        0,
    );
    assert_eq!(doc.permissions.default, DocRole::None);
    assert_eq!(doc.permissions.gm_role, Some(DocRole::None));
    assert_eq!(doc.permissions.users.get(&owner), Some(&DocRole::Owner));
    assert_eq!(
        doc.permissions.users.get(&recipient),
        Some(&DocRole::Observer)
    );
}

#[test]
fn build_message_doc_whisper_self_recipient_does_not_downgrade_owner() {
    let owner = Uuid::from_u128(1);
    let doc = build_message_doc(
        Uuid::from_u128(9),
        owner,
        "whispers".into(),
        None,
        Audience::Whisper {
            recipients: vec![owner],
        },
        plain_text_content("note to self"),
        0,
    );
    assert_eq!(
        doc.permissions.users.get(&owner),
        Some(&DocRole::Owner),
        "a redundant self-recipient must never downgrade the owner to Observer"
    );
}

#[test]
fn build_message_doc_gm_only_has_no_named_recipients() {
    let owner = Uuid::from_u128(1);
    let doc = build_message_doc(
        Uuid::from_u128(9),
        owner,
        "gm".into(),
        None,
        Audience::GmOnly,
        plain_text_content("only the GM sees this"),
        0,
    );
    assert_eq!(doc.permissions.default, DocRole::None);
    assert_eq!(doc.permissions.gm_role, Some(DocRole::Observer));
    assert_eq!(
        doc.permissions.users.len(),
        1,
        "only the owner is individually listed — every GM sees it dynamically via gm_role"
    );
    assert_eq!(doc.permissions.users.get(&owner), Some(&DocRole::Owner));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat --lib chat::tests::audience`
Expected: FAIL — `cannot find type \`Audience\` in this scope`.

- [ ] **Step 3: Write the `Audience` enum**

In `src/server/src/chat/mod.rs`, insert immediately after the closing `}` of `ActorOwnerRef` (after line 58, before the `MessageKind` doc comment):

```rust
/// The intended readership of a message, beyond the ordinary world-readable
/// default. Carried on the `SendMessage` frame and stored verbatim in
/// `MessageSystem`; drives the document's `PermissionSet` in
/// `build_message_doc` (see that function for the exact mapping). `channel`
/// stays a purely client-chosen label — the server never validates it or
/// derives audience from it; a client module choosing to post into a "GM"
/// channel is what sets `Audience::GmOnly`, not the channel string itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Audience {
    /// Every world member may read (c-1's original, unrestricted shape).
    #[default]
    Public,
    /// Only `recipients` (plus the sender) may read. The GM reads it ONLY if
    /// their own uuid is among `recipients` — not automatically.
    Whisper { recipients: Vec<Uuid> },
    /// Only whoever currently holds `WorldRole::Gm` (plus the sender) may
    /// read — resolved dynamically, not a frozen roster at send time.
    GmOnly,
}
```

- [ ] **Step 4: Extend `MessageSystem` and rewrite `build_message_doc`**

Replace the `MessageSystem` doc comment + struct (current lines 93-105) with:

```rust
/// The message document's `system` body. Opaque on the wire (no ts-rs); the
/// client declares its own Zod mirror in M11d.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSystem {
    pub channel: String,
    /// The owning user; server-set to the authenticated poster (== `Document.owner`).
    pub user_owner: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_owner: Option<ActorOwnerRef>,
    pub kind: MessageKind,
    pub audience: Audience,
    pub content: Vec<Segment>,
}
```

Replace `build_message_doc` (current lines 107-146) with:

```rust
/// Server-construct a message `Document`. INVARIANT: only the server calls
/// this (via `handle_send_message`); clients never build message docs.
/// `audience` drives the document's `PermissionSet`:
/// - `Public` — `default: Observer`, `gm_role: None` (c-1's original,
///   world-readable shape; the GM's unconditional access is unaffected).
/// - `Whisper { recipients }` — `default: None`, `gm_role: Some(None)` (the
///   GM reads only if individually listed), `users` holds `owner: Owner` plus
///   each recipient as `Observer`.
/// - `GmOnly` — `default: None`, `gm_role: Some(Observer)` (ANY current GM
///   reads, resolved dynamically — not a frozen roster), `users` holds only
///   `owner: Owner`.
/// In every case `owner` is inserted into `users` LAST, so a `Whisper` that
/// redundantly names the sender as their own recipient can never downgrade
/// them from `Owner` to `Observer` via map-insertion order.
pub fn build_message_doc(
    world_id: Uuid,
    user: Uuid,
    channel: String,
    actor_owner: Option<ActorOwnerRef>,
    audience: Audience,
    content: Vec<Segment>,
    now: i64,
) -> Document {
    let (default, gm_role, mut users) = match &audience {
        Audience::Public => (DocRole::Observer, None, BTreeMap::new()),
        Audience::Whisper { recipients } => {
            let mut users = BTreeMap::new();
            for &r in recipients {
                if r != user {
                    users.insert(r, DocRole::Observer);
                }
            }
            (DocRole::None, Some(DocRole::None), users)
        }
        Audience::GmOnly => (DocRole::None, Some(DocRole::Observer), BTreeMap::new()),
    };
    users.insert(user, DocRole::Owner);
    let system = MessageSystem {
        channel,
        user_owner: user,
        actor_owner,
        kind: MessageKind::Normal,
        audience,
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
            default,
            users,
            gm_role,
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

- [ ] **Step 5: Update every existing `build_message_doc` call site in this file's tests**

Every existing test that calls `build_message_doc` now needs `Audience::Public` inserted as the 5th argument (between `actor_owner` and `content`). Replace each of the following test bodies verbatim:

```rust
#[test]
fn build_message_doc_is_server_owned_message() {
    let world = Uuid::from_u128(10);
    let user = Uuid::from_u128(20);
    let doc = build_message_doc(
        world,
        user,
        "all".into(),
        None,
        Audience::Public,
        plain_text_content("hi"),
        1234,
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
    assert_eq!(sys.audience, Audience::Public);
    assert_eq!(sys.content, vec![Segment::Text { text: "hi".into() }]);
}

#[test]
fn ops_target_message_detects_message_create_and_update() {
    let msg = build_message_doc(
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        "all".into(),
        None,
        Audience::Public,
        vec![],
        0,
    );
    assert!(ops_target_message(&[Operation::Create {
        doc: msg.clone()
    }]));
    assert!(ops_target_message(&[Operation::Delete { doc: msg }]));

    let mut note = build_message_doc(
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        "all".into(),
        None,
        Audience::Public,
        vec![],
        0,
    );
    note.doc_type = "note".into();
    assert!(!ops_target_message(&[Operation::Create { doc: note }]));
}

#[test]
fn ops_target_message_detects_message_in_mixed_batch() {
    // A batch with one innocuous non-message op followed by a message
    // Create must still trip the guard: `.any()` must not short-circuit
    // on the first (non-matching) op.
    let mut note = build_message_doc(
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        "all".into(),
        None,
        Audience::Public,
        vec![],
        0,
    );
    note.doc_type = "note".into();
    let msg = build_message_doc(
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        "all".into(),
        None,
        Audience::Public,
        vec![],
        0,
    );
    assert!(ops_target_message(&[
        Operation::Create { doc: note },
        Operation::Create { doc: msg },
    ]));
}
```

The `posted_message_is_searchable_by_members` test (near the end of the file) also calls `build_message_doc` once — update its call the same way:

```rust
        let doc = build_message_doc(
            w.id,
            player,
            "all".into(),
            None,
            Audience::Public,
            plain_text_content("banshee wail"),
            1,
        );
```

(Leave the rest of `posted_message_is_searchable_by_members` unchanged — it does not call `handle_send_message`.)

- [ ] **Step 6: Run tests**

Run: `cargo test -p shadowcat --lib chat::tests`
Expected: PASS — new `Audience`/`build_message_doc` tests plus every existing chat test (now updated for the new parameter).

- [ ] **Step 7: Commit**

```bash
git add src/server/src/chat/mod.rs
git commit -m "feat(chat/m11c-2): Audience enum + audience-driven build_message_doc permissions"
```

---

### Task 4: `handle_send_message` audience + fail-closed recipient validation

**Files:**
- Modify: `src/server/src/chat/mod.rs` (`SendMessageError`, `handle_send_message` signature + body, existing call sites in `handle_send_message_publishes_and_broadcasts`)
- Test: inline `#[cfg(test)]` in `src/server/src/chat/mod.rs`

**Interfaces:**
- Consumes: `Audience` (Task 3), `Repository::member_role` (Task 2).
- Produces: `SendMessageError::UnknownRecipient` (new variant).
- Produces: `handle_send_message(room, repo, ctx, rate, channel, content, actor_owner, audience: Audience, now, budget_per_min) -> Result<Command, SendMessageError>` — **signature changed**: `audience` inserted between `actor_owner` and `now`.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests { ... }` block in `src/server/src/chat/mod.rs`:

```rust
#[tokio::test]
async fn handle_send_message_rejects_unknown_whisper_recipient() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    // A uuid that belongs to no user at all, let alone this world.
    let foreign = Uuid::from_u128(99_999);
    let err = handle_send_message(
        &room,
        &repo,
        &ctx,
        &rate,
        "whispers".into(),
        "psst".into(),
        None,
        Audience::Whisper {
            recipients: vec![foreign],
        },
        100,
        30,
    )
    .await;
    assert!(matches!(err, Err(SendMessageError::UnknownRecipient)));

    // Nothing was persisted — the seq was never consumed.
    assert!(repo.events_since(w.id, 0).await.unwrap().is_empty());
}

#[tokio::test]
async fn handle_send_message_accepts_a_whisper_to_a_real_member() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let recipient = repo
        .create_user("re", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    repo.add_member(w.id, recipient, WorldRole::Player)
        .await
        .unwrap();
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let cmd = handle_send_message(
        &room,
        &repo,
        &ctx,
        &rate,
        "whispers".into(),
        "psst".into(),
        None,
        Audience::Whisper {
            recipients: vec![recipient],
        },
        100,
        30,
    )
    .await
    .unwrap();
    assert_eq!(cmd.seq, 1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat --lib chat::tests::handle_send_message_rejects_unknown_whisper_recipient`
Expected: FAIL — `no variant \`UnknownRecipient\`` / signature mismatch (too few arguments to `handle_send_message`).

- [ ] **Step 3: Write minimal implementation**

Replace the `SendMessageError` enum (current lines 155-169) with:

```rust
/// Why `handle_send_message` refused to ingest a `SendMessage` frame.
#[derive(Debug)]
pub enum SendMessageError {
    /// Content is empty after trimming whitespace, or `channel` is empty
    /// after trimming whitespace.
    Empty,
    /// Content exceeds `MAX_MESSAGE_CHARS`, or `channel` exceeds
    /// `MAX_CHANNEL_CHARS`. Reused for both — the surface stays minimal since
    /// neither the caller nor the wire protocol distinguishes which field.
    TooLong,
    /// The user's per-minute flood budget is exhausted.
    RateLimited,
    /// An `Audience::Whisper` recipient uuid does not belong to this world.
    /// Fail-closed: the whole send is rejected, nothing is persisted.
    UnknownRecipient,
    /// The authoritative write (`Room::publish`) failed.
    Data(DataError),
}
```

Replace `handle_send_message` (current lines 171-213) with:

```rust
/// Server-authoritative message ingest: flood-limit, validate, CONSTRUCT the
/// message doc, and publish it via the authoritative path. The sole message-
/// authoring entry point (see module-level INVARIANT comment) — a client can
/// only ever reach a stored `message` doc through this function.
#[allow(clippy::too_many_arguments)]
pub async fn handle_send_message(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    rate: &PingRateLimiter,
    channel: String,
    content: String,
    actor_owner: Option<ActorOwnerRef>,
    audience: Audience,
    now: i64,
    budget_per_min: usize,
) -> Result<Command, SendMessageError> {
    if content.trim().is_empty() {
        return Err(SendMessageError::Empty);
    }
    if content.chars().count() > MAX_MESSAGE_CHARS {
        return Err(SendMessageError::TooLong);
    }
    if channel.trim().is_empty() {
        return Err(SendMessageError::Empty);
    }
    if channel.chars().count() > MAX_CHANNEL_CHARS {
        return Err(SendMessageError::TooLong);
    }
    if !rate.check(ctx.user_id, now, budget_per_min) {
        return Err(SendMessageError::RateLimited);
    }
    if let Audience::Whisper { recipients } = &audience {
        for &r in recipients {
            let is_member = repo
                .member_role(room.world_id, r)
                .await
                .map_err(SendMessageError::Data)?
                .is_some();
            if !is_member {
                return Err(SendMessageError::UnknownRecipient);
            }
        }
    }
    let doc = build_message_doc(
        room.world_id,
        ctx.user_id,
        channel,
        actor_owner,
        audience,
        plain_text_content(&content),
        now,
    );
    room.publish(repo, ctx, vec![Operation::Create { doc }], now)
        .await
        .map_err(SendMessageError::Data)
}
```

- [ ] **Step 4: Update every existing `handle_send_message` call site in `handle_send_message_publishes_and_broadcasts`**

That test (currently ~lines 356-493) calls `handle_send_message` seven times. Replace the whole test body with:

```rust
#[tokio::test]
async fn handle_send_message_publishes_and_broadcasts() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let (mut rx, _current) = room.subscribe();
    let rate = PingRateLimiter::new();

    let cmd = handle_send_message(
        &room,
        &repo,
        &ctx,
        &rate,
        "all".into(),
        "hello".into(),
        None,
        Audience::Public,
        100,
        30,
    )
    .await
    .unwrap();
    assert_eq!(cmd.seq, 1);
    let got = rx.recv().await.unwrap();
    assert_eq!(got.event_seq(), Some(1));

    // Rate limit: exhaust the budget then expect RateLimited.
    let rate2 = PingRateLimiter::new();
    for _ in 0..2 {
        let _ = handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate2,
            "all".into(),
            "x".into(),
            None,
            Audience::Public,
            100,
            2,
        )
        .await;
    }
    let err = handle_send_message(
        &room,
        &repo,
        &ctx,
        &rate2,
        "all".into(),
        "x".into(),
        None,
        Audience::Public,
        100,
        2,
    )
    .await;
    assert!(matches!(err, Err(SendMessageError::RateLimited)));

    // Empty + too-long rejected before any publish.
    assert!(matches!(
        handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            "all".into(),
            "".into(),
            None,
            Audience::Public,
            100,
            30
        )
        .await,
        Err(SendMessageError::Empty)
    ));
    let long = "a".repeat(MAX_MESSAGE_CHARS + 1);
    assert!(matches!(
        handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            "all".into(),
            long,
            None,
            Audience::Public,
            100,
            30
        )
        .await,
        Err(SendMessageError::TooLong)
    ));

    // Empty/over-long channel rejected before any publish; seq unchanged.
    let seq_before = room.subscribe().1;
    assert!(matches!(
        handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            "".into(),
            "hi".into(),
            None,
            Audience::Public,
            100,
            30
        )
        .await,
        Err(SendMessageError::Empty)
    ));
    let long_channel = "c".repeat(MAX_CHANNEL_CHARS + 1);
    assert!(matches!(
        handle_send_message(
            &room,
            &repo,
            &ctx,
            &rate,
            long_channel,
            "hi".into(),
            None,
            Audience::Public,
            100,
            30
        )
        .await,
        Err(SendMessageError::TooLong)
    ));
    assert_eq!(
        room.subscribe().1,
        seq_before,
        "rejected channel must not publish"
    );
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p shadowcat --lib chat::tests`
Expected: PASS — every test in the module, including the two new whisper-validation tests and the updated `handle_send_message_publishes_and_broadcasts`.
Run: `cargo test -p shadowcat` (full) — Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/chat/mod.rs
git commit -m "feat(chat/m11c-2): fail-closed whisper recipient validation in handle_send_message"
```

---

### Task 5: WS wire — `ClientMsg::SendMessage.audience`

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (`ClientMsg::SendMessage` variant + its round-trip test)
- Modify: `src/server/src/ws/conn.rs` (pass `audience` through)
- Test: inline `#[cfg(test)]` in `src/server/src/ws/protocol.rs`

**Interfaces:**
- Consumes: `crate::chat::Audience` (Task 3), `crate::chat::handle_send_message` (Task 4).
- Produces: `ClientMsg::SendMessage { channel, content, actor_owner, audience: Audience }` — `audience` is `#[serde(default)]` (defaults to `Audience::Public` via `Audience`'s `#[default]`), so an incoming frame that omits it (e.g. an older/naive client, or existing test JSON) still parses.

- [ ] **Step 1: Write the failing tests**

In `src/server/src/ws/protocol.rs`, replace the existing `send_message_frame_parses` test with:

```rust
#[test]
fn send_message_frame_parses() {
    let raw = r#"{"type":"send_message","channel":"all","content":"hi","actor_owner":null}"#;
    let msg: ClientMsg = serde_json::from_str(raw).unwrap();
    match msg {
        ClientMsg::SendMessage {
            channel,
            content,
            actor_owner,
            audience,
        } => {
            assert_eq!(channel, "all");
            assert_eq!(content, "hi");
            assert!(actor_owner.is_none());
            assert_eq!(
                audience,
                crate::chat::Audience::Public,
                "omitted audience defaults to Public"
            );
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn send_message_frame_parses_whisper_audience() {
    let raw = r#"{"type":"send_message","channel":"all","content":"psst","actor_owner":null,"audience":{"kind":"whisper","recipients":["00000000-0000-0000-0000-000000000001"]}}"#;
    let msg: ClientMsg = serde_json::from_str(raw).unwrap();
    match msg {
        ClientMsg::SendMessage { audience, .. } => {
            assert_eq!(
                audience,
                crate::chat::Audience::Whisper {
                    recipients: vec![Uuid::from_u128(1)]
                }
            );
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn send_message_frame_parses_gm_only_audience() {
    let raw = r#"{"type":"send_message","channel":"gm","content":"for your eyes only","actor_owner":null,"audience":{"kind":"gm_only"}}"#;
    let msg: ClientMsg = serde_json::from_str(raw).unwrap();
    match msg {
        ClientMsg::SendMessage { audience, .. } => {
            assert_eq!(audience, crate::chat::Audience::GmOnly);
        }
        _ => panic!("wrong variant"),
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat --lib ws::protocol::tests::send_message_frame_parses`
Expected: FAIL — pattern `ClientMsg::SendMessage { channel, content, actor_owner, audience }` does not match (missing field) / compile error.

- [ ] **Step 3: Write minimal implementation**

In `src/server/src/ws/protocol.rs`, update the import and the `SendMessage` variant:

```rust
use crate::chat::{ActorOwnerRef, Audience};
```

```rust
    SendMessage {
        channel: String,
        content: String,
        #[serde(default)]
        actor_owner: Option<ActorOwnerRef>,
        #[serde(default)]
        audience: Audience,
    },
```

In `src/server/src/ws/conn.rs`, update the `ClientMsg::SendMessage` dispatch arm:

```rust
Ok(ClientMsg::SendMessage {
    channel,
    content,
    actor_owner,
    audience,
}) => {
    // Server-authoritative chat ingest: flood-limit, validate, CONSTRUCT
    // the message doc, and publish. Success is confirmed by the broadcast
    // echo of the authored Event (like Intent); a `SendMessage` frame
    // carries no intent_id, so a rejection has no matching frame to
    // correlate a Reject to and is logged only.
    if let Err(e) = crate::chat::handle_send_message(
        &room,
        repo.as_ref(),
        &ctx,
        &message_rate,
        channel,
        content,
        actor_owner,
        audience,
        now_millis(),
        MESSAGE_RATE_PER_MIN,
    )
    .await
    {
        tracing::debug!(world = %world_id, user = %user_id, ?e, "message rejected");
    }
}
```

- [ ] **Step 4: Run tests + regenerate ts-rs bindings**

Run: `cargo test -p shadowcat --lib ws::protocol::tests`
Expected: PASS.
Run: `cargo test -p shadowcat` (full server suite) — Expected: PASS.
Run: `git status src/types/generated/`
Expected: `Audience.ts` created, `ClientMsg.ts` modified (the `send_message` variant now includes `audience: Audience`).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/ws/protocol.rs src/server/src/ws/conn.rs src/types/generated/
git commit -m "feat(chat/m11c-2): wire Audience onto the SendMessage ClientMsg frame"
```

---

### Task 6: Client wire mirror — `AudienceSchema` + `PermissionSetSchema.gm_role` + `SendMessageSchema`

**Files:**
- Modify: `src/client/core/src/wire.ts`
- Modify: `src/client/core/src/wire.test.ts`

**Interfaces:**
- Produces: `AudienceSchema` (Zod discriminated union), `WireAudience` type.
- Produces: `PermissionSetSchema.gm_role: DocRoleSchema.nullable()`.
- Produces: `SendMessageSchema.audience: AudienceSchema` (defaulted to `{ kind: "public" }`), `ClientMsg`'s `send_message` variant gains `audience: WireAudience`.

- [ ] **Step 1: Write the failing tests**

In `src/client/core/src/wire.test.ts`, add `WireAudience` and `AudienceSchema` to the import list, then replace the `describe("SendMessageSchema", ...)` block with:

```ts
import {
  parseServerMsg,
  DocRoleSchema,
  VisibilitySchema,
  WorldRoleSchema,
  RejectReasonSchema,
  ResyncSourceSchema,
  WsErrorCodeSchema,
  SendMessageSchema,
  AudienceSchema,
  type ServerMsg,
  type ClientMsg,
  type WireOperation,
  type WireAudience,
} from "./wire";
```

```ts
describe("wire drift guard — Audience", () => {
  it("Audience kind tags", () => {
    expectTypeOf<WireAudience["kind"]>().toEqualTypeOf<Ts.Audience["kind"]>();
  });
});

describe("SendMessageSchema", () => {
  it("parses a send_message frame + actor_owner ref", () => {
    expect(
      SendMessageSchema.parse({
        type: "send_message",
        channel: "all",
        content: "hi",
        actor_owner: {
          kind: "actor",
          actor_id: "00000000-0000-0000-0000-000000000001",
        },
      }).actor_owner?.kind,
    ).toBe("actor");
  });

  it("defaults audience to public when omitted", () => {
    const parsed = SendMessageSchema.parse({
      type: "send_message",
      channel: "all",
      content: "hi",
      actor_owner: null,
    });
    expect(parsed.audience).toEqual({ kind: "public" });
  });

  it("parses a whisper audience with recipients", () => {
    const parsed = SendMessageSchema.parse({
      type: "send_message",
      channel: "whispers",
      content: "psst",
      actor_owner: null,
      audience: {
        kind: "whisper",
        recipients: ["00000000-0000-0000-0000-000000000001"],
      },
    });
    expect(parsed.audience).toEqual({
      kind: "whisper",
      recipients: ["00000000-0000-0000-0000-000000000001"],
    });
  });

  it("parses a gm_only audience", () => {
    const parsed = SendMessageSchema.parse({
      type: "send_message",
      channel: "gm",
      content: "for your eyes only",
      actor_owner: null,
      audience: { kind: "gm_only" },
    });
    expect(parsed.audience).toEqual({ kind: "gm_only" });
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test`
Expected: FAIL — `AudienceSchema` is not exported from `./wire`.

- [ ] **Step 3: Write minimal implementation**

In `src/client/core/src/wire.ts`, add near `ActorOwnerRefSchema`:

```ts
/** Mirrors `crate::chat::Audience` (chat message readership). */
export const AudienceSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("public") }),
  z.object({ kind: z.literal("whisper"), recipients: z.array(z.string()) }),
  z.object({ kind: z.literal("gm_only") }),
]);
export type WireAudience = z.infer<typeof AudienceSchema>;
```

Update `PermissionSetSchema`:

```ts
export const PermissionSetSchema = z.object({
  default: DocRoleSchema,
  users: z.record(DocRoleSchema),
  property_overrides: z.record(VisibilitySchema),
  capabilities: CapabilityGrantsSchema,
  gm_role: DocRoleSchema.nullable(),
});
```

Update the hand-written `ClientMsg` type's `send_message` member:

```ts
  | {
      type: "send_message";
      channel: string;
      content: string;
      actor_owner: WireActorOwnerRef | null;
      audience: WireAudience;
    };
```

Update `SendMessageSchema`:

```ts
export const SendMessageSchema = z.object({
  type: z.literal("send_message"),
  channel: z.string(),
  content: z.string(),
  actor_owner: ActorOwnerRefSchema.nullable(),
  audience: AudienceSchema.default({ kind: "public" }),
});
```

- [ ] **Step 4: Run tests + typecheck**

Run: `pnpm --filter @shadowcat/core test`
Expected: PASS.
Run: `pnpm --filter @shadowcat/core exec tsc --noEmit`
Expected: PASS (vitest strips types — this is what actually typechecks).

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/wire.ts src/client/core/src/wire.test.ts
git commit -m "feat(chat/m11c-2): Audience + PermissionSet.gm_role Zod mirrors"
```

---

### Task 7: Integration proof — whisper visibility on every egress path

**Files:**
- Create: `src/server/tests/chat_audience.rs`

**Interfaces:**
- Consumes: the `/ws` WS endpoint (`send_message`, `search` frames), the `GET /api/worlds/{id}/documents?type=message` HTTP route — both already exist and are untouched by this checkpoint; this task only proves `Audience::Whisper` behaves correctly through them.

> This is the checkpoint's actual security proof (per Buddy-check directives). Harness mirrors `chat_delivery.rs`'s `spawn`/`add_member`/`connect_with`/`recv_until` pattern (duplicated per-file, matching this codebase's existing integration-test convention — see `chat_ingress.rs` and `chat_delivery.rs`, which each carry their own copy rather than sharing one).

- [ ] **Step 1: Write the harness + failing tests**

Create `src/server/tests/chat_audience.rs`:

```rust
//! Integration proof (M11c-2): a restricted-audience message (`Audience::Whisper`
//! / `Audience::GmOnly`) is invisible to anyone outside its audience on every
//! egress path — broadcast, HTTP resync/load, and full-text search — and the
//! GM's usual unconditional access is itself gated by that audience. Harness
//! mirrors `chat_delivery.rs`'s `spawn`/`add_member`/`connect_with`/`recv_until`.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use shadowcat::auth::password::hash_password;
use shadowcat::auth::role::ServerRole;
use shadowcat::config::Config;
use shadowcat::data::document::WorldRole;
use shadowcat::data::repository::Repository;
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::http::{self, AppState};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

struct Harness {
    addr: String,
    world: Uuid,
    repo: Arc<SqliteRepository>,
}

/// Spawns the harness and returns the world-owning user's id alongside it —
/// that user is GM (per `create_world_owned`), and several tests below need
/// to address the GM by uuid (e.g. to name them in a whisper's recipients).
async fn spawn() -> (Harness, Uuid) {
    let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
    let hash = hash_password("pw").unwrap();
    let uid = repo
        .create_user("u", Some(&hash), ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("test", uid, 0).await.unwrap();

    let state = AppState {
        repo: repo.clone(),
        config: Arc::new(Config::default()),
        setup_token: None,
        initialized: Arc::new(AtomicBool::new(true)),
        ws: shadowcat::ws::WsState::new(),
        upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
    };
    let app = http::router(state).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (
        Harness {
            addr,
            world: world.id,
            repo,
        },
        uid,
    )
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

impl Harness {
    async fn connect_with(&self, cookie: &str) -> Ws {
        let url = format!("ws://{}/ws?world={}", self.addr, self.world);
        let mut req = url.into_client_request().unwrap();
        req.headers_mut().insert("cookie", cookie.parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        ws
    }

    async fn login(&self, username: &str, password: &str) -> String {
        let client = reqwest::Client::builder()
            .cookie_store(true)
            .build()
            .unwrap();
        let res = client
            .post(format!("http://{}/api/login", self.addr))
            .json(&serde_json::json!({ "username": username, "password": password }))
            .send()
            .await
            .unwrap();
        res.headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    async fn add_member(&self, username: &str, role: WorldRole) -> (Uuid, String) {
        let hash = hash_password("pw").unwrap();
        let id = self
            .repo
            .create_user(username, Some(&hash), ServerRole::User, 0)
            .await
            .unwrap();
        self.repo.add_member(self.world, id, role).await.unwrap();
        (id, self.login(username, "pw").await)
    }

    /// `GET /api/worlds/{world}/documents?type=message` as `cookie` — the
    /// resync/load path a fresh connection or page load uses.
    async fn list_messages(&self, cookie: &str) -> Vec<serde_json::Value> {
        let client = reqwest::Client::new();
        let res = client
            .get(format!(
                "http://{}/api/worlds/{}/documents?type=message",
                self.addr, self.world
            ))
            .header("cookie", cookie)
            .send()
            .await
            .unwrap();
        res.json().await.unwrap()
    }
}

/// Drain frames until one of `type` arrives (skips welcome/ping/etc.).
async fn recv_until(ws: &mut Ws, ty: &str) -> serde_json::Value {
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("timeout")
            .expect("stream ended")
            .expect("ws error");
        if let Message::Text(t) = msg {
            let v: serde_json::Value = serde_json::from_str(&t).unwrap();
            if v["type"] == ty {
                return v;
            }
        }
    }
}

/// Drain frames until the next `event` whose command carries a `message`
/// Create op, returning that op's `doc`. Used as a timeout-free absence
/// proof: send a distinguishable follow-up public message after the message
/// under test, then assert THIS returns the follow-up's doc, not the one
/// under test — proving the one under test was never delivered at all.
async fn recv_next_message_create(ws: &mut Ws) -> serde_json::Value {
    loop {
        let evt = recv_until(ws, "event").await;
        let op = &evt["command"]["ops"][0];
        if op["op"] == "create" && op["doc"]["doc_type"] == "message" {
            return op["doc"].clone();
        }
    }
}

fn send_message_frame(channel: &str, content: &str, audience: serde_json::Value) -> Message {
    Message::Text(
        serde_json::json!({
            "type": "send_message",
            "channel": channel,
            "content": content,
            "actor_owner": null,
            "audience": audience,
        })
        .to_string(),
    )
}

fn whisper_audience(recipients: &[Uuid]) -> serde_json::Value {
    serde_json::json!({ "kind": "whisper", "recipients": recipients })
}

/// A whisper reaches its named recipient but never a third, unnamed member —
/// proven by having the bystander's next observed message-create be a
/// distinguishable follow-up public message, not the whisper.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whisper_reaches_only_the_named_recipient() {
    let (h, _gm_id) = spawn().await;
    let (_sender_id, cookie_sender) = h.add_member("sender", WorldRole::Player).await;
    let (recipient_id, cookie_recipient) = h.add_member("recipient", WorldRole::Player).await;
    let (_bystander_id, cookie_bystander) = h.add_member("bystander", WorldRole::Player).await;

    let mut ws_sender = h.connect_with(&cookie_sender).await;
    let mut ws_recipient = h.connect_with(&cookie_recipient).await;
    let mut ws_bystander = h.connect_with(&cookie_bystander).await;
    recv_until(&mut ws_sender, "welcome").await;
    recv_until(&mut ws_recipient, "welcome").await;
    recv_until(&mut ws_bystander, "welcome").await;

    ws_sender
        .send(send_message_frame(
            "whispers",
            "secret",
            whisper_audience(&[recipient_id]),
        ))
        .await
        .unwrap();
    let recipient_doc = recv_next_message_create(&mut ws_recipient).await;
    assert_eq!(recipient_doc["system"]["content"][0]["text"], "secret");

    ws_sender
        .send(send_message_frame("all", "marker", serde_json::json!({ "kind": "public" })))
        .await
        .unwrap();
    let bystander_doc = recv_next_message_create(&mut ws_bystander).await;
    assert_eq!(
        bystander_doc["system"]["content"][0]["text"], "marker",
        "the bystander's first observed message-create is the PUBLIC marker, \
         proving the whisper was never delivered to them at all"
    );
}

/// The GM does NOT see a whisper that doesn't name them — the same
/// absence-proof pattern as above, applied to the world's GM.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whisper_excludes_the_gm_unless_named() {
    let (h, _gm_id) = spawn().await;
    let gm_cookie = h.login("u", "pw").await;
    let (_a_id, cookie_a) = h.add_member("a", WorldRole::Player).await;
    let (b_id, _cookie_b) = h.add_member("b", WorldRole::Player).await;

    let mut ws_gm = h.connect_with(&gm_cookie).await;
    let mut ws_a = h.connect_with(&cookie_a).await;
    recv_until(&mut ws_gm, "welcome").await;
    recv_until(&mut ws_a, "welcome").await;

    ws_a.send(send_message_frame(
        "whispers",
        "player-to-player secret",
        whisper_audience(&[b_id]),
    ))
    .await
    .unwrap();

    ws_a.send(send_message_frame("all", "marker", serde_json::json!({ "kind": "public" })))
        .await
        .unwrap();
    let gm_doc = recv_next_message_create(&mut ws_gm).await;
    assert_eq!(
        gm_doc["system"]["content"][0]["text"], "marker",
        "the GM's first observed message-create is the PUBLIC marker — the \
         unnamed whisper never reached them"
    );
}

/// A whisper that DOES name the GM reaches them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whisper_reaches_the_gm_when_named() {
    let (h, gm_id) = spawn().await;
    let gm_cookie = h.login("u", "pw").await;
    let (_a_id, cookie_a) = h.add_member("a", WorldRole::Player).await;

    let mut ws_gm = h.connect_with(&gm_cookie).await;
    let mut ws_a = h.connect_with(&cookie_a).await;
    recv_until(&mut ws_gm, "welcome").await;
    recv_until(&mut ws_a, "welcome").await;

    ws_a.send(send_message_frame(
        "whispers",
        "for the GM's eyes too",
        whisper_audience(&[gm_id]),
    ))
    .await
    .unwrap();

    let gm_doc = recv_next_message_create(&mut ws_gm).await;
    assert_eq!(gm_doc["system"]["content"][0]["text"], "for the GM's eyes too");
}

/// A whisper naming a uuid that is not a world member is rejected wholesale
/// — fail-closed — and nothing is persisted (proven by a subsequent public
/// message landing at seq 1, not seq 2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whisper_to_unknown_recipient_is_rejected_and_nothing_persists() {
    let (h, _gm_id) = spawn().await;
    let (_sender_id, cookie_sender) = h.add_member("sender", WorldRole::Player).await;

    let mut ws_sender = h.connect_with(&cookie_sender).await;
    recv_until(&mut ws_sender, "welcome").await;

    let foreign = Uuid::from_u128(999_999);
    ws_sender
        .send(send_message_frame(
            "whispers",
            "should never persist",
            whisper_audience(&[foreign]),
        ))
        .await
        .unwrap();

    ws_sender
        .send(send_message_frame("all", "marker", serde_json::json!({ "kind": "public" })))
        .await
        .unwrap();
    let doc = recv_next_message_create(&mut ws_sender).await;
    assert_eq!(doc["system"]["content"][0]["text"], "marker");

    let seqs = h.repo.events_since(h.world, 0).await.unwrap();
    assert_eq!(
        seqs.len(),
        1,
        "only the marker was persisted — the rejected whisper consumed no seq"
    );
}

/// Search: the whisper's own recipient finds it; a non-recipient's search
/// for the same unique term returns nothing, even though the FTS index
/// contains the text (the per-hit READ-gate filter, not the index split,
/// enforces this — see design doc §1).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whisper_content_is_hidden_from_a_non_recipient_search() {
    let (h, _gm_id) = spawn().await;
    let (_sender_id, cookie_sender) = h.add_member("sender", WorldRole::Player).await;
    let (recipient_id, cookie_recipient) = h.add_member("recipient", WorldRole::Player).await;
    let (_bystander_id, cookie_bystander) = h.add_member("bystander", WorldRole::Player).await;

    let mut ws_sender = h.connect_with(&cookie_sender).await;
    let mut ws_recipient = h.connect_with(&cookie_recipient).await;
    let mut ws_bystander = h.connect_with(&cookie_bystander).await;
    recv_until(&mut ws_sender, "welcome").await;
    recv_until(&mut ws_recipient, "welcome").await;
    recv_until(&mut ws_bystander, "welcome").await;

    ws_sender
        .send(send_message_frame(
            "whispers",
            "xylophone galaxy secret",
            whisper_audience(&[recipient_id]),
        ))
        .await
        .unwrap();
    recv_next_message_create(&mut ws_recipient).await; // wait for delivery before searching

    let search = |ws: &mut Ws, request_id: Uuid| {
        Message::Text(
            serde_json::json!({
                "type": "search",
                "request_id": request_id,
                "query": "xylophone",
                "limit": 10,
                "cursor": null,
            })
            .to_string(),
        )
    };

    let req_recipient = Uuid::from_u128(1);
    ws_recipient
        .send(search(&mut ws_recipient, req_recipient))
        .await
        .unwrap();
    let result = recv_until(&mut ws_recipient, "search_result").await;
    assert_eq!(
        result["hits"].as_array().unwrap().len(),
        1,
        "the recipient finds their own whisper"
    );

    let req_bystander = Uuid::from_u128(2);
    ws_bystander
        .send(search(&mut ws_bystander, req_bystander))
        .await
        .unwrap();
    let result = recv_until(&mut ws_bystander, "search_result").await;
    assert_eq!(
        result["hits"].as_array().unwrap().len(),
        0,
        "a non-recipient's search must not surface the whisper"
    );
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p shadowcat --test chat_audience`
Expected: PASS — all 6 tests in this file. If a field name or route shape doesn't match, diff against `chat_delivery.rs` / `http/routes.rs` (this plan was written against the current source, but re-verify before trusting a mismatch is a real bug rather than drift).

- [ ] **Step 3: Commit**

```bash
git add src/server/tests/chat_audience.rs
git commit -m "test(chat/m11c-2): prove whisper visibility on broadcast, resync, and search"
```

---

### Task 8: Integration proof — GM-only channel + dynamic promotion/demotion

**Files:**
- Modify: `src/server/tests/chat_audience.rs` (append tests; reuses the harness from Task 7)

- [ ] **Step 1: Write the failing tests**

Append to `src/server/tests/chat_audience.rs`:

```rust
/// A GM-only message is visible to the GM and invisible to a regular member
/// — same absence-proof pattern as the whisper tests.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gm_only_channel_visible_to_gm_hidden_from_regular_member() {
    let (h, _gm_id) = spawn().await;
    let gm_cookie = h.login("u", "pw").await;
    let (_a_id, cookie_a) = h.add_member("a", WorldRole::Player).await;
    let (_b_id, cookie_b) = h.add_member("b", WorldRole::Player).await;

    let mut ws_gm = h.connect_with(&gm_cookie).await;
    let mut ws_a = h.connect_with(&cookie_a).await;
    let mut ws_b = h.connect_with(&cookie_b).await;
    recv_until(&mut ws_gm, "welcome").await;
    recv_until(&mut ws_a, "welcome").await;
    recv_until(&mut ws_b, "welcome").await;

    // Any regular member may post into the GM-only channel.
    ws_a.send(send_message_frame(
        "gm",
        "for the GM's eyes only",
        serde_json::json!({ "kind": "gm_only" }),
    ))
    .await
    .unwrap();
    let gm_doc = recv_next_message_create(&mut ws_gm).await;
    assert_eq!(gm_doc["system"]["content"][0]["text"], "for the GM's eyes only");

    ws_a.send(send_message_frame("all", "marker", serde_json::json!({ "kind": "public" })))
        .await
        .unwrap();
    let b_doc = recv_next_message_create(&mut ws_b).await;
    assert_eq!(
        b_doc["system"]["content"][0]["text"], "marker",
        "a regular member's first observed message-create is the PUBLIC \
         marker — the GM-only message never reached them"
    );
}

/// A user promoted to GM AFTER a GM-only message was sent immediately sees
/// it on their next resync/load — proving dynamic (not frozen-roster)
/// resolution. Uses the HTTP `GET .../documents?type=message` resync path,
/// since this is about backlog visibility, not a live broadcast.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gm_only_channel_promotion_immediately_grants_backlog_access() {
    let (h, _gm_id) = spawn().await;
    let (co_gm_id, co_gm_cookie) = h.add_member("cogm", WorldRole::Player).await;

    let mut ws_co = h.connect_with(&co_gm_cookie).await;
    recv_until(&mut ws_co, "welcome").await;
    ws_co
        .send(send_message_frame(
            "gm",
            "posted before promotion",
            serde_json::json!({ "kind": "gm_only" }),
        ))
        .await
        .unwrap();
    // Wait for the send to land (the poster is not the GM here, so this
    // message is invisible even to them on the broadcast path — but it is
    // durably persisted; drain past it via the authoritative log instead of
    // a WS frame).
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if !h.repo.events_since(h.world, 0).await.unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    // Before promotion: the poster (still a Player) does not see it in resync.
    let before = h.list_messages(&co_gm_cookie).await;
    assert!(
        before.is_empty(),
        "a Player, even the GM-only message's own sender, does not read it \
         back via resync before being promoted (gm_role gates by CURRENT \
         WorldRole, not by having posted it)"
    );

    h.repo
        .set_role(h.world, co_gm_id, WorldRole::Gm)
        .await
        .unwrap();

    let after = h.list_messages(&co_gm_cookie).await;
    assert_eq!(
        after.len(),
        1,
        "immediately after promotion, the same cookie's next resync sees the \
         GM-only backlog — dynamic resolution, not a frozen roster"
    );
    assert_eq!(
        after[0]["system"]["content"][0]["text"], "posted before promotion"
    );
}

/// A co-GM demoted back to Player immediately loses resync access to prior
/// GM-only messages.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gm_only_channel_demotion_immediately_revokes_backlog_access() {
    let (h, _gm_id) = spawn().await;
    let (co_gm_id, co_gm_cookie) = h.add_member("cogm", WorldRole::Gm).await;

    let mut ws_co = h.connect_with(&co_gm_cookie).await;
    recv_until(&mut ws_co, "welcome").await;
    ws_co
        .send(send_message_frame(
            "gm",
            "gm staff note",
            serde_json::json!({ "kind": "gm_only" }),
        ))
        .await
        .unwrap();
    recv_next_message_create(&mut ws_co).await; // the poster is GM, so they see the live echo

    let before = h.list_messages(&co_gm_cookie).await;
    assert_eq!(before.len(), 1, "the co-GM reads it back while still GM");

    h.repo
        .set_role(h.world, co_gm_id, WorldRole::Player)
        .await
        .unwrap();

    let after = h.list_messages(&co_gm_cookie).await;
    assert!(
        after.is_empty(),
        "immediately after demotion, the same cookie's next resync no longer \
         sees the GM-only backlog"
    );
}

/// Search: the GM finds a GM-only message; a regular member's search for the
/// same unique term returns nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gm_only_channel_content_is_hidden_from_a_regular_members_search() {
    let (h, _gm_id) = spawn().await;
    let gm_cookie = h.login("u", "pw").await;
    let (_a_id, cookie_a) = h.add_member("a", WorldRole::Player).await;

    let mut ws_gm = h.connect_with(&gm_cookie).await;
    let mut ws_a = h.connect_with(&cookie_a).await;
    recv_until(&mut ws_gm, "welcome").await;
    recv_until(&mut ws_a, "welcome").await;

    ws_a.send(send_message_frame(
        "gm",
        "marmoset quokka signal",
        serde_json::json!({ "kind": "gm_only" }),
    ))
    .await
    .unwrap();
    recv_next_message_create(&mut ws_gm).await; // wait for delivery before searching

    let search = |request_id: Uuid| {
        Message::Text(
            serde_json::json!({
                "type": "search",
                "request_id": request_id,
                "query": "marmoset",
                "limit": 10,
                "cursor": null,
            })
            .to_string(),
        )
    };

    ws_gm.send(search(Uuid::from_u128(10))).await.unwrap();
    let result = recv_until(&mut ws_gm, "search_result").await;
    assert_eq!(result["hits"].as_array().unwrap().len(), 1, "the GM finds it");

    ws_a.send(search(Uuid::from_u128(11))).await.unwrap();
    let result = recv_until(&mut ws_a, "search_result").await;
    assert_eq!(
        result["hits"].as_array().unwrap().len(),
        0,
        "a regular member's search must not surface the GM-only message"
    );
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p shadowcat --test chat_audience`
Expected: PASS — all 10 tests in `chat_audience.rs` (6 from Task 7 + 4 from this task).
Run: `cargo test -p shadowcat` (full server suite) — Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/server/tests/chat_audience.rs
git commit -m "test(chat/m11c-2): prove GM-only channel visibility and dynamic promotion/demotion"
```

---

### Task 9: `shadowcat-codebase-chat` + `shadowcat-codebase-documents-permissions` skill updates (reviewed skill-update gate)

**Files:**
- Modify: `.claude/skills/shadowcat-codebase-chat/SKILL.md`
- Modify: `.claude/skills/shadowcat-codebase-documents-permissions/SKILL.md`

> **Corrected during execution:** this note originally assumed `.claude/` is largely git-ignored (copied from the c-1 plan's text). It is not — only `.claude/settings*.json` and `.claude/skills/graphify/` are ignored; `shadowcat-codebase-*` skill files are tracked and were committed for c-1 too. Commit both skill updates normally after the gate passes. Per CLAUDE.md's reviewed skill-update gate, dispatch `shadowcat-spec-reviewer` on both skill diffs to confirm they accurately capture the c-2 surface before this checkpoint is considered done.

- [ ] **Step 1: Update `shadowcat-codebase-chat`.** Add/replace content so the skill captures: the `Audience` enum (`Public`/`Whisper{recipients}`/`GmOnly`) and its exact `PermissionSet` mapping table (from the design doc §2); that `channel` remains purely a client label with zero server-enforced meaning — `Audience` is the only server-enforced visibility concept; that `MessageSystem.audience` is stored verbatim; that `handle_send_message` fail-closed-validates `Whisper` recipients via `Repository::member_role` before constructing anything; and a pointer to the new `PermissionSet.gm_role` field this checkpoint added (owned by `documents-permissions`, but load-bearing here). Update the module-level "M11c-2 (whisper allowlist) is next" framing to "DONE" and describe what actually shipped, mirroring how `shadowcat-codebase-chat` already documents the c-1 surface.

- [ ] **Step 2: Update `shadowcat-codebase-documents-permissions`.** Add a Hard Invariant entry (or extend the existing `can_see`/`resolve_access` one) documenting: `PermissionSet.gm_role: Option<DocRole>` makes the GM's usual unconditional `resolve_access` short-circuit conditional per-document; `None` (every pre-existing doc type) is unchanged; `Some(role)` caps the GM to the same per-document role-floor everyone else uses, via the shared `effective_role` helper — and that `resolve_access_world` deliberately reuses the SAME `effective_role` (not `doc.permissions.default`) so world-level grants layer consistently for a `gm_role`-capped GM too. Cross-reference `shadowcat-codebase-chat` as the first (and so far only) consumer.

- [ ] **Step 3: Reviewed skill-update gate.** Dispatch `shadowcat-spec-reviewer` on both skill diffs; fix any inaccuracy it finds; record the PASS. Commit both files (see corrected note above).

---

### Task 10: Whole-checkpoint verification + docs sync

**Files:**
- Modify: `docs/PLAN.md` (M11c-2 status)

- [ ] **Step 1: Full verification.**
Run: `cargo test -p shadowcat` (all server tests) — Expected: PASS.
Run: `cargo clippy --all-targets -- -D warnings` — Expected: clean except any pre-existing unrelated warning already tracked in `docs/TODO.md` (do not fix it here). If a NEW warning appears in touched code, fix it.
Run: `git diff --exit-code src/types/generated` — Expected: no diff (all regenerated bindings committed in Tasks 5-6).
Run: `pnpm --filter @shadowcat/core test && pnpm -r exec tsc --noEmit` — Expected: PASS. **Corrected during execution:** the original text here scoped typecheck to `@shadowcat/core` only, but `PermissionSetSchema.gm_role` becoming a required field (Task 6) breaks any OTHER package that constructs a `WireDocument`/`permissions` literal — it broke `@shadowcat/render`'s test fixtures too (fixed as a Task 6 follow-up). Verification must cover the whole monorepo (`pnpm -r exec tsc --noEmit`), not just `@shadowcat/core`.

- [ ] **Step 2: Update `docs/PLAN.md`.** Record M11c-2 DONE under the M11c section: `PermissionSet.gm_role` (the single new permission primitive), the `Audience` enum and its exact mapping, GM excluded from an unnamed whisper / included dynamically for a GM-only channel, fail-closed recipient validation, and that M11c-3 (sanitizer + commands + edit path) is next.

- [ ] **Step 3: Commit.**

```bash
git add docs/PLAN.md
git commit -m "docs(chat/m11c-2): record M11c-2 complete in PLAN.md"
```

---

## Self-Review

**Spec coverage (design doc §1-§6):**
- `PermissionSet.gm_role` primitive + `effective_role`-based `resolve_access`/`resolve_access_world` → Task 1. ✓
- `Repository::member_role` (needed for fail-closed recipient validation, not explicitly named in the design doc but required by §3's "Recipients must resolve to real world members") → Task 2. ✓
- `Audience` enum + `MessageSystem.audience` + the exact `PermissionSet` mapping table (§2) → Task 3. ✓
- Fail-closed unknown-recipient rejection + self-recipient owner-downgrade guard (§3) → Task 3 (guard, in `build_message_doc`) + Task 4 (validation, in `handle_send_message`). ✓
- `SendMessage` wire frame + Zod mirror (§4) → Tasks 5-6. ✓
- Testing strategy (§5): non-recipient excluded on every egress path, GM excluded/included correctly, dynamic promotion/demotion, malformed recipient rejection, `Audience` ts-rs↔Zod parity → Tasks 6-8. ✓
- Reviewed skill-update gate (§6) → Task 9. ✓
- Mandatory buddy-check (§5, parent spec risk rating) → Buddy-check directives + Task 1/3 escalation note. ✓

**Placeholder scan:** every code step shows concrete, complete code. The one deliberately-imperfect scaffold (Task 7 Step 1's redundant `gm_id` lookup) is explicitly called out and cleaned up in Task 7 Step 3, not left as a TODO.

**Type consistency:** `Audience`, `MessageSystem.audience`, `build_message_doc`'s new `audience: Audience` parameter (between `actor_owner` and `content`), `handle_send_message`'s new `audience: Audience` parameter (between `actor_owner` and `now`), `SendMessageError::UnknownRecipient`, `Repository::member_role`, `PermissionSet.gm_role`, `effective_role` are used identically across every task that references them. `AudienceSchema`/`WireAudience`/`PermissionSetSchema.gm_role` on the client side match the server shapes field-for-field.

**Open adaptation risks flagged for the implementer:** re-verify exact line numbers in `document.rs`/`permission.rs`/`repository.rs`/`sqlite.rs`/`chat/mod.rs`/`protocol.rs` against the live source before editing (this plan was written against the current M11c-1-complete source, but earlier tasks in THIS plan shift line numbers for later tasks in the same file — e.g. Task 3's edits shift line numbers Task 4 then reads against). The `chat_audience.rs` harness assumes `AppState`/`http::router`/`Config::default()` shapes identical to `chat_delivery.rs` — diff against that file if compilation fails on harness setup.
