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

## Blocked on a wire-facing Tier/recalc construction path
- TODO: `eval::classify::classify`'s ladder lookup (`Vec<Tier>`) has no construction-time
  validation that `margin_offset` values are unique — a malformed ladder with a duplicate
  offset ties on `max_by_key`/`min_by_key`'s caller-order-dependent semantics (documented in
  `classify.rs`'s doc comment), so which duplicate wins depends on vec order rather than being
  deterministic. Not reachable today (M11b-1 authors `Tier` lists directly, no untrusted
  construction path exists yet); add a uniqueness/sortedness guard when a wire-facing
  `Tier` construction path appears. STILL OPEN after M11d-2: the wire boundary it wired is
  notation-only, and notation has no tier-ladder syntax — `Tier` lists remain
  struct-authored with no untrusted path. (Surfaced by the M11b-1 whole-branch review.)
- TODO: `DieKind::Faces` (M11b-3) has two unguarded panic surfaces, mirroring the existing
  `min > max` / dice-count-cap gaps above — `DieKind::validate()` (which rejects an empty
  `faces` list) is never called from any production code path, only from `spec.rs`'s own unit
  tests. (1) An empty-`faces` `Faces` die reaching `roll_uniform(rng, 0, faces.len() as i32 - 1)`
  (`eval::mod::roll_expr`) computes a degenerate `span == 0`, causing an unconditional
  divide-by-zero panic (`u32::MAX % span32`), not a silent underflow. (2) An out-of-range
  `natural` reaching `face_value_and_symbols`'s `faces[natural as usize]` (`eval::groups`)
  panics via index-out-of-bounds — concretely reachable via `recalc::RecalcOp::ReplaceDie`,
  which (unlike `RerollDice`) has no `Faces`-vs-`Numeric` gate at all and will happily write an
  arbitrary `natural` onto a `Faces` die's base record. Neither is reachable from untrusted input
  today (no notation path constructs `Faces` yet — M11b-3 is struct-only for face-lists).
  PARTIALLY RESOLVED (M11d-2): the wire boundary (`chat/rolls.rs::validate_pre_roll`) now
  calls `DieKind::validate()` on every parsed group, closing (1) for any future
  notation-constructed `Faces`; (2) — `RecalcOp::ReplaceDie` writing an out-of-range
  `natural` onto a `Faces` record — remains open and resolves whenever recalculate gains a
  wire exposure (recalc-from-chat is itself deferred, see the Follow-on feature sub-projects
  section below).
  (Surfaced by the M11b-3 Task 5 code review.)

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

## Actionable now — server-side hex-grid movement support (design approved, plan not yet written)
- TODO: Hex grid was explicitly in original scope (`docs/PLAN.md`: "grid (square / hex)") and the client already renders/measures hex correctly, but the server has ZERO hex-aware movement infrastructure — `movement.rs`'s line-traversal primitive, `pathfinding.rs`'s A* router, and `scene/mod.rs`'s visibility-mask cell iteration (which feeds both fog-of-war secrecy and the movement gate) are all hardcoded square-grid. This was previously mis-filed as "blocked on hex-grid pathfinding support," which wrongly implied hex itself was unbuilt. Design approved: `docs/superpowers/specs/2026-07-22-hex-grid-server-movement-design.md` (a `GridShape` abstraction generalizing the existing square-grid modules, with a frozen-fixture parity proof before any hex cutover). Awaiting an implementation plan.

## Blocked on real-time per-recipient move-streaming
- TODO: Live cross-animation concurrency for streamed move vision (`MoveStream`). M2 precomputes each move's per-recipient vision clip at *its* execute time, so two tokens moving simultaneously do NOT reveal each other mid-walk when a watcher's vision opens after the clip — it reconciles at the stop + next `vision` rebroadcast. Wanted eventually. Needs real-time per-recipient streaming (a per-move server loop recomputing each recipient's visibility of every concurrently-moving token as positions advance) instead of execute-time precompute. No correctness/secrecy impact today — only a missed transient reveal. (Design `2026-06-25-m2-streamed-continuous-vision-design.md` §8; user wants it as a follow-up.)

## Blocked on `@shadowcat/formula` gaining more consumer-callback resolver boundaries
- TODO: `evaluate.ts`'s `ref` case and `template.ts`'s `substituteIdentifier` both wrap a consumer resolver call in a near-identical try/catch → `resolver-error` FormulaError. `graph.ts`'s equivalent catch is entangled with the internal `NeedsDependency` trampoline signal and can't share a naive helper without leaking that control-flow type across `internal.ts`'s validation-only boundary — so only `evaluate.ts`/`template.ts` are realistically unifiable. Factor a small shared helper for those two call sites if `@shadowcat/formula` grows more consumer-callback boundaries. (Surfaced by the M13a whole-branch buddy-check fix-confirmation review.)

## Blocked on real-world need (low-priority polish, inert until it matters in practice)
- Server shortcodes: pre-parse replacement also fires inside markdown code spans; refine to
  skip code spans if it ever matters in practice.
- TODO: `handle_send_message`'s ownership check (`ActorNotSpeakable`) verifies existence/doc_type/owner but not that the actor doc's world scope matches the sending world — inert today (foreign refs fail closed to no attribution on every reader's client); pin the scope when convenient.

## Blocked on stronger backup/restore atomicity becoming operationally necessary
- TODO: The backup mechanism's assets-copy step is not transactionally coupled to the `VACUUM INTO` DB snapshot. An asset REPLACE (not create) in flight during backup commits its DB row before renaming its temp file into place (`http/assets.rs` — `replace`), so a backup racing an in-flight replace can capture updated asset metadata with the pre-replace file bytes for a few milliseconds' window. Inherent property of any online (no-downtime) backup of a live mutable system; add a brief write-quiesce mode during backup if stronger consistency is ever needed in practice.
- TODO: `restore_backup`'s destination writes (`tokio::fs::copy` for `world.db`, `remove_dir_all` + `copy_dir_recursive` for the assets directory) are not atomic swaps. A failure partway through (disk full, permission error, process kill) can leave the destination db truncated or the assets directory in a state worse than either the pre- or post-restore content. Accepted tradeoff for the "basic" gate-precondition feature; a stronger-consistency restore (write to a temp path, atomic rename into place) is a candidate follow-up if this is ever exercised in an environment where a mid-restore crash is a real operational risk.

## Reference notes (not deferrals — kept for context)
- `axum_test::TestServer` builds request URLs via the `url` crate's `Url::set_path`, which performs WHATWG dot-segment normalization CLIENT-SIDE before the request is ever sent — a path segment that is an EXACT match to `.`/`..`/`%2e`/`%2e%2e` (and case variants) is silently collapsed/popped before it reaches the router, let alone a handler. Any HTTP-level test in this codebase that tries to smuggle a bare dot-segment through `TestServer::get`/`.post` to exercise a server-side path-traversal guard is vacuous — it proves nothing about the guard logic, since the segment never survives to hit it (confirmed: `serve_module_file_rejects_an_id_segment_that_escapes_the_modules_root` in `src/server/src/http/module_routes.rs` still passes against a deliberately-reverted, vulnerable guard). Segments that are NOT an exact dot-form match (e.g. `%2e%2e%2fsecret.txt` as one combined segment) are NOT normalized and do reach the handler intact — only bare exact-match dot segments are affected. Write future path-traversal HTTP tests either as (a) a pure unit test of the containment predicate itself, (b) a symlink/alias-based HTTP repro (module_routes.rs's `self-link`-style test), or (c) an encoded segment embedded inside a longer non-exact-match string. (Surfaced by the M13-1 Task 5 buddy-check fix-confirmation review.)
- Design note (module requirements are advisory): module-declared manifest `requirements` are
  published to clients as advisory UX only and are NOT server-enforced at `apply_intent` (server
  authority stays with the GM's `world_cap_requirements`, per ARCHITECTURE invariant 6). A future
  explicit "GM adopts a module's requirements into the world policy" mechanism could make them
  enforced if desired. (Surfaced by the M13-1 Tasks 8+10 buddy-check.)
- Module-toolchain scope exclusions: module upload/install UI (M13-1 T2) — install stays manual-extract into `<data-dir>/modules/<id>/`; sandboxing/permissions for installed module JS (M13-1 T2) — modules are admin-trusted, same tier as the server binary; hot enable/disable of installed modules without a client reload (M13-1 §2); module marketplace/registry, signing, or update channels (M13-1 §2).

## Follow-on feature sub-projects (own brainstorm → spec → plan each)

Out of scope for the Phase-1 cleanup burndown; built after Sub-project 1, one design pass each
(user: build ALL of bucket C):

1. **Recalc-from-chat** — persist `spec`/`raws` on `RollEmbed` (persistence + secrecy fork);
   closes the `DieKind::Faces` `ReplaceDie` guard at the same boundary (see above).
2. **Link-preview extensions** — server-fetch-cache-as-asset **image** pipeline + async
   post-publish enrichment (`WriteOrigin` path) + **shared preview cache** + **oEmbed** provider
   embeds (user opted both edge items in; oEmbed carries SSRF/privacy surface → threat-model it).
3. **Per-world export/import** — world-scoped row subset preserving cross-FK referential
   integrity + shared asset references.
4. **Dice-notation grammar growth** — math fns (floor/ceil/round/abs/min/max) + crit-event /
   tier-ladder notation syntax (also opens the Tier-uniqueness guard above).
5. **Per-channel / per-message dice-settings overrides** — needs a channel model.
6. **In-body doc-link chat segment** (`Segment::DocLink`) — actor-name → sheet navigation shipped
   in M12c, but a free-form doc-link segment has no server producer or client authoring path yet;
   needs a server producer + authoring affordance.
7. **Speak-as-token-instance** — `ActorOwnerRef::TokenInstance` is REJECTED at ingest (fail-closed,
   no first-party producer) — build the composer/token-context UX and lift the rejection together.
