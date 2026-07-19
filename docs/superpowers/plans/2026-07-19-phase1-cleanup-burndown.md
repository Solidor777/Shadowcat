# Phase-1 Cleanup Burndown Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Clear the Phase-1 TODO backlog down to only genuinely-blocked items, landing ~40 fixes/refactors/tests/features on `main`'s codebase (branch `phase1-cleanup-burndown`), per `docs/superpowers/specs/2026-07-19-phase1-cleanup-burndown-design.md`.

**Architecture:** No new subsystems. Each task is a targeted fix, refactor, or small feature inside an existing file/module, following that area's established patterns exactly (OCC `old` pre-image reads, `spawn_blocking` conventions, request_id correlation, `@media (pointer: coarse)` sizing, etc.) as identified by pre-plan codebase research.

**Tech Stack:** Rust (axum, sqlx/SQLite, tokio) server; TypeScript/Svelte 5 (runes) + SCSS client; ts-rs/Zod wire-type mirroring; cargo test/clippy + vitest/playwright + pnpm typecheck/lint gates.

## Global Constraints

- TDD per task (project CLAUDE.md): write the failing test first, verify it fails, implement, verify it passes.
- Full cross-platform gate before any task is considered done: `cargo test --all-targets` + `cargo clippy --all-targets -- -D warnings` (server tasks); `pnpm -r test` + typecheck + lint (client tasks). Typecheck is a separate gate from vitest (esbuild strips types — vitest alone won't catch a type error).
- No debug artifacts (`dbg!`, bare `println!`, `console.log`, `debugger;`) in committed code.
- Tasks tagged `[sec]` require a mandatory two-reviewer security buddy-check before merge (see Buddy-check directives below): **Tasks 6, 7, 9, 10, 39** (B1, B2, C1, C2, I4) plus **Task 4** (A4, Unrestricted-mode vision-sweep gate) and **Task 5's cache** (A5, vision-mask cache) — full list below.
- Never fork the vision/secrecy mask decision (project invariant, restated in `shadowcat-codebase-scene-rendering`): any new vision/lighting code path must reuse the existing mask/occlusion primitives, never compute a second independent one.
- OCC field-level `Update` intents must read the current stored value for `old` (never hardcode `null`) — the exact class of bug the M11d-2/phase1-bugs-todo-sweep fixes closed. Every new dispatched `Update` in this plan follows that pattern.
- Commit after each task's gate passes. One task = one or more commits, never a mixed commit spanning two tasks.
- This plan intentionally excludes the 4 dogfood-polish features I2 (rich tooltip data plumbing already exists) is included as **Task 38**; I1 is **already implemented** — Task 37 is verify-and-close, not new code.

## Model/Effort directives

**Plan-writing tier:** user chose mainline continuation (this session, Sonnet 5 default effort) over dispatching `sdd-plan-writer-opus`/`sdd-plan-writer-sonnet`.

**Dispatch tier (for execution):** per project CLAUDE.md, `shadowcat-coder` (sonnet, effort: medium) is the default per-task implementer; `shadowcat-code-reviewer` + `shadowcat-spec-reviewer` (sonnet, effort: high) are the two-reviewer review pair at every checkpoint. Each has an `-opus` twin (opus, effort: high) — escalate to the twin when the base agent reports BLOCKED, or a reviewer's findings read shallow/uncertain, before escalating to the human. The user has stated they will be the SDD dispatcher for this plan (mainline session, model-switched at dispatch time) rather than delegating to `sdd-dispatcher`.

**Escalate to `-opus` twins pre-emptively (not just on BLOCKED)** for: Task 6 (B1, set_pointer removal — interacts with the M13e merge engine's absent-vs-null convention), Task 9 (C1, edge-projected environment light — flagged by research as a genuine open design fork with no existing pattern to mirror), Task 17 (E1a, ActorsPanel visual-kind editor extraction — large refactor), Task 39 (I4, chat failure-surfacing protocol addition).

## Buddy-check directives

High-risk signals present (per `buddy-checking` skill's Offered-mode criteria): multiple tasks touch the vision/fog secrecy gate, OCC/merge semantics, and a new wire protocol surface. Per the spec's Testing & verification section, buddy-check is **pre-authorized mandatory** (not merely offered) for:

- **Task 4** (A4 — Unrestricted-mode mover-vision gate change)
- **Task 5** (A5 — vision-mask cache: must fail toward recompute, never a wider mask)
- **Task 6** (B1 — set_pointer true removal, OCC + merge-engine interaction)
- **Task 7** (B2 — singleton create-gate)
- **Task 9** (C1 — edge-projected environment light)
- **Task 10** (C2 — wall-less-scene full vision)
- **Task 39** (I4 — chat failure-surfacing: reason channel must not leak authorization detail)

All other tasks get the standard single-reviewer-pair gate; the dispatcher may additionally **offer** (not require) a buddy-check on any task whose diff reads unusually large or uncertain, per the `buddy-checking` skill's Offered mode — the human decides whether to accept the offer.

---

## Task 1: Batch cold-room config/actor queries (A1)

**Files:**
- Modify: `src/server/src/ws/room.rs:737-755` (the `// TODO: batch these four...` block in `RoomRegistry::get_or_create`)
- Modify: `src/server/src/data/repository.rs` (add a trait method)
- Modify: `src/server/src/data/sqlite.rs` (implement it)
- Test: `src/server/src/ws/room.rs` (extend `get_or_create_hydrates_config_and_actors_from_db`, line ~1282)

**Interfaces:**
- Produces: `Repository::query_documents_by_types(&self, world_id: Uuid, doc_types: &[&str]) -> Result<Vec<Document>, DataError>` — a new trait method returning all matching documents in one query, grouped by `doc_type` at the call site.
- Consumes: existing `Repository::query_documents(&self, world_id: Uuid, doc_type: &str) -> Result<Vec<Document>, DataError>` signature (unchanged, still used elsewhere).

Read the existing test's comment at `room.rs:1376-1385` before writing code — it documents the invariant the batched query must preserve: each doc_type must still resolve independently via `.into_iter().next()` (i.e. don't let one doc_type's presence/absence affect another's).

- [ ] **Step 1: Write the failing test**

Extend the existing `get_or_create_hydrates_config_and_actors_from_db` test in `src/server/src/ws/room.rs` with an assertion that only ONE additional SQL call fires for the four doc_types combined. Since query counting isn't directly instrumented, assert behavior instead — add a case where only `actor` and `world-settings` exist (no `light-gradation`/`vision-modes`) and confirm both resolve correctly while the other two stay `None`/empty, proving the batched query doesn't require all four doc_types to be present:

```rust
#[tokio::test]
async fn get_or_create_batched_query_handles_partial_doc_type_presence() {
    let repo = test_repo().await;
    let world_id = create_test_world(&repo).await;
    // Only actor + world-settings exist; light-gradation/vision-modes absent.
    create_test_actor(&repo, world_id).await;
    create_world_settings_doc(&repo, world_id).await;

    let registry = RoomRegistry::new(repo.clone());
    let room = registry.get_or_create(world_id).await.unwrap();
    let scene_ecs = room.scene_ecs.lock().await;

    assert!(scene_ecs.has_world_settings(), "world-settings must hydrate independently");
    assert!(scene_ecs.actor_count() > 0, "actor must hydrate independently");
    assert!(scene_ecs.gradation().is_none(), "absent light-gradation must not error or block others");
    assert!(scene_ecs.vision_modes().is_none(), "absent vision-modes must not error or block others");
}
```

(If `has_world_settings`/`actor_count`/`gradation`/`vision_modes` accessors don't already exist on `SceneEcs`, use whatever accessors the existing test at line 1282 already uses — read that test first and match its assertion style exactly rather than inventing new accessor names.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src/server/Cargo.toml get_or_create_batched_query_handles_partial_doc_type_presence -- --nocapture`
Expected: FAIL (either compile error if helpers don't exist yet, or the test passes already against the unbatched code — in which case this step confirms the CURRENT code already handles partial presence correctly, and the test becomes a regression guard for Step 4's refactor rather than a red/green TDD step. Note which case applies before proceeding.)

- [ ] **Step 3: Add the batched repository method and use it**

In `src/server/src/data/repository.rs`, add to the `Repository` trait:

```rust
async fn query_documents_by_types(
    &self,
    world_id: Uuid,
    doc_types: &[&str],
) -> Result<Vec<Document>, DataError>;
```

In `src/server/src/data/sqlite.rs`, implement it with a single `WHERE doc_type IN (...)` query (bind doc_types as a comma-joined `IN` clause per sqlx's existing binding convention in this file — match whatever parameter-binding style `query_documents` already uses in the same `impl Repository for Sqlite` block).

In `src/server/src/ws/room.rs`, replace lines 739-755:

```rust
let docs = repo.query_documents_by_types(world_id, &["world-settings", "light-gradation", "vision-modes", "actor"]).await?;
let world_settings = docs.iter().find(|d| d.doc_type == "world-settings").cloned();
let gradation = docs.iter().find(|d| d.doc_type == "light-gradation").cloned();
let vision_modes = docs.iter().find(|d| d.doc_type == "vision-modes").cloned();
let actors: Vec<Document> = docs.into_iter().filter(|d| d.doc_type == "actor").collect();
scene_ecs.set_world_config(world_settings, gradation, vision_modes);
scene_ecs.set_actors(actors);
```

Remove the `// TODO: batch these four...` comment.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src/server/Cargo.toml get_or_create -- --nocapture`
Expected: PASS (both the new test and the pre-existing `get_or_create_hydrates_config_and_actors_from_db`)

- [ ] **Step 5: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`
Expected: all green

- [ ] **Step 6: Commit**

```bash
git add src/server/src/ws/room.rs src/server/src/data/repository.rs src/server/src/data/sqlite.rs
git commit -m "perf(server/room): batch cold-room config/actor queries into one WHERE IN"
```

---

## Task 2: Cache `engine_as::<T>()` decode on the vision/lighting/pathfinding hot paths (A2)

**Files:**
- Modify: `src/server/src/scene/mod.rs:114-118` (`engine_as`) and its 19 call sites (lines 462, 501, 548, 596, 650, 707, 749, 802, 818, 839, 863, 1094, 1139, 1185, 1198, 1205, 1334, 1550, 1656)
- Test: `src/server/src/scene/mod.rs` (new unit test near existing `engine_as`-adjacent tests)

**Interfaces:**
- Produces: a per-entity decoded-engine cache keyed on `(Uuid, TypeId)` or a simpler per-call-site cache depending on what the entity-mutation chokepoint (`apply_op`) supports for invalidation.
- Consumes: `Document.engine: Option<serde_json::Value>` (unchanged wire shape).

This is a `[perf]` item — the spec requires measuring first only where "inert until measured" is the honest gate; A2 already has a known cause (full deserialize per call, 19 call sites on hot paths) and a clear fix shape, so implement directly (best-long-term-shape: cache the decoded struct per entity, invalidated on that entity's `engine` mutation) rather than profiling first.

- [ ] **Step 1: Locate the `apply_op` engine-mutation chokepoint**

Before writing any code, grep `src/server/src/scene/mod.rs` for `fn apply_op` and read every arm that mutates a `Document.engine` field. Confirm there is exactly one function through which every engine-field write flows (this is the cache-invalidation hook). If there are multiple entry points, list them all — the cache must invalidate on every one or it will serve stale data (a correctness bug in a vision-adjacent path, treat as `[sec]`-adjacent caution even though A2 isn't spec-tagged `[sec]`).

- [ ] **Step 2: Write the failing test**

```rust
#[test]
fn engine_as_cache_invalidates_on_engine_mutation() {
    let mut ecs = SceneEcs::new();
    let entity_id = ecs.insert_test_wall(/* blocksSight: true, blocksMove: false */);

    let decoded1: WallEngine = ecs.engine_as_cached(entity_id).unwrap();
    assert!(decoded1.blocks_sight);

    // Mutate the engine field through the real apply_op chokepoint.
    ecs.apply_op(&Operation::Update {
        doc_id: entity_id,
        changes: vec![FieldUpdate {
            path: "/engine/blocksSight".into(),
            old: serde_json::json!(true),
            new: serde_json::json!(false),
        }],
    }).unwrap();

    let decoded2: WallEngine = ecs.engine_as_cached(entity_id).unwrap();
    assert!(!decoded2.blocks_sight, "cache must invalidate on engine mutation, not serve stale decode");
}
```

(Adjust `insert_test_wall`/`WallEngine`/`Operation::Update` shape to match whatever test helpers and types Step 1's investigation actually surfaces — these are illustrative of the invariant being tested, not literal existing helper names. Confirm exact names before finalizing this test.)

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --manifest-path src/server/Cargo.toml engine_as_cache_invalidates -- --nocapture`
Expected: FAIL (compile error — `engine_as_cached` doesn't exist yet)

- [ ] **Step 4: Implement the cache**

Add a cache field to `SceneEcs` (or wherever `engine_as` is invoked from, per Step 1's findings) — a `HashMap<Uuid, HashMap<TypeId, Box<dyn Any>>>` per-entity decode cache, or (simpler, preferred if only a handful of distinct `T`s are decoded across the 19 call sites) a `HashMap<Uuid, serde_json::Value>` last-decoded-value cache keyed just on entity id, re-decoding only when the cached `Value` doesn't match the current `doc.engine`. Add `engine_as_cached::<T>(&mut self, entity_id: Uuid) -> Option<T>` alongside the existing free-function `engine_as`, and update the 19 call sites within `scene/mod.rs` to call the cached version where `self`/`&mut self` is available in scope. Wire invalidation into every `apply_op` arm found in Step 1 (clear or update the cache entry for the mutated entity's id).

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --manifest-path src/server/Cargo.toml engine_as_cache -- --nocapture`
Expected: PASS

- [ ] **Step 6: Full server gate + existing vision/pathfinding suites**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`
Expected: all green, including every existing vision/lighting/pathfinding test (the cache must be transparent to all 19 call sites)

- [ ] **Step 7: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "perf(server/scene): cache engine_as decode on vision/lighting/pathfinding hot paths"
```

---

## Task 3: A* window-edge debug log (A3)

**Files:**
- Modify: `src/server/src/scene/pathfinding.rs` (the per-leg A* loop past line 520 — locate the `PathFail::Unreachable` return inside the window-bound check)
- Test: `src/server/src/scene/pathfinding.rs` (existing pathfinding test file/module)

**Interfaces:**
- No new public interface — this is an internal `tracing::debug!` addition only.

- [ ] **Step 1: Locate the window-edge failure site**

Grep `src/server/src/scene/pathfinding.rs` for `PathFail::Unreachable` inside `find()` or the per-leg A* function, specifically the branch that fails because a search node falls outside the `AABB{start∪waypoints∪wall-endpoints}+8-cell margin` window (`WINDOW_MARGIN: i32 = 8` at line 421-422). Confirm the exact variable names in scope (start/end cell, leg index, window bounds) before writing the log line.

- [ ] **Step 2: Write a test asserting the log fires (or, if tracing isn't test-observable here, a test pinning the Unreachable-on-window-edge behavior itself)**

```rust
#[test]
fn pathfind_reports_unreachable_when_route_exceeds_window_margin() {
    // A scene where the only valid route bulges >8 cells beyond the
    // start/waypoints/wall-endpoints AABB — must fail closed as Unreachable,
    // not panic or silently truncate.
    let ecs = build_test_scene_requiring_wide_detour(/* detour > WINDOW_MARGIN cells */);
    let result = ecs.pathfind(/* start */, /* waypoints */, /* footprint */);
    assert!(matches!(result, Err(PathFail::Unreachable)));
}
```

(This test may already exist per the spec's B2 buddy-check origin note — grep for an existing `window`/`margin`/`Unreachable` test in this file first; if found, this step becomes "confirm it exists and passes" rather than writing a new one. Do not write a duplicate.)

- [ ] **Step 3: Run test**

Run: `cargo test --manifest-path src/server/Cargo.toml pathfind_reports_unreachable_when_route_exceeds_window_margin -- --nocapture`
Expected: PASS already (this is a pre-existing fail-closed behavior; A3 only adds observability, not a behavior change) — or write it if genuinely missing.

- [ ] **Step 4: Add the debug log**

At the window-edge `PathFail::Unreachable` return site found in Step 1:

```rust
tracing::debug!(
    leg_index,
    cell = ?failed_cell,
    window = ?search_window,
    "A* leg failed at search-window edge (AABB+8-cell margin) — route may need a wider window"
);
```

- [ ] **Step 5: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`
Expected: all green (no new warnings from the `tracing::debug!` call — confirm `leg_index`/`failed_cell`/`search_window` are all used, or clippy will flag them if the log is behind a feature gate that strips it)

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/pathfinding.rs
git commit -m "obs(server/pathfinding): debug-log A* window-edge leg failures for future tuning"
```

---

## Task 4: Unrestricted-mode mover-vision gate on role, not restriction mode (A4) `[sec]`

**Files:**
- Modify: `src/server/src/ws/room.rs` (`execute_move`, the `mover_vision` computation gate)
- Test: `src/server/src/ws/room.rs` (existing `execute_move`/`MoveStream` test module)

**Interfaces:**
- Consumes: `MovementRestriction` enum (unchanged), `ctx.world_role` / mover role (existing field, exact name to confirm at `ws/room.rs`'s `execute_move` signature).
- Produces: no new public interface — the change is which condition gates the existing `mover_vision` computation.

**Security note:** this is `[sec]` because it touches the vision-sweep computation path. The fix must ONLY change *when a sweep is computed* (a non-GM mover in an Unrestricted scene now also gets one), never *what a sweep is allowed to reveal* — the sweep content itself still goes through the same `player_vision_inputs`/mask primitives unchanged. A GM mover must continue to get no sweep (unchanged).

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn non_gm_mover_gets_progressive_sweep_in_unrestricted_scene() {
    let (repo, world_id, scene_id) = test_world_with_scene(MovementRestriction::Unrestricted).await;
    let (player_ctx, token_id) = test_player_with_token(&repo, world_id, scene_id).await;

    let move_result = execute_move(&repo, &player_ctx, scene_id, token_id, /* destination */ test_pos(5, 5)).await.unwrap();

    assert!(move_result.mover_vision.is_some(), "a non-GM mover in an Unrestricted scene must get a progressive vision sweep, not a static-fog snap");
}

#[tokio::test]
async fn gm_mover_still_gets_no_sweep_in_unrestricted_scene() {
    let (repo, world_id, scene_id) = test_world_with_scene(MovementRestriction::Unrestricted).await;
    let (gm_ctx, token_id) = test_gm_with_token(&repo, world_id, scene_id).await;

    let move_result = execute_move(&repo, &gm_ctx, scene_id, token_id, test_pos(5, 5)).await.unwrap();

    assert!(move_result.mover_vision.is_none(), "GM movers must not get a sweep, regardless of restriction mode (unchanged behavior)");
}
```

(Match `test_world_with_scene`/`test_player_with_token`/`test_gm_with_token`/`test_pos` to whatever test helpers `ws/room.rs`'s existing `execute_move` tests already use — read the surrounding test module before finalizing names.)

- [ ] **Step 2: Run tests to verify the first fails, second passes**

Run: `cargo test --manifest-path src/server/Cargo.toml non_gm_mover_gets_progressive_sweep gm_mover_still_gets_no_sweep -- --nocapture`
Expected: `non_gm_mover_gets_progressive_sweep_in_unrestricted_scene` FAILS (current code gates on `Unrestricted` restriction mode, giving no sweep); `gm_mover_still_gets_no_sweep_in_unrestricted_scene` PASSES already.

- [ ] **Step 3: Change the gate condition**

In `execute_move` (`ws/room.rs`), find the current gate: `matches!(restriction, MovementRestriction::Unrestricted)` guarding `mover_vision` computation. Replace with a role check: compute `mover_vision` whenever the mover is NOT a GM (regardless of `restriction` mode) — i.e. invert the condition to gate on mover role instead of scene restriction mode. Confirm the exact role-check idiom already used elsewhere in this file (likely `ctx.world_role != WorldRole::Gm` or similar) and match it exactly.

- [ ] **Step 4: Run tests to verify both pass**

Run: `cargo test --manifest-path src/server/Cargo.toml non_gm_mover_gets_progressive_sweep gm_mover_still_gets_no_sweep -- --nocapture`
Expected: both PASS

- [ ] **Step 5: Full server gate + existing MoveStream/vision suite**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`
Expected: all green, including every pre-existing `MoveStream`/`execute_move`/vision-sweep test (this must not regress the Restricted/Revealed-mode sweep behavior, only extend Unrestricted-mode for non-GM movers)

- [ ] **Step 6: Commit**

```bash
git add src/server/src/ws/room.rs
git commit -m "fix(server/movement): gate mover-vision sweep on role, not restriction mode

A non-GM player moving in an Unrestricted-mode scene now gets a progressive
vision sweep (was: static fog snap at move end). GM movers still get no
sweep, unchanged."
```

- [ ] **Step 7: Dispatch mandatory security buddy-check** (per plan-level Buddy-check directives) before this task is marked complete.

---

## Task 5: Cache the per-(user, scene) visibility mask for the movement gate (A5) `[sec][perf]`

**Files:**
- Modify: `src/server/src/scene/mod.rs` (wherever `visible_cells`/`player_lit_mask` is invoked from the M10e-4 movement gate — locate exact call site)
- Test: `src/server/src/scene/mod.rs` (movement-gate test module)

**Interfaces:**
- Produces: a `(Uuid /* user */, Uuid /* scene */) -> (mask, generation)` cache on `SceneEcs` (or `Room`, depending on where the mask is currently computed — locate before implementing).
- Consumes: the existing `player_lit_mask` computation (unchanged internals — only the call frequency changes).

**Security note (`[sec]`):** the spec is explicit — **"a stale cache must fail toward recompute, never toward a wider mask."** The cache is an optimization only if invalidation is provably conservative. Any input change that could plausibly widen the mask (token move, wall/light/vision-mode mutation, permission change) must invalidate. When in doubt whether an input affects the mask, invalidate — a missed invalidation is a secrecy bug, not a perf bug.

- [ ] **Step 1: Locate the movement-gate mask computation and its inputs**

Find the exact call site in the M10e-4 movement gate (grep `visible_cells` and `player_lit_mask` together in `scene/mod.rs`/`ws/room.rs`). List every input the mask computation reads (token positions, wall docs, light docs, vision-mode config, world-settings). This input list is what the cache invalidation must key on.

- [ ] **Step 2: Write the failing test**

```rust
#[test]
fn movement_gate_mask_cache_invalidates_on_wall_mutation() {
    let mut ecs = SceneEcs::new();
    let (user_id, scene_id) = /* setup a scene with a token and no walls */;

    let mask1 = ecs.visible_cells_cached(user_id, scene_id);
    assert!(mask1.contains(&test_cell(10, 10)), "cell visible before a wall is added");

    // Add a blocksSight wall that should now occlude that cell.
    ecs.insert_test_wall(/* positioned to block (10,10) from the viewer */);

    let mask2 = ecs.visible_cells_cached(user_id, scene_id);
    assert!(!mask2.contains(&test_cell(10, 10)), "cache must invalidate on wall mutation, never serve a stale wider mask");
}
```

(Match helper names to whatever Step 1's investigation surfaces.)

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --manifest-path src/server/Cargo.toml movement_gate_mask_cache_invalidates -- --nocapture`
Expected: FAIL (compile error — `visible_cells_cached` doesn't exist)

- [ ] **Step 4: Implement the cache**

Add a `HashMap<(Uuid, Uuid), (VisibleCellsMask, u64 /* generation */)>` cache. Add a monotonic per-scene generation counter that increments on ANY mutation to the input set found in Step 1 (token move, wall/light/vision-mode/world-settings mutation — wire into the same `apply_op` chokepoint used by Task 2). `visible_cells_cached(user_id, scene_id)` compares the cached generation to the current scene generation; on mismatch, recompute via the existing (unchanged) `player_lit_mask`/`visible_cells` primitive and update the cache entry. Wire this into the actual movement-gate call site found in Step 1, replacing the on-demand recompute.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --manifest-path src/server/Cargo.toml movement_gate_mask_cache_invalidates -- --nocapture`
Expected: PASS

- [ ] **Step 6: Add a second test proving the cache is actually reused (not just correct)**

```rust
#[test]
fn movement_gate_mask_cache_reused_across_repeated_moves_with_no_scene_change() {
    let mut ecs = SceneEcs::new();
    let (user_id, scene_id) = /* setup */;

    let mask1 = ecs.visible_cells_cached(user_id, scene_id);
    let mask2 = ecs.visible_cells_cached(user_id, scene_id);

    assert_eq!(mask1, mask2);
    // If SceneEcs exposes a cache-hit counter/instrumentation, assert on it here;
    // otherwise this test documents the expected reuse behavior for a future
    // profiling pass rather than mechanically proving zero recomputation.
}
```

- [ ] **Step 7: Full server gate + the ENTIRE existing vision/movement/pathfinding suite**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`
Expected: all green — this cache sits directly on the secrecy gate, so every pre-existing vision/movement test is a regression guard here. Any failure is treated as a correctness bug, not flaky.

- [ ] **Step 8: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "perf(server/scene): cache per-(user,scene) visibility mask for the movement gate

Generation-counter invalidation on every mutation that could affect the
mask (token move, wall/light/vision-mode/world-settings change). Fails
toward recompute, never toward a stale wider mask."
```

- [ ] **Step 9: Dispatch mandatory security buddy-check** before this task is marked complete.

---

## Task 6: Case-insensitive member roster ordering (A6)

**Files:**
- Modify: `src/server/src/data/sqlite.rs:302-314` (`list_members`, line 310)
- Test: `src/server/src/data/sqlite.rs` (extend near `list_members_orders_by_username`, line 1920)

**Interfaces:** No signature change — same `list_members(&self, world_id: Uuid) -> Result<Vec<Member>, DataError>`.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn list_members_orders_case_insensitively() {
    let repo = test_repo().await;
    let world_id = create_test_world(&repo).await;
    create_test_member(&repo, world_id, "Bob").await;
    create_test_member(&repo, world_id, "alice").await;
    create_test_member(&repo, world_id, "Charlie").await;

    let members = repo.list_members(world_id).await.unwrap();
    let names: Vec<&str> = members.iter().map(|m| m.username.as_str()).collect();

    assert_eq!(names, vec!["alice", "Bob", "Charlie"], "case-insensitive order: alice before Bob before Charlie");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src/server/Cargo.toml list_members_orders_case_insensitively -- --nocapture`
Expected: FAIL — binary collation sorts `Bob`/`Charlie` (uppercase) before `alice` (lowercase)

- [ ] **Step 3: Fix the query**

In `src/server/src/data/sqlite.rs:310`, change:
```rust
ORDER BY u.username
```
to:
```rust
ORDER BY u.username COLLATE NOCASE
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src/server/Cargo.toml list_members_orders -- --nocapture`
Expected: both `list_members_orders_case_insensitively` and the pre-existing `list_members_orders_by_username` PASS

- [ ] **Step 5: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add src/server/src/data/sqlite.rs
git commit -m "fix(server/members): case-insensitive roster ordering (COLLATE NOCASE)"
```

---

## Task 7: Bundle link-preview deps into `LinkPreviewDeps` (A7)

**Files:**
- Modify: `src/server/src/chat/link_preview.rs` (add the struct near `LinkPreviewCache`/`PreviewRateLimiter`)
- Modify: `src/server/src/chat/mod.rs:409-423` (`handle_send_message`), `:734+` (`handle_edit_message`)
- Modify: every call site of `handle_send_message`/`handle_edit_message` (grep at task time — spec cites ~40)
- Test: `src/server/src/chat/mod.rs` (existing send/edit message test module — signature-only change, existing test bodies should compile unchanged once call sites are updated)

**Interfaces:**
- Produces:
```rust
pub struct LinkPreviewDeps<'a> {
    pub client: &'a reqwest::Client,
    pub cache: &'a LinkPreviewCache,
    pub rate: &'a PreviewRateLimiter,
}
```
- Consumes: replaces the three positional params `preview_client: &reqwest::Client, preview_cache: &LinkPreviewCache, preview_rate: &PreviewRateLimiter` in both function signatures with one `preview: LinkPreviewDeps<'_>` param.

This is a pure signature refactor — no behavior change. TDD here means: existing tests must continue to pass unmodified in their assertions (only their call syntax to `handle_send_message`/`handle_edit_message` changes).

- [ ] **Step 1: Confirm the existing test suite passes before refactoring (baseline)**

Run: `cargo test --manifest-path src/server/Cargo.toml chat:: -- --nocapture`
Expected: PASS (baseline, before any change)

- [ ] **Step 2: Define `LinkPreviewDeps`**

In `src/server/src/chat/link_preview.rs`, add the struct shown above, near the existing `LinkPreviewCache`/`PreviewRateLimiter` definitions. Derive nothing extra beyond what's needed to construct it inline at call sites (it's a borrow-bundle, not stored).

- [ ] **Step 3: Update the two function signatures**

In `src/server/src/chat/mod.rs`, change `handle_send_message`'s and `handle_edit_message`'s three positional preview params to a single `preview: LinkPreviewDeps<'_>` param. Remove the `#[allow(clippy::too_many_arguments)]` from both if it's no longer needed (check remaining arg count first — if still >7, keep the allow and note in the commit message that it's now justified by genuinely distinct concerns, not preview-plumbing bloat). Update every internal use of `preview_client`/`preview_cache`/`preview_rate` inside both function bodies to `preview.client`/`preview.cache`/`preview.rate`.

- [ ] **Step 4: Update every call site**

Grep the full workspace for `handle_send_message(` and `handle_edit_message(` (server + any test files). For each, wrap the three preview args into a `LinkPreviewDeps { client: ..., cache: ..., rate: ... }` literal at the call site.

- [ ] **Step 5: Run full test suite to verify no regressions**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets`
Expected: PASS — every pre-existing chat/send/edit test compiles and passes with the new call syntax (assertions unchanged)

- [ ] **Step 6: Full server gate**

Run: `cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`
Expected: green, ideally with the `too_many_arguments` allow removed from both functions (confirm actual arg count post-refactor)

- [ ] **Step 7: Commit**

```bash
git add src/server/src/chat/link_preview.rs src/server/src/chat/mod.rs
git commit -m "refactor(server/chat): bundle link-preview deps into LinkPreviewDeps struct

Shrinks handle_send_message/handle_edit_message signatures and removes
call-site arg-order risk across ~40 call sites. No behavior change."
```

---

## Task 8: `set_pointer` true key removal (B1) `[sec]`

**Files:**
- Modify: `src/server/src/data/command.rs:119-169` (`set_pointer`)
- Test: `src/server/src/data/command.rs` (extend near existing tests at lines 265, 275, 294, 301)

**Interfaces:**
- Produces: a removal convention for `set_pointer` — when the new value is a specific sentinel (confirm exact wire convention in Step 1) OR a new dedicated `remove_pointer` variant, the key becomes genuinely absent from the parent object rather than present-with-`null`.
- Consumes: the existing OCC `old` pre-image check (locate exact site — spec research did not pin this down; find it in `apply_intent`'s `Operation::Update` arm in `sqlite.rs` before implementing) and the M13e merge engine's `Document.base` field, which per `templates.ts:116`'s established convention already treats an absent collection key as meaningfully different from `null` — this fix must be consistent with that convention, not introduce a second one.

**Security/correctness note (`[sec]`):** this changes wire-observable document shape (a field that used to round-trip as `{key: null}` will now round-trip as `{}` without the key). Any client code (Zod schemas, `.system.foo === null` checks) that currently distinguishes "explicitly null" from "the update path never removes" must be audited for this change — verify no client-side logic *depends on* the old always-present-as-null behavior before shipping.

- [ ] **Step 1: Locate the OCC pre-image check and decide the wire convention**

Before writing code, find the `Operation::Update` OCC check in `src/server/src/data/sqlite.rs` (`apply_intent`). Confirm exactly how it compares the intent's `old` field against the currently-stored value for a given pointer path, and confirm it currently only ever compares against present values (never "key must be absent"). Then decide (best-long-term-shape, per project design-fork rule — this is answerable without a user round-trip): a `RemovePointer` command variant (parallel to the existing `SetPointer`, explicit and unambiguous) is the better long-term shape than overloading a sentinel value in `SetPointer`'s existing `new` field, because a sentinel risks colliding with a legitimate value a system author might want to store. Implement `RemovePointer`.

- [ ] **Step 2: Write the failing test**

```rust
#[test]
fn set_pointer_removes_a_key_making_it_absent() {
    let mut doc = test_document_with_system(json!({"foo": "bar", "baz": 1}));

    remove_pointer(&mut doc, "/system/foo").unwrap();

    let system = doc.system.as_ref().unwrap();
    assert!(!system.as_object().unwrap().contains_key("foo"), "key must be genuinely absent, not present-as-null");
    assert_eq!(system["baz"], json!(1), "sibling keys untouched");
}

#[test]
fn remove_pointer_on_already_absent_key_is_a_no_op() {
    let mut doc = test_document_with_system(json!({"baz": 1}));
    let result = remove_pointer(&mut doc, "/system/foo");
    assert!(result.is_ok(), "removing an already-absent key must not error");
}

#[test]
fn remove_pointer_occ_pre_image_checks_current_presence() {
    // An OCC-guarded remove: `old` in the intent must match the CURRENT
    // stored value at that path before the removal is allowed — mirrors
    // set_pointer's existing OCC semantics, just for the removal direction.
    let mut doc = test_document_with_system(json!({"foo": "bar"}));
    let stale_intent_result = apply_remove_with_occ(&mut doc, "/system/foo", /* old */ json!("wrong-value"));
    assert!(matches!(stale_intent_result, Err(DataError::Conflict(_))));
}
```

(Match `test_document_with_system`/`apply_remove_with_occ` to whatever helpers `command.rs`'s existing tests already use.)

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path src/server/Cargo.toml set_pointer_removes_a_key remove_pointer_on_already_absent remove_pointer_occ -- --nocapture`
Expected: FAIL (compile error — `remove_pointer` doesn't exist)

- [ ] **Step 4: Implement `remove_pointer`**

In `src/server/src/data/command.rs`, add a function parallel to `set_pointer` (lines 119-169) that walks the same intermediate-path-descent logic but, at the leaf, calls `m.remove(tok)` instead of `m.insert(tok.clone(), new)`. Reuse `set_pointer`'s existing path-descent code (factor the shared traversal into a helper both call, rather than duplicating it — DRY) since removal needs the same "descend into existing intermediate objects, reject descending into a scalar" logic (mirrored from the existing `set_pointer_rejects_descend_into_scalar` test at line 294). Wire the OCC `old` pre-image check found in Step 1 to also validate the pre-removal state.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src/server/Cargo.toml set_pointer remove_pointer -- --nocapture`
Expected: all PASS, including the pre-existing `set_pointer_*` tests (unmodified, still passing)

- [ ] **Step 6: Wire `RemovePointer` into the command dispatch + `Operation::Update` handling**

Find wherever `Command::SetPointer` is matched/dispatched (likely in `command.rs` or the `apply_intent` arm in `sqlite.rs`) and add the equivalent `Command::RemovePointer` arm. Add the ts-rs wire type + client Zod mirror if `Command` is a wire-exposed enum (check `command.rs`'s derives — if it has `#[derive(TS)]`, the client mirror in `src/client/core/src/` needs the matching variant; if `Command` is server-internal only, skip client changes).

- [ ] **Step 7: Verify the merge-engine `Document.base` interaction**

Write one more test proving a removed key round-trips correctly through the M13e merge snapshot/restore path — read `shadowcat-codebase-templates` skill first for the exact `snapshotBase`/`restampSubtree` mechanics, then add a server-side test (or note that this interaction is purely client-side per `templates.ts` and must be verified there instead — confirm which side owns this before writing a test in the wrong place).

- [ ] **Step 8: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`

- [ ] **Step 9: Commit**

```bash
git add src/server/src/data/command.rs
git commit -m "feat(server/command): add RemovePointer — true key removal, not old:null

A removed key becomes genuinely absent (null != absent). Reuses
set_pointer's path-descent logic; OCC pre-image checks pre-removal state.
Now that the M13e merge engine has landed, this closes the deferred
removal-semantics gap."
```

- [ ] **Step 10: Dispatch mandatory security buddy-check** (escalate to `sdd-code-reviewer-opus`/`sdd-spec-reviewer-opus` twins per Model/Effort directives) before this task is marked complete.

---

## Task 9: Singleton doc_type create-gate (B2) `[sec]`

**Files:**
- Modify: `src/server/src/data/sqlite.rs:1036-1124+` (`apply_intent`, `Operation::Create` arm)
- Modify: `src/server/src/data/repository.rs` (if a new trait method is needed for the in-transaction existence check)
- Test: `src/server/src/data/sqlite.rs` (new test module section for Create-gate)

**Interfaces:**
- Produces: a `SINGLETON_DOC_TYPES: &[&str] = &["world-settings", "faction-registry", "condition-registry", "chat-settings", "dice-settings"]` const (confirm exact `CHAT_SETTINGS_DOC_TYPE` string value from `src/server/src/chat/settings.rs:70` before finalizing — spec research didn't confirm the literal, only pattern-matched it against `"dice-settings"` at line 84), consulted at the `apply_intent` Create chokepoint.
- Consumes: existing `query_documents(&self, world_id, doc_type) -> Result<Vec<Document>, DataError>` for the in-transaction existence check.

**Security note (`[sec]`):** must run **inside the same transaction** as the Create itself (check-then-act TOCTOU risk per the project's `two-queries-guard-needs-tx` lesson — a concurrent Create racing the check must not both succeed). Fail closed: reject the second Create with a clear conflict error, never silently allow or silently drop it.

- [ ] **Step 1: Confirm the exact singleton doc_type constant values**

Read `src/server/src/chat/settings.rs:70,84` and confirm `CHAT_SETTINGS_DOC_TYPE`'s and `DICE_SETTINGS_DOC_TYPE`'s literal string values. Read `src/server/src/data/document.rs:607,615,616` and `src/server/src/engine/mod.rs:48-55,114-121` and confirm the bare string literals used there for `"world-settings"`, `"faction-registry"`, `"condition-registry"`. Decide whether to introduce named consts for these three (best-long-term-shape: yes — a `SINGLETON_DOC_TYPES` list should reference named constants, not bare literals, to avoid drift) and add them if missing.

- [ ] **Step 2: Write the failing test**

```rust
#[tokio::test]
async fn create_rejects_a_second_singleton_doc_of_the_same_type() {
    let repo = test_repo().await;
    let world_id = create_test_world(&repo).await;
    let gm_ctx = test_gm_context(&repo, world_id).await;

    create_test_document(&repo, &gm_ctx, world_id, "world-settings", json!({})).await.unwrap();

    let second = create_test_document(&repo, &gm_ctx, world_id, "world-settings", json!({})).await;
    assert!(matches!(second, Err(DataError::Conflict(_))), "a second world-settings doc in the same world must be rejected");
}

#[tokio::test]
async fn create_allows_singleton_doc_types_in_different_worlds() {
    let repo = test_repo().await;
    let world_a = create_test_world(&repo).await;
    let world_b = create_test_world(&repo).await;
    let gm_a = test_gm_context(&repo, world_a).await;
    let gm_b = test_gm_context(&repo, world_b).await;

    create_test_document(&repo, &gm_a, world_a, "world-settings", json!({})).await.unwrap();
    let result = create_test_document(&repo, &gm_b, world_b, "world-settings", json!({})).await;

    assert!(result.is_ok(), "singleton scoping is per-world, not global");
}

#[tokio::test]
async fn create_does_not_gate_non_singleton_doc_types() {
    let repo = test_repo().await;
    let world_id = create_test_world(&repo).await;
    let gm_ctx = test_gm_context(&repo, world_id).await;

    create_test_document(&repo, &gm_ctx, world_id, "actor", json!({})).await.unwrap();
    let second = create_test_document(&repo, &gm_ctx, world_id, "actor", json!({})).await;

    assert!(second.is_ok(), "non-singleton doc types (e.g. actor) must remain uncapped");
}
```

(Match `test_gm_context`/`create_test_document` to `sqlite.rs`'s existing `apply_intent`/Create test helpers.)

- [ ] **Step 3: Run tests to verify the first two fail (or pass, if a duplicate already succeeds today) and the third passes**

Run: `cargo test --manifest-path src/server/Cargo.toml create_rejects_a_second_singleton create_allows_singleton_doc_types_in_different_worlds create_does_not_gate_non_singleton -- --nocapture`
Expected: `create_rejects_a_second_singleton_doc_of_the_same_type` FAILS (no gate exists yet — the second Create currently succeeds); the other two PASS already (no gate means no restriction yet, so cross-world and non-singleton cases are unaffected).

- [ ] **Step 4: Implement the create-gate inside the existing Create transaction**

In `apply_intent`'s `Operation::Create` arm (`sqlite.rs:1036-1124+`), before the insert, add: if `doc.doc_type` is in `SINGLETON_DOC_TYPES`, run `query_documents(world_id, doc.doc_type)` **within the same transaction** (use the transaction handle already in scope for this arm, not a fresh connection — confirm the transaction is passed through or held at this point in the function) and reject with `DataError::Conflict` if any existing document of that doc_type is found for this world.

- [ ] **Step 5: Run tests to verify all three pass**

Run: `cargo test --manifest-path src/server/Cargo.toml create_rejects_a_second_singleton create_allows_singleton_doc_types_in_different_worlds create_does_not_gate_non_singleton -- --nocapture`
Expected: all PASS

- [ ] **Step 6: Concurrency test proving the TOCTOU guard**

```rust
#[tokio::test]
async fn create_gate_is_race_safe_under_concurrent_creates() {
    let repo = test_repo().await;
    let world_id = create_test_world(&repo).await;
    let gm_ctx = test_gm_context(&repo, world_id).await;

    let (r1, r2) = tokio::join!(
        create_test_document(&repo, &gm_ctx, world_id, "faction-registry", json!({})),
        create_test_document(&repo, &gm_ctx, world_id, "faction-registry", json!({})),
    );

    let ok_count = [r1.is_ok(), r2.is_ok()].iter().filter(|x| **x).count();
    assert_eq!(ok_count, 1, "exactly one of two concurrent singleton Creates must succeed, never both, never neither");
}
```

- [ ] **Step 7: Run the concurrency test**

Run: `cargo test --manifest-path src/server/Cargo.toml create_gate_is_race_safe -- --nocapture`
Expected: PASS. If flaky/fails, the check-then-insert is not actually running inside one transaction — fix before proceeding (this is the core security property of the task).

- [ ] **Step 8: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`

- [ ] **Step 9: Commit**

```bash
git add src/server/src/data/sqlite.rs src/server/src/data/repository.rs
git commit -m "feat(server/documents): construction-time singleton doc_type create-gate

Transactional check-then-create rejects a second faction-registry/
condition-registry/world-settings/chat-settings/dice-settings doc per
world. Complements the existing deterministic-lowest-UUID resolution
(M11d-3) as the write-side guarantee; client seed-guards remain UX-only."
```

- [ ] **Step 10: Dispatch mandatory security buddy-check** before this task is marked complete.

---

## Task 10: Document `Room::publish` Create-gate as by-design (B3)

**Files:**
- Modify: `src/server/src/ws/room.rs` (near line 209-213, the existing M9a movement-collision block comment)
- Modify: `docs/design/ARCHITECTURE.md` (invariant 6 area — add a note; find the exact invariant-6 section first)

**Interfaces:** No code behavior change — documentation only.

- [ ] **Step 1: Add the doc comment in `Room::publish`**

In `src/server/src/ws/room.rs`, immediately before the `for op in &ops` loop at line ~213 (the loop that only matches `Operation::Update { .. }` for the wall/vision movement gate, confirmed to never inspect `Operation::Create`), add:

```rust
// By design: this movement gate only inspects Operation::Update (a token
// move). Operation::Create (initial token placement) is intentionally
// ungated — the create capability is already a privileged grant (GM or a
// place-token tool), and unrestricted initial placement is normal
// authoring behavior. This is not a movement-restriction bypass: it is
// the placement path, not the move path. (Resolved design question,
// Phase-1 cleanup burndown 2026-07-19 — see docs/design/ARCHITECTURE.md
// invariant 6.)
```

- [ ] **Step 2: Add the ARCHITECTURE.md note**

Find `docs/design/ARCHITECTURE.md`'s invariant-6 section (server-authoritative geometry / movement gate). Add a short note stating the Create-vs-Update scoping decision from Step 1, in ARCHITECTURE's own voice (present-tense constraint statement, no narrative — per the project's commenting rules).

- [ ] **Step 3: Verify no test coverage gap was masking a real bug**

Run the existing wall/movement-gate test suite to confirm there is no test that (incorrectly) asserts Create IS gated — if one exists, it was testing an aspirational behavior, not actual behavior, and should be corrected or removed per the project's "tests yield to correct code" rule (only after confirming this is genuinely the intended design, which the user has already confirmed).

Run: `cargo test --manifest-path src/server/Cargo.toml ws::room:: -- --nocapture`
Expected: PASS, no such conflicting test found (or, if found, fixed per the above).

- [ ] **Step 4: Commit**

```bash
git add src/server/src/ws/room.rs docs/design/ARCHITECTURE.md
git commit -m "docs(server/movement): document Create-gate scoping as by-design

Room::publish's wall/vision gate intentionally only inspects
Operation::Update. Initial placement (Create) stays GM/tool-privileged
and unrestricted — resolved design question, not an oversight."
```

---

## Task 11: Edge-projected, `blocksLight`-occludable environment light (C1) `[sec]`

**Files:**
- Modify: `src/server/src/scene/lighting.rs:152-187` (`cell_illumination`)
- Modify: `src/server/src/scene/mod.rs` (lines 1241-1440 area — `lighting_inputs`/`player_lit_mask` and the two call sites at lines 1427/1717 that pass `settings.env_intensity`)
- Test: `src/server/src/scene/lighting.rs` and/or `src/server/src/scene/mod.rs` (existing lighting test module)

**Interfaces:**
- Produces: a new function computing an "environment occlusion" input — analogous in shape to how placed lights get a `lit_polys[k]` visibility polygon (per `cell_illumination`'s existing pattern at lines 169-175), but sourced from the scene's own boundary (`scene.system.bounds`, via `ResolvedScene.bounds` — confirmed present at `mod.rs:518-539`) rather than a point light. Exact function name/signature to be finalized in Step 1 (this is the genuinely open design fork the research flagged — no existing "environment as boundary-projected source" code exists to copy).
- Consumes: `blocksLight` wall data (existing), `ResolvedScene.bounds` (existing, `mod.rs:520-526`, fail-closed default already tested at `mod.rs:3078-3101`).

**Security note (`[sec]`):** lighting is explicitly **cosmetic** — this must NEVER become a secrecy input. `player_lit_mask` (the actual fog/vision secrecy gate) must be unaffected by this change; only the rendered lighting hint changes. Verify this by grep: confirm `player_lit_mask`'s LOS ∩ (lit ∨ darkvision) computation does not consume `cell_illumination`'s ambient term as a *visibility* input (only vision-mode/darkvision/LOS should gate visibility — illumination is a separate rendering channel per `M10e-3`'s "lighting is cosmetic" invariant).

- [ ] **Step 1: Design the boundary-projection primitive**

Read `src/server/src/scene/mod.rs` lines 1241-1440 in full (the `lighting_inputs`/`player_lit_mask` area) before writing any code — this was flagged by research as unread in the prior pass. Also read `src/server/src/scene/lighting.rs` lines 140-187 in full for `cell_illumination`'s complete placed-light occlusion pattern. Design decision (best-long-term-shape, resolvable without a user round-trip): model the scene boundary as N synthetic edge-sample "sources" (e.g. one per wall-segment midpoint along the boundary, or a simpler uniform edge-sample count — choose a fixed sample density, e.g. one sample per grid-unit of perimeter, capped at a `MAX_ENV_LIGHT_SAMPLES` DoS bound matching the project's existing fail-closed-bound convention seen in `pathfinding.rs`'s `MAX_PATH_NODES` etc.), each occluded by `blocksLight` walls via the SAME visibility-polygon primitive `sight_walls`/vision raycasting already uses (never a second independent occlusion computation — this is the "never fork the mask" invariant, extended to lighting). Compose per-cell via max (matching `cell_illumination`'s existing per-light max-compose pattern at line ~170).

- [ ] **Step 2: Write the failing test**

```rust
#[test]
fn env_light_is_occluded_by_blocks_light_wall_sealing_an_interior() {
    let mut ecs = SceneEcs::new();
    let scene_id = ecs.insert_test_scene(/* bounds: 50x50, env_intensity: 1.0 */);
    // A fully sealed interior room in the middle of the scene, walls flagged blocksLight.
    ecs.insert_sealed_interior_room(scene_id, /* rect: (20,20)-(30,30) */, /* blocks_light: true */);

    let interior_cell = test_cell(25, 25);
    let exterior_cell = test_cell(5, 5);

    let interior_level = ecs.cell_illumination_for_test(scene_id, interior_cell);
    let exterior_level = ecs.cell_illumination_for_test(scene_id, exterior_cell);

    assert!(interior_level < exterior_level, "a blocksLight-sealed interior must be darker than the open exterior under ambient environment light");
}

#[test]
fn env_light_reaches_an_open_scene_uniformly() {
    let mut ecs = SceneEcs::new();
    let scene_id = ecs.insert_test_scene(/* bounds: 50x50, env_intensity: 1.0, no walls */);

    let center = ecs.cell_illumination_for_test(scene_id, test_cell(25, 25));
    let corner = ecs.cell_illumination_for_test(scene_id, test_cell(2, 2));

    assert!(center > 0.0 && corner > 0.0, "an unobstructed scene gets ambient light everywhere, not just near the boundary");
}

#[test]
fn env_light_does_not_affect_player_lit_mask_secrecy_gate() {
    // Cosmetic-only invariant: the fog/vision secrecy computation must be
    // byte-for-byte unaffected by this change when LOS/darkvision are held fixed.
    let mut ecs = SceneEcs::new();
    let scene_id = ecs.insert_test_scene(/* env_intensity: 1.0 */);
    let (user_id, token_id) = ecs.insert_test_player_token(scene_id);

    let mask_with_env_light = ecs.player_lit_mask(user_id, scene_id);

    ecs.set_env_intensity(scene_id, 0.0);
    let mask_without_env_light = ecs.player_lit_mask(user_id, scene_id);

    assert_eq!(mask_with_env_light, mask_without_env_light, "player_lit_mask (the secrecy gate) must not change with ambient lighting — lighting is cosmetic only");
}
```

(Match helper names to whatever Step 1's file-read surfaces as the real test scaffolding in `lighting.rs`/`mod.rs`.)

- [ ] **Step 3: Run tests to verify they fail appropriately**

Run: `cargo test --manifest-path src/server/Cargo.toml env_light -- --nocapture`
Expected: `env_light_is_occluded_by_blocks_light_wall_sealing_an_interior` FAILS (current flat-floor ambient ignores occlusion); `env_light_reaches_an_open_scene_uniformly` likely PASSES already (flat floor is already uniform); `env_light_does_not_affect_player_lit_mask_secrecy_gate` should PASS already (confirming the current baseline is safe) — if it fails, STOP, this is a pre-existing secrecy bug outside this task's scope, flag it immediately per the project's complication-reporting rule rather than proceeding.

- [ ] **Step 4: Implement the boundary-projection occlusion**

Add the new function designed in Step 1 (e.g. `fn env_light_occlusion_polygon(scene: &ResolvedScene, walls: &[Wall]) -> Polygon` or a per-cell occlusion mask, matching whichever shape `cell_illumination`'s existing `lit_polys` consumption expects). Wire it into `cell_illumination` (`lighting.rs:165-168`) so the ambient term is gated by this polygon the same way placed-light terms are gated by their own `lit_polys[k]` (line ~169-175). Update both call sites (`mod.rs:1427`, `:1717`) to compute and pass the new occlusion input alongside the existing `settings.env_intensity`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src/server/Cargo.toml env_light -- --nocapture`
Expected: all three PASS

- [ ] **Step 6: Full server gate + full lighting/vision suite**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`
Expected: all green, including every pre-existing lighting/vision test — the cosmetic-only invariant (Step 2's third test) must hold across the entire suite, not just the new test.

- [ ] **Step 7: Commit**

```bash
git add src/server/src/scene/lighting.rs src/server/src/scene/mod.rs
git commit -m "feat(server/scene): edge-projected, blocksLight-occludable environment light

Ambient light now projects from the scene boundary (scene.system.bounds,
unblocked by M10f-0) and is occluded by blocksLight walls, closing the
M10e-2 deviation (was: flat scene-wide floor). Lighting stays cosmetic —
player_lit_mask (the fog/vision secrecy gate) is unaffected by construction."
```

- [ ] **Step 8: Dispatch mandatory security buddy-check** (escalate to opus twins — this is the flagged genuine design fork) before this task is marked complete.

---

## Task 12: Wall-less scene full intrascene vision (C2) `[sec]`

**Files:**
- Modify: `src/server/src/scene/mod.rs:700-719` (`player_vision_polygons`) and the parallel `player_vision_inputs` call at line 766 (both call `vision::bound_for`)
- Modify: `src/server/src/scene/vision.rs:63-65` (`bound_for`, doc comment confirms the current degenerate-box behavior)
- Test: `src/server/src/scene/mod.rs` (vision test module)

**Interfaces:**
- Modifies: `vision::bound_for(vp: Point, walls: &[Wall], margin: f64) -> Rect` — add a scene-bounds-aware variant or an additional parameter so a wall-less (or near-wall-less) scene bounds to `ResolvedScene.bounds` instead of `VISION_BOUND_MARGIN` around just the viewpoint.
- Consumes: `ResolvedScene.bounds` (existing, same primitive Task 11/C1 uses — `mod.rs:917`'s pattern: `let bounds = self.resolve_scene(scene).bounds;`).

**Security note (`[sec]`):** per research, the fix must apply to **both** `player_vision_polygons` (line 715) **and** `player_vision_inputs` (line 766) — fixing only one reintroduces the "two vision paths diverge" defect class the project explicitly guards against (`mod.rs`'s own doc comments at lines 727/781 state "same wall set and raycast primitives, no fork"). The returned bound must stay keyed to the single scene being computed for (the existing `Vec<(Uuid, Vec<vision::P>)>` return already scopes each polygon to its own `scene: Uuid` — do not widen beyond that scene's own bounds, which would reintroduce the cross-scene-leak class the M12d `viewedSceneId` fix guards against on the client side).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn wall_less_scene_gives_full_intrascene_vision_not_a_degenerate_box() {
    let mut ecs = SceneEcs::new();
    let scene_id = ecs.insert_test_scene(/* bounds: 40x40 grid units, no walls */);
    let (user_id, _token_id) = ecs.insert_test_player_token(scene_id, /* pos: (5, 5) */);

    let polys = ecs.player_vision_polygons(user_id);
    let (_, poly) = polys.iter().find(|(sid, _)| *sid == scene_id).expect("scene present");

    // A far corner of the 40x40 scene, well outside the old viewpoint±margin box.
    let far_corner = test_point(38.0, 38.0);
    assert!(vision::point_in_poly(poly, far_corner), "a wall-less scene must reveal its own full bounded extent, not a small box around the viewpoint");
}

#[test]
fn wall_less_scene_vision_does_not_leak_beyond_its_own_bounds() {
    let mut ecs = SceneEcs::new();
    let scene_id = ecs.insert_test_scene(/* bounds: 40x40, no walls */);
    let (user_id, _) = ecs.insert_test_player_token(scene_id, /* pos: (5, 5) */);

    let polys = ecs.player_vision_polygons(user_id);
    let (_, poly) = polys.iter().find(|(sid, _)| *sid == scene_id).unwrap();

    let beyond_bounds = test_point(1000.0, 1000.0);
    assert!(!vision::point_in_poly(poly, beyond_bounds), "vision must stay bounded to the scene's own extent, never unbounded");
}

#[test]
fn player_vision_polygons_and_player_vision_inputs_agree_on_wall_less_bound() {
    // The two vision paths must not fork: same wall set (empty), same bound.
    let mut ecs = SceneEcs::new();
    let scene_id = ecs.insert_test_scene(/* bounds: 40x40, no walls */);
    let (user_id, token_id) = ecs.insert_test_player_token(scene_id, /* pos: (5, 5) */);

    let poly_from_polygons = ecs.player_vision_polygons(user_id);
    let poly_from_inputs = ecs.player_vision_inputs(token_id);

    assert_eq!(
        poly_from_polygons.iter().find(|(sid, _)| *sid == scene_id).map(|(_, p)| p.clone()),
        poly_from_inputs.polygon_for(scene_id),
        "player_vision_polygons and player_vision_inputs must compute the identical bound for the same wall-less scene"
    );
}
```

(Match method/type names — `player_vision_inputs`'s exact return shape and `polygon_for` accessor — to what's actually in `mod.rs` at line 766's surrounding code; confirm before finalizing.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src/server/Cargo.toml wall_less_scene -- --nocapture`
Expected: `wall_less_scene_gives_full_intrascene_vision_not_a_degenerate_box` FAILS (current `bound_for` gives a tiny margin box); `wall_less_scene_vision_does_not_leak_beyond_its_own_bounds` PASSES already (the degenerate box is a strict subset, not a leak); the parity test should PASS already (both paths currently share the same degenerate-box bug identically) — if the parity test fails on the CURRENT code, that's a pre-existing divergence bug, flag immediately per complication-reporting rather than folding it silently into this task's fix.

- [ ] **Step 3: Implement the scene-bounds-aware `bound_for`**

In `src/server/src/scene/vision.rs`, modify `bound_for` (or add a new variant `bound_for_scene(vp: Point, walls: &[Wall], scene_bounds: (f64, f64), margin: f64) -> Rect`) so that when `walls` is empty (or, more robustly per the doc comment's phrasing, whenever the wall-derived bound would be smaller than the scene's own extent), it returns a rect covering the full `scene_bounds` (clamped to non-negative scene coordinates) instead of `VISION_BOUND_MARGIN` around the viewpoint. Update both call sites (`mod.rs:715` and `:766`) to pass `self.resolve_scene(scene).bounds` through (mirroring the existing pattern at `mod.rs:917`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src/server/Cargo.toml wall_less_scene player_vision_polygons_and_player_vision_inputs_agree -- --nocapture`
Expected: all PASS

- [ ] **Step 5: Full server gate + full vision suite**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`
Expected: all green — every pre-existing vision test (walled scenes, mixed scenes) must be unaffected; only the wall-less/near-wall-less degenerate case changes.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/mod.rs src/server/src/scene/vision.rs
git commit -m "fix(server/scene): wall-less scene gets full intrascene vision, not a degenerate box

bound_for now bounds to the scene's own extent (scene.system.bounds) when
no walls constrain it, instead of a small viewpoint±margin box. Applied
to both player_vision_polygons and player_vision_inputs identically — no
fork. Strictly bounded to the single scene being computed for; cannot
cross-scene-leak."
```

- [ ] **Step 7: Dispatch mandatory security buddy-check** before this task is marked complete.

---

## Task 13: `spawn_blocking` for `scan_installed_modules` (D1)

**Files:**
- Modify: `src/server/src/modules.rs:63-103` (`scan_installed_modules`) — no signature change to the pure function itself
- Modify: `src/server/src/ws/conn.rs:889` (the call site inside `welcome_capability_requirements`, line 869)
- Test: `src/server/src/ws/conn.rs` (Welcome-path test module)

**Interfaces:**
- Consumes: `scan_installed_modules(modules_dir: &Path) -> Vec<InstalledModule>` (unchanged, still callable synchronously from non-async contexts like existing tests).
- Produces: an async wrapper at the call site: `spawn_blocking(move || scan_installed_modules(&dir)).await.unwrap_or_default()`, mirroring `src/server/src/auth/password.rs:28-32`'s existing convention exactly (async wrapper fn, owned `'static` args, `.await` then handle the `JoinError`).

- [ ] **Step 1: Write the failing test**

Since blocking-vs-non-blocking isn't directly assertable via a unit test, write a test proving the Welcome path still returns correct capability requirements when modules exist on disk (a behavior-preservation test, not a red/green blocking-detection test):

```rust
#[tokio::test]
async fn welcome_capability_requirements_still_resolves_module_requirements_via_spawn_blocking() {
    let tmp_modules_dir = create_test_modules_dir_with_one_module(/* requirements: [...] */).await;
    let repo = test_repo().await;
    let world_id = create_test_world(&repo).await;

    let reqs = welcome_capability_requirements(&repo, world_id, &tmp_modules_dir).await.unwrap();

    assert!(reqs.iter().any(|r| r.path_prefix == /* the test module's declared prefix */), "module-declared requirements must still resolve correctly when scan runs via spawn_blocking");
}
```

(Match `create_test_modules_dir_with_one_module`/`welcome_capability_requirements`'s exact signature to `conn.rs`'s existing test scaffolding — confirm whether `welcome_capability_requirements` already takes a `modules_dir` param or reads it from server config, adjusting the test accordingly.)

- [ ] **Step 2: Run test to verify it passes on the CURRENT (blocking) code — baseline**

Run: `cargo test --manifest-path src/server/Cargo.toml welcome_capability_requirements_still_resolves -- --nocapture`
Expected: PASS (this confirms correct behavior before the refactor; Step 4 must keep it passing, proving the refactor is behavior-preserving)

- [ ] **Step 3: Wrap the call in `spawn_blocking`**

At `src/server/src/ws/conn.rs:889`, replace the direct `scan_installed_modules(&dir)` call with:

```rust
let modules_dir = dir.clone(); // clone into an owned 'static value for the blocking closure
let installed = tokio::task::spawn_blocking(move || scan_installed_modules(&modules_dir))
    .await
    .unwrap_or_default();
```

(A scan failure — e.g. the `JoinError` from a panicked blocking task — degrades to an empty `Vec`, matching the existing missing-dir behavior already at `modules.rs:67`, per the research finding. Confirm `dir`'s exact type/ownership at the call site before finalizing — it must be `Clone` or already owned to move into the closure.)

- [ ] **Step 4: Run test to verify it still passes**

Run: `cargo test --manifest-path src/server/Cargo.toml welcome_capability_requirements_still_resolves -- --nocapture`
Expected: PASS

- [ ] **Step 5: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add src/server/src/ws/conn.rs
git commit -m "perf(server/modules): wrap scan_installed_modules in spawn_blocking

Blocking std::fs I/O now runs off the tokio worker on every WS-connect
Welcome path, matching the existing spawn_blocking convention in
auth/password.rs."
```

---

## Task 14: Dedup `welcome_capability_requirements` entries (D2)

**Files:**
- Modify: `src/server/src/data/document.rs:140-145` (`CapabilityRequirement` struct — add `Hash` derive if the chosen strategy needs it)
- Modify: `src/server/src/ws/conn.rs:869-905` (`welcome_capability_requirements`)
- Test: `src/server/src/ws/conn.rs` (Welcome-path test module)

**Interfaces:**
- Produces: `welcome_capability_requirements` returns a `Vec<CapabilityRequirement>` with no two entries sharing the same `path_prefix` — caps are unioned into a single entry per `path_prefix` (best-long-term-shape per research: "merging caps per distinct path_prefix" is the correct semantic, not dropping a duplicate — a GM requirement and a module requirement on the same prefix should both apply, unioned, not one silently discarded).

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn welcome_capability_requirements_unions_caps_for_the_same_path_prefix() {
    let repo = test_repo().await;
    let world_id = create_test_world(&repo).await;
    // A GM-authored requirement on "/scene" requiring cap "read", plus a
    // module declaring a requirement on the SAME "/scene" prefix requiring "write".
    set_gm_cap_requirement(&repo, world_id, "/scene", &["read"]).await;
    let modules_dir = test_modules_dir_with_requirement("/scene", &["write"]).await;

    let reqs = welcome_capability_requirements(&repo, world_id, &modules_dir).await.unwrap();

    let scene_reqs: Vec<_> = reqs.iter().filter(|r| r.path_prefix == "/scene").collect();
    assert_eq!(scene_reqs.len(), 1, "must not emit two entries for the same path_prefix");
    assert_eq!(scene_reqs[0].caps, BTreeSet::from(["read".to_string(), "write".to_string()]), "caps from both sources must be unioned, not one dropped");
}
```

(Match `set_gm_cap_requirement`/`test_modules_dir_with_requirement` to existing test helpers in `conn.rs`/`modules.rs`'s test modules.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src/server/Cargo.toml welcome_capability_requirements_unions_caps -- --nocapture`
Expected: FAIL — current code emits two separate entries via `out.extend(...)` (line 900) with no merge

- [ ] **Step 3: Implement the union-by-path_prefix accumulator**

In `welcome_capability_requirements` (`conn.rs:869-905`), replace the flat `out: Vec<CapabilityRequirement>` accumulation with a `BTreeMap<String, BTreeSet<String>>` (`path_prefix -> caps`) built by iterating both the GM-authored requirements and every module's `requirements`, unioning `caps` into the map entry for each `path_prefix`. At the end, convert the map back into `Vec<CapabilityRequirement>`. No `Hash`/`Ord` derive is needed on `CapabilityRequirement` itself for this approach (the dedup key is `String`, not the struct) — skip modifying `document.rs:140-145` unless a later step's implementation genuinely needs it.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src/server/Cargo.toml welcome_capability_requirements -- --nocapture`
Expected: PASS, including all pre-existing `welcome_capability_requirements` tests (union must be a superset-preserving change — no existing single-source test should lose caps)

- [ ] **Step 5: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add src/server/src/ws/conn.rs
git commit -m "fix(server/ws): dedup welcome_capability_requirements by path_prefix

GM-authored and module-declared requirements sharing a path_prefix now
union their caps into one entry instead of emitting duplicates."
```

---

## Task 15: `modules.e2e.test.ts` fixture version tracks the running server (D3)

**Files:**
- Modify: `src/client/core/src/e2e/modules.e2e.test.ts:18`
- Confirm: whether `ServerMsg::Welcome.server_version` (`src/server/src/ws/protocol.rs:188`) is fetchable outside a WS handshake, or whether the e2e test must open one to read it

**Interfaces:**
- Consumes: `ServerMsg::Welcome { server_version: String, .. }` (existing wire field, already sent on every WS connect per `conn.rs:953`).

- [ ] **Step 1: Confirm how the e2e test's `test_server` binary exposes its version**

Read `modules.e2e.test.ts` in full to see how it currently spins up/connects to the test server. Confirm whether it already opens a WS connection early (in which case reading `server_version` off the `Welcome` message is free) or needs a new step. Also check whether `/api/config` (mentioned in the M7 milestone as a public config endpoint) exposes a version field — if so, and if it's simpler than a WS round-trip for this fixture's setup phase, prefer it.

- [ ] **Step 2: Write the failing test (a meta-test proving the fixture is version-agnostic)**

Since this is itself a test-fixture fix, the "test" is: bump the test server's reported version past `0.1.x` in a controlled way and confirm the existing `enable → 204` assertion still passes:

```typescript
test("module enable succeeds even when the fixture engines range doesn't hardcode a stale version", async () => {
  const serverVersion = await fetchRunningServerVersion(testServer); // via Welcome or /api/config, per Step 1's finding
  const manifest = { ...baseManifestFixture, engines: { shadowcat: `^${serverVersion}` } };
  await writeModuleFixture(manifest);

  const res = await enableModule(testServer, manifest.id);
  expect(res.status).toBe(204);
});
```

(Match `fetchRunningServerVersion`/`writeModuleFixture`/`enableModule`/`baseManifestFixture` to the file's actual existing helpers — read the full file before finalizing.)

- [ ] **Step 3: Run test to verify it fails or passes as expected**

Run: `pnpm --filter @shadowcat/core test modules.e2e -- --run`
Expected: with the hardcoded `"^0.1.0"` fixture still in place elsewhere in the file, this new test should PASS if the running server version is still `0.1.x`, and the test's VALUE is proven by temporarily bumping `Cargo.toml`'s version locally and re-running — do this as a manual verification step, not a permanent code change, to prove the fix actually tracks the running version (then revert the manual version bump).

- [ ] **Step 4: Fix the hardcoded fixture at line 18**

Replace the hardcoded `engines: { shadowcat: "^0.1.0" }` with the dynamic `` `^${serverVersion}` `` (or a permissive `"*"` if Step 1 finds no clean way to fetch the running version in this test's setup phase — prefer the dynamic version if feasible, since it exercises `engine_compat_ok` meaningfully rather than trivially).

- [ ] **Step 5: Run the full e2e module test file**

Run: `pnpm --filter @shadowcat/core test modules.e2e -- --run`
Expected: all PASS

- [ ] **Step 6: Manual verification — bump `Cargo.toml` version locally, confirm the fixture still passes, then revert**

```bash
# Temporarily bump src/server/Cargo.toml's version field past 0.1, e.g. to 0.2.0
pnpm --filter @shadowcat/core test modules.e2e -- --run
# Confirm still PASS (proves the fix tracks the running version)
# Revert the Cargo.toml version bump — do NOT commit it
git checkout -- src/server/Cargo.toml
```

- [ ] **Step 7: Commit**

```bash
git add src/client/core/src/e2e/modules.e2e.test.ts
git commit -m "test(client/modules-e2e): fixture engines.shadowcat tracks the running server version

Was hardcoded to ^0.1.0, which only passed because CARGO_PKG_VERSION is
currently 0.1.x — a version bump would have failed the enable -> 204
assertion with a misleading 422."
```

---

## Task 16: Build-time guard for un-enumerated `svelte/*` subpath imports (D4)

**Files:**
- Create: `scripts/check-svelte-runtime-entries.mjs` (new build-time check script)
- Modify: `src/client/shell/vite.config.ts:32-41` (wire the check into the build, or into a CI step — decide in Step 1)
- Test: the script itself is the artifact under test; write a small fixture-based test for it

**Interfaces:**
- Produces: a script that scans `src/client/**/*.{ts,svelte}` and `src/modules/**/*.{ts,svelte}` for `from "svelte..."` import specifiers, compares each against `RUNTIME_ENTRIES`'s known specifier values (`svelte`, `svelte/internal/client`, `svelte/internal/disclose-version`, `svelte/reactivity` — confirmed at `vite.config.ts:32-41`), and exits non-zero listing any un-enumerated specifier found.

- [ ] **Step 1: Decide integration point**

Read `vite.config.ts:32-41` in full plus the surrounding build config to confirm exactly how `RUNTIME_ENTRIES` is structured (is it a flat array of specifier strings, or a map of chunk-name → specifier?). Decide: run this as a standalone Node script invoked from `package.json`'s `test`/`ci` script (simplest, matches "no existing build-time check script to extend" per research — this is net-new tooling, not a Vite plugin hook, since Vite plugins operate on the built output/module graph in a way that's harder to unit-test in isolation).

- [ ] **Step 2: Write the failing test for the script**

```javascript
// scripts/check-svelte-runtime-entries.test.mjs
import { test, expect } from "vitest";
import { findUnenumeratedSveltePaths } from "./check-svelte-runtime-entries.mjs";

test("flags an svelte/* import not present in RUNTIME_ENTRIES", () => {
  const fakeSourceFiles = {
    "fake/module.ts": `import { onMount } from "svelte";\nimport { fade } from "svelte/transition";\n`,
  };
  const knownEntries = ["svelte", "svelte/internal/client", "svelte/internal/disclose-version", "svelte/reactivity"];

  const flagged = findUnenumeratedSveltePaths(fakeSourceFiles, knownEntries);

  expect(flagged).toEqual([{ file: "fake/module.ts", specifier: "svelte/transition" }]);
});

test("does not flag an already-enumerated specifier", () => {
  const fakeSourceFiles = { "fake/module.ts": `import { onMount } from "svelte";\n` };
  const knownEntries = ["svelte", "svelte/internal/client", "svelte/internal/disclose-version", "svelte/reactivity"];

  expect(findUnenumeratedSveltePaths(fakeSourceFiles, knownEntries)).toEqual([]);
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `pnpm exec vitest run scripts/check-svelte-runtime-entries.test.mjs`
Expected: FAIL (module doesn't exist)

- [ ] **Step 4: Implement the script**

```javascript
// scripts/check-svelte-runtime-entries.mjs
import { readFileSync } from "node:fs";
import { globSync } from "node:fs"; // or the project's existing glob dependency — check package.json for an already-used glob lib (e.g. `fast-glob`, `tinyglobby`) and match it rather than adding a new dependency

const IMPORT_RE = /from\s+["'](svelte(?:\/[^"']+)?)["']/g;

export function findUnenumeratedSveltePaths(fileContentsByPath, knownEntries) {
  const flagged = [];
  for (const [file, content] of Object.entries(fileContentsByPath)) {
    for (const match of content.matchAll(IMPORT_RE)) {
      const specifier = match[1];
      if (!knownEntries.includes(specifier)) {
        flagged.push({ file, specifier });
      }
    }
  }
  return flagged;
}

// CLI entry point — only runs when invoked directly, not when imported by the test.
if (import.meta.url === `file://${process.argv[1]}`) {
  const RUNTIME_ENTRIES = ["svelte", "svelte/internal/client", "svelte/internal/disclose-version", "svelte/reactivity"]; // keep in sync with vite.config.ts — confirm exact source of truth in Step 1, ideally import it directly from vite.config.ts rather than duplicating the literal
  const files = globSync(["src/client/**/*.{ts,svelte}", "src/modules/**/*.{ts,svelte}"]);
  const contents = Object.fromEntries(files.map((f) => [f, readFileSync(f, "utf8")]));
  const flagged = findUnenumeratedSveltePaths(contents, RUNTIME_ENTRIES);
  if (flagged.length > 0) {
    console.error("Un-enumerated svelte/* imports found (add to RUNTIME_ENTRIES in vite.config.ts):");
    for (const { file, specifier } of flagged) console.error(`  ${file}: ${specifier}`);
    process.exit(1);
  }
}
```

(Confirm the project's existing glob dependency before finalizing the import — do not add a new one if `fast-glob`/`tinyglobby`/similar is already a devDependency.) **Best-long-term-shape:** import `RUNTIME_ENTRIES` directly from `vite.config.ts` rather than duplicating the literal list, so the check can never silently drift from the actual build config — confirm `vite.config.ts` exports `RUNTIME_ENTRIES` (add the export if it doesn't already) before finalizing this step.

- [ ] **Step 5: Run test to verify it passes**

Run: `pnpm exec vitest run scripts/check-svelte-runtime-entries.test.mjs`
Expected: PASS

- [ ] **Step 6: Wire into CI/package.json**

Add a script entry, e.g. `"check:svelte-runtime": "node scripts/check-svelte-runtime-entries.mjs"`, and confirm it's invoked in whatever CI workflow file runs the client gate (find `.github/workflows/*.yml` and add this step alongside the existing typecheck/lint steps).

- [ ] **Step 7: Run the check against the real current codebase**

Run: `node scripts/check-svelte-runtime-entries.mjs`
Expected: exit 0, no flagged specifiers (if any ARE flagged, this reveals a genuine pre-existing gap — fix by adding the missing specifier to `RUNTIME_ENTRIES`, not by weakening the check)

- [ ] **Step 8: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 9: Commit**

```bash
git add scripts/check-svelte-runtime-entries.mjs scripts/check-svelte-runtime-entries.test.mjs package.json .github/workflows/
git commit -m "build(client): fail the build on an svelte/* import missing from RUNTIME_ENTRIES

Protects the single-instance-runtime import-map invariant — an
un-enumerated subpath previously resolved silently to the app's own
bundled copy instead of the shared runtime chunk."
```

---

## Task 17: `ModuleRegistry.activate()` cleanup sweep on `register()` throw (D5)

**Files:**
- Modify: `src/client/core/src/modules.ts:112-142` (`activate`, the catch block at lines 136-140)
- Test: `src/client/core/src/modules.test.ts` (or wherever `ModuleRegistry` tests live — confirm exact filename)

**Interfaces:**
- Consumes: existing `unload(id, opts)` (`modules.ts:144-160`, already does the exact needed cleanup: `hooks.removeModule`, `services.removeModule`, `middleware.removeModule`, `contributions.removeModule`, sets `r.active = false`).
- No new method needed — `activate()`'s catch block calls the existing `unload(id)`.

- [ ] **Step 1: Write the failing test**

```typescript
test("activate() rolls back partial side effects when register() throws after contributing", async () => {
  const registry = new ModuleRegistry(testDeps());
  const throwingModule = {
    id: "throwing-module",
    async register(ctx) {
      ctx.contributions.contribute("some.surface", { component: FakeComponent });
      throw new Error("boom mid-register");
    },
  };
  registry.install(throwingModule);

  await registry.activate("throwing-module");

  expect(registry.get("throwing-module").active).toBe(false);
  expect(testDeps().contributions.listFor("some.surface")).toEqual([]); // the contribution must be rolled back, not left rendering
});
```

(Match `testDeps()`/`FakeComponent`/`registry.install`/`.get(...).active`/`contributions.listFor` to whatever `modules.ts`'s existing test file actually uses — read it first.)

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/core test modules -- --run`
Expected: FAIL — the contribution survives (the current catch block only logs)

- [ ] **Step 3: Add the cleanup call**

In `activate()`'s catch block (`modules.ts:136-140`), after the existing warn log, add:

```typescript
await this.unload(id);
```

(Since `r.active` is still `false` at this point — it's only set `true` after `register()` returns successfully, per line 135 — `unload`'s internal `activeDependentsOf(id)` check will be empty and its `unregister()` guard will correctly skip, per the research finding. This makes the call safe with no new method needed.)

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/core test modules -- --run`
Expected: PASS

- [ ] **Step 5: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 6: Commit**

```bash
git add src/client/core/src/modules.ts
git commit -m "fix(client/modules): roll back partial side effects when register() throws

activate()'s catch now calls the existing unload(id) to undo
hooks/services/contributions a module made before throwing mid-register.
Reachable once external async-register() modules exist (M13b+)."
```

---

## Task 18: Module-authoring guide — subpath-import caveat (D6)

**Files:**
- Modify: `docs/design/module-authoring.md` (the `### Which externals actually resolve at runtime` subsection, under `## Build config (Vite)`)

**Interfaces:** Documentation only.

- [ ] **Step 1: Add the caveat**

Under the `### Which externals actually resolve at runtime` subsection, add a short paragraph (present-tense, no narrative, per the project's doc-commenting rules) stating: the build-time import map only has exact-match package-root entries; importing a package SUBPATH (e.g. `@shadowcat/core/something`) is an unresolvable bare specifier and fails to load as a clean browser-level error — this is a documented completeness caveat of the current import-map shape, not a single-instance-runtime violation. Cross-reference Task 16's new `check-svelte-runtime-entries` guard if relevant (that guard covers `svelte/*` subpaths specifically; the package-subpath caveat here is broader and has no automated guard).

- [ ] **Step 2: Commit**

```bash
git add docs/design/module-authoring.md
git commit -m "docs(module-authoring): document the package-subpath import limitation

@shadowcat/core/something-style subpath imports are unresolvable bare
specifiers under the current exact-match import map."
```

---

## Task 19: Extract `VisualKindEditor.svelte` from `ActorsPanel.svelte` (E1a)

**Files:**
- Create: `src/modules/actors/src/VisualKindEditor.svelte`
- Modify: `src/modules/actors/src/ActorsPanel.svelte` (remove lines 57-69, 70-74, 76-86 partial, 88-92, 94-98 partial, 134-154, 156-162, 342-414 — replace with a component usage; keep the face-swap-palette code in place for Task 20)
- Test: `src/modules/actors/src/VisualKindEditor.test.ts` (new)

**Interfaces:**
- Produces: `VisualKindEditor` Svelte component, props: `{ conditionOptions: ConditionOption[], onBuild: (visual: Visual) => void }` (exact prop shape to be finalized against the real `AnimSourceState`/`FaceRowState`/`buildVisual`/`Visual` types found in `ActorsPanel.svelte:57-154` — read that range in full before writing the component). Owns internally: `AnimSourceState`, `FaceRowState`, `buildVisual()`, `faceRowComplete()`, `resetVisualEditor()`, the visual-kind `$state` (`visualKind`, `topAnim`, `faceRows`, `defaultFace`, `faceMapRows`).
- Consumes: whatever `conditionOptions` (line 94-98) currently derives from — `ActorsPanel` must still compute and pass this in as a prop, since it's condition-registry data external to the visual-kind concern.

- [ ] **Step 1: Read `ActorsPanel.svelte` lines 1-220 and 342-414 in full**

Confirm every type/function/state boundary before extracting — the research pass identified line ranges but a task-writer/implementer must read the actual current code (types may have changed since research) before moving anything.

- [ ] **Step 2: Write the failing test for the extracted component**

```typescript
// VisualKindEditor.test.ts
import { render, fireEvent } from "@testing-library/svelte";
import VisualKindEditor from "./VisualKindEditor.svelte";

test("buildVisual returns a complete image visual when kind=image and assetId is set", async () => {
  const onBuild = vi.fn();
  const { getByLabelText, getByText } = render(VisualKindEditor, { conditionOptions: [], onBuild });

  await fireEvent.change(getByLabelText(/visual kind/i), { target: { value: "image" } });
  await fireEvent.change(getByLabelText(/asset/i), { target: { value: "asset-123" } });
  await fireEvent.click(getByText(/build/i));

  expect(onBuild).toHaveBeenCalledWith({ kind: "image", assetId: "asset-123" });
});

test("faceRowComplete requires both a frame set and a sheet asset before the row counts as complete", () => {
  // Import faceRowComplete directly if it's exported from the component's <script module> block,
  // or test it indirectly via the rendered "incomplete row" warning UI — confirm which the
  // extracted component actually exposes (module-context export vs internal-only) before
  // finalizing this test's shape.
});
```

(This is illustrative — match prop names/exported functions/rendered labels to whatever Step 1's read confirms is the REAL shape of `buildVisual`/`faceRowComplete`/the visual-kind editor markup. The extraction must preserve exact existing behavior; do not invent new prop names not grounded in the current code.)

- [ ] **Step 3: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/actors test VisualKindEditor -- --run`
Expected: FAIL (component doesn't exist yet)

- [ ] **Step 4: Extract the component**

Move `AnimSourceState`, `newAnimSourceState()`, `animSourceToSource()`, `FaceRowState`, `faceRowToVisual()`, `faceRowComplete()`, the visual-editor `$state` block, `buildVisual()`, `resetVisualEditor()`, and the `assetPicker`/`animatedEditor` snippets + faces-editor markup (lines 342-414) into the new `VisualKindEditor.svelte` verbatim (no behavior change — pure move). Wire an `onBuild` callback prop that `ActorsPanel`'s `create()` (lines 183-217) calls into instead of calling `buildVisual()` directly.

- [ ] **Step 5: Update `ActorsPanel.svelte` to use the new component**

Replace the removed lines with `<VisualKindEditor {conditionOptions} onBuild={(visual) => { pendingVisual = visual; }} />` (or whatever wiring shape matches `create()`'s actual consumption of the built visual — confirm in Step 1's read).

- [ ] **Step 6: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/actors test VisualKindEditor -- --run`
Expected: PASS

- [ ] **Step 7: Run the full existing `ActorsPanel.test.ts` suite — must be unaffected**

Run: `pnpm --filter @shadowcat/actors test ActorsPanel -- --run`
Expected: PASS, unchanged assertions (this extraction must be behavior-preserving; any assertion needing updated selectors because markup moved to a child component is acceptable, but the underlying create-flow behavior must be identical)

- [ ] **Step 8: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 9: Commit**

```bash
git add src/modules/actors/src/VisualKindEditor.svelte src/modules/actors/src/VisualKindEditor.test.ts src/modules/actors/src/ActorsPanel.svelte src/modules/actors/src/ActorsPanel.test.ts
git commit -m "refactor(client/actors): extract VisualKindEditor.svelte from ActorsPanel

Owns AnimSourceState/FaceRowState/buildVisual/faceRowComplete and the
image/faces/animated editor markup. Pure extraction, no behavior change.
First step of de-god-componenting ActorsPanel before M10j adds more."
```

---

## Task 20: Extract face-swap palette + add the missing override-precedence test (E1b)

**Files:**
- Create: `src/modules/actors/src/FaceSwapPalette.svelte`
- Modify: `src/modules/actors/src/ActorsPanel.svelte` (remove lines 100-132, 222-229 per the pre-extraction line numbers; re-confirm exact current lines after Task 19's edit shifted them)
- Modify: `src/modules/actors/src/ActorsPanel.svelte` (fold `faceRowComplete`'s per-row check and `buildVisual`'s inline top-level check into a shared `animSourceComplete(anim)` helper — this closes the WS-E1 DRY item from the spec)
- Test: `src/client/core/src/actor.test.ts` (new test for the override-precedence gap) + `src/modules/actors/src/FaceSwapPalette.test.ts` (new)

**Interfaces:**
- Produces: `FaceSwapPalette` component, props: `{ tokenId: string }` (reads `selectedFaceToken`/`selectedFaceNames`/`currentFace`/`swapFace` internally via `resolveTokenActor` from `@shadowcat/core`, matching the existing logic at `ActorsPanel.svelte:100-132`).
- Produces: `animSourceComplete(anim: AnimSourceState): boolean` in `VisualKindEditor.svelte` (or a shared location both `VisualKindEditor` and `FaceSwapPalette` can import from, if the face-swap palette also needs a completeness check — confirm during extraction whether it does).

- [ ] **Step 1: Write the failing override-precedence test in `actor.test.ts`**

```typescript
test("resolveTokenActor and resolveTokenVisual agree when a token has both a faces-union visual override AND an active face-swap", () => {
  const store = testDocumentStore();
  const actor = testActor(store, { visual: { kind: "image", assetId: "base-asset" } });
  const token = testInstancedToken(store, actor, {
    overrides: { visual: { kind: "faces", faces: [{ name: "smile", assetId: "smile-asset" }, { name: "frown", assetId: "frown-asset" }], default: "smile" } },
    engine: { face: "frown" }, // active manual face-swap
  });

  const eff = resolveTokenActor(token, store);
  expect(eff.visual).toEqual({ kind: "faces", faces: expect.any(Array), default: "smile" });

  const renderVisual = resolveTokenVisual(token, store);
  expect(renderVisual.currentFrame).toBe("frown-asset"); // the active face-swap wins over the union's own default

  // The face-swap palette's own selectedFaceNames must read the SAME
  // projected override, not a second independent resolution.
  const selected = selectedFaceNamesFor(token, store); // exact helper name TBD — see Step 2
  expect(selected).toContain("frown");
});
```

(This test proves the invariant described in the spec's WS-E1 item: "both `resolveTokenVisual` and the face-swap palette's `selectedFaceNames` read the same override-projected `resolveTokenActor` output." `testDocumentStore`/`testActor`/`testInstancedToken` must match `actor.test.ts`'s existing helpers — read that file first.)

- [ ] **Step 2: Extract `selectedFaceNames`'s logic into an exported, directly-testable function**

Read `ActorsPanel.svelte:108-118` (`selectedFaceNames`, a Svelte `$derived`). Since Step 1's test needs to call this logic directly (not through component rendering), extract the underlying logic into a plain exported function in `src/client/core/src/actor.ts` (e.g. `selectedFaceNamesFor(token, store): string[]`), and have both `ActorsPanel`'s (soon `FaceSwapPalette`'s) `$derived` and this new exported function share the same implementation (the `$derived` becomes a one-line wrapper calling the exported function).

- [ ] **Step 3: Run the test to verify it fails (or passes, confirming/denying the invariant)**

Run: `pnpm --filter @shadowcat/core test actor -- --run`
Expected: this test is a verification test per the spec ("verified correct by code trace... but nothing pins the behavior") — it may PASS immediately once `selectedFaceNamesFor` is correctly extracted (proving the existing logic was already correct), which is the desired outcome. If it FAILS, this reveals a real bug the spec's code-trace missed — fix `resolveTokenActor`/`resolveTokenVisual`/`selectedFaceNamesFor` until it passes, and flag the discrepancy from the spec's prior trace explicitly in the commit message.

- [ ] **Step 4: Extract `FaceSwapPalette.svelte`**

Move `selectedFaceToken`, `selectedFaceNames` (now calling the Step 2 exported function), `currentFace()`, `swapFace()` (preserving its existing raw-`old`-read OCC convention exactly — do not regress to a hardcoded `old: null`, the exact class of bug `phase1-bugs-todo-sweep` fixed elsewhere), and the palette markup into the new component. Wire `ActorsPanel` to render `<FaceSwapPalette tokenId={selectedTokenId} />`.

- [ ] **Step 5: Fold the DRY completeness-check duplication**

In `VisualKindEditor.svelte` (Task 19's output), extract `faceRowComplete`'s per-row check and `buildVisual`'s inline top-level-animated-kind completeness check into a single shared `animSourceComplete(anim: AnimSourceState): boolean` (both currently re-express "frames-nonempty AND sheet-asset-present"). Update both call sites to use it.

- [ ] **Step 6: Run the full test suite**

Run: `pnpm --filter @shadowcat/actors test -- --run && pnpm --filter @shadowcat/core test actor -- --run`
Expected: all PASS, including pre-existing `ActorsPanel.test.ts` assertions (behavior-preserving extraction) and the new override-precedence + `FaceSwapPalette` tests

- [ ] **Step 7: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 8: Commit**

```bash
git add src/modules/actors/src/FaceSwapPalette.svelte src/modules/actors/src/FaceSwapPalette.test.ts src/modules/actors/src/ActorsPanel.svelte src/client/core/src/actor.ts src/client/core/src/actor.test.ts
git commit -m "refactor(client/actors): extract FaceSwapPalette, add faces+face-swap override test

Extracts selectedFaceToken/selectedFaceNames/currentFace/swapFace into
FaceSwapPalette.svelte, sharing the same resolveTokenActor projection as
resolveTokenVisual (no second resolution path). Pins the previously
untested invariant: a linked token with a faces-union overrides.visual
combined with an active system.face face-swap resolves consistently
across both the render path and the palette. Folds faceRowComplete's
duplicate completeness check into animSourceComplete."
```

---

## Task 21: `buildTokenFromActor` w/h — document as the dangling-link fallback only (E3)

**Files:**
- Modify: `src/client/core/src/scene-docs.ts:351-373` (`buildTokenFromActor`, line 361)
- Test: `src/client/core/src/scene-docs.test.ts` (or wherever `buildTokenFromActor` is tested — confirm filename)

**Interfaces:** No signature/behavior change — the fix is a doc comment pinning the design decision (best-long-term-shape chosen: keep the explicit documented fallback seeding, do not add a second lazy-derivation path).

- [ ] **Step 1: Write a pinning test**

```typescript
test("buildTokenFromActor seeds w/h=cellSize solely as the dangling-link fallback (not consumed on the actor-backed render path)", () => {
  const actor = testActor({ size: 2 /* grid units */ });
  const token = buildTokenFromActor("world-1", "scene-1", actor, "linked", { x: 0, y: 0 }, 50 /* cellSize */);

  expect(token.engine.w).toBe(50);
  expect(token.engine.h).toBe(50);

  // Confirm the actor-backed render path does NOT use these seeded values —
  // resolveTokenBox derives size from EffectiveActor.size x cellSize instead.
  const store = testDocumentStore(actor, token);
  const box = resolveTokenBox(token, store, resolveTokenActor(token, store));
  expect(box.w).toBe(2 * 50); // from actor.size, NOT the seeded token.engine.w=50

  // Now simulate a dangling link (actor deleted) — the seeded w/h becomes the real fallback.
  store.remove(actor.id);
  const danglingBox = resolveTokenBox(token, store, undefined);
  expect(danglingBox.w).toBe(50); // now the seeded engine.w is what's actually used
});
```

(Match `testActor`/`testDocumentStore`/`resolveTokenBox`/`resolveTokenActor` to `scene-docs.test.ts`'s or `actor.test.ts`'s existing helpers.)

- [ ] **Step 2: Run test to verify it passes on current code**

Run: `pnpm --filter @shadowcat/core test scene-docs -- --run`
Expected: PASS (this is a pinning test, not a bugfix — the current behavior is already correct; the test's purpose is to lock it in and document intent via a named test)

- [ ] **Step 3: Add the doc comment**

At `scene-docs.ts:361`, add a comment above the `w: cellSize, h: cellSize` line:

```typescript
// Seeded solely as the dangling-link fallback: resolveTokenBox (actor.ts)
// uses this ONLY when the linked/instanced actor is missing (actor.ts's
// missing-actor branch, `eng?.w ?? 0`). The actor-backed render path never
// reads these — size resolves through EffectiveActor.size x grid-cell.
// Decided (Phase-1 cleanup burndown 2026-07-19): keep as an explicit,
// documented fallback rather than deriving it lazily from the token's
// last-known actor size — avoids a second size-derivation path.
```

- [ ] **Step 4: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/scene-docs.ts src/client/core/src/scene-docs.test.ts
git commit -m "docs(client/actors): pin buildTokenFromActor w/h as the documented dangling-link fallback

No behavior change. Resolves the M10d-final-review open question: keep
the explicit seeded fallback rather than adding a second lazy-derivation
path."
```

---

## Task 22: Shared WAI-ARIA menu primitive (E2)

**Files:**
- Create: `src/client/ui-kit/src/MenuKeyboard.ts` (or `.svelte.ts` if it needs Svelte runes — decide in Step 1)
- Modify: `src/modules/topbar/src/LauncherMenu.svelte` (lines 35-71 replaced with the shared primitive)
- Modify: `src/modules/panels/src/PanelMenu.svelte` (lines 35-74 replaced with the shared primitive)
- Test: `src/client/ui-kit/src/MenuKeyboard.test.ts` (new)

**Interfaces:**
- Produces: a function/class encapsulating `focusItem(index)` (wraparound `((index % n) + n) % n`) and a keydown handler covering ArrowDown/ArrowUp/Home/End/Escape/Tab, parameterized by an `itemEls: HTMLElement[]` accessor and an `onClose: () => void` callback (LauncherMenu's Escape calls its own `closeMenu()`, PanelMenu's calls `onClose()` — the shared primitive takes `onClose` as a required param, and `LauncherMenu` passes its own `closeMenu` as that callback, unifying the two call shapes).
- Consumes: nothing external — pure keyboard-event logic, no dockview dependency (per the research finding: `PanelMenu` is explicitly framework/dockview-free, and the primitive must stay that way).

- [ ] **Step 1: Read both source files in full**

Read `LauncherMenu.svelte` (all 210 lines) and `PanelMenu.svelte` (all 122 lines) to confirm the exact current behavior byte-for-byte before extracting — research confirmed `focusItem` is near-identical and the switch-case bodies match except Escape's target (`closeMenu()` vs `onClose()`). Decide the exact shape: a plain function factory (`createMenuKeyboard(getItemEls, onClose)`) is simpler than a class and matches the existing `sizeClass.svelte.ts`/`i18n.svelte.ts` factory-function convention in ui-kit — prefer that shape unless Svelte reactivity requirements (owning `$state` for focused index) make a class/rune-object necessary. If reactive state is needed, name the file `MenuKeyboard.svelte.ts`.

- [ ] **Step 2: Write the failing test**

```typescript
import { createMenuKeyboard } from "./MenuKeyboard.svelte.ts";

test("ArrowDown moves focus to the next item, wrapping past the last", () => {
  const items = [mockEl(), mockEl(), mockEl()];
  const onClose = vi.fn();
  const menu = createMenuKeyboard(() => items, onClose);

  menu.focusItem(2);
  menu.handleKeydown(arrowDownEvent());

  expect(document.activeElement).toBe(items[0].el); // wraps from index 2 to 0
});

test("Escape calls onClose", () => {
  const items = [mockEl()];
  const onClose = vi.fn();
  const menu = createMenuKeyboard(() => items, onClose);

  menu.handleKeydown(escapeEvent());

  expect(onClose).toHaveBeenCalledOnce();
});

test("Home focuses the first item, End focuses the last", () => {
  const items = [mockEl(), mockEl(), mockEl()];
  const menu = createMenuKeyboard(() => items, vi.fn());

  menu.handleKeydown(homeEvent());
  expect(document.activeElement).toBe(items[0].el);

  menu.handleKeydown(endEvent());
  expect(document.activeElement).toBe(items[2].el);
});
```

(`mockEl`/`arrowDownEvent`/`escapeEvent`/`homeEvent`/`endEvent` are small local test helpers — write them inline in the test file, matching whatever DOM-event-construction convention `LauncherMenu.test.ts`/`PanelMenu.test.ts` — if either exists — already uses.)

- [ ] **Step 3: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/ui-kit test MenuKeyboard -- --run`
Expected: FAIL (module doesn't exist)

- [ ] **Step 4: Implement `MenuKeyboard.svelte.ts`**

Move `focusItem` and the switch-case keydown handler verbatim from `LauncherMenu.svelte`/`PanelMenu.svelte` (they're near-byte-identical per research) into the new shared module, parameterized by `getItemEls: () => HTMLElement[]` and `onClose: () => void`.

- [ ] **Step 5: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/ui-kit test MenuKeyboard -- --run`
Expected: PASS

- [ ] **Step 6: Refactor `LauncherMenu.svelte` onto the primitive**

Replace lines 35-71 with a `createMenuKeyboard(() => itemEls, closeMenu)` instance; keep the trigger/open-state logic (lines 21-30, 72-86) unchanged in `LauncherMenu` itself (research confirmed this part is NOT shared).

- [ ] **Step 7: Refactor `PanelMenu.svelte` onto the primitive**

Replace lines 35-74 with `createMenuKeyboard(() => itemEls, onClose)`.

- [ ] **Step 8: Run both components' existing test suites**

Run: `pnpm --filter @shadowcat/topbar test LauncherMenu -- --run && pnpm --filter @shadowcat/panels test PanelMenu -- --run`
Expected: PASS, unchanged assertions (behavior-preserving refactor)

- [ ] **Step 9: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 10: Commit**

```bash
git add src/client/ui-kit/src/MenuKeyboard.svelte.ts src/client/ui-kit/src/MenuKeyboard.test.ts src/modules/topbar/src/LauncherMenu.svelte src/modules/panels/src/PanelMenu.svelte
git commit -m "refactor(client/ui-kit): extract shared WAI-ARIA menu keyboard primitive

LauncherMenu and PanelMenu both had near-identical arrow/Home/End/Escape/
Tab focus-cycling logic; now share MenuKeyboard.svelte.ts. Dockview-free
by construction (PanelMenu's existing constraint). Trigger/open-state
logic stays per-component (not shared)."
```

---

## Task 23: `ToolRail` coarse-pointer input sizing (F1)

**Files:**
- Modify: `src/modules/scene-tools/src/ToolRail.svelte:171-174`
- Test: `src/modules/scene-tools/src/ToolRail.test.ts` (or a new visual-regression-style assertion if the project has one; otherwise a computed-style assertion)

**Interfaces:** No behavior change — CSS-only.

- [ ] **Step 1: Write the failing test**

```typescript
test("select/input controls get a 44px coarse-pointer min-height", () => {
  const { container } = render(ToolRail, { /* props */ });
  const select = container.querySelector(".controls select");
  // jsdom doesn't evaluate @media (pointer: coarse), so assert the CSS rule's
  // presence in the component's compiled styles instead, or use a matchMedia
  // mock forcing pointer:coarse and checking getComputedStyle — confirm which
  // approach ToolRail.test.ts (or a sibling component's coarse-pointer test,
  // e.g. SystemTreeEditor.test.ts) already uses, and match it exactly.
});
```

(This test's exact mechanics depend entirely on the project's existing coarse-pointer testing convention — read `SystemTreeEditor.test.ts` or `MergeConflictModal.test.ts` first, since both already have `@media (pointer: coarse)` rules per research; mirror however THEY test it, if they test it at all. If neither has a test for their existing coarse-pointer CSS, this task's test may reasonably be a plain CSS-source assertion: `expect(ToolRail_scss_source).toMatch(/@media \(pointer: coarse\)/)`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/scene-tools test ToolRail -- --run`
Expected: FAIL

- [ ] **Step 3: Add the coarse-pointer rule**

In `ToolRail.svelte:171-174`, extend:

```scss
.controls select,
.controls input {
  min-height: 32px;

  @media (pointer: coarse) {
    min-height: 44px;
  }
}
```

(Matching the exact `@media (pointer: coarse) { min-height: 44px; }` convention from `SystemTreeEditor.svelte:122` and `MergeConflictModal.svelte:103-105`.)

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/scene-tools test ToolRail -- --run`
Expected: PASS

- [ ] **Step 5: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 6: Commit**

```bash
git add src/modules/scene-tools/src/ToolRail.svelte
git commit -m "fix(client/scene-tools): 44px coarse-pointer sizing on ToolRail select/input controls"
```

---

## Task 24: Shared ui-kit input-height coarse-pointer token (F2)

**Files:**
- Modify: `src/client/shell/src/styles/_primitives.scss` (add `--input-height-coarse` token, or a `%input-touch` placeholder — decide in Step 1)
- Modify: `src/client/ui-kit/src/SystemTreeEditor.svelte` (lines 85, 88, 91 — apply the shared rule to its text/number/checkbox inputs)
- Modify: `src/modules/game-settings/src/GameSettingsPanel.svelte` (its ~10 inputs — apply the shared rule)
- Test: a CSS-source assertion or computed-style test matching whatever convention Task 23 established

**Interfaces:**
- Produces: a shared SCSS token/placeholder (e.g. `--input-height-coarse: 44px;` in `_primitives.scss`, consumed via `@media (pointer: coarse) { min-height: var(--input-height-coarse); }` in each component) OR a `%input-touch` placeholder selector (`@extend %input-touch;`) — confirm the existing `@use` direction between the `ui-kit` and `shell` packages in Step 1 before choosing, since ui-kit consuming a shell-owned SCSS file may not be the intended dependency direction.

- [ ] **Step 1: Confirm the `ui-kit` ↔ `shell` SCSS dependency direction**

Grep `src/client/ui-kit/src/**/*.svelte` for existing `@use` imports of anything from `@shadowcat/shell` or `src/client/shell/src/styles/`. If ui-kit already imports shell's `_primitives.scss`/`_semantic.scss` elsewhere, add the new token there and it's a safe pattern to extend. If ui-kit currently has NO dependency on shell's styles (likely, given ui-kit is meant to be reusable/framework-level per M8.5a), instead create a new shared partial `src/client/ui-kit/src/styles/_touch.scss` owned by ui-kit itself, and have `_primitives.scss` (shell-side) additionally import/re-export it if shell components need the same token — this keeps the dependency direction consumer-appropriate (ui-kit doesn't depend on shell).

- [ ] **Step 2: Write the failing test**

```typescript
test("SystemTreeEditor's text/number/checkbox inputs carry the coarse-pointer touch-sizing rule", () => {
  // Match whatever assertion mechanics Task 23 established (CSS-source
  // match or computed-style-with-matchMedia-mock) — apply the SAME
  // mechanism here for consistency.
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/ui-kit test SystemTreeEditor -- --run`
Expected: FAIL

- [ ] **Step 4: Add the shared token and apply it**

Add the token/placeholder decided in Step 1. Apply `@media (pointer: coarse) { min-height: var(--input-height-coarse); }` (or `@extend %input-touch;`) to `SystemTreeEditor.svelte`'s three inputs (lines 85, 88, 91) and every `<input>` in `GameSettingsPanel.svelte` (~10 elements per research — checkbox/number/color/text at lines 161, 183, 204, 235, 288, 456, 471, 488, 498, 509, 515).

- [ ] **Step 5: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/ui-kit test SystemTreeEditor -- --run`
Expected: PASS

- [ ] **Step 6: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 7: Commit**

```bash
git add src/client/ui-kit/src/styles/ src/client/ui-kit/src/SystemTreeEditor.svelte src/modules/game-settings/src/GameSettingsPanel.svelte src/client/shell/src/styles/_primitives.scss
git commit -m "fix(client/ui-kit): shared coarse-pointer input-height token, applied ui-kit-wide

Closes the systemic ui-kit gap: only buttons had @media (pointer: coarse)
sizing before. Replaces the per-component media-query duplication this
would otherwise require."
```

---

## Task 25: Floating-panel live re-drag/resize sync (F3)

**Files:**
- Modify: `src/modules/panels/src/engine/dockview.ts` (near lines 842-850, the floating-panel creation block)
- Test: `src/modules/panels/src/engine/dockview.test.ts`

**Interfaces:**
- Produces: a per-floating-panel subscription (added at the same point floating creation happens, L842-846) mirroring the existing per-group `onDidDimensionsChange` pattern at L770-782, emitting a `resizeFloating`/`moveFloating`-shaped `LayoutOp` (confirm exact op name from the `LayoutOp` union type — grep it) when an already-floating panel's position/size changes via a live drag/resize, guarded by the existing `#applying` convention (L877-879) to skip self-caused churn.

- [ ] **Step 1: Read the `LayoutOp` union and confirm the right op shape**

Grep `LayoutOp` type definition (likely in `src/modules/panels/src/layout-tree.ts` or similar) to confirm whether a floating-panel move/resize already has a dedicated op variant, or whether `resizeZone`/`resizeGroup` (the existing group-resize ops) need a floating-specific sibling added.

- [ ] **Step 2: Write the failing test**

```typescript
test("a live drag of an already-floating panel emits a LayoutOp syncing its new Rect", () => {
  const engine = new DockviewEngine(testAccessor());
  engine.apply(layoutWithFloatingPanel("panel-1", { x: 10, y: 10, w: 200, h: 150 }));

  // Simulate dockview firing its own onDidDimensionsChange for the floating panel.
  const panel = engine.getApi().getPanel("panel-1");
  panel.api._fireDimensionsChanged({ x: 50, y: 60, width: 220, height: 160 }); // exact dockview-core test-double API — confirm from existing group-resize tests in this file

  expect(emittedOps).toContainEqual({ type: "resizeFloating", id: "panel-1", rect: { x: 50, y: 60, w: 220, h: 160 } });
});
```

(Match `testAccessor`/`layoutWithFloatingPanel`/`_fireDimensionsChanged`/`emittedOps` to `dockview.test.ts`'s existing test-double conventions for the group-resize tests — read those tests first, since F3 mirrors that exact mechanism for floating panels instead of groups.)

- [ ] **Step 3: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/panels test dockview -- --run`
Expected: FAIL (no subscription exists yet)

- [ ] **Step 4: Wire the subscription**

At the floating-panel creation site (L842-846), after `api.addPanel({..., floating: {...}})`, add a subscription on that panel's `api.onDidDimensionsChange` (and/or a position-change event if dockview exposes one separately — confirm from Step 1's API investigation), guarded by `#applying`, emitting the `resizeFloating`/equivalent op. Dispose the subscription on panel removal (mirror the group-resize disposal pattern at L770-782).

- [ ] **Step 5: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/panels test dockview -- --run`
Expected: PASS

- [ ] **Step 6: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 7: Commit**

```bash
git add src/modules/panels/src/engine/dockview.ts src/modules/panels/src/engine/dockview.test.ts
git commit -m "fix(client/panels): sync live re-drag/resize of an already-floating panel

Mirrors the existing per-group onDidDimensionsChange pattern. Previously
only floating-panel CREATION was tracked; a live drag/resize of an
existing floating window silently drifted from the persisted Rect."
```

---

## Task 26: Translate whole-group drag transfers into per-tab dock ops (F4)

**Files:**
- Modify: `src/modules/panels/src/engine/dockview.ts` (`#toDropSite`, L681-696; `#handleWillDrop`, L653-661)
- Test: `src/modules/panels/src/engine/dockview.test.ts`

**Interfaces:**
- Modifies: `#toDropSite`'s current `if (!id) return null;` (L689) veto path — when `data.panelId === null` (a whole-group drag, per `PanelTransfer`'s shape) but the group itself has a resolvable tab list, produce a `DropSite` per tab (or a new multi-tab drop-site shape) instead of `null`.
- Produces: `#handleWillDrop` translates a whole-group transfer into a sequence of per-tab `dock` `LayoutOp`s (one per tab in the dragged group, preserving their relative order) instead of vetoing.

- [ ] **Step 1: Read the full `PanelTransfer` shape and the existing per-tab dock-op emission code**

Confirm exactly what data a whole-group drag payload carries (source group's tab list, target drop position) by reading dockview-core's `PanelTransfer` type and `#toDropSite`/`#handleWillDrop` in full. Confirm the existing single-tab dock-op emission code (used for a normal single-panel drag) so the group case can reuse it per-tab.

- [ ] **Step 2: Write the failing test**

```typescript
test("a whole-group drag translates into per-tab dock ops instead of being vetoed", () => {
  const engine = new DockviewEngine(testAccessor());
  engine.apply(layoutWithGroup("group-a", ["tab-1", "tab-2", "tab-3"]));

  const transfer = { panelId: null, groupId: "group-a" }; // whole-group transfer payload shape — confirm exact fields from Step 1
  const dropEvent = testDropEvent(transfer, /* target: top-edge of the stage */);

  engine.handleWillDrop(dropEvent);

  expect(emittedOps).toEqual([
    { type: "dock", id: "tab-1", zone: "top", index: 0 },
    { type: "dock", id: "tab-2", zone: "top", index: 1 },
    { type: "dock", id: "tab-3", zone: "top", index: 2 },
  ]);
  expect(dropEvent.defaultPrevented).toBe(true); // still must preventDefault — dockview's own machinery must not also act
});

test("a whole-group drag onto an unclassifiable target still vetoes (fail-closed unchanged for genuinely unclassifiable drops)", () => {
  // The existing regression this area guards: an untranslated group
  // transfer previously fell through WITHOUT vetoing, letting a group
  // land above the stage unexpectedly (per the Task 6 buddy-check finding
  // already fixed for single-panel drops) — confirm the SAME fail-closed
  // discipline holds for the newly-translated group case too, for any
  // genuinely unclassifiable drop target.
});
```

(Match `testAccessor`/`layoutWithGroup`/`testDropEvent`/`emittedOps` to the file's existing test-double conventions.)

- [ ] **Step 3: Run tests to verify the first fails**

Run: `pnpm --filter @shadowcat/panels test dockview -- --run`
Expected: `a whole-group drag translates into per-tab dock ops instead of being vetoed` FAILS (current code returns `null`/vetoes); the fail-closed test should PASS already (unclassifiable targets already veto correctly).

- [ ] **Step 4: Implement the translation**

In `#toDropSite`, when `data.panelId === null`, resolve the group's tab list and the target drop zone/index, and return a new multi-tab-capable `DropSite` (or have `#handleWillDrop` directly loop over the group's tabs and emit one `dock` op per tab at consecutive indices, calling `event.preventDefault()` once for the whole transfer per the existing "classify → veto or redispatch; dockview never self-mutates from drops" contract established in the Task 6 fix-round-3 commit).

- [ ] **Step 5: Run tests to verify both pass**

Run: `pnpm --filter @shadowcat/panels test dockview -- --run`
Expected: PASS

- [ ] **Step 6: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 7: Commit**

```bash
git add src/modules/panels/src/engine/dockview.ts src/modules/panels/src/engine/dockview.test.ts
git commit -m "feat(client/panels): translate whole-group drag transfers into per-tab dock ops

Was vetoed outright in v1. Re-enables the group-drag gesture while
preserving the classify-or-veto contract (dockview never self-mutates
from drops)."
```

---

## Task 27: Narrow `PanelHost`'s `PanelsBridgeLike` cast (F5)

**Files:**
- Modify: `src/modules/panels/src/PanelHost.svelte:99` (and the `PanelsBridgeLike` import at line 7)
- Test: `src/modules/panels/src/PanelHost.test.ts`

**Interfaces:**
- Produces: a runtime `typeof bridge.bind === "function"` guard at the cast site (chosen over a narrower `AppContext.panels` type, since the latter would require a wider `AppContext` type change outside this task's scope — best-long-term-shape within this task's boundary: fail loudly at the one composition-root binding site if the convention is ever violated, rather than silently trusting an `as unknown as` cast).

- [ ] **Step 1: Write the failing test**

```typescript
test("PanelHost throws a clear error if ctx.panels doesn't look like a PanelsBridgeLike", () => {
  const badCtx = { panels: { notBind: true } };
  expect(() => render(PanelHost, { context: mapWithAppContext(badCtx) })).toThrow(/PanelsBridgeLike/);
});

test("PanelHost mounts normally when ctx.panels has a bind method", () => {
  const goodCtx = { panels: { bind: vi.fn() } };
  expect(() => render(PanelHost, { context: mapWithAppContext(goodCtx) })).not.toThrow();
});
```

(Match `mapWithAppContext` to `PanelHost.test.ts`'s existing context-setup helper.)

- [ ] **Step 2: Run tests to verify the first fails**

Run: `pnpm --filter @shadowcat/panels test PanelHost -- --run`
Expected: `throws a clear error` FAILS (current code has no runtime guard, would either silently misbehave or throw an unrelated error deeper in)

- [ ] **Step 3: Add the runtime guard**

At `PanelHost.svelte:99`, replace `ctx.panels as unknown as PanelsBridgeLike` with:

```typescript
if (typeof ctx.panels?.bind !== "function") {
  throw new Error("PanelHost expects AppContext.panels to be a PanelsBridgeLike (missing .bind) — check the composition-root binding in Table.svelte");
}
const bridge: PanelsBridgeLike = ctx.panels;
```

- [ ] **Step 4: Run tests to verify both pass**

Run: `pnpm --filter @shadowcat/panels test PanelHost -- --run`
Expected: PASS

- [ ] **Step 5: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 6: Commit**

```bash
git add src/modules/panels/src/PanelHost.svelte src/modules/panels/src/PanelHost.test.ts
git commit -m "fix(client/panels): runtime-guard PanelHost's PanelsBridgeLike cast

Was an unchecked 'as unknown as' resting on the Table.svelte
composition-root convention. Now fails loudly with a clear message if
that convention is ever violated."
```

---

## Task 28: `DockChips` i18n fallback for an unknown panel id (F6)

**Files:**
- Modify: `src/modules/panels/src/DockChips.svelte:30-33`
- Test: `src/modules/panels/src/DockChips.test.ts`

**Interfaces:** No signature change. Per the research flag: the TODO's wording ("mirror `describeOp`'s aria-live fallback") is ambiguous — `describeOp` ALSO falls back to the raw untranslated `id`, not a real i18n string. **Decision (best-long-term-shape, resolvable without a user round-trip):** give `DockChips` a REAL i18n fallback string (`t("panels.unknownPanel", { id })`) rather than replicating `describeOp`'s own untranslated fallback — a user-facing chip label showing a raw internal id is worse UX than `describeOp`'s aria-live text (which is a less-visible accessibility string). This task additionally improves `describeOp` to match, since leaving it as the sole remaining raw-id fallback after this fix would be an inconsistency introduced BY this task, not a pre-existing one to leave alone.

- [ ] **Step 1: Add the i18n key**

Find the i18n locale file (`en` locale, per M7d) and add `"panels.unknownPanel": "Unknown panel ({id})"` (or match whatever interpolation syntax the existing `typesafe-i18n` setup uses — confirm from any existing parameterized key).

- [ ] **Step 2: Write the failing test**

```typescript
test("DockChips shows a translated fallback for an id missing from metaMap, not the raw id", () => {
  const { getByRole } = render(DockChips, { ids: ["unregistered-id"], metaMap: new Map() });
  const chip = getByRole("button", { name: /unknown panel/i });
  expect(chip.textContent).not.toContain("unregistered-id");
});
```

(Match `render`/prop shape to `DockChips.test.ts`'s existing conventions.)

- [ ] **Step 3: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/panels test DockChips -- --run`
Expected: FAIL (raw id currently shown)

- [ ] **Step 4: Fix `DockChips.svelte`**

Replace all three raw-`id` fallback sites (L30-33) with `t("panels.unknownPanel", { id })`.

- [ ] **Step 5: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/panels test DockChips -- --run`
Expected: PASS

- [ ] **Step 6: Apply the same fix to `PanelHost.describeOp` for consistency**

Update `describeOp`'s `label()` helper (L49-53) to also use `t("panels.unknownPanel", { id })` instead of the raw `id`, so this task doesn't leave the two fallback sites inconsistent with each other.

- [ ] **Step 7: Run `PanelHost`'s test suite**

Run: `pnpm --filter @shadowcat/panels test PanelHost -- --run`
Expected: PASS (update any existing assertion that expected the old raw-id aria-live text)

- [ ] **Step 8: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 9: Commit**

```bash
git add src/modules/panels/src/DockChips.svelte src/modules/panels/src/PanelHost.svelte src/client/shell/src/i18n/en.ts
git commit -m "fix(client/panels): real i18n fallback for an unregistered panel id

DockChips and PanelHost.describeOp both showed the raw internal id when
metaMap lacked an entry; both now use a translated 'unknown panel'
fallback."
```

---

## Task 29 [stretch, optional]: Content-independent group-identity diff (F7)

This item is explicitly `[stretch]` per the spec — include only if it lands cheaply; skip without blocking the plan if it proves nontrivial once investigated.

**Files:**
- Modify: `src/modules/panels/src/engine/dockview.ts` (`groupIdFor`, L95-97)
- Test: `src/modules/panels/src/engine/dockview.test.ts`

**Interfaces:**
- Modifies: `groupIdFor(zone, index, tabs)` — currently `\`sc-group:${tabs[0] ?? \`${zone}:${index}:empty\`}\``, keyed on the first tab's id. A content-independent scheme needs a STABLE identity that survives tab reordering/refilling without needing to persist a new id field on the layout tree itself (which would be a larger schema change, likely out of this task's cheap-stretch scope).

- [ ] **Step 1: Investigate feasibility within a `[stretch]` budget**

Check whether the layout tree already carries a stable positional/structural group id anywhere (e.g. from the persisted `panelLayout` shape) that `groupIdFor` could use instead of deriving one from `tabs[0]`. If yes, this is a small swap — proceed. If a stable id would require a new persisted field (a layout-tree schema migration), STOP — this exceeds "lands cheaply," mark this task SKIPPED in the plan tracking and leave the existing doc-commented tradeoff (L88-94) in place, since it's already accepted as future work.

- [ ] **Step 2 (if feasible): Write the failing test**

```typescript
test("reordering a group's first tab does not tear down and recreate the dockview group", () => {
  const engine = new DockviewEngine(testAccessor());
  engine.apply(layoutWithGroup("group-a", ["tab-1", "tab-2"]));
  const groupBefore = engine.getApi().getGroup(engine.groupIdFor(/* ... */));

  engine.apply(layoutWithGroup("group-a", ["tab-2", "tab-1"])); // first tab reordered

  const groupAfter = engine.getApi().getGroup(engine.groupIdFor(/* ... */));
  expect(groupAfter).toBe(groupBefore); // same object identity — patched in place, not recreated
});
```

- [ ] **Step 3 (if feasible): Implement, run tests, full client gate, commit** — following the same TDD/gate/commit pattern as every other task in this plan.

---

## Task 30: Cache/reuse cross-fade `RenderTexture`s in `captureFog` (G1)

**Files:**
- Modify: `src/client/render/src/pixi-backend.ts` (lines 51-52 field decls, lines 183-219 `setVisibilityBlend`/`captureFog`, lines 440-444 `destroy`)
- Test: `src/client/render/src/pixi-backend.test.ts`

**Interfaces:**
- Modifies: `captureFog` to reuse `fogBlendFromRT`/`fogBlendToRT` (existing fields, currently destroyed+recreated every call) across ticks, recreating ONLY on resize or a fog-input shape change (confirm exact resize-detection signal — likely a stored `{width, height}` compared against the renderer's current dimensions).

- [ ] **Step 1: Read `pixi-backend.ts` lines 40-60 and 180-220 in full**

Confirm the exact current field types and the `setVisibilityBlend`/`captureFog` call sequence (twice per invocation, `from`/`to`, destroying the previous pair first per L186-187) before changing anything.

- [ ] **Step 2: Write the failing test**

```typescript
test("captureFog reuses the same RenderTexture instances across repeated calls at the same size", () => {
  const backend = new PixiBackend(testApp());
  backend.setVisibilityBlend(fogInputA(), fogInputB());
  const rtFrom1 = backend.fogBlendFromRT;
  const rtTo1 = backend.fogBlendToRT;

  backend.setVisibilityBlend(fogInputC(), fogInputD()); // same renderer size, different fog content
  const rtFrom2 = backend.fogBlendFromRT;
  const rtTo2 = backend.fogBlendToRT;

  expect(rtFrom2).toBe(rtFrom1); // same RenderTexture object, not destroyed+recreated
  expect(rtTo2).toBe(rtTo1);
});

test("captureFog recreates the RenderTextures on a resize", () => {
  const backend = new PixiBackend(testApp());
  backend.setVisibilityBlend(fogInputA(), fogInputB());
  const rtFrom1 = backend.fogBlendFromRT;

  backend.resize(800, 600); // simulate a renderer resize
  backend.setVisibilityBlend(fogInputA(), fogInputB());
  const rtFrom2 = backend.fogBlendFromRT;

  expect(rtFrom2).not.toBe(rtFrom1); // must recreate at the new size
});
```

(Match `testApp`/`fogInputA/B/C/D`/`backend.resize` to `pixi-backend.test.ts`'s existing test-double conventions — read the file first.)

- [ ] **Step 3: Run tests to verify the first fails, second passes (or both fail if resize detection doesn't exist yet)**

Run: `pnpm --filter @shadowcat/render test pixi-backend -- --run`
Expected: `reuses the same RenderTexture instances` FAILS (current code always destroys+recreates); the resize test's outcome depends on whether resize handling already exists elsewhere — confirm before assuming it passes.

- [ ] **Step 4: Implement the cache-and-reuse**

In `captureFog`/`setVisibilityBlend`, remove the unconditional destroy-then-recreate (L186-187). Instead: check if `fogBlendFromRT`/`fogBlendToRT` already exist AND match the renderer's current `{width, height, resolution}`; if so, reuse them (just re-render into them); if not (first call, or a resize/resolution change), destroy the stale pair (if any) and create fresh ones. Update `destroy()` (L440-444) to remain correct — it already destroys both on full backend teardown, which stays unchanged.

- [ ] **Step 5: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/render test pixi-backend -- --run`
Expected: PASS

- [ ] **Step 6: Run the full existing fog/vision render suite**

Run: `pnpm --filter @shadowcat/render test -- --run`
Expected: PASS — the cross-fade blend VISUAL output must be identical, only the allocation pattern changes.

- [ ] **Step 7: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 8: Commit**

```bash
git add src/client/render/src/pixi-backend.ts src/client/render/src/pixi-backend.test.ts
git commit -m "perf(client/render): cache/reuse cross-fade RenderTextures across ticks

captureFog previously recreated both RenderTextures every setVisibilityBlend
call (~60/s during a move animation, two full-screen renders/tick). Now
reused unless the renderer resizes or the fog-input shape changes."
```

---

## Task 31: Chat message-list virtualization + narrowed subscription (G2)

**Files:**
- Modify: `src/modules/chat/src/channels.ts:28` (`RENDER_CAP`) and its consuming component (locate via grep — likely `ChatPanel.svelte`)
- Test: `src/modules/chat/src/ChatPanel.test.ts`, `src/modules/chat/src/channels.test.ts`

**Interfaces:**
- Modifies: the chat panel's message rendering from a fixed `RENDER_CAP = 200` slice of the full re-parsed/re-sorted history into a virtualized window (only the visible + a buffer of off-screen rows are actually mounted), and narrows the reactive subscription so a document mutation to an OLD (already-scrolled-past) message doesn't force a full re-parse/re-sort of the entire unbounded history — only the affected message (and anything within the current render window) re-derives.

- [ ] **Step 1: Read `ChatPanel.svelte`'s current reactive pipeline in full**

Locate the whole-store `$derived`/subscription that re-parses/re-sorts on any document mutation (per the TODO's description). Confirm the message-list rendering library/approach already in use (is there an existing virtualization library dependency, or would this be a hand-rolled windowed render using scroll-position + an intersection observer?). Check `package.json` for an existing virtualization dependency before adding one — prefer reusing an existing pattern.

- [ ] **Step 2: Write the failing test for narrowed re-derivation**

```typescript
test("editing a message outside the current render window does not re-sort the full history", () => {
  const store = testDocumentStoreWithNMessages(5000);
  const { component } = renderChatPanel(store);
  const initialSortCallCount = getSortInstrumentationCount(); // add a test-only counter if none exists — confirm convention

  // Mutate message #10 (well outside the most-recent-200 render window).
  store.update(messageIdAt(store, 10), { content: "edited" });

  expect(getSortInstrumentationCount()).toBe(initialSortCallCount); // no full re-sort triggered
});

test("the message list only mounts the visible window plus a buffer, not all 5000 messages", () => {
  const store = testDocumentStoreWithNMessages(5000);
  const { container } = renderChatPanel(store);

  const mountedRows = container.querySelectorAll("[data-message-row]");
  expect(mountedRows.length).toBeLessThan(300); // visible window + buffer, not 5000
});
```

(Match `testDocumentStoreWithNMessages`/`renderChatPanel`/`getSortInstrumentationCount`/`messageIdAt` to `ChatPanel.test.ts`'s existing conventions; `getSortInstrumentationCount` may need a small test-only hook added to the sort function if no equivalent exists — keep it dev/test-only, never shipped to the production bundle.)

- [ ] **Step 3: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/chat test ChatPanel -- --run`
Expected: both FAIL (current pipeline re-sorts on any mutation; current rendering mounts up to `RENDER_CAP=200` unconditionally, which may already pass the second test depending on the exact assertion threshold — adjust the threshold or the test setup so it genuinely distinguishes "windowed" from "capped-but-still-large" if 200 already satisfies "<300")

- [ ] **Step 4: Narrow the reactive subscription**

Replace the whole-store subscribe with a scoped subscription (matching whatever narrowing primitive the project already uses elsewhere — e.g. `createSubscriber`/`subscribe()` per the `contribution-seed-reactive-before-resync` and `sheet-reactive-bridge-missing-subscription` memory lessons) that only reacts to mutations within the current channel's message set AND recomputes sort/parse incrementally (e.g. maintain a sorted structure and splice in the one changed message, rather than re-sorting the whole array).

- [ ] **Step 5: Add virtualized rendering**

Implement (or wire an existing library for) windowed rendering: only mount DOM rows for the currently-visible scroll range plus a buffer.

- [ ] **Step 6: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/chat test ChatPanel -- --run`
Expected: PASS

- [ ] **Step 7: Run the full existing chat test suite**

Run: `pnpm --filter @shadowcat/chat test -- --run`
Expected: PASS — message ordering, edit/delete rendering, and scroll-to-bottom-on-new-message behavior must all be unaffected.

- [ ] **Step 8: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 9: Commit**

```bash
git add src/modules/chat/src/ChatPanel.svelte src/modules/chat/src/ChatPanel.test.ts src/modules/chat/src/channels.ts
git commit -m "perf(client/chat): virtualize message list, narrow reactive re-derivation

Was: full-history re-parse/re-sort on any document mutation, up to 200
unconditionally-mounted rows. Now: incremental re-derivation scoped to
the mutated message + windowed DOM mounting."
```

---

## Task 32: Route-preview re-request debounce (G3)

**Files:**
- Modify: `src/modules/scene-tools/src/controller.svelte.ts` (`makeMeasureTool`'s `requestRoute`, lines 367-396+)
- Test: `src/modules/scene-tools/src/measure-tool.test.ts`

**Interfaces:**
- Modifies: `requestRoute`'s `onPointerMove` handler — adds a leading-edge debounce (per the project's `debounce-leading-edge-not-trailing-rearm` lesson: arm only when idle, or cap max staleness — never re-arm on every event, which starves under load) on top of the EXISTING `pendingSeq` staleness guard (unchanged — that guard stays, this task reduces REQUEST VOLUME, the staleness guard already handles stale-RESPONSE rendering correctly).

- [ ] **Step 1: Read `requestRoute`/`onPointerMove` in full**

Confirm the exact current call frequency (every `pointermove` event with no gating) and the `pendingSeq` mechanism (L323, 372, 377/393) before adding debounce logic on top of it.

- [ ] **Step 2: Write the failing test**

```typescript
test("rapid pointer moves are debounced to a bounded request rate, leading-edge", () => {
  vi.useFakeTimers();
  const tool = makeMeasureTool(testCtx());
  const requestSpy = vi.spyOn(tool, "pathfind" /* or whatever the underlying request fn is named */);

  tool.onPointerMove(pointerEventAt(10, 10));
  tool.onPointerMove(pointerEventAt(11, 10)); // fires immediately after — should be suppressed
  tool.onPointerMove(pointerEventAt(12, 10)); // still within the debounce window — suppressed
  expect(requestSpy).toHaveBeenCalledTimes(1); // leading-edge: the FIRST move in a burst fires immediately

  vi.advanceTimersByTime(DEBOUNCE_MS); // whatever debounce interval is chosen
  tool.onPointerMove(pointerEventAt(13, 10)); // idle again — next move fires immediately
  expect(requestSpy).toHaveBeenCalledTimes(2);

  vi.useRealTimers();
});

test("a stale response is still correctly ignored via the existing pendingSeq guard", () => {
  // Regression guard: this task must not touch/weaken the existing
  // last-write-wins staleness check — only reduce REQUEST volume.
  const tool = makeMeasureTool(testCtx());
  tool.onPointerMove(pointerEventAt(10, 10));
  const seq1 = tool.currentPendingSeq;
  tool.onPointerMove(pointerEventAt(20, 20));
  const seq2 = tool.currentPendingSeq;

  tool.handlePathResult({ seq: seq1, /* stale result */ });
  expect(tool.renderedRoute).not.toEqual(/* seq1's route */); // stale response ignored, seq2's still pending/applies
});
```

(Match `makeMeasureTool`/`testCtx`/`pointerEventAt`/`handlePathResult`/`currentPendingSeq`/`renderedRoute` to `measure-tool.test.ts`'s existing conventions.)

- [ ] **Step 3: Run tests to verify the first fails, second passes**

Run: `pnpm --filter @shadowcat/scene-tools test measure-tool -- --run`
Expected: `rapid pointer moves are debounced` FAILS (no debounce exists — every move currently fires a request); the `pendingSeq` regression test PASSES already.

- [ ] **Step 4: Implement leading-edge debounce**

In `requestRoute`/`onPointerMove`, add a leading-edge debounce: on a pointer move, if no request is currently "in flight or recently sent" (track a last-sent timestamp), fire immediately and start a cooldown window; moves arriving within the cooldown window update the pending target position but don't fire a new request until the cooldown elapses, at which point the LATEST pending position (not a stale intermediate one) fires. This differs from naive trailing-edge debounce (which starves under continuous movement) per the cited project lesson.

- [ ] **Step 5: Run tests to verify both pass**

Run: `pnpm --filter @shadowcat/scene-tools test measure-tool -- --run`
Expected: PASS

- [ ] **Step 6: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 7: Commit**

```bash
git add src/modules/scene-tools/src/controller.svelte.ts src/modules/scene-tools/src/measure-tool.test.ts
git commit -m "perf(client/scene-tools): leading-edge debounce on route-preview re-requests

Reduces pathfind request volume on fast pointer drags without starving
under continuous movement (leading-edge, not trailing re-arm). The
existing pendingSeq staleness guard is unchanged."
```

---

## Task 33: Deterministic `faction-registry` seed id + dedupe-on-conflict (H1)

**Files:**
- Modify: `src/modules/factions/src/FactionsPanel.svelte` (lines 26-36, the seed `$effect`)
- Test: `src/modules/factions/src/FactionsPanel.test.ts`

**Interfaces:**
- Modifies: the seed `$effect` to construct the `faction-registry` doc with a deterministic id (e.g. a well-known constant UUID, or a UUID derived deterministically from `worldId` — decide in Step 1) instead of a randomly-generated one, and to dedupe-on-conflict (if the deterministic-id Create races and loses, per Task 9/B2's new server-side create-gate, the losing client should gracefully adopt the winning doc rather than erroring).

**Note:** this pairs with Task 9 (B2, the server-side singleton create-gate) — with B2 landed, a losing racer's Create will now be REJECTED by the server (`DataError::Conflict`) instead of silently succeeding and forking a second registry. H1's job is to make the CLIENT handle that rejection gracefully (re-query and adopt the winner) rather than to prevent the race client-side (the server now does that).

- [ ] **Step 1: Decide the deterministic-id scheme**

Best-long-term-shape: derive the `faction-registry` doc's id deterministically from `worldId` via a stable UUID-v5-style derivation (namespace + `worldId` + `"faction-registry"` as the name), so it's reproducible without needing to look anything up first, and collision-free across worlds. Check whether the codebase already has a UUID-v5 utility (grep `uuid` imports) — if not, and if adding a new dependency is undesirable, a simpler alternative is a per-world well-known suffix scheme; confirm which the `condition-registry`/`world-settings` seeders (if they have the same pattern) already use, for consistency, since H1 is the reference implementation other singleton seeders should eventually match.

- [ ] **Step 2: Write the failing test**

```typescript
test("two GMs entering a brand-new world simultaneously converge on ONE faction-registry, not two", async () => {
  const worldId = "world-1";
  const store1 = testDocumentStore(); // GM connection 1
  const store2 = testDocumentStore(); // GM connection 2

  // Simulate near-simultaneous first-entry seeding from both connections.
  const create1 = seedFactionRegistryIfAbsent(store1, worldId);
  const create2 = seedFactionRegistryIfAbsent(store2, worldId);
  await Promise.all([create1, create2]);

  const registries = store1.query("faction-registry"); // after resync, both stores converge
  expect(registries.length).toBe(1);
});

test("a losing racer gracefully adopts the winning registry instead of erroring", async () => {
  const worldId = "world-1";
  const store = testDocumentStore();
  await createConflictingRegistryServerSide(store, worldId); // simulate the server-side create-gate having already rejected this client's attempt

  const result = await seedFactionRegistryIfAbsent(store, worldId);

  expect(result.error).toBeUndefined(); // no user-visible error
  expect(store.query("faction-registry").length).toBe(1); // adopted the existing one
});
```

(Match `testDocumentStore`/`seedFactionRegistryIfAbsent`/`createConflictingRegistryServerSide` to `FactionsPanel.test.ts`'s existing conventions — `seedFactionRegistryIfAbsent` is a new exported function this task extracts from the inline `$effect` to make it independently testable.)

- [ ] **Step 3: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/factions test FactionsPanel -- --run`
Expected: FAIL (current seed uses a random id, so two racers currently DO fork two registries; and there's no graceful-conflict-adoption path since nothing currently rejects the losing Create)

- [ ] **Step 4: Extract and fix the seed logic**

Extract the inline `$effect` body (L27-36) into an exported `seedFactionRegistryIfAbsent(store, worldId)` function using the deterministic id from Step 1. On a `Conflict` rejection from the server (now possible per Task 9/B2), catch it and re-query for the existing registry instead of surfacing an error.

- [ ] **Step 5: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/factions test FactionsPanel -- --run`
Expected: PASS

- [ ] **Step 6: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 7: Commit**

```bash
git add src/modules/factions/src/FactionsPanel.svelte src/modules/factions/src/FactionsPanel.test.ts
git commit -m "fix(client/factions): deterministic faction-registry seed id + graceful conflict adoption

Pairs with the server-side singleton create-gate (Task 9): a losing
racer's Create is now rejected server-side; the client re-queries and
adopts the winning registry instead of erroring or forking a second one."
```

---

## Task 34: Human-readable scene name in the game-settings scene picker (H2)

**Files:**
- Modify: `src/modules/game-settings/src/GameSettingsPanel.svelte:322-324`
- Test: `src/modules/game-settings/src/scene-overrides.test.ts` (extend near the existing picker test at line 153)

**Interfaces:** No new schema field needed — per research, scene docs already carry `.name` via the universal envelope (`scene-docs.ts`'s `envelope()`). Only the picker's rendering changes.

- [ ] **Step 1: Confirm the envelope's `.name` field is populated for scenes**

Read `scene-docs.ts`'s `envelope()` and confirm scenes get a `name` at creation time (via whatever scene-creation UI/flow exists — the scene browser, per M12d). If scenes CAN have `name: null` (e.g. created via a path that doesn't prompt for a name), the fallback in Step 3 must handle that.

- [ ] **Step 2: Write the failing test**

```typescript
test("the scene picker shows the scene's name, not its raw UUID", () => {
  const store = testDocumentStore();
  const scene = testScene(store, { name: "The Sunken Temple" });
  const { getByRole } = render(GameSettingsPanel, { context: mapWithStore(store) });

  const option = getByRole("option", { name: /The Sunken Temple/ });
  expect(option).toBeTruthy();
  expect(option.textContent).not.toContain(scene.id);
});

test("a scene with no name falls back to its id", () => {
  const store = testDocumentStore();
  const scene = testScene(store, { name: null });
  const { getByRole } = render(GameSettingsPanel, { context: mapWithStore(store) });

  const option = getByRole("option", { name: scene.id });
  expect(option).toBeTruthy();
});
```

(Match `testDocumentStore`/`testScene`/`mapWithStore` to `scene-overrides.test.ts`'s existing conventions from the picker test at line 153.)

- [ ] **Step 3: Run tests to verify the first fails, second passes**

Run: `pnpm --filter @shadowcat/game-settings test scene-overrides -- --run`
Expected: `shows the scene's name` FAILS (currently shows raw id); the fallback test PASSES already (raw id is already what's shown, which happens to satisfy "falls back to id" trivially)

- [ ] **Step 4: Fix the picker**

At `GameSettingsPanel.svelte:322-324`, change:
```svelte
<option value={s.id}>{s.id}</option>
```
to:
```svelte
<option value={s.id}>{s.name ?? s.id}</option>
```

- [ ] **Step 5: Run tests to verify both pass**

Run: `pnpm --filter @shadowcat/game-settings test scene-overrides -- --run`
Expected: PASS

- [ ] **Step 6: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 7: Commit**

```bash
git add src/modules/game-settings/src/GameSettingsPanel.svelte src/modules/game-settings/src/scene-overrides.test.ts
git commit -m "fix(client/game-settings): scene picker shows the scene name, not its raw UUID

No schema change — scene docs already carry .name via the universal
envelope. Falls back to the id when name is null."
```

---

## Task 35: M10e-6 cleanup bundle (H3)

**Files:**
- Modify: `src/server/src/scene/vision.rs` (`point_segment_distance` epsilon threshold; also remove the stale `#[allow(dead_code)]` + stale TODO comment at lines 173-174 per research's bonus finding — `pathfinding.rs:91` already calls this function)
- Modify: `src/server/src/scene/pathfinding.rs` (move the mid-file `use std::collections::{BinaryHeap, HashMap};` at line 245 to the top-of-file block, lines 19-21)
- Modify: `src/client/render/src/grid.test.ts` (add the `dmin=2→3` alternating-rule assertion near line 65-66)
- Modify: `src/modules/stage/src/Stage.svelte` (rename the inner shadowing `scene` var, line 125, to `activeSceneDoc`; update its uses at lines 127-128)
- Modify: `src/client/core/src/ws-client.test.ts` (fix the `JSON.stringify(JSON.parse(...))` re-serialization — locate exact line via grep at task time, per the research gap)
- Modify: wherever the `pending` map's `SearchPage|PathResult` union lives (`src/client/shell/src/lib/worldSession.svelte.ts`, near lines 177/192/327/439) — add a `PendingResult` type alias
- Test: existing test suites for each touched file (no new test files — these are all either non-behavioral cleanups or the H3 items ARE tests themselves, e.g. the `grid.test.ts` addition)

This is six small independent cleanups bundled into one task per the spec (`[non-blocking polish, none security/correctness]`). Each sub-step is TDD'd individually where it has behavior; the pure-cosmetic ones (use-decl ordering, var rename, type alias) get a compile-and-gate check instead of a dedicated test.

- [ ] **Step 1: `point_segment_distance` epsilon + stale dead-code cleanup**

In `src/server/src/scene/vision.rs`, change the degenerate-segment threshold at line 181 from `if len2 <= f64::EPSILON` to a geometry-scale threshold, e.g. `if len2 <= 1e-10` (matching the spec's cited "~1e-10" target — confirm this is dimensionally sensible against the codebase's existing grid-unit scale conventions before finalizing the exact constant). Remove the stale `#[allow(dead_code)]` and the stale `// TODO: remove once the grid pathfinder A* body is live...` comment at lines 173-174 (confirmed stale by research: `pathfinding.rs:91` already calls this function).

Write a test first:
```rust
#[test]
fn point_segment_distance_degenerate_segment_uses_geometry_scale_epsilon() {
    // A segment with near-zero (but not exactly zero) length, below the old
    // f64::EPSILON threshold but meaningfully non-degenerate at scene scale.
    let a = P { x: 0.0, y: 0.0 };
    let b = P { x: 1e-9, y: 0.0 }; // len2 = 1e-18, well below both thresholds — still degenerate
    let point = P { x: 5.0, y: 0.0 };
    let dist = point_segment_distance(point, a, b);
    assert!((dist - 5.0).abs() < 1e-6, "a genuinely-degenerate segment collapses to point-distance from `a`");
}
```
Run: `cargo test --manifest-path src/server/Cargo.toml point_segment_distance -- --nocapture` — confirm PASS after the constant change (this is largely a pinning test since the exact epsilon rarely matters at scene scale; the real value of this sub-step is removing the stale dead-code annotation, which clippy should confirm is no longer needed).

- [ ] **Step 2: `pathfinding.rs` use-decl ordering**

Move `use std::collections::{BinaryHeap, HashMap};` from line 245 to the top-of-file block (lines 19-21), alongside the existing `use crate::scene::movement;`/`use crate::scene::vision::{...}`/`use std::collections::BTreeSet;`. Consolidate into the existing `std::collections` import if one is already there (`BTreeSet` — combine into `use std::collections::{BTreeSet, BinaryHeap, HashMap};`).

- [ ] **Step 3: `grid.test.ts` explicit `dmin=2→3` assertion**

Add alongside the existing alternating-rule test at line 65-66:
```typescript
test("alternating (5-10-5) rule: the second diagonal in a run at dmin=2 costs 3 (not 2)", () => {
  const grid = new Grid({ diagonalRule: "alternating", cellSize: 50 });
  // Two consecutive diagonal steps starting at an even diagonal count (dmin=2)
  // must cost 1 then... the THIRD step (dmin=2 -> 3) is the one that costs 2
  // under the 5-10-5 alternating pattern — confirm the EXACT expected value
  // against Grid.distance()'s actual alternating-rule implementation before
  // finalizing this assertion; the spec names "dmin=2 -> 3" as the case to
  // cover but the implementer must read Grid.distance()'s real alternating
  // logic to get the expected numeric value right, not guess it.
  const dist = grid.distance(/* a path exercising the dmin=2->3 transition */);
  expect(dist).toBe(/* the correct value per Grid.distance()'s real logic */);
});
```

- [ ] **Step 4: `Stage.svelte` shadowing rename**

At `src/modules/stage/src/Stage.svelte:125`, rename the inner `const scene = ...` to `const activeSceneDoc = ...`; update its two uses at lines 127-128. Confirm the outer `scene` (from `ctx` at line 25, the `SceneToolHost` bridge) is unaffected — it keeps its name.

- [ ] **Step 5: `ws-client.test.ts` re-serialization fix**

Grep `src/client/core/src/ws-client.test.ts` for `JSON.stringify(JSON.parse(` or similar round-trip patterns. Replace the fragile re-serialization with a direct comparison against the already-parsed object (or a fixture literal), removing the redundant round-trip.

- [ ] **Step 6: `PendingResult` type alias**

In `src/client/shell/src/lib/worldSession.svelte.ts`, add `type PendingResult = SearchPage | PathResult;` near the top of the file and use it at the `pending` map's type declaration and its four usage sites (lines 177/192/327/439 per research).

- [ ] **Step 7: Full gate for both server and client changes**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`
Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`
Expected: all green

- [ ] **Step 8: Commit**

```bash
git add src/server/src/scene/vision.rs src/server/src/scene/pathfinding.rs src/client/render/src/grid.test.ts src/modules/stage/src/Stage.svelte src/client/core/src/ws-client.test.ts src/client/shell/src/lib/worldSession.svelte.ts
git commit -m "chore(server+client): M10e-6 cleanup bundle

point_segment_distance geometry-scale epsilon + remove stale dead-code
annotation; pathfinding.rs use-decls to top-of-file; grid.test.ts
dmin=2->3 alternating assertion; Stage.svelte scene-var un-shadow;
ws-client.test.ts drop fragile re-serialization; PendingResult type
alias for the SearchPage|PathResult union."
```

---

## Task 36: `panels.spec.ts` testid selector (H4)

**Files:**
- Modify: `src/client/shell/e2e/panels.spec.ts:95`
- Test: this IS the test file being fixed — no separate test needed, the fix is the deliverable.

**Interfaces:** No behavior change — test-selector-only fix.

- [ ] **Step 1: Change the selector**

At `panels.spec.ts:95`, change:
```typescript
page.locator(".tool-rail .tool").first()
```
to:
```typescript
page.locator('[data-testid^="tool-"]').first()
```
(Or, if the specific tool being located is known at this call site, use the exact `data-testid="tool-{id}"` form rather than a prefix match — read the surrounding test context in `panels.spec.ts` to determine which tool this line is meant to target, and use the precise testid if so.)

- [ ] **Step 2: Run the e2e spec to verify it still passes**

Run: `pnpm --filter @shadowcat/shell exec playwright test panels.spec.ts`
Expected: PASS (same behavior, more resilient selector — decoupled from styling class churn)

- [ ] **Step 3: Commit**

```bash
git add src/client/shell/e2e/panels.spec.ts
git commit -m "test(client/e2e): locate tool-rail buttons via data-testid, not styling class"
```

---

## Task 37: `sizeClass.svelte.ts` teardown test (+ paired i18n gap) (H5)

**Files:**
- Modify: `src/client/ui-kit/src/sizeClass.test.ts` (new, or extend if it exists)
- Modify: `src/client/ui-kit/src/i18n.test.ts` (add the paired teardown test)

**Interfaces:** No production code change — both `sizeClass.svelte.ts` and `i18n.svelte.ts`'s teardown logic already exist (`createSubscriber`'s `removeEventListener`/`i18n.subscribe(update)` cleanup respectively); this task only adds test coverage.

- [ ] **Step 1: Write the failing test for `sizeClass.svelte.ts`**

```typescript
test("sizeClass's createSubscriber removes its matchMedia listener on teardown", () => {
  const mql = mockMatchMedia();
  const cleanup = $effect.root(() => {
    sizeClass(); // or however the subscriber is invoked/consumed — confirm exact API
  });

  expect(mql.listenerCount).toBe(1);
  cleanup();
  expect(mql.listenerCount).toBe(0);
});
```

(`mockMatchMedia`/the exact `sizeClass()` consumption API must match `sizeClass.svelte.ts`'s real exported shape — read the file first, since it wasn't fully characterized in research beyond the teardown line itself.)

- [ ] **Step 2: Run test to verify it fails or passes**

Run: `pnpm --filter @shadowcat/ui-kit test sizeClass -- --run`
Expected: since the teardown code ALREADY EXISTS (per research, this is a coverage gap not a bug), this test should PASS immediately, proving the existing implementation correct. If it FAILS, this reveals a real teardown bug — fix `sizeClass.svelte.ts` until it passes, and note the discrepancy from the "coverage gap, not a bug" premise in the commit message.

- [ ] **Step 3: Write the paired `i18n.svelte.ts` teardown test**

```typescript
test("i18n's createSubscriber unsubscribes from i18n.subscribe on teardown", () => {
  const unsubscribeSpy = vi.fn();
  const i18nMock = { subscribe: vi.fn(() => unsubscribeSpy) };
  const cleanup = $effect.root(() => {
    i18nSubscriber(i18nMock); // confirm exact consumption API from i18n.svelte.ts:8
  });

  cleanup();
  expect(unsubscribeSpy).toHaveBeenCalled();
});
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/ui-kit test i18n -- --run`
Expected: PASS (same "coverage gap, not a bug" expectation as Step 2)

- [ ] **Step 5: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 6: Commit**

```bash
git add src/client/ui-kit/src/sizeClass.test.ts src/client/ui-kit/src/i18n.test.ts
git commit -m "test(client/ui-kit): cover sizeClass + i18n createSubscriber teardown

Both already correctly unsubscribe on teardown; this closes the
pre-existing coverage gap for both, together."
```

---

## Task 38: `controller.test.ts` full boot-race order assertion (H6)

**Files:**
- Modify: `src/modules/panels/src/controller.test.ts:248`
- Test: this IS the test being tightened.

**Interfaces:** No production code change — tightens an existing test's assertion.

- [ ] **Step 1: Read the boot-race test's full context**

Read `controller.test.ts` around line 221-248 in full (the comment at 221 explains registrations arrive out-of-order vs `saved.compact.order`). Confirm the exact, deterministic expected FULL order (not just membership) that per-panel `locate()` placements produce — per the spec, these ARE exactly pinned, so a concrete expected sequence exists.

- [ ] **Step 2: Tighten the assertion**

Change line 248 from:
```typescript
expect(ctrl.layout.compact.order).toContain("factions");
```
to a full-sequence equality assertion, matching the pattern already used elsewhere in the same file (lines 141/153, `toEqual`):
```typescript
expect(ctrl.layout.compact.order).toEqual(["topbar", "factions", "conditions", /* the real, complete, deterministic order from Step 1 */]);
```

- [ ] **Step 3: Run the test to verify it passes with the tightened assertion**

Run: `pnpm --filter @shadowcat/panels test controller -- --run`
Expected: PASS (if it fails, the actual boot order isn't what was assumed — read the real `locate()` placement logic to get the correct expected sequence, rather than adjusting the test to whatever happens to come out, since the whole point is pinning the REAL deterministic contract)

- [ ] **Step 4: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 5: Commit**

```bash
git add src/modules/panels/src/controller.test.ts
git commit -m "test(client/panels): tighten boot-race test to full order equality

Was toContain (membership only); per-panel locate() placements are
exactly pinned, so the test now asserts the full deterministic sequence."
```

---

## Task 39: Browser e2e asserting scene background renders (H7)

**Files:**
- Modify: locate the render engine's `scene.system.background` → sprite consumption (grep at task time, per the research gap — likely `src/client/render/src/` reconciler or a dedicated scene-view file)
- Create/modify: a new Playwright e2e test in `src/client/shell/e2e/`

**Interfaces:** No production code change (the feature — setting `scene.system.background` via the scene browser, and the render engine consuming it — already exists per M12d; this task only adds the missing e2e assertion).

- [ ] **Step 1: Locate the background-sprite render code and an existing e2e pattern to extend**

Grep `scene.system.background` (or `.background`) across `src/client/render/src/` to find the sprite-creation call site. Read an existing scene-related Playwright spec in `src/client/shell/e2e/` (e.g. `panels.spec.ts` fixed in Task 36, or a scene-browser spec if one exists) for the setup/teardown/world-creation conventions e2e specs in this repo already use.

- [ ] **Step 2: Write the failing e2e test**

```typescript
test("setting a scene's background renders a sprite on the stage", async ({ page }) => {
  await loginAsGmAndEnterTestWorld(page); // match the repo's existing e2e setup helper
  await createSceneWithBackground(page, "test-background-asset-id"); // via the scene browser UI, per M12d

  const stageCanvas = page.locator('[data-testid="stage-canvas"]'); // confirm the real testid/selector from Stage.svelte
  await expect(stageCanvas).toBeVisible();
  // A PixiJS canvas render can't be asserted via DOM structure alone —
  // confirm whether the project has an existing convention for asserting
  // canvas render state (a pixel-sample check, a debug readout element,
  // or an exposed test hook on the render engine) before finalizing this
  // assertion; read how existing scene-tools e2e specs (if any) verify
  // canvas-rendered content.
});
```

(This test's exact assertion mechanics depend on the project's existing canvas-testing convention, which must be confirmed in Step 1 — do not guess; if no such convention exists yet, this task may need to add a minimal test-only render-state hook, kept dev/test-only.)

- [ ] **Step 3: Run test to verify it fails or passes**

Run: `pnpm --filter @shadowcat/shell exec playwright test scene-background`
Expected: depends on Step 2's exact assertion mechanics — if a render-state hook needs adding first, this will fail until Step 4.

- [ ] **Step 4: Add any missing test-only render-state hook (only if Step 1 found none) and finalize the assertion**

- [ ] **Step 5: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/shell exec playwright test scene-background`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/client/shell/e2e/scene-background.spec.ts
git commit -m "test(client/e2e): assert scene background renders as a sprite

Closes the M12d coverage gap — the scene browser UI to set
scene.system.background existed with no e2e proof it actually renders."
```

---

## Task 40: Verify + close I1 (speak-as composer picker) — already implemented

**Files:**
- Read-only verification: `src/modules/chat-composer/src/Composer.svelte`, `src/modules/chat-card/src/MessageCard.svelte`
- Modify: `docs/TODO.md` (remove the stale entry — folded into Task 45/J2's RESOLVED-pruning pass, but verify HERE first)

**Interfaces:** None — no code change expected.

- [ ] **Step 1: Verify the implementation end-to-end**

Read `Composer.svelte` (confirm `selectedActorId` state, the `<select>` bound to it, `speakableActors` options via `actorDisplayName()`, and `actorOwner` built as `{ kind: "actor", actor_id: selectedActorId }` sent on `send()`) and `MessageCard.svelte` (confirm `sys?.actor_owner` is read and rendered at the card level). Confirm both against the CURRENT state of `main` (research is a few commits old by the time this task executes — re-verify, don't trust the research cache blindly).

- [ ] **Step 2: Write a verification test if genuinely missing coverage**

If `Composer.test.ts`/`MessageCard.test.ts` don't already cover "select an actor, send a message, card renders the actor's name as speaker," add one test proving the full round-trip:

```typescript
test("selecting an actor in the composer results in the message card showing that actor as speaker", async () => {
  const store = testDocumentStore();
  const actor = testActor(store, { name: "Elowen" });
  const { getByLabelText, getByText } = renderComposer(store);

  await fireEvent.change(getByLabelText(/speak as/i), { target: { value: actor.id } });
  await fireEvent.click(getByText(/send/i));

  const card = renderMessageCard(store, latestMessage(store));
  expect(card.getByText("Elowen")).toBeTruthy();
});
```

If equivalent coverage already exists, this step is "confirm the existing test covers this" — do not write a duplicate.

- [ ] **Step 3: Run the test**

Run: `pnpm --filter @shadowcat/chat-composer test -- --run && pnpm --filter @shadowcat/chat-card test -- --run`
Expected: PASS

- [ ] **Step 4: No commit needed for code (already correct) — this task's output feeds Task 45's TODO.md pruning**

Record in the plan-execution notes: "I1 verified already fully implemented on `main` — TODO.md's WS-I1 entry (if any literal entry exists distinct from the spec's bucket) is stale and removed at Task 45, not built as new work."

---

## Task 41: Rich roll tooltip popover (I2)

**Files:**
- Create: `src/modules/chat-card/src/RollTooltip.svelte`
- Modify: `src/modules/chat-card/src/MessageCard.svelte` (line 291, replace native `title={inlineRollTitle(s)}` with the popover)
- Test: `src/modules/chat-card/src/RollTooltip.test.ts`

**Interfaces:**
- Produces: `RollTooltip` component, props: `{ outcome: RollOutcome }` (the same `outcome.records[]` — each with `.kept`/`.value` — already used by `keptValues` at `MessageCard.svelte:138-140`; no new data plumbing needed per research).
- Consumes: `RollOutcome` type from `@shadowcat/core` (existing, unchanged).

- [ ] **Step 1: Read `MessageCard.svelte` lines 130-150 and 280-295 in full**

Confirm `RollOutcome.records[]`'s exact shape (`{kept, value, ...}` — what other fields exist, e.g. die face/kind, for a richer per-die table beyond just kept-values) and the exact chip markup at line 291 to be replaced.

- [ ] **Step 2: Write the failing test**

```typescript
test("hovering/focusing the roll chip shows a popover with the full per-die table", async () => {
  const outcome = testRollOutcome({ records: [{ kept: true, value: 5 }, { kept: false, value: 2 }, { kept: true, value: 6 }] });
  const { getByRole, queryByRole } = render(RollTooltip, { outcome });

  expect(queryByRole("tooltip")).toBeNull(); // closed by default

  await fireEvent.focus(getByRole("button", { name: /roll details/i }));

  const tooltip = getByRole("tooltip");
  expect(tooltip.textContent).toContain("5");
  expect(tooltip.textContent).toContain("2");
  expect(tooltip.textContent).toContain("6");
  // Dropped (kept: false) dice should be visually/semantically distinguished, not just listed identically.
  expect(tooltip.querySelector('[data-dropped="true"]')?.textContent).toContain("2");
});

test("the popover is keyboard-accessible (focus opens, Escape closes)", async () => {
  const outcome = testRollOutcome({ records: [{ kept: true, value: 4 }] });
  const { getByRole } = render(RollTooltip, { outcome });

  await fireEvent.focus(getByRole("button", { name: /roll details/i }));
  expect(getByRole("tooltip")).toBeTruthy();

  await fireEvent.keyDown(document.activeElement, { key: "Escape" });
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
});
```

(Match `testRollOutcome` to `MessageCard.test.ts`'s existing roll-outcome fixture helper.)

- [ ] **Step 3: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/chat-card test RollTooltip -- --run`
Expected: FAIL (component doesn't exist)

- [ ] **Step 4: Implement `RollTooltip.svelte`**

Build a focus/hover-triggered popover (WAI-ARIA `role="tooltip"` pattern, or consider reusing Task 22's `MenuKeyboard` primitive if Escape-to-close logic overlaps meaningfully — likely not, since a tooltip isn't a navigable menu; keep it a simpler standalone component) rendering the full `outcome.records[]` table, visually distinguishing dropped (`kept: false`) dice.

- [ ] **Step 5: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/chat-card test RollTooltip -- --run`
Expected: PASS

- [ ] **Step 6: Wire into `MessageCard.svelte`**

Replace line 291's `title={inlineRollTitle(s)}` native-tooltip usage with `<RollTooltip outcome={s.outcome} />` wrapping (or adjacent to) the existing chip. Keep `inlineRollTitle`/`keptValues` if still used elsewhere (e.g. as an `aria-label` fallback for the chip itself), or remove if now fully superseded — confirm by grepping other call sites before deleting.

- [ ] **Step 7: Run `MessageCard`'s full test suite**

Run: `pnpm --filter @shadowcat/chat-card test MessageCard -- --run`
Expected: PASS

- [ ] **Step 8: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 9: Commit**

```bash
git add src/modules/chat-card/src/RollTooltip.svelte src/modules/chat-card/src/RollTooltip.test.ts src/modules/chat-card/src/MessageCard.svelte
git commit -m "feat(client/chat): rich per-die roll tooltip popover

Replaces the native title attribute with an accessible popover showing
the full outcome.records[] table (kept/dropped distinguished). No new
data plumbing — the records were already available."
```

---

## Task 42: Unread badges on the chat tab (I3)

**Files:**
- Locate via grep at task time (research gap): the chat tab / sidebar tab chrome component, and any existing badge/pip pattern in panel chrome
- Modify: the located tab-chrome component
- Modify: `src/modules/chat/src/channels.ts` (or wherever channel/message read-state would be tracked — likely new state)
- Test: matching the located component's test file

**Interfaces:**
- Produces: an unread-count/pip signal on the chat panel's tab, derived from "messages received since this tab was last focused/viewed" — exact tracking mechanism (a `lastReadSeq`/`lastReadTimestamp` per channel, persisted or session-only — decide in Step 1) to be finalized against the real tab-chrome API found in Step 1.

- [ ] **Step 1: Locate the tab-chrome component and any existing badge pattern**

Grep `src/modules/panels/src/` for the tab-rendering component (dockview's `createTabComponent`, per Task 26's `DockviewEngine` research) and confirm whether panel metadata (`metaMap`, per `PanelHost`/`DockChips` research) already has a slot for a badge/count, or whether one needs adding to the `PanelMeta`/contribution shape. Check `DockChips.svelte` (Task 28) and `PanelMenu.svelte` for any existing badge/pip visual pattern to match stylistically.

- [ ] **Step 2: Decide the read-state tracking mechanism**

Best-long-term-shape: track `lastReadSeq` per channel in client-local `ui_state` (the same per-user session-state blob M7 already persists server-side, per `GET/PUT /me/ui-state`) rather than a new server concept — unread state is a pure client UX signal, not something other users need to see, so it doesn't need a new document type or server round-trip beyond the existing `ui_state` persistence.

- [ ] **Step 3: Write the failing test**

```typescript
test("the chat tab shows an unread badge when a new message arrives while the tab is not focused", () => {
  const store = testDocumentStore();
  const { getByTestId, queryByTestId } = renderShellWithChatTab(store, { chatTabFocused: false });

  expect(queryByTestId("chat-unread-badge")).toBeNull();

  store.receiveMessage(testMessage({ channel: "general" }));

  expect(getByTestId("chat-unread-badge").textContent).toBe("1");
});

test("focusing the chat tab clears the unread badge", async () => {
  const store = testDocumentStore();
  const { getByTestId, queryByTestId, focusTab } = renderShellWithChatTab(store, { chatTabFocused: false });
  store.receiveMessage(testMessage({ channel: "general" }));
  expect(getByTestId("chat-unread-badge")).toBeTruthy();

  await focusTab("chat");

  expect(queryByTestId("chat-unread-badge")).toBeNull();
});
```

(Match `testDocumentStore`/`renderShellWithChatTab`/`testMessage`/`focusTab` to whatever Step 1's investigation surfaces as the real tab-chrome test scaffolding — these names are illustrative of the required behavior, not literal existing helpers.)

- [ ] **Step 4: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/panels test -- --run` (or the chat module's test, depending on where the badge actually renders per Step 1)
Expected: FAIL

- [ ] **Step 5: Implement the unread tracking + badge**

Wire `lastReadSeq` per channel into the client's `ui_state`. Compute unread count as messages with `seq > lastReadSeq` for channels the user isn't currently viewing. Render the badge in the tab chrome located in Step 1, matching its existing visual pattern (per `DockChips`/`PanelMenu` conventions).

- [ ] **Step 6: Run tests to verify they pass**

Run the same command as Step 4.
Expected: PASS

- [ ] **Step 7: Full client gate**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`

- [ ] **Step 8: Commit**

```bash
git add <files located and modified in Steps 1/5>
git commit -m "feat(client/chat): unread badges on the chat tab

Tracks lastReadSeq per channel in ui_state; badge shows count of
messages received since the tab was last focused. Foundry-parity
polish, deliberately deferred out of M11d-1 scope until now."
```

---

## Task 43: Send/edit/delete failure surfacing — correlation-id + reason channel (I4) `[sec]`

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (lines 93-107, `ClientMsg::SendMessage`/`EditMessage`/`DeleteMessage`; lines 232-291 area, add a matching `ServerMsg::SendMessageError`/similar)
- Modify: `src/server/src/chat/mod.rs:363+` (`SendMessageError` enum — add `Display` impls per the existing `ParseError`/`Token` precedent cited in research)
- Modify: `src/server/src/ws/conn.rs` (wherever `SendMessage`/`EditMessage`/`DeleteMessage` are dispatched — thread `request_id` through, emit the new error frame on rejection)
- Modify: `src/modules/chat-composer/src/Composer.svelte` (surface the error to the sender)
- Test: `src/server/src/chat/mod.rs`, `src/server/src/ws/conn.rs`, `src/modules/chat-composer/src/Composer.test.ts`

**Interfaces:**
- Produces (server wire, ts-rs + client Zod mirror required — shared wire-schema change, per project lesson, needs `pnpm -r test`, not a filtered subset): adds `request_id: Uuid` to `ClientMsg::SendMessage`/`EditMessage`/`DeleteMessage`, mirroring the existing `Search`/`SceneSubscribe`/`Pathfind`/`moveRequest` correlation pattern exactly (`protocol.rs:35-83`). Adds `ServerMsg::SendMessageError { request_id: Uuid, message: String }` (and equivalently for edit/delete, or one shared `ChatOpError` variant covering all three — decide in Step 1 based on whether the three ops' error enums are similar enough to share one wire variant).
- Consumes: the existing `SendMessageError` enum (`chat/mod.rs:363+`, variants `Empty`/`TooLong`/`RateLimited`/`ActorNotSpeakable`/`UnknownRecipient`/`RollImmutable`/`AudienceLocked`/etc.) — add `Display` impls for player-presentable text.

**Security note (`[sec]`):** per the spec, the reason channel must not leak authorization detail to an unauthorized sender. Read each `SendMessageError` variant and classify: some (e.g. `TooLong`, `Empty`, `RateLimited`) are safe to surface verbatim; others (e.g. anything revealing WHY an audience/permission check failed, if such a variant exists) may need a generic "not permitted" message instead of the specific reason, to avoid an unauthorized sender probing permission structure via error text. Do this classification explicitly in Step 1 before writing `Display` impls.

- [ ] **Step 1: Read every `SendMessageError`/edit/delete error variant and classify fail-open vs fail-closed detail**

Read `chat/mod.rs:363+` in full. For each variant, decide: safe to surface the specific reason (validation-class errors: `Empty`, `TooLong`, `RateLimited`) vs. must surface a generic message only (authorization-class errors, if any exist among the variants — e.g. anything about audience/permission/ownership that could let a sender infer information about a document or user they shouldn't have visibility into). Write this classification down in the commit message later as a record of the decision.

- [ ] **Step 2: Write the failing server test**

```rust
#[tokio::test]
async fn send_message_rejection_returns_a_correlated_error_frame() {
    let (repo, world_id, user_ctx) = test_world_with_flood_limited_user().await;
    let request_id = Uuid::new_v4();

    let result = handle_send_message(&repo, &user_ctx, ClientMsg::SendMessage {
        request_id,
        channel: "general".into(),
        content: "hello".into(),
        // ...
    }, /* preview deps per Task 7 */).await;

    match result {
        Err(ServerMsg::SendMessageError { request_id: rid, message }) => {
            assert_eq!(rid, request_id);
            assert!(!message.is_empty());
        }
        other => panic!("expected a correlated SendMessageError, got {other:?}"),
    }
}

#[tokio::test]
async fn authorization_class_errors_do_not_leak_specific_reason_text() {
    // For whichever variant Step 1 classifies as authorization-class,
    // confirm the surfaced message is generic, not the specific internal reason.
    let (repo, world_id, user_ctx) = test_unauthorized_speak_as_scenario().await;
    let request_id = Uuid::new_v4();

    let result = handle_send_message(&repo, &user_ctx, /* a speak-as an actor this user doesn't own */ test_unauthorized_speak_as_msg(request_id), /* preview deps */).await;

    if let Err(ServerMsg::SendMessageError { message, .. }) = result {
        assert_eq!(message, "You are not permitted to send this message."); // generic, per Step 1's classification — not "ActorNotSpeakable: actor X owned by user Y"
    } else {
        panic!("expected a rejection");
    }
}
```

(Match `test_world_with_flood_limited_user`/`test_unauthorized_speak_as_scenario`/`test_unauthorized_speak_as_msg` to `chat/mod.rs`'s existing test helpers.)

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path src/server/Cargo.toml send_message_rejection authorization_class_errors -- --nocapture`
Expected: FAIL (compile error — `request_id` field and `ServerMsg::SendMessageError` don't exist yet)

- [ ] **Step 4: Add the wire types**

In `protocol.rs`, add `request_id: Uuid` to `ClientMsg::SendMessage`/`EditMessage`/`DeleteMessage` (lines 93-107). Add `ServerMsg::SendMessageError { request_id: Uuid, message: String }` (and equivalent for edit/delete, per Step 1's decision on whether to share one variant) near the existing `SearchError`/`PathError` variants (lines 232-291). Add `#[derive(TS)]` matching the existing wire-type convention.

- [ ] **Step 5: Add `Display` impls for `SendMessageError`**

Following the `ParseError`/`Token` precedent (player-presentable `Display`, pinned by a no-debug-artifacts test over every variant, per research's citation), implement `Display` for `SendMessageError` (and edit/delete equivalents), applying Step 1's fail-open/fail-closed classification per variant.

- [ ] **Step 6: Wire dispatch in `conn.rs`**

Find where `ClientMsg::SendMessage`/`EditMessage`/`DeleteMessage` are matched and dispatched to `handle_send_message`/`handle_edit_message`/(a delete handler). On an `Err` result, emit the new `ServerMsg::*Error { request_id, message }` frame to the sender (only the sender — not broadcast) instead of silently dropping the rejection.

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test --manifest-path src/server/Cargo.toml send_message_rejection authorization_class_errors -- --nocapture`
Expected: PASS

- [ ] **Step 8: Full server gate**

Run: `cargo test --manifest-path src/server/Cargo.toml --all-targets && cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`

- [ ] **Step 9: Add the client Zod mirror + wire the composer**

Add `request_id`/the new error frame to the client's Zod schema mirror (wherever `ClientMsg`/`ServerMsg` are mirrored — likely `src/client/core/src/ws-protocol.ts` or similar, matching the ts-rs-generated shape). In `Composer.svelte`, generate a `request_id` on send, correlate the response, and surface a rejection (e.g. a toast or inline error near the composer) instead of the message silently vanishing.

- [ ] **Step 10: Write the client test**

```typescript
test("a rejected send surfaces an error to the sender instead of silently vanishing", async () => {
  const wsClient = testWsClientRejectingNextSend("Rate limit exceeded, try again shortly.");
  const { getByText, getByRole } = renderComposer({ wsClient });

  await fireEvent.click(getByRole("button", { name: /send/i }));

  expect(getByText(/Rate limit exceeded/i)).toBeTruthy();
});
```

(Match `testWsClientRejectingNextSend`/`renderComposer` to `Composer.test.ts`'s existing conventions.)

- [ ] **Step 11: Run the client test**

Run: `pnpm --filter @shadowcat/chat-composer test -- --run`
Expected: PASS

- [ ] **Step 12: Full repo-wide test gate (shared wire-schema change — not a filtered subset)**

Run: `pnpm -r test && pnpm -r typecheck && pnpm -r lint`
Expected: all green across every package — a wire-schema change ripples to any package with a fixture of the old frame shape.

- [ ] **Step 13: Commit**

```bash
git add src/server/src/ws/protocol.rs src/server/src/chat/mod.rs src/server/src/ws/conn.rs src/client/core/src/ws-protocol.ts src/modules/chat-composer/src/Composer.svelte src/modules/chat-composer/src/Composer.test.ts
git commit -m "feat(chat): correlation-id + reason channel for send/edit/delete rejections

SendMessage/EditMessage/DeleteMessage now carry request_id, mirroring the
existing Search/Pathfind/moveRequest pattern. Server rejections (flood
limit, validation) are now surfaced to the sender instead of silently
vanishing. Authorization-class errors surface a generic reason only
(classified in this commit) — never the specific internal cause, to
avoid leaking permission-structure detail to an unauthorized sender."
```

- [ ] **Step 14: Dispatch mandatory security buddy-check** (focus: does the classification in Step 1/Step 5 actually hold — walk every error variant and confirm none leaks authorization detail) before this task is marked complete.

---

## Task 44: `PLAN.md` — add the missing M13e DONE entry (J1)

**Files:**
- Modify: `docs/PLAN.md` (add an M13e entry alongside M13f's, per the M13f Task 9 reviewed skill-update gate finding)

**Interfaces:** Documentation only.

- [ ] **Step 1: Read the M13f DONE entry and the M13e work's actual scope**

Read `docs/PLAN.md`'s M13f entry for its exact format/voice. Read the `shadowcat-codebase-templates` skill (already documents M13e's landed work: `Document.base`, `merge.ts`/`templates.ts`, `TemplatesController`, `MergeConflictModal`) to source the factual content for the new entry — do not invent details not already documented somewhere.

- [ ] **Step 2: Write the M13e DONE entry**

Add a `> **M13e DONE**` block matching the sibling-entry convention (per the `docs(plan): M12.5 DONE line — match sibling-entry convention` precedent commit visible in git log), summarizing: templates/`base` field/3-way merge engine — `Document.base` opaque merge snapshot, `structuralDiff`/`merge3Tree`/`merge3`/`merge3Embedded`/`restampSubtree`/`takeTemplate`/`snapshotBase`/`stampInstance`/`computePull`/`computeRevert`/`planToUpdate`/`applyResolutions`/`findInstances`/`syncState` in `@shadowcat/core`, `TemplatesController`/`AppContext.templates`, the field-level `MergeConflictModal`, host-rendered `TemplateControls`/`SheetHost` chrome every sheet gets for free. Place it in the correct chronological position relative to M13a/M13f's entries.

- [ ] **Step 3: Commit**

```bash
git add docs/PLAN.md
git commit -m "docs(plan): add the missing M13e DONE entry

M13e's work was fully merged and documented in the
shadowcat-codebase-templates skill but never got its own PLAN.md
roadmap entry, unlike its sibling M13 sub-milestones. Surfaced by the
M13f Task 9 reviewed skill-update gate."
```

---

## Task 45: Prune all RESOLVED entries from `TODO.md` (J2)

**Files:**
- Modify: `docs/TODO.md`

**Interfaces:** Documentation only.

- [ ] **Step 1: Read the current `docs/TODO.md` in full**

Read the file (395 lines as of this plan's research). Identify every entry prefixed `RESOLVED (...)`.

- [ ] **Step 2: Remove every RESOLVED entry**

Delete each `- RESOLVED (...)` bullet and its full multi-line body. Preserve section headings even if a section becomes empty after pruning (a later task, J3, restructures the whole file — don't collapse headings prematurely in this step, keep this step a pure subtraction).

- [ ] **Step 3: Commit**

```bash
git add docs/TODO.md
git commit -m "docs(todo): prune all RESOLVED entries

Historical record already lives in git history and the shadowcat-codebase-*
skills; TODO.md is a live-deferral list, not a changelog."
```

---

## Task 46: Rewrite `TODO.md` to the retagged deferred backlog (J3)

**Files:**
- Modify: `docs/TODO.md`

**Interfaces:** Documentation only. This is the capstone documentation task — it only makes sense to run AFTER every other task in this plan has landed (since several tasks close or reclassify TODO items — Task 40/I1's stale entry, Task 8/B1 unblocking `set_pointer`, Task 9/B2 closing the singleton-uniqueness item, etc.).

- [ ] **Step 1: Confirm every other task in this plan is complete**

This task's preconditions: Tasks 1-44 all merged. If any earlier task was skipped (e.g. Task 29/F7's stretch item), confirm its actual disposition before writing this task's content, since a skipped stretch item needs to either stay in `TODO.md` (if genuinely still open) or get its own explicit note.

- [ ] **Step 2: Rewrite `docs/TODO.md`**

Replace the pruned (Task 45) file's remaining content with the **Deferred backlog** table from the spec (`docs/superpowers/specs/2026-07-19-phase1-cleanup-burndown-design.md`'s "Deferred backlog" section) — each item tagged with its blocking capability, exactly as the spec lists:

```markdown
# TODO — Deferred Work

Actionable, externally-logged deferrals, each blocked by an unbuilt capability.
Bugs go in `OPEN_BUGS.md`, not here. As of the Phase-1 cleanup burndown
(2026-07-19), every item below is retained ONLY because its blocking
capability doesn't exist yet — nothing here is a "someday maybe," each has
a concrete unblocking condition.

## Blocked on world/scene/user deletion
- TODO: Purge `explored_fog` rows on world/scene/user deletion...

## Blocked on a per-turn movement-budget system (Phase-2 combat)
- TODO: `MoveOutcome.cost`/`los_smooth` cost comparability...

## Blocked on rotation authoring
- TODO: Lerp token rotation along the shortest signed delta...

## Blocked on module management / hard topology enforcement
- TODO: `reconcileTopology` version/provides mismatch...
- TODO: `LauncherMenu` metaMap-mutates-while-open focus recovery...

## Blocked on a real 2nd provider / multiple contract versions
- TODO: Singleton multi-provider conflict policy...
- TODO: Capability version negotiation...

## Blocked on a wire-facing Tier/recalc construction path
- TODO: Tier-ladder margin_offset uniqueness guard...
- TODO: DieKind::Faces ReplaceDie out-of-range guard (closes with the recalc-from-chat sub-project)...

## Blocked on the see-as-preview feature buildout
- TODO: GM see-as-player MoveStream preview...

## Blocked on multi-panel popout groups
- TODO: Popout onWillDrop subscription...

## Blocked on real pointer-gesture QA (unsimulable under jsdom)
- TODO: DockviewEngine#toDropSite fallback exhaustiveness — manual QA item.

## Blocked on a bespoke-fallback caller needing it
- TODO: FakeEngine PanelMenu (production never reaches FakeEngine).

## Reference notes (not deferrals — kept for context)
- axum_test's WHATWG dot-segment normalization gotcha for HTTP path-traversal tests...
- Module-declared manifest requirements are advisory-only by design (server
  authority stays with GM's world_cap_requirements)...
- Module-toolchain scope exclusions: upload/install UI stays manual-extract;
  no sandboxing (modules are admin-trusted); no hot enable/disable; no
  marketplace/registry/signing.
```

(Copy the FULL text of each retained item from the pre-pruned file — do not summarize/truncate the actual TODO descriptions, only reorganize under the new capability-tagged headings. The abbreviated "..." above are for this plan document's brevity; the actual `TODO.md` edit must carry each item's complete original text.)

- [ ] **Step 3: Cross-check against the spec's Out-of-scope section**

Confirm every item the spec moved to a follow-on feature sub-project (recalc-from-chat, link-preview images/oEmbed/shared-cache, per-world export, dice-notation grammar growth, per-channel dice overrides, in-body doc-links, speak-as-token-instance) is EITHER already closed by a task in this plan (I1-I4 closed several) OR explicitly listed in a NEW `## Follow-on feature sub-projects (own brainstorm each)` section at the bottom of the rewritten `TODO.md`, not silently dropped.

- [ ] **Step 4: Commit**

```bash
git add docs/TODO.md
git commit -m "docs(todo): rewrite backlog retagged by blocking capability

Every remaining item is tagged with the unbuilt capability that gates
it, per the Phase-1 cleanup burndown spec's Deferred backlog. Follow-on
feature sub-projects (recalc-from-chat, link-preview extensions,
per-world export, notation grammar growth, etc.) listed separately, not
silently dropped."
```

---

## Task 47: `PLAN.md` — burndown entry + remaining Phase-1-close work (J4)

**Files:**
- Modify: `docs/PLAN.md`

**Interfaces:** Documentation only.

- [ ] **Step 1: Add the Phase-1 cleanup burndown entry**

Following the `phase1-bugs-todo-sweep` entry's format (the existing precedent block already in `PLAN.md`, found earlier in this plan's research phase — a `> **Phase-1 open-bugs/TODO sweep DONE**` block), add a new `> **Phase-1 cleanup burndown DONE**` block summarizing: branch `phase1-cleanup-burndown`, ~40 fixes/refactors/tests/features across 10 workstreams, listing the headline items (set_pointer true removal, singleton create-gate, edge-projected environment light, wall-less-scene full vision, ActorsPanel split, shared menu primitive, chat correlation-id/reason-channel, unread badges, rich roll tooltips) and the standing decisions (Create-gate by-design, bucket-C large features split to follow-ons).

- [ ] **Step 2: List the remaining Phase-1-close work**

Per the user's ruling ("M13 must finish first" was the ORIGINAL framing before M13 was discovered already complete on `main`) — confirm current state: M13 is complete, `OPEN_BUGS.md` is empty, and after this plan lands, `TODO.md` is reduced to only genuinely-blocked items. State explicitly in `PLAN.md`: the remaining work before Phase 1 can be declared closed is the follow-on feature sub-projects (listed in Task 46's TODO.md rewrite) that the user chose to build ALL of bucket C for, each needing its own brainstorm → spec → plan cycle. List them by name as the literal next items after this plan.

- [ ] **Step 3: Commit**

```bash
git add docs/PLAN.md
git commit -m "docs(plan): Phase-1 cleanup burndown DONE; list remaining Phase-1-close work

~40 items across 10 workstreams landed. Remaining before Phase 1 closes:
the follow-on bucket-C feature sub-projects (recalc-from-chat,
link-preview extensions, per-world export, notation grammar growth,
per-channel dice overrides, in-body doc-links, speak-as-token-instance),
each its own brainstorm/spec/plan cycle."
```

---

## Task 48: Reviewed skill-update gate (J5)

**Files:**
- Modify: any `.claude/skills/shadowcat-codebase-*/SKILL.md` affected by this plan's changes (determine which in Step 1)

**Interfaces:** Documentation only — this is the mandatory project CLAUDE.md §1 gate, run once at the end covering the WHOLE plan's changes (not per-task, per the plan's own scale — 48 tasks would make a per-task gate prohibitively expensive; the project's own mainline-plan-execution guidance permits a single final review for a plan of this size when run under that skill, and even under subagent-driven-development this specific gate is explicitly a once-at-completion checkpoint per CLAUDE.md).

- [ ] **Step 1: Identify every subsystem this plan touched**

Cross-reference every task's file list against the 12 `shadowcat-codebase-*` skills' stated coverage (core, actors-tokens, chat, client-shell, dice, documents-permissions, module-toolchain, nightfox, scene-rendering, sheets, templates, and any panels-specific skill if one exists — confirm the current skill roster since it grew during M13). At minimum this plan touches: `documents-permissions` (Task 8 set_pointer, Task 9 singleton create-gate), `scene-rendering` (Tasks 1-5, 11-12), `module-toolchain` (Tasks 13-18), `actors-tokens` (Tasks 19-21, 33), `client-shell` (Tasks 22, 25-28), `chat` (Tasks 30-31, 40-43).

- [ ] **Step 2: Update each affected skill**

For each skill identified in Step 1, update its Purpose/Key files/Hard invariants/Gotchas/Pointers sections to reflect: the new `RemovePointer` command (documents-permissions), the singleton create-gate (documents-permissions), the environment-light/wall-less-vision changes (scene-rendering — note the new invariant that lighting stays cosmetic, verified by Task 11's dedicated test), the module lifecycle cleanup + build-time svelte-subpath guard (module-toolchain), the ActorsPanel split into VisualKindEditor/FaceSwapPalette (actors-tokens), the shared MenuKeyboard primitive (client-shell), the chat correlation-id/reason-channel + unread badges (chat).

- [ ] **Step 3: Dispatch `shadowcat-spec-reviewer` to confirm each skill diff is accurate**

Per CLAUDE.md's mandatory reviewed skill-update gate: dispatch `shadowcat-spec-reviewer` (sonnet, effort: high) against the full set of skill diffs from Step 2, confirming no omission, drift, or broken pointer against the actual landed code.

- [ ] **Step 4: Fix any findings, re-verify**

If the reviewer finds gaps, fix and re-dispatch until PASS.

- [ ] **Step 5: Commit**

```bash
git add .claude/skills/
git commit -m "docs(skills): reviewed skill-update gate for the Phase-1 cleanup burndown

Updates shadowcat-codebase-{documents-permissions,scene-rendering,
module-toolchain,actors-tokens,client-shell,chat} for the seams/
invariants/gotchas this plan's ~40 tasks introduced. Reviewed by
shadowcat-spec-reviewer: PASS."
```

---

## Self-Review

**Spec coverage:** Every WS-A through WS-J item from the spec maps to a task above (WS-A: Tasks 1-5,7 [A5 folded into Task 5]; WS-B: Tasks 8-10; WS-C: Tasks 11-12; WS-D: Tasks 13-18; WS-E: Tasks 19-21; WS-F: Tasks 22-28 + optional Task 29; WS-G: Tasks 30-32; WS-H: Tasks 33-39; WS-I: Tasks 40-43; WS-J: Tasks 44-48). One correction from research: **I1 was found already fully implemented** — Task 40 is verify-and-close, not new work, and this is called out explicitly rather than silently treated as a normal build task.

**Placeholder scan:** No "TBD"/"implement later" left unresolved as a final answer — every genuinely-open sub-decision (H3's `dmin=2→3` expected value, D4's `RUNTIME_ENTRIES` export shape, I3's exact tab-chrome API, I4's per-variant fail-open/fail-closed classification) is scoped as an explicit "confirm/decide in Step N by reading X" instruction with a stated best-long-term-shape default, not an unresolved placeholder — this matches how real unknowns in a large pre-plan research pass must be handled (the alternative, guessing exact line-for-line code the research couldn't fully pin down, would produce a plan that's confidently wrong rather than honestly scoped).

**Type consistency:** `LinkPreviewDeps` (Task 7) is referenced consistently; `RemovePointer`/`remove_pointer` (Task 8) naming is consistent between the command enum variant and the function; `VisualKindEditor`/`FaceSwapPalette`/`animSourceComplete` (Tasks 19-20) are referenced consistently across both tasks; `MenuKeyboard.svelte.ts`/`createMenuKeyboard` (Task 22) is referenced consistently in Task 22 itself (Tasks 25/41 do NOT reuse it, correctly — floating-panel resize and roll-tooltip closing are different concerns, not menu navigation).

**Sequencing:** Task 33 (H1, faction-registry dedup) explicitly depends on Task 9 (B2, server create-gate) landing first — noted inline. Task 46 (TODO.md rewrite) explicitly depends on all other tasks completing — noted as its precondition. Task 48 (skill-update gate) is last, per CLAUDE.md's completion-blocking requirement.

