# Post-Work Findings

Living record of issues surfaced during review/audit. NOT a to-do list — entries
are observations awaiting triage, not committed work.

- Title: M10e-2 environment light is flat ambient, not edge-projected. Summary: the M10e spec
  (§6/§12.5) specifies environment light as edge-projected + `blocksLight`-occludable, but the scene
  model is dimensionless (dimensions deferred), so there is no boundary to project edge light from.
  `player_lit_mask` implements environment as a flat scene-wide ambient floor; placed-light
  `blocksLight` occlusion IS implemented. Inert by default (`env.intensity` = 0.0). Status: Accepted
  (constraint-forced; logged to TODO.md `Server / scene-vision`, revisit with scene dimensions/M12).

- Title: M10e-2 vision-mode entry missing `illuminationFloor` is silently dropped. Summary:
  `resolved_vision_modes` skips a mode entry that lacks `illuminationFloor` (fail-closed; mirrors the
  client `resolveVisionModes`, which also does no per-mode validation). A typo'd floor key silently
  removes that mode from the resolved registry with no diagnostic. Status: Accepted (client parity;
  add GM-facing validation/warning if authoring friction surfaces).

- Title: M10e-4 dark scene under `movementRestriction:"visible"` freezes non-GM movement — INTENDED.
  Summary: a fresh scene (default `lightingEnabled:true` + `environmentLight` + `environment.intensity:0.0`,
  no placed lights) has an empty lit mask, so EVERY non-GM move's supercover fails the gate. This is the
  DESIRED behavior (user-confirmed): a player who cannot see a cell must not be able to move into it.
  A GM enables movement by lighting the scene (placed lights / `globalIllumination`) or setting
  `revealed`/`unrestricted`. Status: Resolved — working as designed. Do NOT "fix" this by softening the
  defaults; the freeze is the correct fail-closed outcome.

- Title: M10e-4 lenient-mode near-corner move can be spuriously rejected.
  Summary: `supercover_cells` uses a magnitude-relative epsilon to detect exact lattice-corner
  crossings and emit BOTH flanking cells (no thin-line slip). It can OVER-fire on a near-corner the
  true segment doesn't exactly cross, emitting an extra flanking cell; if that cell is dark, a legal
  player move whose path merely grazes a corner is rejected. The direction is fail-safe (over-include
  ⇒ reject a fine move, never admit a forbidden one — security is preserved), but it is a rare
  player-visible false-reject. Status: Accepted (security-safe). Revisit only if reports surface;
  a future tightening to the exact crossed-cell set would remove it.

- Title: offline-intent flush can precede the async `#onWelcome` body on reconnect.
  Summary: `WsClient` fires `onResyncComplete` (→ `WorldSession.#flushOfflineQueue`)
  synchronously on the caught-up Welcome branch / on `resync_end`, while
  `#onWelcome` runs as an unawaited `void` async (it awaits a member fetch before
  re-establishing scene subscriptions). So queued offline intents can transmit
  before scene subs re-establish. Not a correctness defect: flushed intents reach
  the server regardless, and scene-derived read state is eventually consistent via
  the egress re-evaluation debounce; FIFO confirm-correlation is unaffected.
  Status: Accepted (eventually-consistent ordering). If a stricter ordering is ever
  needed, gate the flush on an "onWelcome settled" promise.

- Title: capability model — `core:delete` is GM-only by default (behavior change
  from M5). Summary: the capability floor grants Owners `core:read` +
  `core:write_fields` but NOT `core:delete`, so a document Owner can no longer
  delete by default (M5's binary `can_write` allowed it). Intended per the
  capability spec; grant `core:delete` per-document or via a world default to
  restore owner-delete. Status: Accepted (documented behavior change).

- Title: capability model — grants can target `DocRole::None`. Summary: a GM may
  add capabilities to the `None` (no-access) role via `by_role`, widening what
  the floor denies. GM-authored only (not an escalation), and a coherent way to
  raise the default tier; recorded as intentional flexibility rather than
  restricted (restricting only the world-defaults endpoint's `validate_grants`
  would be inconsistent — per-document grants set at create / via PATCH
  `/permissions` bypass it). Status: Accepted (design note from Phase 1 review).

- Title: a saturated lagged WS connection is slow to auto-converge on the
  ubuntu-latest CI runner. Summary: `converges_with_publishing_during_resync`
  originally asserted the deliberately-lagged client reached the tail seq (300)
  in real time while the publisher ran concurrently. On ubuntu-latest the lagged
  connection delivered a contiguous-but-incomplete prefix (e.g. 1..234) and then
  emitted nothing for >10s — even after an explicit `ResyncRequest` on that same
  connection (zero frames). A fresh connection's `ResyncRequest` converges fine
  on the same runner (`all_clients_converge_after_reconnect` passes), so the
  durable resync path is sound; the symptom is auto-convergence latency/stall on
  an already-saturated lagged egress under Linux scheduling. The test now asserts
  the load-bearing invariant (no DROPS during the overlap → contiguous prefix)
  plus full recoverability via a fresh client. Status: Needs triage — determine
  whether the lagged egress genuinely stalls (a latency bug in the egress
  select/replay loop under heavy backpressure) or it is purely CI-runner
  saturation; reproduce with a constrained-CPU local run before changing
  `conn.rs`. Update (M8b-1 push, 2026-06-22): a *second* manifestation observed —
  the authoritative-seq assertion at `ws_convergence.rs:408`
  (`h.authoritative_seqs().last() == Some(300)`) failed `Some(277)` on
  ubuntu-latest after the test's 30s drain-wait budget (300×100ms), with the whole
  test taking 45s; i.e. even the *server-side* single-writer ingress→apply of 300
  queued intents didn't finish in 30s under runner saturation. Passed on
  windows+macos in the same run and locally (4.5s); cleared on job re-run.
  Unrelated to M8b-1 (the failing assertion is on DB ingress throughput, which
  M8b-1 does not touch). If it recurs, widen the drain budget (e.g. 600×100ms) or
  gate the count on a constrained-CPU repro before touching the ingress path.
  Update (2026-06-23) — **RESOLVED.** Recurred on ubuntu-latest even at the widened
  600×100ms=60s budget (`Some(289)` vs `Some(300)`, 79s test): a 300-intent ingress
  backlog genuinely cannot drain within budget on a saturated runner. Root cause is
  the test's *volume*, not a correctness defect — forcing the slow client's resync
  over a real socket was never deterministic (OS buffering absorbs a non-reading
  client; same non-portability resolved for the `Lagged` test in commit 2acf9f7), so
  the high count bought no coverage while loading the single-writer ingress past CI
  capacity. Fix: `converges_with_publishing_during_resync` now uses a modest
  `TOTAL=100` and asserts only its load-bearing invariants (no drop/reorder under
  concurrent publishing + full fresh-client resync recovery); the deterministic
  broadcast-`Lagged` → resync path is unit-tested against `egress_loop` with a
  credit-gated sink (`ws::conn::tests::egress_lag_triggers_resync_and_converges`).
  Full ws_convergence suite now ~2s locally.

- Title: `filter_command` redacts replayed history against the *current*
  PermissionSet. Summary: `src/server/src/data/permission.rs` loads each
  `Update` op's document via `get_document` to resolve visibility, so on
  resync/replay a property whose `GmOnly`↔`All` visibility was flipped after the
  event is redacted under the *new* policy, not the policy in force at the
  command's seq. Acceptable for M5 (visibility flips are rare; replay is
  recovery, not audit) but the redaction is not point-in-time faithful. Status:
  Needs triage — if audit-grade replay is ever required, snapshot the relevant
  permissions into the event or attach them to the broadcast.

- Title: an `Update` to a since-deleted document is silently dropped on replay.
  Summary: `filter_command`'s `Update` arm does `let Ok(Some(cur)) =
  get_document(..) else { continue }`; if the doc was later deleted the op is
  skipped. seq/command ordering is preserved and the later `Delete` still
  replays, so final-state convergence and the sequence guard are unaffected — a
  client just sees Create → (missing Update) → Delete. Harmless for end state;
  noted as a replay-fidelity limitation. Status: Accepted.

- Title: no smaller "caption" text-size token in the M7d token set. Summary: the
  M8b-2 asset panel's tile filename (`Assets.svelte` `.name`) renders at inherited
  body size — `_primitives.scss`/`_semantic.scss` define `--space-*`, `--radius-*`,
  `--font-sans`, and `--text-*` *colors* but no smaller font-*size* token (the plan's
  assumed `--text-sm` does not exist). Captions/secondary labels therefore can't be
  visually de-emphasized by size via a token. Status: **Deferred to M12** by the M8c-2
  §10 re-audit — canvas chrome (the M8 audit's scope) renders no text, so a font-*size*
  scale is out of scope here; it belongs with the text-dense default sheets/browsers in
  M12 (the second token re-audit point per `PLAN.md` M7).

- Title: M8c-2 §10 canvas-chrome token re-audit (outcome). Summary: re-audited the M7d
  3-tier token set against the first rendered canvas chrome. (1) Added a semantic
  `--grid-line` token (= `--slate-700`) so the canvas grid is decoupled from UI
  `--border`. (2) Fixed a latent M8c-1 bug: `Stage.svelte`'s `readColor` used
  `getComputedStyle().getPropertyValue("--token")`, which returns the unresolved
  `var(...)` string for aliased custom properties — so the grid silently used its
  fallback color and ignored the theme; it now resolves the real color via a
  computed-`color` probe. (3) Background uses `--surface-base` (already correct). (4)
  Fog-state colors (dimmed/unexplored) deferred to M9 (no visible fog in identity mode).
  Status: Resolved for M8c (canvas chrome); caption size token → M12 (above).

- Title: M10e-1 config-doc seed races resync (rare double-create). Summary: contribution
  panels (`GameSettingsPanel`, like `FactionsPanel`/`ConditionsPanel`) seed world config-docs
  from a reactive `$effect` that mounts during `#onWelcome` BEFORE the resync stream populates
  the optimistic store (`ws-client.ts`: welcome → onWelcome+module-activate → resync_request →
  event frames → resync_end). The `createSubscriber`+`subscribe()` reactivity + per-doc-type
  `length === 0` guard make a duplicate seed rare, but a GM whose first effect run lands with an
  empty store before resync can still create a duplicate `world-settings`/`light-gradation`/
  `vision-modes`. This is the SAME project-accepted condition as the `worldSession` scene
  auto-create (`worldSession.svelte.ts` "rare multi-GM ... double-create is accepted (M12
  dedupes)"). Status: **Accepted / deferred to M12** (singleton-config dedup). Not a regression.

- Title: M10e-1 world-defaults editor exposes a subset of `WorldSceneDefaults`. Summary: the
  `game-settings` world-defaults panel authors movement-restriction, lighting-enabled, light-mode,
  diagonal-rule, and animation only. `losRestriction`/`fog`/`observerVision`/`partialCellLeniency`
  and the world-level `environment` are present in `DEFAULT_WORLD_SETTINGS` (resolve correctly) but
  are authorable only as PER-SCENE overrides, not as world defaults. Matches the M10e-1 plan (Task
  6 scope); flagged so the M10e-2+ consumer knows world-level toggles for those axes are not yet in
  the UI. Status: Intentional V1 scope; revisit if world-level authoring of those axes is needed.

- Title: M10e-3 lighting soft edges via blur, not gradients. Summary: the lighting layer softens
  band/edge boundaries with a single Pixi BlurFilter; per-cell radial gradients (crisper falloff)
  were deferred. Status: Revisit (cosmetic; client-render-only).

- Title: M10e-3 darkvision render is an overlay approximation. Summary: darkvision-only cells get a
  low-alpha neutral gray wash; true desaturation needs a masked ColorMatrixFilter over the scene
  layers. The wire payload already carries the faithful per-cell renderHint, so the refinement is
  client-render-only (no server change). Status: Revisit.

- Title: Route stricter than the authoritative gate (footprint vs center-based). Summary: M10e-6's
  previewed route enforces full geometric footprint clearance (`cell_enterable` — the token's
  bounding-disc must clear all `blocksMove` segments and ALL footprint cells must be in the
  non-GM mask); the authoritative movement gate (M9/M10e-4) stays center-based (parent spec §14).
  A wide token can therefore be dragged (gate allows the center path) along a path the router
  refuses to preview through a narrow gap. This is the intended asymmetry: route ⊆ gate-allowed
  keeps the preview from suggesting a move the router would reject, while never admitting a move the
  gate would block. Not a bug. Status: Recorded; revisit when footprint-aware authoritative blocking
  lands.

- Title: Multi-leg alternating parity is per-leg-greedy (cost-display only). Summary: `find` threads
  each leg's min-cost end-parity into the next leg's start; for the `alternating` (5-10-5) rule this
  is not guaranteed globally cost-optimal across waypoints — a different parity threading could yield a
  lower total cost across the full multi-leg route. This is a cost-DISPLAY inaccuracy at waypoint
  boundaries only; the route itself remains valid (footprint-clear, mask-bounded, gate-passable). The
  spec §4.2 requires parity carry across legs, not global optimality. Documented in-code. Status:
  Recorded.

- Title: M12a module-API friction — `register()` cannot reach AppContext, forcing lazy controller
  construction. Summary: `Module.register(ctx)` runs in the framework-neutral `ModuleContext`
  (no role, no `uiState`, no `PanelsBridge`), so the panels module cannot construct its
  layout-owning `PanelsController` at registration; `PanelHost` builds it lazily at mount from
  AppContext and binds it into the shell's `PanelsBridge`, and the chip strip must read the SAME
  bridge reactively instead of holding a controller. Workable (documented in
  `panels/src/index.ts`), but every future stateful module that needs session context will repeat
  this construct-at-mount + bridge dance — treated as an API bug report per the M12 "built against
  the public API" rule. Status: Needs Review (candidate: a session-scoped module hook or
  context-bearing activation phase, weigh at M12c sheet-registry design time).

- Title: M12a verification gap — native pointer tab-drag not exercisable by automation. Summary:
  Task 10's sweep could not fire a REAL native drag of a dockview tab (CDP does not synthesize
  `dragstart`; the coarse-pointer backend engaged dockview's drop overlay but committed no move).
  Coverage substitutes the `PanelMenu` a11y commands, which share the identical classified-
  `LayoutOp` pipeline (parity documented in `dockview.ts`), plus geometric assertion of the zone
  move through a reload; drop-POSITION classification fidelity against real pointer geometry
  (edge vs center vs tab-strip index) remains manual-QA-only (also logged as the
  `#toDropSite` fallback TODO). Status: Needs Review (a human mouse pass over dock/float/reorder
  gestures would close it; a synthetic-DragEvent harness is a possible future e2e investment).

- Title: M12b compact toolrail DOM order precedes main content. Summary: with the compact toolrail
  no longer display:none (M12b Task 1), markup order (topbar/toolrail/main/statusbar) diverges from
  compact visual order (topbar/main/toolrail/statusbar) — tab/reading order hits toolrail controls
  before panel content. Grid areas own visual placement in BOTH modes, so reordering the markup
  (toolrail after main) fixes tab order without visual change. Status: Resolved — M12b Task 6
  (commit fce8910) reordered the markup; grid areas kept both visual layouts identical.
  (M12b Task 1 code review.)

- Title: M13-0 Task 3 — message doc `/engine` copy goes stale on edit/delete (interim, pre-Task-7).
  Summary: `build_message_doc` (`chat/mod.rs`) intentionally writes the SAME `MessageSystem` body
  into both `/system` and `/engine` — the `system` copy stays load-bearing because chat reads still
  deserialize `cur.system` until Task 7 re-roots them, so setting `system: {}` now would break live
  chat. `handle_edit_message`/`handle_delete_message` only construct a `/system` `FieldChange`
  (`WriteOrigin::ServerMessageRevision`); they never touch `/engine`. So a message's `/engine` copy
  reflects only its ORIGINAL Create body — any subsequent edit or soft-delete leaves it stale,
  diverging from the authoritative `/system` body. Nothing reads the message `/engine` band today
  (dead data until Task 7), and this is pre-v1 with a zero-migration policy (no shipped worlds), so
  no cleanup is required — Task 7 must simply not trust any pre-Task-7 `/engine` copy of a
  previously-edited message when it re-roots chat. Separately: `apply_command`
  (`data/sqlite.rs::apply_command`) — an ungated trusted substrate with zero production callers
  today — does NOT carry the same broadcast/event-log `/engine` normalization gate added to
  `apply_intent` by this fix; if `apply_command` is ever wired to real undo/replay functionality,
  it must gain the identical gate first. Status: Resolved — M13-0 Task 7 re-rooted both chat READS
  and WRITES onto `/engine` (`handle_edit_message`/`handle_delete_message` now construct an
  `/engine` `FieldChange`, not `/system`; `build_message_doc` writes `system: {}`); `/engine` is now
  the sole source of truth and the staleness window is closed. Re-confirmed at Task 7: `apply_command`
  still has zero production callers (grep across `src/server/src`), so it was not accidentally wired
  into the chat re-root; its missing `/engine` normalization gate remains inert until a real
  undo/replay caller is added, at which point it needs the same gate as `apply_intent`.

- Title: Movement gate: token_move gate-dispatch is opt-in on ECS hydration (fail-open shape,
  reachability unconfirmed). Summary: `Room::publish`'s per-operation movement gate only runs when
  `SceneEcs::token_move` returns `Some(...)`, which requires the token to already exist in the
  hydrated in-memory ECS (`self.index.get(&token_id)`). An `Update` operation touching
  `/engine/x,y` on a token the ECS hasn't yet hydrated (e.g. a same-batch Create+Update sequence,
  or an Update racing ECS hydration) would commit UNGATED — the gate is skipped (returns `None`,
  falls through to the ordinary write path), not rejected. This is a PRE-EXISTING shape, unchanged
  by Task 6 itself (Task 6 only moved the gate's read target from `/system` to `/engine`).
  Status: Investigated — unreachable: `apply_intent`'s Phase 1 (`data/sqlite.rs`) validates every
  op in a single batch sequentially, in array order, BEFORE any Phase 2 row mutation runs — an
  `Operation::Update`'s Phase 1 branch calls `Self::load_document(&mut *tx, *doc_id)` and rejects
  with `DataError::Conflict` if it returns `None`; a same-batch `Create` for that same `doc_id`
  has not yet inserted its row (insertion is Phase 2 only), so a `[Create(token),
  Update(token, /engine/x=...)]` batch is rejected outright at Phase 1, never reaching commit.
  Separately, `Room::publish` acquires `publish_guard` (a `tokio::sync::Mutex<()>`) for its ENTIRE
  gate→`apply_intent`→ECS-hydrate critical section (guard taken at function entry, held through
  `commit_ops_locked`, never re-entered or released early) — every `publish` call for a room is
  therefore fully serialized against every other `publish` call for that same room, including
  across separate WS requests, so an `Update` racing a DIFFERENT publish call's ECS hydration for
  the same token cannot land mid-hydration either: the racing publish cannot even begin its own
  Phase 1 validation (which needs the same guard, via `commit_ops_locked`) until the prior
  publish's hydration has fully completed. No code change required.

- Title: External-module i18n registration seam missing. Summary: An out-of-tree module
  (Nightfox sheets, M13c) has no public seam to register i18n keys into the shell catalog; M13c
  ships a built-in English fallback map (`nfT`/`NF_MESSAGES`) with a `ctx.t` override hook as a
  workaround. First surfaced in M13c Task 1's own code comment; reinforced by a separately
  discovered test-context gotcha (`setAppContextForTest`'s default `t: (k) => k` echo means `nfT`
  always resolves through its English fallback under test, never through a real translation
  catalog).
  Status: Needs Review (candidate engine seam for a later checkpoint).

- Title: `effect` doc_type constant has no engine home. Summary: D9 makes `effect` a
  client-semantics doc_type but neither M12c (which owns `ITEM_DOC_TYPE` in
  `scene-docs.ts`) nor the M13b rules plan declares an `EFFECT_DOC_TYPE`; M13c defines it in
  the Nightfox barrel (`index.ts`). Consider promoting it beside `ITEM_DOC_TYPE` if a second
  consumer appears. Status: Needs Review.

- Title: No browser e2e harness for external modules. Summary: The M13-1 toolchain e2e is
  HTTP-only (no DOM); the spec §11 "Playwright e2e" for M13c has no browser harness, so the
  author→equip→toggle→revert flow (M13c Task 11) is covered by a component-level integration
  test instead. Status: Needs Review (Playwright harness is a toolchain follow-up).

- Title: StatTable drag/drop reorder has no touch-triggerable fallback on iOS Safari. Summary:
  the reorder mechanism is pure native HTML5 Drag-and-Drop (`draggable` + `ondragstart`/
  `ondragover`/`ondrop`); WebKit on iOS does not fire `dragstart` from touch on `draggable`
  elements (a long-standing WebKit gap, distinct from desktop/Android Chrome). 44px sizing
  satisfies touch target SIZE but not touch TRIGGERING. This violates the CLAUDE.md
  cross-platform touch directive and the spec's own "touch-friendly... cross-platform directive"
  framing for this exact feature, on a named target platform, with zero test signal (tests only
  fire synthetic dragStart/drop events). A pointer-events-based (or long-press) reorder
  implementation is needed. Status: Needs Review (buddy-check Important, deferred with explicit
  reviewer sign-off).

- Title: StatRow's numeric field edits silently no-op on invalid input with no visible feedback.
  Summary: `editNumber` returns early on non-finite input with no dispatch and no error
  indicator; because numeric inputs use one-way `value={...}` bindings (not `bind:value`), the
  DOM is never forced back to the last valid value, so stale/invalid typed text can persist
  indefinitely with no chip or signal. No invalid value is ever dispatched (no
  correctness/security impact) — a UX papercut only. Status: Needs Review.

- Title: M13d dice-label fix (`bf494c1`) — no Rust-side test for `RollOutcome` missing the
  `labeled_consts` key. Summary: the report's cited backward-compat regression test
  (`stored_pre_m11d2_message_still_deserializes`) doesn't actually exercise a `RollOutcome`-
  shaped blob, so `#[serde(default)]` on `labeled_consts` is backed only by the mechanical,
  already-precedented pattern (`symbol_counts` uses it identically) plus a client-side
  (`chat-docs.test.ts`) legacy-fixture test, not a Rust-side one. Low real risk. Status: Needs
  Review (code-review Minor, non-blocking).

- Title: M13d dice-label fix (`bf494c1`) — a labeled constant's displayed value ignores an
  enclosing `Neg`/`Mul` operator (e.g. `-3[dex]` displays as `3[dex]`, not `-3`).
  `collect_labeled_consts` shows each `ConstTerm`'s raw literal value, mirroring how `DieRecord`
  raw faces are already shown regardless of an enclosing sign — a real precedent, not an
  oversight, but a fidelity gap against `labeled_consts`'s own provenance-transparency intent
  since `total` itself is unaffected and correct. Status: Needs Review (code-review Minor,
  disclosed by the implementer, non-blocking).

- Title: `ResolvedScene.bounds` has two contradictory unit interpretations, and the grid-unit one is
  wrong on hex. Summary: surfaced by the 14e-7 `[sec]` review (reviewer flagged the hex half as
  inferred; the dispatcher traced the rest and found the inconsistency underneath). TWO consumers of
  the same `.bounds` value disagree on its units. (1) `navmesh::build_navmesh` (`navmesh.rs:131`)
  treats it as GRID UNITS: `(w_px, h_px) = (w * cell, h * cell)`. (2) `vision::bound_for_scene`
  (`vision.rs:96-107`) treats it as PIXELS, feeding `width`/`height` straight into `maxx`/`maxy`
  alongside raw wall coordinates. Both read `resolve_scene(scene).bounds`. On a square scene with
  `cell != 1` these two cannot both be right. Separately, even granting the grid-unit reading, the
  conversion is wrong for hex: `HexGrid.size` is the pointy-top OUTER RADIUS, so `w` hexes span
  `w * sqrt(3) * size` horizontally and `h * 1.5 * size + 0.5 * size` vertically — not `w * size` /
  `h * size`. A hex + continuous scene would therefore get a navmesh rectangle roughly 58% of the
  intended width, making legitimately reachable far cells report `Unreachable`. NOT a secrecy issue
  (under-reveal / over-restriction direction) and NOT cell indexing, so it is outside 14e-7's three
  named sites and was deliberately left unfixed there. Status: Needs review — resolve which unit
  `bounds` actually carries FIRST (that decides whether (1) or (2) is the defect), then fix the hex
  extent conversion. Note the hex + continuous combination has no test coverage for mesh extent.

- Title: Hex continuous-weighted preview cost is ~1.73x too small (unit-parity defeated on hex).
  Summary: found by the 14e-7 `[sec]` code review; pre-existing (M10f-4 era), not introduced by
  `2e6800c`. The weighted continuous branch converts the grid router's cost to scene units with
  `weighted.cost * cell` (`scene/mod.rs:1206-1209`). That conversion assumes one step spans `cell`
  world units, which holds for `SquareGrid`. On hex, `resolve_grid_shape_with_rule` returns a
  `HexGrid` whose `neighbors_with_cost` is a uniform 1.0 per step (`grid_shape.rs:250-259`), but
  adjacent hex centers are `sqrt(3) * size ~= 1.732 * cell` apart (`axial_to_pixel((1,0)) =
  (size*sqrt(3), 0)`). So a hex + continuous + terrain scene reports a preview cost about 1.73x too
  small, while the sibling pure-polyanya branch on the same scene reports true Euclidean length —
  defeating exactly the unit parity the `* cell` line exists to guarantee. Preview/budget only, not
  a gate. Status: Needs review — likely wants the step-to-world-distance factor to come from the
  `GridShape` rather than being assumed equal to `cell`. Note this compounds with the `bounds`
  units question logged above; both are hex + continuous, which has thin test coverage.

- Title: Hex + continuous `clip_to_visible_mask` has no end-to-end call-site coverage.
  Summary: from the 14e-7 `[sec]` code review. The one integration test for the hex + continuous
  chain (`scene/mod.rs:4726-4783`) runs with `is_gm: true`, so `mask` is `None` and
  `clip_to_visible_mask` returns early at its `mask.is_none() && walls.is_empty()` branch
  (`navmesh.rs:348`). Call-site wiring is therefore pinned end-to-end only for `truncate_at_arrest`;
  the shape threaded into `clip_to_visible_mask` — the actual fog gate on the pure-polyanya branch —
  is covered by its unit test but never exercised through `pathfind`. Not a defect (both call sites
  pass the same `&*grid_shape` binding), but a non-GM hex + continuous `pathfind` test with a real
  mask would close the class. Status: Actionable — add the test.
