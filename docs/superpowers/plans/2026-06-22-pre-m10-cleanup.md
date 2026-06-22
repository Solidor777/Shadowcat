# Pre-M10 Cleanup Implementation Plan

> **For agentic workers:** This plan is executed via the user's
> **`mainline-plan-execution`** skill (Fable-class working rule) — run inline in
> this session with a per-task inline spec-compliance check and ONE dispatched
> fresh-context branch review at the end. Per-task review dispatch is NOT used.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the 11 fixable bugs/follow-ups triaged from `POST_WORK_FINDINGS.md`
and `TODO.md` before M10 — uniform authz surfaces, embedded-doc security/size
hardening, last-GM protection, per-user rate limits, a deterministic lag test, a
two-axis capability model (GM-configured WorldRole defaults + per-document DocRole
overrides) with a `core:create` gate, see-as-by-username, and convergent offline
intent replay.

**Architecture:** Mostly localized hardening. The one structural change is the
world capability store: the bare `CapabilityGrants` under `world_caps:{world}`
becomes a `WorldCapDefaults { all, by_type, role_caps }` — per-document additive
grants gain `doc_type` scoping (`all ∪ by_type[t]`) and a new `WorldRole`-keyed
`role_caps` carries world-level capabilities (`core:create`). Server-authoritative
only; the client neither stores nor enforces world grants today, so the wire shape
of Welcome's `world_default_grants` is unchanged (the projected `all` portion).

**Tech Stack:** Rust (axum, sqlx/SQLite, tokio broadcast), Svelte 5 runes +
TypeScript client, Vitest + Playwright.

Reference spec: `docs/superpowers/specs/2026-06-22-pre-m10-cleanup-design.md`.

## Global Constraints

- **No migration / no compat shims:** no users or deployments exist. Replace data
  shapes directly; do NOT write backward-compat deserializers.
- **Cross-platform:** `std::path` for any path; no OS-specific code. CI matrix
  (ubuntu/macos/windows) is the proof.
- **Server is authoritative** for every capability/visibility decision; client
  capability functions are advisory and stay unused here.
- **Capability tokens** are `<namespace>:<verb>`; the server enforces possession,
  never interprets meaning.
- **Rust test command:** lib unit tests `cargo test -p shadowcat --lib`;
  integration `cargo test -p shadowcat --test ws_convergence` (run from repo root).
- **Client test command:** `pnpm --filter @shadowcat/core test` (Vitest),
  `pnpm --filter @shadowcat/ui test` (Vitest+jsdom), `pnpm --filter @shadowcat/ui build`
  before any cargo build that embeds the SPA, Playwright e2e `pnpm --filter @shadowcat/ui e2e`.
- **Comment rules:** present-tense, invariant-leading, no history/process meta.
- **Buddy-check** Tasks 3 (#3 embedded redaction) and 12 (#11 offline replay)
  before the final merge (see Buddy-check directives).

---

### Task 1: #1 — by-id document routes return 404 to non-members

**Files:**
- Modify: `src/server/src/http/routes.rs` (`get_document` ~417, `patch_document` ~445, `delete_document` ~469)
- Test: `src/server/src/http/routes.rs` (`#[cfg(test)]` module) or the existing http test module

**Context.** All three by-id routes resolve the doc's world then build a
`permission_context`, which returns `DataError::Forbidden` → 403 for a non-member —
distinguishable from the 404 of a nonexistent id. The in-world-unreadable case
already 404s. World-scoped routes (`list_documents`, `list_members`, asset routes)
must KEEP returning 403.

- [ ] **Step 1: Write the failing test** — a non-member gets 404 on GET by-id.

```rust
// in routes.rs tests (mirror existing http-layer test setup: create two users,
// a world owned by user A, a doc in it; user B is a non-member).
#[tokio::test]
async fn by_id_get_hides_existence_from_non_member() {
    let (state, world, doc_id, _owner) = seed_world_with_doc().await; // existing/new helper
    let stranger = state.repo.create_user("stranger", None, ServerRole::User, 0).await.unwrap();
    let err = get_document(
        AuthUser { id: stranger, role: ServerRole::User },
        State(state.clone()),
        Path(doc_id),
    ).await.unwrap_err();
    assert!(matches!(err, AppError::NotFound), "non-member must get 404, not 403");
    let _ = world;
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat --lib by_id_get_hides_existence_from_non_member`
Expected: FAIL — currently returns `AppError::Forbidden`.

- [ ] **Step 3: Map the non-member Forbidden to NotFound on the three by-id routes.**

In `get_document`, change the `permission_context` call:

```rust
    let ctx = state
        .repo
        .permission_context(world, user.id, user.role)
        .await
        .map_err(by_id_not_found)?;
```

For `patch_document` and `delete_document`, the membership check happens inside
`write_ops`. Resolve membership explicitly first and map, before delegating:

```rust
    // by-id routes hide existence: a non-member is 404, not 403 (uniform with a
    // nonexistent id). World-scoped routes keep 403.
    state.repo.permission_context(world, user.id, user.role)
        .await.map_err(by_id_not_found)?;
    write_ops(&state, &user, world, vec![/* Update | Delete as today */]).await
```

Add the helper near the route module:

```rust
/// On by-id document routes, a non-member's `Forbidden` is remapped to `NotFound`
/// so 403-vs-404 cannot confirm a document id exists. Other errors pass through.
fn by_id_not_found(e: crate::data::DataError) -> AppError {
    match e {
        crate::data::DataError::Forbidden => AppError::NotFound,
        other => other.into(),
    }
}
```

- [ ] **Step 4: Add PATCH/DELETE non-member 404 tests** (same shape as Step 1, calling `patch_document` with a `PatchRequest { changes: vec![] }` and `delete_document`), plus a guard test that a world-scoped route still 403s:

```rust
#[tokio::test]
async fn world_scoped_route_still_forbids_non_member() {
    let (state, world, _doc, _owner) = seed_world_with_doc().await;
    let stranger = state.repo.create_user("s2", None, ServerRole::User, 0).await.unwrap();
    let err = list_documents(
        AuthUser { id: stranger, role: ServerRole::User },
        State(state), Path(world), Query(DocQuery { r#type: "actor".into() }),
    ).await.unwrap_err();
    assert!(matches!(err, AppError::Forbidden), "world-scoped route keeps 403");
}
```

- [ ] **Step 5: Run all four tests**

Run: `cargo test -p shadowcat --lib by_id_ world_scoped_route_still_forbids_non_member`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/http/routes.rs
git commit -m "fix(authz): by-id document routes return 404 to non-members"
```

---

### Task 2: #2 — `validate_system_size` recurses into embedded children

**Files:**
- Modify: `src/server/src/data/validation.rs:8`
- Test: `src/server/src/data/validation.rs` tests

**Context.** `validate_system_size` measures only `doc.system`; embedded children
(stored inline) bypass the 256 KiB cap. Each body must be ≤ `MAX_SYSTEM_BYTES`.

- [ ] **Step 1: Write the failing test** — an oversized embedded child is rejected.

```rust
#[test]
fn oversized_embedded_child_is_rejected() {
    let mut parent = doc_with_system(serde_json::json!({ "hp": 1 }));
    let mut child = doc_with_system(serde_json::json!({ "blob": "x".repeat(MAX_SYSTEM_BYTES + 1) }));
    child.id = Uuid::from_u128(2);
    parent.embedded.insert(child.id, child);
    assert!(matches!(validate_system_size(&parent), Err(DataError::TooLarge(_))));
}

#[test]
fn small_embedded_tree_passes() {
    let mut parent = doc_with_system(serde_json::json!({ "hp": 1 }));
    let mut child = doc_with_system(serde_json::json!({ "k": 1 }));
    child.id = Uuid::from_u128(2);
    parent.embedded.insert(child.id, child);
    assert!(validate_system_size(&parent).is_ok());
}
```

(`Document.embedded` is a `BTreeMap<Uuid, Document>` — confirm the key type in
`document.rs` and adjust the `insert` accordingly.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat --lib oversized_embedded_child_is_rejected`
Expected: FAIL — only the parent body is measured.

- [ ] **Step 3: Recurse the per-body cap.**

```rust
/// Reject a document (and every embedded descendant) whose opaque `system` body
/// exceeds the size cap. Embedded children are stored inline in the parent JSON,
/// so each child's body is bounded independently.
pub fn validate_system_size(doc: &Document) -> Result<(), DataError> {
    let bytes = serde_json::to_vec(&doc.system)?.len();
    if bytes > MAX_SYSTEM_BYTES {
        return Err(DataError::TooLarge(bytes));
    }
    for child in doc.embedded.values() {
        validate_system_size(child)?;
    }
    Ok(())
}
```

(The embedded tree depth is bounded by the same self-FK/visited-set discipline
enforced at create; a doc cannot embed itself. Recursion mirrors `embedded`'s
finite stored depth.)

- [ ] **Step 4: Add a nested-grandchild rejection test**, then run

```rust
#[test]
fn oversized_grandchild_is_rejected() {
    let mut parent = doc_with_system(serde_json::json!({}));
    let mut child = doc_with_system(serde_json::json!({}));  child.id = Uuid::from_u128(2);
    let mut gc = doc_with_system(serde_json::json!({ "blob": "x".repeat(MAX_SYSTEM_BYTES + 1) }));
    gc.id = Uuid::from_u128(3);
    child.embedded.insert(gc.id, gc);
    parent.embedded.insert(child.id, child);
    assert!(matches!(validate_system_size(&parent), Err(DataError::TooLarge(_))));
}
```

Run: `cargo test -p shadowcat --lib system_size embedded_child embedded_tree grandchild`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/data/validation.rs
git commit -m "fix(validation): cap embedded children's system body size"
```

---

### Task 3: #3 — `filter_properties` recurses redaction into embedded children (BUDDY-CHECK)

**Files:**
- Modify: `src/server/src/data/permission.rs` (`filter_properties` ~201)
- Test: `src/server/src/data/permission.rs` tests

**Context.** `filter_properties` strips only the parent's `property_overrides`. An
embedded child's own `GmOnly` overrides leak to players. Covers Create/Delete
egress + REST because both call `filter_properties`.

**Buddy-check focus (see directives):** the related `filter_command` `Update` arm
redacts `/embedded/...` writes against the PARENT's gm-only set, not the child's.
This task fixes `filter_properties`; the reviewer decides whether the Update-arm
extension is in this pass.

- [ ] **Step 1: Write the failing test** — a player's view of a parent omits an
  embedded child's GM-only property.

```rust
#[test]
fn embedded_child_gm_only_is_stripped_for_non_gm() {
    let mut parent = doc(PermissionSet { default: DocRole::Observer, ..Default::default() },
                         serde_json::json!({ "public": 1 }));
    let mut child = doc(PermissionSet::default(), serde_json::json!({ "secret": 9, "shown": 2 }));
    child.id = Uuid::from_u128(2);
    child.permissions.property_overrides.insert("/system/secret".into(), Visibility::GmOnly);
    parent.embedded.insert(child.id, child);

    let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &parent);
    let view = filter_properties(&parent, &player);
    let child_view = view.embedded.get(&Uuid::from_u128(2)).unwrap();
    assert_eq!(child_view.system.get("secret"), None, "child gm-only stripped");
    assert_eq!(child_view.system["shown"], serde_json::json!(2));

    let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &parent);
    let gm_view = filter_properties(&parent, &gm);
    assert_eq!(gm_view.embedded.get(&Uuid::from_u128(2)).unwrap().system["secret"], serde_json::json!(9));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat --lib embedded_child_gm_only_is_stripped_for_non_gm`
Expected: FAIL — the child's secret survives.

- [ ] **Step 3: Recurse `filter_properties` into embedded children.**

Replace the body so each child is filtered with the SAME `access` (the
`see_gm_only` flag is the recipient's, applied at every depth), then the parent's
own overrides are stripped:

```rust
pub fn filter_properties(doc: &Document, access: &Access) -> Document {
    let mut out = doc.clone();
    if access.see_gm_only {
        return out;
    }
    // Recurse first: each embedded child carries its own `property_overrides`,
    // independent of the parent's. A non-GM recipient must not see any GmOnly
    // field at any depth.
    out.embedded = out
        .embedded
        .into_iter()
        .map(|(id, child)| (id, filter_properties(&child, access)))
        .collect();
    let gm_only: Vec<String> = doc
        .permissions
        .property_overrides
        .iter()
        .filter(|(_, v)| **v == Visibility::GmOnly)
        .map(|(p, _)| p.clone())
        .collect();
    let mut whole = serde_json::to_value(&out).expect("document serializes");
    for pointer in gm_only {
        strip_pointer(&mut whole, &pointer);
    }
    serde_json::from_value(whole).expect("filtered document deserializes")
}
```

(`out.embedded.into_iter()` requires owning `out`; it does. Confirm `embedded`'s
value type is `Document` and key `Uuid`.)

- [ ] **Step 4: Add a Create-broadcast end-to-end redaction test** through
  `filter_command` (mirrors `filter_command_strips_and_preserves_seq`): create a
  parent with a GM-only embedded child, build a `Create` command, filter for a
  player, assert the child's secret is absent from the broadcast op.

- [ ] **Step 5: Run**

Run: `cargo test -p shadowcat --lib embedded_child filter_command`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/data/permission.rs
git commit -m "fix(security): redact embedded children's GmOnly properties"
```

---

### Task 4: #4 — reject removing/demoting the last GM

**Files:**
- Modify: `src/server/src/data/sqlite.rs` (`set_role` ~223, `remove_member` ~242)
- Test: `src/server/src/data/sqlite.rs` tests (or the membership test module)

**Context.** Reuse `DataError::Conflict(String)` → `AppError::Conflict` → 409. A
server admin remains GM everywhere via `permission_context`, so the world is never
permanently orphaned; the guard only blocks the accidental self-lockout.

- [ ] **Step 1: Write failing tests.**

```rust
#[tokio::test]
async fn cannot_remove_sole_gm() {
    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap(); // owner joins as GM
    let err = r.remove_member(w.id, gm).await.unwrap_err();
    assert!(matches!(err, DataError::Conflict(_)));
}

#[tokio::test]
async fn cannot_demote_sole_gm() {
    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let err = r.set_role(w.id, gm, WorldRole::Player).await.unwrap_err();
    assert!(matches!(err, DataError::Conflict(_)));
}

#[tokio::test]
async fn can_remove_gm_when_another_exists() {
    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm1 = r.create_user("gm1", None, ServerRole::User, 0).await.unwrap();
    let gm2 = r.create_user("gm2", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm1, 0).await.unwrap();
    r.add_member(w.id, gm2, WorldRole::Gm).await.unwrap();
    assert!(r.remove_member(w.id, gm1).await.is_ok());
}
```

(Verify `create_world_owned` joins the owner as GM; if not, `add_member(.., Gm)`
the GM first.)

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p shadowcat --lib sole_gm gm_when_another_exists`
Expected: FAIL — both mutations currently succeed.

- [ ] **Step 3: Add a GM-count guard.** Add a private helper and call it before
  the mutating query in each function:

```rust
/// Count GMs in a world. The last GM may not be removed or demoted (availability
/// guard; admin-recovery via server role remains the escape hatch).
async fn gm_count(&self, world: Uuid) -> Result<i64, DataError> {
    let role = serde_json::to_value(WorldRole::Gm)?.as_str().unwrap().to_string();
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM world_members WHERE world_id = ? AND role = ?",
    )
    .bind(world.to_string())
    .bind(role)
    .fetch_one(&self.pool)
    .await?;
    Ok(n)
}
```

In `remove_member`, before the DELETE:

```rust
    if matches!(self.member_role(world, user).await?, Some(WorldRole::Gm))
        && self.gm_count(world).await? <= 1
    {
        return Err(DataError::Conflict("cannot remove the world's only GM".into()));
    }
```

In `set_role`, before the UPDATE (only when demoting away from GM):

```rust
    if role != WorldRole::Gm
        && matches!(self.member_role(world, user).await?, Some(WorldRole::Gm))
        && self.gm_count(world).await? <= 1
    {
        return Err(DataError::Conflict("cannot demote the world's only GM".into()));
    }
```

- [ ] **Step 4: Run**

Run: `cargo test -p shadowcat --lib sole_gm gm_when_another_exists`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/data/sqlite.rs
git commit -m "fix(membership): reject removing/demoting a world's last GM"
```

---

### Task 5: #5 — rate-limit the asset replace endpoint

**Files:**
- Modify: `src/server/src/http/assets.rs` (`replace` ~263)
- Modify: `docs/TODO.md` (close the deferral), `docs/superpowers/plans/2026-06-21-m8b-1-assets-server.md` (note rate-limit now covers replace) — fold into this task's commit
- Test: `src/server/src/http/assets.rs` tests (mirror the upload rate-limit test)

**Context.** `replace` streams a full file like `upload` but has no rate guard.
Apply the identical per-user tiered guard; the limiter is shared with upload
(`state.upload_rate`), capping total write volume per user.

- [ ] **Step 1: Write the failing test** — replacing past the tier cap → 429.
  (Mirror the existing upload rate-limit test: set `config` so
  `effective_rate_per_min` is small, seed an asset, call `replace` until it
  returns `TooManyRequests`.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat --lib replace_rate_limit`
Expected: FAIL — replace currently never rate-limits.

- [ ] **Step 3: Add the guard to `replace`.** After `require_gm`, before streaming:

```rust
    let ctx = require_gm(&state, &user, existing.world_id).await?;
    let now = crate::ws::time::now_millis();
    if !state.upload_rate.check(user.id, now, state.config.effective_rate_per_min(ctx.world_role)) {
        return Err(AppError::TooManyRequests("replace rate limit exceeded".into()));
    }
```

Wrap the fallible work (stream + DB commit + rename) in an `async { ... }.await`
block exactly as `upload` does, and on the `Err` arm call
`state.upload_rate.refund(user.id, now);` before returning the error. The success
arm returns the `Json(Asset { .. })`.

- [ ] **Step 4: Add a refund test** (a failed replace — e.g. oversized — refunds
  the hit so the next replace is allowed), then run

Run: `cargo test -p shadowcat --lib replace_rate_limit replace_refund`
Expected: PASS.

- [ ] **Step 5: Update docs and commit**

Close the `Rate-limit the asset replace endpoint` TODO in `docs/TODO.md`; add a one
-line note to the M8b-1 plan that the per-user tier now covers replace.

```bash
git add src/server/src/http/assets.rs docs/TODO.md docs/superpowers/plans/2026-06-21-m8b-1-assets-server.md
git commit -m "fix(assets): rate-limit the replace endpoint per-user (tiered)"
```

---

### Task 6: #6 — per-user ping rate limiter

**Files:**
- Create: `PingRateLimiter` in `src/server/src/ws/mod.rs` (or alongside `WsState`)
- Modify: `src/server/src/http/mod.rs` (`AppState` + every constructor: `test_state`, the production builder, `ws_convergence.rs` `spawn`, any other)
- Modify: `src/server/src/ws/conn.rs` (`ScenePing` arm ~322, remove the per-connection `ping_times`/`PING_PER_MIN`)
- Test: a unit test on `PingRateLimiter`

**Interfaces:**
- Produces: `PingRateLimiter::new() -> Self`, `check(&self, user: Uuid, now_ms: i64, per_min: usize) -> bool`.
- Consumes (Task 7 also edits `AppState` construction — keep field additions consistent).

**Context.** Mirror `UploadRateLimiter` (per-user `Mutex<HashMap<Uuid, Vec<i64>>>`,
60 s window). No `refund` — a ping is fire-and-forget.

- [ ] **Step 1: Write the failing test.**

```rust
#[test]
fn ping_limit_is_shared_across_connections_per_user() {
    let lim = PingRateLimiter::new();
    let u = Uuid::from_u128(1);
    for i in 0..30 { assert!(lim.check(u, 1_000 + i, 30), "first 30 allowed"); }
    assert!(!lim.check(u, 1_031, 30), "31st in window denied (per-user, not per-conn)");
    // A different user is independent.
    assert!(lim.check(Uuid::from_u128(2), 1_032, 30));
}
```

- [ ] **Step 2: Run to verify it fails (does not compile / type missing)**

Run: `cargo test -p shadowcat --lib ping_limit_is_shared`
Expected: FAIL — `PingRateLimiter` undefined.

- [ ] **Step 3: Implement `PingRateLimiter`** (copy `UploadRateLimiter`'s window
  logic, drop `refund`):

```rust
/// Per-user sliding-window ping budget on shared state — unlike the per-connection
/// window it replaces, N concurrent sockets share one budget (a stronger abuse
/// backstop). 60 s window; over-budget pings drop silently at the call site.
#[derive(Default)]
pub struct PingRateLimiter {
    hits: std::sync::Mutex<std::collections::HashMap<Uuid, Vec<i64>>>,
}

impl PingRateLimiter {
    pub fn new() -> Self { Self::default() }

    pub fn check(&self, user: Uuid, now_ms: i64, per_min: usize) -> bool {
        let mut g = self.hits.lock().unwrap();
        let v = g.entry(user).or_default();
        v.retain(|&t| t > now_ms - 60_000);
        if v.len() >= per_min { return false; }
        v.push(now_ms);
        true
    }
}
```

- [ ] **Step 4: Add `ping_rate: Arc<crate::ws::PingRateLimiter>` to `AppState`**
  and initialize it (`Arc::new(PingRateLimiter::new())`) in EVERY `AppState`
  construction site (`http/mod.rs` production builder + `test_state`,
  `tests/ws_convergence.rs` `spawn`, and any others — grep `AppState {`).

- [ ] **Step 5: Use it in the `ScenePing` arm** and delete the per-connection
  window:

```rust
Ok(ClientMsg::ScenePing { scene, x, y }) => {
    // Per-user budget (shared across this user's sockets). Membership already
    // gated; coordinates unvalidated; over-budget pings drop silently.
    let now = now_millis();
    if state.ping_rate.check(user_id, now, 30) {
        room.broadcast_aux(ServerMsg::ScenePing { scene, x, y, user: user_id });
    }
}
```

Remove `const PING_PER_MIN` and `let mut ping_times` from the ingress loop
(`conn.rs:230-231`). Confirm `state` is in scope in `handle_socket` (it is — the fn
takes `state: AppState`).

- [ ] **Step 6: Run unit test + full lib build**

Run: `cargo test -p shadowcat --lib ping_limit_is_shared && cargo build -p shadowcat`
Expected: PASS / clean build.

- [ ] **Step 7: Commit**

```bash
git add src/server/src/ws/ src/server/src/http/mod.rs src/server/tests/ws_convergence.rs
git commit -m "fix(ws): make the ping rate limit per-user on AppState"
```

---

### Task 7: #7 — deterministic `Lagged` regression test

**Files:**
- Modify: `src/server/src/ws/room.rs` (`Room::new`, `RoomRegistry::get_or_create`, `BROADCAST_CAPACITY` usage)
- Modify: `src/server/tests/ws_convergence.rs` (`spawn` variant + `lagged_drops` accessor + the `slow_reader_recovers_via_resync` test)
- Test: the updated `slow_reader_recovers_via_resync`

**Context.** Make broadcast capacity injectable (default 256) so a test can force
overflow; assert `lagged_drops > 0` via an in-process harness accessor.

- [ ] **Step 1: Thread capacity into `Room`.** Add a field and parameter; default
  the public path to 256.

```rust
// room.rs
fn new(world_id: Uuid, seed_seq: i64, scene: SceneEcs, broadcast_capacity: usize) -> Self {
    let (tx, _rx) = broadcast::channel(broadcast_capacity);
    // ...unchanged...
}
```

In `RoomRegistry`, store an optional capacity override (default `BROADCAST_CAPACITY`)
so production behavior is unchanged and tests can shrink it:

```rust
pub struct RoomRegistry {
    rooms: DashMap<Uuid, Arc<Room>>,
    broadcast_capacity: usize,
}
impl RoomRegistry {
    pub fn new() -> Self { Self { rooms: DashMap::new(), broadcast_capacity: BROADCAST_CAPACITY } }
    #[cfg(test)]
    pub fn with_capacity(cap: usize) -> Self { Self { rooms: DashMap::new(), broadcast_capacity: cap } }
    // get_or_create: Arc::new(Room::new(world_id, world.seq, scene_ecs, self.broadcast_capacity))
}
```

Thread `with_capacity` through `WsState` (add `WsState::with_broadcast_capacity(cap)`),
used only by the test harness. Production `WsState::new()` keeps 256.

- [ ] **Step 2: Add a harness accessor + small-capacity spawn.** In
  `ws_convergence.rs`:

```rust
impl Harness {
    async fn lagged_drops(&self) -> u64 {
        // In-process read of the room's telemetry (no admin HTTP needed).
        self.state.ws.rooms.get(self.world).unwrap().stats.lagged_drops.load(Ordering::Relaxed)
    }
}
```

To reach `state`, store it on `Harness` (add `state: AppState`) and build the room
registry with a tiny capacity: in `spawn`, construct
`ws: shadowcat::ws::WsState::with_broadcast_capacity(8)`. (Keep a default `spawn`
for the other tests; add `spawn_with_capacity(cap)` or make `spawn` take the cap and
update callers.)

- [ ] **Step 3: Rewrite `slow_reader_recovers_via_resync` to assert the lag path.**

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_reader_recovers_via_resync() {
    let h = spawn_with_capacity(8).await; // ring of 8 guarantees overflow
    let mut slow = h.connect().await;
    let _ = slow.next().await; // Welcome — then we stop reading `slow`

    let mut pubc = h.connect().await;
    let _ = pubc.next().await;
    for n in 0..400 { pubc.send(create_intent(h.world, n)).await.unwrap(); }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // The non-reading slow socket's broadcast receiver provably overflowed an
    // 8-slot ring → the Lagged path fired.
    assert!(h.lagged_drops().await > 0, "Lagged path must fire deterministically");

    // And it still converges via resync.
    let seqs = drain_until_seq(&mut slow, 400).await;
    assert_eq!(*seqs.last().unwrap(), 400);
    let mut sorted = seqs.clone(); sorted.sort(); sorted.dedup();
    assert_eq!(seqs, sorted, "no duplicates or reordering after resync");
    assert_eq!(*h.authoritative_seqs().await.last().unwrap(), 400);
}
```

- [ ] **Step 4: Run**

Run: `cargo test -p shadowcat --test ws_convergence slow_reader_recovers_via_resync`
Expected: PASS, and `lagged_drops > 0` holds.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/ws/room.rs src/server/tests/ws_convergence.rs
git commit -m "test(ws): force the broadcast Lagged path deterministically (injectable capacity)"
```

---

### Task 8: #10 — `WorldCapDefaults` (doc_type-scoped per-document grants)

**Files:**
- Modify: `src/server/src/data/document.rs` (new `WorldCapDefaults`, `RoleCaps`)
- Modify: `src/server/src/data/sqlite.rs` (`world_cap_defaults`, `set_world_cap_defaults`, `apply_intent`, search ~1230)
- Modify: `src/server/src/data/mod.rs` (Repository trait signature for `world_cap_defaults`)
- Modify: `src/server/src/data/permission.rs` (`filter_command` signature + 3 `resolve_access_world` calls)
- Modify: `src/server/src/http/routes.rs` (`list_documents`, `get_document`, `set_world_cap_defaults` handler, `validate_grants`)
- Modify: `src/server/src/ws/conn.rs` (load `WorldCapDefaults`; `filter_command` calls)
- Modify: the Welcome broadcast site (projects `defaults.all`)
- Test: `document.rs` unit tests for `grants_for`; an `apply_intent`/`filter` integration test

**Interfaces:**
- Produces: `WorldCapDefaults { all: CapabilityGrants, by_type: BTreeMap<String, CapabilityGrants>, role_caps: RoleCaps }`; `WorldCapDefaults::grants_for(&self, doc_type: &str) -> CapabilityGrants`; `RoleCaps { all: BTreeMap<WorldRole, BTreeSet<String>>, by_type: BTreeMap<String, BTreeMap<WorldRole, BTreeSet<String>>> }`. Consumed by Task 9.

- [ ] **Step 1: Write the failing test** for `grants_for` merge.

```rust
#[test]
fn grants_for_merges_all_and_by_type() {
    let mut d = WorldCapDefaults::default();
    d.all.by_role.entry(DocRole::Owner).or_default().insert("core:manage_embedded".into());
    d.by_type.entry("token".into()).or_default()
        .by_role.entry(DocRole::Owner).or_default().insert("dnd5e:move".into());
    let g = d.grants_for("token");
    let owner = g.by_role.get(&DocRole::Owner).unwrap();
    assert!(owner.contains("core:manage_embedded") && owner.contains("dnd5e:move"));
    // A type with no override gets only `all`.
    assert!(!d.grants_for("actor").by_role.get(&DocRole::Owner).unwrap().contains("dnd5e:move"));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat --lib grants_for_merges_all_and_by_type`
Expected: FAIL — type undefined.

- [ ] **Step 3: Define the types + helper** in `document.rs`:

```rust
/// World-level capability configuration (one row per world, JSON in settings).
/// `all`/`by_type` are additive per-document grants over the DocRole floor
/// (doc-type-scoped). `role_caps` carries world-level capabilities keyed by
/// WorldRole (e.g. `core:create`), distinct because creation has no document and
/// thus no DocRole. GM/admin is never keyed here — it holds every capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorldCapDefaults {
    #[serde(default)]
    pub all: CapabilityGrants,
    #[serde(default)]
    pub by_type: BTreeMap<String, CapabilityGrants>,
    #[serde(default)]
    pub role_caps: RoleCaps,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RoleCaps {
    #[serde(default)]
    pub all: BTreeMap<WorldRole, BTreeSet<String>>,
    #[serde(default)]
    pub by_type: BTreeMap<String, BTreeMap<WorldRole, BTreeSet<String>>>,
}

impl WorldCapDefaults {
    /// Per-document additive grants for `doc_type`: `all` ∪ `by_type[doc_type]`.
    pub fn grants_for(&self, doc_type: &str) -> CapabilityGrants {
        let mut g = self.all.clone();
        if let Some(t) = self.by_type.get(doc_type) {
            for (r, caps) in &t.by_role { g.by_role.entry(*r).or_default().extend(caps.iter().cloned()); }
            for (u, caps) in &t.by_user { g.by_user.entry(*u).or_default().extend(caps.iter().cloned()); }
        }
        g
    }

    /// Whether `role` holds `cap` at world level for `doc_type` (role_caps).
    pub fn role_has(&self, role: WorldRole, doc_type: &str, cap: &str) -> bool {
        self.role_caps.all.get(&role).is_some_and(|s| s.contains(cap))
            || self.role_caps.by_type.get(doc_type).and_then(|m| m.get(&role))
                .is_some_and(|s| s.contains(cap))
    }
}
```

(`WorldRole` needs `Ord` for the `BTreeMap` key — add `PartialOrd, Ord` to its
derive in `document.rs` if absent.)

- [ ] **Step 4: Change storage + trait.**
  - `world_cap_defaults(world) -> Result<WorldCapDefaults, DataError>` (trait in
    `data/mod.rs` and impl in `sqlite.rs:1180`); default = `WorldCapDefaults::default()`.
  - `set_world_cap_defaults(world, defaults: &WorldCapDefaults)` (`sqlite.rs:508`).

- [ ] **Step 5: Thread `doc_type` at every consumer.**
  - `routes.rs` `list_documents`: `let defaults = state.repo.world_cap_defaults(world).await?; let world_grants = defaults.grants_for(&q.r#type);` then `resolve_access_world(.., &world_grants)`.
  - `routes.rs` `get_document`: `defaults.grants_for(&doc.doc_type)`.
  - `sqlite.rs` `apply_intent`: per op, `let g = world_defaults.grants_for(&doc.doc_type);` pass `&g` to `resolve_access_world`. (`world_defaults` is now `WorldCapDefaults`, loaded once.)
  - `sqlite.rs` search (~1230): grants_for the searched type.
  - `permission.rs` `filter_command(repo, cmd, ctx, world_defaults: &WorldCapDefaults)`: in each arm compute `world_defaults.grants_for(&doc.doc_type)` (Create/Delete use `doc`, Update uses `cur`) and pass to `resolve_access_world`.
  - `conn.rs`: load `let world_defaults = repo.world_cap_defaults(world_id).await...` as `WorldCapDefaults`; pass `&world_defaults` to `filter_command`/`replay`.
  - Welcome broadcast: `project_grants_for(&defaults.all, user)` — wire shape unchanged (still a `CapabilityGrants`).
  - `routes.rs` `set_world_cap_defaults` handler + `validate_grants`: accept a `WorldCapDefaults` body; validate every capability token across `all`, `by_type`, and `role_caps` with `validate_capability`.
  - **Update existing test call sites:** every `filter_command(.., &CapabilityGrants::default())` in `permission.rs` tests (and the Create-broadcast test added in Task 3) becomes `&WorldCapDefaults::default()`. Grep `filter_command(` to catch all.

- [ ] **Step 6: Add an integration test** that a `by_type["token"]` Owner grant lets
  an Owner manage-embedded on a token but not on an actor (resolve via
  `resolve_access_world(.., grants_for("token"))` vs `grants_for("actor")`).

- [ ] **Step 7: Run + build**

Run: `cargo test -p shadowcat --lib grants_for world_cap && cargo build -p shadowcat`
Expected: PASS / clean.

- [ ] **Step 8: Commit**

```bash
git add src/server/src/data/ src/server/src/http/ src/server/src/ws/conn.rs
git commit -m "feat(caps): doc_type-scoped world capability defaults (WorldCapDefaults)"
```

---

### Task 9: #9 — `core:create` gate via GM-configured WorldRole `role_caps`

**Files:**
- Modify: `src/server/src/data/sqlite.rs` (`apply_intent` Create arm ~865)
- Test: `src/server/src/data/sqlite.rs` integration tests

**Interfaces:**
- Consumes: `WorldCapDefaults::role_has` (Task 8), `cap::CREATE` (`permission.rs:21`).

**Context.** GM/admin always create. A non-GM creates only if their `WorldRole`
holds `core:create` for the doc's type in `role_caps`. Default empty → GM-only.
403 on denial (no target id → no existence leak).

- [ ] **Step 1: Write failing tests.**

```rust
#[tokio::test]
async fn non_gm_create_denied_by_default() {
    let (r, w, player_ctx) = world_with_player().await; // helper: world + a Player member ctx
    let doc = actor_doc(w); // owned by the player
    let err = r.apply_intent(&player_ctx, w, vec![Operation::Create { doc }], 1).await.unwrap_err();
    assert!(matches!(err, DataError::Forbidden));
}

#[tokio::test]
async fn non_gm_create_allowed_with_role_grant() {
    let (r, w, player_ctx) = world_with_player().await;
    let mut d = WorldCapDefaults::default();
    d.role_caps.all.entry(WorldRole::Player).or_default().insert("core:create".into());
    r.set_world_cap_defaults(w, &d).await.unwrap();
    let doc = actor_doc(w);
    assert!(r.apply_intent(&player_ctx, w, vec![Operation::Create { doc }], 1).await.is_ok());
}

#[tokio::test]
async fn role_grant_is_type_scoped() {
    let (r, w, player_ctx) = world_with_player().await;
    let mut d = WorldCapDefaults::default();
    d.role_caps.by_type.entry("token".into()).or_default()
        .entry(WorldRole::Player).or_default().insert("core:create".into());
    r.set_world_cap_defaults(w, &d).await.unwrap();
    // token allowed, actor denied
    assert!(r.apply_intent(&player_ctx, w, vec![Operation::Create { doc: token_doc(w) }], 1).await.is_ok());
    let err = r.apply_intent(&player_ctx, w, vec![Operation::Create { doc: actor_doc(w) }], 2).await.unwrap_err();
    assert!(matches!(err, DataError::Forbidden));
}
```

(Build `world_with_player`: create a GM-owned world, `add_member(w, player, Player)`,
a `PermissionContext { user_id: player, world_role: Player }`. The player must own
the created doc so the existing `WRITE_FIELDS` floor passes — isolating the
`core:create` gate as the cause of denial.)

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p shadowcat --lib non_gm_create role_grant_is_type_scoped`
Expected: `non_gm_create_denied_by_default` FAILs (create currently succeeds).

- [ ] **Step 3: Add the create gate** in `apply_intent`'s Create arm, after the
  existing `WRITE_FIELDS` / declared-caps checks:

```rust
    // World-level create authorization (#9): GM/admin hold every capability;
    // otherwise the actor's WorldRole must hold core:create for this doc type.
    // Create has no document, so this rides WorldRole (role_caps), not DocRole.
    if ctx.world_role != WorldRole::Gm
        && !world_defaults.role_has(ctx.world_role, &doc.doc_type, cap::CREATE)
    {
        tracing::debug!(user = %ctx.user_id, doc_type = %doc.doc_type, "create denied: missing core:create");
        return Err(DataError::Forbidden);
    }
```

- [ ] **Step 4: Run**

Run: `cargo test -p shadowcat --lib non_gm_create role_grant_is_type_scoped`
Expected: PASS. Also run the full suite to catch tests that create as a non-GM and
now need a grant: `cargo test -p shadowcat`.

- [ ] **Step 5: Update docs + commit.** Close the #9 entry in
  `POST_WORK_FINDINGS.md`.

```bash
git add src/server/src/data/sqlite.rs docs/POST_WORK_FINDINGS.md
git commit -m "feat(caps): gate document creation on WorldRole core:create (GM default)"
```

---

### Task 10: #8 (server) — `list_members` returns usernames

**Files:**
- Modify: `src/server/src/data/sqlite.rs` (`list_members` ~251)
- Modify: `src/server/src/http/routes.rs` (`MemberEntry` ~326, `list_members` handler)
- Test: `sqlite.rs` test for the JOIN; a routes test for the 403-non-GM guard

**Interfaces:**
- Produces: `MemberEntry { user: Uuid, username: String, role: WorldRole }` (JSON `{ user, username, role }`).

- [ ] **Step 1: Write the failing test.**

```rust
#[tokio::test]
async fn list_members_includes_usernames() {
    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r.create_user("alice", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let members = r.list_members(w.id).await.unwrap();
    assert!(members.iter().any(|(_, name, _)| name == "alice"));
}
```

- [ ] **Step 2: Run to verify it fails (signature mismatch)**

Run: `cargo test -p shadowcat --lib list_members_includes_usernames`
Expected: FAIL — `list_members` returns `(Uuid, WorldRole)`.

- [ ] **Step 3: JOIN users and widen the return type.**

```rust
pub async fn list_members(&self, world: Uuid) -> Result<Vec<(Uuid, String, WorldRole)>, DataError> {
    let rows = sqlx::query(
        "SELECT m.user_id, u.username, m.role \
         FROM world_members m JOIN users u ON u.id = m.user_id \
         WHERE m.world_id = ?",
    )
    .bind(world.to_string())
    .fetch_all(&self.pool)
    .await?;
    rows.into_iter().map(|r| {
        let uid = Uuid::parse_str(r.get::<String, _>("user_id").as_str())
            .map_err(|e| DataError::OpFailed(e.to_string()))?;
        let username: String = r.get("username");
        let role: WorldRole = serde_json::from_value(serde_json::Value::String(r.get::<String, _>("role")))?;
        Ok((uid, username, role))
    }).collect()
}
```

- [ ] **Step 4: Update `MemberEntry` + handler** (`routes.rs`):

```rust
#[derive(Serialize)]
pub struct MemberEntry { pub user: Uuid, pub username: String, pub role: WorldRole }
// ...
        .map(|(user, username, role)| MemberEntry { user, username, role })
```

- [ ] **Step 5: Run + build**

Run: `cargo test -p shadowcat --lib list_members && cargo build -p shadowcat`
Expected: PASS / clean. (Fix any other `list_members` caller for the new arity.)

- [ ] **Step 6: Commit**

```bash
git add src/server/src/data/sqlite.rs src/server/src/http/routes.rs
git commit -m "feat(members): list_members returns usernames"
```

---

### Task 11: #8 (client) — label the see-as picker by username

**Files:**
- Modify: `src/client/ui/src/lib/api.ts` (add `listWorldMembers`)
- Modify: `src/client/ui/src/lib/appContext.ts` (add `members: Map<string, string>`)
- Modify: `src/client/ui/src/lib/worldSession.svelte.ts` (fetch members on Welcome when GM; expose via context)
- Modify: `src/client/ui/src/modules/core-ui/panels/Stage.svelte` (label options by username)
- Test: `pnpm --filter @shadowcat/ui` unit test; extend `stage.spec.ts`

**Interfaces:**
- Consumes: `GET /api/worlds/{id}/members` → `{ user, username, role }[]` (Task 10).
- Produces: `AppContext.members: Map<userId, username>`.

- [ ] **Step 1: Add the API call** to `api.ts`:

```typescript
export interface WorldMember { user: string; username: string; role: "gm" | "player" | "spectator"; }

export function listWorldMembers(world: string): Promise<WorldMember[]> {
  return getJson<WorldMember[]>(`/api/worlds/${world}/members`);
}
```

- [ ] **Step 2: Add `members` to AppContext** (`appContext.ts` interface):

```typescript
  /** userId -> username for the world's members (GM-only source; empty for players). */
  members: Map<string, string>;
```

- [ ] **Step 3: Write the failing UI test** — given a members map, the picker shows
  usernames; an unknown owner falls back to the short id. (Mirror existing
  `Stage.svelte` test harness with a stub AppContext exposing `members` and a token
  whose `owner` is in the map.)

- [ ] **Step 4: Run to verify it fails**

Run: `pnpm --filter @shadowcat/ui test -- stage`
Expected: FAIL — option still renders `owner.slice(0,8)`.

- [ ] **Step 5: Populate `members` on Welcome (GM only)** in `worldSession.svelte.ts`.
  Add a field `members = $state(new Map<string, string>())`, expose it on the
  AppContext object the session builds, and in `#onWelcome` after `this.role` is set:

```typescript
      if (this.role === "gm" && this.world) {
        try {
          const list = await listWorldMembers(this.world);
          this.members = new Map(list.map((m) => [m.user, m.username]));
        } catch (e) {
          this.#logger.warn("member list fetch failed", e);
        }
      }
```

- [ ] **Step 6: Label by username** in `Stage.svelte`. Pull `members` from context
  (`const { ..., members } = getAppContext();`) and change the option:

```svelte
<option value={`as:${owner}`}>See as {members.get(owner) ?? owner.slice(0, 8)}</option>
```

(Optionally extend `playerOptions` to union token owners with all non-GM members
from `members` so a member who owns no token still appears.)

- [ ] **Step 7: Run unit + e2e**

Run: `pnpm --filter @shadowcat/ui test -- stage` then `pnpm --filter @shadowcat/ui build && pnpm --filter @shadowcat/ui e2e -- stage`
Expected: PASS.

- [ ] **Step 8: Update docs + commit.** Close the M9c-2 see-as TODO in `docs/TODO.md`.

```bash
git add src/client/ui/src/lib/api.ts src/client/ui/src/lib/appContext.ts src/client/ui/src/lib/worldSession.svelte.ts src/client/ui/src/modules/core-ui/panels/Stage.svelte docs/TODO.md
git commit -m "feat(client): label see-as-player picker by username"
```

---

### Task 12: #11 — convergent offline intent replay (BUDDY-CHECK)

**Files:**
- Modify: `src/client/ui/src/lib/worldSession.svelte.ts` (`dispatchIntent` ~89; flush after resync)
- Modify: `src/client/core/src/ws-client.ts` (expose a resync-complete hook, or a `running`/`connected` distinction) — only if needed for the signal
- Test: `pnpm --filter @shadowcat/ui test` (and/or `@shadowcat/core` for queue logic)

**Context.** While `running` but transport is null (reconnecting): apply
optimistically AND enqueue `{ intentId, ops }`. Flush in FIFO order after
`resync_end`. Reject when not `running`. Convergence is the contract: the
`OptimisticClient` rebases predictions onto authoritative state; queued intents then
confirm/reject normally.

**Buddy-check focus:** (a) FIFO correlation across offline→flush; (b) rebase when
resync updates land before queued intents are sent; (c) reject-when-stopped leaves
no orphaned pending entry.

- [ ] **Step 1: Decide the reconnect/stopped signal.** `WsClient.connected` is
  `transport !== null`; `running` distinguishes "reconnecting" (true) from "stopped"
  (false). Expose `get running()` on `WsClient` if not already public, and an
  `onResyncComplete` callback option (invoked in the `resync_end` case at
  `ws-client.ts:228`) so `WorldSession` knows when to flush.

- [ ] **Step 2: Write the failing test** — an intent dispatched while transport is
  null is applied optimistically and replayed on reconnect.

```typescript
test("offline intent is applied optimistically and flushed after resync", () => {
  const session = makeTestSession(); // existing harness; #ws stubbed
  session.__setTransport(null, /*running*/ true);   // reconnecting
  session.dispatchIntent([{ op: "create", doc: makeDoc("d1") }]);
  // Applied optimistically despite no transport:
  expect(session.documents.get("d1")).toBeDefined();
  // Reconnect + resync completes:
  session.__reconnectAndResync();
  // The queued intent was sent exactly once, in order:
  expect(session.__sentIntents().map((i) => i.ops[0].doc.id)).toEqual(["d1"]);
});

test("dispatch while stopped is rejected with no pending entry", () => {
  const session = makeTestSession();
  session.__setTransport(null, /*running*/ false);  // stopped
  session.dispatchIntent([{ op: "create", doc: makeDoc("d2") }]);
  expect(session.documents.get("d2")).toBeUndefined();
  expect(session.documents.pendingIntents?.() ?? []).toEqual([]);
});
```

(Adapt to the existing WorldSession test seams; add minimal `__`-prefixed test
hooks only if the harness lacks them.)

- [ ] **Step 3: Run to verify it fails**

Run: `pnpm --filter @shadowcat/ui test -- worldSession`
Expected: FAIL — `dispatchIntent` currently drops when not connected.

- [ ] **Step 4: Implement the queue + flush.** Add a field
  `#offlineQueue: { intentId: string; ops: WireOperation[] }[] = []` and rewrite
  `dispatchIntent`:

```typescript
  dispatchIntent(ops: WireOperation[]): void {
    const intentId = crypto.randomUUID();
    if (this.#ws?.connected) {
      this.#optimistic.applyIntent(intentId, ops);
      this.#ws.send({ type: "intent", intent_id: intentId, ops });
      return;
    }
    if (this.#ws?.running) {
      // Reconnecting: predict now (immediate feedback) and queue for FIFO replay
      // after resync. Every offline intent queues, so send-order == optimistic
      // FIFO order — the confirm-correlation contract holds.
      this.#optimistic.applyIntent(intentId, ops);
      this.#offlineQueue.push({ intentId, ops });
      return;
    }
    // Stopped: no reconnect is coming; drop without a pending entry.
    this.#logger.warn("dropping intent: session stopped");
  }
```

Flush after resync completes (wire `onResyncComplete` from Step 1):

```typescript
  #flushOfflineQueue(): void {
    if (!this.#ws?.connected || this.#offlineQueue.length === 0) return;
    const queued = this.#offlineQueue;
    this.#offlineQueue = [];
    for (const { intentId, ops } of queued) {
      // Prediction already applied at dispatch; just transmit, preserving order.
      this.#ws.send({ type: "intent", intent_id: intentId, ops });
    }
  }
```

Call `#flushOfflineQueue()` from the resync-complete handler (after scene
re-subscription in the Welcome path, so authoritative state is current first).

- [ ] **Step 5: Run**

Run: `pnpm --filter @shadowcat/ui test -- worldSession`
Expected: PASS.

- [ ] **Step 6: Update docs + commit.** Close the #11 TODO in `docs/TODO.md`.

```bash
git add src/client/ui/src/lib/worldSession.svelte.ts src/client/core/src/ws-client.ts docs/TODO.md
git commit -m "feat(client): replay offline intents on resync (convergent FIFO flush)"
```

---

## Buddy-check directives

Per the project rule, run an independent buddy-check (two blind reviewers, debate to
convergence) on the security/secrecy-touching changes BEFORE the final branch merge:

- **Task 3 (#3 embedded redaction):** verify no GM-only field at any embedded depth
  reaches a non-GM on Create/Delete egress and REST; explicitly RULE on the related
  `filter_command` `Update`-arm sub-case (does an `/embedded/.../...` write redact
  against the child's overrides?). If ruled out of this pass, log it to `TODO.md`.
- **Task 12 (#11 offline replay):** verify FIFO confirm-correlation across the
  offline→flush boundary, the rebase interaction when resync updates precede queued
  sends, and that reject-when-stopped leaves no orphaned pending entry.

The remaining tasks get the single dispatched fresh-context branch review that
`mainline-plan-execution` performs at the end.

## Documentation sync (final task, before merge)

- `docs/POST_WORK_FINDINGS.md`: remove the closed `Needs triage` entries (#1, #2,
  #3, #4, #7, #9, #10). Leave the `Accepted` entries.
- `docs/TODO.md`: remove closed deferrals (asset replace rate-limit, per-user ping,
  see-as username, offline replay). Add any buddy-check-produced deferral (e.g.
  embedded-`Update` redaction) if ruled out of scope.
- `docs/superpowers/plans/2026-06-21-m8b-1-assets-server.md`: note replace is now
  rate-limited.
- `docs/PLAN.md`: note pre-M10 cleanup complete.
- `OPEN_BUGS.md`: unchanged (empty).
