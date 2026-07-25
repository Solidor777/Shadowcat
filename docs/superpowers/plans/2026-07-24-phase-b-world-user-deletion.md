# Phase B — World & User Deletion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: On a Fable-class session use `mainline-plan-execution`; otherwise use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** End-to-end world deletion and user deletion: `DELETE /api/worlds/{id}` and `DELETE /api/users/{id}` with live-connection eviction, complete row/file cleanup, a scene-delete fog purge, a transactional `add_member`, and minimal UI affordances.

**Architecture:** Deletion is server-authoritative and ordered around the project's delete convention (commit DB rows first, then remove files). World deletion tombstones + evicts the live room, runs one cascade transaction (FK cascades + explicit purges for the FK-less `explored_fog` and per-world `settings` rows), then removes the world's asset directory. User deletion is admin-gated with a last-admin guard, revokes sessions **in the same transaction** (the `AuthUser` extractor trusts the session record without re-reading `users`), and kicks live connections cross-world via a new targeted `ServerMsg::Evicted` frame.

**Tech Stack:** Rust (axum 0.8, sqlx 0.9/SQLite, dashmap, ts-rs), Svelte 5 Runes, Zod wire mirror, Vitest, Playwright.

**Campaign context:** `docs/superpowers/specs/2026-07-24-phase1-closeout-campaign-design.md` §Phase B (lines 111–131). Plan-time verification already done (2026-07-24): FTS `AFTER DELETE` triggers **do** fire under FK cascade (verified empirically, both `recursive_triggers` settings; Task 4 pins it against the bundled SQLite 3.46), and the full FK audit produced the gap list this plan closes (§Global Constraints, "Audit deltas").

## Global Constraints

- **Worktree setup ordering:** after creating the worktree, run `pnpm install` then `pnpm build` **before any `cargo` command** — `rust-embed` validates `../../dist/` at compile time. If vitest fails with MAX_PATH errors, add a worktree `.npmrc` with `virtual-store-dir-max-length=40` and reinstall.
- **Never edit migrations `0001`–`0009`** — sqlx checksums applied migrations. New DDL goes in `0010_explored_fog_world_index.sql` and `0011_assets_created_by_set_null.sql` only. Stale comments inside old migration files stay stale; fix live-code comments instead.
- **Run cargo from the repo root** via `--manifest-path src/server/Cargo.toml` (a persisted `cd src/server` breaks the Edit hook and git paths).
- **Wire-schema tasks (1, 3, 6) gate with `pnpm -r test` AND `pnpm -r typecheck`** — a ts-rs/Zod change can break sibling packages' fixtures that a filtered run misses.
- **Every server commit must be `cargo fmt`-clean** (CI-enforced at push).
- **Commit per task** once its verification passes; **no push** until the phase merges (milestone push convention).
- **HTTP conventions:** success = `204 No Content`; guard rejections = `409 Conflict` with a verbatim client-actionable message; world-scoped routes return `403` for non-members (no 404 remap — that is only for by-id document routes); unknown target = `404 {"error":"not found"}`.
- **Deletion of dev files:** use `trash`, never `rm`/`Remove-Item`.
- **Subagent dispatch (if executing via SDD):** every implementer brief must include (a) the explicit cd-to-worktree + verify-branch guard, (b) the delivery-channel instruction ("return your report as the Agent tool result"), (c) a no-destructive-git line, and (d) the session's verbatim user directive.

**Audit deltas vs the spec text** (found at plan time, all in scope — no-deferral campaign rule):
1. Five per-world `settings` rows (`world_caps:`, `world_caps_req:`, `world_contracts:`, `world_schemas:`, `world_modules:`) have no FK and must be purged in B1's transaction (Task 4, via a single `world_settings_keys` source).
2. `explored_fog.user_id` has no FK — B2 must purge fog rows by user (Task 7). **Decision: no `user_id` index** — user deletion is a rare admin op; a scan is acceptable, and the fog write path is hot (every reveal) so a permanent index tax is the wrong trade.
3. Sessions: `tower_sessions` has no `user_id` column, and a deleted user's cookie stays fully authenticated (`AuthUser` never re-reads `users`). B2 revokes via `json_extract(data, '$.data.user.id')` **inside the delete transaction** (Task 7). JSON1 is built into the bundled SQLite (libsqlite3-sys 0.30.1 = SQLite 3.46).
4. Room re-creation race: `get_or_create` re-hydrates any world whose row still exists, from both `conn.rs:176` and `routes.rs:419` — the evict-then-transact window needs a registry tombstone (Task 2).
5. The client silently ignores `error` frames and reconnect-loops forever — eviction needs a wire frame + client terminal handling (Tasks 1, 3).
6. FTS delete triggers fire under cascade — **no explicit FTS deletes in B1's tx**, but Task 4 pins the behavior with a Rust test.

**Design decisions locked (best long-term shape):**
- One targeted eviction frame `ServerMsg::Evicted { user: Option<Uuid> }` serves both B1 (`None` = whole room) and B2 (`Some` = that user, broadcast to all rooms) — one mechanism, no fork.
- Self-deletion of one's own account is refused (409) — removes the "revoke the caller's session mid-request" problem; another admin performs it. The in-tx last-admin guard stays as the structural backstop.
- B3's fog purge is folded into a shared `delete_document_tx` helper called by BOTH `apply_intent` and `apply_command` (never-fork), unconditional by id (no `doc_type` predicate to drift).
- B4 becomes one guarded `INSERT…SELECT…ON CONFLICT` that also proves world existence (admin-on-unknown-world becomes 404 instead of today's FK 500), preserving the last-GM demotion guard in the same tx.
- B5's type-the-name confirm is an inline arm-to-delete row control (entry has no AppContext and no `@shadowcat/core` dep — a modal service would break the swappable-entry property).

## Model/Effort directives

Plan written mainline on Fable 5 / effort high (per CLAUDE.md, `sdd-plan-writer-*` dispatch applies to non-Fable sessions only). Execution: **user chose mainline** (2026-07-24) — `mainline-plan-execution` in the Fable session, per-task inline compliance checks, one dispatched whole-branch final review plus the mandated two-reviewer pair below.

## Buddy-check directives

Phase B is security-sensitive (campaign spec: two-reviewer phases = A, B, C, G). **Mandated:** at final review, dispatch the two-reviewer pair — `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` — over the whole branch, and the reviewed skill-update gate applies to Task 14's skill diffs.

---

### Task 1: `ServerMsg::Evicted` frame + egress termination (server)

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (ServerMsg enum, after the `Error` variant at :241)
- Modify: `src/server/src/ws/conn.rs` (egress_loop non-Event match at :1303)
- Regenerated: `src/types/generated/ServerMsg.ts` (via `cargo test`)
- Test: `src/server/src/http/mod.rs` `tests` module (real-transport WS tests)

**Interfaces:**
- Consumes: `Room::broadcast_aux(ServerMsg)` (room.rs:193), `text(&ServerMsg) -> Message` helper in conn.rs, `ctx.user_id` in egress_loop.
- Produces: `ServerMsg::Evicted { user: Option<Uuid> }` — wire form `{"type":"evicted","user":"<uuid>"|null}` (enum is `#[serde(tag="type", rename_all="snake_case")]`). Tasks 2, 3, 5, 8 rely on this exact variant.

- [ ] **Step 1: Write the failing test.** In `src/server/src/http/mod.rs`'s `tests` module, using the existing `real_transport_server`/`login_server` helpers (mod.rs:528/:654) and the established WS-test idiom in that module (copy the connect/welcome handshake from a neighbouring real-transport test):

```rust
#[tokio::test]
async fn evicted_frame_targets_and_closes() {
    // Arrange: state + logged-in GM with a world, real WS transport, welcome consumed.
    // (Use the same setup lines as the nearest existing real-transport WS test.)
    let state = initialized_state().await;
    // ... connect a socket for user A to their world, read past Welcome ...
    let room = state
        .ws
        .rooms
        .get(world)
        .expect("room exists after join");

    // A frame targeted at a DIFFERENT user must not be delivered and must not close A.
    room.broadcast_aux(crate::ws::protocol::ServerMsg::Evicted { user: Some(uuid::Uuid::new_v4()) });
    // A frame targeted at nobody (world deletion) is delivered to A, then the socket closes.
    room.broadcast_aux(crate::ws::protocol::ServerMsg::Evicted { user: None });

    // Assert: the NEXT text frame A receives is the evicted{user:null} frame
    // (proving the targeted-at-other frame was skipped), then the socket yields
    // a Close/None (connection terminated).
}
```

Fill the elided arrange/assert lines from the neighbouring test's exact socket API — the assertion contract is: next text frame contains `"type":"evicted"` and `"user":null`, and the read after that is a close/stream-end.

- [ ] **Step 2: Run it to verify it fails** (variant doesn't exist → compile error is the failure):
`cargo test --manifest-path src/server/Cargo.toml evicted_frame_targets_and_closes` → FAIL.

- [ ] **Step 3: Add the variant** in `protocol.rs` immediately after `Error { code: WsErrorCode, message: String },` (:241):

```rust
/// Terminal eviction notice: the recipient's world or account is being
/// deleted. `user: None` addresses every connection in the room (world
/// deletion); `Some(id)` addresses only that user's connections (account
/// deletion — broadcast to every room, non-targets skip it silently). The
/// egress loop delivers this frame, sends a protocol Close, and terminates
/// the connection; the client must treat it as terminal (no reconnect).
Evicted { user: Option<Uuid> },
```

- [ ] **Step 4: Add the egress arm** in `conn.rs`, inside `let should_break = match msg.as_ref() {` (:1303), **before** the `other =>` arm:

```rust
ServerMsg::Evicted { user } => {
    // Targeted eviction. Delivery of the frame is best-effort; the
    // Close and the `break` are the point — the ingress loop tears the
    // connection down when this egress task exits.
    if user.is_none() || *user == Some(ctx.user_id) {
        let _ = sink.send(text(msg.as_ref())).await;
        let _ = sink.send(Message::Close(None)).await;
        true
    } else {
        false
    }
}
```

(`text` and `Message` are already in scope in conn.rs — `text(&out)` at :1329.)

- [ ] **Step 5: Run the test to verify it passes**, then regenerate bindings and check the full server suite:
`cargo test --manifest-path src/server/Cargo.toml` → PASS (this also rewrites `src/types/generated/ServerMsg.ts`; verify it now contains the `evicted` variant with `user: string | null`).

- [ ] **Step 6: Client-side gate** (generated type changed): `pnpm -r typecheck && pnpm -r test` → PASS. (The Zod mirror is intentionally NOT updated yet — `ServerMsgSchema.safeParse` simply rejects unknown frames, which is today's behavior; Task 3 adds the mirror. If a wire drift-guard test compares the generated union to the Zod union and fails here, do Task 3's `wire.ts` addition in this commit instead and say so in the commit body.)

- [ ] **Step 7: Commit.** `git add -- src/server/src/ws/protocol.rs src/server/src/ws/conn.rs src/server/src/http/mod.rs src/types/generated/ServerMsg.ts` then `git commit -m "feat(server): targeted Evicted ws frame with egress termination"`

---

### Task 2: `RoomRegistry` deletion tombstone + `evict_user`

**Files:**
- Modify: `src/server/src/ws/room.rs` (`RoomRegistry` struct :783, `get_or_create` :810)
- Test: `src/server/src/ws/room.rs` tests module

**Interfaces:**
- Consumes: `ServerMsg::Evicted` (Task 1), `dashmap::DashSet`.
- Produces:
  - `pub fn begin_delete(&self, world_id: Uuid) -> Option<Arc<Room>>` — tombstones + removes; Task 5 calls it.
  - `pub fn finish_delete(&self, world_id: Uuid)` — lifts the tombstone; Task 5 calls it on ALL exit paths.
  - `pub fn evict_user(&self, user: Uuid)` — broadcasts `Evicted{Some(user)}` to every live room; Task 8 calls it.
  - `get_or_create` returns `Ok(None)` for a tombstoned world (same refusal the caller already handles for a missing world row).

- [ ] **Step 1: Write the failing tests** in room.rs's tests module (reuse the module's existing repo/room fixtures — e.g. however `get_or_create` tests build a repo with a world):

```rust
#[tokio::test]
async fn begin_delete_tombstones_and_removes() {
    // repo with one world; registry.get_or_create succeeds.
    // begin_delete returns Some(room); a second get_or_create now returns
    // Ok(None) EVEN THOUGH the world row still exists (tombstone, not row-absence).
    // finish_delete lifts it; get_or_create succeeds again.
}

#[tokio::test]
async fn evict_user_reaches_every_room() {
    // Two worlds, two rooms. subscribe() a receiver on each.
    // registry.evict_user(u) → both receivers yield Evicted{user: Some(u)}.
}
```

- [ ] **Step 2: Run to verify they fail** (methods don't exist): `cargo test --manifest-path src/server/Cargo.toml begin_delete_tombstones evict_user_reaches` → FAIL.

- [ ] **Step 3: Implement.** Add the field (and `use dashmap::DashSet;`):

```rust
pub struct RoomRegistry {
    rooms: DashMap<Uuid, Arc<Room>>,
    /// Worlds mid-deletion. `get_or_create` refuses these so an evicted
    /// client's reconnect (or a racing HTTP document write) cannot re-hydrate
    /// a room between the eviction broadcast and the DB commit that removes
    /// the world row. Lifted by `finish_delete` on success AND failure.
    deleting: DashSet<Uuid>,
    /// Broadcast ring capacity for rooms created by this registry.
    broadcast_capacity: usize,
}
```

Initialize `deleting: DashSet::new()` in `new()`/`with_capacity()`. At the top of `get_or_create` (before the fast path):

```rust
if self.deleting.contains(&world_id) {
    // Mid-deletion: refuse exactly like an absent world row.
    return Ok(None);
}
```

New methods beside `reap_if_empty`:

```rust
/// Begin a world deletion: tombstone the world (blocking room re-creation)
/// and unconditionally remove its live room, returning it so the caller can
/// broadcast the eviction frame. Every cache the world holds (navmesh,
/// engine, visible-cells, hecs world, ring) is Room-owned, so dropping the
/// last Arc frees them all. Pair with `finish_delete` on ALL exit paths.
pub fn begin_delete(&self, world_id: Uuid) -> Option<Arc<Room>> {
    self.deleting.insert(world_id);
    self.rooms.remove(&world_id).map(|(_, room)| room)
}

/// End a world deletion (success or failure), lifting the tombstone. After
/// a committed delete, re-creation is refused by the missing world row;
/// after a failure the world is live again and re-creation is legitimate.
pub fn finish_delete(&self, world_id: Uuid) {
    self.deleting.remove(&world_id);
}

/// Address every connection of `user` across all live rooms with a terminal
/// eviction frame (account deletion). Rooms without that user's connections
/// skip the frame in their egress loops.
pub fn evict_user(&self, user: Uuid) {
    for entry in self.rooms.iter() {
        entry
            .value()
            .broadcast_aux(ServerMsg::Evicted { user: Some(user) });
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**, plus the whole ws module: `cargo test --manifest-path src/server/Cargo.toml ws::` → PASS.

- [ ] **Step 5: Commit.** `git commit -m "feat(server): room-registry deletion tombstone + cross-room user eviction"` (add only the room.rs change).

---

### Task 3: Client eviction handling (wire mirror + terminal stop + route home)

**Files:**
- Modify: `src/client/core/src/wire.ts` (`ServerMsgSchema` union :215)
- Modify: `src/client/core/src/ws-client.ts` (handlers type, message switch :318)
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts` (WsClient handlers :370)
- Modify: `src/client/shell/src/App.svelte` (WorldSession construction; `leaveWorld` at :100)
- Test: `src/client/core/src/ws-client.test.ts`, `src/client/shell/src/lib/worldSession.test.ts`, `src/client/core/src/wire.test.ts` (drift guard, if it enumerates variants)

**Interfaces:**
- Consumes: wire frame `{"type":"evicted","user":string|null}` (Task 1).
- Produces: `WsClientOptions.handlers.onEvicted?: () => void`; `WorldSession` constructor option `onEvicted?: () => void`. On an evicted frame the client calls `stop()` (terminal — `running === false`, no reconnect) and then the handler.

- [ ] **Step 1: Write the failing tests.**

`ws-client.test.ts` (copy the file's existing fake-transport test shape — a `connect` fn that captures `handlers` and returns a transport):

```ts
test("evicted frame stops the client (no reconnect) and fires onEvicted", async () => {
  let push!: (frame: unknown) => void;
  let opens = 0;
  const connect: Connect = (handlers) => {
    opens += 1;
    push = (frame) => handlers.onMessage(JSON.stringify(frame));
    return Promise.resolve({ send: () => {}, close: () => handlers.onClose() });
  };
  const onEvicted = vi.fn();
  const client = new WsClient({ connect, handlers: { onEvicted } });
  await client.start();
  push({ type: "evicted", user: null });
  await Promise.resolve();
  expect(onEvicted).toHaveBeenCalledOnce();
  expect(client.running).toBe(false);
  expect(opens).toBe(1); // no reconnect attempt followed
});
```

(Adapt constructor arguments to the file's existing minimal-options fixture — `handlers` there may require other members; spread the fixture's defaults.)

`worldSession.test.ts` (using the existing `mockConnect`-style push pattern at :118-123):

```ts
test("evicted frame surfaces through onEvicted", async () => {
  let push!: (frame: unknown) => void;
  const connect: Connect = (handlers) => {
    push = (frame) => handlers.onMessage(JSON.stringify(frame));
    queueMicrotask(() => handlers.onMessage(JSON.stringify(welcomeFrame)));
    return Promise.resolve({ send: () => {}, close: () => handlers.onClose() });
  };
  const onEvicted = vi.fn();
  const session = new WorldSession({ selfId: "u1", connect, onEvicted, /* …the suite's other required opts… */ });
  await session.enter("w1");
  push({ type: "evicted", user: null });
  await Promise.resolve();
  expect(onEvicted).toHaveBeenCalledOnce();
});
```

- [ ] **Step 2: Run to verify they fail:** `pnpm --filter @shadowcat/core test`, `pnpm --filter @shadowcat/shell test` → FAIL (schema rejects the frame / option unknown).

- [ ] **Step 3: Implement.**

`wire.ts` — add to the `ServerMsgSchema` union (:215), with the same comment style as neighbours:

```ts
// Terminal eviction (world or account deletion); the server closes the
// socket right after. Terminal: the client must stop, not reconnect.
z.object({ type: z.literal("evicted"), user: z.string().nullable() }),
```

`ws-client.ts` — add to the handlers interface (beside `onError`):

```ts
/** Terminal eviction (world/account deleted). The client has already
 * stopped (no reconnect) when this fires; route the user out of the world. */
onEvicted?: () => void;
```

and in the message switch (beside `case "error":` at :321):

```ts
case "evicted":
  // Terminal: the server is deleting this world or account. Stop first
  // (running=false → the onClose path will not schedule a reconnect),
  // then let the shell route away.
  this.stop();
  this.safeEmit(() => this.opts.handlers.onEvicted?.());
  break;
```

`worldSession.svelte.ts` — add `onEvicted?: () => void;` to the session's options type, and wire it in `enter()`'s `handlers` block (beside `onError` at :387):

```ts
onEvicted: () => this.opts.onEvicted?.(),
```

`App.svelte` — in the `new WorldSession({ … })` construction, add:

```ts
onEvicted: () => leaveWorld(),
```

(`leaveWorld` at App.svelte:100 already does `session.leave()` + `navigate({ name: "worlds" })` — an evicted user lands on the world list, where the deleted world is gone on refresh.)

- [ ] **Step 4: Check the wire drift guard.** Open `src/client/core/src/wire.test.ts`; if it enumerates `ServerMsg` variants against the generated type, add `evicted`. Run the full gates: `pnpm -r typecheck && pnpm -r test` → PASS.

- [ ] **Step 5: Commit.** `git commit -m "feat(client): terminal handling for evicted frames (stop + route to world list)"`

---

### Task 4: Migration 0010 + `SqliteRepository::delete_world` (+ FTS-under-cascade pin)

**Files:**
- Create: `src/server/migrations/0010_explored_fog_world_index.sql`
- Modify: `src/server/src/data/sqlite.rs` (beside the five `world_*_key` fns at :2319-2341)
- Test: `src/server/src/data/sqlite.rs` tests module

**Interfaces:**
- Consumes: the five existing per-world settings key builders (`world_caps_key` … `world_modules_key`).
- Produces:
  - `fn world_settings_keys(world: Uuid) -> [String; 5]` (associated, same impl block as the key fns — match their `Self::`/free-fn form).
  - `pub async fn delete_world(&self, world: Uuid) -> Result<(), DataError>` — `NotFound` when no row; Task 5 calls it.

- [ ] **Step 1: Migration.** Create `src/server/migrations/0010_explored_fog_world_index.sql`:

```sql
-- World deletion (B1) purges explored_fog by world_id (the column 0007
-- denormalized for exactly this); index it so the purge is not a full scan.
-- No user_id index: user deletion is a rare admin op and the fog write path
-- is hot — a purge-by-user scan is the right trade.
CREATE INDEX idx_explored_fog_world ON explored_fog(world_id);
```

- [ ] **Step 2: Write the failing tests** in sqlite.rs's tests module (reuse the module's existing fixtures — `create_user`, world creation, `doc(perms, system)` helper, `set_explored`, the settings setters):

```rust
#[tokio::test]
async fn delete_world_removes_every_keyed_row() {
    // Arrange one world with: a member, a document with searchable text
    // (so FTS rows exist in BOTH split tables), a child document (parent_id),
    // an asset row, a world_events row (write via apply_intent), an invite,
    // an explored_fog row (set_explored), and all five settings blobs
    // (set_world_cap_defaults, set_world_cap_requirements,
    //  set_world_contract_declarations, set_world_schema_declarations,
    //  set_world_enabled_modules).
    // A SECOND world with one of each proves scoping (its rows survive).

    repo.delete_world(w1).await.expect("delete");

    // Assert zero rows WHERE world/scene/key matches w1 in: worlds,
    // world_members, documents, world_events, assets, world_invites,
    // explored_fog, settings — and, THE PIN, zero rows in
    // documents_fts_public AND documents_fts_gm for w1's doc ids
    // (FTS delete triggers under FK cascade, bundled-SQLite behavior).
    // Assert w2's rows all survive.
}

#[tokio::test]
async fn delete_world_not_found() {
    assert!(matches!(
        repo.delete_world(Uuid::new_v4()).await,
        Err(DataError::NotFound)
    ));
}
```

- [ ] **Step 3: Run to verify they fail:** `cargo test --manifest-path src/server/Cargo.toml delete_world` → FAIL (method missing).

- [ ] **Step 4: Implement** (beside the settings key fns; match their associated/free form):

```rust
/// The per-world `settings` keys. SINGLE SOURCE for "what world-scoped
/// settings blobs exist": `delete_world`'s purge iterates this array, so a
/// new per-world blob added here is purged automatically (never-fork; adding
/// a sixth key fn without extending this array is the drift this prevents).
fn world_settings_keys(world: Uuid) -> [String; 5] {
    [
        Self::world_caps_key(world),
        Self::world_caps_req_key(world),
        Self::world_contracts_key(world),
        Self::world_schemas_key(world),
        Self::world_modules_key(world),
    ]
}

/// Delete a world and every row keyed to it, in one transaction. FK cascades
/// cover world_members/documents/world_events/assets/world_invites, and the
/// FTS AFTER DELETE triggers fire under cascade (pinned by test).
/// `explored_fog` and the per-world `settings` blobs have no FK and are
/// purged explicitly. Files on disk are the caller's concern — delete
/// ordering is rows-first, files-second (http/assets.rs delete convention).
pub async fn delete_world(&self, world: Uuid) -> Result<(), DataError> {
    let mut tx = self.pool.begin().await?;
    let res = sqlx::query("DELETE FROM worlds WHERE id = ?")
        .bind(world.to_string())
        .execute(&mut *tx)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DataError::NotFound);
    }
    sqlx::query("DELETE FROM explored_fog WHERE world_id = ?")
        .bind(world.to_string())
        .execute(&mut *tx)
        .await?;
    for key in Self::world_settings_keys(world) {
        sqlx::query("DELETE FROM settings WHERE key = ?")
            .bind(key)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
```

- [ ] **Step 5: Run tests to verify they pass**, then the full crate: `cargo test --manifest-path src/server/Cargo.toml` → PASS.

- [ ] **Step 6: Commit.** `git commit -m "feat(server): transactional world deletion with fog/settings purge and FTS-cascade pin"`

---

### Task 5: `DELETE /api/worlds/{id}` route

**Files:**
- Modify: `src/server/src/http/routes.rs` (new handler beside `create_world` :490)
- Modify: `src/server/src/http/mod.rs` (route table — extend the `/api/worlds/{id}/…` block's sibling `/api/worlds` area)
- Test: `src/server/src/http/mod.rs` tests module

**Interfaces:**
- Consumes: `require_gm` (routes.rs:441 — admins resolve to GM, so this IS "server admin or that world's GM"), `RoomRegistry::{begin_delete, finish_delete}` (Task 2), `ServerMsg::Evicted` (Task 1), `repo.delete_world` (Task 4), `state.write_barrier`, `state.config.assets_path()`.
- Produces: `DELETE /api/worlds/{id}` → 204 | 403 (non-GM member & non-member) | 404 (admin on unknown world) — Task 11's UI consumes it.

- [ ] **Step 1: Write the failing tests** in http/mod.rs tests (harness: `initialized_state`, `server_with_user`, `seed_admin`, `login_server`, `.save_cookies()`):

```rust
#[tokio::test]
async fn world_delete_authz_matrix() {
    // GM of the world → 204; world row gone.
    // Server admin who is NOT a member → 204 on another world.
    // Player member → 403; row survives.
    // Non-member non-admin → 403.
    // Admin, unknown world id → 404.
}

#[tokio::test]
async fn world_delete_removes_asset_dir_and_room() {
    // Seed a world with one uploaded asset (multipart, as tests/assets.rs
    // does) so <assets_path>/<world>/ exists on disk; join it over a real WS
    // transport so a live room exists.
    // DELETE → 204. Assert: assets dir for the world is gone
    // (tokio::fs::metadata → NotFound), state.ws.rooms.get(world) is None,
    // and a fresh get_or_create returns Ok(None) (row gone, tombstone lifted
    // — i.e. finish_delete ran).
    // Also: the connected socket received evicted{user:null} then closed
    // (reuse Task 1's read pattern).
}

#[tokio::test]
async fn world_delete_failure_lifts_tombstone() {
    // Admin DELETE on an unknown world → 404, and afterwards
    // get_or_create for an EXISTING world id is unaffected; for the unknown
    // id, rooms is not tombstoned (call begin-delete-free path: create a
    // real world, delete it twice — second delete → 404 — then create a NEW
    // world and confirm get_or_create works; assert
    // state.ws.rooms.get(unknown) is None and no refusal for live worlds).
}
```

- [ ] **Step 2: Run to verify they fail:** `cargo test --manifest-path src/server/Cargo.toml world_delete` → FAIL (404 route missing).

- [ ] **Step 3: Implement the handler** in routes.rs (import `crate::ws::protocol::ServerMsg` following assets.rs's usage):

```rust
/// DELETE /api/worlds/{id} — server admin or that world's GM (`require_gm`
/// resolves admins to GM; one symbol, no authz fork). Ordering: tombstone +
/// evict the live room FIRST (no new joins, existing connections get a
/// terminal frame), then one DB transaction, then the asset directory —
/// delete convention: rows first, files second, so a crash orphans files on
/// disk rather than leaving a live world missing them. The barrier read side
/// spans the commit + dir removal so a backup never snapshots half a delete.
pub async fn delete_world(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    let room = state.ws.rooms.begin_delete(world);
    if let Some(room) = &room {
        room.broadcast_aux(ServerMsg::Evicted { user: None });
    }
    // Everything below must lift the tombstone on the way out.
    let result = async {
        let _read_permit = state.write_barrier.read().await;
        state.repo.delete_world(world).await?;
        let dir = state.config.assets_path().join(world.to_string());
        match tokio::fs::remove_dir_all(&dir).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                tracing::warn!(?e, %world, "world asset dir removal failed after row delete")
            }
        }
        Ok::<(), AppError>(())
    }
    .await;
    state.ws.rooms.finish_delete(world);
    result?;
    Ok(StatusCode::NO_CONTENT)
}
```

Route registration in http/mod.rs (beside the existing `/api/worlds` entry):

```rust
.route("/api/worlds/{id}", delete(routes::delete_world))
```

- [ ] **Step 4: Run the tests to verify they pass**, then the whole crate + fmt: `cargo test --manifest-path src/server/Cargo.toml && cargo fmt --check --manifest-path src/server/Cargo.toml` → PASS.

- [ ] **Step 5: Commit.** `git commit -m "feat(server): DELETE /api/worlds/{id} with room eviction and asset-dir removal"`

---

### Task 6: Migration 0011 — `assets.created_by` → nullable `ON DELETE SET NULL`

**Files:**
- Create: `src/server/migrations/0011_assets_created_by_set_null.sql`
- Modify: `src/server/src/data/asset.rs` (:17), `src/server/src/data/sqlite.rs` (`insert_asset` :125-143, `asset_from_row` :145-160), `src/server/src/http/assets.rs` (:204)
- Regenerated: `src/types/generated/Asset.ts`
- Test: `src/server/src/data/sqlite.rs` tests module

**Interfaces:**
- Consumes: nothing new.
- Produces: `Asset.created_by: Option<Uuid>` (`Asset.ts` → `created_by: string | null`); user deletion no longer FK-fails on authored assets. Task 7 relies on the SET NULL.

- [ ] **Step 1: Migration.** Create `src/server/migrations/0011_assets_created_by_set_null.sql`:

```sql
-- User deletion (B2): `created_by` becomes nullable with ON DELETE SET NULL.
-- Previously NOT NULL + implicit NO ACTION, which made any account that had
-- ever uploaded an asset undeletable (FK violation). SQLite cannot alter a
-- constraint in place; `assets` has no child tables, so the plain rebuild is
-- safe under foreign_keys=ON with no PRAGMA toggling. idx_assets_world drops
-- with the table and must be recreated.
CREATE TABLE assets_new (
  id            TEXT PRIMARY KEY,
  world_id      TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
  storage_key   TEXT NOT NULL,
  original_name TEXT NOT NULL,
  content_type  TEXT NOT NULL,
  byte_size     INTEGER NOT NULL,
  created_by    TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at    INTEGER NOT NULL,
  version       INTEGER NOT NULL
);
INSERT INTO assets_new
  SELECT id, world_id, storage_key, original_name, content_type, byte_size,
         created_by, created_at, version
  FROM assets;
DROP TABLE assets;
ALTER TABLE assets_new RENAME TO assets;
CREATE INDEX idx_assets_world ON assets(world_id);
```

- [ ] **Step 2: Write the failing test** in sqlite.rs tests:

```rust
#[tokio::test]
async fn user_delete_nulls_asset_created_by() {
    // create_user + world + insert_asset with created_by: Some(user).
    // Raw `DELETE FROM users WHERE id = ?` on the pool (repo.delete_user is
    // Task 7; this test pins the FK action itself).
    // get_asset → created_by is None; row otherwise intact.
}
```

- [ ] **Step 3: Run to verify it fails:** compile error once the struct changes, or FK violation under the old schema — either failure mode is acceptable evidence. `cargo test --manifest-path src/server/Cargo.toml user_delete_nulls_asset` → FAIL.

- [ ] **Step 4: Implement the type ripple.**
  - `asset.rs:17`: `pub created_by: Option<Uuid>,` with doc line: `/// NULL when the uploading account has been deleted.`
  - `sqlite.rs` `insert_asset`: `.bind(a.created_by.map(|u| u.to_string()))`
  - `sqlite.rs` `asset_from_row`: read `Option<String>` and parse through the function's existing uuid-parse idiom, e.g. `row.get::<Option<String>, _>("created_by").map(|s| parse(&s)).transpose()?` — mirror the surrounding style exactly.
  - `http/assets.rs:204` (upload): `created_by: Some(user.id),`
  - Fix any other compile sites the change surfaces (`cargo check` drives this; the audit found exactly four production sites, plus tests `sqlite.rs:2870` and `tests/assets.rs:85`).

- [ ] **Step 5: Run + regenerate + client gate:** `cargo test --manifest-path src/server/Cargo.toml` → PASS (regenerates `Asset.ts` → `created_by: string | null`); `pnpm -r typecheck && pnpm -r test` → PASS (the three client fixtures pass strings, which satisfy `string | null`).

- [ ] **Step 6: Commit.** `git commit -m "feat(server): assets.created_by nullable with ON DELETE SET NULL (user-deletion prereq)"`

---

### Task 7: `SqliteRepository::delete_user` + last-admin guard + stale comments

**Files:**
- Modify: `src/server/src/data/sqlite.rs` (beside `is_last_gm` :289 and `create_admin_if_none` :645)
- Test: `src/server/src/data/sqlite.rs` tests module

**Interfaces:**
- Consumes: `ServerRole::as_str()` (`auth/role.rs`), SET NULL FKs (0001/0009/0011), `SqlxSqliteStore::{new, migrate}` + `SessionStore::save` (session.rs — tests only).
- Produces: `pub async fn delete_user(&self, target: Uuid) -> Result<(), DataError>` — `NotFound` unknown; `Conflict("cannot delete the server's only administrator")`; Task 8 calls it.

- [ ] **Step 1: Write the failing tests** in sqlite.rs tests:

```rust
#[tokio::test]
async fn delete_user_scrubs_everything() {
    // Seed: admin A (create_admin_if_none or seed pattern), user U.
    // U: member of a world, owner of a document, author of a world_events row
    // (apply_intent as U), an asset (created_by U), an explored_fog row, and
    // a session row — build the session via SqlxSqliteStore::new(repo.pool()
    // .clone()); store.migrate().await; then store.save(...) a Record whose
    // data map has "user" → serde_json::json!({"id": U, "username": "u",
    // "role": "user"}) (mirroring SESSION_USER_KEY's shape in auth/session.rs).
    // Also a session for A (must survive).

    repo.delete_user(u).await.expect("delete");

    // users row gone; world_members rows gone; documents.owner_id IS NULL;
    // world_events.author_id IS NULL; assets.created_by IS NULL;
    // explored_fog rows for U gone (another user's fog survives);
    // tower_sessions: U's row gone, A's row survives.
}

#[tokio::test]
async fn delete_user_guards() {
    // Unknown id → NotFound.
    // Sole admin → Conflict (message: "cannot delete the server's only
    // administrator").
    // With TWO admins, deleting one → Ok.
}
```

- [ ] **Step 2: Run to verify they fail:** `cargo test --manifest-path src/server/Cargo.toml delete_user` → FAIL.

- [ ] **Step 3: Implement** (beside `is_last_gm`; import `ServerRole` as the file already does for role strings):

```rust
/// Whether `user` is the server's sole administrator, evaluated on the
/// supplied tx connection for the same TOCTOU reason as `is_last_gm`: the
/// count check and the delete must be one atomic unit on the single-writer
/// pool, or two concurrent deletes could each pass the check.
async fn is_last_admin(
    tx: &mut sqlx::SqliteConnection,
    user: Uuid,
) -> Result<bool, DataError> {
    let target: Option<String> =
        sqlx::query_scalar("SELECT server_role FROM users WHERE id = ?")
            .bind(user.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    if target.as_deref() != Some(crate::auth::role::ServerRole::Admin.as_str()) {
        return Ok(false);
    }
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE server_role = ?")
        .bind(crate::auth::role::ServerRole::Admin.as_str())
        .fetch_one(&mut *tx)
        .await?;
    Ok(n <= 1)
}

/// Delete a user account and everything keyed to it, in one transaction:
/// memberships CASCADE; documents.owner_id / world_events.author_id /
/// world_invites.{created_by,consumed_by} SET NULL; assets.created_by SET
/// NULL (0011); explored_fog rows (no FK; unindexed scan — rare admin op)
/// and live sessions are purged explicitly. Sessions MUST die in this same
/// transaction: `AuthUser` trusts the session record without re-reading
/// `users`, so a surviving row keeps a deleted account authenticated until
/// cookie expiry. Refuses to delete the last administrator.
/// Implicit coupling: `tower_sessions` is created by `SqlxSqliteStore::
/// migrate` at boot (main.rs), before any route can reach this; repo-level
/// tests must run that migrate themselves.
pub async fn delete_user(&self, target: Uuid) -> Result<(), DataError> {
    let mut tx = self.pool.begin().await?;
    if Self::is_last_admin(&mut tx, target).await? {
        return Err(DataError::Conflict(
            "cannot delete the server's only administrator".into(),
        ));
    }
    let res = sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(target.to_string())
        .execute(&mut *tx)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DataError::NotFound);
    }
    sqlx::query("DELETE FROM explored_fog WHERE user_id = ?")
        .bind(target.to_string())
        .execute(&mut *tx)
        .await?;
    // Session identity lives at $.data.user.id inside the JSON blob (the
    // store has no user_id column); JSON1 ships in the bundled SQLite.
    sqlx::query("DELETE FROM tower_sessions WHERE json_extract(data, '$.data.user.id') = ?")
        .bind(target.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
```

(Adjust the `ServerRole` path to the file's existing import; if `as_str` doesn't exist, bind the same serde-string idiom the file uses for `WorldRole`.)

- [ ] **Step 4: Update the two stale live-code comments** (do NOT touch migration files):
  - `create_admin_if_none` (sqlite.rs:645-657): replace the "Unreachable while no delete-user or demote-admin route exists…" sentence with: `/// Reachable since B2's DELETE /api/users/{id}: deletion is last-admin-guarded, so "users exist but no admin" still cannot arise — the NOCASE guard below stays as the structural backstop.`
  - `set_explored` (sqlite.rs:517-524): replace "not yet wired — worlds aren't deletable" with a pointer: `// Purged by delete_world (world-scoped), delete_user (user-scoped), and delete_document_tx (scene-scoped).`

- [ ] **Step 5: Run tests to verify they pass**, then the full crate: `cargo test --manifest-path src/server/Cargo.toml` → PASS.

- [ ] **Step 6: Commit.** `git commit -m "feat(server): transactional user deletion with last-admin guard and session revocation"`

---

### Task 8: `DELETE /api/users/{id}` route + live kick

**Files:**
- Modify: `src/server/src/http/routes.rs` (beside `create_user`/`list_users` :347/:383)
- Modify: `src/server/src/http/mod.rs` (route table, beside `/api/users` :83-86)
- Test: `src/server/src/http/mod.rs` tests module

**Interfaces:**
- Consumes: `AdminUser` extractor (auth/session.rs:198 — server-tier gate; never `require_gm`, which any world GM satisfies), `repo.delete_user` (Task 7), `RoomRegistry::evict_user` (Task 2).
- Produces: `DELETE /api/users/{id}` → 204 | 403 (non-admin) | 404 (unknown) | 409 (self-delete; last-admin). Task 12's UI consumes it.

- [ ] **Step 1: Write the failing tests** in http/mod.rs tests:

```rust
#[tokio::test]
async fn user_delete_authz_and_guards() {
    // Admin deletes an ordinary user → 204; row gone.
    // Admin deletes THEMSELVES → 409 {"error":"cannot delete your own account"}.
    // Non-admin caller → 403.
    // Admin, unknown id → 404.
}

#[tokio::test]
async fn user_delete_revokes_sessions_and_kicks() {
    // Login as user U on a second cookie-jar server (login_server); verify
    // GET /api/me → 200 with U's cookie. Connect U to a world over a real
    // WS transport (Welcome consumed).
    // Admin DELETE /api/users/{U} → 204.
    // U's cookie: GET /api/me → 401 (session died in the tx).
    // U's socket: next frame is evicted{user:U}, then close (Task 1 pattern).
}
```

- [ ] **Step 2: Run to verify they fail:** `cargo test --manifest-path src/server/Cargo.toml user_delete` → FAIL.

- [ ] **Step 3: Implement:**

```rust
/// DELETE /api/users/{id} — server-admin only (`AdminUser`: server tier,
/// never a world-role check). Self-deletion is refused so the operation
/// never has to answer "who revokes the caller's own live session
/// mid-request" — another admin performs it; the in-tx last-admin guard
/// (repo) remains the structural backstop. After commit, live connections
/// are kicked across every room; the account's cookies died inside the same
/// transaction, so a reconnect fails authentication.
pub async fn delete_user(
    admin: AdminUser,
    State(state): State<AppState>,
    Path(target): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    if admin.0.id == target {
        return Err(AppError::Conflict("cannot delete your own account".into()));
    }
    state.repo.delete_user(target).await?;
    state.ws.rooms.evict_user(target);
    Ok(StatusCode::NO_CONTENT)
}
```

Route registration:

```rust
.route("/api/users/{id}", delete(routes::delete_user))
```

- [ ] **Step 4: Run the tests to verify they pass**, then the whole crate + fmt → PASS.

- [ ] **Step 5: Commit.** `git commit -m "feat(server): DELETE /api/users/{id} with cross-room kick"`

---

### Task 9 (B3): Unified document-delete side-effects + scene fog purge

**Files:**
- Modify: `src/server/src/data/sqlite.rs` (`apply_intent` Delete branch :1887-1894, `apply_command` Delete branch :1411-1419, new helper beside `delete_document_fts` :1212)
- Test: `src/server/src/data/sqlite.rs` tests module

**Interfaces:**
- Consumes: `delete_document_fts(conn, id)` (:1212).
- Produces: `async fn delete_document_tx(tx: &mut sqlx::SqliteConnection, id: Uuid) -> Result<(), DataError>` — the ONLY place document-delete side-effects live.

- [ ] **Step 1: Write the failing tests:**

```rust
#[tokio::test]
async fn scene_delete_purges_fog_via_apply_intent() {
    // World + scene doc + a token child + set_explored rows for the scene
    // (two users) + a fog row for a DIFFERENT scene (must survive).
    // apply_intent(Operation::Delete{ scene doc }) as GM.
    // explored_fog: zero rows for the deleted scene_id; other scene intact.
    // (Descendant expansion already covered by
    //  deleting_a_scene_expands_to_descendant_delete_ops :2615.)
}

#[tokio::test]
async fn scene_delete_purges_fog_via_apply_command() {
    // Same arrangement, driven through apply_command (the trusted
    // undo/replay substrate) — pins the never-fork parity of the two paths.
}
```

- [ ] **Step 2: Run to verify they fail:** `cargo test --manifest-path src/server/Cargo.toml scene_delete_purges` → FAIL.

- [ ] **Step 3: Implement the helper** (beside `delete_document_fts`):

```rust
/// Apply a document Delete inside `tx`: the row, its FTS entries, and its
/// explored-fog rows. SINGLE SOURCE for delete side-effects — BOTH
/// authoritative delete paths (`apply_intent`, `apply_command`) call this,
/// so they cannot drift (never-fork). The fog purge is unconditional by id:
/// only scene documents ever appear as `explored_fog.scene_id`, so it is a
/// no-op for every other doc_type and carries no doc_type predicate that
/// could drift from the fog writer's keying.
async fn delete_document_tx(
    tx: &mut sqlx::SqliteConnection,
    id: Uuid,
) -> Result<(), DataError> {
    sqlx::query("DELETE FROM documents WHERE id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    Self::delete_document_fts(&mut *tx, id).await?;
    sqlx::query("DELETE FROM explored_fog WHERE scene_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(())
}
```

Replace **both** Delete-apply bodies (`apply_intent` :1887-1894 and `apply_command` :1411-1419): the two statements (`DELETE FROM documents…` + `delete_document_fts`) become one call `Self::delete_document_tx(&mut tx, doc.id).await?;` — keep each branch's surrounding `normalized_ops.push(...)` untouched.

- [ ] **Step 4: Run the new tests + the full crate** (the existing delete/FTS tests prove no regression) → PASS.

- [ ] **Step 5: Commit.** `git commit -m "fix(server): unify document-delete side-effects; purge scene fog on delete"`

---

### Task 10 (B4): `upsert_member` — resolve + guard + write in one transaction

**Files:**
- Modify: `src/server/src/data/sqlite.rs` (beside `add_member` :756)
- Modify: `src/server/src/http/routes.rs` (`add_member` handler :549-576)
- Test: `src/server/src/data/sqlite.rs` + `src/server/src/http/mod.rs` tests

**Interfaces:**
- Consumes: `is_last_gm(tx, world, user)` (:289), the `WorldRole` serde-string idiom.
- Produces: `pub async fn upsert_member(&self, world: Uuid, user: Uuid, role: WorldRole) -> Result<(), DataError>` — `NotFound` when the user OR world doesn't exist; `Conflict` on sole-GM demotion. The handler keeps its `POST /api/worlds/{id}/members` contract (404 unknown user — now also unknown world instead of an FK 500 — 409 demotion, 204 success).

- [ ] **Step 1: Write the failing repo tests:**

```rust
#[tokio::test]
async fn upsert_member_inserts_updates_and_guards() {
    // New member insert → role readable via member_role.
    // Same call again with a different role → role updated (upsert).
    // Demoting the world's ONLY GM → Conflict("cannot demote the world's only GM").
    // Promoting a second GM, then demoting the first → Ok.
    // Unknown user id → NotFound. Unknown world id → NotFound.
}
```

- [ ] **Step 2: Run to verify it fails** → FAIL (method missing).

- [ ] **Step 3: Implement:**

```rust
/// Add a member or change an existing member's role — resolve, guard, and
/// write in ONE transaction (the standalone user_exists → member_role →
/// set_role/add_member sequence was a TOCTOU: a user deleted between the
/// check and the insert resurfaces the FK 500 the 404 contract exists to
/// prevent). The guarded INSERT..SELECT proves user AND world existence
/// atomically with the upsert: rows_affected == 0 ⇔ target user or world
/// missing → NotFound. The sole-GM demotion guard runs on the same tx.
pub async fn upsert_member(
    &self,
    world: Uuid,
    user: Uuid,
    role: WorldRole,
) -> Result<(), DataError> {
    let mut tx = self.pool.begin().await?;
    if role != WorldRole::Gm && Self::is_last_gm(&mut tx, world, user).await? {
        return Err(DataError::Conflict(
            "cannot demote the world's only GM".into(),
        ));
    }
    let role_s = serde_json::to_value(role)?.as_str().unwrap().to_string();
    let res = sqlx::query(
        "INSERT INTO world_members (world_id, user_id, role) \
         SELECT ?, ?, ? \
         WHERE EXISTS (SELECT 1 FROM users WHERE id = ?) \
           AND EXISTS (SELECT 1 FROM worlds WHERE id = ?) \
         ON CONFLICT(world_id, user_id) DO UPDATE SET role = excluded.role",
    )
    .bind(world.to_string())
    .bind(user.to_string())
    .bind(role_s)
    .bind(user.to_string())
    .bind(world.to_string())
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DataError::NotFound);
    }
    tx.commit().await?;
    Ok(())
}
```

Rewrite the handler body (routes.rs:549-576) — keep the request struct and the FK-vs-404 doc comment, updated:

```rust
pub async fn add_member(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
    Json(body): Json<AddMemberRequest>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    state.repo.upsert_member(world, body.user, body.role).await?;
    Ok(StatusCode::NO_CONTENT)
}
```

- [ ] **Step 4: Dead-code check.** Grep `user_exists` callers; if `add_member` was the only production caller, remove the method and migrate any tests onto `upsert_member`; if other callers exist, leave it.

- [ ] **Step 5: Run repo tests + the existing HTTP membership tests** (they pin the 404/409 contract through the route) + full crate → PASS.

- [ ] **Step 6: Commit.** `git commit -m "fix(server): add_member resolve+guard+write in one transaction"`

---

### Task 11 (B5a): World delete UI — entry world list

**Files:**
- Modify: `src/modules/entry/src/entryApi.ts`, `src/modules/entry/src/views/WorldSelect.svelte`
- Modify: `src/client/ui-kit/src/locales/en.ts` (`worlds.*` block :14-22), `src/client/shell/src/styles/_semantic.scss` (:15)
- Test: `src/modules/entry/src/entryApi.test.ts`, `src/modules/entry/src/views/WorldSelect.test.ts`

**Interfaces:**
- Consumes: `DELETE /api/worlds/{id}` (Task 5); `WorldEntry.role` — admins see every world with `role: "gm"` (sqlite.rs:388-430), so `world.role === "gm"` gates the button to exactly B1's authz set with no `/api/me` widening.
- Produces: `export async function deleteWorld(id: string): Promise<void>` in entryApi (entry keeps zero `@shadowcat/core` deps); `--on-danger` semantic token; the `.danger`/`.danger-outline` button styles.

- [ ] **Step 1: Failing API test** in `entryApi.test.ts` (the file's `vi.spyOn(globalThis, "fetch")` pattern):

```ts
test("deleteWorld issues DELETE and throws on failure", async () => {
  const fetchMock = vi
    .spyOn(globalThis, "fetch")
    .mockResolvedValue(new Response(null, { status: 204 }));
  await deleteWorld("w1");
  expect(fetchMock).toHaveBeenCalledWith("/api/worlds/w1", { method: "DELETE" });
  fetchMock.mockResolvedValue(new Response(null, { status: 403 }));
  await expect(deleteWorld("w1")).rejects.toThrow();
});
```

- [ ] **Step 2: Failing component tests** in `WorldSelect.test.ts` (English-string assertions — entry uses the real catalog):

```ts
test("delete affordance is gm-only and arms on exact name", async () => {
  vi.spyOn(api, "listWorlds").mockResolvedValue([
    { id: "w1", name: "Alpha", role: "gm" },
    { id: "w2", name: "Beta", role: "player" },
  ]);
  const del = vi.spyOn(api, "deleteWorld").mockResolvedValue(undefined);
  render(WorldSelect, { props: { onEnter: vi.fn() } });
  expect(await screen.findAllByRole("button", { name: "Delete", exact: true })).toHaveLength(1); // gm row only
  await fireEvent.click(screen.getByRole("button", { name: "Delete", exact: true }));
  const confirm = screen.getByRole("button", { name: "Delete forever" });
  expect(confirm).toBeDisabled();
  await fireEvent.input(screen.getByLabelText(/Type the world name/), { target: { value: "Alph" } });
  expect(confirm).toBeDisabled();
  await fireEvent.input(screen.getByLabelText(/Type the world name/), { target: { value: "Alpha" } });
  expect(confirm).not.toBeDisabled();
  await fireEvent.click(confirm);
  expect(del).toHaveBeenCalledWith("w1");
});

test("delete failure shows the delete error", async () => {
  // listWorlds one gm world; deleteWorld rejects; arm + confirm;
  // getByRole("alert").textContent === "Could not delete the world."
});
```

- [ ] **Step 3: Run to verify both fail:** `pnpm --filter @shadowcat/entry test` → FAIL.

- [ ] **Step 4: Implement.**

`entryApi.ts` (beside `createWorld`):

```ts
export async function deleteWorld(id: string): Promise<void> {
  const res = await fetch(`/api/worlds/${encodeURIComponent(id)}`, { method: "DELETE" });
  if (!res.ok) throw new Error(`/api/worlds/${id} → ${res.status}`);
}
```

`en.ts` (`worlds.*` block):

```ts
"worlds.delete": "Delete",
"worlds.deleteTypeName": "Type the world name to confirm deletion",
"worlds.deleteConfirm": "Delete forever",
"worlds.errorDelete": "Could not delete the world.",
```

`_semantic.scss` (beside `--danger` at :15):

```scss
--on-danger: var(--slate-950); // text on a filled danger control (AA on --red-500)
```

`WorldSelect.svelte` — script additions:

```ts
import { listWorlds, createWorld, acceptInvite, deleteWorld } from "../entryApi";

let confirmingDelete = $state<string | null>(null);
let deleteName = $state("");
let deleting = $state(false);

function armDelete(id: string) {
  confirmingDelete = confirmingDelete === id ? null : id;
  deleteName = "";
  error = "";
}

async function confirmDelete(world: WorldEntry) {
  if (deleteName !== world.name || deleting) return;
  deleting = true;
  error = "";
  try {
    await deleteWorld(world.id);
    confirmingDelete = null;
    deleteName = "";
    await refresh();
  } catch {
    error = t("worlds.errorDelete");
  } finally {
    deleting = false;
  }
}
```

Row markup (replacing the single-button `<li>`; the enter button keeps the `{world.name} <small>({world.role})</small>` label so the e2e's name-regex selector still matches exactly one button — the delete labels contain no world name):

```svelte
<li>
  <div class="row">
    <button class="enter" onclick={() => onEnter(world.id)}>
      {world.name} <small>({world.role})</small>
    </button>
    {#if world.role === "gm"}
      <button class="danger-outline" onclick={() => armDelete(world.id)}>
        {t("worlds.delete")}
      </button>
    {/if}
  </div>
  {#if confirmingDelete === world.id}
    <form
      class="confirm-delete"
      onsubmit={(e) => {
        e.preventDefault();
        confirmDelete(world);
      }}
    >
      <label>
        {t("worlds.deleteTypeName")}
        <input bind:value={deleteName} placeholder={world.name} />
      </label>
      <button type="submit" class="danger" disabled={deleteName !== world.name || deleting}>
        {t("worlds.deleteConfirm")}
      </button>
    </form>
  {/if}
</li>
```

Styles (scoped block; **rescope the existing `li button { width: 100%; … }` rules to `.enter`**, keep their surface/hover values):

```scss
li .row {
  display: flex;
  gap: var(--space-2);
  align-items: stretch;
}
li .row .enter {
  flex: 1;
  /* existing li button rules move here unchanged */
}
.confirm-delete {
  display: flex;
  gap: var(--space-2);
  align-items: end;
  margin-top: var(--space-2);
}
button.danger-outline {
  background: transparent;
  border: 1px solid var(--danger);
  color: var(--danger);
}
button.danger {
  background: var(--danger);
  border: 1px solid var(--danger);
  color: var(--on-danger);
}
button.danger:disabled {
  opacity: 0.5;
}
@media (pointer: coarse) {
  button.danger,
  button.danger-outline {
    min-height: var(--input-height-coarse);
  }
}
```

- [ ] **Step 5: Run the gates:** `pnpm -r test && pnpm -r typecheck && pnpm lint` → PASS.

- [ ] **Step 6: Commit.** `git commit -m "feat(entry): world deletion with type-the-name confirm"`

---

### Task 12 (B5b): User delete UI — admin user manager

**Files:**
- Modify: `src/client/core/src/user-rest.ts` (beside `createUser` :48), `src/client/core/src/index.ts` (:81 barrel)
- Modify: `src/modules/settings/src/UserManager.svelte`
- Modify: `src/client/ui-kit/src/locales/en.ts` (`settings.users.*` block :51-58)
- Test: `src/client/core/src/user-rest.test.ts`, `src/modules/settings/src/UserManager.test.ts`

**Interfaces:**
- Consumes: `DELETE /api/users/{id}` (Task 8; a 409's message — last-admin guard — surfaces verbatim via `restError`), `AppContext.selfId`.
- Produces: `export async function deleteUser(id: string): Promise<void>` (exported from the core barrel).

- [ ] **Step 1: Failing REST test** in `user-rest.test.ts` (the file's fetch-mock idiom; mirror the `revokeWorldInvite` test at :169):

```ts
test("deleteUser issues DELETE and surfaces the server error body", async () => {
  const fetchMock = vi
    .spyOn(globalThis, "fetch")
    .mockResolvedValue(new Response(null, { status: 204 }));
  await deleteUser("u-2");
  expect(fetchMock.mock.calls[0][0]).toBe("/api/users/u-2");
  expect((fetchMock.mock.calls[0][1] as RequestInit).method).toBe("DELETE");
  fetchMock.mockResolvedValue(
    new Response(JSON.stringify({ error: "cannot delete the server's only administrator" }), { status: 409 }),
  );
  await expect(deleteUser("u-2")).rejects.toThrow("cannot delete the server's only administrator");
});
```

- [ ] **Step 2: Failing component tests** in `UserManager.test.ts` — extend the `vi.mock("@shadowcat/core", …)` factory (:6-18) with `deleteUser: vi.fn()`, preserving the non-admin fetches-nothing test:

```ts
test("delete asks for confirmation, deletes, reloads; self row has no delete", async () => {
  vi.mocked(core.listUsers).mockResolvedValue([
    { id: "u-self", username: "me", server_role: "admin" },
    { id: "u-2", username: "bob", server_role: "user" },
  ]);
  vi.mocked(core.deleteUser).mockResolvedValue(undefined);
  vi.spyOn(window, "confirm").mockReturnValue(true);
  render(UserManager, { context: setAppContextForTest({ role: "player", serverRole: "admin" }) });
  // selfId defaults to "u-self" in the fixture → exactly ONE delete button.
  const buttons = await screen.findAllByRole("button", { name: "settings.users.delete" });
  expect(buttons).toHaveLength(1);
  await fireEvent.click(buttons[0]);
  expect(window.confirm).toHaveBeenCalledWith("settings.users.deleteConfirm");
  expect(core.deleteUser).toHaveBeenCalledWith("u-2");
  expect(core.listUsers).toHaveBeenCalledTimes(2); // initial + reload
});

test("declined confirm does not delete; failure shows deleteError", async () => {
  // confirm → false: deleteUser never called.
  // confirm → true + deleteUser rejects(new Error("nope")):
  //   await screen.findByText("settings.users.deleteError") visible.
});
```

- [ ] **Step 3: Run to verify they fail** → FAIL.

- [ ] **Step 4: Implement.**

`user-rest.ts` (beside `createUser`, using the file's `restError`):

```ts
/** Delete a user account (server-admin only). The server refuses self-
 * deletion and deleting the last administrator with a 409 whose message is
 * client-actionable — surface it verbatim. */
export async function deleteUser(id: string): Promise<void> {
  const res = await fetch(`/api/users/${encodeURIComponent(id)}`, { method: "DELETE" });
  if (!res.ok) throw new Error(await restError(res, "delete user failed"));
}
```

Barrel `index.ts:81`: add `deleteUser` to the `user-rest` export list.

`en.ts` (`settings.users.*` block):

```ts
"settings.users.delete": "Delete",
"settings.users.deleteConfirm": "Delete account {username}? This cannot be undone.",
"settings.users.deleteError": "Could not delete account: {message}",
```

`UserManager.svelte` — destructure `selfId` too (`const { t, serverRole, selfId } = getAppContext();`), add state + handler:

```ts
let deleteError = $state<string | null>(null);

async function removeUser(u: ServerUser) {
  if (!window.confirm(t("settings.users.deleteConfirm", { username: u.username }))) return;
  busy = true;
  deleteError = null;
  try {
    await deleteUser(u.id);
    await load();
  } catch (e) {
    deleteError = e instanceof Error ? e.message : String(e);
  } finally {
    busy = false;
  }
}
```

List markup (:85-87 becomes; the self row hides delete — advisory only, the server's 409 is the real guard):

```svelte
{#each users as u (u.id)}
  <li>
    <span>{u.username} <span class="tier">{u.server_role}</span></span>
    {#if u.id !== selfId}
      <button class="danger-outline" onclick={() => removeUser(u)} disabled={busy}>
        {t("settings.users.delete")}
      </button>
    {/if}
  </li>
{/each}
```

After the existing error paragraph:

```svelte
{#if deleteError}
  <p class="error">{t("settings.users.deleteError", { message: deleteError })}</p>
{/if}
```

Styles: `li { display: flex; gap: var(--space-2); align-items: center; justify-content: space-between; }` (the InviteManager row idiom) + the same `button.danger-outline` rules as Task 11 (scoped per component — no shared stylesheet exists for modules).

- [ ] **Step 5: Run the gates:** `pnpm -r test && pnpm -r typecheck && pnpm lint` → PASS.

- [ ] **Step 6: Commit.** `git commit -m "feat(settings): admin user deletion with confirm and server-message errors"`

---

### Task 13: World-delete e2e (Playwright)

**Files:**
- Create: `src/client/shell/e2e/world-delete.spec.ts`

**Interfaces:**
- Consumes: the seeded admin (`ops`/`pw-boot` from `playwright.config.ts` webServer.env), Task 11's UI labels.

- [ ] **Step 1: Write the spec** (unique names — `SHADOWCAT_DB=sqlite::memory:` + `reuseExistingServer` make state persist across local runs):

```ts
import { expect, test, type Page } from "@playwright/test";

async function login(page: Page, username: string, password: string): Promise<void> {
  await page.goto("/");
  await page.getByLabel("Username").fill(username);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Log in" }).click();
}

test("world delete: type-the-name confirm removes the world server-side", async ({ page }) => {
  const worldName = `Delete Me ${Date.now().toString(36)}`;
  await login(page, "ops", "pw-boot");
  await page.getByLabel("New world name").fill(worldName);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(page.getByTestId("stage-canvas")).toBeVisible();

  // Back to the world list (session cookie persists; entry lands on the roster).
  await page.goto("/");
  const row = page.locator("li", { hasText: worldName });
  await row.getByRole("button", { name: "Delete", exact: true }).click();
  await row.getByLabel("Type the world name to confirm deletion").fill(worldName);
  await row.getByRole("button", { name: "Delete forever" }).click();
  await expect(page.getByRole("button", { name: new RegExp(worldName) })).toHaveCount(0);

  // Server-side, not just local state: still gone after a reload.
  await page.reload();
  await expect(page.getByRole("button", { name: new RegExp(worldName) })).toHaveCount(0);
});
```

- [ ] **Step 2: Run it:** `pnpm --filter @shadowcat/shell e2e` (builds client + server binary first) → PASS. If the row locator is ambiguous against the armed form, scope with `row.locator(".row")` — adjust to what Task 11 actually rendered, not the other way around.

- [ ] **Step 3: Commit.** `git commit -m "test(e2e): world deletion flow"`

---

### Task 14: Docs, skills, and graph sync (final gate)

**Files:**
- Modify: `docs/TODO.md`, `.claude/skills/shadowcat-codebase-realtime-sync/SKILL.md`, `.claude/skills/shadowcat-codebase-documents-permissions/SKILL.md`, `.claude/skills/shadowcat-codebase-assets/SKILL.md`, `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md`

- [ ] **Step 1: `docs/TODO.md`** — remove three now-closed sections in full: "Blocked on world/user deletion" (:21-22), "Actionable now — `explored_fog` purge on scene deletion" (:24-25), "Blocked on user deletion existing — `add_member` resolve+write is two queries" (:106-111).
- [ ] **Step 2: Skill updates** (orientation-level, pointer-style — no code dumps):
  - **realtime-sync:** the eviction seam — `ServerMsg::Evicted { user: Option<Uuid> }` (None = room-wide world deletion, Some = per-user account deletion via `RoomRegistry::evict_user`); the registry deletion tombstone (`begin_delete`/`finish_delete`, `get_or_create` refusal); the invariant that `AuthUser` trusts the session record without re-reading `users`, so **user deletion must revoke sessions in the same transaction** (`json_extract` on `tower_sessions.data`); client terminal handling (`WsClient.stop()` on `evicted`, no reconnect).
  - **documents-permissions:** `delete_document_tx` as the single source for document-delete side-effects (row + FTS + scene fog purge; both `apply_intent` and `apply_command` call it — never-fork); `Asset.created_by` now nullable (`ON DELETE SET NULL`, wire `string | null`).
  - **assets:** world deletion removes `<assets_path>/<world_id>/` AFTER the row transaction (delete convention: rows first, files second) under the write barrier's read side.
  - **scene-rendering:** one line in the fog section: `explored_fog` rows are purged on scene delete (`delete_document_tx`), world delete, and user delete; the M9c "orphan harmlessly" note is historical.
- [ ] **Step 3:** `graphify update .`
- [ ] **Step 4: Full verification battery:** `cargo test --manifest-path src/server/Cargo.toml && cargo clippy --manifest-path src/server/Cargo.toml --all-targets && cargo fmt --check --manifest-path src/server/Cargo.toml && pnpm -r test && pnpm -r typecheck && pnpm lint` → all PASS.
- [ ] **Step 5: Commit.** `git commit -m "docs(skills): phase-b deletion seams — eviction, delete_document_tx, nullable created_by"`
- [ ] **Step 6 (gate):** dispatch `shadowcat-spec-reviewer` on the skill diffs (reviewed skill-update gate), then the phase's two-reviewer final pair per the Buddy-check directives.

---

## Self-review record

- **Spec coverage:** B1 → Tasks 1, 2, 4, 5 (+3 client eviction); B2 → Tasks 6, 7, 8 (+2 `evict_user`, +3 client); B3 → Task 9; B4 → Task 10; B5 → Tasks 11, 12, 13. Plan-time verification duties (FTS-under-cascade, FK audit) → done, pinned by Task 4's test; audit deltas (settings purge, user-fog purge, session revocation, re-creation race, client reconnect-loop) → Tasks 4, 7, 2, 3.
- **Known intentional non-goals:** no `explored_fog.user_id` index (rationale in Global Constraints); `world_events` retention untouched (H3's concern); no client UX beyond eviction-routes-to-world-list; migration-file comments left stale (checksums).
- **Type consistency:** `Evicted { user: Option<Uuid> }` ↔ `{ type: "evicted", user: string | null }` (Tasks 1/3); `begin_delete → Option<Arc<Room>>` / `finish_delete` / `evict_user` (Tasks 2/5/8); `delete_world`/`delete_user` `Result<(), DataError>` with `NotFound`/`Conflict` mapping to 404/409 (Tasks 4/5, 7/8); `deleteWorld`/`deleteUser` `Promise<void>` (Tasks 11/12).
