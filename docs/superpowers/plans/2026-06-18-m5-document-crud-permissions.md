# M5 Document CRUD + Permissions + Server-side Rollback Implementation Plan

> **For agentic workers:** Executed via the `mainline-plan-execution` skill (user-scope guidance, Opus/Fable-class) — tasks run inline in-session with a per-task inline enumerative spec-compliance check and ONE dispatched fresh-context branch review at the end. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Build the authoritative document read/write path: per-world membership/roles, a `PermissionContext` gating reads/writes and filtering every broadcast per recipient, document CRUD over HTTP + WS intents through one core write path, and field-level optimistic concurrency (intent/confirm/reject) over the existing pre-image substrate.

**Architecture:** A `world_members` table supplies per-world roles. `PermissionContext{user_id, world_role}` is built per WS connection / HTTP request. One core write path (`Room::publish` → `repo.apply_intent`) authorizes, validates, checks per-op pre-images, and applies — all in one transaction; success broadcasts a confirmed `Event`, failure returns a `Reject`. Each WS egress task filters every outgoing command for its recipient (reusing `filter_properties`), seq-preserving.

**Tech Stack:** Rust, axum 0.8 (HTTP + WS), sqlx/SQLite, tokio broadcast, ts-rs.

## Global Constraints

- **Permissive licenses only** (ARCHITECTURE §2.9). No new runtime deps expected.
- **Single crate** `shadowcat`; modules under `src/server/src/`.
- **Server-authoritative + structural validation only** (invariants #1, #6): the server never semantically validates the `system` body — size cap, field-path validity, `deny_unknown_fields` only.
- **Ordered realtime** (invariant #2): broadcast order == seq order; the per-world publish guard wraps allocate-seq → apply → send.
- **Per-recipient filtering** (invariant #4): hidden properties stripped before transmission, never sent-then-hidden.
- **ts-rs bindings emit to `src/types/generated/`, CI-enforced sync** (`cargo test` regenerates; `git diff --exit-code src/types/generated`).
- **No debug code in release**: diagnostics via `tracing` only.
- **Commit message trailers** (every commit):
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Htozbntnxh8N3meNWAeoNp
  ```

## File Structure

| File | Responsibility |
|---|---|
| `src/server/migrations/0003_world_members.sql` (create) | `world_members` table |
| `src/server/src/data/membership.rs` (create) | `PermissionContext`; `WorldRole` SQL codec; membership queries module doc |
| `src/server/src/data/document.rs` (modify) | `WorldRole::as_db_str` / `from_db_str` |
| `src/server/src/data/mod.rs` (modify) | `DataError::{Forbidden, Conflict}`; `pub mod membership` |
| `src/server/src/data/permission.rs` (modify) | `filter_command` (async) |
| `src/server/src/data/repository.rs` (modify) | `apply_intent`, `get_world` (exists); membership trait methods |
| `src/server/src/data/sqlite.rs` (modify) | `create_world_owned`, membership methods, `permission_context`, `apply_intent` impl |
| `src/server/src/ws/protocol.rs` (modify) | `Intent`, `Reject`, `RejectReason`; retire `EmitTest`; `Event` gains `intent_id` |
| `src/server/src/ws/room.rs` (modify) | generalize `publish` to `(ctx, ops, ts)` via `apply_intent` |
| `src/server/src/ws/conn.rs` (modify) | build `PermissionContext` at join (require membership); ingress `Intent`; egress + resync filter |
| `src/server/src/http/routes.rs` (modify) | world-create, membership, document CRUD handlers |
| `src/server/src/http/mod.rs` (modify) | routes |
| `src/server/tests/ws_convergence.rs` (modify) | intents replace `EmitTest`; permission/conflict cases |

---

### Task 1: world_members migration + membership queries + world creation

**Files:**
- Create: `src/server/migrations/0003_world_members.sql`
- Create: `src/server/src/data/membership.rs`
- Modify: `src/server/src/data/mod.rs` (`pub mod membership;`)
- Modify: `src/server/src/data/document.rs` (`WorldRole` SQL codec)
- Modify: `src/server/src/data/sqlite.rs` (membership methods + `create_world_owned`)

**Interfaces:**
- Produces:
  - `WorldRole::as_db_str(&self) -> &'static str`, `WorldRole::from_db_str(&str) -> Option<WorldRole>`.
  - `SqliteRepository::create_world_owned(name: &str, creator: Uuid, now: i64) -> Result<World, DataError>` (world + creator-as-GM membership, one tx).
  - `SqliteRepository::add_member(world, user, WorldRole, now)`, `remove_member(world, user)`, `set_role(world, user, WorldRole)`, `world_role(world, user) -> Result<Option<WorldRole>, DataError>`, `list_members(world) -> Result<Vec<(Uuid, WorldRole)>, DataError>`.

- [ ] **Step 1: Migration.** Create `src/server/migrations/0003_world_members.sql`:

```sql
CREATE TABLE world_members (
  world_id   TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
  user_id    TEXT NOT NULL REFERENCES users(id)  ON DELETE CASCADE,
  world_role TEXT NOT NULL,
  PRIMARY KEY (world_id, user_id)
);
```

- [ ] **Step 2: WorldRole SQL codec.** In `document.rs`, add below the `WorldRole` enum:

```rust
impl WorldRole {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            WorldRole::Gm => "gm",
            WorldRole::Player => "player",
            WorldRole::Spectator => "spectator",
        }
    }
    pub fn from_db_str(s: &str) -> Option<WorldRole> {
        match s {
            "gm" => Some(WorldRole::Gm),
            "player" => Some(WorldRole::Player),
            "spectator" => Some(WorldRole::Spectator),
            _ => None,
        }
    }
}
```

- [ ] **Step 3: `pub mod membership;`** in `data/mod.rs` (after `pub mod document;`). Create `src/server/src/data/membership.rs` with just a module doc comment for now (PermissionContext lands in Task 2):

```rust
//! Per-world membership: roles, the per-actor PermissionContext, and the
//! queries that resolve a user's role within a world.
```

- [ ] **Step 4: Write failing membership tests** in `sqlite.rs` test module:

```rust
    #[tokio::test]
    async fn world_owned_makes_creator_gm() {
        let r = repo().await;
        let creator = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
        let w = r.create_world_owned("W", creator, 0).await.unwrap();
        assert_eq!(r.world_role(w.id, creator).await.unwrap(), Some(WorldRole::Gm));
        let stranger = Uuid::from_u128(123);
        assert_eq!(r.world_role(w.id, stranger).await.unwrap(), None);
    }

    #[tokio::test]
    async fn members_add_set_remove() {
        let r = repo().await;
        let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
        let p = r.create_user("p", None, ServerRole::User, 0).await.unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();
        r.add_member(w.id, p, WorldRole::Player, 0).await.unwrap();
        assert_eq!(r.world_role(w.id, p).await.unwrap(), Some(WorldRole::Player));
        r.set_role(w.id, p, WorldRole::Spectator).await.unwrap();
        assert_eq!(r.world_role(w.id, p).await.unwrap(), Some(WorldRole::Spectator));
        let members = r.list_members(w.id).await.unwrap();
        assert_eq!(members.len(), 2); // gm + p
        r.remove_member(w.id, p).await.unwrap();
        assert_eq!(r.world_role(w.id, p).await.unwrap(), None);
    }
```

(`repo()` is the existing sqlite test helper that connects in-memory.)

- [ ] **Step 5: Run to verify failure.** `cargo test -p shadowcat world_owned_makes_creator_gm` → FAIL (methods missing).

- [ ] **Step 6: Implement** in `sqlite.rs` `impl SqliteRepository` (inherent block). Add `use crate::data::document::WorldRole;` if needed:

```rust
    pub async fn create_world_owned(&self, name: &str, creator: Uuid, now: i64) -> Result<World, DataError> {
        let mut tx = self.pool.begin().await?;
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO worlds (id, name, seq, created_at, updated_at) VALUES (?, ?, 0, ?, ?)")
            .bind(id.to_string()).bind(name).bind(now).bind(now)
            .execute(&mut *tx).await?;
        sqlx::query("INSERT INTO world_members (world_id, user_id, world_role) VALUES (?, ?, ?)")
            .bind(id.to_string()).bind(creator.to_string()).bind(WorldRole::Gm.as_db_str())
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(World { id, name: name.to_string(), seq: 0, created_at: now, updated_at: now })
    }

    pub async fn add_member(&self, world: Uuid, user: Uuid, role: WorldRole, _now: i64) -> Result<(), DataError> {
        sqlx::query("INSERT INTO world_members (world_id, user_id, world_role) VALUES (?, ?, ?) \
                     ON CONFLICT(world_id, user_id) DO UPDATE SET world_role = excluded.world_role")
            .bind(world.to_string()).bind(user.to_string()).bind(role.as_db_str())
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn set_role(&self, world: Uuid, user: Uuid, role: WorldRole) -> Result<(), DataError> {
        let res = sqlx::query("UPDATE world_members SET world_role = ? WHERE world_id = ? AND user_id = ?")
            .bind(role.as_db_str()).bind(world.to_string()).bind(user.to_string())
            .execute(&self.pool).await?;
        if res.rows_affected() == 0 { return Err(DataError::NotFound); }
        Ok(())
    }

    pub async fn remove_member(&self, world: Uuid, user: Uuid) -> Result<(), DataError> {
        sqlx::query("DELETE FROM world_members WHERE world_id = ? AND user_id = ?")
            .bind(world.to_string()).bind(user.to_string())
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn world_role(&self, world: Uuid, user: Uuid) -> Result<Option<WorldRole>, DataError> {
        let row = sqlx::query("SELECT world_role FROM world_members WHERE world_id = ? AND user_id = ?")
            .bind(world.to_string()).bind(user.to_string())
            .fetch_optional(&self.pool).await?;
        Ok(row.and_then(|r| WorldRole::from_db_str(r.get::<String, _>("world_role").as_str())))
    }

    pub async fn list_members(&self, world: Uuid) -> Result<Vec<(Uuid, WorldRole)>, DataError> {
        let rows = sqlx::query("SELECT user_id, world_role FROM world_members WHERE world_id = ?")
            .bind(world.to_string()).fetch_all(&self.pool).await?;
        Ok(rows.into_iter().filter_map(|r| {
            let uid = Uuid::parse_str(r.get::<String, _>("user_id").as_str()).ok()?;
            let role = WorldRole::from_db_str(r.get::<String, _>("world_role").as_str())?;
            Some((uid, role))
        }).collect())
    }
```

- [ ] **Step 7: Run tests.** `cargo test -p shadowcat membership` and `cargo test -p shadowcat world_owned` → PASS. Run `cargo test -p shadowcat data::` → all green.

- [ ] **Step 8: Commit.**
```bash
git add src/server/migrations/0003_world_members.sql src/server/src/data/membership.rs src/server/src/data/mod.rs src/server/src/data/document.rs src/server/src/data/sqlite.rs
git commit -m "feat(m5): world_members migration, membership queries, owned world creation"
```

---

### Task 2: PermissionContext + resolver

**Files:**
- Modify: `src/server/src/data/membership.rs` (`PermissionContext`)
- Modify: `src/server/src/data/sqlite.rs` (`permission_context`)
- Modify: `src/server/src/data/mod.rs` (`DataError::Forbidden`)

**Interfaces:**
- Consumes: `world_role`, `ServerRole`, `WorldRole`.
- Produces:
  - `membership::PermissionContext { user_id: Uuid, world_role: WorldRole }`.
  - `DataError::Forbidden`.
  - `SqliteRepository::permission_context(world: Uuid, user: Uuid, server_role: ServerRole) -> Result<PermissionContext, DataError>` — admin ⇒ GM; member ⇒ their role; non-member non-admin ⇒ `Err(Forbidden)`.

- [ ] **Step 1: Add `DataError::Forbidden`** in `data/mod.rs`:
```rust
    #[error("forbidden")]
    Forbidden,
```

- [ ] **Step 2: `PermissionContext`** in `membership.rs`:
```rust
use uuid::Uuid;
use crate::data::document::WorldRole;

/// A resolved per-actor authority within one world. Built once per WS
/// connection and per HTTP request; gates writes/reads and filters broadcasts.
#[derive(Debug, Clone, Copy)]
pub struct PermissionContext {
    pub user_id: Uuid,
    pub world_role: WorldRole,
}
```

- [ ] **Step 3: Write failing test** in `sqlite.rs` tests:
```rust
    #[tokio::test]
    async fn permission_context_resolves_role_or_forbids() {
        use crate::data::membership::PermissionContext;
        let r = repo().await;
        let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
        let admin = r.create_user("ad", None, ServerRole::Admin, 0).await.unwrap();
        let stranger = r.create_user("s", None, ServerRole::User, 0).await.unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();

        let c: PermissionContext = r.permission_context(w.id, gm, ServerRole::User).await.unwrap();
        assert_eq!(c.world_role, WorldRole::Gm);
        // Server admin is GM even without a membership row.
        let ac = r.permission_context(w.id, admin, ServerRole::Admin).await.unwrap();
        assert_eq!(ac.world_role, WorldRole::Gm);
        // Non-member, non-admin → forbidden.
        assert!(matches!(r.permission_context(w.id, stranger, ServerRole::User).await, Err(DataError::Forbidden)));
    }
```

- [ ] **Step 4: Run → FAIL** (`permission_context` missing).

- [ ] **Step 5: Implement** in `sqlite.rs`:
```rust
    pub async fn permission_context(
        &self, world: Uuid, user: Uuid, server_role: ServerRole,
    ) -> Result<crate::data::membership::PermissionContext, DataError> {
        use crate::data::membership::PermissionContext;
        if server_role == ServerRole::Admin {
            return Ok(PermissionContext { user_id: user, world_role: WorldRole::Gm });
        }
        match self.world_role(world, user).await? {
            Some(role) => Ok(PermissionContext { user_id: user, world_role: role }),
            None => Err(DataError::Forbidden),
        }
    }
```

- [ ] **Step 6: Run tests.** `cargo test -p shadowcat permission_context` → PASS.

- [ ] **Step 7: Commit.**
```bash
git add src/server/src/data/membership.rs src/server/src/data/sqlite.rs src/server/src/data/mod.rs
git commit -m "feat(m5): PermissionContext + membership-backed resolver"
```

---

### Task 3: apply_intent — transactional authorize + validate + pre-image check + apply

**Files:**
- Modify: `src/server/src/data/mod.rs` (`DataError::Conflict`)
- Modify: `src/server/src/data/repository.rs` (`apply_intent` trait method)
- Modify: `src/server/src/data/sqlite.rs` (impl)

**Interfaces:**
- Consumes: `PermissionContext`, `resolve_access`, `validation::*`, `Operation`, `set_pointer`, `apply_command` internals.
- Produces: `Repository::apply_intent(ctx: &PermissionContext, world_id: Uuid, ops: Vec<Operation>, ts: i64) -> Result<Command, DataError>`. Errors: `Forbidden` (no write access), `Conflict(String)` (pre-image mismatch / id collision / missing), `BadPath`/`TooLarge` (invalid), `NotFound` (world).

> apply_intent mirrors `apply_command` (allocate seq → append event → apply ops) but adds, inside the same transaction and *before* mutating: per-op authorize, structural validation, and pre-image checks. The single-writer pool already serializes across worlds.

- [ ] **Step 1: Add `DataError::Conflict`** in `data/mod.rs`:
```rust
    #[error("conflict: {0}")]
    Conflict(String),
```

- [ ] **Step 2: Add to the `Repository` trait** (`repository.rs`), after `apply_command`:
```rust
    /// Authorize (per `ctx`), structurally validate, check per-op pre-images,
    /// then apply + sequence + log — all in one transaction. Field-level
    /// optimistic concurrency: an `Update` whose `FieldChange.old` does not match
    /// the current stored value yields `Conflict`.
    async fn apply_intent(
        &self,
        ctx: &crate::data::membership::PermissionContext,
        world_id: Uuid,
        ops: Vec<crate::data::command::Operation>,
        ts: i64,
    ) -> Result<Command, DataError>;
```

- [ ] **Step 3: Write failing tests** in `sqlite.rs` tests. (`world_doc` helper exists in the test module from M2; it builds a world-scoped Document.)
```rust
    #[tokio::test]
    async fn apply_intent_create_then_conflicting_update() {
        use crate::data::command::{FieldChange, Operation};
        use crate::data::membership::PermissionContext;
        let r = repo().await;
        let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();
        let ctx = PermissionContext { user_id: gm, world_role: WorldRole::Gm };
        let doc = world_doc(1, w.id, serde_json::json!({ "hp": 10 }));
        // Create.
        let c1 = r.apply_intent(&ctx, w.id, vec![Operation::Create { doc: doc.clone() }], 1).await.unwrap();
        assert_eq!(c1.seq, 1);
        // Matching pre-image Update succeeds.
        let ok = r.apply_intent(&ctx, w.id, vec![Operation::Update {
            doc_id: doc.id,
            changes: vec![FieldChange { path: "/system/hp".into(), old: serde_json::json!(10), new: serde_json::json!(5) }],
        }], 2).await.unwrap();
        assert_eq!(ok.seq, 2);
        // Stale pre-image Update → Conflict (current is now 5, not 10).
        let conflict = r.apply_intent(&ctx, w.id, vec![Operation::Update {
            doc_id: doc.id,
            changes: vec![FieldChange { path: "/system/hp".into(), old: serde_json::json!(10), new: serde_json::json!(1) }],
        }], 3).await;
        assert!(matches!(conflict, Err(DataError::Conflict(_))));
        // State unchanged after the rejected intent.
        assert_eq!(r.get_document(doc.id).await.unwrap().unwrap().system["hp"], serde_json::json!(5));
    }

    #[tokio::test]
    async fn apply_intent_rejects_unauthorized_and_oversized() {
        use crate::data::command::Operation;
        use crate::data::document::{DocRole, PermissionSet};
        use crate::data::membership::PermissionContext;
        let r = repo().await;
        let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();
        // A doc only the GM can write (default None).
        let mut doc = world_doc(2, w.id, serde_json::json!({}));
        doc.permissions = PermissionSet { default: DocRole::None, ..Default::default() };
        let gm_ctx = PermissionContext { user_id: gm, world_role: WorldRole::Gm };
        r.apply_intent(&gm_ctx, w.id, vec![Operation::Create { doc: doc.clone() }], 1).await.unwrap();
        // A player tries to update it → Forbidden.
        let player = r.create_user("p", None, ServerRole::User, 0).await.unwrap();
        let p_ctx = PermissionContext { user_id: player, world_role: WorldRole::Player };
        use crate::data::command::FieldChange;
        let forbidden = r.apply_intent(&p_ctx, w.id, vec![Operation::Update {
            doc_id: doc.id,
            changes: vec![FieldChange { path: "/system/x".into(), old: serde_json::json!(null), new: serde_json::json!(1) }],
        }], 2).await;
        assert!(matches!(forbidden, Err(DataError::Forbidden)));
        // Oversized create → TooLarge.
        let big = world_doc(3, w.id, serde_json::json!({ "blob": "x".repeat(300*1024) }));
        let too_large = r.apply_intent(&gm_ctx, w.id, vec![Operation::Create { doc: big }], 3).await;
        assert!(matches!(too_large, Err(DataError::TooLarge(_))));
    }
```

- [ ] **Step 4: Run → FAIL** (`apply_intent` missing).

- [ ] **Step 5: Implement** in `sqlite.rs` `impl Repository for SqliteRepository`. Add `use crate::data::permission::resolve_access; use crate::data::validation;` at the top of the file if absent.

```rust
    async fn apply_intent(
        &self,
        ctx: &crate::data::membership::PermissionContext,
        world_id: Uuid,
        ops: Vec<Operation>,
        ts: i64,
    ) -> Result<Command, DataError> {
        let mut tx = self.pool.begin().await?;

        // 1. Authorize + validate + pre-image check (no mutation yet).
        for op in &ops {
            match op {
                Operation::Create { doc } => {
                    check_command_scope(doc, world_id)?;
                    validation::validate_system_size(doc)?;
                    if !resolve_access(ctx.user_id, ctx.world_role, doc).can_write {
                        return Err(DataError::Forbidden);
                    }
                    if Self::load_document(&mut *tx, doc.id).await?.is_some() {
                        return Err(DataError::Conflict(format!("document {} already exists", doc.id)));
                    }
                }
                Operation::Delete { doc } => {
                    let cur = Self::load_document(&mut *tx, doc.id).await?
                        .ok_or_else(|| DataError::Conflict(format!("document {} missing", doc.id)))?;
                    if !resolve_access(ctx.user_id, ctx.world_role, &cur).can_write {
                        return Err(DataError::Forbidden);
                    }
                }
                Operation::Update { doc_id, changes } => {
                    let cur = Self::load_document(&mut *tx, *doc_id).await?
                        .ok_or_else(|| DataError::Conflict(format!("document {doc_id} missing")))?;
                    if !resolve_access(ctx.user_id, ctx.world_role, &cur).can_write {
                        return Err(DataError::Forbidden);
                    }
                    let whole = serde_json::to_value(&cur)?;
                    for ch in changes {
                        validation::validate_field_path(&ch.path)?;
                        let actual = whole.pointer(&ch.path).cloned().unwrap_or(serde_json::Value::Null);
                        if actual != ch.old {
                            return Err(DataError::Conflict(format!("stale pre-image at {}", ch.path)));
                        }
                    }
                    validation::validate_system_size(&cur)?; // body cap also re-checked after merge below
                }
            }
        }

        // 2. Allocate seq, apply, log — identical machinery to apply_command.
        let seq: i64 = sqlx::query("UPDATE worlds SET seq = seq + 1 WHERE id = ? RETURNING seq")
            .bind(world_id.to_string())
            .fetch_optional(&mut *tx).await?
            .ok_or(DataError::NotFound)?
            .get("seq");
        let sequenced = Command { seq, world_id, author: ctx.user_id, ts, ops };

        for op in &sequenced.ops {
            match op {
                Operation::Create { doc } => Self::upsert_document(&mut *tx, doc, seq).await?,
                Operation::Delete { doc } => {
                    sqlx::query("DELETE FROM documents WHERE id = ?").bind(doc.id.to_string())
                        .execute(&mut *tx).await?;
                }
                Operation::Update { doc_id, changes } => {
                    let mut doc = Self::load_document(&mut *tx, *doc_id).await?
                        .ok_or(DataError::NotFound)?;
                    let mut whole = serde_json::to_value(&doc)?;
                    for ch in changes { set_pointer(&mut whole, &ch.path, ch.new.clone())?; }
                    doc = serde_json::from_value(whole)?;
                    check_command_scope(&doc, world_id)?;
                    validation::validate_system_size(&doc)?;
                    doc.updated_at = ts;
                    Self::upsert_document(&mut *tx, &doc, seq).await?;
                }
            }
        }
        sqlx::query("INSERT INTO world_events (world_id, seq, author_id, ts, command_json) VALUES (?, ?, ?, ?, ?)")
            .bind(sequenced.world_id.to_string()).bind(seq).bind(sequenced.author.to_string())
            .bind(ts).bind(serde_json::to_string(&sequenced)?)
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(sequenced)
    }
```

> If a private `load_document(executor, id)` helper does not already exist (the public `get_document` uses `&self.pool`), add a small executor-generic version mirroring `get_document`'s row→Document mapping so it can run inside the transaction. Verify against the existing `get_document` body during implementation; reuse its mapping exactly.

- [ ] **Step 6: Run tests.** `cargo test -p shadowcat apply_intent` → PASS. `cargo test -p shadowcat data::` → green.

- [ ] **Step 7: Commit.**
```bash
git add src/server/src/data/mod.rs src/server/src/data/repository.rs src/server/src/data/sqlite.rs
git commit -m "feat(m5): apply_intent — transactional authorize/validate/pre-image check"
```

---

### Task 4: filter_command (per-recipient redaction)

**Files:**
- Modify: `src/server/src/data/permission.rs`

**Interfaces:**
- Consumes: `Command`, `Operation`, `PermissionContext`, `resolve_access`, `filter_properties`, `Repository::get_document`.
- Produces: `async fn filter_command(repo: &dyn Repository, cmd: &Command, ctx: &PermissionContext) -> Command` — seq/world/author/ts preserved; ops redacted/dropped for the recipient.

- [ ] **Step 1: Write failing test** in `permission.rs` tests:
```rust
    #[tokio::test]
    async fn filter_command_strips_and_preserves_seq() {
        use crate::data::command::{Command, FieldChange, Operation};
        use crate::data::membership::PermissionContext;
        use crate::data::repository::Repository;
        use crate::data::sqlite::SqliteRepository;
        use crate::auth::role::ServerRole;

        let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();
        let gm_ctx = PermissionContext { user_id: gm, world_role: WorldRole::Gm };

        let mut d = doc(
            PermissionSet { default: DocRole::Observer, ..Default::default() },
            serde_json::json!({ "secret": 1, "public": 2 }),
        );
        d.scope = Scope::World { world_id: w.id };
        d.permissions.property_overrides.insert("/system/secret".into(), Visibility::GmOnly);
        r.apply_intent(&gm_ctx, w.id, vec![Operation::Create { doc: d.clone() }], 1).await.unwrap();

        // An Update touching both a GmOnly and a public field.
        let cmd = Command { seq: 2, world_id: w.id, author: gm, ts: 0, ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![
                FieldChange { path: "/system/secret".into(), old: serde_json::json!(1), new: serde_json::json!(9) },
                FieldChange { path: "/system/public".into(), old: serde_json::json!(2), new: serde_json::json!(8) },
            ],
        }]};

        let player = PermissionContext { user_id: Uuid::from_u128(77), world_role: WorldRole::Player };
        let filtered = filter_command(&r, &cmd, &player).await;
        assert_eq!(filtered.seq, 2); // seq preserved
        if let Operation::Update { changes, .. } = &filtered.ops[0] {
            assert_eq!(changes.len(), 1); // GmOnly change dropped
            assert_eq!(changes[0].path, "/system/public");
        } else { panic!("expected Update"); }

        // GM sees everything.
        let gm_view = filter_command(&r, &cmd, &gm_ctx).await;
        if let Operation::Update { changes, .. } = &gm_view.ops[0] {
            assert_eq!(changes.len(), 2);
        } else { panic!(); }
    }
```

- [ ] **Step 2: Run → FAIL** (`filter_command` missing).

- [ ] **Step 3: Implement** in `permission.rs`:
```rust
use crate::data::command::{Command, Operation};
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;

/// The recipient's view of a broadcast command: ops on unreadable documents are
/// dropped, GmOnly properties/changes stripped. seq/world/author/ts are
/// preserved so the recipient's sequence guard never sees a false gap (a fully
/// redacted command keeps its seq with empty ops).
pub async fn filter_command(repo: &dyn Repository, cmd: &Command, ctx: &PermissionContext) -> Command {
    let mut out_ops = Vec::with_capacity(cmd.ops.len());
    for op in &cmd.ops {
        match op {
            Operation::Create { doc } => {
                let access = resolve_access(ctx.user_id, ctx.world_role, doc);
                if access.can_read {
                    out_ops.push(Operation::Create { doc: filter_properties(doc, access) });
                }
            }
            Operation::Delete { doc } => {
                // Deletes are visible to anyone who could read the doc.
                let access = resolve_access(ctx.user_id, ctx.world_role, doc);
                if access.can_read {
                    out_ops.push(Operation::Delete { doc: filter_properties(doc, access) });
                }
            }
            Operation::Update { doc_id, changes } => {
                let Ok(Some(cur)) = repo.get_document(*doc_id).await else { continue };
                let access = resolve_access(ctx.user_id, ctx.world_role, &cur);
                if !access.can_read { continue; }
                let kept: Vec<_> = if access.see_gm_only {
                    changes.clone()
                } else {
                    changes.iter().cloned()
                        .filter(|ch| cur.permissions.property_overrides.get(&ch.path) != Some(&Visibility::GmOnly))
                        .collect()
                };
                out_ops.push(Operation::Update { doc_id: *doc_id, changes: kept });
            }
        }
    }
    Command { seq: cmd.seq, world_id: cmd.world_id, author: cmd.author, ts: cmd.ts, ops: out_ops }
}
```

- [ ] **Step 4: Run tests.** `cargo test -p shadowcat filter_command` → PASS.

- [ ] **Step 5: Commit.**
```bash
git add src/server/src/data/permission.rs
git commit -m "feat(m5): per-recipient filter_command with seq-preserving redaction"
```

---

### Task 5: WS protocol — Intent, Reject; retire EmitTest

**Files:**
- Modify: `src/server/src/ws/protocol.rs`

**Interfaces:**
- Produces: `ClientMsg::Intent { intent_id: Uuid, ops: Vec<Operation> }` (replaces `EmitTest`); `ServerMsg::Event { command, intent_id: Option<Uuid> }`; `ServerMsg::Reject { intent_id: Uuid, reason: RejectReason }`; `RejectReason { Forbidden, Conflict, Invalid }`.

- [ ] **Step 1: Edit `protocol.rs`.** Replace `EmitTest { nonce: u64 }` with `Intent { intent_id: Uuid, ops: Vec<crate::data::command::Operation> }`; add `intent_id: Option<Uuid>` to `Event`; add `Reject` + `RejectReason`. Add `use crate::data::command::{Command, Operation};`.

```rust
    // in ClientMsg:
    Intent { intent_id: Uuid, ops: Vec<Operation> },
```
```rust
    // ServerMsg::Event becomes:
    Event { command: Command, intent_id: Option<Uuid> },
    // add:
    Reject { intent_id: Uuid, reason: RejectReason },
```
```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum RejectReason { Forbidden, Conflict, Invalid }
```

- [ ] **Step 2: Update `event_seq`/`event_ts`** — the `Event { command, .. }` pattern still matches; add `..` for the new field:
```rust
            ServerMsg::Event { command, .. } => Some(command.seq),
```
(both methods).

- [ ] **Step 3: Update protocol tests** for the new shapes (e.g. an `Event` constructed in tests now needs `intent_id: None`; add a `Reject` round-trip):
```rust
    #[test]
    fn reject_round_trips() {
        let m = ServerMsg::Reject { intent_id: Uuid::from_u128(3), reason: RejectReason::Conflict };
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("\"type\":\"reject\""));
        assert!(s.contains("\"reason\":\"conflict\""));
        let _back: ServerMsg = serde_json::from_str(&s).unwrap();
    }
```

- [ ] **Step 4: Run + regenerate bindings.** `cargo test -p shadowcat` → this will FAIL to compile in `room.rs`/`conn.rs`/`ws_convergence.rs` (they still reference `EmitTest` and the old `Event` shape). That is expected — Tasks 6-9 update them. To keep this task's commit green, do Steps 1-3 **together with Task 6** (they are mutually dependent: protocol shape ↔ its only callers). Treat Tasks 5+6 as one commit (see Task 6 Step 6).

- [ ] **Step 5:** (folded into Task 6.)

---

### Task 6: Room::publish generalized to real ops

**Files:**
- Modify: `src/server/src/ws/room.rs`
- Modify: `src/server/src/ws/conn.rs` (EmitTest arm → see Task 7; for now the build must pass)

**Interfaces:**
- Consumes: `apply_intent`, `PermissionContext`, `Operation`.
- Produces: `Room::publish(repo: &dyn Repository, ctx: &PermissionContext, ops: Vec<Operation>, ts: i64) -> Result<Command, DataError>`.

- [ ] **Step 1: Generalize `Room::publish`** in `room.rs`:
```rust
    pub async fn publish(
        &self,
        repo: &dyn Repository,
        ctx: &crate::data::membership::PermissionContext,
        ops: Vec<crate::data::command::Operation>,
        ts: i64,
    ) -> Result<Command, DataError> {
        let _guard = self.publish_guard.lock().await;
        let cmd = repo.apply_intent(ctx, self.world_id, ops, ts).await?;
        let msg = Arc::new(ServerMsg::Event { command: cmd.clone(), intent_id: None });
        self.ring.lock().await.push(msg.clone());
        self.current_seq.store(cmd.seq, Ordering::Release);
        let _ = self.tx.send(msg);
        self.stats.events_published.fetch_add(1, Ordering::Relaxed);
        Ok(cmd)
    }
```

> The broadcast `Event` carries `intent_id: None`; the originator's confirm correlation is handled at the connection layer (Task 7) where the intent_id is known. (Keeping intent_id off the shared broadcast avoids leaking one client's intent id to others; the originator matches on the command's `(author, seq)` echo plus its own outstanding-intent bookkeeping in M6. For M5 the server additionally sends a direct `Event{intent_id: Some}` to the originator — see Task 7.)

- [ ] **Step 2: Update the `room_tests`** that called the old `publish(repo, author, ts)`:
```rust
    // replace publish(&repo, author, 0) calls with:
    let ctx = PermissionContext { user_id: author, world_role: WorldRole::Gm };
    room.publish(&repo, &ctx, vec![], 0).await.unwrap();
```
Add `use crate::data::membership::PermissionContext; use crate::data::document::WorldRole;` to the test module. Empty `ops` still allocates a seq and broadcasts (the M4 convergence behavior is preserved with an empty command).

- [ ] **Step 3: Apply Task 5's protocol edits** now (Intent/Reject/Event shape) so the crate compiles as a unit.

- [ ] **Step 4: Update `conn.rs` ingress** minimally so it compiles: replace the `EmitTest` arm with an `Intent` arm that publishes (full handling refined in Task 7):
```rust
    Ok(ClientMsg::Intent { intent_id, ops }) => {
        match room.publish(repo.as_ref(), &ctx, ops, now_millis()).await {
            Ok(_cmd) => { /* confirm via Task 7 */ }
            Err(e) => {
                let reason = reject_reason(&e);
                let _ = etx.send(Egress::Frame(Arc::new(ServerMsg::Reject { intent_id, reason }))).await;
            }
        }
    }
```
Add a helper in `conn.rs`:
```rust
fn reject_reason(e: &crate::data::DataError) -> crate::ws::protocol::RejectReason {
    use crate::data::DataError::*;
    use crate::ws::protocol::RejectReason;
    match e {
        Forbidden => RejectReason::Forbidden,
        Conflict(_) => RejectReason::Conflict,
        _ => RejectReason::Invalid,
    }
}
```
This references `ctx` — Task 7 builds it at join. For this task, build `ctx` at join with a temporary GM context if needed to compile, then Task 7 replaces it with the real `permission_context` lookup. (Simplest: do Task 7's join-context change in the same commit — see Step 6.)

- [ ] **Step 5: Update any other `Event {` constructions** (egress `text()` of redacted events etc.) to include `intent_id: None` where the server fabricates events; resync replay events use `intent_id: None`.

- [ ] **Step 6: Build + test.** Because protocol (T5), room (T6), and the conn join-context (T7) are mutually dependent, implement Tasks 5+6+7 and commit them together once `cargo test -p shadowcat` compiles and the unit tests pass. Run `cargo test -p shadowcat` (the convergence harness will still fail to compile until Task 9 — gate that by also doing Task 9 before the combined commit, OR temporarily `#[ignore]`-ing the harness; prefer doing 5+6+7+9 as one cohesive commit since they share the protocol change). Bindings regenerate (`ClientMsg.ts`, `ServerMsg.ts`, `RejectReason.ts`).

```bash
git add src/server/src/ws/protocol.rs src/server/src/ws/room.rs src/server/src/ws/conn.rs src/types/generated
git commit -m "feat(m5): real-ops publish + Intent/Reject protocol; PermissionContext at WS join"
```

---

### Task 7: Connection — PermissionContext at join, intent handling, egress filtering

**Files:**
- Modify: `src/server/src/ws/conn.rs`

**Interfaces:**
- Consumes: `permission_context`, `filter_command`, `AuthUser` (`.id`, `.role`), `PermissionContext`.
- Produces: membership-gated join; per-recipient filtered live + resync sends; intent confirm/reject.

- [ ] **Step 1: Build PermissionContext at join, require membership.** In `handle_socket`, after resolving the room, before subscribing:
```rust
    let ctx = match state.repo.permission_context(world_id, user_id, user_role).await {
        Ok(c) => c,
        Err(_) => {
            let mut s = socket;
            let _ = s.send(text(&ServerMsg::Error { code: WsErrorCode::Forbidden, message: "not a member of this world".into() })).await;
            let _ = s.send(Message::Close(None)).await;
            return;
        }
    };
```
Add `WsErrorCode::Forbidden` to `protocol.rs`. Thread `user_role: ServerRole` into `handle_socket` (from `AuthUser.role` in `ws_handler`). `ctx` is `Copy`, pass it into the egress task and use in the ingress `Intent` arm.

- [ ] **Step 2: Egress filters every send.** In `egress_loop`, the egress task gets `ctx` and `repo`. Replace direct `text(&msg)` sends of live `Event`s and resync events with a filtered version:
```rust
    // helper inside conn.rs
    async fn send_event<S: Sink<Message> + Unpin>(sink: &mut S, repo: &dyn Repository, ctx: &PermissionContext, msg: &ServerMsg) -> Result<(), ()> {
        let to_send = match msg {
            ServerMsg::Event { command, intent_id } => {
                ServerMsg::Event { command: crate::data::permission::filter_command(repo, command, ctx).await, intent_id: *intent_id }
            }
            other => other.clone(),
        };
        sink.send(text(&to_send)).await.map_err(|_| ())
    }
```
Use `send_event(&mut sink, repo.as_ref(), &ctx, &msg)` for live broadcast events; in `replay`, filter each replayed event the same way.

- [ ] **Step 3: Intent confirm to originator.** On `room.publish` success in the ingress arm, send the originator a confirmed, filtered Event with its intent_id:
```rust
    Ok(cmd) => {
        let _ = etx.send(Egress::Frame(Arc::new(ServerMsg::Event { command: cmd, intent_id: Some(intent_id) }))).await;
    }
```
The egress `Egress::Frame` path must also filter `Event` frames (route them through `send_event`). The shared broadcast still delivers the unfiltered-intent_id `Event{intent_id:None}` to everyone (filtered per recipient); the originator additionally gets the `Some(intent_id)` confirm. Dedup: the egress sequence guard drops the broadcast copy if its seq `< next_expected` after the direct confirm advanced it — acceptable (originator sees exactly one Event for its seq). Document this in a comment.

- [ ] **Step 4: Build + integration smoke.** Covered by Task 9. Build with `cargo build -p shadowcat`.

- [ ] **Step 5:** Commit folded into Task 6's combined commit (5+6+7), plus Task 9.

---

### Task 8: HTTP CRUD + membership + world-create endpoints

**Files:**
- Modify: `src/server/src/http/routes.rs`, `src/server/src/http/mod.rs`

**Interfaces:**
- Consumes: `AuthUser`, `permission_context`, `Room::publish` via `state.ws.rooms.get_or_create`, `apply_intent`, `get_document`, `query_documents`, `filter_properties`, membership methods.
- Produces: `POST /api/worlds` (create), `GET/POST /api/worlds/:id/members`, `DELETE /api/worlds/:id/members/:user`, `POST /api/worlds/:id/documents` (create), `GET /api/worlds/:id/documents`, `GET/PATCH/DELETE /api/documents/:id`.

- [ ] **Step 1: World create.** Handler (AuthUser): `create_world_owned(name, user.id, now)`, return the `World` JSON. Any authenticated user may create a world (becomes its GM).
- [ ] **Step 2: Membership.** `POST members {user, role}` and `DELETE members/:user` — gated: caller must be world GM or server admin (`permission_context(...).world_role == Gm`). `GET members` returns the list for a GM/admin.
- [ ] **Step 3: Document writes route through the shared path.** For create/update/delete, resolve `ctx = permission_context(world, user, role)?` (403 on Forbidden), get the room (`state.ws.rooms.get_or_create(repo, world)`), call `room.publish(repo, &ctx, ops, now)`. Map `Ok(cmd)` → 200 + filtered command; `Err(Forbidden)` → 403; `Err(Conflict)` → 409; `Err(BadPath|TooLarge)` → 422; else 500. PATCH body carries the `Vec<FieldChange>`; the handler wraps it in `Operation::Update { doc_id, changes }`.
- [ ] **Step 4: Document reads.** `GET /api/documents/:id`: load, build ctx for its world, `resolve_access`; `can_read` ? return `filter_properties(doc, access)` : 404. `GET /api/worlds/:id/documents?type=`: `query_documents`, filter each, drop unreadable.
- [ ] **Step 5: Tests** (axum-test, in `http/mod.rs` tests or routes tests):
  - non-member create-document → 403; GM create → 200; player update of a GM-only-writable doc → 403.
  - same-field conflicting PATCH → 409.
  - GET as player strips a GmOnly property.
  - membership add by non-GM → 403.
- [ ] **Step 6: Wire routes** in `http/mod.rs` `router()`. Run `cargo test -p shadowcat http` → PASS.
- [ ] **Step 7: Commit.**
```bash
git add src/server/src/http/routes.rs src/server/src/http/mod.rs
git commit -m "feat(m5): HTTP document CRUD, membership, and world-create endpoints"
```

---

### Task 9: Convergence/integration harness — intents replace EmitTest

**Files:**
- Modify: `src/server/tests/ws_convergence.rs`

**Interfaces:**
- Consumes: the new `Intent`/`Event`/`Reject` protocol; membership; `create_world_owned`.

- [ ] **Step 1: Update `spawn()`** to make the seeded user a GM of the world: replace `create_world("test", 0)` with `create_world_owned("test", <seeded user id>, 0)`. The seeded `u` must be the world GM so its intents are authorized. Capture the user id from `create_user`.
- [ ] **Step 2: Replace `emit(nonce)`** with an `intent` helper that sends a `Create` (first call) then `Update`s, or a minimal repeatable `Create` of distinct docs:
```rust
fn create_intent(n: u64) -> Message {
    let id = uuid::Uuid::from_u128(1000 + n as u128);
    Message::Text(serde_json::json!({
        "type": "intent",
        "intent_id": uuid::Uuid::from_u128(n as u128),
        "ops": [{ "op": "create", "doc": minimal_doc(id) }]
    }).to_string())
}
```
where `minimal_doc(id)` builds a valid world-scoped `Document` JSON (id, scope {kind:"world", world_id}, doc_type, schema_version:1, system:{}, created_at, updated_at, permissions default, embedded {}). The world_id must be the harness world.
- [ ] **Step 3: Update existing convergence tests** — `join_welcome_emit_receive`, `all_clients_converge_after_reconnect`, `slow_reader_recovers_via_resync`, `converges_with_publishing_during_resync` — to send `create_intent(n)` (distinct ids) instead of `emit(n)`. The seq assertions are unchanged (each create is one sequenced event). `drain_event_seqs` still keys on `v["type"] == "event"` and `v["command"]["seq"]`.
- [ ] **Step 4: Add permission/conflict integration tests:**
  - `conflicting_same_field_update_is_rejected`: GM client creates a doc (seq 1), then sends two Updates to `/system/hp` with the same `old` pre-image back-to-back without reading between; assert one `Event` (seq 2) and one `Reject{reason:"conflict"}` arrive.
  - `player_write_forbidden`: add a second user as `player` (via the GM's HTTP `POST members` or `repo.add_member`), connect as that user, send an Update intent to a GM-owned doc → expect `Reject{reason:"forbidden"}`.
  - `gm_only_property_hidden_from_player`: GM creates a doc with a `/system/secret` GmOnly override and a public field; a player client (member) receives the create Event with `secret` stripped but `public` present, and the seq still advances.
- [ ] **Step 5: Run** `cargo test -p shadowcat --test ws_convergence` → PASS.
- [ ] **Step 6: Commit** (if not already folded into the 5+6+7 commit; otherwise its own commit):
```bash
git add src/server/tests/ws_convergence.rs
git commit -m "feat(m5): convergence harness uses real intents; permission/conflict cases"
```

---

### Task 10: Telemetry, lint, docs sync

**Files:**
- Modify: `src/server/src/ws/conn.rs` (tracing on reject/forbidden), `docs/PLAN.md`, `docs/TODO.md`

- [ ] **Step 1: Tracing.** `tracing::debug!` on intent reject (reason), `tracing::info!` on membership-denied join. No `println!`.
- [ ] **Step 2: Format + lint.** `cargo fmt --all` ; `cargo clippy --all-targets -- -D warnings` → clean.
- [ ] **Step 3: Full test + bindings sync.** `cargo test -p shadowcat` ; `git diff --exit-code src/types/generated`.
- [ ] **Step 4: Docs.** `docs/PLAN.md`: M5 → ✅. `docs/TODO.md`: remove the "Enforce validation::validate_system_size / validate_field_path on the write path" item (now wired by `apply_intent`); keep the `set_pointer` removal-semantics item (still unaddressed — Update writes null, not delete-key).
- [ ] **Step 5: Commit.**
```bash
git add src/server/src/ws/conn.rs docs/PLAN.md docs/TODO.md
git commit -m "feat(m5): reject/deny telemetry; mark M5 complete; close validation TODO"
```

---

## Self-Review

**1. Spec coverage** (spec §11 decisions → tasks):
- world_members + roles + owned world creation → Task 1; membership management → Task 8.
- PermissionContext + resolver → Task 2.
- Field-level OCC (intent/confirm/reject) → Task 3 (apply_intent) + Task 6/7 (publish/confirm) + Task 9 (conflict test).
- One core write path, WS Intent + HTTP CRUD → Task 6 (publish) + Task 7 (WS) + Task 8 (HTTP).
- Per-recipient filtering, seq-preserving → Task 4 (filter_command) + Task 7 (egress/resync).
- HTTP filtered GET reads → Task 8.
- Copy independence (id-absent enforcement + Source) → Task 3 (Create id-absent check); Source provenance is carried on the Document the client sends (no server change beyond faithful storage).
- Asset-UUID references → carried as a `system`/property value; structural validity is the field-path/size check (no dedicated code; documented as client-supplied, M8 adds upload).
- Validation wired → Task 3; TODO closed → Task 10.
- EmitTest retired → Task 5/6 + Task 9.

**2. Placeholder scan:** Tasks 5/6/7 are explicitly a mutually-dependent unit committed together (protocol shape ↔ callers) — not placeholders; each has concrete code. No `TODO`/`TBD` in shipped code.

**3. Type consistency:** `PermissionContext{user_id, world_role}` consistent across Tasks 2-8. `apply_intent(ctx, world_id, ops, ts)` consistent (Tasks 3, 6). `filter_command(repo, cmd, ctx)` consistent (Tasks 4, 7). `Event{command, intent_id}` consistent (Tasks 5, 6, 7, 9). `RejectReason{Forbidden,Conflict,Invalid}` consistent (Tasks 5, 6, 8).

**Note (spec §6 refinement):** `filter_command` is async and loads each `Update` op's document via `repo.get_document` (the command's Update ops carry only deltas, not the doc's PermissionSet). One read per Update op per recipient — acceptable at M5 scale; the attach-perms-to-broadcast optimization is deferred.

## Buddy-check directives

M5 is high-risk: a **permission/security surface** (per-recipient filtering, write authorization, membership gating) plus **concurrency correctness** (in-transaction pre-image OCC, the confirm/broadcast dedup interplay with M4's sequence guard). A buddy-check (two independent blind reviewers + debate) is **offered** for the final branch review, above the single dispatched review. For M3 a buddy-check caught a login timing oracle; the security surface here is comparable. Default if not requested: single fresh-context branch review. Outcome recorded at execution handoff.
