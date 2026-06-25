# M10e-4 — Movement Restriction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** At the `Room::publish` chokepoint, reject a non-GM token move whose path leaves the moving user's visibility mask — entire move (supercover) must lie within `visible` or `revealed` (per scene config), with partial-cell leniency; GM exempt; rejected with `DataError::Forbidden` before the write so it consumes no seq.

**Architecture:** Server-only (no client, no protocol/wire change). Extends the existing M9a non-GM movement gate in `src/server/src/ws/room.rs` (which already rejects `blocksMove`-wall crossings). After the wall check, the gate resolves the scene's `movementRestriction` and, when restricted, rasterizes the move segment `a0→a1` to its supercover cells and requires every cell to be in the user's mask. The mask is the **same** lighting-aware visibility computed for egress (the M10e-2 secrecy gate) so the gate can never forbid what was shipped as visible, nor permit what wasn't (§13). A new `visible_cells(user, scene, lenient)` method reuses the exact `player_lit_mask` primitives via extracted seams; `revealed` adds the server-persisted explored set.

**Tech Stack:** Rust, `hecs` ECS, `tokio` (async publish path), existing `vision`/`lighting`/`explored` scene modules, `sqlx` (explored blob).

## Global Constraints

- **Server crate is `shadowcat`** (NOT `shadowcat-server`). Build/test: `cargo test -p shadowcat`, `cargo fmt`, `cargo clippy -p shadowcat --all-targets -- -D warnings`.
- **Cross-platform** — pure Rust, no OS-specific paths, no platform-gated code. Determinism: no `HashMap` iteration into ordered/wire output (use `BTreeSet`/`BTreeMap` or pre-sorted `Vec`).
- **Secrecy gate fails closed** (`fog-is-the-secrecy-gate-fail-closed`): a missing/garbled/degenerate signal hides everything — an empty or unresolvable mask rejects the move; an oversized move (DoS) is rejected, not truncated-then-allowed.
- **Server mirrors client resolver exactly** (`server-mirrors-client-resolver-semantics`): the `movementRestriction`/`partialCellLeniency` resolution must equal `resolveSceneSettings` in `src/client/core/src/scene-docs.ts` — verify against that source, not this paraphrase.
- **Gate and egress use the SAME mask** (spec §13): `visible_cells` under the strict rule must equal `player_lit_mask`'s cells for that scene; do not fork the visibility math.
- **No debug code**: leveled `tracing` only; no `println!`/`dbg!`.
- **Comments**: present-tense current-state, cite algorithm sources, lead with invariants/coupling (project `CLAUDE.md`).

**Authoritative inputs (read before coding):**
- Spec §8 (Movement restriction) + §13 (Security): `docs/superpowers/specs/2026-06-24-m10e-vision-lighting-movement-design.md`.
- The existing M9a gate: `src/server/src/ws/room.rs:171-214` (`Room::publish`).
- The egress mask: `src/server/src/scene/mod.rs:769-1022` (`player_lit_mask`), `:301-392` (`resolve_scene`), `:486-506` (`token_move`), `:1027+` (`blocks_move`).
- Explored: `src/server/src/scene/explored.rs`, `src/server/src/data/sqlite.rs:442-481` (`get_explored`/`set_explored`).
- Client resolver: `src/client/core/src/scene-docs.ts:212-232` (`resolveSceneSettings`), `:77-90` (`DEFAULT_WORLD_SETTINGS`).

---

## File Structure

- **Modify** `src/server/src/scene/mod.rs`
  - `ResolvedScene` struct gains `movement_restriction: MovementRestriction`, `partial_cell_leniency: bool`.
  - New `MovementRestriction` enum + `parse_movement_restriction`.
  - `resolve_scene` parses the two new fields (mirrors the client).
  - Extract reusable seams from `player_lit_mask`: `cell_visible` (free fn), `lighting_inputs` (method), `source_los_poly` (free fn) — `player_lit_mask` delegates to them (behavior-preserving).
  - New `visible_cells(&self, user, scene, lenient) -> BTreeSet<(i32, i32)>` method.
  - `scene/movement` module declared (`mod movement;`).
- **Create** `src/server/src/scene/movement.rs`
  - Pure `supercover_cells(a0, a1, cell) -> Option<BTreeSet<(i32, i32)>>` (None ⇒ over-cap, reject) + `MAX_MOVE_CELLS`.
- **Modify** `src/server/src/ws/room.rs`
  - The non-GM block in `Room::publish` gains the movement-restriction check after `blocks_move`.
  - New `#[tokio::test]`s for visible/revealed/unrestricted/entire-move/leniency/no-seq.
- **Docs (closeout, Task 6)**: `docs/PLAN.md`, `docs/TODO.md`, `docs/POST_WORK_FINDINGS.md`, skill `.claude/skills/shadowcat-codebase-scene-rendering`.

---

### Task 1: `MovementRestriction` config resolution

Resolve `movementRestriction` (scene-overridable) and `partialCellLeniency` (world-only) into `ResolvedScene`, mirroring `resolveSceneSettings`.

**Files:**
- Modify: `src/server/src/scene/mod.rs:27-35` (`ResolvedScene`), `:301-392` (`resolve_scene`)
- Test: `src/server/src/scene/mod.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `pub enum MovementRestriction { Visible, Revealed, Unrestricted }` (derive `Clone, Copy, Debug, PartialEq, Eq`).
  - `ResolvedScene.movement_restriction: MovementRestriction`, `ResolvedScene.partial_cell_leniency: bool`.

- [ ] **Step 1: Write the failing tests**

Add to the existing `#[cfg(test)] mod tests` in `mod.rs`. These mirror `scene-docs.test.ts` cases (default, world override, scene override, partial-doc fallback, leniency world-only).

```rust
#[test]
fn resolve_scene_movement_restriction_defaults_to_visible_and_lenient() {
    // No world-settings doc, no scene override → built-in defaults.
    let ecs = SceneEcs::new();
    let r = ecs.resolve_scene(Uuid::from_u128(1));
    assert_eq!(r.movement_restriction, MovementRestriction::Visible);
    assert!(r.partial_cell_leniency);
}

#[test]
fn resolve_scene_movement_restriction_world_override_and_leniency_off() {
    use serde_json::json;
    let mut ecs = SceneEcs::new();
    // A complete world-settings system (scene+pathfinding+animation) so the structural guard passes.
    ecs.set_world_settings_for_test(json!({
        "scene": { "losRestriction": true, "fog": true, "lightingEnabled": true,
                   "lightMode": "environmentLight", "environment": {"color":"#0a0e1a","intensity":0.0},
                   "observerVision": false, "movementRestriction": "revealed", "partialCellLeniency": false },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    let r = ecs.resolve_scene(Uuid::from_u128(1));
    assert_eq!(r.movement_restriction, MovementRestriction::Revealed);
    assert!(!r.partial_cell_leniency, "partialCellLeniency is world-only and was set false");
}

#[test]
fn resolve_scene_movement_restriction_scene_override_beats_world() {
    use serde_json::json;
    let mut ecs = SceneEcs::new();
    let scene_id = Uuid::from_u128(7);
    ecs.set_world_settings_for_test(json!({
        "scene": { "movementRestriction": "visible", "partialCellLeniency": true },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    // Scene overrides vision.movementRestriction to "unrestricted".
    ecs.insert_scene_for_test(scene_id, json!({
        "grid": { "kind": "square", "size": 100 },
        "vision": { "movementRestriction": "unrestricted" }
    }));
    let r = ecs.resolve_scene(scene_id);
    assert_eq!(r.movement_restriction, MovementRestriction::Unrestricted);
    // partialCellLeniency has NO scene override → still the world default (true here).
    assert!(r.partial_cell_leniency);
}

#[test]
fn resolve_scene_movement_restriction_null_override_inherits_world() {
    use serde_json::json;
    let mut ecs = SceneEcs::new();
    let scene_id = Uuid::from_u128(8);
    ecs.set_world_settings_for_test(json!({
        "scene": { "movementRestriction": "revealed", "partialCellLeniency": true },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    // null clears the override → inherit world "revealed" (mirrors `?? d.scene.movementRestriction`).
    ecs.insert_scene_for_test(scene_id, json!({
        "grid": { "kind": "square", "size": 100 },
        "vision": { "movementRestriction": null }
    }));
    let r = ecs.resolve_scene(scene_id);
    assert_eq!(r.movement_restriction, MovementRestriction::Revealed);
}
```

If `set_world_settings_for_test` / `insert_scene_for_test` helpers do not already exist in the test module, add minimal `#[cfg(test)]` helpers on `SceneEcs` that set `self.world_settings = Some(doc_with_system(json))` and spawn a scene entity into `self.world`/`self.index`. Reuse `crate::data::document::tests::world_scoped_doc` for the document shells (see how `room.rs:632` builds config docs). Keep them test-only.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat resolve_scene_movement_restriction`
Expected: FAIL — `no field movement_restriction on ResolvedScene` / `cannot find type MovementRestriction`.

- [ ] **Step 3: Add the enum, struct fields, and parsing**

In `mod.rs`, near `LightMode`:

```rust
/// Per-scene movement gate mode. Mirrors `MovementRestriction` in `scene-docs.ts`.
/// `Visible` = move cells must be currently visible; `Revealed` = visible ∪ explored memory;
/// `Unrestricted` = walls only (the M9a gate alone).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovementRestriction {
    Visible,
    Revealed,
    Unrestricted,
}

/// Parse a movement-restriction string; any unknown/missing value fails closed to `Visible`
/// (the most restrictive non-frozen mode — never silently widens to `Unrestricted`).
fn parse_movement_restriction(s: &str) -> MovementRestriction {
    match s {
        "revealed" => MovementRestriction::Revealed,
        "unrestricted" => MovementRestriction::Unrestricted,
        _ => MovementRestriction::Visible,
    }
}
```

Add to `ResolvedScene`:

```rust
pub struct ResolvedScene {
    pub los_restriction: bool,
    pub fog: bool,
    pub observer_vision: bool,
    pub lighting_enabled: bool,
    pub light_mode: LightMode,
    pub env_color: u32,
    pub env_intensity: f64,
    pub movement_restriction: MovementRestriction,
    pub partial_cell_leniency: bool,
}
```

In `resolve_scene`, in the world-default layer (after `d_env_int`), add:

```rust
// movementRestriction: scene `vision.movementRestriction` ?? world ?? "visible".
let d_move = ws_scene
    .and_then(|s| s.get("movementRestriction"))
    .and_then(|v| v.as_str())
    .unwrap_or("visible");
// partialCellLeniency: world-only (no per-scene override; mirrors `d.scene.partialCellLeniency`).
let d_lenient = ws_scene
    .and_then(|s| s.get("partialCellLeniency"))
    .and_then(|v| v.as_bool())
    .unwrap_or(true);
```

In the scene-override layer (after `env_int`), add the scene override for movement only:

```rust
// Scene may override movementRestriction (string); null/absent ⇒ inherit world. Mirrors
// `v.movementRestriction ?? d.scene.movementRestriction`. partialCellLeniency has no scene override.
let move_str = s
    .and_then(|s| s.pointer("/vision/movementRestriction"))
    .and_then(|v| v.as_str())
    .unwrap_or(d_move);
```

In the returned struct literal, add:

```rust
            movement_restriction: parse_movement_restriction(move_str),
            partial_cell_leniency: d_lenient,
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat resolve_scene_movement_restriction`
Expected: PASS (4 tests).

- [ ] **Step 5: Verify no regression + lint**

Run: `cargo test -p shadowcat scene:: && cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "feat(m10e-4): resolve movementRestriction + partialCellLeniency into ResolvedScene"
```

---

### Task 2: Supercover rasterizer (`scene/movement.rs`)

A pure function returning every grid cell the move segment `a0→a1` passes through. Supercover (not thin Bresenham): a diagonal that clips a cell corner includes *both* flanking cells, so the gate cannot be slipped through an unseen cell a thin line would skip.

**Files:**
- Create: `src/server/src/scene/movement.rs`
- Modify: `src/server/src/scene/mod.rs` (add `mod movement;` and re-export if needed)
- Test: `src/server/src/scene/movement.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces: `pub fn supercover_cells(a0: (f64, f64), a1: (f64, f64), cell: f64) -> Option<BTreeSet<(i32, i32)>>`
  - Returns `Some(set)` of cells (world coords ÷ `cell`, floored). Always includes the cells of both endpoints.
  - Returns `None` when `cell <= 0.0` (degenerate; caller fails closed) or the candidate span exceeds `MAX_MOVE_CELLS` (DoS guard; caller rejects).
- Produces: `pub(crate) const MAX_MOVE_CELLS: i64`

- [ ] **Step 1: Write the failing tests**

```rust
//! Tests live alongside the implementation below.
#[cfg(test)]
mod tests {
    use super::*;

    fn cells(a0: (f64, f64), a1: (f64, f64), cell: f64) -> std::collections::BTreeSet<(i32, i32)> {
        supercover_cells(a0, a1, cell).expect("within cap")
    }

    #[test]
    fn single_cell_when_endpoints_share_a_cell() {
        // a0 == a1 (no-op) and a tiny intra-cell move both → exactly the one cell.
        let c = cells((50.0, 50.0), (50.0, 50.0), 100.0);
        assert_eq!(c.len(), 1);
        assert!(c.contains(&(0, 0)));
        let c2 = cells((10.0, 10.0), (90.0, 90.0), 100.0);
        assert_eq!(c2, c, "still inside cell (0,0)");
    }

    #[test]
    fn horizontal_move_covers_each_crossed_cell() {
        // (50,50)->(250,50) at cell 100 crosses cells x=0,1,2 at row 0.
        let c = cells((50.0, 50.0), (250.0, 50.0), 100.0);
        assert!(c.contains(&(0, 0)) && c.contains(&(1, 0)) && c.contains(&(2, 0)));
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn pure_diagonal_through_corner_includes_both_flanking_cells() {
        // (50,50)->(150,150): the line passes exactly through the shared corner (100,100).
        // Supercover includes the two diagonal cells AND both off-diagonal flankers — a thin
        // line would visit only (0,0),(1,1) and let a move slip past an unseen (1,0)/(0,1).
        let c = cells((50.0, 50.0), (150.0, 150.0), 100.0);
        assert!(c.contains(&(0, 0)) && c.contains(&(1, 1)));
        assert!(
            c.contains(&(1, 0)) || c.contains(&(0, 1)),
            "supercover includes at least one corner-flanking cell"
        );
    }

    #[test]
    fn endpoints_always_present_for_a_sloped_move() {
        let c = cells((50.0, 50.0), (370.0, 130.0), 100.0);
        assert!(c.contains(&(0, 0)), "start cell present");
        assert!(c.contains(&(3, 1)), "end cell present");
    }

    #[test]
    fn nonpositive_cell_is_none() {
        assert!(supercover_cells((0.0, 0.0), (10.0, 10.0), 0.0).is_none());
    }

    #[test]
    fn oversized_move_exceeds_cap_returns_none() {
        // cell 1, a 10_000-long move → > MAX_MOVE_CELLS candidate span → None (caller rejects).
        assert!(supercover_cells((0.0, 0.0), (10_000.0, 10_000.0), 1.0).is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat scene::movement`
Expected: FAIL — module/function does not exist.

- [ ] **Step 3: Implement the rasterizer**

```rust
//! Movement-segment rasterization for the M10e-4 movement-restriction gate. Pure, clean-room,
//! headless. INVARIANT: `supercover_cells` is the SAME cell set the gate tests against the
//! visibility mask, so the authoritative move gate and (M10e-6) path preview agree.

use std::collections::BTreeSet;

/// A grid cell coordinate `(i, j)`; cell `(i,j)` covers `[i*cell,(i+1)*cell) × [j*cell,(j+1)*cell)`.
pub type Cell = (i32, i32);

/// DoS guard: a single move may not rasterize more than this many candidate cells. A non-GM
/// move spanning more is rejected (fail-closed), never truncated. Sized to a generous drag at a
/// fine grid; far below a coordinate-overflow stall.
pub(crate) const MAX_MOVE_CELLS: i64 = 1_000_000;

/// Every grid cell the segment `a0→a1` passes through (supercover, not a thin line). Source:
/// supercover line of Euclidean segments — the symmetric extension of Amanatides & Woo (1987)
/// voxel traversal that also emits both cells flanking a shared corner, chosen over Bresenham so
/// a diagonal cannot thread an unseen cell. Both endpoint cells are always included.
///
/// `None` ⇒ caller must fail closed: `cell <= 0.0` (degenerate grid) or the candidate span
/// exceeds `MAX_MOVE_CELLS`.
pub fn supercover_cells(a0: (f64, f64), a1: (f64, f64), cell: f64) -> Option<BTreeSet<Cell>> {
    if cell <= 0.0 {
        return None;
    }
    let to_cell = |v: f64| (v / cell).floor() as i32;
    let (x0, y0) = a0;
    let (x1, y1) = a1;
    let (mut ci, mut cj) = (to_cell(x0), to_cell(y0));
    let (ei, ej) = (to_cell(x1), to_cell(y1));

    // Span guard (bbox of endpoint cells) before any allocation/iteration.
    let span = (ci as i64 - ei as i64).abs().saturating_add(1)
        .saturating_mul((cj as i64 - ej as i64).abs().saturating_add(1));
    if span > MAX_MOVE_CELLS {
        return None;
    }

    let mut out = BTreeSet::new();
    out.insert((ci, cj));
    if (ci, cj) == (ei, ej) {
        return Some(out); // intra-cell move (covers a0 == a1)
    }

    let dx = x1 - x0;
    let dy = y1 - y0;
    let step_i = if dx > 0.0 { 1 } else { -1 };
    let step_j = if dy > 0.0 { 1 } else { -1 };

    // Parametric grid traversal: tMaxI/tMaxJ = parameter t∈[0,1] at the next vertical/horizontal
    // grid line; tDeltaI/tDeltaJ = t advance per full cell. A near-zero component yields INFINITY
    // (that axis never steps), so axis-aligned moves degrade to a 1-D walk.
    let next_boundary = |c: i32, step: i32, origin: f64, d: f64| -> f64 {
        if d == 0.0 {
            return f64::INFINITY;
        }
        let line = if step > 0 { (c + 1) as f64 * cell } else { c as f64 * cell };
        (line - origin) / d
    };
    let mut t_max_i = next_boundary(ci, step_i, x0, dx);
    let mut t_max_j = next_boundary(cj, step_j, y0, dy);
    let t_delta_i = if dx != 0.0 { (cell / dx).abs() } else { f64::INFINITY };
    let t_delta_j = if dy != 0.0 { (cell / dy).abs() } else { f64::INFINITY };

    let mut guard: i64 = 0;
    while (ci, cj) != (ei, ej) {
        guard += 1;
        if guard > MAX_MOVE_CELLS {
            return None; // belt-and-suspenders against a pathological loop
        }
        if (t_max_i - t_max_j).abs() < f64::EPSILON {
            // Exact corner crossing: emit BOTH flanking cells (supercover), then step diagonally.
            out.insert((ci + step_i, cj));
            out.insert((ci, cj + step_j));
            ci += step_i;
            cj += step_j;
            t_max_i += t_delta_i;
            t_max_j += t_delta_j;
        } else if t_max_i < t_max_j {
            ci += step_i;
            t_max_i += t_delta_i;
        } else {
            cj += step_j;
            t_max_j += t_delta_j;
        }
        out.insert((ci, cj));
    }
    Some(out)
}
```

In `mod.rs`, add the module declaration near the other `mod` lines (e.g. beside `mod explored;`):

```rust
mod movement;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat scene::movement`
Expected: PASS (6 tests).

- [ ] **Step 5: Lint**

Run: `cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/movement.rs src/server/src/scene/mod.rs
git commit -m "feat(m10e-4): supercover move-segment rasterizer (DoS-capped)"
```

---

### Task 3: Extract reusable visibility seams from `player_lit_mask`

Factor the per-cell visibility predicate and the per-scene lighting/wall setup out of `player_lit_mask` so `visible_cells` (Task 4) consumes the **identical** math (spec §13 anti-drift). Behavior-preserving: all existing M10e-2/-3 tests stay green.

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`player_lit_mask` body + new helpers)
- Test: `src/server/src/scene/mod.rs` (one new unit test for `cell_visible`; existing tests guard the refactor)

**Interfaces:**
- Produces (visible to Task 4):
  - `fn cell_visible(floors: &[(f64, f64, Option<String>)], cl_level: f64, dist_cells: f64) -> bool`
  - `struct LightingInputs { all_bright: bool, lights: Vec<lighting::Light>, lit_polys: Vec<Vec<vision::P>>, sight_walls: Vec<vision::Seg> }`
  - `fn lighting_inputs(&self, scene: Uuid, settings: &ResolvedScene) -> LightingInputs` (method)
  - `fn source_los_poly(vp: vision::P, sight_walls: &[vision::Seg], los_restriction: bool) -> Vec<vision::P>`

- [ ] **Step 1: Write the failing test for the extracted predicate**

```rust
#[test]
fn cell_visible_predicate_honors_floor_and_range() {
    // floors: (floor_min_value, range_cells, render_hint). A normal mode (floor "dim" ~0.34),
    // range 0 = unbounded. Lit level 1.0 ≥ 0.34 → visible; 0.1 < 0.34 → not.
    let normal = vec![(0.34_f64, 0.0_f64, None)];
    assert!(cell_visible(&normal, 1.0, 5.0));
    assert!(!cell_visible(&normal, 0.1, 5.0));
    // Darkvision floor 0.0 within range 6 admits an unlit cell; beyond range it does not.
    let dark = vec![(0.0_f64, 6.0_f64, Some("desaturate".into()))];
    assert!(cell_visible(&dark, 0.0, 3.0), "unlit but within darkvision range");
    assert!(!cell_visible(&dark, 0.0, 9.0), "beyond darkvision range, unlit → not visible");
    // No in-range mode → not visible (fail closed).
    assert!(!cell_visible(&[], 1.0, 1.0));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat cell_visible_predicate`
Expected: FAIL — `cannot find function cell_visible`.

- [ ] **Step 3: Extract the seams and delegate**

Add the free functions (near `player_lit_mask`):

```rust
/// Per-cell visibility decision shared by `player_lit_mask` (egress/secrecy gate) and
/// `visible_cells` (movement gate). INVARIANT: identical for both so the move gate never
/// forbids a shipped-visible cell nor permits an unshipped one (spec §13). A cell is visible iff
/// some in-range vision mode's illumination floor is met. `floors`: `(floor_min, range_cells,
/// hint)`; `range == 0.0` ⇒ unbounded. Returns false when no mode is in range (fail closed).
fn cell_visible(floors: &[(f64, f64, Option<String>)], cl_level: f64, dist_cells: f64) -> bool {
    let mut min_floor = f64::INFINITY;
    for (fmin, range, _hint) in floors {
        if *range == 0.0 || dist_cells <= *range {
            min_floor = min_floor.min(*fmin);
        }
    }
    min_floor.is_finite() && cl_level >= min_floor
}

/// The LOS polygon for one vision source: the raycast visibility polygon when `los_restriction`
/// is on, else the whole bound box as a rectangle (whole-scene visible). Source: M9 raycast
/// (`vision::visibility_polygon`).
fn source_los_poly(
    vp: vision::P,
    sight_walls: &[vision::Seg],
    los_restriction: bool,
) -> Vec<vision::P> {
    let b = vision::bound_for(vp, sight_walls, VISION_BOUND_MARGIN);
    if los_restriction {
        vision::visibility_polygon(vp, sight_walls, b)
    } else {
        vec![
            (b.minx, b.miny),
            (b.maxx, b.miny),
            (b.maxx, b.maxy),
            (b.minx, b.maxy),
        ]
    }
}
```

Add the struct + method (the method goes in `impl SceneEcs`):

```rust
/// Scene-shared lighting/wall inputs for the visibility mask. Computed once per scene per
/// dispatch and reused for every source. `all_bright` short-circuits light raycasts under
/// lighting-off or globalIllumination (spec §3/§6).
pub(crate) struct LightingInputs {
    all_bright: bool,
    lights: Vec<lighting::Light>,
    lit_polys: Vec<Vec<vision::P>>,
    sight_walls: Vec<vision::Seg>,
}

pub(crate) fn lighting_inputs(&self, scene: Uuid, settings: &ResolvedScene) -> LightingInputs {
    let all_bright = !settings.lighting_enabled
        || matches!(settings.light_mode, LightMode::GlobalIllumination);
    let lights = if all_bright { Vec::new() } else { self.scene_lights(scene) };
    let light_walls = if all_bright { Vec::new() } else { self.light_walls(scene) };
    let lit_polys: Vec<Vec<vision::P>> = lights
        .iter()
        .map(|l| {
            let b = vision::bound_for(l.pos, &light_walls, VISION_BOUND_MARGIN);
            vision::visibility_polygon(l.pos, &light_walls, b)
        })
        .collect();
    LightingInputs {
        all_bright,
        lights,
        lit_polys,
        sight_walls: self.sight_walls(scene),
    }
}
```

> Use the correct module paths for `lighting::Light` / `vision::P` / `vision::Seg` as they appear in `mod.rs` (e.g. `crate::scene::lighting::Light`). Match the existing `use`/path style in the file.

Now rewrite the relevant parts of `player_lit_mask` to delegate (behavior-preserving):
- Replace the inline `all_bright`/`lights`/`light_walls`/`lit_polys`/`sight_walls` block (mod.rs ~867-888) with `let li = self.lighting_inputs(scene, settings);` and use `li.all_bright`, `li.lights`, `li.lit_polys`, `li.sight_walls` below.
- Replace the inline LOS-polygon construction (mod.rs ~894-905) with `let poly = source_los_poly(src.vp, &li.sight_walls, settings.los_restriction);`.
- Replace the inline visibility decision. Today the loop computes `visible_floor`/`admit_floor`/`admit_hint` together. Keep the `admit_floor`/`admit_hint` (hint) logic exactly as-is, but replace the final visibility test `if visible_floor.is_finite() && cl.level >= visible_floor` with `if cell_visible(&src.floors, cl.level, dist_cells)`. (The `visible_floor` local may be dropped if it was used only for that test; keep `admit_floor`/`admit_hint`.)

> Do not change tint/band/hint output. The strict (center-only) sampling is unchanged — only the source of the helpers moves.

- [ ] **Step 4: Run the predicate test + the full existing scene suite**

Run: `cargo test -p shadowcat scene::`
Expected: PASS — the new `cell_visible_predicate_honors_floor_and_range` plus every pre-existing `player_lit_mask`/lighting/vision test (the refactor must not move any output).

- [ ] **Step 5: Run the broader server suite + lint**

Run: `cargo test -p shadowcat && cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: PASS (227 server tests baseline, +1).

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "refactor(m10e-4): extract cell_visible + lighting_inputs + source_los_poly seams from player_lit_mask"
```

---

### Task 4: `visible_cells(user, scene, lenient)` mask

The movement gate's mask: the cells visible to `user` in `scene`, sampled per leniency. Strict (center) must equal `player_lit_mask`'s cells for that scene; lenient additionally includes any cell whose vision polygon overlaps it (approximated by the 4 corners + center — a superset of strict, never widening beyond polygon overlap, fail-safe).

**Files:**
- Modify: `src/server/src/scene/mod.rs` (new method on `SceneEcs`)
- Test: `src/server/src/scene/mod.rs` (inline)

**Interfaces:**
- Consumes: `cell_visible`, `lighting_inputs`/`LightingInputs`, `source_los_poly` (Task 3); `resolve_scene`, `scene_grid_sizes`, `token_vision_floors`, `scene_lights`, `lighting::cell_illumination`, `vision::point_in_poly`.
- Produces: `pub fn visible_cells(&self, user: Uuid, scene: Uuid, lenient: bool) -> std::collections::BTreeSet<(i32, i32)>`

- [ ] **Step 1: Write the failing tests**

These reuse the in-test scene/world/token/light construction pattern from `room.rs:620-711`. Add helpers in the test module if needed (a single GM-owned light at the token cell makes a few cells lit).

```rust
#[test]
fn visible_cells_strict_equals_player_lit_mask_cells() {
    // §13 parity: under strict sampling, the gate mask == the egress mask for the scene.
    let (ecs, user, scene) = scene_with_lit_player_token(); // test helper: builds ws+scene+token+light
    let strict: std::collections::BTreeSet<(i32, i32)> = ecs.visible_cells(user, scene, false);
    let egress: std::collections::BTreeSet<(i32, i32)> = ecs
        .player_lit_mask(user)
        .into_iter()
        .filter(|s| s.scene == scene)
        .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
        .collect();
    assert_eq!(strict, egress, "strict gate mask must equal the egress secrecy mask");
    assert!(!strict.is_empty());
}

#[test]
fn visible_cells_lenient_is_a_superset_of_strict() {
    let (ecs, user, scene) = scene_with_lit_player_token();
    let strict = ecs.visible_cells(user, scene, false);
    let lenient = ecs.visible_cells(user, scene, true);
    assert!(strict.iter().all(|c| lenient.contains(c)), "lenient ⊇ strict");
    assert!(lenient.len() >= strict.len());
}

#[test]
fn visible_cells_empty_when_user_has_no_source_in_scene() {
    let (ecs, _user, scene) = scene_with_lit_player_token();
    let stranger = Uuid::from_u128(999);
    assert!(ecs.visible_cells(stranger, scene, true).is_empty(), "no sources → empty (fail closed)");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat visible_cells`
Expected: FAIL — `no method visible_cells`.

- [ ] **Step 3: Implement `visible_cells`**

```rust
/// The set of cells visible to `user` in `scene` for the movement gate. Reuses the exact
/// egress primitives (`lighting_inputs`, `source_los_poly`, `cell_visible`) so it agrees with
/// the secrecy mask (spec §13). `lenient` selects the rasterization rule (spec §8): strict
/// samples the cell CENTER only (≡ `player_lit_mask`); lenient also samples the four corners, so
/// a cell whose vision polygon merely overlaps it counts — a superset, never extending past
/// polygon overlap. Empty ⇒ no in-scene vision source (fail closed).
pub fn visible_cells(
    &self,
    user: Uuid,
    scene: Uuid,
    lenient: bool,
) -> std::collections::BTreeSet<(i32, i32)> {
    use std::collections::BTreeSet;
    let mut out: BTreeSet<(i32, i32)> = BTreeSet::new();
    let settings = self.resolve_scene(scene);
    let cell = self
        .scene_grid_sizes()
        .get(&scene)
        .copied()
        .unwrap_or(100.0);
    if cell <= 0.0 {
        return out;
    }

    // 1. Gather this user's vision sources in THIS scene (owner ∪ observer-tier when
    //    observerVision). Mirrors player_lit_mask's source gather, scene-filtered.
    struct Src {
        vp: vision::P,
        floors: Vec<(f64, f64, Option<String>)>,
    }
    let mut sources: Vec<Src> = Vec::new();
    for e in self.world.query::<&SceneEntity>().iter() {
        if e.doc.doc_type != "token" || e.doc.parent_id != Some(scene) {
            continue;
        }
        let owns = e.doc.owner == Some(user);
        let is_source = owns
            || (settings.observer_vision && {
                let role = e
                    .doc
                    .permissions
                    .users
                    .get(&user)
                    .copied()
                    .unwrap_or(e.doc.permissions.default);
                role <= crate::data::document::DocRole::Observer
            });
        if !is_source {
            continue;
        }
        if let (Some(x), Some(y)) = (sys_f64(&e.doc, "/x"), sys_f64(&e.doc, "/y")) {
            sources.push(Src {
                vp: (x, y),
                floors: self.token_vision_floors(&e.doc),
            });
        }
    }
    if sources.is_empty() {
        return out;
    }

    // 2. Scene-shared lighting inputs (once), then per-source per-cell test.
    let li = self.lighting_inputs(scene, &settings);
    for src in &sources {
        let poly = source_los_poly(src.vp, &li.sight_walls, settings.los_restriction);
        if poly.len() < 3 {
            continue;
        }
        let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &(x, y) in &poly {
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x);
            maxy = maxy.max(y);
        }
        // Lenient samples corners, so a cell just outside the center-bbox can still qualify:
        // expand the scan by one cell each side under leniency.
        let pad = if lenient { 1 } else { 0 };
        let i0 = (minx / cell).floor() as i32 - pad;
        let i1 = (maxx / cell).floor() as i32 + pad;
        let j0 = (miny / cell).floor() as i32 - pad;
        let j1 = (maxy / cell).floor() as i32 + pad;
        let w = i1 as i64 - i0 as i64 + 1;
        let h = j1 as i64 - j0 as i64 + 1;
        if w.saturating_mul(h) > crate::scene::explored::MAX_CELLS_PER_POLYGON {
            tracing::warn!("visible_cells scan exceeds cap; skipping source");
            continue;
        }
        for i in i0..=i1 {
            for j in j0..=j1 {
                if out.contains(&(i, j)) {
                    continue;
                }
                // Sample points: center (strict) + the four corners (lenient).
                let samples: &[(f64, f64)] = if lenient {
                    &[
                        ((i as f64 + 0.5) * cell, (j as f64 + 0.5) * cell),
                        (i as f64 * cell, j as f64 * cell),
                        ((i + 1) as f64 * cell, j as f64 * cell),
                        (i as f64 * cell, (j + 1) as f64 * cell),
                        ((i + 1) as f64 * cell, (j + 1) as f64 * cell),
                    ]
                } else {
                    &[((i as f64 + 0.5) * cell, (j as f64 + 0.5) * cell)]
                };
                for &(sx, sy) in samples {
                    if !vision::point_in_poly(&poly, (sx, sy)) {
                        continue;
                    }
                    let cl = if li.all_bright {
                        crate::scene::lighting::CellLight { level: 1.0, tint: 0 }
                    } else {
                        crate::scene::lighting::cell_illumination(
                            (sx, sy),
                            settings.env_intensity,
                            settings.env_color,
                            &li.lights,
                            &li.lit_polys,
                            cell,
                        )
                    };
                    let dist_cells =
                        (((sx - src.vp.0).powi(2) + (sy - src.vp.1).powi(2)).sqrt()) / cell;
                    if cell_visible(&src.floors, cl.level, dist_cells) {
                        out.insert((i, j));
                        break;
                    }
                }
            }
        }
    }
    out
}
```

> Match exact module paths in the file. If `lenient` arrays-of-slices cause a borrow lifetime nit, bind each arm to a local `Vec`/array first. The strict arm MUST sample only the center so the §13 parity test passes.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat visible_cells`
Expected: PASS (3 tests), including the §13 parity equality.

- [ ] **Step 5: Full suite + lint**

Run: `cargo test -p shadowcat && cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "feat(m10e-4): visible_cells gate mask (strict==egress; lenient corner-sampled superset)"
```

---

### Task 5: Movement-restriction gate in `Room::publish`

Wire the gate into the existing non-GM block. After `blocks_move`, resolve the scene's `movementRestriction`; `Unrestricted` skips; `Visible`/`Revealed` require the move's supercover cells ⊆ mask. `Revealed` unions the server-persisted explored set. GM exempt. Rejected with `DataError::Forbidden` before the write (no seq). Over-cap move ⇒ `Forbidden`.

**Files:**
- Modify: `src/server/src/ws/room.rs:177-191` (the non-GM block in `publish`)
- Test: `src/server/src/ws/room.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `scene.resolve_scene`, `scene.visible_cells`, `scene.scene_grid_sizes`, `scene.token_move`, `scene.blocks_move`, `movement::supercover_cells`, `repo.get_explored`, `ExploredSet`, `PermissionContext{user_id, world_role}`, `MovementRestriction`.
- Produces: no new public API (gate is internal to `publish`).

- [ ] **Step 1: Write the failing tests**

Build on the existing M9a test scaffold (`room.rs:507-618`). Add a helper that publishes world-settings (with a chosen `movementRestriction`), a scene, a player-owned token, and optionally a light — mirroring `get_or_create_hydrates_config_and_actors_from_db` (`room.rs:620-711`).

```rust
#[tokio::test]
async fn movement_restriction_visible_blocks_move_into_darkness() {
    // Scene: lightingEnabled, environmentLight, env intensity 0 → only the light's radius is lit.
    // movementRestriction "visible". A player move that ends in an unlit (unseen) cell is rejected
    // before the write; a move staying within the lit radius is allowed; GM is exempt.
    let h = movement_scene("visible", /*with_light=*/ true).await;
    let seq0 = h.room.current_seq();

    // Move from the lit token cell out to a far, unlit cell → rejected, no seq consumed.
    let blocked = h.room.publish(&h.repo, &h.player, vec![h.mv_to(2000.0, 2000.0)], 0).await;
    assert!(matches!(blocked, Err(DataError::Forbidden)));
    assert_eq!(h.room.current_seq(), seq0, "blocked move consumes no seq");

    // A small move within the lit radius is allowed.
    h.room.publish(&h.repo, &h.player, vec![h.mv_to(60.0, 60.0)], 0).await.unwrap();
    assert_eq!(h.room.current_seq(), seq0 + 1);

    // GM moves into darkness freely (exempt).
    h.room.publish(&h.repo, &h.gm, vec![h.mv_to(2000.0, 2000.0)], 0).await.unwrap();
}

#[tokio::test]
async fn movement_restriction_unrestricted_allows_move_into_darkness() {
    let h = movement_scene("unrestricted", /*with_light=*/ false).await;
    // No light, no LOS coverage — but unrestricted means walls-only, so a non-crossing move passes.
    h.room.publish(&h.repo, &h.player, vec![h.mv_to(2000.0, 2000.0)], 0).await.unwrap();
}

#[tokio::test]
async fn movement_restriction_revealed_allows_move_into_explored_memory() {
    // movementRestriction "revealed": pre-seed the player's explored set with the destination cell.
    // The move into a currently-unlit but previously-explored cell is allowed; a never-seen cell is not.
    let h = movement_scene("revealed", /*with_light=*/ true).await;
    let cell = 100.0_f64;
    // Seed explored with the destination cell (e.g. (5,5)) via the repo (mirrors conn.rs accumulation).
    let mut seed = crate::scene::explored::ExploredSet::new();
    seed.mark_polygons(
        &[vec![5.0 * cell, 5.0 * cell, 6.0 * cell, 5.0 * cell, 6.0 * cell, 6.0 * cell, 5.0 * cell, 6.0 * cell]],
        cell,
    );
    h.repo.set_explored(h.world_id, h.scene_id, h.player.user_id, &seed.to_bytes()).await.unwrap();

    // Move ending at the center of explored cell (5,5) → allowed.
    h.room.publish(&h.repo, &h.player, vec![h.mv_to(550.0, 550.0)], 0).await.unwrap();

    // A move ending in a never-seen, unlit cell (far away) is still rejected.
    let blocked = h.room.publish(&h.repo, &h.player, vec![h.mv_to(9000.0, 9000.0)], 0).await;
    assert!(matches!(blocked, Err(DataError::Forbidden)));
}

#[tokio::test]
async fn movement_restriction_checks_entire_move_not_just_endpoint() {
    // A move whose endpoint is visible but whose path crosses an unseen intermediate cell is
    // rejected (supercover, not endpoint-only). Construct two lit pockets with a dark gap between.
    let h = movement_scene_two_lit_pockets().await; // helper: lights at the start and end, gap dark
    let blocked = h.room.publish(&h.repo, &h.player, vec![h.mv_start_to_far_pocket()], 0).await;
    assert!(matches!(blocked, Err(DataError::Forbidden)), "dark gap on the path blocks the move");
}

#[tokio::test]
async fn movement_restriction_lenient_allows_partial_cell() {
    // Default partialCellLeniency=true: a move ending in a cell only partially covered by the
    // vision/light polygon is allowed (would be rejected under strict). Pair-test with a
    // leniency-off world-settings variant proving the same move is then rejected.
    let lenient = movement_scene_partial_cell(/*lenient=*/ true).await;
    lenient.room.publish(&lenient.repo, &lenient.player, vec![lenient.mv_to_partial_cell()], 0).await.unwrap();

    let strict = movement_scene_partial_cell(/*lenient=*/ false).await;
    let blocked = strict.room.publish(&strict.repo, &strict.player, vec![strict.mv_to_partial_cell()], 0).await;
    assert!(matches!(blocked, Err(DataError::Forbidden)));
}
```

> The helpers (`movement_scene`, `mv_to`, etc.) are test scaffolding — implement them in the test module by composing the existing `repo_with_world` + `wdoc` + `publish(Create …)` pattern (`room.rs:620-711`). Keep the lit/dark geometry simple: a single white light `intensity 1.0, brightRadius/dimRadius` a few cells wide at the token, cell size 100. `mv_to(x,y)` builds the `Operation::Update` with `/system/x`,`/system/y` changes (see `mv` at `room.rs:541`). If the two-pocket / partial-cell geometry proves fiddly, assert the same semantics by seeding `set_explored` and/or toggling `lightMode:"globalIllumination"` (all-bright) vs `environmentLight` to control which cells are in the mask — the gate logic is what's under test, not the raycaster.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat movement_restriction`
Expected: FAIL — the gate does not yet reject (moves succeed where a `Forbidden` is asserted).

- [ ] **Step 3: Implement the gate**

In `Room::publish`, the existing non-GM block holds `let scene = self.scene.read().await;` and loops ops calling `token_move`/`blocks_move`. Extend it. Collect any `Revealed`-mode pending checks (which need an async `get_explored`) and resolve them after dropping the read guard, so no lock is held across `await`.

```rust
if ctx.world_role != crate::data::document::WorldRole::Gm {
    // Pending revealed-mode checks deferred past the ECS read borrow: (scene, move_cells, visible).
    let mut revealed_pending: Vec<(Uuid, std::collections::BTreeSet<(i32, i32)>, std::collections::BTreeSet<(i32, i32)>)> = Vec::new();
    {
        let scene = self.scene.read().await;
        // Memoize the visible mask per scene within this publish (a batch may move several tokens).
        let mut visible_cache: std::collections::HashMap<(Uuid, bool), std::collections::BTreeSet<(i32, i32)>> = std::collections::HashMap::new();
        for op in &ops {
            if let Operation::Update { doc_id, changes } = op {
                if let Some((scene_id, a0, a1)) = scene.token_move(*doc_id, changes) {
                    // M9a wall gate (unchanged).
                    if scene.blocks_move(scene_id, a0, a1) {
                        return Err(DataError::Forbidden);
                    }
                    // M10e-4 movement restriction.
                    let settings = scene.resolve_scene(scene_id);
                    if matches!(settings.movement_restriction, crate::scene::MovementRestriction::Unrestricted) {
                        continue;
                    }
                    let cell = scene.scene_grid_sizes().get(&scene_id).copied().unwrap_or(100.0);
                    // Supercover of the move; over-cap or degenerate grid ⇒ fail closed.
                    let Some(move_cells) = crate::scene::movement::supercover_cells(a0, a1, cell) else {
                        return Err(DataError::Forbidden);
                    };
                    let lenient = settings.partial_cell_leniency;
                    let visible = visible_cache
                        .entry((scene_id, lenient))
                        .or_insert_with(|| scene.visible_cells(ctx.user_id, scene_id, lenient))
                        .clone();
                    match settings.movement_restriction {
                        crate::scene::MovementRestriction::Visible => {
                            if !move_cells.iter().all(|c| visible.contains(c)) {
                                return Err(DataError::Forbidden);
                            }
                        }
                        crate::scene::MovementRestriction::Revealed => {
                            // explored ∪ visible — explored is async; defer past the read guard.
                            revealed_pending.push((scene_id, move_cells, visible));
                        }
                        crate::scene::MovementRestriction::Unrestricted => {}
                    }
                }
            }
        }
    } // scene read guard dropped here

    for (scene_id, move_cells, visible) in revealed_pending {
        let explored = match repo.get_explored(scene_id, ctx.user_id).await {
            Ok(Some(blob)) => crate::scene::explored::ExploredSet::from_bytes(&blob),
            _ => crate::scene::explored::ExploredSet::new(), // fail closed: visible-only
        };
        if !move_cells.iter().all(|c| visible.contains(c) || explored.contains(*c)) {
            return Err(DataError::Forbidden);
        }
    }
}
```

> Ensure `MovementRestriction` and `movement::supercover_cells` are reachable (`pub` / `pub(crate)` paths). Confirm `Operation` import is already in scope (the existing block uses it). Keep the M9a wall check exactly where it is (first, so a wall crossing short-circuits before mask work).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat movement_restriction`
Expected: PASS (5 tests).

- [ ] **Step 5: Verify M9a + full suite still green + lint**

Run: `cargo test -p shadowcat && cargo fmt --check && cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: PASS — including the pre-existing `token_move_uses_post_image_resisting_forged_bypasses` (M9a) test, untouched.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/ws/room.rs
git commit -m "feat(m10e-4): movement-restriction gate at Room::publish (visible/revealed/unrestricted, supercover ⊆ mask, GM exempt)"
```

---

### Task 6: Closeout — docs + skill sync (non-TDD)

The cadence's documentation-sync + reviewed-skill-update gates. Do this after the whole-branch buddy-check (so it records verified reality), but the content is specified here.

**Files:**
- Modify: `docs/PLAN.md` (mark M10e-4 done, point to this plan), `docs/TODO.md`, `docs/POST_WORK_FINDINGS.md`, `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md`

- [ ] **Step 1: `docs/POST_WORK_FINDINGS.md`** — log the complication found during planning (do NOT treat as a to-do):

```markdown
- Title: Dark-default × movementRestriction "visible" freezes players.
  Summary: Default world-settings = lightingEnabled true + environmentLight + environment.intensity
  0.0 (inert) and movementRestriction "visible". Composed, a fresh scene with no placed lights has
  an empty lit mask, so EVERY non-GM move is rejected until the GM adds a light, switches to
  globalIllumination, or sets unrestricted. This is the spec's fail-closed intent (a player can't
  drag into unseen map), but it is a sharp out-of-box edge. Status: Needs Review (whether to soften
  the default env intensity or the default restriction is a separate decision).
```

- [ ] **Step 2: `docs/TODO.md`** — log the deferred caching optimization (spec §8 "settle caching in the plan" — chosen: recompute on demand):

```markdown
- Server / scene-vision: the movement-restriction gate recomputes `visible_cells` on demand per
  move (human-paced; acceptable per spec §8). If profiling shows this hot, cache the last
  egress-computed mask per `(user, scene)` and reuse it in the gate. Inert until measured.
```

- [ ] **Step 3: `docs/PLAN.md`** — move M10e-4 to completed; note `{e-3,e-4}` both done, next `e-6` (e-5 anytime).

- [ ] **Step 4: Update `shadowcat-codebase-scene-rendering` skill** — add the movement-restriction seam: `Room::publish` gate now does wall (M9a) + restriction (M10e-4); the gate mask = `SceneEcs::visible_cells` (strict==`player_lit_mask`, lenient corner-sampled); `revealed` unions `get_explored`; `supercover_cells` in `scene/movement.rs`; `MovementRestriction` resolved in `resolve_scene`. Note the invariant: gate and egress share `cell_visible`/`lighting_inputs` (§13).

- [ ] **Step 5: Reviewed skill-update gate** — dispatch `shadowcat-spec-reviewer` on the skill diff to confirm it accurately captures the change (no omission/drift/broken pointer). Record PASS.

- [ ] **Step 6: Commit**

```bash
git add docs/ .claude/skills/shadowcat-codebase-scene-rendering/
git commit -m "docs(m10e-4): PLAN/TODO/POST_WORK + scene-rendering skill sync for movement restriction"
```

---

## Self-Review (completed during authoring)

**Spec §8 coverage:**
- `unrestricted`/`visible`/`revealed` → Task 5 gate (three arms). ✓
- Supercover entire-move check → Task 2 + Task 5 (`move_cells.iter().all(...)`); `checks_entire_move` test. ✓
- Partial-cell leniency selects rasterization rule → Task 1 (resolve) + Task 4 (`lenient` corner sampling) + Task 5 (passes `settings.partial_cell_leniency`); `lenient_allows_partial_cell` test. ✓
- Reject `DataError::Forbidden` before the write, no seq, GM exempt → Task 5; `blocks_move_into_darkness` (no-seq + GM) test. ✓
- Reuses the V2 mask for `(U, scene)`; gate and egress same mask (§13) → Task 3/4 shared seams + strict==egress parity test. ✓
- `revealed` = explored (`get_explored`) ∪ visible → Task 5; `revealed_allows_explored_memory` test. ✓
- Caching settled in plan (recompute on demand) → Task 6 TODO. ✓
- Security §13: authoritative start = committed ECS position (M9a `token_move` post-image) — unchanged; fail-closed on degenerate/over-cap → Task 2/5. ✓

**Type consistency:** `MovementRestriction`/`ResolvedScene` fields (Task 1) ↔ `resolve_scene`/`visible_cells`/gate (Tasks 4,5); `cell_visible`/`lighting_inputs`/`source_los_poly` (Task 3) ↔ `visible_cells` (Task 4); `supercover_cells -> Option<BTreeSet>` (Task 2) ↔ gate `let Some(... ) else` (Task 5); `get_explored -> Option<Vec<u8>>` ↔ `ExploredSet::from_bytes`. Consistent.

**No client/protocol change:** confirmed — `DataError::Forbidden` rollback is the existing M9a optimistic-move UX; movement animation (e-5) and path preview (e-6) are separate checkpoints.

## Buddy-check directives

This checkpoint is a **security-sensitive server gate** (the secrecy mask now also gates movement; a drift between gate and egress, or a leniency that widens past polygon overlap, is a confidentiality/UX bug). Per the M8/M9/M10 cadence and the elevated risk: after all tasks pass, run a **whole-branch two-reviewer buddy-check on Opus** (`shadowcat-spec-reviewer` + `shadowcat-code-reviewer`), reconciled to convergence. Focus the reviewers on: (1) §13 parity — `visible_cells` strict ≡ `player_lit_mask` cells (no fork); (2) the supercover entire-move guarantee (no thin-line slip); (3) fail-closed on empty mask / over-cap / missing explored; (4) the lenient sampler never includes a cell with zero polygon overlap; (5) GM-exempt + no-seq-on-reject preserved. Record the outcome (and any Critical/Important fixes) before merge.
```
