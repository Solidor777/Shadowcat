# TODO — Deferred Work

Actionable, externally-logged deferrals. Bugs go in `OPEN_BUGS.md`, not here.
As of the Phase-1 cleanup burndown (2026-07-19), most items below are
retained because their blocking capability doesn't exist yet — a concrete
unblocking condition, not a "someday maybe." A few headings are explicitly
labeled "Actionable now": these are NOT blocked on anything — the underlying
capability already exists — but are deferred as out-of-scope-for-now work.

## Blocked on world/user deletion
- TODO: Purge `explored_fog` rows on world/user deletion. Neither has a route at all — world and user are DB rows, not documents, and no deletion path exists yet. The M9c table denormalizes `world_id` for a world-scoped purge; wire a `DELETE FROM explored_fog WHERE world_id = ?` when world deletion lands, and index `world_id` then. (Surfaced by the M9c-1 buddy check.)

## Actionable now — `explored_fog` purge on scene deletion
- TODO: Scene deletion is already reachable today via the generic `DELETE /api/documents/{id}` route (`http/routes.rs::delete_document`, which reaches `Operation::Delete` for ANY document, scenes included — no doc_type restriction) — but nothing purges the scene's `explored_fog` rows when that happens, and there's no GM-facing "delete this scene" button in the UI yet. Since scene deletion already works mechanically, closing the fog-purge gap is now simple: wire a `DELETE FROM explored_fog WHERE scene_id = ?` into the scene-delete path. Orphaned rows are harmless in the meantime (reads key on the exact never-reused `(scene_id, user_id)` UUIDs) but accumulate unboundedly over a server's lifetime.

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

## Blocked on user deletion existing — `add_member` resolve+write is two queries
- TODO: `add_member` (`http/routes.rs:514`) checks `user_exists` and then inserts membership in two
  separate pool round-trips. A user row deleted between them reintroduces the foreign-key 500 that
  the 404 fix removed. Unreachable today — no user-deletion route exists anywhere in the server —
  but it is the check-then-act shape this project has been bitten by before, and it goes live the
  moment account deletion ships. Fix then by wrapping resolve+write in one transaction.

## Actionable now (deferred on cost, NOT blocked) — inherited owner is a stranger at egress
- TODO: Task 14i made token ownership EFFECTIVE (`effective_owner(token) = the token's own owner,
  else the linked actor's owner`) on the write-authz path and on the four scene vision/lit-mask
  sites, but the EGRESS path was not migrated: `filter_properties`, `collect_hidden`,
  `filter_command` and the document routes still resolve `is_owner` from the literal `doc.owner`.
  So a player who inherits a token through its actor can MOVE it and SEE through it, while being
  treated as a stranger for that token's `owner_or_gm` property tiers and its `/base` field. The
  direction is under-permit (fail-closed), so nothing leaks — the inconsistency is that write says
  "owner" and egress says "stranger" for the same user and document. Closing it needs a
  per-recipient, per-event actor lookup on the egress hot path, which `filter_command`'s own
  comment already flags as pool-contended. Nothing prevents doing it today — the deferral is a
  cost judgement (a query per recipient per frame), not a missing prerequisite; a resolved-actor
  cache would make it cheap but is not scoped work and must not be treated as a blocker.
  **Second, sharper consequence, not just the `owner_or_gm` tiers:** on a token whose
  `permissions.default` is `"none"` with no per-user entry, an inheriting owner gets
  `WRITE_FIELDS` at `apply_intent` (via the owner floor) but is denied `cap::READ` at egress — a
  document its owner can write but never receives. Unreachable on `buildTokenDoc`'s shipping
  `observer` default, so not live today, but whoever closes this should fix the asymmetry, not
  only the property tiers. (Disclosed by Task 14i; see
  `.superpowers/sdd/task-14i-token-ownership-report.md`.)

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

