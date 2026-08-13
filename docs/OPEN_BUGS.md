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

- **A stale `Update` from before a document's deletion is redacted against a NEW document that
  later reuses the same id, not dropped as the closing analysis assumed.** Document ids are
  client-supplied: `envelope` accepts an optional explicit id and falls back to
  `crypto.randomUUID()`, and both server-side authoritative write loops
  (`Operation::Create`'s arms in `SqliteRepository::apply_command` and
  `SqliteRepository::apply_intent`) accept it verbatim — their validation covers scope, system
  size, property overrides, engine tree, system schema, self-parent and capabilities, never the
  id. `SqliteRepository::upsert_document` inserts with `ON CONFLICT(id) DO UPDATE`, so nothing on
  the Create path checks whether an id was ever used before, and `SqliteRepository::delete_document_tx`
  performs a genuine hard delete, freeing the id for reuse.
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

- **`property_overrides` keys are not restricted to the four egress-special-cased fields; a
  self-targeting `/permissions` key silently substitutes the fail-closed default permissions
  object for a redacted viewer.** `validate_property_overrides`
  (gated on both Create and Update ingress inside `SqliteRepository::apply_intent`) checks only
  that a key is a well-formed non-empty JSON pointer (starts with `/`, no trailing `/`) — nothing
  restricts which top-level `Document` field it names. `filter_properties` special-cases only
  `/system`, `/engine`, `/name`, `/base` (nulled in place); any other hidden `property_overrides`
  pointer — including one naming `/permissions` or `/permissions/...` itself —
  falls through to the generic `strip_pointer`, which
  does a plain `Map::remove` on whatever top-level key the pointer names. For a single-token
  `/permissions` pointer, this removes the entire `permissions` object from the serialized document
  before `serde_json::from_value` re-deserializes it. Because `Document.permissions` carries
  `#[serde(default)]` and `PermissionSet` derives
  `Default` with `default: DocRole::None` (via `DocRole`'s own `Default` impl —
  fail-closed), re-deserialization does not panic; it silently substitutes the fail-closed default
  `PermissionSet` for the real one.
  - **A NESTED `/permissions/...` key is worse: it PANICS the request.** `PermissionSet`'s `default`,
    `users` and `property_overrides` fields carry **no** `#[serde(default)]` — only `capabilities`
    and `gm_role` do. So an override naming
    `/permissions/default`, `/permissions/users` or `/permissions/property_overrides` strips a
    REQUIRED field while the enclosing `permissions` object survives, leaving a value that cannot
    deserialize as `PermissionSet` — and the tail of `filter_properties` is
    `serde_json::from_value(whole).expect("filtered document deserializes")`. The `expect` is not a
    cold-path assertion:
    `filter_properties` runs per-recipient on the WS broadcast egress path (`filter_command`),
    on FTS search hits (`SqliteRepository::search`), and on the HTTP get-document routes
    (`list_documents` and `get_document`). Any recipient who cannot see the offending tier
    crashes the request handling their read — i.e. a denial-of-service against every such reader of
    that document, authorable by one holder of `cap::EDIT_PERMISSIONS`.
  - **Reachability:** requires `cap::EDIT_PERMISSIONS` on the document's `doc_type` — every GM has
    this; a non-GM could hold it only via an explicit `by_role`/`users` capability grant. No UI path
    in this codebase constructs a `property_overrides` key outside `/system`, `/engine`, `/name`,
    `/base` today; a raw protocol Update/Create message is not otherwise blocked from doing so.
  - **Effect:** a viewer who cannot see the offending override tier receives a document whose
    `permissions` field is the fail-closed default rather than the real one — a data-integrity
    defect (e.g. a client computing `isHidden`-style checks from the received `permissions` would
    misreport). **Not an authorization bypass**: write authorization always re-resolves server-side
    against the stored row, never against a redacted client-facing copy, and the substituted default
    is strictly more restrictive than the real value, never less.
  - **Fix shape DECIDED (user-directed, 2026-08-01): redaction operates on content bands, never on
    the envelope.** `Document`'s fields split into four CONTENT bands — `name`, `engine`, `system`,
    `base`, already the exact set `filter_properties` special-cases
    and the exact set `required_cap_for_path` maps to
    `cap::WRITE_FIELDS` — and the STRUCTURAL remainder (`id`, `scope`, `doc_type`,
    `schema_version`, `source`, `owner`, `permissions`, `parent_id`, `embedded`, `created_at`,
    `updated_at`), which nothing may redact. Three parts:
    1. **One shared classifier** in the `data::permission` module — `REDACTABLE_BANDS: [&str; 4]` plus
       `redaction_target(pointer) -> Option<RedactionTarget>` returning `Band` (null in place —
       today's four-arm match) or `Within` (`strip_pointer`, now provably landing inside an
       untyped `serde_json::Value` or an `Option`, never a required field). Ingress and egress
       currently duplicate the judgement of what a pointer means; this panic is what that fork
       looks like when it drifts, so the two paths must read ONE symbol, not agree by inspection.
       `collect_hidden` uses it too, so the change-delta path cannot diverge from whole-document
       egress.
    2. **Ingress rejects an unclassifiable pointer.** `validate_property_overrides`
       keeps its well-formedness checks and adds the
       classifier, at both existing call sites in `SqliteRepository::apply_intent` (Create and
       Update). `/permissions`, `/permissions/default`, `/owner`, `/id`,
       `/embedded/items/0` all become `DataError::BadPath`.
    3. **`filter_properties` returns `Result<Document, RedactionError>`**, deleting both
       `.expect()`s. Callers fail CLOSED: `filter_command` drops delivery to that recipient;
       `get_document`/`search` error rather than ship a half-redacted document. The whitelist
       alone closes the reachable bug; the `Result` covers what a whitelist structurally cannot —
       a band added to `Document` without updating the classifier, or a future nested pointer
       landing in a required field. A secrecy gate that meets an input it cannot classify must
       withhold, never panic and never guess (same posture as the fog invariant).
    **No migration and no compatibility shim**: no worlds or users exist yet, and every
    `property_overrides` key constructed anywhere in the repo — server, client, and tests — is
    already inside the whitelist (`/name`, `/engine`, `/engine/vision`, `/system/*`; verified by
    repo-wide grep 2026-08-01).
    **Tests required:** per-pointer ingress rejection for each envelope field; acceptance for the
    four bands and their nested forms; a regression test that the exact `/permissions/default`
    input returns `BadPath` instead of panicking; and a mutation check that removing a band from
    `REDACTABLE_BANDS` fails the suite — a parity test that passes because both paths are wrong
    the same way proves nothing.
    **Scheduling:** own branch, after Sweep 11 merges — a server fix does not belong batched into
    a docs sweep.

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
    branch with the `property_overrides` fix above; found by the Sweep 11 whole-branch review,
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
    seeding `hyperlinks: null`. Runtime change; belongs on the follow-up branch with
    `property_overrides` and `makeTemplateTool`. Found during Sweep 12 Task 6 by the dispatcher and
    independently confirmed by both reviewers, from writing the doc comment that sits above it.
