# M10f-1 — movementModel axis + dispatch + polyanya router — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `movementModel` scene axis (grid-stepped default, continuous opt-in), dispatch `SceneEcs::pathfind` to a new headless polyanya navmesh router for continuous scenes, and render an honest fog-safe route preview with a Euclidean budget — while grid movement stays byte-for-byte unchanged and continuous move *execution* stays disabled until M10f-2/3.

**Architecture:** `scene/navmesh.rs` (new, pure) builds a footprint-inflated `polyanya::Mesh` from a scene's `bounds` (M10f-0) and `blocksMove` wall segments (via `spade`-backed CDT + `geo::Buffer` segment inflation), memoized per `(scene, footprint radius)` on `SceneEcs` and invalidated on wall/bounds mutation. `SceneEcs::pathfind` dispatches on a new `movementModel` resolved axis (mirrors `movement_restriction`) to either the existing grid `pathfinding::find` or the new `navmesh::navmesh_find`; both return the same `PathOutcome`. The any-angle route is arc-length-sampled and cell-gated against the *same* `visible_cells` mask the grid router uses, so `route ⊆ gate-allowed` holds across both engines with zero new secrecy code. Client: `GameSettingsPanel` gets a `movementModel` world-default + scene-override editor (mirrors the existing `movementRestriction` editor); `commitRoute` in the measure-tool controller is gated off for continuous scenes (preview keeps working; execution is M10f-3).

**Tech Stack:** Rust (server), `polyanya 0.16` (any-angle navmesh, headless CDT via internal `spade`), `geo 0.32` (segment→capsule obstacle inflation via `Buffer`), TypeScript/Svelte 5 (client).

## Global Constraints

- Grid-stepped scenes MUST be byte-for-byte unchanged — no continuous-related code path may alter `pathfinding::find`'s behavior or output.
- Continuous move **execution** (commit) MUST be disabled in this checkpoint — no grid-snap fallback. Preview only.
- `movementModel`/`bounds`/`movementRestriction`-style scene axes are **opaque `system`-body JSON, never ts-rs types** — the server stays structural-only (string-match parse, fail-closed to the safe default on anything unrecognized). Do **not** add a ts-rs derive or regenerate bindings for `movementModel`.
  **Erratum vs. the design doc (buddy-check finding, 2026-07-02):** the approved design spec's §4.1/§8 say "ts-rs → regenerate → Zod mirror" for `movementModel`, mirroring what they (inaccurately) believed `movementRestriction` does. Verified against the actual codebase: `MovementRestriction` has no `ts_rs`/`#[ts(export)]` derive anywhere in `scene/mod.rs`, and its client `scene-docs.ts` type is hand-authored, not generated — the design doc's ts-rs language is itself stale on this point. This plan's constraint (no ts-rs) is the technically correct choice, following real precedent, not a deviation to flag further.
- `polyanya = { version = "0.16", default-features = false }` (drop `async` + `recast` features — blocking `Mesh::path()` only, no Recast import); `geo = "0.32"` pinned to unify with polyanya's own dependency copy.
- The binary-size budget is the existing CI check: release binary `< 62914560` bytes (60 MiB). Measure the delta from these deps immediately after adding them (Task 1) — don't wait until the end to discover a problem.
- The cell-sampled visibility post-filter MUST reuse the *existing* `visible_cells` mask, `movement::supercover_cells`, and `move_stream::sample_path` primitives — no new secrecy/visibility decision logic.
- Any new `SceneEcs` interior-mutability field must be `Send + Sync`-safe: `SceneEcs` lives behind a `tokio::sync::RwLock` shared across connection tasks, so use `std::sync::Mutex` + `Arc` (never `RefCell`/`Rc`).
- Follow existing fail-closed conventions exactly: degenerate/malformed/over-cap geometric input → `None`/`Unreachable`, never a silent all-pass or panic.

---

## Task 1: Add polyanya + geo dependencies; lock the real API with a smoke test

**Files:**
- Modify: `src/server/Cargo.toml`
- Create: `src/server/src/scene/navmesh.rs`
- Modify: `src/server/src/scene/mod.rs:1-12` (module registration)

**Interfaces:**
- Produces: the `navmesh` module (empty except the smoke test) that later tasks build on.

- [ ] **Step 1: Add the dependencies**

In `src/server/Cargo.toml`, in the `[dependencies]` section (after the `hecs = "0.11"` line), add:

```toml
polyanya = { version = "0.16", default-features = false }
geo = "0.32"
glam = "0.30"
```

`glam` is used directly (`glam::Vec2`) in the adapter code below. Even though it's already a transitive dependency of `polyanya`, Rust requires a crate used by name to be a *direct* Cargo dependency — declare it explicitly here rather than relying on re-export. Pin the same major/minor polyanya resolves (`^0.30.8` per its `Cargo.toml`) so cargo unifies to one compiled copy instead of two.

- [ ] **Step 2: Register the new module**

In `src/server/src/scene/mod.rs`, in the module list at the top of the file (currently):

```rust
pub mod explored;
pub mod lighting;
pub(crate) mod move_exec;
pub(crate) mod move_stream;
pub mod movement;
pub(crate) mod pathfinding;
pub(crate) mod regions;
pub mod vision;
```

change it to:

```rust
pub mod explored;
pub mod lighting;
pub(crate) mod move_exec;
pub(crate) mod move_stream;
pub mod movement;
pub(crate) mod navmesh;
pub(crate) mod pathfinding;
pub(crate) mod regions;
pub mod vision;
```

- [ ] **Step 3: Write the smoke test (locks the real polyanya/geo API before building the adapter)**

Create `src/server/src/scene/navmesh.rs`:

```rust
//! M10f-1 continuous (navmesh) pathfinding adapter. Pure geometry: builds a footprint-inflated
//! `polyanya::Mesh` from a scene's bounds + `blocksMove` wall segments, and queries any-angle
//! routes over it. Engine-owned geometry (ARCHITECTURE §6 exception), mirroring the grid A*
//! router's fail-closed discipline (`scene/pathfinding.rs`) — this checkpoint carries WALLS ONLY;
//! impassable/terrain regions land in M10f-4 (parent spec §7/§10).

#[cfg(test)]
mod smoke {
    // Locks down the real polyanya 0.16 headless API before the real adapter is built on top:
    // a bare rectangle with no obstacles, queried start->goal, must return a straight path whose
    // length equals the Euclidean distance.
    #[test]
    fn bare_rectangle_paths_straight_line() {
        let outer = [
            glam::Vec2::new(0.0, 0.0),
            glam::Vec2::new(1000.0, 0.0),
            glam::Vec2::new(1000.0, 1000.0),
            glam::Vec2::new(0.0, 1000.0),
        ];
        let tri = polyanya::Triangulation::from_outer_edges(&outer);
        let mesh = tri.as_navmesh();
        let path = mesh
            .path(glam::Vec2::new(50.0, 50.0), glam::Vec2::new(950.0, 50.0))
            .expect("straight route across an empty rectangle must exist");
        assert!(
            (path.length - 900.0).abs() < 1.0,
            "expected ~900, got {}",
            path.length
        );
        assert!(path.path.len() >= 2, "path must have at least 2 vertices");
        let last = path.path.last().unwrap();
        assert!(
            (last.x - 950.0).abs() < 1.0 && (last.y - 50.0).abs() < 1.0,
            "last vertex must be the goal, got {:?}",
            last
        );
    }

    // Buddy-check finding (2026-07-02, Important): Task 6 puts `polyanya::Mesh` behind a
    // `std::sync::Mutex` cache on `SceneEcs`, which itself lives behind a `tokio::sync::RwLock`
    // shared across connection tasks — this REQUIRES `Mesh: Send + Sync`. Assert it here, at the
    // point the dependency is first added, rather than discovering a violation 4-5 commits later
    // in Task 6 after the cache design is already built on top of it.
    #[test]
    fn mesh_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<polyanya::Mesh>();
    }
}
```

- [ ] **Step 4: Run the smoke test; fix any API mismatches**

Run: `cargo test -p shadowcat --lib scene::navmesh::smoke -- --nocapture`

Expected: both tests either pass, or fail to *compile* against the exact polyanya 0.16 API. If `bare_rectangle_paths_straight_line` fails to compile, consult `cargo doc --open -p polyanya` (or docs.rs) for the exact `Triangulation`/`Mesh`/`Path` shapes and adjust the smoke test until it compiles and passes. If `mesh_is_send_and_sync` fails to compile, `polyanya::Mesh` is not `Send + Sync` as designed — STOP and surface this as a complication before proceeding to Task 6, since the cache design in this plan assumes it (do not silently switch to `Rc`/`RefCell`, which would make `SceneEcs` unusable behind its `RwLock`). Do not proceed to Task 4/5 until both tests are green — every later step in this plan depends on these exact type/method names and trait bounds being correct.

- [ ] **Step 5: Measure the binary-size delta against the 60 MiB CI budget**

Run:
```bash
cd src/server
cargo build --release
wc -c target/release/shadowcat* 2>/dev/null || (cd ../.. && ls -la src/server/target/release/)
```

Compare the reported byte count to `62914560` (60 MiB, the exact ceiling in `.github/workflows/ci.yml`'s "Binary size budget" step). Record the before/after delta in the task's commit message. If the release binary is already close to or over the ceiling, STOP and surface this as a complication before continuing (per the plan's Global Constraints) — do not silently proceed.

- [ ] **Step 6: Commit**

```bash
git add src/server/Cargo.toml src/server/Cargo.lock src/server/src/scene/mod.rs src/server/src/scene/navmesh.rs
git commit -m "feat(m10f-1): add polyanya + geo deps, lock the navmesh API with a smoke test"
```

---

## Task 2: `MovementModel` server axis — enum, resolver, dispatch stub

**Files:**
- Modify: `src/server/src/scene/mod.rs`

**Interfaces:**
- Produces: `pub enum MovementModel { GridStepped, Continuous }`, `pub(crate) fn parse_movement_model(s: &str) -> MovementModel`, `ResolvedScene.movement_model: MovementModel`.
- Consumes: the existing `resolve_scene` world/scene-layer resolution pattern (`ws_scene`, `scene_sys`, `d_*` variables).

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `src/server/src/scene/mod.rs` (near the existing `resolve_scene_movement_restriction_*` tests, e.g. after line ~2800):

```rust
    #[test]
    fn resolve_scene_movement_model_defaults_to_grid_stepped() {
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
        let r = ecs.resolve_scene(Uuid::from_u128(10));
        assert_eq!(r.movement_model, MovementModel::GridStepped);
    }

    #[test]
    fn resolve_scene_movement_model_world_override_to_continuous() {
        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#0a0e1a", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "movementModel": "continuous",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        let r = ecs.resolve_scene(Uuid::from_u128(10));
        assert_eq!(r.movement_model, MovementModel::Continuous);
    }

    #[test]
    fn resolve_scene_movement_model_scene_override_beats_world() {
        let mut ecs = SceneEcs::from_documents(
            vec![entity_doc_top(
                10,
                "scene",
                json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                        "vision": { "movementModel": "continuous" } }),
            )],
            0,
        );
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#0a0e1a", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "movementModel": "grid-stepped",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        let r = ecs.resolve_scene(Uuid::from_u128(10));
        assert_eq!(r.movement_model, MovementModel::Continuous);
    }

    #[test]
    fn resolve_scene_movement_model_null_scene_override_inherits_world() {
        let mut ecs = SceneEcs::from_documents(
            vec![entity_doc_top(
                10,
                "scene",
                json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                        "vision": { "movementModel": null } }),
            )],
            0,
        );
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#0a0e1a", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "movementModel": "continuous",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        let r = ecs.resolve_scene(Uuid::from_u128(10));
        assert_eq!(r.movement_model, MovementModel::Continuous);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat --lib resolve_scene_movement_model -- --nocapture`
Expected: FAIL with "no field `movement_model`" / "no variant `MovementModel`" (compile error).

- [ ] **Step 3: Add the enum + parser**

In `src/server/src/scene/mod.rs`, immediately after the existing `parse_movement_restriction` function (after line 47), add:

```rust
/// Per-scene movement/pathfinding engine choice (M10f-1). Mirrors `MovementModel` in
/// `scene-docs.ts`. `GridStepped` = the existing grid A* router; `Continuous` = the polyanya
/// navmesh router.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovementModel {
    GridStepped,
    Continuous,
}

/// Parse a movement-model string; any unknown/missing value fails closed to `GridStepped` —
/// the pre-existing, fully-proven engine. A scene is never silently switched to the newer
/// navmesh router without an explicit author choice.
fn parse_movement_model(s: &str) -> MovementModel {
    match s {
        "continuous" => MovementModel::Continuous,
        _ => MovementModel::GridStepped,
    }
}
```

- [ ] **Step 4: Add the field to `ResolvedScene`**

In `src/server/src/scene/mod.rs`, in the `ResolvedScene` struct (around line 56-69), add a field after `movement_restriction`:

```rust
    pub movement_restriction: MovementRestriction,
    /// Per-scene/world-default pathfinding engine choice (M10f-1). `GridStepped` dispatches to
    /// `pathfinding::find`; `Continuous` dispatches to `navmesh::navmesh_find`.
    pub movement_model: MovementModel,
```

- [ ] **Step 5: Wire world-default + scene-override resolution**

In `src/server/src/scene/mod.rs`, in `resolve_scene`, immediately after the `d_move` world-default block (around line 423-427):

```rust
        // movementRestriction: scene `vision.movementRestriction` ?? world ?? "visible".
        let d_move = ws_scene
            .and_then(|s| s.get("movementRestriction"))
            .and_then(|v| v.as_str())
            .unwrap_or("visible");
```

add:

```rust
        // movementModel (M10f-1): scene `vision.movementModel` ?? world ?? "grid-stepped".
        let d_model = ws_scene
            .and_then(|s| s.get("movementModel"))
            .and_then(|v| v.as_str())
            .unwrap_or("grid-stepped");
```

Then, immediately after the existing `move_str` scene-override read (around line 463-468):

```rust
        let move_str = s
            .and_then(|s| s.pointer("/vision/movementRestriction"))
            .and_then(|v| v.as_str())
            .unwrap_or(d_move);
```

add:

```rust
        let model_str = s
            .and_then(|s| s.pointer("/vision/movementModel"))
            .and_then(|v| v.as_str())
            .unwrap_or(d_model);
```

Finally, in the `ResolvedScene { ... }` construction (around line 487-502), add a field after `movement_restriction`:

```rust
            movement_restriction: parse_movement_restriction(move_str),
            movement_model: parse_movement_model(model_str),
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p shadowcat --lib resolve_scene_movement_model -- --nocapture`
Expected: PASS (4 tests).

- [ ] **Step 7: Run the full server test suite + clippy (no regressions)**

Run: `cargo test -p shadowcat --lib && cargo clippy --all-targets -- -D warnings`
Expected: all green.

- [ ] **Step 8: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "feat(m10f-1): add MovementModel scene axis (grid-stepped default, world+scene resolution)"
```

---

## Task 3: `MovementModel` client axis — type, resolver

**Files:**
- Modify: `src/client/core/src/scene-docs.ts`
- Modify: `src/client/core/src/scene-docs.test.ts`

**Interfaces:**
- Produces: `export type MovementModel = "grid-stepped" | "continuous";`, `ResolvedSceneSettings.movementModel`.
- Consumes: the existing `resolveSceneSettings` world/scene-layer merge pattern.

- [ ] **Step 1: Write the failing tests**

Add to `src/client/core/src/scene-docs.test.ts` (near the existing `movementRestriction` resolver tests):

```ts
  it("movementModel defaults to grid-stepped", () => {
    const scene = buildSceneDoc("w1", {}, "scene1");
    const r = resolveSceneSettings(scene, storeWith());
    expect(r.movementModel).toBe("grid-stepped");
  });

  it("movementModel: world override applies", () => {
    const ws = buildWorldSettingsDoc("w1", {
      ...DEFAULT_WORLD_SETTINGS,
      scene: { ...DEFAULT_WORLD_SETTINGS.scene, movementModel: "continuous" },
    }, "ws1");
    const scene = buildSceneDoc("w1", {}, "scene1");
    const r = resolveSceneSettings(scene, storeWith(ws));
    expect(r.movementModel).toBe("continuous");
  });

  it("movementModel: scene override beats world", () => {
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    const scene = buildSceneDoc("w1", {
      vision: { movementModel: "continuous" },
    }, "scene1");
    const r = resolveSceneSettings(scene, storeWith(ws));
    expect(r.movementModel).toBe("continuous");
  });

  it("movementModel: null scene override inherits world", () => {
    const ws = buildWorldSettingsDoc("w1", {
      ...DEFAULT_WORLD_SETTINGS,
      scene: { ...DEFAULT_WORLD_SETTINGS.scene, movementModel: "continuous" },
    }, "ws1");
    const scene = buildSceneDoc("w1", {
      vision: { movementModel: null },
    }, "scene1");
    const r = resolveSceneSettings(scene, storeWith(ws));
    expect(r.movementModel).toBe("continuous");
  });
```

Check the top of `scene-docs.test.ts` for the existing `storeWith(...)` test helper (it already backs the `movementRestriction` tests in this file) and reuse it as-is — do not add a new helper.

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- scene-docs`
Expected: FAIL (`movementModel` is `undefined`, or a TS compile error if the test file is type-checked).

- [ ] **Step 3: Add the type + wire the resolver**

In `src/client/core/src/scene-docs.ts`, change the type alias block (line 11):

```ts
export type MovementRestriction = "visible" | "revealed" | "unrestricted";
```

to:

```ts
export type MovementRestriction = "visible" | "revealed" | "unrestricted";
/** Per-scene pathfinding engine choice (M10f-1). `grid-stepped` = the existing grid A* router;
 * `continuous` = the M10f polyanya navmesh router (preview only until M10f-3 ships execution). */
export type MovementModel = "grid-stepped" | "continuous";
```

In `SceneVisionOverrides` (line 42-47), add a field after `movementRestriction`:

```ts
export interface SceneVisionOverrides {
  losRestriction?: boolean | null;
  fog?: boolean | null;
  observerVision?: boolean | null;
  movementRestriction?: MovementRestriction | null;
  movementModel?: MovementModel | null;
}
```

In `WorldSceneDefaults` (line 69-78), add a field after `movementRestriction`:

```ts
export interface WorldSceneDefaults {
  losRestriction: boolean;
  fog: boolean;
  lightingEnabled: boolean;
  lightMode: LightMode;
  environment: EnvironmentLight;
  observerVision: boolean;
  movementRestriction: MovementRestriction;
  movementModel: MovementModel;
  partialCellLeniency: boolean;
}
```

In `DEFAULT_WORLD_SETTINGS.scene` (line 90-99), add a field after `movementRestriction: "visible",`:

```ts
export const DEFAULT_WORLD_SETTINGS: WorldSettingsSystem = deepFreeze({
  scene: {
    losRestriction: true,
    fog: true,
    lightingEnabled: true,
    lightMode: "environmentLight",
    environment: { color: "#0a0e1a", intensity: 0.0 },
    observerVision: false,
    movementRestriction: "visible",
    movementModel: "grid-stepped",
    partialCellLeniency: true,
  },
  pathfinding: { diagonalRule: "chebyshev" },
  animation: { speedCellsPerSec: 6, easing: "easeInOut" },
});
```

In `ResolvedSceneSettings` (line 107-120), add a field after `movementRestriction`:

```ts
export interface ResolvedSceneSettings {
  losRestriction: boolean;
  fog: boolean;
  observerVision: boolean;
  movementRestriction: MovementRestriction;
  movementModel: MovementModel;
  lightingEnabled: boolean;
  lightMode: LightMode;
  environment: EnvironmentLight;
  partialCellLeniency: boolean;
  diagonalRule: DiagonalRule;
  animation: { speedCellsPerSec: number; easing: EasingMode };
  gridDistance: GridDistance;
  bounds: SceneDimensions;
}
```

In `resolveSceneSettings`'s return object (line 245-258), add a field after `movementRestriction`:

```ts
    movementRestriction: v.movementRestriction ?? d.scene.movementRestriction,
    movementModel: v.movementModel ?? d.scene.movementModel,
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/core test -- scene-docs`
Expected: PASS.

- [ ] **Step 5: Run the full client gate (typecheck + lint + test)**

Run: `pnpm -r typecheck && pnpm lint && pnpm -r test`
Expected: all green (confirms `movementModel` doesn't break any other `WorldSceneDefaults`/`ResolvedSceneSettings` consumer — a required object literal elsewhere would now fail typecheck if it constructs those types without the new field).

- [ ] **Step 6: Commit**

```bash
git add src/client/core/src/scene-docs.ts src/client/core/src/scene-docs.test.ts
git commit -m "feat(m10f-1): add MovementModel client axis (mirrors movementRestriction resolution)"
```

---

## Task 4: `scene/navmesh.rs` — mesh construction (bounds + wall inflation)

**Files:**
- Modify: `src/server/src/scene/navmesh.rs`

**Interfaces:**
- Produces: `pub(crate) struct NavMesh` (opaque, holds a built `polyanya::Mesh`), `pub(crate) const MAX_NAVMESH_OBSTACLE_SEGMENTS: usize`, `pub(crate) fn build_navmesh(bounds: (f64, f64), cell: f64, walls: &[vision::Seg], footprint_radius_cells: f64) -> Option<NavMesh>`.
- Consumes: `crate::scene::vision::Seg` (existing).

- [ ] **Step 1: Write the failing tests**

Replace the ENTIRE contents of `src/server/src/scene/navmesh.rs` (including the file-level doc comment and the `mod smoke` block from Task 1 — both are reproduced below, unchanged, so nothing from Task 1 is lost) with:

```rust
//! M10f-1 continuous (navmesh) pathfinding adapter. Pure geometry: builds a footprint-inflated
//! `polyanya::Mesh` from a scene's bounds + `blocksMove` wall segments, and queries any-angle
//! routes over it. Engine-owned geometry (ARCHITECTURE §6 exception), mirroring the grid A*
//! router's fail-closed discipline (`scene/pathfinding.rs`) — this checkpoint carries WALLS ONLY;
//! impassable/terrain regions land in M10f-4 (parent spec §7/§10).

#[cfg(test)]
mod smoke {
    // Locks down the real polyanya 0.16 headless API before the real adapter is built on top:
    // a bare rectangle with no obstacles, queried start->goal, must return a straight path whose
    // length equals the Euclidean distance.
    #[test]
    fn bare_rectangle_paths_straight_line() {
        let outer = [
            glam::Vec2::new(0.0, 0.0),
            glam::Vec2::new(1000.0, 0.0),
            glam::Vec2::new(1000.0, 1000.0),
            glam::Vec2::new(0.0, 1000.0),
        ];
        let tri = polyanya::Triangulation::from_outer_edges(&outer);
        let mesh = tri.as_navmesh();
        let path = mesh
            .path(glam::Vec2::new(50.0, 50.0), glam::Vec2::new(950.0, 50.0))
            .expect("straight route across an empty rectangle must exist");
        assert!(
            (path.length - 900.0).abs() < 1.0,
            "expected ~900, got {}",
            path.length
        );
        assert!(path.path.len() >= 2, "path must have at least 2 vertices");
        let last = path.path.last().unwrap();
        assert!(
            (last.x - 950.0).abs() < 1.0 && (last.y - 50.0).abs() < 1.0,
            "last vertex must be the goal, got {:?}",
            last
        );
    }

    #[test]
    fn mesh_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<polyanya::Mesh>();
    }
}

use crate::scene::vision::Seg;
use geo::algorithm::buffer::Buffer;
use geo::Line;

/// DoS guard: a scene with more `blocksMove` segments than this fails closed (no navmesh) rather
/// than triangulating an unbounded obstacle count. Generous relative to a hand-authored scene
/// (mirrors the generosity of `movement::MAX_MOVE_CELLS` / `regions::MAX_REGION_CELLS`).
pub(crate) const MAX_NAVMESH_OBSTACLE_SEGMENTS: usize = 5_000;

/// A built, footprint-inflated navmesh for one `(scene, footprint radius)` pair. Immutable after
/// construction — `SceneEcs`'s cache (Task 6) rebuilds a new one on wall/bounds mutation.
pub(crate) struct NavMesh {
    pub(crate) mesh: polyanya::Mesh,
}

/// Build a footprint-inflated navmesh from a scene's bounds (grid units; converted to scene
/// pixels via `cell`) and `blocksMove` wall segments. Fails closed (`None`) on: non-finite/
/// non-positive bounds or cell size, a non-finite/negative/over-cap footprint radius, an obstacle
/// count over `MAX_NAVMESH_OBSTACLE_SEGMENTS`, or a triangulation/mesh-build failure — callers
/// MUST treat `None` as "no navmesh" (the scene reports `Unreachable`, never a silent all-pass).
pub(crate) fn build_navmesh(
    bounds: (f64, f64),
    cell: f64,
    walls: &[Seg],
    footprint_radius_cells: f64,
) -> Option<NavMesh> {
    let (w, h) = bounds;
    if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return None;
    }
    if !cell.is_finite() || cell <= 0.0 {
        return None;
    }
    // Buddy-check finding (2026-07-02, Critical): the grid engine's `pathfinding::find` bounds
    // `footprint_radius` to `0.0..=MAX_FOOTPRINT_CELLS` before doing any work; this checkpoint's
    // continuous dispatch had no equivalent ceiling, so an untrusted `footprintRadius` on the wire
    // (`Pathfind` request) could drive an unbounded `geo::Buffer` inflation AND an unbounded
    // `pathfinding::footprint_cells` scan later in `clip_to_visible_mask` (its nested loop grows
    // with `(radius/cell)^2`). Reuse the SAME bound as the grid engine — no new DoS surface.
    if !(0.0..=crate::scene::pathfinding::MAX_FOOTPRINT_CELLS).contains(&footprint_radius_cells) {
        return None;
    }
    if walls.len() > MAX_NAVMESH_OBSTACLE_SEGMENTS {
        return None;
    }
    let footprint_scene = (footprint_radius_cells * cell).max(0.01);
    let (w_px, h_px) = (w * cell, h * cell);

    let outer = [
        glam::Vec2::new(0.0, 0.0),
        glam::Vec2::new(w_px as f32, 0.0),
        glam::Vec2::new(w_px as f32, h_px as f32),
        glam::Vec2::new(0.0, h_px as f32),
    ];
    let mut tri = polyanya::Triangulation::from_outer_edges(&outer);

    for seg in walls {
        if !seg.a.0.is_finite() || !seg.a.1.is_finite() || !seg.b.0.is_finite() || !seg.b.1.is_finite() {
            continue; // a malformed wall segment is skipped, never turned into a bogus obstacle
        }
        // `blocksMove` walls have no thickness field — inflating the zero-width segment by the
        // agent's footprint radius is the correct Minkowski obstacle for a disc-footprint agent.
        let line = Line::new((seg.a.0, seg.a.1), (seg.b.0, seg.b.1));
        let inflated = line.buffer(footprint_scene);
        for poly in inflated.iter() {
            let ring: Vec<glam::Vec2> = poly
                .exterior()
                .points()
                .map(|p| glam::Vec2::new(p.x() as f32, p.y() as f32))
                .collect();
            if ring.len() >= 3 {
                tri.add_obstacle(ring);
            }
        }
    }

    Some(NavMesh {
        mesh: tri.as_navmesh(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degenerate_bounds_fail_closed() {
        assert!(build_navmesh((0.0, 100.0), 100.0, &[], 0.4).is_none());
        assert!(build_navmesh((100.0, -1.0), 100.0, &[], 0.4).is_none());
        assert!(build_navmesh((f64::NAN, 100.0), 100.0, &[], 0.4).is_none());
        assert!(build_navmesh((f64::INFINITY, 100.0), 100.0, &[], 0.4).is_none());
    }

    #[test]
    fn degenerate_cell_fails_closed() {
        assert!(build_navmesh((100.0, 100.0), 0.0, &[], 0.4).is_none());
        assert!(build_navmesh((100.0, 100.0), -1.0, &[], 0.4).is_none());
    }

    #[test]
    fn negative_or_non_finite_footprint_fails_closed() {
        assert!(build_navmesh((100.0, 100.0), 100.0, &[], -0.1).is_none());
        assert!(build_navmesh((100.0, 100.0), 100.0, &[], f64::NAN).is_none());
    }

    #[test]
    fn over_cap_footprint_radius_fails_closed() {
        // Mirrors `pathfinding::find`'s `MAX_FOOTPRINT_CELLS` ceiling — an untrusted wire
        // `footprintRadius` must not drive an unbounded geo::Buffer inflation.
        let over_cap = crate::scene::pathfinding::MAX_FOOTPRINT_CELLS + 1.0;
        assert!(build_navmesh((100.0, 100.0), 100.0, &[], over_cap).is_none());
        assert!(build_navmesh((100.0, 100.0), 100.0, &[], f64::INFINITY).is_none());
    }

    #[test]
    fn over_cap_obstacle_count_fails_closed() {
        let walls: Vec<Seg> = (0..(MAX_NAVMESH_OBSTACLE_SEGMENTS + 1))
            .map(|i| Seg {
                a: (i as f64, 0.0),
                b: (i as f64, 1.0),
            })
            .collect();
        assert!(build_navmesh((10_000.0, 100.0), 100.0, &walls, 0.4).is_none());
    }

    #[test]
    fn empty_scene_builds_a_navmesh() {
        assert!(build_navmesh((100.0, 100.0), 100.0, &[], 0.4).is_some());
    }

    #[test]
    fn a_malformed_wall_segment_is_skipped_not_fatal() {
        let walls = vec![Seg {
            a: (f64::NAN, 0.0),
            b: (10.0, 10.0),
        }];
        assert!(build_navmesh((100.0, 100.0), 100.0, &walls, 0.4).is_some());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat --lib scene::navmesh::tests -- --nocapture`
Expected: FAIL (the `tests` module doesn't compile yet against the real function — this file is being replaced in this step, so run it AFTER Step 1's replacement to see whichever assertions don't hold, e.g. if `over_cap_obstacle_count_fails_closed` fails because the cap check is missing).

- [ ] **Step 3: Fix any failures**

The implementation above should already satisfy all seven tests. If `over_cap_obstacle_count_fails_closed`, `over_cap_footprint_radius_fails_closed`, or the degenerate-input tests fail, double check the guard order at the top of `build_navmesh` matches the code above exactly.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat --lib scene::navmesh -- --nocapture`
Expected: PASS (all of Task 1's smoke tests + Task 4's 7 tests).

- [ ] **Step 5: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean. If `Buffer`'s import path differs from `geo::algorithm::buffer::Buffer` in the resolved `geo 0.32`, clippy/build will surface it — correct the `use` statement to match (check with `cargo doc --open -p geo` if needed).

- [ ] **Step 6: Commit**

```bash
git add src/server/src/scene/navmesh.rs
git commit -m "feat(m10f-1): navmesh construction — bounds + footprint-inflated wall obstacles"
```

---

## Task 5: `scene/navmesh.rs` — query (`navmesh_find`)

**Files:**
- Modify: `src/server/src/scene/navmesh.rs`
- Modify: `src/server/src/scene/pathfinding.rs` (add `Clone` to `PathOutcome`'s derive list)

**Interfaces:**
- Consumes: `NavMesh` (Task 4), `crate::scene::pathfinding::{PathOutcome, PathFail}` (existing), `crate::scene::vision::P` (existing).
- Produces: `pub(crate) fn navmesh_find(nav: &NavMesh, start: vision::P, waypoints: &[vision::P]) -> Result<pathfinding::PathOutcome, pathfinding::PathFail>`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `src/server/src/scene/navmesh.rs` (after the Task 4 tests):

```rust
    #[test]
    fn empty_waypoints_is_invalid() {
        let nav = build_navmesh((100.0, 100.0), 100.0, &[], 0.4).unwrap();
        let r = navmesh_find(&nav, (50.0, 50.0), &[]);
        assert_eq!(r, Err(crate::scene::pathfinding::PathFail::Invalid));
    }

    #[test]
    fn straight_route_cost_is_euclidean() {
        let nav = build_navmesh((10.0, 10.0), 100.0, &[], 0.1).unwrap();
        let outcome = navmesh_find(&nav, (50.0, 50.0), &[(950.0, 50.0)]).unwrap();
        assert!(
            (outcome.cost - 900.0).abs() < 2.0,
            "expected ~900, got {}",
            outcome.cost
        );
        assert!(!outcome.arrested, "M10f-1 navmesh carries no regions");
        assert_eq!(outcome.path.first(), Some(&(50.0, 50.0)));
        let last = *outcome.path.last().unwrap();
        assert!((last.0 - 950.0).abs() < 1.0 && (last.1 - 50.0).abs() < 1.0);
    }

    #[test]
    fn a_wall_in_the_direct_path_forces_a_detour() {
        // A vertical wall from (500,0) to (500,600) blocks the direct horizontal line at y=50,
        // forcing the route to detour around its bottom end (600 < 1000 scene height).
        let walls = vec![Seg {
            a: (500.0, 0.0),
            b: (500.0, 600.0),
        }];
        let nav = build_navmesh((10.0, 10.0), 100.0, &walls, 0.1).unwrap();
        let outcome = navmesh_find(&nav, (50.0, 50.0), &[(950.0, 50.0)]).unwrap();
        assert!(
            outcome.cost > 900.5,
            "a detour around the wall must cost more than the blocked straight line, got {}",
            outcome.cost
        );
    }

    #[test]
    fn multi_leg_route_concatenates_without_a_duplicated_join_vertex() {
        let nav = build_navmesh((10.0, 10.0), 100.0, &[], 0.1).unwrap();
        let outcome =
            navmesh_find(&nav, (50.0, 50.0), &[(500.0, 50.0), (950.0, 50.0)]).unwrap();
        // No two consecutive vertices should be exactly equal (a duplicated leg-join point).
        for w in outcome.path.windows(2) {
            assert_ne!(w[0], w[1], "consecutive duplicate vertex at a leg join: {:?}", w);
        }
    }

    // Buddy-check finding (2026-07-02, Critical, agreed by both reviewers): `navmesh_find` had no
    // input-validation parity with `pathfinding::find` — an untrusted wire `Pathfind` request could
    // carry unbounded waypoints (unbounded `Mesh::path` calls) or non-finite coordinates (risking a
    // panic inside polyanya's internal spatial queries; this codebase has already hit exactly this
    // failure mode once, in `move_stream::sample_path`'s NaN-propagation history). Reuse the SAME
    // caps the grid engine enforces — no new DoS/panic surface for continuous scenes.
    #[test]
    fn over_cap_waypoints_is_invalid() {
        let nav = build_navmesh((100.0, 100.0), 100.0, &[], 0.1).unwrap();
        let waypoints: Vec<(f64, f64)> = (0..(crate::scene::pathfinding::MAX_WAYPOINTS + 1))
            .map(|i| (i as f64, 0.0))
            .collect();
        let r = navmesh_find(&nav, (50.0, 50.0), &waypoints);
        assert_eq!(r, Err(crate::scene::pathfinding::PathFail::Invalid));
    }

    #[test]
    fn non_finite_start_or_waypoint_is_invalid() {
        let nav = build_navmesh((100.0, 100.0), 100.0, &[], 0.1).unwrap();
        assert_eq!(
            navmesh_find(&nav, (f64::NAN, 50.0), &[(90.0, 50.0)]),
            Err(crate::scene::pathfinding::PathFail::Invalid)
        );
        assert_eq!(
            navmesh_find(&nav, (50.0, 50.0), &[(f64::INFINITY, 50.0)]),
            Err(crate::scene::pathfinding::PathFail::Invalid)
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat --lib scene::navmesh::tests -- --nocapture`
Expected: FAIL with "cannot find function `navmesh_find`" (compile error).

- [ ] **Step 3: Add `Clone` to `PathOutcome`'s derive list**

Task 7 (next) needs `PathOutcome: Clone` for its tests. In `src/server/src/scene/pathfinding.rs`, find the `PathOutcome` struct (around line 443-448):

```rust
pub struct PathOutcome {
    pub path: Vec<vision::P>,
    pub cost: f64,
    pub arrested: bool,
}
```

check its derive attribute immediately above (`#[derive(Debug, PartialEq)]` or similar) and add `Clone`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct PathOutcome {
    pub path: Vec<vision::P>,
    pub cost: f64,
    pub arrested: bool,
}
```

- [ ] **Step 4: Implement `navmesh_find`**

In `src/server/src/scene/navmesh.rs`, add after `build_navmesh`:

```rust
/// Any-angle route `start -> waypoints[0] -> ... -> waypoints[last]` over a built navmesh.
/// Euclidean distance; concatenates per-leg polylines without a duplicated join vertex.
/// Validation mirrors `pathfinding::find`'s `Invalid` guard exactly (Buddy-check finding,
/// 2026-07-02, Critical): waypoints non-empty and bounded by `MAX_WAYPOINTS`, `start`/every
/// waypoint finite — an untrusted wire `Pathfind` request must not reach `Mesh::path` unbounded
/// or with a non-finite coordinate. Any leg with no route ⇒ `Unreachable`. `arrested` is always
/// `false` — the M10f-1 navmesh carries no region field (M10f-4 wires regions onto the navmesh).
pub(crate) fn navmesh_find(
    nav: &NavMesh,
    start: crate::scene::vision::P,
    waypoints: &[crate::scene::vision::P],
) -> Result<crate::scene::pathfinding::PathOutcome, crate::scene::pathfinding::PathFail> {
    use crate::scene::pathfinding::{PathFail, PathOutcome, MAX_WAYPOINTS};

    if waypoints.is_empty() || waypoints.len() > MAX_WAYPOINTS {
        return Err(PathFail::Invalid);
    }
    let all_finite = start.0.is_finite()
        && start.1.is_finite()
        && waypoints.iter().all(|w| w.0.is_finite() && w.1.is_finite());
    if !all_finite {
        return Err(PathFail::Invalid);
    }

    let mut full_path: Vec<crate::scene::vision::P> = vec![start];
    let mut cost = 0.0_f64;
    let mut leg_start = start;

    for &wp in waypoints {
        let from = glam::Vec2::new(leg_start.0 as f32, leg_start.1 as f32);
        let to = glam::Vec2::new(wp.0 as f32, wp.1 as f32);
        let Some(path) = nav.mesh.path(from, to) else {
            return Err(PathFail::Unreachable);
        };
        cost += path.length as f64;

        for (i, v) in path.path.iter().enumerate() {
            let pt = (v.x as f64, v.y as f64);
            if i == 0 {
                // polyanya's returned polyline may or may not repeat the query start vertex;
                // skip it only if it coincides with the point we already have, so the assembled
                // polyline never gets a duplicated join vertex regardless of that detail.
                let dx = pt.0 - leg_start.0;
                let dy = pt.1 - leg_start.1;
                if (dx * dx + dy * dy).sqrt() < 1e-6 {
                    continue;
                }
            }
            full_path.push(pt);
        }
        leg_start = wp;
    }

    Ok(PathOutcome {
        path: full_path,
        cost,
        arrested: false,
    })
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p shadowcat --lib scene::navmesh -- --nocapture`
Expected: PASS (all tests in the module, including the two new validation tests).

- [ ] **Step 6: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add src/server/src/scene/navmesh.rs src/server/src/scene/pathfinding.rs
git commit -m "feat(m10f-1): navmesh query — any-angle multi-leg routing, input-validation parity with the grid engine"
```

---

## Task 6: Memoized navmesh cache on `SceneEcs` + invalidation on geometry mutation

**Files:**
- Modify: `src/server/src/scene/mod.rs`

**Interfaces:**
- Produces: `SceneEcs::navmesh_for(&self, scene: Uuid, footprint_radius_cells: f64) -> Option<std::sync::Arc<navmesh::NavMesh>>` (builds-or-fetches, memoized).
- Consumes: `resolve_scene` (Task 2, for `bounds`), `move_walls` (existing), `scene_grid_sizes` (existing), `navmesh::build_navmesh` (Task 4).

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `src/server/src/scene/mod.rs`:

```rust
    #[test]
    fn navmesh_for_is_memoized_across_calls() {
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
        let scene = Uuid::from_u128(10);
        let a = ecs.navmesh_for(scene, 0.4).expect("navmesh builds");
        let b = ecs.navmesh_for(scene, 0.4).expect("navmesh builds");
        assert!(
            std::sync::Arc::ptr_eq(&a, &b),
            "same (scene, radius) must return the SAME cached Arc, not rebuild"
        );
    }

    #[test]
    fn navmesh_for_distinguishes_footprint_radii() {
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
        let scene = Uuid::from_u128(10);
        let a = ecs.navmesh_for(scene, 0.4).expect("navmesh builds");
        let b = ecs.navmesh_for(scene, 0.9).expect("navmesh builds");
        assert!(
            !std::sync::Arc::ptr_eq(&a, &b),
            "distinct footprint radii must get distinct cached meshes"
        );
    }

    #[test]
    fn wall_mutation_invalidates_the_navmesh_cache() {
        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
        let scene = Uuid::from_u128(10);
        let a = ecs.navmesh_for(scene, 0.4).expect("navmesh builds");
        ecs.apply_op(&Operation::Create {
            doc: entity_doc(
                20,
                10,
                "wall",
                json!({ "seg": { "x1": 10.0, "y1": 0.0, "x2": 10.0, "y2": 50.0 },
                        "blocksMove": true, "blocksSight": false, "blocksLight": false }),
            ),
        });
        let b = ecs.navmesh_for(scene, 0.4).expect("navmesh rebuilds");
        assert!(
            !std::sync::Arc::ptr_eq(&a, &b),
            "adding a blocksMove wall must invalidate the cached navmesh"
        );
    }

    #[test]
    fn bounds_mutation_invalidates_the_navmesh_cache() {
        let mut ecs = SceneEcs::from_documents(
            vec![entity_doc_top(
                10,
                "scene",
                json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
            )],
            0,
        );
        let scene = Uuid::from_u128(10);
        let a = ecs.navmesh_for(scene, 0.4).expect("navmesh builds");
        ecs.apply_op(&Operation::Update {
            doc_id: scene,
            changes: vec![crate::data::command::FieldChange {
                path: "/system/bounds".into(),
                old: json!(null),
                new: json!({ "width": 40, "height": 40 }),
            }],
        });
        let b = ecs.navmesh_for(scene, 0.4).expect("navmesh rebuilds");
        assert!(
            !std::sync::Arc::ptr_eq(&a, &b),
            "changing scene bounds must invalidate the cached navmesh"
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat --lib navmesh_for -- --nocapture`
Expected: FAIL with "no method `navmesh_for`" (compile error).

- [ ] **Step 3: Add the cache field**

In `src/server/src/scene/mod.rs`, in the `SceneEcs` struct (around line 167-180), add a field after `actors` (find the field via the constructor at line 188-198, since the struct's field list and its `new()` initializer must both change):

Find the struct definition (it ends with the `actors` field before the closing brace, mirroring `new()`'s literal) and add:

```rust
    /// M10f-1 footprint-inflated navmesh cache, keyed by `(scene, quantized footprint-radius
    /// millicells)`. `std::sync::Mutex` (not `RefCell`) + `Arc` (not `Rc`): `SceneEcs` sits behind
    /// a `tokio::sync::RwLock` shared across connection tasks, so concurrent readers may call
    /// `pathfind`/`navmesh_for` simultaneously — the cache needs `Sync` interior mutability.
    /// Never held across an `.await` (lookup + build are synchronous). Quantized to the nearest
    /// 1/1000 cell (Buddy-check finding, 2026-07-02, Important: the design spec explicitly calls
    /// for "quantized footprintRadius" so the cache "stays bounded" given token sizes are a small
    /// discrete set — exact f64-bit keying was an unjustified departure from that, vulnerable to
    /// floating-point noise in a client-computed radius producing distinct bit-patterns for what
    /// is logically the same size).
    navmesh_cache: std::sync::Mutex<HashMap<(Uuid, i64), std::sync::Arc<navmesh::NavMesh>>>,
```

- [ ] **Step 4: Initialize the field in `new()`**

In `src/server/src/scene/mod.rs`, in `SceneEcs::new()` (line 188-198), add the field to the `Self { ... }` literal:

```rust
    pub fn new() -> Self {
        Self {
            world: hecs::World::new(),
            index: HashMap::new(),
            committed_seq: 0,
            world_settings: None,
            gradation: None,
            vision_modes: None,
            actors: HashMap::new(),
            navmesh_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }
```

- [ ] **Step 5: Add `navmesh_for`**

In `src/server/src/scene/mod.rs`, add a new method on `SceneEcs` (near `pathfind`, e.g. right before it):

```rust
    /// Build-or-fetch the footprint-inflated navmesh for `(scene, footprint_radius_cells)`,
    /// memoized in `navmesh_cache` keyed on a quantized radius (nearest 1/1000 cell — see the
    /// field doc comment). Returns `None` when `navmesh::build_navmesh` fails closed (degenerate
    /// bounds/cell/footprint, or an over-cap obstacle count) — callers must treat this exactly
    /// like the grid router's `Unreachable` (no silent all-pass).
    ///
    /// Known accepted tradeoff (Buddy-check finding, 2026-07-02, Minor): the cache-miss path is
    /// lock→check→unlock→build→lock→insert, not atomic under the build. Two concurrent callers
    /// requesting the same new key can each build a redundant (but equally valid) `NavMesh` before
    /// one wins the insert — wasted compute, never a correctness issue (both builds are pure
    /// functions of the same inputs). Not addressed in M10f-1; revisit if profiling shows
    /// concurrent-miss contention is a real cost.
    pub(crate) fn navmesh_for(
        &self,
        scene: Uuid,
        footprint_radius_cells: f64,
    ) -> Option<std::sync::Arc<navmesh::NavMesh>> {
        // Quantize to the nearest 1/1000 cell so floating-point noise in a client-computed radius
        // (e.g. derived via division) collapses onto the same cache entry as the canonical value.
        let quantized = (footprint_radius_cells * 1000.0).round() as i64;
        let key = (scene, quantized);
        if let Some(cached) = self.navmesh_cache.lock().unwrap().get(&key) {
            return Some(cached.clone());
        }
        let bounds = self.resolve_scene(scene).bounds;
        let cell = self
            .scene_grid_sizes()
            .get(&scene)
            .copied()
            .unwrap_or(100.0);
        let walls = self.move_walls(scene);
        let built = navmesh::build_navmesh(bounds, cell, &walls, footprint_radius_cells)?;
        let arc = std::sync::Arc::new(built);
        self.navmesh_cache
            .lock()
            .unwrap()
            .insert(key, arc.clone());
        Some(arc)
    }
```

- [ ] **Step 6: Wire cache invalidation into `apply_op`**

In `src/server/src/scene/mod.rs`, in `apply_op` (line 285-365), the cache must be cleared whenever a `wall` or `scene` document is created, updated, or deleted (bounds live on the scene doc; obstacles come from `wall` docs). Change the function signature's body to resolve the affected doc's type BEFORE the existing match, then clear the cache after:

```rust
    pub fn apply_op(&mut self, op: &Operation) {
        // M10f-1: determine whether this op can affect any cached navmesh's geometry (a `wall`
        // doc's blocksMove/seg fields, or a `scene` doc's bounds) BEFORE applying it — an Update
        // needs the existing entity's doc_type (Update never changes doc_type, so a pre-lookup
        // is safe), Create/Delete carry their own doc_type directly.
        let touches_navmesh_geometry = match op {
            Operation::Create { doc } | Operation::Delete { doc } => {
                matches!(doc.doc_type.as_str(), "wall" | "scene")
            }
            Operation::Update { doc_id, .. } => self
                .index
                .get(doc_id)
                .and_then(|&e| self.world.get::<&SceneEntity>(e).ok())
                .map(|c| matches!(c.doc.doc_type.as_str(), "wall" | "scene"))
                .unwrap_or(false),
        };

        match op {
            Operation::Create { doc } if is_scene_entity(doc) => {
                if let Some(&e) = self.index.get(&doc.id) {
                    let _ = self.world.despawn(e);
                }
                let e = self.world.spawn((SceneEntity { doc: doc.clone() },));
                self.index.insert(doc.id, e);
            }
            Operation::Update { doc_id, changes } => {
                // An Update never changes scene-entity membership: `parent_id`
                // and `doc_type` are envelope fields, immutable via field-path
                // Update (`required_cap_for_path` maps them to no capability).
                // INVARIANT: if `parent_id` becomes mutable, this arm must
                // re-evaluate `is_scene_entity` and spawn/despawn accordingly.
                // TODO: re-evaluate is_scene_entity here once parent_id is mutable.
                if let Some(&e) = self.index.get(doc_id) {
                    if let Ok(mut comp) = self.world.get::<&mut SceneEntity>(e) {
                        // Mirror the same field-path changes apply_intent applied
                        // to SQLite, via Value round-trip (server stays
                        // structural-only; no semantic interpretation).
                        if let Ok(mut v) = serde_json::to_value(&comp.doc) {
                            for ch in changes {
                                let _ = set_pointer(&mut v, &ch.path, ch.new.clone());
                            }
                            if let Ok(updated) = serde_json::from_value::<Document>(v) {
                                comp.doc = updated;
                            }
                        }
                    }
                }
                // Config singletons + actors (not in the hecs index).
                Self::apply_config_update(&mut self.world_settings, *doc_id, changes);
                Self::apply_config_update(&mut self.gradation, *doc_id, changes);
                Self::apply_config_update(&mut self.vision_modes, *doc_id, changes);
                if let Some(a) = self.actors.get_mut(doc_id) {
                    if let Ok(mut v) = serde_json::to_value(&*a) {
                        for ch in changes {
                            let _ = set_pointer(&mut v, &ch.path, ch.new.clone());
                        }
                        if let Ok(updated) = serde_json::from_value::<Document>(v) {
                            *a = updated;
                        }
                    }
                }
            }
            Operation::Delete { doc } => {
                if let Some(e) = self.index.remove(&doc.id) {
                    let _ = self.world.despawn(e);
                }
                match doc.doc_type.as_str() {
                    "world-settings"
                        if self.world_settings.as_ref().map(|d| d.id) == Some(doc.id) =>
                    {
                        self.world_settings = None;
                    }
                    "light-gradation" if self.gradation.as_ref().map(|d| d.id) == Some(doc.id) => {
                        self.gradation = None;
                    }
                    "vision-modes" if self.vision_modes.as_ref().map(|d| d.id) == Some(doc.id) => {
                        self.vision_modes = None;
                    }
                    "actor" => {
                        self.actors.remove(&doc.id);
                    }
                    _ => {}
                }
            }
            Operation::Create { doc } => {
                match doc.doc_type.as_str() {
                    "world-settings" => self.world_settings = Some(doc.clone()),
                    "light-gradation" => self.gradation = Some(doc.clone()),
                    "vision-modes" => self.vision_modes = Some(doc.clone()),
                    "actor" => {
                        self.actors.insert(doc.id, doc.clone());
                    }
                    _ => {} // other non-scene document: ignored
                }
            }
        }

        if touches_navmesh_geometry {
            // Coarse but correct: clear every cached navmesh (all scenes), not just the one
            // touched. Over-invalidation only costs an extra rebuild on next query, never
            // staleness — the safe failure direction, matching the project's established
            // fail-safe-direction convention (e.g. `supercover_cells`'s over-include-on-corner).
            self.navmesh_cache.lock().unwrap().clear();
        }
    }
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p shadowcat --lib navmesh_for -- --nocapture`
Expected: PASS (4 tests).

- [ ] **Step 8: Run the full server suite + clippy**

Run: `cargo test -p shadowcat --lib && cargo clippy --all-targets -- -D warnings`
Expected: all green (confirms `SceneEcs`'s `Send`/`Sync` bounds still hold everywhere it's used behind `tokio::sync::RwLock` — a compile error here means the `Mutex`/`Arc` choice needs revisiting, not `RefCell`/`Rc`).

- [ ] **Step 9: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "feat(m10f-1): memoized per-(scene,footprint) navmesh cache, invalidated on wall/bounds mutation"
```

---

## Task 7: Cell-sampled visibility post-filter (fog-safe preview, route ⊆ gate-allowed)

**Files:**
- Modify: `src/server/src/scene/navmesh.rs`

**Interfaces:**
- Consumes: `move_stream::sample_path` (existing, `pub(crate)`), `movement::supercover_cells` (existing), `pathfinding::footprint_cells` (made `pub(crate)` in Step 1), `crate::scene::segments_cross` (existing, `pub(crate)`, in `scene/mod.rs`).
- Produces: `pub(crate) fn clip_to_visible_mask(outcome: pathfinding::PathOutcome, mask: Option<&std::collections::BTreeSet<pathfinding::Cell>>, cell: f64, footprint_radius_cells: f64, walls: &[vision::Seg]) -> pathfinding::PathOutcome` — truncates `outcome.path` at the first arc-length sample whose footprint/supercover cells leave `mask`, OR whose chord from the previous retained sample crosses a `blocksMove` wall (mask `None` AND no wall crossing ⇒ unconstrained, returned unchanged).

- [ ] **Step 1: Expose `footprint_cells` to the crate**

In `src/server/src/scene/pathfinding.rs`, change the function's visibility (around line 59):

```rust
fn footprint_cells(anchor: Cell, ctr: vision::P, r_scene: f64, cell: f64) -> Vec<Cell> {
```

to:

```rust
pub(crate) fn footprint_cells(anchor: Cell, ctr: vision::P, r_scene: f64, cell: f64) -> Vec<Cell> {
```

- [ ] **Step 2: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `src/server/src/scene/navmesh.rs`:

```rust
    use std::collections::BTreeSet;

    #[test]
    fn clip_returns_unchanged_when_mask_is_none_and_no_walls() {
        let outcome = crate::scene::pathfinding::PathOutcome {
            path: vec![(50.0, 50.0), (950.0, 50.0)],
            cost: 900.0,
            arrested: false,
        };
        let clipped = clip_to_visible_mask(outcome.clone(), None, 100.0, 0.1, &[]);
        assert_eq!(clipped.path, outcome.path);
        assert_eq!(clipped.cost, outcome.cost);
    }

    #[test]
    fn clip_truncates_at_the_mask_boundary() {
        // A route from (50,50) to (950,50): only cells x=0..3 (i.e. up to x=400) are visible.
        let mut mask = BTreeSet::new();
        for i in 0..4 {
            mask.insert((i, 0));
        }
        let outcome = crate::scene::pathfinding::PathOutcome {
            path: vec![(50.0, 50.0), (950.0, 50.0)],
            cost: 900.0,
            arrested: false,
        };
        let clipped = clip_to_visible_mask(outcome, Some(&mask), 100.0, 0.1, &[]);
        let last = *clipped.path.last().unwrap();
        assert!(
            last.0 <= 400.0 + 1e-6,
            "route must truncate at the visible-mask boundary, last x = {}",
            last.0
        );
        assert!(
            clipped.path.len() < 2 || clipped.cost < 900.0,
            "a truncated route must report a shorter cost than the full route"
        );
    }

    #[test]
    fn clip_leaves_a_fully_visible_route_untouched() {
        let mut mask = BTreeSet::new();
        for i in 0..10 {
            mask.insert((i, 0));
        }
        let outcome = crate::scene::pathfinding::PathOutcome {
            path: vec![(50.0, 50.0), (950.0, 50.0)],
            cost: 900.0,
            arrested: false,
        };
        let clipped = clip_to_visible_mask(outcome.clone(), Some(&mask), 100.0, 0.1, &[]);
        let last_orig = *outcome.path.last().unwrap();
        let last_clipped = *clipped.path.last().unwrap();
        assert!((last_orig.0 - last_clipped.0).abs() < 1e-6);
    }

    // Buddy-check finding (2026-07-02, Important, agreed): design §9 explicitly requires "a goal
    // outside the mask ⇒ Unreachable" as a test; nothing previously exercised a mask that excludes
    // the ENTIRE corridor beyond the start cell (as opposed to a partial-route truncation). This is
    // the actual code path Task 8's dispatch maps to `PathFail::Unreachable`.
    #[test]
    fn clip_with_a_mask_excluding_the_whole_corridor_yields_a_single_point_path() {
        let mut mask = BTreeSet::new();
        mask.insert((0, 0)); // only the start cell is visible; the goal and everything beyond is not
        let outcome = crate::scene::pathfinding::PathOutcome {
            path: vec![(50.0, 50.0), (950.0, 50.0)],
            cost: 900.0,
            arrested: false,
        };
        let clipped = clip_to_visible_mask(outcome, Some(&mask), 100.0, 0.1, &[]);
        assert_eq!(
            clipped.path.len(),
            1,
            "a goal wholly outside the mask must truncate to just the start point"
        );
        assert_eq!(clipped.cost, 0.0);
    }

    // Buddy-check finding (2026-07-02, Important, converged after debate): the mask check alone
    // does not guarantee the returned preview never crosses a WALL — a chord between two
    // arc-length samples can cut across geometry the true navmesh polyline routed around. This is
    // a router-fidelity issue (walls are public geometry, not a secrecy leak), but still means a
    // returned preview could visually cross a wall. Verified independent of sample-cap spacing: any
    // chord that geometrically crosses a `blocksMove` wall must truncate there.
    #[test]
    fn clip_truncates_a_chord_that_crosses_a_wall() {
        // A wall directly bisecting the straight line from (50,50) to (950,50) at x=500.
        let walls = vec![crate::scene::vision::Seg {
            a: (500.0, -100.0),
            b: (500.0, 200.0),
        }];
        let outcome = crate::scene::pathfinding::PathOutcome {
            path: vec![(50.0, 50.0), (950.0, 50.0)],
            cost: 900.0,
            arrested: false,
        };
        let clipped = clip_to_visible_mask(outcome, None, 100.0, 0.1, &walls);
        let last = *clipped.path.last().unwrap();
        assert!(
            last.0 <= 500.0 + 1e-6,
            "a chord crossing a wall must truncate before the wall, last x = {}",
            last.0
        );
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p shadowcat --lib scene::navmesh::tests -- --nocapture`
Expected: FAIL with "cannot find function `clip_to_visible_mask`" (compile error).

- [ ] **Step 4: Implement `clip_to_visible_mask`**

In `src/server/src/scene/navmesh.rs`, add after `navmesh_find`:

```rust
/// Arc-length-samples `outcome.path` and truncates it at the first sample whose chord (from the
/// previous retained sample) either (a) touches a cell outside `mask` (footprint disc ∪ the
/// step's supercover) or (b) crosses a `blocksMove` wall. `mask: None` skips check (a) — the
/// SAME per-cell predicate `pathfinding::cell_enterable` applies, no forked visibility decision,
/// so a continuous preview is fog-safe and `route ⊆ gate-allowed` holds across both engines
/// (parent spec §6.3). Check (b) always runs, independent of `mask` — this is a router-fidelity
/// guarantee (walls are public geometry, not a secrecy concern; Buddy-check finding 2026-07-02,
/// converged): the navmesh's true polyline may detour around a wall corner, but once downsampled
/// to at most `MAX_VISION_SAMPLES` arc-length samples, a chord between two samples straddling that
/// corner could otherwise cross the wall the true route avoided. `mask: None` and `walls: &[]`
/// together ⇒ returned unchanged.
///
/// A zero/one-sample truncation (the very first sample already fails a check) yields a
/// single-point path at `outcome.path[0]` with `cost: 0.0` — the caller (dispatch, Task 8)
/// is responsible for treating a degenerate result as appropriate for its context.
pub(crate) fn clip_to_visible_mask(
    outcome: crate::scene::pathfinding::PathOutcome,
    mask: Option<&std::collections::BTreeSet<crate::scene::pathfinding::Cell>>,
    cell: f64,
    footprint_radius_cells: f64,
    walls: &[crate::scene::vision::Seg],
) -> crate::scene::pathfinding::PathOutcome {
    if outcome.path.len() < 2 {
        return outcome;
    }
    if mask.is_none() && walls.is_empty() {
        return outcome;
    }

    let r_scene = footprint_radius_cells.max(0.0) * cell;
    // Dummy duration: `sample_path` is a time/arc-length sampler shared with `MoveStream`
    // playback; only `.pos` is used here, so the duration value is immaterial.
    let samples = crate::scene::move_stream::sample_path(&outcome.path, cell, 1.0);

    let mut truncated: Vec<(f64, f64)> = vec![samples[0].pos];
    let mut prev = samples[0].pos;
    for s in samples.iter().skip(1) {
        let mask_ok = match mask {
            None => true,
            Some(mask) => {
                let to_cell = ((s.pos.0 / cell).floor() as i32, (s.pos.1 / cell).floor() as i32);
                let footprint =
                    crate::scene::pathfinding::footprint_cells(to_cell, s.pos, r_scene, cell);
                footprint.iter().all(|c| mask.contains(c))
                    && match crate::scene::movement::supercover_cells(prev, s.pos, cell) {
                        Some(cells) => cells.iter().all(|c| mask.contains(c)),
                        None => false, // fail-closed: a degenerate/over-cap span truncates here
                    }
            }
        };
        let wall_ok = !walls
            .iter()
            .any(|w| crate::scene::segments_cross(prev, s.pos, w.a, w.b));
        if !mask_ok || !wall_ok {
            break;
        }
        truncated.push(s.pos);
        prev = s.pos;
    }

    // Recompute cost as the Euclidean length of the truncated polyline (the original `cost`
    // is only valid for the full, untruncated route).
    let new_cost: f64 = truncated
        .windows(2)
        .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
        .sum();

    crate::scene::pathfinding::PathOutcome {
        path: truncated,
        cost: new_cost,
        arrested: outcome.arrested,
    }
}
```

(`PathOutcome::clone()` used by these tests is already available — `Clone` was added to its derive list in Task 5 Step 3.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p shadowcat --lib scene::navmesh -- --nocapture`
Expected: PASS (all tests in the module).

- [ ] **Step 6: Run the full server suite + clippy**

Run: `cargo test -p shadowcat --lib && cargo clippy --all-targets -- -D warnings`
Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add src/server/src/scene/pathfinding.rs src/server/src/scene/navmesh.rs
git commit -m "feat(m10f-1): cell-sampled visibility post-filter — fog-safe continuous preview"
```

---

## Task 8: Dispatch — wire `movementModel` into `SceneEcs::pathfind`

**Files:**
- Modify: `src/server/src/scene/mod.rs`

**Interfaces:**
- Consumes: `resolve_scene(scene).movement_model` (Task 2), `navmesh_for` (Task 6), `navmesh::navmesh_find` (Task 5), `navmesh::clip_to_visible_mask` (Task 7).
- Produces: no new public interface — `pathfind`'s existing signature/return type is unchanged; only its internal dispatch changes.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `src/server/src/scene/mod.rs` (near the existing `pathfind_*` tests):

```rust
    #[test]
    fn pathfind_dispatches_to_the_navmesh_router_for_a_continuous_scene() {
        let mut ecs = SceneEcs::from_documents(
            vec![entity_doc_top(
                10,
                "scene",
                json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                        "vision": { "movementModel": "continuous" } }),
            )],
            0,
        );
        ecs.set_world_settings_for_test(json!({
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
        }));
        let outcome = ecs
            .pathfind(
                Uuid::from_u128(1),
                Uuid::from_u128(10),
                (50.0, 50.0),
                &[(950.0, 50.0)],
                0.1,
                true, // GM: unrestricted mask
                None,
            )
            .expect("continuous route over an open bounded scene");
        // Euclidean straight line ≈ 900, unlike a grid diagonal-rule cost — proves the navmesh
        // path was actually taken, not the grid router.
        assert!(
            (outcome.cost - 900.0).abs() < 2.0,
            "expected ~900 (Euclidean), got {}",
            outcome.cost
        );
    }

    #[test]
    fn pathfind_grid_stepped_scene_is_byte_for_byte_unchanged() {
        // Same fixture/assertions as the existing `pathfind_gm_unconstrained_routes_without_a_mask`
        // test, proving the default (grid-stepped) dispatch branch is untouched by this checkpoint.
        let (ecs, _user, scene) = scene_with_lit_player_token();
        let r = ecs.pathfind(
            Uuid::from_u128(1),
            scene,
            (50.0, 50.0),
            &[(250.0, 50.0)],
            0.1,
            true,
            None,
        );
        let outcome = r.expect("GM route");
        assert!((outcome.cost - 2.0).abs() < 1e-9, "grid Chebyshev cost unchanged");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p shadowcat --lib pathfind_dispatches_to_the_navmesh_router -- --nocapture`
Expected: FAIL — `pathfind` currently always calls `pathfinding::find`, which returns cost in grid CELLS, not scene-pixel units (confirmed by `pathfinding.rs`'s existing tests, e.g. a 200-scene-unit horizontal move at cell=100 costs `2.0`). For this test's (50,50)→(950,50) move at cell=100, the pre-fix grid dispatch gives cost ≈9.0 (9 cells), clearly distinct from the ≈900 (Euclidean scene units) the test asserts — this is exactly why the test cleanly proves dispatch occurred, not a coincidental near-miss.

- [ ] **Step 3: Implement the dispatch branch**

In `src/server/src/scene/mod.rs`, in `pathfind` (around line 897-951), replace the tail of the function (from the mask-building `let mask = ...` through the final `pathfinding::find(...)` call) with a dispatch on `movement_model`:

```rust
    #[allow(clippy::too_many_arguments)]
    pub fn pathfind(
        &self,
        user: Uuid,
        scene: Uuid,
        start: (f64, f64),
        waypoints: &[(f64, f64)],
        footprint_radius: f64,
        is_gm: bool,
        explored: Option<&crate::scene::explored::ExploredSet>,
    ) -> Result<pathfinding::PathOutcome, pathfinding::PathFail> {
        let cell = self
            .scene_grid_sizes()
            .get(&scene)
            .copied()
            .unwrap_or(100.0);
        let rule = self.resolved_diagonal_rule();
        let walls = self.move_walls(scene);
        let settings = self.resolve_scene(scene);

        // Build the per-(user,scene) mask (None ⇒ unconstrained). Shared by both engines —
        // §13/§6.3: never fork the per-cell visibility decision.
        let mask: Option<std::collections::BTreeSet<pathfinding::Cell>> = if is_gm {
            None
        } else {
            match settings.movement_restriction {
                MovementRestriction::Unrestricted => None,
                MovementRestriction::Visible => {
                    Some(self.visible_cells(user, scene, settings.partial_cell_leniency))
                }
                MovementRestriction::Revealed => {
                    let mut m = self.visible_cells(user, scene, settings.partial_cell_leniency);
                    if let Some(ex) = explored {
                        m.extend(ex.iter());
                    }
                    Some(m)
                }
            }
        };

        match settings.movement_model {
            MovementModel::GridStepped => {
                // Per-requester region field (spec §4): GM (or `is_gm`) sees the authoritative
                // field; a non-GM requester's field silently omits any region they cannot see, so
                // a secret region never influences their route or budget (it "springs" only at
                // execution, `move_exec`, which always reads the authoritative field).
                let regions = self.region_field(scene, if is_gm { None } else { Some(user) });
                pathfinding::find(
                    start,
                    waypoints,
                    footprint_radius,
                    cell,
                    rule,
                    &walls,
                    mask.as_ref(),
                    Some(&regions),
                )
            }
            MovementModel::Continuous => {
                // M10f-1: navmesh carries walls only (no regions yet — M10f-4); the cell-sampled
                // post-filter is the ONLY visibility gate, reusing the same mask as the grid path.
                let nav = self
                    .navmesh_for(scene, footprint_radius)
                    .ok_or(pathfinding::PathFail::Unreachable)?;
                let raw = navmesh::navmesh_find(&nav, start, waypoints)?;
                let clipped = navmesh::clip_to_visible_mask(
                    raw,
                    mask.as_ref(),
                    cell,
                    footprint_radius,
                    &walls,
                );
                if clipped.path.len() < 2 {
                    return Err(pathfinding::PathFail::Unreachable);
                }
                Ok(clipped)
            }
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat --lib pathfind -- --nocapture`
Expected: PASS (both new tests, plus every pre-existing `pathfind_*` test unchanged).

- [ ] **Step 5: Run the full server suite + clippy**

Run: `cargo test -p shadowcat --lib && cargo test -p shadowcat --all && cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings`
Expected: all green — this is the full CI-equivalent server gate.

- [ ] **Step 6: Re-measure the binary-size delta**

Run the same commands as Task 1 Step 5. Confirm the release binary is still under `62914560` bytes. Record the final delta.

- [ ] **Step 7: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "feat(m10f-1): dispatch SceneEcs::pathfind on movementModel (grid A* vs navmesh)"
```

---

## Task 9: Client — `movementModel` editor in `GameSettingsPanel`

**Files:**
- Modify: `src/modules/game-settings/src/GameSettingsPanel.svelte`
- Modify: `src/modules/game-settings/src/world-defaults.test.ts`
- Modify: `src/modules/game-settings/src/scene-overrides.test.ts`
- Modify: `src/client/ui-kit/src/locales/en.ts`

**Interfaces:**
- Consumes: `MovementModel` type + `movementModel` resolution (Task 3).

- [ ] **Step 1: Write the failing tests**

Add to `src/modules/game-settings/src/world-defaults.test.ts` (inside the existing `describe("world defaults editor", ...)` block):

```ts
  it("changing movement model dispatches a JSON-pointer update", async () => {
    const dispatchIntent = vi.fn();
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.movementModel") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "continuous" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "ws1", changes: [{ path: "/system/scene/movementModel", old: null, new: "continuous" }] },
    ]);
  });
```

Add to `src/modules/game-settings/src/scene-overrides.test.ts` (inside the existing `describe("per-scene overrides", ...)` block), mirroring the file's existing `movementRestriction` scene-override tests exactly:

```ts
  it("setting movement model override writes to the selected scene doc", async () => {
    const dispatchIntent = vi.fn();
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    const scene = buildSceneDoc("w1", {}, "scene1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws, scene), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.scene.movementModel") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "continuous" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "scene1", changes: [{ path: "/system/vision/movementModel", old: null, new: "continuous" }] },
    ]);
  });

  it("selecting inherit on a previously-set movementModel override dispatches null to clear it", async () => {
    const dispatchIntent = vi.fn();
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    // Pre-populate the scene with a movementModel override already set.
    const scene = buildSceneDoc("w1", { vision: { movementModel: "continuous" } }, "scene1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws, scene), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.scene.movementModel") as HTMLSelectElement;
    // The control reflects the current override ("continuous"); selecting "" clears it to null.
    await fireEvent.change(sel, { target: { value: "" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "scene1", changes: [{ path: "/system/vision/movementModel", old: null, new: null }] },
    ]);
  });
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/module-game-settings test`
Expected: FAIL (`getByLabelText("gameSettings.movementModel")` not found).

- [ ] **Step 3: Add the i18n keys**

In `src/client/ui-kit/src/locales/en.ts`, add after `"gameSettings.movementRestriction": "Movement restriction",`:

```ts
  "gameSettings.movementModel": "Movement model",
```

and after `"gameSettings.scene.movementRestriction": "Movement restriction (override)",`:

```ts
  "gameSettings.scene.movementModel": "Movement model (override)",
```

- [ ] **Step 4: Add the world-default editor**

In `src/modules/game-settings/src/GameSettingsPanel.svelte`, add a new constant near `MOVEMENT` (line 59):

```ts
  const MOVEMENT = ["visible", "revealed", "unrestricted"] as const;
  const MOVEMENT_MODEL = ["grid-stepped", "continuous"] as const;
```

Add a new `<label>` block immediately after the existing movement-restriction world-default `<label>` (after line 105, before the `lightingEnabled` label):

```svelte
    <label>
      {ctx.t("gameSettings.movementModel")}
      <select aria-label="gameSettings.movementModel" value={wsys.scene.movementModel}
        onchange={(e) => set(ws.id, "/system/scene/movementModel", (e.currentTarget as HTMLSelectElement).value)}>
        {#each MOVEMENT_MODEL as m}<option value={m}>{m}</option>{/each}
      </select>
    </label>
```

- [ ] **Step 5: Add the scene-override editor**

In `src/modules/game-settings/src/GameSettingsPanel.svelte`, add a new `<label>` block immediately after the existing movement-restriction scene-override `<label>` (after line 229, before the `losRestriction` scene-override label):

```svelte
      <label>
        {ctx.t("gameSettings.scene.movementModel")}
        <select aria-label="gameSettings.scene.movementModel"
          value={ssys.vision?.movementModel ?? ""}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLSelectElement).value;
            setScene("/system/vision/movementModel", v === "" ? null : v);
          }}>
          <option value="">{ctx.t("gameSettings.inherit")}</option>
          {#each MOVEMENT_MODEL as m}<option value={m}>{m}</option>{/each}
        </select>
      </label>
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/module-game-settings test`
Expected: PASS.

- [ ] **Step 7: Run the full client gate**

Run: `pnpm -r typecheck && pnpm lint && pnpm -r test`
Expected: all green.

- [ ] **Step 8: Commit**

```bash
git add src/modules/game-settings/src/GameSettingsPanel.svelte src/modules/game-settings/src/world-defaults.test.ts src/modules/game-settings/src/scene-overrides.test.ts src/client/ui-kit/src/locales/en.ts
git commit -m "feat(m10f-1): movementModel world-default + scene-override editor"
```

---

## Task 10: Client — disable route commit in continuous scenes (preview stays live)

**Files:**
- Modify: `src/modules/scene-tools/src/controller.svelte.ts`
- Modify: `src/modules/scene-tools/src/measure-tool.test.ts`

**Interfaces:**
- Consumes: `resolveSceneSettings` (existing, `@shadowcat/core`), `MovementModel` (Task 3).

- [ ] **Step 1: Extend the `seedRouteCtx` fixture with an optional scene-vision override**

In `src/modules/scene-tools/src/measure-tool.test.ts`, the `seedRouteCtx` helper (around line 311-336) currently builds its scene doc with a fixed `grid` system and no `vision` override. Extend its `over` parameter and scene-doc construction:

```ts
function seedRouteCtx(over: {
  pathfind: ToolContext["pathfind"];
  moveRequest?: ToolContext["moveRequest"];
  dispatchIntent?: (ops: WireOperation[]) => void;
  animateAlongPath?: (id: string, path: [number, number][]) => void;
  onClearOverlay?: () => void;
  onClearMeasure?: () => void;
  tokenAt: { id: string; x: number; y: number };
  /** Scene-level vision overrides (M10f-1: movementModel). Absent ⇒ grid-stepped default. */
  sceneVision?: { movementModel?: "grid-stepped" | "continuous" };
}): { ctx: ToolContext; now: FakeNow; docs: DocumentStore } {
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      {
        op: "create",
        doc: buildSceneDoc("w1", {
          grid: { kind: "square", size: 100, distance: { perCell: 5, unit: "ft" } },
          ...(over.sceneVision ? { vision: over.sceneVision } : {}),
        }, "s1"),
      },
      {
        op: "create",
        doc: buildTokenDoc("w1", "s1", {
          x: over.tokenAt.x, y: over.tokenAt.y, w: 100, h: 100, rotation: 0,
          visual: { kind: "image", asset: "a" },
        }, over.tokenAt.id),
      },
    ],
  });
```

(The rest of `seedRouteCtx`'s body — the sequence counter, `now`, `bridge`, `sel`, `defaultDispatch`, the returned `ctx` object, and the closing `return { ctx, now, docs };` — is unchanged.)

- [ ] **Step 2: Write the failing test**

Add to `src/modules/scene-tools/src/measure-tool.test.ts`, immediately after the existing `test("double-click commits via moveRequest (animation is broadcast-driven)", ...)` test (after line 397):

```ts
test("commitRoute does nothing in a continuous-movement-model scene (double-click is a no-op)", async () => {
  const moves: Array<{ tokenId: string; path: [number, number][] }> = [];
  const moveRequest: ToolContext["moveRequest"] = async (_s, tokenId, path) => {
    moves.push({ tokenId, path });
    return { requestId: "r1", tokenId, mover: "u1", scene: "s1", startServerMs: 0, durationMs: 300, stop: path.at(-1)!, samples: [], moverVision: null, cost: 1 };
  };
  const { ctx, now } = seedRouteCtx({
    pathfind: async () => ({ path: [[0, 0], [100, 0], [100, 100]] as [number, number][], cost: 2, arrested: false }),
    moveRequest,
    tokenAt: { id: "tok1", x: 0, y: 0 },
    sceneVision: { movementModel: "continuous" },
  });
  const tool = makeMeasureTool(ctx);
  tool.onPointerDown({ x: 100, y: 100 }, ev()); tool.onPointerUp({ x: 100, y: 100 }, ev());
  now.advance(100);
  tool.onPointerDown({ x: 100, y: 100 }, ev()); tool.onPointerUp({ x: 100, y: 100 }, ev());
  await drain();
  expect(moves).toEqual([]);
});
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `pnpm --filter @shadowcat/module-scene-tools test -- measure-tool`
Expected: FAIL (moveRequest IS called today — continuous scenes have no gate yet).

- [ ] **Step 4: Add the gate**

In `src/modules/scene-tools/src/controller.svelte.ts`, add `resolveSceneSettings` to the existing `@shadowcat/core` import (line 6):

```ts
import { rectPoints, ellipsePoints, circlePoints, conePoints, squarePoints, parseColor, type SceneTool, type Point } from "@shadowcat/render";
import { buildTokenDoc, buildTokenFromActor, buildSceneEntityDoc, resolveTokenBox, resolveTokenActor, footprintRadius, buildRegionDoc, setRegionVisibility, resolveSceneSettings, type ReadableDocuments, type AssetResolver, type WireOperation, type PathResult, type MoveStream } from "@shadowcat/core";
```

In `commitRoute` (line 422), add the gate as the first check:

```ts
  function commitRoute(goal: Point): void {
    if (!ctx.pathfind || !ctx.moveRequest || !ctx.tokenSelection || ctx.tokenSelection.ids.size !== 1) return;
    const sceneDoc = ctx.documents.query("scene")[0];
    if (sceneDoc && resolveSceneSettings(sceneDoc, ctx.documents).movementModel === "continuous") {
      // Continuous-scene execution lands in M10f-3; the router + preview already work (§9 of the
      // M10f-1 design doc), but committing a route here would need the M10f-2 unified sampled
      // executor, which does not exist yet. No grid-snap fallback — clear silence, not a fake move.
      return;
    }
    const scene = activeScene(ctx);
    const start = tokenCenter();
    if (!scene || !start) return;
    // ...unchanged...
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `pnpm --filter @shadowcat/module-scene-tools test -- measure-tool`
Expected: PASS. Also re-run the full file to confirm no existing route-mode test regressed (a grid-stepped scene, the default, must still commit normally):

Run: `pnpm --filter @shadowcat/module-scene-tools test`
Expected: all green.

- [ ] **Step 6: Run the full client gate**

Run: `pnpm -r typecheck && pnpm lint && pnpm -r test`
Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add src/modules/scene-tools/src/controller.svelte.ts src/modules/scene-tools/src/measure-tool.test.ts
git commit -m "feat(m10f-1): disable route commit in continuous scenes (preview unaffected, execution is M10f-3)"
```

---

## Post-implementation gates (not a task — required before merge)

1. **Full three-OS CI equivalent locally where feasible:** `pnpm build && cd src/server && cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test --all && cargo build --release` — confirms the embed-ordering invariant (client built before `cargo`) and the release binary still passes the 60 MiB budget.
2. **Reviewed skill-update gate (CLAUDE.md):** update `shadowcat-codebase-scene-rendering` — new `MovementModel` axis + `pathfind` dispatch branch, `scene/navmesh.rs` seam (construction/query/cache/invalidation/cell-sampled post-filter), the `movementModel` client resolver + editor, and the `commitRoute` continuous-scene gate. Dispatch `shadowcat-spec-reviewer` to confirm the skill diff is accurate before merge.
3. **Whole-branch review** (per M10 cadence): dispatch the two-reviewer pair (`shadowcat-code-reviewer` + `shadowcat-spec-reviewer`) or the pre-authorized buddy check (see directives below) over the full branch diff before merging to local `main` (`--no-ff`, not pushed — push gate is the full M10f milestone per the M10 convention).
4. **Update `docs/PLAN.md`**: mark M10f-1 DONE with the same level of detail as M10f-0's entry, and set "Next = M10f-2".

## Model/Effort directives

- Plan-writer: **mainline continuation** (chosen over dispatching `sdd-plan-writer-sonnet`/`sdd-plan-writer-opus`) — Sonnet 5, effort high (session default, unchanged for this plan-writing turn).
- Dispatcher: **mainline** (chosen over dispatching `sdd-dispatcher` to own the full loop) — this session runs the SDD dispatch loop directly. Recommended tier per `sdd-model-effort-tiers.md` is Sonnet/low for this role; the session's actual effort is whatever `/effort` is currently set to (left to the human's control, not overridden here). Implementer/per-task-reviewer/final-reviewer roles are never a mainline choice — always the named `sdd-*` subagents per CLAUDE.md.

## Buddy-check directives

- **Plan buddy check: DONE 2026-07-02, findings folded in.** Two independent reviewers (PHASE=spec, packet = this plan vs. the approved design doc + the real codebase) converged after 3 debate rounds. Agreed findings (all fixed inline in this document):
  - **[Critical]** `navmesh_find`/`build_navmesh` had no input-validation parity with `pathfinding::find` (unbounded waypoints, unbounded/negative footprint radius, non-finite coordinates) — a real DoS/panic surface on the untrusted `Pathfind` wire request. Fixed: Task 4 now caps `footprint_radius_cells` at `MAX_FOOTPRINT_CELLS`; Task 5's `navmesh_find` now caps `waypoints.len()` at `MAX_WAYPOINTS` and rejects non-finite coordinates — reusing the grid engine's exact constants.
  - **[Important]** Cache keyed on exact f64 bits, contradicting the design's explicit "quantized footprintRadius... cache stays bounded" (§5.2). Fixed: Task 6 now quantizes to the nearest 1/1000 cell.
  - **[Important]** Design §9's explicit "goal outside the mask ⇒ Unreachable" test was missing. Fixed: added to Task 7.
  - **[Important]** The design doc's own §4.1/§8 ts-rs instruction is stale (contradicted by real `movementRestriction` precedent, which the plan correctly followed instead) — now noted as an explicit erratum in Global Constraints so the discrepancy isn't silently overridden.
  - **[Important]** `polyanya::Mesh: Send + Sync` was unverified until Task 6, several commits after the dependency is added. Fixed: a compile-time assertion added to Task 1's smoke test.
  - **[Important]** `PathOutcome: Clone` (required by Task 7's tests) was only an informal prose "Note," not a real step. Fixed: promoted to Task 5 Step 3 with an explicit diff.
  - **[Important, converged after a 3-round debate]** The post-sample chord in `clip_to_visible_mask` was tested against the visibility mask only, never against walls — for routes long enough to force undersampling (>~32 cells), a chord straddling a sharp corner could visually cross a wall the true navmesh route avoided. Reviewer B's initial "fog-leak" framing was refuted (the chord IS what's tested and IS what's transmitted, so the secrecy invariant was never actually at risk) and reframed by Reviewer A as a router-fidelity issue; both reviewers converged on this scoping. Fixed: `clip_to_visible_mask` now also tests each chord against `walls` via `segments_cross`, independent of the mask check.
  - **[Minor, agreed]** `navmesh_for`'s cache-miss path is a benign (non-atomic but correctness-safe) build race under concurrency — noted as an accepted tradeoff in Task 6's doc comment. Task 8's guidance text asserted the wrong pre-fix grid cost (~900 instead of the actual ~9 cells) — corrected.
  - **Unresolved (Minor, left as-is):** no fallback guidance for a Cargo dependency-*resolution* failure distinct from an API-shape mismatch (held: routine Rust troubleshooting, not worth pre-scripting); a documentation-precision nit in Task 8's splice-boundary prose (held: zero execution risk). Both were debated once and held with substantive reasoning; not pursued further given Minor severity.
- Flagged tasks: **4, 5, 6, 7** — pre-authorized to replace both single-reviewer review stages with a buddy check during execution.
- Unflagged tasks showing risk signals during execution: **ask** the human before upgrading.
