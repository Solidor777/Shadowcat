# M10f-4 — Regions on the Continuous (navmesh) Engine — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the three M10g region behaviors (`terrain` cost, `impassable`, `arrest`) into the **continuous** movement model's server-side router + preview, reusing the M10g per-requester cell region field as the single weighting authority — polyanya is never asked to weight.

**Architecture:** In `SceneEcs::pathfind`'s `Continuous` branch, compute the per-requester region field (already done for the grid branch). If it carries terrain or impassable, route via the existing weighted grid A* (`pathfinding::find`) with the diagonal rule **forced to `Euclidean`**, then **LOS-smooth** the cell-center polyline back to any-angle geometry. Otherwise take the unchanged pure-polyanya path, plus an arrest post-filter. Execution (`move_exec`) is already engine-agnostic (M10f-2/3) and needs no production change — only verification tests. Secrecy is inherited unchanged (per-requester field for the router; authoritative field springs secret regions at execution).

**Tech Stack:** Rust (server, `src/server`). Pure geometry helpers in `scene/navmesh.rs` + `scene/pathfinding.rs` + `scene/regions.rs`; dispatch in `scene/mod.rs`. No new crate. `cargo test` / `cargo fmt` / `cargo clippy` from `src/server/`.

## Global Constraints

- **No new crate; cargo-bloat budget untouched** (60 MiB / `wc -c` release binary `< 62914560`).
- **Cross-platform:** `#[cfg]`-free pure geometry only; `std::path` for any path work (none here); three-OS CI matrix is the proof.
- **The M10g cell region field is the ONLY weighting authority for both engines.** Never delegate terrain weighting to polyanya (source-verified infeasible in `polyanya-0.16.1`; design spec §2).
- **`route ⊆ gate-allowed` is a secrecy invariant.** Any route the router returns to a non-GM requester must stay inside that requester's visibility mask. Never fork the per-cell visibility decision across engines (parent spec §6.3/§13).
- **Fail closed.** Degenerate inputs (non-finite/≤0 `cell`, out-of-range footprint, over-cap span) produce the most restrictive output (un-smoothed/un-routed), never an unguarded all-pass.
- **The `GridStepped` branch of `pathfind` stays byte-for-byte unchanged** (guarded by the existing test `pathfind_grid_stepped_scene_is_byte_for_byte_unchanged`). All changes are confined to the `Continuous` branch + new pure helpers.
- **Continuous-engine `cost` is in scene units** (the polyanya path returns Euclidean scene-unit length; the existing test asserts ~900 for a 900px route). The weighted-grid path returns cost in **cells**, so it MUST be multiplied by `cell` to report scene units on the continuous engine.

## Model/Effort directives

Per explicit user direction (2026-07-03): **write and execute this plan mainline on the current session model (Opus 4.8, effort high) — no model switch, no `sdd-plan-writer` dispatch.** This overrides the default "recommend dispatching a plan-writer subagent." Execution tier for implementer subagents (if Subagent-Driven is chosen at handoff) follows the project default (`shadowcat-coder` at `effort: medium`, escalating to `-opus` on BLOCKED); the two-reviewer gate uses `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` at `effort: high`.

## Buddy-check directives

This checkpoint touches the secrecy-critical continuous route-preview path (the same invariant class every M9/M10e/M2/M10g/M10f-1 checkpoint buddy-checked). Pre-authorized buddy-checks (per `buddy-checking` skill, Offered mode → run):

- **Task 2 (`los_smooth`)** — the one genuinely new algorithm; its cost-guard is the load-bearing correctness/secrecy boundary (a straightened chord that entered an out-of-mask, impassable, arrest, or higher-cost cell would break `route ⊆ gate-allowed` or the weighting). Buddy-check.
- **Task 4 (dispatch + secrecy + cost-unit)** — the integration point where the per-requester field, the mask, and the cost-unit conversion all meet. Buddy-check (focus: secret-region absence from a player's route; `route ⊆ gate-allowed`; cost-unit consistency).
- **Whole-branch buddy-check** before merge (opus, two-reviewer), per the M10f cadence.

Tasks 1, 3, 5, 6 use the standard two-reviewer gate.

---

## File Structure

- `src/server/src/scene/regions.rs` — **modify**: add `RegionField::has_terrain_or_impassable()` (dispatch predicate). Pure.
- `src/server/src/scene/navmesh.rs` — **modify**: add `los_smooth(...)` (weighted-route LOS smoothing) and `truncate_at_arrest(...)` (arrest post-filter for the polyanya path). Pure; siblings of the existing `clip_to_visible_mask`.
- `src/server/src/scene/mod.rs` — **modify**: rewrite the `Continuous` branch of `pathfind` (lines ~1079-1109) to dispatch on `has_terrain_or_impassable`, thread the region field, convert cell→scene-unit cost, apply smoothing/arrest. Add integration tests.
- `src/server/src/scene/move_exec.rs` — **modify (tests only)**: add a verification test that the engine-agnostic executor handles a weighted+smoothed any-angle continuous polyline (zero production change).
- `docs/PLAN.md`, `docs/superpowers/specs/2026-07-02-m10f-continuous-navmesh-movement-design.md`, `docs/superpowers/specs/2026-06-24-m10-tokens-design.md`, `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md` — **modify (docs)**: record deviations + reviewed skill update.

---

## Task 1: `RegionField::has_terrain_or_impassable()` dispatch predicate

**Files:**
- Modify: `src/server/src/scene/regions.rs` (add method to `impl RegionField`, ~line 203)
- Test: `src/server/src/scene/regions.rs` (`#[cfg(test)] mod tests`, existing)

**Interfaces:**
- Consumes: existing `RegionField { cells: BTreeMap<Cell, RegionEffect> }`, `RegionEffect::{Impassable, Arrest, Terrain(f64)}`.
- Produces: `pub(crate) fn has_terrain_or_impassable(&self) -> bool` — true iff any cell is `Impassable` or `Terrain(m)` with `m > 1.0`. Used by Task 4 to choose the weighted vs pure-polyanya continuous path.

- [ ] **Step 1: Write the failing test**

Add inside `regions.rs` `mod tests`:

```rust
#[test]
fn has_terrain_or_impassable_detects_weight_but_not_arrest() {
    let cell = 100.0;
    let rect = RegionShape::Rect { x0: 0.0, y0: 0.0, x1: 100.0, y1: 100.0 };

    let mut b = RegionField::builder();
    b.add(&rect, RegionBehavior::Terrain, 2.0, cell);
    assert!(b.build().has_terrain_or_impassable(), "terrain mult>1 counts");

    let mut b = RegionField::builder();
    b.add(&rect, RegionBehavior::Impassable, 1.0, cell);
    assert!(b.build().has_terrain_or_impassable(), "impassable counts");

    let mut b = RegionField::builder();
    b.add(&rect, RegionBehavior::Arrest, 1.0, cell);
    assert!(!b.build().has_terrain_or_impassable(), "arrest alone does not count");

    let mut b = RegionField::builder();
    b.add(&rect, RegionBehavior::Terrain, 1.0, cell);
    assert!(!b.build().has_terrain_or_impassable(), "terrain mult==1 is a no-op, does not count");

    assert!(!RegionField::builder().build().has_terrain_or_impassable(), "empty field");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src/server && cargo test -p shadowcat-server has_terrain_or_impassable_detects_weight_but_not_arrest`
Expected: FAIL — `no method named has_terrain_or_impassable`.

- [ ] **Step 3: Write minimal implementation**

Add to `impl RegionField` (after `terrain_multiplier`, ~line 203):

```rust
/// True iff any cell is `Impassable` or weighted `Terrain` (multiplier > 1.0). The
/// dispatch predicate the continuous router uses to decide between the weighted grid
/// route (terrain/impassable present) and the pure any-angle polyanya route (neither).
/// Arrest is excluded: it neither bends the route nor requires route-around, so an
/// arrest-only scene stays on the polyanya path with an arrest post-filter.
pub(crate) fn has_terrain_or_impassable(&self) -> bool {
    self.cells.values().any(|e| match e {
        RegionEffect::Impassable => true,
        RegionEffect::Terrain(m) => *m > 1.0,
        RegionEffect::Arrest => false,
    })
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src/server && cargo test -p shadowcat-server has_terrain_or_impassable_detects_weight_but_not_arrest`
Expected: PASS.

- [ ] **Step 5: fmt + clippy + commit**

```bash
cd src/server && cargo fmt && cargo clippy -- -D warnings
git add src/server/src/scene/regions.rs
git commit -m "feat(m10f-4): RegionField::has_terrain_or_impassable dispatch predicate"
```

---

## Task 2: `los_smooth` — weighted-route LOS smoothing

**Files:**
- Modify: `src/server/src/scene/navmesh.rs` (add `los_smooth`, after `clip_to_visible_mask` ~line 400)
- Test: `src/server/src/scene/navmesh.rs` (`#[cfg(test)] mod tests`, existing)

**Interfaces:**
- Consumes: `pathfinding::PathOutcome { path: Vec<(f64,f64)>, cost, arrested }`; `RegionField::{is_impassable,is_arrest,terrain_multiplier}`; `pathfinding::{footprint_cells, Cell, MAX_FOOTPRINT_CELLS}`; `movement::supercover_cells`; `move_stream::sample_path`; `crate::scene::segments_cross`; `vision::Seg`.
- Produces: `pub(crate) fn los_smooth(outcome: pathfinding::PathOutcome, walls: &[vision::Seg], mask: Option<&BTreeSet<pathfinding::Cell>>, field: &regions::RegionField, cell: f64, footprint_radius_cells: f64) -> pathfinding::PathOutcome` — the input's `cost` and `arrested` are carried through unchanged; only `path` geometry is straightened. Used by Task 4.

**Invariant (load-bearing):** a span is straightened only when every cell its chord enters is in `mask` (if `Some`), crossed by no wall, and is NOT impassable / arrest / weighted-terrain. The single grid step `path[i]→path[i+1]` is always kept unconditionally (it already passed `find`'s gate), guaranteeing progress and keeping cells adjacent to special terrain grid-stepped. This makes the smoothed route's gate/secrecy/cost properties `≤` the grid route's.

- [ ] **Step 1: Write the failing tests**

Add inside `navmesh.rs` `mod tests` (note `use super::*;` is already present; add `use crate::scene::vision::Seg;` and `use crate::scene::regions::{RegionField, RegionShape, RegionBehavior};` and `use crate::scene::pathfinding::PathOutcome;` at the top of the test module if not present):

```rust
fn oc(path: Vec<(f64, f64)>) -> PathOutcome {
    PathOutcome { path, cost: 7.0, arrested: false }
}
fn empty_field() -> RegionField {
    RegionField::builder().build()
}
fn terrain_on(x0: f64, y0: f64, x1: f64, y1: f64, mult: f64) -> RegionField {
    let mut b = RegionField::builder();
    b.add(&RegionShape::Rect { x0, y0, x1, y1 }, RegionBehavior::Terrain, mult, 100.0);
    b.build()
}

// Base L-route at cell=100: right two cells then up one. Cells (0,0),(1,0),(2,0),(2,1).
// The straight shortcut (50,50)->(250,150) enters cell (1,1); the horizontal first leg
// (50,50)->(250,50) enters only row-0 cells.
const L_ROUTE: [(f64, f64); 3] = [(50.0, 50.0), (250.0, 50.0), (250.0, 150.0)];

#[test]
fn los_smooth_straightens_an_open_l_route() {
    let out = los_smooth(oc(L_ROUTE.to_vec()), &[], None, &empty_field(), 100.0, 0.1);
    assert_eq!(out.path.first().copied(), Some((50.0, 50.0)));
    assert_eq!(out.path.last().copied(), Some((250.0, 150.0)));
    assert_eq!(out.path.len(), 2, "open route straightens to a single chord");
    assert_eq!(out.cost, 7.0, "cost carried through unchanged");
    assert!(!out.arrested);
}

#[test]
fn los_smooth_refuses_shortcut_through_terrain() {
    // Terrain (mult 2) on cell (1,1) = Rect [100,100]-[200,200]; the shortcut enters it.
    let field = terrain_on(100.0, 100.0, 200.0, 200.0, 2.0);
    let out = los_smooth(oc(L_ROUTE.to_vec()), &[], None, &field, 100.0, 0.1);
    assert_eq!(out.path, L_ROUTE.to_vec(), "terrain on the shortcut blocks straightening");
}

#[test]
fn los_smooth_refuses_shortcut_through_impassable() {
    let mut b = RegionField::builder();
    b.add(&RegionShape::Rect { x0: 100.0, y0: 100.0, x1: 200.0, y1: 200.0 }, RegionBehavior::Impassable, 1.0, 100.0);
    let out = los_smooth(oc(L_ROUTE.to_vec()), &[], None, &b.build(), 100.0, 0.1);
    assert_eq!(out.path, L_ROUTE.to_vec(), "impassable on the shortcut blocks straightening");
}

#[test]
fn los_smooth_refuses_shortcut_across_a_wall() {
    // Vertical wall x=150, y in [80,200]. Shortcut (50,50)->(250,150) crosses it at (150,100);
    // the horizontal first leg at y=50 passes below the wall (y=50 < 80), so the middle vertex
    // is retained.
    let wall = Seg { a: (150.0, 80.0), b: (150.0, 200.0) };
    let out = los_smooth(oc(L_ROUTE.to_vec()), &[wall], None, &empty_field(), 100.0, 0.1);
    assert_eq!(out.path, L_ROUTE.to_vec(), "wall on the shortcut blocks straightening");
}

#[test]
fn los_smooth_refuses_shortcut_leaving_the_mask() {
    // Mask = every cell the L-route touches, MINUS (1,1) which only the shortcut enters.
    let mut mask: std::collections::BTreeSet<crate::scene::pathfinding::Cell> = std::collections::BTreeSet::new();
    for c in [(0, 0), (1, 0), (2, 0), (2, 1), (0, 1)] {
        mask.insert(c);
    }
    let out = los_smooth(oc(L_ROUTE.to_vec()), &[], Some(&mask), &empty_field(), 100.0, 0.1);
    assert_eq!(out.path, L_ROUTE.to_vec(), "a shortcut cell outside the mask blocks straightening");
}

#[test]
fn los_smooth_two_point_route_is_unchanged() {
    let out = los_smooth(oc(vec![(50.0, 50.0), (250.0, 50.0)]), &[], None, &empty_field(), 100.0, 0.1);
    assert_eq!(out.path.len(), 2, "nothing to straighten with < 3 vertices");
}

#[test]
fn los_smooth_degenerate_cell_fails_closed_to_input() {
    let out = los_smooth(oc(L_ROUTE.to_vec()), &[], None, &empty_field(), 0.0, 0.1);
    assert_eq!(out.path, L_ROUTE.to_vec(), "degenerate cell returns the grid route unchanged");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src/server && cargo test -p shadowcat-server los_smooth`
Expected: FAIL — `cannot find function los_smooth`.

- [ ] **Step 3: Write minimal implementation**

Add to `navmesh.rs` after `clip_to_visible_mask` (before `#[cfg(test)] mod tests`):

```rust
/// Line-of-sight smoothing (string-pull) for a WEIGHTED continuous route (M10f-4). Input is the
/// cell-center polyline `pathfinding::find` produced over the region field; output restores
/// any-angle geometry by straightening spans that pass ONLY through plain, visible, unobstructed
/// cells. A span `path[i]..path[j]` (j >= i+2) is straightened only when every cell its chord
/// enters is (a) in `mask` when `Some`, (b) crossed by no `blocksMove` wall, (c) not impassable,
/// (d) not arrest, (e) not weighted terrain (`terrain_multiplier <= 1.0`). Conditions (c)-(e) keep
/// smoothing away from any "special" cell, so a straightened chord can never shortcut INTO
/// terrain/impassable/arrest the weighted search routed around or truncated at — the smoothed
/// route's gate/secrecy/cost properties are therefore <= the grid route's. The single grid step
/// `path[i] -> path[i+1]` is ALWAYS kept unconditionally (it already passed `find`'s per-cell
/// gate), so progress to the goal is guaranteed and cells adjacent to special terrain stay
/// grid-stepped. `cost` and `arrested` are carried through unchanged (the grid weighted cost is a
/// valid, slightly-conservative budget for the straighter geometry — same preview-vs-execution
/// divergence class already logged for the grid engine in `docs/TODO.md`). "Entered cells" = the
/// destination footprint disc ∪ the step supercover, the SAME union `pathfinding::cell_enterable`
/// and `clip_to_visible_mask` apply. Fail-closed: `< 3` vertices, or a degenerate
/// `cell`/`footprint_radius_cells`, or an over-cap `supercover_cells`, returns the input unchanged.
pub(crate) fn los_smooth(
    outcome: crate::scene::pathfinding::PathOutcome,
    walls: &[crate::scene::vision::Seg],
    mask: Option<&std::collections::BTreeSet<crate::scene::pathfinding::Cell>>,
    field: &crate::scene::regions::RegionField,
    cell: f64,
    footprint_radius_cells: f64,
) -> crate::scene::pathfinding::PathOutcome {
    use crate::scene::pathfinding::{footprint_cells, Cell};
    if outcome.path.len() < 3
        || !cell.is_finite()
        || cell <= 0.0
        || !(0.0..=crate::scene::pathfinding::MAX_FOOTPRINT_CELLS).contains(&footprint_radius_cells)
    {
        return outcome;
    }
    let r_scene = footprint_radius_cells.max(0.0) * cell;
    let path = outcome.path.clone();

    // True iff the straight chord a->b passes only through plain, visible, unobstructed cells.
    let chord_ok = |a: (f64, f64), b: (f64, f64)| -> bool {
        let samples = crate::scene::move_stream::sample_path(&[a, b], cell, 1.0);
        let mut prev = samples[0].pos;
        for s in samples.iter().skip(1) {
            let to = (
                (s.pos.0 / cell).floor() as i32,
                (s.pos.1 / cell).floor() as i32,
            );
            let mut entered: Vec<Cell> = footprint_cells(to, s.pos, r_scene, cell);
            match crate::scene::movement::supercover_cells(prev, s.pos, cell) {
                Some(sc) => entered.extend(sc),
                None => return false, // degenerate/over-cap span: fail closed (do not straighten)
            }
            for c in &entered {
                if field.is_impassable(*c)
                    || field.is_arrest(*c)
                    || field.terrain_multiplier(*c) > 1.0
                {
                    return false;
                }
                if let Some(m) = mask {
                    if !m.contains(c) {
                        return false;
                    }
                }
            }
            // Wall crossing (public geometry, checked independent of mask). Skip a malformed
            // (non-finite-endpoint) wall so one NaN wall cannot fail-open the check — mirrors
            // `clip_to_visible_mask`.
            if walls
                .iter()
                .filter(|w| {
                    w.a.0.is_finite() && w.a.1.is_finite() && w.b.0.is_finite() && w.b.1.is_finite()
                })
                .any(|w| crate::scene::segments_cross(prev, s.pos, w.a, w.b))
            {
                return false;
            }
            prev = s.pos;
        }
        true
    };

    let n = path.len();
    let mut smoothed: Vec<(f64, f64)> = vec![path[0]];
    let mut i = 0usize;
    while i < n - 1 {
        // Keep the single grid step unconditionally; greedily extend as far as a chord stays clear.
        let mut best = i + 1;
        let mut j = i + 2;
        while j < n && chord_ok(path[i], path[j]) {
            best = j;
            j += 1;
        }
        smoothed.push(path[best]);
        i = best;
    }

    crate::scene::pathfinding::PathOutcome {
        path: smoothed,
        cost: outcome.cost,
        arrested: outcome.arrested,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src/server && cargo test -p shadowcat-server los_smooth`
Expected: PASS (all 7).

- [ ] **Step 5: fmt + clippy + commit**

```bash
cd src/server && cargo fmt && cargo clippy -- -D warnings
git add src/server/src/scene/navmesh.rs
git commit -m "feat(m10f-4): los_smooth — cost-guarded LOS smoothing for weighted continuous routes"
```

---

## Task 3: `truncate_at_arrest` — arrest post-filter for the polyanya path

**Files:**
- Modify: `src/server/src/scene/navmesh.rs` (add `truncate_at_arrest`, after `los_smooth`)
- Test: `src/server/src/scene/navmesh.rs` (`mod tests`)

**Interfaces:**
- Consumes: `pathfinding::PathOutcome`; `RegionField::is_arrest`; `move_stream::sample_path`.
- Produces: `pub(crate) fn truncate_at_arrest(outcome: pathfinding::PathOutcome, field: &regions::RegionField, cell: f64) -> pathfinding::PathOutcome` — if the route enters a visible arrest cell, truncate at that sample and set `arrested`; otherwise return the input unchanged. Used by Task 4 on the pure-polyanya path (the weighted path's `find` already truncates arrest internally).

- [ ] **Step 1: Write the failing tests**

Add to `navmesh.rs` `mod tests`:

```rust
fn arrest_on(x0: f64, y0: f64, x1: f64, y1: f64) -> RegionField {
    let mut b = RegionField::builder();
    b.add(&RegionShape::Rect { x0, y0, x1, y1 }, RegionBehavior::Arrest, 1.0, 100.0);
    b.build()
}

#[test]
fn truncate_at_arrest_cuts_at_first_visible_arrest_cell() {
    // Straight route (50,50)->(450,50): cells (0,0)..(4,0). Arrest on cell (2,0) = Rect
    // [200,0]-[300,100]. Route truncates on entry to (2,0); the surviving path stays within x<=300.
    let route = PathOutcome { path: vec![(50.0, 50.0), (450.0, 50.0)], cost: 400.0, arrested: false };
    let out = truncate_at_arrest(route, &arrest_on(200.0, 0.0, 300.0, 100.0), 100.0);
    assert!(out.arrested, "arrest flag set");
    assert!(out.path.last().unwrap().0 <= 300.0 + 1e-6, "truncated at/near the arrest cell entry");
    assert!(out.path.last().unwrap().0 >= 200.0, "reached the arrest cell");
}

#[test]
fn truncate_at_arrest_no_arrest_is_unchanged() {
    let route = PathOutcome { path: vec![(50.0, 50.0), (450.0, 50.0)], cost: 400.0, arrested: false };
    let out = truncate_at_arrest(route.clone(), &empty_field(), 100.0);
    assert_eq!(out.path, route.path, "no arrest region: route unchanged");
    assert!(!out.arrested);
}

#[test]
fn truncate_at_arrest_start_cell_is_not_a_trigger() {
    // Arrest on the START cell (0,0); the token is already standing there, so it is not "entering".
    let route = PathOutcome { path: vec![(50.0, 50.0), (450.0, 50.0)], cost: 400.0, arrested: false };
    let out = truncate_at_arrest(route, &arrest_on(0.0, 0.0, 100.0, 100.0), 100.0);
    assert!(out.path.last().unwrap().0 > 100.0, "start-cell arrest does not immediately truncate");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src/server && cargo test -p shadowcat-server truncate_at_arrest`
Expected: FAIL — `cannot find function truncate_at_arrest`.

- [ ] **Step 3: Write minimal implementation**

Add to `navmesh.rs` after `los_smooth`:

```rust
/// Truncate a continuous (polyanya) route at the first VISIBLE arrest cell (M10f-4), mirroring
/// `pathfinding::find`'s arrest truncation (spec §5, "arrest is honest in preview") for the
/// walls-only continuous path — which does not go through `find`. Arc-length-samples the route and
/// cuts at the first sample whose cell is an arrest cell in `field` (the per-requester field, so a
/// secret arrest is absent and never truncates a player's preview — it springs at `move_exec`).
/// The start sample is never a trigger (a token already standing in a cell is not "entering" it —
/// parity with `find`'s `.skip(1)`). A route with no arrest hit is returned UNCHANGED (no
/// resample). On truncation, cost is recomputed as the Euclidean length of the surviving polyline.
pub(crate) fn truncate_at_arrest(
    outcome: crate::scene::pathfinding::PathOutcome,
    field: &crate::scene::regions::RegionField,
    cell: f64,
) -> crate::scene::pathfinding::PathOutcome {
    if outcome.path.len() < 2 || !cell.is_finite() || cell <= 0.0 {
        return outcome;
    }
    let samples = crate::scene::move_stream::sample_path(&outcome.path, cell, 1.0);
    let hit = samples.iter().skip(1).position(|s| {
        let c = (
            (s.pos.0 / cell).floor() as i32,
            (s.pos.1 / cell).floor() as i32,
        );
        field.is_arrest(c)
    });
    let Some(pos) = hit else {
        return outcome; // no arrest cell on the route: unchanged
    };
    let end = pos + 1; // `position` is relative to `.skip(1)`; +1 = index into `samples`
    let kept: Vec<(f64, f64)> = samples[..=end].iter().map(|s| s.pos).collect();
    let cost: f64 = kept
        .windows(2)
        .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
        .sum();
    crate::scene::pathfinding::PathOutcome {
        path: kept,
        cost,
        arrested: true,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src/server && cargo test -p shadowcat-server truncate_at_arrest`
Expected: PASS (all 3).

- [ ] **Step 5: fmt + clippy + commit**

```bash
cd src/server && cargo fmt && cargo clippy -- -D warnings
git add src/server/src/scene/navmesh.rs
git commit -m "feat(m10f-4): truncate_at_arrest — arrest post-filter for the polyanya continuous path"
```

---

## Task 4: wire the `Continuous` dispatch in `pathfind`

**Files:**
- Modify: `src/server/src/scene/mod.rs` — the `MovementModel::Continuous` arm of `pathfind` (lines ~1079-1109)
- Test: `src/server/src/scene/mod.rs` (`mod tests`)

**Interfaces:**
- Consumes: `RegionField::has_terrain_or_impassable` (Task 1); `navmesh::los_smooth` (Task 2); `navmesh::truncate_at_arrest` (Task 3); existing `region_field`, `navmesh_for`, `navmesh::navmesh_find`, `navmesh::clip_to_visible_mask`, `pathfinding::{find, DiagonalRule, PathFail, PathOutcome}`, `mask`, `walls`, `cell`.
- Produces: the continuous branch now honors regions. The `GridStepped` branch and all non-continuous behavior are unchanged.

**Key correctness note (cost units):** the polyanya path returns `cost` in **scene units** (Euclidean length); `pathfinding::find` returns `cost` in **cells**. On the weighted path, multiply `find`'s cost by `cell` so the continuous engine reports scene units consistently regardless of which sub-path ran.

- [ ] **Step 1: Write the failing integration tests**

Add to `mod.rs` `mod tests`. (Reuse the continuous-scene idiom from `pathfind_dispatches_to_the_navmesh_router_for_a_continuous_scene`, ~line 3665, and add a local `region_doc` helper mirroring the one in `move_exec.rs` tests.)

```rust
fn continuous_scene_docs() -> Vec<crate::data::document::Document> {
    vec![entity_doc_top(
        10,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                "vision": { "movementModel": "continuous" } }),
    )]
}
fn continuous_world_settings() -> serde_json::Value {
    json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "movementModel": "continuous",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    })
}
fn region_doc_top(id: u128, parent: u128, behavior: &str, cost: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> crate::data::document::Document {
    entity_doc(
        id, parent, "region",
        json!({ "shape": { "kind": "rect", "points": [x0, y0, x1, y1] },
                "behavior": behavior, "cost": cost, "enabled": true }),
    )
}

#[test]
fn pathfind_continuous_terrain_bends_the_route_and_costs_scene_units() {
    // Continuous scene, terrain mult 5 on cell (1,0) = Rect [100,0]-[200,100] between start and
    // goal. The weighted grid route (forced Euclidean) detours through row 1 (2 diagonal steps,
    // ~2*sqrt(2) cells => *cell = ~283 scene units) instead of straight through the mult-5 cell
    // (would be 1+5 = 6 cells => 600 scene units). Proves terrain BENDS the continuous route and
    // that cost is in scene units.
    let mut docs = continuous_scene_docs();
    docs.push(region_doc_top(12, 10, "terrain", 5.0, 100.0, 0.0, 200.0, 100.0));
    let mut ecs = SceneEcs::from_documents(docs, 0);
    ecs.set_world_settings_for_test(continuous_world_settings());
    let out = ecs
        .pathfind(Uuid::from_u128(1), Uuid::from_u128(10), (50.0, 50.0), &[(250.0, 50.0)], 0.1, true, None)
        .expect("weighted continuous route");
    assert!(out.cost < 400.0, "detour taken (scene units ~283), got {}", out.cost);
    assert!(out.cost > 150.0, "cost is scene units, not cells, got {}", out.cost);
    assert!(
        out.path.iter().any(|p| p.1 > 90.0),
        "route bends off the y=50 line to avoid the terrain: {:?}",
        out.path
    );
}

#[test]
fn pathfind_continuous_no_region_is_a_straight_polyanya_route() {
    // Same scene WITHOUT a region: the pure polyanya path is taken — a straight 200px route.
    let mut ecs = SceneEcs::from_documents(continuous_scene_docs(), 0);
    ecs.set_world_settings_for_test(continuous_world_settings());
    let out = ecs
        .pathfind(Uuid::from_u128(1), Uuid::from_u128(10), (50.0, 50.0), &[(250.0, 50.0)], 0.1, true, None)
        .expect("polyanya route");
    assert!((out.cost - 200.0).abs() < 3.0, "straight Euclidean ~200, got {}", out.cost);
}

#[test]
fn pathfind_continuous_impassable_routes_around() {
    // Impassable wall-of-cells on column 1 (Rect [100,0]-[200,300]) blocks the straight line;
    // the weighted route must detour and still reach the goal.
    let mut docs = continuous_scene_docs();
    docs.push(region_doc_top(12, 10, "impassable", 1.0, 100.0, 0.0, 200.0, 300.0));
    let mut ecs = SceneEcs::from_documents(docs, 0);
    ecs.set_world_settings_for_test(continuous_world_settings());
    let out = ecs
        .pathfind(Uuid::from_u128(1), Uuid::from_u128(10), (50.0, 50.0), &[(250.0, 350.0)], 0.1, true, None)
        .expect("route around impassable");
    // No route point falls inside an impassable cell (column 1, y in [0,300)).
    assert!(
        !out.path.iter().any(|p| p.0 >= 100.0 && p.0 < 200.0 && p.1 >= 0.0 && p.1 < 300.0),
        "route threads no impassable cell: {:?}",
        out.path
    );
}

#[test]
fn pathfind_continuous_secret_terrain_absent_from_player_route_present_for_gm() {
    // gm_only terrain (mult 5) on cell (1,0). A player (non-GM) never sees it: their route is the
    // straight polyanya line (no bend, ~200 scene units). The GM's route bends (weighted).
    let mut docs = continuous_scene_docs();
    let mut secret = region_doc_top(12, 10, "terrain", 5.0, 100.0, 0.0, 200.0, 100.0);
    // Mark the region gm_only on the envelope (secret): permissions.default = "none".
    secret.permissions.default = crate::data::permission::Access::None;
    docs.push(secret);
    let mut ecs = SceneEcs::from_documents(docs, 0);
    ecs.set_world_settings_for_test(continuous_world_settings());
    let player = Uuid::from_u128(2);
    // Player (non-GM, unrestricted movement => no mask): secret terrain absent => straight route.
    let p = ecs
        .pathfind(player, Uuid::from_u128(10), (50.0, 50.0), &[(250.0, 50.0)], 0.1, false, None)
        .expect("player route");
    assert!((p.cost - 200.0).abs() < 5.0, "secret terrain does not bend the player route, got {}", p.cost);
    // GM sees the authoritative field => bends.
    let g = ecs
        .pathfind(Uuid::from_u128(1), Uuid::from_u128(10), (50.0, 50.0), &[(250.0, 50.0)], 0.1, true, None)
        .expect("gm route");
    assert!(g.cost < 400.0 && g.cost > 150.0, "GM route is weighted, got {}", g.cost);
}
```

> Note: confirm `Document.permissions.default` / `Access::None` are the exact field path + variant used by M10g's `setRegionVisibility` server mirror; if the test harness exposes a `gm_only` helper (grep the M10g region tests), prefer it. The behavioral assertion (secret absent from player route, present for GM) is the load-bearing part.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src/server && cargo test -p shadowcat-server pathfind_continuous_terrain pathfind_continuous_no_region pathfind_continuous_impassable pathfind_continuous_secret`
Expected: FAIL — the current continuous branch ignores regions (terrain test won't bend; cost stays polyanya).

- [ ] **Step 3: Rewrite the `Continuous` branch**

Replace the `MovementModel::Continuous => { ... }` arm (lines ~1079-1109) with:

```rust
MovementModel::Continuous => {
    // M10f-4: the per-requester region field is the SINGLE weighting authority for the
    // continuous engine too (polyanya cannot weight — design spec §2). Terrain or impassable
    // present ⇒ route via the weighted grid A* forced to Euclidean (continuous base metric),
    // then LOS-smooth back to any-angle geometry. Otherwise the unchanged pure polyanya route
    // + an arrest post-filter. Arrest applies on both paths. The per-requester field omits any
    // region a non-GM cannot see (secret regions spring only at `move_exec`).
    let regions = self.region_field(scene, if is_gm { None } else { Some(user) });
    if regions.has_terrain_or_impassable() {
        let weighted = pathfinding::find(
            start,
            waypoints,
            footprint_radius,
            cell,
            pathfinding::DiagonalRule::Euclidean,
            &walls,
            mask.as_ref(),
            Some(&regions),
        )?;
        // `find` reports cost in CELLS; the continuous engine reports SCENE UNITS (parity with
        // the polyanya path below). Convert before smoothing carries it through.
        let weighted = pathfinding::PathOutcome {
            cost: weighted.cost * cell,
            ..weighted
        };
        Ok(navmesh::los_smooth(
            weighted,
            &walls,
            mask.as_ref(),
            &regions,
            cell,
            footprint_radius,
        ))
    } else {
        let nav = self
            .navmesh_for(scene, footprint_radius)
            .ok_or(pathfinding::PathFail::Unreachable)?;
        let raw = navmesh::navmesh_find(&nav, start, waypoints)?;
        let raw_was_trivial = raw.path.len() < 2;
        let clipped = navmesh::clip_to_visible_mask(
            raw,
            mask.as_ref(),
            cell,
            footprint_radius,
            &walls,
        );
        if clipped.path.len() < 2 && !raw_was_trivial {
            return Err(pathfinding::PathFail::Unreachable);
        }
        Ok(navmesh::truncate_at_arrest(clipped, &regions, cell))
    }
}
```

- [ ] **Step 4: Run the new tests + the full continuous/grid regression suite**

Run: `cd src/server && cargo test -p shadowcat-server pathfind_continuous pathfind_grid_stepped_scene_is_byte_for_byte_unchanged pathfind_dispatches`
Expected: PASS — new tests pass; the grid byte-for-byte test and the existing continuous dispatch/clip tests still pass.

- [ ] **Step 5: fmt + clippy + commit**

```bash
cd src/server && cargo fmt && cargo clippy -- -D warnings
git add src/server/src/scene/mod.rs
git commit -m "feat(m10f-4): route regions on the continuous engine (weighted grid + smoothing, arrest post-filter)"
```

---

## Task 5: execution + cost-secrecy verification (tests only)

**Files:**
- Modify (tests only): `src/server/src/scene/move_exec.rs` (`mod tests`)

**Interfaces:**
- Consumes: `execute_move` (unchanged); `RegionField`; the same `region_doc`/`entity_doc` test helpers already in `move_exec.rs` tests.
- Produces: proof that the engine-agnostic executor handles a weighted+smoothed **any-angle** continuous polyline identically to the cell field's dictate (zero production change — the test IS the proof, per M10f-2/3 precedent).

- [ ] **Step 1: Write the failing test**

Add to `move_exec.rs` `mod tests` (reuse existing `entity_doc`, `region_doc`, `visible_grid` helpers seen at ~line 759):

```rust
#[test]
fn execute_move_handles_an_any_angle_weighted_continuous_polyline() {
    // A continuous (any-angle) route whose vertices are > 1 cell apart, crossing a terrain cell
    // (mult 3) and stopping at an arrest cell. Proves the engine-agnostic executor (M10f-2
    // gate_walk) gates + accrues terrain cost + arrests on a continuous polyline with no executor
    // change.
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(10, 0, "scene", json!({ "grid": { "size": 100 } })),
            entity_doc(11, 10, "token", json!({ "x": 50.0, "y": 50.0 })),
            region_doc(12, 10, "terrain", 3.0, 100.0, 0.0, 200.0, 100.0),
            region_doc(13, 10, "arrest", 1.0, 300.0, 0.0, 400.0, 100.0),
        ],
        0,
    );
    let visible = visible_grid(6);
    // Any-angle polyline: (50,50) -> (250,50) -> (350,50); the first leg is 2 cells in one hop.
    let out = execute_move(
        &ecs,
        scene_id,
        token_id,
        &[(50.0, 50.0), (250.0, 50.0), (350.0, 50.0)],
        MovementRestriction::Unrestricted,
        &visible,
        100.0,
    )
    .expect("executor handles the any-angle weighted polyline");
    assert!(out.truncated, "arrest cell (3,0) truncates the move");
    // Terrain cell (1,0) mult 3 was entered once before the arrest; cost reflects the multiplier.
    assert!(out.cost >= 3.0, "terrain multiplier accrued, got {}", out.cost);
}
```

> Note: match `execute_move`'s exact signature + `MoveOutcome` field names (`truncated`, `cost`, `render_path`) to the sibling tests in the same module; adjust the assertions to the real `MoveOutcome` shape if the field names differ. The load-bearing assertions: the move truncates at the arrest cell and accrues the terrain multiplier — with no change to `move_exec` production code.

- [ ] **Step 2: Run test to verify it passes (no production change expected)**

Run: `cd src/server && cargo test -p shadowcat-server execute_move_handles_an_any_angle_weighted_continuous_polyline`
Expected: PASS with **zero** edits to non-test code. If it fails, STOP — a failure means the executor is NOT actually engine-agnostic for this input; investigate before editing production code (halt-and-verify, per the M10f-2/3 precedent — do not "fix" the test to make it pass).

- [ ] **Step 3: Confirm `MoveStream.cost` trusted-only invariant is engine-agnostic**

Locate the existing M10g test that asserts `MoveStream.cost` is `Some` for the mover/GM and `None` for a clipped observer (grep `src/server/src/ws/conn.rs` tests for `cost` + `MoveStream`/`clip_move_stream`). Confirm it does not branch on `movement_model` (it clips on vision, not engine). Add a one-line code comment at the clip site if not already present:

```rust
// `cost` is a whole-move scalar: Some for mover/GM, None for a clipped observer — engine-agnostic
// (grid or continuous), because a continuous weighted cost may reflect gm_only terrain (M10f-4).
```

- [ ] **Step 4: Run the full server suite**

Run: `cd src/server && cargo test -p shadowcat-server`
Expected: PASS (whole suite green).

- [ ] **Step 5: fmt + clippy + commit**

```bash
cd src/server && cargo fmt && cargo clippy -- -D warnings
git add src/server/src/scene/move_exec.rs src/server/src/ws/conn.rs
git commit -m "test(m10f-4): prove engine-agnostic executor + cost-secrecy hold for weighted continuous routes"
```

---

## Task 6: docs + reviewed skill update + deviation strikes

**Files:**
- Modify: `docs/PLAN.md` (M10f-4 DONE entry + M10 status)
- Modify: `docs/superpowers/specs/2026-07-02-m10f-continuous-navmesh-movement-design.md` (§7 terrain-cost-layer strike)
- Modify: `docs/superpowers/specs/2026-06-24-m10-tokens-design.md` (§10.2/§10.3/§10.5 polyanya-cost-layer strike)
- Modify: `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md` (new continuous-weighted-route dispatch + `los_smooth`/`truncate_at_arrest` seams + "polyanya does not weight" invariant)
- Modify: `docs/TODO.md` (note: LOS smoothing keeps the grid weighted cost — a conservative preview budget; a per-cell-exact smoothed continuous cost is deferred, same divergence class as the existing M10g preview/execution cost TODO)

**Interfaces:** docs only; no code.

- [ ] **Step 1: Update `docs/PLAN.md`** — add the M10f-4 DONE entry (mirror the M10f-3 entry style): reused the M10g cell field as the single weighting authority; polyanya proven unable to weight (crate-source verified); weighted continuous = `find(Euclidean)` + cost-guarded `los_smooth`; impassable routes around; arrest via `truncate_at_arrest`/`find`; execution engine-agnostic (zero change); cost converted cell→scene-units. Mark M10f COMPLETE and note the push gate (full M10).

- [ ] **Step 2: Strike the stale cost-layer claims** — in the parent spec §7 and M10-tokens §10.2/§10.3/§10.5, annotate that "terrain → polyanya cost-layer (Split-Mesh)" is infeasible in `polyanya-0.16.1` (cite the M10f-4 design spec §2 crate-source verification) and is replaced by the shared cell field + Euclidean-weighted grid search + LOS smoothing. Do not delete history — annotate in place (immutable-history discipline).

- [ ] **Step 3: Update the scene-rendering codebase skill** — add: the continuous `pathfind` dispatch (`has_terrain_or_impassable` → weighted `find(Euclidean)` + `los_smooth`, else polyanya + `truncate_at_arrest`); the cell→scene-unit cost conversion on the weighted continuous path; `los_smooth`'s cost-guard invariant; the "polyanya does not weight; the cell field is the universal overlay" hard invariant; that the `GridStepped` branch is unchanged.

- [ ] **Step 4: Reviewed skill-update gate** — dispatch `shadowcat-spec-reviewer` on the skill diff to confirm it accurately captures the change (no omission/drift/broken pointer). This blocks completion at the doc-sync tier.

- [ ] **Step 5: Commit**

```bash
git add docs/ .claude/skills/shadowcat-codebase-scene-rendering/SKILL.md
git commit -m "docs(m10f-4): PLAN sync, strike stale polyanya-cost-layer claims, update scene-rendering skill"
```

---

## Self-Review

**1. Spec coverage** (design spec `2026-07-03-m10f-4-regions-on-navmesh-design.md`):
- §2.1 cell field is the weighting authority → Task 4 (reads `region_field`, no polyanya cost-layer). ✓
- §2.2 polyanya cannot weight; strike stale claim → Task 6 §2. ✓
- §2.3 weighted route = `find(Euclidean)` + smooth → Task 4 + Task 2. ✓
- §2.4 LOS smoothing ships in v1 → Task 2. ✓
- §2.5 grid is a cost-search discretization, not snapping → no snap code touched; documented (Task 6). ✓
- §4 dispatch (predicate, Euclidean force, GM=None) → Task 1 + Task 4. ✓
- §5 LOS smoothing with cost-guard → Task 2. ✓
- §6 arrest (both paths) + impassable route-around → Task 3 (polyanya path) + `find` (weighted path, existing) + Task 4 wiring. ✓
- §7 execution already done, verify → Task 5. ✓
- §8 secrecy inherited; `route ⊆ gate-allowed`; `MoveStream.cost` trusted-only → Task 4 (per-requester field, secret-region test) + Task 5 (cost-secrecy). ✓
- §9 protocol no change → no wire task. ✓
- §10 client no change → no client task (verified: no new frames). ✓
- §11 no new crate → Global Constraints. ✓
- §14 testing → Tasks 1-5 mirror the M10g §11 suite over the continuous route. ✓

**2. Placeholder scan:** All code steps contain full code. The two "Note:" callouts (Task 4 permission-field exact path; Task 5 `MoveOutcome` field names) direct the implementer to confirm exact identifiers against sibling tests — the behavioral assertions are concrete; the identifiers are real-but-verify. No "TODO/TBD/handle edge cases" placeholders.

**3. Type consistency:** `PathOutcome { path: Vec<(f64,f64)>, cost: f64, arrested: bool }`, `Cell = (i32,i32)`, `DiagonalRule::Euclidean`, `PathFail::Unreachable`, `RegionField::{has_terrain_or_impassable, is_impassable, is_arrest, terrain_multiplier, builder}`, `RegionShape::Rect{x0,y0,x1,y1}`, `RegionBehavior::{Terrain,Impassable,Arrest}`, `los_smooth`/`truncate_at_arrest` signatures — all match across Tasks 1-5 and the verified current source (`find` at `pathfinding.rs:454`, `clip_to_visible_mask` at `navmesh.rs:311`, `pathfind` Continuous arm at `mod.rs:1079`). Cost-unit conversion (`× cell`) applied once, on the weighted continuous path only.
