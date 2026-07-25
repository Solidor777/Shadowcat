# Phase D-α — Movement Authority & Secrecy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the `gm_only`-wall route-shape leak, make player movement request-only so `execute_move` becomes the sole movement gate, restore the GM "ignore walls" override, and make the authoritative gate footprint-aware.

**Architecture:** Four sequential items. D10 gives the routing wall set a per-requester view mirroring `region_field`'s two-value contract. D9 refuses non-GM token position writes, deleting the duplicated *traversal* gate in `Room::publish` and repointing its point-placement machinery at `Create`. D8 exempts GMs from every gameplay gate in `execute_move`. D4 adopts the router's footprint predicate in the now-single gate, with the footprint derived server-side from the token.

**Tech Stack:** Rust (axum/tokio/sqlx, `hecs` ECS), Svelte 5 runes + TypeScript, ts-rs → Zod wire mirror, Vitest + `cargo test`, Playwright for canvas.

**Spec:** `docs/superpowers/specs/2026-07-25-phase-d-alpha-movement-authority-secrecy-design.md`

## Global Constraints

- Build order: `pnpm build` (produces `dist/`) MUST precede any `cargo` build — `rust-embed` validates `../../dist/` at compile time.
- Full gate before any commit is considered green: `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint`, `cargo test`, `cargo fmt`, `cargo clippy --all-targets -- -D warnings`.
- Server crate is rustfmt-clean and CI-enforced; keep every server commit `cargo fmt`-clean.
- Cross-platform: `std::path` only, no hardcoded separators, no OS-specific code without `#[cfg]` for all three targets.
- Never fork a decision across two paths. Where two paths must agree, make one derive from the other or have both read one shared symbol. Pin with an anti-drift test that fails if either side changes.
- **Signature-widening rule.** When a task widens a function signature, it MUST enumerate *every* call site including ones in `#[cfg(test)]` modules and other files, list them all in **Files** and in the `git add` line, and update them in the same commit. A commit that does not build is a plan failure. (Three separate instances of this were missed in the pre-buddy-check draft.)
- **Test snippets are real code.** Every prescribed test must compile against the *current* signatures. Fixture helpers that do not yet exist are marked **NEW** and their construction is specified; helpers that do exist are cited with a verified `file:line`. Use array literals (`&[(x, y), …]`), never `&vec![…]` — `clippy::useless_vec` fails the `-D warnings` gate.
- **I1** — a GM bypasses every gameplay gate (walls, mask, impassable, arrest, footprint) and **no** resource guard (`MAX_GATE_WALK_COORD`, `MAX_GATE_WALK_SAMPLES`, non-finite refusal, scene-existence refusal, `MAX_FOOTPRINT_CELLS`, `TokenEngine::validate`).
- **I2** — `execute_move` is the sole implementation of the per-cell movement *traversal* decision. Any future second write path to a token position must call it, never re-implement it.
- **I3** — wall secrecy is a two-value contract: `None` = authoritative, `Some(user)` = per-requester. Callers pass `None` for a GM. Never a third mode.
- **I4** — `route-admissible ⇔ gate-admissible` holds **for non-GM movers on `MovementModel::GridStepped`**, modulo geometry the router may not see (secret regions, `gm_only` walls — both spring at execution). On `Continuous` the two sides evaluate at different granularity (the router at cell centers, the gate at `gate_walk` sample points), so only the weaker `route ⊆ gate-allowed` direction is claimed there. Scoping verified in Task 9.
- **I5** — `sight_walls`/`light_walls` keep the FULL wall set including `gm_only`. Do not unify them with the routing wall set. Pinned by a test (Task 1).
- Comments: present-tense current state, no history/process meta, cite algorithmic decisions. Delete or update any comment the change contradicts, on contact.
- `ts-rs` types are generated — edit the Rust struct, regenerate, mirror in the client Zod schema.
- Never delete files with `rm`/`Remove-Item`; use `trash`.

## Model/Effort directives

Decided at the writing-plans handoff (per `~/.claude/docs/sdd-model-effort-tiers.md`):

- **Plan-writer:** mainline in the calling session (Opus, high).
- **Dispatch loop:** mainline in the calling session.
- **Implementer:** `sdd-implementer` (Sonnet, medium) default. Escalate BLOCKED/DONE_WITH_CONCERNS → `sdd-implementer-highthink` → `sdd-implementer-opus`, never skipping a rung. No task in this plan is a candidate for `sdd-implementer-haiku` — even Task 10 and Task 11 require judgment.
- **Per-task reviewer:** `sdd-reviewer` (Sonnet, high). Escalate to `sdd-reviewer-opus` for Tasks **1, 3, 4, 6, 8, 9** — security-sensitive or multi-file.
- **Final whole-branch review:** the project's two-reviewer pair, `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (spec mandate for a security-sensitive phase). `sdd-final-reviewer` is NOT used in their place.
- **Fix subagents:** reuse whichever implementer tier produced the original task.

## Buddy-check directives

- **Plan buddy check: done 2026-07-25, findings folded in.** Two blind reviewers over this document with repo read access, then a brokered debate over four rounds to full convergence (no unresolved disagreements). Outcome: **5 Critical, 14 Important, 14 Minor**, all confirmed against source by the broker before revision. This document is the post-revision version.
- Flagged tasks: none pre-authorized for a task-level buddy check — the plan-level check was the agreed instrument.
- Unflagged tasks showing risk signals during execution: **ask** the human.
- The final branch is covered by the spec-mandated two-reviewer pair; a second final-branch buddy check was declined as overlapping.

**The five Critical findings, and where each is now fixed:**

1. `token_move` never compares pre/post image, so a `.is_some()` refusal would refuse *every* non-GM token `Update` (rotation, visual, conditions) — **Task 4**, predicate now compares `a0 != a1`.
2. Replacing `blocks_move` with disc-clearance alone drops `cell_enterable`'s `segments_cross` check, making walls permeable to any default-footprint token on the sole remaining gate — **Task 9**, both checks retained.
3. `sendMoves` is called from both the throttled `onPointerMove` (`controller.svelte.ts:851`) and `onPointerUp` (`:859`), so role-branching alone fires a burst of server-executed moves per drag — **Task 5**, gesture split into preview/commit.
4. `ToolContext` has no `role` field, so `ctx.role === "gm"` is unwritable as specified — **Task 5**, `role` added to the interface and projected in `ToolRail.svelte` (which already reads `ctx.role` from AppContext).
5. `Pathfind`'s new `token` gets no authorization, and the prescribed docstring falsely claims it "serves as the non-GM presence proof" — contradicting the live `INVARIANT (scene presence)` block at `conn.rs:569-574` whose premise ("a `Pathfind` frame names no token") this task falsifies, while the one pinning test goes blind — **Task 8**, ownership + `parent_id == scene` required, invariant comment rewritten, pinning test extended.

## File Structure

**Server — modified**

- `src/server/src/scene/mod.rs` — `move_walls` gains `viewer`; `navmesh_for` re-keyed on the wall set; `pathfind` threads the per-requester wall set; new `resolve_token_footprint` + `token_shape_and_size`; test-module `execute_move` calls updated for arity in Tasks 6 and 9.
- `src/server/src/scene/move_exec.rs` — `execute_move` gains `is_gm` and `footprint_radius_cells`; GM exemption; footprint-aware wall + mask + impassable checks; new footprint admissibility guard; `ingress_bound_equals_gate_walks_exactly`'s companion `gate_walk` pairing referenced from Task 4.
- `src/server/src/scene/navmesh.rs` — doc correction: the wall check is also a secrecy gate when the wall set is unfiltered.
- `src/server/src/scene/pathfinding.rs` — no behavior change; `point_segment_distance` and `MAX_FOOTPRINT_CELLS` consumed by Task 9.
- `src/server/src/ws/room.rs` — non-GM token position `Update` refused; traversal gate deleted; point-placement machinery repointed at `Create`; four coordinate-bound tests dispositioned; `execute_move` call site passes `is_gm` + footprint.
- `src/server/src/ws/protocol.rs` — `Pathfind` gains `token: Option<Uuid>`.
- `src/server/src/ws/conn.rs` — `Pathfind` handler authorizes `token` then derives the footprint; `INVARIANT (scene presence)` rewritten; pinning test extended.
- `src/server/src/data/engine/token.rs` — `ingress_bound_equals_gate_walks_exactly` gains the `gate_walk` half of the parity pairing.

**Client — modified**

- `src/modules/scene-tools/src/controller.svelte.ts` — `ToolContext` gains `role`; drag gesture split into `previewMoves`/`commitMoves`; `footprintFor(id)` extracted; `requestRoute`/`commitRoute` send `token`.
- `src/modules/scene-tools/src/ToolRail.svelte` — projects `role` into the `ToolContext` it assembles.
- `src/client/ui-kit/src/appContext.ts` — `AppContext.pathfind` type gains `token`.
- `src/client/shell/src/lib/Table.svelte` — the `pathfind` wiring arrow forwards `token` (a 4-param arrow silently drops a 5th argument and typecheck cannot see it).
- `src/client/shell/src/lib/worldSession.svelte.ts` — `pathfind` forwards `token`.
- `src/client/core/src/ws-client.ts` — `pathfind` method gains `token`.
- `src/client/core/src/wire.ts` — Zod mirror for `token`.

**Docs/skills — modified**

- `docs/PLAN.md`, `docs/TODO.md`, `docs/POST_WORK_FINDINGS.md`, `docs/CLOSED_BUGS.md`, the campaign spec, `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md`.

---

## D10 — Wall secrecy axis

### Task 1: Per-requester routing wall set

**Files:**
- Modify: `src/server/src/scene/mod.rs:1104-1127` (`move_walls`)
- Test: `src/server/src/scene/mod.rs` (in-module `#[cfg(test)]`)

**Interfaces:**
- Consumes: `region_field`'s per-requester filter pattern (same file), `crate::data::permission::{resolve_access, effective_owner}`, `crate::data::document::{Visibility, WorldRole}`.
- Produces: `SceneEcs::move_walls(&self, scene: Uuid, viewer: Option<Uuid>) -> Vec<vision::Seg>`.

- [ ] **Step 1: Write the failing tests**

Three tests. **NEW fixtures** — `scene_with_grid`, `wall_doc_eng`, `scene_with_public_and_secret_move_walls`, and `scene_with_invisible_barrier_wall` do **not** exist and must be created in this task. Copy the shape of the existing `scene_with_two_walls_one_blocking` (verified at **`mod.rs:4968`**) for scene+wall construction.

```rust
#[test]
fn move_walls_omits_a_gm_only_wall_for_a_player_viewer() {
    let (ecs, scene, player) = scene_with_public_and_secret_move_walls();
    assert_eq!(
        ecs.move_walls(scene, None).len(),
        2,
        "authoritative view carries every blocksMove wall"
    );
    let visible = ecs.move_walls(scene, Some(player));
    assert_eq!(visible.len(), 1, "a gm_only wall is omitted from a player's routing set");
    assert_eq!(
        (visible[0].a, visible[0].b),
        ((100.0, 0.0), (100.0, 200.0)),
        "the surviving wall is the public one"
    );
}

#[test]
fn move_walls_keeps_a_blocks_sight_false_wall_for_a_player() {
    // An invisible BARRIER (blocksSight:false, blocksMove:true) is a PUBLIC document: the router
    // must honor it. Only document-level secrecy filters — the two kinds are not the same axis.
    let (ecs, scene, player) = scene_with_invisible_barrier_wall();
    assert_eq!(
        ecs.move_walls(scene, Some(player)).len(),
        1,
        "a blocksSight:false wall is public geometry and stays in the player's routing set"
    );
}

/// I5 anti-drift: vision and lighting keep the FULL wall set; only routing filters. This is a
/// must-NOT-converge constraint, so it gets a test rather than only a doc comment.
#[test]
fn vision_and_lighting_keep_a_gm_only_wall_that_routing_drops() {
    let (ecs, scene, player) = scene_with_public_and_secret_move_walls();
    assert_eq!(ecs.sight_walls(scene).len(), 2, "sight_walls keeps the gm_only wall (M9b)");
    assert_eq!(ecs.light_walls(scene).len(), 2, "light_walls keeps the gm_only wall (M9b)");
    assert_eq!(
        ecs.move_walls(scene, Some(player)).len(),
        1,
        "only the ROUTING set filters per requester"
    );
}
```

Fixture construction — mark secrecy the way `region_field` actually reads it (`property_overrides["/engine"]`, **not** `permissions.default`; a fixture using `permissions.default` proves nothing about this filter):

```rust
/// NEW. A scene with grid size `cell` and no walls.
fn scene_with_grid(cell: f64) -> (SceneEcs, Uuid) { /* mirror mod.rs:4968's scene setup */ }

/// NEW. A `wall` doc parented to `scene`, blocksMove+blocksSight+blocksLight all true.
fn wall_doc_eng(scene: Uuid, a: (f64, f64), b: (f64, f64)) -> Document { /* … */ }

/// NEW. One public blocksMove wall at x=100 and one `gm_only` blocksMove wall at x=150.
/// Both also carry blocksSight+blocksLight so the I5 test can observe them in the vision sets.
fn scene_with_public_and_secret_move_walls() -> (SceneEcs, Uuid, Uuid) {
    let (mut ecs, scene) = scene_with_grid(100.0);
    let player = Uuid::new_v4();
    let public = wall_doc_eng(scene, (100.0, 0.0), (100.0, 200.0));
    let mut secret = wall_doc_eng(scene, (150.0, 0.0), (150.0, 200.0));
    secret
        .permissions
        .property_overrides
        .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
    ecs.apply_op(&Operation::Create { doc: public });
    ecs.apply_op(&Operation::Create { doc: secret });
    (ecs, scene, player)
}

/// NEW. One wall with blocksSight:false, blocksMove:true, default permissions.
fn scene_with_invisible_barrier_wall() -> (SceneEcs, Uuid, Uuid) { /* … */ }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test move_walls_omits_a_gm_only_wall -- --nocapture`
Expected: FAIL — compile error, `move_walls` takes 1 argument but 2 were supplied.

- [ ] **Step 3: Add the `viewer` parameter and the filter**

```rust
    /// The scene's `blocksMove` wall segments. Mirrors the wall filter in `blocks_move`
    /// (doc_type "wall", parent = scene, `engine.blocksMove == true`, endpoints at
    /// `engine.seg.{x1,y1,x2,y2}`). INVARIANT: same filter as `blocks_move` — any divergence
    /// would allow the pathfinder to route through walls the movement gate would then reject.
    ///
    /// Two-value secrecy contract, identical to `region_field`'s and never a third mode:
    /// `viewer: None` is the AUTHORITATIVE set — used by `execute_move` and by a GM requester;
    /// `viewer: Some(user)` is the PER-REQUESTER set used by the routers, where a wall is included
    /// only when `user` can see the visibility tier declared on its `/engine`. A `gm_only` wall is
    /// therefore absent from a non-GM's route (its geometry cannot be inferred from route shape)
    /// but still blocks at execution, exactly as a secret region springs. Callers MUST pass `None`
    /// for a GM requester.
    ///
    /// Scope: this is the ROUTING wall set only. `sight_walls`/`light_walls` deliberately carry the
    /// full set including `gm_only` walls (M9b full-wall-set invariant) — a wall you cannot see
    /// still blocks your sight, which under-reveals and is correct. Do not unify the two.
    pub(crate) fn move_walls(&self, scene: Uuid, viewer: Option<Uuid>) -> Vec<vision::Seg> {
        let mut out = Vec::new();
        for w in self.world.query::<&SceneEntity>().iter() {
            if w.doc.doc_type != "wall" || w.doc.parent_id != Some(scene) {
                continue;
            }
            let Some(wall) = self.engine_as_cached::<eng::WallEngine>(w.doc.id, &w.doc) else {
                continue;
            };
            if wall.blocks_move != Some(true) {
                continue;
            }
            if let Some(user) = viewer {
                let tier = w
                    .doc
                    .permissions
                    .property_overrides
                    .get("/engine")
                    .copied()
                    .unwrap_or(crate::data::document::Visibility::All);
                // A wall doc never carries an actor link, so the no-join effective-owner
                // resolution is exact — identical to `region_field`'s own call.
                let access = crate::data::permission::resolve_access(
                    user,
                    crate::data::document::WorldRole::Player,
                    &w.doc,
                    crate::data::permission::effective_owner(&w.doc, None),
                );
                if !access.can_see(tier) {
                    continue;
                }
            }
            out.push(vision::Seg {
                a: (wall.seg.x1, wall.seg.y1),
                b: (wall.seg.x2, wall.seg.y2),
            });
        }
        out
    }
```

- [ ] **Step 4: Update every existing call site to pass `None`**

Behavior-preserving. The call sites are exactly three, verified:

- `mod.rs:1173` — inside `navmesh_for`
- `mod.rs:1215` — inside `pathfind`
- `mod.rs:5093` — `move_walls_returns_only_blocks_move_segments_for_the_scene`

`move_exec` does **not** call `move_walls` today (it uses `ecs.blocks_move`); Task 9 adds its first call. Confirm with:

Run: `cd src/server && grep -n "move_walls(" src/server/src`

Do NOT introduce the per-requester call yet — Task 3 does that, so this task's diff reviews as "new capability, zero behavior change".

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src/server && cargo test move_walls && cargo test sight_walls`
Expected: PASS, including the pre-existing `move_walls_returns_only_blocks_move_segments_for_the_scene`.

- [ ] **Step 6: Mutation-verify the tests are non-vacuous**

Temporarily replace the `if !access.can_see(tier)` body with nothing (never skip). Run: `cd src/server && cargo test move_walls_omits_a_gm_only_wall`
Expected: FAIL. Revert and re-run to PASS.

- [ ] **Step 7: Commit**

```bash
cd src/server && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/server/src/scene/mod.rs
git commit -m "feat(server/scene): per-requester routing wall set

move_walls gains a viewer parameter with region_field's two-value secrecy
contract: None is authoritative, Some(user) filters gm_only walls through
the same resolve_access/property_overrides mechanism. All three call sites
pass None, so behavior is unchanged until the routers adopt it.

Vision and lighting keep the full wall set (M9b), pinned by a test."
```

---

### Task 2: Navmesh cache keyed on the requester's wall set

**Files:**
- Modify: `src/server/src/scene/mod.rs:256-262` (cache field doc), `:1129-1180` (`navmesh_for`)
- Test: `src/server/src/scene/mod.rs`

**Interfaces:**
- Consumes: `move_walls(scene, viewer)` (Task 1).
- Produces: `SceneEcs::navmesh_for(&self, scene: Uuid, footprint_radius_cells: f64, walls: &[vision::Seg]) -> Option<Arc<navmesh::NavMesh>>`.

**Why:** `build_navmesh` inflates walls into obstacles, so a mesh is valid only for the wall set it was built from. Keyed on `(scene, quantized_footprint)` alone, the first requester's mesh is served to a requester who sees a different wall subset — a GM's mesh leaking secret-wall geometry into a player's route.

**Spec deviation, recorded:** the spec says "an order-independent hash of the included wall-document **id** set". This task keys on the **segment geometry** instead, because (a) a mesh's validity depends on geometry, not identity, and (b) `move_walls` returns only `Seg`s, so ids are not in hand. Further, it keys on an **exact sorted value**, not a hash — a hash collision here would serve a player a mesh built from the GM's wall set, which is the precise leak D10 closes, and at these set sizes an exact key costs nothing.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn navmesh_for_does_not_share_a_mesh_across_differing_wall_sets() {
    let (ecs, scene, player) = scene_with_public_and_secret_move_walls();
    let gm_walls = ecs.move_walls(scene, None);
    let player_walls = ecs.move_walls(scene, Some(player));
    let gm_mesh = ecs.navmesh_for(scene, 0.4, &gm_walls).expect("gm mesh builds");
    let player_mesh = ecs.navmesh_for(scene, 0.4, &player_walls).expect("player mesh builds");
    assert!(
        !Arc::ptr_eq(&gm_mesh, &player_mesh),
        "a differing wall set must not be served a mesh built from another set"
    );
}

#[test]
fn navmesh_for_shares_a_mesh_across_identical_wall_sets() {
    let (ecs, scene, _player) = scene_with_public_and_secret_move_walls();
    let walls = ecs.move_walls(scene, None);
    let a = ecs.navmesh_for(scene, 0.4, &walls).expect("first build");
    let b = ecs.navmesh_for(scene, 0.4, &walls).expect("second build");
    assert!(Arc::ptr_eq(&a, &b), "an identical wall set reuses the memoized mesh");
}

#[test]
fn navmesh_for_wall_key_is_order_independent() {
    // `hecs` iteration order is not stable, so the same set produced in a different order must
    // still hit the cache.
    let (ecs, scene, _player) = scene_with_public_and_secret_move_walls();
    let walls = ecs.move_walls(scene, None);
    let mut reversed = walls.clone();
    reversed.reverse();
    let a = ecs.navmesh_for(scene, 0.4, &walls).expect("first build");
    let b = ecs.navmesh_for(scene, 0.4, &reversed).expect("reordered lookup");
    assert!(Arc::ptr_eq(&a, &b), "wall-set key is order-independent");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test navmesh_for_does_not_share -- --nocapture`
Expected: FAIL — compile error, `navmesh_for` takes 2 arguments.

- [ ] **Step 3: Re-key the cache and take the wall set as a parameter**

Add the exact key helper:

```rust
/// Exact, order-independent key for a routing wall set — the third component of the navmesh cache
/// key. A mesh is only valid for the wall set it was inflated from, so two requesters share a mesh
/// exactly when they see the same walls. An EXACT sorted key rather than a hash: a collision would
/// serve one requester a mesh built from another's wall set, which is the leak D10 exists to close,
/// and wall counts here are bounded by `MAX_NAVMESH_OBSTACLE_SEGMENTS` so the cost is irrelevant.
/// Sorted on the raw bit patterns so `hecs`'s unstable iteration order cannot cause a miss.
fn wall_set_key(walls: &[vision::Seg]) -> Vec<(u64, u64, u64, u64)> {
    let mut k: Vec<(u64, u64, u64, u64)> = walls
        .iter()
        .map(|s| (s.a.0.to_bits(), s.a.1.to_bits(), s.b.0.to_bits(), s.b.1.to_bits()))
        .collect();
    k.sort_unstable();
    k
}
```

Change the cache field type to `Mutex<HashMap<(Uuid, i64, Vec<(u64, u64, u64, u64)>), Arc<NavMesh>>>` and update its doc comment to state the third component. Then in `navmesh_for`:

```rust
    pub(crate) fn navmesh_for(
        &self,
        scene: Uuid,
        footprint_radius_cells: f64,
        walls: &[vision::Seg],
    ) -> Option<Arc<navmesh::NavMesh>> {
        // Validate BEFORE the quantized key or any cache touch: doing it after the lookup lets
        // NaN/small-negative inputs alias onto an already-cached legitimate mesh.
        if !(0.0..=pathfinding::MAX_FOOTPRINT_CELLS).contains(&footprint_radius_cells) {
            return None;
        }
        // `.round()` is load-bearing (see the field doc): floating-point noise in a client-computed
        // radius must collapse onto the canonical value's entry. A bare `as i64` truncates.
        let quantized = (footprint_radius_cells * 1000.0).round() as i64;
        let key = (scene, quantized, wall_set_key(walls));
        if let Some(cached) = self.navmesh_cache.lock().unwrap().get(&key) {
            return Some(cached.clone());
        }
        // ... existing bounds/cell resolution unchanged ...
        let arc = Arc::new(navmesh::build_navmesh(bounds, cell, walls, footprint_radius_cells)?);
        self.navmesh_cache.lock().unwrap().insert(key, arc.clone());
        Some(arc)
    }
```

Keep `.lock().unwrap()` — the file's existing convention at `mod.rs:1165`/`:1176`/`:616`. Do not switch to `.ok()?`, which would silently return `None` (a routing refusal) on a poisoned mutex.

Remove the internal `self.move_walls(scene)` call — the wall set now arrives as a parameter.

- [ ] **Step 4: Update `navmesh_for`'s call site in `pathfind`**

The `Continuous` branch passes the `walls` binding already resolved in `pathfind`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src/server && cargo test navmesh`
Expected: PASS, including the pre-existing cache/quantization tests at `mod.rs:5150` and `:5165` (the negative-radius and zero-radius quantization cases — these depend on `.round()`, so they also confirm Step 3 kept it).

- [ ] **Step 6: Commit**

```bash
cd src/server && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/server/src/scene/mod.rs
git commit -m "feat(server/scene): key the navmesh cache on the requester's wall set

A mesh is only valid for the wall set it was inflated from, so the cache key
gains an exact order-independent key over the included segments and the wall
set becomes a parameter. Two requesters with identical sets still share one
mesh; differing sets can no longer alias.

Keys on exact sorted bit patterns rather than a hash: a collision would serve
a mesh built from another requester's wall set."
```

---

### Task 3: Routers consume the per-requester wall set

**Files:**
- Modify: `src/server/src/scene/mod.rs:1215` (`pathfind`'s wall resolution)
- Modify: `src/server/src/scene/navmesh.rs` (doc correction on `clip_to_visible_mask`, and `los_smooth`'s `chord_ok` if it repeats the claim)
- Test: `src/server/src/scene/mod.rs`

**Interfaces:**
- Consumes: `move_walls(scene, viewer)` (Task 1), `navmesh_for(scene, footprint, walls)` (Task 2).
- Produces: no new signatures. `pathfind`'s route omits `gm_only` walls for non-GM requesters.

**Fixture preconditions — load-bearing.** A non-GM `pathfind` resolves the scene's own `movement_restriction`, which **defaults to `Visible`**, and an empty non-GM mask makes `find` return `Unreachable` by design. Both fixtures below MUST therefore either give the player a lit token whose mask covers the corridor, or set the scene `unrestricted`. Without that, `.expect(...)` panics for the wrong reason (empty mask, not the wall) and the property under test is silently lost. Scene `bounds` must also be wide enough to contain the GM's detour around the secret wall's endpoints.

- [ ] **Step 1: Write the failing tests**

**NEW fixture** `scene_with_secret_wall_between_two_cells` returns the token id (needed by `execute_move`) and sets `movement_restriction: Unrestricted` so the mask is not the variable under test:

```rust
/// NEW. A scene whose corridor from (50,50) to (250,50) is crossed by a FINITE `gm_only`
/// blocksMove wall at x=150 spanning y∈[0,100]. `movement_restriction: Unrestricted` so the
/// visibility mask is not the variable under test. Bounds are wide enough (400×400) that a detour
/// around the wall's y=100 endpoint exists. Returns the owning user and the token.
fn scene_with_secret_wall_between_two_cells(owner_is_gm: bool) -> (SceneEcs, Uuid, Uuid, Uuid) {
    // -> (ecs, scene, user, token)
}

#[test]
fn non_gm_route_crosses_a_gm_only_wall_that_springs_at_execution() {
    // The router cannot see the secret wall, so it routes straight through it; the executor reads
    // the authoritative set and stops there. Same spring-at-execution shape as a secret region.
    let (ecs, scene, player, token) = scene_with_secret_wall_between_two_cells(false);
    let out = ecs
        .pathfind(player, scene, (50.0, 50.0), &[(250.0, 50.0)], 0.4, false, None)
        .expect("the player's route ignores a wall it cannot see");
    assert!(out.path.len() >= 2, "a route is produced despite the secret wall across it");

    let visible = ecs.visible_cells(player, scene, false);
    let exec = crate::scene::move_exec::execute_move(
        &ecs,
        scene,
        token,
        &out.path,
        MovementRestriction::Unrestricted,
        &visible,
        100.0,
    )
    .expect("execution is admissible");
    assert!(exec.truncated, "the secret wall springs at execution and truncates the move");
}

#[test]
fn gm_route_does_not_cross_a_gm_only_wall() {
    // A GM passes viewer=None, so the secret wall IS in their routing set and no route SEGMENT
    // may cross the wall segment. Asserted structurally via segments_cross — NOT by testing
    // distance from the wall's x-line, which a legitimate detour around a finite wall's endpoint
    // necessarily crosses (and which, at cell size 100, every column-1 cell center sits exactly on).
    let (ecs, scene, gm, _token) = scene_with_secret_wall_between_two_cells(true);
    let out = ecs
        .pathfind(gm, scene, (50.0, 50.0), &[(250.0, 50.0)], 0.4, true, None)
        .expect("a GM route exists (bounds admit a detour around the wall's endpoint)");
    let wall = ((150.0, 0.0), (150.0, 100.0));
    for seg in out.path.windows(2) {
        assert!(
            !crate::scene::segments_cross(seg[0], seg[1], wall.0, wall.1),
            "no GM route segment crosses the wall it can see: {:?}",
            seg
        );
    }
}
```

**Arity note for later tasks:** this test's `execute_move` call uses the current 7-argument form. Task 6 adds `is_gm` (pass `false`) and Task 9 adds `footprint_radius_cells` (pass `0.4`); both tasks must update this call, with the assertion unchanged. Both list `scene/mod.rs` in their Files and `git add`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test gm_only_wall -- --nocapture`
Expected: FAIL — `non_gm_route_crosses_a_gm_only_wall_that_springs_at_execution` fails because the router currently sees the secret wall and detours around it (so `exec.truncated` is false).

- [ ] **Step 3: Thread the per-requester set through `pathfind`**

Replace the unconditional `let walls = self.move_walls(scene);` at `mod.rs:1215`:

```rust
        // Per-requester routing wall set (D10): a non-GM's route omits `gm_only` walls, so their
        // geometry cannot be inferred from route shape. The executor always reads the authoritative
        // set (`None`) and springs a secret wall at execution, exactly as a secret region springs.
        // Hoisted above the engine dispatch so BOTH engines receive the SAME slice — never a forked
        // wall computation (the same discipline `mask` follows above).
        let walls = self.move_walls(scene, if is_gm { None } else { Some(user) });
```

Cite `mask` only in that comment. `region_field` is **not** hoisted — it is computed separately inside each engine branch (`mod.rs:1247` and `:1270`), so naming it here would ship a false statement about neighbouring code.

- [ ] **Step 4: Correct the wall-check documentation in `navmesh.rs`**

`clip_to_visible_mask`'s doc asserts the wall check has "no confidentiality stake" because "walls are public geometry". False for a `gm_only` wall. Replace that paragraph:

```rust
    /// **Two checks, both secrecy-relevant — do not reuse the pre-D10 framing.** The mask check is
    /// a secrecy gate (route ⊆ gate-allowed). The wall check is a router-FIDELITY guarantee for
    /// PUBLIC walls (an undersampled chord between two corner-straddling samples could otherwise
    /// visually cross a wall the true route avoided) AND a secrecy gate whenever the `walls` slice
    /// carries geometry the requester cannot see. The caller closes the secrecy half by
    /// construction: `SceneEcs::pathfind` passes the PER-REQUESTER `move_walls(scene, Some(user))`
    /// set for a non-GM, so a `gm_only` wall never reaches this function on a non-GM's behalf and
    /// cannot truncate their route into a shape that discloses it.
```

Apply the same correction to `los_smooth`/`chord_ok` if it repeats the claim.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src/server && cargo test`
Expected: PASS. Pre-existing router tests are unaffected — they use public walls, where the per-requester set equals the authoritative one.

- [ ] **Step 6: Mutation-verify**

Change the hoisted line to always pass `None`. Run: `cd src/server && cargo test non_gm_route_crosses_a_gm_only_wall`
Expected: FAIL. Revert and re-run to PASS.

- [ ] **Step 7: Commit**

```bash
cd src/server && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/server/src/scene/mod.rs src/server/src/scene/navmesh.rs
git commit -m "fix(server/scene): stop leaking gm_only walls through route shape

pathfind resolves the routing wall set per requester and passes the same
slice to both engines; the executor keeps the authoritative set, so a secret
wall springs at execution instead of bending a player's preview.

Corrects navmesh.rs's claim that the wall check has no confidentiality
stake — true only for public walls."
```

---

## D9 — Player moves are request-only

### Task 4: Refuse non-GM position writes; gate `Create` placement

**Files:**
- Modify: `src/server/src/ws/room.rs:216-…` (the non-GM block in `publish`)
- Modify: `src/server/src/scene/mod.rs:886-895` (`token_move`'s doc comment)
- Modify: `src/server/src/data/engine/token.rs:100-114` (`ingress_bound_equals_gate_walks_exactly` gains the `gate_walk` half)
- Test: `src/server/src/ws/room.rs`

**Interfaces:**
- Consumes: `SceneEcs::token_move(doc_id, changes) -> Option<(Uuid, (f64,f64), (f64,f64))>`, `visible_cells_cached(user, scene, lenient)`, `resolve_scene(scene)`, `Repository::get_explored`, `GridShape::cell_of`.
- Produces: no new signatures.

**Merged from two tasks.** The refusal and the `Create` gate land in ONE commit. They share the same op loop and the same retained machinery, and splitting them leaves an intermediate commit whose retained locals are unused — which `-D warnings` fails on (and `#[allow(dead_code)]` is the wrong attribute for unused *locals* anyway).

**Design note — what goes and what stays.** Refusal is strictly stricter than gating, so the **traversal** machinery goes: the `blocks_move` wall gate, the `line_traversal` traversed-cell set, the per-cell mask membership test over a path, and the coordinate-magnitude block (`TokenEngine::validate` bounds every write at ingress unconditionally; a point placement needs finiteness only). The **point-placement** machinery is RETAINED and repointed at `Create`: the scene-existence refusal, the `MovementRestriction` dispatch, the `visible_cache` memo, and the deferred `revealed_pending`/`get_explored` pass (the explored fetch must still not run under the `scene.read()` guard).

**Why `Create` needs gating:** `room.rs:239` leaves it ungated on the reasoning that `core:create` is already privileged. `data/document.rs:531-532` asserts a world CAN grant `WorldRole::Player` `core:create` on `token`, so arbitrary player placement is reachable by configuration — and placing a token in an unseen room reveals it through that token's own vision, a strictly larger capability than the movement this same block refuses.

- [ ] **Step 1: Write the failing tests**

`publish`'s real signature is six arguments, verified at `room.rs:206-213`:
`publish(&self, repo: &dyn Repository, ctx: &PermissionContext, ops: Vec<Operation>, ts: i64, origin: WriteOrigin)`.

Build fixtures on the existing harness: `repo_with_world()` (`room.rs:1042`) plus the mock `Repository` impl (`room.rs:1087-1210`), and copy the lit-scene/vision setup from `movement_blocked_for_player_crossing_wall_but_gm_bypasses` (`room.rs:1282`). **NEW** fixtures — `room_with_player_owned_token`, `room_with_gm_and_blocking_wall`, `room_with_player_create_capability_and_lit_corner`, `room_with_gm_and_lit_corner`, `room_with_player_create_and_unrestricted_scene`, `token_doc_at` — must be written in this task following those shapes. A fixture with an empty mask would make the Create tests pass for the wrong reason, so each lit-corner fixture must assert its own mask is non-empty before use.

```rust
#[tokio::test]
async fn non_gm_token_position_update_is_refused() {
    let h = room_with_player_owned_token().await;
    let ops = vec![Operation::Update {
        doc_id: h.token,
        changes: vec![
            FieldChange { path: "/engine/x".into(), old: json!(50.0), new: json!(150.0), remove: false },
            FieldChange { path: "/engine/y".into(), old: json!(50.0), new: json!(50.0), remove: false },
        ],
    }];
    let err = h
        .room
        .publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client)
        .await
        .expect_err("a player may not write a token position");
    assert!(matches!(err, DataError::Forbidden), "refused as Forbidden, got {err:?}");
}

#[tokio::test]
async fn gm_token_position_update_still_succeeds_through_a_wall() {
    // A GM places a token where they like, walls included (M9 §5).
    let h = room_with_gm_and_blocking_wall().await;
    let ops = vec![Operation::Update {
        doc_id: h.token,
        changes: vec![FieldChange {
            path: "/engine/x".into(), old: json!(50.0), new: json!(250.0), remove: false,
        }],
    }];
    h.room
        .publish(&h.repo, &h.gm_ctx, ops, 0, WriteOrigin::Client)
        .await
        .expect("a GM position write is unconditional");
}

#[tokio::test]
async fn non_gm_wholesale_engine_write_that_moves_a_token_is_refused() {
    // Post-image detection: `token_move` applies all changes in array order over the committed
    // /engine band, so replacing the whole band cannot smuggle a position change past a per-path check.
    let h = room_with_player_owned_token().await;
    let ops = vec![Operation::Update {
        doc_id: h.token,
        changes: vec![FieldChange {
            path: "/engine".into(),
            old: json!({"x": 50.0, "y": 50.0}),
            new: json!({"x": 150.0, "y": 50.0}),
            remove: false,
        }],
    }];
    let err = h.room.publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client).await
        .expect_err("a wholesale engine write is caught");
    assert!(matches!(err, DataError::Forbidden));
}

#[tokio::test]
async fn non_gm_non_position_token_update_still_succeeds() {
    // The refusal is scoped to POSITION CHANGE, not to token writes generally. `token_move`
    // returns Some for any token with readable x/y, so an `.is_some()` predicate would refuse
    // this — the pre/post comparison is what makes this test pass.
    let h = room_with_player_owned_token().await;
    let ops = vec![Operation::Update {
        doc_id: h.token,
        changes: vec![FieldChange {
            path: "/engine/rotation".into(), old: json!(0.0), new: json!(90.0), remove: false,
        }],
    }];
    h.room.publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client).await
        .expect("a player may still rotate a token they own");
}

#[tokio::test]
async fn non_gm_engine_write_leaving_position_unchanged_succeeds() {
    // The boundary of the pre/post comparison: an /engine write that re-states the SAME x,y is
    // not a move and must be allowed.
    let h = room_with_player_owned_token().await;
    let ops = vec![Operation::Update {
        doc_id: h.token,
        changes: vec![FieldChange {
            path: "/engine/x".into(), old: json!(50.0), new: json!(50.0), remove: false,
        }],
    }];
    h.room.publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client).await
        .expect("a no-op position write is not a move");
}

#[tokio::test]
async fn non_gm_token_create_outside_the_mask_is_refused() {
    let h = room_with_player_create_capability_and_lit_corner().await;
    let ops = vec![Operation::Create { doc: token_doc_at(h.scene, 500.0, 500.0) }];
    let err = h.room.publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client).await
        .expect_err("placement in fog is refused");
    assert!(matches!(err, DataError::Forbidden));
}

#[tokio::test]
async fn non_gm_token_create_inside_the_mask_succeeds() {
    let h = room_with_player_create_capability_and_lit_corner().await;
    let ops = vec![Operation::Create { doc: token_doc_at(h.scene, 50.0, 50.0) }];
    h.room.publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client).await
        .expect("placement in a visible cell is allowed");
}

#[tokio::test]
async fn gm_token_create_anywhere_succeeds() {
    let h = room_with_gm_and_lit_corner().await;
    let ops = vec![Operation::Create { doc: token_doc_at(h.scene, 500.0, 500.0) }];
    h.room.publish(&h.repo, &h.gm_ctx, ops, 0, WriteOrigin::Client).await
        .expect("a GM places a token anywhere");
}

#[tokio::test]
async fn unrestricted_scene_ungates_non_gm_token_create() {
    let h = room_with_player_create_and_unrestricted_scene().await;
    let ops = vec![Operation::Create { doc: token_doc_at(h.scene, 500.0, 500.0) }];
    h.room.publish(&h.repo, &h.player_ctx, ops, 0, WriteOrigin::Client).await
        .expect("Unrestricted ungates placement, as it ungates movement");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test non_gm_token_position_update_is_refused non_gm_token_create_outside_the_mask -- --nocapture`
Expected: FAIL — the position move is currently *allowed* (a legal in-mask move) and `Create` is ungated, so both `expect_err` calls panic.

- [ ] **Step 3: Replace the traversal gate with a refusal, and gate `Create`**

Hoist one `scene.read()` guard outside the op loop. The predicate compares pre- and post-image — `token_move` performs **no** such comparison itself (verified at `mod.rs:912-934`: it returns `Some` for any token doc with readable `/engine/x,y`), so `.is_some()` would refuse every non-GM token write:

```rust
            // D9: a non-GM may not CHANGE a token's position. Gated movement is request-only and
            // server-executed (`ClientMsg::MoveRequest` → `execute_move`), the only path that can
            // gate each step, arrest a token partway, and stream the authoritative trajectory. A
            // client-authored position write can do none of those, so it is refused rather than
            // validated — strictly stricter than the traversal gate this replaces, leaving
            // `execute_move` the SOLE implementation of the per-cell traversal decision (I2).
            //
            // `token_move` yields (scene, committed_start, post_image_end) over the whole /engine
            // band with all changes applied in array order, so a wholesale `/engine` write or
            // duplicate `/engine/x` entries cannot present a safe target while committing a moved
            // one. It does NOT itself test whether the position changed, so the comparison is here:
            // a write that re-states the same coordinates is not a move. Bitwise inequality, not an
            // epsilon window — an epsilon would grant a free sub-threshold teleport per op.
            //
            // GMs are exempt: a GM places a token where they choose, walls included (M9 §5).
            for op in &ops {
                if let Operation::Update { doc_id, changes } = op {
                    if let Some((_, a0, a1)) = scene.token_move(*doc_id, changes) {
                        if a0 != a1 {
                            return Err(DataError::Forbidden);
                        }
                    }
                }
                if let Operation::Create { doc } = op {
                    // A created token's position is authorized against the SAME mask accessor the
                    // movement gate used. Placement was ungated on the reasoning that `core:create`
                    // is privileged, but a world can grant it to Player (data/document.rs:531), and
                    // placing a token in an unseen cell reveals that area through the new token's
                    // own vision — a strictly larger capability than the movement refused above.
                    // Center-cell only: a placement is a point, not a traversal.
                    if doc.doc_type != "token" {
                        continue;
                    }
                    let Some(scene_id) = doc.parent_id else { continue };
                    let Some(eng) = doc.engine.as_ref().and_then(|v| {
                        serde_json::from_value::<crate::data::engine::TokenEngine>(v.clone()).ok()
                    }) else {
                        return Err(DataError::Forbidden); // unparseable engine ⇒ fail closed
                    };
                    if !eng.x.is_finite() || !eng.y.is_finite() {
                        return Err(DataError::Forbidden);
                    }
                    // Scene-existence refusal (parity axis 6): an absent entry means no scene
                    // document, so no authored cell size exists to index the mask against.
                    let Some(cell) = scene.scene_grid_sizes().get(&scene_id).copied() else {
                        return Err(DataError::Forbidden);
                    };
                    let settings = scene.resolve_scene(scene_id);
                    let lenient = settings.partial_cell_leniency;
                    let target = scene.resolve_grid_shape(scene_id, cell).cell_of((eng.x, eng.y));
                    match settings.movement_restriction {
                        crate::scene::MovementRestriction::Unrestricted => {}
                        crate::scene::MovementRestriction::Visible => {
                            let mask = visible_cache
                                .entry((scene_id, lenient))
                                .or_insert_with(|| {
                                    scene.visible_cells_cached(ctx.user_id, scene_id, lenient)
                                });
                            if !mask.contains(&target) {
                                return Err(DataError::Forbidden);
                            }
                        }
                        crate::scene::MovementRestriction::Revealed => {
                            let mask = visible_cache
                                .entry((scene_id, lenient))
                                .or_insert_with(|| {
                                    scene.visible_cells_cached(ctx.user_id, scene_id, lenient)
                                })
                                .clone();
                            // Explored needs an async fetch, which must not run under the scene
                            // read guard — defer exactly as the movement gate did.
                            revealed_pending.push((scene_id, [target].into_iter().collect(), mask));
                        }
                    }
                }
            }
```

`visible_cells_cached` — **not** `visible_cells`. The retained memo at `room.rs:331` uses the cached accessor; swapping to the uncached primitive forks the mask source and perturbs the existing recompute-count assertions.

Delete the traversal code: the `blocks_move` call, the `grid.line_traversal` / `move_cells` set, the per-cell `visible.contains` test, and the coordinate-magnitude block. Keep the post-lock `revealed_pending` loop that fetches `get_explored` and checks `cells ⊆ mask ∪ explored`, failing closed on `Err`.

- [ ] **Step 4: Rewrite `token_move`'s stale doc comment**

Its first paragraph (`mod.rs:886-895`) describes a "client-driven drag-move path ... pending Task 8/9" and frames the function as feeding a collision check. After this task, `publish`'s only use is the refusal predicate, and the referenced drag path is gone. Rewrite as a present-tense statement of that role, dropping the process-meta reference. Keep the post-image/last-write-wins paragraph — it remains accurate and load-bearing.

- [ ] **Step 5: Disposition the four coordinate-bound tests explicitly**

These four exist and must each be handled by name — Step 6's general instruction does **not** apply to them:

| Test | `room.rs` | Behavior after the refusal | Disposition |
|---|---|---|---|
| `publish_move_gate_rejects_an_over_magnitude_start_coordinate` | 2807 | still passes, but sourced from the refusal, not `MAX_GATE_WALK_COORD` | **delete** |
| `publish_move_gate_rejects_over_magnitude_coordinate_on_a_square_scene` | 2855 | same | **delete** |
| `publish_move_gate_rejects_over_magnitude_coordinate_on_a_hex_scene` | 2889 | same | **delete** |
| `publish_move_gate_admissibility_bound_equals_gate_walks` | 2915 | **fails loudly** — ends on `.expect("a coordinate exactly AT the bound is admissible on both gates")` for a player publish | **delete** |

Converting any of them to "assert refusal" would leave a green, plausibly-named test asserting `Forbidden` twice — passing regardless of `MAX_GATE_WALK_COORD`'s value or a `>`/`>=` flip. That is worse than deletion because it reads as live coverage.

The property survives independently, verified: `ingress_bound_equals_gate_walks_exactly` (`token.rs:100`) reads `crate::scene::move_exec::MAX_GATE_WALK_COORD` by symbol and asserts at-bound-admissible / over-bound-refused on the ingress side; `gate_walk_fails_closed_on_coordinate_over_the_magnitude_bound` (`move_exec.rs:1820`) and `gate_walk_accepts_coordinate_at_the_magnitude_bound` (`:1831`) pin the walk side. Name both owners in the deletion commit body.

Close the one real residual gap — no single test asserts the two senses are *equal* in one place — by extending `ingress_bound_equals_gate_walks_exactly`:

```rust
    // The walk side of the same bound, asserted here so one test pins the EQUALITY of the two
    // senses rather than leaving it inferred across two files.
    let at = crate::scene::move_exec::MAX_GATE_WALK_COORD;
    assert!(
        crate::scene::move_exec::gate_walk(&[(at - 100.0, 0.0), (at, 0.0)], 100.0).is_some(),
        "gate_walk admits a coordinate exactly AT the bound"
    );
    assert!(
        crate::scene::move_exec::gate_walk(&[(at - 100.0, 0.0), (at + 1.0, 0.0)], 100.0).is_none(),
        "gate_walk refuses a coordinate over the bound"
    );
```

- [ ] **Step 6: Run the tests; audit each pre-existing failure**

Run: `cd src/server && cargo test`
Expected: the nine new tests PASS.

Other pre-existing tests that asserted **a legal player drag succeeds** now fail — those test the deleted capability; convert them to assert refusal. **Audit rule:** before converting any test, confirm the assertion being replaced was "a player's in-mask drag is allowed" (a capability D9 removes) and **not** a *coupling* property that merely used a successful drag as its vehicle. The four tests in Step 5 are the second kind; if any other test turns out to be, disposition it explicitly rather than converting it. Tests asserting a player's *illegal* drag is refused keep passing unchanged.

- [ ] **Step 7: Verify no other non-GM position path survives**

Run:
```bash
cd src/server && grep -rn "token_move" src/server/src
cd /c/Dev/Shadowcat && grep -rn '"/engine/x"\|"/engine/y"' src/modules src/client --include=*.ts --include=*.svelte | grep -v "\.test\."
```
Confirm every `token_move` caller is either this refusal or `execute_move`'s scene derivation, and for each client-side writer note whether it is GM-only, now expected to fail (and handled in Task 5), or needs re-pointing at `moveRequest`. Record the findings in the commit body.

- [ ] **Step 8: Mutation-verify**

Change `if a0 != a1` to `if true`. Run: `cd src/server && cargo test non_gm_non_position_token_update_still_succeeds`
Expected: FAIL (proving the comparison is load-bearing). Then change `!mask.contains(&target)` to `false` and run `cargo test non_gm_token_create_outside_the_mask` — expected FAIL. Revert both and re-run to PASS.

- [ ] **Step 9: Commit**

```bash
cd src/server && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/server/src/ws/room.rs src/server/src/scene/mod.rs src/server/src/data/engine/token.rs
git commit -m "feat(server/ws): player token movement and placement are gated

A non-GM Update that CHANGES a token position is refused (a write re-stating
the same coordinates is not a move); players move only via MoveRequest, the
sole path that gates each step, can arrest a token partway, and streams the
authoritative trajectory. Create placement is authorized against the same
mask accessor — a world can grant Player core:create on token, and placing a
token in an unseen room reveals it through that token's vision.

GMs are exempt on both paths (M9 §5). Deletes publish's duplicated traversal
gate, leaving execute_move the sole per-cell traversal decision.

Deletes four publish-based coordinate-bound tests whose subject (publish's own
magnitude check) is gone; the bound-parity property is owned by
token.rs::ingress_bound_equals_gate_walks_exactly and move_exec.rs's
gate_walk tests, and the former now pins both senses in one place."
```

---

### Task 5: Client — player drag becomes one move request

**Files:**
- Modify: `src/modules/scene-tools/src/controller.svelte.ts:18-58` (`ToolContext` gains `role`), `:386-394` (`footprintFor` extraction), `:768-863` (the drag tool)
- Modify: `src/modules/scene-tools/src/ToolRail.svelte` (project `role` into the assembled `ToolContext`)
- Test: `src/modules/scene-tools/src/controller.test.ts`

**Interfaces:**
- Consumes: `ctx.moveRequest(scene, tokenId, path)`, `ctx.pathfind(scene, start, waypoints, footprintRadius)`, `ctx.scene.previewOverlay`, `ctx.onMoveOutcome`.
- Produces: `ToolContext.role: WorldRole`; `previewMoves(delta)` and `commitMoves(delta)` replacing `sendMoves(delta)`; `footprintFor(id): number`.

**Two seams, then the restructure.**

`ToolContext` (`controller.svelte.ts:18`) has **no** `role` field — verified, `grep -n role controller.svelte.ts` returns nothing. `ToolContext` is assembled in **`ToolRail.svelte`** (not `Table.svelte`), which already reads `ctx.role` from AppContext for its own `const isGm = ctx.role === "gm";`. So the fix is: add `role: WorldRole` to the `ToolContext` interface, and add `role: ctx.role` to the object `ToolRail.svelte` projects. No new plumbing.

The drag lifecycle is the substantive fix. `sendMoves` is called **twice**: from the throttled `onPointerMove` at `:851` ("leading-edge coalesced stream", `DRAG_THROTTLE_MS = 50`) and again from `onPointerUp` at `:859`. Role-branching alone would fire one `pathfind` + one `moveRequest` **per 50 ms tick per selected token**. Each `moveRequest` is server-executed and commits a position, and `execute_move` holds a per-token in-flight lock, so the first mid-drag tick wins and every later request — including the pointer-up one carrying the actual destination — is refused. The token would animate to a stale intermediate point. So the gesture splits: **preview on move, commit once on release.**

- [ ] **Step 1: Write the failing tests**

```ts
it("a non-GM drag issues exactly one moveRequest per selected token, on release", async () => {
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }, { id: "t2", x: 100, y: 0 }] });
  h.select(["t1", "t2"]);
  await h.dragWithTicks({ dx: 100, dy: 0, ticks: 5 }); // 5 throttle windows
  expect(h.moveRequests).toEqual([
    { scene: "s1", token: "t1", goal: [100, 0] },
    { scene: "s1", token: "t2", goal: [200, 0] },
  ]);
  expect(h.dispatchedOps).toEqual([]);
});

it("a GM drag issues one batched update and zero move requests", async () => {
  const h = harness({ role: "gm", tokens: [{ id: "t1", x: 0, y: 0 }] });
  h.select(["t1"]);
  await h.drag({ dx: 100, dy: 0 });
  expect(h.moveRequests).toEqual([]);
  expect(h.dispatchedOps).toEqual([
    { op: "update", doc_id: "t1", changes: [
      { path: "/engine/x", old: 0, new: 100 },
      { path: "/engine/y", old: 0, new: 0 },
    ] },
  ]);
});

it("a non-GM drag does not move the rendered token before a MoveStream arrives", async () => {
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }] });
  h.select(["t1"]);
  await h.drag({ dx: 100, dy: 0 });
  expect(h.documents.get("t1").engine.x).toBe(0);
  expect(h.previewOverlayCalls.length).toBeGreaterThan(0);
});

it("a refused player move surfaces feedback rather than failing silently", async () => {
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }], moveRequestRejects: true });
  h.select(["t1"]);
  await h.drag({ dx: 100, dy: 0 });
  expect(h.moveOutcomes.length).toBeGreaterThan(0);
  expect(h.previewOverlayCalls.at(-1)).toEqual([]); // preview cleared
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `pnpm --filter @shadowcat/scene-tools test -- controller`
Expected: FAIL — the player case produces update ops, moves the optimistic document, and (once role-branched naively) issues 5+ move requests instead of 1.

- [ ] **Step 3: Add the `role` seam**

In `controller.svelte.ts`, add to the `ToolContext` interface:

```ts
  /** The caller's per-world role. Movement authority is role-asymmetric: a GM writes a token
   * position directly, a player's move is request-only and server-executed. */
  role: WorldRole;
```

In `ToolRail.svelte`'s projected object, add `role: ctx.role,`. Import `WorldRole` from `@shadowcat/core` in the controller. `ctx.role` compares against the `WorldRole` serde representation — use the same `=== "gm"` form `ToolRail.svelte` already uses for `isGm`, not a guessed literal.

- [ ] **Step 4: Split the gesture**

Replace `sendMoves` with two functions and repoint both call sites:

```ts
  /** Per-token footprint radius in cells. Single source shared with `resolveFootprint()` so the
   * preview and the drag commit cannot derive different sizes. */
  const footprintFor = (id: string): number => {
    const doc = ctx.documents.get(id);
    const eff = doc ? resolveTokenActor(doc, ctx.documents) : null;
    return eff ? footprintRadius(eff) : 0.4;
  };

  /** Drag feedback only — never a document write and never a move request. A player's token must
   * not appear to move until the server executes it (no optimistic prediction for a gated move). */
  const previewMoves = (delta: Point): void => {
    const pts: number[] = [];
    for (const [, o] of origins) {
      const target = ctx.scene.snap({ x: o.x + delta.x, y: o.y + delta.y });
      pts.push(o.x, o.y, target.x, target.y);
    }
    ctx.scene.previewOverlay(
      pts.length > 0 ? [{ points: pts, closed: false, stroke: { color: ROUTE_COLOR, width: 2 }, fill: null }] : [],
    );
  };

  /** Commit the gesture, exactly once, on release. A GM writes the position directly — a GM places
   * a token where they choose, walls included. A player's move is request-only: each selected token
   * gets its own pathfind + moveRequest, and its rendered position advances only when the resulting
   * MoveStream arrives. Per-token rather than batched: moveRequest is per-token on the wire and the
   * server gates each token independently, so one token arresting while another completes is
   * correct. */
  const commitMoves = (delta: Point): void => {
    if (ctx.role === "gm") {
      const ops: WireOperation[] = [];
      for (const [id, o] of origins) {
        const target = ctx.scene.snap({ x: o.x + delta.x, y: o.y + delta.y });
        const eng = ctx.documents.get(id)?.engine as { x?: number; y?: number } | undefined;
        ops.push({ op: "update", doc_id: id, changes: [
          { path: "/engine/x", old: eng?.x ?? null, new: target.x },
          { path: "/engine/y", old: eng?.y ?? null, new: target.y },
        ] });
      }
      if (ops.length > 0) ctx.dispatchIntent(ops);
      return;
    }
    const scene = activeScene(ctx);
    if (!scene || !ctx.pathfind || !ctx.moveRequest) return;
    const pathfind = ctx.pathfind;
    const moveRequest = ctx.moveRequest;
    for (const [id, o] of origins) {
      const target = ctx.scene.snap({ x: o.x + delta.x, y: o.y + delta.y });
      pathfind(scene.id, [o.x, o.y], [[target.x, target.y]], footprintFor(id), id)
        .then((result) => {
          if (result.path.length >= 2) return moveRequest(scene.id, id, result.path);
        })
        .catch(() => {
          // Post-D9 a refusal is the NORMAL outcome for a player (out-of-mask destination, a
          // springing secret wall, an arrest region, the per-token in-flight lock). Swallowing it
          // would leave the token silently not moving with no explanation, indistinguishable from
          // a hung connection, so it goes through the existing outcome seam.
          ctx.scene.previewOverlay([]);
          ctx.onMoveOutcome?.({ token: id, refused: true });
        });
    }
  };
```

Repoint the call sites: `onPointerMove`'s throttled branch calls `previewMoves(delta)`; `onPointerUp` calls `commitMoves(...)` and then clears the overlay. Have `resolveFootprint()` delegate to `footprintFor` **only in its single-selection branch** — its existing "selection size is not exactly 1 ⇒ 0.4" guard is unchanged, since a multi-token preview has no single footprint.

Verify the `onMoveOutcome` payload shape against `appContext.ts:143-146` and use it as-is rather than inventing fields.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `pnpm --filter @shadowcat/scene-tools test -- controller && pnpm -r typecheck`
Expected: PASS. Typecheck is a separate requirement — Vitest strips types via esbuild and will not catch a signature error.

- [ ] **Step 6: Commit**

```bash
pnpm -r test && pnpm -r typecheck && pnpm lint
git add src/modules/scene-tools/src/controller.svelte.ts src/modules/scene-tools/src/ToolRail.svelte src/modules/scene-tools/src/controller.test.ts
git commit -m "feat(client/scene-tools): player drag commits one move request on release

The drag gesture splits into preview (on move) and commit (on release): a
player's drag previously would have fired one pathfind+moveRequest per 50ms
throttle tick, and execute_move's per-token in-flight lock refuses all but
the first, landing the token on a stale intermediate point.

A player's token now advances only on the server's MoveStream — no optimistic
write for a gated move — and a refusal surfaces through onMoveOutcome instead
of failing silently, since post-D9 refusal is the normal outcome. A GM's drag
is unchanged: one batched direct position write.

ToolContext gains role, projected from AppContext in ToolRail."
```

---

## D8 — GM gate-exemption unification

### Task 6: GMs bypass every gameplay gate in `execute_move`

**Files:**
- Modify: `src/server/src/scene/move_exec.rs` (`execute_move`)
- Modify: `src/server/src/ws/room.rs:553-557` (call site + the stale comment)
- Modify: `src/server/src/scene/mod.rs` (Task 3's test-module `execute_move` call gains `is_gm: false`)
- Test: `src/server/src/scene/move_exec.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `execute_move(ecs, scene, token, path, restriction, visible, cell, is_gm: bool) -> Result<MoveOutcome, MoveReject>`.

**Fixture note.** `execute_move` requires `path[0]` to equal the token's committed ECS position within `EPS`. Every **NEW** fixture below must place its token at the path's first point — the snippets start at `(50.0, 50.0)`, whereas existing fixtures in this file place tokens at `(0.0, 0.0)`. State each fixture's token position explicitly.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn gm_move_crosses_a_blocks_move_wall_untruncated() {
    let (ecs, scene, token) = scene_with_wall_across_the_path(); // token committed at (50,50)
    let path = [(50.0, 50.0), (150.0, 50.0), (250.0, 50.0)];
    let out = execute_move(
        &ecs, scene, token, &path, MovementRestriction::Unrestricted, &empty_mask(), 100.0, true,
    )
    .expect("a GM move is admissible");
    assert!(!out.truncated, "a GM move is not truncated by a wall (M9 §5)");
    assert_eq!(
        out.render_path.last().copied(),
        Some((250.0, 50.0)),
        "the GM lands at the requested destination"
    );
}

#[test]
fn gm_move_ignores_impassable_and_arrest_regions() {
    let (ecs, scene, token) = scene_with_impassable_then_arrest_region();
    let path = [(50.0, 50.0), (150.0, 50.0), (250.0, 50.0)];
    let out = execute_move(
        &ecs, scene, token, &path, MovementRestriction::Unrestricted, &empty_mask(), 100.0, true,
    )
    .expect("admissible");
    assert!(!out.truncated, "neither impassable nor arrest stops a GM");
}

#[test]
fn non_gm_move_is_still_blocked_by_the_same_wall() {
    // The exemption must not widen: pin that non-GM behavior is unchanged.
    let (ecs, scene, token) = scene_with_wall_across_the_path();
    let out = execute_move(
        &ecs, scene, token, &[(50.0, 50.0), (150.0, 50.0)],
        MovementRestriction::Unrestricted, &empty_mask(), 100.0, false,
    )
    .expect("admissible");
    assert!(out.truncated, "a non-GM is still stopped by the wall");
}

#[test]
fn gm_move_is_still_refused_beyond_the_coordinate_bound() {
    // I1: a GM bypasses gameplay gates and NO resource guard.
    let (ecs, scene, token) = scene_with_wall_across_the_path();
    let over = MAX_GATE_WALK_COORD + 1.0;
    let err = execute_move(
        &ecs, scene, token, &[(50.0, 50.0), (over, 50.0)],
        MovementRestriction::Unrestricted, &empty_mask(), 100.0, true,
    )
    .expect_err("a resource guard is never exempted");
    assert!(matches!(err, MoveReject::TooLong), "got {err:?}");
}

#[test]
fn gm_move_still_accrues_terrain_cost() {
    // Cost is information, not a gate — accrual is independent of the exemption.
    let (ecs, scene, token) = scene_with_terrain_multiplier_3();
    let out = execute_move(
        &ecs, scene, token, &[(50.0, 50.0), (150.0, 50.0)],
        MovementRestriction::Unrestricted, &empty_mask(), 100.0, true,
    )
    .expect("admissible");
    assert!(out.cost >= 3.0, "terrain still accrues for a GM, got {}", out.cost);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test gm_move_crosses_a_blocks_move_wall -- --nocapture`
Expected: FAIL — compile error (arity) first; once the parameter exists, the wall and region assertions fail.

- [ ] **Step 3: Add the exemption**

A plain `is_gm` flag with early-outs, not a shared profile struct: after D9 there is exactly one gate, and a shared symbol with a single consumer is indirection with no second party to keep honest.

```rust
    // Gameplay gates apply to non-GMs only. A GM may make an illegal move: they move with or
    // without pathfinding, and a placement lands where asked (M9 §5), matching `publish`'s GM
    // position write. Resource guards — `gate_walk`'s MAX_GATE_WALK_COORD / MAX_GATE_WALK_SAMPLES,
    // non-finite refusal, and the scene-existence refusal — are NOT exempted for a GM (I1).
    let check_walls = !is_gm;
    let check_regions = !is_gm;
    let check_mask = !is_gm && !matches!(restriction, MovementRestriction::Unrestricted);
```

Guard step 1 with `if check_walls && ecs.blocks_move(scene, prev, next)`. In step 3, guard only the *stopping* decisions and leave cost accrual unconditional:

```rust
        let next_cell = to_cell(next);
        if next_cell != last_region_cell {
            if check_regions && regions.is_impassable(next_cell) {
                stopped_early = true;
                break;
            }
            // Cost accrues regardless of the exemption: it is information, not a gate.
            cost += regions.terrain_multiplier(next_cell);
            if check_regions && regions.is_arrest(next_cell) {
                stop_idx = i;
                stopped_early = true;
                break;
            }
            last_region_cell = next_cell;
        }
```

- [ ] **Step 4: Update both call sites and invert the stale comment**

`room.rs` — pass `is_gm` (the same role check that decides `restriction`) and replace the comment at `:553-557`, which currently instructs against exactly this change:

```rust
            // GMs are exempt from every gameplay gate here — walls, mask, impassable and arrest —
            // per the M9 design spec's GM "ignore walls" override (M9 §5), matching `publish`'s own
            // GM position write. Resource guards (`gate_walk`'s coordinate/sample bounds, the
            // scene-existence refusal) stay unconditional for a GM.
```

`scene/mod.rs` — Task 3's `non_gm_route_crosses_a_gm_only_wall_that_springs_at_execution` gains `false` as the eighth argument; its assertion is unchanged.

- [ ] **Step 5: Run the tests; audit each pre-existing failure**

Run: `cd src/server && cargo test`
Expected: PASS. Existing tests asserting a GM is wall-blocked or arrest-stopped now fail — they encode the regression this task fixes (the M9 spec at `2026-06-22-m9-walls-vision-fog-design.md:103` grants the override). For each, confirm from the assertion's intent that GM wall/region blocking specifically was the subject, then update it to the M9 §5 behavior. Note `room.rs:1469`'s existing test already asserts the correct GM bypass for `publish` and should keep passing.

- [ ] **Step 6: Commit**

```bash
cd src/server && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/server/src/scene/move_exec.rs src/server/src/ws/room.rs src/server/src/scene/mod.rs
git commit -m "fix(server/scene): restore the GM ignore-walls override in execute_move

execute_move enforced walls and impassable/arrest against GMs, diverging from
the M9 design spec's GM override (M9 §5) and from publish's own GM behavior.
GMs now bypass every gameplay gate and land at the requested destination;
resource guards stay unconditional, and terrain cost still accrues."
```

---

## D4 — Footprint-aware authoritative gate

### Task 7: Server-side footprint resolver mirroring the client

**Files:**
- Modify: `src/server/src/scene/mod.rs` (new `resolve_token_footprint`, private `token_shape_and_size`)
- Test: `src/server/src/scene/mod.rs`

**Interfaces:**
- Consumes: `eng::{TokenEngine, ActorEngine, TokenOverrides, Size}` (`data/engine/token.rs:140-143` `Size {w,h}`, `:224-231` `ActorEngine {size, shape}`, `:120-135` `TokenOverrides`), `self.actors`, the `resolveTokenActor` join `token_vision_floors` implements (`mod.rs:1529-1548`).
- Produces: `SceneEcs::resolve_token_footprint(&self, token: Uuid) -> Option<f64>` and `pub(crate) const DEFAULT_FOOTPRINT_RADIUS_CELLS: f64 = 0.4;`

**Returns `Option`, deliberately.** An out-of-range radius must be a refusal, not a silent clamp: clamping to `MAX_FOOTPRINT_CELLS` (64.0, `pathfinding.rs:535`) would route and gate a map-scale token as a 64-cell disc, letting it squeeze through gaps its real footprint cannot enter — a geometric fail-open. `None` propagates to the caller as a refusal with a `tracing::warn!` diagnostic, so a legitimately-authored oversized token is diagnosable rather than silently immobile.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn footprint_radius_mirrors_the_client_formula() {
    // Mirrors footprintRadius (src/client/core/src/actor.ts:177-180):
    //   circle ⇒ max(w,h)/2 ; square (and any other shape) ⇒ hypot(w,h)/2
    // Representative + boundary cases; `Size` is a free {w,h} pair, so there is no finite domain
    // to enumerate exhaustively.
    let cases = [
        ("square", 1.0, 1.0, std::f64::consts::SQRT_2 / 2.0),
        ("square", 2.0, 2.0, std::f64::consts::SQRT_2),
        ("square", 1.0, 2.0, 5.0f64.sqrt() / 2.0),
        ("circle", 1.0, 1.0, 0.5),
        ("circle", 2.0, 3.0, 1.5),
        // A shape outside {"circle","square"} takes the square branch, mirroring the client's
        // `shape === "circle" ? … : hypot(…)` fallthrough.
        ("blob", 1.0, 1.0, std::f64::consts::SQRT_2 / 2.0),
    ];
    for (shape, w, h, expected) in cases {
        let (ecs, token) = scene_with_linked_token_sized(shape, w, h);
        let got = ecs.resolve_token_footprint(token).expect("in-range");
        assert!((got - expected).abs() < 1e-12, "shape={shape} w={w} h={h}: want {expected}, got {got}");
    }
}

#[test]
fn footprint_radius_falls_back_to_the_client_default_for_an_actorless_token() {
    let (ecs, token) = scene_with_raw_token_no_actor();
    assert_eq!(
        ecs.resolve_token_footprint(token),
        Some(DEFAULT_FOOTPRINT_RADIUS_CELLS),
        "an actorless token uses the same 0.4 default the client's resolveFootprint uses"
    );
}

#[test]
fn footprint_radius_honors_a_per_token_size_override() {
    let (ecs, token) = scene_with_linked_token_overriding_size("circle", 4.0, 4.0);
    assert!((ecs.resolve_token_footprint(token).expect("in-range") - 2.0).abs() < 1e-12);
}

#[test]
fn footprint_radius_refuses_an_oversized_token_rather_than_clamping() {
    // w=h=1000 ⇒ ~707 cells, far over MAX_FOOTPRINT_CELLS (64.0). Clamping would gate a
    // map-scale token as a 64-cell disc — a geometric fail-open.
    let (ecs, token) = scene_with_linked_token_sized("square", 1000.0, 1000.0);
    assert_eq!(ecs.resolve_token_footprint(token), None, "an out-of-range footprint is refused");
}

#[test]
fn footprint_radius_admits_a_token_exactly_at_the_bound() {
    let at = pathfinding::MAX_FOOTPRINT_CELLS; // 64.0
    let (ecs, token) = scene_with_linked_token_sized("circle", at * 2.0, at * 2.0);
    assert_eq!(ecs.resolve_token_footprint(token), Some(at), "AT the bound is admissible");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test footprint_radius -- --nocapture`
Expected: FAIL — `resolve_token_footprint` does not exist.

- [ ] **Step 3: Implement the resolver**

```rust
/// The footprint radius used when no effective actor resolves. Mirrors the client's
/// `resolveFootprint` fallback (`src/modules/scene-tools/src/controller.svelte.ts:393`).
/// PARITY-BOUND, not a fail-closed choice: it is more permissive than a 1×1 square's 0.707, and
/// changing it here without changing the client re-forks the router and the gate. Change both or
/// neither.
pub(crate) const DEFAULT_FOOTPRINT_RADIUS_CELLS: f64 = 0.4;

    /// A token's bounding-disc radius in GRID UNITS (cells). Mirrors the client's `footprintRadius`
    /// formula (`src/client/core/src/actor.ts:177`): a circle uses `max(w,h)/2`, any other shape its
    /// half-diagonal `hypot(w,h)/2` (conservative enclosure). Effective-actor resolution mirrors
    /// `resolveTokenActor` via the SAME join `token_vision_floors` implements: a LINKED token
    /// resolves the shared actor and applies the per-token override whitelist; a dangling link
    /// ignores overrides; an INSTANCED token uses its embedded copy and overrides do not apply.
    ///
    /// `None` means REFUSE — the derived radius is outside `[0, MAX_FOOTPRINT_CELLS]`, or the
    /// stored size is degenerate. Callers must fail closed, never substitute a default: clamping an
    /// oversized token to the bound would route and gate it as a smaller disc, letting it enter
    /// gaps its real footprint cannot (a geometric fail-open).
    ///
    /// DELIBERATE DIVERGENCE from the client on degenerate input: the client's `footprintRadius`
    /// has no finite/sign guard and propagates `NaN` (rejected later by `find`'s range check),
    /// whereas this refuses. Both fail closed; only the mechanism differs.
    pub(crate) fn resolve_token_footprint(&self, token: Uuid) -> Option<f64> {
        let Some((shape, size)) = self.token_shape_and_size(token) else {
            return Some(DEFAULT_FOOTPRINT_RADIUS_CELLS);
        };
        let (w, h) = (size.w, size.h);
        if !w.is_finite() || !h.is_finite() || w < 0.0 || h < 0.0 {
            tracing::warn!(?token, w, h, "token size is degenerate; refusing a footprint");
            return None;
        }
        let r = if shape == "circle" { w.max(h) / 2.0 } else { w.hypot(h) / 2.0 };
        if !(0.0..=pathfinding::MAX_FOOTPRINT_CELLS).contains(&r) {
            tracing::warn!(?token, r, "token footprint exceeds MAX_FOOTPRINT_CELLS; refusing");
            return None;
        }
        Some(r)
    }
```

Implement private `token_shape_and_size(&self, token: Uuid) -> Option<(String, eng::Size)>` by copying the branch structure of `token_vision_floors` (`mod.rs:1529-1548`) and reading `shape`/`size` instead of `vision`: linked (`token_eng.actor_id` → `self.actors.get(&id)`, then `overrides.shape`/`overrides.size` take precedence), dangling link → `None`, instanced → the embedded actor read through the deliberately-**uncached** direct `engine_as` path (an embedded actor's own id differs from the token's, so caching under either goes stale).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src/server && cargo test footprint_radius`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cd src/server && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/server/src/scene/mod.rs
git commit -m "feat(server/scene): resolve a token's footprint radius server-side

Mirrors the client's footprintRadius formula through the same token→actor
join token_vision_floors uses, with the client's own 0.4 fallback for an
actorless token, pinned by a parity test so the router and the gate cannot
derive different footprints.

Returns Option: an out-of-range radius is refused with a diagnostic rather
than clamped, since clamping would gate a map-scale token as a smaller disc."
```

---

### Task 8: `Pathfind` names its token — authorized, then derived

**Files:**
- Modify: `src/server/src/ws/protocol.rs:69-76` (`Pathfind`)
- Modify: `src/server/src/ws/conn.rs:566-574` (the `INVARIANT (scene presence)` block), `:576-632` (`handle_pathfind`), and the pinning test at `:2316`
- Modify: `src/client/core/src/ws-client.ts:574-596`, `src/client/core/src/wire.ts:381`, `src/client/ui-kit/src/appContext.ts:126`, `src/client/shell/src/lib/worldSession.svelte.ts:244-251`, `src/client/shell/src/lib/Table.svelte:95`
- Test: `src/server/src/ws/conn.rs`, `src/client/core/src/wire.test.ts`

**Interfaces:**
- Consumes: `resolve_token_footprint` (Task 7), `token_effective_owner` (the rule `user_owns_token_in_scene` already routes through).
- Produces: wire field `token: Option<Uuid>` on `Pathfind`; client `pathfind(scene, start, waypoints, footprintRadius, token?)`.

**The authorization is the point of this task, not an add-on.** `handle_pathfind` already has a non-GM presence gate (`user_owns_token_in_scene`, `conn.rs:602-613`) documented by an `INVARIANT (scene presence)` block whose first sentence reads *"a `Pathfind` frame names no token, so the derive-the-scene-from-the-token rule `Room::execute_move` applies has no counterpart here"* — a premise **this task falsifies**. Under the project's stale-comment-on-contact rule the implementer is obligated to rewrite that comment, so the plan must supply the correct replacement rather than a rationale that invites deleting the gate. Deriving a footprint from an unauthorized token id would also make the field an oracle: `conn.rs:569-574` states the returned polyline discloses the scene's `blocksMove` wall layout, and the pinning test's own fixture (`token_id` in `scene_a`, walls in `scene_b`) is that attack verbatim.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn pathfind_naming_an_unowned_token_is_refused() {
    // The named token must be effectively owned by the requester: otherwise its size is readable
    // through route reachability, and the presence gate could be delegated to an attacker value.
    let h = harness_two_players_one_scene().await;
    let res = h.pathfind_as(h.player_a, h.scene, (50.0, 50.0), &[(250.0, 50.0)], 0.4, Some(h.player_b_token)).await;
    assert!(matches!(res, ServerMsg::PathError { .. }), "an unowned token is refused generically");
}

#[tokio::test]
async fn pathfind_naming_a_token_in_another_scene_is_refused() {
    // Guards the cross-scene axis: presence in scene A must not derive a footprint from a token
    // living in scene B, nor disclose B's wall layout.
    let h = harness_player_with_token_in_scene_a_and_walls_in_scene_b().await;
    let res = h.pathfind_as(h.player, h.scene_b, (50.0, 50.0), &[(250.0, 50.0)], 0.4, Some(h.token_in_scene_a)).await;
    assert!(matches!(res, ServerMsg::PathError { .. }), "a token outside the named scene is refused");
}

#[tokio::test]
async fn pathfind_naming_an_owned_token_ignores_a_lying_wire_footprint() {
    let h = harness_with_large_owned_token_and_narrow_gap().await;
    // The wire value claims a tiny footprint; the server derives the real (large) one.
    let res = h.pathfind_as(h.player, h.scene, (50.0, 50.0), &[(450.0, 50.0)], 0.01, Some(h.token)).await;
    assert!(
        matches!(res, ServerMsg::PathError { .. }),
        "the derived footprint does not fit the gap, so no route is returned"
    );
}

#[tokio::test]
async fn pathfind_without_a_token_uses_the_wire_footprint() {
    let h = harness_with_large_owned_token_and_narrow_gap().await;
    let res = h.pathfind_as(h.player, h.scene, (50.0, 50.0), &[(450.0, 50.0)], 0.01, None).await;
    assert!(matches!(res, ServerMsg::PathResult { .. }), "a token-less preview honors the wire radius");
}

#[tokio::test]
async fn pathfind_refuses_an_oversized_derived_footprint() {
    // Task 7 returns None for a radius over MAX_FOOTPRINT_CELLS; the handler must refuse, never
    // fall back to the wire value (which would reopen the understated-footprint hole).
    let h = harness_with_map_scale_owned_token().await;
    let res = h.pathfind_as(h.player, h.scene, (50.0, 50.0), &[(250.0, 50.0)], 0.4, Some(h.token)).await;
    assert!(matches!(res, ServerMsg::PathError { .. }));
}
```

Extend the **existing** pinning test `pathfind_refuses_a_scene_the_requester_controls_no_token_in` (`conn.rs:2316`) with a token-named case — its positional calls at `:2432-2441` and `:2449-2458` must gain the new argument, and passing `None` at both would leave the token-named path (the one carrying the leak) untested:

```rust
    // The same cross-scene probe, now naming a token the requester DOES own in scene A. The
    // presence gate must still refuse for scene B: naming a token is not presence in a scene.
    let reply = handle_pathfind(
        Uuid::new_v4(), scene_b, (50.0, 50.0), vec![(250.0, 50.0)], 0.4,
        Some(token_id), &player_ctx, &room, &repo,
    ).await;
    assert!(matches!(reply, ServerMsg::PathError { .. }), "naming an owned token grants no presence in another scene");
```

```ts
it("pathfind sends the token id when given", () => {
  const sent: unknown[] = [];
  const c = clientWithCapture(sent);
  c.pathfind("s1", [0, 0], [[100, 0]], 0.4, "t1");
  expect(sent[0]).toMatchObject({ type: "pathfind", footprint_radius: 0.4, token: "t1" });
});

it("the Pathfind schema accepts an absent token", () => {
  expect(() => PathfindSchema.parse({
    type: "pathfind", request_id: "r", scene: "s", start: [0, 0], waypoints: [[1, 1]], footprint_radius: 0.4,
  })).not.toThrow();
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test pathfind_naming -- --nocapture` then `pnpm --filter @shadowcat/core test -- wire`
Expected: FAIL on both — the field does not exist.

- [ ] **Step 3: Add the wire field, with no false security claim**

```rust
    /// A one-shot grid pathfinding request, correlated by `request_id`. `start`/`waypoints` are
    /// scene coords; `waypoints`' LAST element is the goal. The route is mask-bounded for non-GM
    /// requesters.
    ///
    /// `token`, when present, is the token the route is for: the server AUTHORIZES it (effectively
    /// owned by the requester AND parented to `scene`) and then DERIVES the footprint from its
    /// document, IGNORING `footprint_radius` — so a route preview and the authoritative gate cannot
    /// disagree about the mover's size. It is NOT a presence proof: scene presence remains the
    /// separate ownership scan in `handle_pathfind`, which naming a token neither replaces nor
    /// satisfies. When absent, `footprint_radius` (grid units, the client's `footprintRadius`) is
    /// honored and the result is an explicitly hypothetical preview carrying no
    /// preview-equals-execution guarantee.
    Pathfind {
        request_id: Uuid,
        scene: Uuid,
        start: (f64, f64),
        waypoints: Vec<(f64, f64)>,
        footprint_radius: f64,
        #[serde(default)]
        token: Option<Uuid>,
    },
```

- [ ] **Step 4: Authorize and derive inside the existing read guard**

There is no `scene_ecs` binding in `handle_pathfind` — the ECS is reached through short-lived guards. Fold this into the Step-3 guard that already exists (around `conn.rs:630`) so the read count is unchanged, placing it **after** the Step-0 presence gate:

```rust
    // The named token is authorized before it is used: effective ownership (the same
    // `token_effective_owner` rule the presence gate and write-authz use — never a forked, looser
    // test) AND membership in the named scene. A caller-supplied token id that skipped either check
    // would be a size oracle, and a cross-scene id would source a footprint from a scene the
    // requester has no presence in. Failure returns the SAME generic PathError an unreachable route
    // gets, disclosing nothing about the token's existence. A GM is exempt from the ownership half
    // (they control the scene) but not from the scene-membership half.
    let footprint_radius = match token {
        Some(t) => {
            let derived = {
                let s = room.scene().read().await;
                match s.token_scene_and_effective_owner(t) {
                    Some((t_scene, _)) if t_scene != scene => None,
                    Some((_, owner)) if !is_gm && owner != Some(ctx.user_id) => None,
                    Some(_) => s.resolve_token_footprint(t),
                    None => None,
                }
            };
            match derived {
                Some(r) => r,
                None => {
                    return ServerMsg::PathError { request_id, message: "unreachable".to_string() }
                }
            }
        }
        None => footprint_radius,
    };
```

Add `token_scene_and_effective_owner(&self, token: Uuid) -> Option<(Uuid, Option<Uuid>)>` to `SceneEcs` if no equivalent accessor exists, routed through the same `token_effective_owner` helper `user_owns_token_in_scene` uses. Never fall back to the wire `footprint_radius` on failure — that would silently reopen the understated-footprint hole this task exists to close.

- [ ] **Step 5: Rewrite the falsified invariant comment**

`conn.rs:566-574`'s block opens with "a `Pathfind` frame names no token", which is no longer true. Rewrite it to state the post-change design, preserving the reason the presence gate exists (it is what stops a player route-previewing — and reading the wall layout of — a scene they have never entered), and preserving the "Deliberate asymmetry — do NOT 'fix' it by forking a looser ownership test" warning at `:595-601`, which now also covers the token authorization:

```rust
/// INVARIANT (scene presence): a non-GM requester must control a token in the named scene. A
/// `Pathfind` frame MAY name a token (`token`), but that is a footprint source, not a presence
/// proof — it is separately authorized (owned + parented to this scene) and never substitutes for
/// this scan. Without the scan a player can route-preview inside a scene they have never entered:
/// an `unrestricted` scene has no visibility mask to fail closed on, and the returned polyline
/// discloses that scene's `blocksMove` wall layout.
```

- [ ] **Step 6: Thread `token` through all five client seam layers**

A 4-parameter arrow is assignable to a 5-parameter function type, so `pnpm -r typecheck` **cannot** catch a dropped argument — each layer must be edited by hand:

| Layer | Site | Change |
|---|---|---|
| frame sender | `ws-client.ts:581-596` | add `token?: string`, include in the sent frame |
| session | `worldSession.svelte.ts:244-251` | add the param, forward it to `#ws.pathfind` |
| AppContext type | `appContext.ts:126` | widen the `pathfind` signature |
| AppContext wiring | `Table.svelte:95` | `pathfind: (s, st, wp, fr, tk) => session.pathfind(s, st, wp, fr, tk)` |
| ToolContext type | `controller.svelte.ts:42-47` | widen the `pathfind` signature |

`ToolRail.svelte` projects `pathfind: ctx.pathfind` **by reference**, so it needs no arity change (unlike `role` in Task 5). Regenerate ts-rs types and mirror in `wire.ts` (`token: z.string().uuid().nullish()`).

Verify no site was missed:
```bash
cd /c/Dev/Shadowcat && grep -rn "pathfind(" src/client src/modules --include=*.ts --include=*.svelte | grep -v "\.test\."
```
Every non-test call site must either pass 5 arguments or forward `...args`.

- [ ] **Step 7: Run the full gate**

Run: `cd src/server && cargo test` then `cd /c/Dev/Shadowcat && pnpm build && pnpm -r test && pnpm -r typecheck`
Expected: PASS. `pnpm -r test` is required for a shared wire-schema change — a new field breaks untyped frame fixtures across packages, and typecheck alone will not catch a dropped Zod field.

- [ ] **Step 8: Commit**

```bash
git add src/server/src/ws/protocol.rs src/server/src/ws/conn.rs src/server/src/scene/mod.rs src/types/generated src/client/core/src/wire.ts src/client/core/src/ws-client.ts src/client/ui-kit/src/appContext.ts src/client/shell/src/lib/worldSession.svelte.ts src/client/shell/src/lib/Table.svelte
git commit -m "feat(ws): Pathfind names its token, authorized then derived

A client-supplied footprint could understate a token's size and obtain a
route the authoritative gate then refuses. When token is present the server
authorizes it (effectively owned + parented to the named scene) and derives
the footprint from the document, ignoring the wire value and refusing rather
than falling back on failure.

The named token is explicitly NOT a presence proof: the ownership scan is
unchanged, and conn.rs's scene-presence invariant is rewritten to say so —
its prior text assumed a Pathfind frame names no token, which this change
falsifies. Without that, a caller-supplied id would delegate the gate and
expose any scene's blocksMove layout through the returned polyline."
```

---

### Task 9: The authoritative gate adopts the footprint predicate

**Files:**
- Modify: `src/server/src/scene/move_exec.rs` (`execute_move`)
- Modify: `src/server/src/ws/room.rs` (call site passes the derived footprint)
- Modify: `src/server/src/scene/mod.rs` (Task 3's test-module call gains the footprint argument)
- Test: `src/server/src/scene/move_exec.rs`

**Interfaces:**
- Consumes: `resolve_token_footprint` (Task 7), `GridShape::{footprint_cells, line_traversal}`, `pathfinding::{point_segment_distance, MAX_FOOTPRINT_CELLS}`, `crate::scene::segments_cross`, `move_walls(scene, None)` (Task 1).
- Produces: `execute_move(..., is_gm: bool, footprint_radius_cells: f64)`.

**Mirror `cell_enterable` exactly — all four checks, verified at `pathfinding.rs:88-146`:**

| Router check | `pathfinding.rs` | Gate must |
|---|---|---|
| (1) footprint disc vs every `blocksMove` wall | `:92-97` | apply |
| (2) mask over `footprint_cells ∪ line_traversal` | `:110-129` | apply |
| (3) center-to-center step crosses no wall (`segments_cross`) | `:131-136` | **apply — do NOT drop this** |
| (4) impassable over footprint cells | `:139-145` | apply |
| arrest / terrain | **center-cell only, deliberately** | keep **center-cell** |

Check (3) is not redundant with (1): at the default 0.4-cell footprint, a wall between two adjacent cell centers sits 0.5 cell from each, so `0.5 > 0.4` passes the disc test and the wall would become permeable on the sole remaining gate. And `cell_enterable`'s own comment at `:135-138` states arrest/terrain are **not** footprint-gated ("they represent effects on the mover's own position rather than solid geometry it must clear"), so footprint-gating arrest here would make the gate stricter than the router and break I4 in the direction the parity test measures.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn route_admissible_implies_gate_admissible_for_a_non_gm_grid() {
    // I4 forward direction. Scoped to GridStepped: there `gate_walk` is the identity on
    // cell-center input, so the gate's sample points ARE the cell centers `cell_enterable`
    // evaluates at. On Continuous the two evaluate at different granularity — asserted separately
    // below as the weaker route ⊆ gate-allowed.
    for kind in ["square", "hex"] {
        let (ecs, scene, token, user) = scene_with_narrow_gap_and_wide_token(kind, MovementModel::GridStepped);
        let fp = ecs.resolve_token_footprint(token).expect("in-range");
        let mask = ecs.visible_cells(user, scene, false);
        // NOT `if let Ok` — a fixture that yields no route must fail the test, not skip it.
        let route = ecs
            .pathfind(user, scene, (50.0, 50.0), &[(450.0, 50.0)], fp, false, None)
            .expect("the fixture is routable for this footprint");
        let out = execute_move(
            &ecs, scene, token, &route.path, MovementRestriction::Visible, &mask, 100.0, false, fp,
        )
        .expect("a routed path is admissible");
        assert!(!out.truncated, "kind={kind}: the gate accepts every routed step");
    }
}

#[test]
fn gate_refused_steps_are_absent_from_every_route_non_gm_grid() {
    // I4 REVERSE direction, which the spec requires and which is what catches a gate MORE
    // permissive than the router (e.g. a dropped segments_cross check).
    let (ecs, scene, token, user) = scene_with_wall_between_adjacent_cells_and_default_footprint();
    let fp = ecs.resolve_token_footprint(token).expect("in-range"); // 0.4
    let mask = ecs.visible_cells(user, scene, false);
    let candidates = [
        [(50.0, 50.0), (150.0, 50.0)],
        [(50.0, 50.0), (150.0, 150.0)],
        [(50.0, 50.0), (50.0, 150.0)],
    ];
    for path in candidates {
        let out = execute_move(
            &ecs, scene, token, &path, MovementRestriction::Visible, &mask, 100.0, false, fp,
        )
        .expect("admissible input");
        if out.truncated {
            let route = ecs.pathfind(user, scene, path[0], &[path[1]], fp, false, None);
            if let Ok(r) = route {
                assert!(
                    r.path.last().copied() != Some(path[1]),
                    "the gate refuses {:?} but a route reaches it — the gate is more permissive \
                     than the router",
                    path
                );
            }
        }
    }
}

#[test]
fn a_default_footprint_step_across_a_wall_is_still_truncated() {
    // Regression for the dropped segments_cross check: a wall between two adjacent cell centers
    // sits 0.5 cell from each, so the 0.4-radius disc test alone would pass it.
    let (ecs, scene, token, user) = scene_with_wall_between_adjacent_cells_and_default_footprint();
    let mask = ecs.visible_cells(user, scene, false);
    let out = execute_move(
        &ecs, scene, token, &[(50.0, 50.0), (150.0, 50.0)],
        MovementRestriction::Visible, &mask, 100.0, false, 0.4,
    )
    .expect("admissible");
    assert!(out.truncated, "the wall still blocks a default-footprint step");
}

#[test]
fn a_wide_token_cannot_enter_a_cell_whose_footprint_overlaps_fog() {
    let (ecs, scene, token, user) = scene_with_lit_center_line_only();
    let fp = ecs.resolve_token_footprint(token).expect("in-range"); // > 0.5
    let mask = ecs.visible_cells(user, scene, false);
    let out = execute_move(
        &ecs, scene, token, &[(50.0, 50.0), (150.0, 50.0)],
        MovementRestriction::Visible, &mask, 100.0, false, fp,
    )
    .expect("admissible");
    assert!(out.truncated, "a footprint cell outside the mask stops a wide token");
}

#[test]
fn a_sub_half_cell_footprint_diagonal_stays_admissible() {
    // The buddy-check P1 case: a small footprint's diagonal must not regress.
    let (ecs, scene, token, user) = scene_with_open_lit_area();
    let mask = ecs.visible_cells(user, scene, false);
    let out = execute_move(
        &ecs, scene, token, &[(50.0, 50.0), (150.0, 150.0)],
        MovementRestriction::Visible, &mask, 100.0, false, 0.4,
    )
    .expect("admissible");
    assert!(!out.truncated, "a 0.4-radius diagonal step is still allowed");
}

#[test]
fn arrest_stays_center_cell_matching_the_router() {
    // cell_enterable does NOT footprint-gate arrest (pathfinding.rs:135-138). A wide token whose
    // FOOTPRINT touches an arrest cell but whose CENTER does not must not be arrested, or the gate
    // becomes stricter than the router and I4 breaks.
    let (ecs, scene, token, user) = scene_with_arrest_cell_beside_the_path_and_wide_token();
    let fp = ecs.resolve_token_footprint(token).expect("in-range"); // > 0.5
    let mask = ecs.visible_cells(user, scene, false);
    let out = execute_move(
        &ecs, scene, token, &[(50.0, 50.0), (150.0, 50.0)],
        MovementRestriction::Visible, &mask, 100.0, false, fp,
    )
    .expect("admissible");
    assert!(!out.truncated, "arrest is center-cell only, matching the router");
}

#[test]
fn a_gm_is_exempt_from_every_footprint_check() {
    let (ecs, scene, token, _user) =
        scene_with_narrow_gap_and_wide_token("square", MovementModel::GridStepped);
    let out = execute_move(
        &ecs, scene, token, &[(50.0, 50.0), (450.0, 50.0)],
        MovementRestriction::Unrestricted, &empty_mask(), 100.0, true, 5.0,
    )
    .expect("admissible");
    assert!(!out.truncated, "a GM squeezes a wide token through anything");
}

#[test]
fn execute_move_refuses_an_out_of_range_footprint() {
    // I1: the new gate input gets an admissibility guard like every other. A NaN radius would make
    // every `dist < r_scene` comparison false — fail-open.
    let (ecs, scene, token) = scene_with_wall_across_the_path();
    for bad in [f64::NAN, -1.0, pathfinding::MAX_FOOTPRINT_CELLS + 1.0] {
        let err = execute_move(
            &ecs, scene, token, &[(50.0, 50.0), (150.0, 50.0)],
            MovementRestriction::Unrestricted, &empty_mask(), 100.0, false, bad,
        )
        .expect_err("an out-of-range footprint is refused");
        assert!(matches!(err, MoveReject::Degenerate), "bad={bad}: got {err:?}");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test route_admissible_implies_gate_admissible -- --nocapture`
Expected: FAIL — compile error (arity) first; then `a_wide_token_cannot_enter_a_cell_whose_footprint_overlaps_fog` fails because the gate is center-based.

- [ ] **Step 3: Add the guard and adopt the router's checks**

At the top of `execute_move`, alongside the existing `cell` validation:

```rust
    // I1: a resource/admissibility guard, never exempted for a GM. `contains` rejects NaN and ±Inf.
    if !(0.0..=pathfinding::MAX_FOOTPRINT_CELLS).contains(&footprint_radius_cells) {
        return Err(MoveReject::Degenerate);
    }
```

Resolve the authoritative wall set **once** before the loop (never per step, never per-requester):

```rust
    // The executor always reads the AUTHORITATIVE wall set: a `gm_only` wall omitted from the
    // requester's route springs here, exactly as a secret region does (D10, I3).
    let gate_walls = ecs.move_walls(scene, None);
```

Then in the walk loop — both wall checks, mirroring `cell_enterable`'s (1) and (3):

```rust
        // Step 1: wall gate. TWO checks, both from `cell_enterable`: the footprint disc must clear
        // every wall, AND the step segment must cross none. The disc alone is insufficient — at a
        // 0.4-cell footprint a wall midway between adjacent cell centers is 0.5 cell away and would
        // pass, making walls permeable on the sole movement gate.
        if check_walls {
            let r_scene = footprint_radius_cells.max(0.0) * cell;
            let disc_blocked = gate_walls
                .iter()
                .any(|w| pathfinding::point_segment_distance(next, w.a, w.b) < r_scene);
            let crossed = gate_walls
                .iter()
                .any(|w| crate::scene::segments_cross(prev, next, w.a, w.b));
            if disc_blocked || crossed {
                stopped_early = true;
                break;
            }
        }

        // Step 2: vision-mask gate over the FOOTPRINT, not the center — the same
        // `footprint_cells ∪ line_traversal` union `cell_enterable` requires. Both halves come from
        // the resolved shape; the free square functions (`pathfinding::footprint_cells`,
        // `movement::supercover_cells`) are SquareGrid internals and would test square-indexed
        // cells against a hex mask.
        if check_mask {
            let Some(mut cells) = grid.line_traversal(prev, next, cell) else {
                stopped_early = true;
                break;
            };
            let r_scene = footprint_radius_cells.max(0.0) * cell;
            cells.extend(grid.footprint_cells(to_cell(next), next, r_scene, cell));
            if !cells.iter().all(|c| visible.contains(c)) {
                stopped_early = true;
                break;
            }
        }
```

In step 3, footprint-gate **impassable only**; arrest and terrain stay center-cell:

```rust
        let next_cell = to_cell(next);
        if next_cell != last_region_cell {
            // Impassable IS footprint-gated (cell_enterable check 4): a wide body cannot fit past
            // impassable terrain any more than past a wall.
            let r_scene = footprint_radius_cells.max(0.0) * cell;
            let fp_cells = grid.footprint_cells(next_cell, next, r_scene, cell);
            if check_regions && fp_cells.iter().any(|c| regions.is_impassable(*c)) {
                stopped_early = true;
                break;
            }
            cost += regions.terrain_multiplier(next_cell);
            // Arrest and terrain are CENTER-CELL only, matching cell_enterable's documented
            // asymmetry: they act on the mover's own position rather than being solid geometry it
            // must clear. Footprint-gating arrest here would make the gate stricter than the
            // router and break I4.
            if check_regions && regions.is_arrest(next_cell) {
                stop_idx = i;
                stopped_early = true;
                break;
            }
            last_region_cell = next_cell;
        }
```

`ecs.blocks_move` loses its `move_exec` caller here and its `room.rs` caller in Task 4. **Retain it** — it is the segment-crossing predicate, and `crate::scene::segments_cross` above is the primitive it wraps. If `clippy -D warnings` reports it unused after both tasks, keep the method and add a `#[cfg(test)]`-visible justification comment rather than deleting it; the wall-crossing semantics must stay expressed in one place.

- [ ] **Step 4: Update both call sites**

`room.rs` — pass the derived footprint, refusing when Task 7 returns `None`:

```rust
            let Some(fp) = scene.resolve_token_footprint(token) else {
                return Err(DataError::Forbidden); // an out-of-range footprint refuses, never clamps
            };
```

`scene/mod.rs` — Task 3's test call gains `0.4` as the ninth argument; assertion unchanged.

- [ ] **Step 5: Run the tests; audit every fixture change**

Run: `cd src/server && cargo test`
Expected: PASS. The frozen king-step parity fixtures may shift where a fixture token's footprint now clears differently. For each change, derive from the fixture's geometry why the new outcome is correct under footprint semantics **before** updating it, and record the reasoning in the commit body. A fixture that changes for an unexplained reason is a defect signal, not a fixture to rewrite.

- [ ] **Step 6: Mutation-verify the wall pair**

Delete the `crossed` term from Step 3's wall check. Run: `cd src/server && cargo test a_default_footprint_step_across_a_wall_is_still_truncated`
Expected: FAIL. Restore it and re-run to PASS.

- [ ] **Step 7: Commit**

```bash
cd src/server && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/server/src/scene/move_exec.rs src/server/src/ws/room.rs src/server/src/scene/mod.rs
git commit -m "feat(server/scene): footprint-aware authoritative movement gate

execute_move adopts cell_enterable's predicate set: footprint-disc clearance
AND segment-crossing for walls, footprint_cells ∪ line_traversal for the mask,
and footprint-wide impassable. Arrest and terrain stay center-cell, matching
the router's documented asymmetry — footprint-gating them would make the gate
stricter than the router.

Keeping the segment-crossing check is load-bearing: at the default 0.4-cell
footprint a wall midway between adjacent cell centers clears the disc test,
so disc-only would make walls permeable on the sole movement gate.

Both mask halves come from the resolved GridShape, never the square-only free
functions. GMs are exempt from all of it; the new footprint input gets an
admissibility guard that GMs do not bypass."
```

---

### Task 10: Client sends the token on every route request

**Files:**
- Modify: `src/modules/scene-tools/src/controller.svelte.ts` (`requestRoute` at `:404`, `commitRoute`'s fallback pathfind at `:529`)
- Test: `src/modules/scene-tools/src/controller.test.ts`

**Interfaces:**
- Consumes: `ctx.pathfind(scene, start, waypoints, footprintRadius, token?)` (Task 8), `footprintFor(id)` (Task 5).
- Produces: no new signatures.

- [ ] **Step 1: Write the failing tests**

```ts
it("a single-selection route preview names the token it is for", async () => {
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }] });
  h.select(["t1"]);
  await h.previewRouteTo({ x: 300, y: 0 });
  expect(h.pathfindCalls[0]).toMatchObject({ token: "t1" });
});

it("a multi-selection route preview omits the token", async () => {
  // resolveFootprint()'s existing "not exactly one selected ⇒ 0.4" rule has no per-token
  // analogue, so a multi-token preview stays a hypothetical: no token, wire footprint honored.
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }, { id: "t2", x: 100, y: 0 }] });
  h.select(["t1", "t2"]);
  await h.previewRouteTo({ x: 300, y: 0 });
  expect(h.pathfindCalls[0].token).toBeUndefined();
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `pnpm --filter @shadowcat/scene-tools test -- controller`
Expected: FAIL — `token` is undefined in the single-selection call.

- [ ] **Step 3: Pass the token when exactly one is selected**

In `requestRoute` and `commitRoute`'s fallback pathfind, pass the selected token id as the fifth argument **only** when `ctx.tokenSelection.ids.size === 1` — mirroring `resolveFootprint()`'s existing guard so the preview's footprint source and the server's derivation agree. Task 5's `commitMoves` already passes a per-token id, since it iterates one token at a time.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `pnpm -r test && pnpm -r typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
pnpm -r test && pnpm -r typecheck && pnpm lint
git add src/modules/scene-tools/src/controller.svelte.ts src/modules/scene-tools/src/controller.test.ts
git commit -m "feat(client/scene-tools): single-selection route requests name their token

The server derives the footprint from the named token, so a preview and the
authoritative gate agree on the mover's size. A multi-token selection omits
the token, matching resolveFootprint's existing multi-selection rule."
```

---

### Task 11: Documentation and skill sync

**Files:**
- Modify: `docs/PLAN.md:345`, `docs/TODO.md`, `docs/POST_WORK_FINDINGS.md`, `docs/CLOSED_BUGS.md`
- Modify: `docs/superpowers/specs/2026-07-24-phase1-closeout-campaign-design.md` (Phase D amendment)
- Modify: `docs/superpowers/specs/2026-07-25-phase-d-alpha-movement-authority-secrecy-design.md` (spec corrections)
- Modify: `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md`

- [ ] **Step 1: Correct the spec's own two errors**

The buddy check found two factual errors in the D-α spec that must not outlive the branch:

1. D4's "replacing the center-based `blocks_move` test" — the gate keeps **both** the disc and segment-crossing checks (Task 9). Reword.
2. D4's claim that footprint-gating arrest "matches the router" — `cell_enterable` deliberately does **not** footprint-gate arrest (`pathfinding.rs:135-138`). Record that arrest stays center-cell.

- [ ] **Step 2: Invert the stale GM-enforcement statement**

`docs/PLAN.md:345` states `execute_move` is "GM wall-honored, diverging from `publish`'s legacy GM wall-bypass". Replace with the M9 §5 rule: a GM bypasses every gameplay gate on both paths, and no resource guard on either.

- [ ] **Step 3: Update the scene-rendering skill**

Five edits, each verified against the merged diff:

1. **Parity checklist** — `publish` no longer gates non-GM traversal, so `execute_move` is the sole implementation of the per-cell traversal decision and axes 1-3 describe a fork that no longer exists. Keep the axes as present-tense constraints on any future second gate (**I2**). Note that axis 6 (scene-existence refusal) still lives in `publish` for `Create` placement.
2. **Wall sets** — add the `move_walls(scene, viewer)` two-value contract and **I5**; state that vision/lighting keep the full set and must not be unified with routing.
3. **GM exemption** — replace the "Do NOT re-grant GM wall-bypass in `execute_move`" text at `802-804` with **I1** (gameplay gates exempt, resource guards never), citing M9 §5 as governing.
4. **Footprint predicate** — the gate is now footprint-aware for walls, mask, and impassable; **arrest and terrain remain center-cell**, so the documented asymmetry is *narrowed, not retired*. **I4** holds for non-GM movers on `GridStepped`; on `Continuous` only `route ⊆ gate-allowed` is claimed.
5. **`Pathfind` token** — the frame may name a token as a footprint source, authorized by ownership + scene membership, and explicitly not a presence proof.

- [ ] **Step 4: Close the findings entries**

Mark resolved in `POST_WORK_FINDINGS.md`: "Route stricter than the authoritative gate" (D4 — now narrowed to arrest/terrain only). Move any newly-closed bug to `CLOSED_BUGS.md` with its root cause. Do **not** close the D-β entries (bounds units, hex cost, `env_light_polys` hex extent, lighting polish) — they belong to the next phase.

- [ ] **Step 5: Amend the campaign spec**

Record the D-α/D-β split, the three added items (D10, D9, D8), that D5 shipped in `513aef8`/`e1156ae`, and the plan-level buddy check's outcome (5 Critical / 14 Important / 14 Minor, folded in before execution).

- [ ] **Step 6: Dispatch the reviewed skill-update gate**

Dispatch `shadowcat-spec-reviewer` on the skill diff specifically, confirming each edit accurately captures the implemented change with no omission, drift, or broken pointer. Per `reviewed-skill-update-gate-needs-its-own-adversarial-check`, a single clean pass is not sufficient assurance on its own — the whole-branch two-reviewer pair also covers it.

- [ ] **Step 7: Commit**

```bash
git add docs .claude/skills
git commit -m "docs(skills): movement authority unified — Phase D-alpha doc sync

Records the single-gate collapse, the routing/vision wall-set split, the GM
gameplay-vs-resource exemption rule, the narrowed (not retired) center-cell
asymmetry for arrest/terrain, and Pathfind's token as a footprint source
rather than a presence proof.

Inverts the stale GM-wall-enforcement intent in favor of M9 section 5, and
corrects two factual errors the plan-level buddy check found in the D-alpha
spec itself."
```

---

## Self-Review

**Spec coverage.** D10 → Tasks 1-3. D9 → Tasks 4-5. D8 → Task 6. D4 → Tasks 7-10. Cross-cutting: **I1** Tasks 6 and 9 (both with dedicated resource-guard tests), **I2** Task 4, **I3** Tasks 1 and 9, **I4** Task 9 (both directions, scoped to GridStepped), **I5** Tasks 1 and 11. Doc/skill obligations → Task 11. The spec's `navmesh_for` cache decision → Task 2 (with the id→geometry and hash→exact-key deviations recorded). The spec's Create gap → Task 4. Two spec errors corrected → Task 11 Step 1. No spec requirement is unimplemented.

**Placeholder scan.** No TBD/TODO markers. Every code step carries real code; every test step names an exact command and expected result. Every fixture is labelled **NEW** (to be created, with its required properties stated) or cited at a verified `file:line`.

**Type consistency.** `move_walls(scene, viewer)` (Task 1) is consumed with that arity in Tasks 2, 3, 9. `navmesh_for(scene, footprint, walls)` (Task 2) is called with three arguments in Task 3. `execute_move` gains `is_gm` in Task 6 (8 args) and `footprint_radius_cells` in Task 9 (9 args); Task 3's test call is explicitly updated in **both** tasks, and both list `scene/mod.rs` in Files and `git add`. `resolve_token_footprint` (Task 7) returns `Option<f64>`, consumed as such in Tasks 8 and 9 with explicit refusal on `None`. `DEFAULT_FOOTPRINT_RADIUS_CELLS` is defined in Task 7 and referenced in its own tests only. The client `pathfind` gains `token` in Task 8 across all five seam layers and is used in Tasks 5 and 10. `ToolContext.role` is added in Task 5 and used only there.

**Verified-against-source claims.** Every `file:line` citation in this plan was confirmed against the working tree at `0a581c9`: `token_move` performs no pre/post comparison (`mod.rs:912-934`); `cell_enterable`'s four checks and its arrest/terrain exclusion (`pathfinding.rs:88-146`); `sendMoves`'s two call sites (`controller.svelte.ts:851`, `:859`); `ToolContext`'s lack of `role` and its assembly in `ToolRail.svelte`; `handle_pathfind`'s presence gate and `INVARIANT (scene presence)` block (`conn.rs:566-613`); `publish`'s six-argument signature (`room.rs:206-213`); `navmesh_for`'s `.round()` and `.lock().unwrap()` (`mod.rs:1163-1176`); `visible_cells_cached` at `room.rs:331`; the four coordinate-bound tests (`room.rs:2807`, `:2855`, `:2889`, `:2915`); their surviving replacements (`token.rs:100`, `move_exec.rs:1820`, `:1831`); `MAX_FOOTPRINT_CELLS = 64.0` (`pathfinding.rs:535`); `scene_with_two_walls_one_blocking` at `mod.rs:4968`; and `Table.svelte:95`'s 4-parameter `pathfind` arrow.

**Ordering and intermediate-state notes.**
- Tasks 4 and 5 are one **user-visible** unit: after Task 4 the shipped client still emits `/engine/x,y` writes the server now refuses, so player drags are broken until Task 5 lands. Every commit compiles and passes tests, but do not merge the branch between them. Task 4's own merge of the former Tasks 4+5 removes the earlier `#[allow(dead_code)]` problem entirely.
- Tasks 6 and 9 each change `execute_move`'s arity and must update Task 3's test call in the same commit (Signature-widening rule).
- Task 8 must land before Tasks 9 and 10 — Task 9's `room.rs` call site consumes `resolve_token_footprint`'s `Option`, and Task 10 consumes the widened client seam.
