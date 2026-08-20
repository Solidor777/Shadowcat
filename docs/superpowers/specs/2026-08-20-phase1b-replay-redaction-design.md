# Phase 1b — commit-time visibility snapshot for replay redaction

**Status:** re-brainstormed from `docs/superpowers/specs/2026-08-13-phase1b-design-findings.md`
(two blind reviewers, both "needs rework," on the first proposal). This document is the corrected
design, built directly against current source (verified 2026-08-20). Read the findings doc first
— this document assumes its "Owner rulings" and "What both reviewers found independently"
sections as settled and does not re-litigate them.

**Revision note:** this design itself went through one round of buddy-checking (two independent
blind reviewers plus a debate round, both fully converged) before this revision. Six gaps were
found and are fixed in this text — three Critical (a world-role bypass that also defeats the
ownership-reassignment fix, a missing final-state accumulator that reopens the exact
mid-command-leak defect via the write loops' natural shape, and an undercounted plumbing chain
from commit through live-broadcast/replay), two Important (a retraction-tier regression, a
missing current-doc lookup for `Delete`), one Minor (an undocumented pre-existing gap). None of
this went unaddressed — see the sections below, each of which states plainly what changed and
why, since a future reader without the review transcript needs the reasoning, not just the
conclusion.

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

**The CLIENT-FACING wire (`Operation`, `Command`, `ClientMsg`, `ServerMsg`, the Zod schema) does
not change at all.** No ts-rs regen, no Zod schema edit, no client-side change of any kind. This
is the design's real invariant, and it is narrower than "only two insertion points" — the
snapshot's plumbing spans more of the server's internals than that phrase implies, and every one
of those internals is named explicitly below (a buddy-check debate round caught the original text
under-specifying this; the plan-writer must not discover the gap mid-implementation).

**Everywhere `Command` is used as an internal transport — not just where it's first built — must
carry `StoredCommand` instead. Concretely, every one of these signatures changes:**

- `Repository::apply_command` / `Repository::apply_intent` — return `Result<StoredCommand,
  DataError>` (today: `Result<Command, DataError>`). Both trait implementations (`SqliteRepository`
  and any mock/test double — `ws/room.rs`'s test mock included) update together.
- `Repository::events_since` — return `Result<Vec<StoredCommand>, DataError>` (today:
  `Vec<Command>`, confirmed via the mock impl in `ws/room.rs`). Back-compat parsing (§5 below)
  lives here, at the read boundary.
- `Room.tx: broadcast::Sender<Arc<ServerMsg>>` — today ONE `Arc<ServerMsg>` is shared between the
  ring buffer AND the live-broadcast channel every already-connected session reads from
  (`commit_ops_locked` builds one `Arc<ServerMsg::Event>` and pushes it to both `self.ring` and
  `self.tx`). Since `ServerMsg` itself must stay wire-shaped, this single shared value can no
  longer carry the snapshot. Introduce a genuinely separate **internal-only** enum, e.g.
  `RoomEvent { Event(Arc<StoredCommand>), Other(Arc<ServerMsg>) }` (exact shape is the
  implementer's call — a newtype wrapping `ServerMsg` with the `Event` variant's payload swapped
  for `StoredCommand` is another valid shape), and change `Room.tx`'s element type,
  `RingBuffer.events`'s element type, and `commit_ops_locked`'s construction to build this instead
  of `Arc<ServerMsg>` directly, for the `Event` case. Every OTHER `ServerMsg` variant `Room`
  broadcasts (pings, presence, etc.) passes through this wrapper unchanged — do not widen the
  snapshot concept to messages that were never `Command`-shaped.
- `Egress::Frame`'s payload type (`ws/conn.rs`) and `send_filtered`'s signature — both currently
  typed to `&ServerMsg`; both must accept the new internal wrapper for the `Event` case (or two
  call shapes: one for `Event` frames carrying `&StoredCommand`, one pass-through for everything
  else) and reduce to a plain wire `ServerMsg::Event { command }` only at the point where the
  frame is actually serialized and sent — never earlier.
- `resync_range` (`Room`, the cold + hot resync path) — its hot branch reads `self.ring` (now
  `RoomEvent`/`StoredCommand`-shaped) and its cold branch reads `repo.events_since(...)` (now
  returning `Vec<StoredCommand>`) — both branches already return the SAME shape to the caller
  today (`(Vec<Arc<ServerMsg>>, ResyncSource)`); change that return type symmetrically so
  `send_filtered` sees one consistent shape regardless of which branch answered.

```
apply_command / apply_intent
  └─ mutation loop commits all ops in the command (existing, unchanged) — accumulates a
     HashMap<Uuid, Document> of final per-touched-doc-id post-images as it goes, last-write-wins
     on repeat (see Components §4 — this is NOT optional bookkeeping, it is what keeps the
     snapshot construction below from reopening the mid-command leak)
  └─ NEW: build CommandSnapshot from that accumulated post-image map (after the loop, once —
     never from an op's own per-iteration local `doc`)
  └─ StoredCommand { command: Command, snapshot: CommandSnapshot }
       ├─ Room: RoomEvent::Event(Arc<StoredCommand>) → ring buffer AND Room.tx (both, same value)
       └─ persisted → world_events.command_json (serde_json::to_string(&StoredCommand))

send_filtered (live delivery AND replay — same path, unchanged)
  └─ per recipient, per StoredCommand-shaped Event frame:
       filter_command(&stored.command, &stored.snapshot, ctx, current, ...)
       ├─ hidden_current  = collect_hidden(current_doc, access_current)                — TODAY's logic, unchanged
       ├─ hidden_commit   = collect_hidden(snapshot.overrides_at_commit, access_commit) — NEW
       ├─ hidden_effective = hidden_current ∪ hidden_commit
       └─ redact_change / retraction against hidden_effective → plain Command (wire-shape, unchanged)
  └─ every OTHER ServerMsg variant passes through exactly as today
```

## Components

### 1. `CommandSnapshot` (new type, `src/server/src/data/command.rs` or a new `redaction_snapshot.rs`
next to it — implementer's call, follow existing module boundaries)

```rust
/// Server-internal, never serialized to the wire. Index-aligned with `Command.ops` — built ONCE
/// from the whole command's post-image, after every op in the command has applied (never from an
/// op's own intermediate state — a per-op snapshot mid-command leaks values a LATER op in the
/// same command hides; see the findings doc's "leaks within a single command" finding, and
/// Components §4 for the accumulator that makes "post-image" a real, available value rather than
/// an assumption).
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
    /// Whether the recipient's role AT THIS OP'S COMMIT would have satisfied `see_gm_only` —
    /// i.e., "was there a GM (this recipient specifically) at commit time." See the world-role
    /// note below: this is a NEW field added after buddy-check review, not present in the first
    /// draft of this spec.
    pub gm_at_commit: bool,
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
    /// the FULL (unpruned) commit-time hidden-pointer set for the doc, WITH each pointer's
    /// `Visibility` tier retained (NOT a bare `Vec<String>` — see the retraction fix below), used
    /// to drive the retraction pass at REPLAY time using this command's own post-image, not
    /// whatever is live at replay time (each retraction-triggering command owns its own
    /// retraction moment). Filtered per-recipient via `can_see` at redaction time, exactly like
    /// `overrides_at_commit` — this is what keeps an owner's own `OwnerOrGm` fields from being
    /// wrongly nulled by a retraction pass that (before this fix) applied identically to every
    /// recipient regardless of their own access.
    pub retraction_hidden_at_commit: Option<Vec<(String, Visibility)>>,
    /// Present only for `Update`/`Delete` (ops addressing a PRE-EXISTING doc_id — `Create`
    /// establishes a fresh generation and needs no witness). The target document's
    /// `documents.created_seq` as read at commit time — compared against the CURRENT document's
    /// `created_seq` at redaction time; a mismatch means the id has been deleted and recreated
    /// since, and the op is DROPPED (not redacted-and-delivered against the wrong generation).
    /// Closes "Defect 2 is only half closed."
    pub created_seq_at_commit: Option<i64>,
}
```

**World role/`see_gm_only` MUST be snapshotted — corrected after buddy-check review; the first
draft of this spec got this wrong.** The first draft argued `see_gm_only` was safe to read live
for both halves of the conjunction, reasoning that a non-GM never sees `GmOnly` content regardless
of timing. Two independent reviewers found this false and, on debate, confirmed it against source:
`Access::can_see(OwnerOrGm) = self.see_gm_only || self.is_owner` (`permission.rs:556-562`) is a
**disjunction** — a recipient who is CURRENTLY a GM satisfies `can_see(OwnerOrGm)` regardless of
`is_owner`, which means reading `see_gm_only` live for the commit-time half doesn't just leak pure
`GmOnly` content on GM promotion, it also **short-circuits and defeats `owner_at_commit` itself**
for the `OwnerOrGm` tier — the exact tier `OPEN_BUGS.md` names as the ownership-reassignment
defect. A player promoted to GM and resyncing from before their promotion would see every
historical `GmOnly` AND `OwnerOrGm` value that was hidden from them specifically because they held
neither role at commit time. World role is mutable in production (`upsert_member`,
`sqlite.rs:1225`, `ON CONFLICT ... DO UPDATE SET role = excluded.role`) — this is reachable, not
theoretical. The first draft's justification ("mirrors the owner's 'world-grants... design note,
not a bug' ruling") was the spec author's own unreviewed extension of a ruling that was actually
scoped only to the `by_user` capability-grants map, never to `see_gm_only`/GM-tier membership —
stated here explicitly so it's clear this was corrected by review, not silently left as one
author's inference standing in for an owner ruling.

**Fix:** `OpSnapshot.gm_at_commit: bool` (above) — whether the SPECIFIC recipient held GM standing
in this world at commit time. Redaction-time `Access` for the commit-time half becomes `Access {
see_gm_only: op_snapshot.gm_at_commit, is_owner: (recipient == op_snapshot.owner_at_commit), .. }`
— fully commit-time, no live component. World GRANTS (the `by_user` capability map, as opposed to
world ROLE) stay current-only — that narrower scoping is the actual, correctly-scoped owner
ruling, unaffected by this correction. Recording `gm_at_commit` per-op means iterating the
command's post-image once against every CURRENT world member's role (small — world membership is
bounded, unlike documents) and capturing a `HashMap<Uuid, bool>` of "is this user GM as of this
command" alongside the doc-specific fields above; `filter_command` looks up the redacting
recipient's own entry. State this shape explicitly in the implementation plan — it's a genuinely
new per-command computation this corrected design adds, not a field rename.

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

Add parameters: `snapshot: &CommandSnapshot` and nothing else — no new live-state parameter, by
design (signature deprivation: the function that computes `hidden_at_commit` must be structurally
unable to take a repository/actor lookup, so a future "just add a quick current-state read here"
change is a loud diff, not a one-line addition; make this its own inner function if the module
boundary helps enforce it).

**A whole-document access gate applies uniformly to all three op kinds, evaluated BEFORE any
per-field logic** — corrected after buddy-check review found the first draft's asymmetry (Create/
Delete gated by commit∧current access, Update's top-level existence check left current-only)
reachable via warm resync (not just cold start — a full-state hydration on cold connect,
`query_documents_by_types`, sidesteps this, but `resync_range`'s warm-reconnect path is pure event
replay through this same function and IS reachable): a recipient denied at a document's Create
commit-time but currently permitted would have the Create dropped while a LATER Update to the same
doc_id was still forwarded — a client receiving field-level Updates for a document it was never
told exists, which is both a data-integrity problem and, for any `All`-visibility fields on that
Update, a genuine disclosure the recipient was never meant to have. The gate, per op:

- **`Create`**: forward only if BOTH commit-time access (`is_owner_at_commit` +
  `gm_at_commit`/caps evaluated against `op_snapshot.doc_type` and the doc's own carried body) AND
  current access (today's live check, unchanged) permit.
- **`Update`**: unchanged existence check stays (`current.get(doc_id)`, drop if absent). NEW: if
  `op_snapshot.created_seq_at_commit` is `Some(s)` and the current doc's `created_seq != s`, drop
  the op (id reused). NEW (the asymmetry fix): the SAME whole-document conjunction gate as
  `Create`/`Delete` — forward only if BOTH commit-time access AND current access permit at the
  document level — evaluated first, THEN per-field redaction proceeds for whatever passes:
  `hidden_current` exactly as today; `hidden_commit` via `collect_hidden`-against-
  `overrides_at_commit` using the corrected `Access { see_gm_only: op_snapshot.gm_at_commit,
  is_owner: (recipient == op_snapshot.owner_at_commit), .. }`; redact against `hidden_current ∪
  hidden_commit`. Retraction pass (when `touches_permissions`): use
  `op_snapshot.retraction_hidden_at_commit`, filtered through `can_see` for THIS recipient (using
  the same commit-time `Access` just constructed) — never applied as a flat, recipient-blind list.
- **`Delete`**: same `created_seq_at_commit` mismatch check against the CURRENT state at this
  `doc_id` (drop if a current doc exists and its `created_seq` doesn't match). Same whole-document
  conjunction gate as `Create`/`Update`.

**The current-doc lookup this needs for `Create`/`Delete` does not exist today — `load_update_docs`
is scoped to `Operation::Update` doc_ids only** (`permission.rs:159`,
`if let Operation::Update { doc_id, .. } = op`), confirmed by buddy-check review against source.
**Fix:** widen `load_update_docs` (rename to something that reflects the broader scope, e.g.
`load_current_docs`) to also load the current row for every `Create`/`Delete` doc_id in the
command — a `Create`'s doc_id is trivially known from the op itself; a `Delete`'s likewise. This
adds one additional per-recipient-independent (the load happens once per command, not per
recipient, matching today's existing hoist-before-redaction-loop shape) row read per Create/Delete
op in the command — state this cost plainly in the task brief rather than letting it surface as a
surprise during implementation.

### 4. `apply_command` / `apply_intent` (both authoritative write loops)

**The mutation loop must accumulate a `HashMap<Uuid, Document>` of final per-touched-doc-id
post-images AS IT RUNS, last-write-wins on repeat — this is not optional bookkeeping, and it is
not what the loops do today.** Corrected after buddy-check review found neither loop
(`apply_command`, `sqlite.rs:1936-2023`; `apply_intent`, `sqlite.rs:2424-2510`) retains one: each
iteration computes a local `doc: Document`, applies that op, upserts, and discards the variable —
the loop's existing shape has no map surviving past the iteration. Building `CommandSnapshot` from
each op's own local `doc` (the natural move given that shape) reproduces the "leaks within a
single command" defect (finding-9) this design's whole "snapshot once, after the whole loop"
architecture exists to prevent — a SECOND, independent path back to the same bug the design's own
prose already warns against once. State explicitly: add `let mut post_images: HashMap<Uuid,
Document> = HashMap::new();` before the loop, insert/overwrite at the end of each iteration keyed
by the op's target doc_id (a `Create`'s new id; an `Update`/`Delete`'s existing `doc_id`), and
build `CommandSnapshot` from THIS map after the loop — never from a per-iteration local.

Once the accumulator exists: compute `owner_at_commit` via `effective_owner_via` against the
post-image actor table (the same kind of lookup the loop already has access to — no new
live-state dependency, only its *result* is now captured instead of discarded); compute
`gm_at_commit` once per command (world membership is small — see Components §1) as a
`HashMap<Uuid, bool>`; read `overrides_at_commit`/`retraction_hidden_at_commit` from each op's
`post_images` entry (walk `property_overrides` + `embedded`, exactly `collect_hidden`'s existing
traversal, reused as a pure function of `(doc, prefix)` rather than `(doc, access, prefix)` —
split `collect_hidden` into a tier-agnostic "collect all overrides with their `Visibility`" pass,
and a small `can_see`-filtering pass over that output, so both the live path and the
snapshot-construction path share one traversal instead of duplicating it — see Testing below for
why this split is load-bearing, not just tidy); read `created_seq_at_commit` via the new
repository accessor for `Update`/`Delete` targets. Persist `StoredCommand` (not bare `Command`)
into `world_events.command_json` and into `Room`'s broadcast/ring path per the Architecture
section's full signature list above.

`apply_command`'s Create arm stays id-blind (upsert) — `created_seq` is correctly preserved
across an ON-CONFLICT hit by the SQL shape above regardless of this loop's trust level.

### 5. Back-compat (`world_events` rows written before this change)

`command_json` is opaque `TEXT` — no SQL migration needed for the JSON shape itself. On read (at
the `Repository::events_since` boundary — see Architecture), attempt
`serde_json::from_str::<StoredCommand>`; a pre-fix row (bare `Command` JSON, no `snapshot` key)
fails that parse. **Fall back to constructing `StoredCommand { command: <parsed as Command>,
snapshot: CommandSnapshot { per_op: vec![None; command.ops.len()] } }`** — never a live-lookup
fallback. Per the findings doc's explicit ruling, a `None` `OpSnapshot` means `filter_command`
DROPS that op on replay rather than falling back to today's (defective) current-state-only
redaction — a one-time, reviewed, accepted cost against pre-fix history, not a silent gap. State
this in the migration/rollout notes so it isn't mistaken for a regression when old worlds resync.

## Testing

- **Positive**: reproduce both `OPEN_BUGS.md` scenarios as failing-before/passing-after
  differential tests (property-tier narrowing then widening; ownership reassignment) — the exact
  discrimination-check discipline used throughout this campaign.
- **World-role promotion (the corrected finding)**: a player, hidden from a `GmOnly` and a
  separate `OwnerOrGm` field while a non-GM non-owner, is later promoted to GM and resyncs from
  before the promotion — both fields must stay hidden (the pointer was never theirs to see at
  commit time). Write this as a failing-before/passing-after differential against the FIRST
  draft's live-`see_gm_only` design, not just against the pre-fix baseline — it is the case that
  slipped through the first review pass.
- **Reused-id**: a document deleted then a new one created at the same id; a stale `Update` and a
  stale `Delete` from the old generation must both be dropped on replay, never applied to the new
  generation.
- **Cross-op existence consistency (the asymmetry fix)**: a recipient denied commit-time access to
  a document's Create, later granted current access, must ALSO have every subsequent Update to
  that doc_id dropped by the same whole-document gate — not just the Create.
- **Retraction correctness under replay, per-recipient**: (a) a command that narrows visibility,
  replayed long after a LATER command has narrowed it further — the retraction pass must reflect
  what the CHOSEN command itself hid (its own post-image), not whatever is live now or what a
  later command additionally hid. (b) the SAME retracting command, replayed to the document's own
  owner — the owner's legitimately-visible `OwnerOrGm` fields must NOT be nulled by retraction
  (this is the regression the first draft's flat `Vec<String>` shape would have caused).
- **Multi-op-in-one-command leak (the finding-9 case), both routes**: (a) `[Update{secret:=X},
  Update{add gm_only override}]` in ONE command — a non-GM/non-owner recipient must NOT see `X` on
  replay. (b) the SAME scenario via the accumulator specifically — a test that would fail if
  `CommandSnapshot` were (incorrectly) built from each op's own per-iteration local `doc` rather
  than the post-loop `post_images` map, proving the accumulator is load-bearing, not cosmetic.
- **Behavioural mutation test (per the findings doc's "signature deprivation" pairing)**: mutate
  each live input independently — the target's overrides, its default, the linked actor's owner,
  an embedded child's index, the world grants, the recipient's world role — and assert
  `filter_command`'s CURRENT-time output is unaffected by history and its COMMIT-time output is
  unaffected by anything live. The embedded-index case is the one a grep-shaped review could never
  catch on its own.
- **Traversal-split unit test**: the shared `(doc, prefix) → Vec<(String, Visibility)>` traversal
  used by both the live path and snapshot construction must produce byte-identical output for the
  same document — this is what keeps the two paths from re-forking (this codebase's own
  highest-frequency defect class) the moment one of them is touched later.
- **Wire-unaffected**: confirm `Operation`/`Command`/`ServerMsg`/`ClientMsg` and the Zod schema
  are byte-identical before/after (no ts-rs diff, no ts-rs regen needed) — this is the design's
  own claim and should be pinned, not assumed. This is narrower than "nothing server-internal
  changes" — the internal broadcast/ring/repository signatures DO change; only the client-facing
  shape is pinned here.
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

- World GRANTS (the `by_user` capability map) stay current-only at both halves of the conjunction
  — the actual, correctly-scoped owner ruling. World ROLE (`see_gm_only`) does NOT get this
  treatment — see Components §1's correction; it IS snapshotted.
- Audit-grade point-in-time replay (a queryable history) is its own later milestone per
  `docs/PLAN.md` — this phase is its prerequisite (the commit-time snapshot seam), not a
  duplicate.
- No client-visible change of any kind.
- **The recipient's role is resolved once at socket open and never refreshed mid-connection** (a
  demotion isn't honoured until reconnect) — pre-existing, separate from this phase's defects, not
  addressed here. Named explicitly (per the design-findings doc's own "worth naming" note) so its
  absence from this design isn't mistaken for an oversight.

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
- Exact shape of the internal `RoomEvent`-style wrapper distinguishing a `StoredCommand`-carrying
  broadcast from every other `ServerMsg` variant — read `Room.tx`'s full set of producers/
  consumers (not just `commit_ops_locked`) before committing to a shape, so no existing non-Event
  broadcast path is missed.
- `load_update_docs`'s widened scope (renamed or not) — confirm every existing call site of the
  current function is updated consistently, and that the added Create/Delete row reads don't
  regress a latency-sensitive path (state the measured or reasoned-about cost in the task brief).
