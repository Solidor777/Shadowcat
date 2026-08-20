# Phase 1b — commit-time visibility snapshot for replay redaction

**Status:** re-brainstormed from `docs/superpowers/specs/2026-08-13-phase1b-design-findings.md`
(two blind reviewers, both "needs rework," on the first proposal). This document is the
corrected design, built directly against current source (verified 2026-08-20), not against the
superseded proposal. Read the findings doc first — this document assumes its "Owner rulings" and
"What both reviewers found independently" sections as settled and does not re-litigate them.

**Closes:** the two defects in `docs/OPEN_BUGS.md` — (1) `filter_command`/`collect_hidden`
redacting replay against a document's CURRENT permission set instead of the policy in force at
the historical seq (and its `OwnerOrGm`-tier analog under ownership reassignment); (2) a stale
`Update`/`Delete` from before a document's deletion redacting/applying against a NEW document
that later reuses the same id.

**Already done, orthogonal:** `Room::resync_floor` (tasks 13j/13k, enforced by default) bounds
*how far back* a client may resync. This design governs *what's disclosed* within whatever range
is reachable — no overlap, no conflict.

## The corrected problem

> Redaction must be the conjunction of two views — what was permitted at commit, and what is
> permitted now — where the commit-time view is carried by the op and the current view may only
> WITHHOLD visibility, never grant it.

A pointer is redacted from a recipient's view of a historical change iff it was hidden from them
**at commit time**, OR it is hidden from them **now**. Never fewer checks than today (current-state
redaction stays, unchanged, as the "now" half); a new commit-time half is added and unioned in.

## Architecture

**The wire (`Operation`, `Command`, `ClientMsg`, `ServerMsg`, the Zod schema) does not change at
all.** No ts-rs regen, no Zod schema edit, no client-side change. The snapshot is a purely
server-internal wrapper introduced at exactly two points: where a command is committed
(`SqliteRepository::apply_command`/`apply_intent`, after their mutation loop completes) and where
it's stored/replayed (`world_events.command_json`, `Room`'s ring buffer). `filter_command` reads
the wrapper's snapshot half for the commit-time view and the wrapper's `Command` half — through
the existing current-state lookups, unchanged — for the current view; its output is the same
plain, snapshot-free `Command` sent over the wire today.

This directly satisfies "client-forgeable" (the client never sends or receives a type that has a
snapshot field — structurally impossible to forge, not a discipline) and "keep off the wire by
construction" (there is no wire type to keep it off of — it was never on one).

```
apply_command / apply_intent
  └─ mutation loop commits all ops in the command (existing, unchanged)
  └─ NEW: build CommandSnapshot from the POST-image (after the loop, once — not per-op)
  └─ StoredCommand { command: Command, snapshot: CommandSnapshot }
       ├─ Room::publish → ring buffer (StoredCommand, not Command)
       └─ persisted → world_events.command_json (serde_json::to_string(&StoredCommand))

send_filtered (live delivery AND replay — same path, unchanged)
  └─ per recipient: filter_command(&stored.command, &stored.snapshot, ctx, current, ...)
       ├─ hidden_current  = collect_hidden(current_doc, access_current)          — TODAY's logic, unchanged
       ├─ hidden_commit   = collect_hidden(snapshot.overrides_at_commit, access_commit) — NEW
       ├─ hidden_effective = hidden_current ∪ hidden_commit
       └─ redact_change / retraction against hidden_effective → plain Command (wire-shape, unchanged)
```

## Components

### 1. `CommandSnapshot` (new type, `src/server/src/data/command.rs` or a new `redaction_snapshot.rs`
next to it — implementer's call, follow existing module boundaries)

```rust
/// Server-internal, never serialized to the wire. Index-aligned with `Command.ops` — built ONCE
/// from the whole command's post-image, after every op in the command has applied (never from an
/// op's own intermediate state — a per-op snapshot mid-command leaks values a LATER op in the
/// same command hides; see the findings doc's "leaks within a single command" finding).
pub struct CommandSnapshot {
    pub per_op: Vec<Option<OpSnapshot>>,
}

/// Commit-time redaction inputs for one op, sufficient to compute `collect_hidden`'s output
/// WITHOUT any live lookup — no `&Repository`, no actor-lookup closure, by construction (the
/// anti-drift enforcement mechanism: a live parameter can't be reintroduced without a loud
/// signature change). `None` in `CommandSnapshot::per_op` means "no snapshot recorded" — see
/// Back-compat below; every op produced by this design onward is `Some`.
pub struct OpSnapshot {
    /// Effective owner at commit (`effective_owner_via` evaluated against the post-image's
    /// actor-link state) — closes the `OwnerOrGm`-tier ownership-reassignment defect. `None` if
    /// the document has no effective owner (matches `Document.owner`'s own optionality).
    pub owner_at_commit: Option<Uuid>,
    /// `doc_type` at commit. Redundant for `Create`/`Delete` (already on the carried `doc`) —
    /// included on `Update`'s snapshot too, since `Operation::Update` doesn't carry `doc_type`
    /// and `effective_role`'s token owner-floor check needs it (see findings doc's "TWO uses").
    pub doc_type: String,
    /// The document's permission-override tree at commit: `property_overrides` for the doc
    /// itself plus every embedded descendant, addressed identically to `collect_hidden`'s live
    /// walk (`{prefix}/embedded/{key}/{idx}` — the SAME positional addressing, built from the
    /// POST-image's `embedded` map, so an index means what it meant at commit, never a later
    /// insert/remove's shifted index). PRUNED to the ancestor/descendant closure of this op's own
    /// `changes` paths (see `redact_change`'s existing prefix-match logic) UNLESS
    /// `retraction_hidden_at_commit` is `Some` (below), in which case this is the RAW pruned set
    /// still — retraction carries its own separate, unpruned field precisely so the common case
    /// stays cheap.
    pub overrides_at_commit: Vec<(String, Visibility)>,
    /// Present only when this op's own changes `touches_permissions` (narrows visibility) —
    /// the FULL (unpruned) commit-time hidden-pointer set for the doc, used to drive the
    /// retraction pass at REPLAY time using this command's own post-image, not whatever is live
    /// at replay time (each retraction-triggering command owns its own retraction moment).
    pub retraction_hidden_at_commit: Option<Vec<String>>,
    /// Present only for `Update`/`Delete` (ops addressing a PRE-EXISTING doc_id — `Create`
    /// establishes a fresh generation and needs no witness). The target document's
    /// `documents.created_seq` as read at commit time — compared against the CURRENT document's
    /// `created_seq` at redaction time; a mismatch means the id has been deleted and recreated
    /// since, and the op is DROPPED (not redacted-and-delivered against the wrong generation).
    /// Closes "Defect 2 is only half closed."
    pub created_seq_at_commit: Option<i64>,
}
```

Why not snapshot `Access` (world role, world grants) at commit time: `Access::can_see`'s
`GmOnly`/`OwnerOrGm` tiers depend only on `see_gm_only` (world-role-derived) and `is_owner`
(`owner_at_commit`-derived, above). `see_gm_only` is deliberately read CURRENT, at both the
"commit" and "current" evaluation — a non-GM recipient never sees `GmOnly` content regardless of
which point in time is being asked about, so using the live value for both is exact, not an
approximation. This mirrors the owner's existing "world-grants input is recorded as a design
note, not a bug" ruling — same reasoning, extended to world role for the same structural reason.
**State this explicitly in the implementation** so a future reader doesn't mistake the omission
for an oversight.

### 2. `documents.created_seq` (new column, edit `src/server/migrations/0001_init.sql` in place —
this repo has no data migrations pre-customers; there is no existing "creation-only, never
updated" column to reuse — `documents.seq` is overwritten by every write via `upsert_document`'s
`ON CONFLICT ... SET seq=excluded.seq`, tracking "last touched," not "created")

```sql
-- documents table, add:
created_seq INTEGER NOT NULL DEFAULT 0,
```

`upsert_document`'s INSERT lists `created_seq` bound to the write's own `seq`; its
`ON CONFLICT ... DO UPDATE SET` clause does **not** list `created_seq` — SQLite's `excluded.*`
semantics mean an unlisted column keeps its stored value across an UPDATE, so `created_seq` is
set once, at the row's genuine first INSERT, and never touched again by subsequent updates to a
still-live row. A hard delete (`delete_document_tx`) removes the row entirely; a later `Create`
reusing the same id is a genuine fresh INSERT, getting a NEW `created_seq` — exactly the
generation marker needed. No Rust-level `Document` struct field is required — this is
repository-internal bookkeeping, read directly by the write loops when building `OpSnapshot`
(a small `SqliteRepository` accessor, e.g. `document_created_seq(id) -> Option<i64>`, is enough;
implementer's call whether to fold it into an existing query or add a dedicated one).

### 3. `filter_command` (changes)

Add two parameters: `snapshot: &CommandSnapshot` (or the per-op slice already indexed by the
caller) and nothing else — no new live-state parameter, by design (signature deprivation: the
function that computes `hidden_at_commit` must be structurally unable to take a repository/actor
lookup, so a future "just add a quick current-state read here" change is a loud diff, not a
one-line addition; make this its own inner function if the module boundary helps enforce it).

Per op:
- **`Update`**: unchanged existence check (`current.get(doc_id)`, drop if absent — this stays;
  removing it was never proposed, only the reused-id gap alongside it). NEW: if
  `op_snapshot.created_seq_at_commit` is `Some(s)` and the current doc's `created_seq != s`, drop
  the op (id reused — this generation's Update does not belong to what's live now). Otherwise:
  compute `hidden_current` exactly as today; compute `hidden_commit` via
  `collect_hidden`-against-`overrides_at_commit` using `Access { see_gm_only: <current, live>,
  is_owner: (recipient == op_snapshot.owner_at_commit), .. }`; redact against
  `hidden_current ∪ hidden_commit`. Retraction pass (when `touches_permissions`): use
  `op_snapshot.retraction_hidden_at_commit` (unpruned, this command's own post-image) instead of
  today's live `collect_hidden(cur, access)` call.
- **`Delete`**: same `created_seq_at_commit` mismatch check against the CURRENT state at this
  `doc_id` (if a current doc exists and its `created_seq` doesn't match, drop — this Delete
  targeted a generation that's already gone and been replaced; applying it client-side would wrongly
  remove the live document). Access check becomes the conjunction: forward only if BOTH the
  commit-time access (`is_owner_at_commit` + live `see_gm_only`/caps against `op_snapshot.doc_type`)
  AND the current access (today's live check, unchanged) permit.
- **`Create`**: same conjunction as `Delete` (commit-time access AND current access) — no
  `created_seq` witness needed (establishes a fresh generation, nothing to compare against).

### 4. `apply_command` / `apply_intent` (both authoritative write loops)

After the existing mutation loop completes (post-image reached, per-op ordering preserved), build
`CommandSnapshot` once by iterating the just-applied `ops` against the now-final documents:
compute `owner_at_commit` via `effective_owner_via` against the POST-image actor table (the same
kind of lookup the loop already has access to — no new live-state dependency is being introduced,
only its *result* is now captured instead of discarded); read `overrides_at_commit`/
`retraction_hidden_at_commit` from the post-image document (walk `property_overrides` +
`embedded`, exactly `collect_hidden`'s existing traversal, reused as a pure function of `(doc,
prefix)` rather than `(doc, access, prefix)` — split `collect_hidden` into a tier-agnostic
"collect all overrides with their `Visibility`" pass, and a small `can_see`-filtering pass over
that output, so both the live path and the snapshot-construction path share one traversal instead
of duplicating it — see Testing below for why this split is load-bearing, not just tidy); read
`created_seq_at_commit` via the new repository accessor for `Update`/`Delete` targets. Persist
`StoredCommand` (not bare `Command`) into `world_events.command_json` and push it into
`Room::publish`'s ring buffer, replacing today's bare `Command` in both places.

`apply_command`'s Create arm stays id-blind (upsert) — `created_seq` is correctly preserved
across an ON-CONFLICT hit by the SQL shape above regardless of this loop's trust level.

### 5. Back-compat (`world_events` rows written before this change)

`command_json` is opaque `TEXT` — no SQL migration needed for the JSON shape itself. On read,
attempt `serde_json::from_str::<StoredCommand>`; a pre-fix row (bare `Command` JSON, no
`snapshot` key) fails that parse. **Fall back to constructing `StoredCommand { command: <parsed
as Command>, snapshot: CommandSnapshot { per_op: vec![None; command.ops.len()] } }`** — never a
live-lookup fallback. Per the findings doc's explicit ruling, a `None` `OpSnapshot` means
`filter_command` DROPS that op on replay rather than falling back to today's (defective)
current-state-only redaction — a one-time, reviewed, accepted cost against pre-fix history, not a
silent gap. State this in the migration/rollout notes so it isn't mistaken for a regression when
old worlds resync.

## Testing

- **Positive**: reproduce both `OPEN_BUGS.md` scenarios as failing-before/passing-after
  differential tests (property-tier narrowing then widening; ownership reassignment) — the exact
  discrimination-check discipline used throughout this campaign.
- **Reused-id**: a document deleted then a new one created at the same id; a stale `Update` and a
  stale `Delete` from the old generation must both be dropped on replay, never applied to the new
  generation.
- **Retraction correctness under replay**: a command that narrows visibility, replayed long after
  a LATER command has narrowed it further — the retraction pass must reflect what the CHOSEN
  command itself hid (its own post-image), not whatever is live now or what a later command
  additionally hid.
- **Multi-op-in-one-command leak (the finding-9 case)**: `[Update{secret:=X}, Update{add
  gm_only override}]` in ONE command — a non-GM/non-owner recipient must NOT see `X` on replay
  (this is exactly what "snapshot from the post-image, once, after the loop" fixes; write it as a
  test that would fail under a naive per-op-inline snapshot).
- **Behavioural mutation test (per the findings doc's "signature deprivation" pairing)**: mutate
  each live input independently — the target's overrides, its default, the linked actor's owner,
  an embedded child's index, the world grants — and assert `filter_command`'s CURRENT-time output
  is unaffected by history and its COMMIT-time output is unaffected by anything live. The
  embedded-index case is the one a grep-shaped review could never catch on its own.
- **Traversal-split unit test**: the shared `(doc, prefix) → Vec<(String, Visibility)>` traversal
  used by both the live path and snapshot construction must produce byte-identical output for the
  same document — this is what keeps the two paths from re-forking (this codebase's own
  highest-frequency defect class) the moment one of them is touched later.
- **Wire-unaffected**: confirm `Operation`/`Command`/`ServerMsg`/`ClientMsg` and the Zod schema
  are byte-identical before/after (no ts-rs diff, no ts-rs regen needed) — this is the design's
  own claim and should be pinned, not assumed.
- **`Operation::invert`**: per the findings doc's flag, decide and test its behavior now that a
  server-internal `StoredCommand`/`CommandSnapshot` exists alongside it. `invert` operates on the
  wire `Operation`/`Command` only (never on `StoredCommand`) and has no live caller today
  (confirmed: only test/doctest call sites) — state explicitly that inversion is defined over the
  snapshot-free type and produces a snapshot-free `Operation`, so `CommandSnapshot` is simply not
  its concern; add a doc-comment note on `invert` cross-referencing this so a future undo/redo
  feature discovers the question rather than silently carrying a stale snapshot forward.
- Full CI gate battery, including `cargo test --all`, doc-coverage clippy passes, and (since this
  touches no client/wire code) confirm the client/TS gates are unaffected rather than skipped —
  run them once to prove it, don't just assert it.

## Non-goals / explicitly out of scope (owner-ruled or newly-scoped here, state plainly)

- World role and world grants stay current-only at both halves of the conjunction — not a gap,
  a deliberate, justified choice (see Components §1).
- Audit-grade point-in-time replay (a queryable history) is its own later milestone per
  `docs/PLAN.md` — this phase is its prerequisite (the commit-time snapshot seam), not a
  duplicate.
- No client-visible change of any kind.

## Open questions the implementation plan must settle against exact current signatures (not
guessed here — verify at plan-writing time)

- Exact function name/location for the shared `(doc, prefix) → Vec<(String, Visibility)>`
  traversal split out of `collect_hidden` — module boundary is the implementer's call, following
  existing `permission.rs` conventions.
- Whether `effective_owner_via`'s actor-lookup closure is cheaply reusable post-mutation-loop
  inside `apply_command`/`apply_intent`, or needs a small adapter — read both write loops in full
  before writing the task brief.
- Repository accessor shape for `document_created_seq` — a dedicated query vs. folding into an
  existing document-load call; pick whichever avoids an extra round trip on the hot write path.
