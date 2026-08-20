# Phase 1b — Commit-Time Replay Redaction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the two defects in `docs/OPEN_BUGS.md`'s first two bullets — replay redacting
against a document's CURRENT permission set instead of the policy in force at the historical
seq being replayed, and a stale `Update`/`Delete` redacting against a new document that reuses
a deleted id — by carrying a commit-time redaction snapshot alongside every `Command`, server-side
only.

**Architecture:** A new server-internal `StoredCommand { command: Command, snapshot:
CommandSnapshot }` type replaces bare `Command` everywhere it is used as internal transport
(both authoritative write loops, `world_events.command_json`, the room broadcast/ring, and
`Repository::events_since`). At redaction time `filter_command` computes the recipient's hidden
set as `hidden_current ∪ hidden_commit`, where `hidden_commit` is derived purely from the
snapshot (no live lookup). The CLIENT-FACING wire (`Operation`, `Command`, `ClientMsg`,
`ServerMsg`, the Zod schema) is untouched.

**Tech Stack:** Rust (axum/tokio/sqlx), `cargo fmt`/`cargo clippy`/`cargo test`, SQLite (single
baseline migration, no incremental migrations pre-customers).

**Spec:** `docs/superpowers/specs/2026-08-20-phase1b-replay-redaction-design.md` — read in full
before implementing any task. Its "Revision note" documents six buddy-check-found corrections
that are binding, not optional. `docs/OPEN_BUGS.md`'s first two bullets are the authoritative
attack scenarios this closes; `docs/superpowers/specs/2026-08-13-phase1b-design-findings.md`
(cited by the design doc) documents the earlier rejected proposals — do not re-litigate them.

## Global Constraints

- **Iron rule (campaign directive, binding on every subagent this campaign):** No deferrals of
  existing work or new work as it comes up — fix it now unless the user gives EXPRESS
  authorization. The only exception is a bug/TODO with a genuine blocker already logged in a
  milestone in `docs/PLAN.md` that has not started. When faced with a design fork, determine the
  best long-term shape in keeping with our plans and goals and implement accordingly; only ask the
  user if that question is genuinely unanswerable. Churn is not a concern. **This paragraph must
  be copied verbatim into every subagent dispatched for this campaign.**
- **No lint suppressions of any kind.** `#[allow(dead_code)]`, `#[allow(unused*)]`,
  `#[allow(clippy::*)]`, and `#[expect(...)]` are ALL forbidden — no exceptions, no per-instance
  sign-off requests in this campaign. Fix the code, make it live, `#[cfg(test)]`-scope test-only
  items, or delete them. Finding one already in the tree during this work is a defect to fix, not
  a precedent to follow.
- **RULE 15:** cite symbols (type/function/method names) in code comments, never file names or
  line numbers.
- **RULE 16 (no ephemeral referents in CODE comments):** no milestone ids, no dated doc pointers,
  no history/process narration in `.rs` source comments. The design spec and this plan are
  `docs/superpowers/` artifacts and are exempt from this rule themselves, but nothing from either
  may be copy-pasted into a CODE comment as a citation (e.g. never write `// per Phase 1b spec` in
  a `.rs` file — state the invariant/reason directly instead).
- **This is a server-only, no-client-touch change.** `Operation`, `Command`, `ClientMsg`,
  `ServerMsg`, and the generated `src/types/generated/*` / client Zod mirror must be
  byte-identical before and after every task (verified explicitly in Task 4 and again at
  campaign close via `git diff --exit-code src/types/generated`, mirroring CI's own check). No
  task in this plan touches `src/client/`, `src/modules/`, or any `.svelte`/`.ts` file. If a task
  implementer finds themselves about to touch client code, STOP and report — that means a design
  assumption broke, not that the touch is fine.
- **No data migrations pre-customers.** Edit `src/server/migrations/0001_init.sql` in place; do
  not add an incremental migration file. A dev database predating this edit fails the sqlx
  checksum check — delete the dev DB file and restart (this project's existing documented policy,
  stated at the top of `0001_init.sql`).
- **Per-task CI gate battery** (run from the repository root — the workspace `Cargo.toml` at
  `C:\Dev\Shadowcat\Cargo.toml` has members `src/server` and `src/server/test-support`), all must
  exit 0 before a task is considered done:
  1. `cargo fmt --all -- --check`
  2. `cargo clippy --all-targets -- -D warnings`
  3. `cargo test --all`
  4. `cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D missing-docs -D clippy::missing-docs-in-private-items`
  5. `cargo +nightly doc --manifest-path src/server/Cargo.toml --document-private-items --no-deps --target-dir target/nightly-doc`
- **No client/TS gate runs for this plan's tasks.** `pnpm -r test`/`pnpm -r typecheck`/`pnpm lint`
  are NOT part of this plan's per-task gate, because no task touches client code (see above). If
  that invariant is ever violated mid-task, the implementer stops and reports immediately rather
  than silently adding the client gate.
- **Every new/changed item needs a doc comment.** `src/server/src/**` enforces
  `#![deny(missing_docs)]` + `#![deny(clippy::missing_docs_in_private_items)]` per-module; gate 4
  above re-enforces it project-wide with `-D` for CI parity.
- **Never fork a decision across two paths.** Where two code paths must agree (e.g. the live
  redaction traversal and the commit-time snapshot traversal), make one derive from the other or
  have both read one shared symbol. This plan's Task 2 makes `collect_overrides` that shared
  symbol for the override-tree walk.
- Never delete files with `rm`/`Remove-Item`; use `trash`.
- **Reviewed Skill-Update Gate:** this work touches `src/server/src/data/permission.rs`,
  `src/server/src/data/sqlite.rs`, `src/server/src/data/repository.rs`, `src/server/src/ws/room.rs`,
  `src/server/src/ws/conn.rs`, and adds `src/server/src/data/snapshot.rs` — squarely
  `shadowcat-codebase-documents-permissions` (redaction/permission core) and
  `shadowcat-codebase-realtime-sync` (Room/broadcast/resync internals) territory. Task 5 updates
  both skills, dispatches `shadowcat-spec-reviewer` on the skill diffs, and bumps
  `.claude/.claude-plugin/plugin.json`'s `version` from `1.0.51` to `1.0.52` (then re-runs the
  marketplace update in any consuming repo, per the gate's own instructions — none exists for this
  repo today, so this step is a no-op beyond the version bump here).
- **TODO.md cleanup:** once Task 4 is verified end-to-end, Task 5 removes the entire
  `## Actionable now — Phase 1b re-brainstorm: point-in-time replay redaction (commit-time
  snapshot)` heading and its body from `docs/TODO.md` (the design is now implemented, not merely
  scheduled) and adds a `docs/OPEN_BUGS.md`-to-`docs/CLOSED_BUGS.md` move for both closed defects.

## Task Decomposition Rationale

Five tasks, each producing an independently reviewable, independently testable deliverable:

1. **Foundational types + schema + small accessors** — no behavior change yet; a reviewer can
   fully verify `StoredCommand`/`CommandSnapshot`/`OpSnapshot`'s shape, the `created_seq` column
   semantics, and the new repository accessors in isolation.
2. **`permission.rs` redaction core** — the pure, DB-free heart of the fix (`collect_overrides`
   split, `filter_command` rewrite). Independently testable by hand-constructing
   `CommandSnapshot`/`OpSnapshot` values; does not need the write loops to exist yet.
3. **Both write loops (`apply_command` + `apply_intent`) + `events_since` + the `Repository`
   trait signature changes.** These three land together on purpose: they implement one trait
   whose signature changes atomically, and both write loops share one new helper
   (`SqliteRepository::build_op_snapshot`) whose correctness only means something once both
   callers exist. Splitting this further would leave the crate non-compiling mid-task.
4. **Room/ring/broadcast plumbing + `ws/conn.rs` egress rewiring** — the internal transport
   change (`RoomEvent`, `Room.tx`, `RingBuffer`, `send_filtered` → `send_room_event`/
   `send_filtered_event`/`send_plain`, `resync_range`, `replay`). Depends on Tasks 2–3;
   independently reviewable as "does the broadcast/replay pipe correctly carry and reduce the
   snapshot."
5. **Documentation/skill closeout** — `TODO.md`/`OPEN_BUGS.md`/`CLOSED_BUGS.md` sync, skill
   updates + `shadowcat-spec-reviewer` dispatch, plugin version bump, final full gate re-run.

---

## Task 1: Commit-time snapshot types, `created_seq` column, and small repository accessors

**Files:**
- Create: `src/server/src/data/snapshot.rs`
- Modify: `src/server/src/data/mod.rs` (register the new module)
- Modify: `src/server/migrations/0001_init.sql` (add `documents.created_seq`)
- Modify: `src/server/src/data/sqlite.rs` — `upsert_document` (bind `created_seq`), add
  `document_created_seq`, `world_member_roles`, `get_document_with_created_seq` (trait impl)
- Modify: `src/server/src/data/repository.rs` — add `get_document_with_created_seq` to the
  `Repository` trait
- Modify: `src/server/src/ws/room.rs:1223-1353` (`DeleteMidHydration` test mock — add
  `get_document_with_created_seq` delegate)
- Modify: `src/server/src/data/command.rs:111-133` (`Operation::invert` doc comment)
- Test: `src/server/src/data/snapshot.rs` (`#[cfg(test)] mod tests`)
- Test: `src/server/src/data/sqlite.rs` (`#[cfg(test)] mod tests` — new tests for the three new
  accessors)

**Interfaces:**
- Produces: `crate::data::snapshot::{StoredCommand, CommandSnapshot, OpSnapshot}` (all `pub`, all
  `Debug + Clone + PartialEq + Serialize + Deserialize`); `StoredCommand::from_stored_json(raw:
  &str) -> Result<StoredCommand, serde_json::Error>`.
- Produces: `SqliteRepository::document_created_seq<'e, E>(executor: E, id: Uuid) ->
  Result<Option<i64>, DataError>` (private, tx-generic).
- Produces: `SqliteRepository::world_member_roles<'e, E>(executor: E, world_id: Uuid) ->
  Result<HashMap<Uuid, WorldRole>, DataError>` (private, tx-generic).
- Produces: `Repository::get_document_with_created_seq(&self, id: Uuid) ->
  Result<Option<(Document, i64)>, DataError>` (new trait method, implemented by
  `SqliteRepository` and delegated by `DeleteMidHydration`).
- Consumed by: Task 2 (`OpSnapshot`/`CommandSnapshot` fields, `get_document_with_created_seq`),
  Task 3 (`StoredCommand`, `document_created_seq`, `world_member_roles`), Task 4
  (`StoredCommand`).

### Design note settled here (spec gap, resolved — see also this plan-writer's dispatch report)

The design spec's Components §1 literally types `OpSnapshot.gm_at_commit: bool` (one bool per
op) but its own Components §4 prose describes computing it as a `HashMap<Uuid, bool>` **once per
command** and having `filter_command` "look up the redacting recipient's own entry" — a single
per-op `bool` cannot represent an arbitrary future recipient's status, since `OpSnapshot` is built
once and shared across every future replay. This is a genuine internal contradiction in the spec.
**Resolved here in favor of the workable, prose-supported shape:** the field moves from
`OpSnapshot` to `CommandSnapshot` as `world_gm_at_commit: HashMap<Uuid, bool>` — computed once per
command (world role has nothing to do with which documents an op touches; it is genuinely
command-scoped, not op-scoped).

A second gap, also resolved here (used starting Task 3): the spec's `OpSnapshot` fields
(`owner_at_commit`, `doc_type`, `overrides_at_commit`, `retraction_hidden_at_commit`,
`created_seq_at_commit`) are insufficient to construct the whole-document `cap::READ` gate for an
`Update` op (Components §3's asymmetry fix), because `permission::resolve_access`/
`effective_role`/`role_floor` — the ONLY correct, non-forked way to resolve a document's
capability floor — require the document's own `PermissionSet` (`default`/`users`/`gm_role`/
`capabilities`), which an `Operation::Update` carries no copy of (unlike `Create`/`Delete`, whose
carried `doc` already has it). `OpSnapshot` therefore also carries
`permissions_at_commit: Option<PermissionSet>` (`Some` only for `Update`; `property_overrides` is
always left empty on it — that data is separately, already captured, pruned, in
`overrides_at_commit`/`retraction_hidden_at_commit`). This is stated now so it isn't
rediscovered mid-Task-2/3 as a surprise.

- [ ] **Step 1: Read the design spec's Components §1 and §4 in full again**, confirming the two
  resolutions above against the literal text (`docs/superpowers/specs/2026-08-20-phase1b-replay-redaction-design.md`).

- [ ] **Step 2: Create `src/server/src/data/snapshot.rs`**

```rust
//! Commit-time redaction snapshot: `StoredCommand`, `CommandSnapshot`, `OpSnapshot`. Carries
//! the policy in force AT COMMIT alongside a `Command`, so replay redaction can compute the
//! recipient's hidden set as `hidden_current ∪ hidden_commit` instead of re-deriving
//! `hidden_commit` from today's (wrong) policy. Server-internal only: never serialized to the
//! wire — `Operation`/`Command`/`ClientMsg`/`ServerMsg` and their ts-rs/Zod mirrors are
//! untouched by this module's existence.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::data::command::Command;
use crate::data::document::{PermissionSet, Visibility};

/// Commit-time redaction inputs for one op in a `Command`, sufficient to compute the
/// commit-time half of redaction WITHOUT any live lookup — no `&Repository`, no actor-lookup
/// closure, by construction (a live parameter cannot be reintroduced here without a loud
/// signature change). Built ONCE per command, from the command's own post-image (never from an
/// op's own per-iteration intermediate state — a per-op snapshot mid-command would leak a value
/// a LATER op in the same command hides).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpSnapshot {
    /// Effective owner at commit (`permission::effective_owner`/`SqliteRepository::
    /// load_effective_owner` evaluated against the post-image's actor-link state). `None` if
    /// the document has no effective owner.
    pub owner_at_commit: Option<Uuid>,
    /// `doc_type` at commit. Carried here too (not just read off `Operation::Create`/`Delete`'s
    /// own `doc`) because `Operation::Update` has no `doc_type` of its own, and
    /// `permission::effective_role`'s token-owner-floor check needs it.
    pub doc_type: String,
    /// The document's permission-override tree at commit: `property_overrides` for the document
    /// itself plus every embedded descendant, addressed identically to the live redaction
    /// walk's convention (`{prefix}/embedded/{key}/{idx}`, built from the POST-image's
    /// `embedded` map). For `Update`, pruned to the ancestor/descendant closure of this op's own
    /// `changes` paths (only an overlapping override can possibly redact THIS op's field-level
    /// deltas) UNLESS `retraction_hidden_at_commit` is `Some`, in which case that field carries
    /// the full, unpruned set separately. For `Create`/`Delete` (whose "changed paths" are the
    /// whole document), this is the full, unpruned set.
    pub overrides_at_commit: Vec<(String, Visibility)>,
    /// Present only when this `Update` op's own `changes` narrow visibility
    /// (`permission::touches_permissions`): the FULL (unpruned) commit-time hidden-pointer set
    /// for the document, with each pointer's `Visibility` tier retained (never a bare pointer
    /// list — the tier is needed to filter per-recipient via `Access::can_see` at replay time,
    /// not apply the same retraction to every recipient regardless of their own access). Always
    /// `None` for `Create`/`Delete` (a whole-document reveal/removal needs no incremental
    /// retraction of stale client-side field values).
    pub retraction_hidden_at_commit: Option<Vec<(String, Visibility)>>,
    /// Present only for `Update`/`Delete` (a `Create` establishes a fresh generation and needs
    /// no witness): the target document's `documents.created_seq` as read at commit time.
    /// Compared against the CURRENT document's `created_seq` at redaction time; a mismatch means
    /// the id was deleted and recreated since commit, and the op is dropped rather than
    /// redacted-and-delivered against the wrong generation.
    pub created_seq_at_commit: Option<i64>,
    /// The target document's OWN `PermissionSet` at commit — `default`/`users`/`gm_role`/
    /// `capabilities` only; `property_overrides` is always empty here (that data is separately
    /// captured, pruned, in `overrides_at_commit`/`retraction_hidden_at_commit`). `Some` only
    /// for `Update`, whose `Operation` carries no `permissions` of its own to reuse directly
    /// (unlike `Create`/`Delete`, which reuse their own carried `doc.permissions` verbatim).
    /// Feeds the whole-document commit-time `cap::READ` gate via `permission::resolve_access`,
    /// reused unmodified rather than re-derived.
    pub permissions_at_commit: Option<PermissionSet>,
}

/// Commit-time redaction inputs for a whole `Command`, index-aligned with `Command.ops`. Built
/// ONCE per command, after every op in the command has applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandSnapshot {
    /// `None` at an index means "no snapshot recorded for this op" — the back-compat case (a
    /// `world_events` row written before this design existed). `filter_command` DROPS an op
    /// whose snapshot is `None` on replay, rather than falling back to a live-lookup redaction.
    pub per_op: Vec<Option<OpSnapshot>>,
    /// Whether each of the world's members held GM standing in this world AT THIS COMMAND'S
    /// COMMIT — computed once per command (world role has nothing to do with which documents an
    /// op touches, unlike `overrides_at_commit`). `filter_command` looks up the redacting
    /// recipient's own entry, defaulting to `false` (fail-closed, non-GM) for a user absent from
    /// this map (not yet a world member at commit time).
    pub world_gm_at_commit: HashMap<Uuid, bool>,
}

/// A `Command` paired with its commit-time redaction snapshot. Server-internal transport shape:
/// never serialized to the wire. Persisted into `world_events.command_json` and carried through
/// the room broadcast/ring/resync path in place of a bare `Command`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredCommand {
    /// The wire-shaped command.
    pub command: Command,
    /// The commit-time redaction snapshot, index-aligned with `command.ops`.
    pub snapshot: CommandSnapshot,
}

impl StoredCommand {
    /// Deserialize a `world_events.command_json` row, tolerating pre-fix rows written before
    /// this design (bare `Command` JSON, with neither a `command` nor a `snapshot` key at the
    /// top level — the two shapes are structurally disjoint, so `Command`'s own fields never
    /// satisfy `StoredCommand`'s). A pre-fix row is wrapped with an all-`None` `CommandSnapshot`
    /// and an empty `world_gm_at_commit` map: `filter_command` then drops every op in it on
    /// replay rather than falling back to a live-lookup redaction — a one-time, accepted cost
    /// against pre-fix history, never a silent gap.
    pub fn from_stored_json(raw: &str) -> Result<Self, serde_json::Error> {
        if let Ok(stored) = serde_json::from_str::<StoredCommand>(raw) {
            return Ok(stored);
        }
        let command: Command = serde_json::from_str(raw)?;
        let per_op = vec![None; command.ops.len()];
        Ok(StoredCommand {
            command,
            snapshot: CommandSnapshot {
                per_op,
                world_gm_at_commit: HashMap::new(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::command::Operation;

    fn sample_command() -> Command {
        Command {
            seq: 7,
            world_id: Uuid::from_u128(1),
            author: Uuid::from_u128(2),
            ts: 100,
            ops: vec![Operation::Update {
                doc_id: Uuid::from_u128(3),
                changes: vec![],
            }],
        }
    }

    #[test]
    fn stored_command_round_trips_through_json() {
        let stored = StoredCommand {
            command: sample_command(),
            snapshot: CommandSnapshot {
                per_op: vec![Some(OpSnapshot {
                    owner_at_commit: Some(Uuid::from_u128(4)),
                    doc_type: "actor".into(),
                    overrides_at_commit: vec![("/system/secret".into(), Visibility::GmOnly)],
                    retraction_hidden_at_commit: None,
                    created_seq_at_commit: Some(5),
                    permissions_at_commit: Some(PermissionSet::default()),
                })],
                world_gm_at_commit: HashMap::from([(Uuid::from_u128(4), true)]),
            },
        };
        let s = serde_json::to_string(&stored).unwrap();
        let back = StoredCommand::from_stored_json(&s).unwrap();
        assert_eq!(stored, back);
    }

    #[test]
    fn from_stored_json_falls_back_for_a_pre_fix_bare_command_row() {
        let cmd = sample_command();
        let raw = serde_json::to_string(&cmd).unwrap();
        let stored = StoredCommand::from_stored_json(&raw).unwrap();
        assert_eq!(stored.command, cmd);
        assert_eq!(stored.snapshot.per_op, vec![None]);
        assert!(stored.snapshot.world_gm_at_commit.is_empty());
    }

    #[test]
    fn from_stored_json_rejects_genuinely_malformed_json() {
        assert!(StoredCommand::from_stored_json("{not json").is_err());
    }
}
```

- [ ] **Step 3: Register the module in `src/server/src/data/mod.rs`**

Add, alphabetically among the other `pub mod` lines (after `pub mod search;`, before `pub mod
sqlite;`):

```rust
/// Commit-time redaction snapshot types (`StoredCommand`/`CommandSnapshot`/`OpSnapshot`) —
/// server-internal transport, never serialized to the wire.
pub mod snapshot;
```

- [ ] **Step 4: Run the new module's tests**

Run: `cargo test --manifest-path src/server/Cargo.toml data::snapshot`
Expected: 3 tests pass (`stored_command_round_trips_through_json`,
`from_stored_json_falls_back_for_a_pre_fix_bare_command_row`,
`from_stored_json_rejects_genuinely_malformed_json`).

- [ ] **Step 5: Add the `created_seq` column to `src/server/migrations/0001_init.sql`**

In the `documents` table definition (currently at lines 32-48), add the new column right after
`seq INTEGER NOT NULL DEFAULT 0,`:

```sql
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
  created_seq INTEGER NOT NULL DEFAULT 0,
  json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  parent_id TEXT REFERENCES documents(id) ON DELETE CASCADE
);
```

- [ ] **Step 6: Bind `created_seq` in `SqliteRepository::upsert_document`**

In `src/server/src/data/sqlite.rs`, the `upsert_document` function (currently at lines 1642-1720):
widen the INSERT column list and VALUES placeholder count by one, bind the write's own `seq` as
`created_seq`, and — critically — do **not** list `created_seq` in the `ON CONFLICT ... DO UPDATE
SET` clause (SQLite's `excluded.*` semantics then leave the STORED value untouched across an
UPDATE, which is what makes this "set once, at genuine first INSERT" rather than "last touched").

Replace the `sqlx::query(...)` call (the SQL string and its first 15 `.bind(...)` calls) with:

```rust
        sqlx::query(
            "INSERT INTO documents (id, scope_kind, world_id, pack, doc_type, schema_version, \
             source_id, source_pack, source_version, owner_id, parent_id, seq, created_seq, json, \
             created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET scope_kind=excluded.scope_kind, world_id=excluded.world_id, \
             pack=excluded.pack, doc_type=excluded.doc_type, schema_version=excluded.schema_version, \
             source_id=excluded.source_id, source_pack=excluded.source_pack, \
             source_version=excluded.source_version, owner_id=excluded.owner_id, \
             parent_id=excluded.parent_id, seq=excluded.seq, \
             json=excluded.json, updated_at=excluded.updated_at",
        )
        .bind(doc.id.to_string())
        .bind(scope_kind)
        .bind(world_id.clone())
        .bind(pack)
        .bind(&doc.doc_type)
        .bind(doc.schema_version as i64)
        .bind(source_id)
        .bind(source_pack)
        .bind(source_version)
        .bind(doc.owner.map(|o| o.to_string()))
        .bind(doc.parent_id.map(|p| p.to_string()))
        .bind(seq)
        .bind(seq)
        .bind(json)
        .bind(doc.created_at)
        .bind(doc.updated_at)
        .execute(&mut *conn)
        .await?;
```

Note: `created_seq` is deliberately **absent** from the `ON CONFLICT ... DO UPDATE SET` list —
that omission is what makes it "set once at genuine first INSERT." The two `.bind(seq)` calls in
a row are correct: the first binds the `seq` placeholder, the second binds `created_seq` (both
are the SAME write's own `seq` value on a genuine first insert; on a conflict-update the second
bind is present in the VALUES list but never applied, because `created_seq` is unlisted in the
UPDATE SET clause).

- [ ] **Step 7: Add `document_created_seq` (tx-generic accessor) to `SqliteRepository`**

In `src/server/src/data/sqlite.rs`, add this new private method to `impl SqliteRepository`
(place it directly after `load_effective_owner`, currently ending at line 1563, before
`singleton_doc_exists`):

```rust
    /// `documents.created_seq` for `id`, or `None` if the row doesn't exist. Set once at a
    /// row's genuine first INSERT (`upsert_document`'s `ON CONFLICT` clause omits it, so
    /// SQLite's `excluded.*` semantics leave it untouched across an update) and never touched
    /// again by subsequent updates to a still-live row — the generation marker
    /// `OpSnapshot::created_seq_at_commit` compares against to detect an id reused after a hard
    /// delete. Runs on the caller's transaction (never `&self.pool`, which would deadlock
    /// mid-transaction on the single-writer pool).
    async fn document_created_seq<'e, E>(executor: E, id: Uuid) -> Result<Option<i64>, DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let row = sqlx::query("SELECT created_seq FROM documents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(executor)
            .await?;
        Ok(row.map(|r| r.get::<i64, _>("created_seq")))
    }
```

- [ ] **Step 8: Add `world_member_roles` (tx-generic accessor) to `SqliteRepository`**

Directly after `document_created_seq`:

```rust
    /// Every CURRENT member's world role, on an arbitrary executor (so it can run inside the
    /// `apply_command`/`apply_intent` transaction). Feeds `CommandSnapshot::world_gm_at_commit`
    /// — captured once per command, at the point the command is committing, which IS "at commit
    /// time" for this purpose: the whole point of capturing it now is to freeze what would
    /// otherwise be re-derived live on every future replay.
    async fn world_member_roles<'e, E>(
        executor: E,
        world_id: Uuid,
    ) -> Result<std::collections::HashMap<Uuid, WorldRole>, DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let rows = sqlx::query("SELECT user_id, role FROM world_members WHERE world_id = ?")
            .bind(world_id.to_string())
            .fetch_all(executor)
            .await?;
        rows.into_iter()
            .map(|r| {
                let uid = Uuid::parse_str(r.get::<String, _>("user_id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                let role: WorldRole =
                    serde_json::from_value(serde_json::Value::String(r.get::<String, _>("role")))?;
                Ok((uid, role))
            })
            .collect()
    }
```

`WorldRole` is already imported in `sqlite.rs` (`use crate::data::document::{..., WorldRole};`
near the top of the file) — confirm this import exists before adding the method; if the exact
import list differs from what you see, add `WorldRole` to it.

- [ ] **Step 9: Add `get_document_with_created_seq` to the `Repository` trait**

In `src/server/src/data/repository.rs`, add this new trait method to `pub trait Repository`
(place it directly after `get_document`, currently ending at line 66):

```rust
    /// A document by id together with its `documents.created_seq` generation marker, or `None`
    /// if it does not exist. One round trip, not two: this is the redaction hot path's own read
    /// (`permission::load_current_docs`, called once per recipient per event), where a second
    /// separate `created_seq` query would double an already-hot per-recipient cost. Unredacted,
    /// like `get_document` — callers gate egress themselves.
    async fn get_document_with_created_seq(
        &self,
        id: Uuid,
    ) -> Result<Option<(Document, i64)>, DataError>;
```

- [ ] **Step 10: Implement `get_document_with_created_seq` on `SqliteRepository`**

In `src/server/src/data/sqlite.rs`, inside `impl Repository for SqliteRepository` (add directly
after the existing `get_document` implementation, currently ending around line 2536):

```rust
    async fn get_document_with_created_seq(
        &self,
        id: Uuid,
    ) -> Result<Option<(Document, i64)>, DataError> {
        let row = sqlx::query("SELECT json, created_seq FROM documents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => {
                let doc: Document = serde_json::from_str(r.get::<String, _>("json").as_str())?;
                let created_seq: i64 = r.get("created_seq");
                Ok(Some((doc, created_seq)))
            }
            None => Ok(None),
        }
    }
```

- [ ] **Step 11: Delegate the new method on `DeleteMidHydration`**

In `src/server/src/ws/room.rs`, inside the `impl Repository for DeleteMidHydration<'_>` block
(currently spanning lines 1223-1353), add, directly after `get_document` (currently ending at
line 1246):

```rust
        async fn get_document_with_created_seq(
            &self,
            id: Uuid,
        ) -> Result<Option<(Document, i64)>, DataError> {
            self.inner.get_document_with_created_seq(id).await
        }
```

- [ ] **Step 12: Write tests for the three new accessors**

In `src/server/src/data/sqlite.rs`'s existing `#[cfg(test)] mod tests` block, add:

```rust
    #[tokio::test]
    async fn created_seq_is_set_once_and_survives_updates() {
        use crate::data::command::{FieldChange, Operation, UnsequencedCommand};
        use crate::data::document::{DocRole, PermissionSet, Scope};
        use crate::data::membership::PermissionContext;

        let r = repo().await;
        let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();
        let ctx = PermissionContext { user_id: gm, world_role: WorldRole::Gm };
        let mut perms = PermissionSet::default();
        perms.users.insert(gm, DocRole::Owner);
        let mut d = tests_engine_doc(perms, "item", serde_json::json!({ "hp": 1 }));
        d.scope = Scope::World { world_id: w.id };
        let doc_id = d.id;

        let stored = r
            .apply_intent(&ctx, w.id, vec![Operation::Create { doc: d }], 1, WriteOrigin::Client)
            .await
            .unwrap();
        let first_seq = stored.command.seq;

        let mut tx = r.pool.begin().await.unwrap();
        let created_after_create = SqliteRepository::document_created_seq(&mut *tx, doc_id)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(created_after_create, Some(first_seq));

        r.apply_intent(
            &ctx,
            w.id,
            vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/hp".into(),
                    old: serde_json::json!(1),
                    new: serde_json::json!(2),
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let mut tx = r.pool.begin().await.unwrap();
        let created_after_update = SqliteRepository::document_created_seq(&mut *tx, doc_id)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            created_after_update, created_after_create,
            "created_seq must not change across an update to a still-live row"
        );
    }

    #[tokio::test]
    async fn created_seq_is_absent_for_a_missing_document() {
        let r = repo().await;
        let mut tx = r.pool.begin().await.unwrap();
        let missing = SqliteRepository::document_created_seq(&mut *tx, Uuid::new_v4())
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(missing, None);
    }

    #[tokio::test]
    async fn world_member_roles_reflects_every_current_member() {
        let r = repo().await;
        let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
        let player = r.create_user("pl", None, ServerRole::User, 0).await.unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();
        r.add_member(w.id, player, WorldRole::Player).await.unwrap();

        let mut tx = r.pool.begin().await.unwrap();
        let roles = SqliteRepository::world_member_roles(&mut *tx, w.id).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(roles.get(&gm), Some(&WorldRole::Gm));
        assert_eq!(roles.get(&player), Some(&WorldRole::Player));
    }

    #[tokio::test]
    async fn get_document_with_created_seq_matches_a_separate_created_seq_read() {
        use crate::data::command::Operation;
        use crate::data::document::{DocRole, PermissionSet, Scope};
        use crate::data::membership::PermissionContext;

        let r = repo().await;
        let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();
        let ctx = PermissionContext { user_id: gm, world_role: WorldRole::Gm };
        let mut perms = PermissionSet::default();
        perms.users.insert(gm, DocRole::Owner);
        let mut d = tests_engine_doc(perms, "item", serde_json::json!({ "hp": 1 }));
        d.scope = Scope::World { world_id: w.id };
        let doc_id = d.id;
        r.apply_intent(&ctx, w.id, vec![Operation::Create { doc: d }], 1, WriteOrigin::Client)
            .await
            .unwrap();

        let (doc, created_seq) = r
            .get_document_with_created_seq(doc_id)
            .await
            .unwrap()
            .expect("document must exist");
        assert_eq!(doc.id, doc_id);
        let mut tx = r.pool.begin().await.unwrap();
        let separate = SqliteRepository::document_created_seq(&mut *tx, doc_id).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(Some(created_seq), separate);
    }
```

Check the exact name of the existing test-repo constructor helper (`repo()`) and the engine-doc
fixture helper (`tests_engine_doc`) by reading the surrounding `#[cfg(test)] mod tests` block in
`src/server/src/data/sqlite.rs` before pasting — use whatever the file's own existing tests
already call (these two helpers are used by dozens of existing tests in the same module, e.g. the
`create_of_non_engine_doc_type_with_engine_body_is_rejected` test read during plan-writing).

- [ ] **Step 13: Run the new sqlite tests**

Run: `cargo test --manifest-path src/server/Cargo.toml data::sqlite::tests::created_seq -- --nocapture`
Run: `cargo test --manifest-path src/server/Cargo.toml data::sqlite::tests::world_member_roles`
Run: `cargo test --manifest-path src/server/Cargo.toml data::sqlite::tests::get_document_with_created_seq`
Expected: all 4 new tests pass.

- [ ] **Step 14: Settle `Operation::invert`'s relationship to the snapshot (spec's own flagged
  open item)**

In `src/server/src/data/command.rs`, extend `Operation::invert`'s existing doc comment (currently
at lines 111-112) to state explicitly that inversion is defined over the snapshot-free wire type
and has no `StoredCommand`/`CommandSnapshot` concern:

```rust
    /// The inverse operation: Create<->Delete; Update swaps old/new per change, reversed.
    ///
    /// Operates on the wire `Operation` only — `StoredCommand`/`CommandSnapshot` (server-internal
    /// commit-time redaction state) do not exist at this layer and are not this function's
    /// concern; a future undo/redo feature that resurrects a `StoredCommand`'s `command` via
    /// `invert` must derive a FRESH snapshot for the inverted write, never carry the original's
    /// forward.
    ///
```

(Keep the existing `# Examples` doctest block immediately below unchanged.)

- [ ] **Step 15: Run the full per-task CI gate battery**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D missing-docs -D clippy::missing-docs-in-private-items
cargo +nightly doc --manifest-path src/server/Cargo.toml --document-private-items --no-deps --target-dir target/nightly-doc
```

Expected: all five exit 0. Note: the dev SQLite database file (if any exists locally from a prior
run) predates the `created_seq` column and will fail the sqlx migration checksum on next server
start — delete it (via `trash`, never `rm`) before any manual server run; `cargo test` uses fresh
in-memory databases per test and is unaffected.

- [ ] **Step 16: Commit**

```bash
git add src/server/src/data/snapshot.rs src/server/src/data/mod.rs \
  src/server/migrations/0001_init.sql src/server/src/data/sqlite.rs \
  src/server/src/data/repository.rs src/server/src/ws/room.rs src/server/src/data/command.rs
git commit -m "feat(data): add commit-time redaction snapshot types and created_seq column"
```

---

## Task 2: `permission.rs` redaction core rewrite

**Files:**
- Modify: `src/server/src/data/permission.rs` (imports; `collect_hidden` split into
  `collect_overrides` + `hidden_from_overrides`; `touches_permissions`/`paths_overlap` visibility;
  `load_update_docs` replaced by `CurrentDoc`/`load_current_docs`; `filter_command` full rewrite)
- Test: `src/server/src/data/permission.rs` (`#[cfg(test)] mod tests` — new pure unit tests +
  mechanical fix of ~40 pre-existing `filter_command`/`load_update_docs` call sites)

**Interfaces:**
- Consumes: `crate::data::snapshot::{CommandSnapshot, OpSnapshot}` (Task 1); `Repository::
  get_document_with_created_seq` (Task 1).
- Produces: `pub struct CurrentDoc { pub doc: Document, pub created_seq: i64 }`.
- Produces: `pub async fn load_current_docs(repo: &dyn Repository, cmd: &Command) ->
  HashMap<Uuid, CurrentDoc>` (replaces `load_update_docs` — renamed AND widened to cover
  `Create`/`Delete` doc_ids, not just `Update`).
- Produces: `pub fn filter_command<'a>(cmd: &Command, snapshot: &CommandSnapshot, ctx:
  &PermissionContext, world_defaults: &WorldCapDefaults, current: &HashMap<Uuid, CurrentDoc>,
  actor_lookup: impl Fn(&Uuid) -> Option<&'a Document>) -> Command` (signature widened: `snapshot`
  inserted after `cmd`; `current`'s value type changed from `Document` to `CurrentDoc`).
- Produces (crate-visible, consumed by Task 3's `SqliteRepository::build_op_snapshot`):
  `pub(crate) fn collect_overrides(doc: &Document, prefix: &str, out: &mut Vec<(String,
  Visibility)>) -> Result<(), RedactionError>`; `pub(crate) fn touches_permissions(path: &str) ->
  bool` (visibility widened from private); `pub(crate) fn paths_overlap(a: &str, b: &str) -> bool`
  (visibility widened from private).
- Consumed by Task 4: `load_current_docs`, `filter_command`'s new signature, `CurrentDoc`.

### Step 1: Read the current file in full

Re-read `src/server/src/data/permission.rs` end to end (non-test code spans lines 1-1188; the
rest is `#[cfg(test)] mod tests`) immediately before editing — this task rewrites a large,
security-critical fraction of it and stale line numbers from an earlier read are a real risk.

### Step 2: Widen imports

At the top of `src/server/src/data/permission.rs`, change:

```rust
use std::collections::BTreeSet;
```

to:

```rust
use std::collections::{BTreeSet, HashMap};
```

and change:

```rust
use crate::data::document::{
    CapabilityGrants, CapabilityRequirement, DocRole, Document, Visibility, WorldCapDefaults,
    WorldRole,
};
```

to:

```rust
use crate::data::document::{
    CapabilityGrants, CapabilityRequirement, DocRole, Document, PermissionSet, Visibility,
    WorldCapDefaults, WorldRole,
};
```

and add, alongside the existing `use crate::data::membership::PermissionContext;` /
`use crate::data::repository::Repository;` lines:

```rust
use crate::data::snapshot::CommandSnapshot;
```

- [ ] **Step 3: Write the failing pure unit tests FIRST** (spec's Testing section, the
  pure-`filter_command`-testable subset — hand-built `CommandSnapshot`/`CurrentDoc` inputs, no DB)

Add this whole block to the end of the existing `#[cfg(test)] mod tests` block in
`src/server/src/data/permission.rs` (after the last existing test, before the closing `}` of the
module):

```rust
    // -------------------------------------------------------------------
    // Commit-time snapshot redaction — pure `filter_command` unit tests.
    // Hand-built `CommandSnapshot`/`CurrentDoc` inputs; no repository round trip.
    // -------------------------------------------------------------------

    use crate::data::snapshot::{CommandSnapshot, OpSnapshot};

    /// A `CurrentDoc` wrapping `doc` at generation `created_seq`.
    fn current_doc(doc: Document, created_seq: i64) -> CurrentDoc {
        CurrentDoc { doc, created_seq }
    }

    /// An `OpSnapshot` for an `Update` op: commit-time owner/gm/permissions plus a pruned
    /// override set, no retraction, no created_seq mismatch (matches the current generation).
    fn op_snapshot_update(
        owner_at_commit: Option<Uuid>,
        overrides_at_commit: Vec<(&str, Visibility)>,
        permissions_at_commit: PermissionSet,
    ) -> OpSnapshot {
        OpSnapshot {
            owner_at_commit,
            doc_type: "actor".into(),
            overrides_at_commit: overrides_at_commit
                .into_iter()
                .map(|(p, v)| (p.to_string(), v))
                .collect(),
            retraction_hidden_at_commit: None,
            created_seq_at_commit: None,
            permissions_at_commit: Some(permissions_at_commit),
        }
    }

    fn snapshot_one_op(op: OpSnapshot, world_gm_at_commit: HashMap<Uuid, bool>) -> CommandSnapshot {
        CommandSnapshot {
            per_op: vec![Some(op)],
            world_gm_at_commit,
        }
    }

    fn permissions_default_observer() -> PermissionSet {
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        }
    }

    fn field_change_update_cmd(
        world: Uuid,
        author: Uuid,
        doc_id: Uuid,
        path: &str,
        old: serde_json::Value,
        new: serde_json::Value,
    ) -> Command {
        Command {
            seq: 1,
            world_id: world,
            author,
            ts: 0,
            ops: vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: path.into(),
                    old,
                    new,
                }],
            }],
        }
    }

    #[test]
    fn filter_command_drops_an_op_with_no_recorded_snapshot() {
        let world = Uuid::from_u128(9);
        let author = Uuid::from_u128(1);
        let doc_id = Uuid::from_u128(2);
        let cmd = field_change_update_cmd(
            world,
            author,
            doc_id,
            "/system/x",
            serde_json::json!(1),
            serde_json::json!(2),
        );
        let snapshot = CommandSnapshot {
            per_op: vec![None],
            world_gm_at_commit: HashMap::new(),
        };
        let cur = doc(permissions_default_observer(), serde_json::json!({ "x": 2 }));
        let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
        let ctx = PermissionContext {
            user_id: Uuid::from_u128(3),
            world_role: WorldRole::Player,
        };
        let out = filter_command(
            &cmd,
            &snapshot,
            &ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        assert!(out.ops.is_empty(), "a None op-snapshot must drop the op on replay");
    }

    #[test]
    fn world_role_promotion_does_not_disclose_pre_promotion_gm_only_or_owner_or_gm_history() {
        // A player, hidden from a GmOnly field and a separate OwnerOrGm field while a non-GM
        // non-owner, is later promoted to GM and resyncs from before the promotion — both
        // fields must stay hidden. INVARIANT: `Access::can_see(OwnerOrGm)` is a disjunction
        // (`see_gm_only || is_owner`), so resolving the commit-time half's `see_gm_only` from
        // the recipient's CURRENT world role would defeat `owner_at_commit` for the OwnerOrGm
        // tier too, not just leak the GmOnly field — both must come from the snapshot alone.
        let world = Uuid::from_u128(9);
        let author = Uuid::from_u128(1);
        let doc_id = Uuid::from_u128(2);
        let owner = Uuid::from_u128(10);
        let recipient = Uuid::from_u128(20);
        let cmd = Command {
            seq: 1,
            world_id: world,
            author,
            ts: 0,
            ops: vec![Operation::Update {
                doc_id,
                changes: vec![
                    FieldChange {
                        remove: false,
                        path: "/system/secret".into(),
                        old: serde_json::Value::Null,
                        new: serde_json::json!("gm secret"),
                    },
                    FieldChange {
                        remove: false,
                        path: "/system/owner_note".into(),
                        old: serde_json::Value::Null,
                        new: serde_json::json!("owner note"),
                    },
                ],
            }],
        };
        let op = op_snapshot_update(
            Some(owner),
            vec![
                ("/system/secret", Visibility::GmOnly),
                ("/system/owner_note", Visibility::OwnerOrGm),
            ],
            permissions_default_observer(),
        );
        // The recipient was NOT GM at commit.
        let snapshot = snapshot_one_op(op, HashMap::from([(recipient, false)]));
        let mut cur = doc(permissions_default_observer(), serde_json::json!({}));
        cur.owner = Some(owner);
        let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
        // The recipient IS currently GM (post-promotion) — this is the defect scenario.
        let ctx = PermissionContext {
            user_id: recipient,
            world_role: WorldRole::Gm,
        };
        let out = filter_command(
            &cmd,
            &snapshot,
            &ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Update { changes, .. } = &out.ops[0] else {
            panic!("expected an Update op");
        };
        assert!(
            changes.is_empty(),
            "both the GmOnly and OwnerOrGm fields must stay hidden: {changes:?}"
        );
    }

    #[test]
    fn reused_id_drops_a_stale_update_against_the_new_generation() {
        let world = Uuid::from_u128(9);
        let author = Uuid::from_u128(1);
        let doc_id = Uuid::from_u128(2);
        let cmd = field_change_update_cmd(
            world,
            author,
            doc_id,
            "/system/x",
            serde_json::json!(1),
            serde_json::json!(2),
        );
        let mut op = op_snapshot_update(None, vec![], permissions_default_observer());
        op.created_seq_at_commit = Some(5); // the OLD generation's created_seq
        let snapshot = snapshot_one_op(op, HashMap::new());
        let cur = doc(permissions_default_observer(), serde_json::json!({ "x": 2 }));
        // The CURRENT document at this id is generation 9 (a later Create reused the id).
        let current = HashMap::from([(doc_id, current_doc(cur, 9))]);
        let ctx = PermissionContext {
            user_id: Uuid::from_u128(3),
            world_role: WorldRole::Player,
        };
        let out = filter_command(
            &cmd,
            &snapshot,
            &ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        assert!(out.ops.is_empty(), "a created_seq mismatch must drop the stale Update");
    }

    #[test]
    fn cross_op_existence_consistency_drops_an_update_denied_at_create_commit_time() {
        // A recipient denied commit-time access to a document's Create, later granted current
        // access, must ALSO have every subsequent Update to that doc_id dropped by the SAME
        // whole-document gate — not just the Create.
        let world = Uuid::from_u128(9);
        let author = Uuid::from_u128(1);
        let doc_id = Uuid::from_u128(2);
        let recipient = Uuid::from_u128(3);
        let cmd = field_change_update_cmd(
            world,
            author,
            doc_id,
            "/system/x",
            serde_json::json!(1),
            serde_json::json!(2),
        );
        // Commit-time permissions: default = None (nobody without an explicit grant may read).
        let denied_at_commit = PermissionSet {
            default: DocRole::None,
            ..Default::default()
        };
        let op = op_snapshot_update(None, vec![], denied_at_commit);
        let snapshot = snapshot_one_op(op, HashMap::new());
        // Current permissions: default = Observer (now anyone may read) — the asymmetry.
        let cur = doc(permissions_default_observer(), serde_json::json!({ "x": 2 }));
        let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
        let ctx = PermissionContext {
            user_id: recipient,
            world_role: WorldRole::Player,
        };
        let out = filter_command(
            &cmd,
            &snapshot,
            &ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        assert!(
            out.ops.is_empty(),
            "commit-time denial must drop the Update even though current access now permits it"
        );
    }

    #[test]
    fn retraction_uses_the_commands_own_commit_moment_not_whatever_is_live() {
        // (a) A command that narrows visibility, replayed long after a LATER command has
        // narrowed it further — the retraction pass must reflect what the CHOSEN command
        // itself hid at ITS OWN commit, not whatever is live now.
        let world = Uuid::from_u128(9);
        let author = Uuid::from_u128(1);
        let doc_id = Uuid::from_u128(2);
        let recipient = Uuid::from_u128(3);
        let cmd = Command {
            seq: 1,
            world_id: world,
            author,
            ts: 0,
            ops: vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/permissions/property_overrides/~1system~1a".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!("gm_only"),
                }],
            }],
        };
        let mut op = op_snapshot_update(None, vec![], permissions_default_observer());
        // This command's OWN narrowing: only "/system/a" became hidden at ITS commit.
        op.retraction_hidden_at_commit =
            Some(vec![("/system/a".to_string(), Visibility::GmOnly)]);
        let snapshot = snapshot_one_op(op, HashMap::from([(recipient, false)]));
        let cur = doc(
            permissions_default_observer(),
            serde_json::json!({ "a": 1, "b": 2 }),
        );
        let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
        let ctx = PermissionContext {
            user_id: recipient,
            world_role: WorldRole::Player,
        };
        let out = filter_command(
            &cmd,
            &snapshot,
            &ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Update { changes, .. } = &out.ops[0] else {
            panic!("expected an Update op");
        };
        assert!(
            changes.iter().any(|c| c.path == "/system/a" && c.new.is_null()),
            "retraction must null the field THIS command hid: {changes:?}"
        );
        assert!(
            !changes.iter().any(|c| c.path == "/system/b"),
            "retraction must not touch a field this command never hid: {changes:?}"
        );
    }

    #[test]
    fn retraction_does_not_null_the_owners_own_owner_or_gm_fields() {
        // (b) The SAME retracting command, replayed to the document's own OWNER — the owner's
        // legitimately-visible OwnerOrGm fields must NOT be nulled by retraction.
        let world = Uuid::from_u128(9);
        let author = Uuid::from_u128(1);
        let doc_id = Uuid::from_u128(2);
        let owner = Uuid::from_u128(3);
        let cmd = Command {
            seq: 1,
            world_id: world,
            author,
            ts: 0,
            ops: vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/permissions/property_overrides/~1system~1name".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!("owner_or_gm"),
                }],
            }],
        };
        let mut op = op_snapshot_update(Some(owner), vec![], permissions_default_observer());
        op.retraction_hidden_at_commit =
            Some(vec![("/system/name".to_string(), Visibility::OwnerOrGm)]);
        let snapshot = snapshot_one_op(op, HashMap::new());
        let mut cur = doc(permissions_default_observer(), serde_json::json!({ "name": "PC" }));
        cur.owner = Some(owner);
        let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
        let ctx = PermissionContext {
            user_id: owner,
            world_role: WorldRole::Player,
        };
        let out = filter_command(
            &cmd,
            &snapshot,
            &ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Update { changes, .. } = &out.ops[0] else {
            panic!("expected an Update op");
        };
        assert!(
            !changes.iter().any(|c| c.path == "/system/name"),
            "the owner's own OwnerOrGm field must not be retracted: {changes:?}"
        );
    }

    #[test]
    fn multi_op_leak_within_one_command_is_closed_by_the_post_loop_accumulator() {
        // Within ONE command, an Update that sets a secret value followed by an Update that
        // adds a gm_only override on the SAME pointer must have BOTH ops' snapshots reflect the
        // FINAL (post-loop) override tree: BOTH ops' `overrides_at_commit` carry the gm_only
        // override here, even though only the SECOND op is the one that added it. A snapshot
        // built from each op's own per-iteration local state instead of the whole command's
        // final post-image would leave the FIRST op's `overrides_at_commit` empty, and this
        // test would then fail (the secret would leak to a non-GM/non-owner recipient on the
        // first op).
        let world = Uuid::from_u128(9);
        let author = Uuid::from_u128(1);
        let doc_id = Uuid::from_u128(2);
        let recipient = Uuid::from_u128(3);
        let cmd = Command {
            seq: 1,
            world_id: world,
            author,
            ts: 0,
            ops: vec![
                Operation::Update {
                    doc_id,
                    changes: vec![FieldChange {
                        remove: false,
                        path: "/system/secret".into(),
                        old: serde_json::Value::Null,
                        new: serde_json::json!("X"),
                    }],
                },
                Operation::Update {
                    doc_id,
                    changes: vec![FieldChange {
                        remove: false,
                        path: "/permissions/property_overrides/~1system~1secret".into(),
                        old: serde_json::Value::Null,
                        new: serde_json::json!("gm_only"),
                    }],
                },
            ],
        };
        let final_overrides = vec![("/system/secret", Visibility::GmOnly)];
        let op0 = op_snapshot_update(None, final_overrides.clone(), permissions_default_observer());
        let op1 = op_snapshot_update(None, final_overrides, permissions_default_observer());
        let snapshot = CommandSnapshot {
            per_op: vec![Some(op0), Some(op1)],
            world_gm_at_commit: HashMap::new(),
        };
        let cur = doc(
            permissions_default_observer(),
            serde_json::json!({ "secret": "X" }),
        );
        let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
        let ctx = PermissionContext {
            user_id: recipient,
            world_role: WorldRole::Player,
        };
        let out = filter_command(
            &cmd,
            &snapshot,
            &ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Update { changes, .. } = &out.ops[0] else {
            panic!("expected an Update op for the FIRST op");
        };
        assert!(
            changes.is_empty(),
            "the first op's own snapshot must already reflect the LATER op's override: {changes:?}"
        );
    }

    #[test]
    fn behavioural_mutation_current_output_unaffected_by_history_commit_output_unaffected_by_live() {
        // Mutate each live input independently — the target's overrides, its default, the
        // linked actor's owner, an embedded child's index, the recipient's world role — and
        // assert `filter_command`'s CURRENT-time output is unaffected by history and its
        // COMMIT-time output is unaffected by anything live.
        let world = Uuid::from_u128(9);
        let author = Uuid::from_u128(1);
        let doc_id = Uuid::from_u128(2);
        let recipient = Uuid::from_u128(3);
        let cmd = field_change_update_cmd(
            world,
            author,
            doc_id,
            "/system/x",
            serde_json::json!(1),
            serde_json::json!(2),
        );
        // Baseline: nothing hidden at commit, nothing hidden currently.
        let op = op_snapshot_update(None, vec![], permissions_default_observer());
        let snapshot = snapshot_one_op(op, HashMap::from([(recipient, false)]));
        let cur = doc(permissions_default_observer(), serde_json::json!({ "x": 2 }));
        let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
        let ctx = PermissionContext { user_id: recipient, world_role: WorldRole::Player };
        let baseline = filter_command(
            &cmd, &snapshot, &ctx, &WorldCapDefaults::default(), &current, |_| None,
        );
        let Operation::Update { changes, .. } = &baseline.ops[0] else { panic!("expected Update") };
        assert_eq!(changes.len(), 1, "baseline: field visible to everyone");

        // Mutate ONLY the live default (current-time) — commit-time snapshot untouched.
        let denied_current = PermissionSet { default: DocRole::None, ..Default::default() };
        let mut cur2 = doc(denied_current, serde_json::json!({ "x": 2 }));
        cur2.owner = None;
        let current2 = HashMap::from([(doc_id, current_doc(cur2, 0))]);
        let out_live_mutated = filter_command(
            &cmd, &snapshot, &ctx, &WorldCapDefaults::default(), &current2, |_| None,
        );
        assert!(
            out_live_mutated.ops.is_empty(),
            "mutating ONLY the live default must change the CURRENT-time gate outcome"
        );

        // Mutate ONLY the commit-time permissions (recipient denied at commit) — live unchanged.
        let mut op_denied_commit =
            op_snapshot_update(None, vec![], PermissionSet { default: DocRole::None, ..Default::default() });
        op_denied_commit.doc_type = "actor".into();
        let snapshot_denied_commit =
            snapshot_one_op(op_denied_commit, HashMap::from([(recipient, false)]));
        let out_commit_mutated = filter_command(
            &cmd, &snapshot_denied_commit, &ctx, &WorldCapDefaults::default(), &current, |_| None,
        );
        assert!(
            out_commit_mutated.ops.is_empty(),
            "mutating ONLY the commit-time permissions must change the COMMIT-time gate outcome"
        );
    }

    #[test]
    fn embedded_child_index_in_the_commit_time_snapshot_is_independent_of_the_current_embedded_array() {
        // The commit-time override set is a flat, ALREADY-ADDRESSED pointer list
        // (`OpSnapshot::overrides_at_commit`), never re-derived from the CURRENT document's
        // embedded array. Mutating ONLY the current embedded structure (here: inserting
        // siblings so the commit-time secret child now sits at a different position) must not
        // change what the commit-time half redacts at a pointer the snapshot already names.
        let world = Uuid::from_u128(9);
        let author = Uuid::from_u128(1);
        let doc_id = Uuid::from_u128(2);
        let recipient = Uuid::from_u128(3);
        let cmd = Command {
            seq: 1,
            world_id: world,
            author,
            ts: 0,
            ops: vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/embedded/actor/1/system/name".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!("Hidden At Commit"),
                }],
            }],
        };
        let op = op_snapshot_update(
            None,
            vec![("/embedded/actor/1/system/name", Visibility::GmOnly)],
            permissions_default_observer(),
        );
        let snapshot = snapshot_one_op(op, HashMap::from([(recipient, false)]));
        // CURRENT structure: THREE children under "actor" — none carries the override (the
        // override lives only in the snapshot), so hidden_current alone would NOT redact this
        // pointer; only hidden_commit does.
        let mut cur = doc(permissions_default_observer(), serde_json::json!({}));
        cur.embedded.insert(
            "actor".into(),
            vec![
                doc(permissions_default_observer(), serde_json::json!({})),
                doc(permissions_default_observer(), serde_json::json!({})),
                doc(permissions_default_observer(), serde_json::json!({})),
            ],
        );
        let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
        let ctx = PermissionContext { user_id: recipient, world_role: WorldRole::Player };
        let out = filter_command(
            &cmd, &snapshot, &ctx, &WorldCapDefaults::default(), &current, |_| None,
        );
        let Operation::Update { changes, .. } = &out.ops[0] else { panic!("expected Update") };
        assert!(
            changes.is_empty(),
            "the commit-time snapshot's own recorded pointer must still redact, regardless of \
             what the CURRENT embedded array now holds at that index: {changes:?}"
        );
    }

    #[test]
    fn linked_token_actor_owner_mutation_only_affects_the_current_time_half() {
        // `effective_owner_via` joins the CURRENT actor table via the caller-supplied closure;
        // mutating what it returns changes ONLY the current-time half's ownership resolution,
        // never the commit-time half's (`OpSnapshot::owner_at_commit`, frozen at commit).
        let world = Uuid::from_u128(9);
        let author = Uuid::from_u128(1);
        let token_id = Uuid::from_u128(2);
        let actor_id = Uuid::from_u128(50);
        let recipient = Uuid::from_u128(3);
        let mut perms = permissions_default_observer();
        perms
            .property_overrides
            .insert("/system/name".into(), Visibility::OwnerOrGm);
        let cmd = Command {
            seq: 1,
            world_id: world,
            author,
            ts: 0,
            ops: vec![Operation::Update {
                doc_id: token_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/name".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!("Owner-visible name"),
                }],
            }],
        };
        // Commit-time: the recipient WAS the effective owner at commit (owner_at_commit is
        // Some(recipient)) — so the commit-time half admits this pointer regardless of what
        // the CURRENT actor_lookup later resolves. Isolates the mutation to the current-time
        // half only: union semantics mean a commit-time admission is never overridden into a
        // reveal, but a current-time denial always adds further hiding on top of it.
        let mut op = op_snapshot_update(
            Some(recipient),
            vec![("/system/name", Visibility::OwnerOrGm)],
            perms.clone(),
        );
        op.doc_type = "token".into();
        let snapshot = snapshot_one_op(op, HashMap::new());
        let mut token_doc = doc(perms, serde_json::json!({ "name": "Token PC" }));
        token_doc.doc_type = "token".into();
        token_doc.engine = Some(serde_json::json!({ "actor_id": actor_id.to_string() }));
        let current = HashMap::from([(token_id, current_doc(token_doc, 0))]);
        let ctx = PermissionContext { user_id: recipient, world_role: WorldRole::Player };

        // actor_lookup resolves the recipient as the CURRENT linked actor's owner.
        let mut owning_actor = doc(PermissionSet::default(), serde_json::json!({}));
        owning_actor.id = actor_id;
        owning_actor.doc_type = "actor".into();
        owning_actor.owner = Some(recipient);
        let out_owner_now = filter_command(
            &cmd,
            &snapshot,
            &ctx,
            &WorldCapDefaults::default(),
            &current,
            |id| if *id == actor_id { Some(&owning_actor) } else { None },
        );
        let Operation::Update { changes, .. } = &out_owner_now.ops[0] else {
            panic!("expected Update")
        };
        assert_eq!(
            changes.len(),
            1,
            "current-time ownership (via the actor_lookup closure) must admit OwnerOrGm now: {changes:?}"
        );

        // Same command, same snapshot — actor_lookup now resolves NO owner (mutate ONLY the
        // live input). The commit-time half still admits the pointer (unchanged), but the
        // current-time half now denies it, and denial from EITHER half hides a pointer.
        let out_no_owner = filter_command(
            &cmd, &snapshot, &ctx, &WorldCapDefaults::default(), &current, |_| None,
        );
        let Operation::Update { changes, .. } = &out_no_owner.ops[0] else {
            panic!("expected Update")
        };
        assert!(
            changes.is_empty(),
            "mutating ONLY the actor_lookup closure must change the CURRENT-time outcome: {changes:?}"
        );
    }

    #[test]
    fn traversal_split_produces_byte_identical_output_for_the_same_document() {
        // The shared `(doc, prefix) -> Vec<(String, Visibility)>` traversal used by both the
        // live path (`collect_hidden`, via `hidden_from_overrides`) and snapshot construction
        // must be exactly the traversal `collect_overrides` performs — pinned here so a future
        // change to one cannot silently diverge from the other.
        let child = doc(
            perms_with(&[("/system/name", Visibility::OwnerOrGm)]),
            serde_json::json!({ "name": "Hidden" }),
        );
        let mut parent = doc(
            perms_with(&[("/system/secret", Visibility::GmOnly)]),
            serde_json::json!({ "secret": "S" }),
        );
        parent.embedded.insert("actor".into(), vec![child]);

        let mut overrides = Vec::new();
        collect_overrides(&parent, "", &mut overrides).unwrap();
        let pointers: std::collections::BTreeSet<&str> =
            overrides.iter().map(|(p, _)| p.as_str()).collect();
        assert!(pointers.contains("/system/secret"));
        assert!(pointers.contains("/base"));
        assert!(pointers.contains("/embedded/actor/0/system/name"));
        assert!(pointers.contains("/embedded/actor/0/base"));

        // hidden_from_overrides + collect_overrides together must reproduce collect_hidden's
        // own output exactly, for the same (doc, access).
        let mut via_collect_hidden = Vec::new();
        collect_hidden(&parent, &non_gm(), "", &mut via_collect_hidden).unwrap();
        let via_split = hidden_from_overrides(&overrides, &non_gm());
        let mut a: Vec<&str> = via_collect_hidden.iter().map(String::as_str).collect();
        let mut b: Vec<&str> = via_split.iter().map(String::as_str).collect();
        a.sort();
        b.sort();
        assert_eq!(a, b, "collect_hidden must equal collect_overrides + hidden_from_overrides");
    }
```

- [ ] **Step 4: Run the new tests to verify they fail to compile** (the production code they
  depend on — `CurrentDoc`, `load_current_docs`, `filter_command`'s new signature,
  `collect_overrides`, `hidden_from_overrides` — does not exist yet)

Run: `cargo test --manifest-path src/server/Cargo.toml data::permission -- --list`
Expected: compile error (`cannot find type CurrentDoc`, `cannot find function collect_overrides`,
etc.).

- [ ] **Step 5: Replace `collect_hidden` with the shared traversal + filtering split**

In `src/server/src/data/permission.rs`, replace the entire existing `collect_hidden` function
(currently at lines 921-954) with:

```rust
/// Collect every `(absolute_pointer, tier)` pair in `doc`'s own `property_overrides`, plus the
/// hardcoded `/base` `OwnerOrGm` entry (see `filter_properties`'s doc comment), recursing into
/// embedded descendants (parent-absolute addressing: a child at `embedded[key][i]` contributes
/// `/embedded/<key>/<i>{pointer}` — the SAME positional addressing `filter_properties`'s own
/// recursion uses). Access-independent: every override regardless of tier, so ONE traversal
/// feeds BOTH the live redaction path (`collect_hidden`, via `hidden_from_overrides`) and
/// commit-time snapshot construction (`OpSnapshot::overrides_at_commit`) — they cannot diverge
/// on how an embedded index is addressed because they share this one walk.
///
/// Classifies every REAL override pointer via `redaction_target` at traversal time (not lazily,
/// unlike a per-recipient filter would): safe because every document reaching this function has
/// already passed `validation::validate_property_overrides` at its OWN write time (both
/// `apply_command` and `apply_intent` call it on the full post-image, recursing into every
/// embedded descendant, before any document reaches storage) — an unclassifiable REAL override
/// pointer cannot exist in persisted data. Still returns `Result` to fail closed on
/// pre-validation legacy/hand-seeded data. The synthetic `/base` entry is never classified (it
/// is hardcoded, not user-supplied — mirrors the un-classified unconditional `/base` push this
/// function replaces).
pub(crate) fn collect_overrides(
    doc: &Document,
    prefix: &str,
    out: &mut Vec<(String, Visibility)>,
) -> Result<(), RedactionError> {
    for (p, v) in &doc.permissions.property_overrides {
        if redaction_target(p).is_none() {
            return Err(RedactionError { pointer: p.clone() });
        }
        out.push((format!("{prefix}{p}"), *v));
    }
    // Mirrors `filter_properties`' hardcoded `OwnerOrGm` policy for `/base` — see that
    // function's comment. Fires at every embedded depth too (each recursive call gets its own
    // `prefix`), covering an embedded child's own `base` the same way.
    out.push((format!("{prefix}/base"), Visibility::OwnerOrGm));
    for (key, children) in &doc.embedded {
        for (idx, child) in children.iter().enumerate() {
            collect_overrides(child, &format!("{prefix}/embedded/{key}/{idx}"), out)?;
        }
    }
    Ok(())
}

/// Filter `overrides` (as produced by `collect_overrides`) down to the absolute pointers
/// `access` may NOT see — the `can_see`-filtering half of the traversal split.
fn hidden_from_overrides(overrides: &[(String, Visibility)], access: &Access) -> Vec<String> {
    overrides
        .iter()
        .filter(|(_, v)| !access.can_see(*v))
        .map(|(p, _)| p.clone())
        .collect()
}

/// Lets `Update`-delta redaction honor hidden fields at any embedded depth — the same coverage
/// `filter_properties` gives whole-document egress. A thin wrapper: `collect_overrides` performs
/// the traversal (and classification), `hidden_from_overrides` performs the `can_see` filter —
/// kept as one function because this is `filter_command`'s ONE call site's exact shape.
fn collect_hidden(
    doc: &Document,
    access: &Access,
    prefix: &str,
    out: &mut Vec<String>,
) -> Result<(), RedactionError> {
    let mut overrides = Vec::new();
    collect_overrides(doc, prefix, &mut overrides)?;
    out.extend(hidden_from_overrides(&overrides, access));
    Ok(())
}
```

- [ ] **Step 6: Widen `touches_permissions` and `paths_overlap` to `pub(crate)`**

Change (currently at line 961):

```rust
fn touches_permissions(path: &str) -> bool {
```

to:

```rust
pub(crate) fn touches_permissions(path: &str) -> bool {
```

Change (currently at line 423):

```rust
fn paths_overlap(a: &str, b: &str) -> bool {
```

to:

```rust
pub(crate) fn paths_overlap(a: &str, b: &str) -> bool {
```

Add one line to each function's existing doc comment noting the cross-module consumer, e.g. for
`touches_permissions`: `/// Consumed cross-module by SqliteRepository::build_op_snapshot to decide
whether an Update op's commit-time snapshot needs a retraction set.` — and for `paths_overlap`:
`/// Consumed cross-module by SqliteRepository::build_op_snapshot to prune an Update op's
commit-time override set to its own changed-paths closure.`

- [ ] **Step 7: Replace `load_update_docs` with `CurrentDoc` + `load_current_docs`**

Replace the entire existing `load_update_docs` function (currently at lines 147-168) with:

```rust
/// A document's current state, as loaded for redaction: its live envelope plus its
/// `documents.created_seq` generation marker. The marker is compared against
/// `OpSnapshot::created_seq_at_commit` to detect a document id reused since a replayed
/// command's commit (the id was deleted and a new document created at the same id).
pub struct CurrentDoc {
    /// The document's current envelope.
    pub doc: Document,
    /// `documents.created_seq` — this id's current generation marker.
    pub created_seq: i64,
}

/// Current documents for every `Update`, `Create`, and `Delete` op in `cmd`, keyed by the op's
/// own doc_id (a `Create`'s newly-created id; an `Update`/`Delete`'s existing target). A missing
/// key means the document does not currently exist at that id; `filter_command` drops the
/// corresponding op. Widened from the `Update`-only `load_update_docs` this design supersedes:
/// the whole-document commit∧current access gate applies uniformly to all three op kinds (see
/// `filter_command`'s doc comment), so `Create`/`Delete` need a current-state read too, not just
/// `Update`. Hoisted out of the redaction core so it can be awaited ONCE, before any scene-guard
/// scope is entered — one pool read per distinct doc_id in `cmd`, per recipient (count-neutral
/// for `Update` vs. the function this replaces; new, and unavoidable, for `Create`/`Delete` — the
/// whole-document gate cannot be evaluated for them without a current-state read).
pub async fn load_current_docs(
    repo: &dyn Repository,
    cmd: &Command,
) -> std::collections::HashMap<Uuid, CurrentDoc> {
    let mut out = std::collections::HashMap::new();
    for op in &cmd.ops {
        let doc_id = match op {
            Operation::Update { doc_id, .. } => *doc_id,
            Operation::Create { doc } => doc.id,
            Operation::Delete { doc } => doc.id,
        };
        if !out.contains_key(&doc_id) {
            if let Ok(Some((doc, created_seq))) = repo.get_document_with_created_seq(doc_id).await
            {
                out.insert(doc_id, CurrentDoc { doc, created_seq });
            }
        }
    }
    out
}
```

- [ ] **Step 8: Rewrite `filter_command`**

Replace the entire existing `filter_command` function (currently at lines 976-1093) with:

```rust
/// The recipient's view of a broadcast command: ops on unreadable documents are dropped,
/// GmOnly/OwnerOrGm properties/changes stripped. seq/world/author/ts are preserved so the
/// recipient's sequence guard never sees a false gap — a fully redacted command keeps its seq
/// with empty ops.
///
/// Redaction is the CONJUNCTION of two views: what was permitted at commit (`snapshot`) and what
/// is permitted now (`current`) — never fewer checks than either view alone would apply. A
/// pointer is redacted iff it was hidden at commit OR is hidden now; a whole op is dropped
/// unless BOTH the commit-time and current-time whole-document `cap::READ` gate admit it — this
/// asymmetry-closing gate applies uniformly to `Create`/`Update`/`Delete`, not just `Update`
/// (a recipient denied at a document's Create commit-time but currently permitted must not see
/// a LATER Update to the same doc_id either, or they receive field-level data for a document
/// they were never told exists).
///
/// `effective_owner_via` is joined through a caller-supplied in-memory actor source, so this
/// never queries the pool for the CURRENT-time half. The loads this needs (`current`, from
/// `load_current_docs`) are hoisted and awaited by the caller BEFORE calling in. The commit-time
/// half never queries anything live — it is fully derived from `snapshot`, by construction (no
/// live-state parameter exists on this function to reintroduce one from).
pub fn filter_command<'a>(
    cmd: &Command,
    snapshot: &CommandSnapshot,
    ctx: &PermissionContext,
    world_defaults: &WorldCapDefaults,
    current: &HashMap<Uuid, CurrentDoc>,
    actor_lookup: impl Fn(&Uuid) -> Option<&'a Document>,
) -> Command {
    let mut out_ops = Vec::with_capacity(cmd.ops.len());
    for (idx, op) in cmd.ops.iter().enumerate() {
        // Back-compat: a `None` snapshot (a pre-fix `world_events` row) drops the op on replay
        // rather than falling back to a live-lookup redaction.
        let Some(op_snapshot) = snapshot.per_op.get(idx).and_then(|s| s.as_ref()) else {
            continue;
        };
        let gm_at_commit = snapshot
            .world_gm_at_commit
            .get(&ctx.user_id)
            .copied()
            .unwrap_or(false);
        let world_role_commit = if gm_at_commit {
            WorldRole::Gm
        } else {
            WorldRole::Player
        };
        match op {
            Operation::Create { doc } => {
                let access_commit =
                    resolve_access(ctx.user_id, world_role_commit, doc, op_snapshot.owner_at_commit);
                let owner_current = effective_owner_via(doc, &actor_lookup);
                let access_current = resolve_access_world(
                    ctx.user_id,
                    ctx.world_role,
                    doc,
                    &world_defaults.grants_for(&doc.doc_type),
                    owner_current,
                );
                if !access_commit.has(cap::READ) || !access_current.has(cap::READ) {
                    continue;
                }
                match filter_properties(doc, &access_current) {
                    Ok(filtered) => out_ops.push(Operation::Create { doc: filtered }),
                    Err(e) => {
                        tracing::warn!(doc_id = %doc.id, error = %e, "redaction failed; dropping Create op for recipient");
                    }
                }
            }
            Operation::Delete { doc } => {
                // Existence check is INVERTED vs Update: a Delete's current doc is EXPECTED to
                // be absent (that is the point of the op). The created_seq mismatch check
                // applies only when a current doc DOES exist (the id was reused).
                if let Some(commit_seq) = op_snapshot.created_seq_at_commit {
                    if let Some(cur) = current.get(&doc.id) {
                        if cur.created_seq != commit_seq {
                            continue;
                        }
                    }
                }
                let access_commit =
                    resolve_access(ctx.user_id, world_role_commit, doc, op_snapshot.owner_at_commit);
                let owner_current = effective_owner_via(doc, &actor_lookup);
                let access_current = resolve_access_world(
                    ctx.user_id,
                    ctx.world_role,
                    doc,
                    &world_defaults.grants_for(&doc.doc_type),
                    owner_current,
                );
                if !access_commit.has(cap::READ) || !access_current.has(cap::READ) {
                    continue;
                }
                match filter_properties(doc, &access_current) {
                    Ok(filtered) => out_ops.push(Operation::Delete { doc: filtered }),
                    Err(e) => {
                        tracing::warn!(doc_id = %doc.id, error = %e, "redaction failed; dropping Delete op for recipient");
                    }
                }
            }
            Operation::Update { doc_id, changes } => {
                // Absent = does not currently exist → drop, preserving today's semantics.
                let Some(cur) = current.get(doc_id) else {
                    continue;
                };
                if let Some(commit_seq) = op_snapshot.created_seq_at_commit {
                    if cur.created_seq != commit_seq {
                        continue;
                    }
                }
                let commit_doc = Document {
                    doc_type: op_snapshot.doc_type.clone(),
                    permissions: op_snapshot
                        .permissions_at_commit
                        .clone()
                        .unwrap_or_default(),
                    ..cur.doc.clone()
                };
                let access_commit = resolve_access(
                    ctx.user_id,
                    world_role_commit,
                    &commit_doc,
                    op_snapshot.owner_at_commit,
                );
                let owner_current = effective_owner_via(&cur.doc, &actor_lookup);
                let access_current = resolve_access_world(
                    ctx.user_id,
                    ctx.world_role,
                    &cur.doc,
                    &world_defaults.grants_for(&cur.doc.doc_type),
                    owner_current,
                );
                if !access_commit.has(cap::READ) || !access_current.has(cap::READ) {
                    continue;
                }
                let kept: Vec<FieldChange> = if access_current.see_gm_only
                    && access_commit.see_gm_only
                {
                    changes.clone()
                } else {
                    let mut hidden_current = Vec::new();
                    if let Err(e) = collect_hidden(&cur.doc, &access_current, "", &mut hidden_current)
                    {
                        tracing::warn!(doc_id = %doc_id, error = %e, "redaction failed; dropping Update op for recipient");
                        continue;
                    }
                    let hidden_commit =
                        hidden_from_overrides(&op_snapshot.overrides_at_commit, &access_commit);
                    let mut hidden = hidden_current;
                    hidden.extend(hidden_commit);
                    hidden.sort();
                    hidden.dedup();
                    let mut kept: Vec<FieldChange> = changes
                        .iter()
                        .filter_map(|ch| redact_change(ch, &hidden))
                        .collect();
                    // Retraction: use this command's OWN commit-time hidden set, filtered
                    // through THIS recipient's commit-time access only — never the union, and
                    // never whatever is live now. Each retracting command owns its own
                    // retraction moment.
                    if changes.iter().any(|c| touches_permissions(&c.path)) {
                        if let Some(retraction) = &op_snapshot.retraction_hidden_at_commit {
                            for ptr in hidden_from_overrides(retraction, &access_commit) {
                                kept.push(FieldChange {
                                    remove: false,
                                    path: ptr,
                                    old: serde_json::Value::Null,
                                    new: serde_json::Value::Null,
                                });
                            }
                        }
                    }
                    kept
                };
                out_ops.push(Operation::Update {
                    doc_id: *doc_id,
                    changes: kept,
                });
            }
        }
    }
    Command {
        seq: cmd.seq,
        world_id: cmd.world_id,
        author: cmd.author,
        ts: cmd.ts,
        ops: out_ops,
    }
}
```

- [ ] **Step 9: Mechanically fix every pre-existing `filter_command`/`load_update_docs` call site
  in this file's own test module**

There are ~40 such call sites (all inside `#[cfg(test)] mod tests`, spanning roughly lines
1868-3760 before this task's edits). Apply exactly these two mechanical transformations,
in order, to every one:

**(a) Rename + retype every `load_update_docs` call**, and change what it's assigned from
`HashMap<Uuid, Document>`-shaped usage to `HashMap<Uuid, CurrentDoc>`-shaped usage. Before:

```rust
let current = load_update_docs(&r, &cmd).await;
```

After (identical — `load_current_docs` is a drop-in rename; the VALUE type changed, so any
downstream code in the SAME test that reads a field off a `current.get(&id)` result must add
`.doc` — see (c) below):

```rust
let current = load_current_docs(&r, &cmd).await;
```

**(b) Every `filter_command(&cmd, ...)` call needs a `snapshot` argument inserted as the SECOND
positional argument.** Since every one of these pre-existing tests writes exactly ONE command via
`r.apply_intent(...)` before calling `filter_command`, and Task 3 makes `apply_intent` return
`StoredCommand` (which carries `.command` AND `.snapshot` together), the correct, minimal-diff fix
is to capture BOTH from the SAME `apply_intent` call the test already makes, rather than
constructing a snapshot by hand. Before (representative pattern — the exact preceding lines vary
per test, but every one of these ~40 sites follows this shape: an earlier `r.apply_intent(...)`
whose result is bound to `let cmd = ...` or discarded and re-fetched via `load_update_docs`):

```rust
let cmd = r
    .apply_intent(&gm_ctx, w.id, vec![Operation::Update { doc_id, changes }], ts, WriteOrigin::Client)
    .await
    .unwrap();
let current = load_update_docs(&r, &cmd).await;
let out = filter_command(&cmd, &ctx, &WorldCapDefaults::default(), &current, lookup);
```

After:

```rust
let stored = r
    .apply_intent(&gm_ctx, w.id, vec![Operation::Update { doc_id, changes }], ts, WriteOrigin::Client)
    .await
    .unwrap();
let cmd = &stored.command;
let snapshot = &stored.snapshot;
let current = load_current_docs(&r, cmd).await;
let out = filter_command(cmd, snapshot, &ctx, &WorldCapDefaults::default(), &current, lookup);
```

For a test whose PRECEDING setup call (the write establishing the document, before the write
under test) also needs no snapshot at all (i.e. the setup call's own `StoredCommand` is
irrelevant, only the SUBJECT command's snapshot matters) — the SAME `let stored = ...; let cmd =
&stored.command;` pattern applies at the LAST `apply_intent`/`apply_command` call before
`filter_command` runs; every EARLIER setup write in the same test may keep discarding its return
value (`.await.unwrap();` with no binding) exactly as it does today — Task 3 does not change
whether a discarded return value compiles.

**(c) Any place that reads a field off a `current.get(&id)`/`current[&id]` result directly (e.g.
`current.get(&doc_id).unwrap().owner`) needs `.doc` inserted**: `current.get(&doc_id).unwrap().doc.owner`.

Run: `cargo build --tests --manifest-path src/server/Cargo.toml` after each file-section fix to
get the compiler's own exhaustive, authoritative list of remaining call sites — apply (a)/(b)/(c)
above at each reported location until it exits 0. **Every pre-existing test's ASSERTIONS must
remain unchanged** (only the call SHAPE changes) — for a single-write-then-immediately-redact
test with no permission change between commit and now, `hidden_commit` and `hidden_current` are
identical, so the union changes nothing observable. If any pre-existing assertion genuinely needs
to change to pass, STOP and treat that as a signal to re-derive the fix rather than adjust the
assertion — it means the mechanical fix above was misapplied at that site, not that the test's
original expectation was wrong.

- [ ] **Step 10: Run the full permission.rs test suite**

Run: `cargo test --manifest-path src/server/Cargo.toml data::permission`
Expected: every test (pre-existing and new) passes. This includes the 9 new tests from Step 3.

- [ ] **Step 11: Run the full per-task CI gate battery** (same five commands as Task 1 Step 15)

- [ ] **Step 12: Commit**

```bash
git add src/server/src/data/permission.rs
git commit -m "feat(data): redact replay against commit-time AND current-time policy"
```

---

## Task 3: `apply_command` + `apply_intent` + `events_since` — build and persist the snapshot

**Files:**
- Modify: `src/server/src/data/repository.rs` — `apply_command`, `apply_intent`, `events_since`
  return types
- Modify: `src/server/src/data/sqlite.rs` — `apply_command`, `apply_intent`, `events_since`
  bodies; new `build_op_snapshot` helper
- Modify: `src/server/src/ws/room.rs:1223-1353` (`DeleteMidHydration` — three more method
  signatures)
- Modify: **~138 pre-existing call sites** of `.apply_command(`/`.apply_intent(`/`.events_since(`
  across `src/server/src/data/sqlite.rs` (own test module), `src/server/src/chat/mod.rs`,
  `src/server/src/ws/room.rs` (already handled for the two production call sites in Task 4 — this
  task only touches its OWN test-module call sites, see Step 8), and `src/server/src/bin/test_server.rs`
- Test: `src/server/src/data/sqlite.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `crate::data::snapshot::{StoredCommand, CommandSnapshot, OpSnapshot}` (Task 1);
  `SqliteRepository::{document_created_seq, world_member_roles}` (Task 1); `permission::
  {collect_overrides, touches_permissions, paths_overlap}` (Task 2).
- Produces: `Repository::apply_command(&self, cmd: UnsequencedCommand) -> Result<StoredCommand,
  DataError>`; `Repository::apply_intent(...) -> Result<StoredCommand, DataError>`;
  `Repository::events_since(&self, world_id: Uuid, seq: i64) -> Result<Vec<StoredCommand>,
  DataError>`.
- Produces (private): `SqliteRepository::build_op_snapshot(tx: &mut sqlx::SqliteConnection, op:
  &Operation, post_images: &HashMap<Uuid, Document>, deleted_created_seqs: &HashMap<Uuid, i64>) ->
  Result<OpSnapshot, DataError>` — shared by both write loops.
- Consumed by Task 4: `Repository::apply_intent`'s and `events_since`'s new return types.

- [ ] **Step 1: Read `apply_command` and `apply_intent` in full again immediately before
  editing** (`src/server/src/data/sqlite.rs`, currently spanning roughly lines 1877-2523) — this
  task rewrites their persistence tail; stale line numbers are a real risk given Task 1/2 already
  touched this file.

- [ ] **Step 2: Change the `Repository` trait's three signatures**

In `src/server/src/data/repository.rs`:

```rust
    async fn apply_command(
        &self,
        cmd: UnsequencedCommand,
    ) -> Result<crate::data::snapshot::StoredCommand, DataError>;
```

```rust
    async fn apply_intent(
        &self,
        ctx: &crate::data::membership::PermissionContext,
        world_id: Uuid,
        ops: Vec<crate::data::command::Operation>,
        ts: i64,
        origin: crate::data::command::WriteOrigin,
    ) -> Result<crate::data::snapshot::StoredCommand, DataError>;
```

```rust
    async fn events_since(
        &self,
        world_id: Uuid,
        seq: i64,
    ) -> Result<Vec<crate::data::snapshot::StoredCommand>, DataError>;
```

Keep every existing doc comment on these three methods (only the return type changes); append one
sentence to each noting the snapshot: for `apply_command`/`apply_intent`, `"Returns the
commit-time redaction snapshot alongside the command — see StoredCommand."`; for `events_since`,
`"Each row's StoredCommand back-compat-parses a pre-fix bare-Command row via
StoredCommand::from_stored_json, carrying an all-None snapshot."`

- [ ] **Step 3: Add `build_op_snapshot` to `SqliteRepository`**

In `src/server/src/data/sqlite.rs`, add this new private method to `impl SqliteRepository`
(place it directly after `document_created_seq`/`world_member_roles`, which Task 1 added):

```rust
    /// Build one op's commit-time redaction snapshot from the command's FINAL post-image state
    /// (`post_images`, accumulated across the WHOLE mutation loop) and, for a `Delete`, its
    /// created_seq captured BEFORE the row was removed (`deleted_created_seqs` — the row is gone
    /// by the time this runs, so it cannot be read here). Runs on the caller's open transaction,
    /// after every op in the command has applied and every write has landed. Shared by
    /// `apply_command` and `apply_intent` — the ONE place either loop computes a snapshot, so
    /// they cannot diverge.
    async fn build_op_snapshot(
        tx: &mut sqlx::SqliteConnection,
        op: &Operation,
        post_images: &std::collections::HashMap<Uuid, Document>,
        deleted_created_seqs: &std::collections::HashMap<Uuid, i64>,
    ) -> Result<crate::data::snapshot::OpSnapshot, DataError> {
        use crate::data::snapshot::OpSnapshot;
        match op {
            Operation::Create { doc } => {
                let owner_at_commit = Self::load_effective_owner(&mut *tx, doc).await?;
                let mut overrides_at_commit = Vec::new();
                crate::data::permission::collect_overrides(doc, "", &mut overrides_at_commit)
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                Ok(OpSnapshot {
                    owner_at_commit,
                    doc_type: doc.doc_type.clone(),
                    overrides_at_commit,
                    retraction_hidden_at_commit: None,
                    created_seq_at_commit: None,
                    permissions_at_commit: None,
                })
            }
            Operation::Delete { doc } => {
                let owner_at_commit = Self::load_effective_owner(&mut *tx, doc).await?;
                let mut overrides_at_commit = Vec::new();
                crate::data::permission::collect_overrides(doc, "", &mut overrides_at_commit)
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                Ok(OpSnapshot {
                    owner_at_commit,
                    doc_type: doc.doc_type.clone(),
                    overrides_at_commit,
                    retraction_hidden_at_commit: None,
                    created_seq_at_commit: deleted_created_seqs.get(&doc.id).copied(),
                    permissions_at_commit: None,
                })
            }
            Operation::Update { doc_id, changes } => {
                let doc = post_images.get(doc_id).ok_or_else(|| {
                    DataError::OpFailed(format!("post-image missing for updated document {doc_id}"))
                })?;
                let owner_at_commit = Self::load_effective_owner(&mut *tx, doc).await?;
                let mut overrides_full = Vec::new();
                crate::data::permission::collect_overrides(doc, "", &mut overrides_full)
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                let touches_perms = changes
                    .iter()
                    .any(|c| crate::data::permission::touches_permissions(&c.path));
                let retraction_hidden_at_commit = if touches_perms {
                    Some(overrides_full.clone())
                } else {
                    None
                };
                // Pruned to the ancestor/descendant closure of this op's own changed paths —
                // only an overlapping override can possibly redact THIS op's field-level deltas.
                let overrides_at_commit: Vec<(String, crate::data::document::Visibility)> =
                    overrides_full
                        .into_iter()
                        .filter(|(p, _)| {
                            changes
                                .iter()
                                .any(|c| crate::data::permission::paths_overlap(p, &c.path))
                        })
                        .collect();
                let created_seq_at_commit = Self::document_created_seq(&mut *tx, *doc_id).await?;
                Ok(OpSnapshot {
                    owner_at_commit,
                    doc_type: doc.doc_type.clone(),
                    overrides_at_commit,
                    retraction_hidden_at_commit,
                    created_seq_at_commit,
                    permissions_at_commit: Some(crate::data::document::PermissionSet {
                        property_overrides: Default::default(),
                        ..doc.permissions.clone()
                    }),
                })
            }
        }
    }
```

- [ ] **Step 4: Rewrite `apply_command`'s persistence tail**

In `apply_command` (`src/server/src/data/sqlite.rs`), change the signature line:

```rust
    async fn apply_command(&self, cmd: UnsequencedCommand) -> Result<StoredCommand, DataError> {
```

(add `use crate::data::snapshot::{CommandSnapshot, StoredCommand};` to the file's import list at
the top if not already present via a glob).

Immediately before the existing `let mut normalized_ops = Vec::with_capacity(sequenced.ops.len());`
line, insert:

```rust
        let mut post_images: std::collections::HashMap<Uuid, Document> =
            std::collections::HashMap::new();
        let mut deleted_created_seqs: std::collections::HashMap<Uuid, i64> =
            std::collections::HashMap::new();
```

Inside the existing `for op in &sequenced.ops { match op { ... } }` loop that builds
`normalized_ops`, add exactly these three insertions (do not otherwise change the existing
validation/upsert/normalization logic in this loop):

- In the `Operation::Create { doc }` arm, immediately after `Self::upsert_document(&mut tx, &doc,
  seq).await?;` and before `normalized_ops.push(Operation::Create { doc });`, insert:
  `post_images.insert(doc.id, doc.clone());`
- In the `Operation::Delete { doc }` arm, immediately BEFORE the existing
  `Self::delete_document_tx(&mut tx, doc.id).await?;` line, insert:
  ```rust
                    if let Some(cs) = Self::document_created_seq(&mut *tx, doc.id).await? {
                        deleted_created_seqs.insert(doc.id, cs);
                    }
  ```
- In the `Operation::Update { doc_id, changes }` arm, immediately after `Self::upsert_document(&mut
  tx, &doc, seq).await?;` and before the `let normalized_doc_json = ...` line, insert:
  `post_images.insert(*doc_id, doc.clone());`

Immediately after the existing `sequenced.ops = normalized_ops;` line and BEFORE the existing
`sqlx::query("INSERT INTO world_events ...")` block, insert:

```rust
        let world_gm_at_commit: std::collections::HashMap<Uuid, bool> =
            Self::world_member_roles(&mut *tx, sequenced.world_id)
                .await?
                .into_iter()
                .map(|(uid, role)| (uid, role == WorldRole::Gm))
                .collect();
        let mut per_op = Vec::with_capacity(sequenced.ops.len());
        for op in &sequenced.ops {
            per_op.push(Some(
                Self::build_op_snapshot(&mut tx, op, &post_images, &deleted_created_seqs).await?,
            ));
        }
        let stored = StoredCommand {
            command: sequenced,
            snapshot: CommandSnapshot {
                per_op,
                world_gm_at_commit,
            },
        };
```

Then change the `INSERT INTO world_events` block's bindings and the function's final two lines.
Replace:

```rust
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
```

with:

```rust
        sqlx::query("INSERT INTO world_events (world_id, seq, author_id, ts, command_json) VALUES (?, ?, ?, ?, ?)")
            .bind(stored.command.world_id.to_string())
            .bind(seq)
            .bind(stored.command.author.to_string())
            .bind(stored.command.ts)
            .bind(serde_json::to_string(&stored)?)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(stored)
    }
```

- [ ] **Step 5: Rewrite `apply_intent`'s persistence tail**

In `apply_intent`'s signature line, change the return type:

```rust
    async fn apply_intent(
        &self,
        ctx: &crate::data::membership::PermissionContext,
        world_id: Uuid,
        mut ops: Vec<Operation>,
        ts: i64,
        origin: WriteOrigin,
    ) -> Result<StoredCommand, DataError> {
```

Apply the SAME three pattern of edits Step 4 made to `apply_command`'s Phase-2 loop (the one
building `normalized_ops` from `sequenced.ops`, currently starting at the comment "Rebuilt in
place of `sequenced.ops`"), to `apply_intent`'s Phase-2 loop:

- Insert `let mut post_images: std::collections::HashMap<Uuid, Document> =
  std::collections::HashMap::new(); let mut deleted_created_seqs: std::collections::HashMap<Uuid,
  i64> = std::collections::HashMap::new();` immediately before that loop.
- In its `Operation::Create { doc }` arm: after `Self::upsert_document(&mut tx, doc,
  seq).await?;`, insert `post_images.insert(doc.id, doc.clone());`.
- In its `Operation::Delete { doc }` arm: before `Self::delete_document_tx(&mut tx,
  doc.id).await?;`, insert:
  ```rust
                    if let Some(cs) = Self::document_created_seq(&mut *tx, doc.id).await? {
                        deleted_created_seqs.insert(doc.id, cs);
                    }
  ```
- In its `Operation::Update { doc_id, changes }` arm: after `Self::upsert_document(&mut tx, &doc,
  seq).await?;`, insert `post_images.insert(*doc_id, doc.clone());`.

Then apply the SAME snapshot-building + persistence-tail rewrite Step 4 made, using `world_id`
(the function's own parameter, already in scope) in place of `sequenced.world_id`:

```rust
        let world_gm_at_commit: std::collections::HashMap<Uuid, bool> =
            Self::world_member_roles(&mut *tx, world_id)
                .await?
                .into_iter()
                .map(|(uid, role)| (uid, role == WorldRole::Gm))
                .collect();
        let mut per_op = Vec::with_capacity(sequenced.ops.len());
        for op in &sequenced.ops {
            per_op.push(Some(
                Self::build_op_snapshot(&mut tx, op, &post_images, &deleted_created_seqs).await?,
            ));
        }
        let stored = StoredCommand {
            command: sequenced,
            snapshot: CommandSnapshot {
                per_op,
                world_gm_at_commit,
            },
        };
```

Replace the final `INSERT INTO world_events` block and return, exactly matching the pattern in
Step 4 (`stored.command.world_id`/`.author`/`.ts`, `serde_json::to_string(&stored)?`, `Ok(stored)`).

- [ ] **Step 6: Rewrite `events_since`**

Replace the entire `events_since` implementation (`src/server/src/data/sqlite.rs`, currently
lines 2648-2663) with:

```rust
    async fn events_since(&self, world_id: Uuid, seq: i64) -> Result<Vec<StoredCommand>, DataError> {
        let rows = sqlx::query(
            "SELECT command_json FROM world_events WHERE world_id = ? AND seq > ? ORDER BY seq",
        )
        .bind(world_id.to_string())
        .bind(seq)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                StoredCommand::from_stored_json(r.get::<String, _>("command_json").as_str())
                    .map_err(DataError::from)
            })
            .collect()
    }
```

- [ ] **Step 7: Delegate the three changed methods on `DeleteMidHydration`**

In `src/server/src/ws/room.rs`, inside `impl Repository for DeleteMidHydration<'_>`, change the
three method signatures' return types to match the trait (bodies are unchanged — they already
just delegate to `self.inner.<method>(...).await`, and the delegating call's OWN return type
follows the trait automatically once the surrounding `fn` signature matches):

```rust
        async fn apply_command(
            &self,
            cmd: crate::data::command::UnsequencedCommand,
        ) -> Result<crate::data::snapshot::StoredCommand, DataError> {
            self.inner.apply_command(cmd).await
        }
        async fn apply_intent(
            &self,
            ctx: &crate::data::membership::PermissionContext,
            world_id: Uuid,
            ops: Vec<Operation>,
            ts: i64,
            origin: WriteOrigin,
        ) -> Result<crate::data::snapshot::StoredCommand, DataError> {
            self.inner
                .apply_intent(ctx, world_id, ops, ts, origin)
                .await
        }
```

and:

```rust
        async fn events_since(
            &self,
            world_id: Uuid,
            seq: i64,
        ) -> Result<Vec<crate::data::snapshot::StoredCommand>, DataError> {
            self.inner.events_since(world_id, seq).await
        }
```

- [ ] **Step 8: Write the new integration/differential tests**

Add to `src/server/src/data/sqlite.rs`'s existing `#[cfg(test)] mod tests` block:

```rust
    #[tokio::test]
    async fn multi_op_command_snapshot_reflects_the_final_post_loop_state_for_every_op() {
        // The write-loop counterpart of permission.rs's
        // multi_op_leak_within_one_command_is_closed_by_the_post_loop_accumulator: proves
        // apply_intent's OWN snapshot construction (not a hand-built one) gives the FIRST op's
        // OpSnapshot the override the SECOND op in the SAME command adds.
        use crate::data::command::{FieldChange, Operation};
        use crate::data::document::{DocRole, PermissionSet, Scope, Visibility};
        use crate::data::membership::PermissionContext;

        let r = repo().await;
        let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();
        let ctx = PermissionContext { user_id: gm, world_role: WorldRole::Gm };
        let mut perms = PermissionSet::default();
        perms.users.insert(gm, DocRole::Owner);
        let mut d = tests_engine_doc(perms, "actor", serde_json::json!({ "secret": "X" }));
        d.scope = Scope::World { world_id: w.id };
        let doc_id = d.id;
        r.apply_intent(&ctx, w.id, vec![Operation::Create { doc: d }], 1, WriteOrigin::Client)
            .await
            .unwrap();

        let stored = r
            .apply_intent(
                &ctx,
                w.id,
                vec![
                    Operation::Update {
                        doc_id,
                        changes: vec![FieldChange {
                            remove: false,
                            path: "/system/secret".into(),
                            old: serde_json::json!("X"),
                            new: serde_json::json!("Y"),
                        }],
                    },
                    Operation::Update {
                        doc_id,
                        changes: vec![FieldChange {
                            remove: false,
                            path: "/permissions/property_overrides/~1system~1secret".into(),
                            old: serde_json::Value::Null,
                            new: serde_json::json!("gm_only"),
                        }],
                    },
                ],
                2,
                WriteOrigin::Client,
            )
            .await
            .unwrap();

        let op0 = stored.snapshot.per_op[0].as_ref().unwrap();
        assert!(
            op0.overrides_at_commit
                .iter()
                .any(|(p, v)| p == "/system/secret" && *v == Visibility::GmOnly),
            "the FIRST op's snapshot must already carry the override the SECOND op adds: {:?}",
            op0.overrides_at_commit
        );
    }

    #[tokio::test]
    async fn reused_id_gets_a_fresh_created_seq_and_the_stale_ops_own_snapshot_witnesses_the_old_one() {
        use crate::data::command::Operation;
        use crate::data::document::{DocRole, PermissionSet, Scope};
        use crate::data::membership::PermissionContext;

        let r = repo().await;
        let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();
        let ctx = PermissionContext { user_id: gm, world_role: WorldRole::Gm };
        let mut perms = PermissionSet::default();
        perms.users.insert(gm, DocRole::Owner);
        let reused_id = Uuid::new_v4();
        let mut d1 = tests_engine_doc(perms.clone(), "item", serde_json::json!({}));
        d1.id = reused_id;
        d1.scope = Scope::World { world_id: w.id };
        let stored_create1 = r
            .apply_intent(&ctx, w.id, vec![Operation::Create { doc: d1 }], 1, WriteOrigin::Client)
            .await
            .unwrap();
        let old_created_seq = stored_create1.command.seq;

        let old_doc = r.get_document(reused_id).await.unwrap().unwrap();
        r.apply_intent(&ctx, w.id, vec![Operation::Delete { doc: old_doc }], 2, WriteOrigin::Client)
            .await
            .unwrap();

        let mut d2 = tests_engine_doc(perms, "item", serde_json::json!({}));
        d2.id = reused_id;
        d2.scope = Scope::World { world_id: w.id };
        let stored_create2 = r
            .apply_intent(&ctx, w.id, vec![Operation::Create { doc: d2 }], 3, WriteOrigin::Client)
            .await
            .unwrap();
        let new_created_seq = stored_create2.command.seq;
        assert_ne!(old_created_seq, new_created_seq, "a reused id must get a FRESH created_seq");

        let (_, current_created_seq) = r
            .get_document_with_created_seq(reused_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current_created_seq, new_created_seq);
    }

    #[tokio::test]
    async fn events_since_back_compat_parses_a_pre_fix_bare_command_row() {
        use crate::data::command::{Command, Operation, UnsequencedCommand};

        let r = repo().await;
        let gm = r.create_user("gm", None, ServerRole::User, 0).await.unwrap();
        let w = r.create_world_owned("W", gm, 0).await.unwrap();

        // Simulate a pre-fix row: bump the world seq and insert a bare Command directly,
        // bypassing apply_command/apply_intent's StoredCommand-shaped persistence.
        let cmd = Command {
            seq: 1,
            world_id: w.id,
            author: gm,
            ts: 0,
            ops: vec![],
        };
        sqlx::query(
            "INSERT INTO world_events (world_id, seq, author_id, ts, command_json) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(cmd.world_id.to_string())
        .bind(cmd.seq)
        .bind(cmd.author.to_string())
        .bind(cmd.ts)
        .bind(serde_json::to_string(&cmd).unwrap())
        .execute(&r.pool)
        .await
        .unwrap();
        sqlx::query("UPDATE worlds SET seq = 1 WHERE id = ?")
            .bind(w.id.to_string())
            .execute(&r.pool)
            .await
            .unwrap();

        let replayed = r.events_since(w.id, 0).await.unwrap();
        assert_eq!(replayed.len(), 1);
        assert_eq!(replayed[0].command, cmd);
        assert!(replayed[0].snapshot.per_op.is_empty());
        assert!(replayed[0].snapshot.world_gm_at_commit.is_empty());
        // Ensure `UnsequencedCommand`'s type stays imported/used elsewhere in this module.
        let _ = UnsequencedCommand {
            world_id: w.id,
            author: gm,
            ts: 0,
            ops: vec![Operation::Create { doc: seed_actor_doc(Uuid::new_v4(), w.id, None) }],
        };
    }
```

The last two lines of `events_since_back_compat_parses_a_pre_fix_bare_command_row` (the unused
`UnsequencedCommand`/`seed_actor_doc` construction) exist only if this exact test module does not
already import `UnsequencedCommand`/have a `seed_actor_doc` helper in scope; check the surrounding
`use` statements before pasting and delete that trailing block if it would be a genuinely dead,
unused local (which would itself trip `cargo clippy -D warnings`) — keep only if it is needed to
silence an "unused import" for `UnsequencedCommand`, otherwise omit those two lines entirely.

- [ ] **Step 9: Mechanically fix every pre-existing call site of
  `.apply_command(`/`.apply_intent(`/`.events_since(`**

There are approximately 138 in `src/server/src/data/sqlite.rs`'s own test module, ~10 already
handled in `src/server/src/data/permission.rs` (Task 2 Step 9), ~11 in
`src/server/src/chat/mod.rs`, and 2 in `src/server/src/bin/test_server.rs`. Apply this rule at
every site, verified against representative examples read during plan-writing:

**No change needed** when the call's result is discarded as a bare statement
(`repo.apply_command(...).await.unwrap();` with no `let` binding) or matched via `.unwrap_err()`/
`.is_ok()`/`.is_err()`/`.err()` (these compile identically regardless of the `Ok` type) — verified
this covers `bin/test_server.rs`'s both call sites and the majority of `chat/mod.rs`'s.

**One-token fix** when the result IS bound and its `Command`-shaped fields (`.seq`, `.ops`,
`.author`, `.ts`, `.world_id`) are read afterward: append `.command` to the existing `.unwrap()`
(or equivalent) chain. Before:

```rust
let cmd = r.apply_intent(&ctx, w.id, ops, ts, WriteOrigin::Client).await.unwrap();
assert_eq!(cmd.seq, 1);
```

After:

```rust
let cmd = r.apply_intent(&ctx, w.id, ops, ts, WriteOrigin::Client).await.unwrap().command;
assert_eq!(cmd.seq, 1);
```

**`events_since` fix** when a bound `Vec<Command>` result is read as `Command` fields downstream
(rare — only `sqlite.rs`'s own `events_since_returns_the_suffix` test, currently at line 7209, is
known to do this from the file structure read during plan-writing; verify against the compiler).
Before:

```rust
let replayed = r.events_since(w.id, 1).await.unwrap();
assert_eq!(replayed[0].seq, 2);
```

After:

```rust
let replayed = r.events_since(w.id, 1).await.unwrap();
assert_eq!(replayed[0].command.seq, 2);
```

Run `cargo build --tests --manifest-path src/server/Cargo.toml` repeatedly, fixing every reported
site by applying whichever of the two rules above the diagnostic calls for, until it exits 0.
**Every pre-existing test's ASSERTIONS must remain unchanged** — same standard as Task 2 Step 9.

- [ ] **Step 10: Run the full data::sqlite test suite**

Run: `cargo test --manifest-path src/server/Cargo.toml data::sqlite`
Expected: every test (pre-existing and new) passes, including the 3 new tests from Step 8.

- [ ] **Step 11: Run the full chat::mod and data::repository test suites**

Run: `cargo test --manifest-path src/server/Cargo.toml chat::`
Run: `cargo build --tests --manifest-path src/server/Cargo.toml`
Expected: both succeed (the repository trait itself has no `#[cfg(test)]` module of its own to
run; the build step is the check that its signature change compiles everywhere).

- [ ] **Step 12: Run the full per-task CI gate battery** (same five commands as Task 1 Step 15)

- [ ] **Step 13: Commit**

```bash
git add src/server/src/data/repository.rs src/server/src/data/sqlite.rs \
  src/server/src/ws/room.rs src/server/src/chat/mod.rs src/server/src/bin/test_server.rs
git commit -m "feat(data): persist the commit-time redaction snapshot from both write loops"
```

---

## Task 4: Room/ring/broadcast plumbing + `ws/conn.rs` egress rewiring

**Files:**
- Modify: `src/server/src/ws/room.rs` — `RingBuffer` (element type), new `RoomEvent` enum, `Room`
  (`tx` field type, `subscribe`, `broadcast_aux`, `commit_ops_locked`, `resync_range`),
  `ring_tests` module's `event()` helper, `room_tests` module's `evict_user_reaches_every_room`
  and `commit_ops_writes_and_broadcasts_without_gating`
- Modify: `src/server/src/ws/conn.rs` — `Egress` enum (unchanged payload type, confirmed below),
  `send_filtered` split into `send_filtered_event` + `send_plain` + `send_room_event`,
  `egress_loop`'s broadcast-receive arm, `replay`
- Modify: `src/server/src/ws/conn.rs` test module — one test rewrite
  (`handle_move_request`-adjacent test reading `rx.recv()` and dereferencing `ServerMsg` directly)

**Interfaces:**
- Consumes: `StoredCommand`/`CommandSnapshot` (Task 1); `Repository::apply_intent`/`events_since`
  returning `StoredCommand`/`Vec<StoredCommand>` (Task 3); `permission::{load_current_docs,
  filter_command, CurrentDoc}` (Task 2).
- Produces (crate-visible, `ws/room.rs`): `pub(crate) enum RoomEvent { Event(Arc<StoredCommand>),
  Other(Arc<ServerMsg>) }` with inherent `event_seq(&self) -> Option<i64>` / `event_ts(&self) ->
  Option<i64>`.
- Changes `Room::subscribe(&self) -> (broadcast::Receiver<RoomEvent>, i64)` (was
  `broadcast::Receiver<Arc<ServerMsg>>`).
- Changes `Room::resync_range(...) -> Result<(Vec<RoomEvent>, ResyncSource), DataError>` (was
  `Vec<Arc<ServerMsg>>`).
- No change to `Room::publish`/`Room::commit_ops_locked`'s own public return type
  (`Result<Command, DataError>` — unaffected callers in `chat/mod.rs` and `ws/conn.rs`'s ingress
  loop keep compiling unchanged).
- No change to `Egress::Frame`'s payload type (`Arc<ServerMsg>`) — verified during plan-writing
  that every construction site (`Reject`, `SearchResult`/`SearchError`, `MoveError`, `ChatError`
  ×3, `PathResult`/`PathError`, malformed-frame `Error`) is a locally-synthesized non-`Event`
  frame; none ever carries `StoredCommand` data, so widening it would be unjustified churn.

- [ ] **Step 1: Read `src/server/src/ws/room.rs` (lines 1-1087, the non-test portion) and
  `src/server/src/ws/conn.rs` (lines 1-1715, the non-test portion) in full again immediately
  before editing** — confirm exact current line numbers before pasting, since Tasks 1-3 did not
  touch these two files but the campaign's own git history may have shifted lines since
  plan-writing.

- [ ] **Step 2: Add `RoomEvent` to `src/server/src/ws/room.rs`**

Add `use crate::data::snapshot::StoredCommand;` to the file's import list (alongside the existing
`use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};`).

Insert this new type directly after the `RingBuffer` `impl Default for RingBuffer` block (before
the `RoomStats` struct):

```rust
/// Internal broadcast/ring element `Room` fans out on `Room.tx` and buffers in `RingBuffer`.
/// Never serialized to the wire — the client-facing `ServerMsg` (including its own `Event`
/// variant) is untouched by this type's existence. Distinguishes a `StoredCommand`-carrying
/// broadcast (the only case needing the commit-time redaction snapshot) from every OTHER
/// `ServerMsg` variant `Room` broadcasts (pings, presence, `MoveStream`, ...), which pass
/// through unchanged.
#[derive(Debug, Clone)]
pub(crate) enum RoomEvent {
    /// A committed command awaiting per-recipient redaction and reduction to a plain wire
    /// `ServerMsg::Event` at send time.
    Event(Arc<StoredCommand>),
    /// Any other broadcast `ServerMsg`, forwarded unchanged.
    Other(Arc<ServerMsg>),
}

impl RoomEvent {
    /// seq of an `Event` variant, else `None`. Mirrors `ServerMsg::event_seq`.
    pub(crate) fn event_seq(&self) -> Option<i64> {
        match self {
            RoomEvent::Event(stored) => Some(stored.command.seq),
            RoomEvent::Other(msg) => msg.event_seq(),
        }
    }

    /// server-stamped ts of an `Event` variant, else `None`. Mirrors `ServerMsg::event_ts`.
    pub(crate) fn event_ts(&self) -> Option<i64> {
        match self {
            RoomEvent::Event(stored) => Some(stored.command.ts),
            RoomEvent::Other(msg) => msg.event_ts(),
        }
    }
}
```

- [ ] **Step 3: Change `RingBuffer`'s element type**

Change the struct field (currently `events: VecDeque<Arc<ServerMsg>>`) to:

```rust
pub struct RingBuffer {
    /// Buffered frames, ascending seq; every entry is a `RoomEvent::Event`.
    events: VecDeque<RoomEvent>,
}
```

Change `push`'s signature and body:

```rust
    /// Append an `Event` frame and prune by count then age.
    pub fn push(&mut self, msg: RoomEvent) {
        debug_assert!(msg.event_seq().is_some(), "only Event frames are buffered");
        self.events.push_back(msg);
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
        if let Some(newest) = self.events.back().and_then(|m| m.event_ts()) {
            while let Some(oldest) = self.events.front().and_then(|m| m.event_ts()) {
                if newest - oldest > MAX_AGE_MS {
                    self.events.pop_front();
                } else {
                    break;
                }
            }
        }
    }
```

Change `range_from`'s return type (body is otherwise unchanged — `.cloned()` on a `RoomEvent`
works identically since it derives `Clone`):

```rust
    pub fn range_from(&self, from_seq: i64) -> Option<Vec<RoomEvent>> {
        match self.events.front().and_then(|m| m.event_seq()) {
            Some(oldest) if oldest <= from_seq => Some(
                self.events
                    .iter()
                    .filter(|m| m.event_seq().map(|s| s >= from_seq).unwrap_or(false))
                    .cloned()
                    .collect(),
            ),
            _ => None,
        }
    }
```

The doctest in `RingBuffer::new`'s doc comment (`RingBuffer::new().range_from(1).is_none()`) is
unaffected (no type is named in it).

- [ ] **Step 4: Change `Room.tx`'s element type and `subscribe`/`broadcast_aux`**

Change the field:

```rust
    /// The lossy broadcast sender every connection subscribes to.
    tx: broadcast::Sender<RoomEvent>,
```

Change `Room::new`'s local binding: `let (tx, _rx) = broadcast::channel(broadcast_capacity);`
stays syntactically identical (type is inferred from the field it's assigned to — no source
change needed there).

Change `subscribe`:

```rust
    /// Subscribe to live frames; also returns the room's current seq so a joiner knows whether
    /// it needs to resync.
    pub fn subscribe(&self) -> (broadcast::Receiver<RoomEvent>, i64) {
        (
            self.tx.subscribe(),
            self.current_seq.load(Ordering::Acquire),
        )
    }
```

Change `broadcast_aux`:

```rust
    pub fn broadcast_aux(&self, msg: ServerMsg) {
        let _ = self.tx.send(RoomEvent::Other(std::sync::Arc::new(msg)));
    }
```

- [ ] **Step 5: Rewrite `commit_ops_locked`'s persistence tail**

Replace the body from `let cmd = repo.apply_intent(...)` through the function's end with:

```rust
    pub(crate) async fn commit_ops_locked(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        ops: Vec<Operation>,
        ts: i64,
        origin: WriteOrigin,
    ) -> Result<Command, DataError> {
        let stored = repo
            .apply_intent(ctx, self.world_id, ops, ts, origin)
            .await?;
        // Hydrate the derived ECS from the committed command while still holding
        // publish_guard (enforced by the caller), so the ECS is consistent with the seq
        // before the Event (and any derived recompute keyed to that seq) is observable.
        {
            let mut scene = self.scene.write().await;
            for op in &stored.command.ops {
                scene.apply_op(op);
            }
            scene.set_committed_seq(stored.command.seq);
        }
        let stored = Arc::new(stored);
        let ev = RoomEvent::Event(stored.clone());
        self.ring.lock().await.push(ev.clone());
        self.current_seq.store(stored.command.seq, Ordering::Release);
        let _ = self.tx.send(ev); // Err only when there are no receivers
        self.stats.events_published.fetch_add(1, Ordering::Relaxed);
        Ok(stored.command.clone())
    }
```

- [ ] **Step 6: Rewrite `resync_range`**

```rust
    pub async fn resync_range(
        &self,
        repo: &dyn Repository,
        from_seq: i64,
    ) -> Result<(Vec<RoomEvent>, ResyncSource), DataError> {
        if let Some(hot) = self.ring.lock().await.range_from(from_seq) {
            self.stats.resyncs_hot.fetch_add(1, Ordering::Relaxed);
            return Ok((hot, ResyncSource::Buffer));
        }
        let cmds = repo.events_since(self.world_id, from_seq - 1).await?;
        self.stats.resyncs_cold.fetch_add(1, Ordering::Relaxed);
        let frames = cmds
            .into_iter()
            .map(|stored| RoomEvent::Event(Arc::new(stored)))
            .collect();
        Ok((frames, ResyncSource::Log))
    }
```

- [ ] **Step 7: Fix `ring_tests`' `event()` helper**

Replace:

```rust
    fn event(seq: i64, ts: i64) -> Arc<ServerMsg> {
        Arc::new(ServerMsg::Event {
            command: Command {
                seq,
                world_id: Uuid::from_u128(1),
                author: Uuid::from_u128(2),
                ts,
                ops: vec![],
            },
            intent_id: None,
        })
    }
```

with:

```rust
    fn event(seq: i64, ts: i64) -> RoomEvent {
        RoomEvent::Event(Arc::new(StoredCommand {
            command: Command {
                seq,
                world_id: Uuid::from_u128(1),
                author: Uuid::from_u128(2),
                ts,
                ops: vec![],
            },
            snapshot: crate::data::snapshot::CommandSnapshot {
                per_op: vec![],
                world_gm_at_commit: std::collections::HashMap::new(),
            },
        }))
    }
```

Every existing call site in `ring_tests` (`rb.push(event(s, 0))`,
`all.first().unwrap().event_seq().unwrap()`, etc.) compiles unchanged — `RoomEvent::event_seq()`
has the identical shape `ServerMsg::event_seq()` had.

- [ ] **Step 8: Fix `evict_user_reaches_every_room`**

Replace:

```rust
        for rx in [&mut rx1, &mut rx2] {
            match rx.recv().await.unwrap().as_ref() {
                ServerMsg::Evicted { user } => assert_eq!(*user, Some(target)),
                other => panic!("expected Evicted, got {other:?}"),
            }
        }
```

with:

```rust
        for rx in [&mut rx1, &mut rx2] {
            match rx.recv().await.unwrap() {
                RoomEvent::Other(msg) => match msg.as_ref() {
                    ServerMsg::Evicted { user } => assert_eq!(*user, Some(target)),
                    other => panic!("expected Evicted, got {other:?}"),
                },
                RoomEvent::Event(_) => panic!("expected a non-Event broadcast (Evicted)"),
            }
        }
```

- [ ] **Step 9: Fix `commit_ops_writes_and_broadcasts_without_gating`**

Replace:

```rust
        assert!(matches!(
            &*rx.recv().await.unwrap(),
            ServerMsg::Event { .. }
        ));
```

with:

```rust
        assert!(matches!(
            rx.recv().await.unwrap(),
            RoomEvent::Event(_)
        ));
```

- [ ] **Step 10: Rewrite `send_filtered` in `src/server/src/ws/conn.rs`**

Replace the entire existing `send_filtered` function with three functions:

```rust
/// Redact a `StoredCommand`-carrying `Event` frame for `ctx` (per-recipient, seq-preserving)
/// and send it, reduced to the plain wire `ServerMsg::Event` at this — the ONLY — point where it
/// is serialized. Used for live broadcast delivery AND replay (the same path).
async fn send_filtered_event<S>(
    sink: &mut S,
    repo: &dyn Repository,
    room: &Room,
    ctx: &PermissionContext,
    world_defaults: &WorldCapDefaults,
    stored: &crate::data::snapshot::StoredCommand,
) -> Result<(), ()>
where
    S: Sink<Message> + Unpin,
{
    // Loads complete BEFORE the guard: no lock across await. The guard is held only around the
    // synchronous core — the same short-read-guard discipline as clip_move_stream.
    let current = crate::data::permission::load_current_docs(repo, &stored.command).await;
    let filtered = {
        let ecs = room.scene().read().await;
        crate::data::permission::filter_command(
            &stored.command,
            &stored.snapshot,
            ctx,
            world_defaults,
            &current,
            |id| ecs.actor(id),
        )
    };
    let out = ServerMsg::Event {
        command: filtered,
        intent_id: None,
    };
    sink.send(text(&out)).await.map_err(|_| ())
}

/// Send a non-`Event` broadcast frame unchanged. `MoveStream` must never reach here — it
/// requires per-recipient clipping in the egress loop (`clip_move_stream`); this guard catches a
/// future routing regression at test time.
async fn send_plain<S>(sink: &mut S, msg: &ServerMsg) -> Result<(), ()>
where
    S: Sink<Message> + Unpin,
{
    debug_assert!(
        !matches!(msg, ServerMsg::MoveStream { .. }),
        "MoveStream must be clipped per-recipient in egress_loop, not sent via send_plain"
    );
    sink.send(text(msg)).await.map_err(|_| ())
}

/// Dispatch a `RoomEvent` to its wire representation: `Event` frames are redacted per-recipient
/// via `send_filtered_event`; every other frame passes through `send_plain` unchanged. Shared by
/// live broadcast delivery and replay (`replay`).
async fn send_room_event<S>(
    sink: &mut S,
    repo: &dyn Repository,
    room: &Room,
    ctx: &PermissionContext,
    world_defaults: &WorldCapDefaults,
    ev: &crate::ws::room::RoomEvent,
) -> Result<(), ()>
where
    S: Sink<Message> + Unpin,
{
    match ev {
        crate::ws::room::RoomEvent::Event(stored) => {
            send_filtered_event(sink, repo, room, ctx, world_defaults, stored).await
        }
        crate::ws::room::RoomEvent::Other(msg) => send_plain(sink, msg.as_ref()).await,
    }
}
```

- [ ] **Step 11: Fix `handle_socket`'s `Egress::Frame` call site**

At the `Egress::Frame(f)` arm (currently `if send_filtered(&mut sink, repo.as_ref(), &room, &ctx,
&world_defaults, f.as_ref()).await.is_err() { break; }`), replace with:

```rust
                Some(Egress::Frame(f)) => {
                    if send_plain(&mut sink, f.as_ref()).await.is_err() { break; }
                }
```

- [ ] **Step 12: Fix `egress_loop`'s broadcast-receive arm**

Replace the block starting `msg = rx.recv() => match msg { Ok(msg) => { if let Some(seq) =
msg.event_seq() { ... } else { ... let should_break = match msg.as_ref() { ... } ... } } ...}`
with:

```rust
            msg = rx.recv() => match msg {
                Ok(msg) => {
                    if let Some(seq) = msg.event_seq() {
                        if seq < next_expected {
                            continue; // already delivered via a resync
                        }
                        if seq > next_expected {
                            room.stats.gaps_detected.fetch_add(1, Ordering::Relaxed);
                            tracing::debug!(world = %world_id, expected = next_expected, got = seq, "gap detected");
                            match replay(&mut sink, &room, repo.as_ref(), &ctx, &world_defaults, next_expected).await {
                                Ok(to_seq) => next_expected = to_seq + 1,
                                Err(_) => break,
                            }
                            if seq < next_expected { continue; }
                        }
                        if send_room_event(&mut sink, repo.as_ref(), &room, &ctx, &world_defaults, &msg).await.is_err() { break; }
                        next_expected = seq + 1;
                        if (!subs.is_empty() || !scene_subs.is_empty())
                            && reeval_deadline.is_none()
                        {
                            reeval_deadline = Some(tokio::time::Instant::now() + SEARCH_DEBOUNCE);
                        }
                    } else {
                        let crate::ws::room::RoomEvent::Other(inner) = &msg else {
                            unreachable!("event_seq() is Some for every RoomEvent::Event");
                        };
                        let should_break = match inner.as_ref() {
                            ServerMsg::MoveStream { .. } => {
                                let see_as = if ctx.world_role
                                    == crate::data::document::WorldRole::Gm
                                {
                                    scene_subs
                                        .values()
                                        .find(|s| {
                                            s.channel == "vision"
                                                && s.view_ctx.user_id != ctx.user_id
                                        })
                                        .map(|s| s.view_ctx)
                                } else {
                                    None
                                };
                                match clip_move_stream(inner.as_ref(), &ctx, see_as, &room).await {
                                    Some(out) => sink.send(text(&out)).await.is_err(),
                                    None => false,
                                }
                            }
                            ServerMsg::Evicted { user } => {
                                if user.is_none() || *user == Some(ctx.user_id) {
                                    let _ = sink.send(text(inner.as_ref())).await;
                                    let _ = sink.send(Message::Close(None)).await;
                                    true
                                } else {
                                    false
                                }
                            }
                            other => send_plain(&mut sink, other).await.is_err(),
                        };
                        if should_break {
                            break;
                        }
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    room.stats.lagged_drops.fetch_add(n, Ordering::Relaxed);
                    tracing::warn!(world = %world_id, dropped = n, "broadcast lagged");
                    match replay(&mut sink, &room, repo.as_ref(), &ctx, &world_defaults, next_expected).await {
                        Ok(to_seq) => next_expected = to_seq + 1,
                        Err(_) => break,
                    }
                }
                Err(RecvError::Closed) => break,
            },
```

This preserves every existing comment's INTENT from the original block (see the pre-edit source
read during plan-writing for the full comment text on the debounce arming, the see-as resolution,
and the targeted-eviction delivery) — carry those comments forward verbatim onto their same
logical lines; they are omitted above only for this plan document's brevity, not to be dropped
from the actual edit.

- [ ] **Step 13: Fix `replay`**

Replace the loop body:

```rust
    for f in frames {
        send_filtered(sink, repo, room, ctx, world_defaults, f.as_ref()).await?;
    }
```

with:

```rust
    for f in &frames {
        send_room_event(sink, repo, room, ctx, world_defaults, f).await?;
    }
```

(`frames: Vec<RoomEvent>` is no longer consumed by value here since `send_room_event` borrows;
`to_seq`'s computation two lines above, `frames.last().and_then(|m| m.event_seq())`, is unaffected
— `RoomEvent::event_seq()` has the same shape.)

- [ ] **Step 14: Fix the one remaining `conn.rs` test that dereferences a broadcast frame
  directly**

In the test exercising `handle_move_request`'s success broadcast (the one reading `rx.recv()`
after subscribing and matching `*msg` against `ServerMsg::MoveStream`), replace:

```rust
        let bcast = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        if matches!(*msg, ServerMsg::MoveStream { .. }) {
                            return Some((*msg).clone());
                        }
                        // Skip other frames (e.g. position Event from commit_ops_locked).
                    }
                    Err(_) => return None,
                }
            }
        })
```

with:

```rust
        let bcast = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                match rx.recv().await {
                    Ok(crate::ws::room::RoomEvent::Other(msg)) => {
                        if matches!(msg.as_ref(), ServerMsg::MoveStream { .. }) {
                            return Some((*msg).clone());
                        }
                        // Skip other out-of-band frames.
                    }
                    Ok(crate::ws::room::RoomEvent::Event(_)) => {
                        // Skip: a committed Event frame (e.g. the move's position Update), not
                        // MoveStream.
                    }
                    Err(_) => return None,
                }
            }
        })
```

- [ ] **Step 15: Rebuild and fix any remaining compiler errors**

Run: `cargo build --tests --manifest-path src/server/Cargo.toml`
Expected after Steps 2-14: 0 errors. If any remain (e.g. another test in `conn.rs` or `room.rs`
this plan's read did not surface — both files are large), each will be a variant of the SAME
three patterns already fixed above (an `.event_seq()`-only usage needs no change; a
`*msg`/`.as_ref()` dereference into `ServerMsg` needs the `RoomEvent::Other(...)` match added; a
`ServerMsg::Event { .. }` match needs `RoomEvent::Event(_)`). Apply the matching pattern and
re-run until it exits 0.

- [ ] **Step 16: Write a new end-to-end WS-level differential test**

Add to `src/server/src/ws/conn.rs`'s `#[cfg(test)] mod tests` block:

```rust
    #[tokio::test]
    async fn e2e_replay_redacts_a_field_that_was_gm_only_at_commit_after_the_override_is_later_widened() {
        // A field hidden (GmOnly) across several historical writes, then WIDENED to fully
        // public in a LATER command, must still redact its intermediate historical values on
        // replay — reading the current value as public does not make its whole secret evolution
        // public. This is the discriminating shape: by the time redaction runs, hidden_current
        // is EMPTY (the override was widened), so ONLY hidden_commit (this design's fix) keeps
        // the historical value hidden; an implementation that redacted against current policy
        // alone would leak it here.
        use crate::data::command::{FieldChange, Operation};
        use crate::data::document::{DocRole, PermissionSet, Scope};
        use crate::data::membership::PermissionContext;
        use crate::data::sqlite::SqliteRepository;

        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
        let player = repo.create_user("pl", None, ServerRole::User, 0).await.unwrap();
        let w = repo.create_world_owned("W", gm, 0).await.unwrap();
        repo.add_member(w.id, player, WorldRole::Player).await.unwrap();
        let gm_ctx = PermissionContext { user_id: gm, world_role: WorldRole::Gm };

        let mut perms = PermissionSet { default: DocRole::Observer, ..Default::default() };
        perms.property_overrides.insert(
            "/system/secret".into(),
            crate::data::document::Visibility::GmOnly,
        );
        let mut d = crate::data::document::tests::world_scoped_doc(w.id, Uuid::new_v4(), "actor");
        d.permissions = perms;
        d.system = serde_json::json!({ "secret": "S0" });
        let doc_id = d.id;
        repo.apply_intent(&gm_ctx, w.id, vec![Operation::Create { doc: d }], 0, WriteOrigin::Client)
            .await
            .unwrap();

        // Historical, intermediate value — GmOnly at commit — this is the value that must
        // never reach the player, at any resync, no matter how the override later changes.
        repo.apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/secret".into(),
                    old: serde_json::json!("S0"),
                    new: serde_json::json!("S1_NEVER_RELEASED"),
                }],
            }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        // The GM later widens the override to All — the CURRENT value becomes public, but the
        // historical S1_NEVER_RELEASED value must remain hidden on replay.
        repo.apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/permissions/property_overrides/~1system~1secret".into(),
                    old: serde_json::json!("gm_only"),
                    new: serde_json::json!("all"),
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

        let reg = crate::ws::room::RoomRegistry::new();
        let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
        let (frames, _src) = room.resync_range(&repo, 1).await.unwrap();
        let player_ctx = PermissionContext { user_id: player, world_role: WorldRole::Player };
        let world_defaults = crate::data::document::WorldCapDefaults::default();
        let mut sink = Vec::<Message>::new();
        for f in &frames {
            send_room_event(&mut sink, &repo, &room, &player_ctx, &world_defaults, f)
                .await
                .unwrap();
        }
        let mut saw_the_permission_widening_op = false;
        for msg in &sink {
            let text = match msg {
                Message::Text(t) => t.as_str(),
                _ => continue,
            };
            let v: serde_json::Value = serde_json::from_str(text).unwrap();
            if v["type"] != "event" {
                continue;
            }
            for op in v["command"]["ops"].as_array().unwrap() {
                if op["op"] != "update" {
                    continue;
                }
                for ch in op["changes"].as_array().unwrap() {
                    if ch["path"] == "/permissions/property_overrides/~1system~1secret" {
                        saw_the_permission_widening_op = true;
                    }
                    assert_ne!(
                        ch["new"], "S1_NEVER_RELEASED",
                        "a value that was GmOnly at commit must stay hidden even after the \
                         override is later widened to All"
                    );
                }
            }
        }
        assert!(
            saw_the_permission_widening_op,
            "sanity check: the widening command itself must be visible to the player \
             (only the earlier GmOnly value must stay hidden)"
        );
    }
```

`sink: Vec<Message>` needs `S: Sink<Message> + Unpin` to be satisfied by `Vec<Message>` — verify
the crate already has a `Sink<Message>` impl usable for a `Vec` in its test helpers (search
`src/server/src/ws/conn.rs`'s existing test module for a similar collecting-sink pattern used by
other tests reading serialized frames, e.g. any test using `futures_util::sink::unfold` or a
custom test `Sink` type); if none exists, add a minimal one directly above this test:

```rust
    /// A `Sink<Message>` that collects every sent frame into a `Vec`, for tests that inspect
    /// serialized output directly rather than driving a real socket.
    struct CollectingSink(Vec<Message>);
    impl futures_util::Sink<Message> for CollectingSink {
        type Error = std::convert::Infallible;
        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn start_send(self: std::pin::Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
            self.get_mut().0.push(item);
            Ok(())
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }
```

and replace `let mut sink = Vec::<Message>::new();` / the `for f in &frames { send_room_event(&mut
sink, ...` / `for msg in &sink` lines with `let mut sink = CollectingSink(Vec::new()); for f in
&frames { send_room_event(&mut sink, &repo, &room, &player_ctx, &world_defaults, f).await.unwrap();
} for msg in &sink.0 { ... }`.

- [ ] **Step 17: Run the new and existing conn/room test suites**

Run: `cargo test --manifest-path src/server/Cargo.toml ws::room`
Run: `cargo test --manifest-path src/server/Cargo.toml ws::conn`
Expected: every test passes, including `e2e_replay_redacts_a_field_that_was_gm_only_at_commit_after_the_override_is_later_widened`.

- [ ] **Step 18: Verify the wire is genuinely unaffected**

Run: `cargo test --all` (this regenerates ts-rs bindings as a side effect of running the crate's
doctests/`#[ts(export)]`-annotated types' own tests).
Run: `git diff --exit-code src/types/generated`
Expected: exit 0, no diff — proves `Operation`/`Command`/`ClientMsg`/`ServerMsg` and their
generated TypeScript mirrors are byte-identical to before this campaign, mirroring CI's own
"ts-rs bindings in sync" check.
Run: `git status --porcelain src/client src/modules`
Expected: empty output — no client/module file was touched by this task.

- [ ] **Step 19: Run the full per-task CI gate battery** (same five commands as Task 1 Step 15)

- [ ] **Step 20: Commit**

```bash
git add src/server/src/ws/room.rs src/server/src/ws/conn.rs
git commit -m "feat(ws): carry the commit-time snapshot through broadcast, ring, and replay"
```

---

## Task 5: Documentation, skill updates, and campaign closeout

**Files:**
- Modify: `docs/TODO.md` (remove the "Actionable now — Phase 1b re-brainstorm" heading and body)
- Modify: `docs/OPEN_BUGS.md` (remove the two closed bullets)
- Modify: `docs/CLOSED_BUGS.md` (add the two closed defects' resolution entries)
- Modify: `.claude/skills/shadowcat-codebase-documents-permissions/SKILL.md`
- Modify: `.claude/skills/shadowcat-codebase-realtime-sync/SKILL.md`
- Modify: `.claude/.claude-plugin/plugin.json` (`version`: `1.0.51` → `1.0.52`)

**Interfaces:**
- Consumes: nothing new (Tasks 1-4 must all be merged/green first).
- Produces: nothing consumed by a later task — this is the plan's final task.

- [ ] **Step 1: Read the current `docs/TODO.md`, `docs/OPEN_BUGS.md`, and `docs/CLOSED_BUGS.md`
  in full** immediately before editing (their exact current line numbers/surrounding content may
  have shifted since plan-writing).

- [ ] **Step 2: Remove the closed heading from `docs/TODO.md`**

Delete the entire `## Actionable now — Phase 1b re-brainstorm: point-in-time replay redaction
(commit-time snapshot)` heading and its bulleted body (the design is now implemented and verified
by Tasks 1-4's test suites, not merely scheduled).

- [ ] **Step 3: Move the two closed bugs from `docs/OPEN_BUGS.md` to `docs/CLOSED_BUGS.md`**

Read `docs/CLOSED_BUGS.md`'s existing entry format first (to match its established style), then
remove both bullets (the `filter_command`'s `Update` arm / `collect_hidden` bullet and the stale
`Update`-from-before-deletion bullet, including all of their nested sub-bullets) from
`docs/OPEN_BUGS.md`, and add a corresponding closed-entry pair to `docs/CLOSED_BUGS.md` stating:
both defects closed by carrying a commit-time redaction snapshot (`StoredCommand`/
`CommandSnapshot`/`OpSnapshot`) alongside every `Command` through the write loops, `world_events`,
and the room broadcast/ring/resync path; `filter_command` now redacts against `hidden_current ∪
hidden_commit`; a document id reuse after hard delete is detected via `documents.created_seq`
generation-marker comparison. Reference the closing test names
(`world_role_promotion_does_not_disclose_pre_promotion_gm_only_or_owner_or_gm_history`,
`reused_id_drops_a_stale_update_against_the_new_generation`,
`e2e_replay_redacts_a_field_that_was_gm_only_at_commit_after_the_override_is_later_widened`) rather than task
numbers or dates, per RULE 15/RULE 16 (`docs/CLOSED_BUGS.md` is itself a `docs/` artifact, not
code, so citing test names here — not file:line — is the correct level of specificity for a
durable-doc record, distinct from the code-comment ban on file:line citations).

- [ ] **Step 4: Update `shadowcat-codebase-documents-permissions`'s SKILL.md**

Read the current skill file in full. Add, in whichever of its existing sections (Key files / Hard
invariants / Gotchas) most closely matches its established shape:
- Key files: `src/server/src/data/snapshot.rs` (`StoredCommand`/`CommandSnapshot`/`OpSnapshot`).
- Hard invariant: redaction is the conjunction of commit-time and current-time policy —
  `filter_command` takes a `CommandSnapshot` alongside `Command`; a pointer is hidden iff hidden
  at either commit or now.
- Gotcha: the commit-time half must never take a live parameter (`filter_command`'s signature has
  none by construction — a future "just add a quick lookup" change would be a loud diff).
- Gotcha: `documents.created_seq` is set once at genuine first INSERT (via `upsert_document`'s
  `ON CONFLICT` clause omitting it) and is the sole signal distinguishing a reused document id
  from its predecessor generation.

- [ ] **Step 5: Update `shadowcat-codebase-realtime-sync`'s SKILL.md**

Read the current skill file in full. Add:
- Key files: the `RoomEvent` enum in `src/server/src/ws/room.rs` (internal broadcast/ring
  element, `Event(Arc<StoredCommand>)` | `Other(Arc<ServerMsg>)`).
- Hard invariant: `Room.tx`/`RingBuffer` carry `RoomEvent`, never `Arc<ServerMsg>` directly, for
  the `Event` case — the client-facing `ServerMsg::Event` shape is reconstructed only at
  `send_filtered_event`, the single serialization point.
- Gotcha: `Egress::Frame`'s payload stays plain `Arc<ServerMsg>` (every construction site is a
  locally-synthesized non-`Event` frame) — do not widen it to `RoomEvent` without first
  re-verifying that invariant against every current `Egress::Frame` construction site.

- [ ] **Step 6: Dispatch `shadowcat-spec-reviewer` on both skill diffs**

Dispatch (no `name` parameter — this must be a run-to-completion worker whose final text IS the
result):

```
Task(shadowcat-spec-reviewer, "Review the diffs to
.claude/skills/shadowcat-codebase-documents-permissions/SKILL.md and
.claude/skills/shadowcat-codebase-realtime-sync/SKILL.md against the actual code changes in
src/server/src/data/snapshot.rs, src/server/src/data/permission.rs, src/server/src/data/sqlite.rs,
src/server/src/ws/room.rs, and src/server/src/ws/conn.rs from this campaign (commit-time replay
redaction, StoredCommand/CommandSnapshot/OpSnapshot, RoomEvent). Confirm each skill diff
accurately captures the change — no omission, drift, or broken pointer. Report PASS or a list of
concrete corrections needed.", effort: high)
```

Address every reported correction inline before proceeding; re-dispatch if any correction was
non-trivial.

- [ ] **Step 7: Bump the plugin version**

In `.claude/.claude-plugin/plugin.json`, change `"version": "1.0.51"` to `"version": "1.0.52"`.

- [ ] **Step 8: Run the FULL final CI gate battery one more time, from a clean state**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D missing-docs -D clippy::missing-docs-in-private-items
cargo +nightly doc --manifest-path src/server/Cargo.toml --document-private-items --no-deps --target-dir target/nightly-doc
git diff --exit-code src/types/generated
```

Expected: all six commands exit 0.

- [ ] **Step 9: Commit**

```bash
git add docs/TODO.md docs/OPEN_BUGS.md docs/CLOSED_BUGS.md \
  .claude/skills/shadowcat-codebase-documents-permissions/SKILL.md \
  .claude/skills/shadowcat-codebase-realtime-sync/SKILL.md \
  .claude/.claude-plugin/plugin.json
git commit -m "docs: close the two commit-time-redaction OPEN_BUGS entries and sync skills"
```

- [ ] **Step 10: Push** (per this project's Documentation Standards: push only on a completed
  milestone — this plan's final task IS that milestone)

```bash
git push origin phase1b-replay-redaction
gh run watch
```

If CI goes red, fix forward starting from the topmost error layer — do not pause to ask
permission; only stop if genuinely stuck after diagnosing the root cause.
