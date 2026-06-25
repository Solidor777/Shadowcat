# M10e-6 — Grid A\* Pathfinder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A server-authoritative, engine-owned grid A\* pathfinder (request/response like search) that returns a footprint-clear, mask-bounded cell-stepped route + movement cost; plus a client measure-tool route mode with path preview, a movement-budget readout, and an `alternating` (5-10-5) ruler rule.

**Architecture:** A pure `scene/pathfinding.rs` module (king-move A\*, four diagonal-cost rules, 5-10-5 parity in the node state, full geometric footprint-disc clearance, fail-closed search bounds) consumes assembled inputs (`blocksMove` walls, the per-`(user,scene)` visibility mask, cell size, rule, footprint). A thin `SceneEcs::pathfind` assembles those inputs — reusing the **same** `visible_cells` mask as the M10e-4 movement gate (parent §13) — and new `Pathfind`/`PathResult`/`PathError` frames (mirroring `Search`, one-shot to the requester) wire it up. The client mirrors `WsClient.search` with `pathfind`, extends the measure tool into a waypoint router, and teaches the ruler the alternating rule.

**Tech Stack:** Rust (`shadowcat` crate), `hecs` ECS, `tokio`, `ts-rs` (wire→TS), `serde_json`; client TypeScript (`@shadowcat/core` wire + ws-client, `@shadowcat/ui-kit` AppContext, `@shadowcat/module-scene-tools`, `@shadowcat/render` grid), Zod, Vitest.

## Global Constraints

- **Server crate is `shadowcat`** (NOT `shadowcat-server`). Build/test: `cargo test -p shadowcat`, `cargo fmt`, `cargo clippy -p shadowcat --all-targets -- -D warnings`. Client: `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint`.
- **`dist/` must be built before any `cargo` build of the server** (`rust-embed` validates `../../dist/` at compile time): `pnpm build` first if the server build complains.
- **Cross-platform** — pure Rust, no OS-specific paths/`#[cfg]`. Determinism: no `HashMap` iteration into ordered/wire output; tie-break the A\* open set deterministically; `BTreeSet`/sorted `Vec` for wire.
- **Same mask as the gate (parent §13):** the pathfinder's per-cell mask test is the existing `SceneEcs::visible_cells(user, scene, lenient)` (strict ≡ `player_lit_mask`); never fork the per-cell visibility decision.
- **Fail-closed** (`fog-is-the-secrecy-gate-fail-closed`): degenerate request / over-cap search / empty non-GM mask ⇒ `PathError`, never a partial or geometry-leaking route.
- **Server mirrors client resolver exactly** (`server-mirrors-client-resolver-semantics`): the `diagonalRule` resolution must equal the client default in `src/client/core/src/scene-docs.ts` — verify against that source, not this plan's paraphrase.
- **ts-rs is the wire source of truth:** new frames are Rust enums with `#[ts(export, export_to = "../../types/generated/")]`, `#[serde(tag = "type", rename_all = "snake_case")]`; the client Zod schema mirrors them.
- **No debug code:** leveled `tracing` only; no `println!`/`dbg!`/`console.log` (client logger only). Comments: present-tense current-state, cite algorithm sources, lead with invariants/coupling (project `CLAUDE.md`).

**Authoritative inputs (read before coding):**
- Spec: `docs/superpowers/specs/2026-06-25-m10e-6-grid-pathfinder-design.md` (the focused spec) + the parent `2026-06-24-m10e-vision-lighting-movement-design.md` §10 + §13.
- Search frame model: `src/server/src/ws/protocol.rs:38-45` (`Search`), `:158-164` (`SearchResult`/`SearchError`); `src/server/src/ws/conn.rs` (one-shot ingress dispatch to the requesting connection).
- The mask + scene resolvers: `src/server/src/scene/mod.rs` — `visible_cells` (`:1073-1210`), `resolve_scene` (`:324-433`), `blocks_move` + `segments_cross` (`:1215-1244`, `:1355`), `scene_grid_sizes` (`:582`), `sight_walls`/`light_walls` (`:601`,`:631`).
- Geometry: `src/server/src/scene/vision.rs:11-18` (`P`, `Seg{a,b}`). Cells: `src/server/src/scene/movement.rs` / `explored.rs` (`type Cell = (i32,i32)`); `ExploredSet::cells()` iterator (`explored.rs:44-46`), `contains` (`:40`).
- Explored repo: `src/server/src/data/repository.rs:91` (`get_explored(scene,user) -> Result<Option<Vec<u8>>>`).
- Client: `src/client/core/src/ws-client.ts:323-348` (`search`), `:80-88,256-263` (`pending` correlation); `src/client/core/src/wire.ts:140-258` (Zod + `ClientMsg`); `src/modules/scene-tools/src/controller.svelte.ts:16-38,175-192,211-318` (ToolContext + measure/draw tools); `src/client/render/src/types.ts:64-120` (`ShapeNodeSpec`, `SceneToolHost`); `src/client/ui-kit/src/appContext.ts:17-71`, `sceneInteraction.ts`; `src/client/render/src/grid.ts:34-43` (`distance`); `src/client/core/src/scene-docs.ts:17` (`GridDistance`).

---

## File Structure

- **Create** `src/server/src/scene/pathfinding.rs` — the pure pather: `DiagonalRule`, `PathGrid`, `PathFail`, `cell_enterable`, `astar_leg`, `find`, plus `MAX_WAYPOINTS`/`MAX_FOOTPRINT_CELLS`/`MAX_PATH_NODES`. No I/O, no ECS borrow.
- **Modify** `src/server/src/scene/vision.rs` — add `pub(crate) fn point_segment_distance(p, a, b) -> f64`.
- **Modify** `src/server/src/scene/mod.rs` — lift `segments_cross` to `pub(crate)`; add `mod pathfinding;`; add `move_walls(scene) -> Vec<vision::Seg>`, `resolved_diagonal_rule() -> pathfinding::DiagonalRule`, and `pathfind(...)` assembly method.
- **Modify** `src/server/src/ws/protocol.rs` — `ClientMsg::Pathfind`, `ServerMsg::{PathResult, PathError}`.
- **Modify** `src/server/src/ws/conn.rs` — ingress handler for `Pathfind` (one-shot to requester; `get_explored` for non-GM `Revealed`).
- **Modify** `src/client/core/src/wire.ts` — Zod for the 3 frames + `ClientMsg` union member.
- **Modify** `src/client/core/src/ws-client.ts` — `pathfind(...)` mirroring `search`.
- **Modify** `src/client/ui-kit/src/appContext.ts` + `src/client/shell/src/lib/worldSession.svelte.ts` + `src/client/shell/src/lib/Table.svelte` — `AppContext.pathfind` seam.
- **Modify** `src/modules/scene-tools/src/controller.svelte.ts` — measure tool route mode (waypoints + pathfind + preview + budget).
- **Modify** `src/client/render/src/grid.ts` — `distance()` `alternating` rule.
- **Docs (closeout)** — `docs/PLAN.md`, `docs/TODO.md`, `docs/POST_WORK_FINDINGS.md`, `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md`.

---

### Task 1: `point_segment_distance` geometry helper + `move_walls` accessor

The footprint-disc clearance test needs point-to-segment distance; the pather needs the scene's `blocksMove` segments. Add both as small, reused primitives.

**Files:**
- Modify: `src/server/src/scene/vision.rs` (new free fn)
- Modify: `src/server/src/scene/mod.rs` (lift `segments_cross` to `pub(crate)`; add `move_walls`)
- Test: `src/server/src/scene/vision.rs` (inline), `src/server/src/scene/mod.rs` (inline)

**Interfaces:**
- Produces: `pub(crate) fn point_segment_distance(p: P, a: P, b: P) -> f64` (in `vision`)
- Produces: `pub(crate) fn segments_cross(p1: P, p2: P, p3: P, p4: P) -> bool` (visibility widened from private)
- Produces: `pub(crate) fn move_walls(&self, scene: Uuid) -> Vec<vision::Seg>` (on `SceneEcs`)

- [ ] **Step 1: Write the failing tests**

In `vision.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn point_segment_distance_endpoints_midpoint_and_perpendicular() {
    let a = (0.0, 0.0);
    let b = (10.0, 0.0);
    // Perpendicular foot inside the segment.
    assert!((point_segment_distance((5.0, 3.0), a, b) - 3.0).abs() < 1e-9);
    // Beyond an endpoint clamps to that endpoint.
    assert!((point_segment_distance((-4.0, 0.0), a, b) - 4.0).abs() < 1e-9);
    // On the segment → 0.
    assert!(point_segment_distance((7.0, 0.0), a, b) < 1e-9);
    // Degenerate segment (a == b) → distance to the point.
    assert!((point_segment_distance((3.0, 4.0), (0.0, 0.0), (0.0, 0.0)) - 5.0).abs() < 1e-9);
}
```

In `mod.rs`'s test module:

```rust
#[test]
fn move_walls_returns_only_blocks_move_segments_for_the_scene() {
    // Reuse the test scene scaffold (see blocks_move_geometry_scene_scoping_and_filters at mod.rs ~:1568).
    // A scene with one blocksMove wall and one non-blocksMove wall yields exactly the blocking segment.
    let (ecs, scene) = scene_with_two_walls_one_blocking(); // test helper (compose like the blocks_move test)
    let walls = ecs.move_walls(scene);
    assert_eq!(walls.len(), 1, "only the blocksMove wall is returned");
    let w = walls[0];
    assert_eq!((w.a, w.b), ((100.0, 0.0), (100.0, 200.0)));
}
```

> If `scene_with_two_walls_one_blocking` doesn't exist, build it in the test module from the same wall-doc construction the existing `blocks_move` test uses (`doc_type:"wall"`, `system:{ "seg": {x1,y1,x2,y2}, "blocksMove": bool }`, `parent_id = scene`).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat point_segment_distance && cargo test -p shadowcat move_walls_returns_only`
Expected: FAIL — functions do not exist (`move_walls` not found; `point_segment_distance` not found).

- [ ] **Step 3: Implement**

In `vision.rs` (near the other geometry helpers):

```rust
/// Euclidean distance from point `p` to segment `a→b`, clamping the projection to the segment.
/// Source: standard point-to-segment projection (clean-room). Used by the pathfinder footprint
/// clearance: a footprint disc of radius R is wall-clear iff this distance ≥ R for every wall.
pub(crate) fn point_segment_distance(p: P, a: P, b: P) -> f64 {
    let (px, py) = p;
    let (ax, ay) = a;
    let (bx, by) = b;
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= f64::EPSILON {
        0.0 // degenerate segment: distance to point `a`
    } else {
        (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
    };
    let (fx, fy) = (ax + t * dx, ay + t * dy);
    ((px - fx).powi(2) + (py - fy).powi(2)).sqrt()
}
```

In `mod.rs`: change `fn segments_cross(` (line ~1355) to `pub(crate) fn segments_cross(`. Add the accessor on `impl SceneEcs` (next to `light_walls`):

```rust
/// The scene's `blocksMove` wall segments. Feeds the M10e-6 pathfinder's footprint clearance
/// and step tests; mirrors the wall filter in `blocks_move` (doc_type "wall", parent = scene,
/// `system.blocksMove == true`, endpoints at `system.seg.{x1,y1,x2,y2}`).
pub(crate) fn move_walls(&self, scene: Uuid) -> Vec<vision::Seg> {
    let mut out = Vec::new();
    for w in self.world.query::<&SceneEntity>().iter() {
        if w.doc.doc_type != "wall" || w.doc.parent_id != Some(scene) {
            continue;
        }
        if w.doc.system.pointer("/blocksMove").and_then(|v| v.as_bool()) != Some(true) {
            continue;
        }
        if let (Some(x1), Some(y1), Some(x2), Some(y2)) = (
            sys_f64(&w.doc, "/seg/x1"),
            sys_f64(&w.doc, "/seg/y1"),
            sys_f64(&w.doc, "/seg/x2"),
            sys_f64(&w.doc, "/seg/y2"),
        ) {
            out.push(vision::Seg { a: (x1, y1), b: (x2, y2) });
        }
    }
    out
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat point_segment_distance && cargo test -p shadowcat move_walls_returns_only`
Expected: PASS.

- [ ] **Step 5: Lint**

Run: `cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/vision.rs src/server/src/scene/mod.rs
git commit -m "feat(m10e-6): point_segment_distance + move_walls accessor (pathfinder geometry inputs)"
```

---

### Task 2: `DiagonalRule` + `resolved_diagonal_rule` resolver

Resolve `world-settings.pathfinding.diagonalRule` into an enum (server mirrors the client default). `diagonalRule` is world-only (no per-scene override; the scene doc overrides only `vision`/`lighting`/`grid` — parent §5.2).

**Files:**
- Create: `src/server/src/scene/pathfinding.rs` (the enum lives here; the module's other items come in Tasks 3-5)
- Modify: `src/server/src/scene/mod.rs` (`mod pathfinding;` + the resolver method)
- Test: `src/server/src/scene/mod.rs` (inline)

**Interfaces:**
- Produces: `pub enum DiagonalRule { Chebyshev, Manhattan, Euclidean, Alternating }` (derive `Clone, Copy, Debug, PartialEq, Eq`) in `pathfinding`
- Produces: `pub fn parse_diagonal_rule(s: &str) -> DiagonalRule` (unknown ⇒ `Chebyshev`)
- Produces: `pub(crate) fn resolved_diagonal_rule(&self) -> pathfinding::DiagonalRule` (on `SceneEcs`)

- [ ] **Step 1: Write the failing tests**

In `mod.rs`'s test module (reuse `set_world_settings_for_test` from the M10e-4 tests):

```rust
#[test]
fn diagonal_rule_defaults_to_chebyshev_without_world_settings() {
    let ecs = SceneEcs::new();
    assert_eq!(ecs.resolved_diagonal_rule(), crate::scene::pathfinding::DiagonalRule::Chebyshev);
}

#[test]
fn diagonal_rule_reads_world_settings_and_unknown_falls_back() {
    use serde_json::json;
    let mut ecs = SceneEcs::new();
    ecs.set_world_settings_for_test(json!({
        "scene": { "movementRestriction": "visible", "partialCellLeniency": true },
        "pathfinding": { "diagonalRule": "alternating" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    }));
    assert_eq!(ecs.resolved_diagonal_rule(), crate::scene::pathfinding::DiagonalRule::Alternating);

    ecs.set_world_settings_for_test(json!({
        "scene": {}, "pathfinding": { "diagonalRule": "bogus" }, "animation": {}
    }));
    assert_eq!(ecs.resolved_diagonal_rule(), crate::scene::pathfinding::DiagonalRule::Chebyshev,
        "unknown rule fails to chebyshev (mirrors client default)");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat diagonal_rule`
Expected: FAIL — `pathfinding` module / `resolved_diagonal_rule` do not exist.

- [ ] **Step 3: Create the module + enum, wire the resolver**

Create `src/server/src/scene/pathfinding.rs` with the module header + enum (the rest of the module is added in Tasks 3-5):

```rust
//! Server-authoritative grid A* pathfinder (M10e-6). Pure + headless: callers pass parsed inputs
//! (walls, mask, cell size, rule, footprint); this module owns no I/O and borrows no ECS.
//! Engine-owned geometry (ARCHITECTURE §6 exception); clean-room A* (Hart, Nilsson & Raphael 1968).
//!
//! INVARIANT (spec §13): the per-cell mask test consumes the SAME `visible_cells` set the M10e-4
//! movement gate uses — the route can never thread the unknown nor leak hidden geometry.

/// Grid diagonal-cost rule (from `world-settings.pathfinding.diagonalRule`). All four are the same
/// king-move graph; they differ only in diagonal cost + the admissible heuristic. `Alternating`
/// (PF1e/3.5 "5-10-5") costs diagonals 1,2,1,2… and so requires a parity bit in the search node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagonalRule {
    Chebyshev,
    Manhattan,
    Euclidean,
    Alternating,
}

/// Parse the diagonal-rule string; unknown/missing ⇒ `Chebyshev` (mirrors the client
/// `DEFAULT_WORLD_SETTINGS.pathfinding.diagonalRule` in `scene-docs.ts`).
pub fn parse_diagonal_rule(s: &str) -> DiagonalRule {
    match s {
        "manhattan" => DiagonalRule::Manhattan,
        "euclidean" => DiagonalRule::Euclidean,
        "alternating" => DiagonalRule::Alternating,
        _ => DiagonalRule::Chebyshev,
    }
}
```

In `mod.rs`, add `mod pathfinding;` near `mod movement;`, and the resolver on `impl SceneEcs`:

```rust
/// The world's pathfinding diagonal-cost rule. World-scoped (no per-scene override; the scene doc
/// overrides only vision/lighting/grid — parent §5.2). Reads `world-settings.pathfinding.diagonalRule`.
pub(crate) fn resolved_diagonal_rule(&self) -> pathfinding::DiagonalRule {
    let s = self
        .world_settings
        .as_ref()
        .and_then(|d| d.system.pointer("/pathfinding/diagonalRule"))
        .and_then(|v| v.as_str())
        .unwrap_or("chebyshev");
    pathfinding::parse_diagonal_rule(s)
}
```

> Match the actual field name for the world-settings doc on `SceneEcs` (the M10e-2 side-table; e.g. `self.world_settings`). If it is wrapped (`Option<Document>`), adjust the `.as_ref().and_then(|d| d.system.pointer(...))` accordingly. `mod pathfinding;` must be `pub(crate)` if `conn.rs` (Task 7) needs `pathfinding::PathFail` — declare it `pub(crate) mod pathfinding;`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat diagonal_rule`
Expected: PASS (2 tests).

- [ ] **Step 5: Lint**

Run: `cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/pathfinding.rs src/server/src/scene/mod.rs
git commit -m "feat(m10e-6): DiagonalRule enum + resolved_diagonal_rule (mirrors client default)"
```

---

### Task 3: Footprint passability — `PathGrid` + `cell_enterable`

The core passability predicate: a candidate cell is enterable iff (1) the footprint disc at its center clears every `blocksMove` wall (full geometric clearance), (2) every footprint-overlapped cell is in the mask (non-GM), and (3) the center-to-center step crosses no wall. Pure.

**Files:**
- Modify: `src/server/src/scene/pathfinding.rs`
- Test: `src/server/src/scene/pathfinding.rs` (inline)

**Interfaces:**
- Consumes: `vision::{P, Seg, point_segment_distance}`, `crate::scene::segments_cross` (Task 1).
- Produces:
  - `pub type Cell = (i32, i32);`
  - `pub struct PathGrid<'a> { pub cell: f64, pub rule: DiagonalRule, pub footprint_radius_cells: f64, pub walls: &'a [vision::Seg], pub mask: Option<&'a std::collections::BTreeSet<Cell>>, pub window: (i32, i32, i32, i32) }`
  - `pub fn cell_center(c: Cell, cell: f64) -> vision::P`
  - `pub(crate) fn cell_enterable(grid: &PathGrid, from: Cell, to: Cell) -> bool`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::vision::Seg;
    use std::collections::BTreeSet;

    fn grid<'a>(
        walls: &'a [Seg],
        mask: Option<&'a BTreeSet<Cell>>,
        footprint: f64,
    ) -> PathGrid<'a> {
        PathGrid { cell: 100.0, rule: DiagonalRule::Chebyshev, footprint_radius_cells: footprint,
                   walls, mask, window: (-100, -100, 100, 100) }
    }

    #[test]
    fn open_neighbor_is_enterable_no_walls_no_mask() {
        let walls: Vec<Seg> = vec![];
        let g = grid(&walls, None, 0.2);
        assert!(cell_enterable(&g, (0, 0), (1, 0)));
        assert!(cell_enterable(&g, (0, 0), (1, 1)));
    }

    #[test]
    fn step_crossing_a_blocks_move_wall_is_not_enterable() {
        // A vertical wall on the x=100 grid line blocks the (0,0)->(1,0) center step (50,50)->(150,50).
        let walls = vec![Seg { a: (100.0, 0.0), b: (100.0, 200.0) }];
        let g = grid(&walls, None, 0.2);
        assert!(!cell_enterable(&g, (0, 0), (1, 0)), "center step crosses the wall");
    }

    #[test]
    fn footprint_disc_too_wide_for_a_gap_is_not_enterable() {
        // Two walls one cell apart (x=100 and x=200). A footprint radius 0.7 cell (=70 units) at the
        // center of cell (1,0) (center x=150) is within 50 units of BOTH walls → blocked (the body
        // can't fit the 1-cell gap). A small radius (0.2 cell = 20 units) clears it.
        let walls = vec![
            Seg { a: (100.0, 0.0), b: (100.0, 200.0) },
            Seg { a: (200.0, 0.0), b: (200.0, 200.0) },
        ];
        let wide = grid(&walls, None, 0.7);
        let narrow = grid(&walls, None, 0.2);
        // Use a step that does not itself cross a wall: (1,1)->(1,0) (vertical, x=150 throughout).
        assert!(!cell_enterable(&wide, (1, 1), (1, 0)), "wide footprint cannot fit the gap");
        assert!(cell_enterable(&narrow, (1, 1), (1, 0)), "narrow footprint fits");
    }

    #[test]
    fn footprint_cell_outside_mask_is_not_enterable() {
        // Non-GM mask containing only cell (1,0). A footprint disc that overlaps neighbors requires
        // all overlapped cells in the mask. Radius 0.6 cell at center of (1,0) overlaps (0,0)/(2,0)/
        // (1,-1)/(1,1) edges → those must be in mask. With only (1,0) present → not enterable.
        let walls: Vec<Seg> = vec![];
        let mut mask = BTreeSet::new();
        mask.insert((1, 0));
        let g = grid(&walls, Some(&mask), 0.6);
        assert!(!cell_enterable(&g, (1, 1), (1, 0)), "overlapped neighbor cells not in mask");

        // A point-sized footprint overlaps only (1,0) → enterable.
        let gp = grid(&walls, Some(&mask), 0.0);
        assert!(cell_enterable(&gp, (1, 1), (1, 0)));
    }

    #[test]
    fn cell_outside_window_is_not_enterable() {
        let walls: Vec<Seg> = vec![];
        let mut g = grid(&walls, None, 0.2);
        g.window = (0, 0, 2, 2);
        assert!(!cell_enterable(&g, (2, 2), (3, 2)), "outside the search window");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat scene::pathfinding`
Expected: FAIL — `PathGrid`/`cell_enterable`/`Cell` not defined.

- [ ] **Step 3: Implement**

Append to `pathfinding.rs`:

```rust
use crate::scene::vision::{self, point_segment_distance};
use std::collections::BTreeSet;

/// A grid cell `(i, j)`; cell `(i,j)` covers `[i*cell,(i+1)*cell) × [j*cell,(j+1)*cell)`.
pub type Cell = (i32, i32);

/// Assembled, borrow-only inputs for one A* search. `mask = None` ⇒ unconstrained (GM or
/// `unrestricted`); `Some(set)` ⇒ a cell (and every footprint-overlapped cell) must be in the set.
/// `window` (i0,j0,i1,j1 inclusive) bounds the search so a GM query with an unreachable goal can't
/// wander unboundedly.
pub struct PathGrid<'a> {
    pub cell: f64,
    pub rule: DiagonalRule,
    pub footprint_radius_cells: f64,
    pub walls: &'a [vision::Seg],
    pub mask: Option<&'a BTreeSet<Cell>>,
    pub window: (i32, i32, i32, i32),
}

/// Center of cell `c` in scene coords.
pub fn cell_center(c: Cell, cell: f64) -> vision::P {
    ((c.0 as f64 + 0.5) * cell, (c.1 as f64 + 0.5) * cell)
}

/// Cells whose AABB the footprint disc (center `ctr`, radius `r_scene`) overlaps. A cell overlaps
/// the disc iff the disc center is within `r_scene` of the cell's AABB. The anchor cell is always
/// included (a zero-radius disc overlaps exactly its own cell).
fn footprint_cells(anchor: Cell, ctr: vision::P, r_scene: f64, cell: f64) -> Vec<Cell> {
    let mut out = Vec::new();
    let i0 = ((ctr.0 - r_scene) / cell).floor() as i32;
    let i1 = ((ctr.0 + r_scene) / cell).floor() as i32;
    let j0 = ((ctr.1 - r_scene) / cell).floor() as i32;
    let j1 = ((ctr.1 + r_scene) / cell).floor() as i32;
    for i in i0..=i1 {
        for j in j0..=j1 {
            // Distance from disc center to this cell's AABB.
            let minx = i as f64 * cell;
            let maxx = (i + 1) as f64 * cell;
            let miny = j as f64 * cell;
            let maxy = (j + 1) as f64 * cell;
            let dx = (minx - ctr.0).max(0.0).max(ctr.0 - maxx);
            let dy = (miny - ctr.1).max(0.0).max(ctr.1 - maxy);
            if dx * dx + dy * dy <= r_scene * r_scene {
                out.push((i, j));
            }
        }
    }
    if out.is_empty() {
        out.push(anchor);
    }
    out
}

/// Whether a token may step from `from` into `to`. INVARIANT (spec §4.3): full geometric footprint
/// clearance — (1) the footprint disc at `to` clears every `blocksMove` wall, (2) every
/// footprint-overlapped cell is in the mask (non-GM), (3) the center step `from→to` crosses no wall.
pub(crate) fn cell_enterable(grid: &PathGrid, from: Cell, to: Cell) -> bool {
    let (i0, j0, i1, j1) = grid.window;
    if to.0 < i0 || to.0 > i1 || to.1 < j0 || to.1 > j1 {
        return false;
    }
    let r_scene = grid.footprint_radius_cells.max(0.0) * grid.cell;
    let ctr = cell_center(to, grid.cell);

    // (1) Footprint disc vs every blocksMove wall.
    for w in grid.walls {
        if point_segment_distance(ctr, w.a, w.b) < r_scene {
            return false;
        }
    }
    // (2) Mask: every footprint-overlapped cell must be visible/revealed (non-GM).
    if let Some(mask) = grid.mask {
        for c in footprint_cells(to, ctr, r_scene, grid.cell) {
            if !mask.contains(&c) {
                return false;
            }
        }
    }
    // (3) Center-to-center step clears every wall (reuses the M9 segment-cross predicate).
    let a = cell_center(from, grid.cell);
    for w in grid.walls {
        if crate::scene::segments_cross(a, ctr, w.a, w.b) {
            return false;
        }
    }
    true
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat scene::pathfinding`
Expected: PASS (5 tests).

- [ ] **Step 5: Lint**

Run: `cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/pathfinding.rs
git commit -m "feat(m10e-6): PathGrid + cell_enterable (full geometric footprint clearance + mask + step)"
```

---

### Task 4: Single-leg A\* — `astar_leg` (rules, parity, heuristics, bounds)

King-move A\* over `cell_enterable`. Node = `(cell, parity)`; parity tracks 5-10-5 diagonal count and is a no-op for the other rules. Per-rule admissible+consistent heuristics. Deterministic tie-break; node-cap backstop.

**Files:**
- Modify: `src/server/src/scene/pathfinding.rs`
- Test: `src/server/src/scene/pathfinding.rs` (inline)

**Interfaces:**
- Consumes: `PathGrid`, `cell_enterable`, `DiagonalRule` (Task 3/2).
- Produces:
  - `pub enum PathFail { Invalid, Unreachable, Exceeded }` (derive `Debug, PartialEq, Eq`)
  - `pub(crate) const MAX_PATH_NODES: usize = 200_000;`
  - `pub(crate) fn astar_leg(grid: &PathGrid, start: Cell, goal: Cell, start_parity: u8) -> Result<(Vec<Cell>, f64, u8), PathFail>` (returns leg cells start..=goal, leg cost, end parity)

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod astar_tests {
    use super::*;
    use crate::scene::vision::Seg;

    fn open(rule: DiagonalRule, footprint: f64) -> PathGrid<'static> {
        const NO_WALLS: [Seg; 0] = [];
        PathGrid { cell: 100.0, rule, footprint_radius_cells: footprint, walls: &NO_WALLS,
                   mask: None, window: (-50, -50, 50, 50) }
    }

    #[test]
    fn chebyshev_diagonal_is_cost_one_per_step() {
        let g = open(DiagonalRule::Chebyshev, 0.1);
        let (cells, cost, _p) = astar_leg(&g, (0, 0), (3, 3), 0).unwrap();
        assert!((cost - 3.0).abs() < 1e-9, "3 diagonal steps at cost 1 each");
        assert_eq!(cells.first(), Some(&(0, 0)));
        assert_eq!(cells.last(), Some(&(3, 3)));
    }

    #[test]
    fn manhattan_diagonal_costs_two() {
        let g = open(DiagonalRule::Manhattan, 0.1);
        let (_c, cost, _p) = astar_leg(&g, (0, 0), (2, 2), 0).unwrap();
        // Manhattan distance to (2,2) is 4 whether via diagonals (cost 2 each) or orthogonals.
        assert!((cost - 4.0).abs() < 1e-9);
    }

    #[test]
    fn euclidean_diagonal_costs_sqrt2() {
        let g = open(DiagonalRule::Euclidean, 0.1);
        let (_c, cost, _p) = astar_leg(&g, (0, 0), (1, 1), 0).unwrap();
        assert!((cost - std::f64::consts::SQRT_2).abs() < 1e-9);
    }

    #[test]
    fn alternating_five_ten_five_parity() {
        // Two consecutive diagonals from parity 0: first costs 1, second costs 2 → total 3, end parity 0.
        let g = open(DiagonalRule::Alternating, 0.1);
        let (_c, cost, parity) = astar_leg(&g, (0, 0), (2, 2), 0).unwrap();
        assert!((cost - 3.0).abs() < 1e-9, "5-10-5: diagonals cost 1 then 2");
        assert_eq!(parity, 0, "two diagonals → parity back to 0");

        // Three diagonals from parity 0: 1 + 2 + 1 = 4, end parity 1.
        let (_c, cost3, parity3) = astar_leg(&g, (0, 0), (3, 3), 0).unwrap();
        assert!((cost3 - 4.0).abs() < 1e-9);
        assert_eq!(parity3, 1);
    }

    #[test]
    fn walled_off_goal_is_unreachable() {
        // Box the goal cell in with blocksMove walls on all four sides → Unreachable (terminates,
        // bounded by the window).
        let c = 100.0;
        let walls = vec![
            Seg { a: (3.0 * c, 3.0 * c), b: (4.0 * c, 3.0 * c) },
            Seg { a: (3.0 * c, 4.0 * c), b: (4.0 * c, 4.0 * c) },
            Seg { a: (3.0 * c, 3.0 * c), b: (3.0 * c, 4.0 * c) },
            Seg { a: (4.0 * c, 3.0 * c), b: (4.0 * c, 4.0 * c) },
        ];
        let g = PathGrid { cell: c, rule: DiagonalRule::Chebyshev, footprint_radius_cells: 0.1,
                           walls: &walls, mask: None, window: (-10, -10, 10, 10) };
        assert_eq!(astar_leg(&g, (0, 0), (3, 3), 0), Err(PathFail::Unreachable));
    }

    #[test]
    fn start_equals_goal_is_a_single_cell_zero_cost() {
        let g = open(DiagonalRule::Chebyshev, 0.1);
        let (cells, cost, p) = astar_leg(&g, (2, 2), (2, 2), 1).unwrap();
        assert_eq!(cells, vec![(2, 2)]);
        assert!(cost.abs() < 1e-9);
        assert_eq!(p, 1, "parity is carried unchanged when no step is taken");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat scene::pathfinding::astar_tests`
Expected: FAIL — `astar_leg`/`PathFail` not defined.

- [ ] **Step 3: Implement**

Append to `pathfinding.rs`:

```rust
use std::collections::{BinaryHeap, HashMap};

/// Why a path request fails. Mapped to a `PathError` message at the wire boundary.
#[derive(Debug, PartialEq, Eq)]
pub enum PathFail {
    Invalid,     // degenerate request (no destination, non-finite, out-of-range footprint)
    Unreachable, // no route within walls/mask/window
    Exceeded,    // search exceeded MAX_PATH_NODES (DoS backstop)
}

/// DoS backstop: total node expansions per leg. For non-GM the mask is the tighter bound; this caps
/// a GM search whose window is large.
pub(crate) const MAX_PATH_NODES: usize = 200_000;

/// f64 ordering wrapper for the min-heap. Orders by `f` ascending (via reversed `total_cmp`),
/// tie-broken by `(cell, parity)` so identical requests yield identical routes (determinism).
#[derive(PartialEq)]
struct QNode {
    f: f64,
    cell: Cell,
    parity: u8,
}
impl Eq for QNode {}
impl Ord for QNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Min-heap: smaller f is "greater". Reverse the f comparison; tie-break ascending on key.
        other
            .f
            .total_cmp(&self.f)
            .then_with(|| self.cell.cmp(&other.cell))
            .then_with(|| self.parity.cmp(&other.parity))
    }
}
impl PartialOrd for QNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Step cost and successor parity for moving by `(di, dj)` (each in -1..=1, not both 0) under `rule`
/// from a node with diagonal-parity `parity`. Source: standard grid metrics; `Alternating` is the
/// PF1e/3.5 5-10-5 rule (k-th diagonal costs 1 if k odd else 2).
fn step_cost(rule: DiagonalRule, di: i32, dj: i32, parity: u8) -> (f64, u8) {
    let diagonal = di != 0 && dj != 0;
    if !diagonal {
        return (1.0, parity);
    }
    match rule {
        DiagonalRule::Chebyshev => (1.0, parity),
        DiagonalRule::Manhattan => (2.0, parity),
        DiagonalRule::Euclidean => (std::f64::consts::SQRT_2, parity),
        DiagonalRule::Alternating => {
            let cost = if parity == 0 { 1.0 } else { 2.0 };
            (cost, 1 - parity)
        }
    }
}

/// Admissible + consistent heuristic from `c` to `goal` under `rule`.
fn heuristic(rule: DiagonalRule, c: Cell, goal: Cell) -> f64 {
    let di = (goal.0 - c.0).abs();
    let dj = (goal.1 - c.1).abs();
    let (dmax, dmin) = (di.max(dj) as f64, di.min(dj) as f64);
    match rule {
        // Alternating's optimistic bound assumes every diagonal is cheap (cost 1) → Chebyshev.
        DiagonalRule::Chebyshev | DiagonalRule::Alternating => dmax,
        DiagonalRule::Manhattan => (di + dj) as f64,
        DiagonalRule::Euclidean => (dmax - dmin) + std::f64::consts::SQRT_2 * dmin,
    }
}

/// A* over one leg `start → goal`. Node = `(cell, parity)`; goal is any node with `cell == goal`.
/// Returns the leg's cells (start..=goal), its cost, and the end parity (to thread into the next leg).
pub(crate) fn astar_leg(
    grid: &PathGrid,
    start: Cell,
    goal: Cell,
    start_parity: u8,
) -> Result<(Vec<Cell>, f64, u8), PathFail> {
    if start == goal {
        return Ok((vec![start], 0.0, start_parity));
    }
    let mut g_score: HashMap<(Cell, u8), f64> = HashMap::new();
    let mut came_from: HashMap<(Cell, u8), (Cell, u8)> = HashMap::new();
    let mut open = BinaryHeap::new();
    g_score.insert((start, start_parity), 0.0);
    open.push(QNode { f: heuristic(grid.rule, start, goal), cell: start, parity: start_parity });

    let dirs = [
        (1, 0), (-1, 0), (0, 1), (0, -1),
        (1, 1), (1, -1), (-1, 1), (-1, -1),
    ];
    let mut expansions = 0usize;

    while let Some(QNode { cell, parity, .. }) = open.pop() {
        let g = *g_score.get(&(cell, parity)).unwrap_or(&f64::INFINITY);
        if cell == goal {
            // Reconstruct start..=goal.
            let mut path = vec![cell];
            let mut node = (cell, parity);
            while let Some(&prev) = came_from.get(&node) {
                path.push(prev.0);
                node = prev;
            }
            path.reverse();
            return Ok((path, g, parity));
        }
        expansions += 1;
        if expansions > MAX_PATH_NODES {
            return Err(PathFail::Exceeded);
        }
        for (di, dj) in dirs {
            let next = (cell.0 + di, cell.1 + dj);
            if !cell_enterable(grid, cell, next) {
                continue;
            }
            let (sc, next_parity) = step_cost(grid.rule, di, dj, parity);
            let tentative = g + sc;
            let key = (next, next_parity);
            if tentative < *g_score.get(&key).unwrap_or(&f64::INFINITY) {
                came_from.insert(key, (cell, parity));
                g_score.insert(key, tentative);
                open.push(QNode {
                    f: tentative + heuristic(grid.rule, next, goal),
                    cell: next,
                    parity: next_parity,
                });
            }
        }
    }
    Err(PathFail::Unreachable)
}
```

> A popped node whose `cell == goal` is optimal under a consistent heuristic; non-goal pops re-relax neighbors idempotently (a stale duplicate heap entry's `tentative < g_score` check fails, so it's a cheap no-op). Do NOT add lazy-deletion bookkeeping unless a test shows a regression.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat scene::pathfinding::astar_tests`
Expected: PASS (6 tests).

- [ ] **Step 5: Lint**

Run: `cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/pathfinding.rs
git commit -m "feat(m10e-6): astar_leg (king-moves, 4 diagonal rules, 5-10-5 parity, admissible heuristics, node cap)"
```

---

### Task 5: Multi-leg `find` (validation, parity carry, cost sum, point output)

The public pure entry: validate the request, build the search window, run `astar_leg` per leg threading parity, sum cost, and emit cell-center scene points (de-duping shared leg endpoints).

**Files:**
- Modify: `src/server/src/scene/pathfinding.rs`
- Test: `src/server/src/scene/pathfinding.rs` (inline)

**Interfaces:**
- Consumes: `astar_leg`, `PathGrid`, `PathFail`, `cell_center`, `DiagonalRule` (Tasks 3-4).
- Produces:
  - `pub(crate) const MAX_WAYPOINTS: usize = 32;`
  - `pub(crate) const MAX_FOOTPRINT_CELLS: f64 = 64.0;`
  - `pub fn find(start: vision::P, waypoints: &[vision::P], footprint_radius: f64, cell: f64, rule: DiagonalRule, walls: &[vision::Seg], mask: Option<&BTreeSet<Cell>>) -> Result<(Vec<vision::P>, f64), PathFail>` (path = cell-center scene points incl. start & goal; cost in cells)

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod find_tests {
    use super::*;
    use crate::scene::vision::Seg;

    const NO_WALLS: [Seg; 0] = [];

    #[test]
    fn empty_waypoints_is_invalid() {
        let r = find((50.0, 50.0), &[], 0.1, 100.0, DiagonalRule::Chebyshev, &NO_WALLS, None);
        assert_eq!(r, Err(PathFail::Invalid));
    }

    #[test]
    fn nonfinite_or_bad_footprint_is_invalid() {
        assert_eq!(
            find((f64::NAN, 0.0), &[(150.0, 50.0)], 0.1, 100.0, DiagonalRule::Chebyshev, &NO_WALLS, None),
            Err(PathFail::Invalid)
        );
        assert_eq!(
            find((50.0, 50.0), &[(150.0, 50.0)], -1.0, 100.0, DiagonalRule::Chebyshev, &NO_WALLS, None),
            Err(PathFail::Invalid)
        );
        assert_eq!(
            find((50.0, 50.0), &[(150.0, 50.0)], 0.1, 0.0, DiagonalRule::Chebyshev, &NO_WALLS, None),
            Err(PathFail::Invalid)
        );
    }

    #[test]
    fn straight_route_returns_cell_centers_and_cost() {
        // (50,50)->(250,50): cells (0,0)->(2,0), 2 chebyshev steps. Points = centers of (0,0),(1,0),(2,0).
        let (path, cost) =
            find((50.0, 50.0), &[(250.0, 50.0)], 0.1, 100.0, DiagonalRule::Chebyshev, &NO_WALLS, None).unwrap();
        assert!((cost - 2.0).abs() < 1e-9);
        assert_eq!(path.first(), Some(&(50.0, 50.0)));
        assert_eq!(path.last(), Some(&(250.0, 50.0)));
        assert_eq!(path.len(), 3);
    }

    #[test]
    fn waypoint_legs_sum_cost_and_carry_alternating_parity() {
        // Leg A: (0,0)->(1,1) one diagonal (alternating cost 1, end parity 1).
        // Leg B: (1,1)->(2,2) one diagonal from parity 1 (cost 2). Total 3, not 1+1.
        let start = (50.0, 50.0);
        let wp = (150.0, 150.0);
        let goal = (250.0, 250.0);
        let (_p, cost) =
            find(start, &[wp, goal], 0.1, 100.0, DiagonalRule::Alternating, &NO_WALLS, None).unwrap();
        assert!((cost - 3.0).abs() < 1e-9, "parity carries across the waypoint (1 + 2)");
    }

    #[test]
    fn too_many_waypoints_is_invalid() {
        let wps: Vec<vision::P> = (0..(MAX_WAYPOINTS + 1)).map(|i| (i as f64 * 100.0 + 50.0, 50.0)).collect();
        assert_eq!(
            find((50.0, 50.0), &wps, 0.1, 100.0, DiagonalRule::Chebyshev, &NO_WALLS, None),
            Err(PathFail::Invalid)
        );
    }

    #[test]
    fn empty_mask_makes_a_nongm_route_unreachable() {
        let mask = BTreeSet::new();
        assert_eq!(
            find((50.0, 50.0), &[(250.0, 50.0)], 0.1, 100.0, DiagonalRule::Chebyshev, &NO_WALLS, Some(&mask)),
            Err(PathFail::Unreachable)
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat scene::pathfinding::find_tests`
Expected: FAIL — `find`/`MAX_WAYPOINTS`/`MAX_FOOTPRINT_CELLS` not defined.

- [ ] **Step 3: Implement**

Append to `pathfinding.rs`:

```rust
/// Max ordered waypoints (incl. goal) per request (DoS guard).
pub(crate) const MAX_WAYPOINTS: usize = 32;
/// Max footprint radius in cells (DoS guard on the per-cell footprint scan).
pub(crate) const MAX_FOOTPRINT_CELLS: f64 = 64.0;
/// Search-window margin (cells) added around the point/wall AABB so detours around walls stay reachable.
const WINDOW_MARGIN: i32 = 8;

fn to_cell(p: vision::P, cell: f64) -> Cell {
    ((p.0 / cell).floor() as i32, (p.1 / cell).floor() as i32)
}

/// Plan a footprint-clear, mask-bounded route `start → waypoints[0] → … → waypoints[last]`.
/// `waypoints` is the full ordered leg list whose LAST element is the goal (empty ⇒ `Invalid`).
/// Returns cell-center scene points (incl. start and goal) and the total cost in cells.
pub fn find(
    start: vision::P,
    waypoints: &[vision::P],
    footprint_radius: f64,
    cell: f64,
    rule: DiagonalRule,
    walls: &[vision::Seg],
    mask: Option<&BTreeSet<Cell>>,
) -> Result<(Vec<vision::P>, f64), PathFail> {
    // Validation (fail-closed).
    if waypoints.is_empty() || waypoints.len() > MAX_WAYPOINTS {
        return Err(PathFail::Invalid);
    }
    if !footprint_radius.is_finite() || footprint_radius < 0.0 || footprint_radius > MAX_FOOTPRINT_CELLS {
        return Err(PathFail::Invalid);
    }
    if !cell.is_finite() || cell <= 0.0 {
        return Err(PathFail::Invalid);
    }
    let finite = |p: &vision::P| p.0.is_finite() && p.1.is_finite();
    if !finite(&start) || !waypoints.iter().all(finite) {
        return Err(PathFail::Invalid);
    }

    // Search window = AABB of {start ∪ waypoints ∪ wall endpoints} in cells, expanded by margin.
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let mut acc = |x: f64, y: f64| {
        minx = minx.min(x);
        miny = miny.min(y);
        maxx = maxx.max(x);
        maxy = maxy.max(y);
    };
    acc(start.0, start.1);
    for p in waypoints {
        acc(p.0, p.1);
    }
    for w in walls {
        acc(w.a.0, w.a.1);
        acc(w.b.0, w.b.1);
    }
    let window = (
        (minx / cell).floor() as i32 - WINDOW_MARGIN,
        (miny / cell).floor() as i32 - WINDOW_MARGIN,
        (maxx / cell).floor() as i32 + WINDOW_MARGIN,
        (maxy / cell).floor() as i32 + WINDOW_MARGIN,
    );

    let grid = PathGrid {
        cell,
        rule,
        footprint_radius_cells: footprint_radius,
        walls,
        mask,
        window,
    };

    // Run each leg, threading parity; concat cells de-duping the shared boundary cell.
    let mut cells: Vec<Cell> = Vec::new();
    let mut total = 0.0;
    let mut parity = 0u8;
    let mut from = to_cell(start, cell);
    for wp in waypoints {
        let goal = to_cell(*wp, cell);
        let (leg, cost, end_parity) = astar_leg(&grid, from, goal, parity)?;
        total += cost;
        parity = end_parity;
        if cells.is_empty() {
            cells.extend(leg);
        } else {
            // Skip the first cell of subsequent legs (== last cell of the previous leg).
            cells.extend(leg.into_iter().skip(1));
        }
        from = goal;
    }

    let path: Vec<vision::P> = cells.into_iter().map(|c| cell_center(c, cell)).collect();
    Ok((path, total))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat scene::pathfinding::find_tests`
Expected: PASS (6 tests).

- [ ] **Step 5: Full pathfinding module + lint**

Run: `cargo test -p shadowcat scene::pathfinding && cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/pathfinding.rs
git commit -m "feat(m10e-6): multi-leg find (validation, window, parity carry, cost sum, cell-center output)"
```

---

### Task 6: `SceneEcs::pathfind` assembly

Assemble the pure search's inputs from the ECS: resolve rule + movement restriction, build the mask (reusing `visible_cells`, unioning `explored` for `Revealed`), gather `move_walls`, dispatch by GM/restriction. This is the one place the pathfinder touches scene state.

**Files:**
- Modify: `src/server/src/scene/mod.rs` (new method on `SceneEcs`)
- Test: `src/server/src/scene/mod.rs` (inline)

**Interfaces:**
- Consumes: `resolved_diagonal_rule`, `resolve_scene`, `visible_cells`, `move_walls`, `scene_grid_sizes`, `MovementRestriction`, `pathfinding::{find, PathFail, Cell}`, `explored::ExploredSet::cells`.
- Produces: `pub fn pathfind(&self, user: Uuid, scene: Uuid, start: (f64,f64), waypoints: &[(f64,f64)], footprint_radius: f64, is_gm: bool, explored: Option<&crate::scene::explored::ExploredSet>) -> Result<(Vec<(f64,f64)>, f64), pathfinding::PathFail>`

- [ ] **Step 1: Write the failing tests**

Reuse the M10e-4 movement-scene helpers (a world-settings doc + scene + player token + optional light). Add a couple targeted cases:

```rust
#[test]
fn pathfind_gm_unconstrained_routes_without_a_mask() {
    // GM (is_gm=true): no mask; an open scene routes start→goal at chebyshev cost.
    let (ecs, _user, scene) = scene_with_lit_player_token(); // existing M10e-4 helper
    let r = ecs.pathfind(Uuid::from_u128(1), scene, (50.0, 50.0), &[(250.0, 50.0)], 0.1, true, None);
    let (path, cost) = r.expect("GM route");
    assert!((cost - 2.0).abs() < 1e-9);
    assert_eq!(path.last(), Some(&(250.0, 50.0)));
}

#[test]
fn pathfind_nongm_visible_is_bounded_by_the_mask() {
    // Non-GM under movementRestriction "visible": a goal outside the lit mask is Unreachable.
    let (ecs, user, scene) = scene_with_lit_player_token();
    let lenient = ecs.resolve_scene(scene).partial_cell_leniency;
    let mask = ecs.visible_cells(user, scene, lenient);
    assert!(!mask.is_empty(), "the lit token has a non-empty mask");
    // A far goal well outside the lit radius → Unreachable.
    let far = ecs.pathfind(user, scene, (50.0, 50.0), &[(5000.0, 5000.0)], 0.1, false, None);
    assert_eq!(far, Err(crate::scene::pathfinding::PathFail::Unreachable));
}

#[test]
fn pathfind_revealed_unions_explored_memory() {
    // movementRestriction "revealed": an explored corridor covering start..goal makes an otherwise-unlit
    // goal routable.
    let (ecs, user, scene) = scene_revealed_player_token(); // helper: world-settings movementRestriction "revealed"
    let cell = 100.0;
    let mut explored = crate::scene::explored::ExploredSet::new();
    // Mark cells (0,0)..(3,0) as explored (a straight corridor).
    explored.mark_polygons(
        &[vec![0.0, 0.0, 4.0 * cell, 0.0, 4.0 * cell, cell, 0.0, cell]],
        cell,
    );
    let r = ecs.pathfind(user, scene, (50.0, 50.0), &[(350.0, 50.0)], 0.1, false, Some(&explored));
    assert!(r.is_ok(), "explored corridor makes the goal routable under revealed");
}
```

> If `scene_revealed_player_token` doesn't exist, build it like `scene_with_lit_player_token` but set the world-settings `movementRestriction` to `"revealed"` and omit the light (so the route depends on explored memory, not current light).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat pathfind_`
Expected: FAIL — `no method pathfind`.

- [ ] **Step 3: Implement**

On `impl SceneEcs` in `mod.rs`:

```rust
/// Plan a route for `user`'s token in `scene` (M10e-6). Reuses the M10e-4 `visible_cells` mask so
/// the preview agrees with the movement gate (spec §13). `is_gm`/`unrestricted` ⇒ no mask;
/// `visible` ⇒ `visible_cells`; `revealed` ⇒ `visible_cells ∪ explored`. `explored` is the caller's
/// pre-fetched `ExploredSet` (only consulted under `revealed`; the handler fetches it off the lock).
pub fn pathfind(
    &self,
    user: Uuid,
    scene: Uuid,
    start: (f64, f64),
    waypoints: &[(f64, f64)],
    footprint_radius: f64,
    is_gm: bool,
    explored: Option<&crate::scene::explored::ExploredSet>,
) -> Result<(Vec<(f64, f64)>, f64), pathfinding::PathFail> {
    let cell = self.scene_grid_sizes().get(&scene).copied().unwrap_or(100.0);
    let rule = self.resolved_diagonal_rule();
    let walls = self.move_walls(scene);

    // Build the per-(user,scene) mask (None ⇒ unconstrained).
    let mask: Option<std::collections::BTreeSet<pathfinding::Cell>> = if is_gm {
        None
    } else {
        let settings = self.resolve_scene(scene);
        match settings.movement_restriction {
            MovementRestriction::Unrestricted => None,
            MovementRestriction::Visible => {
                Some(self.visible_cells(user, scene, settings.partial_cell_leniency))
            }
            MovementRestriction::Revealed => {
                let mut m = self.visible_cells(user, scene, settings.partial_cell_leniency);
                if let Some(ex) = explored {
                    m.extend(ex.cells());
                }
                Some(m)
            }
        }
    };

    pathfinding::find(start, waypoints, footprint_radius, cell, rule, &walls, mask.as_ref())
}
```

> Confirm `MovementRestriction` and `pathfinding` are in scope in `mod.rs` (both local). `ExploredSet::cells()` returns an iterator of `Cell` — `m.extend(ex.cells())` unions it in.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat pathfind_`
Expected: PASS (3 tests).

- [ ] **Step 5: Full scene suite + lint**

Run: `cargo test -p shadowcat scene:: && cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "feat(m10e-6): SceneEcs::pathfind assembly (reuses visible_cells mask; unions explored for revealed)"
```

---

### Task 7: Wire frames + `conn.rs` handler

Add the `Pathfind`/`PathResult`/`PathError` frames (ts-rs exported) and the one-shot ingress handler that resolves GM status, fetches `explored` off the lock for non-GM `revealed`, calls `SceneEcs::pathfind`, and replies to the requesting connection only.

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (3 frames + round-trip test)
- Modify: `src/server/src/ws/conn.rs` (ingress handler + test)
- Test: `src/server/src/ws/protocol.rs`, `src/server/src/ws/conn.rs` (inline)

**Interfaces:**
- Consumes: `SceneEcs::pathfind`, `repo.get_explored`, `PermissionContext{user_id, world_role}`, `pathfinding::PathFail`, the egress sink (`Egress::Frame`).
- Produces (wire): `ClientMsg::Pathfind { request_id, scene, start, waypoints, footprint_radius }`; `ServerMsg::PathResult { request_id, path, cost }`, `ServerMsg::PathError { request_id, message }`. ts-rs → `src/types/generated/ClientMsg.ts`/`ServerMsg.ts`.

- [ ] **Step 1: Write the failing tests**

In `protocol.rs` tests (mirror the existing `Search` round-trip at `:312`):

```rust
#[test]
fn pathfind_frames_round_trip() {
    let req = ClientMsg::Pathfind {
        request_id: Uuid::from_u128(1),
        scene: Uuid::from_u128(2),
        start: (50.0, 50.0),
        waypoints: vec![(150.0, 50.0), (250.0, 50.0)],
        footprint_radius: 0.5,
    };
    let s = serde_json::to_string(&req).unwrap();
    assert!(s.contains("\"type\":\"pathfind\""));
    let back: ClientMsg = serde_json::from_str(&s).unwrap();
    assert!(matches!(back, ClientMsg::Pathfind { .. }));

    let ok = ServerMsg::PathResult { request_id: Uuid::from_u128(1), path: vec![(50.0, 50.0)], cost: 2.0 };
    assert!(serde_json::to_string(&ok).unwrap().contains("\"type\":\"path_result\""));
    let err = ServerMsg::PathError { request_id: Uuid::from_u128(1), message: "unreachable".into() };
    assert!(serde_json::to_string(&err).unwrap().contains("\"type\":\"path_error\""));
}
```

In `conn.rs` tests: a non-GM `Pathfind` over a lit scene returns a `PathResult` for an in-mask goal and a `PathError` for an out-of-mask goal, replied to that connection only. Use the conn-level harness the search/scene-subscribe tests use; if a full socket test is heavy, assert at the handler-helper boundary (see the Step-3 note on extracting `handle_pathfind`).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat pathfind_frames_round_trip`
Expected: FAIL — `ClientMsg::Pathfind` / `ServerMsg::PathResult` not defined.

- [ ] **Step 3: Implement the frames + handler**

In `protocol.rs`, add to `ClientMsg` (after `ScenePing`):

```rust
    /// A one-shot grid pathfinding request, correlated by `request_id`. `start`/`waypoints` are
    /// scene coords; `waypoints`' LAST element is the goal. `footprint_radius` is in grid units
    /// (cells; the client's `footprintRadius`). The route is mask-bounded for non-GM requesters.
    Pathfind {
        request_id: Uuid,
        scene: Uuid,
        start: (f64, f64),
        waypoints: Vec<(f64, f64)>,
        footprint_radius: f64,
    },
```

Add to `ServerMsg` (near `SearchError`):

```rust
    /// The route for the `Pathfind` with this `request_id`: ordered cell-center scene points
    /// (incl. start + goal) and the total cost in cells (client multiplies `grid.distance.perCell`).
    PathResult { request_id: Uuid, path: Vec<(f64, f64)>, cost: f64 },
    /// The `Pathfind` with this `request_id` failed (unreachable / invalid request / search exceeded).
    PathError { request_id: Uuid, message: String },
```

In `conn.rs`, add a `Pathfind` arm to the `ClientMsg` ingress match (mirror the one-shot `Search` arm). Resolve GM status from the connection's `PermissionContext.world_role`; for non-GM `Revealed`, fetch `get_explored` AFTER dropping the scene read guard (no lock across await); then call `scene.pathfind`:

```rust
ClientMsg::Pathfind { request_id, scene, start, waypoints, footprint_radius } => {
    let is_gm = ctx.world_role == crate::data::document::WorldRole::Gm;
    // Decide whether explored is needed (non-GM + revealed) WITHOUT holding the lock across await.
    let need_explored = !is_gm && {
        let s = room.scene.read().await;
        matches!(
            s.resolve_scene(scene).movement_restriction,
            crate::scene::MovementRestriction::Revealed
        )
    };
    let explored = if need_explored {
        match repo.get_explored(scene, ctx.user_id).await {
            Ok(Some(blob)) => Some(crate::scene::explored::ExploredSet::from_bytes(&blob)),
            _ => None, // fail closed: revealed degrades to visible-only
        }
    } else {
        None
    };
    let frame = {
        let s = room.scene.read().await;
        match s.pathfind(ctx.user_id, scene, start, &waypoints, footprint_radius, is_gm, explored.as_ref()) {
            Ok((path, cost)) => ServerMsg::PathResult { request_id, path, cost },
            Err(e) => ServerMsg::PathError {
                request_id,
                message: match e {
                    crate::scene::pathfinding::PathFail::Invalid => "invalid request",
                    crate::scene::pathfinding::PathFail::Unreachable => "unreachable",
                    crate::scene::pathfinding::PathFail::Exceeded => "search exceeded",
                }
                .to_string(),
            },
        }
    };
    let _ = egress.send(Egress::Frame(std::sync::Arc::new(frame))).await;
}
```

> Match the EXACT local names the search/scene-subscribe arms use for the room handle (`room` vs `self.room`), the egress sender (`egress`/`tx`), the `PermissionContext` binding (`ctx`), and the `Egress::Frame(Arc::new(...))` wrapper. `pathfinding` must be a `pub(crate) mod` (Task 2) so `conn.rs` can name `PathFail`. If the search arm extracts a free `handle_*` helper, mirror that shape with a `handle_pathfind` for direct unit-testing.

- [ ] **Step 4: Regenerate ts-rs types + run tests**

Run: `cargo test -p shadowcat pathfind`
Expected: PASS (protocol round-trip + conn handler). The ts-rs export tests regenerate `ClientMsg.ts`/`ServerMsg.ts`; confirm `src/types/generated/ClientMsg.ts` now has a `pathfind` member and `ServerMsg.ts` has `path_result`/`path_error`.

- [ ] **Step 5: fmt + clippy + full server suite**

Run: `cargo fmt && cargo test -p shadowcat && cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/ws/protocol.rs src/server/src/ws/conn.rs src/types/generated/
git commit -m "feat(m10e-6): Pathfind/PathResult/PathError frames + one-shot conn handler (GM + revealed explored off-lock)"
```

---

### Task 8: Client wire schema + `WsClient.pathfind` + AppContext seam

Mirror the server frames in the client Zod/types, add `pathfind` to `WsClient` (mirroring `search`), and expose it through `AppContext.pathfind` (wired via `WorldSession` + `Table.svelte`).

**Files:**
- Modify: `src/client/core/src/wire.ts` (Zod for the 3 frames + `ClientMsg` member + parse)
- Modify: `src/client/core/src/ws-client.ts` (`pathfind` method + `pending` correlation reuse)
- Modify: `src/client/ui-kit/src/appContext.ts` (`pathfind` field)
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts` (`pathfind` impl on the session)
- Modify: `src/client/shell/src/lib/Table.svelte` (wire `pathfind` into `setAppContext`)
- Test: `src/client/core/src/wire.test.ts`, `src/client/core/src/ws-client.test.ts`

**Interfaces:**
- Consumes (server, Task 7): `pathfind` ClientMsg; `path_result`/`path_error` ServerMsg.
- Produces:
  - `wire.ts`: `path_result`/`path_error` added to `ServerMsgSchema`; `{ type: "pathfind"; request_id; scene; start; waypoints; footprint_radius }` in `ClientMsg`.
  - `ws-client.ts`: `pathfind(scene: string, start: [number, number], waypoints: [number, number][], footprintRadius: number, opts?: { timeoutMs?: number }): Promise<{ path: [number, number][]; cost: number }>`
  - `appContext.ts`: `pathfind: (scene, start, waypoints, footprintRadius) => Promise<{ path: [number, number][]; cost: number }>`

- [ ] **Step 1: Write the failing tests**

In `wire.test.ts` (mirror the existing search-frame parse test):

```ts
it("parses path_result and path_error server frames", () => {
  const ok = parseServerMsg({
    type: "path_result",
    request_id: "00000000-0000-0000-0000-000000000001",
    path: [[50, 50], [150, 50]],
    cost: 2,
  });
  expect(ok.type).toBe("path_result");
  const err = parseServerMsg({
    type: "path_error",
    request_id: "00000000-0000-0000-0000-000000000001",
    message: "unreachable",
  });
  expect(err.type).toBe("path_error");
});
```

In `ws-client.test.ts` (mirror the search resolve test with a mock transport): a `pathfind(...)` call sends a `pathfind` frame and resolves when a matching `path_result` arrives; rejects on `path_error`.

```ts
it("pathfind resolves on path_result and rejects on path_error", async () => {
  const { client, transport } = makeTestClient(); // existing helper used by the search test
  const p = client.pathfind("scene-1", [50, 50], [[250, 50]], 0.5);
  const sent = transport.lastSent(); // the {type:"pathfind", request_id, ...} frame
  expect(sent.type).toBe("pathfind");
  transport.deliver({ type: "path_result", request_id: sent.request_id, path: [[50, 50], [250, 50]], cost: 2 });
  await expect(p).resolves.toEqual({ path: [[50, 50], [250, 50]], cost: 2 });

  const p2 = client.pathfind("scene-1", [50, 50], [[9999, 9999]], 0.5);
  const sent2 = transport.lastSent();
  transport.deliver({ type: "path_error", request_id: sent2.request_id, message: "unreachable" });
  await expect(p2).rejects.toThrow("unreachable");
});
```

> Reuse whatever mock-transport helper the existing `search` ws-client test uses; match its API (`lastSent`/`deliver` are illustrative names).

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- wire` and `pnpm --filter @shadowcat/core test -- ws-client`
Expected: FAIL — schema members / `pathfind` method missing.

- [ ] **Step 3: Implement**

In `wire.ts`, add to `ServerMsgSchema` (the discriminated union):

```ts
  z.object({
    type: z.literal("path_result"),
    request_id: z.string(),
    path: z.array(z.tuple([z.number(), z.number()])),
    cost: z.number(),
  }),
  z.object({
    type: z.literal("path_error"),
    request_id: z.string(),
    message: z.string(),
  }),
```

Add to the `ClientMsg` union:

```ts
  | {
      type: "pathfind";
      request_id: string;
      scene: string;
      start: [number, number];
      waypoints: [number, number][];
      footprint_radius: number;
    }
```

In `ws-client.ts`, add (mirroring `search` at `:323-348`, reusing the `pending` map + adding `path_result`/`path_error` cases in the message handler near `:256-263`):

```ts
pathfind(
  scene: string,
  start: [number, number],
  waypoints: [number, number][],
  footprintRadius: number,
  opts: { timeoutMs?: number } = {},
): Promise<{ path: [number, number][]; cost: number }> {
  const request_id = crypto.randomUUID();
  const timeoutMs = opts.timeoutMs ?? 10_000;
  return new Promise((resolve, reject) => {
    if (!this.transport) {
      reject(new Error("not connected"));
      return;
    }
    const timer = setTimeout(() => {
      this.pending.delete(request_id);
      reject(new Error("pathfind request timeout"));
    }, timeoutMs);
    this.pending.set(request_id, { resolve, reject, timer });
    this.send({ type: "pathfind", request_id, scene, start, waypoints, footprint_radius: footprintRadius });
  });
}
```

In the server-message handler, add cases alongside `search_result`/`search_error` that look up `this.pending` by `request_id`, clear the timer, and `resolve({ path: msg.path, cost: msg.cost })` for `path_result` / `reject(new Error(msg.message))` for `path_error`. If `pending` entries are typed to `SearchPage`, widen the stored resolve type to a union (or `unknown` cast at the call site) — match the existing pattern.

In `appContext.ts`, add to the interface:

```ts
  pathfind: (
    scene: string,
    start: [number, number],
    waypoints: [number, number][],
    footprintRadius: number,
  ) => Promise<{ path: [number, number][]; cost: number }>;
```

In `worldSession.svelte.ts`, add (mirroring `sendPing`/`subscribeScene`):

```ts
pathfind(
  scene: string,
  start: [number, number],
  waypoints: [number, number][],
  footprintRadius: number,
): Promise<{ path: [number, number][]; cost: number }> {
  return this.#ws.pathfind(scene, start, waypoints, footprintRadius);
}
```

In `Table.svelte`'s `setAppContext({...})`, add: `pathfind: (s, st, wp, fr) => session.pathfind(s, st, wp, fr),`.

- [ ] **Step 4: Run tests + typecheck**

Run: `pnpm --filter @shadowcat/core test && pnpm -r typecheck`
Expected: PASS. The final `typecheck` is the real gate — Vitest's esbuild strips types (`vitest-skips-typecheck-in-sdd`).

- [ ] **Step 5: Lint**

Run: `pnpm lint`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/client/core/src/wire.ts src/client/core/src/ws-client.ts src/client/ui-kit/src/appContext.ts src/client/shell/src/lib/worldSession.svelte.ts src/client/shell/src/lib/Table.svelte src/client/core/src/wire.test.ts src/client/core/src/ws-client.test.ts
git commit -m "feat(m10e-6): client pathfind frame schema + WsClient.pathfind + AppContext seam"
```

---

### Task 9: Measure-tool route mode (waypoints + preview + budget)

Extend the measure tool into a waypoint router: click-to-add waypoints from the selected token, request a path on change, render the route via `previewOverlay`, and show a movement-budget label (`cost × grid.distance.perCell + unit`). Clears on tool swap/release (mid-gesture-clear gotcha).

**Files:**
- Modify: `src/modules/scene-tools/src/controller.svelte.ts` (the measure tool factory + `ToolContext`)
- Test: `src/modules/scene-tools/src/controller.test.ts` (or the existing scene-tools test file)

**Interfaces:**
- Consumes: `ToolContext` (`scene`, `documents`, + a new `pathfind` plumbed from the app context), `ctx.scene.previewOverlay`/`clearOverlay`/`snap`/`drawMeasure`/`clearMeasure`, `footprintRadius` + `resolveTokenActor` (from `@shadowcat/core` `actor.ts`), the scene system `grid.distance`.
- Produces: the measure tool, when a token is selected, accumulates waypoints and renders a routed polyline + budget; with no selection it falls back to the current anchor→point measure behavior.

- [ ] **Step 1: Write the failing test**

```ts
it("measure tool routes via pathfind for the selected token and previews the path", async () => {
  const overlay: unknown[] = [];
  let label = "";
  const ctx = makeToolContext({
    selectedTokenId: "tok-1", // tok-1 at (50,50); scene grid.distance { perCell: 5, unit: "ft" }
    scene: {
      previewOverlay: (s: unknown[]) => overlay.push(...s),
      clearOverlay: () => (overlay.length = 0),
      snap: (p: { x: number; y: number }) => p,
      gridDistance: () => 1,
      drawMeasure: (_f: unknown, _t: unknown, l: string) => (label = l),
      clearMeasure: () => (label = ""),
    },
    pathfind: async () => ({ path: [[50, 50], [150, 50]] as [number, number][], cost: 2 }),
  });
  const tool = makeMeasureTool(ctx);
  tool.onPointerDown({ x: 50, y: 50 });
  tool.onPointerMove({ x: 150, y: 50 });
  await flush(); // allow the async pathfind to resolve
  expect(overlay.length).toBeGreaterThan(0); // a routed polyline was previewed
  expect(label).toContain("10 ft"); // budget = cost(2) × perCell(5)
});
```

> Match the actual `ToolContext` shape and the measure tool's label channel (`drawMeasure(from, to, label)` today). `makeToolContext`/`flush` are the existing scene-tools test helpers — extend `makeToolContext` to carry `selectedTokenId` + `pathfind`.

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/module-scene-tools test -- measure`
Expected: FAIL — the measure tool does not call `pathfind`/render a route.

- [ ] **Step 3: Implement**

In `controller.svelte.ts`, extend `makeMeasureTool` and the `ToolContext` type (add `pathfind` plumbed from the app context). When a token is selected:
- `onPointerDown`: start the waypoint list at the selected token's center (resolve via `resolveTokenActor` + the token doc position); snap each clicked point with `ctx.scene.snap`.
- `onPointerMove`: treat the moving point as the provisional goal; debounce/coalesce a `ctx.pathfind(sceneId, start, [...waypoints, goal], footprintRadius(eff))` call (mirror the move tool's coalesced sends); on resolve, `ctx.scene.previewOverlay([{ points: flattenPath(path), closed: false, stroke: {...}, fill: null }])` and set the budget label via `ctx.scene.drawMeasure(start, goal, `${cost * perCell} ${unit}`)` (read `scene.system.grid.distance ?? {perCell:5,unit:"ft"}`). On a rejected promise (`path_error`), `clearOverlay()` and show a "no route" label.
- Click adds a waypoint (push the current point); the commit gesture (double-click / Enter) finalizes; M10e-6 produces only the route + preview — the actual move stays the existing optimistic path (M9 + M10e-4 gate).
- `onPointerUp`/tool swap: `ctx.scene.clearOverlay()` + `ctx.scene.clearMeasure()` and reset waypoints (the mid-gesture-clear gotcha).
- With NO token selected: keep the current anchor→point measure behavior unchanged.

> Import `footprintRadius` + `resolveTokenActor` from `@shadowcat/core` (`actor.ts`); resolve the selected token's `EffectiveActor`. Reuse the draw/template multi-point accumulation (`controller.svelte.ts:211-318`).

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/module-scene-tools test -- measure`
Expected: PASS.

- [ ] **Step 5: Typecheck + lint + full scene-tools suite**

Run: `pnpm --filter @shadowcat/module-scene-tools test && pnpm -r typecheck && pnpm lint`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/modules/scene-tools/
git commit -m "feat(m10e-6): measure-tool route mode (waypoints + pathfind preview + movement-budget readout)"
```

---

### Task 10: Ruler `alternating` (5-10-5) distance rule

Teach `grid.distance()` the `alternating` rule for square grids alongside `chebyshev`, selecting it from the scene's resolved `pathfinding.diagonalRule`.

**Files:**
- Modify: `src/client/render/src/grid.ts` (`distance()` + a rule input on the grid spec)
- Test: `src/client/render/src/grid.test.ts`

**Interfaces:**
- Consumes: the scene's `pathfinding.diagonalRule` (read by the caller; `distance` selects on `this.spec.diagonalRule`).
- Produces: `distance(a, b)` returns the 5-10-5 cost for square grids under `alternating` (diagonals cost 1,2,1,2…), unchanged chebyshev otherwise; hex untouched.

- [ ] **Step 1: Write the failing test**

In `grid.test.ts`:

```ts
it("alternating (5-10-5) costs diagonals 1,2,1,2 for square grids", () => {
  const g = makeGrid({ kind: "square", size: 100, diagonalRule: "alternating" }); // extend the test factory
  // 3 diagonal steps from origin: 1 + 2 + 1 = 4.
  expect(g.distance({ x: 50, y: 50 }, { x: 350, y: 350 })).toBe(4);
  // 1 diagonal + 1 orthogonal: diagonal(1) + orth(1) = 2.
  expect(g.distance({ x: 50, y: 50 }, { x: 250, y: 150 })).toBe(2);
});

it("chebyshev remains 1-per-diagonal (default)", () => {
  const g = makeGrid({ kind: "square", size: 100, diagonalRule: "chebyshev" });
  expect(g.distance({ x: 50, y: 50 }, { x: 350, y: 350 })).toBe(3);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/render test -- grid`
Expected: FAIL — no alternating rule.

- [ ] **Step 3: Implement**

In `grid.ts`, thread the rule (add `diagonalRule?: "chebyshev" | "manhattan" | "euclidean" | "alternating"` to the grid spec; default `"chebyshev"`). For square grids:

```ts
distance(a: Point, b: Point): number {
  const ca = this.cellOf(a);
  const cb = this.cellOf(b);
  const dCol = Math.abs(cb.col - ca.col);
  const dRow = Math.abs(cb.row - ca.row);
  if (this.spec.kind !== "square") {
    const sCol = cb.col - ca.col;
    const sRow = cb.row - ca.row;
    return (Math.abs(sCol) + Math.abs(sRow) + Math.abs(sCol + sRow)) / 2;
  }
  const dmax = Math.max(dCol, dRow);
  const dmin = Math.min(dCol, dRow);
  switch (this.spec.diagonalRule ?? "chebyshev") {
    case "manhattan": return dCol + dRow;
    case "euclidean": return (dmax - dmin) + Math.SQRT2 * dmin;
    case "alternating": return (dmax - dmin) + dmin + Math.floor(dmin / 2); // diagonals 1,2,1,2…
    default: return dmax; // chebyshev
  }
}
```

> `alternating`: `dmin` diagonals cost `dmin + floor(dmin/2)` (1,2,1,2…), plus `(dmax − dmin)` straight steps. The caller reads the scene's resolved `pathfinding.diagonalRule` (world-settings) and passes it into the grid spec, the same way `size` is read (`controller.svelte.ts:33-38`).

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/render test -- grid`
Expected: PASS.

- [ ] **Step 5: Typecheck + lint**

Run: `pnpm --filter @shadowcat/render test && pnpm -r typecheck && pnpm lint`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/client/render/src/grid.ts src/client/render/src/grid.test.ts
git commit -m "feat(m10e-6): ruler alternating (5-10-5) diagonal rule for square grids"
```

---

### Task 11: Closeout — docs + skill sync (non-TDD)

The cadence's documentation-sync + reviewed-skill-update gates. Do this after the whole-branch buddy-check (so it records verified reality); the content is specified here.

**Files:**
- Modify: `docs/PLAN.md`, `docs/TODO.md`, `docs/POST_WORK_FINDINGS.md`, `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md`

- [ ] **Step 1: `docs/PLAN.md`** — move M10e-6 to completed (point to this plan + the spec); note M10e is now e-1..e-4 + e-6 done, **e-5 (movement animation) the only M10e remainder**, then M10f (continuous) + M10g (regions). Update the "Next =" line.

- [ ] **Step 2: `docs/TODO.md`** — log the deferred items surfaced here:

```markdown
- Server / pathfinding: `cost_field` ships inert (uniform weight 1) in M10e-6; wire per-cell
  weights from the `region` doc_type in M10g (weighted/impassable regions).
- Client / scene-tools: the route preview re-requests on waypoint change with a fixed debounce;
  if profiling shows chattiness on fast drags, switch to leading-edge + max-staleness
  (`debounce-leading-edge-not-trailing-rearm`). Inert until measured.
- Pathfinding: hex-grid pathfinding (M10e-6 is square-only); the ruler's hex distance is untouched.
```

- [ ] **Step 3: `docs/POST_WORK_FINDINGS.md`** — log complications found during implementation (NOT a to-do list). At minimum:

```markdown
- Title: Route stricter than the authoritative gate (footprint vs center-based).
  Summary: The M10e-6 preview enforces full geometric footprint clearance; the authoritative move
  gate (M9/M10e-4) stays center-based (parent §14). A wide token can therefore be dragged (gate
  allows, center-based) along a path the router refuses to preview through a narrow gap. This is the
  intended asymmetry (route ⊆ gate-allowed keeps the preview from suggesting a rejected move), not a
  bug. Status: Recorded; revisit when footprint-aware blocking lands.
```

- [ ] **Step 4: Update `shadowcat-codebase-scene-rendering` skill** — add the pathfinding seam to *Key files & seams* and *Hard invariants*: `scene/pathfinding.rs` (pure grid A*: `DiagonalRule`, `PathGrid`, `cell_enterable` [full geometric footprint-disc clearance + footprint-cell mask + center-step], `astar_leg` [king-moves, 4 rules, 5-10-5 parity in the node, admissible heuristics, node cap], `find` [validation, window, parity carry, cost sum]); `SceneEcs::pathfind` reuses `visible_cells` (the SAME mask as the M10e-4 gate — §13), unions `explored` for `revealed`, GM unconstrained; `move_walls` accessor; `resolved_diagonal_rule` (world-only); `Pathfind`/`PathResult`/`PathError` frames (one-shot to requester, `get_explored` off-lock). Note the invariant: the route is footprint-stricter than the center-based authoritative gate but shares the mask.

- [ ] **Step 5: Reviewed skill-update gate** — dispatch `shadowcat-spec-reviewer` on the skill diff to confirm it accurately captures the change (no omission/drift/broken pointer). Record PASS.

- [ ] **Step 6: Commit**

```bash
git add docs/ .claude/skills/shadowcat-codebase-scene-rendering/
git commit -m "docs(m10e-6): PLAN/TODO/POST_WORK + scene-rendering skill sync for the grid A* pathfinder"
```

---

## Self-Review (completed during authoring)

**Spec coverage (spec §3-§6 + §9):**
- Seam `find(start,goal,waypoints,footprint,costField,model)` → Task 5 `find` (goal = `waypoints.last()`; `cost_field` inert). ✓
- Frames `Pathfind`/`PathResult`/`PathError`, one-shot to requester, ts-rs → Task 7. ✓
- King-move A*, 4 diagonal rules + admissible heuristics → Task 4 (`step_cost`/`heuristic`). ✓
- 5-10-5 parity in the node, carried across waypoint legs → Task 4 (node `(cell,parity)`) + Task 5 (parity thread). ✓
- Full geometric footprint clearance (disc vs walls) + footprint-cell mask + step test → Task 3 `cell_enterable`. ✓
- Same mask as the gate (`visible_cells`; revealed ∪ explored; GM none) → Task 6 `pathfind`. ✓
- Search bounds (waypoints/footprint/coords/window/node-cap → PathError) → Tasks 4-5 + Task 7 mapping. ✓
- Client measure-tool route mode + preview + budget → Task 9. ✓
- Client `pathfind` mirror + AppContext seam → Task 8. ✓
- Ruler `alternating` → Task 10. ✓
- Diagonal-rule resolver (server mirrors client) → Task 2. ✓
- Docs + skill sync → Task 11. ✓

**Type consistency:** `Cell=(i32,i32)`, `vision::{P,Seg}` uniform; `PathGrid`/`cell_enterable` (Task 3) ↔ `astar_leg` (Task 4) ↔ `find` (Task 5) ↔ `SceneEcs::pathfind` (Task 6); `PathFail::{Invalid,Unreachable,Exceeded}` (Task 4) ↔ handler message mapping (Task 7); frame field names/types (`start:(f64,f64)`, `waypoints:Vec<(f64,f64)>`, `footprint_radius:f64`, `path:Vec<(f64,f64)>`, `cost:f64`) identical server (Task 7) ↔ client Zod/`pathfind` (Task 8). `footprintRadius`/`resolveTokenActor` are existing client exports (Task 9). Consistent.

**Placeholder scan:** no TBD/"handle edge cases"/"similar to Task N"; every code step shows code; test bodies concrete. Client steps that adapt to existing helper names flag the exact file:line to match rather than guess.

## Buddy-check directives

This checkpoint is **security-sensitive**: the per-`(user,scene)` visibility mask now also bounds the routed path, so a drift between the pathfinder's mask test and the egress/gate mask, or a footprint/leniency error, is a confidentiality bug (a preview could leak hidden geometry) — and the A* + 5-10-5 parity is subtle correctness. Per the M8/M9/M10 cadence and the elevated risk: after all tasks pass, run a **whole-branch two-reviewer buddy-check on Opus** (`shadowcat-spec-reviewer` + `shadowcat-code-reviewer`), reconciled to convergence. Focus the reviewers on: (1) the pathfinder consumes the **same** `visible_cells` mask as the M10e-4 gate — no forked per-cell decision (§13); (2) full geometric footprint clearance is correct (a token wider than a gap cannot route through it) and the footprint-cell mask test never lets the body extend into unseen cells; (3) 5-10-5 parity is tracked in the node and **carried across waypoint legs** (cost 1,2,1,2…, not reset per leg); (4) per-rule heuristics are admissible + consistent (no over-estimate → optimal paths); (5) fail-closed on empty mask / over-cap / invalid request / non-finite input → `PathError`, never a partial route; (6) the handler fetches `get_explored` off the scene read lock (no lock across await) and replies only to the requesting connection; (7) the route is footprint-stricter than the center-based authoritative gate but never *weaker* (route ⊆ gate-allowed). Record the outcome (and any Critical/Important fixes) before merge.
