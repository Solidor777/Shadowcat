# TODO — Deferred Work

Actionable, externally-logged deferrals. Bugs go in `OPEN_BUGS.md`, not here.
As of the Phase-1 cleanup burndown (2026-07-19), most items below are
retained because their blocking capability doesn't exist yet — a concrete
unblocking condition, not a "someday maybe." A few headings are explicitly
labeled "Actionable now": these are NOT blocked on anything — the underlying
capability already exists — but are deferred as out-of-scope-for-now work.

## Blocked on a reverse-proxy deployment story
- TODO: `ClientIp` (`http/throttle.rs`) resolves solely from `ConnectInfo<SocketAddr>` — the real
  peer address of the accepted TCP connection — with no `X-Forwarded-For`/`Forwarded` handling.
  Behind a reverse proxy that does not preserve the original client address, every request
  resolves to the proxy's own address, so the per-IP throttle bucket (`login:ip:<>`/
  `invite:ip:<>`) degrades to a single shared bucket across every real client — throttling still
  functions per-identity, just not per-real-IP. No reverse-proxy deployment story exists or is
  scoped today (verified: `docs/design/` and `config.rs` have no proxy/trusted-header handling);
  resolve alongside whatever design adds one (a naive trust-any-`X-Forwarded-For` fix would be
  its own spoofing vulnerability without a configured trusted-proxy list).

## Blocked on a per-turn movement-budget system (Phase-2 combat)
- TODO: `move_exec.rs`'s `MoveOutcome.cost` accumulates only the entered cell's terrain multiplier per step (`cost += regions.terrain_multiplier(region_cell)`); `pathfinding.rs`'s router cost also multiplies by the diagonal-rule `step_cost` (`sc * mult`, where `sc` is 1.0/2.0/√2/alternating depending on `world-settings.pathfinding.diagonalRule`). The two "cost" values are not numerically comparable once diagonal movement is involved under any non-Chebyshev rule — they coincide only because Chebyshev's diagonal step cost is 1.0. This is a deliberate M10g Task 7 scoping decision (move_exec's center-cell, terrain-only accounting model), not an oversight, and nothing currently consumes or compares the two values. Resolve before any per-turn movement-budget system consumes `MoveOutcome.cost`/`MoveStream.cost`: decide whether move_exec should thread the diagonal rule + per-step parity to match the router's preview cost, or whether route-preview cost and execution cost are intentionally distinct quantities. (Surfaced by the M10g Task 7 buddy check.)
- TODO: `navmesh::los_smooth` (M10f-4) reports the smoothed continuous route's `cost` as the PRE-smoothing weighted grid cost, unchanged — it does not recompute an exact per-span cost for the straightened any-angle chords, only guarantees the reported value is a conservative (never cheaper) budget preview. Same preview-vs-execution divergence class as the `MoveOutcome.cost`/router-cost split logged above: a per-cell-exact smoothed continuous cost is deferred, not implemented. Resolve alongside the item above if a per-turn movement-budget system ever needs an exact continuous-engine cost.

## Blocked on rotation authoring
- TODO: Lerp token rotation along the shortest signed delta (`((b-a+540)%360)-180`) with a wrap-aware ε-settle, when M8d-2 adds rotation control. M8d-1's `TokenAnimator` lerps rotation as a raw scalar (350°→10° tweens the long way); cannot manifest until rotation is authorable. (Surfaced by the M8d-1 buddy check.)

## Blocked on a scene-background authoring UI
- TODO: Build a minimal scene-**background** authoring UI, then add a browser e2e asserting the background renders (Scene `engine.background` → sprite). The render-consumption half already works (`SceneBrowserPanel` shows the thumbnail via `bgOf`, the stage paints the sprite), but no client UI anywhere sets `engine.background`: `buildSceneDoc` accepts a `background` field, yet the only call site (`SceneBrowserPanel.create()`) never supplies one, and `configure()` merely deep-links to `GameSettingsPanel`, which has no background control. Add a background-asset picker (e.g. on `SceneBrowserPanel`'s existing thumbnail slot) that OCC-dispatches an `Update` to `/engine/background` with the raw current stored value as `old` (mirroring `activate()`/`setScene()`'s raw-old-value convention) — then add the e2e assertion once that UI exists.

## Blocked on module management / hard topology enforcement
- TODO: Extend `reconcileTopology` beyond presence-by-`module_id` to flag version and `provides`/`requires` mismatches for modules present on both sides (a stale local build providing a contract the world no longer declares currently reconciles silently). Land with module management / hard topology enforcement.
- TODO: `LauncherMenu` has no handling/test for `metaMap` mutating while the menu is open (a panel uninstall would drop focus out of the menu's closed loop). Modules only install/uninstall at world entry today; add a focus-recovery path (or at least a pinning test) when live module management lands.

## Blocked on a real 2nd provider / multiple contract versions
- TODO: Resolve multi-provider conflict policy for `singleton` surface contracts in the UI contribution architecture — when two modules provide the same `singleton` contract (e.g. both claim "the sidebar"), decide the winner (load order, explicit priority, or user selection) instead of the current deterministic loud-fail. Design once a real second provider exists to validate the semantics; the contract model already carries the `singleton`/`multi` cardinality marker the policy slots into.
- TODO: Add capability version negotiation to contract-based module dependencies (`requires`) — match a required contract against a provider by version range, not presence alone. Deferred until multiple providers of a contract exist at differing versions.

## Blocked on multi-panel popout groups
- TODO: an already-open popout window has no `onWillDrop` subscription wired
  (`#groupWillDropSubs` is populated only inside `apply()`'s zone loop) — dockview-core's own
  popout design supports drag-and-drop of a further panel into the popout's nested gridview, so a
  same-origin cross-window drag into an open popout would bypass the reducer's veto/classify
  pipeline entirely (`applyOp` invariant "all layout mutations flow through applyOp" would not
  hold for that gesture), and `#poppedOutGroupPanels`'s single-panel-array assumption wouldn't be
  updated to include the dragged-in panel — silently unaccounted for on window close. Out of the
  M12e Task 5 brief's scope (menu pop-out + its own close translation only); wire it if/when
  multi-panel popout groups become a supported gesture.

## Actionable now — e2e per-worker accounts
- TODO: Give the e2e suite per-worker accounts (instead of all 6 Playwright workers sharing the
  `ops` account) so parallel workers stop contending on one user's `global.lastWorld`/ui-state
  slice — the deeper test-hygiene fix behind the `panels.spec.ts` reload failures. The Task 4
  route-first boot fix (`docs/CLOSED_BUGS.md`) already makes a reload deterministic regardless of
  which account entered which world last, so this is hygiene/isolation, not a correctness gap.

## Blocked on real pointer-gesture QA (unsimulable under jsdom)
- TODO: `DockviewEngine#toDropSite`'s one remaining fallback branch (a drop's target group
  falling outside the engine's own zone bookkeeping) is a best-effort approximation (falls back
  to an edge-zone dock), not exhaustively verified against every dockview drag path. The
  intercept-and-redispatch translation mechanism itself (preventDefault + emit + reconcile
  through `apply()`) IS exercised directly by unit tests now, not approximated. Real drag-and-drop
  still cannot be simulated under jsdom (no native `DragEvent`/`PointerEvent` gesture) — the
  residual manual-QA item narrows to drop-position classification fidelity for real pointer
  geometry (edge vs center vs tab-strip index resolution against an actual drag gesture) before
  shipping.

## Blocked on a bespoke-fallback caller needing it
- TODO: `FakeEngine`'s plain tab strip has no `PanelMenu` (dock/float/minimize/pop-out commands)
  — that menu is mounted by `DockviewEngine.createTabComponent` only. A panel docked under
  `FakeEngine` (bespoke-fallback engine; production never reaches it) can only reach a
  minimized/closed state going forward via `PanelsChipsView.restore`, not back out of a zone
  through any UI affordance. Orthogonal to the width-containment fix (`docs/CLOSED_BUGS.md`):
  giving `FakeEngine` its own menu is future work if a bespoke-fallback caller needs it.

## Blocked on real-time per-recipient move-streaming
- TODO: Live cross-animation concurrency for streamed move vision (`MoveStream`). M2 precomputes each move's per-recipient vision clip at *its* execute time, so two tokens moving simultaneously do NOT reveal each other mid-walk when a watcher's vision opens after the clip — it reconciles at the stop + next `vision` rebroadcast. Wanted eventually. Needs real-time per-recipient streaming (a per-move server loop recomputing each recipient's visibility of every concurrently-moving token as positions advance) instead of execute-time precompute. No correctness/secrecy impact today — only a missed transient reveal. (Design `2026-06-25-m2-streamed-continuous-vision-design.md` §8; user wants it as a follow-up.)

## Blocked on `@shadowcat/formula` gaining more consumer-callback resolver boundaries
- TODO: `evaluate.ts`'s `ref` case and `template.ts`'s `substituteIdentifier` both wrap a consumer resolver call in a near-identical try/catch → `resolver-error` FormulaError. `graph.ts`'s equivalent catch is entangled with the internal `NeedsDependency` trampoline signal and can't share a naive helper without leaking that control-flow type across `internal.ts`'s validation-only boundary — so only `evaluate.ts`/`template.ts` are realistically unifiable. Factor a small shared helper for those two call sites if `@shadowcat/formula` grows more consumer-callback boundaries. (Surfaced by the M13a whole-branch buddy-check fix-confirmation review.)

## Blocked on real-world need (low-priority polish, inert until it matters in practice)
- Stored `explored_fog` blobs (`ExploredSet::to_bytes`, `(i32,i32)` per cell) carry no grid-kind tag. A blob is indexed in the scene's grid kind at write time (square `(i,j)` or hex axial `(q,r)`); reads (`ExploredSet::contains`/`iter`) are pure set membership, so a fixed-grid-kind scene has one consistent interpretation and needs no migration. A GM switching a LIVE scene square<->hex would reinterpret an existing blob under the new kind's coordinate system (stale explored cells until re-fogged) — an accepted edge, not corruption. Add a grid-kind tag to the blob header + a re-index-or-clear-on-switch step only if live grid-kind switching of populated scenes becomes a real workflow.
- Server shortcodes: pre-parse replacement also fires inside markdown code spans; refine to
  skip code spans if it ever matters in practice.

## Blocked on a module-facing i18n registration seam
- Community/external modules cannot add entries to the host i18n catalog — `Ii18n.t` resolves
  only the built-in `locales/*` catalogs and a missing key falls back to the key string itself
  (verified 2026-07-30: no `addMessages`/`registerLocale`-style API exists in `@shadowcat/core`
  i18n or ui-kit). Consequence: a community module's `PanelMeta.labelKey` renders as its literal
  value, so the creating-a-module guide instructs authors to use a human-readable label as the
  key. When the seam lands (module-supplied catalog fragments merged per locale, collision
  rules), update the guide + the `examples/module-initiative-tracker` comment to register real
  keys.

## Follow-on feature sub-projects (own brainstorm → spec → plan each)

Out of scope for the Phase-1 cleanup burndown; built after Sub-project 1, one design pass each
(user: build ALL of bucket C):

1. **Recalc-from-chat** — persist `spec`/`raws` on `RollEmbed` (persistence + secrecy fork).
2. **Link-preview extensions** — server-fetch-cache-as-asset **image** pipeline + async
   post-publish enrichment (`WriteOrigin` path) + **shared preview cache** + **oEmbed** provider
   embeds (user opted both edge items in; oEmbed carries SSRF/privacy surface → threat-model it).
3. **Per-world export/import** — world-scoped row subset preserving cross-FK referential
   integrity + shared asset references.
4. **Dice-notation grammar growth** — math fns (floor/ceil/round/abs/min/max) + crit-event /
   tier-ladder notation syntax.
5. **Per-channel / per-message dice-settings overrides** — needs a channel model.
6. **In-body doc-link chat segment** (`Segment::DocLink`) — actor-name → sheet navigation shipped
   in M12c, but a free-form doc-link segment has no server producer or client authoring path yet;
   needs a server producer + authoring affordance.
7. **Speak-as-token-instance** — `ActorOwnerRef::TokenInstance` is REJECTED at ingest (fail-closed,
   no first-party producer) — build the composer/token-context UX and lift the rejection together.

## Actionable now — `setGmViewedScene` leaves a stale cross-scene token selection
- TODO: `setGmViewedScene` (`src/client/shell/src/lib/worldSession.svelte.ts`) does not scene-scope
  or clear `tokenSelection`, while `commitRoute` (`src/modules/scene-tools/src/controller.svelte.ts`)
  sends `activeScene(ctx).id`. So a GM who selects a token in scene A, roams to scene B, then
  commits a measured route sends `scene: B` with a token that lives in A and gets a silent
  `MoveError`. **The rejection is CORRECT and must not be relaxed** — before Task 14j that exact
  request shape was the cross-scene movement-gate bypass, and the server now derives the gate's
  scene from the token and refuses the mismatch. This is purely a client UX gap: the failure is
  silent. **Prefer scene-scoping `tokenSelection` over clearing it** — a GM roaming B and back to A
  would otherwise lose their selection for no reason. Client-package change, so gate it with
  `pnpm -r test` rather than a filtered run (a client change can break sibling packages' fixtures).
  (Surfaced by the Task 14j `[sec]` review; deliberately kept out of the server security commit to
  avoid batching unrelated concerns.)

## Actionable now — ui-state per-key merge (Task 4 final-review backlog)
- TODO: `worlds` in the stored `ui_state` blob is grow-only — `merge_ui_state` never removes a
  `worlds.<id>` entry or a leaf key within it, only inserts/replaces. A world a user leaves (or a
  stale panel-layout/chat-read key a module retires) accumulates forever, and there is no recovery
  path if an accumulated blob ever exceeds the 64KB merged cap short of a manual DB edit. Add
  `null`-removes-entry/key semantics to the merge rule (mirroring `FieldChange.remove` elsewhere in
  the data layer) plus client-side pruning (e.g. dropping a `worlds.<id>` entry for a world no
  longer in the caller's membership list) so an over-cap blob is recoverable.
- TODO: `put_ui_state` (`http/routes.rs`) opens the single-connection tx via `merge_ui_state`
  before the merged-size check runs (the check happens inside the tx, after the read). A cheap
  route-level pre-check (e.g. rejecting a patch whose own serialized size already exceeds
  `MAX_UI_STATE_BYTES`, before touching the pool) would reject an obviously-oversized patch
  without holding the single-writer connection for the read+merge+serialize round trip.
- TODO: `sessionState.svelte.ts` has no in-flight-PUT ordering guard — `schedulePersist`'s leading
  edge can fire a second `persist()` while an earlier one's `putUiState` is still unresolved (e.g.
  a slow network on the first write, a new mutation arriving before it settles), so two writes for
  the same account can be in flight concurrently with no ordering guarantee on which lands last at
  the server. Defer the leading edge while a persist is unresolved (a simple in-flight flag,
  scheduling the deferred attempt for when the current one settles) instead of the current
  fire-and-forget leading edge.
- TODO: `sessionState.svelte.ts`'s `loaded` flag is never reset to `false` on logout, so a
  mutation landing inside a re-login `loadSessionState()`'s `await getUiState()` window passes the
  `loaded` guard and can persist a pre-login `state` value under the new session's cookie.
  `clearDirty()` at load start covers only the marker half of re-login hygiene; reset `loaded`
  (and cancel the cooldown timer) at logout so the write guard is structural.
- TODO: `buildGlobalPatch`/`buildWorldPatch` (`sessionState.svelte.ts`) enumerate the leaf keys by
  hand — adding a third key to `UiState["worlds"][string]` (or a new `global` field) widens the
  type but silently drops the new key from every patch, with no compile error. Drive the copy from
  an exhaustive `Record<WorldKey, …>`/switch so a widened union becomes a type error.

## Actionable now — render-ready audit backlog (2026-07-31, non-defect items)
- TODO: `ws/conn.rs`'s Welcome preamble runs `spawn_blocking(scan_installed_modules)` — a full
  filesystem scan — on EVERY WS connect. Cache the scan result (invalidate on module
  install/uninstall) so reconnect storms and multi-client entry don't re-walk the modules dir.
- TODO: tower-sessions shares the single-connection SQLite pool (`auth/session.rs` builds
  `SqlxSqliteStore` over `repo.pool()`), so every authenticated request queues the session read
  behind app writes on `max_connections(1)`. Give the session store its own connection (or a read
  pool) — the write path's deliberate single-writer serialization stays untouched.
- TODO: `Stage.svelte`'s backend-init failure path sets `data-render-error="true"` silently. Route
  it through the project logger so a real WebGL/backend init failure is distinguishable from a
  timeout in e2e output and user bug reports.
- TODO: `WsClient.open()` (`ws-client.ts`) adopts a resolving transport (`this.transport =
  await this.opts.connect(...)`) without re-checking `running_` after the await — a `stop()`
  call during a pending connect leaves an adopted-but-unwatched socket assigned to
  `this.transport`. Re-check `running_` immediately after the connect await and close/discard the
  transport if the client was stopped in the meantime.
- TODO: `App.svelte`'s `boot()` captures `currentRoute()` once, BEFORE the `listWorlds` await (and
  the `withRetry` delays widening that window further) — a hash change that lands during that
  await (e.g. a user clicking a different deep link while "Loading…" is showing) is silently
  ignored; `resolveBootWorld` resolves against the STALE route captured before the await, not the
  URL the page now shows. Re-read `currentRoute()` immediately before calling `resolveBootWorld`,
  or detect and re-resolve on a hash change observed during boot.
- TODO: `WorldSession#onWelcome`'s activation `catch` (`worldSession.svelte.ts`) rethrows out of
  the inner `try` around `#modules.activate()`, which is caught by the OUTER per-Welcome `try` that
  wraps the entire handler body — so while activation keeps failing (e.g. a persistent contract
  cycle), EVERY subsequent step (the member-username fetch, `reconcileTopology`, scene
  re-subscription, the GM first-scene seed) is skipped on every single Welcome, not just the
  failing one. Pre-fix (single `#bootstrapped` latch) those steps ran on subsequent Welcomes once
  the latch was set regardless of activation success; this is a narrower-but-still-real regression
  of that behavior. Fix direction: log the activation failure in place (already logged via
  `#logger.error` in the outer catch) and continue running the rest of the handler instead of
  letting the rethrow short-circuit it — activation failure should degrade Surfaces, not silently
  skip member names/topology/scene resubscription too.
- TODO: `App.svelte`'s `boot()` worst case is roughly 2.4 minutes stuck on "Loading…": three
  sequential `withRetry`-wrapped awaits (`getMe`, `loadSessionState`'s `getUiState`, `listWorlds`),
  each up to 3 attempts at the 15s `FETCH_TIMEOUT_MS` plus `withRetry`'s flat `[500, 1500]`ms
  inter-attempt delays — and unlike the WS client's full-jitter backoff (`scheduleReconnect`),
  these delays are UNJITTERED, so many concurrently-booting clients retry in lockstep against a
  struggling backend. Fix direction: an overall boot deadline (fail to the login/worlds route
  sooner than three full retry cycles), a visible "still trying…" state instead of a bare
  "Loading…" spinner, and jittered retry delays matching the WS backoff's convention.
- TODO: `actor.ts`'s `effectiveOwner` mirrors the server's `effective_owner` PRECEDENCE (token's
  own `/owner`, else the linked actor's owner) but omits the server's `actor.scope === doc.scope`
  guard (`data/permission.rs`'s `effective_owner` rejects a resolved actor whose `scope` differs
  from the token's; `store.get(actorId)` in the client is a plain id lookup with no scope filter).
  Add the same `actor.scope === doc.scope` check to the client so the parity is STRUCTURAL rather
  than dependent on an unstated invariant. This is defense-in-depth, not a live bug: the client's
  `DocumentStore` is fed only by the single connected world's WS stream (a `"compendium"`-scoped
  id never enters `store`; ids are globally unique), so `store.get()` cannot today return a
  cross-scope document, and `effectiveOwner` is advisory-only (the server re-resolves in its own
  transaction). Needs its own runtime-change test + review, not a docs-only edit.

## Actionable now — Phase D-alpha (movement authority & secrecy) backlog
- TODO: `src/server/src/ws/room.rs`'s `Room::execute_move` re-derives `is_gm` via its own
  `ctx.world_role == WorldRole::Gm` comparison a second time, instead of reusing the `is_gm`
  binding already in scope from earlier in the same function. Harmless (both read the same
  field), but two spellings of one role decision in one function is exactly the kind of thing
  that drifts. (Surfaced by Phase D-alpha's final whole-branch review.)
- TODO: `SceneEcs::blocks_move` (`src/server/src/scene/mod.rs`) lost its last production caller
  when Task 9 (Phase D-alpha) moved the wall-crossing check onto `crate::scene::segments_cross`
  directly — only test callers remain. It is deliberately retained (one home for wall-crossing
  semantics) rather than deleted. It is `pub`, not `pub(crate)`, so `clippy -D warnings`'
  `dead_code` lint does not fire regardless of caller count — if a future change narrows its
  visibility to `pub(crate)`, `-D warnings` will immediately flag it as dead code and it should
  be revisited then (either re-wire a caller or delete it for real). (Surfaced by Phase
  D-alpha's final whole-branch review.)
- TODO: `execute_move`'s footprint-aware wall/mask gate has a residual anchor asymmetry on
  off-center input: the wall-disc check is anchored at the literal continuous point `next`,
  while the mask/impassable-disc checks are anchored at `grid.cell_center(next_cell)` (a
  deliberate fix for a corner-anchoring bug found during Task 9). The two anchors coincide for
  routed `GridStepped` input (where I4's route↔gate equivalence is claimed), but a
  client-supplied `MoveRequest` path is not re-snapped to cell centers, so on `Continuous` or
  off-center `GridStepped` input a wide token's mask-check disc can miss a cell its
  wall-check disc genuinely overlaps. This is strictly SAFER than the pre-Phase-D-alpha gate
  (which had no footprint mask check at all) and is not a regression, but full I4-equivalence
  on the `Continuous` engine would need the mask disc anchored at the true point too. Resolve
  if/when the `Continuous` engine gets its own full I4 parity pass. (Surfaced by Phase
  D-alpha's final whole-branch review.)


