# TODO — Deferred Work

Actionable, externally-logged deferrals. Bugs go in `OPEN_BUGS.md`, not here.

## Data layer
- TODO: Batch the four `get_or_create` config/actor `query_documents` calls in `ws/room.rs` into one `WHERE doc_type IN (...)` query to halve DB round-trips on cold room creation.
- TODO: Purge `explored_fog` rows on world/scene/user deletion. The M9c table denormalizes `world_id` for a world-scoped purge, but no deletion path consumes it yet (worlds aren't deletable; scene deletion goes through the `apply_intent` document cascade, which doesn't touch `explored_fog`). Orphaned rows are harmless (reads key on the exact never-reused `(scene_id, user_id)` UUIDs) but accumulate unboundedly over a server's lifetime. Wire a `DELETE FROM explored_fog WHERE world_id = ?` (and a per-scene purge into the scene-delete cascade) when world/scene deletion lands; index `world_id` then. (Surfaced by the M9c-1 buddy check.)
- TODO: `command::set_pointer` is set-only — an Update that conceptually removes a key writes `null` (key stays present as null) rather than removing it. `null` ≠ absent. Resolve removal semantics when the merge engine lands.

## Test infrastructure
- TODO: `axum_test::TestServer` builds request URLs via the `url` crate's `Url::set_path`, which performs WHATWG dot-segment normalization CLIENT-SIDE before the request is ever sent — a path segment that is an EXACT match to `.`/`..`/`%2e`/`%2e%2e` (and case variants) is silently collapsed/popped before it reaches the router, let alone a handler. Any HTTP-level test in this codebase that tries to smuggle a bare dot-segment through `TestServer::get`/`.post` to exercise a server-side path-traversal guard is vacuous — it proves nothing about the guard logic, since the segment never survives to hit it (confirmed: `serve_module_file_rejects_an_id_segment_that_escapes_the_modules_root` in `src/server/src/http/module_routes.rs` still passes against a deliberately-reverted, vulnerable guard). Segments that are NOT an exact dot-form match (e.g. `%2e%2e%2fsecret.txt` as one combined segment) are NOT normalized and do reach the handler intact — only bare exact-match dot segments are affected. Write future path-traversal HTTP tests either as (a) a pure unit test of the containment predicate itself, (b) a symlink/alias-based HTTP repro (module_routes.rs's `self-link`-style test), or (c) an encoded segment embedded inside a longer non-exact-match string. (Surfaced by the M13-1 Task 5 buddy-check fix-confirmation review.)

## Repo hygiene
- RESOLVED (M10h merge): `.superpowers/sdd/task-5-report.md` and `.superpowers/sdd/task-8-report.md` were tracked in git despite `.superpowers/sdd/.gitignore` declaring `*` — `task-5-report.md` first force-added at or before the M2 checkpoint (`3505a7c`), carried forward by filename reuse across M10g/M10f-1/M10f-2/M10f-3/M10h; `task-8-report.md` was a fresh instance of the same mistake introduced mid-M10h (a fix-round implementer's broad `git add` picked it up for the first time). During M10h's Task 9 review, a reviewer subagent ran `git checkout -- .superpowers/sdd/task-5-report.md` without checking diff/stashing first, discarding that session's uncommitted report content — no durable work was lost (redundant with the real code commit + the SDD progress ledger + the session transcript), but it was the second real incident this bug enabled. Both files `git rm --cached` at the M10h merge (working-tree copies preserved; not rewriting history). If a future checkpoint's `git add` re-tracks another `.superpowers/sdd/*.md` file, repeat this cleanup at that checkpoint's merge.

## Client / formula library (M13a)
- TODO: `evaluate.ts`'s `ref` case and `template.ts`'s `substituteIdentifier` both wrap a consumer resolver call in a near-identical try/catch → `resolver-error` FormulaError. `graph.ts`'s equivalent catch is entangled with the internal `NeedsDependency` trampoline signal and can't share a naive helper without leaking that control-flow type across `internal.ts`'s validation-only boundary — so only `evaluate.ts`/`template.ts` are realistically unifiable. Factor a small shared helper for those two call sites if `@shadowcat/formula` grows more consumer-callback boundaries. (Surfaced by the M13a whole-branch buddy-check fix-confirmation review.)

## Server / lint hygiene
- RESOLVED: `move_exec.rs::region_doc` (test helper) tripped `clippy::too_many_arguments` (8 positional params) under `cargo clippy --all-targets -- -D warnings`. Fixed by bundling the four rect coordinates into a single `rect: (f64, f64, f64, f64)` tuple param (destructured via `let (x0, y0, x1, y1) = rect;`), reducing the signature to 5 params; all 9 call sites updated to pass a tuple. No `#[allow(...)]` suppression used.

## Server / scene ECS (M13-0)
- TODO: `engine_as::<T>()` (`scene/mod.rs`) clones the document's `engine` JSON value and runs a full `serde_json::from_value` deserialize on every call, replacing the old per-field pointer reads (`sys_f64`/`v.pointer(...)`) across the vision/lighting/pathfinding hot paths (`region_field`, `resolve_scene`, `player_vision_polygons`, per-step `move_exec`/A* callers). A constant-factor perf regression vs. the old direct reads; profile and add a per-entity cached-decode (or a cheaper borrowed-deserialize) if it shows up under load.

## Server / pathfinding
- TODO: Buddy-check Minor (B2): the A* search window = AABB{start∪waypoints∪wall-endpoints}+8-cell margin; a legitimate route whose detour must bulge >8 cells beyond that AABB is reported Unreachable (fail-closed). Inert until a real map hits it; add a `tracing::debug!` at window-edge leg failures for future tuning if needed.
- TODO: Hex-grid pathfinding (M10e-6 is square-grid-only; the ruler's hex distance is untouched by the `alternating` rule addition).

## Server / move-execution (M1 server-authoritative / M10g regions)
- TODO: `move_exec.rs`'s `MoveOutcome.cost` accumulates only the entered cell's terrain multiplier per step (`cost += regions.terrain_multiplier(region_cell)`); `pathfinding.rs`'s router cost also multiplies by the diagonal-rule `step_cost` (`sc * mult`, where `sc` is 1.0/2.0/√2/alternating depending on `world-settings.pathfinding.diagonalRule`). The two "cost" values are not numerically comparable once diagonal movement is involved under any non-Chebyshev rule — they coincide only because Chebyshev's diagonal step cost is 1.0. This is a deliberate M10g Task 7 scoping decision (move_exec's center-cell, terrain-only accounting model), not an oversight, and nothing currently consumes or compares the two values. Resolve before any per-turn movement-budget system consumes `MoveOutcome.cost`/`MoveStream.cost`: decide whether move_exec should thread the diagonal rule + per-step parity to match the router's preview cost, or whether route-preview cost and execution cost are intentionally distinct quantities. (Surfaced by the M10g Task 7 buddy check.)
- TODO: `navmesh::los_smooth` (M10f-4) reports the smoothed continuous route's `cost` as the PRE-smoothing weighted grid cost, unchanged — it does not recompute an exact per-span cost for the straightened any-angle chords, only guarantees the reported value is a conservative (never cheaper) budget preview. Same preview-vs-execution divergence class as the `MoveOutcome.cost`/router-cost split logged above: a per-cell-exact smoothed continuous cost is deferred, not implemented. Resolve alongside the item above if a per-turn movement-budget system ever needs an exact continuous-engine cost.
- DESIGN QUESTION (needs user decision, not an actionable dev task): `Room::publish`'s movement gate only inspects `Operation::Update` — `Operation::Create` is unchecked for wall/vision, so create-capable clients can place a token at any coordinate (and a delete+recreate could relocate one this way). Confirm whether initial placement should be gated (wall/visibility-checked like a move) or is intentionally GM/tool-privileged (unrestricted placement is normal authoring behavior for a GM or a place-token tool).

## Server / move-streaming (M2 continuous vision)

- TODO: Live cross-animation concurrency for streamed move vision (`MoveStream`). M2 precomputes each move's per-recipient vision clip at *its* execute time, so two tokens moving simultaneously do NOT reveal each other mid-walk when a watcher's vision opens after the clip — it reconciles at the stop + next `vision` rebroadcast. Wanted eventually. Needs real-time per-recipient streaming (a per-move server loop recomputing each recipient's visibility of every concurrently-moving token as positions advance) instead of execute-time precompute. No correctness/secrecy impact today — only a missed transient reveal. (Design `2026-06-25-m2-streamed-continuous-vision-design.md` §8; user wants it as a follow-up.)
- TODO: GM see-as-player preview does not reflect the previewed player's actual `MoveStream` view. `clip_move_stream` (`ws/conn.rs`) keys its GM branch on `ctx.world_role == Gm` / `ctx.user_id`, ignoring the design §3.3 "see-as" effective-view-user concept used elsewhere (e.g. the `vision` subscription's `asUser`). Not a leak (a GM is authorized to see everything unclipped) but a UX-accuracy gap: a GM previewing as a specific player still gets the full unclipped trajectory instead of that player's clipped view, and a GM previewing as the mover gets no fog sweep (`mover_vision` stays `None` for any GM branch). Thread the see-as target through `clip_move_stream` when the see-as-preview feature is built out.
- TODO: A non-GM player moving in an `Unrestricted`-mode scene gets no progressive vision-sweep even though they have real LOS fog. `execute_move` (`ws/room.rs`) gates `mover_vision` computation on `matches!(restriction, MovementRestriction::Unrestricted)` (scene movement-restriction mode) rather than on mover role — so in an `Unrestricted` scene, fog stays static during the move and snaps at the end instead of sweeping. Cosmetic gap in Design Goal 2 for that specific scene configuration, not a leak (GM movers correctly get no sweep either, and this branch can't currently distinguish "GM in Unrestricted" from "player in Unrestricted"). Gate on mover role instead of restriction mode when this is revisited.

## Server / scene-vision
- TODO: Implement edge-projected, `blocksLight`-occludable environment light now that scenes have dimensions. M10e-2's `player_lit_mask` treats environment light as a flat scene-wide ambient floor (inert by default, `env.intensity` = 0.0) because the scene model was dimensionless — there was no boundary to project edge light from, so a `blocksLight`-sealed interior was not darkened by the *ambient* term (placed-light occlusion IS implemented). **Unblocked by M10f-0** (`scene.system.bounds{width,height}`, fail-closed to a 100×100 grid-unit default) — the boundary now exists — but the light-projection implementation itself is deliberately still homed to **M12** (design review 2026-07-02: M10f-0 ships the bounds primitive only, not this). (Constraint-forced deviation from the M10e spec §6/§12.5.)
- TODO: Cache the per-`(user, scene)` visibility mask for the M10e-4 movement gate. The gate recomputes `visible_cells` on demand per move (human-paced; acceptable per spec §8). If profiling shows it hot — e.g. under M10e-6 multi-waypoint preview/commit — reuse the last egress-computed `player_lit_mask` for `(user, scene)` instead of recomputing. Inert until measured.

## Client / scene-tools
- TODO: Route preview re-requests on waypoint change with a fixed debounce/seq-guard; if profiling shows chattiness on fast drags, switch to leading-edge + max-staleness (`debounce-leading-edge-not-trailing-rearm`). Inert until measured.
- TODO: M10e-6 optional cleanups (non-blocking polish, none security/correctness): `point_segment_distance` degenerate-segment threshold uses `f64::EPSILON` vs a geometry-scale ~1e-10 (inert at scene scale); `pathfinding.rs` module `use` decls sit mid-file vs top-of-file idiom; `grid.test.ts` could add an explicit `dmin=2 → 3` alternating assert; `Stage.svelte` inner `scene` var shadows the outer AppContext `scene` (rename to `activeSceneDoc`); `ws-client.test.ts` re-serializes a parsed object (fragile); the `pending` map union (`SearchPage|PathResult`) could use a `PendingResult` alias before it grows.

## Client / UI
- RESOLVED (phase1-bugs-todo-sweep): `FactionsPanel.svelte` and `ConditionsPanel.svelte`'s per-field `update()` helpers hardcoded `old: null` in their dispatched field-update intents (config-doc singleton editors), rather than reading the raw stored value like `controller.svelte.ts`'s `sendMoves` does. `Repository::apply_intent`'s field-level OCC check rejects an `Update` whose `old` doesn't match the current stored value, so once any of these fields had been written once (stored value ≠ absent/null), every SUBSEQUENT edit to that same field within a session sent a stale `old: null` and got rejected with `Conflict` — the editor silently stopped taking effect after the first successful write per field per session. Both `update()` helpers now read `sys.factions[id]?.[k] ?? null` / `sys.conditions[id]?.[k] ?? null` from the registry's current stored system body before dispatching, mirroring `GameSettingsPanel.svelte`'s already-fixed `set()` pattern (M11d-2 Task 8). Regression tests added covering a second same-session edit to the same field in both panels.
- TODO: Game-settings scene picker shows raw scene UUIDs; display a human-readable scene name/label once scene docs carry one.
- TODO: Make the M10b `module-factions` GM seed safe against a multi-GM first-entry race. The seed `$effect` creates the `faction-registry` only when absent + a local `seeded` guard, which is correct for the single-GM norm; two GMs entering a brand-new world simultaneously could each create a registry before the other's create broadcasts back, forking two registries. Resolve with a deterministic registry id (and dedupe-on-conflict) or server-side seeding when multi-GM concurrency matters. (Noted in the M10b plan; harmless for single-GM.)
- TODO: Extend `reconcileTopology` beyond presence-by-`module_id` to flag version and `provides`/`requires` mismatches for modules present on both sides (a stale local build providing a contract the world no longer declares currently reconciles silently). Land with module management / hard topology enforcement.
- TODO: Resolve multi-provider conflict policy for `singleton` surface contracts in the UI contribution architecture — when two modules provide the same `singleton` contract (e.g. both claim "the sidebar"), decide the winner (load order, explicit priority, or user selection) instead of the current deterministic loud-fail. Design once a real second provider exists to validate the semantics; the contract model already carries the `singleton`/`multi` cardinality marker the policy slots into.
- TODO: Add capability version negotiation to contract-based module dependencies (`requires`) — match a required contract against a provider by version range, not presence alone. Deferred until multiple providers of a contract exist at differing versions.

## Client / actors-tokens
- TODO: Evaluate whether `buildTokenFromActor` should continue seeding `w/h = cellSize` into the token's `system` fields. These values are not used on the actor-backed render path (size resolves through `EffectiveActor.size × grid-cell` in `resolveTokenBox`), but they DO serve as the dangling-link fallback (`box.w/h` when the actor is missing). Decide — when M10a is next touched — whether to keep them solely as that fallback or compute the fallback differently (e.g. derive it lazily from the token's last-known actor size). (Surfaced by the M10d final review.)
- TODO: `ActorsPanel.svelte` (531 lines as of M12d, was 466 at M10h) has grown into a cumulative god-component — actor list/selection, GM per-row inline editors, the create form, the visual-kind editor (image/faces/animated + `buildVisual`/`faceRowComplete` validation), the per-token face-swap palette, and now (M12d) live FTS search + open-sheet all live in one file. Not yet a hard defect (snippet-extracted, still navigable), but M10j (fx/emotes authoring) is slated to add more to the same file. Extract the visual-kind editor into its own `VisualKindEditor.svelte` (owning `AnimSourceState`/`FaceRowState`/`buildVisual`) and consider the same for the face-swap palette before M10j lands. (Surfaced by the M10h whole-branch review.)
- TODO: No test covers a linked token whose per-token `overrides.visual` is itself a `faces` union combined with an active manual face-swap (`token.system.face`) — the one place the projected-`EffectiveActor.visual` override precedence meets face selection. Verified correct by code trace (both `resolveTokenVisual` and the face-swap palette's `selectedFaceNames` read the same override-projected `resolveTokenActor` output), but nothing pins the behavior against a future change to either seam. Add a `resolveTokenVisual`/palette test with a token-level `overrides.visual:{kind:"faces",...}` + `system.face`. (Surfaced by the M10h whole-branch review.)
- TODO: `ActorsPanel.svelte`'s `faceRowComplete` (per-face-row completeness check) and `buildVisual`'s inline top-level-animated-kind completeness check re-express the same frames-nonempty/sheet-asset-present logic in two places (one per-row, one top-level) — not a bug, but a small DRY opportunity. Fold both into a shared `animSourceComplete(anim: AnimSourceState)` helper next time this file is touched. (Surfaced by the M10h whole-branch review.)

## Client / render
- TODO: Lerp token rotation along the shortest signed delta (`((b-a+540)%360)-180`) with a wrap-aware ε-settle, when M8d-2 adds rotation control. M8d-1's `TokenAnimator` lerps rotation as a raw scalar (350°→10° tweens the long way); cannot manifest until rotation is authorable. (Surfaced by the M8d-1 buddy check.)
- RESOLVED (M12d): active-scene selection landed — `resolveViewedScene` (client-side resolver:
  a resolvable `gmViewedScene` GM-local roam → a resolvable `world-settings.activeScene` →
  the first scene) replaces every bare `query("scene")[0]` call site across `WorldSession`,
  the render engine (`toVisibility`/`toLighting`/the reconciler/all five doc views), Stage's
  grid driver, and scene-tools. `sceneScopedDocs` scene-filters every doc view by `parent_id`.
- TODO: Add browser e2e asserting the scene **background** renders (Scene `system.background` → sprite). The scene browser (M12d) now provides a UI to set `scene.system.background` on create/configure. Add the assertion when this file is next touched.
- TODO: Give a wall-less scene full intrascene vision instead of the degenerate viewpoint-bound box. M9b's `player_vision_polygons` bounds a wall-less scene to a viewpoint±margin box (leak-safe under-reveal, but a player in an open scene sees only a small square). A payload-level `mode:"all"` shortcut is NOT viable — it clears fog globally and would reveal a *different* walled scene (cross-scene leak, and M12d's `viewedSceneId`/`pendingDerived` fix now specifically guards against this class). The fix needs a per-scene vision mode (or a scene-extent so the wall-less polygon can cover the whole scene). (Surfaced by the M9b buddy check.)
- TODO: Cache/reuse the two cross-fade `RenderTexture`s across ticks in `pixi-backend.ts`'s `captureFog` (recreate only on resize or fog-input change) instead of a full screen-sized recapture on every `setVisibilityBlend` call — a sweep ticks ~60/s, so this is two full-screen renders/tick for the duration of every in-flight move animation. No correctness impact; a real cost only if profiling shows it hot. (Surfaced by the M2 Task 7 review.)

## Server / dice (M11a)
- RESOLVED (M11a Task 9): `DieKind::Numeric { min, max }` with `min > max` (or a degenerate non-positive span) was only guarded by a `debug_assert!` inside `rng::roll_uniform` — a release build reaching it silently returned a value unrelated to the intended range instead of erroring. The notation parser (`dice::notation::parser::factor`) now rejects a non-positive sides count at parse time (`ParseError::InvalidDieSides`), before a `DieKind::Numeric` is ever constructed from untrusted notation input. `dice::spec`/`dice::rng` themselves remain unvalidated by design (pure library, M11a scope) — the `debug_assert!` in `rng::roll_uniform` still stands as the last-resort guard for any caller that bypasses the parser. Re-check when M11d wires a wire-facing `RollSpec` construction path — that boundary needs the same `sides < 1` validation independent of the notation parser, since it can build a `RollSpec` directly.
- RESOLVED (M11d-2 Task 1): every `Option<>` field on types reachable from `RollSpec` now
  carries `#[serde(default)]` (incl. `Tier.label`/`tier_value`, `TotalConfig.difficulty`,
  `SuccessConfig.required_successes`/`crit_success`/`crit_fail`, `Face.value`), pinned by a
  partial-JSON deserialization test — closing the original gap: `RollSpec.success`/`required_successes` lacked `#[serde(default)]` — today's only round-trip always populates every field, so it passes, but a future hand-built or partial JSON (an M11d wire payload, or an older persisted `RollSpec` after a schema addition) that omits these keys will fail to deserialize with "missing field" instead of defaulting to `None`. Add `#[serde(default)]` before M11d exposes these types past the pure-library boundary. (Surfaced by the M11a Task 2 code review.)
- RESOLVED (M11d-2 Tasks 1+3 + buddy-check): the transport boundary now caps
  `MAX_ROLL_DICE=100`/`MAX_ROLL_RECORDS=1000` (`chat/rolls.rs`), `RawRoll::push` guards
  `next_id` via `checked_add` (debug-assert + saturate), and `eval::sum::fold` saturates BOTH
  the per-group sum AND — a buddy-check catch beyond this entry's original scope — every
  `Expr::Bin` Add/Sub/Mul arm (unbounded `Const` terms/`*` chains were deterministically
  overflowable with zero dice). Original entry: two overflow sites shared the root cause — no per-roll dice-count ceiling — and the same resolution path (whatever cap Task 9 / M11d establishes): (1) `RawRoll::push` (`dice/outcome.rs`) increments `next_id: DieId (u32)` with no overflow guard — release profile has no `overflow-checks`, so wraparound is silent, not a panic, and a sufficiently long-lived `RawRoll` could reissue id `0` and collide with a retained entry, violating the doc comment's own "ids never collide" invariant; (2) `eval::sum::fold` (`dice/eval/sum.rs`) sums a `Dice` group's kept records into an `i64` with no cap on `DiceGroup.count`, so an uncapped count is an unbounded-work / overflow surface at evaluation time as well as at roll time. Neither is reachable within one roll today (`DiceGroup::count` bound doesn't exist yet, see the min>max validation TODO above), but both become real once Task 9 / a transport boundary lifts that ceiling. Add `checked_add`/saturating guards at both sites alongside whatever per-roll dice-count cap Task 9 (or M11d) establishes — mirrors the `roll_uniform` full-span guard already added in `rng.rs` (commit `49c63ed`). (Surfaced by the M11a Task 3 code review; broadened by the M11a Task 6 buddy-check.)
- Bound `SuccessConfig.expertise` (u32) at the M11d untrusted-transport boundary,
  alongside the per-roll dice-count cap: `eval::expertise::allocate` is `O(N·E²)`, so
  an unbounded `E` from an untrusted `RollSpec` is a DoS vector via `die_values`'s
  `(0..=e).map(...).collect()`, which allocates `e+1` entries per kept die with no cap.
  Additionally, `adjust`'s `let k = k as i32` cast could silently wrap to a negative
  value if `k`/`e` ever exceeded `i32::MAX` (still representable in the `u32` field),
  moving a face the wrong direction instead of erroring — both facets are resolved
  together by whatever sane bound gets enforced at that boundary (design intent: `E` is
  single digits in every real system). Pure-library M11b-2 stays cap-agnostic by design.
  (Surfaced by the M11b-2 Task 2/Task 3 code reviews.)
- RESOLVED (M11d-2 Task 1): `ParseError` (and `Token`) now implement player-presentable
  `Display`, consumed by the chat System error notices; pinned by a no-debug-artifacts test
  over every variant. Original entry: messages were raw Rust `Debug`-formatted `Token` values (e.g. `Some(BangP)`), not human-readable text. Fine while errors stay server-internal, but before M11c/d surfaces a parse failure directly to a player in a chat UI, add a `Display` impl for `Token` (and route the error arms through it) so the message reads as dice notation, not a Rust enum dump. (Surfaced by the M11a Task 9 review fix — `ParseError::DuplicateSuccessRule` was added with a clean fixed message; the pre-existing variants were not.)
- Dice notation: extended math functions (floor/ceil/round/abs/min/max) are not yet
  parsed. M11a covers dice + arithmetic +-*/() + keep/drop/explode/reroll + cs/cf. Add
  as the notation grammar grows with system demand.
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
  wire exposure (recalc-from-chat is itself deferred, see the M11d-2 deferrals below).
  (Surfaced by the M11b-3 Task 5 code review.)
- TODO: construction-time uniqueness for the singleton config doc_types
  (`CHAT_SETTINGS_DOC_TYPE`, `DICE_SETTINGS_DOC_TYPE`, and the faction/condition/world-settings
  registries) — nothing at the `apply_intent` Create chokepoint stops a second doc of a
  singleton doc_type from being created in a world (the GM editors' seed guards are
  client-side-only, racy across two GMs/connections). **The "explicit tie-break ordering" half
  of the original ask is now DONE + tested (M11d-3):** `resolve_content_policy`/
  `resolve_dice_context` resolve DETERMINISTICALLY by lowest UUID (`query_documents` `ORDER BY
  id`), documented at both resolvers and pinned by
  `duplicate_settings_docs_resolve_deterministically_by_lowest_id` — so a duplicate can never
  cause a nondeterministic policy, and the fail-closed direction bounds a stray doc to
  widening-only. **Still deferred:** the STRONGER construction-time guard (a singleton
  doc-type create-gate). Reason for deferral, re-evaluated when M11d-3's GM chat-settings write
  path landed: it is its own change — a singleton-doctype registry consulted on every Create
  across factions/conditions/world-settings/dice-settings/chat-settings uniformly — and the
  residual risk is low (deterministic + fail-closed resolution already makes a duplicate inert
  beyond needing GM-authored content). Build it when a singleton config gains a create path that
  isn't already idempotent-seed-guarded, or as a dedicated hardening sweep.
  (Surfaced M11c-3; tie-break resolved + re-logged M11d-3 T6 review.)

## Client / chat display (M11d-1)
- Message-list virtualization: the panel renders only the most recent 200 messages per view
  (older history exists in the store via resync but is unrendered), and the reactive pipeline
  re-parses/re-sorts the full unbounded message history on ANY document mutation (whole-store
  subscribe, same idiom as the small bounded registries). Fine at current scale; virtualize +
  narrow the subscription when long-session history makes it observable.
- Unread badges / notification pips on the chat tab (and tab popout windows, if ever wanted) —
  Foundry-parity polish, deliberately out of M11d-1 scope.
- TODO: A generic in-body doc-link chat segment (`Segment::DocLink`) — actor-name → sheet
  navigation shipped in M12c, but a free-form doc-link segment has no server producer or
  client authoring path yet; add the segment kind + an authoring affordance when a use case
  lands.
- Speaking-as-actor composer UX (`actor_owner` picker) — lands with M11d-2 roll attribution;
  the wire field, storage, and card rendering already support it.
- Send/edit/delete failure surfacing: the chat frames carry no correlation id, so server
  rejections (e.g. flood limit) are invisible to the sender beyond client pre-validation —
  needs a protocol-level reason channel (pre-existing M11c deferral, re-affirmed).
- RESOLVED (M12a): the TabbedSurface roving-tabindex and sidebar-collapse-persistence entries
  are obsolete — the tabbed sidebar and `TabbedSurface` were deleted wholesale; panels now dock/
  float/minimize via `module-panels` (dockview supplies the tab keyboard model; layout persists
  per-world in `ui_state.worlds[w].panelLayout`).
- Server shortcodes: pre-parse replacement also fires inside markdown code spans; refine to
  skip code spans if it ever matters in practice.

## Chat / dice wire (M11d-2)
- Recalculate-from-chat (reroll failures, replace dice on a posted roll): the RollEmbed
  deliberately stores only `{formula, outcome}` — no `spec`/`raws` — so stored rolls cannot be
  recalculated. Revisit in Phase 2 with a persistence decision (and close the ReplaceDie
  Faces-natural gap above at the same boundary).
- Rich roll tooltips: the inline chip uses the native `title` attribute; a popover with the
  full per-die table when a design pass wants it.
- Speak-as-token-instance attribution: `ActorOwnerRef::TokenInstance` is REJECTED at ingest
  (fail-closed, no first-party producer) — build the composer/token-context UX and lift the
  rejection together.
- Attribution scope pinning: the ingest ownership check (`ActorNotSpeakable`) verifies
  existence/doc_type/owner but not that the actor doc's world scope matches the sending world
  — inert today (foreign refs fail closed to no attribution on every reader's client); pin the
  scope when convenient.
- Notation syntax for crit-event configs (`CritSuccess`/`CritFail` structs remain
  struct-authored only) and for tier ladders — grows with system demand.
- Per-channel / per-message dice-settings overrides (world-level only today).
- `handle_send_message`/`handle_edit_message` now take 3 extra positional args
  (preview_client/preview_cache/preview_rate) across ~40 call sites: bundle the link-preview
  deps into a `LinkPreviewDeps`-style struct to shrink both signatures and reduce call-site
  arg-order risk.

## Client / UI chrome (M12b whole-branch review deferrals)
- TODO: `list_members` sorts with SQLite binary collation (`ORDER BY u.username`) — uppercase
  sorts before lowercase in the Presence roster (`Bob` before `alice`). Switch to
  `COLLATE NOCASE` if case-insensitive display order is wanted; the covering test uses
  all-lowercase names so it won't catch the change either way.
- TODO: `LauncherMenu.svelte` and `PanelMenu.svelte` duplicate the WAI-ARIA menu keyboard/focus
  logic (arrows/Home/End/Escape/Tab + wrap-around focusItem). The seam boundary blocks direct
  reuse across the modules; extract a shared menu primitive into ui-kit before a third menu
  triplicates it.
- TODO: `ToolRail` `.controls select/input` are 32px min-height — above the 24px a11y floor but
  below the ~44px coarse-pointer aim, and now touch-reachable on phones via the compact strip.
  Bump their coarse-pointer sizing in a density pass (`@media (pointer: coarse)` or a wrapper).
- TODO: `panels.spec.ts` locates tool buttons via the styling class (`.tool-rail .tool`) instead
  of the existing `data-testid="tool-{id}"` — swap to the testid form when next touched.
- TODO: `LauncherMenu` has no handling/test for `metaMap` mutating while the menu is open (a
  panel uninstall would drop focus out of the menu's closed loop). Modules only install/uninstall
  at world entry today; add a focus-recovery path (or at least a pinning test) when live module
  management lands.

## Client / ui-kit forms (M12c Task 5 buddy check)
- TODO: no ui-kit component has a coarse-pointer touch-sizing rule for text/number/checkbox
  `<input>` elements (only buttons get the `@media (pointer: coarse)` 44px bump — see
  `SystemTreeEditor.svelte` and the pre-existing gap in `GameSettingsPanel.svelte`). This is a
  systemic ui-kit baseline gap, not a per-component one; fix with a shared input-height token/rule
  rather than duplicating a media query into every form component.

## Client / panels (M12e Task 5 buddy check)
- TODO: an already-open popout window has no `onWillDrop` subscription wired
  (`#groupWillDropSubs` is populated only inside `apply()`'s zone loop) — dockview-core's own
  popout design supports drag-and-drop of a further panel into the popout's nested gridview, so a
  same-origin cross-window drag into an open popout would bypass the reducer's veto/classify
  pipeline entirely (`applyOp` invariant "all layout mutations flow through applyOp" would not
  hold for that gesture), and `#poppedOutGroupPanels`'s single-panel-array assumption wouldn't be
  updated to include the dragged-in panel — silently unaccounted for on window close. Out of the
  M12e Task 5 brief's scope (menu pop-out + its own close translation only); wire it if/when
  multi-panel popout groups become a supported gesture.

## Client / panels (M12a Task 6 — DockviewEngine)
- RESOLVED (M12a Task 6 buddy-check fix wave): live resize (`resizeZone`/`resizeGroup`)
  translation is now wired — `group.api.onDidDimensionsChange` (`DockviewGroupPanelApi`, inherited
  from `PanelApi` via `GridviewPanelApi`) fires per managed group; `DockviewEngine` subscribes one
  listener per group at creation (disposed on removal/`destroy()`) and emits `resizeZone`/
  `resizeGroup` ops, guarded by `#applying` and a sub-pixel-delta dedupe. The original entry's
  premise ("no event surface found") was false — a buddy-check reviewer traced the event through
  `dockview-core`'s own source.
- TODO: Whole-GROUP drag transfers (a titlebar drag of an entire tab group, `PanelTransfer`'s
  `panelId === null`) are vetoed outright in v1 rather than translated into per-tab dock ops.
  `DockviewEngine#handleWillDrop` fails closed on any payload it cannot classify into a
  `DropSite` — container-edge transfers via the component-level `api.onWillDrop` wire, and
  group-onto-group transfers via a per-group `group.model.onWillDrop` subscription
  (`#groupWillDropSubs`, added because the component never forwards a group's `onWillDrop`
  on its own — see `DockviewEngine`'s doc comments for the citation). Translate whole-group
  transfers into per-tab dock ops to re-enable the group-drag gesture.
  (Surfaced by the Task 6 buddy-check: an untranslated group transfer previously fell through
  `#handleWillDrop` WITHOUT vetoing, letting a group land above the stage on a top-edge drop.)
- TODO: Floating-panel position/size sync in `DockviewEngine.apply()` for an ALREADY-floating
  panel — creation is handled (`api.addPanel({..., floating: {...}})`), but a live re-drag or
  resize of an existing floating window is not mirrored back into the tree, so the persisted
  `Rect` can drift from what the user sees.
- RESOLVED (M12a Task 6 fix round 3): `DockviewEngine`'s gesture contract is now uniformly
  "classify → veto or redispatch; dockview never self-mutates from drops" — `#handleWillDrop`
  `preventDefault()`s an ALLOWED classification too and emits the classified `LayoutOp` itself,
  so dockview's own internal move machinery (`_onMove`) is never reached for a completed
  same-instance drag, and `onDidRemovePanel` can no longer see an internal-move removal outside
  `apply()`'s `#applying` window (closing the spurious-close-op defect a real browser drag used
  to hit). `onDidDrop`/`#handleDidDrop` are removed: `onDidDrop` only fires when a drop's
  `PanelTransfer.viewId` doesn't match this instance's own `accessor.id`, which no drag reaching
  this class (one `DockviewApi` per `PanelHost`, no popout/multi-instance support) can ever
  satisfy.
- TODO: `DockviewEngine#toDropSite`'s one remaining fallback branch (a drop's target group
  falling outside the engine's own zone bookkeeping) is a best-effort approximation (falls back
  to an edge-zone dock), not exhaustively verified against every dockview drag path. The
  intercept-and-redispatch translation mechanism itself (preventDefault + emit + reconcile
  through `apply()`) IS exercised directly by unit tests now, not approximated. Real drag-and-drop
  still cannot be simulated under jsdom (no native `DragEvent`/`PointerEvent` gesture) — the
  residual manual-QA item narrows to drop-position classification fidelity for real pointer
  geometry (edge vs center vs tab-strip index resolution against an actual drag gesture) before
  shipping.
- TODO: `DockviewEngine.apply()`'s group-identity scheme keys a dockview group id off its
  first tab's id (`groupIdFor`), not a stable positional/structural id — reordering a group's
  first tab, or emptying then refilling a group, causes that group to be torn down and
  recreated rather than patched in place (harmless: content survives via the persistent slot
  element; only the dockview chrome/tab-order animation resets). A finer, content-independent
  diff is future work if this churn becomes visible in practice.
- RESOLVED (M12a Task 9): docked-panel-to-floating was itself a latent gap in `apply()`'s
  floating loop — it only handled floating-panel CREATION (`!api.getPanel(f.id)`), never a
  panel already docked that the tree newly lists under `expanded.floating`, leaving it
  stranded in its old (tree-orphaned) group. Fixed with the same remove+re-add-under-groupId
  pattern the zone loop already used for cross-group moves, keyed on
  `existing.group.api.location.type !== "floating"` (dockview's own public location
  discriminant). Surfaced by wiring the `PanelMenu`'s "Float" command — the first UI affordance
  to ever trigger a "float" op against the production engine.
- TODO: `PanelMenu`'s "Float" command is the ONLY current trigger for a `float` `LayoutOp`, and
  floating a panel via its OWN tab menu necessarily destroys that same tab (and its menu
  button) as part of the docked→floating transition. `DockviewEngine`'s focus-return-to-invoker
  mechanism (`#floatInvokers`/`#teardownFloatingA11y`) is correct and general: the T9 review
  (Finding 1) fixed a bug where the transient remove/re-add inside `apply()`'s floating loop
  discarded the `#floatInvokers` entry BEFORE the new floating dialog was even created —
  `#floatTransitionIds` now brackets that transient removal so the entry survives, and focus
  correctly returns to any invoker still attached to the document when the panel is later
  closed (`dockview.test.ts`'s Finding 1 test proves this by reattaching a captured invoker).
  For THIS milestone's only reachable trigger, the invoker is always the tab's own menu button,
  which dockview detaches from the DOM synchronously as part of the SAME transition (before
  `onDidRemovePanel` even fires) — so focus-return still degrades to a safe no-op in practice
  for the self-referential case specifically, by construction, not because the mechanism is
  broken. A future non-self-referential float trigger (a command palette, a chip-strip "float"
  action) gets full, working focus-return with no further change needed here.

## Client / panels (M12a whole-branch review deferrals)
- TODO: `FakeEngine`'s plain tab strip has no `PanelMenu` (dock/float/minimize/pop-out commands)
  — that menu is mounted by `DockviewEngine.createTabComponent` only. A panel docked under
  `FakeEngine` (bespoke-fallback engine; production never reaches it) can only reach a
  minimized/closed state going forward via `PanelsChipsView.restore`, not back out of a zone
  through any UI affordance. Orthogonal to the width-containment fix (`docs/CLOSED_BUGS.md`):
  giving `FakeEngine` its own menu is future work if a bespoke-fallback caller needs it.
- TODO: Narrow `PanelHost.svelte`'s `PanelsBridgeLike` inline cast — either a runtime
  `typeof bridge.bind` guard or a narrower `AppContext.panels` type; today it rests on the
  composition-root convention (`Table.svelte` is the sole binding site).
- TODO: `DockChips.svelte` falls back to the raw untranslated panel id when `metaMap` lacks an
  entry (same class: `PanelHost.describeOp`'s aria-live fallback) — unreachable today (`prune`
  keeps layout ids ⊆ registered); give it an i18n fallback if a reachable path ever appears.
- TODO: `sizeClass.svelte.ts` teardown path (createSubscriber removeEventListener) has no test —
  matches the pre-existing i18n teardown-test gap; cover both together.
- TODO: `controller.test.ts` boot-race test asserts `compact.order` membership (`toContain`)
  but not full order equality (per-panel `locate()` placements ARE exactly pinned); tighten to
  a full-sequence assert when next touched.

## Chat / link previews (M11d-3)
- Preview images: v1 stores title+description only. An image URL rendered as `<img src>` would
  make the client fetch it and leak the viewer's IP — the invariant-preserving path is
  server-fetch-and-cache-as-asset (scheme/size/content-type-validated), its own pipeline. Build
  when link-preview images are wanted.
- Async post-publish enrichment: v1 fetches synchronously before publish (a first-seen link adds
  up to the 5s deadline of send latency; cached links are instant). A UX upgrade would post the
  message immediately and enrich moments later via a spawned task + a server-authored Update
  (needs a WriteOrigin path + message-deleted-mid-fetch handling).
- Persistent/shared preview cache: in-memory per process today (a multi-process deploy re-fetches
  per process — fine, re-fetchable). Add a shared cache only if fetch volume ever warrants it.
- oEmbed / provider-specific rich embeds; `<meta http-equiv=refresh>` following — out of scope.
- Bundle the link-preview deps (`preview_client`/`cache`/`preview_rate`) into a `LinkPreviewDeps`
  struct to shrink `handle_send_message`/`handle_edit_message` signatures (~40 call sites now under
  `#[allow(clippy::too_many_arguments)]`) and reduce call-site arg-order risk.

## Module toolchain (M13-1 Tasks 8+10)
- TODO: `welcome_capability_requirements` (`ws/conn.rs`) emits duplicate `(path_prefix, caps)`
  entries when a GM-authored requirement and an enabled module (or two modules) declare the same
  `path_prefix` — inert today (`declared_caps_for_path` checks inclusion, not count) but inflates
  the Welcome payload. Add dedup once a dedup-key strategy for `CapabilityRequirement` (not
  currently `Hash`/`Ord`) is chosen. (Surfaced by the M13-1 Tasks 8+10 buddy-check.)
- TODO: `scan_installed_modules` does blocking `std::fs` I/O with no `spawn_blocking`; Task 10
  moved it onto the per-WS-connect Welcome path (was admin-HTTP-only), so it now blocks a tokio
  worker on every session join. Latent scaling concern at large module counts / concurrent
  reconnects — wrap in `spawn_blocking` or introduce the deferred module-discovery cache.
  (Surfaced by the M13-1 Tasks 8+10 buddy-check.)
- Design note (module requirements are advisory): module-declared manifest `requirements` are
  published to clients as advisory UX only and are NOT server-enforced at `apply_intent` (server
  authority stays with the GM's `world_cap_requirements`, per ARCHITECTURE invariant 6). A future
  explicit "GM adopts a module's requirements into the world policy" mechanism could make them
  enforced if desired. (Surfaced by the M13-1 Tasks 8+10 buddy-check.)
- TODO: No build-time guard exists against a first-party or module change introducing a new
  `svelte/*` subpath (e.g. `svelte/store`, `svelte/transition`) not enumerated in
  `vite.config.ts`'s `RUNTIME_ENTRIES` — an un-enumerated subpath would resolve to the app's own
  bundled copy instead of the shared runtime chunk, degrading the single-instance sharing
  invariant silently rather than failing the build. Add a build-time check (scan built output or
  source for `from "svelte/..."` specifiers absent from `RUNTIME_ENTRIES` and fail the build).
  (Surfaced by the M13-1 Task 14 buddy-check.)
- TODO: The build-time import map (`vite.config.ts`) has only exact-match package-root entries —
  an external module importing a package SUBPATH (e.g. `@shadowcat/core/something`) is an
  unresolvable bare specifier under the current map. This is a clean browser-level failure caught
  by the per-module load containment (a documented completeness caveat, not a single-instance
  violation), but the module-authoring guide (Task 17) should call it out explicitly. (Surfaced by
  the M13-1 Task 14 buddy-check.)
- TODO: `ModuleRegistry.activate()` (`modules.ts`) now contains a throwing module's `register()`
  per-module (logs + skips, doesn't abort the batch), but its catch does NOT roll back partial
  side effects the module already made before throwing — `ctx.hooks.on`, `ctx.services.provide`,
  `ctx.use`, and especially `ctx.contributions.contribute` (contributions render via `<Surface>`
  regardless of a module's `active` flag). A module whose async `register()` contributes UI then
  throws leaves a live, rendered contribution behind while the registry reports it inactive.
  Unreachable today (all first-party modules `register()` synchronously with no interleaved
  awaits), but reachable once external modules with async `register()` bodies exist (M13b+). Decide
  the `register()` lifecycle contract when Nightfox first exercises it: either wrap the `register()`
  call with a `removeModule(id)` cleanup sweep on catch, or document `register()` as required to be
  effect-free until its final synchronous step. (Surfaced by the M13-1 Task 15 review
  fix-confirmation.)

## Server / backups (M12.5)
- TODO: Per-world granular export/import (sharing a single world between server instances without a whole-database snapshot) — M12.5 ships whole-server snapshot/restore only. Real complexity (world-scoped row subset while preserving referential integrity across cross-table FKs, shared asset references, admin/global tables) deferred as a distinct future feature, not required for the dogfood-alpha gate.
- TODO: The backup mechanism's assets-copy step is not transactionally coupled to the `VACUUM INTO` DB snapshot. An asset REPLACE (not create) in flight during backup commits its DB row before renaming its temp file into place (`http/assets.rs` — `replace`), so a backup racing an in-flight replace can capture updated asset metadata with the pre-replace file bytes for a few milliseconds' window. Inherent property of any online (no-downtime) backup of a live mutable system; add a brief write-quiesce mode during backup if stronger consistency is ever needed in practice.
- TODO: `restore_backup`'s destination writes (`tokio::fs::copy` for `world.db`, `remove_dir_all` + `copy_dir_recursive` for the assets directory) are not atomic swaps. A failure partway through (disk full, permission error, process kill) can leave the destination db truncated or the assets directory in a state worse than either the pre- or post-restore content. Accepted tradeoff for the "basic" gate-precondition feature; a stronger-consistency restore (write to a temp path, atomic rename into place) is a candidate follow-up if this is ever exercised in an environment where a mid-restore crash is a real operational risk.
