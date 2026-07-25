# Phase C — Egress Ownership Unification Implementation Plan

> **For agentic workers:** On a Fable-class session, REQUIRED SUB-SKILL: `mainline-plan-execution`
> (per project CLAUDE.md — inline compliance check per task, ONE dispatched final review). On a
> non-Fable session, use superpowers:subagent-driven-development with the `shadowcat-coder` /
> two-reviewer agents instead. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every egress path resolves token ownership through the same `effective_owner` rule the
write path enforces (C1), which structurally grants the owner-floor `cap::READ` at egress and
closes the write-but-never-receive asymmetry (C2).

**Architecture:** Make the effective owner an explicit parameter of access resolution (remove the
literal-`doc.owner` convenience wrappers), hoist `filter_command`'s per-op loads so its core is
synchronous, and join the linked actor per egress site from the cheapest correct source: the
room's existing in-memory `SceneEcs.actors` side-table on the WS broadcast hot path, a batched
prefetch on `list_documents`, and one pool read (`load_effective_owner` /
`Repository::effective_owner_of`) on the single-doc routes and search. No new cache structure, no
DB migration, no wire change.

**Tech Stack:** Rust (tokio/axum/sqlx), server crate only + one 3-line Svelte-adjacent TS change
in `@shadowcat/ui-kit`.

**Campaign context:** Phase C of the Phase-1 close-out campaign
(`docs/superpowers/specs/2026-07-24-phase1-closeout-campaign-design.md` §Phase C). Closes
`docs/TODO.md` § "Actionable now — inherited owner is a stranger at egress".

## Model/Effort directives

Execution mode (user-directed 2026-07-25): **subagent-driven-development**, with this Fable 5
session as the mainline controller of the dispatch loop. Per the project CLAUDE.md agent-dispatch
rules: implementer = `shadowcat-coder` (sonnet, effort medium; `-opus` twin on BLOCKED); per-task
review = `shadowcat-code-reviewer` (effort high) running the SDD task-reviewer contract (spec
compliance + quality verdicts); final whole-branch review = the two-reviewer pair below. The
sdd-* ladder applies only if a non-Fable session picks this plan up.

## Buddy-check directives

Phase C is **security-sensitive** (permissions/egress). The campaign spec mandates the
**two-reviewer pair** after execution: `shadowcat-spec-reviewer` + `shadowcat-code-reviewer`
(effort: high), whole-branch, before merge. This is the recorded buddy-check outcome — no
additional per-task review. Reviewer briefs MUST include: the no-destructive-git line, an explicit
delivery channel ("return findings as the Agent tool result"), and this plan's path.

## Design decisions (spec interpretation — best long-term shape)

1. **No materialized token→owner map.** The spec's C1 names "an in-memory resolved-owner cache
   (token → linked-actor owner) maintained on document mutation alongside the room's existing
   side-tables". The room's existing `SceneEcs.actors` side-table (hydrated at room creation,
   maintained by `apply_op`) IS that cache in un-materialized form: `effective_owner(doc,
   actors.get(link))` answers token → linked-actor owner from memory with zero pool queries. A
   second, stamped map would be a second ownership representation requiring invalidation on actor
   re-own / link change / delete — exactly the never-fork defect class, and the codebase already
   rejects stamping at three documented sites ("nothing is stamped"). C1's actual requirements —
   in-memory, no per-recipient pool query, maintained on document mutation — are all met by the
   live join.
2. **Owner-explicit access API.** `resolve_access`/`resolve_access_world` (literal-owner
   convenience wrappers) are REMOVED; the `_with_owner` variants are renamed to the primary names.
   Every call site states its owner source explicitly. This is the structural never-fork fix: a
   future egress site cannot silently fall back to literal `doc.owner`.
3. **Scope check moves INTO `permission::effective_owner`.** The `actor.scope == doc.scope`
   filter currently lives only in `load_effective_owner` (sqlite.rs). A cross-scope candidate is
   the same class of illegitimate join as a wrong-id/wrong-type candidate, which
   `effective_owner` already rejects. Moving it in gives every source (ECS map, batched prefetch,
   pool read) the check for free and deletes the duplicate.
4. **Client `TemplatesController.#isOwnerOrGm` migrates to core `effectiveOwner`.** Discovered
   during plan research: `templatesController.svelte.ts:45` gates template controls on literal
   `doc.owner === selfId` while the client already ships the canonical mirror
   (`@shadowcat/core` `effectiveOwner`). Post-C the server delivers `/base` to inheriting owners;
   leaving the UI gate literal forks the ownership rule client-side. Included per the campaign
   no-deferral directive (small, same seam).
5. **Search index partitioning is NOT touched** (explicit non-goal). The public/full FTS split is
   recipient-independent; C only fixes the per-hit READ gate. An inheriting owner's `OwnerOrGm`
   content remains unsearchable exactly as a stamped literal owner's does today (two partitions
   only) — parity, not a gap.

## Global Constraints

- **Never fork a decision across two paths** — ownership resolution goes through
  `permission::effective_owner` at every site; sources differ only in where the actor doc comes
  from (ARCHITECTURE §2; core skill's never-fork invariant).
- **Fail closed** — degenerate input (dangling link, cross-scope actor, missing doc) resolves to
  no owner / no delivery, never default-allow.
- **No per-recipient pool query added to the WS egress hot path** (spec C1). The pre-existing
  per-recipient `get_document` for Update ops is retained unchanged (count-neutral hoist); the
  actor join adds zero pool reads there.
- **No lock across await** — the scene read guard is acquired only after all loads complete, and
  only around the synchronous `filter_command` core.
- **No wire/schema change** — `Access` is not a wire type; no ts-rs regen, no migration, no client
  Zod change. (Task 7 is UI logic only.)
- **Immutable history; commit per task once local checks pass; push only at milestone end.**
- Per-commit gate (server tasks): `cargo fmt` + `cargo clippy --all-targets` + `cargo test`
  (from `src/server/`). Client task gate: `pnpm -r test` + `pnpm -r typecheck` (repo root; a
  client change can break sibling packages' fixtures).
- `dist/` must exist before any `cargo` build (`rust-embed` compile-time check): run `pnpm build`
  once in the worktree before the first cargo command.
- Deletion only via `trash` with RELATIVE paths (trash-cli no-ops silently on absolute Windows
  paths).

## Execution protocol (worktree)

1. `EnterWorktree` (branches from origin/main = 5db39cf).
2. COPY this plan into the worktree (`docs/superpowers/plans/…`) and commit it there first; then
   `trash` the main-tree copy using a RELATIVE path and verify with `Test-Path`.
3. Branch name: `phase-c-egress-ownership`.
4. `pnpm install` if node_modules absent; `pnpm build`; then `cargo test` baseline green before
   Task 1.

---

### Task 1: Scope check inside `effective_owner`

**Files:**
- Modify: `src/server/src/data/permission.rs:61-71` (`effective_owner`)
- Modify: `src/server/src/data/sqlite.rs:1182-1207` (`load_effective_owner` — drop duplicate filter)
- Test: `src/server/src/data/permission.rs` (existing `effective_owner` test block, ~line 2087)

**Interfaces:**
- Produces: `effective_owner(doc, linked_actor)` now also rejects a scope-mismatched actor
  candidate (returns `None` → fail-closed). Signature unchanged.
- Consumers unchanged: `token_effective_owner` (scene/mod.rs), `load_effective_owner` (sqlite.rs).

- [ ] **Step 1: Write the failing test** (in the `effective_owner` test block beside
  `effective_owner_fails_closed_on_degenerate_links`):

```rust
#[test]
fn effective_owner_rejects_a_cross_scope_actor() {
    // A candidate from another scope is an illegitimate join, same class as a
    // wrong-id or wrong-type candidate: fail closed to no owner.
    let actor_id = Uuid::from_u128(42);
    let mut token = token_linked_to(Some(actor_id));
    token.scope = Scope::World { world_id: Uuid::from_u128(1000) };
    let mut foreign = actor_owned_by(actor_id, Some(Uuid::from_u128(1)));
    foreign.scope = Scope::World { world_id: Uuid::from_u128(2000) };
    assert_eq!(effective_owner(&token, Some(&foreign)), None);

    // Same scope still resolves.
    let mut same = actor_owned_by(actor_id, Some(Uuid::from_u128(1)));
    same.scope = token.scope.clone();
    assert_eq!(effective_owner(&token, Some(&same)), Some(Uuid::from_u128(1)));
}
```

(If `Scope` is not already in the test module's imports, add it; it derives `PartialEq`/`Clone` —
verify at `data/document.rs` and adjust `.clone()` to a rebuild if it doesn't derive `Clone`.)

- [ ] **Step 2: Run to verify it fails**
  — `cargo test effective_owner_rejects_a_cross_scope_actor` → FAIL (returns `Some(1)`).

- [ ] **Step 3: Implement** — in `effective_owner`, extend the identity re-check:

```rust
    if actor.id != link || actor.doc_type != "actor" || actor.scope != doc.scope {
        return None;
    }
```

Update the function's doc comment: add scope mismatch to the fail-closed list ("…a `linked_actor`
that is not the document `token_actor_link` names **or lives in a different scope**, and an
unowned actor all resolve to `None`").

- [ ] **Step 4: Remove the now-duplicate filter in `load_effective_owner`** — delete
  `let actor = actor.filter(|a| a.scope == doc.scope);` (sqlite.rs:1202) and rewrite the comment
  above it: the world-scoping rationale (cross-world `actor_id` must not resolve; keeps the
  reachable set equal to `SceneEcs.actors` by construction) now lives with the check inside
  `permission::effective_owner` — leave a one-line pointer, not a copy.

- [ ] **Step 5: Full gate + commit**
  — `cargo fmt && cargo clippy --all-targets && cargo test` → green (existing cross-world tests
  in sqlite.rs must still pass — they now exercise the moved check).

```bash
git add src/server/src/data/permission.rs src/server/src/data/sqlite.rs
git commit -m "refactor(server): move the actor scope check into effective_owner"
```

---

### Task 2: Owner-explicit access API (remove literal-owner wrappers)

**Files:**
- Modify: `src/server/src/data/permission.rs:346-431` (delete wrappers, rename `_with_owner`)
- Modify: every caller (compiler-driven): `src/server/src/data/sqlite.rs` (write path renames +
  search site at :2441), `src/server/src/http/routes.rs:885,925`, `src/server/src/ws/conn.rs:535`,
  `src/server/src/scene/mod.rs:1397`, `filter_command`'s three internal sites, all tests.
- Modify: `src/server/src/data/document.rs:312` (doc-comment reference, wording only).

**Interfaces:**
- Produces (later tasks consume these EXACT signatures):

```rust
pub fn resolve_access(user: Uuid, world_role: WorldRole, doc: &Document,
                      effective_owner: Option<Uuid>) -> Access
pub fn resolve_access_world(user: Uuid, world_role: WorldRole, doc: &Document,
                            world_grants: &CapabilityGrants,
                            effective_owner: Option<Uuid>) -> Access
```

- Behavior-NEUTRAL commit: not-yet-migrated egress sites pass their current literal value
  explicitly; two sites that can never link (scene ping, region field) get their FINAL form now.

- [ ] **Step 1: Rename.** Delete the 3/4-arg `resolve_access` and `resolve_access_world`
  wrappers; rename `resolve_access_with_owner` → `resolve_access` and
  `resolve_access_world_with_owner` → `resolve_access_world`. Merge the wrapper doc comments into
  the survivors: state that the caller MUST resolve the owner (`effective_owner`) from its
  source — ECS actor table (ws egress), batched prefetch (list route), `effective_owner_of` /
  `load_effective_owner` (single-doc routes, search, write path) — and that passing literal
  `doc.owner` is correct ONLY for doc types that can never carry an actor link.

- [ ] **Step 2: Fix callers, compiler-driven.** Exact per-site values:
  - `sqlite.rs` write path (4 sites, :1735/:1831/:1921/:2000): rename only (already pass a
    loaded owner).
  - `sqlite.rs:2441` (search per-hit): `resolve_access_world(ctx.user_id, ctx.world_role, &doc,
    &world_defaults.grants_for(&doc.doc_type), doc.owner)` — literal, explicit (migrated in
    Task 5).
  - `routes.rs:885` (`list_documents`): `…, d.owner)` — literal (Task 5 migrates).
  - `routes.rs:925` (`get_document`): `…, doc.owner)` — literal (Task 5 migrates).
  - `permission.rs` `filter_command` internal Create/Delete/Update sites: `doc.owner` /
    `cur.owner` — literal (Task 4 migrates).
  - `ws/conn.rs:535` (`scene_ping_permitted`): FINAL form
    `…, crate::data::permission::effective_owner(&doc, None))` + one-line comment: a scene doc
    never carries an actor link, so the no-join resolution is exact (and fails closed if a
    non-scene doc ever reached here).
  - `scene/mod.rs:1397` (`region_field`): FINAL form
    `crate::data::permission::resolve_access(user, WorldRole::Player, doc,
    crate::data::permission::effective_owner(doc, None))` + same-style comment (region docs never
    link).
  - Tests: mechanical — pass `d.owner` (or the test's known owner) as the fourth/fifth arg.

- [ ] **Step 3: Full gate + commit** — `cargo fmt && cargo clippy --all-targets && cargo test`.

```bash
git add -A src/server
git commit -m "refactor(server): make the effective owner an explicit access-resolution parameter"
```

---

### Task 3: `Repository::effective_owner_of`

**Files:**
- Modify: `src/server/src/data/repository.rs` (trait), `src/server/src/data/sqlite.rs` (impl)
- Test: `src/server/src/data/sqlite.rs` (beside the `effective_owner` write-path tests, ~:7790)

**Interfaces:**
- Produces: `async fn effective_owner_of(&self, doc: &Document) -> Result<Option<Uuid>, DataError>`
  on the `Repository` trait (declared in the trait's existing async style — match neighbors).
- Consumes: `load_effective_owner` (Task 1's form).

- [ ] **Step 1: Write the failing test.** Reuse the fixtures of
  `an_effective_owner_cannot_reassign_or_widen_ownership` (sqlite.rs:8063) — copy its
  world/actor/token construction helpers verbatim (actors are an ENGINE doc type: creation via
  `apply_intent` needs a valid `ActorEngine` body — that existing test already has one; do not
  invent a new fixture shape):

```rust
#[tokio::test]
async fn effective_owner_of_joins_the_linked_actor_on_the_pool() {
    // setup: world w, player p (create_user), actor owned by p, token linked to
    // the actor with token.owner == None — copied from
    // an_effective_owner_cannot_reassign_or_widen_ownership's arrangement.
    // …
    let token = r.get_document(token_id).await.unwrap().unwrap();
    assert_eq!(r.effective_owner_of(&token).await.unwrap(), Some(p));

    // Dangling link fails closed.
    let mut dangling = token.clone();
    dangling.engine = Some(serde_json::json!({
        "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
        "actor_id": Uuid::from_u128(999999).to_string()
    }));
    assert_eq!(r.effective_owner_of(&dangling).await.unwrap(), None);

    // A non-token resolves to its literal owner without any join.
    let actor = r.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(r.effective_owner_of(&actor).await.unwrap(), Some(p));
}
```

- [ ] **Step 2: Run to verify it fails** — no such method (compile error is the failure).

- [ ] **Step 3: Implement.** Trait doc comment + SQLite impl:

```rust
/// Resolve `doc`'s effective owner against LIVE actor state — the same
/// `permission::effective_owner` rule the write path enforces, joining the
/// linked actor with one pool read when `doc` is a linked token. For egress
/// read routes and search; the ws broadcast hot path joins through the room's
/// in-memory actor table instead (zero pool reads per recipient).
async fn effective_owner_of(&self, doc: &Document) -> Result<Option<Uuid>, DataError>;
```

```rust
async fn effective_owner_of(&self, doc: &Document) -> Result<Option<Uuid>, DataError> {
    Self::load_effective_owner(&self.pool, doc).await
}
```

- [ ] **Step 4: Full gate + commit**

```bash
git add src/server/src/data/repository.rs src/server/src/data/sqlite.rs
git commit -m "feat(server): expose effective_owner_of on the Repository seam"
```

---

### Task 4: `filter_command` restructure + WS/`write_ops` migration (the C1 hot path)

**Files:**
- Modify: `src/server/src/data/permission.rs:555-652` (`filter_command` → sync core;
  new `load_update_docs`, new `effective_owner_via`)
- Modify: `src/server/src/scene/mod.rs:1472-1476` (`token_effective_owner` → delegate to
  `effective_owner_via`)
- Modify: `src/server/src/ws/conn.rs` (`send_filtered` gains `room`; `egress_loop`/`replay`
  call sites)
- Modify: `src/server/src/http/routes.rs:424-457` (`write_ops` filter block)
- Test: `src/server/src/data/permission.rs` (convert ~7 `filter_command` tests; add C1/C2 tests)

**Interfaces:**
- Produces (Tasks 5-6 and all callers rely on these EXACT shapes):

```rust
/// effective_owner joined through a caller-supplied in-memory actor source.
pub fn effective_owner_via<'a>(
    doc: &Document,
    actor_lookup: &impl Fn(&Uuid) -> Option<&'a Document>,
) -> Option<Uuid>

/// Current documents for every Update op in `cmd` (absent = deleted → op dropped).
pub async fn load_update_docs(
    repo: &dyn Repository, cmd: &Command,
) -> std::collections::HashMap<Uuid, Document>

pub fn filter_command<'a>(
    cmd: &Command,
    ctx: &PermissionContext,
    world_defaults: &WorldCapDefaults,
    current: &std::collections::HashMap<Uuid, Document>,
    actor_lookup: impl Fn(&Uuid) -> Option<&'a Document>,
) -> Command
```

- Consumes: Task 2's `resolve_access_world(user, role, doc, grants, owner)`.

- [ ] **Step 1: Write the failing C1/C2 test** (permission.rs test module; sync core makes this
  a plain `#[tokio::test]` only for the repo setup):

```rust
#[tokio::test]
async fn filter_command_admits_the_inheriting_owner_of_a_linked_token() {
    // token: permissions.default = None, owner = None, linked to an actor owned
    // by P. Literal-owner egress treated P as a stranger (op dropped); the
    // effective join must now deliver Create/Update/Delete AND OwnerOrGm-tier
    // content to P, while a true stranger still receives nothing. This is C2:
    // a document P can write (owner floor at apply_intent) is one P receives.
    let p = Uuid::from_u128(1);
    let stranger = Uuid::from_u128(2);
    let actor_id = Uuid::from_u128(42);
    let actor = actor_owned_by(actor_id, Some(p));
    let mut token = token_linked_to(Some(actor_id));
    token.permissions.default = DocRole::None;
    token
        .permissions
        .property_overrides
        .insert("/system/notes".into(), Visibility::OwnerOrGm);

    let cmd = Command {
        seq: 1,
        world_id: Uuid::from_u128(7),
        author: Uuid::from_u128(9),
        ts: 0,
        ops: vec![Operation::Create { doc: token.clone() }],
    };
    let lookup = |id: &Uuid| (id == &actor.id).then_some(&actor);
    let current = std::collections::HashMap::new();

    let p_ctx = PermissionContext { user_id: p, world_role: WorldRole::Player };
    let out = filter_command(&cmd, &p_ctx, &WorldCapDefaults::default(), &current, lookup);
    assert_eq!(out.ops.len(), 1, "inheriting owner must RECEIVE the create (C2)");

    let s_ctx = PermissionContext { user_id: stranger, world_role: WorldRole::Player };
    let out = filter_command(&cmd, &s_ctx, &WorldCapDefaults::default(), &current, lookup);
    assert!(out.ops.is_empty(), "a stranger still receives nothing (fail closed)");

    // Without the actor join (dangling source) the op is withheld even from P:
    // degenerate input under-permits, never over-permits.
    let out = filter_command(&cmd, &p_ctx, &WorldCapDefaults::default(), &current, |_| None);
    assert!(out.ops.is_empty());
}
```

Add a sibling test `filter_command_update_keeps_owner_or_gm_changes_for_the_inheriting_owner`:
same doc arrangement persisted via `apply_intent` (reuse the harness of
`filter_command_update_drops_base_field_change_for_non_owner_non_gm`, permission.rs:1131 — real
repo, real world, actor + linked token created through the write path; NOTE actors/tokens are
engine doc types — copy the valid engine bodies from the sqlite.rs effective-owner tests), with an
Update op changing `/system/notes` and a `/base` FieldChange: P keeps both changes, the stranger's
op is dropped entirely (no READ).

- [ ] **Step 2: Run to verify both fail** (signature doesn't exist yet → compile failure).

- [ ] **Step 3: Implement the core.**
  1. Add `effective_owner_via` + `load_update_docs` as specified in Interfaces (bodies below):

```rust
pub fn effective_owner_via<'a>(
    doc: &Document,
    actor_lookup: &impl Fn(&Uuid) -> Option<&'a Document>,
) -> Option<Uuid> {
    let linked = token_actor_link(doc).and_then(|l| actor_lookup(&l));
    effective_owner(doc, linked)
}

pub async fn load_update_docs(
    repo: &dyn Repository,
    cmd: &Command,
) -> std::collections::HashMap<Uuid, Document> {
    let mut out = std::collections::HashMap::new();
    for op in &cmd.ops {
        if let Operation::Update { doc_id, .. } = op {
            if !out.contains_key(doc_id) {
                if let Ok(Some(d)) = repo.get_document(*doc_id).await {
                    out.insert(*doc_id, d);
                }
            }
        }
    }
    out
}
```

  2. Rewrite `filter_command` per the Interfaces signature: drop the `repo` param and all
     `.await`s; the Update branch becomes `let Some(cur) = current.get(doc_id) else { continue; }`
     (absent = deleted → drop, preserving today's semantics); each branch resolves
     `let owner = effective_owner_via(doc_or_cur, &actor_lookup);` and passes `owner` to
     `resolve_access_world`. Keep the seq-preserving comment; update the header comment: loads are
     hoisted to `load_update_docs` (still once per op per recipient — count-neutral), and the
     actor join is in-memory via `actor_lookup` — the C1 no-pool-query-on-hot-path property.
  3. Rewire `SceneEcs::token_effective_owner` to the shared join:

```rust
pub fn token_effective_owner(&self, token: &Document) -> Option<Uuid> {
    crate::data::permission::effective_owner_via(token, &|id| self.actors.get(id))
}
```

- [ ] **Step 4: Migrate the call sites.**
  - `conn.rs send_filtered`: add `room: &Room` parameter (after `repo`); Event branch:

```rust
ServerMsg::Event { command, intent_id } => {
    // Loads complete BEFORE the guard: no lock across await. The guard is held
    // only around the synchronous core — the same short-read-guard discipline
    // as clip_move_stream.
    let current = crate::data::permission::load_update_docs(repo, command).await;
    let filtered = {
        let ecs = room.scene().read().await;
        crate::data::permission::filter_command(
            command, ctx, world_defaults, &current, |id| ecs.actor(id),
        )
    };
    ServerMsg::Event { command: filtered, intent_id: *intent_id }
}
```

  - Thread `&room` through both `egress_loop` call sites (:1166, :1286-ish `other =>` arm at
    :1346) and through `replay` (:1487) — `replay` already receives `&room`.
  - `routes.rs write_ops` (:454-456): same shape — room is already in scope:

```rust
let world_defaults = state.repo.world_cap_defaults(world).await?;
let current =
    crate::data::permission::load_update_docs(state.repo.as_ref(), &cmd).await;
let filtered = {
    let ecs = room.scene().read().await;
    crate::data::permission::filter_command(&cmd, &ctx, &world_defaults, &current, |id| {
        ecs.actor(id)
    })
};
Ok(Json(filtered))
```

  - Convert the existing `filter_command` tests: `let current = load_update_docs(&r, &cmd).await;`
    then the sync call with `|_| None` (they use non-token docs; behavior identical).

- [ ] **Step 5: Full gate + commit** — `cargo fmt && cargo clippy --all-targets && cargo test`.
  Verify by eye (compliance check): no `.await` inside any scene-guard scope introduced here.

```bash
git add -A src/server
git commit -m "feat(server): resolve egress ownership through the room actor table (C1 hot path)"
```

---

### Task 5: HTTP read routes + search egress

**Files:**
- Modify: `src/server/src/http/routes.rs:868-935` (`list_documents`, `get_document`)
- Modify: `src/server/src/data/sqlite.rs` search per-hit gate (~:2438-2454)
- Test: `src/server/src/http/mod.rs` (route tests) + `src/server/src/data/sqlite.rs` (search test)

**Interfaces:**
- Consumes: Task 3's `effective_owner_of`, Task 4's `effective_owner_via`.

- [ ] **Step 1: Write the failing route test** (http/mod.rs test module; harness:
  `initialized_state`/`seed_user`/`login_server`/`doc_json` — see
  `get_document_strips_gm_only_for_player` at http/mod.rs:2097). Build token/actor JSON by
  mutating `doc_json`'s output; FIRST read `data/engine/token.rs` + the actor engine struct to
  confirm the minimal valid bodies (deny_unknown_fields):

```rust
#[tokio::test]
async fn read_routes_admit_the_inheriting_owner_of_a_default_none_token() {
    let state = initialized_state().await;
    seed_user(&state, "gm").await;
    let p_id = seed_user(&state, "pl").await;
    let s_id = seed_user(&state, "st").await;
    let gm = login_server(&state, "gm").await;
    let pl = login_server(&state, "pl").await;
    let st = login_server(&state, "st").await;
    // world + both players seated (copy the member-POST lines from
    // get_document_strips_gm_only_for_player).
    // actor doc: doc_type "actor", owner = p_id, valid ActorEngine body.
    // token doc: doc_type "token", owner null, permissions.default "none",
    // engine.actor_id = actor id, property_overrides {"/system/notes":"owner_or_gm"},
    // system {"notes":"secret","label":"pub"}.
    // (both created by the GM via POST /api/worlds/{w}/documents)

    // C2 at the routes: the inheriting owner READS it…
    let got: serde_json::Value = pl.get(&format!("/api/documents/{token_id}")).await.json();
    assert_eq!(got["system"]["notes"], "secret", "OwnerOrGm tier visible to inheriting owner");
    // …the stranger gets a uniform 404 (existence hiding preserved)…
    st.get(&format!("/api/documents/{token_id}"))
        .await
        .assert_status(StatusCode::NOT_FOUND);
    // …and the list route agrees.
    let listed: serde_json::Value =
        pl.get(&format!("/api/worlds/{world_id}/documents?type=token")).await.json();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let listed_s: serde_json::Value =
        st.get(&format!("/api/worlds/{world_id}/documents?type=token")).await.json();
    assert!(listed_s.as_array().unwrap().is_empty());
}
```

- [ ] **Step 2: Run to verify it fails** — the player currently 404s (literal owner ⇒ no READ).

- [ ] **Step 3: Implement.**
  - `get_document`: before access resolution, `let owner = state.repo.effective_owner_of(&doc)
    .await?;` then pass `owner`.
  - `list_documents`: one batched prefetch, join in memory:

```rust
// One batched actor fetch when listing tokens; the per-doc join is then
// in-memory (token_actor_link returns None for every other doc_type, so the
// map is simply unused there).
let actors: std::collections::HashMap<Uuid, Document> = if q.r#type == "token" {
    state
        .repo
        .query_documents(world, "actor")
        .await?
        .into_iter()
        .map(|a| (a.id, a))
        .collect()
} else {
    std::collections::HashMap::new()
};
let visible = docs
    .into_iter()
    .filter_map(|d| {
        let owner = crate::data::permission::effective_owner_via(&d, &|id| actors.get(id));
        let access = resolve_access_world(ctx.user_id, ctx.world_role, &d, &world_grants, owner);
        access.has(cap::READ).then(|| filter_properties(&d, &access))
    })
    .collect();
```

  - search per-hit gate (sqlite.rs): after the candidate `get_document`,
    `let owner = Self::load_effective_owner(&self.pool, &doc).await?;` then pass `owner`. Extend
    the loop's comment: one extra pool read per linked-token candidate, bounded by `MAX_SCAN`;
    the ws hot path never enters here.

- [ ] **Step 4: Add the search test** (sqlite.rs, beside the existing search tests): default-none
  linked token whose PUBLIC content matches a query — the inheriting owner's search returns the
  hit, the stranger's returns none.

- [ ] **Step 5: Full gate + commit**

```bash
git add -A src/server
git commit -m "feat(server): effective-owner egress on read routes and search (C2)"
```

---

### Task 6: Write-receive parity + adversarial egress tests

**Files:**
- Test: `src/server/src/data/permission.rs` (or sqlite.rs where the harness fits best)

**Interfaces:** consumes everything above; produces only tests.

- [ ] **Step 1: Parity test** — the C2 asymmetry pinned shut end-to-end at the seam level:

```rust
#[tokio::test]
async fn a_document_you_can_write_is_a_document_you_receive() {
    // default-none linked token, actor owned by P (real repo, engine-valid docs —
    // reuse Task 4 Step 1's persisted arrangement). P patches /system/notes via
    // apply_intent (owner floor grants WRITE_FIELDS) — must SUCCEED — and the
    // resulting command filtered for P must RETAIN the op (owner floor grants
    // READ at egress through the same owner value). Before C this pair was
    // write-ok / receive-dropped.
    // 1. apply_intent as P: assert Ok.
    // 2. filter_command of the returned command for P with the actor join:
    //    assert ops.len() == 1.
    // 3. filter_command for a stranger: assert ops.is_empty().
}
```

- [ ] **Step 2: Adversarial egress cases** (unit tests beside Task 4's, all through
  `filter_command`/`effective_owner_via` with in-memory fixtures):
  - `egress_ownership_ignores_a_cross_scope_actor` — actor map contains the linked id but from a
    different scope → op withheld from the would-be inheritor (Task 1's check, exercised through
    the egress join).
  - `egress_ownership_honors_the_per_token_override` — token.owner = A, linked actor owned by B:
    A receives, B does not (override wins, same precedence as the write path).
  - `egress_gm_and_gm_role_cap_are_unchanged` — a GM still receives everything on a plain doc;
    on a `gm_role: Some(DocRole::None)` doc (message-style) the capped GM's op is still dropped —
    the owner plumb must not have widened the gm_role cap.

- [ ] **Step 3: Full gate + commit**

```bash
git add src/server/src
git commit -m "test(server): pin write-receive parity and adversarial egress ownership"
```

---

### Task 7: Client — `TemplatesController` uses the canonical `effectiveOwner` mirror

**Files:**
- Modify: `src/client/ui-kit/src/templatesController.svelte.ts:44-46`
- Test: `src/client/ui-kit/src/templatesController.svelte.test.ts`

**Interfaces:**
- Consumes: `effectiveOwner(doc, store)` from `@shadowcat/core` (already exported,
  `src/client/core/src/actor.ts:77`); `#deps.documents: ReadableDocuments` (already in deps).

- [ ] **Step 1: Write the failing test** (follow the existing construction pattern at
  templatesController.svelte.test.ts:87 — doc helper + canEdit stub):

```ts
it("treats the inheriting owner of a linked token as owner for template controls", () => {
  // token instance: owner null, engine.actor_id -> actor owned by self.
  // Literal doc.owner gate hid pull; the effectiveOwner mirror must show it.
  // Build: actor doc (owner "u-self"), token doc (owner null,
  // engine: { actor_id: actor.id }, source: { id: template.id }), template doc.
  // Store contains all three; deps: role "player", selfId "u-self",
  // canEdit: () => true.
  expect(ctrl.canPull(token.id)).toBe(true);
});
```

- [ ] **Step 2: Run to verify it fails** — `pnpm --filter @shadowcat/ui-kit test` → FAIL.

- [ ] **Step 3: Implement**:

```ts
import { effectiveOwner } from "@shadowcat/core"; // merge into the existing @shadowcat/core import

#isOwnerOrGm(doc: WireDocument): boolean {
  // Ownership is EFFECTIVE (core effectiveOwner: per-doc override, else the
  // linked actor's owner) — the same rule the server now enforces at egress;
  // a literal doc.owner read here forks it.
  return this.#deps.role === "gm" || effectiveOwner(doc, this.#deps.documents) === this.#deps.selfId;
}
```

- [ ] **Step 4: Full client gate + commit** — `pnpm -r test && pnpm -r typecheck` (repo root;
  vitest alone skips type errors).

```bash
git add src/client/ui-kit/src/templatesController.svelte.ts src/client/ui-kit/src/templatesController.svelte.test.ts
git commit -m "fix(client): template controls resolve ownership via effectiveOwner"
```

---

### Task 8: Documentation, TODO closure, skills sync

**Files:**
- Modify: `docs/TODO.md` (delete lines 100-119, the "inherited owner is a stranger at egress"
  section — verified resolved by Tasks 4-6)
- Modify: `.claude/skills/shadowcat-codebase-documents-permissions/SKILL.md`
- Modify: `.claude/skills/shadowcat-codebase-realtime-sync/SKILL.md`
- Run: `graphify update .`

**Steps:**

- [ ] **Step 1: TODO.md** — remove the whole section (both paragraphs, incl. the
  "write-but-never-receive" consequence — that asymmetry is now test-pinned closed).

- [ ] **Step 2: documents-permissions skill.** Rewrite the "**The EGRESS half … KNOWN
  under-permit**" portion of the token-ownership bullet to the new reality:
  - egress ownership = write ownership, through `permission::effective_owner`, with the owner an
    EXPLICIT parameter of `resolve_access`/`resolve_access_world` (the literal-owner wrappers are
    gone — a new egress site must state its owner source);
  - the three join sources and when each is correct (ECS actor table on the ws hot path /
    batched prefetch on `list_documents` / `effective_owner_of`+`load_effective_owner` on
    single-doc routes and search);
  - scope check lives INSIDE `effective_owner`;
  - `filter_command` is now a sync core over `load_update_docs` + `actor_lookup` (loads hoisted,
    count-neutral; no pool read for the actor join on the hot path).
  Remove the TODO.md pointer sentence.

- [ ] **Step 3: realtime-sync skill.** In the `send_filtered` / egress description, note the
  Event branch's shape: loads → short scene read guard → sync `filter_command` with the room's
  actor table (no lock across await; same discipline as `clip_move_stream`), and that
  `send_filtered` now takes the room.

- [ ] **Step 4: Reviewed skill-update gate** — dispatch `shadowcat-spec-reviewer`
  (effort: high) on the two skill diffs (delivery channel: Agent tool result; no-destructive-git
  line in the brief). Fix any findings before commit.

- [ ] **Step 5: `graphify update .`; commit.**

```bash
git add docs/TODO.md .claude/skills graphify-out
git commit -m "docs(skills): egress ownership unified — TODO closure + permissions/realtime skill sync"
```

---

## Non-goals (explicit, with rationale)

- **Search index partitioning** — recipient-independent two-partition design untouched; see
  Design decision 5.
- **Removing the pre-existing per-recipient `get_document` in Update filtering** — flagged as
  pool-contended, but a shared-per-event filter cache is real machinery beyond C's scope; the
  hoist is deliberately count-neutral. (Already covered by the existing hot-path comment; not a
  new deferral, no TODO entry required.)
- **Client UI affordances beyond Task 7** — Phase E owns client authoring/UX work.
- **`region_field`'s hardcoded `WorldRole::Player`** — pre-existing, correct (GM callers pass
  `viewer: None`), unrelated to ownership.

## Self-review record

- Spec C1: covered by Tasks 2 (API), 4 (ws hot path + write_ops), 5 (routes + search); every
  egress site enumerated during plan research is migrated or given its final no-join form
  (scene ping, region field) — no site resolves from literal `doc.owner` implicitly anymore.
- Spec C2: structural via the owner floor (`effective_role`'s `owner_floor`); pinned by Task 4
  Step 1 (receive), Task 5 (routes/search READ), Task 6 (write-receive parity).
- Types cross-checked: `resolve_access_world(user, role, doc, grants, owner)` used identically in
  Tasks 2/4/5; `effective_owner_via(&doc, &lookup)` reference-taking form consistent between
  Task 4's definition and Task 5's `list_documents` usage; `filter_command`'s five-param sync
  form consistent across conn.rs / routes.rs / tests.
- Known verify-at-execution points (flagged inline): minimal valid `TokenEngine`/`ActorEngine`
  fixture bodies (read `data/engine/` first); `Scope`'s `Clone` derive; the trait's async
  declaration style in repository.rs.

## Completion checklist (after Task 8)

- `cargo fmt` clean, `cargo clippy --all-targets` clean, `cargo test` green (src/server),
  `pnpm -r test` + `pnpm -r typecheck` green (root).
- Two-reviewer pair (spec + code, effort high) on the whole branch → findings resolved or
  explicitly accepted with rationale.
- Merge `--no-ff` to main; three-OS CI matrix green; push; memory campaign-state update
  (Phase C done → Phase D next).
