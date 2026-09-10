# Post-Work Findings

Living record of issues surfaced during review/audit. NOT a to-do list — entries
are observations awaiting triage, not committed work.

- Title: the comment-reference gate reads a doc example's Rust code as prose. Summary: a doctest
  in `chat::TableDrawSegment`'s doc comment carried the struct-literal field `spec: None`, which
  `check-comment-refs.mjs`'s "unnamed spec reference" pattern matched; the campaign implementer
  cleared it by respelling the literal as field-init shorthand (`spec,`). The code was never a
  reference to anything outside the tree, so no rule intent was crossed, but the fix dodged the
  detector rather than teaching it: fenced code inside a `///` block is code, and only its own
  `//` comment lines are prose. With ~790 doctests now in the crate, the same false positive recurs
  the next time a field named `spec` (or a marker-shaped token) appears in an example. Status:
  Needs Review (owner's call whether `subjectGroups` should treat a doc-comment fence body as
  code, scanning only its comment lines).

- Title: two public items have no observable behaviour to exemplify. Summary: `ws::room::RingBuffer`
  and `RingBuffer::new` are `pub`, but every method that does anything (`push`, `range_from`) is
  `pub(crate)` and the buffer's contents have no public accessor, so their required doc examples
  construct the value and can assert nothing; the reviewer confirmed no route through `Room`'s
  public surface observes the ring. The examples say so in a comment rather than asserting a
  tautology. Status: Needs Review (either the type's visibility is wider than its use, or a
  construction-only example is the accepted shape for such items).

- Title: the Rust doc-example gate adds a measurable cost to `cargo test --all`. Summary: the
  server crate went from 94 to ~790 doctests. Each is a separate rustc invocation; the full
  doctest pass measured 390s at 8 test threads on the development machine (94 took 44s), and
  runs on all three CI matrix legs. Edition 2024's merged doctests would collapse most of that,
  but the crate is edition 2021. Status: Needs Review (one measurement; watch the `rust` job's
  duration on the first CI runs before deciding anything).

- Title: whole-record embedded pre-image still key-count-conflicts under load. Summary: the OCC
  pre-image comparison now re-reads an ENGINE-band pre-image through the ingress normalizer, so a
  client pre-image that omits keys the store holds as explicit nulls no longer conflicts.
  `permission::targets_engine_band` deliberately refuses a whole-record `/embedded/<coll>/<idx>`
  path, because such a pre-image carries envelope fields the `Document` round trip would relax —
  admitting it would weaken OCC outside the engine band. A whole-record client write built from an
  optimistic view would therefore still conflict under load. No such writer exists today (every
  whole-object writer targets an `/engine/...` path). Status: Needs Review (revisit if a
  whole-record embedded writer is added).

- Title: Argon2 runs under a size-optimized release profile. Summary: `[profile.release]` sets
  `opt-level = "z"` workspace-wide, which applies to the KDF as well. Dev builds were an order of
  magnitude slower still until `[profile.dev.package.argon2]`/`[profile.dev.package.blake2]` were
  set to `opt-level = 3`; the release profile was left alone. Size optimization of a deliberately
  CPU-bound hash costs login throughput without adding security — an attacker uses an optimized
  implementation regardless — so a matching release-side package override may be warranted.
  Status: Needs Review (measure a release-build hash before changing anything).

- Title: a skill citation of the TypeScript `void` keyword cannot resolve. Summary: the
  symbol-citation gate reports one broken citation in the codebase-skill corpus — a backtick-cited
  `void` in the client-shell skill, describing a return shape. `void` is a language keyword, not a
  symbol the tree declares, so it resolves against nothing. The line belongs to another session's
  uncommitted work in the shared plugin checkout and was deliberately not edited here. Status:
  Needs Review (either rephrase so the keyword is not cited as a symbol, or add a named
  acknowledgement entry covering language keywords alongside the existing `true`/`false`/`NaN`).

- Title: vitest worker start-up timeouts under sibling-worktree cargo load. Summary: on
  the M17c gate run, `pnpm run test:scripts` failed 4 then 2 tests across two runs (each a
  20s/5s per-test timeout, the failing test moving between runs, incl. the skill-corpus
  conservation check), and `pnpm --filter @shadowcat/core test:e2e` failed once on the module
  toolchain spec and once with a `vitest-pool` "Timeout waiting for worker to respond" while
  starting a different spec — all while another session's cargo build was running on the
  machine. Every failing file passed in isolation (117/117, 97/97) with zero code change; no
  assertion failed at any point. Same contention class as the render-ready flakes, but at
  worker start-up rather than inside a test. Status: Needs Review (if it recurs on a quiet
  machine, raise the per-test timeout on the corpus-scanning script tests, which read the
  whole skills checkout).

- Title: ui-e2e hex-movement setup step flaked once on CI. Summary: on the docs
  Phase-1 push (run 30558898300, ubuntu), `hex-movement.spec.ts` "non-GM
  wall-crossing drag rejected" failed at a SETUP assertion — the GM view's
  `Effective owner: <player>` text did not appear within its 15s timeout after
  the Token-owner select — while the other 15 tests passed. Same commit: local
  full suite 16/16 and the CI re-run green with zero code change (the branch's
  only client-runtime-adjacent diffs were type-only), so this is a timing flake,
  not a regression. Status: Needs Review (if it recurs, await the owner-select
  round-trip explicitly or raise that assertion's timeout).

- Title: ui-e2e panels reload test flaked once locally. Summary: on the docs
  Sweep-4 local matrix (doc-comment-only branch), `panels.spec.ts` "a panel
  opened from the launcher docks and survives a full page reload" timed out
  (30s) waiting for `.stage-host[data-render-ready=true]` after `page.reload()`;
  the other 15 tests passed. Immediate re-run of the spec: 3/3 green in 4s with
  zero code change. Same timing-flake class as the hex-movement setup flake
  above (render-ready wait after a heavyweight reload). Status: Needs Review
  (two members of the class now — if a third appears, audit the render-ready
  signal's startup path rather than the individual specs). 2026-07-31 update:
  this SAME spec later failed twice at a DIFFERENT assert (the post-reload
  panel-restore `asset-upload` visibility, not the render-ready wait) — that
  failure mode was root-caused to the same-user ui_state clobber race
  (resolved — see the 2026-07-31 note below and CLOSED_BUGS.md), and is NOT a
  member of this render-ready class (which was then at two members). 2026-07-31
  resolution: the panel-restore assert failure mode is fixed — the ui_state
  clobber race is closed by the per-slice merge (server `merge_ui_state` +
  client dirty-slice `UiStatePatch` writes, commits `daf5eae`/`819d2c0`; see
  `docs/CLOSED_BUGS.md` "Server + client / ui-state persistence"). The
  post-fix Task-3 verification matrix was 15/16 on the full 6-worker
  `pnpm --filter @shadowcat/shell e2e` run — the one failure was THIS spec
  timing out on `.stage-host[data-render-ready=true]` again, i.e. a THIRD
  occurrence of the render-ready class (immediate isolated re-run: 3/3 green,
  same zero-code-change signature as the first two), adjudicated a
  non-regression per this entry's own re-run protocol, not a recurrence of
  the now-closed clobber failure mode. Per this entry's stated policy above
  ("if a third appears, audit the render-ready signal's startup path rather
  than the individual specs"), that audit trigger now FIRES — the class is at
  three members and the startup-path audit is a pending follow-up, not yet
  done. Separately, 5 additional targeted repeats of `panels.spec.ts` alone
  (1 worker, no contention) ran 5/5 green, confirming the per-slice merge
  removes the clobber interference mechanism specifically. 2026-07-31 AUDIT
  RESOLUTION: the startup-path audit ran. Proximate cause of every class
  member: `playwright.config.ts` set no `timeout`, so Playwright's 30s
  default TEST budget equaled the specs' own 30s render-ready ASSERTION
  budget — under measured 6-worker contention (15–53s per test vs 3–8s
  isolated) the assertion could never use its stated window. Fixed: the
  config now sets `timeout: 120_000` + `expect: { timeout: 15_000 }`
  (hex-movement keeps its larger explicit 180s). The audit also surfaced WHY
  the symptom is "element(s) not found" rather than slow-but-green: the
  client has NO Welcome watchdog — plus three adjacent unbounded/unretried
  startup awaits — now closed defects, see `docs/CLOSED_BUGS.md` "Client /
  silent-hang startup paths". Status: suite defect RESOLVED; recurrence
  under the new budgets would indicate a regression in that fix, not the
  specs.

- Title: ui-e2e assets test flaked once locally at the post-login worlds list.
  Summary: on the migration-squash local matrix (schema-file-only branch),
  `assets.spec.ts` "upload an image…" timed out (default 5s expect) waiting for
  `getByText("Your worlds")` right after the login click, while the other 15
  tests — including every other spec's identical login step — passed; sibling
  tests in the same 6-worker run took 15–53s under contention. Isolated re-run:
  3/3 green with zero code change. NOT the render-ready class (that signal is
  the stage's `data-render-ready`; this is the worlds-list route render) — this
  is plain load contention against a 5s default expect timeout. Status:
  RESOLVED 2026-07-31 — exactly this entry's recommendation shipped with the
  render-ready audit fix: `playwright.config.ts` now sets
  `expect: { timeout: 15_000 }` suite-wide.

- Title: ui-e2e stage draw-freehand test flaked once during Task 4 (ui-state clobber fix wave)
  verification. Summary: on the full 6-worker `pnpm --filter @shadowcat/shell e2e` run,
  `stage.spec.ts` "draw a freehand stroke via the tool rail; the drawing renders" failed
  `data-shape-count` at "1" (stayed "0") within its 15s timeout, while the other 15 tests passed.
  Isolated re-run (1 worker, no contention): 1/1 green in 7s with zero code change. Unrelated to
  this task's diff (drawing/shape-count assertion, not ui-state/panel-layout persistence) and NOT
  a member of either documented flake class above (not a render-ready wait, not a login-step
  worlds-list wait) — a new, as-yet-single-occurrence worker-contention timing flake on the tool
  rail's draw-commit path. Status: Needs Review, mitigated 2026-07-31 — the render-ready audit's
  suite fix (`timeout: 120_000` in `playwright.config.ts`) means the 15s shape-count assert no
  longer competes with a 30s whole-test budget; if it still recurs, audit the freehand-draw
  commit's paint timing under parallel-worker contention.

- Title: Phase-B world delete swallows asset-directory removal failures. Summary: `delete_world`
  returns 204 even when `remove_dir_all` on `<assets_path>/<world_id>/` fails for a
  reason other than NotFound (permission error, Windows open-handle lock); the failure is a
  `tracing::warn!` only, so an admin deleting a world for data-removal reasons gets no signal that
  bytes survived on disk. Status: Accepted (final-review Minor; matches the project-wide delete
  convention — rows first, files best-effort, a crash orphans files rather than blocking or leaving
  a live world missing them; revisit only if an admin-visible cleanup report surface is ever built).

- Title: Phase-B world delete evicts live connections before its DB transaction commits. Summary:
  a transient (non-NotFound) failure of `delete_world`'s transaction would leave users kicked from
  a world that still exists — they can immediately rejoin (the tombstone lifts on the failure
  path). Status: Accepted (final-review Minor; the plan explicitly chose evict-first ordering so no
  join can re-hydrate a room mid-deletion — the reverse ordering reopens the ghost-room window the
  tombstone exists to close).

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
  `egress_loop`. Update (M8b-1 push, 2026-06-22): a *second* manifestation observed —
  the authoritative-seq assertion in `converges_with_publishing_during_resync`
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

- Title: an `Update` to a since-deleted document is delivered on replay, redacted against whatever
  document has reoccupied that id.
  Summary: this was believed dropped on replay, with the resulting end state accepted as
  harmless. Neither premise holds: the same lookup can instead resolve to an unrelated document
  that has since reoccupied the deleted document's id, so the stale `Update` is delivered,
  redacted against the wrong document's permission set, rather than dropped. This is a confirmed,
  reachable secrecy defect, not a replay-fidelity limitation. Status: Moved — do not re-file
  here.

- Title: no smaller "caption" text-size token in the M7d token set. Summary: the
  M8b-2 asset panel's tile filename (`Assets`'s `.name`) renders at inherited
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
  `--border`. (2) Fixed a latent M8c-1 bug: `Stage`'s `readColor` used
  `getComputedStyle().getPropertyValue("--token")`, which returns the unresolved
  `var(...)` string for aliased custom properties — so the grid silently used its
  fallback color and ignored the theme; it now resolves the real color via a
  computed-`color` probe. (3) Background uses `--surface-base` (already correct). (4)
  Fog-state colors (dimmed/unexplored) deferred to M9 (no visible fog in identity mode).
  Status: Resolved for M8c (canvas chrome); caption size token → M12 (above).

- Title: M10e-1 config-doc seed races resync (rare double-create). Summary: contribution
  panels (`GameSettingsPanel`, like `FactionsPanel`/`ConditionsPanel`) seed world config-docs
  from a reactive `$effect` that mounts during `#onWelcome` BEFORE the resync stream populates
  the optimistic store (`WsClient`'s frame sequence: welcome → onWelcome+module-activate →
  resync_request →
  event frames → resync_end). The `createSubscriber`+`subscribe()` reactivity + per-doc-type
  `length === 0` guard make a duplicate seed rare, but a GM whose first effect run lands with an
  empty store before resync can still create a duplicate `world-settings`/`light-gradation`/
  `vision-modes`. This is the SAME project-accepted condition as the `WorldSession` scene
  auto-create ("rare multi-GM ... double-create is accepted (M12
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
  previewed route enforced full geometric footprint clearance (`cell_enterable` — the token's
  bounding-disc must clear all `blocksMove` segments and ALL footprint cells must be in the
  non-GM mask); the authoritative movement gate (M9/M10e-4) stayed center-based (parent spec §14).
  Never a bug — the asymmetry was intended (route ⊆ gate-allowed). Status: **Resolved/narrowed
  (Phase D-alpha, D4).** `execute_move` now adopts the router's footprint-aware predicate for
  walls, mask, and impassable — the asymmetry this entry described for those three axes is gone
  (route-admissible ⇔ gate-admissible, **I4**, on `GridStepped`; `route ⊆ gate-allowed` on
  `Continuous`). Arrest and terrain deliberately remain center-cell-only on BOTH sides
  (`cell_enterable` and `execute_move` alike — footprint-gating arrest would make the gate stricter
  than the router and break **I4**), so a narrower version of the same asymmetry persists by design
  for those two axes only. See `docs/superpowers/specs/2026-07-25-phase-d-alpha-movement-authority-secrecy-design.md`
  (D4) and the `shadowcat-codebase-scene-rendering` skill's footprint-predicate bullet (skill
  source: the standalone `shadowcat-codebase` plugin, not this repo).

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
  the `panels` module's barrel), but every future stateful module that needs session context will repeat
  this construct-at-mount + bridge dance — treated as an API bug report per the M12 "built against
  the public API" rule. Status: Needs Review (candidate: a session-scoped module hook or
  context-bearing activation phase, weigh at M12c sheet-registry design time).

- Title: M12a verification gap — native pointer tab-drag not exercisable by automation. Summary:
  Task 10's sweep could not fire a REAL native drag of a dockview tab (CDP does not synthesize
  `dragstart`; the coarse-pointer backend engaged dockview's drop overlay but committed no move).
  Coverage substitutes the `PanelMenu` a11y commands, which share the identical classified-
  `LayoutOp` pipeline (parity documented on `DockviewEngine`), plus geometric assertion of the zone
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
  Summary: `build_message_doc` intentionally writes the SAME `MessageSystem` body
  into both `/system` and `/engine` — the `system` copy stays load-bearing because chat reads still
  deserialize `cur.system` until Task 7 re-roots them, so setting `system: {}` now would break live
  chat. `handle_edit_message`/`handle_delete_message` only construct a `/system` `FieldChange`
  (`WriteOrigin::ServerMessageRevision`); they never touch `/engine`. So a message's `/engine` copy
  reflects only its ORIGINAL Create body — any subsequent edit or soft-delete leaves it stale,
  diverging from the authoritative `/system` body. Nothing reads the message `/engine` band today
  (dead data until Task 7), and this is pre-v1 with a zero-migration policy (no shipped worlds), so
  no cleanup is required — Task 7 must simply not trust any pre-Task-7 `/engine` copy of a
  previously-edited message when it re-roots chat. Separately: `SqliteRepository::apply_command`
  — an ungated trusted substrate with zero production callers
  today — does NOT carry the same broadcast/event-log `/engine` normalization gate added to
  `apply_intent` by this fix; if `apply_command` is ever wired to real undo/replay functionality,
  it must gain the identical gate first. Status: Resolved — M13-0 Task 7 re-rooted both chat READS
  and WRITES onto `/engine` (`handle_edit_message`/`handle_delete_message` now construct an
  `/engine` `FieldChange`, not `/system`; `build_message_doc` writes `system: {}`); `/engine` is now
  the sole source of truth and the staleness window is closed. Re-confirmed at Task 7: `apply_command`
  still has zero production callers (grep across `src/server/src`), so it was not accidentally wired
  into the chat re-root; its missing `/engine` normalization gate remains inert until a real
  undo/replay caller is added, at which point it needs the same gate as `apply_intent`.
  Update — **Resolved.** `SqliteRepository::apply_command` now carries the identical `/engine`
  normalization gate, pinned by
  `apply_command_update_normalizes_engine_broadcast_and_event_log_smuggled_key`.

- Title: Movement gate: token_move gate-dispatch is opt-in on ECS hydration (fail-open shape,
  reachability unconfirmed). Summary: `Room::publish`'s per-operation movement gate only runs when
  `SceneEcs::token_move` returns `Some(...)`, which requires the token to already exist in the
  hydrated in-memory ECS (`self.index.get(&token_id)`). An `Update` operation touching
  `/engine/x,y` on a token the ECS hasn't yet hydrated (e.g. a same-batch Create+Update sequence,
  or an Update racing ECS hydration) would commit UNGATED — the gate is skipped (returns `None`,
  falls through to the ordinary write path), not rejected. This is a PRE-EXISTING shape, unchanged
  by Task 6 itself (Task 6 only moved the gate's read target from `/system` to `/engine`).
  Status: Investigated — unreachable: `SqliteRepository::apply_intent`'s Phase 1 validates every
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

- Title: External-module i18n registration seam missing. Summary: An out-of-tree sheet module has
  no public seam to register i18n keys into the shell catalog, so it must ship its own built-in
  English fallback map behind a `ctx.t` override hook as a workaround. Reinforced by a separately
  discovered test-context gotcha (`setAppContextForTest`'s default `t: (k) => k` echo means such a
  helper always resolves through its own English fallback under test, never through a real
  translation catalog).
  Status: Needs Review (candidate engine seam for a later checkpoint).

- Title: `effect` doc_type constant has no engine home. Summary: D9 makes `effect` a
  client-semantics doc_type but neither M12c (which owns `ITEM_DOC_TYPE`)
  nor the M13b rules plan declares an `EFFECT_DOC_TYPE`; an out-of-tree system module defines its
  own `EFFECT_DOC_TYPE` in its barrel instead. Consider promoting it beside `ITEM_DOC_TYPE` if a
  second consumer appears. Status: Needs Review.

- Title: No browser e2e harness for external modules. Summary: The M13-1 toolchain e2e is
  HTTP-only (no DOM); the spec §11 "Playwright e2e" for M13c has no browser harness, so the
  author→equip→toggle→revert flow (M13c Task 11) is covered by a component-level integration
  test instead. Status: Needs Review (Playwright harness is a toolchain follow-up).

- Title: Two consumer sheet-layer findings moved to the consumer's own repo. Summary: a
  touch-reorder gap on iOS Safari and a silent numeric no-op, both in an out-of-tree system
  module's sheet components, were recorded here while that module's packages were being built
  from this repo. A consumer owns its own source and its own trackers, so both are open bugs in
  that repo instead. Engine-API friction stays here — the split is by which repo's SOURCE carries
  the defect, not by which repo surfaced it.
  Status: Moved — do not re-file here.

- Title: M13d dice-label fix (`bf494c1`) — no Rust-side test for `RollOutcome` missing the
  `labeled_consts` key. Summary: the report's cited backward-compat regression test
  (`stored_pre_m11d2_message_still_deserializes`) doesn't actually exercise a `RollOutcome`-
  shaped blob, so `#[serde(default)]` on `labeled_consts` is backed only by the mechanical,
  already-precedented pattern (`symbol_counts` uses it identically) plus the client-side test
  "parses a labeled Const term in labeled_consts, defaulting to [] when absent", not a Rust-side
  one. Low real risk. Status:
  **Resolved.** `roll_outcome_missing_defaulted_keys_deserializes` now deserializes a
  `RollOutcome`-shaped JSON blob missing both `labeled_consts` and `symbol_counts`, pinning
  `#[serde(default)]` on both directly.

- Title: M13d dice-label fix (`bf494c1`) — a labeled constant's displayed value ignores an
  enclosing `Neg`/`Mul` operator (e.g. `-3[dex]` displays as `3[dex]`, not `-3`).
  `collect_labeled_consts` shows each `ConstTerm`'s raw literal value, mirroring how `DieRecord`
  raw faces are already shown regardless of an enclosing sign — a real precedent, not an
  oversight, but a fidelity gap against `labeled_consts`'s own provenance-transparency intent
  since `total` itself is unaffected and correct. Status: **Resolved.** `collect_labeled_consts`
  now threads an effective sign through `Neg`/`Sub` (`Mul`/`Div` still keep the
  literal, matching `DieRecord`'s own precedent), pinned by
  `labeled_const_display_carries_effective_sign`.

- Title: `ResolvedScene.bounds` has two contradictory unit interpretations, and the grid-unit one is
  wrong on hex. Summary: surfaced by the 14e-7 `[sec]` review (reviewer flagged the hex half as
  inferred; the dispatcher traced the rest and found the inconsistency underneath). TWO consumers of
  the same `.bounds` value disagree on its units. (1) `build_navmesh`
  treats it as GRID UNITS: `(w_px, h_px) = (w * cell, h * cell)`. (2) `bound_for_scene`
  treats it as PIXELS, feeding `width`/`height` straight into `maxx`/`maxy`
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
  `2e6800c`. The weighted continuous branch, in `SceneEcs::pathfind`, converts the grid router's
  cost to scene units with
  `weighted.cost * cell`. That conversion assumes one step spans `cell`
  world units, which holds for `SquareGrid`. On hex, `resolve_grid_shape_with_rule` returns a
  `HexGrid` whose `neighbors_with_cost` is a uniform 1.0 per step, but
  adjacent hex centers are `sqrt(3) * size ~= 1.732 * cell` apart (`axial_to_pixel((1,0)) =
  (size*sqrt(3), 0)`). So a hex + continuous + terrain scene reports a preview cost about 1.73x too
  small, while the sibling pure-polyanya branch on the same scene reports true Euclidean length —
  defeating exactly the unit parity the `* cell` line exists to guarantee. Preview/budget only, not
  a gate. Status: Needs review — likely wants the step-to-world-distance factor to come from the
  `GridShape` rather than being assumed equal to `cell`. Note this compounds with the `bounds`
  units question logged above; both are hex + continuous, which has thin test coverage.

- Title: Hex + continuous `clip_to_visible_mask` has no end-to-end call-site coverage.
  Summary: from the 14e-7 `[sec]` code review. The one integration test for the hex + continuous
  chain runs with `is_gm: true`, so `mask` is `None` and
  `clip_to_visible_mask` returns early at its `mask.is_none() && walls.is_empty()` branch.
  (Citation note: the test this sentence originally named no longer matches its own description —
  it could not be reidentified with confidence during the 2026-08-06 Rule 15 pass; whoever next
  touches this entry should re-locate the actual hex+continuous integration test, if one still
  exists, and cite it by name.)
  Call-site wiring is therefore pinned end-to-end only for `truncate_at_arrest`;
  the shape threaded into `clip_to_visible_mask` — the actual fog gate on the pure-polyanya branch —
  is covered by its unit test but never exercised through `pathfind`. Not a defect (both call sites
  pass the same `&*grid_shape` binding), but a non-GM hex + continuous `pathfind` test with a real
  mask would close the class. Status: Actionable — add the test.

- Title: `panels.spec.ts` reload failures were one pre-existing product defect, not a branch
  regression — Task 3/4 forensic closeout (silent-hang-startup, 2026-07-31). Summary: the three
  distinct-looking `panels.spec.ts` reload failure shapes observed across this investigation
  (wrong-world dock miss and worlds-list-bounce-on-a-since-deleted-world, both captured by this
  task's own trace forensics and reported in `docs/CLOSED_BUGS.md`; render-ready timeout in a busy
  world, the third-occurrence class member logged above) are ONE mechanism: `App`'s `boot()`
  ignored the URL hash's world route on every load and
  unconditionally entered `ui.global.lastWorld` instead. Under the shared-account 6-worker e2e
  suite (all workers authenticate as the same `ops` account and every `enterWorld` persists
  `global.lastWorld`), a reload's `boot()` restores whichever world ANY concurrent worker entered
  last, not the URL's own world — proven by a captured Playwright network trace showing the
  reload's `GET /api/me/ui-state` returning a different worker's world id, followed by the page
  entering that foreign world. This is a real product defect independent of the e2e harness: a
  human reloading a deep-linked world URL in production would be teleported away from it the same
  way. Fixed in Task 4 (route-first boot resolution, see `docs/CLOSED_BUGS.md`). This corrects
  Task 3's report, which adjudicated the render-ready-timeout shape as a startup-contention timing
  flake (and separately misquoted the brief's blocking criterion) — the underlying cause was this
  routing defect, not contention, though the fixed 120s/15s Playwright budgets from that task
  remain independently correct. The Task 4 escalating-watchdog hypothesis (raising
  `welcomeTimeoutMs` to 60s) was FALSIFIED at its own Step-1 gate: the suite still failed
  `panels.spec.ts` at 60s, and the captured trace showed the page landing on the worlds list, not a
  stalled-but-present stage — every captured trace across every investigation round shows the
  `Welcome` frame arriving in under 1s, so no watchdog kill was ever observed. The shipped fixed
  10s watchdog window is exonerated and unchanged. Separately, a stale-binary confound corrupted a
  night's worth of run evidence during this investigation: `pnpm --filter @shadowcat/shell e2e`'s
  `e2e:build` step embeds `dist/` into the server binary at compile time (`rust-embed`), so a
  source edit reverted via `git checkout` WITHOUT an intervening rebuild leaves the previously
  built (and already-embedded) behavior in the binary under test — a constant change believed
  reverted can silently still be live for however many runs preceded the next rebuild. Status:
  RESOLVED — Task 4's route-first fix + this entry close the class; the render-ready timing-flake
  audit and its 120s/15s budget fix remain valid as independent, separate work. The traced run
  underlying this forensic closeout also failed `hex-movement.spec.ts` (trace kept, not separately
  analyzed at the time); it did not recur across the three post-fix full-suite runs (16/16 each,
  Task 4 verification) and needs no further action.

- Title: `TokenAnimator`'s two Event-vs-MoveStream ordering comments contradict each
  other. Summary: `TokenAnimator.animateSamples`'s own comment states "the authoritative
  position Event arrives before the
  MoveStream broadcast (normal server ordering), so reconcile() -> setTarget already
  registered an ease entry", while `TokenAnimator.setTarget`'s own JSDoc and its inline
  guard comment both state the opposite — "handles the typical
  MoveStream-before-Event server ordering". Each cites the other as its rationale. Both
  guards are defensive and order-independent (`animateSamples` deletes any competing ease
  Anim; `setTarget` returns early on `if (this.samplesAnim.has(id))`),
  so playback is correct whichever order actually holds — but exactly one of the two
  comments is wrong about the server, and a maintainer reasoning from the wrong one could
  remove the guard that is in fact load-bearing. Determining the real ordering requires
  reading the server's Event/MoveStream emission sequence, not the client. Found by the
  Sweep 11 whole-branch code review; out of scope there (different package, and Sweep 11
  is comment-only in `src/modules`). Status: Needs Review — resolve when a sweep reaches
  `src/client/render`, or sooner if movement playback is touched.

- Title: `redeem` has no network-exception path, unlike its three sibling handlers.
  Summary: `WorldSelect`'s `refresh`, `create` and
  `confirmDelete` each wrap their work in a bare `catch {}`, which absorbs BOTH an HTTP
  rejection and a network-level `fetch` failure. `redeem` has no `try`/`catch` at all: it
  relies on `acceptInvite` collapsing every HTTP rejection to `null`,
  which is correct and deliberate for the no-oracle
  property. But `postJson` does not catch its own `fetch`, and
  `acceptInvite` does not catch either — so an offline/DNS failure rejects and propagates
  out of the submit handler as an unhandled rejection, showing the user nothing, where the
  other three would have shown their generic error. Reachability: any invite redemption
  attempted while the network is down. Pre-existing; not introduced by Docs Sweep 12, which
  is comment-only. The doc comment now states the gap explicitly rather than implying
  `redeem` is exception-safe. Status: Needs Review — a runtime fix (wrap `redeem`, or catch
  inside `acceptInvite`) belongs on the runtime follow-up branch, not in a docs sweep.

- Title: Client and server enumerate the invite-rejection cases as five vs six.
  Summary: `acceptInvite`'s doc lists five rejection
  categories ("unknown, malformed, expired, revoked, already used"); the server's own
  `accept_invite` doc lists six, treating "wrong secret" (a known invite
  id with a non-matching secret) as distinct from "unknown" (no such invite id). Not a false
  claim: both are caller-indistinguishable, so "unknown" is a defensible abstraction rather
  than an omission — and the code makes that stronger than the prose does, since
  `accept_invite`'s `record.filter(|_| verified)` collapses BOTH conditions into a single
  `AppError::NotFound` branch, so no separate path exists that could leak the difference.
  Found by the Task 4 spec review, which noted the report's verification step quoted the
  server's six-item list and never reconciled it against the five-item client prose it had
  decided to leave verbatim. Status: No action needed — recorded so the asymmetry is not
  rediscovered as a defect.

- Title: `ModuleManager`'s `Promise.all` is correct — do not "harmonize" it to `Promise.allSettled`.
  Summary: Docs Sweep 12 Task 5's Rule 11 pass compared failure handling across the three settings
  panels and reported `ModuleManager`'s `load()` as sharing the
  affordance-loss shape `InviteManager.refresh()`'s `Promise.allSettled` guards against. On
  verification that conclusion is wrong, and the naive fix is destructive. The two reads
  (`listInstalledModules()`, `getEnabledModules(world)`) are independent network calls but NOT
  independent state: `enabled` is the payload `save()` sends as a whole-set replace
  (`setEnabledModules(world, [...enabled])`). Under `Promise.all`, a failure of either leaves
  `installed` empty, so the markup takes its `installed.length === 0` branch — no checkboxes, no
  Save button, plus a visible error paragraph. Under `allSettled` with `listInstalledModules()`
  succeeding and `getEnabledModules()` failing, the list would render with EVERY checkbox unchecked
  beside a live Save button, and one click would persist the empty set, disabling every module in
  the world. `InviteManager`'s `allSettled` is right for the opposite reason: its two reads feed two
  independent displays, so a surviving half is honestly renderable alone. The distinguishing
  question is whether a partial result can be shown without asserting something false — not whether
  the two calls can fail independently. This closes the campaign's outstanding "ModuleManager
  Promise.all, pending Task 5 confirmation" runtime follow-up item: confirmed NOT a defect, and the
  code comment now carries the rationale so it is not re-litigated. Status: RESOLVED — no action.

- Title: `eslint.docs.config.js`'s warn-tier-versus-ratcheted `ignores`-array split is a stale
  record. Summary: this entry described a four-`ignores`-array structure (a `.ts` warn block, a
  `.ts` ratcheted block, and their `.svelte` counterparts) inside a single lint config that no
  longer exists — `eslint.docs.config.js` now carries exactly two `ignores` arrays, one for `.ts`
  and one for `.svelte`, and its own header states there is no advisory tier and no per-package
  staging; a later ratchet collapsed the warn/ratcheted split this entry was reasoning about. The
  entry also miscounted its own `.ts` array as five entries where six exist. The cross-file
  property this repo still enforces — `eslint.docs.config.js`'s and `eslint.props.config.js`'s
  `.ts` `ignores` arrays staying byte-identical — is a DIFFERENT, separately documented invariant
  that this entry was never about. Status: Closed — stale record, not a re-verified disposition.

- Title: `ui-e2e` failed 15 of 16 Playwright tests on `main` @ `11cac8f`, then passed on an
  unmodified re-run of the same commit — a whole-suite flake mode with no captured evidence.
  Summary: CI run `31066304785` reported 15 failures, every one the same shape — after the login
  form was filled and submitted, `getByLabel("New world name")` never appeared and the test burned
  its full 120s timeout (21.1m wall clock, 2 workers). The single pass was `entry-flow.spec.ts`,
  which drives that same flow, so the flow itself is not broken. `web`, `docs`, `e2e` and all three
  `rust` jobs were green in the same run; the client bundle built and the server compiled with no
  panic and no error in the log. Re-running the job on the identical commit turned it green, which
  establishes non-determinism and rules out a code regression at that SHA.
  Three candidate causes were tested and eliminated: (1) a sweep-12 source regression — the whole
  window `7c0dbb9..11cac8f` is comment-only for runtime code, proved by stripping comment markers
  from the entire non-`.md` diff, whose only residual is one vitest test name and ESLint config
  globs; (2) i18n catalog resolution, since the label comes from `aria-label={t("worlds.newName")}`
  — but the same failing tests resolve `t("common.password")` one line earlier, and `I18n` is
  unchanged in the window; (3) in-memory SQLite handing each pooled connection its own empty
  database — `SqliteRepository::connect` sets `max_connections(1)`.
  A local `pnpm e2e` against a freshly compiled binary passed 16/16 in 1.7m on 6 workers, with no
  leftover server on port 31999 that `reuseExistingServer` could have silently reused.
  Cause remains UNKNOWN and is deliberately not guessed at. The leading untested candidate is
  contention: every HTTP and WS request serializes through that single writer connection, and the
  runner is slower and less parallel than the dev machine — but nothing yet distinguishes that from
  a Chromium/runner-level stall. Status: MITIGATED, NOT FIXED — `ui-e2e` now retains Playwright
  traces on failure and uploads `test-results/` (commit `050ec3d`), because this failure was
  undiagnosable purely for lack of captured evidence. The next occurrence must be diagnosed from
  the trace's network log rather than re-reasoned from the console log. Do not treat the green
  re-run as evidence the underlying cause is gone.

- Title: `Table.svelte` captures `session`'s sub-objects at construction —
  `state_referenced_locally`. Summary: repo-wide `pnpm -r typecheck` reports two
  svelte-check warnings (the only warnings in 28 projects), both in `Table.svelte`,
  where `SheetsController` is constructed with `session.contributions` and
  `session.documents` read directly from a `$props()` value. svelte-check's point is
  that these capture the INITIAL value rather than tracking reassignment. Whether that
  is a defect depends on whether a `Table` instance can ever outlive the `WorldSession`
  it was built from: `App.leaveWorld` discards the session and constructs a fresh one,
  so if `Table` remounts per world entry the capture is correct and the warning is a
  false positive — and `WorldSession` is documented as single-use per entry. NOT
  verified either way; the remount path was not traced. Flagged because this codebase
  has shipped real bugs of exactly this shape (a reactive bridge missing its
  subscription, and a contribution seed that had to be made reactive because it mounted
  before resync), and because a stale `contributions` reference is the documented
  failure mode for session reuse — it renders the PREVIOUS world's contributions, which
  is the harder failure to notice. Surfaced while verifying gates for docs sweep 13
  Task 6; the file is untouched by that task and the warnings are pre-existing.
  Status: Needs Review — trace whether `Table` is keyed/remounted per world entry
  before deciding whether to fix or suppress.
  Update — the remount path is now half traced. `App` holds `session` as nullable state:
  `App.leaveWorld` assigns null and `App.enterWorld` assigns a fresh `WorldSession`, and
  `Table` renders only under a guard requiring `session?.role` and `session?.world`. So
  the leave-then-enter path drops the guard, unmounts `Table`, and remounts it against
  the new session — on that path the capture is correct and the warning is a false
  positive. What remains open is narrower: whether `App.enterWorld` can run while the
  guard still holds, i.e. a world-to-world switch that never passes through null. On
  that path `Table` is reused, `session` is reassigned underneath it, and
  `SheetsController` keeps the previous session's `contributions` and `documents`.
  Status: Needs Review — determine whether `App.enterWorld` is reachable without
  `App.leaveWorld` running first. If it is not, the warning is a false positive and the
  capture should be documented as deliberate rather than changed.

- Title: Deletion deny-list may only block the naive invocation. Summary: the
  `permissions.deny` block in `.claude/settings.json` (Shadowcat, and the
  byte-identical copy added to a consuming repo during the agent-parity work)
  lists `Bash(rm *)`, `Bash(sudo rm *)`, `PowerShell(Remove-Item *)` and
  the shell aliases. If Claude Code matches these as literal prefixes, the rules
  catch `rm -rf build` but NOT a chained invocation (`echo ok && rm -rf build`),
  a path-qualified one (`/bin/rm -rf build`), or PowerShell's fully-qualified
  `Microsoft.PowerShell.Management\Remove-Item`. The user directive states `rm`
  "is banned and denied via permissions" — if that is intended as a hard
  backstop rather than a guard against the obvious case, the gap defeats it.
  Predates the branch that surfaced it: the block was already present in
  Shadowcat before any of that work began, and `.claude/settings.json` is
  git-ignored, so `git log -S` cannot date it. The gap remains UNVERIFIED and
  will stay that way: **`rm` is universally banned in every form, and that
  includes running it as a PROBE** (owner ruling). A harmless-looking lookalike
  such as `rm --help` would indeed reveal the matching semantics without touching
  the filesystem, and it is still forbidden — a rule whose enforcement you test by
  performing the banned act is a rule you have already broken, and "it was only a
  probe" is precisely the reasoning the ban exists to refuse.
  Status: Actionable — resolve by WIDENING, never by testing. Knowing whether the
  literal-prefix reading holds is not a precondition for closing the gap: adding
  the chained, path-qualified and fully-qualified forms to the deny list costs
  nothing, is correct under either reading, and needs no experiment. Treat the
  unverifiability as settled, not as pending work.

- Title: A written stat key's `[` can absorb a template's later `]` as a label.
  Summary: in `@shadowcat/formula`'s notation-template grammar, `claimLabelSpan`
  scans forward for a closing `]` through the whole template, so a key containing
  an unmatched `[` behaves two ways depending on its surroundings.
  `checkNotationKey("hp[max")` rejects with a `"parse"` error, while
  `resolveNotationTemplate("hp[max + 1d6] + str", ...)` returns
  `"7[hp][max + 1d6] + 7[str]"` — no error, and the intervening dice expression is
  swallowed as a label rather than rolled. An authoring UI calling the checker is
  told "this will not run" about a key that does run, wrong. Both behaviours are
  measured, documented on `checkNotationKey`, and pinned by a case in
  `template.test`; nothing here was changed. Status:
  **Resolved — ruling: no, and no grammar change.** `[` is reserved
  notation syntax (a label span) and is never absorbable into a valid
  key; a run confirms `checkNotationKey("a[b]")` already returns
  `{ intact: false }` (SPLIT: `identifier "a"` + `label "[b]"`, never one
  claim), so a key carrying `[` is already correctly refused as
  non-intact — the exact signal an authoring UI needs to reject it before
  it is ever saved. The asymmetry in the summary above is not a grammar
  defect: `checkNotationKey` answers "is this key, in isolation, safe to
  use as a reference" and `resolveNotationTemplate` answers "what does
  this whole template's text mean" — two different, correctly-scoped
  questions, exactly as `checkNotationKey`'s own doc already states
  ("the answer is over the key in ISOLATION"). The genuine fix — never
  letting a `[`-carrying key exist to be embedded in a template in the
  first place — belongs at the key-authoring boundary, calling
  `checkNotationKey(key).intact`. Shadowcat's own document field paths
  are structural (typed schema, not freely authored strings), so this
  repo has no such boundary to wire. That boundary is the consuming
  system's (the same one the authorized `validateStatKey` →
  `checkNotationKey` swap targets), not a Shadowcat action item.

- Title: shell ui-e2e reuses a foreign server on its fixed port. Summary: during the
  M15a run (worktree `Shadowcat-m15a`, another session active in the main checkout),
  `pnpm --filter @shadowcat/shell e2e` failed 6/17 — every first spec per worker — at
  the `Created account …` toast, with the error-context snapshot already inside a
  world with an open chat panel: `playwright.config.ts` pins `baseURL` to
  `127.0.0.1:31999` with `reuseExistingServer: !process.env.CI`, so whichever
  session's `test_server` holds the port answers both runs. Rerun alone: 17/17 in
  3.3 min. `http::tests::accept_invite_throttles_by_ip_over_real_transport` (fixed
  port 8004) showed the same shape (404 where 429 was expected) under the same
  contention and passed 3/3 alone. Status: Needs Review (a per-run port or
  `reuseExistingServer: false` locally would remove the class; deliberately not
  changed in M15a).

- Title: `pnpm docs:api:ts` (typedoc) fails on pre-M15 generated engine types.
  Summary: `typedoc` exits 4 with `treatWarningsAsErrors` on
  `CombatantKind.__type.type` and `ResourceBinding.__type.kind` (serde tag
  properties in `src/types/generated/engine/*`, from commit `e4622687`, before the
  M15a branch point), so `pnpm build:all` — and the CI docs job — cannot pass on
  either branch until those tagged unions carry a documented discriminant or an
  exemption. M15a produced `dist-docs` locally with warnings non-fatal only to run
  the local-only `check-skill-api-refs` gate. Status: **Resolved.** The M14c-2
  combat-shapes regeneration carried the discriminants' doc comments through:
  verified `pnpm docs:api:ts` green on post-M14c-2 main (2026-08-31, the
  M14c-4 baseline) with `treatWarningsAsErrors` intact — no exemption needed.

- Title: `shadowcat-codebase-documents-permissions` cites three symbols an unmerged
  branch declares. Summary: running `node scripts/check-skill-symbol-refs-cli.mjs`
  during M14c-5's skill-update gate reported three broken citations in the
  `Operation::Move` bullet — `` `check_parent_placement` ``, `` `check_move_acyclic` ``
  and `` `old_parent_id` `` — because the checker indexes the tree it runs in. They are
  not renames: `git grep` finds all three declared in `src/server/src/data/` on
  `m15b-asset-browser` and `m15-move-hardening` (the Move-op work, whose skill commit
  `docs: Move joins the document-mutation invariants` wrote the bullet) and on neither
  `main` nor `m14c-5-templates-merge`. The skill corpus is shared across worktrees while
  the code is branch-relative, so a citation is checkable only from the branch that
  declares it until that branch merges. Status: **Resolved by merge order** — the
  citations verify once the M15 branches land on main; nothing to correct.

- Title: `shadowcat-codebase-documents-permissions`'s "no bespoke server
  `get_pointer`" claim went stale mid-campaign. Summary: M14c-5's Task 2 added
  `merge::tree::get_pointer` to the server crate; the documents-permissions skill's
  null-intermediate-handling paragraph asserted flatly that no such helper exists
  server-side, which the new function falsified without anyone editing that skill.
  Caught only because M14c-5's Task 6 happened to run the symbol-ref checker over
  the whole corpus, not because anything about the merge module's own review process
  touched this skill. Status: **Resolved** — the claim is rescoped to
  `data::command`'s own pointer family (which still has no bespoke helper) and the
  checker's `get_pointer` acknowledgement entry (`scripts/check-skill-symbol-refs.mjs`)
  removed, since it no longer names a genuine non-existent symbol. Residual risk: a
  "never exists" claim about a whole SERVER (rather than one named module) is
  inherently fragile against future modules landing their own same-named helper —
  future negative-existence claims in this skill family should scope to the specific
  module/subsystem they were verified against, not the whole binary.

- Title: HTTP throttle tests flaked under machine contention during M14c-6.
  Summary: `login_throttles_identity_after_budget_spending_no_argon2`,
  `accept_invite_throttles_by_ip_over_real_transport`, and
  `login_throttles_by_ip_over_real_transport` intermittently failed timing-window
  assertions while three other background agents' own `cargo`/`vitest` runs shared
  the machine concurrently (confirmed via each process's own log identifying its
  own worktree). Isolated re-runs (no contention) passed. Same class as the
  pre-existing "shell ui-e2e reuses a foreign server" and render-ready-timeout
  entries above: real-clock rate-limit windows compete with CPU-bound sibling
  processes for wall-clock budget. Status: Needs Review — not a regression;
  flagged for the same audit already pending against the render-ready class.

- Title: `@shadowcat/shell`'s full `vitest run` intermittently timed out 3 of 10
  worker files (`[vitest-pool-runner]: Timeout waiting for worker to respond` on
  `bootResolution.test.ts`, `route.test.ts`, `smoke.test.ts`) during M14c-6, while
  every file passed individually and the same full run with
  `--no-file-parallelism` passed 10/10 files (115 tests) cleanly. Reproduced
  twice with other background agents' cargo/vitest processes concurrently active.
  Status: Needs Review — machine-contention class, not a code regression; a
  future CI-parity check should confirm the default (parallel) mode is stable
  when run alone.

- Title: Skill symbol citations reference unbuilt combat-resolution symbols. Summary: the
  local-only `check-skill-symbol-refs` gate reports 25 broken citations in the
  `shadowcat-codebase-combat`, `-formula`, and `-scene-rendering` skills (`resolve_budget`,
  `eval_formula`, `formula_host`, `budget_cells`, `replay_step_costs`, and kin) — symbols the
  combat-resolution sub-project of M14c describes but has not yet built. Surfaced during the
  M15b skill-update gate (whose own four skill diffs verify clean); the fix belongs to that
  sub-project's close-out, either by building the cited symbols or re-scoping the prose to
  what exists. Status: Needs Review.

- Title: Skill symbol citations run ahead of the code for M14c/M17/M18 symbols.
  Summary: the local-only `check-skill-symbol-refs` gate reports 59 broken
  citations against the skills corpus at its HEAD commit (measured via a HEAD
  extraction through `SHADOWCAT_CODEBASE_SKILLS_DIR`), spread across the
  actors-tokens, chat, client-shell, combat, dice, documents-permissions,
  formula, module-toolchain, panels, and scene-rendering skills — symbols the
  M14c combat-resolution sub-projects, M17 (vision/lighting), and M18 (token
  enrichment: `ActorEngine.aura`, `sound`, `vfx`, `VfxAnchor`,
  `TokenNodeSpec.aura`, `updateTokenAura`, `auraKey`, `regionTriggers`, and
  kin) describe but have not built. Surfaced during the M16 close-out gates;
  M16's own skill diffs (panels rewrite, client-shell theming bullet) add zero
  new breakage against that baseline. Same class as the M15b
  combat-resolution finding above: the fix belongs to those milestones'
  close-outs, either by building the cited symbols or re-scoping the prose to
  what exists. Status: Needs Review.

- Title: The browser suite's cost is scenario length, not slow operations.
  Summary: a trace of the carried-torch mid-walk spec records 237 Playwright
  actions across three browser contexts in 17.4s with ZERO actions over one
  second (slowest 0.91s); per-action cost there (~73ms) matches the fast stage
  spec (~60ms). Two hypotheses were tested and DISCONFIRMED: CSS/Svelte
  animation stalling actionability checks (the app declares transitions in one
  file), and accumulated state in the shared `sqlite::memory:` database (a
  freshly booted server measured 3.2m against 3.0m for an accumulated one at
  the same worker count). What does drive per-test latency is worker
  oversubscription: the popped-out-arrangement test measures 3.1s at four
  workers and 59.4s at twelve on one machine, ~19x, while total wall-clock
  barely differs. Capping workers took the suite from 235s to 146s and turned
  the one failing upload assertion green. Status: Resolved for the cap; the
  remaining cost is that the dual-session specs re-run a full world + account +
  invite + join + actor-authoring setup per test. Sharing that setup would
  trade per-test isolation for roughly 35s and is an owner decision, not taken.

- Title: 94 Rust doctests cost 41.3s, a quarter of `cargo test --all`.
  Summary: the Rust suite runs 2719 tests in 165s with zero failures, and the
  library unit tests are 2502 of those in 7.94s. The single largest line item is
  `Doc-tests shadowcat` at 41.31s for 94 tests (~440ms each), which is compile
  and link cost per doctest binary rather than runtime. No defect; recorded so
  the figure is not re-diagnosed as a runtime regression. Status: Needs Review.

- Title: 96 `expect(getBy*(...)).toBeTruthy()` assertions add nothing.
  Summary: across 33 unit test files, assertions wrap a Testing Library `getBy*`
  query in `.toBeTruthy()`. Those queries throw when no element matches and
  otherwise return an element that is always truthy, so the wrapper cannot fail
  independently of the query. These are not false passes — the query itself
  still fails the test when the element is missing — so they are redundant
  rather than defective, and were left untouched. Replacing each with a real
  assertion on the element (text, attribute, state) would raise coverage.
  Status: Needs Review.

- Title: The browser suite's entry-flow spec is fully covered elsewhere.
  Summary: both of its assertions are proven by other specs — the world-list
  visibility by the asset-browser and chat-media specs' opening steps, and the
  post-creation stage canvas by the stage spec's first test, which additionally
  proves `data-render-ready`, a non-zero bounding box, and teardown. Removing it
  deletes coverage that is duplicated rather than unique, but it is a deletion
  of a test and was left to the owner. Status: Needs Review.

- Title: The browser suite hangs intermittently at the capped worker count.
  Summary: CORRECTS the entry above, which reported 34/34 in 146s at four
  workers as if it were representative. It was one sample. Two later runs at the
  same worker count on the same commit produced 9 failures in 17.9m and 7
  failures in 20.0m. The bulk of that wall-clock is three dual-session specs
  (the combat tracker turn cycle, the movement-budget gate, and the hex
  wall-crossing rejection) each burning their full `DUAL_SESSION_TIMEOUT_MS`
  budget — six minutes apiece — so these are HANGS, not slowness. Collateral
  timeouts then hit tests that pass in about a second alone, including the
  token place-and-drag test failing on `mouse.move` after 60s.
  Ruled out by measurement, not argument: the machine and the built artifact are
  fine (the stage spec runs 11 tests in 12.4s isolated, every test 0.6-1.6s, right
  after a failing suite run); disk is not the constraint (`PhysicalDisk` queue
  length 0 and 0% disk time during a slow run); and no competing session or
  foreign server was present (nothing listening on the suite's port, no other
  working copy's processes). What a live sample DOES show is one
  `chrome-headless-shell` consuming 577% of a core — 5.8 cores — with a second at
  218%, which is consistent with a hung spec spinning a software-rasterized WebGL
  canvas and starving its sibling workers. That is a mechanism consistent with the
  observations, NOT a diagnosed cause; the trigger is unknown and the hang did not
  reproduce in any isolated single-spec run attempted.
  Also corrects a measurement method used throughout: the browser process is
  `chrome-headless-shell`, not `chrome`, so any process-count or RSS figure
  gathered by sampling `chrome` undercounted the browsers to zero.
  Status: Needs Review — open, reproducible, undiagnosed. The worker cap itself
  remains supported (per-test latency 3.1s at four workers against 59.4s at
  twelve, same machine, fresh server); what is NOT supported is any claim that
  capping workers makes the suite reliably green.

- Title: Software-rasterized canvases, not worker count, are the browser suite's
  ceiling. Summary: REFINES the entry above, which recorded the 577%-of-a-core
  renderer as a mechanism "consistent with the observations, NOT a diagnosed
  cause". A per-process CPU-delta sample during a stalled full-suite run
  measures two `chrome-headless-shell` renderers at 883% and 869% of a core --
  roughly 8.8 cores EACH -- with browser and node processes summing to 1856% of
  the 2400% the machine has, while 44.9 GB of RAM sits free. The renderer
  initializes Pixi with `antialias: true` at the device pixel ratio, which is
  nearly free on a GPU and multiplies per-pixel work under SwiftShader, and
  SwiftShader parallelizes across every core it can reach. So ONE canvas can
  demand ~9 cores and a dual-session spec runs TWO.
  This overturns the worker-count framing: the machine saturates at roughly two
  active canvases regardless of how many workers are configured, which is
  precisely why wall-clock barely moved across 4, 6 and 12 workers while
  per-test latency inflated ~19x. Raising workers subdivides a fixed
  rasterization budget rather than adding throughput. Under saturation a page
  stops servicing Playwright's protocol at all -- a `.poll()` with its own 20s
  budget ran past the test's 360s budget without firing, and two of the three
  six-minute hangs stalled in `playerCtx.close()` during context teardown, not
  in any assertion.
  Bounded by measurement, not argument: the five dual-session specs run ALONE at
  the same worker count pass 8/8 in 5.1m, so no single spec hangs; the hang needs
  the full suite's combined canvas load and is therefore a saturation threshold
  rather than a defect in one spec. Disk is not involved (`PhysicalDisk` queue
  length 0, 0% disk time) and neither is memory.
  What is NOT yet established: whether disabling antialias for the suite (the
  single largest per-pixel term, and one no assertion in the suite depends on)
  drops the per-canvas cost enough to clear the threshold. That is a render-init
  change and is the owner's call. Status: Needs Review.

- Title: Disabling multisampling clears the browser suite's saturation
  threshold; the continuous re-render behind it remains.
  Summary: ANSWERS the open question in the entry above. Four consecutive
  full-suite runs with `antialias` off pass 33/33 in 105-138s with slowest tests
  of 3.4s, 9.7s and 9.8s. The comparable runs with it on are two passes at 146s
  and 169s and two failures at 17.9m and 20.0m, with slowest tests of 31.3s and
  59.3s. Multisampling is a per-pixel term a GPU absorbs and a software
  rasterizer pays in CPU, and no assertion in the suite inspects edge quality --
  the specs read the stage through its debug attributes, never its pixels.
  A retraction worth recording with it: a single per-process CPU sample taken
  during the first antialias-off run showed renderers still at 888%, 857% and
  650% of a core, and that was read as the change having failed. It had not; the
  instrument was wrong for the question. Per-process CPU stays high because the
  software rasterizer parallelizes across whatever cores exist, so it reports
  demand rather than the per-frame work that actually changed. The measure that
  answers "did this help" is the suite's own pass rate and per-test durations
  across repeated runs, and one run of either arm decides nothing given a spread
  that runs from 105s to 20m.
  What remains, and is NOT fixed: `pixi-backend`'s `createPixiBackend` registers
  a ticker callback and the render package contains no `app.stop()`, no
  `autoStart: false` and no `maxFPS`, so Pixi re-renders the full canvas on
  every tick whether or not the scene changed. The engine's `pingsActive` and
  `emotesActive` flags gate only those overlays' redraw, not the canvas render.
  A canvas therefore costs a full software rasterization ~60x/second while a
  test sits waiting on an assertion, and on a GPU-less host that is the standing
  cost antialias only trimmed. Rendering on demand would remove it and would
  also stop an idle client burning a laptop's battery, but it is a render-
  architecture change requiring every mutation source to invalidate correctly.
  Status: Needs Review.

- Title: The browser CI job is green; the canvas cost behind it is unchanged.
  Summary: `ui-e2e` had failed on every run for some time. It now passes, from
  two changes that both follow the rasterization mechanism rather than guessing
  at the symptom: the suite's build drops multisampling, and the hosted runner
  executes the suite single-worker because it has two cores and a dual-session
  spec opens two canvases by itself, so any second worker guarantees more
  canvases than cores. Evidence, stated at the confidence it has: two of two
  reruns of the same commit pass at one worker; at two workers the same commit
  gave one pass and one failure (the movement-budget gate spec consuming its
  whole 360s budget), which is a threshold being crossed rather than a defect in
  that spec; before either change, four consecutive runs failed. Two samples is
  what supports this, not a characterization of long-run stability.
  The underlying cost is untouched: the renderer still redraws the full canvas
  every tick whether or not the scene changed, and the fixes buy headroom under
  it. The suite is therefore one runner-capacity change away from red again, and
  the durable remedy is rendering on demand. Status: Needs Review.

- Title: The browser suite was software-rendering by CONFIGURATION, not necessity.
  Summary: CORRECTS the three entries above, which treated software rasterization
  as an inherent property of running headless and reasoned only about how to
  reduce the work it did. Probing the same browser the suite launches shows the
  machine's real device was available to it the whole time: default headless
  reports `SwiftShader`, while the identical launch with `--use-gl=angle`
  reports the host's discrete GPU, as does a headed launch. Selecting the
  platform GL backend takes the full suite from 235s with failures to 21s with
  33/33, over three consecutive runs. ANGLE falls back to SwiftShader by itself
  where no device exists, so the flag needs no per-environment gating.
  The measurements in those entries were not wrong -- whole cores per canvas,
  saturation at roughly two canvases, the unbounded per-tick redraw -- but the
  conclusion drawn around them was aimed at the wrong layer. The question
  "how do we make software rasterization cheaper" was never the question; it
  was "why is this rasterizing in software at all", and nothing measured to that
  point had asked it. Every accommodation built beforehand (dropping
  multisampling, capping workers) was optimizing a cost that did not need to
  exist on a host with a GPU.
  It also exposed a real defect the slow renderer had been hiding, since fixed:
  scene gestures converted a canvas-local point to page coordinates from a
  bounding box read earlier and then clicked absolute coordinates, which performs
  no actionability check. Closing a docked panel resizes the canvas, so the box
  could describe a layout that no longer existed and a placement landed on the
  panel, silently placing nothing. It passed for years only because the renderer
  was always slow enough to finish the relayout first. Status: Resolved.
  Still true and still unfixed: the renderer redraws the full canvas every tick
  whether or not the scene changed. A GPU absorbs that; a host without one does
  not, which is what CI still runs on.

- Title: Cargo's job budget is not clamped to the host, and a repo-scoped one
  reached CI.
  Summary: A repo-scoped `[build] jobs` value exists to keep a build from
  starving the desktop it shares with its user. It is not a ceiling relative to
  the host: with 40 requested against 24 logical cores and a workload of 40
  independent crates, 32 concurrent rustc were observed. On a host with FEWER
  cores than the budget it therefore raises concurrency rather than lowering it,
  and multiplies peak memory. A runner pays for that as an allocation failure
  rather than as slowness -- the observed shape is a metadata read failing with
  the OS reporting the paging file too small, which reads like a corrupt
  artifact and not like a resource budget. The comment in the config asserted
  the opposite ("cargo already caps concurrency at the available parallelism")
  and had never been measured; two runs carrying the config passed before one
  failed, so the config is not established as the cause of that failure, only as
  something that moves its likelihood. `CARGO_BUILD_JOBS` overrides the file
  (measured: 6 requested inside the repo yields 6, not the file's 12), so the
  build hosts that compete with nothing now set it to their own width.
  Status: Resolved.

- Title: A fix reported as closing a defect class had reached half its sites.
  Summary: Scene gestures that convert canvas-local points through a separately
  read bounding box were corrected in the three spec files whose tests had
  failed, and the class was reported closed. Three more carried the same shape,
  including one never named in the original diagnosis. The cause is structural
  rather than an oversight: six spec files each held a PRIVATE copy of the same
  gesture decision, so there was no single place a fix could land, and the three
  that happened to fail were the three the slow renderer had exposed. The
  gestures now live in one shared module and every private copy is deleted; the
  origin needed for the pointer primitives that cannot be expressed
  element-relatively is reachable only through a helper that settles the layout
  first, so a caller cannot reintroduce the conversion. Status: Resolved.

- Title: The browser specs were never typechecked.
  Summary: The shell's tsconfig included only `src`, so nothing type-checked the
  Playwright specs -- Playwright transpiles without checking. A spec could name a
  helper that does not exist, or pass the wrong shape, and only a run would say
  so. The directory is now in the include, verified by a positive control rather
  than by a clean result: a deliberate type error is reported, and its removal
  returns the check to clean. Status: Resolved.
