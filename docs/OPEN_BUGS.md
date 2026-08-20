# Open Bugs

Currently open, confirmed-real defects. Deferrals belong in `TODO.md`, not here.

- **`filter_command`'s `Update` arm and `collect_hidden` resolve replay visibility against a
  document's CURRENT permission set, not the policy in force at the historical seq being
  replayed.** `collect_hidden` derives the hidden-pointer set from
  `cur.permissions.property_overrides`, with no knowledge of what the override was when a given
  historical `FieldChange` was committed, and `redact_change` drops a change only if its path is
  CURRENTLY hidden. So if a pointer was `GmOnly` while its value changed several times, and a GM
  later makes it visible, every historical `FieldChange` for that pointer — including intermediate
  values never intended for release — replays unredacted once any recipient gains visibility of
  the current value. Reading the current value as public does not make its whole secret evolution
  public; those are different disclosures. The reverse direction (visible → later hidden) is
  correctly safe: those changes are dropped against current policy, which is over-redaction only.
  - **The same shape recurs for the `OwnerOrGm` tier under ownership reassignment:** a
    newly-assigned owner's replay discloses the previous owner's historical `OwnerOrGm` values.
  - **Reachability:** `world_events` has no compaction or expiry. Its rows outlive even the user
    who authored them: `world_events.author_id` is `ON DELETE SET NULL`, and
    `SqliteRepository::delete_user` never deletes event rows, so a deleted user's authored events
    persist with the author nulled. Only world deletion removes them, via
    `SqliteRepository::delete_world`'s `world_id` FK cascade. `ResyncRequest{from_seq}` is
    entirely client-supplied with no lower
    bound (`Room::resync_range` → `Repository::events_since` queries `seq > from_seq - 1`), so any
    client can pull a document's entire history at any time.
  - **Fix shape DECIDED (ruled, not yet built): snapshot the relevant visibility into the
    event/command at commit time**, so replay redacts against the policy in force at that sequence
    rather than against today's policy — the redaction decision is made once, at commit, and
    stored with the event, rather than re-derived on every replay: the same shape as any two paths
    required to agree deriving from one, instead of separately re-verifying agreement. Two other
    shapes were considered and rejected: an
    append-only "ever hidden" set permanently over-redacts history once a pointer is ever
    restricted; current-state snapshots for non-GM resync sidestep the problem rather than solve
    it, and change resync semantics for every document carrying an override.
  - **Scheduling:** its own phase — its own branch, its own brainstorm → spec → plan cycle,
    scheduled immediately after this phase merges and before the next. The next phase does not
    depend on it, but the fix changes the command representation, the event log, and resync, which
    is foundational enough that no later phase should be built on the current shape.

- **A stale `Update` from before a document's deletion is redacted against a NEW document that
  later reuses the same id, not dropped as the closing analysis assumed.** Document ids are
  client-supplied: `envelope` accepts an optional explicit id and falls back to
  `crypto.randomUUID()`. The two server-side authoritative write loops treat a reused id
  differently, and neither stops reuse: `SqliteRepository::apply_command`'s `Operation::Create`
  arm calls `SqliteRepository::upsert_document` with `ON CONFLICT(id) DO UPDATE` and performs no
  existence check at all — genuinely id-blind. `SqliteRepository::apply_intent`'s
  `Operation::Create` arm does check first — it loads the document by id inside the transaction
  and rejects a currently-live duplicate as a conflict ("Create is non-clobbering: an existing id
  is a conflict, not a silent overwrite (unlike upsert in apply_command)") — but that check only
  sees PRESENT table state: a hard-deleted id is absent from it, so reuse after
  `SqliteRepository::delete_document_tx`'s genuine hard delete passes the check exactly as a
  never-used id would.
  `permission::load_update_docs` builds the `current` map `filter_command` consults via a
  present-tense `get_document` lookup with no sequence parameter. Its call site,
  `ws::conn::send_filtered`'s Event branch, serves both live broadcast and historical replay
  (`conn::replay`, driven by `Room::resync_range`), and replay redacts every event identically to
  live delivery.
  - **Reachable sequence:** a user deletes their own document at some id, then creates an
    unrelated document that happens to reuse that id — an ordinary two-call sequence needing no id
    guessing and no cross-user interaction. A client resyncing through history then meets the
    stale `Update` for that id; `permission::load_update_docs`'s lookup now resolves to the NEW
    document, so the drop branch never fires and the stale op is redacted and delivered.
  - **What actually breaks — and what does not.** Final-state convergence DOES survive: the
    corrective Delete and Create frames follow in the same resync batch, so the client's
    persisted end state is correct — the original closing argument answered that question and it
    was the wrong one to ask. What fails is that the stale `Update` is redacted against the
    **wrong document's** permission set: in the window before the corrective frames land, a
    recipient can receive a field from the deleted generation that only its GM was meant to see
    (over-reveal), or have the update dropped entirely because the new document's owner differs
    from the old one's (under-reveal).
  - **Root cause shared with `filter_command`'s current-permission-set replay redaction:** both
    are a chokepoint needing point-in-time state — "what did this document's permission set look
    like when the historical event was committed" — served instead by a current-state lookup. The
    already-ruled remediation for that defect (snapshotting the relevant state into the event or
    command at commit time) is expected to close this one too; fixed together in the same phase
    rather than forked across phases.

- **`App.svelte`'s `boot()` can render one stale frame of `<Table>`'s "Connecting…" branch after
  falling back to `login` on a `#/world/<id>` deep link.** `navigate()` only writes
  `location.hash` synchronously; `currentRoute()`'s reactive `route` state updates asynchronously,
  off the `hashchange` listener (`route.svelte.ts`). `boot()`'s deadline-timeout callback and its
  `catch` block both call `navigate({ name: "login" })` and then set `booted = true` in the same
  synchronous tick — so on a page loaded at a world deep link, there is a one-tick window where
  `booted` is already `true` but `route.name` is still `"world"`, hitting the template's
  `{:else if route.name === "world"}` branch and rendering a stale "Connecting…" before the
  `hashchange` event lands and flips the view to `<Entry>`.
  - **Reachable sequence:** load the SPA on a `#/world/<id>` link with a backend that is down or
    unreachable for the whole boot window; either the deadline fires or the outer `getMe`/
    `loadSessionState` `catch` fires, both of which navigate to `login` — the flicker window is the
    same either way, since `navigate()`'s hash-write-then-async-listener structure predates the
    boot deadline and is not specific to it.
  - **Fix shape not yet decided:** either make `navigate()`'s effect on `route` synchronous
    (removing the `hashchange`-listener indirection for same-tick reads) or have `boot()`'s
    fallback paths await a microtask/the dispatched `hashchange` before setting `booted = true`.
    Needs its own investigation into every other `navigate()` call site before choosing, since the
    fix touches the router primitive itself, not just `boot()`.
