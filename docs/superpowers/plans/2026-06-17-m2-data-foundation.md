# M2 — Data Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `mainline-plan-execution` to implement this plan task-by-task (per user-scope workflow guidance, which replaces `superpowers:subagent-driven-development` / `superpowers:executing-plans` for this project). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the pure-Rust data layer — document model, SQLite schema, transactional command log with `invert()`, durable per-world sequence, permission schema + pure helpers, validation, no-op migrate seam, and the `Repository` trait — with no HTTP and full unit-test coverage.

**Architecture:** A `data` module under `src/server/src/data/`. Documents are typed envelopes with an opaque `system` body and nested, addressable embedded docs; mutations are multi-document transactional `Command`s appended to an append-only per-world log; `invert()` is the single substrate shared by optimistic rollback and undo; a `Repository` trait abstracts storage with a `SqliteRepository` implementation. Source of design truth: [`docs/design/M2-data-foundation.md`](../../design/M2-data-foundation.md).

**Tech Stack:** Rust, sqlx 0.9 (SQLite, runtime queries), serde / serde_json, uuid, async-trait, thiserror; tests against in-memory + temp-file SQLite.

## Global Constraints

Copied from the spec and `docs/design/ARCHITECTURE.md`. Every task implicitly includes these.

- **SQLite-only.** No Postgres, no other DB. Use `sqlx` with the `sqlite` feature only.
- **No HTTP / no networking / no client in M2.** Pure data layer.
- **`system` body is opaque** — never inspected or validated semantically; type is `serde_json::Value`.
- **Envelope uses `#[serde(deny_unknown_fields)]`**; unknown envelope fields are rejected.
- **Commands are multi-document and transactional**; `apply_command` is all-or-nothing in one SQLite transaction.
- **Per-world sequence is the `worlds.seq` column** — single durable source, allocated inside the apply transaction.
- **Permissive licenses only** (new deps: uuid, serde_json, async-trait, thiserror — all MIT/Apache).
- **sqlx runtime queries** (`sqlx::query`/`query_as` with `.bind`), **not** the compile-time `query!` macro — keeps CI free of a database or `cargo sqlx prepare` step. Compile-time-checked queries can be adopted later.
- **No ts-rs exports for M2 types** — they gain `#[derive(TS)]` when the client first consumes them (M4/M6); M2 adds no generated TypeScript.
- Release profile, lint, and CI from M1 remain green (`cargo fmt`, `clippy -D warnings`, the ts-rs sync check, size budget).

---

## File Structure

```
src/server/
  Cargo.toml                      (modify: add uuid, serde_json, async-trait, thiserror; dev: tempfile)
  migrations/
    0001_init.sql                 (create: full schema)
  src/
    lib.rs                        (modify: `pub mod data;`)
    data/
      mod.rs                      (create: re-exports + DataError)
      document.rs                 (create: Document, Scope, Source, World, User, Member, role enums, PermissionSet)
      command.rs                  (create: Command, UnsequencedCommand, Operation, FieldChange, invert, set_pointer)
      permission.rs               (create: Access, resolve_access, filter_properties)
      validation.rs               (create: size cap, field-path validation)
      migrate.rs                  (create: CURRENT_SCHEMA_VERSION, migrate no-op)
      repository.rs               (create: Repository trait)
      sqlite.rs                   (create: SqliteRepository)
```

**Responsibility boundaries:** `document.rs` owns all persisted types; `command.rs` owns the mutation representation and JSON-pointer application; `permission.rs`, `validation.rs`, `migrate.rs` are pure helpers; `repository.rs` is the storage contract; `sqlite.rs` is the only implementation. `mod.rs` holds the shared `DataError`.

---

## Task 1: Core types & serialization

**Files:**
- Modify: `src/server/Cargo.toml`, `src/server/src/lib.rs`
- Create: `src/server/src/data/mod.rs`, `src/server/src/data/document.rs`

**Interfaces:**
- Produces: `data::DataError`; `data::document::{Document, Scope, Source, World, DocRole, Visibility, WorldRole, PermissionSet}`. Consumed by every later task.

- [ ] **Step 1: Add dependencies to `src/server/Cargo.toml`**

Append to `[dependencies]` and add a `[dev-dependencies]` section:

```toml
uuid = { version = "1", features = ["v4", "serde"] }
serde_json = "1"
async-trait = "0.1"
thiserror = "2"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Create `src/server/src/data/mod.rs` with the error type and module wiring**

```rust
pub mod document;

use thiserror::Error;

/// All fallible operations in the data layer return this.
#[derive(Debug, Error)]
pub enum DataError {
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("invalid field path: {0}")]
    BadPath(String),
    #[error("system body too large: {0} bytes")]
    TooLarge(usize),
    #[error("not found")]
    NotFound,
    #[error("operation failed: {0}")]
    OpFailed(String),
}
```

- [ ] **Step 3: Declare the module in `src/server/src/lib.rs`**

```rust
pub mod data;
pub mod db;
pub mod health;
```

- [ ] **Step 4: Write the failing test in `src/server/src/data/document.rs`**

```rust
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Storage/runtime scope of a document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    Compendium { pack: String },
    World { world_id: Uuid },
}

/// Provenance link for the deferred pull/push merge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: Uuid,
    pub pack: Option<String>,
    pub version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocRole {
    Owner,
    Observer,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    All,
    GmOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldRole {
    Gm,
    Player,
    Spectator,
}

/// Document-level permissions: default role, per-user overrides, and
/// property-level visibility keyed by JSON pointer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PermissionSet {
    pub default: DocRoleDefault,
    pub users: BTreeMap<Uuid, DocRole>,
    pub property_overrides: BTreeMap<String, Visibility>,
}

/// `DocRole` defaults to `None` for `PermissionSet::default()`.
pub type DocRoleDefault = DocRole;

impl Default for DocRole {
    fn default() -> Self {
        DocRole::None
    }
}

/// The persisted document: typed envelope around an opaque `system` body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub id: Uuid,
    pub scope: Scope,
    pub doc_type: String,
    pub schema_version: u32,
    #[serde(default)]
    pub source: Option<Source>,
    #[serde(default)]
    pub owner: Option<Uuid>,
    #[serde(default)]
    pub permissions: PermissionSet,
    #[serde(default)]
    pub embedded: BTreeMap<String, Vec<Document>>,
    pub system: serde_json::Value,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A world row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct World {
    pub id: Uuid,
    pub name: String,
    pub seq: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_doc() -> Document {
        Document {
            id: Uuid::from_u128(1),
            scope: Scope::World { world_id: Uuid::from_u128(9) },
            doc_type: "actor".to_string(),
            schema_version: 1,
            source: Some(Source { id: Uuid::from_u128(2), pack: Some("dnd5e".into()), version: 3 }),
            owner: Some(Uuid::from_u128(5)),
            permissions: PermissionSet::default(),
            embedded: BTreeMap::new(),
            system: serde_json::json!({ "hp": 10 }),
            created_at: 100,
            updated_at: 100,
        }
    }

    #[test]
    fn document_round_trips_through_json() {
        let doc = sample_doc();
        let s = serde_json::to_string(&doc).unwrap();
        let back: Document = serde_json::from_str(&s).unwrap();
        assert_eq!(doc, back);
    }

    #[test]
    fn unknown_envelope_field_is_rejected() {
        let mut value = serde_json::to_value(sample_doc()).unwrap();
        value.as_object_mut().unwrap().insert("bogus".into(), serde_json::json!(1));
        let err = serde_json::from_value::<Document>(value);
        assert!(err.is_err(), "deny_unknown_fields should reject the bogus key");
    }

    #[test]
    fn permissionset_default_role_is_none() {
        assert_eq!(PermissionSet::default().default, DocRole::None);
    }
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p shadowcat data::document`
Expected: PASS (3 tests). If the `DocRoleDefault`/`Default` wiring fails to compile, the implementation above is complete — confirm green.

- [ ] **Step 6: Format and commit**

```bash
cargo fmt --all
git add src/server/Cargo.toml Cargo.lock src/server/src/lib.rs src/server/src/data/mod.rs src/server/src/data/document.rs
git commit -m "feat(m2): document envelope, scope, source, permission, world types"
```

---

## Task 2: Command log, inversion, field-path application

**Files:**
- Create: `src/server/src/data/command.rs`
- Modify: `src/server/src/data/mod.rs` (add `pub mod command;`)

**Interfaces:**
- Consumes: `Document` (Task 1), `DataError` (Task 1).
- Produces: `data::command::{Command, UnsequencedCommand, Operation, FieldChange, set_pointer}`; `UnsequencedCommand::invert`, `apply_op` semantics used by Task 6.

- [ ] **Step 1: Add the module to `src/server/src/data/mod.rs`**

```rust
pub mod command;
pub mod document;
```

- [ ] **Step 2: Write the failing tests in `src/server/src/data/command.rs`**

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::data::document::Document;
use crate::data::DataError;

/// One field-level change with its pre-image, so it is self-inverting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldChange {
    pub path: String, // JSON pointer, e.g. "/system/hp"
    pub old: Value,
    pub new: Value,
}

/// A single operation within a command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    Create { doc: Document },
    Delete { doc: Document },
    Update { doc_id: Uuid, changes: Vec<FieldChange> },
}

/// A command awaiting a sequence number (constructed by callers).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnsequencedCommand {
    pub world_id: Uuid,
    pub author: Uuid,
    pub ts: i64,
    pub ops: Vec<Operation>,
}

/// A command that has been assigned a per-world sequence number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Command {
    pub seq: i64,
    pub world_id: Uuid,
    pub author: Uuid,
    pub ts: i64,
    pub ops: Vec<Operation>,
}

impl Operation {
    /// The inverse operation: Create<->Delete; Update swaps old/new per change, reversed.
    pub fn invert(&self) -> Operation {
        match self {
            Operation::Create { doc } => Operation::Delete { doc: doc.clone() },
            Operation::Delete { doc } => Operation::Create { doc: doc.clone() },
            Operation::Update { doc_id, changes } => Operation::Update {
                doc_id: *doc_id,
                changes: changes
                    .iter()
                    .rev()
                    .map(|c| FieldChange { path: c.path.clone(), old: c.new.clone(), new: c.old.clone() })
                    .collect(),
            },
        }
    }
}

impl UnsequencedCommand {
    /// The inverse command: every op inverted, op order reversed.
    pub fn invert(&self) -> UnsequencedCommand {
        UnsequencedCommand {
            world_id: self.world_id,
            author: self.author,
            ts: self.ts,
            ops: self.ops.iter().rev().map(Operation::invert).collect(),
        }
    }
}

impl Command {
    /// Inverse as an unsequenced command (re-applied gets a fresh seq).
    pub fn invert(&self) -> UnsequencedCommand {
        UnsequencedCommand {
            world_id: self.world_id,
            author: self.author,
            ts: self.ts,
            ops: self.ops.iter().rev().map(Operation::invert).collect(),
        }
    }
}

/// Set `new` at JSON-pointer `pointer` in `root`, creating intermediate
/// objects as needed. Existing array indices may be replaced; array growth
/// and `-` append are out of scope (handled by the deferred merge engine).
pub fn set_pointer(root: &mut Value, pointer: &str, new: Value) -> Result<(), DataError> {
    if pointer.is_empty() {
        *root = new;
        return Ok(());
    }
    let tokens: Vec<String> = pointer
        .split('/')
        .skip(1)
        .map(|t| t.replace("~1", "/").replace("~0", "~"))
        .collect();
    let mut cur = root;
    for (i, tok) in tokens.iter().enumerate() {
        let last = i == tokens.len() - 1;
        if last {
            match cur {
                Value::Object(m) => {
                    m.insert(tok.clone(), new);
                    return Ok(());
                }
                Value::Array(a) => {
                    let idx: usize = tok.parse().map_err(|_| DataError::BadPath(pointer.to_string()))?;
                    if idx < a.len() {
                        a[idx] = new;
                        return Ok(());
                    }
                    return Err(DataError::BadPath(pointer.to_string()));
                }
                _ => return Err(DataError::BadPath(pointer.to_string())),
            }
        }
        cur = match cur {
            Value::Object(m) => m.entry(tok.clone()).or_insert_with(|| Value::Object(Default::default())),
            Value::Array(a) => {
                let idx: usize = tok.parse().map_err(|_| DataError::BadPath(pointer.to_string()))?;
                a.get_mut(idx).ok_or_else(|| DataError::BadPath(pointer.to_string()))?
            }
            _ => return Err(DataError::BadPath(pointer.to_string())),
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(id: u128) -> Document {
        Document {
            id: Uuid::from_u128(id),
            scope: crate::data::document::Scope::World { world_id: Uuid::from_u128(9) },
            doc_type: "item".into(),
            schema_version: 1,
            source: None,
            owner: None,
            permissions: Default::default(),
            embedded: Default::default(),
            system: serde_json::json!({}),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn create_inverts_to_delete_and_back() {
        let op = Operation::Create { doc: doc(1) };
        assert_eq!(op.invert(), Operation::Delete { doc: doc(1) });
        assert_eq!(op.invert().invert(), op);
    }

    #[test]
    fn update_invert_swaps_old_and_new_in_reverse() {
        let op = Operation::Update {
            doc_id: Uuid::from_u128(1),
            changes: vec![
                FieldChange { path: "/system/a".into(), old: serde_json::json!(1), new: serde_json::json!(2) },
                FieldChange { path: "/system/b".into(), old: serde_json::json!(3), new: serde_json::json!(4) },
            ],
        };
        let inv = op.invert();
        assert_eq!(
            inv,
            Operation::Update {
                doc_id: Uuid::from_u128(1),
                changes: vec![
                    FieldChange { path: "/system/b".into(), old: serde_json::json!(4), new: serde_json::json!(3) },
                    FieldChange { path: "/system/a".into(), old: serde_json::json!(2), new: serde_json::json!(1) },
                ],
            }
        );
        assert_eq!(op.invert().invert(), op);
    }

    #[test]
    fn unsequenced_command_invert_is_round_trip() {
        let cmd = UnsequencedCommand {
            world_id: Uuid::from_u128(9),
            author: Uuid::from_u128(5),
            ts: 1,
            ops: vec![Operation::Create { doc: doc(1) }, Operation::Update {
                doc_id: Uuid::from_u128(1),
                changes: vec![FieldChange { path: "/system/x".into(), old: serde_json::json!(null), new: serde_json::json!(7) }],
            }],
        };
        assert_eq!(cmd.invert().invert(), cmd);
    }

    #[test]
    fn set_pointer_sets_existing_and_creates_intermediate() {
        let mut v = serde_json::json!({ "system": { "hp": 10 } });
        set_pointer(&mut v, "/system/hp", serde_json::json!(5)).unwrap();
        assert_eq!(v["system"]["hp"], serde_json::json!(5));

        set_pointer(&mut v, "/system/attributes/str", serde_json::json!(14)).unwrap();
        assert_eq!(v["system"]["attributes"]["str"], serde_json::json!(14));
    }

    #[test]
    fn set_pointer_rejects_descend_into_scalar() {
        let mut v = serde_json::json!({ "hp": 10 });
        let err = set_pointer(&mut v, "/hp/value", serde_json::json!(1));
        assert!(matches!(err, Err(DataError::BadPath(_))));
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p shadowcat data::command`
Expected: PASS (5 tests).

- [ ] **Step 4: Format and commit**

```bash
cargo fmt --all
git add src/server/src/data/mod.rs src/server/src/data/command.rs
git commit -m "feat(m2): command log types, invert(), and JSON-pointer apply"
```

---

## Task 3: Validation & migrate seam

**Files:**
- Create: `src/server/src/data/validation.rs`, `src/server/src/data/migrate.rs`
- Modify: `src/server/src/data/mod.rs`

**Interfaces:**
- Consumes: `Document` (Task 1), `DataError` (Task 1).
- Produces: `validation::{MAX_SYSTEM_BYTES, validate_system_size, validate_field_path}`; `migrate::{CURRENT_SCHEMA_VERSION, migrate}`.

- [ ] **Step 1: Add modules to `src/server/src/data/mod.rs`**

```rust
pub mod command;
pub mod document;
pub mod migrate;
pub mod validation;
```

- [ ] **Step 2: Write `src/server/src/data/validation.rs` with tests**

```rust
use crate::data::document::Document;
use crate::data::DataError;

/// Maximum serialized size of a document's opaque `system` body.
pub const MAX_SYSTEM_BYTES: usize = 256 * 1024;

/// Reject documents whose opaque body exceeds the size cap.
pub fn validate_system_size(doc: &Document) -> Result<(), DataError> {
    let bytes = serde_json::to_vec(&doc.system)?.len();
    if bytes > MAX_SYSTEM_BYTES {
        return Err(DataError::TooLarge(bytes));
    }
    Ok(())
}

/// A valid JSON pointer is empty or a sequence of "/"-prefixed tokens.
pub fn validate_field_path(path: &str) -> Result<(), DataError> {
    if path.is_empty() {
        return Ok(());
    }
    if !path.starts_with('/') {
        return Err(DataError::BadPath(path.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn doc_with_system(system: serde_json::Value) -> Document {
        Document {
            id: Uuid::from_u128(1),
            scope: crate::data::document::Scope::World { world_id: Uuid::from_u128(9) },
            doc_type: "actor".into(),
            schema_version: 1,
            source: None,
            owner: None,
            permissions: Default::default(),
            embedded: Default::default(),
            system,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn small_system_passes() {
        assert!(validate_system_size(&doc_with_system(serde_json::json!({ "hp": 1 }))).is_ok());
    }

    #[test]
    fn oversized_system_is_rejected() {
        let big = "x".repeat(MAX_SYSTEM_BYTES + 1);
        let err = validate_system_size(&doc_with_system(serde_json::json!({ "blob": big })));
        assert!(matches!(err, Err(DataError::TooLarge(_))));
    }

    #[test]
    fn field_paths_validate() {
        assert!(validate_field_path("").is_ok());
        assert!(validate_field_path("/system/hp").is_ok());
        assert!(matches!(validate_field_path("system/hp"), Err(DataError::BadPath(_))));
    }
}
```

- [ ] **Step 3: Write `src/server/src/data/migrate.rs` with a test**

```rust
use crate::data::document::Document;

/// The schema version current builds emit. No migrations exist pre-ship;
/// `migrate` is the machinery only and is a no-op at this version.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Coerce a document to the current schema version. At v1 this returns the
/// document unchanged; the registry of version steps is added when the first
/// real migration exists (post-ship).
pub fn migrate(doc: Document) -> Document {
    if doc.schema_version >= CURRENT_SCHEMA_VERSION {
        return doc;
    }
    // No registered steps yet.
    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn current_version_document_is_unchanged() {
        let doc = Document {
            id: Uuid::from_u128(1),
            scope: crate::data::document::Scope::World { world_id: Uuid::from_u128(9) },
            doc_type: "actor".into(),
            schema_version: CURRENT_SCHEMA_VERSION,
            source: None,
            owner: None,
            permissions: Default::default(),
            embedded: Default::default(),
            system: serde_json::json!({}),
            created_at: 0,
            updated_at: 0,
        };
        assert_eq!(migrate(doc.clone()), doc);
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p shadowcat data::validation data::migrate`
Expected: PASS (4 tests).

- [ ] **Step 5: Format and commit**

```bash
cargo fmt --all
git add src/server/src/data/mod.rs src/server/src/data/validation.rs src/server/src/data/migrate.rs
git commit -m "feat(m2): structural validation and no-op migrate seam"
```

---

## Task 4: Permission resolution helpers

**Files:**
- Create: `src/server/src/data/permission.rs`
- Modify: `src/server/src/data/mod.rs`

**Interfaces:**
- Consumes: `Document`, `DocRole`, `Visibility`, `WorldRole` (Task 1).
- Produces: `permission::{Access, resolve_access, filter_properties}`.

- [ ] **Step 1: Add the module to `src/server/src/data/mod.rs`**

```rust
pub mod command;
pub mod document;
pub mod migrate;
pub mod permission;
pub mod validation;
```

- [ ] **Step 2: Write `src/server/src/data/permission.rs` with tests**

```rust
use uuid::Uuid;

use crate::data::document::{DocRole, Document, Visibility, WorldRole};

/// Effective access for a (user, document) pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    pub can_read: bool,
    pub can_write: bool,
    pub see_gm_only: bool,
}

/// Resolve a user's effective access to a document. A world GM has full
/// access including GM-only properties; otherwise the document's per-user
/// role (falling back to its default role) decides.
pub fn resolve_access(user: Uuid, world_role: WorldRole, doc: &Document) -> Access {
    if world_role == WorldRole::Gm {
        return Access { can_read: true, can_write: true, see_gm_only: true };
    }
    let role = doc.permissions.users.get(&user).copied().unwrap_or(doc.permissions.default);
    match role {
        DocRole::Owner => Access { can_read: true, can_write: true, see_gm_only: false },
        DocRole::Observer => Access { can_read: true, can_write: false, see_gm_only: false },
        DocRole::None => Access { can_read: false, can_write: false, see_gm_only: false },
    }
}

/// Produce the recipient's view of a document: when `access.see_gm_only` is
/// false, strip every property whose override is `GmOnly`.
pub fn filter_properties(doc: &Document, access: Access) -> Document {
    let mut out = doc.clone();
    if access.see_gm_only {
        return out;
    }
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
    out = serde_json::from_value(whole).expect("filtered document deserializes");
    out
}

/// Remove the value at a JSON pointer, if present.
fn strip_pointer(root: &mut serde_json::Value, pointer: &str) {
    let tokens: Vec<String> = pointer
        .split('/')
        .skip(1)
        .map(|t| t.replace("~1", "/").replace("~0", "~"))
        .collect();
    if tokens.is_empty() {
        return;
    }
    let mut cur = root;
    for tok in &tokens[..tokens.len() - 1] {
        match cur.get_mut(tok) {
            Some(next) => cur = next,
            None => return,
        }
    }
    if let serde_json::Value::Object(m) = cur {
        m.remove(&tokens[tokens.len() - 1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::document::{PermissionSet, Scope};

    fn doc(perms: PermissionSet, system: serde_json::Value) -> Document {
        Document {
            id: Uuid::from_u128(1),
            scope: Scope::World { world_id: Uuid::from_u128(9) },
            doc_type: "actor".into(),
            schema_version: 1,
            source: None,
            owner: None,
            permissions: perms,
            embedded: Default::default(),
            system,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn gm_sees_everything() {
        let a = resolve_access(Uuid::from_u128(5), WorldRole::Gm, &doc(Default::default(), serde_json::json!({})));
        assert_eq!(a, Access { can_read: true, can_write: true, see_gm_only: true });
    }

    #[test]
    fn owner_observer_none_resolve_correctly() {
        let mut perms = PermissionSet::default();
        perms.users.insert(Uuid::from_u128(1), DocRole::Owner);
        perms.users.insert(Uuid::from_u128(2), DocRole::Observer);
        let d = doc(perms, serde_json::json!({}));
        assert!(resolve_access(Uuid::from_u128(1), WorldRole::Player, &d).can_write);
        let obs = resolve_access(Uuid::from_u128(2), WorldRole::Player, &d);
        assert!(obs.can_read && !obs.can_write);
        let other = resolve_access(Uuid::from_u128(3), WorldRole::Player, &d);
        assert!(!other.can_read);
    }

    #[test]
    fn gm_only_property_is_stripped_for_non_gm() {
        let mut perms = PermissionSet::default();
        perms.default = DocRole::Observer;
        perms.property_overrides.insert("/system/secret".into(), Visibility::GmOnly);
        let d = doc(perms, serde_json::json!({ "secret": 42, "public": 1 }));

        let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &d);
        let view = filter_properties(&d, player);
        assert_eq!(view.system.get("secret"), None);
        assert_eq!(view.system["public"], serde_json::json!(1));

        let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d);
        assert_eq!(filter_properties(&d, gm).system["secret"], serde_json::json!(42));
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p shadowcat data::permission`
Expected: PASS (3 tests).

- [ ] **Step 4: Format and commit**

```bash
cargo fmt --all
git add src/server/src/data/mod.rs src/server/src/data/permission.rs
git commit -m "feat(m2): permission resolution and property filtering"
```

---

## Task 5: SQLite schema, migrations, and world/user/member storage

**Files:**
- Create: `src/server/migrations/0001_init.sql`, `src/server/src/data/sqlite.rs`
- Modify: `src/server/src/data/mod.rs`

**Interfaces:**
- Consumes: `World` (Task 1), `DataError` (Task 1).
- Produces: `data::sqlite::SqliteRepository` with `connect`, `create_world`, `get_world`, `create_user`, `add_member`, `member_role`.

- [ ] **Step 1: Create `src/server/migrations/0001_init.sql`**

```sql
CREATE TABLE worlds (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  seq INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE users (
  id TEXT PRIMARY KEY,
  username TEXT NOT NULL UNIQUE,
  server_role TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE TABLE world_members (
  world_id TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
  user_id  TEXT NOT NULL REFERENCES users(id)  ON DELETE CASCADE,
  role     TEXT NOT NULL,
  PRIMARY KEY (world_id, user_id)
);

CREATE TABLE documents (
  id TEXT PRIMARY KEY,
  scope_kind TEXT NOT NULL,
  world_id TEXT REFERENCES worlds(id) ON DELETE CASCADE,
  pack TEXT,
  doc_type TEXT NOT NULL,
  schema_version INTEGER NOT NULL,
  source_id TEXT,
  source_pack TEXT,
  source_version INTEGER,
  owner_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  seq INTEGER NOT NULL DEFAULT 0,
  json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX idx_documents_world_type ON documents(world_id, doc_type);
CREATE INDEX idx_documents_source     ON documents(source_pack, source_id);
CREATE INDEX idx_documents_scope      ON documents(scope_kind, pack);

CREATE TABLE world_events (
  world_id TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  author_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  ts INTEGER NOT NULL,
  command_json TEXT NOT NULL,
  PRIMARY KEY (world_id, seq)
);
```

- [ ] **Step 2: Add the module to `src/server/src/data/mod.rs`**

```rust
pub mod command;
pub mod document;
pub mod migrate;
pub mod permission;
pub mod repository;
pub mod sqlite;
pub mod validation;
```

(Create `repository.rs` in Task 6; add the line now only if the crate still compiles — otherwise add `pub mod repository;` in Task 6. To keep this task compiling, omit `pub mod repository;` here and add it in Task 6.)

Use this version for Task 5:

```rust
pub mod command;
pub mod document;
pub mod migrate;
pub mod permission;
pub mod sqlite;
pub mod validation;
```

- [ ] **Step 3: Write `src/server/src/data/sqlite.rs` with tests**

```rust
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::data::document::{World, WorldRole};
use crate::data::DataError;

/// SQLite-backed storage. Holds a connection pool; migrations are embedded
/// from `migrations/` and run at connect time.
pub struct SqliteRepository {
    pool: SqlitePool,
}

impl SqliteRepository {
    /// Connect to `url` (e.g. "sqlite::memory:" or "sqlite:///path/to.db")
    /// and run migrations. Foreign keys are enabled per connection.
    pub async fn connect(url: &str) -> Result<Self, DataError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    sqlx::query("PRAGMA foreign_keys = ON;").execute(conn).await?;
                    Ok(())
                })
            })
            .connect(url)
            .await?;
        sqlx::migrate!("migrations").run(&pool).await.map_err(sqlx::Error::from)?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn create_world(&self, name: &str, now: i64) -> Result<World, DataError> {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO worlds (id, name, seq, created_at, updated_at) VALUES (?, ?, 0, ?, ?)")
            .bind(id.to_string())
            .bind(name)
            .bind(now)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(World { id, name: name.to_string(), seq: 0, created_at: now, updated_at: now })
    }

    pub async fn get_world(&self, id: Uuid) -> Result<Option<World>, DataError> {
        let row = sqlx::query("SELECT id, name, seq, created_at, updated_at FROM worlds WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| World {
            id: Uuid::parse_str(r.get::<String, _>("id").as_str()).unwrap(),
            name: r.get("name"),
            seq: r.get("seq"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        }))
    }

    pub async fn create_user(&self, username: &str, server_role: &str, now: i64) -> Result<Uuid, DataError> {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, username, server_role, created_at) VALUES (?, ?, ?, ?)")
            .bind(id.to_string())
            .bind(username)
            .bind(server_role)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }

    pub async fn add_member(&self, world_id: Uuid, user_id: Uuid, role: WorldRole) -> Result<(), DataError> {
        sqlx::query("INSERT INTO world_members (world_id, user_id, role) VALUES (?, ?, ?)")
            .bind(world_id.to_string())
            .bind(user_id.to_string())
            .bind(serde_json::to_value(role)?.as_str().unwrap().to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn member_role(&self, world_id: Uuid, user_id: Uuid) -> Result<Option<WorldRole>, DataError> {
        let row = sqlx::query("SELECT role FROM world_members WHERE world_id = ? AND user_id = ?")
            .bind(world_id.to_string())
            .bind(user_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => {
                let role: String = r.get("role");
                Ok(Some(serde_json::from_value(serde_json::Value::String(role))?))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn repo() -> SqliteRepository {
        SqliteRepository::connect("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn create_then_get_world() {
        let r = repo().await;
        let w = r.create_world("Test", 100).await.unwrap();
        let got = r.get_world(w.id).await.unwrap().unwrap();
        assert_eq!(got, w);
        assert_eq!(got.seq, 0);
    }

    #[tokio::test]
    async fn members_carry_world_role() {
        let r = repo().await;
        let w = r.create_world("Test", 100).await.unwrap();
        let u = r.create_user("gm", "admin", 100).await.unwrap();
        r.add_member(w.id, u, WorldRole::Gm).await.unwrap();
        assert_eq!(r.member_role(w.id, u).await.unwrap(), Some(WorldRole::Gm));
        assert_eq!(r.member_role(w.id, Uuid::from_u128(123)).await.unwrap(), None);
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p shadowcat data::sqlite`
Expected: PASS (2 tests). If `sqlx::migrate!("migrations")` cannot find the directory, confirm the path is `src/server/migrations/` (relative to the crate's `Cargo.toml`).

- [ ] **Step 5: Format and commit**

```bash
cargo fmt --all
git add src/server/migrations/0001_init.sql src/server/src/data/mod.rs src/server/src/data/sqlite.rs Cargo.lock
git commit -m "feat(m2): SQLite schema, migrations, world/user/member storage"
```

---

## Task 6: Repository trait, apply_command, get_document

**Files:**
- Create: `src/server/src/data/repository.rs`
- Modify: `src/server/src/data/mod.rs`, `src/server/src/data/sqlite.rs`

**Interfaces:**
- Consumes: `Document`, `Command`, `UnsequencedCommand`, `Operation`, `set_pointer`, `World`, `DataError`.
- Produces: `data::repository::Repository` (trait) with `apply_command`, `get_document`; `SqliteRepository` implements it.

- [ ] **Step 1: Add `pub mod repository;` to `src/server/src/data/mod.rs`**

```rust
pub mod command;
pub mod document;
pub mod migrate;
pub mod permission;
pub mod repository;
pub mod sqlite;
pub mod validation;
```

- [ ] **Step 2: Write the trait in `src/server/src/data/repository.rs`**

```rust
use async_trait::async_trait;
use uuid::Uuid;

use crate::data::command::{Command, UnsequencedCommand};
use crate::data::document::Document;
use crate::data::DataError;

/// Storage contract. The only implementation in M2 is `SqliteRepository`;
/// the trait exists so Postgres can be added later behind the same surface.
#[async_trait]
pub trait Repository: Send + Sync {
    /// Allocate the next per-world seq, append the command to the log, and
    /// apply every operation to the document store — all in one transaction.
    async fn apply_command(&self, cmd: UnsequencedCommand) -> Result<Command, DataError>;

    async fn get_document(&self, id: Uuid) -> Result<Option<Document>, DataError>;

    async fn query_documents(&self, world_id: Uuid, doc_type: &str) -> Result<Vec<Document>, DataError>;

    async fn documents_by_source(&self, pack: Option<&str>, source_id: Uuid) -> Result<Vec<Document>, DataError>;

    async fn events_since(&self, world_id: Uuid, seq: i64) -> Result<Vec<Command>, DataError>;
}
```

- [ ] **Step 3: Implement `apply_command` and `get_document` on `SqliteRepository`**

Add to `src/server/src/data/sqlite.rs` (new imports at top, then the `impl Repository`):

```rust
use async_trait::async_trait;

use crate::data::command::{Command, Operation, UnsequencedCommand, set_pointer};
use crate::data::document::{Document, Scope};
use crate::data::repository::Repository;

impl SqliteRepository {
    /// Upsert a document row from its envelope, stamping `seq`.
    async fn upsert_document<'e, E>(executor: E, doc: &Document, seq: i64) -> Result<(), DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let (scope_kind, world_id, pack) = match &doc.scope {
            Scope::Compendium { pack } => ("compendium", None, Some(pack.clone())),
            Scope::World { world_id } => ("world", Some(world_id.to_string()), None),
        };
        let (source_id, source_pack, source_version) = match &doc.source {
            Some(s) => (Some(s.id.to_string()), s.pack.clone(), Some(s.version as i64)),
            None => (None, None, None),
        };
        let json = serde_json::to_string(doc)?;
        sqlx::query(
            "INSERT INTO documents (id, scope_kind, world_id, pack, doc_type, schema_version, \
             source_id, source_pack, source_version, owner_id, seq, json, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET scope_kind=excluded.scope_kind, world_id=excluded.world_id, \
             pack=excluded.pack, doc_type=excluded.doc_type, schema_version=excluded.schema_version, \
             source_id=excluded.source_id, source_pack=excluded.source_pack, \
             source_version=excluded.source_version, owner_id=excluded.owner_id, seq=excluded.seq, \
             json=excluded.json, updated_at=excluded.updated_at",
        )
        .bind(doc.id.to_string())
        .bind(scope_kind)
        .bind(world_id)
        .bind(pack)
        .bind(&doc.doc_type)
        .bind(doc.schema_version as i64)
        .bind(source_id)
        .bind(source_pack)
        .bind(source_version)
        .bind(doc.owner.map(|o| o.to_string()))
        .bind(seq)
        .bind(json)
        .bind(doc.created_at)
        .bind(doc.updated_at)
        .execute(executor)
        .await?;
        Ok(())
    }
}

#[async_trait]
impl Repository for SqliteRepository {
    async fn apply_command(&self, cmd: UnsequencedCommand) -> Result<Command, DataError> {
        let mut tx = self.pool.begin().await?;

        // Allocate the next per-world seq from the single durable source.
        let seq: i64 = sqlx::query("UPDATE worlds SET seq = seq + 1 WHERE id = ? RETURNING seq")
            .bind(cmd.world_id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(DataError::NotFound)?
            .get("seq");

        let sequenced = Command {
            seq,
            world_id: cmd.world_id,
            author: cmd.author,
            ts: cmd.ts,
            ops: cmd.ops,
        };

        // Apply each operation.
        for op in &sequenced.ops {
            match op {
                Operation::Create { doc } => {
                    Self::upsert_document(&mut *tx, doc, seq).await?;
                }
                Operation::Delete { doc } => {
                    sqlx::query("DELETE FROM documents WHERE id = ?")
                        .bind(doc.id.to_string())
                        .execute(&mut *tx)
                        .await?;
                }
                Operation::Update { doc_id, changes } => {
                    let row = sqlx::query("SELECT json FROM documents WHERE id = ?")
                        .bind(doc_id.to_string())
                        .fetch_optional(&mut *tx)
                        .await?
                        .ok_or(DataError::NotFound)?;
                    let mut value: serde_json::Value = serde_json::from_str(row.get::<String, _>("json").as_str())?;
                    for ch in changes {
                        set_pointer(&mut value, &ch.path, ch.new.clone())?;
                    }
                    let doc: Document = serde_json::from_value(value)?;
                    Self::upsert_document(&mut *tx, &doc, seq).await?;
                }
            }
        }

        // Append to the log.
        sqlx::query("INSERT INTO world_events (world_id, seq, author_id, ts, command_json) VALUES (?, ?, ?, ?, ?)")
            .bind(sequenced.world_id.to_string())
            .bind(seq)
            .bind(sequenced.author.to_string())
            .bind(sequenced.ts)
            .bind(serde_json::to_string(&sequenced)?)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(sequenced)
    }

    async fn get_document(&self, id: Uuid) -> Result<Option<Document>, DataError> {
        let row = sqlx::query("SELECT json FROM documents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => Ok(Some(serde_json::from_str(r.get::<String, _>("json").as_str())?)),
            None => Ok(None),
        }
    }

    async fn query_documents(&self, _world_id: Uuid, _doc_type: &str) -> Result<Vec<Document>, DataError> {
        // Implemented in Task 7.
        unimplemented!("query_documents lands in Task 7")
    }

    async fn documents_by_source(&self, _pack: Option<&str>, _source_id: Uuid) -> Result<Vec<Document>, DataError> {
        unimplemented!("documents_by_source lands in Task 7")
    }

    async fn events_since(&self, _world_id: Uuid, _seq: i64) -> Result<Vec<Command>, DataError> {
        unimplemented!("events_since lands in Task 7")
    }
}
```

- [ ] **Step 4: Write tests at the bottom of `src/server/src/data/sqlite.rs` (add to the existing `mod tests`)**

```rust
    use crate::data::command::{FieldChange, Operation, UnsequencedCommand};
    use crate::data::document::{Document, Scope};
    use crate::data::repository::Repository;

    fn world_doc(id: u128, world: Uuid, system: serde_json::Value) -> Document {
        Document {
            id: Uuid::from_u128(id),
            scope: Scope::World { world_id: world },
            doc_type: "actor".into(),
            schema_version: 1,
            source: None,
            owner: None,
            permissions: Default::default(),
            embedded: Default::default(),
            system,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[tokio::test]
    async fn create_update_delete_round_trip_via_invert() {
        let r = repo().await;
        let w = r.create_world("W", 0).await.unwrap();
        let author = Uuid::from_u128(5);

        // Create
        let create = UnsequencedCommand {
            world_id: w.id, author, ts: 1,
            ops: vec![Operation::Create { doc: world_doc(1, w.id, serde_json::json!({ "hp": 10 })) }],
        };
        let c1 = r.apply_command(create.clone()).await.unwrap();
        assert_eq!(c1.seq, 1);
        assert!(r.get_document(Uuid::from_u128(1)).await.unwrap().is_some());

        // Update
        let update = UnsequencedCommand {
            world_id: w.id, author, ts: 2,
            ops: vec![Operation::Update {
                doc_id: Uuid::from_u128(1),
                changes: vec![FieldChange { path: "/system/hp".into(), old: serde_json::json!(10), new: serde_json::json!(3) }],
            }],
        };
        let c2 = r.apply_command(update.clone()).await.unwrap();
        assert_eq!(c2.seq, 2);
        assert_eq!(r.get_document(Uuid::from_u128(1)).await.unwrap().unwrap().system["hp"], serde_json::json!(3));

        // Invert the update — hp returns to 10
        r.apply_command(c2.invert()).await.unwrap();
        assert_eq!(r.get_document(Uuid::from_u128(1)).await.unwrap().unwrap().system["hp"], serde_json::json!(10));

        // Invert the create — document gone
        r.apply_command(c1.invert()).await.unwrap();
        assert!(r.get_document(Uuid::from_u128(1)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn apply_command_on_unknown_world_fails_and_writes_nothing() {
        let r = repo().await;
        let cmd = UnsequencedCommand {
            world_id: Uuid::from_u128(999), author: Uuid::from_u128(5), ts: 1,
            ops: vec![Operation::Create { doc: world_doc(1, Uuid::from_u128(999), serde_json::json!({})) }],
        };
        assert!(r.apply_command(cmd).await.is_err());
        assert!(r.get_document(Uuid::from_u128(1)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn seq_is_durable_across_reconnect() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m2.db");
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());

        let world_id;
        {
            let r = SqliteRepository::connect(&url).await.unwrap();
            let w = r.create_world("W", 0).await.unwrap();
            world_id = w.id;
            r.apply_command(UnsequencedCommand {
                world_id, author: Uuid::from_u128(5), ts: 1,
                ops: vec![Operation::Create { doc: world_doc(1, world_id, serde_json::json!({})) }],
            }).await.unwrap();
        }
        // Reconnect: seq must continue from 2, not restart at 1.
        let r = SqliteRepository::connect(&url).await.unwrap();
        let c = r.apply_command(UnsequencedCommand {
            world_id, author: Uuid::from_u128(5), ts: 2,
            ops: vec![Operation::Create { doc: world_doc(2, world_id, serde_json::json!({})) }],
        }).await.unwrap();
        assert_eq!(c.seq, 2);
    }
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p shadowcat data::sqlite`
Expected: PASS (5 tests total in this module). The `unimplemented!` query methods are not exercised yet.

- [ ] **Step 6: Format and commit**

```bash
cargo fmt --all
git add src/server/src/data/mod.rs src/server/src/data/repository.rs src/server/src/data/sqlite.rs
git commit -m "feat(m2): Repository trait, transactional apply_command, get_document"
```

---

## Task 7: Queries, source lookup, and event replay

**Files:**
- Modify: `src/server/src/data/sqlite.rs`

**Interfaces:**
- Consumes: everything from Task 6.
- Produces: working `query_documents`, `documents_by_source`, `events_since` (replacing the `unimplemented!` stubs).

- [ ] **Step 1: Replace the three stub methods in the `impl Repository for SqliteRepository` block**

```rust
    async fn query_documents(&self, world_id: Uuid, doc_type: &str) -> Result<Vec<Document>, DataError> {
        let rows = sqlx::query("SELECT json FROM documents WHERE world_id = ? AND doc_type = ? ORDER BY id")
            .bind(world_id.to_string())
            .bind(doc_type)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("json").as_str())?))
            .collect()
    }

    async fn documents_by_source(&self, pack: Option<&str>, source_id: Uuid) -> Result<Vec<Document>, DataError> {
        let rows = match pack {
            Some(p) => {
                sqlx::query("SELECT json FROM documents WHERE source_pack = ? AND source_id = ? ORDER BY id")
                    .bind(p)
                    .bind(source_id.to_string())
                    .fetch_all(&self.pool)
                    .await?
            }
            None => {
                sqlx::query("SELECT json FROM documents WHERE source_pack IS NULL AND source_id = ? ORDER BY id")
                    .bind(source_id.to_string())
                    .fetch_all(&self.pool)
                    .await?
            }
        };
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("json").as_str())?))
            .collect()
    }

    async fn events_since(&self, world_id: Uuid, seq: i64) -> Result<Vec<Command>, DataError> {
        let rows = sqlx::query("SELECT command_json FROM world_events WHERE world_id = ? AND seq > ? ORDER BY seq")
            .bind(world_id.to_string())
            .bind(seq)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("command_json").as_str())?))
            .collect()
    }
```

- [ ] **Step 2: Add tests to the `mod tests` block in `src/server/src/data/sqlite.rs`**

```rust
    use crate::data::document::Source;

    #[tokio::test]
    async fn query_documents_filters_by_world_and_type() {
        let r = repo().await;
        let w = r.create_world("W", 0).await.unwrap();
        let author = Uuid::from_u128(5);
        for id in [1u128, 2] {
            r.apply_command(UnsequencedCommand {
                world_id: w.id, author, ts: 1,
                ops: vec![Operation::Create { doc: world_doc(id, w.id, serde_json::json!({})) }],
            }).await.unwrap();
        }
        let actors = r.query_documents(w.id, "actor").await.unwrap();
        assert_eq!(actors.len(), 2);
        assert!(r.query_documents(w.id, "item").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn documents_by_source_finds_instances_for_push() {
        let r = repo().await;
        let w = r.create_world("W", 0).await.unwrap();
        let src = Uuid::from_u128(77);
        let mut doc = world_doc(1, w.id, serde_json::json!({}));
        doc.source = Some(Source { id: src, pack: Some("dnd5e".into()), version: 1 });
        r.apply_command(UnsequencedCommand {
            world_id: w.id, author: Uuid::from_u128(5), ts: 1,
            ops: vec![Operation::Create { doc }],
        }).await.unwrap();

        let found = r.documents_by_source(Some("dnd5e"), src).await.unwrap();
        assert_eq!(found.len(), 1);
        assert!(r.documents_by_source(Some("dnd5e"), Uuid::from_u128(0)).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn events_since_returns_the_suffix() {
        let r = repo().await;
        let w = r.create_world("W", 0).await.unwrap();
        let author = Uuid::from_u128(5);
        for id in [1u128, 2, 3] {
            r.apply_command(UnsequencedCommand {
                world_id: w.id, author, ts: 1,
                ops: vec![Operation::Create { doc: world_doc(id, w.id, serde_json::json!({})) }],
            }).await.unwrap();
        }
        let tail = r.events_since(w.id, 1).await.unwrap();
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].seq, 2);
        assert_eq!(tail[1].seq, 3);
    }
```

- [ ] **Step 3: Run the full data-layer test suite**

Run: `cargo test -p shadowcat data`
Expected: PASS (all data tests across the 7 tasks).

- [ ] **Step 4: Run the full local CI suite (must be green before commit)**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
git diff --exit-code src/types/generated
cargo build --release
pnpm -r typecheck && pnpm -r test && pnpm lint
```
Expected: every command exits 0. (No new ts-rs types were exported, so the sync check stays clean.)

- [ ] **Step 5: Format and commit**

```bash
cargo fmt --all
git add src/server/src/data/sqlite.rs
git commit -m "feat(m2): document queries, source lookup, and event replay"
```

---

## Self-Review

**1. Spec coverage** (against `docs/design/M2-data-foundation.md`):
- Document envelope + opaque body + structured embedded → Task 1. ✓
- `source` provenance, indexed both ways → Task 1 (type) + Task 5 schema indexes + Task 7 `documents_by_source`. ✓
- SQLite schema + `sqlx migrate` → Task 5. ✓
- Multi-doc transactional command log + `invert()` → Tasks 2, 6. ✓
- Per-world durable sequence (`worlds.seq`) → Task 6 (`apply_command`) + the reconnect test. ✓
- Permission types + pure helpers → Tasks 1, 4. ✓
- Structural validation → Task 3. ✓
- No-op migrate seam → Task 3. ✓
- `Repository` trait + `SqliteRepository` → Tasks 6, 7. ✓
- In-memory + temp-file SQLite tests → all tasks. ✓
- Deferred (merge engine, enforcement, HTTP, ts-rs exports) → not implemented, by design. ✓

**2. Placeholder scan:** The `unimplemented!` stubs in Task 6 are deliberately and explicitly replaced in Task 7 (a reviewer could approve Task 6 — apply/get — independently of the query methods), not open-ended placeholders. No "TBD"/"handle errors"/"similar to" anywhere; every code step is complete.

**3. Type consistency:** `Document`, `Scope`, `Source`, `PermissionSet`, `DocRole`/`Visibility`/`WorldRole`, `World` (Task 1) are used unchanged in Tasks 2–7. `Command`/`UnsequencedCommand`/`Operation`/`FieldChange`/`set_pointer` (Task 2) are consumed verbatim in Task 6. `seq` is `i64` consistently (SQLite integer). `Repository` method signatures in Task 6's trait match the Task 7 implementations exactly. `Access` (Task 4) is self-contained.

**Notes for the executor:**
- `cargo fmt` before each commit to keep CI's `fmt --check` green.
- If sqlx's `RETURNING seq` needs a feature on SQLite, confirm `libsqlite3` is ≥ 3.35 (sqlx 0.9 bundles a current SQLite, so `RETURNING` is available).
- Adding `pub mod repository;` only compiles once `repository.rs` exists — keep the `mod.rs` edits ordered as written (repository module added in Task 6).

## Buddy-check directives

- Plan buddy check: not run (the design was settled through brainstorming with the user).
- **Flagged tasks (high-risk): Task 2 (command/`invert()` — serialization save-format + algorithmic core) and Task 6 (`apply_command` — transactional integrity + durable sequence guarantee).** A buddy check is **offered** at the execution handoff for these two tasks; outcome recorded there.
- Unflagged tasks that turn risky during execution: ask.
