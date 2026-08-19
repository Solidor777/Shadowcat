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

- **A WS connection that misses an `AssetChanged{replaced}` frame keeps a stale image forever, with
  no self-healing path — and an ordinary reconnect is enough to miss it.** `Room::broadcast_aux`
  sends AssetChanged out-of-band: it does not push to the
  ring or bump `current_seq`. **Two independent paths reach the identical failure**, and the
  reconnect one is far more common than the lag one:
  - **Plain reconnect (dominant).** `Room::subscribe` returns
    `self.tx.subscribe()` — a fresh `broadcast::Receiver` positioned at the channel's CURRENT TAIL —
    and every new connection calls it, inside `handle_socket`. A client whose socket drops
    for any reason (network blip, laptop sleep/wake, `WsClient.scheduleReconnect`'s own backoff)
    during
    the window a GM replaces an asset comes back subscribed past the frame. No lag, no buffer
    overflow, no unusual load. The `Welcome`/ring resync that follows cannot supply it, because the
    frame was never in the ring.
  - **Broadcast lag.** When a connection falls behind, the egress loop's `Err(RecvError::Lagged(n))`
    arm (inside `egress_loop`) resyncs by calling `replay` against the ring/log
    tiers — which never held the aux frame. The connection is NOT torn down, so the client's
    `AssetResolver` survives with its counter unbumped.
  - **Why nothing recovers it.** `AssetResolver.revs` is a client-local map incremented only by
    `AssetResolver.onAssetChanged`; `AssetResolver.url` appends it as `?v={rev}`
    and reads the asset's server-side `version` nowhere. A missed frame therefore
    leaves the serve URL byte-identical, so no new request is issued at all, so the
    `"{id}-{version}"` ETag built in `serve` is never
    revalidated, and the unchanged URL may additionally be served from cache (`serve` sends no
    `Cache-Control` or `Last-Modified`, so browser behavior here is heuristic and not itself
    load-bearing to this bug). The same lost frame also skips the `items` reload and the render
    re-reconcile that
    `RenderEngine.reconcileNow` documents as required for out-of-band notices.
  - **Reachability/impact:** routine, not load-dependent. The lag path needs a receiver to fall
    past the broadcast channel's capacity (the window `lagged_drops` counts), but the reconnect
    path needs only a dropped socket coinciding with a GM's byte-replace — everyday mobile/sleep
    flakiness is sufficient. Triage this as "happens occasionally in normal use", not "needs
    sustained heavy load". No data loss and no authz effect: `serve` is gated on world membership,
    not per-asset ACL, and a replace changes only bytes and `version`, never permissions — so the
    stale view is strictly the pre-replace image that client was already entitled to see. It
    persists for that connection until a page reload.
  - **Fix shape:** make the cache-bust derive from the asset's authoritative `version` rather
    than a local counter — e.g. carry `version` in the AssetChanged frame and have `AssetResolver.url`
    fall
    back to the version last seen in a document/asset listing, so a resync repairs it. Sending
    AssetChanged through `publish` instead would also fix it but costs a world seq per byte-swap,
    which the `replace` route is deliberately exempt from. Found by the
    Sweep 12 Task 6 Rule 11 dimension pass, which is comment-only and cannot carry the change.

- **`check-comment-refs.mjs`'s "unnamed spec reference" detector does not catch "the brief"/"this
  brief" as the same class of ephemeral-document referent it already catches for "spec".** The
  pattern (`\b(?:the|this|design|parent|wire|per)\s+spec\b|\bspec'?d\b|\bspec\s*§|\bspec\s*:(?!:)`)
  is keyed on the literal word `spec`; "brief" never appears in its vocabulary. RULE 16 bans
  "unnamed spec references" as one instance of a broader class — any reference to an ephemeral
  planning document whose identity a process assigns — and a dispatcher brief is exactly that class
  of document, used throughout this project's own subagent-dispatch workflow (files literally named
  `task-N-brief.md`). A committed test comment reading "...exactly the fixture the brief calls for"
  passed `pnpm lint:comments` cleanly, both before and after being written, confirming the gap by
  direct observation rather than by reading the pattern alone.
  - **Reachability:** any comment written by (or dictated to) a subagent that refers to its own
    dispatch brief by that name, which is common — dispatcher briefs across this project's own
    subagent-driven-development workflow are routinely called "the brief" in prose.
  - **Fix shape:** widen the pattern to also match `(?:the|this|per)\s+brief\b`, mirroring the
    existing `spec` pattern's determiner set and word-boundary care (avoid the false-positive risk
    of "brief" as an ordinary adjective — "a brief pause", "keep it brief" — which the determiner-
    gated shape already avoids for equivalent `spec` cases). Needs the same population-enumeration-
    by-reading and positive/negative-control test discipline this file's other detectors already
    carry; not a one-line patch without that verification.
