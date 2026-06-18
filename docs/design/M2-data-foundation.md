# M2 — Data Foundation (Design)

Design of record for the M2 milestone: the pure-Rust data layer. No HTTP, no networking, no game logic — M2 establishes the document model, SQLite schema, mutation log, sequence counter, permission schema, validation, and the `Repository` seam that M3+ build on. Paired with [`../PLAN.md`](../PLAN.md) M2 and the invariants in [`ARCHITECTURE.md`](ARCHITECTURE.md).

## 1. Scope

**In M2:**
- Typed document envelope + opaque `system` body; structured, addressable embedded documents.
- `source` provenance, indexed both directions, enabling the later pull/push merge.
- SQLite schema + `sqlx migrate` infrastructure.
- Multi-document **transactional** command log; `invert()` as the single substrate shared by optimistic rollback and undo.
- Per-world durable sequence counter.
- Permission types + pure resolution/filter helpers.
- Structural validation; no-op `migrateData` seam.
- `Repository` trait + `SqliteRepository`.
- Unit tests against in-memory SQLite.

**Deferred (seam noted, not built in M2):**
- The pull/push **3-way merge engine** — needs the system schema layer. M2 only stores and indexes `source`. The merge is always explicit (never automatic), bulk-capable (single doc up through actor / scene / folder / compendium / world), and bidirectional: **child-pull** (select instance → merge its source in) and **parent-push** (select source → merge into every instance referencing it). Default array strategy is wholesale replace; systems declare per-field strategy. Cross-world push and embedded-child reverse lookup are merge-engine concerns.
- Permission **enforcement** — `PermissionContext`, per-recipient broadcast filtering (M5).
- HTTP, WebSocket, the client (M3 / M4 / M6).
- Actual schema migrations — none exist pre-ship; the seam is a no-op.
- Compression, content hashing, full-text search.

## 2. Decisions & rationale

- **Embedded documents are nested in the parent (not separate rows), but kept as structured, addressable envelopes.** The engine cannot type system-defined embedded collections (they are opaque), copy-independence falls out for free, and "load a document with its embedded docs" is one row read. Each embedded doc carries its own `id` and `source`, so the command log and merge can address `embedded.<collection>.<id>`.
- **Provenance-based pull/push merge, not live inheritance.** Live resolution of a child diff against a parent would force the engine to deep-merge opaque bodies (array merge is ambiguous without system schemas), strain per-recipient permission stripping (the server would have to merge to filter), make migration harder (a default-adding migration silently shadows the parent), and break the per-world-event determinism model (a compendium edit would change instance state with no world event). Instead, documents are independent copies carrying `source`, and merge is an explicit, reviewable operation run where the system schema and a human exist. M2 builds the storage + index; the engine is deferred.
- **`source` lives in the typed envelope and is indexed both ways** (`source_id`, `source_pack`) — instance→source for pull, source→instances for push.
- **Commands are multi-document and transactional.** One logical action is one command with one sequence number, applied atomically even across several documents. This gives clean undo ("one action, one undo"), consistency, and is the substrate M4 broadcasts and M5 filters. Atomicity is hard to retrofit; it is designed in now.

## 3. Module layout (`src/server/src/data/`)

| File | Responsibility |
|---|---|
| `document.rs` | Envelope types (`Document`, `Scope`, `Source`, embedded), (de)serialization with `deny_unknown_fields`. |
| `command.rs` | `Command` / `Operation` / `FieldChange`, field-path application, `invert()`. |
| `permission.rs` | Role enums, `PermissionSet`, pure `resolve_access` / `filter_properties`. |
| `sequence.rs` | Per-world `AtomicU64` allocation, durable init from the event log. |
| `validation.rs` | Structural validation (size cap, JSON-pointer validity). |
| `migrate.rs` | `migrateData` seam — no-op at current version, registry stub. |
| `repository.rs` | The `Repository` trait. |
| `sqlite/mod.rs`, `sqlite/migrations/` | `SqliteRepository` impl + `sqlx migrate` SQL. |

`data/mod.rs` re-exports the public surface. No HTTP anywhere in M2.

## 4. Document envelope

`#[serde(deny_unknown_fields)]` on the envelope; `system` is an opaque `serde_json::Value` the engine never reads.

```rust
struct Document {
    id: Uuid,
    scope: Scope,                                   // Compendium { pack } | World { world_id }
    doc_type: String,                               // engine-agnostic discriminator: "actor","item","scene",...
    schema_version: u32,
    source: Option<Source>,                         // { id: Uuid, pack: Option<String>, version: u32 }
    owner: Option<UserId>,
    permissions: PermissionSet,
    embedded: BTreeMap<String, Vec<Document>>,      // collections of nested, addressable child envelopes
    system: serde_json::Value,                      // opaque body
    created_at: i64,
    updated_at: i64,
}
```

Embedded docs are full envelopes nested in the parent's row; each carries its own `source` for inheritance and is addressable for the command log.

## 5. SQLite schema (one migration in M2)

```sql
CREATE TABLE worlds (
  id TEXT PRIMARY KEY, name TEXT NOT NULL,
  seq INTEGER NOT NULL DEFAULT 0,            -- last allocated per-world sequence (durable)
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);

CREATE TABLE users (                          -- minimal in M2; auth fields land in M3
  id TEXT PRIMARY KEY, username TEXT NOT NULL UNIQUE,
  server_role TEXT NOT NULL,                  -- 'admin' | 'user'
  created_at INTEGER NOT NULL
);

CREATE TABLE world_members (
  world_id TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
  user_id  TEXT NOT NULL REFERENCES users(id)  ON DELETE CASCADE,
  role     TEXT NOT NULL,                     -- 'gm' | 'player' | 'spectator'
  PRIMARY KEY (world_id, user_id)
);

CREATE TABLE documents (                       -- one row per top-level document; embedded nested in json
  id TEXT PRIMARY KEY,
  scope_kind TEXT NOT NULL,                    -- 'compendium' | 'world'
  world_id TEXT REFERENCES worlds(id) ON DELETE CASCADE,   -- NULL for compendium scope
  pack TEXT,                                   -- NULL for world scope
  doc_type TEXT NOT NULL,
  schema_version INTEGER NOT NULL,
  source_id TEXT, source_pack TEXT, source_version INTEGER, -- provenance (indexed for push)
  owner_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  seq INTEGER NOT NULL DEFAULT 0,              -- seq of the command that last touched this doc
  json TEXT NOT NULL,                          -- full serialized envelope
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE INDEX idx_documents_world_type ON documents(world_id, doc_type);
CREATE INDEX idx_documents_source     ON documents(source_pack, source_id);   -- parent-push reverse lookup
CREATE INDEX idx_documents_scope      ON documents(scope_kind, pack);

CREATE TABLE world_events (                     -- append-only command log
  world_id TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  author_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  ts INTEGER NOT NULL,
  command_json TEXT NOT NULL,
  PRIMARY KEY (world_id, seq)
);
```

The full envelope is stored as `json`; indexed columns are denormalized projections for querying. `documents.seq` is for change detection.

## 6. Command log — the undoable substrate

```rust
struct Command { seq: u64, world_id: Uuid, author: UserId, ts: i64, ops: Vec<Operation> }

enum Operation {
    Create { doc: Document },
    Delete { doc: Document },                       // full doc retained for inversion
    Update { doc_id: Uuid, changes: Vec<FieldChange> },
}

struct FieldChange { path: String /* JSON pointer */, old: serde_json::Value, new: serde_json::Value }

impl Command {
    fn invert(&self) -> Command;   // Create<->Delete; Update swaps old/new; reverse op order
}
```

`invert()` is the whole point: **optimistic rollback** (apply a command optimistically; on rejection apply its inverse) and **undo** (apply the inverse of a committed command) are the same operation over the same representation. Document create and delete are ordinary operations, so the log captures full document lifecycle.

Callers construct an `UnsequencedCommand { world_id, author, ts, ops }` — the same shape without `seq`. `Repository::apply_command` assigns the seq and returns the full `Command`.

## 7. Sequence counter (invariant 2)

The per-world counter is the `worlds.seq` column. `apply_command` allocates the next value with `UPDATE worlds SET seq = seq + 1 RETURNING seq` **inside its own transaction** — atomic and durable, with a single source of truth and no in-memory counter to reconcile on restart. `documents.seq` records the command that last touched each document. (An in-memory cache can be added later if the per-command update ever shows up as hot; M2 keeps the durable single source.)

## 8. Permissions (schema + pure helpers; enforcement is M5)

```rust
enum DocRole { Owner, Observer, None }
enum Visibility { All, GmOnly }
enum WorldRole { Gm, Player, Spectator }

struct PermissionSet {
    default: DocRole,
    users: BTreeMap<UserId, DocRole>,
    property_overrides: BTreeMap<String /* JSON pointer */, Visibility>,
}

fn resolve_access(user: UserId, world_role: WorldRole, doc: &Document) -> Access;
fn filter_properties(doc: &Document, access: Access) -> Document;   // strips GmOnly properties for non-GM
```

`Access` is the resolved effective permission for a (user, document) pair: read/write capability plus whether GM-only properties are visible. `filter_properties` consumes it to produce the recipient's view.

Server role lives on `users`, world role in `world_members`. M2 ships these types and the pure, unit-tested resolution/filter functions only — no connection wiring (that is M5's `PermissionContext`).

## 9. Validation, migration seam

```rust
const MAX_SYSTEM_BYTES: usize = 256 * 1024;
fn validate_envelope(json: &str) -> Result<Document>;   // serde deny_unknown_fields
fn validate_system_size(doc: &Document) -> Result<()>;  // reject oversized opaque bodies
fn validate_field_path(path: &str) -> Result<()>;       // valid JSON pointer

fn migrate(doc: Document) -> Document;   // if schema_version < CURRENT, apply registered steps; currently no-op
```

No semantic validation of `system` — it is opaque. `migrate` is the machinery only; no actual migrations exist pre-ship.

## 10. Repository trait (Postgres-later seam)

```rust
#[async_trait]
trait Repository {
    async fn create_world(&self, name: &str) -> Result<World>;
    async fn get_world(&self, id: Uuid) -> Result<Option<World>>;

    async fn get_document(&self, id: Uuid) -> Result<Option<Document>>;
    async fn query_documents(&self, world_id: Uuid, doc_type: &str) -> Result<Vec<Document>>;
    async fn documents_by_source(&self, pack: Option<&str>, source_id: Uuid) -> Result<Vec<Document>>; // parent-push

    async fn apply_command(&self, world_id: Uuid, cmd: UnsequencedCommand) -> Result<Command>; // alloc seq + append + apply, ONE txn
    async fn events_since(&self, world_id: Uuid, seq: u64) -> Result<Vec<Command>>;

    // minimal user / member accessors
}
```

`apply_command` is the heart: in a single SQLite transaction it allocates the next per-world seq, appends the command to `world_events`, applies every operation to `documents` (create / delete / field-path update), and stamps `documents.seq`. Either the whole command commits or none of it does. `SqliteRepository` is the only implementation in M2.

## 11. Testing (in-memory SQLite)

- Envelope round-trip; unknown-field rejection (`deny_unknown_fields`).
- `apply(cmd)` then `apply(cmd.invert())` returns the store to its prior state (round-trip), for create, delete, and update commands.
- `apply_command` is atomic (a failing op rolls the whole command back) and seq is strictly monotonic per world and continuous across a simulated restart (re-init from the log).
- `events_since` returns the correct suffix.
- `documents_by_source` finds instances for parent-push.
- `resolve_access` and `filter_properties` over owner / observer / none and a GM-only property.
- Validation rejects oversized `system` and malformed field paths.
- `migrate` returns a current-version document unchanged.
