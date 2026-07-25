# Phase D-α — Movement Authority & Secrecy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the `gm_only`-wall route-shape leak, make player movement request-only so `execute_move` becomes the sole movement gate, restore the GM "ignore walls" override, and make the authoritative gate footprint-aware.

**Architecture:** Four sequential items. D10 gives the routing wall set a per-requester view mirroring `region_field`'s two-value contract. D9 refuses non-GM token position writes, deleting the duplicated traversal gate in `Room::publish` and repointing its point-placement machinery at `Create`. D8 exempts GMs from every gameplay gate in `execute_move`. D4 adopts the router's footprint predicate in the now-single gate, with the footprint derived server-side from the token.

**Tech Stack:** Rust (axum/tokio/sqlx, `hecs` ECS), Svelte 5 runes + TypeScript, ts-rs → Zod wire mirror, Vitest + `cargo test`, Playwright for canvas.

**Spec:** `docs/superpowers/specs/2026-07-25-phase-d-alpha-movement-authority-secrecy-design.md`

## Global Constraints

- Build order: `pnpm build` (produces `dist/`) MUST precede any `cargo` build — `rust-embed` validates `../../dist/` at compile time.
- Full gate before any commit is considered green: `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint`, `cargo test`, `cargo fmt`, `cargo clippy`.
- Server crate is rustfmt-clean and CI-enforced; keep every server commit `cargo fmt`-clean.
- Cross-platform: `std::path` only, no hardcoded separators, no OS-specific code without `#[cfg]` for all three targets.
- Never fork a decision across two paths. Where two paths must agree, make one derive from the other or have both read one shared symbol. Pin with an anti-drift test that fails if either side changes.
- **I1** — a GM bypasses every gameplay gate (walls, mask, impassable, arrest, footprint) and **no** resource guard (`MAX_GATE_WALK_COORD`, `MAX_GATE_WALK_SAMPLES`, non-finite refusal, scene-existence refusal, `TokenEngine::validate`).
- **I3** — wall secrecy is a two-value contract: `None` = authoritative, `Some(user)` = per-requester. Callers pass `None` for a GM. Never a third mode.
- **I5** — `sight_walls`/`light_walls` keep the FULL wall set including `gm_only`. Do not unify them with the routing wall set.
- Comments: present-tense current state, no history/process meta, cite algorithmic decisions.
- `ts-rs` types are generated — edit the Rust struct, regenerate, mirror in the client Zod schema.
- Never delete files with `rm`/`Remove-Item`; use `trash`.

## Model/Effort directives

Decided at the writing-plans handoff (per `~/.claude/docs/sdd-model-effort-tiers.md`):

- **Plan-writer:** mainline in the calling session (Opus, high) — chosen over dispatching `sdd-plan-writer-opus`.
- **Dispatch loop:** mainline in the calling session — chosen over delegating to `sdd-dispatcher`.
- **Implementer:** `sdd-implementer` (Sonnet, medium) default. Escalate BLOCKED/DONE_WITH_CONCERNS → `sdd-implementer-highthink` → `sdd-implementer-opus`, never skipping a rung. De-escalate Tasks 11 and 12 to `sdd-implementer-haiku` only if they reduce to pure transcription.
- **Per-task reviewer:** `sdd-reviewer` (Sonnet, high). Escalate to `sdd-reviewer-opus` for Tasks 1, 3, 4, 5, 7, 10 — all security-sensitive or multi-file.
- **Final whole-branch review:** the project's two-reviewer pair, `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (spec mandate for a security-sensitive phase), plus `sdd-final-reviewer` is NOT used in place of them.
- **Fix subagents:** reuse whichever implementer tier produced the original task.

## Buddy-check directives

This plan qualifies as high-risk: it deletes a server-side security gate, closes a live information leak, and changes a wire contract. Decided at the handoff:

- **Buddy-check the PLAN before any code is written** — two blind reviewers over this document, then a brokered debate to convergence. Highest leverage: a planning error here means deleting the `publish` gate incorrectly or mis-scoping the wall filter.
- The final branch is covered by the spec-mandated two-reviewer pair; a second final-branch buddy-check was declined as overlapping.
- Record the buddy-check outcome and any resulting plan amendments in this section before Task 1 begins.

## File Structure

**Server — modified**

- `src/server/src/scene/mod.rs` — `move_walls` gains `viewer`; `navmesh_for` cache re-keyed; `pathfind` threads the per-requester wall set; new `resolve_token_footprint`; `create_placement_allowed` helper for the Create gate.
- `src/server/src/scene/move_exec.rs` — `execute_move` gains `is_gm` and `footprint_radius`; GM exemption on walls/impassable/arrest; footprint-aware wall + mask + region checks.
- `src/server/src/scene/navmesh.rs` — doc correction: the wall check is a secrecy gate when the wall set is unfiltered.
- `src/server/src/ws/room.rs` — non-GM token position `Update` refused; traversal gate deleted; point-placement machinery repointed at `Create`; `execute_move` call site passes `is_gm` + footprint.
- `src/server/src/ws/protocol.rs` — `Pathfind` gains `token: Option<Uuid>`.
- `src/server/src/ws/conn.rs` — `Pathfind` handler derives the footprint from `token`.

**Client — modified**

- `src/modules/scene-tools/src/controller.svelte.ts` — `sendMoves` role-branches; drag preview replaces optimistic write; `requestRoute`/`commitRoute` send `token`.
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

Add to the `#[cfg(test)]` module in `mod.rs`. Build a scene with two `blocksMove` walls, one marked `gm_only` on `/engine`:

```rust
#[test]
fn move_walls_omits_a_gm_only_wall_for_a_player_viewer() {
    let (mut ecs, scene, player) = scene_with_public_and_secret_move_walls();
    // Authoritative view: both walls.
    assert_eq!(ecs.move_walls(scene, None).len(), 2, "authoritative view carries every blocksMove wall");
    // Per-requester view: the gm_only wall is absent.
    let visible = ecs.move_walls(scene, Some(player));
    assert_eq!(visible.len(), 1, "a gm_only wall is omitted from a player's routing set");
    assert_eq!(
        (visible[0].a, visible[0].b),
        ((100.0, 0.0), (100.0, 200.0)),
        "the surviving wall is the public one"
    );
    let _ = &mut ecs;
}

#[test]
fn move_walls_keeps_a_blocks_sight_false_wall_for_a_player() {
    // An invisible BARRIER (blocksSight:false, blocksMove:true) is a public document:
    // the router must honor it, unlike a gm_only wall.
    let (ecs, scene, player) = scene_with_invisible_barrier_wall();
    assert_eq!(
        ecs.move_walls(scene, Some(player)).len(),
        1,
        "a blocksSight:false wall is public geometry and stays in the player's routing set"
    );
}
```

Fixture helper — mark secrecy the way `region_field` actually reads it (`property_overrides["/engine"]`, NOT `permissions.default`):

```rust
/// A scene with one public blocksMove wall at x=100 and one `gm_only` blocksMove wall at x=150.
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
```

Reuse the existing wall-fixture builder in this module (`scene_with_two_walls_one_blocking` at `mod.rs:5090` shows the shape); `wall_doc_eng` builds a `wall` doc with `engine.seg` + `blocksMove: true` + `blocksSight: true`. For `scene_with_invisible_barrier_wall`, set `blocksSight: Some(false)` and leave permissions default.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test move_walls_omits_a_gm_only_wall -- --nocapture`
Expected: FAIL — `move_walls` takes one argument, so this is a compile error (`this method takes 1 argument but 2 arguments were supplied`).

- [ ] **Step 3: Add the `viewer` parameter and the filter**

Replace the signature and add the filter, mirroring `region_field`'s branch exactly:

```rust
    /// The scene's `blocksMove` wall segments. Mirrors the wall filter in `blocks_move`
    /// (doc_type "wall", parent = scene, `engine.blocksMove == true`, endpoints at
    /// `engine.seg.{x1,y1,x2,y2}`). INVARIANT: same filter as `blocks_move` — any divergence
    /// would allow the pathfinder to route through walls the movement gate would then reject.
    ///
    /// Two-value secrecy contract, identical to `region_field`'s and never a third mode:
    /// `viewer: None` is the AUTHORITATIVE set (every enabled wall) — used by `execute_move` and
    /// by a GM requester; `viewer: Some(user)` is the PER-REQUESTER set used by the routers, where
    /// a wall is included only when `user` can see the visibility tier declared on its `/engine`.
    /// A `gm_only` wall is therefore absent from a non-GM's route (its geometry cannot be inferred
    /// from route shape) but still blocks at execution, exactly as a secret region springs.
    /// Callers MUST pass `None` for a GM requester.
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

This step is behavior-preserving. Find them all:

Run: `cd src/server && cargo build 2>&1 | grep -n "move_walls"`

Pass `None` at each site (`pathfind`, `move_exec`'s caller, and any test). Do NOT introduce the per-requester call yet — Task 3 does that, so this task's diff is reviewable as "new capability, zero behavior change".

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src/server && cargo test move_walls`
Expected: PASS, including the pre-existing `move_walls_returns_only_blocks_move_segments_for_the_scene`.

- [ ] **Step 6: Mutation-verify the tests are non-vacuous**

Temporarily change the filter to `if false` (never skip). Run: `cd src/server && cargo test move_walls_omits_a_gm_only_wall`
Expected: FAIL. Revert the mutation and re-run to PASS.

- [ ] **Step 7: Commit**

```bash
cd src/server && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/server/src/scene/mod.rs
git commit -m "feat(server/scene): per-requester routing wall set

move_walls gains a viewer parameter with region_field's two-value secrecy
contract: None is authoritative, Some(user) filters gm_only walls through
the same resolve_access/property_overrides mechanism. Every call site
passes None, so behavior is unchanged until the routers adopt it.

Vision and lighting keep the full wall set (M9b)."
```

---

### Task 2: Navmesh cache keyed on the requester's wall set

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`navmesh_cache` field + `navmesh_for`)
- Test: `src/server/src/scene/mod.rs` (in-module `#[cfg(test)]`)

**Interfaces:**
- Consumes: `move_walls(scene, viewer)` from Task 1.
- Produces: `SceneEcs::navmesh_for(&self, scene: Uuid, footprint_radius_cells: f64, walls: &[vision::Seg]) -> Option<Arc<NavMesh>>`.

**Why:** `build_navmesh` inflates walls into obstacles, so a mesh is only valid for the wall set it was built from. Keyed on `(scene, quantized_footprint)` alone, the first requester's mesh would be served to a requester who sees a different wall subset — a GM's mesh leaking secret-wall geometry into a player's route, or the reverse.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn navmesh_for_does_not_share_a_mesh_across_differing_wall_sets() {
    let (ecs, scene, player) = scene_with_public_and_secret_move_walls();
    let gm_walls = ecs.move_walls(scene, None);
    let player_walls = ecs.move_walls(scene, Some(player));
    let gm_mesh = ecs.navmesh_for(scene, 0.4, &gm_walls).expect("gm mesh builds");
    let player_mesh = ecs
        .navmesh_for(scene, 0.4, &player_walls)
        .expect("player mesh builds");
    assert!(
        !Arc::ptr_eq(&gm_mesh, &player_mesh),
        "a differing wall set must not be served a cached mesh built from another set"
    );
}

#[test]
fn navmesh_for_shares_a_mesh_across_identical_wall_sets() {
    let (ecs, scene, _player) = scene_with_public_and_secret_move_walls();
    let walls = ecs.move_walls(scene, None);
    let a = ecs.navmesh_for(scene, 0.4, &walls).expect("first build");
    let b = ecs.navmesh_for(scene, 0.4, &walls).expect("second build");
    assert!(
        Arc::ptr_eq(&a, &b),
        "an identical wall set must reuse the memoized mesh"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test navmesh_for_does_not_share -- --nocapture`
Expected: FAIL — compile error, `navmesh_for` takes 2 arguments.

- [ ] **Step 3: Re-key the cache and take the wall set as a parameter**

Change the cache key type to `(Uuid, i64, u64)` and add a digest helper:

```rust
/// Order-independent digest of a routing wall set, the third component of the navmesh cache key.
/// A mesh is only valid for the wall set it was inflated from, so two requesters share a mesh
/// exactly when they see the same walls. Order-independent (XOR of per-segment hashes) because
/// `hecs` iteration order is not stable, so the same wall set can be produced in different orders
/// across calls and must still hit the cache.
fn wall_set_digest(walls: &[vision::Seg]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut acc: u64 = 0;
    for s in walls {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        s.a.0.to_bits().hash(&mut h);
        s.a.1.to_bits().hash(&mut h);
        s.b.0.to_bits().hash(&mut h);
        s.b.1.to_bits().hash(&mut h);
        acc ^= h.finish();
    }
    acc
}
```

In `navmesh_for`, keep the existing pre-cache validation of `footprint_radius_cells` (a buddy-check finding: validating after the lookup lets `NaN` alias onto a legitimate cached entry), then include the digest in the key and pass `walls` into `build_navmesh` instead of calling `self.move_walls(scene)` internally:

```rust
    pub(crate) fn navmesh_for(
        &self,
        scene: Uuid,
        footprint_radius_cells: f64,
        walls: &[vision::Seg],
    ) -> Option<Arc<navmesh::NavMesh>> {
        // Validate BEFORE the quantized key or any cache touch (see the field doc comment).
        if !(0.0..=pathfinding::MAX_FOOTPRINT_CELLS).contains(&footprint_radius_cells) {
            return None;
        }
        let qkey = (footprint_radius_cells * 1000.0) as i64;
        let key = (scene, qkey, wall_set_digest(walls));
        if let Some(hit) = self.navmesh_cache.lock().ok()?.get(&key).cloned() {
            return Some(hit);
        }
        // ... existing bounds/cell resolution, then:
        let mesh = Arc::new(navmesh::build_navmesh(bounds, cell, walls, footprint_radius_cells)?);
        self.navmesh_cache.lock().ok()?.insert(key, mesh.clone());
        Some(mesh)
    }
```

Note: XOR of hashes is order-independent but collides on duplicate segments (a set containing the same segment twice digests as if it contained neither). Duplicate identical wall segments are geometrically inert for `build_navmesh` (the second capsule is contained in the first), so a collision between "two copies of W" and "no W" cannot arise from a real wall set — a duplicate can only be present alongside its own first copy, never instead of it.

- [ ] **Step 4: Update `navmesh_for`'s call site in `pathfind`**

The `Continuous` branch currently calls `self.navmesh_for(scene, footprint_radius)`. Pass the wall set already resolved in `pathfind` (`&walls`).

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src/server && cargo test navmesh`
Expected: PASS, including the pre-existing navmesh cache/quantization tests.

- [ ] **Step 6: Commit**

```bash
cd src/server && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/server/src/scene/mod.rs
git commit -m "feat(server/scene): key the navmesh cache on the requester's wall set

A mesh is only valid for the wall set it was inflated from, so the cache
key gains an order-independent digest of the included segments and the
wall set becomes a parameter. Two requesters with identical sets still
share one mesh; differing sets can no longer alias."
```

---

### Task 3: Routers consume the per-requester wall set

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`pathfind`)
- Modify: `src/server/src/scene/navmesh.rs` (doc correction on the wall check)
- Test: `src/server/src/scene/mod.rs`

**Interfaces:**
- Consumes: `move_walls(scene, viewer)` (Task 1), `navmesh_for(scene, footprint, walls)` (Task 2).
- Produces: no new signatures; `pathfind`'s route now omits `gm_only` walls for non-GM requesters.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn non_gm_route_crosses_a_gm_only_wall_that_springs_at_execution() {
    // The router cannot see the secret wall, so it routes straight through it; the executor
    // reads the authoritative set and stops there. Same spring-at-execution shape as a
    // secret region, asserted end-to-end.
    let (ecs, scene, player) = scene_with_secret_wall_between_two_cells(player_owned_token());
    let out = ecs
        .pathfind(player, scene, (50.0, 50.0), &[(250.0, 50.0)], 0.4, false, None)
        .expect("the player's route ignores a wall it cannot see");
    assert!(
        out.path.len() >= 2,
        "a route is produced despite the secret wall standing across it"
    );

    let walls = ecs.move_walls(scene, None); // authoritative
    let visible = ecs.visible_cells(player, scene, false);
    let exec = crate::scene::move_exec::execute_move(
        &ecs, scene, token, &out.path, MovementRestriction::Unrestricted, &visible, 100.0,
    )
    .expect("execution is admissible");
    assert!(
        exec.truncated,
        "the secret wall springs at execution and truncates the move"
    );
    let _ = walls;
}

#[test]
fn gm_route_detours_around_a_gm_only_wall() {
    let (ecs, scene, gm) = scene_with_secret_wall_between_two_cells(gm_owned_token());
    let out = ecs
        .pathfind(gm, scene, (50.0, 50.0), &[(250.0, 50.0)], 0.4, true, None)
        .expect("a GM route exists");
    // A GM passes viewer=None, so the secret wall is in their routing set and the route
    // must not run straight through it.
    assert!(
        out.path.iter().all(|p| (p.0 - 150.0).abs() > 1.0),
        "a GM's route does not pass through the wall's x=150 line"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test gm_only_wall -- --nocapture`
Expected: FAIL — `non_gm_route_crosses_a_gm_only_wall_that_springs_at_execution` fails because the router currently sees the secret wall and detours (or reports `Unreachable`).

- [ ] **Step 3: Thread the per-requester set through `pathfind`**

In `pathfind`, replace the unconditional `let walls = self.move_walls(scene);` with the per-requester resolution, hoisted ABOVE the engine dispatch so both engines receive the same slice (mirroring how `mask` and `region_field` are already hoisted):

```rust
        // Per-requester routing wall set (D10): a non-GM's route omits `gm_only` walls, so their
        // geometry cannot be inferred from route shape. The executor always reads the
        // authoritative set (`None`) and springs a secret wall at execution, exactly as a secret
        // region springs. Hoisted above the engine dispatch so BOTH engines receive the SAME
        // slice — never a forked wall computation (the same discipline `mask` follows).
        let walls = self.move_walls(scene, if is_gm { None } else { Some(user) });
```

Both branches already read `&walls`; the `Continuous` branch's `navmesh_for` call now passes `&walls` (Task 2).

- [ ] **Step 4: Correct the wall-check documentation in `navmesh.rs`**

`clip_to_visible_mask`'s doc comment asserts a two-checks dichotomy where the wall check has "no confidentiality stake" because "walls are public geometry". That is false for a `gm_only` wall. Replace that paragraph:

```rust
    /// **Two checks, both now secrecy-relevant — do not reuse the pre-D10 framing.** The mask check
    /// is a secrecy gate (route ⊆ gate-allowed). The wall check is a router-FIDELITY guarantee for
    /// PUBLIC walls (an undersampled chord between two corner-straddling samples could otherwise
    /// visually cross a wall the true route avoided) AND a secrecy gate whenever the `walls` slice
    /// carries geometry the requester cannot see. The caller closes the secrecy half by construction:
    /// `SceneEcs::pathfind` passes the PER-REQUESTER `move_walls(scene, Some(user))` set for a
    /// non-GM, so a `gm_only` wall never reaches this function on a non-GM's behalf and cannot
    /// truncate their route into a shape that discloses it.
```

Apply the same correction to `los_smooth`'s `chord_ok` doc if it repeats the "public geometry" claim.

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
slice to both engines; the executor keeps the authoritative set, so a
secret wall springs at execution instead of bending a player's preview.

Corrects navmesh.rs's claim that the wall check has no confidentiality
stake — true only for public walls."
```

---

## D9 — Player moves are request-only

### Task 4: Refuse non-GM token position writes; delete the traversal gate

**Files:**
- Modify: `src/server/src/ws/room.rs:216-…` (the non-GM gate block in `publish`)
- Test: `src/server/src/ws/room.rs` (in-module `#[cfg(test)]`)

**Interfaces:**
- Consumes: `SceneEcs::token_move(doc_id, changes) -> Option<(Uuid, (f64,f64), (f64,f64))>`.
- Produces: no new signatures. `publish` refuses a non-GM `Update` that changes a token's `/engine/x` or `/engine/y`.

**Design note:** refusal is strictly stricter than gating, so the *traversal* machinery goes: the `blocks_move` wall gate, the `line_traversal` traversed-cell set, the per-cell mask membership test over a path, and the coordinate-magnitude check (`TokenEngine::validate` bounds every write at ingress, unconditionally, so a point placement needs finiteness only). The *point-placement* machinery — scene-existence refusal, the `MovementRestriction` dispatch, the `visible_cache` memo, the deferred `revealed_pending`/`get_explored` pass — is RETAINED for Task 5's Create gate, not deleted.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn non_gm_token_position_update_is_refused() {
    let (room, ctx_player, token, _scene) = room_with_player_owned_token().await;
    let ops = vec![Operation::Update {
        doc_id: token,
        changes: vec![
            FieldChange { path: "/engine/x".into(), old: json!(50.0), new: json!(150.0), remove: false },
            FieldChange { path: "/engine/y".into(), old: json!(50.0), new: json!(50.0), remove: false },
        ],
    }];
    let err = room.publish(&ctx_player, ops).await.expect_err("a player may not write a token position");
    assert!(matches!(err, DataError::Forbidden), "refused as Forbidden, got {err:?}");
}

#[tokio::test]
async fn gm_token_position_update_still_succeeds_through_a_wall() {
    // A GM places a token where they like, walls included (M9 §5).
    let (room, ctx_gm, token, _scene) = room_with_gm_and_blocking_wall().await;
    let ops = vec![Operation::Update {
        doc_id: token,
        changes: vec![
            FieldChange { path: "/engine/x".into(), old: json!(50.0), new: json!(250.0), remove: false },
        ],
    }];
    room.publish(&ctx_gm, ops).await.expect("a GM position write is unconditional");
}

#[tokio::test]
async fn non_gm_wholesale_engine_write_that_moves_a_token_is_refused() {
    // Post-image detection: the refusal reads the committed /engine band, so replacing the whole
    // band cannot smuggle a position change past a per-path check.
    let (room, ctx_player, token, _scene) = room_with_player_owned_token().await;
    let ops = vec![Operation::Update {
        doc_id: token,
        changes: vec![FieldChange {
            path: "/engine".into(),
            old: json!({"x": 50.0, "y": 50.0}),
            new: json!({"x": 150.0, "y": 50.0}),
            remove: false,
        }],
    }];
    let err = room.publish(&ctx_player, ops).await.expect_err("a wholesale engine write is caught");
    assert!(matches!(err, DataError::Forbidden));
}

#[tokio::test]
async fn non_gm_non_position_token_update_still_succeeds() {
    // The refusal is scoped to position, not to token writes generally.
    let (room, ctx_player, token, _scene) = room_with_player_owned_token().await;
    let ops = vec![Operation::Update {
        doc_id: token,
        changes: vec![FieldChange {
            path: "/engine/rotation".into(), old: json!(0.0), new: json!(90.0), remove: false,
        }],
    }];
    room.publish(&ctx_player, ops).await.expect("a player may still rotate a token they own");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test non_gm_token_position_update_is_refused -- --nocapture`
Expected: FAIL — the move is currently *allowed* (it is a legal in-mask move), so `expect_err` panics.

- [ ] **Step 3: Replace the traversal gate with a refusal**

In the `if ctx.world_role != WorldRole::Gm` block, replace the per-`Update` traversal gating with:

```rust
            // D9: a non-GM may not write a token position at all. Gated movement is
            // request-only and server-executed (`ClientMsg::MoveRequest` → `execute_move`), which
            // is the only path that can gate each step, arrest a token partway, and stream the
            // authoritative trajectory. A client-authored position write cannot do any of those,
            // so it is refused rather than validated — strictly stricter than the traversal gate
            // this replaces, and it leaves `execute_move` as the SOLE implementation of the
            // per-cell movement decision (no second gate to keep in parity).
            //
            // `token_move` reads the COMMITTED post-image over the whole `/engine` band, so a
            // wholesale `/engine` write or duplicate `/engine/x` changes cannot present a safe
            // target while committing a moved one.
            //
            // GMs are exempt: a GM places a token where they choose, walls included (M9 §5).
            for op in &ops {
                if let Operation::Update { doc_id, changes } = op {
                    let scene = self.scene.read().await;
                    if scene.token_move(*doc_id, changes).is_some() {
                        return Err(DataError::Forbidden);
                    }
                }
            }
```

Hoist the `scene.read()` out of the loop rather than re-acquiring per op. Delete the now-unreachable traversal code: the `blocks_move` call, the `line_traversal`/traversed-cell set, the per-cell `visible.contains` test, and the coordinate-magnitude block. Keep `revealed_pending`, `visible_cache`, the scene-existence refusal, and the restriction dispatch — Task 5 consumes them. If Task 5 has not landed yet and the compiler reports them unused, mark them `#[allow(dead_code)]` with a comment naming Task 5, and remove the allow in Task 5.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src/server && cargo test`
Expected: the four new tests PASS. Pre-existing tests that asserted a *legal player drag succeeds* now fail — these are tests of the deleted capability. Convert each to assert refusal, and confirm every such test's intent was "a player's in-mask drag is allowed" (a capability the rule removes) and not something else. Tests asserting a player's ILLEGAL drag is refused keep passing unchanged.

- [ ] **Step 5: Verify no other non-GM position path survives**

Run: `cd src/server && cargo test && grep -rn "token_move" src/server/src`
Confirm every `token_move` caller is either this refusal or `execute_move`'s scene derivation.

- [ ] **Step 6: Commit**

```bash
cd src/server && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/server/src/ws/room.rs
git commit -m "feat(server/ws): player token movement is request-only

A non-GM Update that changes a token position is refused; players move
only via MoveRequest, which is the sole path that gates each step, can
arrest a token partway, and streams the authoritative trajectory. GMs are
unaffected and place tokens unconditionally (M9 §5).

Deletes publish's duplicated traversal gate, leaving execute_move as the
sole implementation of the per-cell movement decision."
```

---

### Task 5: Gate non-GM token Create placement through the movement mask

**Files:**
- Modify: `src/server/src/ws/room.rs` (the retained point-placement machinery)
- Test: `src/server/src/ws/room.rs`

**Interfaces:**
- Consumes: `visible_cells_cached(user, scene, lenient)`, `resolve_scene(scene).movement_restriction`, `Repository::get_explored`, `GridShape::cell_of`.
- Produces: no new public signatures.

**Why:** `room.rs:239` leaves `Operation::Create` ungated on the reasoning that the create capability is already privileged. `data/document.rs:531-532` asserts a world CAN grant `WorldRole::Player` `core:create` on `token`, so arbitrary player placement is reachable by configuration — and placing a token in an unseen room reveals it through the new token's own vision, a strictly larger capability than the movement D9 forbids.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn non_gm_token_create_outside_the_mask_is_refused() {
    let (room, ctx_player, scene) = room_with_player_create_capability_and_lit_corner().await;
    // (500,500) is outside the lit region and outside explored.
    let ops = vec![Operation::Create { doc: token_doc_at(scene, 500.0, 500.0) }];
    let err = room.publish(&ctx_player, ops).await.expect_err("placement in fog is refused");
    assert!(matches!(err, DataError::Forbidden));
}

#[tokio::test]
async fn non_gm_token_create_inside_the_mask_succeeds() {
    let (room, ctx_player, scene) = room_with_player_create_capability_and_lit_corner().await;
    let ops = vec![Operation::Create { doc: token_doc_at(scene, 50.0, 50.0) }];
    room.publish(&ctx_player, ops).await.expect("placement in a visible cell is allowed");
}

#[tokio::test]
async fn gm_token_create_anywhere_succeeds() {
    let (room, ctx_gm, scene) = room_with_gm_and_lit_corner().await;
    let ops = vec![Operation::Create { doc: token_doc_at(scene, 500.0, 500.0) }];
    room.publish(&ctx_gm, ops).await.expect("a GM places a token anywhere");
}

#[tokio::test]
async fn unrestricted_scene_ungates_non_gm_token_create() {
    let (room, ctx_player, scene) = room_with_player_create_and_unrestricted_scene().await;
    let ops = vec![Operation::Create { doc: token_doc_at(scene, 500.0, 500.0) }];
    room.publish(&ctx_player, ops).await.expect("Unrestricted ungates placement, as it ungates movement");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test non_gm_token_create_outside_the_mask -- --nocapture`
Expected: FAIL — Create is currently ungated, so `expect_err` panics.

- [ ] **Step 3: Gate Create placement on the same mask the movement gate uses**

Extend the non-GM block's op loop to handle `Operation::Create` for `doc_type == "token"`, reusing the retained restriction dispatch, `visible_cache` memo, and deferred explored fetch. Never compute a second mask — call the same accessor:

```rust
                if let Operation::Create { doc } = op {
                    // D9: a created token's position is authorized against the SAME mask the
                    // movement gate uses. Placement was previously ungated on the reasoning that
                    // `core:create` is already privileged, but a world can grant it to Player
                    // (data/document.rs), and placing a token in an unseen cell reveals that area
                    // through the new token's own vision — a strictly larger capability than the
                    // movement this same block refuses. Center-cell only: a placement is a point,
                    // not a traversal, so there is no supercover to walk.
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
                    let Some(cell) = scene.scene_grid_sizes().get(&scene_id).copied() else {
                        return Err(DataError::Forbidden); // no scene document ⇒ refuse, never default
                    };
                    let settings = scene.resolve_scene(scene_id);
                    let target = scene.resolve_grid_shape(scene_id, cell).cell_of((eng.x, eng.y));
                    match settings.movement_restriction {
                        MovementRestriction::Unrestricted => {}
                        MovementRestriction::Visible => {
                            let mask = visible_cache
                                .entry((scene_id, settings.partial_cell_leniency))
                                .or_insert_with(|| {
                                    scene.visible_cells(ctx.user_id, scene_id, settings.partial_cell_leniency)
                                });
                            if !mask.contains(&target) {
                                return Err(DataError::Forbidden);
                            }
                        }
                        MovementRestriction::Revealed => {
                            let mask = visible_cache
                                .entry((scene_id, settings.partial_cell_leniency))
                                .or_insert_with(|| {
                                    scene.visible_cells(ctx.user_id, scene_id, settings.partial_cell_leniency)
                                })
                                .clone();
                            // Explored needs an async fetch, which must not run under the scene
                            // read guard — defer exactly as the movement gate did.
                            revealed_pending.push((scene_id, [target].into_iter().collect(), mask));
                        }
                    }
                }
```

Keep the existing post-lock `revealed_pending` loop that fetches `get_explored` and checks `cells ⊆ mask ∪ explored`, failing closed on `Err`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src/server && cargo test`
Expected: PASS. Remove any `#[allow(dead_code)]` added in Task 4.

- [ ] **Step 5: Mutation-verify**

Change `!mask.contains(&target)` to `false`. Run: `cd src/server && cargo test non_gm_token_create_outside_the_mask`
Expected: FAIL. Revert and re-run to PASS.

- [ ] **Step 6: Commit**

```bash
cd src/server && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/server/src/ws/room.rs
git commit -m "fix(server/ws): gate non-GM token placement on the movement mask

A world can grant Player core:create on token, so ungated placement let a
player put a token in an unseen room and read it through that token's
vision — a larger capability than the movement the same block refuses.
Create placement now authorizes against the same mask accessor, center-cell
only. GM and Unrestricted are exempt, mirroring the movement gate."
```

---

### Task 6: Client — player drag becomes a move request

**Files:**
- Modify: `src/modules/scene-tools/src/controller.svelte.ts:810-821` (`sendMoves`)
- Test: `src/modules/scene-tools/src/controller.test.ts`

**Interfaces:**
- Consumes: `ctx.moveRequest(scene, tokenId, path)`, `ctx.pathfind(scene, start, waypoints, footprintRadius)`, `ctx.scene.previewOverlay`, `ctx.role`.
- Produces: `sendMoves(delta)` role-branched — unchanged batched `update` for a GM, one `moveRequest` per selected token for a non-GM.

- [ ] **Step 1: Write the failing tests**

```ts
it("a non-GM drag issues one moveRequest per selected token and zero update ops", async () => {
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }, { id: "t2", x: 100, y: 0 }] });
  h.select(["t1", "t2"]);
  await h.drag({ dx: 100, dy: 0 });
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `pnpm --filter @shadowcat/scene-tools test -- controller`
Expected: FAIL — the player case currently produces update ops and moves the optimistic document.

- [ ] **Step 3: Role-branch `sendMoves`**

```ts
  /** Commit a drag. A GM writes the position directly — a GM places a token where they choose,
   * walls included. A player's move is request-only: the server is the sole executor, so each
   * selected token gets its own pathfind + moveRequest and the token's rendered position moves
   * only when the resulting MoveStream arrives. Per-token, not batched: moveRequest is per-token
   * on the wire and the server gates each token independently, so one token arresting while
   * another completes is the correct outcome. */
  const sendMoves = (delta: Point): void => {
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
      const fp = footprintFor(id);
      pathfind(scene.id, [o.x, o.y], [[target.x, target.y]], fp, id).then(
        (result) => { if (result.path.length >= 2) moveRequest(scene.id, id, result.path); },
        () => { /* an unroutable drop is a no-op; the token never moved locally */ },
      );
    }
  };
```

`footprintFor(id)` resolves the per-token footprint the same way `resolveFootprint()` does at `controller.svelte.ts:393` (`const eff = resolveTokenActor(doc, ctx.documents); return eff ? footprintRadius(eff) : 0.4;`) but for an explicit token id rather than the single selection. Extract it as a helper and have `resolveFootprint()` call it, so the two cannot diverge.

The drag gesture's visual feedback stays a `previewOverlay` call in `onPointerMove` — do not introduce an optimistic document write for a player.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `pnpm --filter @shadowcat/scene-tools test -- controller && pnpm -r typecheck`
Expected: PASS. Typecheck is required separately — Vitest strips types via esbuild and will not catch a signature error.

- [ ] **Step 5: Commit**

```bash
pnpm -r test && pnpm -r typecheck && pnpm lint
git add src/modules/scene-tools/src/controller.svelte.ts src/modules/scene-tools/src/controller.test.ts
git commit -m "feat(client/scene-tools): player drag commits through a move request

A player's drag now pathfinds then sends one moveRequest per selected
token, and the token's rendered position advances only on the server's
MoveStream — no optimistic position write for a gated move. A GM's drag is
unchanged: a direct batched position write."
```

---

## D8 — GM gate-exemption unification

### Task 7: GMs bypass every gameplay gate in `execute_move`

**Files:**
- Modify: `src/server/src/scene/move_exec.rs` (`execute_move`)
- Modify: `src/server/src/ws/room.rs:553-557` (call site + the stale comment)
- Test: `src/server/src/scene/move_exec.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `execute_move(ecs, scene, token, path, restriction, visible, cell, is_gm: bool) -> Result<MoveOutcome, MoveReject>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn gm_move_crosses_a_blocks_move_wall_untruncated() {
    let (ecs, scene, token) = scene_with_wall_across_the_path();
    let path = vec![(50.0, 50.0), (150.0, 50.0), (250.0, 50.0)];
    let out = execute_move(&ecs, scene, token, &path, MovementRestriction::Unrestricted, &empty_mask(), 100.0, true)
        .expect("a GM move is admissible");
    assert!(!out.truncated, "a GM move is not truncated by a wall (M9 §5)");
    assert_eq!(out.render_path.last().copied(), Some((250.0, 50.0)), "the GM lands at the requested destination");
}

#[test]
fn gm_move_ignores_impassable_and_arrest_regions() {
    let (ecs, scene, token) = scene_with_impassable_then_arrest_region();
    let path = vec![(50.0, 50.0), (150.0, 50.0), (250.0, 50.0)];
    let out = execute_move(&ecs, scene, token, &path, MovementRestriction::Unrestricted, &empty_mask(), 100.0, true)
        .expect("admissible");
    assert!(!out.truncated, "neither impassable nor arrest stops a GM");
}

#[test]
fn non_gm_move_is_still_blocked_by_the_same_wall() {
    // The exemption must not widen: assert the non-GM behavior is unchanged.
    let (ecs, scene, token) = scene_with_wall_across_the_path();
    let out = execute_move(&ecs, scene, token, &vec![(50.0, 50.0), (150.0, 50.0)], MovementRestriction::Unrestricted, &empty_mask(), 100.0, false)
        .expect("admissible");
    assert!(out.truncated, "a non-GM is still stopped by the wall");
}

#[test]
fn gm_move_is_still_refused_beyond_the_coordinate_bound() {
    // I1: a GM bypasses gameplay gates and NO resource guard.
    let (ecs, scene, token) = scene_with_wall_across_the_path();
    let over = MAX_GATE_WALK_COORD + 1.0;
    let err = execute_move(&ecs, scene, token, &vec![(50.0, 50.0), (over, 50.0)], MovementRestriction::Unrestricted, &empty_mask(), 100.0, true)
        .expect_err("a resource guard is never exempted");
    assert!(matches!(err, MoveReject::TooLong), "got {err:?}");
}

#[test]
fn gm_move_still_accrues_terrain_cost() {
    // Cost is information, not a gate — accrual is independent of the exemption.
    let (ecs, scene, token) = scene_with_terrain_multiplier_3();
    let out = execute_move(&ecs, scene, token, &vec![(50.0, 50.0), (150.0, 50.0)], MovementRestriction::Unrestricted, &empty_mask(), 100.0, true)
        .expect("admissible");
    assert!(out.cost >= 3.0, "terrain still accrues for a GM, got {}", out.cost);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test gm_move_crosses_a_blocks_move_wall -- --nocapture`
Expected: FAIL — compile error (arity), then the wall/region assertions fail once it compiles.

- [ ] **Step 3: Add the exemption**

Add the parameter and gate each gameplay step. A single `is_gm` flag with early-outs is correct here rather than a shared profile struct: after D9 there is exactly one gate, and a shared symbol with one consumer is indirection with no second party to keep honest.

```rust
    // Gameplay gates apply to non-GMs only. A GM may make an illegal move: they move with or
    // without pathfinding, and a placement lands where asked (M9 §5). Resource guards above —
    // `gate_walk`'s MAX_GATE_WALK_COORD / MAX_GATE_WALK_SAMPLES, non-finite refusal, and the
    // scene-existence refusal — are NOT exempted for a GM and must stay unconditional.
    let check_walls = !is_gm;
    let check_regions = !is_gm;
    let check_mask = !is_gm && !matches!(restriction, MovementRestriction::Unrestricted);
```

In the walk loop, guard step 1 with `if check_walls && ecs.blocks_move(scene, prev, next)`, and in step 3 guard only the *stopping* decisions, leaving cost accrual unconditional:

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

- [ ] **Step 4: Update the call site and invert the stale comment**

In `room.rs`, pass `is_gm` (the same role check that decides `restriction`) and replace the comment at `:553-557`:

```rust
            // GMs are exempt from every gameplay gate here — walls, mask, impassable and arrest —
            // per the M9 design spec's "ignore walls" GM override (M9 §5), matching `publish`'s
            // GM position write. Resource guards (`gate_walk`'s coordinate/sample bounds, the
            // scene-existence refusal) stay unconditional for a GM.
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src/server && cargo test`
Expected: PASS. Existing GM-move tests that asserted wall blocking now fail — they encode the regression this task fixes. Update each to assert the M9 §5 behavior, and confirm from the assertion's intent that it was testing GM wall blocking specifically.

- [ ] **Step 6: Commit**

```bash
cd src/server && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/server/src/scene/move_exec.rs src/server/src/ws/room.rs
git commit -m "fix(server/scene): restore the GM ignore-walls override in execute_move

execute_move enforced walls and impassable/arrest against GMs, diverging
from the M9 design spec's GM override and from publish's own GM behavior.
GMs now bypass every gameplay gate and land at the requested destination;
resource guards stay unconditional, and terrain cost still accrues."
```

---

## D4 — Footprint-aware authoritative gate

### Task 8: Server-side footprint resolver mirroring the client

**Files:**
- Modify: `src/server/src/scene/mod.rs` (new `resolve_token_footprint`)
- Test: `src/server/src/scene/mod.rs`

**Interfaces:**
- Consumes: `eng::{TokenEngine, ActorEngine, TokenOverrides, Size}`, `self.actors`, the `resolveTokenActor` join `token_vision_floors` already implements (`mod.rs:1529-1548`).
- Produces: `SceneEcs::resolve_token_footprint(&self, token: Uuid) -> f64` and `pub(crate) const DEFAULT_FOOTPRINT_RADIUS_CELLS: f64 = 0.4;`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn footprint_radius_mirrors_the_client_table() {
    // Exact mirror of footprintRadius (src/client/core/src/actor.ts:177):
    //   circle ⇒ max(w,h)/2 ; square ⇒ hypot(w,h)/2
    let cases = [
        ("square", 1.0, 1.0, std::f64::consts::SQRT_2 / 2.0),
        ("square", 2.0, 2.0, std::f64::consts::SQRT_2),
        ("square", 1.0, 2.0, (1.0f64 * 1.0 + 2.0 * 2.0).sqrt() / 2.0),
        ("circle", 1.0, 1.0, 0.5),
        ("circle", 2.0, 3.0, 1.5),
    ];
    for (shape, w, h, expected) in cases {
        let (ecs, token) = scene_with_linked_token_sized(shape, w, h);
        let got = ecs.resolve_token_footprint(token);
        assert!(
            (got - expected).abs() < 1e-12,
            "shape={shape} w={w} h={h}: expected {expected}, got {got}"
        );
    }
}

#[test]
fn footprint_radius_falls_back_to_the_client_default_for_an_actorless_token() {
    let (ecs, token) = scene_with_raw_token_no_actor();
    assert_eq!(
        ecs.resolve_token_footprint(token),
        DEFAULT_FOOTPRINT_RADIUS_CELLS,
        "an actorless token uses the same 0.4 default the client's resolveFootprint uses"
    );
}

#[test]
fn footprint_radius_honors_a_per_token_size_override() {
    // resolveTokenActor's override whitelist applies to a LINKED token.
    let (ecs, token) = scene_with_linked_token_overriding_size("circle", 4.0, 4.0);
    assert!((ecs.resolve_token_footprint(token) - 2.0).abs() < 1e-12);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test footprint_radius_mirrors -- --nocapture`
Expected: FAIL — `resolve_token_footprint` does not exist.

- [ ] **Step 3: Implement the resolver**

```rust
/// The footprint radius used when no effective actor resolves. Mirrors the client's
/// `resolveFootprint` fallback (`src/modules/scene-tools/src/controller.svelte.ts:393`). This
/// value is PARITY-BOUND, not a fail-closed choice: it is more permissive than a 1×1 square's
/// 0.707 would be, and changing it here without changing the client re-forks the router and the
/// gate. Change both or neither.
pub(crate) const DEFAULT_FOOTPRINT_RADIUS_CELLS: f64 = 0.4;

    /// A token's bounding-disc radius in GRID UNITS (cells). Exact mirror of `footprintRadius`
    /// (`src/client/core/src/actor.ts:177`): a circle uses `max(w,h)/2`, a square its half-diagonal
    /// `hypot(w,h)/2` (conservative enclosure). Effective-actor resolution mirrors
    /// `resolveTokenActor` via the SAME join `token_vision_floors` implements: a LINKED token
    /// resolves the shared actor and applies the per-token override whitelist; a dangling link
    /// ignores overrides; an INSTANCED token uses its embedded copy and overrides do not apply.
    /// Falls back to `DEFAULT_FOOTPRINT_RADIUS_CELLS` when no actor resolves — the client's own
    /// fallback, so router and gate agree for an actorless token too.
    pub(crate) fn resolve_token_footprint(&self, token: Uuid) -> f64 {
        let Some((shape, size)) = self.token_shape_and_size(token) else {
            return DEFAULT_FOOTPRINT_RADIUS_CELLS;
        };
        let (w, h) = (size.w, size.h);
        if !w.is_finite() || !h.is_finite() || w < 0.0 || h < 0.0 {
            return DEFAULT_FOOTPRINT_RADIUS_CELLS;
        }
        if shape == "circle" {
            w.max(h) / 2.0
        } else {
            w.hypot(h) / 2.0
        }
    }
```

Implement the private `token_shape_and_size(&self, token: Uuid) -> Option<(String, eng::Size)>` by copying the branch structure of `token_vision_floors` (`mod.rs:1529-1548`) and reading `shape`/`size` instead of `vision`: linked (`token_eng.actor_id` → `self.actors.get(&id)`, then `overrides.shape`/`overrides.size` take precedence), dangling link → `None`, instanced → the embedded actor read through the deliberately-uncached direct `engine_as` path (an embedded actor's own id differs from the token's, so caching under either id goes stale).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src/server && cargo test footprint_radius`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cd src/server && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/server/src/scene/mod.rs
git commit -m "feat(server/scene): resolve a token's footprint radius server-side

Exact mirror of the client's footprintRadius, resolved through the same
token→actor join token_vision_floors uses, with the client's own 0.4
fallback for an actorless token. Pinned by a size-table parity test so the
router and the gate cannot derive different footprints."
```

---

### Task 9: `Pathfind` names its token; the server derives the footprint

**Files:**
- Modify: `src/server/src/ws/protocol.rs:69-76` (`Pathfind`)
- Modify: `src/server/src/ws/conn.rs:492-497,581,639`
- Modify: `src/client/core/src/wire.ts:381`, `src/client/core/src/ws-client.ts:574-596`
- Test: `src/server/src/ws/conn.rs`, `src/client/core/src/wire.test.ts`

**Interfaces:**
- Consumes: `resolve_token_footprint` (Task 8).
- Produces: wire field `token: Option<Uuid>` on `Pathfind`; client `pathfind(scene, start, waypoints, footprintRadius, token?)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn pathfind_naming_a_token_ignores_a_lying_wire_footprint() {
    // A client understating its footprint must not obtain a route the gate then refuses.
    let (h, scene, token) = harness_with_large_token_and_narrow_gap().await;
    // The wire value claims a tiny footprint; the server derives the real (large) one.
    let res = h.pathfind(scene, (50.0, 50.0), &[(450.0, 50.0)], 0.01, Some(token)).await;
    assert!(
        matches!(res, Err(PathFail::Unreachable)),
        "the derived footprint does not fit the gap, so no route is returned"
    );
}

#[tokio::test]
async fn pathfind_without_a_token_uses_the_wire_footprint() {
    let (h, scene, _token) = harness_with_large_token_and_narrow_gap().await;
    let res = h.pathfind(scene, (50.0, 50.0), &[(450.0, 50.0)], 0.01, None).await;
    assert!(res.is_ok(), "a token-less hypothetical preview honors the requested radius");
}
```

```ts
it("pathfind sends the token id when given", () => {
  const sent: unknown[] = [];
  const c = clientWithCapture(sent);
  c.pathfind("s1", [0, 0], [[100, 0]], 0.4, "t1");
  expect(sent[0]).toMatchObject({ type: "pathfind", footprint_radius: 0.4, token: "t1" });
});

it("the Pathfind schema accepts an absent token", () => {
  expect(() => PathfindSchema.parse({ type: "pathfind", request_id: "r", scene: "s", start: [0,0], waypoints: [[1,1]], footprint_radius: 0.4 })).not.toThrow();
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test pathfind_naming_a_token -- --nocapture` then `pnpm --filter @shadowcat/core test -- wire`
Expected: FAIL on both — the field does not exist.

- [ ] **Step 3: Add the wire field**

```rust
    /// A one-shot grid pathfinding request, correlated by `request_id`. `start`/`waypoints` are
    /// scene coords; `waypoints`' LAST element is the goal. The route is mask-bounded for non-GM
    /// requesters.
    ///
    /// `token`, when present, is the token the route is for: the server DERIVES the footprint from
    /// that token's document and IGNORES `footprint_radius`, so a route preview and the
    /// authoritative gate cannot disagree about the mover's size. The named token also serves as
    /// the non-GM presence proof. When absent, `footprint_radius` (grid units, the client's
    /// `footprintRadius`) is honored and the result is an explicitly hypothetical preview carrying
    /// no preview-equals-execution guarantee.
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

In `conn.rs`'s handler, derive when `token` is present:

```rust
    let footprint_radius = match token {
        Some(t) => scene_ecs.resolve_token_footprint(t),
        None => footprint_radius,
    };
```

Regenerate ts-rs types, then mirror in `wire.ts` (`token: z.string().uuid().nullish()`) and add the parameter to `ws-client.ts`'s `pathfind`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src/server && cargo test` then `pnpm build && pnpm -r test && pnpm -r typecheck`
Expected: PASS. `pnpm -r test` is the required gate for a shared wire-schema change — a new field breaks untyped frame fixtures across packages, and typecheck alone will not catch a dropped Zod field.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/ws/protocol.rs src/server/src/ws/conn.rs src/types/generated src/client/core/src/wire.ts src/client/core/src/ws-client.ts
git commit -m "feat(ws): Pathfind names its token so the server derives the footprint

A client-supplied footprint could understate a token's size and obtain a
route the authoritative gate then refuses. When token is present the server
derives the footprint from the document and ignores the wire value; the
token-less form stays a hypothetical preview with no parity claim."
```

---

### Task 10: The authoritative gate adopts the footprint predicate

**Files:**
- Modify: `src/server/src/scene/move_exec.rs` (`execute_move`)
- Modify: `src/server/src/ws/room.rs` (`execute_move` call site passes the derived footprint)
- Test: `src/server/src/scene/move_exec.rs`, `src/server/src/scene/grid_shape_parity_tests.rs`

**Interfaces:**
- Consumes: `resolve_token_footprint` (Task 8), `GridShape::{footprint_cells, line_traversal}`, `pathfinding::point_segment_distance`.
- Produces: `execute_move(..., is_gm: bool, footprint_radius_cells: f64)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn route_admissible_implies_gate_admissible_for_a_non_gm() {
    // I4: the equivalence, forward direction, on both grid kinds and both movement models.
    for kind in ["square", "hex"] {
        for model in [MovementModel::GridStepped, MovementModel::Continuous] {
            let (ecs, scene, token, user) = scene_with_narrow_gap_and_wide_token(kind, model);
            let fp = ecs.resolve_token_footprint(token);
            let mask = ecs.visible_cells(user, scene, false);
            if let Ok(route) = ecs.pathfind(user, scene, (50.0, 50.0), &[(450.0, 50.0)], fp, false, None) {
                let out = execute_move(&ecs, scene, token, &route.path, MovementRestriction::Visible, &mask, 100.0, false, fp)
                    .expect("a routed path is admissible");
                assert!(!out.truncated, "kind={kind} model={model:?}: the gate accepts every routed step");
            }
        }
    }
}

#[test]
fn a_wide_token_cannot_enter_a_cell_whose_footprint_overlaps_fog() {
    let (ecs, scene, token, user) = scene_with_lit_center_line_only();
    let fp = ecs.resolve_token_footprint(token); // a Large token, radius > 0.5
    let mask = ecs.visible_cells(user, scene, false);
    let out = execute_move(&ecs, scene, token, &vec![(50.0, 50.0), (150.0, 50.0)], MovementRestriction::Visible, &mask, 100.0, false, fp)
        .expect("admissible");
    assert!(out.truncated, "a footprint cell outside the mask stops a wide token");
}

#[test]
fn a_sub_half_cell_footprint_diagonal_stays_admissible() {
    // The buddy-check P1 case: a small footprint's diagonal must not regress.
    let (ecs, scene, token, user) = scene_with_open_lit_area();
    let mask = ecs.visible_cells(user, scene, false);
    let out = execute_move(&ecs, scene, token, &vec![(50.0, 50.0), (150.0, 150.0)], MovementRestriction::Visible, &mask, 100.0, false, 0.4)
        .expect("admissible");
    assert!(!out.truncated, "a 0.4-radius diagonal step is still allowed");
}

#[test]
fn a_gm_is_exempt_from_every_footprint_check() {
    let (ecs, scene, token, _user) = scene_with_narrow_gap_and_wide_token("square", MovementModel::GridStepped);
    let out = execute_move(&ecs, scene, token, &vec![(50.0, 50.0), (450.0, 50.0)], MovementRestriction::Unrestricted, &empty_mask(), 100.0, true, 5.0)
        .expect("admissible");
    assert!(!out.truncated, "a GM squeezes a wide token through anything");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src/server && cargo test route_admissible_implies_gate_admissible -- --nocapture`
Expected: FAIL — compile error (arity), then `a_wide_token_cannot_enter_a_cell_whose_footprint_overlaps_fog` fails because the gate is center-based.

- [ ] **Step 3: Adopt the router's three checks**

Both halves of the mask predicate must come from the resolved `GridShape`, never the free square functions — calling `pathfinding::footprint_cells` or `movement::supercover_cells` here reintroduces the square-on-hex defect Task 14e-7 fixed.

```rust
        // Step 1: wall gate — footprint-disc clearance, the SAME predicate
        // `pathfinding::cell_enterable` applies, so route-admissible ⇔ gate-admissible for a
        // non-GM (I4). Replaces the center-based segment-cross test: a center path can thread a
        // gap the token's body does not fit through.
        if check_walls {
            let r_scene = footprint_radius_cells * cell;
            if move_walls_authoritative
                .iter()
                .any(|w| pathfinding::point_segment_distance(next, w.a, w.b) < r_scene)
            {
                stopped_early = true;
                break;
            }
        }

        // Step 2: vision-mask gate over the FOOTPRINT, not the center — the same
        // `footprint_cells ∪ line_traversal` union `cell_enterable` requires. Both halves come
        // from the resolved shape; the free square functions are SquareGrid internals and would
        // test square-indexed cells against a hex mask.
        if check_mask {
            let Some(mut cells) = grid.line_traversal(prev, next, cell) else {
                stopped_early = true;
                break;
            };
            cells.extend(grid.footprint_cells(to_cell(next), next, footprint_radius_cells * cell, cell));
            if !cells.iter().all(|c| visible.contains(c)) {
                stopped_early = true;
                break;
            }
        }
```

For step 3, test impassable/arrest over the footprint cells rather than the center cell alone, keying the transition dedup on the center cell as today (the dedup's purpose is per-cell-entry accounting, which stays center-based):

```rust
            let fp_cells = grid.footprint_cells(next_cell, next, footprint_radius_cells * cell, cell);
            if check_regions && fp_cells.iter().any(|c| regions.is_impassable(*c)) {
                stopped_early = true;
                break;
            }
            cost += regions.terrain_multiplier(next_cell);
            if check_regions && fp_cells.iter().any(|c| regions.is_arrest(*c)) {
                stop_idx = i;
                stopped_early = true;
                break;
            }
```

Retain the authoritative wall set for step 1: `execute_move` reads `ecs.move_walls(scene, None)` once before the loop (never per step, never per-requester).

- [ ] **Step 4: Update the call site**

In `room.rs`, pass `scene_ecs.resolve_token_footprint(token)` alongside `is_gm`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src/server && cargo test`
Expected: PASS. The frozen king-step parity fixtures may shift where a fixture token's footprint now clears differently — for each change, confirm from the fixture's geometry that the new outcome is correct under footprint semantics before updating it, and record why in the commit body. A fixture that changes for an unexplained reason is a defect signal, not a fixture to rewrite.

- [ ] **Step 6: Commit**

```bash
cd src/server && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/server/src/scene/move_exec.rs src/server/src/ws/room.rs
git commit -m "feat(server/scene): footprint-aware authoritative movement gate

execute_move adopts the router's footprint predicate — disc-vs-wall
clearance, footprint_cells ∪ line_traversal mask membership, and
footprint-wide impassable/arrest — so route-admissible ⇔ gate-admissible
for a non-GM. Both mask halves come from the resolved GridShape, never the
square-only free functions. GMs are exempt."
```

---

### Task 11: Client sends the token on every route request

**Files:**
- Modify: `src/modules/scene-tools/src/controller.svelte.ts` (`requestRoute`, `commitRoute`)
- Test: `src/modules/scene-tools/src/controller.test.ts`

**Interfaces:**
- Consumes: `ctx.pathfind(scene, start, waypoints, footprintRadius, token?)` (Task 9).
- Produces: no new signatures.

- [ ] **Step 1: Write the failing test**

```ts
it("a route preview names the token it is for", async () => {
  const h = harness({ role: "player", tokens: [{ id: "t1", x: 0, y: 0 }] });
  h.select(["t1"]);
  await h.previewRouteTo({ x: 300, y: 0 });
  expect(h.pathfindCalls[0]).toMatchObject({ token: "t1" });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `pnpm --filter @shadowcat/scene-tools test -- controller`
Expected: FAIL — `token` is undefined in the recorded call.

- [ ] **Step 3: Pass the token**

In `requestRoute` (`controller.svelte.ts:404`) and `commitRoute`'s fallback pathfind, pass the selected token id as the new fifth argument. The measure tool already resolves its footprint from that token, so this makes the server derive the same value rather than trusting the wire.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `pnpm -r test && pnpm -r typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
pnpm -r test && pnpm -r typecheck && pnpm lint
git add src/modules/scene-tools/src/controller.svelte.ts src/modules/scene-tools/src/controller.test.ts
git commit -m "feat(client/scene-tools): route requests name their token

The server derives the footprint from the named token, so a preview and the
authoritative gate agree on the mover's size."
```

---

### Task 12: Documentation and skill sync

**Files:**
- Modify: `docs/PLAN.md:345`, `docs/TODO.md`, `docs/POST_WORK_FINDINGS.md`, `docs/CLOSED_BUGS.md`
- Modify: `docs/superpowers/specs/2026-07-24-phase1-closeout-campaign-design.md` (Phase D amendment note)
- Modify: `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md`

- [ ] **Step 1: Invert the stale GM-enforcement statements**

`docs/PLAN.md:345` states `execute_move` is "GM wall-honored, diverging from `publish`'s legacy GM wall-bypass". Replace with the M9 §5 rule: a GM bypasses every gameplay gate on both paths.

- [ ] **Step 2: Update the scene-rendering skill**

Four edits, each reviewed for accuracy against the merged diff:

1. **Parity checklist** — record that `publish` no longer gates non-GM traversal, so `execute_move` is the sole implementation of the per-cell decision and axes 1-3 describe a fork that no longer exists. Keep the axes documented as history-free present constraints on any future second gate (**I2**).
2. **Wall sets** — add the `move_walls(scene, viewer)` two-value contract and **I5** (vision/lighting keep the full set; do not unify).
3. **GM exemption** — replace the "Do NOT re-grant GM wall-bypass" text at `802-804` with **I1** (gameplay gates exempt, resource guards never).
4. **Footprint asymmetry** — the region gotcha's "center-cell-only in `move_exec`, a deliberate asymmetry" is retired; the gate is now footprint-aware and `⇔` holds for non-GMs modulo secret regions and `gm_only` walls (**I4**).

- [ ] **Step 3: Close the findings entries**

Mark resolved in `POST_WORK_FINDINGS.md`: "Route stricter than the authoritative gate" (D4). Move any newly-closed bug to `CLOSED_BUGS.md` with its root cause. Do NOT close the D-β entries (bounds units, hex cost, `env_light_polys`, lighting polish) — they belong to the next phase.

- [ ] **Step 4: Amend the campaign spec**

Add a Phase D amendment note recording the D-α/D-β split, the three added items (D10, D9, D8), and that D5 shipped in `513aef8`/`e1156ae`.

- [ ] **Step 5: Dispatch the reviewed skill-update gate**

Dispatch `shadowcat-spec-reviewer` on the skill diff specifically, confirming each edit accurately captures the implemented change with no omission, drift, or broken pointer. Per `reviewed-skill-update-gate-needs-its-own-adversarial-check`, a single clean pass is not sufficient assurance on its own — the whole-branch two-reviewer pair also covers it.

- [ ] **Step 6: Commit**

```bash
git add docs .claude/skills
git commit -m "docs(skills): movement authority unified — Phase D-alpha doc sync

Records the single-gate collapse, the routing/vision wall-set split, the
GM gameplay-vs-resource exemption rule, and the retirement of the
center-cell gate asymmetry. Inverts the stale GM-wall-enforcement intent
in favor of M9 section 5."
```

---

## Self-Review

**Spec coverage.** D10 → Tasks 1-3. D9 → Tasks 4-6. D8 → Task 7. D4 → Tasks 8-11. Cross-cutting: **I1** Task 7 (+ the resource-guard test), **I2** Task 4, **I3** Task 1, **I4** Task 10, **I5** Tasks 1 and 12. Doc/skill obligations → Task 12. The spec's `navmesh_for` cache decision → Task 2. The spec's Create gap → Task 5. No spec section is unimplemented.

**Placeholder scan.** No TBD/TODO markers. Every code step carries real code; every test step names an exact command and expected result. Fixture helpers are named and their construction described where not shown in full (`wall_doc_eng`, `scene_with_*`) — each references an existing in-repo fixture whose shape it follows.

**Type consistency.** `move_walls(scene, viewer)` (Task 1) is consumed with that arity in Tasks 2, 3, 10. `navmesh_for(scene, footprint, walls)` (Task 2) is called with three arguments in Task 3. `execute_move` gains `is_gm` in Task 7 and `footprint_radius_cells` in Task 10 — Task 10's tests use the full nine-argument form and Task 7's the eight-argument form, matching the order each task establishes. `resolve_token_footprint` (Task 8) returns `f64`, consumed as such in Tasks 9 and 10. `DEFAULT_FOOTPRINT_RADIUS_CELLS` is defined in Task 8 and referenced only there. `pathfind`'s client signature gains `token` in Task 9 and is used in Tasks 6 and 11.

**Known ordering coupling.** Task 4 may leave the point-placement machinery temporarily unused; it is marked `#[allow(dead_code)]` there and consumed in Task 5. Tasks 4 and 5 must land in that order and should not be split across a merge boundary.
