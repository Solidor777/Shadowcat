# Closed Bugs

Confirmed-real defects that have since been fixed, kept for provenance. New fixes append a new
`##` section (or bullet under an existing one); do not delete resolved entries.

## Tooling — `WORD_NARRATION_TOKEN` false-positived on an enum-variant name cited in a skill

- [CI-fatal, FIXED] `scripts/check-comment-refs.mjs`'s coverage control
  (`"coverage control: the governed skill corpus has no unrecognised candidate tokens"`, run by
  `pnpm run test:scripts`, the `web` CI job's own step, not wrapped by any `pnpm lint:*` alias)
  reported RESIDUE for `Replaced` in `shadowcat-codebase-assets`'s `ws::protocol` bullet, which
  cites `AssetOp::Replaced`'s bare variant name inline in backticks as a wire-protocol value. The
  skill-only half of `WORD_NARRATION_TOKEN` treats `replaced` (among five other words) as
  narration of the code's own past, correctly for ordinary prose but not for a code symbol quoted
  as a value — the same "value, not a process id" collision the general `ACKNOWLEDGED` list
  already resolves for `PanelLayoutV1`/`Vec2`, just unresolved for this checker's separate
  `ACKNOWLEDGED_NARRATION` list. Fixed by adding a new `ACKNOWLEDGED_NARRATION` entry recognizing
  a PascalCase word immediately followed by a closing backtick within the class's `contextChars`
  window — case-sensitive, so it exempts only the enum-variant spelling convention and leaves
  lowercase prose narration, backtick-quoted or not, flagged as residue. Regression coverage: a
  positive control on the exact live occurrence and a second PascalCase word, plus negative
  controls for unquoted lowercase narration and lowercase-backtick-quoted narration.

## Server / data — unrestricted `property_overrides` pointer substituted or panicked the envelope

- [Critical, FIXED] A `property_overrides` pointer was unrestricted to the four content bands the
  egress path special-cased, so a self-targeting `/permissions` pointer silently substituted the
  fail-closed default `PermissionSet` for a redacted viewer, and a nested `/permissions/...`
  pointer stripped a required field and panicked the redacting request — a denial of service
  against every reader of that document, reachable by any holder of `cap::EDIT_PERMISSIONS`.
  Fixed by making redaction operate on content bands, never the structural envelope, through one
  shared classifier: `REDACTABLE_BANDS` names the four redactable bands, and `redaction_target`
  maps a pointer to `RedactionTarget::Band` (null in place), `RedactionTarget::Within` (pointer
  strip, now provably landing inside untyped or optional data), or `None`. Ingress
  (`validate_property_overrides`, enforced at all four write paths — `apply_intent`'s Create and
  Update, `apply_command`'s Create and Update) rejects an unclassifiable pointer as `BadPath`
  before it is ever stored. Egress (`filter_properties`) now returns `Result<Document,
  RedactionError>`; the single panicking assertion this bug tripped — the re-deserialize of the
  redacted value back into a `Document` — is gone, while the function's other `.expect()`, the
  serialize of an owned document into a `Value`, is infallible by construction, is not a redaction
  outcome, and stays. Every caller fails closed
  on `Err` — broadcast drops delivery to that recipient, `list_documents`/`search` omit the item,
  the single-document read errors, and the search-index builder writes empty public content rather
  than failing the write. `collect_hidden` reads the same classifier, so the change-delta path
  cannot diverge from whole-document egress. Regression coverage: per-pointer ingress rejection for
  every structural envelope field, acceptance for the four bands and their nested forms, a
  regression test pinning the exact nested-permissions input to `RedactionError` instead of a
  panic, and a mutation check that removing a band from `REDACTABLE_BANDS` fails the suite.

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
  several lattice-corner ties in a row), `single_endpoint_on_lattice_corner_includes_flankers`
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
  client actually sends.** `WsClient.subscribeScene` sends
  `{ type: "scene_subscribe", request_id, channel, ...(opts.asUser ? { as_user: opts.asUser } : {}) }`,
  but the `ClientMsg` variant declared only `type`, `request_id` and `channel`. The spread bypasses
  TypeScript's excess-property check, so it compiled and there was **never any runtime
  misbehaviour** — the server accepts and gates the field. The defect was that `ClientMsg` is the
  client's statement of record for the protocol and under-described it: a reader could not see that
  `scene_subscribe` carries `as_user`, and any call site constructing the message without a spread
  would have been rejected by the compiler for sending a field the protocol does support.
  **Resolution:** added `as_user?: string;` to that variant, documented with the server's actual
  authorization semantics rather than a paraphrase — GM-only, with a non-GM caller receiving
  `scene_error` "not authorized to view as another user" and a non-member target receiving
  `scene_error` "target user is not a member of this world", the target's role resolved from the
  server's own membership record so a client-supplied role or scope is never trusted
  (`egress_loop`'s `SceneSubscribe` handling). Type-only change; verified zero runtime effect —
  typecheck 0 errors, `@shadowcat/core` 392/392 tests pass, `lint:props` unchanged at 1266 (the new
  field carries its own doc, so it adds no finding). Found by the docs sweep 13 Task 2 implementer
  and deliberately held out of that task, which is comment-only and whose diff was under review;
  fixed immediately once that review closed.

## Server + client / pathfind route cost carried two units under a one-unit contract

- **[hex] The route cost field carried cells from one movement model and world units from the
  other.** `ServerMsg::PathResult`'s doc states its cost is "in cells (client multiplies
  `grid.distance.perCell`)". The grid A* router honoured that. `SceneEcs::pathfind`'s `Continuous`
  branch rescaled its weighted sub-path by `GridShape::world_units_per_cell`, and its pure-polyanya
  sub-path returned a Euclidean world-unit length directly; `conn` forwarded the value unchanged.
  `makeMeasureTool` multiplied by `grid.distance.perCell` regardless, so on a continuous scene it
  applied the game-distance scale twice — at the common authoring of grid `size: 100` and
  `perCell: 5`, a five-cell route labelled 2500 ft where it should read 25 ft. Reproduced verbatim
  before the fix via `pathfind_dispatches_to_the_navmesh_router_for_a_continuous_scene`: the raw
  (pre-fix) assertion pinned `outcome.cost` at ~900 for a 9-cell straight route.
  **Resolution:** one unit on the wire — cells, converted at the boundary, never at the consumer
  (the ruled shape; a unit discriminant on `PathResult` was explicitly rejected as re-opening the
  same forked-decision defect class). `SceneEcs::pathfind`'s weighted continuous sub-path no longer
  multiplies by `world_units_per_cell` (`pathfinding::find`'s cost is already in cells). The
  pure-polyanya sub-path divides its Euclidean scene-unit cost by `world_units_per_cell` exactly
  once, after `navmesh::truncate_at_arrest`, guarded against a non-finite or non-positive divisor
  (refuses via `PathFail::Invalid` rather than dividing into an infinity a client would render as a
  label). Every server test asserting a continuous route cost at world-unit magnitudes was
  re-derived from the cell count and the fixture's own `world_units_per_cell`, including three hex
  fixtures whose straight-line comparison values needed the same conversion; tolerances were
  re-scaled through the same division rather than left at their pre-conversion absolute magnitude.
  A new test, `pathfind_grid_and_continuous_report_the_same_cell_cost_for_a_straight_route`, routes
  identical straight-line geometry through both engines and asserts equal cell costs — witnessed by
  mutating each engine's conversion in turn and confirming the shared test fails, then reverting.
  Client-side, `makeMeasureTool`'s second labelling branch (the no-token-selected fallback, which
  rendered a bare cell count with no unit — `"5"` vs the route branch's `"25 ft"` for the same
  distance) was unified with the route branch through one `formatCellDistance` function; the `⚠`
  arrest marker stays route-branch-only by being appended to that function's return value at the
  route branch's own call site, not by a caller-specific flag on the shared function. Verified: the
  sole production consumer of `PathResult.cost` is the measure tool's label — no gate reads it and
  no per-turn movement budget exists server-side, so this was a display correction with no authz
  dimension.

- **[settings] A vision mode's authored `default_range` reached no mask, leaving its GM control
  inert.** `VisionMode::default_range` was written at three sites inside
  `SceneEcs::resolved_vision_modes` and read by nothing: `SceneEcs::token_vision_floors` looked a
  mode up only for its `illumination_floor` and `render_hint`, taking the range from
  `VisionAssignment::range` unconditionally — a plain `f64` with nowhere for a fallback to attach.
  `GameSettingsPanel`'s GM-only number input persisted, round-tripped and validated a value that
  changed nothing on the table, as did the client's seeded `darkvision` default of 12 cells.
  **Fix:** `VisionAssignment::range` is now `Option<f64>`, and `token_vision_floors` resolves an
  absent range against the referenced mode's default at the point it joins the assignment to the
  registry — never as a struct-level serde default, which would make every reader see a number the
  registry never supplied. Both quantities are authored in cells and are compared against the same
  converted distance, so a resolved default is never scaled relative to an explicit override.
  The unknown-mode path still drops the assignment before any range resolution runs, so a mode
  missing from the registry remains fail-closed. A non-finite or negative default makes the range
  test false unconditionally, which is the under-reveal direction.
  **An omitted range is not an omitted assignment**, and the two now mean opposite things: every
  writer in the tree — including seeds, fixtures and tests — either supplies an explicit numeric
  range or omits the whole assignment, verified by an independent sweep during review.

## Client / render — hex token animation forked the per-step distance from the server

- **[hex] `TokenAnimator`'s tween measured a travelled distance against the grid's indexing scale
  where the server measures it against the per-step distance, so hex token animation ran `√3`
  too slow.** `startAnim` divided the travelled pixel distance by the bare `AnimationConfig`
  field fed from `GridSpec.size` — on hex the cell's OUTER RADIUS, not the distance between
  adjacent centres — while the server converts the same travelled distance through
  `GridShape::world_units_per_cell` to compute the authoritative `MoveExecution::duration_ms`.
  **Fix:** `Grid` gained a public `worldUnitsPerCell()` method (`size` on square, `size *
  sqrt(3)` on hex — mirroring the server's `world_units_per_cell` in the client's casing so the
  two are greppable as one concept), and `AnimationConfig.cellSize` was renamed to
  `worldUnitsPerCell` throughout the animator/`TokenView`/`RenderEngine` chain, with
  `RenderEngine` now deriving the value from `Grid.worldUnitsPerCell()` instead of passing
  `GridSpec.size` straight through. Regression coverage: a duration test pinning a one-hex-step
  tween to the true per-step time, and a cross-language parity test asserting the client's
  measured inter-center distance (derived via `Grid.snap`'s production round-trip, not a
  restated formula) equals `worldUnitsPerCell()` for a stated hex size — witnessed on both the
  client and server sides by mutating each `world_units_per_cell` to return the bare size and
  confirming the corresponding test fails.

## Client / docs tooling — the assembled documentation site was not a valid diff instrument

- [Docs tooling, FIXED] `scripts/assemble-docs.mjs`'s `assemble()` composed `dist-docs/` by
  `cpSync`-ing three source trees into `out` without ever clearing `out` first — `cpSync` overwrites
  a file both sides produce but never removes a destination file whose source stopped producing it,
  so a page renamed or deleted upstream survived in `out` across every local rebuild indefinitely.
  Confirmed as a genuine false-positive generator, not just clutter: a prior comparison of
  `dist-docs/` before/after a real config change showed a 6-file delta that a control build with an
  UNCHANGED config reproduced identically, because the baseline already held roughly 40 stale files
  from unrelated earlier builds. Fixed by calling `rmSync(out, { recursive: true, force: true })` at
  the top of `assemble()`, scoped strictly to the caller-supplied `out` path — treated as cleaning
  disposable, git-ignored, machine-regenerated build output (the same class as `cargo clean`), not a
  case where the project's ban on permanent-deletion commands applies. Regression test: builds into
  a temp `out` twice, with the second build's inputs missing a file the first build's inputs had;
  confirmed failing against the pre-fix code (the stale file survived) before the fix landed.

## Client / panels — `PanelsApi.open` narrated nothing when it changed a panel's placement

- [a11y, FIXED] `PanelHost.svelte`'s `describeOp` mapped every layout-changing op to a `panels.moved`
  screen-reader announcement except `"open"`, on the documented reasoning that no control the host
  renders dispatches it — false: `PanelsApi.open` is public and reachable via
  `SceneBrowserPanel`'s configure button and `SheetsController.openDocument`. `applyOp`'s `"open"`
  case has two focus-bump branches (already docked/floating) and one real-placement-change
  fallthrough covering THREE prior states — minimized, closed, and popped-out — surfacing the panel
  via `detach` + `placeByPlacement`. Fixed by threading the pre-op layout through
  `PanelsController.dispatch` → `onOp` → `describeOp`, so the `"open"` case can call `locate` on the
  BEFORE state and narrate exactly when it was `"docked"`/`"floating"` and skip otherwise — inverted
  to match `applyOp`'s actual condition (`prevWhere === "docked" || prevWhere === "floating"` skips)
  rather than enumerating the fallthrough's members, so a future `PanelLocation` variant can't be
  missed the same way an enumerated-member guard would miss one. Regression tests cover all three
  real-placement-change priors, both focus-bump priors, and the
  true no-op case (asserting `onOp` isn't even called, per `dispatch`'s SAME-REFERENCE NO-OP
  CONTRACT).

## Client / scene-tools — `makeTemplateTool`'s click-to-place default never fired in a snapping scene

- [UI, FIXED] `makeTemplateTool` snapped its drag anchor (`onPointerDown`) but passed the RAW
  pointer point to `sizeDir` from `onPointerMove`/`onPointerUp` — a coordinate-frame mismatch that
  meant `sizeDir`'s own near-zero-drag fallback (`d < 1` → the intended one-cell default template)
  almost never fired in a snapping scene, since an ordinary click lands some arbitrary sub-cell
  distance from the snapped anchor rather than within one scene unit of it. A plain click instead
  took the normal branch and created an arbitrarily small, unintended template. Fixed by snapping
  the pointer point at both call sites before passing it to `sizeDir`, restoring the same
  coordinate frame `onPointerDown`'s anchor already uses — matching this tool's own documented
  design (grouped with `makeWallTool`/`makeRegionTool`/`makePlaceTool` as snapped-point tools,
  `makeDrawTool` the sole documented exception). Deliberately did NOT add a create-skipping extent
  guard (unlike `makeDrawTool`/`makeWallTool`/`makeRegionTool`, which correctly skip persistence on
  a zero-extent gesture): `sizeDir`'s `d < 1` fallback IS this tool's extent guard by design — a
  plain click is meant to place a real, sensibly-sized default template, not skip creation, which is
  what the fallback existed to guarantee before the frame bug defeated it.

## Client / game-settings — the "hyperlinks" checkbox was permanently non-functional on every world

- [UI, FIXED] `GameSettingsPanel`'s hyperlinks checkbox sent `chatsys.hyperlinks ?? false` as the
  OCC pre-image, while the field's actual stored value is `null` (never a bare `false` — ingress
  normalization always stores an explicit `null` for an absent optional field, and no code path
  ever writes a literal `false` there). The server's field-level OCC check compares `Null` against
  `Bool(false)` via the catch-all `_ => a == b`, which is always false, so every toggle was rejected
  as a stale-pre-image conflict, 100% reproducible, with no self-heal since a rejected intent
  mutates nothing and the field stays `null` forever. Every other nullable control in the same panel
  already passed `?? null`; hyperlinks was the sole offender. Fixed by changing the `onchange`
  handler's pre-image argument to `chatsys.hyperlinks ?? null` (the `checked={... ?? false}` DISPLAY
  expression is correct and unchanged — it mirrors the read path's own `unwrap_or(false)`).
  Regression test seeds `hyperlinks: null` specifically, since the pre-existing test's
  `hyperlinks: false` seed is the one value this bug is invisible for (`false ?? false` is a no-op).

## Client / render — hex lighting/fog overlays painted axial indices at square positions

- [hex, FIXED] `PixiBackend.setLighting` and `cellsToRects` (the explored-fog rasterizer) each
  placed a lit/explored cell at `x = i · cellSize, y = j · cellSize` and filled an axis-aligned
  rect — correct on a square grid, but on a hex scene the server sends lit/explored cells as axial
  `(q, r)`, so this painted skewed square overlays under correctly-drawn hexes (grid lines, cursor
  snapping and measurement already went through `Grid`'s correct axial math privately). Fixed
  (`c3fb921c`) by delegating to `Grid.cellVertices`, the active grid's resolved corner geometry, instead
  of assuming a square shape — `cellsToRects` and `LightingFrame`'s cells now carry resolved
  `corners: Point[]` (a rect on square, a hexagon on hex), and the paint methods treat `corners`
  opaquely, performing no grid-kind math of their own.

## Server + client / scene — a hex token was drawn and collided as smaller than the hex it occupies

- [hex, FIXED] `resolveTokenBox` sized a token's drawn footprint as `actor.size.w * cell` using the
  scene's `grid.size` — on hex, the cell's CIRCUMRADIUS, not a usable per-axis scalar — while a
  separate `footprintRadius` reduced the same authored size to a collision disc via an unrelated
  formula; the two didn't agree with each other, and both undersized a hex token (measured, for a
  1×1 token at hex circumradius `size`: drawn box `size × size` against a hex spanning `√3·size` by
  `2·size`; collision radius `0.707·size` against a hex inradius of `0.866·size`), letting gaps a
  hex should block stay passable. Fixed (`0e22c913`) by resolving a token's footprint ONCE,
  server-side, and putting it on the wire — `SceneEcs::resolve_token_footprint` computes a single
  collision radius (a token's authored size counts HEXES; the conservative enclosure of one hex is
  its own circumradius, never the square half-diagonal a square/circle formula gives on hex) and a
  `FootprintsPayload`/`SceneFootprints`/`TokenFootprint` channel carries the matching drawn extent
  to the client, which renders the authoritative resolved geometry instead of mirroring the formula
  that produced it — so the drawn box and the collision disc can no longer disagree, by
  construction rather than by review.

## Client / assets — `AssetResolver`'s cache-bust never self-healed from a missed `AssetChanged` frame

- [Client, FIXED] `AssetResolver.revs` was a client-local, purely relative counter bumped by
  exactly one step whenever `onAssetChanged({op: "replaced"})` fired — it never read the asset's
  server-side authoritative `version`. `AssetChanged` is broadcast out-of-band
  (`Room::broadcast_aux`): it is never pushed to the ring, never resynced, and drops entirely if
  there are no receivers. An ordinary reconnect during the window a GM replaces an asset — no lag,
  no unusual load — comes back subscribed past the frame, and a connection that falls behind
  resyncs via the ring/log tiers, which never held the aux frame either. Because the counter was
  purely relative, a missed bump was invisible and unrecoverable: `url()` returned the same
  cache-busted string forever, no new request was ever issued, and the stale image persisted until
  a page reload. Fixed in two parts. Part A: `ServerMsg::AssetChanged` gained a `version: Option<i64>`
  field (`Some` the bumped, authoritative value for `Replaced`; `None` for `Deleted`, which has no
  version), threaded from `http::assets::replace`'s already-bound `version`; `AssetResolver.onAssetChanged`
  now SETS its per-uuid revision to the frame's absolute `version` (never regressing below a
  version already held) instead of incrementing a relative counter, so two clients that each missed
  a different number of frames still converge once a live frame does arrive. Part B: a new
  `AssetResolver.reconcile(assets: Asset[])` method adopts each listed asset's true `version` (and
  clears a stale `deleted` marker for any id present in the listing), wired into every existing
  touchpoint that already fetches full `Asset[]` records — `Assets.svelte`'s `reload`,
  `AssetPicker.svelte`, and `VisualKindEditor.svelte`'s `refreshAssets` — so opening any of those
  panels self-heals a stale cache-bust state for that session with no new polling mechanism.
  No data loss or authz effect: this was a staleness bug, not a security one.

## Server / assets — `http::assets::delete` broadcast a stale pre-delete version under a racing `replace`

- [High, FIXED] `http::assets::delete` fetched the asset row once at request entry (`existing`),
  before the row was actually removed, and broadcast `existing.version` in the `AssetChanged`
  notice — discarding the authoritative row `delete_asset`'s `DELETE ... RETURNING *` already
  returned. `write_barrier`'s read side (held by both `delete` and `replace`) excludes only a
  backup's write side, not a racing `replace` on the same asset id: a `replace` could commit
  (bumping the version and broadcasting `Replaced{N+1}`) in the window between `delete`'s initial
  read (capturing version `N`) and its own `delete_asset` call — a window with no upper bound,
  since these routes disable `DefaultBodyLimit`. When `delete`'s broadcast then fired with the
  stale `version: N`, a client that had already adopted `Replaced{N+1}` correctly rejected the
  stale `Deleted{N}` notice under `AssetResolver.adoptVersion`'s asymmetric gate — so the client
  never learned the asset was deleted at all, and since a deleted asset is absent from every future
  `listAssets` response, `reconcile` never revisited it either: the client kept resolving a URL that
  now 404s indefinitely, the exact "no self-healing path" failure class the sibling `AssetResolver`
  fix above closed, reintroduced from the server side. Fixed by binding `delete_asset`'s returned
  row and using its `version`/`storage_key`/`world_id` for the post-delete file removal and
  broadcast, instead of the stale `existing` snapshot (`existing` is still used for the pre-delete
  `require_gm`/`NotFound` checks, which must run before the row is touched). `delete_asset`
  returning `None` (a second concurrent delete on the same id, already handled by whichever request
  actually removed the row) now returns `204 No Content` with no broadcast, rather than reading
  fields off a snapshot for a row no longer there. Regression coverage: an HTTP-level test races
  the real `DELETE` endpoint against a direct `replace_asset_bytes` call in one `tokio::join!`,
  reading the broadcast `AssetChanged` frame off a connected WebSocket, with a bounded retry loop
  (freshly uploaded asset per attempt) that keeps racing until a replace genuinely lands inside
  the handler's request-to-delete window and asserts the broadcast then carries the bumped
  version, not the stale pre-delete snapshot — and a concurrent-HTTP-DELETE test asserting no
  panic, no `500`, and never a second `AssetChanged` notice under a real race. Found during a
  scoped re-review of this subsystem's other work, not a client-observed report.

## Tooling — `check-comment-refs.mjs`'s "unnamed spec reference" detector missed the same referent class for "brief"

- [Tooling, FIXED] The `"unnamed spec reference"` pattern was keyed on the literal word `spec` and
  carried no vocabulary for a dispatcher brief, an ephemeral, process-assigned document the same
  RULE 16 class covers. A `"unnamed brief pointer"` entry already existed, but its construction
  (a possessive, or "the brief" as a deferring verb's subject followed by `requires`/`says`/
  `specifies`/`states`) matched only the `the` determiner and missed the exact confirmed instance
  ("...exactly the fixture the brief calls for" — "calls for" was absent from the verb list).
  Fixed by widening the entry to `(?:the|this)[\s]+brief'?s\b` and
  `(?:the|this)[\s]+brief[\s]+(?:requires|says|specifies|states|calls for)\b`, adding `this` as a
  second determiner and `calls for` to the verb list. `per` was deliberately left out: reading
  `src`/`scripts`/`examples` and the tracked skill directories found no `per brief` occurrence, and
  unlike `per spec` (an established compound this project already writes) nothing in the corpus
  shows "per brief" used the same way, so adding it would widen the pattern beyond what the
  population needs.
  The determiner-plus-brief-as-SUBJECT construction was kept, rather than switched to a bare
  `(?:the|this|per)\s+brief\b` match, because reading (not grepping) `check-brief-rules.mjs`'s own
  prose surfaced a real collision: "an implementer obeys the brief, not the guidance" uses "the
  brief" as the deferring verb's OBJECT to describe the category of dispatch briefs in general, not
  to point at one specific, now-gone document. A bare determiner-plus-noun match would have flagged
  it; the subject-side construction does not, because "the brief" there is followed by a comma, not
  a deferring verb.
  Regression coverage: a positive control for each of "the brief calls for" and "this brief
  specifies", negative controls for "a brief pause" and "keep it brief" (the ordinary-adjective
  collision the determiner gate already excluded), and a negative control reproducing the
  `check-brief-rules.mjs` collision verbatim. Verified end-to-end: `node
  scripts/check-comment-refs.mjs` rejects a real "the brief calls for" comment with a nonzero exit,
  not just a report.

## Client / shell — a world-route deep link could render a stale "Connecting…" frame after boot() fell back to login

- [routing, FIXED] `App.svelte`'s `boot()` navigates away (e.g. to `login`) and sets `booted =
  true` in the same synchronous tick on a deadline or a failed `getMe`/`loadSessionState` fetch,
  but `navigate()` (`route.svelte.ts`) only wrote `location.hash`; `currentRoute()`'s reactive
  `route` state updated only off the hashchange listener, which a real browser dispatches
  asynchronously and jsdom does not dispatch at all on a bare hash assignment. On a page loaded at
  a `#/world/<id>` deep link with a down/unreachable backend, this left a window where `booted` was
  already `true` but `route.name` was still `"world"`, hitting the template's `{:else if
  route.name === "world"}` branch (no session) and rendering a stale "Connecting…" before the
  hashchange event landed and flipped the view to `<Entry>`. Fixed by having `navigate()` update
  `currentRoute()`'s state synchronously in the same call, alongside setting `location.hash` — the
  listener still fires afterward for a self-triggered navigation (a harmless redundant
  re-assignment of the same route) and remains the only update path for a navigation this module
  did not initiate (back/forward, the user editing the URL bar). Regression coverage: a
  `route.svelte.ts` unit test proving `currentRoute()` updates with no hashchange event dispatched
  at all, plus an `App.svelte` end-to-end test of the deep-link-with-failing-backend scenario.
