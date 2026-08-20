# TODO — Deferred Work

Actionable, externally-logged deferrals. Bugs go in `OPEN_BUGS.md`, not here.
As of the Phase-1 cleanup burndown (2026-07-19), most items below are
retained because their blocking capability doesn't exist yet — a concrete
unblocking condition, not a "someday maybe." A few headings are explicitly
labeled "Actionable now": these are NOT blocked on anything — the underlying
capability already exists — but are deferred as out-of-scope-for-now work.

## Scheduled after the debt-burndown phases — re-brainstorm point-in-time replay redaction
- TODO: Re-run the design pass for the commit-time redaction context that closes the two
  replay-redaction defects in `OPEN_BUGS.md`. The first proposal was reviewed by two blind reviewers
  and returned needs-rework; **do not restart from scratch and do not patch that proposal** — its
  reviewed findings are the input, captured under `docs/superpowers/specs/` as the Phase-1b design
  findings.
  The corrected framing established by that review: redaction must be the CONJUNCTION of what was
  permitted at commit and what is permitted now, where the commit-time view is carried by the
  operation and the current view may only WITHHOLD visibility, never grant it. A pure snapshot
  closes the loosening leak and opens a tightening one, because reading current state is exactly
  what makes retroactive hiding work — both reviewers found that independently.
  Owner rulings already taken: build the resync lower bound as well as the snapshot (the bound is
  fail-closed, immediate, and independently valuable, since any member can currently request the
  entire world history unvalidated); carry the context on the operation rather than a sibling map or
  a log column; capture per COMMAND, not per op; cover all three operation arms, since create and
  delete are not point-in-time correct despite carrying their document.

## Blocked on a reverse-proxy deployment story
- TODO: `ClientIp` resolves solely from `ConnectInfo<SocketAddr>` — the real
  peer address of the accepted TCP connection — with no `X-Forwarded-For`/`Forwarded` handling.
  Behind a reverse proxy that does not preserve the original client address, every request
  resolves to the proxy's own address, so the per-IP throttle bucket (`login:ip:<>`/
  `invite:ip:<>`) degrades to a single shared bucket across every real client — throttling still
  functions per-identity, just not per-real-IP. No reverse-proxy deployment story exists or is
  scoped today (verified: `docs/design/` and the `config` module have no proxy/trusted-header handling);
  resolve alongside whatever design adds one (a naive trust-any-`X-Forwarded-For` fix would be
  its own spoofing vulnerability without a configured trusted-proxy list).

## Blocked on a per-turn movement-budget system (Phase-2 combat)
- TODO: `move_exec::execute_move`'s `MoveOutcome.cost` accumulates only the entered cell's terrain multiplier per step (`cost += regions.terrain_multiplier(region_cell)`); the `pathfinding` module's router cost also multiplies by the diagonal-rule `step_cost` (`sc * mult`, where `sc` is 1.0/2.0/√2/alternating depending on `world-settings.pathfinding.diagonalRule`). The two "cost" values are not numerically comparable once diagonal movement is involved under any non-Chebyshev rule — they coincide only because Chebyshev's diagonal step cost is 1.0. This is a deliberate M10g Task 7 scoping decision (`move_exec`'s center-cell, terrain-only accounting model), not an oversight, and nothing currently consumes or compares the two values. Resolve before any per-turn movement-budget system consumes `MoveOutcome.cost`/`MoveStream.cost`: decide whether `move_exec` should thread the diagonal rule + per-step parity to match the router's preview cost, or whether route-preview cost and execution cost are intentionally distinct quantities. (Surfaced by the M10g Task 7 buddy check.)
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
- TODO: `evaluate`'s `ref` case and `substituteIdentifier` both wrap a consumer resolver call in a near-identical try/catch → `resolver-error` FormulaError. `resolveAll`'s equivalent catch is entangled with the internal `NeedsDependency` trampoline signal and can't share a naive helper without leaking that control-flow type across the `internal` module's validation-only boundary — so only `evaluate`/`substituteIdentifier` are realistically unifiable. Factor a small shared helper for those two call sites if `@shadowcat/formula` grows more consumer-callback boundaries. (Surfaced by the M13a whole-branch buddy-check fix-confirmation review.)

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

## Actionable now — ui-state per-key merge (Task 4 final-review backlog)
- TODO: `worlds` in the stored `ui_state` blob is grow-only — `merge_ui_state` never removes a
  `worlds.<id>` entry or a leaf key within it, only inserts/replaces. A world a user leaves (or a
  stale panel-layout/chat-read key a module retires) accumulates forever, and there is no recovery
  path if an accumulated blob ever exceeds the 64KB merged cap short of a manual DB edit. Add
  `null`-removes-entry/key semantics to the merge rule (mirroring `FieldChange.remove` elsewhere in
  the data layer) plus client-side pruning (e.g. dropping a `worlds.<id>` entry for a world no
  longer in the caller's membership list) so an over-cap blob is recoverable.
- TODO: `put_ui_state` opens the single-connection tx via `merge_ui_state`
  before the merged-size check runs (the check happens inside the tx, after the read). A cheap
  route-level pre-check (e.g. rejecting a patch whose own serialized size already exceeds
  `MAX_UI_STATE_BYTES`, before touching the pool) would reject an obviously-oversized patch
  without holding the single-writer connection for the read+merge+serialize round trip.
- TODO: the `sessionState` module has no in-flight-PUT ordering guard — `schedulePersist`'s leading
  edge can fire a second `persist()` while an earlier one's `putUiState` is still unresolved (e.g.
  a slow network on the first write, a new mutation arriving before it settles), so two writes for
  the same account can be in flight concurrently with no ordering guarantee on which lands last at
  the server. Defer the leading edge while a persist is unresolved (a simple in-flight flag,
  scheduling the deferred attempt for when the current one settles) instead of the current
  fire-and-forget leading edge.
- TODO: the `sessionState` module's `loaded` flag is never reset to `false` on logout, so a
  mutation landing inside a re-login `loadSessionState()`'s `await getUiState()` window passes the
  `loaded` guard and can persist a pre-login `state` value under the new session's cookie.
  `clearDirty()` at load start covers only the marker half of re-login hygiene; reset `loaded`
  (and cancel the cooldown timer) at logout so the write guard is structural.
- TODO: `buildGlobalPatch`/`buildWorldPatch` enumerate the leaf keys by
  hand — adding a third key to `UiState["worlds"][string]` (or a new `global` field) widens the
  type but silently drops the new key from every patch, with no compile error. Drive the copy from
  an exhaustive `Record<WorldKey, …>`/switch so a widened union becomes a type error.

## Actionable now — Phase D-alpha (movement authority & secrecy) backlog
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



## Actionable now — a literal-field-name key survey misses keys built through a helper
- TODO: When surveying every constructed value of a specific document field (e.g. every
  `property_overrides` pointer key ever set, to confirm none falls outside an allow-listed set),
  a repo-wide grep for the literal field name misses a key built through a helper function whose
  call site never spells the field name itself — the helper's return value is what actually
  reaches the field. Such a survey must also grep the constructing type/helper names, not only
  the literal field. Same family as scoping a search to the shape you imagined rather than the
  shape that exists. No known live miss today — the one such helper found during the
  `property_overrides` band-classifier work was hand-audited and confirmed compliant — but the
  method gap persists for the next survey of this shape.

## Registered and exercised — plugin distribution properties
Registration is per-machine state no committed file can carry. The supported non-interactive path
is the `claude plugin` CLI, not only the TUI command:
`claude plugin marketplace add <your Shadowcat checkout>`, then
`claude plugin install shadowcat-codebase@shadowcat --scope project` run from the consumer repo.
Scope matters — `user` scope would enable it in Shadowcat too and double-register every skill and
agent name.

Per-machine setup actions inside a CONSUMING repo's workspace belong to that repo, not here: a
consumer workspace's trust dialog is tracked in that repo's own backlog.

Settled by direct observation, kept because each is a property to re-check after a refresh:
- A directory-sourced plugin serves a cached SNAPSHOT, not the live repo — CONFIRMED. The payload
  lands at `~/.claude/plugins/cache/<marketplace>/<plugin>/<version>/` as real copied files, and
  the install record pins a `gitCommitSha`. Editing a skill in Shadowcat therefore does NOT reach
  a consumer until `version` in `.claude/.claude-plugin/plugin.json` is bumped and the marketplace
  updated; the "stored exactly once, drift-free" property holds on disk but not at runtime.
- The copy is the whole directory, ignore rules notwithstanding: `settings.json`,
  `settings.local.json`, `kimi.plugin.json` and `graphify/` all ship. The loader consumes only
  `skills/`, `agents/`, `hooks/`, `commands/`, so the rest is inert — but a machine-local
  `settings.json` sitting in a distributed payload is worth re-checking if the loader ever widens.
- Plugin skills ARE addressed under a `<plugin>:<skill>` prefix — the listing offers only
  `shadowcat-codebase:shadowcat-codebase-core`, never the bare id. The agent bodies already name
  this case, and invocation was verified end-to-end from a consumer session.
- The routing hook fires exactly once per session: a consumer that declares no `hooks` key of its
  own gets only the plugin's, while Shadowcat keeps its own wiring and shows the plugin disabled.
- `claude plugin validate` warns that a `CLAUDE.md` at the plugin root is NOT loaded as project
  context. A consumer therefore needs its own adapted `.claude/CLAUDE.md` rather than inheriting
  one — do not "simplify" by deleting it in favour of the shipped copy.
- Both manifests validate with one cosmetic warning each: no `author` field.

## Actionable now — Kimi Code parity is written but never installed
- TODO: In the Kimi TUI run `/plugins install <your Shadowcat checkout>/.claude` then `/reload` (the
  third-party trust prompt defaults to cancel — approve it). CLI/prompt-mode install does not
  exist. Then confirm all six agents register: the twelve ported copies carry Claude-native
  `tools:` names including `Skill`, and whether Kimi recognizes them is unverified — if
  unrecognized entries reject the frontmatter, the agents will not load at all. Separately,
  their bodies say "invoke via the Skill tool", but Kimi's config sets
  `merge_all_available_skills = true`, suggesting skills may be merged into context rather than
  invoked; if so the bodies' BLOCKED branch could fire in a consumer repo even when the context
  is already present. Both need one real dispatch in a consumer workspace to settle.
  - **Attempted, genuinely blocked (not a deferral):** the `kimi` CLI is installed and its
    `config.toml` validates clean (`kimi doctor config`), but `kimi -p "..."` returns
    `provider.api_error: 403 You've reached your usage limit for this billing cycle` — the
    account's quota is exhausted, refreshing next billing cycle. This blocks any real dispatch
    through Kimi (TUI or `-p`/`--agent-file` non-interactive mode) regardless of plugin-install
    status, so the agent-registration and skill-invocation questions above remain unverified for
    a reason outside this session's control. Re-attempt once quota refreshes or extra usage is
    purchased.

## Actionable now — no UI-visible notification channel exists in `ui-kit`
- TODO: Build a UI-visible notification/toast affordance in `@shadowcat/ui-kit` (or wherever the
  host chrome lives) for operation-level feedback a GM or player needs to see — "an operation
  partially applied", "some targets were skipped", and similar. Every existing signal of this
  kind currently goes through `Logger.warn`, whose production implementation writes only to the
  browser console — invisible to an ordinary user. `TemplatesController.push`'s per-instance
  write-authorization exclusion is the concrete instance motivating this: the excluded-instance
  warning is correct and complete against what exists today, but "the user learns" is only
  actually true for a developer with devtools open. Not blocked on anything; deferred because
  building a general notification affordance is a larger, separately-scoped piece of work than
  the task that surfaced the need.
