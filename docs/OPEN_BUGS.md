# Open Bugs

Currently open, confirmed-real defects. Deferrals belong in `TODO.md`, not here.

- **[a11y] A panel opened via `PanelsApi.open` changes placement without any screen-reader
  announcement.** `PanelHost`'s `describeOp` maps each layout-changing op to a `panels.moved` live
  region announcement and returns `null` for ops not worth narrating. `"open"` falls to the
  `default` arm and is never narrated, on the documented reasoning that no live path dispatches
  it. That reasoning is false. `PanelsApi.open` is public and has two reachable callers:
  `SceneBrowserPanel`'s per-scene configure button (which opens the game-settings panel) and
  `SheetsController.openDocument` (which opens a document sheet). Both route through
  `PanelsController.dispatch`, which fires `onOp`, which `PanelHost` feeds to `describeOp` —
  identically to every op that IS narrated.
  - **Why it matters:** `applyOp`'s `"open"` case is not merely a focus bump. Via
    `placeByPlacement` it can surface a minimized or closed panel into a docked group — a real
    placement change, exactly the class of change the live region exists to announce. A
    screen-reader user clicking "configure" gets no indication the settings panel appeared.
  - **Fix shape:** narrate `"open"` when it changes placement, distinguishing that from a focus
    bump within an already-open group or floating window (`locate` before/after the op answers
    this). Not fixed inline: found during a comment-only documentation sweep, which cannot carry
    behavior changes.
  - **Reachability/impact:** no data loss, no authz effect; degrades accessibility only. Both
    call sites are GM-reachable, and `openDocument` is reachable by any role.

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

- **`makeTemplateTool`'s near-zero-drag fallback effectively never fires in a snapping scene, so a
  plain click places an arbitrarily-sized template instead of the intended one-cell default.**
  `makeTemplateTool`'s `onPointerDown` snaps the anchor (`anchor = ctx.scene.snap(p)`) but its
  `onPointerMove`/`onPointerUp` pass the RAW pointer point to
  `sizeDir`. `sizeDir`'s fallback is `if (d < 1) return { size: cell,
  direction: 0 }`, with `d` the distance between those two points — so it fires only
  when the click lands within one scene unit of the snapped anchor. `Grid.snap` returns the cell
  CENTER on BOTH grid kinds, so an ordinary click sits
  some arbitrary distance from the anchor — for a click that presses and releases within one
  cell, bounded by that cell's circumradius, which is the half-diagonal on a square grid and on a
  hex grid `GridSpec.size` itself, the outer radius; a release outside the press
  cell is not bounded by it at all. It takes the normal branch and yields `size = d`, an
  arbitrary template rather than the intended one-cell default. The fallback is reachable only
  by a click landing almost exactly on the snap point.
  - **This is a defect, not a missing feature.** The `d < 1` branch exists precisely to turn a
    click into a real default-sized template rather than a degenerate one; it was written assuming
    both points share a coordinate frame. Mixing a snapped anchor with a raw pointer defeats its
    own stated purpose.
  - **Sibling divergence:** `makeTemplateTool` is the only one of the four authoring tools with no
    extent guard on persist. `makeDrawTool` gates on `hasExtent`, `makeWallTool` on a
    `>= 1` length check, `makeRegionTool` on its own `hasExtentForRegion` check.
  - **Reachability/impact:** GM-only (the `template` tool is `gmOnly`) and non-destructive — no
    data loss and no authz effect. Impact is nonetheless persistent: **no client code anywhere
    constructs an `Operation` with `op: "delete"`.** Outside tests that variant appears only in
    the SHARED wire type and schema (`Operation`'s "delete" variant,
    `OperationSchema`) and in `applyOperation`'s receive-side `case "delete"`. The schema is
    emphatically not receive-only — the
    client's own outbound `intent` frame is typed `ops: WireOperation[]` (in `ClientMsg`'s
    `"intent"` variant) and the
    server executes a client-sent Delete, which is exactly what makes the raw-protocol escape
    below real. That path is `Room::publish` (via `Room::commit_ops_locked`), whose
    `Operation::Delete` arm in `SqliteRepository::apply_intent` authorizes against the stored doc
    under `cap::DELETE` and then executes via `delete_document_tx`
    (called from `SqliteRepository::apply_intent`). Do NOT cite `apply_command`'s Delete arms for
    this: no client frame reaches
    `apply_command`, which is the trusted undo/replay substrate and deliberately does not
    capability-check descendants (`SqliteRepository::apply_command`). The gap is that nothing in the
    client ever CONSTRUCTS one. (Neighbouring `delete` names sit on other axes and are not
    counterexamples. Nearest first: chat's Delete button is the one user-facing document delete
    in the client, and it sends a dedicated `delete_message` frame the server applies as an
    `Operation::Update` tombstone via `handle_delete_message`, explicitly not a hard
    `Operation::Delete`; `unsetField` dispatches `{ op: "update", …,
    remove: true }`, a `FieldChange`-axis key removal, not a document Delete;
    `deleteAsset`/`deleteUser`/`deleteWorld` are REST calls against assets, users and worlds;
    `Diff`'s `"delete"` variant is a template-merge field op; and `deletePointer`/
    `removePointer` are pure JSON-pointer helpers.) So no scene-entity delete UI exists to
    remove the junk template; it
    persists until such a UI ships or a raw protocol Delete is sent. Cost is accumulating scene
    and event-log clutter plus a confusing authoring experience.
  - **Fix shape:** make the two `sizeDir` call sites agree on a frame — either snap the pointer
    point alongside the anchor, or compare the raw pointer against the raw pointer-down point.
    Then add the extent guard its three siblings already carry. Belongs on the runtime follow-up
    branch; found by the Sweep 11 whole-branch review,
    which is comment-only and cannot carry a behavior change.

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

- **The GM Settings "hyperlinks" checkbox is permanently non-functional on every world: it sends a
  coalesced OCC pre-image the server always rejects.** 100% reproducible, not a race.
  `GameSettingsPanel` passes `chatsys.hyperlinks ?? false` as the `old` argument to
  its local `set`. `set`'s own `old ?? null` cannot repair this — `false` is not nullish, so the
  coalesced value is forwarded verbatim as the pre-image.
  - **Why it fires on a fresh world.** `GameSettingsPanel`'s GM-seed effect creates
    the `chat-settings` doc with an explicit JSON `hyperlinks: null` — a literal stored null, not an
    absent key. So the first value ever at `/engine/hyperlinks` is `null`, while the checkbox sends
    `old: false`. **The null is guaranteed, not incidental:** ingress normalization re-serializes
    the typed struct, so an absent optional field is stored as an explicit null rather than a
    missing key — `normalize_engine_opt`, whose own
    doctest asserts exactly that for a `{}` chat-settings body. Seeding the field or
    omitting it therefore reach the same stored state, and no path leaves a `false` there for the
    checkbox's pre-image to match.
  - **Why the server rejects it.** `SqliteRepository::apply_intent`'s field-level OCC check
    computes
    `actual = whole.pointer(&ch.path).cloned().unwrap_or(Value::Null)` and compares it to `ch.old`
    through `values_semantically_eq`. That helper special-cases only Object/Object,
    Array/Array and Number/Number; a `Null` vs `Bool(false)` mismatch falls through to the catch-all
    `_ => a == b`, which is false. Result:
    `DataError::Conflict("stale pre-image at /engine/hyperlinks")`.
  - **Why it never self-heals.** A rejected intent mutates nothing, so the field stays `null` and
    every subsequent click fails identically. No other code path writes a real boolean there.
  - **It is the sole offender in the file.** Every other nullable control passes the raw value:
    `link_previews` uses `?? null`, scene overrides use `?? null`, and `lightingEnabled`
    passes the value directly because its field is a plain `bool` with no null case. `set`'s own
    docblock states the contract this one call site violates: `old` must be the field's real
    current value.
  - **Why the tests miss it.** The test "toggling hyperlinks dispatches a
    JSON-pointer update with the real pre-image" seeds `hyperlinks: false` — the one
    value for which `?? false` is a no-op. The `chatEngine` fixture's own default is
    `hyperlinks: null`,
    the broken case, and no test exercises it. Client tests mock `dispatchIntent`, so they prove
    what is SENT, never what the server ACCEPTS.
  - **Fix shape:** change the `old` argument to `chatsys.hyperlinks ?? null`. The
    `checked={chatsys.hyperlinks ?? false}` display expression is correct and stays — it
    mirrors `ChatContentPolicy::hyperlinks()`'s `unwrap_or(false)` on the read path. Add a test
    seeding `hyperlinks: null`. Runtime change; belongs on the follow-up branch with the
    `makeTemplateTool` fix above. Found during Sweep 12 Task 6 by the dispatcher and
    independently confirmed by both reviewers, from writing the doc comment that sits above it.

- **[hex] The lighting overlay and the explored-fog layer paint axial indices at square
  positions.** On a hex scene the server sends lit and explored cells as axial `(q, r)`, produced
  through `HexGrid`'s `GridShape` implementation. `PixiBackend.setLighting` places each at
  `x = i · cellSize, y = j · cellSize` and fills an axis-aligned rect; `cellsToRects`, which
  rasterizes the explored-memory layer, does the identical thing. Neither consults the grid shape,
  and the frame types carry a cell size with no kind for them to consult.
  - **Why the scene looks half-right:** grid lines, cursor snapping and measurement all go through
    `Grid`, which owns the correct axial math — privately, on the same `RenderEngine` instance.
    The currently-visible fog is correct by construction, because the server sends raycast vertices
    rather than cell indices. So correctly-drawn hexes sit under skewed square overlays.
  - **Impact:** rendering correctness on every hex scene. The overlays misrepresent which cells are
    lit and which are remembered, which is misleading rather than merely ugly — but the underlying
    masks are correct, so nothing is disclosed that should not be.

- **[hex] A token is drawn and collided as something smaller than the hex it occupies, by two
  different factors.** `resolveTokenBox` sizes the drawn footprint as `actor.size.w * cell` by
  `actor.size.h * cell`, where `sceneCellSize` supplies the scene's `grid.size` — on hex the cell's
  CIRCUMRADIUS. Separately `footprintRadius` reduces the same authored size to a bounding-disc
  radius in grid units, which the server multiplies by the same scalar for `r_scene`.
  - **Measured, for a 1×1 token at hex circumradius `size`:** drawn box is `size × size` where the
    hex spans `√3·size` wide by `2·size` tall; collision radius is `0.707·size` against a hex
    inradius of `0.866·size`. Undersized on both, and the two do not agree with each other.
  - **Consequence:** a token under-fills its hex visually, and gaps a hex would block stay passable.
    `topTokenAt`'s hit test and `drawSelection`'s ring read the same resolver, so click targeting and
    the selection ring inherit it — consumers of one defect, not three.
  - **The obvious fix is wrong:** substituting the per-step distance for the indexing scale yields a
    `√3·size` SQUARE — right width, wrong height — because a hex's width and height are not in the
    same ratio. Any single-scalar substitution is wrong before it starts.
  - **Ruled:** a token's authored size counts HEXES, and the drawn box and collision footprint derive
    from ONE resolved geometry rather than two formulas kept in agreement by review. The collision
    disc circumscribes (fail-closed for a movement gate), extending `footprintRadius`'s existing
    "conservative enclosure" convention rather than contradicting it.
  - **Impact:** movement-gate geometry, so this one DOES have a gameplay dimension the other cell-
    scale defects lack — tokens will refuse gaps they previously passed. Correction, not regression.
