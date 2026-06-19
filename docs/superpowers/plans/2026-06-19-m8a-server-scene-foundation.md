# M8a — Server Scene Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: per the user's global guidance, execute this plan with the **mainline-plan-execution** skill (inline per-task spec-compliance check + one final branch review), NOT subagent-driven-development. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the pure-server foundation for scenes — the `parent_id` scene-entity document model, a per-world hecs ECS hydrated from documents, and the `SceneDerived` per-recipient dispatch channel — proven end-to-end by a debug-gated identity consumer, with no rendering and no UI.

**Architecture:** Scene entities (tokens, walls, tiles, …) are top-level `Document`s linked to their scene by a new `parent_id`. Documents stay the sole authority via the existing M5/M6 `apply_intent` pipeline; a per-world `hecs::World` is a *derived read-model* hydrated from those documents and kept in sync inline in the commit path (`Room::publish`, after `apply_intent` commits, before broadcast). Engine-derived state dispatches on a new `SceneDerived` channel that generalizes the M6c-2 live-search egress machinery (per-connection subscription, leading-edge 150 ms coalescing, fingerprint-suppressed pushes, `computed_at_seq` watermark). M8a ships only the seam: a debug-gated `identity` consumer that emits the world's scene-entity count.

**Tech Stack:** Rust, axum 0.8, sqlx 0.9 (SQLite), tokio 1.52, hecs 0.11 (new), ts-rs 12, serde_json.

## Global Constraints

Every task's requirements implicitly include these (values copied verbatim from `ARCHITECTURE.md` and the M8 design spec `docs/superpowers/specs/2026-06-19-m8-ecs-scene-rendering-design.md`):

- **#5 Documents are the source of truth; the ECS is hydrated from documents and is ephemeral.** The hecs world is never persisted and never authoritative.
- **#8 Mutations flow through an undoable boundary.** Every state change is a reversible `Operation` in a sequenced `Command`; nothing mutates authoritative state outside `apply_intent`. Scene/parent deletion expands to explicit descendant Delete ops — never a silent SQL cascade.
- **#6 Server is structural-only.** No semantic interpretation of the `system` body; the ECS stores the document as hydrated, it does not validate game rules.
- **#4 Permissions enforced server-side, per recipient.** `compute_derived` receives the recipient's `PermissionContext`; the channel is built to filter per recipient (M8a's `identity` payload is non-sensitive and global; real per-recipient derivation lands with M9 vision).
- **#2 Ordered, recoverable.** `SceneDerived` carries `computed_at_seq` (an `i64`, matching `Command.seq`) so the client applies it after the document events it reflects; derived state is recomputed on resync, never replayed.
- **Cross-platform (#10).** No hardcoded path separators; the `parent_id` migration is plain SQLite; CI runs the three-OS matrix.
- **ts-rs sync.** New wire types derive `TS` with `#[ts(export, export_to = "../../types/generated/")]`; `cargo test --all` regenerates bindings; CI runs `git diff --exit-code src/types/generated` (Linux). Add each new generated type to `src/types/index.ts`.
- **No debug code in release.** The identity consumer is gated behind `#[cfg(debug_assertions)]`; diagnostics use `tracing`, never `println!`/`dbg!`.
- **DRY, YAGNI, TDD, frequent commits.** One reversible deliverable per task.

---

## File Structure

**New files:**
- `src/server/migrations/0005_scene_entities.sql` — `parent_id` column + index + FK, and an `AFTER DELETE` FTS-cleanup trigger.
- `src/server/src/scene/mod.rs` — the per-world derived ECS: `SceneEcs`, `SceneEntity` component, `is_scene_entity`, `compute_derived`. One responsibility: hold and mutate the hydrated scene world and compute derived payloads.

**Modified files:**
- `src/server/Cargo.toml` — add `hecs = "0.11"`.
- `src/server/src/lib.rs` — register `pub mod scene;`.
- `src/server/src/data/document.rs` — add `Document.parent_id: Option<Uuid>`.
- `src/server/src/data/sqlite.rs` — persist `parent_id` (column bind in `upsert_document`); `query_children`; descendant-delete expansion in `apply_intent`.
- `src/server/src/data/repository.rs` — add `query_children` to the trait.
- `src/server/src/ws/room.rs` — `Room` holds `RwLock<SceneEcs>`; `Room::publish` hydrates inline; `RoomRegistry::get_or_create` builds the initial ECS.
- `src/server/src/ws/protocol.rs` — `ClientMsg::SceneSubscribe`/`SceneUnsubscribe`; `ServerMsg::SceneDerived`/`SceneError`.
- `src/server/src/ws/conn.rs` — egress scene-subscription registry + recompute folded into the existing debounce.
- `src/types/index.ts` — re-export new generated types.

**New test files:**
- `src/server/tests/scene_hydration.rs` — ECS hydration + cascade-delete integration tests over the WS harness.
- `src/server/tests/scene_derived.rs` — `SceneDerived` subscribe/coalesce/push integration tests.

---

## Task 1: `parent_id` scene-entity column + document field + child query

**Files:**
- Create: `src/server/migrations/0005_scene_entities.sql`
- Modify: `src/server/src/data/document.rs:124-145` (Document struct)
- Modify: `src/server/src/data/sqlite.rs` (`upsert_document` ~453-514; add `query_children`)
- Modify: `src/server/src/data/repository.rs` (trait)
- Test: inline `#[cfg(test)]` in `sqlite.rs`

**Interfaces:**
- Produces: `Document.parent_id: Option<Uuid>`; `Repository::query_children(&self, parent_id: Uuid) -> Result<Vec<Document>, DataError>`.
- Consumes: existing `upsert_document`, `query_documents`, the `documents` table.

- [ ] **Step 1: Write the migration**

Create `src/server/migrations/0005_scene_entities.sql`:

```sql
-- Scene entities (tokens, walls, tiles, regions, lights, ...) are top-level
-- documents linked to their scene by parent_id. ON DELETE CASCADE is a DB
-- integrity backstop only: the authoritative delete path (apply_intent) expands
-- a parent delete into explicit, reversible Delete ops for every descendant, so
-- the event log and broadcasts stay complete. The trigger removes a deleted
-- document's FTS row, covering both the per-op delete and any cascade.
ALTER TABLE documents ADD COLUMN parent_id TEXT REFERENCES documents(id) ON DELETE CASCADE;
CREATE INDEX idx_documents_parent ON documents(parent_id);

CREATE TRIGGER documents_fts_delete AFTER DELETE ON documents BEGIN
  DELETE FROM documents_fts WHERE doc_id = old.id;
END;
```

- [ ] **Step 2: Add the `parent_id` field to `Document`**

In `src/server/src/data/document.rs`, add the field to the `Document` struct (after `embedded`, line ~140):

```rust
    #[serde(default)]
    pub embedded: BTreeMap<String, Vec<Document>>,
    /// Scene-entity link: the id of the scene (or other parent) this document
    /// belongs to. `None` for top-level documents (actors, compendium entries,
    /// scenes themselves). Immutable via field-path Update (envelope field).
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    #[ts(type = "unknown")]
    pub system: serde_json::Value,
```

- [ ] **Step 3: Run to verify it fails (sample_doc + persistence don't set parent_id yet)**

Run: `cargo test -p shadowcat data::document`
Expected: compile error or test failures referencing the missing `parent_id` initializer in `sample_doc()` and other `Document { … }` literals.

- [ ] **Step 4: Fix every `Document { … }` literal to include `parent_id: None`**

Add `parent_id: None,` to each struct literal flagged by the compiler (e.g. `document.rs` `sample_doc()` ~161-180; `command.rs` ~176; `migrate.rs` ~35; `search.rs` ~121; `validation.rs` ~43; `permission.rs` ~372; `sqlite.rs` ~1210, ~1837; `test_server.rs` ~50, ~81). Place it next to `embedded: …,` in each.

- [ ] **Step 5: Persist `parent_id` in `upsert_document`**

In `src/server/src/data/sqlite.rs` `upsert_document`, add the column to the INSERT and the conflict update. Extend the column list, the `VALUES` placeholders, and bind `doc.parent_id.map(|p| p.to_string())` in field order:

```rust
    sqlx::query(
        "INSERT INTO documents (id, scope_kind, world_id, pack, doc_type, schema_version, \
         source_id, source_pack, source_version, owner_id, parent_id, seq, json, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET \
           scope_kind=excluded.scope_kind, world_id=excluded.world_id, pack=excluded.pack, \
           doc_type=excluded.doc_type, schema_version=excluded.schema_version, \
           source_id=excluded.source_id, source_pack=excluded.source_pack, \
           source_version=excluded.source_version, owner_id=excluded.owner_id, \
           parent_id=excluded.parent_id, seq=excluded.seq, json=excluded.json, updated_at=excluded.updated_at",
    )
    // ...existing binds in order... then, where owner_id is bound, add immediately after it:
    .bind(doc.parent_id.map(|p| p.to_string()))
```

(`parent_id` is bound between `owner_id` and `seq` to match the column order above. `json` already round-trips the field, so loaders need no change.)

- [ ] **Step 6: Add `query_children` to the repository trait + impl**

In `src/server/src/data/repository.rs`, add to the trait:

```rust
    /// All documents whose `parent_id` equals `parent` (a scene's direct
    /// children). Ordered by id for determinism.
    async fn query_children(&self, parent: Uuid) -> Result<Vec<Document>, DataError>;
```

In `src/server/src/data/sqlite.rs`, implement next to `query_documents`:

```rust
    async fn query_children(&self, parent: Uuid) -> Result<Vec<Document>, DataError> {
        let rows = sqlx::query("SELECT json FROM documents WHERE parent_id = ? ORDER BY id")
            .bind(parent.to_string())
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("json").as_str())?))
            .collect()
    }
```

- [ ] **Step 7: Write the failing test**

Add to the `#[cfg(test)]` module in `sqlite.rs`:

```rust
    #[tokio::test]
    async fn parent_id_round_trips_and_query_children_filters() {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let world = repo.create_world_owned("w", Uuid::from_u128(1), 0).await.unwrap();
        let scene = Uuid::from_u128(10);
        let token = Uuid::from_u128(11);
        // Scene (no parent) + one child token.
        let mut scene_doc = crate::data::document::tests::world_scoped_doc(world.id, scene, "scene");
        scene_doc.parent_id = None;
        let mut token_doc = crate::data::document::tests::world_scoped_doc(world.id, token, "token");
        token_doc.parent_id = Some(scene);
        repo.apply_command(UnsequencedCommand {
            world_id: world.id, author: Uuid::from_u128(1), ts: 0,
            ops: vec![Operation::Create { doc: scene_doc }, Operation::Create { doc: token_doc }],
        }).await.unwrap();

        let children = repo.query_children(scene).await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id, token);
        assert_eq!(children[0].parent_id, Some(scene));
        // The scene itself has no parent, so it is not its own child.
        assert!(repo.query_children(token).await.unwrap().is_empty());
    }
```

Add the shared helper in `document.rs` (make its `tests` module `pub(crate)` so other crate test modules can reuse it):

```rust
pub(crate) fn world_scoped_doc(world_id: Uuid, id: Uuid, doc_type: &str) -> Document {
    let mut d = sample_doc();
    d.id = id;
    d.scope = Scope::World { world_id };
    d.doc_type = doc_type.to_string();
    d.parent_id = None;
    d
}
```

- [ ] **Step 8: Run the test**

Run: `cargo test -p shadowcat parent_id_round_trips`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add src/server/migrations/0005_scene_entities.sql src/server/src/data/ src/server/src/bin/test_server.rs
git commit -m "feat(m8a): parent_id scene-entity column, Document.parent_id, query_children"
```

---

## Task 2: Cascade-delete expansion into reversible descendant Delete ops

**Files:**
- Modify: `src/server/src/data/sqlite.rs` (`apply_intent`, before the authoritative-ops substitution ~742)
- Test: inline `#[cfg(test)]` in `sqlite.rs`

**Interfaces:**
- Consumes: `query_children` (Task 1), `load_document`, `Operation::Delete`.
- Produces: invariant — a `Delete` of a document with descendants yields, in the same `Command`, authoritative `Delete` ops for the document and every descendant (depth-first, children before parents), each authorized in Phase 1.

- [ ] **Step 1: Write the failing integration test**

Add to the `#[cfg(test)]` module in `sqlite.rs`:

```rust
    #[tokio::test]
    async fn deleting_a_scene_expands_to_descendant_delete_ops() {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let owner = Uuid::from_u128(1);
        let world = repo.create_world_owned("w", owner, 0).await.unwrap();
        let scene = Uuid::from_u128(10);
        let t1 = Uuid::from_u128(11);
        let t2 = Uuid::from_u128(12);
        let mk = |id, parent: Option<Uuid>, ty| {
            let mut d = crate::data::document::tests::world_scoped_doc(world.id, id, ty);
            d.parent_id = parent;
            d.owner = Some(owner);
            Operation::Create { doc: d }
        };
        repo.apply_command(UnsequencedCommand { world_id: world.id, author: owner, ts: 0,
            ops: vec![mk(scene, None, "scene"), mk(t1, Some(scene), "token"), mk(t2, Some(scene), "token")] })
            .await.unwrap();

        let ctx = repo.permission_context(world.id, owner, ServerRole::User).await.unwrap();
        // Delete the scene only; expect the Command to carry 3 Delete ops.
        let mut scene_doc = repo.get_document(scene).await.unwrap().unwrap();
        let cmd = repo.apply_intent(&ctx, world.id, vec![Operation::Delete { doc: scene_doc.clone() }], 1)
            .await.unwrap();
        let deleted: Vec<Uuid> = cmd.ops.iter().filter_map(|o| match o {
            Operation::Delete { doc } => Some(doc.id), _ => None }).collect();
        assert_eq!(deleted.len(), 3, "scene + 2 children");
        assert!(deleted.contains(&scene) && deleted.contains(&t1) && deleted.contains(&t2));
        // Children deleted before their parent (reversible-order invariant).
        let scene_pos = deleted.iter().position(|&d| d == scene).unwrap();
        assert!(deleted.iter().position(|&d| d == t1).unwrap() < scene_pos);
        // Store is empty for the world's scene entities.
        assert!(repo.query_children(scene).await.unwrap().is_empty());
        assert!(repo.get_document(t1).await.unwrap().is_none());
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p shadowcat deleting_a_scene_expands`
Expected: FAIL — `deleted.len()` is 1, descendants survive (or FK cascade removed rows but produced no ops).

- [ ] **Step 3: Implement descendant expansion in `apply_intent`**

In `src/server/src/data/sqlite.rs`, the authoritative-ops loop (~742-755) currently substitutes the stored doc for each `Delete`. Replace that loop so a `Delete` first collects descendants depth-first (children before parent) and emits an authoritative `Delete` op for each. Add a helper above `impl Repository` or as an associated fn:

```rust
    /// Depth-first descendant ids of `root` within one transaction (children
    /// before parents), using the parent_id index. Excludes `root`.
    async fn descendants_first(
        tx: &mut sqlx::SqliteConnection,
        root: Uuid,
    ) -> Result<Vec<Uuid>, DataError> {
        let mut out = Vec::new();
        let child_rows = sqlx::query("SELECT id FROM documents WHERE parent_id = ? ORDER BY id")
            .bind(root.to_string())
            .fetch_all(&mut *tx)
            .await?;
        for r in child_rows {
            let child = Uuid::parse_str(r.get::<String, _>("id").as_str())
                .map_err(|e| DataError::OpFailed(e.to_string()))?;
            // Recurse first so deeper descendants precede their parent.
            let mut sub = Box::pin(Self::descendants_first(&mut *tx, child)).await?;
            out.append(&mut sub);
            out.push(child);
        }
        Ok(out)
    }
```

Then rewrite the substitution loop:

```rust
        let mut authoritative_ops = Vec::with_capacity(ops.len());
        for op in ops {
            match op {
                Operation::Delete { doc } => {
                    // Children-first so each op is individually reversible and a
                    // replay re-creates parents before children.
                    for desc in Self::descendants_first(&mut *tx, doc.id).await? {
                        let cur = Self::load_document(&mut *tx, desc).await?.ok_or_else(|| {
                            DataError::Conflict(format!("descendant {desc} missing"))
                        })?;
                        authoritative_ops.push(Operation::Delete { doc: cur });
                    }
                    let cur = Self::load_document(&mut *tx, doc.id).await?.ok_or_else(|| {
                        DataError::Conflict(format!("document {} missing", doc.id))
                    })?;
                    authoritative_ops.push(Operation::Delete { doc: cur });
                }
                other => authoritative_ops.push(other),
            }
        }
```

Note: Phase-1 authorization (~670-684) checks the explicitly-submitted Delete only. Because descendants are discovered here in Phase 2, add a per-descendant `DELETE` capability check inside the loop, reusing the same access resolver the Phase-1 Delete arm uses (`resolve_access` / the existing delete-capability check), and return its error early so the whole command rolls back if the user cannot delete a descendant.

- [ ] **Step 4: Run the test**

Run: `cargo test -p shadowcat deleting_a_scene_expands`
Expected: PASS.

- [ ] **Step 5: Run the full data suite (no regressions in existing delete tests)**

Run: `cargo test -p shadowcat data::`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/data/sqlite.rs
git commit -m "feat(m8a): expand parent delete into reversible descendant Delete ops"
```

---

## Task 3: `hecs` dependency + per-world `SceneEcs` + initial hydration

**Files:**
- Modify: `src/server/Cargo.toml`
- Create: `src/server/src/scene/mod.rs`
- Modify: `src/server/src/lib.rs` (add `pub mod scene;`)
- Test: inline `#[cfg(test)]` in `scene/mod.rs`

**Interfaces:**
- Produces:
  - `pub struct SceneEntity { pub doc: Document }` (hecs component).
  - `pub struct SceneEcs { /* private: world, index */ }` with:
    - `pub fn new() -> Self`
    - `pub fn from_documents(docs: Vec<Document>) -> Self`
    - `pub fn apply_op(&mut self, op: &Operation)`
    - `pub fn entity_count(&self) -> usize`
  - `pub fn is_scene_entity(doc: &Document) -> bool`
- Consumes: `Document` (Task 1), `Operation`, `set_pointer` (from `data::command`).

- [ ] **Step 1: Add the dependency**

In `src/server/Cargo.toml`, under `[dependencies]`, after `dashmap = "6"`:

```toml
hecs = "0.11"
```

- [ ] **Step 2: Register the module**

In `src/server/src/lib.rs`, add alongside the other `pub mod` lines:

```rust
pub mod scene;
```

- [ ] **Step 3: Write `SceneEcs` with the failing test first**

Create `src/server/src/scene/mod.rs`:

```rust
//! Per-world derived scene ECS. Hydrated from documents (#5); never persisted,
//! never authoritative. Holds one hecs entity per scene-entity document so
//! engine-owned systems (M9 vision, M10 pathfinding) can query spatial state.

use std::collections::HashMap;

use uuid::Uuid;

use crate::data::command::{set_pointer, Operation};
use crate::data::document::Document;

/// A hydrated scene-entity document, one per hecs entity.
pub struct SceneEntity {
    pub doc: Document,
}

/// A document is scene runtime state if it is a scene or a child of one.
pub fn is_scene_entity(doc: &Document) -> bool {
    doc.doc_type == "scene" || doc.parent_id.is_some()
}

/// The per-world derived world. Writes are serialized by the caller
/// (`Room::publish` under `publish_guard`); reads (derived recompute) take a
/// shared borrow.
pub struct SceneEcs {
    world: hecs::World,
    index: HashMap<Uuid, hecs::Entity>,
}

impl SceneEcs {
    pub fn new() -> Self {
        Self { world: hecs::World::new(), index: HashMap::new() }
    }

    /// Hydrate from a document set (scene entities only; others are ignored).
    pub fn from_documents(docs: Vec<Document>) -> Self {
        let mut ecs = Self::new();
        for doc in docs {
            if is_scene_entity(&doc) {
                let id = doc.id;
                let e = ecs.world.spawn((SceneEntity { doc },));
                ecs.index.insert(id, e);
            }
        }
        ecs
    }

    /// Reflect one already-committed authoritative op into the derived world.
    pub fn apply_op(&mut self, op: &Operation) {
        match op {
            Operation::Create { doc } if is_scene_entity(doc) => {
                if let Some(&e) = self.index.get(&doc.id) {
                    let _ = self.world.despawn(e);
                }
                let e = self.world.spawn((SceneEntity { doc: doc.clone() },));
                self.index.insert(doc.id, e);
            }
            Operation::Update { doc_id, changes } => {
                if let Some(&e) = self.index.get(doc_id) {
                    if let Ok(mut comp) = self.world.get::<&mut SceneEntity>(e) {
                        // Mirror the same field-path changes apply_intent applied
                        // to SQLite, via Value round-trip (server stays
                        // structural-only; no semantic interpretation).
                        if let Ok(mut v) = serde_json::to_value(&comp.doc) {
                            for ch in changes {
                                let _ = set_pointer(&mut v, &ch.path, ch.new.clone());
                            }
                            if let Ok(updated) = serde_json::from_value::<Document>(v) {
                                comp.doc = updated;
                            }
                        }
                    }
                }
            }
            Operation::Delete { doc } => {
                if let Some(e) = self.index.remove(&doc.id) {
                    let _ = self.world.despawn(e);
                }
            }
            Operation::Create { .. } => {} // non-scene document: ignored
        }
    }

    /// Count of hydrated scene entities (the M8a identity payload source).
    pub fn entity_count(&self) -> usize {
        self.index.len()
    }
}

impl Default for SceneEcs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(id: u128, parent: Option<u128>, ty: &str) -> Document {
        let mut d = crate::data::document::tests::world_scoped_doc(
            Uuid::from_u128(9), Uuid::from_u128(id), ty);
        d.parent_id = parent.map(Uuid::from_u128);
        d
    }

    #[test]
    fn hydrate_counts_scene_entities_only() {
        let ecs = SceneEcs::from_documents(vec![
            doc(10, None, "scene"),
            doc(11, Some(10), "token"),
            doc(99, None, "actor"), // not a scene entity → ignored
        ]);
        assert_eq!(ecs.entity_count(), 2);
    }

    #[test]
    fn apply_op_create_update_delete() {
        let mut ecs = SceneEcs::new();
        ecs.apply_op(&Operation::Create { doc: doc(11, Some(10), "token") });
        assert_eq!(ecs.entity_count(), 1);
        ecs.apply_op(&Operation::Update {
            doc_id: Uuid::from_u128(11),
            changes: vec![crate::data::command::FieldChange {
                path: "/system/x".into(), old: json!(null), new: json!(5),
            }],
        });
        let e = ecs.index[&Uuid::from_u128(11)];
        let comp = ecs.world.get::<&SceneEntity>(e).unwrap();
        assert_eq!(comp.doc.system["x"], json!(5));
        drop(comp);
        ecs.apply_op(&Operation::Delete { doc: doc(11, Some(10), "token") });
        assert_eq!(ecs.entity_count(), 0);
    }
}
```

This reuses `world_scoped_doc` (added to `document.rs`'s `pub(crate)` test module in Task 1, Step 7).

- [ ] **Step 4: Run to verify it fails, then passes**

Run: `cargo test -p shadowcat scene::`
Expected: first FAIL if the helper is missing (compile), then PASS once `SceneEcs` + helper compile.

- [ ] **Step 5: Commit**

```bash
git add src/server/Cargo.toml src/server/src/lib.rs src/server/src/scene/ src/server/src/data/document.rs
git commit -m "feat(m8a): hecs dep + per-world SceneEcs derived read-model"
```

---

## Task 4: Hydration mutation boundary in `Room::publish` + initial hydration on room create

**Files:**
- Modify: `src/server/src/ws/room.rs` (`Room` struct ~105-112; `Room::new` ~115-125; `Room::publish` ~146-164; `RoomRegistry::get_or_create` ~221-247)
- Test: inline `#[cfg(test)]` in `room.rs`

**Interfaces:**
- Consumes: `SceneEcs::from_documents`, `SceneEcs::apply_op` (Task 3); `Repository::query_documents`, `query_children` (Task 1).
- Produces: `Room::scene(&self) -> &tokio::sync::RwLock<SceneEcs>` (read access for the egress recompute, Task 6); the invariant that after `publish` returns, the ECS reflects the committed `Command` at `current_seq`.

- [ ] **Step 1: Add `RwLock<SceneEcs>` to `Room`**

In `src/server/src/ws/room.rs`, extend the struct and constructor:

```rust
use tokio::sync::RwLock;
use crate::scene::SceneEcs;

pub struct Room {
    pub world_id: Uuid,
    tx: broadcast::Sender<Arc<ServerMsg>>,
    ring: Mutex<RingBuffer>,
    publish_guard: Mutex<()>,
    current_seq: AtomicI64,
    scene: RwLock<SceneEcs>,
    pub stats: RoomStats,
}

impl Room {
    fn new(world_id: Uuid, seed_seq: i64, scene: SceneEcs) -> Self {
        let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            world_id,
            tx,
            ring: Mutex::new(RingBuffer::new()),
            publish_guard: Mutex::new(()),
            current_seq: AtomicI64::new(seed_seq),
            scene: RwLock::new(scene),
            stats: RoomStats::default(),
        }
    }

    /// Read access to the derived scene ECS for the per-connection derived
    /// recompute. Writes happen only in `publish` under `publish_guard`.
    pub fn scene(&self) -> &RwLock<SceneEcs> {
        &self.scene
    }
```

- [ ] **Step 2: Hydrate the ECS inline in `publish`, before broadcast**

In `Room::publish`, after `apply_intent` returns the committed `cmd` and before `self.tx.send(msg)`:

```rust
        let _guard = self.publish_guard.lock().await;
        let cmd = repo.apply_intent(ctx, self.world_id, ops, ts).await?;
        // Hydrate the derived ECS from the committed command while still holding
        // publish_guard, so the ECS is consistent with cmd.seq before the Event
        // (and any derived recompute keyed to that seq) is observable.
        {
            let mut scene = self.scene.write().await;
            for op in &cmd.ops {
                scene.apply_op(op);
            }
        }
        let msg = Arc::new(ServerMsg::Event { command: cmd.clone(), intent_id: None });
        self.ring.lock().await.push(msg.clone());
        self.current_seq.store(cmd.seq, Ordering::Release);
        let _ = self.tx.send(msg);
        self.stats.events_published.fetch_add(1, Ordering::Relaxed);
        Ok(cmd)
```

- [ ] **Step 3: Build the initial ECS in `get_or_create`**

In `RoomRegistry::get_or_create`, on the miss path, load the world's scene entities and pass them to `Room::new`. Scene entities = all scene docs plus their children. Load scenes via `query_documents(world_id, "scene")`, then their children via `query_children`:

```rust
        let Some(world) = repo.get_world(world_id).await? else {
            return Ok(None);
        };
        // Hydrate the derived ECS from persisted scene entities (#5).
        let scenes = repo.query_documents(world_id, "scene").await?;
        let mut docs = Vec::new();
        for scene in &scenes {
            docs.extend(repo.query_children(scene.id).await?);
        }
        docs.extend(scenes);
        let scene_ecs = crate::scene::SceneEcs::from_documents(docs);
        let room = self
            .rooms
            .entry(world_id)
            .or_insert_with(|| Arc::new(Room::new(world_id, world.seq, scene_ecs)))
            .clone();
        Ok(Some(room))
```

- [ ] **Step 4: Update other `Room::new` call sites**

If any test or code calls `Room::new(world_id, seq)`, update to `Room::new(world_id, seq, SceneEcs::new())`. Run `cargo build -p shadowcat` and fix each flagged call site.

- [ ] **Step 5: Write the failing test**

Add to the `#[cfg(test)]` module in `room.rs` (build a repo + room and assert the ECS tracks a published create):

```rust
    #[tokio::test]
    async fn publish_hydrates_scene_ecs() {
        let repo = std::sync::Arc::new(
            crate::data::sqlite::SqliteRepository::connect("sqlite::memory:").await.unwrap());
        let owner = Uuid::from_u128(1);
        let world = repo.create_world_owned("w", owner, 0).await.unwrap();
        let reg = RoomRegistry::new();
        let room = reg.get_or_create(repo.as_ref(), world.id).await.unwrap().unwrap();
        assert_eq!(room.scene().read().await.entity_count(), 0);

        let ctx = repo.permission_context(world.id, owner, ServerRole::User).await.unwrap();
        let mut tok = crate::data::document::tests::world_scoped_doc(world.id, Uuid::from_u128(20), "token");
        tok.parent_id = Some(Uuid::from_u128(10));
        tok.owner = Some(owner);
        room.publish(repo.as_ref(), &ctx, vec![Operation::Create { doc: tok }], 0).await.unwrap();
        assert_eq!(room.scene().read().await.entity_count(), 1);
    }
```

- [ ] **Step 6: Run the test**

Run: `cargo test -p shadowcat publish_hydrates_scene_ecs`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/server/src/ws/room.rs
git commit -m "feat(m8a): hydrate per-world SceneEcs inline in Room::publish"
```

---

## Task 5: `SceneDerived` wire frames

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (`ClientMsg` ~14-48; `ServerMsg` ~82-144)
- Modify: `src/types/index.ts`
- Test: inline `#[cfg(test)]` in `protocol.rs`

**Interfaces:**
- Produces (wire):
  - `ClientMsg::SceneSubscribe { request_id: Uuid, channel: String }`
  - `ClientMsg::SceneUnsubscribe { request_id: Uuid }`
  - `ServerMsg::SceneDerived { request_id: Uuid, channel: String, computed_at_seq: i64, payload: serde_json::Value }`
  - `ServerMsg::SceneError { request_id: Uuid, message: String }`
- Consumes: existing `#[serde(tag = "type", rename_all = "snake_case")]` tagging + ts-rs derive on the enums.

- [ ] **Step 1: Add the client frames**

In `src/server/src/ws/protocol.rs`, add to `ClientMsg` (after `Unsubscribe`):

```rust
    /// Subscribe to a derived scene channel (e.g. M9 "vision"). M8a recognizes
    /// only the debug "identity" channel; unknown channels yield SceneError.
    SceneSubscribe { request_id: Uuid, channel: String },
    /// Cancel a derived subscription by request id.
    SceneUnsubscribe { request_id: Uuid },
```

- [ ] **Step 2: Add the server frames**

Add to `ServerMsg` (after `SearchUpdate`):

```rust
    /// A derived-state push: coalesced, per recipient, ordered after the
    /// document events it reflects via `computed_at_seq`. `payload` is opaque to
    /// the transport (#6).
    SceneDerived {
        request_id: Uuid,
        channel: String,
        computed_at_seq: i64,
        #[ts(type = "unknown")]
        payload: serde_json::Value,
    },
    /// A derived subscription failed (e.g. unknown channel).
    SceneError { request_id: Uuid, message: String },
```

- [ ] **Step 3: Write the failing round-trip test**

Add to the `#[cfg(test)]` module in `protocol.rs`:

```rust
    #[test]
    fn scene_frames_round_trip() {
        let sub = ClientMsg::SceneSubscribe { request_id: Uuid::from_u128(1), channel: "identity".into() };
        let j = serde_json::to_value(&sub).unwrap();
        assert_eq!(j["type"], "scene_subscribe");
        assert_eq!(j["channel"], "identity");

        let d = ServerMsg::SceneDerived {
            request_id: Uuid::from_u128(1), channel: "identity".into(),
            computed_at_seq: 7, payload: serde_json::json!({ "entity_count": 3 }),
        };
        let j = serde_json::to_value(&d).unwrap();
        assert_eq!(j["type"], "scene_derived");
        assert_eq!(j["computed_at_seq"], 7);
        assert_eq!(j["payload"]["entity_count"], 3);
    }
```

- [ ] **Step 4: Run the test + regenerate bindings**

Run: `cargo test -p shadowcat scene_frames_round_trip && cargo test --all`
Expected: PASS; `src/types/generated/ClientMsg.ts` and `ServerMsg.ts` now include the new variants.

- [ ] **Step 5: Re-export any new generated types**

`ClientMsg`/`ServerMsg` are already re-exported from `src/types/index.ts`, so no edit is needed unless a new standalone type file appears. Verify:

Run: `cargo test --all && git status --porcelain src/types/generated`
Expected: only `ClientMsg.ts` / `ServerMsg.ts` changed. If a new file appeared, add a matching `export type { … } from "./generated/…";` line to `src/types/index.ts`.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/ws/protocol.rs src/types/generated src/types/index.ts
git commit -m "feat(m8a): SceneDerived/SceneError + SceneSubscribe wire frames"
```

---

## Task 6: Egress scene-subscription registry + coalesced derived recompute

**Files:**
- Modify: `src/server/src/ws/conn.rs` (`Egress` enum ~40-57; constants ~59-63; ingress match for the new ClientMsg ~244-275; `egress_loop` subscription state ~355 and the three arms ~359-484)
- Modify: `src/server/src/scene/mod.rs` (add `compute_derived`)

**Interfaces:**
- Consumes: `Room::scene()` (Task 4), `SceneEcs::entity_count` (Task 3), `PermissionContext`, the existing `reeval_deadline` debounce + `SEARCH_DEBOUNCE`.
- Produces: `pub fn compute_derived(channel: &str, ecs: &SceneEcs, ctx: &PermissionContext) -> Option<serde_json::Value>`; egress handling that pushes `ServerMsg::SceneDerived` on fingerprint change, coalesced on the same leading-edge timer as search.

- [ ] **Step 1: Add `compute_derived` (identity consumer, debug-gated)**

In `src/server/src/scene/mod.rs`:

```rust
use crate::data::membership::PermissionContext;

/// Compute a derived payload for `channel` from the scene ECS, for one
/// recipient. Returns `None` for unknown channels (→ SceneError). `ctx` is
/// accepted so M9 vision can derive per recipient; the M8a identity payload is
/// non-sensitive and global.
pub fn compute_derived(
    channel: &str,
    ecs: &SceneEcs,
    _ctx: &PermissionContext,
) -> Option<serde_json::Value> {
    match channel {
        // Seam proof only; replaced when M9 vision lands. Absent in release.
        #[cfg(debug_assertions)]
        "identity" => Some(serde_json::json!({ "entity_count": ecs.entity_count() })),
        _ => None,
    }
}
```

- [ ] **Step 2: Extend the `Egress` enum and constant**

In `conn.rs`, add to the `Egress` enum:

```rust
    SceneSubscribe { request_id: Uuid, channel: String },
    SceneUnsubscribe { request_id: Uuid },
```

Add near `MAX_SUBSCRIPTIONS`:

```rust
const MAX_SCENE_SUBSCRIPTIONS: usize = 16;
```

Add a derived-sub record near `struct Sub`:

```rust
struct SceneSub {
    channel: String,
    fingerprint: Option<serde_json::Value>,
}
```

- [ ] **Step 3: Route the new client frames in the ingress loop**

In the ingress match (alongside the `Search`/`Unsubscribe` arms), add:

```rust
                ClientMsg::SceneSubscribe { request_id, channel } => {
                    let _ = etx.send(Egress::SceneSubscribe { request_id, channel }).await;
                }
                ClientMsg::SceneUnsubscribe { request_id } => {
                    let _ = etx.send(Egress::SceneUnsubscribe { request_id }).await;
                }
```

- [ ] **Step 4: Hold scene-sub state in `egress_loop` and handle register/unregister**

Near `let mut subs … = HashMap::new();`, add:

```rust
    let mut scene_subs: std::collections::HashMap<Uuid, SceneSub> = std::collections::HashMap::new();
```

In the `Egress` arm of the multiplex loop, handle the new variants. On subscribe, run an initial recompute and either push `SceneDerived` (and store the fingerprint) or send `SceneError` for an unknown channel:

```rust
                Egress::SceneSubscribe { request_id, channel } => {
                    if scene_subs.len() >= MAX_SCENE_SUBSCRIPTIONS {
                        let f = ServerMsg::SceneError { request_id, message: "subscription limit".into() };
                        if sink.send(text(&f)).await.is_err() { return; }
                    } else {
                        let (payload, seq) = {
                            let ecs = room.scene().read().await;
                            (crate::scene::compute_derived(&channel, &ecs, &ctx), room.current_seq())
                        };
                        match payload {
                            Some(p) => {
                                let f = ServerMsg::SceneDerived {
                                    request_id, channel: channel.clone(),
                                    computed_at_seq: seq, payload: p.clone(),
                                };
                                if sink.send(text(&f)).await.is_err() { return; }
                                scene_subs.insert(request_id, SceneSub { channel, fingerprint: Some(p) });
                            }
                            None => {
                                let f = ServerMsg::SceneError { request_id, message: format!("unknown channel: {channel}") };
                                if sink.send(text(&f)).await.is_err() { return; }
                            }
                        }
                    }
                }
                Egress::SceneUnsubscribe { request_id } => {
                    scene_subs.remove(&request_id);
                }
```

- [ ] **Step 5: Arm the debounce for scene subs too**

In the broadcast (Event) arm, the existing debounce arms when `!subs.is_empty()`. Change the idle-arming condition to also consider scene subs:

```rust
                    if (!subs.is_empty() || !scene_subs.is_empty()) && reeval_deadline.is_none() {
                        reeval_deadline = Some(tokio::time::Instant::now() + SEARCH_DEBOUNCE);
                    }
```

- [ ] **Step 6: Recompute scene subs when the debounce fires**

In the debounce-timer arm (after the search re-eval loop), add a scene-sub re-eval that reads the ECS once and pushes only changed fingerprints:

```rust
            let (seq, snapshot) = {
                let ecs = room.scene().read().await;
                let mut out = Vec::new();
                for (id, s) in scene_subs.iter() {
                    out.push((*id, s.channel.clone(), crate::scene::compute_derived(&s.channel, &ecs, &ctx)));
                }
                (room.current_seq(), out)
            };
            for (id, channel, payload) in snapshot {
                if let Some(p) = payload {
                    let sub = scene_subs.get_mut(&id).expect("present");
                    if sub.fingerprint.as_ref() != Some(&p) {
                        sub.fingerprint = Some(p.clone());
                        let f = ServerMsg::SceneDerived { request_id: id, channel, computed_at_seq: seq, payload: p };
                        if sink.send(text(&f)).await.is_err() { return; }
                    }
                }
            }
```

(The ECS read borrow is dropped before sending, mirroring how the search arm collects results before awaiting the sink.)

- [ ] **Step 7: Build and run the WS unit tests**

Run: `cargo build -p shadowcat && cargo test -p shadowcat ws::`
Expected: PASS (no behavioral regressions; new paths exercised by the integration test in Task 7).

- [ ] **Step 8: Commit**

```bash
git add src/server/src/ws/conn.rs src/server/src/scene/mod.rs
git commit -m "feat(m8a): egress scene-subscription registry + coalesced SceneDerived recompute"
```

---

## Task 7: End-to-end integration tests (hydration + cascade + SceneDerived)

**Files:**
- Create: `src/server/tests/scene_hydration.rs`
- Create: `src/server/tests/scene_derived.rs`

**Interfaces:**
- Consumes: the WS `Harness` pattern from `src/server/tests/ws_convergence.rs` (`spawn`, `connect`, frame helpers); the new client/server frames (Task 5).

- [ ] **Step 1: Scene-hydration + cascade integration test**

Create `src/server/tests/scene_hydration.rs`. Reuse the harness shape from `ws_convergence.rs` (copy the `Harness`/`spawn`/`connect`/`intent_msg` helpers, or factor them into a shared `mod common;` if preferred). Add a `create_child_op` helper that sets `parent_id`, then:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scene_delete_cascades_as_events() {
    let h = spawn().await;
    let mut ws = h.connect().await;
    // Drain Welcome.
    let _ = ws.next().await.unwrap().unwrap();

    // Create a scene + two child tokens (one intent, three create ops).
    ws.send(create_scene_with_children(h.world, /*scene*/ 10, &[11, 12])).await.unwrap();
    let evt = drain_until_event(&mut ws).await;
    assert_eq!(evt["command"]["ops"].as_array().unwrap().len(), 3);

    // Delete the scene; expect one Event whose command carries 3 Delete ops.
    ws.send(delete_doc(h.world, 10)).await.unwrap();
    let evt = drain_until_event(&mut ws).await;
    let ops = evt["command"]["ops"].as_array().unwrap();
    assert_eq!(ops.len(), 3);
    assert!(ops.iter().all(|o| o["op"] == "delete"));
    // Authoritative store empty.
    assert!(h.repo.query_children(Uuid::from_u128(10)).await.unwrap().is_empty());
}
```

Provide `create_scene_with_children`, `delete_doc`, and `drain_until_event` helpers (mirroring `create_intent`/`drain_frames` from `ws_convergence.rs`; `delete_doc` sends an `intent` whose op is `{"op":"delete","doc":{…minimal envelope with id…}}`).

- [ ] **Step 2: Run the hydration test**

Run: `cargo test -p shadowcat --test scene_hydration`
Expected: PASS.

- [ ] **Step 3: `SceneDerived` subscribe + coalesced-push integration test**

Create `src/server/tests/scene_derived.rs`:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn identity_channel_pushes_on_scene_change() {
    let h = spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await.unwrap().unwrap(); // Welcome

    // Subscribe to the debug identity channel → initial SceneDerived (count 0).
    ws.send(scene_subscribe(1, "identity")).await.unwrap();
    let first = drain_until_type(&mut ws, "scene_derived").await;
    assert_eq!(first["payload"]["entity_count"], 0);

    // Create a scene entity; after coalescing, expect a SceneDerived with count 1
    // and a computed_at_seq >= the create's seq.
    ws.send(create_scene_with_children(h.world, 10, &[11])).await.unwrap();
    let upd = drain_until_type(&mut ws, "scene_derived").await;
    assert_eq!(upd["payload"]["entity_count"], 2); // scene + child
    assert!(upd["computed_at_seq"].as_i64().unwrap() >= 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_channel_errors() {
    let h = spawn().await;
    let mut ws = h.connect().await;
    let _ = ws.next().await.unwrap().unwrap();
    ws.send(scene_subscribe(2, "no_such_channel")).await.unwrap();
    let err = drain_until_type(&mut ws, "scene_error").await;
    assert!(err["message"].as_str().unwrap().contains("unknown channel"));
}
```

Add `scene_subscribe(request_n, channel)` (sends `{"type":"scene_subscribe","request_id":…,"channel":…}`) and `drain_until_type(ws, ty)` helpers.

- [ ] **Step 4: Run the derived test**

Run: `cargo test -p shadowcat --test scene_derived`
Expected: PASS. (If the coalesced push races the assertion, `drain_until_type` already loops with a timeout budget like `drain_frames`.)

- [ ] **Step 5: Full suite + bindings sync**

Run: `cargo test --all && git diff --exit-code src/types/generated`
Expected: PASS; no binding drift.

- [ ] **Step 6: Commit**

```bash
git add src/server/tests/scene_hydration.rs src/server/tests/scene_derived.rs
git commit -m "test(m8a): e2e scene hydration, cascade-as-events, SceneDerived coalescing"
```

---

## Final verification

- [ ] **Run the full server suite:** `cargo test --all` → PASS.
- [ ] **Lint/format:** `cargo fmt --all && cargo clippy --all-targets -- -D warnings` → clean.
- [ ] **Release build excludes the identity channel:** `cargo build --release` → builds; `compute_derived` has no `"identity"` arm under release (manual confirmation that the `#[cfg(debug_assertions)]` arm is the only one).
- [ ] **Bindings in sync (Linux parity):** `git diff --exit-code src/types/generated` → clean.
- [ ] **TS typecheck:** `pnpm -r typecheck` → PASS (new wire types are additive).

## Buddy-check directives

M8a is high-risk per the buddy-checking skill's signals: it modifies the **data foundation** (`apply_intent`, a schema migration with `ON DELETE CASCADE`), introduces an **app-level cascade-delete** whose authorization must be airtight, and adds a **new per-recipient dispatch channel**. This is the same risk class as the M6b capability slice and the M6c search core, both of which were buddy-checked (and both surfaced Critical bugs).

**Directive:** Offer a two-reviewer buddy-check at execution handoff, focused on: (1) cascade-delete authorization — can a user delete descendants they lack `core:delete` on, or escape it; ordering/atomicity under partial failure; (2) the `apply_intent` Phase-1 vs Phase-2 split now that descendants are discovered in Phase 2; (3) FK `ON DELETE CASCADE` vs the app-level expansion — can any path trigger a silent SQL cascade that bypasses the event log; (4) `SceneDerived` per-recipient correctness and the debug-gating of the identity channel (no dev-only behavior in release).

## Open follow-ups (logged, not M8a)

- Real per-recipient derived payloads (vision) — M9; `compute_derived` already takes `ctx`.
- Whether `parent_id` should ever be mutable (moving a token between scenes) — deferred to M8d; currently immutable via field-path Update (envelope field).
- Offloading the egress derived recompute off the egress task (the M6c-2 `TODO` at `conn.rs:448`) — revisit if recompute cost grows (M9 vision).
