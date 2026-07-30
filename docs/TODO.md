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

