# Closed Bugs

Confirmed-real defects that have since been fixed, kept for provenance. New fixes append a new
`##` section (or bullet under an existing one); do not delete resolved entries.

## Client / render-layer sibling divergence on non-numeric coordinates (2026-07-31 docs sweep 8)

- [Render] `drawing-view.ts` and `template-view.ts`'s `toSpec` accepted non-numeric coordinates
  and emitted a `ShapeNodeSpec` from them, while their near-identical siblings `region-view.ts`
  and `wall-view.ts` rejected such docs — despite all four engine bodies getting identical
  ingress treatment (`normalize_engine` round-trips each through serde; none calls `.validate()`,
  only `"token"` does). Fixed by guarding the RAW authored fields before tessellation in both
  files, matching `region-view.ts`'s placement exactly.
  **Placement, not merely presence, is the fix.** JS coerces `null` to 0 in arithmetic, so a
  guard after tessellation sees finite, plausible-looking geometry and never fires —
  `circlePoints(null, 5, 10)` yields a wrong-but-renderable circle. Tests cover `null` on the
  arithmetic-heavy kinds (circle, cone, ellipse) plus the `direction`-through-`cos`/`sin` path,
  mutation-proven twice: removing a guard fails, AND moving it back after tessellation fails the
  `null` cases specifically.
  Treated as defense-in-depth rather than a claim about a specific upstream conversion. The
  render layer draws the OPTIMISTIC view, so a scene-tool bug building a Create op with a
  missing or non-numeric coordinate reaches `toSpec` on the authoring client before the server
  validates anything.
  **Provenance, recorded because the failure is more instructive than the fix:** found by the
  sweep-8 Task 6 implementer auditing four near-identical siblings. The guard was then justified
  with a false mechanism TWICE — first "an oversized magnitude reaches the client as `Infinity`",
  then "it arrives as `null` because `normalize_engine`'s round-trip nulls non-finite floats".
  Both were wrong: `serde_json`'s lexer rejects an overflowing literal at tokenization
  (`de.rs`'s `f64_from_parts`: `if f.is_infinite() { Err(NumberOutOfRange) }`), so such a payload
  fails the outermost `from_str::<ClientMsg>` and never becomes a `Value` at all; and
  `Value::Number` can only be built via `Number::from_f64`, which already excludes non-finite.
  Each successive explanation was more detailed and equally unverified. The guard was correct
  throughout; only the story about *why* kept being invented. Caught by the Task 6 code review
  and then the Task 6 spec review, each reading the vendored crate rather than accepting the
  stated mechanism.

## Client / hex grid overlay dropped cells near the viewport edge (2026-07-31 docs sweep 8)

- [Render] `Grid.hexLines` (`src/client/render/src/grid.ts`) scanned too narrow a `q` range, so
  hexes whose centers lie **inside the viewport** were never drawn — visible gaps in the hex grid
  overlay toward the top-right and bottom-left. `pixelToAxial` computes
  `q = (√3/3·x − 1/3·y)/size`, mixing x and y with OPPOSITE signs, so q's extrema fall on the
  top-right/bottom-left diagonal; `r = (2/3·y)/size` depends on y alone. The bounds code sampled
  only the top-left and bottom-right corners and padded by ±1, which happens to capture r's true
  extremes but understates q's badly. Measured before the fix: **50 undrawn in-viewport hexes at
  1920×1080 with `size` 50**, 15 at 800×600/40, 6 at 1920×1080/100 — worse the smaller `size` is
  relative to the rect, i.e. worse the more zoomed out the camera. Fixed by sampling all four
  corners of the padded rect for both axes. The pre-existing test asserted only
  `lines.length > 0`, which cannot detect a too-small range; the regression test now walks the
  emitted outlines, reconstructs each drawn hex center, and asserts every in-viewport center is
  covered across three viewport/size combinations. Mutation-proven: restoring two-corner sampling
  fails the new test. Found by the sweep-8 spec reviewer while verifying a NEW doc comment that
  claimed `hexLines` "draws the six-edge outline of every hex whose center falls within `rect`
  plus a margin" — the claim was false, and checking it surfaced the underlying defect.

## Client / silent-hang startup paths (2026-07-31 render-ready audit)

- [Hang] No Welcome watchdog: `WorldSession.enter` awaited the server's `Welcome` frame with no
  timeout, retry, or error state (`worldSession.svelte.ts` — `role` is only set in `#onWelcome`,
  and `App.svelte`'s world gate rendered "Connecting…" until then). The browser's socket `open`
  fires at HTTP 101, BEFORE the server's Welcome preamble (~9 DB round trips + a blocking
  `scan_installed_modules` fs scan per connect, `ws/conn.rs`), so a stalled preamble left the
  client on "Connecting…" forever — reconnect machinery only reacted to socket CLOSE. Fixed by
  arming a `welcomeTimeoutMs` watchdog (default 10s) after `open()` succeeds, cleared on receipt
  of a matching Welcome; an open-but-unwelcomed transport is closed into the normal reconnect
  path instead of hanging silently. A follow-up fix round closed a reintroduction: a Welcome
  frame already queued as a message task when the watchdog closed its connection could still
  deliver after reconnect and incorrectly disarm the successor connection's own watchdog —
  `handleFrame` now tags each `open()` attempt with a monotonically increasing generation id and
  ignores a `"welcome"` frame whose generation doesn't match the current connection before
  acting (`resync_end` carries the same guard, commit `69d32ee`). `src/client/core/src/
  ws-client.ts`. Commits: `69c47c9`, `fb1d5be`.
- [Hang] `webSocketConnect` (`client/core/src/transport.ts`) settled only on the socket's
  `open`/`error` events — a TCP-accepted-but-never-upgraded handshake never settled, and
  `ws-client.ts`'s reconnect path was unreachable behind the unsettled await. Fixed by adding a
  `connectTimeoutMs` handshake bound (default 10s) that rejects and closes the socket so
  `scheduleReconnect` runs, plus a settled-guard so a pre-open close/error only rejects the
  connect promise instead of also leaking into `handlers.onClose` (which could have
  double-scheduled a reconnect). `src/client/core/src/transport.ts`. Commit: `69c47c9`.
- [Hang] `boot()`'s three fetches (`getMe`, `getUiState`, `listWorlds` — `App.svelte`/`api.ts`)
  were unbounded and unretried; any transient non-2xx or connection reset permanently degraded to
  the login or worlds route with no visible error and no retry. Fixed by bounding every
  session/boot fetch with a 15s `AbortSignal.timeout` (`FETCH_TIMEOUT_MS`) and adding `withRetry`
  (3 attempts, flat delays) around the boot chain's three awaits before degrading.
  `src/client/shell/src/lib/api.ts`, `src/client/shell/src/App.svelte`. Commits: `1d2f3b6`,
  `4efea22`.
- [Wedge] `WorldSession.#bootstrapped` latched `true` BEFORE `await #modules.activate()`
  (`worldSession.svelte.ts`), so a failed or hung first activation (e.g. a manifest dependency
  cycle throwing out of `topoSort`) was cached for the session's life: reconnect Welcomes
  short-circuited, `role` was set, the Table mounted, but every Surface stayed empty and
  `.stage-host` never appeared — logged only. Fixed by splitting the single latch into
  `#modulesAdded` (once per session — re-adding would duplicate registrations) and `#activated`
  (latches only on a successful `activate()`, reverting to `false` on a thrown activation so a
  contract-cycle failure is retried on the next Welcome instead of being cached for the session's
  life). `#activated` is still set SYNCHRONOUSLY before the `activate()` await — load-bearing:
  same-tick concurrent Welcomes re-enter `#onWelcome`, and an after-await latch would
  double-activate. `src/client/shell/src/lib/worldSession.svelte.ts`. Commits: `1d2f3b6`,
  `4efea22`.
- [Teleport] `App.svelte`'s `boot()` ignored the current URL hash entirely and re-entered
  `ui.global.lastWorld` on every load, including a reload of a deep-linked `#/world/<id>` route —
  a reload of ANY world URL silently teleported to whichever world was entered last, discarding
  the URL. Proven by a captured Playwright network trace of a failing `panels.spec.ts` reload: the
  reload's `GET /api/me/ui-state` returned a DIFFERENT concurrent e2e worker's `lastWorld`, and the
  page entered that foreign world instead of the URL's own — one mechanism behind three distinct
  observed failure shapes (wrong-world dock miss, worlds-list bounce when the foreign world was
  since deleted, render-ready timeout in a busy foreign world). Fixed with a route-first resolution
  rule: a world route in the URL hash always wins over `lastWorld` (which is not consulted at all
  while a world route is present); `lastWorld` seeds ONLY a bare/non-world load; a route's world id
  absent from `listWorlds()` (deleted/revoked) falls back to the worlds list rather than silently
  substituting `lastWorld`, clearing `lastWorld` only when it is ALSO stale (narrowed in a
  final-review fix wave, commit `69d32ee` — the original fix unconditionally cleared it, wiping an
  otherwise-valid `lastWorld` as a side effect of an unrelated dead deep link). Extracted as a pure
  `resolveBootWorld(route, lastWorld, worlds)` helper so the rule lives in one place and is
  testable without mounting `App.svelte`. `src/client/shell/src/lib/bootResolution.ts`,
  `src/client/shell/src/App.svelte`. Commit: `694415d`.

## Server / move-execution

- [Movement] `movement::supercover_cells` spuriously failed-closed (returned `None`, rejecting an
  otherwise-legal move) on a diagonal king-step whose leg landed exactly on a 4-way grid-line
  intersection at BOTH endpoints. Root cause: the corner-crossing branch stepped BOTH axes on
  every `tMax` tie without checking whether an axis had already reached its target cell — once a
  preceding forced single-axis step (caused by a start coordinate sitting exactly on a grid line)
  put `t_max_i`/`t_max_j` into permanent lockstep, every later tie re-stepped the already-arrived
  axis too, drifting the traversal past `(ei,ej)` until `MAX_MOVE_CELLS` aborted it. Fixed by
  gating the diagonal corner-step on a per-axis remaining-step budget (`remaining_i`/`remaining_j`,
  the exact number of grid-line crossings still owed on each axis): the corner branch now only
  fires when BOTH axes still owe a step (a genuine mid-path crossing with more path beyond); once
  one axis's budget hits zero, only the other axis steps, regardless of any `tMax` tie. This makes
  convergence a property of the (bounded) step budget rather than floating-point tie-breaking, and
  preserves the existing safe-over-include behavior for genuine mid-path corner crossings (both
  flanking cells still emitted whenever real path remains on both axes). Regression tests:
  `diagonal_leg_with_both_endpoints_on_lattice_corners_succeeds` (the exact bug-report repro),
  `perfect_diagonal_across_many_lattice_corners_converges` (a longer 45-degree diagonal crossing
  several lattice-corner ties in a row), `single_endpoint_on_lattice_corner_still_includes_flankers`
  (proves flanker emission is unregressed when only one endpoint is lattice-aligned) —
  `src/server/src/scene/movement.rs`. `execute_move`'s frozen-fixture scenario "diagonal 3-step
  king path, full visible" (`src/server/src/scene/move_exec.rs`) — previously frozen at the
  buggy `truncated: true, cost: 2.0` outcome with an explicit comment marking it as a known,
  pre-existing defect — is now updated to the correct `truncated: false, cost: 3.0` outcome.

## Client / scene-rendering

- [Vision] `RenderEngine.onSceneFrame`/`flushPendingDerived` (`src/client/render/src/engine.ts`)
  had a frame-ordering monotonicity hole: if a vision frame at seq 5 deferred into `pendingDerived`
  (its `computedAtSeq` ahead of `store.appliedSeq`) and a NEWER frame at seq 7 subsequently arrived
  and took the IMMEDIATE-apply branch (its own `computedAtSeq` no longer ahead), `lastAppliedSeq`
  advanced to 7 without clearing the still-set seq-5 entry — `onSceneFrame`'s immediate branch never
  touched `pendingDerived`. A later `flushPendingDerived` call (triggered by any subsequent store
  commit) re-checked only `store.appliedSeq >= p.seq`, found it satisfied, and re-applied the stale
  seq-5 payload — regressing `lastAppliedSeq` back to 5 and overwriting the newer seq-7 mask with an
  older-but-valid one. Not a secrecy leak (both frames re-filter to the current viewed scene, per
  the M12d fog-secrecy fix in `74165e4`), only a momentary/self-correcting flicker. Fixed by adding
  a monotonicity guard to `flushPendingDerived`: a pending entry is now applied only when its `seq`
  is still greater than `lastAppliedSeq` at flush time — otherwise it is DISCARDED, never applied.
  The pending slot is unconditionally cleared as soon as the watermark condition
  (`store.appliedSeq >= p.seq`) is met, whether the entry is applied or discarded, so a superseded
  entry never lingers past this check. The pre-existing scene re-filter-at-flush-time behavior
  (M12d) is untouched — this fix only adds the seq-ordering guard, applied AFTER the existing
  `store.appliedSeq >= p.seq` watermark check and BEFORE the `toVisibility` re-filter call.
  Regression test: `"a stale deferred frame superseded by a later immediate-apply frame is
  discarded, not re-applied, on flush (no lastAppliedSeq regression)"` (`src/client/render/src/
  engine.test.ts`) — drives the exact repro sequence by mutating `store.appliedSeq` directly (a
  plain field) to isolate `RenderEngine`'s own watermark contract from `DocumentStore`'s incidental
  commit-triggers-flush coupling; confirmed failing against the pre-fix code before the fix landed.

## Client / panels (FakeEngine bespoke-fallback only)

- [Panels] `FakeEngine` (`src/modules/panels/src/engine/fake.ts`) lost width containment once a
  third docked group was added to the same zone (`right`/`bottom`/`left`), rendering
  full-viewport-width and covering the stage canvas underneath it. Root cause: `FakeEngine.apply`
  never read `ZoneNode.size` — the zone's own px cross-size basis, already tracked by the
  reducer (`layout/tree.ts`) and driven by dockview's real splitter for the production engine —
  so a zone `<div>` carried no width of its own. `init()` built `host` as a single column flex
  container with `centerEl` and all three zone `<div>`s as plain unstyled siblings; a flex item
  with no explicit width, inside a column flex container, stretches to the container's full
  cross-size (`align-items: stretch`, the flex default) regardless of how many groups are docked
  into it — the "2 groups OK, 3rd breaks" threshold was purely a function of when a zone's
  aggregate content first grew wide enough to visually register the always-present stretch, not a
  structural change in the DOM/CSS at the 3rd group specifically. Fixed by giving `FakeEngine` a
  real docked-layout geometry: `init()` now nests a `row` flex container (`left` zone / `centerEl`
  / `right` zone side by side) inside `host`'s column flow, with `bottom` as a full-width row
  below it; each zone `<div>` (`#makeZoneEl`) carries `flex: 0 0 auto`, `min-width: 0` (so its own
  intrinsic content can never force it wider than its basis), and `overflow: auto` (oversized
  content scrolls WITHIN the zone instead of escaping it). `apply()` now applies `ZoneNode.size`
  as the zone's actual px width (right/left) or height (bottom) on every reconcile — 0 while the
  zone has no groups, so an empty zone reserves no layout space — and each group `<div>` gets
  `width: 100%; min-width: 0` so a wide panel's content can't push its own group wider than the
  zone. Regression test: `"FakeEngine constrains a zone's cross-size to ZoneNode.size once it has
  docked groups, past 2 groups"` (`src/modules/panels/src/engine/fake.test.ts`) — asserts the
  zone container's inline `width`/`flex`/`overflow` styles both at 2 and at 3 docked groups
  (jsdom has no layout engine, so this asserts the CSS containment contract, not computed
  pixels); confirmed failing (`eng.zoneEl is not a function`, then a missing `width` once the
  accessor was added) against the pre-fix code before the fix landed. Not present under the
  production engine (`DockviewEngine`), which was already unaffected — see
  `stage.spec.ts`'s "author an animated (frame-list) actor token" e2e.
## Server / data (OCC)

- [Critical, FIXED] `apply_intent`'s Phase-1 OCC pre-image comparison (`data/sqlite.rs`,
  `actual != ch.old`) used raw `serde_json::Value` equality, which spuriously rejected an
  otherwise up-to-date write. Mechanism: `serde_json::Value::Number` splits whole numbers into
  `PosInt`/`NegInt` and non-whole numbers into `Float`; the server stores an M13-0 `engine` `f64`
  field as `Float` even when its value is a whole number (e.g. `100.0`), but a JS client cannot
  preserve "this was a float" for a whole-number value through `JSON.parse`/re-serialize — the
  echoed OCC pre-image comes back as `PosInt`, and raw `==` treats `PosInt(100)` and `Float(100.0)`
  as unequal. Reachable via an ordinary token drag (`sendMoves`,
  `src/modules/scene-tools/src/controller.svelte.ts`) performed any time after a server-executed
  move (`execute_move`, which commits `/engine/x,y` as `Float`), and via the `ActorsPanel`
  vision-range editor and `GameSettingsPanel` numeric editors, whose pre-images are nested
  arrays/objects containing whole-number `Float` leaves. Fix: `values_semantically_eq`
  (`data/sqlite.rs`), a structural equality that recurses into `Object`/`Array` and treats
  mismatched-variant `Number` leaves as equal when numerically equal. Same-variant integer PAIRS
  (both PosInt/NegInt) are compared EXACTLY as `i128` with no magnitude limit; the `|n| <= 2^53`
  exactness guard applies only to the mixed case (one integer side, one `Float` side), where an
  `f64` comparison is unavoidable. Scoped to the OCC pre-image comparison only — Phase-2
  normalization and all other equality checks are untouched. Regression coverage: 9 unit tests on
  `values_semantically_eq` (whole-number Float/PosInt equality, genuinely stale rejection, nested
  array/object recursion, >2^53 mixed-case precision fallback, negative-number variant mismatch,
  large same-variant integer pairs that alias under f64 but must reject, opposite-sign
  same-magnitude rejection, trivially-equal small integers) plus an integration test
  (`ws::room::room_tests::client_update_with_posint_pre_image_after_execute_move_is_accepted`)
  reproducing the real `execute_move` → client-drag path end to end.
- [Critical, FIXED] Round 2: the fix above's Number-comparison branch had no magnitude guard when
  BOTH sides parsed as same-variant integers, falling through to the lossy `f64` equality used for
  the mixed case. Two distinct same-variant integers whose magnitude exceeds 2^53 (e.g. `2^62` vs
  `2^62 + 1`) alias to the same `f64` and were incorrectly reported equal — an OCC bypass in the
  silent-lost-update direction, strictly worse than raw equality for this case (raw equality would
  have correctly rejected them). Fix: the both-integers case now compares as `i128` exactly and
  never falls through to `f64`; the `f64`-tolerant path is reserved exclusively for the genuinely
  mixed integer/`Float` case. Regression coverage: 4 additional unit tests (large same-variant
  PosInt pair that aliases under `f64`, large same-variant NegInt pair, opposite-sign
  same-magnitude rejection, trivially-equal small-integer pair).

## Client / document store & optimistic

- [Client] Every per-scene vision/lighting override threw `cannot set field on non-container` on a
  default-created scene. `buildSceneDoc` correctly emits `SceneEngine`'s required-but-nullable
  `vision`/`lighting` keys as explicit `null` (their `Option<T>` fields have no `skip_serializing_if`,
  so the server round-trips them back as `null` regardless), but both the client `setPointer`
  (`store.ts`) and the server `set_pointer` (`command.rs`) auto-created only a MISSING intermediate
  and rejected descent through an explicit `null` — so dispatching `/engine/vision/movementRestriction`
  (or any of the ten scene-tier overrides in `GameSettingsPanel`) failed. Root cause was the
  pointer-descent primitive, not the doc builder (omitting the keys would only defer the failure to
  the first server echo). Fixed in lockstep on both sides — the never-fork-a-decision invariant — by
  treating a `null` intermediate as replaceable (auto-create the container), matching how
  `getPointer`/`removePointer` already treat a null intermediate as absent. A follow-up extended
  `remove_pointer`/`removePointer` to also no-op through a null intermediate, so all three pointer
  ops now agree. Anti-drift parity tests on each side (`set_pointer_descends_through_an_explicit_null_intermediate`
  + its two-nested-null and remove-no-op siblings in `command.rs`; the mirrored fixtures in
  `store.test.ts`) plus a realistic `buildSceneDoc`-path regression test. Commits `4c1c46f`,
  `585265c`. (Found by the Task 14c player e2e.)

- [Client, more serious] A single failed optimistic op wedged the write queue: after the override
  failure above, the next unrelated intent threw the same error and never committed, so one bad
  dispatch blocked all later writes on that client until reload. Root cause: `OptimisticClient.rebuildView`
  applied every pending intent's ops directly onto the live view with no isolation, so a throwing op
  aborted the whole rebuild and re-threw on every subsequent `applyIntent`/`applyCommand`/`reject`.
  Fixed by applying each pending intent onto a scratch view adopted only if all its ops succeed; a
  throwing intent is skipped, logged via an injected `Logger`, and left in `pending` for the server's
  confirm/reject to remove. Authoritative `base` application stays strict. Regression tests in
  `optimistic.test.ts` (bad intent between two good ones — good lands, bad stays pending, warn fired;
  `applyCommand`/`reject` still function with a bad intent queued). Commit `4c1c46f`. (Found by the
  Task 14c player e2e.)

## Server + client / ui-state persistence

- [Race] Concurrent sessions of the SAME user clobbered each other's `ui_state`. `PUT
  /api/me/ui-state` replaced the whole blob (`routes.rs::put_ui_state`), and each client session
  held its own in-memory snapshot of the entire `{global, worlds}` object
  (`sessionState.svelte.ts`) — a read-modify-write with no merge or concurrency control. A session
  that fetched its snapshot before another session's write and persisted after it silently
  reverted the other session's slice (e.g. a panel-layout dock made in tab A vanished when tab B
  persisted a locale/lastWorld change). Product impact: same account in two tabs/devices. Test
  impact: the ui-e2e suite runs 6 parallel workers all logged in as `ops`, so cross-worker
  clobbers intermittently broke `panels.spec.ts` "survives a full page reload" at the
  panel-restore assert (2 of 3 full-suite runs on 2026-07-31; 8/8 green in isolation). Fixed by
  narrowing the write granularity to per-slice merge instead of whole-blob replace: server-side,
  `SqliteRepository::merge_ui_state` (`data/sqlite.rs`) merges a partial patch into the stored
  state in a single transaction — each top-level key present in the body replaces the stored key
  wholesale, except `worlds`, whose entries each replace only that world's slice; absent keys are
  untouched, and the 64KB size cap now applies to the merged result
  (`http/routes.rs::put_ui_state`, `MAX_UI_STATE_BYTES`). Client-side, `sessionState.svelte.ts`
  tracks which slices are dirty (`global` flag + a `Set` of touched world ids) and `persist()`/
  `flushOnUnload()` send only a `UiStatePatch` covering those slices (`api.ts::putUiState`),
  clearing the dirty markers on success and re-marking them on failure so a retry doesn't lose the
  write. Concurrent same-user sessions now contend only on slices both sessions actually write,
  instead of last-writer-wins on the whole blob. Commits: `daf5eae` (server per-slice merge),
  `819d2c0` (client dirty-slice patches).

  A whole-branch final review found the clobber still reachable WITHIN one slice: `worlds.<id>`
  holds `panelLayout` (panels module) vs `chatRead` (chat module), and `global` holds `locale` vs
  `lastWorld` (two independent mutators) — two owners of the same slice still last-writer-won each
  other. Fixed by extending the merge/dirty-tracking granularity to the individual leaf key
  (`global.<field>` / `worlds.<id>.<key>`): `merge_ui_state` merges one level inside `worlds.<id>`
  and inside any other top-level object key (a leaf blob like `panelLayout` still replaces
  wholesale — no deep merge), and `sessionState.svelte.ts` tracks dirty fields/keys instead of
  whole slices. `flushOnUnload()` also now re-marks on a rejected keepalive PUT (it previously
  cleared its dirty tracking unconditionally, silently dropping the write on failure).

- **[Type incompleteness] `ClientMsg`'s `scene_subscribe` variant omitted the `as_user` field the
  client actually sends.** `src/client/core/src/ws-client.ts:998` sends
  `{ type: "scene_subscribe", request_id, channel, ...(opts.asUser ? { as_user: opts.asUser } : {}) }`,
  but the `wire.ts` variant declared only `type`, `request_id` and `channel`. The spread bypasses
  TypeScript's excess-property check, so it compiled and there was **never any runtime
  misbehaviour** — the server accepts and gates the field. The defect was that `wire.ts` is the
  client's statement of record for the protocol and under-described it: a reader could not see that
  `scene_subscribe` carries `as_user`, and any call site constructing the message without a spread
  would have been rejected by the compiler for sending a field the protocol does support.
  **Resolution:** added `as_user?: string;` to that variant, documented with the server's actual
  authorization semantics rather than a paraphrase — GM-only, with a non-GM caller receiving
  `scene_error` "not authorized to view as another user" and a non-member target receiving
  `scene_error` "target user is not a member of this world", the target's role resolved from the
  server's own membership record so a client-supplied role or scope is never trusted
  (`src/server/src/ws/conn.rs:1313-1329`). Type-only change; verified zero runtime effect —
  typecheck 0 errors, `@shadowcat/core` 392/392 tests pass, `lint:props` unchanged at 1266 (the new
  field carries its own doc, so it adds no finding). Found by the docs sweep 13 Task 2 implementer
  and deliberately held out of that task, which is comment-only and whose diff was under review;
  fixed immediately once that review closed.
