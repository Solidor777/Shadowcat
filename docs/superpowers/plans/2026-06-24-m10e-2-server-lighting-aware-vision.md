# M10e-2 — Server Lighting-Aware Vision Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task (fresh `shadowcat-coder` per task, per-task two-reviewer gate). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Compute a per-`(user, scene)` lighting-aware grid visibility mask on the server (LOS ∩ (lit ∨ darkvision)) and emit it, secrecy-safe, in the existing per-player vision frame — the reusable engine core that M10e-3 (client render), M10e-4 (movement restriction) and M10e-6 (pathfinder) all consume.

**Architecture:** Hydrate the three world config-docs (`world-settings`/`light-gradation`/`vision-modes`) and actors into `SceneEcs` as in-memory side-tables, so the whole mask computation is **pure and synchronous under the existing scene read-lock** — no per-dispatch repo I/O on the vision path. A new pure `scene/lighting.rs` module owns illumination (radial falloff + `blocksLight` occlusion via the existing M9 raycast + max-compose + gradation banding). `compute_derived("vision", …)` is extended to attach a per-scene `lit` cell mask (each cell tagged with its illumination band + tint) to the masked payload, **additively** — `polygons` and `explored` are unchanged, so this checkpoint ships with zero behavior regression while the client still consumes polygons until e-3.

**Tech Stack:** Rust (server), `hecs` ECS, `serde_json` (opaque `system` bodies, structural-only per ARCHITECTURE #6), clean-room computational geometry.

## Global Constraints

- **Server stays structural-only (ARCHITECTURE #6):** `system` bodies are opaque JSON; the server reads named fields via `serde_json::Value::pointer`, never imposes a typed schema on documents. Mirror the client shapes in `src/client/core/src/scene-docs.ts` (the single source of truth for these `system` shapes).
- **Secrecy gate, fail-closed (`fog-is-the-secrecy-gate-fail-closed`):** a missing/garbled/incomplete config or geometry signal must *under-reveal* (hide more), never over-reveal. The `lit` mask carries ONLY currently-visible cells; illumination outside the mask is never serialized.
- **Server-authoritative vision, no client prediction (ARCHITECTURE §2 inv. 3):** the mask is computed from authoritative ECS state, never client-claimed positions.
- **Clean-room geometry (ARCHITECTURE §7):** standard radial light falloff + threshold banding + the existing angular-sweep raycast; cite sources, consult no proprietary VTT/engine source.
- **Cross-platform:** pure-Rust, no OS-specific paths; the server matrix (ubuntu/macos/windows) is the proof.
- **Determinism:** all per-cell scans iterate in a fixed order (ascending `(i, j)`); no `HashMap` iteration leaks into wire output ordering.
- **Local gate (run before each commit):** `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p shadowcat`. (NOTE: the server crate is named **`shadowcat`** — every `-p shadowcat-server` in this plan's task commands should read `-p shadowcat`; corrected during execution.)

## Decisions baked into this plan

1. **Data access = hydrate into `SceneEcs`** (user-confirmed, 2026-06-24). Config-docs + actors live as in-memory side-tables; the mask is pure over in-memory state. Rationale: e-4's movement gate runs inside `Room::publish` under `publish_guard` and must read the mask synchronously without async repo reads serializing all world writes; e-6's pathfinder is likewise hot. Per-dispatch repo reads were rejected as a shoddy foundation the spec flagged ("settle caching in the plan", §8).
2. **Additive wire shape.** `polygons` (crisp LOS edges) and `explored` are unchanged — the existing client fog keeps working until e-3. The new secrecy-safe `lit` mask is plumbed ahead of its consumer. No NEW leak vs M9 (the pre-existing LOS-polygon delivery is unchanged; e-3 closes the unlit-LOS-shape gap by switching the client to the `lit` mask).
3. **GM unchanged.** GM vision stays `{ "mode": "all" }` (sees everything, fully lit). Lighting gating applies only to the masked (non-GM) path.
4. **Illumination is continuous [0,1]; bands are applied by thresholding.** `max`-compose across contributors (no over-brightening, §6); tint = the dominant (max) contributor's packed-RGB color. Smooth color blending and soft edges are the client's V3 job; the server mask stays cell-granular and authoritative.
5. **`observerVision` reuses the existing `DocRole::Observer` tier** (no new tier — spec §5.6 confirmed): a vision source is a token the user owns OR (when `observerVision` is on) holds `DocRole::Observer`-or-better on.

### Constraint-driven deviation from spec (must log to `docs/TODO.md` at completion)

**Spec §6/§12.5 specifies environment light as *edge-projected and occludable by `blocksLight`*. This plan implements it as a flat scene-wide ambient floor instead.** Reason: the scene model is **dimensionless** (`SceneSystem` in scene-docs.ts — "Dimensions deferred (canvas pans freely)"); there is no scene boundary to project edge light from, so edge-projection is undefined until scene dimensions exist. Flat ambient is the best-defined behavior under that constraint and is inert by default (`environment.intensity` defaults to 0.0). Consequence: while `env.intensity > 0` (GM daylight), a `blocksLight`-sealed interior is NOT darkened by occlusion — placed lights still work and occlude normally; only the *ambient* term is non-occludable. **TODO (log at completion): implement edge-projected, `blocksLight`-occludable environment light once scene dimensions land.** This is a constraint-forced deferral, not a descope of placed-light occlusion (which IS implemented).

## File Structure

- **Create `src/server/src/scene/lighting.rs`** — pure illumination + gradation. `Band`, `Light`, `Falloff`, `CellLight`; `default_bands`/`sorted_bands`/`band_index`/`floor_min`; `light_illumination`; `cell_illumination`. No I/O, no document parsing (callers pass parsed structs).
- **Modify `src/server/src/scene/vision.rs`** — promote a non-test `pub(crate) fn point_in_poly` (reused by lighting occlusion + cell tests). The existing private test copy is removed in favor of it.
- **Modify `src/server/src/scene/mod.rs`** — `pub mod lighting;`; `SceneEcs` side-tables (`world_settings`/`gradation`/`vision_modes`: `Option<Document>`, `actors: HashMap<Uuid, Document>`) + `set_world_config`/`set_actors`; `apply_op` maintenance of those tables; server resolvers (`resolve_scene_settings`, `resolved_bands`, `resolved_vision_modes`, `token_vision_floors`); ECS accessors (`scene_lights`, `light_walls`); the mask computation `player_lit_mask`; the extended `compute_derived("vision", …)` arm.
- **Modify `src/server/src/ws/room.rs`** — `get_or_create` fetches actors + the 3 config-docs and seeds them into the `SceneEcs` via the new setters, alongside the existing scene-entity hydration.
- **Modify `src/server/tests/scene_derived.rs`** — integration: end-to-end vision frame over a lit scene asserts the `lit` mask (bands, darkvision, fail-closed dark scene).

`src/server/src/ws/conn.rs` is **not modified**: the new `lit` data is produced inside `compute_derived` and flows through the existing `SceneDerived` serialization untouched; `enrich_vision_explored` continues to touch only `mode`/`polygons`/`explored`.

---

### Task 1: Gradation bands (pure)

**Files:**
- Create: `src/server/src/scene/lighting.rs`
- Modify: `src/server/src/scene/mod.rs` (add `pub mod lighting;` near `pub mod explored;`)

**Interfaces:**
- Produces: `Band { name: String, min_illumination: f64 }`; `fn default_bands() -> Vec<Band>`; `fn sorted_bands(Vec<Band>) -> Vec<Band>`; `fn band_index(&[Band], f64) -> usize`; `fn floor_min(&[Band], &str) -> f64`.

- [ ] **Step 1: Write the failing test**

In `src/server/src/scene/lighting.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_lookup_and_floor_are_fail_closed() {
        let bands = sorted_bands(default_bands());
        // brightest-first: bright(0.67) → dim(0.34) → dark(0.0)
        assert_eq!(bands[0].name, "bright");
        assert_eq!(band_index(&bands, 0.9), 0); // bright
        assert_eq!(band_index(&bands, 0.5), 1); // dim
        assert_eq!(band_index(&bands, 0.1), 2); // dark
        // floor_min: a normal-vision token (dim floor) needs >= 0.34; darkvision (dark) needs >= 0.0.
        assert_eq!(floor_min(&bands, "dim"), 0.34);
        assert_eq!(floor_min(&bands, "dark"), 0.0);
        // Unknown floor name → most restrictive (brightest band min) = under-reveal.
        assert_eq!(floor_min(&bands, "nonsense"), 0.67);
        // Empty input → defaults (never panics).
        assert_eq!(sorted_bands(vec![])[0].name, "bright");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat-server lighting::tests::band_lookup_and_floor_are_fail_closed`
Expected: FAIL — `lighting` module / symbols not found.

- [ ] **Step 3: Write minimal implementation**

At the top of `src/server/src/scene/lighting.rs`:

```rust
//! Illumination field + gradation banding (M10e-2). Pure, engine-owned (ARCHITECTURE #6),
//! server-authoritative (#3). Clean-room: standard radial light falloff plus threshold banding of a
//! continuous [0,1] illumination field. No proprietary VTT/engine source consulted.
//!
//! Mirrors the client `light-gradation`/`light`/`vision-modes` shapes in scene-docs.ts; the server
//! stays structural-only (callers parse documents and pass these plain structs).

use crate::scene::vision::{point_in_poly, P};

/// A named illumination band. `min_illumination` is the minimum [0,1] light level a cell must reach
/// to qualify for this band. Mirrors the client `GradationBand`.
#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    pub name: String,
    pub min_illumination: f64,
}

/// Built-in three-band gradation (bright → dim → dark). Mirrors `DEFAULT_GRADATION` in scene-docs.ts.
pub fn default_bands() -> Vec<Band> {
    vec![
        Band { name: "bright".into(), min_illumination: 0.67 },
        Band { name: "dim".into(), min_illumination: 0.34 },
        Band { name: "dark".into(), min_illumination: 0.0 },
    ]
}

/// Bands sorted brightest-first (descending `min_illumination`). Fail-closed: empty input → defaults.
pub fn sorted_bands(mut bands: Vec<Band>) -> Vec<Band> {
    if bands.is_empty() {
        return default_bands();
    }
    bands.sort_by(|a, b| {
        b.min_illumination
            .partial_cmp(&a.min_illumination)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    bands
}

/// Index (brightest=0) of the band a given illumination falls into. `bands` MUST be brightest-first.
/// Clamps to the darkest band if nothing matched (defensive; the darkest floor is normally 0.0).
pub fn band_index(bands: &[Band], illumination: f64) -> usize {
    for (i, b) in bands.iter().enumerate() {
        if illumination >= b.min_illumination {
            return i;
        }
    }
    bands.len().saturating_sub(1)
}

/// Minimum illumination to perceive a cell at the named floor band. A token whose vision floor is
/// `floor_name` perceives a cell iff `illumination >= floor_min`. Fail-closed: an unknown floor
/// resolves to the brightest band's min (most restrictive → under-reveal).
pub fn floor_min(bands: &[Band], floor_name: &str) -> f64 {
    bands
        .iter()
        .find(|b| b.name == floor_name)
        .map(|b| b.min_illumination)
        .unwrap_or_else(|| bands.first().map(|b| b.min_illumination).unwrap_or(1.0))
}
```

Add `pub mod lighting;` beneath `pub mod explored;` in `src/server/src/scene/mod.rs`. This task references `vision::point_in_poly` (added in Task 2) only in later code; the `use` line will not compile until Task 2 promotes it — so for THIS task, temporarily omit the `use crate::scene::vision::{point_in_poly, P};` import and inline `pub type P = (f64, f64);` is NOT needed yet (no function here uses `P`). Replace the `use` line with nothing for Task 1; Task 2 adds it back when `cell_illumination` lands.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p shadowcat-server lighting::tests::band_lookup_and_floor_are_fail_closed`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/lighting.rs src/server/src/scene/mod.rs
git commit -m "feat(m10e-2): gradation bands + fail-closed band/floor lookup"
```

---

### Task 2: Promote `point_in_poly` + light falloff (pure)

**Files:**
- Modify: `src/server/src/scene/vision.rs` (promote `point_in_poly` to `pub(crate)`)
- Modify: `src/server/src/scene/lighting.rs` (add `Light`, `Falloff`, `light_illumination`)

**Interfaces:**
- Consumes: `vision::P`.
- Produces: `vision::point_in_poly(&[P], P) -> bool`; `lighting::Falloff { Linear, Quadratic, None }`; `lighting::Light { pos, color, intensity, bright_radius, dim_radius, falloff, enabled }`; `fn light_illumination(&Light, f64) -> f64`.

- [ ] **Step 1: Write the failing test**

In `src/server/src/scene/lighting.rs` `tests`:

```rust
    fn lamp() -> Light {
        Light {
            pos: (0.0, 0.0),
            color: 0xFFEEAA,
            intensity: 1.0,
            bright_radius: 2.0,
            dim_radius: 6.0,
            falloff: Falloff::Linear,
            enabled: true,
        }
    }

    #[test]
    fn falloff_curves_and_radii() {
        let l = lamp();
        assert_eq!(light_illumination(&l, 0.0), 1.0); // center: full
        assert_eq!(light_illumination(&l, 2.0), 1.0); // bright edge: full (continuous)
        assert_eq!(light_illumination(&l, 7.0), 0.0); // beyond dim radius: dark
        // Linear: halfway across (bright=2 → dim=6), dist=4 → t=0.5 → 0.5
        assert!((light_illumination(&l, 4.0) - 0.5).abs() < 1e-9);
        // Quadratic falls off faster than linear at the same distance.
        let q = Light { falloff: Falloff::Quadratic, ..lamp() };
        assert!(light_illumination(&q, 4.0) < light_illumination(&l, 4.0));
        // None: flat dim-band step across (bright, dim].
        let n = Light { falloff: Falloff::None, ..lamp() };
        assert!((light_illumination(&n, 4.0) - 0.5).abs() < 1e-9);
        assert_eq!(light_illumination(&n, 1.0), 1.0); // still full inside bright
        // Disabled / zero dim radius contribute nothing.
        assert_eq!(light_illumination(&Light { enabled: false, ..lamp() }, 0.0), 0.0);
        assert_eq!(light_illumination(&Light { dim_radius: 0.0, ..lamp() }, 0.0), 0.0);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat-server lighting::tests::falloff_curves_and_radii`
Expected: FAIL — `Light`/`Falloff`/`light_illumination` not found.

- [ ] **Step 3: Write minimal implementation**

In `src/server/src/scene/vision.rs`, promote the point-in-polygon test helper to a crate function (place it after `visibility_polygon`, and delete the duplicate `fn point_in_poly` inside `vision.rs`'s `mod tests` so the test module calls `super::point_in_poly`):

```rust
/// Even-odd ray-cast point-in-polygon. Source: standard CG (Shimrat 1962; de Berg et al.).
/// `poly` is a ring of vertices; `< 3` vertices ⇒ no area ⇒ false.
pub(crate) fn point_in_poly(poly: &[P], p: P) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let (px, py) = p;
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if ((yi > py) != (yj > py)) && (px < (xj - xi) * (py - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}
```

In `src/server/src/scene/lighting.rs`, restore the import line at the top (`use crate::scene::vision::{point_in_poly, P};`) and add:

```rust
/// Photometric falloff curve across the dim band `(bright_radius, dim_radius]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Falloff {
    /// Smooth linear taper from full intensity at the bright edge to 0 at the dim edge.
    Linear,
    /// Smooth quadratic taper (faster than linear).
    Quadratic,
    /// No gradient: a flat dim-band step (`0.5 × intensity`) — bright/dim radii feed the gradation
    /// bands directly (spec §5.4). With the default gradation this lands a unit-intensity light's
    /// dim band at 0.5 ∈ [dim 0.34, bright 0.67).
    None,
}

/// A placed light's photometric inputs. Radii are in GRID CELLS; `color` is packed `0xRRGGBB`.
/// Mirrors the client `LightSystem` (scene-docs.ts).
#[derive(Clone, Debug)]
pub struct Light {
    pub pos: P,
    pub color: u32,
    pub intensity: f64,
    pub bright_radius: f64,
    pub dim_radius: f64,
    pub falloff: Falloff,
    pub enabled: bool,
}

/// Illumination this light contributes at distance `dist_cells` (in CELLS), BEFORE occlusion.
/// Full `intensity` within `bright_radius`; tapers across `(bright_radius, dim_radius]` by the
/// curve; 0 beyond `dim_radius`. Disabled / non-positive `dim_radius` ⇒ 0.
pub fn light_illumination(light: &Light, dist_cells: f64) -> f64 {
    if !light.enabled || light.dim_radius <= 0.0 || dist_cells > light.dim_radius {
        return 0.0;
    }
    if dist_cells <= light.bright_radius {
        return light.intensity;
    }
    let span = (light.dim_radius - light.bright_radius).max(1e-9);
    let t = ((light.dim_radius - dist_cells) / span).clamp(0.0, 1.0); // 1 at bright edge → 0 at dim edge
    let f = match light.falloff {
        Falloff::None => 0.5,
        Falloff::Linear => t,
        Falloff::Quadratic => t * t,
    };
    light.intensity * f
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p shadowcat-server scene::lighting && cargo test -p shadowcat-server scene::vision`
Expected: PASS (both modules; the promoted `point_in_poly` keeps the existing vision tests green).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/vision.rs src/server/src/scene/lighting.rs
git commit -m "feat(m10e-2): light falloff + shared point_in_poly"
```

---

### Task 3: Per-cell illumination compose (pure)

**Files:**
- Modify: `src/server/src/scene/lighting.rs` (add `CellLight`, `cell_illumination`)

**Interfaces:**
- Consumes: `Light`, `vision::point_in_poly`.
- Produces: `CellLight { level: f64, tint: u32 }`; `fn cell_illumination(center: P, env_intensity: f64, env_color: u32, lights: &[Light], lit_polys: &[Vec<P>], cell_size: f64) -> CellLight`.

- [ ] **Step 1: Write the failing test**

In `lighting.rs` `tests`:

```rust
    #[test]
    fn cell_illumination_takes_max_and_respects_occlusion() {
        let l = lamp(); // at origin, bright 2 / dim 6 cells, intensity 1, linear
        // No env, cell at the light center, cell_size 100 (world units per cell) → full + light tint.
        let c = cell_illumination((0.0, 0.0), 0.0, 0x000000, &[l.clone()], &[vec![]], 100.0);
        assert_eq!(c.level, 1.0);
        assert_eq!(c.tint, 0xFFEEAA);
        // Environment ambient alone when no light reaches: env wins, env tint.
        let far = cell_illumination((10_000.0, 0.0), 0.3, 0x0A0E1A, &[l.clone()], &[vec![]], 100.0);
        assert_eq!(far.level, 0.3);
        assert_eq!(far.tint, 0x0A0E1A);
        // Max-compose: a brighter env beats a dim faraway light contribution.
        let near = cell_illumination((400.0, 0.0), 0.6, 0x0A0E1A, &[l.clone()], &[vec![]], 100.0); // 4 cells → 0.5
        assert_eq!(near.level, 0.6); // env 0.6 > light 0.5 (no over-brightening)
        // Occlusion: a light whose polygon excludes the cell contributes nothing.
        let occluded_poly = vec![(1000.0, 1000.0), (1001.0, 1000.0), (1001.0, 1001.0)]; // tiny, far away
        let occ = cell_illumination((0.0, 0.0), 0.0, 0x000000, &[l], &[occluded_poly], 100.0);
        assert_eq!(occ.level, 0.0); // cell center not inside the light's poly → dark
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat-server lighting::tests::cell_illumination_takes_max_and_respects_occlusion`
Expected: FAIL — `CellLight`/`cell_illumination` not found.

- [ ] **Step 3: Write minimal implementation**

In `lighting.rs`:

```rust
/// A composed per-cell illumination result: a [0,1] `level` and a packed-RGB `tint` (the dominant
/// contributor's color; `0x000000` when only an unset environment contributes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellLight {
    pub level: f64,
    pub tint: u32,
}

/// Compose illumination at a cell center from a flat environment ambient plus each light, taking the
/// MAX contributor (no over-brightening, spec §6); `tint` follows the dominant contributor.
/// `lit_polys[k]` is `lights[k]`'s `blocksLight` visibility polygon — a light contributes only if the
/// cell center lies inside it (an EMPTY polygon means "no occluder computed" → never occludes).
/// `cell_size` is world units per cell (light radii are in cells, so distance is divided by it).
pub fn cell_illumination(
    center: P,
    env_intensity: f64,
    env_color: u32,
    lights: &[Light],
    lit_polys: &[Vec<P>],
    cell_size: f64,
) -> CellLight {
    let mut best = CellLight { level: env_intensity.clamp(0.0, 1.0), tint: env_color };
    for (k, light) in lights.iter().enumerate() {
        // Occlusion: a non-empty polygon that excludes the cell center kills this light's reach here.
        if let Some(poly) = lit_polys.get(k) {
            if !poly.is_empty() && !point_in_poly(poly, center) {
                continue;
            }
        }
        let d = ((center.0 - light.pos.0).powi(2) + (center.1 - light.pos.1).powi(2)).sqrt();
        let dist_cells = if cell_size > 0.0 { d / cell_size } else { d };
        let level = light_illumination(light, dist_cells);
        if level > best.level {
            best = CellLight { level, tint: light.color };
        }
    }
    best
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p shadowcat-server lighting::tests::cell_illumination_takes_max_and_respects_occlusion`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/lighting.rs
git commit -m "feat(m10e-2): per-cell illumination compose with occlusion + max"
```

---

### Task 4: `SceneEcs` config + actor side-tables

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`SceneEcs` fields, constructors, setters, `apply_op` maintenance)

**Interfaces:**
- Produces: `SceneEcs::set_world_config(world_settings: Option<Document>, gradation: Option<Document>, vision_modes: Option<Document>)`; `SceneEcs::set_actors(Vec<Document>)`; `SceneEcs::actor(&Uuid) -> Option<&Document>`. `apply_op` now mirrors Create/Update/Delete of the three config doc_types and `actor` docs into these tables.

- [ ] **Step 1: Write the failing test**

In `mod.rs` `tests`:

```rust
    #[test]
    fn config_and_actor_side_tables_track_ops() {
        use serde_json::json;
        let mut ecs = SceneEcs::new();
        // Seed via setters (the room-hydration path).
        let mut ws = doc(100, None, "world-settings");
        ws.system = json!({ "scene": { "lightingEnabled": false } });
        ecs.set_world_config(Some(ws), None, None);
        ecs.set_actors(vec![entity_doc_top(200, "actor", json!({ "vision": [] }))]);
        assert!(ecs.actor(&Uuid::from_u128(200)).is_some());

        // A live Create of a vision-modes doc lands in the side table.
        ecs.apply_op(&Operation::Create { doc: doc(101, None, "vision-modes") });
        assert!(ecs.vision_modes_doc().is_some());

        // A field Update to the world-settings doc is mirrored.
        ecs.apply_op(&Operation::Update {
            doc_id: Uuid::from_u128(100),
            changes: vec![crate::data::command::FieldChange {
                path: "/system/scene/lightingEnabled".into(),
                old: json!(false),
                new: json!(true),
            }],
        });
        assert_eq!(
            ecs.world_settings_doc().unwrap().system.pointer("/scene/lightingEnabled"),
            Some(&json!(true))
        );

        // A Delete of the actor removes it.
        ecs.apply_op(&Operation::Delete { doc: doc(200, None, "actor") });
        assert!(ecs.actor(&Uuid::from_u128(200)).is_none());
    }
```

Add the top-level entity helper near the existing `doc`/`entity_doc` helpers in `mod.rs` `tests`:

```rust
    fn entity_doc_top(id: u128, ty: &str, system: serde_json::Value) -> Document {
        let mut d = doc(id, None, ty);
        d.system = system;
        d
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat-server scene::tests::config_and_actor_side_tables_track_ops`
Expected: FAIL — setters/accessors not found.

- [ ] **Step 3: Write minimal implementation**

In `SceneEcs` (struct fields), add:

```rust
    /// World config-docs (singletons) + actors, hydrated for the lighting-aware vision mask
    /// (M10e-2). Held outside the hecs `world` because they are NOT scene entities
    /// (`is_scene_entity` excludes them); they are maintained by `apply_op` and the room setters.
    world_settings: Option<Document>,
    gradation: Option<Document>,
    vision_modes: Option<Document>,
    actors: HashMap<Uuid, Document>,
```

Initialize all four in `SceneEcs::new()` (`None`, `None`, `None`, `HashMap::new()`).

Add setters + accessors:

```rust
    /// Seed the world config-docs (room-hydration path). Each is the singleton of its doc_type, or
    /// `None` when the world has not authored one (resolvers then fall back to built-in defaults).
    pub fn set_world_config(
        &mut self,
        world_settings: Option<Document>,
        gradation: Option<Document>,
        vision_modes: Option<Document>,
    ) {
        self.world_settings = world_settings;
        self.gradation = gradation;
        self.vision_modes = vision_modes;
    }

    /// Seed the actor table (room-hydration path). Keyed by actor doc id.
    pub fn set_actors(&mut self, actors: Vec<Document>) {
        self.actors = actors.into_iter().map(|d| (d.id, d)).collect();
    }

    pub fn actor(&self, id: &Uuid) -> Option<&Document> {
        self.actors.get(id)
    }
    pub fn world_settings_doc(&self) -> Option<&Document> {
        self.world_settings.as_ref()
    }
    pub fn vision_modes_doc(&self) -> Option<&Document> {
        self.vision_modes.as_ref()
    }
    pub fn gradation_doc(&self) -> Option<&Document> {
        self.gradation.as_ref()
    }
```

Extend `apply_op` to maintain these tables. Replace the existing `Operation::Create { .. } => {}` no-op arm and broaden `Update`/`Delete` to also touch the side tables. Concretely, add a helper and route by doc_type:

```rust
    /// Mirror a config/actor field Update into the side tables (Value round-trip, structural-only).
    fn apply_config_update(slot: &mut Option<Document>, doc_id: Uuid, changes: &[crate::data::command::FieldChange]) {
        if let Some(d) = slot {
            if d.id == doc_id {
                if let Ok(mut v) = serde_json::to_value(&*d) {
                    for ch in changes {
                        let _ = set_pointer(&mut v, &ch.path, ch.new.clone());
                    }
                    if let Ok(updated) = serde_json::from_value::<Document>(v) {
                        *d = updated;
                    }
                }
            }
        }
    }
```

In `apply_op`, the `Create` arm: after the scene-entity branch, route non-scene config/actor docs:

```rust
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
```

(The existing `Operation::Create { doc } if is_scene_entity(doc)` guarded arm stays FIRST; this broadened arm replaces the old `Operation::Create { .. } => {}`.)

In the `Update` arm, after the existing scene-entity `self.index` handling, also attempt the config/actor tables (an Update never changes membership, so exactly one table — or the ECS — owns `doc_id`):

```rust
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
```

In the `Delete` arm, after the scene-entity despawn, clear the matching side table:

```rust
                match doc.doc_type.as_str() {
                    "world-settings" => { if self.world_settings.as_ref().map(|d| d.id) == Some(doc.id) { self.world_settings = None; } }
                    "light-gradation" => { if self.gradation.as_ref().map(|d| d.id) == Some(doc.id) { self.gradation = None; } }
                    "vision-modes" => { if self.vision_modes.as_ref().map(|d| d.id) == Some(doc.id) { self.vision_modes = None; } }
                    "actor" => { self.actors.remove(&doc.id); }
                    _ => {}
                }
```

Update the `Default`/`new` and any struct literal accordingly. Keep `entity_count` unchanged (counts only hecs scene entities).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p shadowcat-server scene::tests::config_and_actor_side_tables_track_ops`
Expected: PASS. Also run `cargo test -p shadowcat-server scene::` to confirm the broadened `apply_op` arms keep the existing `apply_op_create_update_delete` test green.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "feat(m10e-2): SceneEcs config-doc + actor side-tables maintained by apply_op"
```

---

### Task 5: Server-side resolvers (scene settings, bands, vision modes)

**Files:**
- Modify: `src/server/src/scene/mod.rs` (resolvers reading the side tables)

**Interfaces:**
- Consumes: side tables from Task 4; `lighting::Band`.
- Produces: `struct ResolvedScene { los_restriction, fog, observer_vision, lighting_enabled, light_mode: LightMode, env_color: u32, env_intensity: f64 }` (+ `enum LightMode { GlobalIllumination, EnvironmentLight }`); `SceneEcs::resolve_scene(scene: Uuid) -> ResolvedScene`; `SceneEcs::resolved_bands() -> Vec<lighting::Band>`; `SceneEcs::resolved_vision_modes() -> HashMap<String, VisionMode>` where `VisionMode { illumination_floor: String, default_range: f64 }`.

- [ ] **Step 1: Write the failing test**

In `mod.rs` `tests`:

```rust
    #[test]
    fn resolvers_layer_world_then_scene_and_fail_closed() {
        use serde_json::json;
        let scene_id = Uuid::from_u128(10);
        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);

        // No config docs → built-in defaults (lighting on, environmentLight, env intensity 0).
        let r0 = ecs.resolve_scene(scene_id);
        assert!(r0.lighting_enabled);
        assert!(matches!(r0.light_mode, LightMode::EnvironmentLight));
        assert_eq!(r0.env_intensity, 0.0);
        assert_eq!(ecs.resolved_bands()[0].name, "bright"); // default gradation
        assert_eq!(ecs.resolved_vision_modes()["darkvision"].illumination_floor, "dark");

        // World default: lighting OFF, global illumination.
        let mut ws = doc(100, None, "world-settings");
        ws.system = json!({
            "scene": { "lightingEnabled": false, "lightMode": "globalIllumination",
                       "environment": { "color": "#0a0e1a", "intensity": 0.25 } },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        ecs.set_world_config(Some(ws), None, None);
        let r1 = ecs.resolve_scene(scene_id);
        assert!(!r1.lighting_enabled);
        assert!(matches!(r1.light_mode, LightMode::GlobalIllumination));
        assert_eq!(r1.env_color, 0x0A0E1A);
        assert!((r1.env_intensity - 0.25).abs() < 1e-9);

        // Per-scene override re-enables lighting (null/absent ⇒ inherit; a present value wins).
        let mut scene = doc(10, None, "scene");
        scene.system = json!({ "grid": { "kind": "square", "size": 100 },
                               "lighting": { "enabled": true } });
        ecs.apply_op(&Operation::Update {
            doc_id: scene_id,
            changes: vec![crate::data::command::FieldChange {
                path: "/system".into(), old: json!(null), new: scene.system.clone(),
            }],
        });
        assert!(ecs.resolve_scene(scene_id).lighting_enabled); // scene override beats world default
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat-server scene::tests::resolvers_layer_world_then_scene_and_fail_closed`
Expected: FAIL — resolvers / `LightMode` / `ResolvedScene` not found.

- [ ] **Step 3: Write minimal implementation**

In `mod.rs`, add the resolved types + a hex parser + the three resolvers. Mirror `resolveSceneSettings`/`resolveGradation`/`resolveVisionModes` (scene-docs.ts) exactly, including the structural fail-closed guard and `null ⇒ inherit` semantics (a JSON `null` override falls through to the world default — `Value::pointer` returning `Null` is treated as absent).

```rust
use crate::scene::lighting::Band;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightMode {
    GlobalIllumination,
    EnvironmentLight,
}

/// The resolved per-scene lighting/vision settings the mask needs (subset of the client
/// `ResolvedSceneSettings`; movement/pathfinding/animation fields are resolved in later checkpoints).
#[derive(Clone, Debug)]
pub struct ResolvedScene {
    pub los_restriction: bool,
    pub fog: bool,
    pub observer_vision: bool,
    pub lighting_enabled: bool,
    pub light_mode: LightMode,
    pub env_color: u32,
    pub env_intensity: f64,
}

/// A resolved vision mode (subset of the client `VisionMode`). `default_range` is in cells.
#[derive(Clone, Debug)]
pub struct VisionMode {
    pub illumination_floor: String,
    pub default_range: f64,
}

/// Parse `#rrggbb` → packed `0xRRGGBB`; fail-closed to `0x000000` (untinted) on any malformed input.
fn parse_hex_color(s: &str) -> u32 {
    let h = s.trim_start_matches('#');
    if h.len() == 6 {
        u32::from_str_radix(h, 16).unwrap_or(0)
    } else {
        0
    }
}

/// Read a bool from a `system` JSON pointer; `null`/absent/non-bool ⇒ `None` (⇒ inherit).
fn opt_bool(v: &serde_json::Value, ptr: &str) -> Option<bool> {
    v.pointer(ptr).and_then(|x| x.as_bool())
}
```

Add to `impl SceneEcs`:

```rust
    /// Resolve a scene's effective lighting/vision settings: built-in defaults < world-settings doc
    /// < per-scene override. Fail-closed and `null ⇒ inherit` (mirrors `resolveSceneSettings`).
    pub fn resolve_scene(&self, scene: Uuid) -> ResolvedScene {
        // World layer — structural guard: a partial world-settings doc falls back to built-ins.
        let ws = self.world_settings.as_ref().map(|d| &d.system);
        let ws_scene = ws.and_then(|s| {
            if s.get("scene").is_some() && s.get("pathfinding").is_some() && s.get("animation").is_some() {
                s.pointer("/scene")
            } else {
                None
            }
        });
        // Built-in defaults (mirror DEFAULT_WORLD_SETTINGS.scene).
        let d_los = ws_scene.and_then(|s| s.get("losRestriction")).and_then(|v| v.as_bool()).unwrap_or(true);
        let d_fog = ws_scene.and_then(|s| s.get("fog")).and_then(|v| v.as_bool()).unwrap_or(true);
        let d_obs = ws_scene.and_then(|s| s.get("observerVision")).and_then(|v| v.as_bool()).unwrap_or(false);
        let d_lit = ws_scene.and_then(|s| s.get("lightingEnabled")).and_then(|v| v.as_bool()).unwrap_or(true);
        let d_mode = ws_scene.and_then(|s| s.get("lightMode")).and_then(|v| v.as_str()).unwrap_or("environmentLight");
        let d_env_color = ws_scene.and_then(|s| s.pointer("/environment/color")).and_then(|v| v.as_str()).unwrap_or("#0a0e1a");
        let d_env_int = ws_scene.and_then(|s| s.pointer("/environment/intensity")).and_then(|v| v.as_f64()).unwrap_or(0.0);

        // Scene override layer (per-scene `vision`/`lighting`; null/absent ⇒ inherit).
        let scene_sys = self
            .index
            .get(&scene)
            .and_then(|&e| self.world.get::<&SceneEntity>(e).ok())
            .map(|c| c.doc.system.clone());
        let s = scene_sys.as_ref();
        let los = s.and_then(|s| opt_bool(s, "/vision/losRestriction")).unwrap_or(d_los);
        let fog = s.and_then(|s| opt_bool(s, "/vision/fog")).unwrap_or(d_fog);
        let obs = s.and_then(|s| opt_bool(s, "/vision/observerVision")).unwrap_or(d_obs);
        let lit = s.and_then(|s| opt_bool(s, "/lighting/enabled")).unwrap_or(d_lit);
        let mode_str = s.and_then(|s| s.pointer("/lighting/mode")).and_then(|v| v.as_str()).unwrap_or(d_mode);
        let env_color = s.and_then(|s| s.pointer("/lighting/environment/color")).and_then(|v| v.as_str()).unwrap_or(d_env_color);
        let env_int = s.and_then(|s| s.pointer("/lighting/environment/intensity")).and_then(|v| v.as_f64()).unwrap_or(d_env_int);

        ResolvedScene {
            los_restriction: los,
            fog,
            observer_vision: obs,
            lighting_enabled: lit,
            light_mode: if mode_str == "globalIllumination" { LightMode::GlobalIllumination } else { LightMode::EnvironmentLight },
            env_color: parse_hex_color(env_color),
            env_intensity: env_int.clamp(0.0, 1.0),
        }
    }

    /// Resolved gradation bands, brightest-first. Fail-closed to the built-in three-band default.
    pub fn resolved_bands(&self) -> Vec<Band> {
        let bands = self
            .gradation
            .as_ref()
            .and_then(|d| d.system.pointer("/bands"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|b| {
                        Some(Band {
                            name: b.get("name")?.as_str()?.to_string(),
                            min_illumination: b.get("minIllumination")?.as_f64()?,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        crate::scene::lighting::sorted_bands(bands)
    }

    /// Resolved vision-mode registry. Fail-closed to the built-in `normal`+`darkvision` seed.
    pub fn resolved_vision_modes(&self) -> HashMap<String, VisionMode> {
        let mut out = HashMap::new();
        let parsed = self
            .vision_modes
            .as_ref()
            .and_then(|d| d.system.pointer("/modes"))
            .and_then(|v| v.as_object());
        if let Some(modes) = parsed {
            for (id, m) in modes {
                if let (Some(floor), range) = (
                    m.get("illuminationFloor").and_then(|v| v.as_str()),
                    m.get("defaultRange").and_then(|v| v.as_f64()).unwrap_or(0.0),
                ) {
                    out.insert(id.clone(), VisionMode { illumination_floor: floor.to_string(), default_range: range });
                }
            }
        }
        if out.is_empty() {
            out.insert("normal".into(), VisionMode { illumination_floor: "dim".into(), default_range: 0.0 });
            out.insert("darkvision".into(), VisionMode { illumination_floor: "dark".into(), default_range: 12.0 });
        }
        out
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p shadowcat-server scene::tests::resolvers_layer_world_then_scene_and_fail_closed`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "feat(m10e-2): server scene-settings/gradation/vision-mode resolvers (fail-closed)"
```

---

### Task 6: Token → vision-mode floor/range resolution

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`token_vision_floors`)

**Interfaces:**
- Consumes: `resolved_vision_modes`, `resolved_bands`, `lighting::floor_min`, the `actors` table.
- Produces: `SceneEcs::token_vision_floors(&self, token: &Document) -> Vec<(f64, f64)>` — a list of `(floor_min_illumination, range_cells)` pairs for the token's effective vision modes (`range_cells == 0.0` ⇒ unlimited). Resolution precedence mirrors the client `resolveTokenActor` (actor.ts): a **linked** token (`actor_id`) resolves the shared actor and `overrides.vision` REPLACES the actor's vision (a *dangling* link → normal, overrides ignored); an **instanced** token (no `actor_id`) uses `embedded.actor[0].system.vision` (overrides do NOT apply); else `[normal]`. I.e. linked(`actor_id`)-with-overrides is checked **before** embedded — NOT `overrides > embedded > actor_id` (that earlier ordering was an error corrected during execution).

- [ ] **Step 1: Write the failing test**

In `mod.rs` `tests`:

```rust
    #[test]
    fn token_vision_floors_resolve_through_actor_join() {
        use serde_json::json;
        let mut ecs = SceneEcs::new();
        // An actor granting darkvision range 6.
        ecs.set_actors(vec![entity_doc_top(
            200, "actor",
            json!({ "vision": [{ "mode": "darkvision", "range": 6 }] }),
        )]);

        // Linked token referencing the actor → darkvision floor (dark=0.0), range 6.
        let mut linked = entity_doc(11, 10, "token", json!({ "x": 0, "y": 0, "actor_id": Uuid::from_u128(200).to_string() }));
        let floors = ecs.token_vision_floors(&linked);
        assert_eq!(floors.len(), 1);
        assert_eq!(floors[0], (0.0, 6.0)); // dark floor, 6-cell range

        // A per-token override REPLACES the actor's vision entirely.
        linked.system["overrides"] = json!({ "vision": [{ "mode": "normal", "range": 0 }] });
        let f2 = ecs.token_vision_floors(&linked);
        assert_eq!(f2[0], (0.34, 0.0)); // dim floor, unlimited range

        // An actorless token → normal only.
        let raw = entity_doc(12, 10, "token", json!({ "x": 0, "y": 0 }));
        assert_eq!(ecs.token_vision_floors(&raw), vec![(0.34, 0.0)]);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat-server scene::tests::token_vision_floors_resolve_through_actor_join`
Expected: FAIL — `token_vision_floors` not found.

- [ ] **Step 3: Write minimal implementation**

In `impl SceneEcs`:

```rust
    /// The token's effective vision modes as `(floor_min_illumination, range_cells)` pairs.
    /// `range_cells == 0.0` ⇒ unlimited. Precedence (mirrors the client EffectiveActor.visionModes):
    /// per-token `overrides.vision` REPLACES; else the instanced `embedded.actor[0].system.vision`;
    /// else the linked `actor(actor_id).system.vision`; else `[normal]`. An unknown mode id is
    /// dropped (fail-closed: it contributes no vision floor). Always returns ≥1 pair (normal fallback).
    pub fn token_vision_floors(&self, token: &Document) -> Vec<(f64, f64)> {
        let modes = self.resolved_vision_modes();
        let bands = self.resolved_bands();

        // Locate the raw `[{mode, range}]` array by precedence.
        let assignments: Option<&serde_json::Value> = token
            .system
            .pointer("/overrides/vision")
            .filter(|v| v.is_array())
            .or_else(|| {
                token
                    .embedded
                    .get("actor")
                    .and_then(|a| a.as_array())
                    .and_then(|a| a.first())
                    .and_then(|a| a.pointer("/system/vision"))
                    .filter(|v| v.is_array())
            })
            .or_else(|| {
                token
                    .system
                    .pointer("/actor_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .and_then(|id| self.actors.get(&id))
                    .and_then(|a| a.system.pointer("/vision"))
                    .filter(|v| v.is_array())
            });

        let mut out: Vec<(f64, f64)> = Vec::new();
        if let Some(arr) = assignments.and_then(|v| v.as_array()) {
            for a in arr {
                let Some(mode_id) = a.get("mode").and_then(|v| v.as_str()) else { continue };
                let Some(vm) = modes.get(mode_id) else { continue }; // unknown mode → drop (fail-closed)
                let range = a.get("range").and_then(|v| v.as_f64()).unwrap_or(vm.default_range);
                out.push((crate::scene::lighting::floor_min(&bands, &vm.illumination_floor), range));
            }
        }
        if out.is_empty() {
            // Normal vision: dim floor, unlimited range.
            let normal_floor = modes.get("normal").map(|m| m.illumination_floor.clone()).unwrap_or_else(|| "dim".into());
            out.push((crate::scene::lighting::floor_min(&bands, &normal_floor), 0.0));
        }
        out
    }
```

Note: this reads `token.embedded` as a `serde_json`-shaped map of arrays — confirm the `Document.embedded` field type during implementation; if `embedded` is a typed `HashMap<String, Vec<Document>>`, adapt the `embedded.actor[0]` access to `.get("actor").and_then(|v| v.first()).and_then(|d| d.system.pointer("/vision"))` against `&Document` instead of `&Value`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p shadowcat-server scene::tests::token_vision_floors_resolve_through_actor_join`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "feat(m10e-2): token→vision-mode floor/range resolution via actor join"
```

---

### Task 7: ECS light + `blocksLight` wall accessors

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`scene_lights`, `light_walls`)

**Interfaces:**
- Produces: `SceneEcs::light_walls(scene: Uuid) -> Vec<vision::Seg>` (the scene's `blocksLight` walls); `SceneEcs::scene_lights(scene: Uuid) -> Vec<lighting::Light>` (enabled `light` docs parented to the scene).

- [ ] **Step 1: Write the failing test**

In `mod.rs` `tests`:

```rust
    #[test]
    fn light_and_blockslight_wall_accessors_filter_by_scene() {
        use serde_json::json;
        let scene = Uuid::from_u128(10);
        let ecs = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                entity_doc(20, 10, "light", json!({
                    "x": 50.0, "y": 50.0, "color": "#ffeeaa", "intensity": 1.0,
                    "brightRadius": 2.0, "dimRadius": 6.0, "enabled": true
                })),
                entity_doc(21, 10, "light", json!({ "x": 0.0, "y": 0.0, "color": "#fff",
                    "intensity": 1.0, "brightRadius": 1.0, "dimRadius": 2.0, "enabled": false })),
                entity_doc(22, 10, "wall", json!({ "seg": {"x1":0,"y1":0,"x2":10,"y2":0}, "blocksLight": true })),
                entity_doc(23, 10, "wall", json!({ "seg": {"x1":0,"y1":5,"x2":10,"y2":5}, "blocksLight": false })),
            ],
            0,
        );
        let lights = ecs.scene_lights(scene);
        assert_eq!(lights.len(), 1); // the disabled light is excluded
        assert_eq!(lights[0].color, 0xFFEEAA);
        assert_eq!(lights[0].bright_radius, 2.0);
        let walls = ecs.light_walls(scene);
        assert_eq!(walls.len(), 1); // only the blocksLight:true wall
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat-server scene::tests::light_and_blockslight_wall_accessors_filter_by_scene`
Expected: FAIL — accessors not found.

- [ ] **Step 3: Write minimal implementation**

In `impl SceneEcs` (model `light_walls` on the existing `sight_walls`; `scene_lights` on `scene_lights`'s sibling iteration):

```rust
    /// The `blocksLight` wall segments of `scene` (the light-occlusion geometry, M10e-2).
    fn light_walls(&self, scene: Uuid) -> Vec<vision::Seg> {
        let mut out = Vec::new();
        for w in self.world.query::<&SceneEntity>().iter() {
            if w.doc.doc_type != "wall" || w.doc.parent_id != Some(scene) {
                continue;
            }
            if w.doc.system.pointer("/blocksLight").and_then(|v| v.as_bool()) != Some(true) {
                continue;
            }
            if let (Some(x1), Some(y1), Some(x2), Some(y2)) = (
                sys_f64(&w.doc, "/seg/x1"), sys_f64(&w.doc, "/seg/y1"),
                sys_f64(&w.doc, "/seg/x2"), sys_f64(&w.doc, "/seg/y2"),
            ) {
                out.push(vision::Seg { a: (x1, y1), b: (x2, y2) });
            }
        }
        out
    }

    /// The enabled `light` docs parented to `scene`, parsed into `lighting::Light`. Disabled lights
    /// are dropped here (they contribute nothing). `falloff` defaults to Linear; missing radii → 0.
    fn scene_lights(&self, scene: Uuid) -> Vec<crate::scene::lighting::Light> {
        use crate::scene::lighting::{Falloff, Light};
        let mut out = Vec::new();
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type != "light" || e.doc.parent_id != Some(scene) {
                continue;
            }
            if e.doc.system.pointer("/enabled").and_then(|v| v.as_bool()) != Some(true) {
                continue;
            }
            let (Some(x), Some(y)) = (sys_f64(&e.doc, "/x"), sys_f64(&e.doc, "/y")) else { continue };
            let color = e.doc.system.pointer("/color").and_then(|v| v.as_str())
                .map(parse_hex_color).unwrap_or(0xFFFFFF);
            let falloff = match e.doc.system.pointer("/falloff/curve").and_then(|v| v.as_str()) {
                Some("quadratic") => Falloff::Quadratic,
                Some("none") => Falloff::None,
                _ => Falloff::Linear,
            };
            out.push(Light {
                pos: (x, y),
                color,
                intensity: e.doc.system.pointer("/intensity").and_then(|v| v.as_f64()).unwrap_or(1.0).clamp(0.0, 1.0),
                bright_radius: e.doc.system.pointer("/brightRadius").and_then(|v| v.as_f64()).unwrap_or(0.0),
                dim_radius: e.doc.system.pointer("/dimRadius").and_then(|v| v.as_f64()).unwrap_or(0.0),
                falloff,
                enabled: true,
            });
        }
        // Deterministic order (entity-query order is unspecified): sort by id-stable position.
        out.sort_by(|a, b| a.pos.partial_cmp(&b.pos).unwrap_or(std::cmp::Ordering::Equal));
        out
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p shadowcat-server scene::tests::light_and_blockslight_wall_accessors_filter_by_scene`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "feat(m10e-2): SceneEcs light + blocksLight wall accessors"
```

---

### Task 8: Per-player lit mask composition

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`player_lit_mask`)

**Interfaces:**
- Consumes: Tasks 4-7 (`resolve_scene`, `resolved_bands`, `token_vision_floors`, `scene_lights`, `light_walls`, `sight_walls`), `vision::{visibility_polygon, bound_for}`, `lighting::cell_illumination`, `explored`-style cell rasterization.
- Produces: `SceneEcs::player_lit_mask(user: Uuid) -> Vec<LitScene>` where `struct LitScene { scene: Uuid, cell: f64, cells: Vec<(i32, i32, usize, u32)> }` — per scene, the visible cells as `(i, j, band_index, tint)`. Empty when the player has no vision source (fail-closed).

- [ ] **Step 1: Write the failing test**

In `mod.rs` `tests`:

```rust
    #[test]
    fn lit_mask_gates_los_by_illumination_and_darkvision() {
        use serde_json::json;
        let player = Uuid::from_u128(7);
        let scene = Uuid::from_u128(10);

        // A normal-vision token at origin in a walled-open scene. lightingEnabled defaults true,
        // environmentLight, env intensity 0 → with NO lights the scene is dark → normal vision sees
        // nothing (fail-closed): the lit mask is empty.
        let mut tok = entity_doc(11, 10, "token", json!({ "x": 50, "y": 50 }));
        tok.owner = Some(player);
        let dark = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok.clone()], 0);
        assert!(dark.player_lit_mask(player).iter().all(|s| s.cells.is_empty()),
            "dark scene + normal vision → empty lit mask");

        // Add a bright light covering the token's cell → that cell becomes visible at the bright band.
        let light = entity_doc(20, 10, "light", json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true }));
        let lit = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok.clone(), light], 0);
        let mask = lit.player_lit_mask(player);
        let s = mask.iter().find(|s| s.scene == scene).expect("scene present");
        assert!(s.cells.iter().any(|&(i, j, band, _)| i == 0 && j == 0 && band == 0),
            "the lit cell at (0,0) is visible at the bright band (cell_size 100)");

        // Darkvision token in the SAME dark scene (no light) sees within range despite darkness.
        let mut dv = entity_doc(12, 10, "token", json!({
            "x": 50, "y": 50, "overrides": { "vision": [{ "mode": "darkvision", "range": 6 }] } }));
        dv.owner = Some(player);
        let dvmask = SceneEcs::from_documents(vec![doc(10, None, "scene"), dv], 0).player_lit_mask(player);
        assert!(dvmask.iter().any(|s| !s.cells.is_empty()),
            "darkvision sees in the dark within range");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat-server scene::tests::lit_mask_gates_los_by_illumination_and_darkvision`
Expected: FAIL — `player_lit_mask` / `LitScene` not found.

- [ ] **Step 3: Write minimal implementation**

In `mod.rs` add the result type and the computation. Reuse the explored rasterizer's per-cell scan (bbox of the LOS polygon, center-in-poly), composing illumination per cell. Reuse `MAX_CELLS_PER_POLYGON`-style guarding by importing the cap or re-deriving a local bound.

```rust
/// One scene's visible cells for a player: `cells` are `(i, j, band_index, tint 0xRRGGBB)`.
pub struct LitScene {
    pub scene: Uuid,
    pub cell: f64,
    pub cells: Vec<(i32, i32, usize, u32)>,
}
```

```rust
    /// The per-player lighting-aware visibility mask: per scene, the cells the user can currently
    /// see = LOS-cells ∩ (illumination ≥ vision floor ∨ darkvision-in-range), each tagged with its
    /// illumination band + tint. Vision sources = owned tokens ∪ (observerVision ? Observer-tier
    /// tokens : ∅). Fail-closed: a source-less player gets empty cells. GM is handled by the caller
    /// (mode:"all"); this is the masked path only.
    pub fn player_lit_mask(&self, user: Uuid) -> Vec<LitScene> {
        // 1. Gather vision-source tokens per scene (owner ∪ observer-tier when observerVision on).
        //    Collect (scene, viewpoint, vision_floors) tuples; drop the query borrow before raycasts.
        struct Src { scene: Uuid, vp: vision::P, floors: Vec<(f64, f64)> }
        let mut sources: Vec<Src> = Vec::new();
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type != "token" {
                continue;
            }
            let Some(scene) = e.doc.parent_id else { continue };
            let owns = e.doc.owner == Some(user);
            let observes = {
                let role = e.doc.permissions.users.get(&user).copied()
                    .unwrap_or(e.doc.permissions.default);
                role <= crate::data::document::DocRole::Observer
            };
            let is_source = owns || (self.resolve_scene(scene).observer_vision && observes);
            if !is_source {
                continue;
            }
            if let (Some(x), Some(y)) = (sys_f64(&e.doc, "/x"), sys_f64(&e.doc, "/y")) {
                sources.push(Src { scene, vp: (x, y), floors: self.token_vision_floors(&e.doc) });
            }
        }
        if sources.is_empty() {
            return Vec::new();
        }

        // 2. Per scene, accumulate visible cells across that scene's sources.
        let grid = self.scene_grid_sizes();
        let bands = self.resolved_bands();
        use std::collections::BTreeMap;
        // (scene) -> map cell (i,j) -> (best_level, band, tint)
        let mut per_scene: BTreeMap<Uuid, (f64, BTreeMap<(i32, i32), (f64, usize, u32)>)> = BTreeMap::new();

        // Distinct scenes among the sources.
        let mut scenes: Vec<Uuid> = sources.iter().map(|s| s.scene).collect();
        scenes.sort();
        scenes.dedup();

        for scene in scenes {
            let settings = self.resolve_scene(scene);
            let cell = grid.get(&scene).copied().unwrap_or(100.0);
            if cell <= 0.0 {
                continue;
            }
            let sight_walls = self.sight_walls(scene);
            // Lighting inputs: under globalIllumination or lighting-off, every LOS cell is bright;
            // else compute per-cell from lights (occluded by blocksLight) + environment.
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

            let entry = per_scene.entry(scene).or_insert_with(|| (cell, BTreeMap::new()));
            for src in sources.iter().filter(|s| s.scene == scene) {
                // LOS polygon for this source (or, LOS off, the whole bound box as a polygon).
                let b = vision::bound_for(src.vp, &sight_walls, VISION_BOUND_MARGIN);
                let poly = if settings.los_restriction {
                    vision::visibility_polygon(src.vp, &sight_walls, b)
                } else {
                    vec![(b.minx, b.miny), (b.maxx, b.miny), (b.maxx, b.maxy), (b.minx, b.maxy)]
                };
                if poly.len() < 3 {
                    continue;
                }
                // Bbox → candidate cells (mirror explored's bounded scan).
                let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for &(x, y) in &poly {
                    minx = minx.min(x); miny = miny.min(y); maxx = maxx.max(x); maxy = maxy.max(y);
                }
                let i0 = (minx / cell).floor() as i32;
                let i1 = (maxx / cell).floor() as i32;
                let j0 = (miny / cell).floor() as i32;
                let j1 = (maxy / cell).floor() as i32;
                let span = (i1 as i64 - i0 as i64 + 1) * (j1 as i64 - j0 as i64 + 1);
                if span > crate::scene::explored::MAX_CELLS_PER_POLYGON {
                    tracing::warn!(span, "lit mask cell scan exceeds cap; skipping source");
                    continue;
                }
                for i in i0..=i1 {
                    for j in j0..=j1 {
                        let cx = (i as f64 + 0.5) * cell;
                        let cy = (j as f64 + 0.5) * cell;
                        if !crate::scene::vision::point_in_poly(&poly, (cx, cy)) {
                            continue;
                        }
                        let cl = crate::scene::lighting::cell_illumination(
                            (cx, cy), settings.env_intensity, settings.env_color, &lights, &lit_polys, cell,
                        );
                        // Darkvision lowers the floor within range; pick the lowest applicable floor.
                        let dist_cells = (((cx - src.vp.0).powi(2) + (cy - src.vp.1).powi(2)).sqrt()) / cell;
                        let mut floor = f64::INFINITY;
                        for &(fmin, range) in &src.floors {
                            if range == 0.0 || dist_cells <= range {
                                floor = floor.min(fmin);
                            }
                        }
                        if floor.is_finite() && cl.level >= floor {
                            let band = crate::scene::lighting::band_index(&bands, cl.level);
                            let slot = entry.1.entry((i, j)).or_insert((cl.level, band, cl.tint));
                            if cl.level > slot.0 {
                                *slot = (cl.level, band, cl.tint); // brightest source wins the band/tint
                            }
                        }
                    }
                }
            }
        }

        per_scene
            .into_iter()
            .map(|(scene, (cell, cells))| LitScene {
                scene,
                cell,
                cells: cells.into_iter().map(|((i, j), (_lvl, band, tint))| (i, j, band, tint)).collect(),
            })
            .collect()
    }
```

This references `crate::scene::explored::MAX_CELLS_PER_POLYGON` — promote that const to `pub(crate)` in `explored.rs` (it is currently private) so the lit-mask scan shares the same guard. Confirm `Document.permissions` exposes `.users` (a `HashMap<Uuid, DocRole>`) and `.default` (a `DocRole`); adapt field access to the real `PermissionSet` shape if it differs.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p shadowcat-server scene::tests::lit_mask_gates_los_by_illumination_and_darkvision`
Expected: PASS. Run `cargo test -p shadowcat-server scene::` to confirm no regression.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/mod.rs src/server/src/scene/explored.rs
git commit -m "feat(m10e-2): per-player lighting-aware visibility mask"
```

---

### Task 9: Attach the lit mask to the vision payload

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`compute_derived` "vision" arm)

**Interfaces:**
- Consumes: `player_lit_mask`.
- Produces: the masked `vision` payload gains a top-level `"bands": [{name, min}]` (world-scoped resolved gradation, emitted once) and `"lit"`: an array of `{ scene, cell, cells: [i, j, band, tint, ...] }`. (`bands` is hoisted to the top level — NOT repeated per `lit` entry — corrected during execution to avoid per-scene wire duplication, while the M10e-3 client consumer is unwritten.) `polygons` + the (post-lock) `explored` are unchanged. GM payload (`mode:"all"`) is unchanged.

- [ ] **Step 1: Write the failing test**

Extend the existing `vision_channel_is_per_recipient` expectations with a new test in `mod.rs` `tests`:

```rust
    #[test]
    fn vision_payload_carries_lit_mask_for_players_not_gm() {
        use crate::data::document::WorldRole;
        use serde_json::json;
        let player = Uuid::from_u128(7);
        let mut tok = entity_doc(11, 10, "token", json!({ "x": 50, "y": 50 }));
        tok.owner = Some(player);
        let light = entity_doc(20, 10, "light", json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true }));
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, light], 0);

        let pl = PermissionContext { user_id: player, world_role: WorldRole::Player };
        let pv = compute_derived("vision", &ecs, &pl).unwrap();
        assert_eq!(pv["mode"], "masked");
        let lit = pv["lit"].as_array().expect("lit present for masked payload");
        assert_eq!(lit.len(), 1);
        assert_eq!(lit[0]["scene"], json!(Uuid::from_u128(10)));
        assert!(lit[0]["cells"].as_array().unwrap().len() >= 4); // ≥ one cell (4 ints/cell)
        assert!(lit[0]["bands"].as_array().unwrap().len() >= 1);

        // GM payload is unchanged — no lit key.
        let gm = PermissionContext { user_id: Uuid::from_u128(1), world_role: WorldRole::Gm };
        let gv = compute_derived("vision", &ecs, &gm).unwrap();
        assert_eq!(gv["mode"], "all");
        assert!(gv.get("lit").is_none());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat-server scene::tests::vision_payload_carries_lit_mask_for_players_not_gm`
Expected: FAIL — payload has no `lit` key.

- [ ] **Step 3: Write minimal implementation**

In `compute_derived`, the `"vision"` arm's masked branch, after building `polygons`, attach `lit`:

```rust
            } else {
                let polygons: Vec<serde_json::Value> = ecs
                    .player_vision_polygons(ctx.user_id)
                    .into_iter()
                    .map(|(scene, poly)| {
                        let points: Vec<f64> = poly.into_iter().flat_map(|(x, y)| [x, y]).collect();
                        serde_json::json!({ "scene": scene, "points": points })
                    })
                    .collect();
                // M10e-2: the secrecy-safe lighting-aware mask — only currently-visible cells, each
                // tagged with its illumination band + tint. Carries the resolved gradation `bands`
                // so the client maps band indices → treatment. Additive: `polygons`/`explored` are
                // unchanged (the client consumes `lit` from M10e-3).
                let bands_json: Vec<serde_json::Value> = ecs
                    .resolved_bands()
                    .into_iter()
                    .map(|b| serde_json::json!({ "name": b.name, "min": b.min_illumination }))
                    .collect();
                let lit: Vec<serde_json::Value> = ecs
                    .player_lit_mask(ctx.user_id)
                    .into_iter()
                    .map(|s| {
                        let flat: Vec<i64> = s
                            .cells
                            .into_iter()
                            .flat_map(|(i, j, band, tint)| {
                                [i as i64, j as i64, band as i64, tint as i64]
                            })
                            .collect();
                        serde_json::json!({ "scene": s.scene, "cell": s.cell, "bands": bands_json, "cells": flat })
                    })
                    .collect();
                Some(serde_json::json!({ "mode": "masked", "polygons": polygons, "lit": lit }))
            }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p shadowcat-server scene::tests::vision_payload_carries_lit_mask_for_players_not_gm`
Expected: PASS. Re-run the existing `vision_channel_is_per_recipient` to confirm `polygons`/`mode` are unchanged.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "feat(m10e-2): attach lighting-aware lit mask to vision payload"
```

---

### Task 10: Room hydration of config-docs + actors, and an end-to-end frame test

**Files:**
- Modify: `src/server/src/ws/room.rs` (`RoomRegistry::get_or_create`)
- Modify: `src/server/tests/scene_derived.rs` (integration)

**Interfaces:**
- Consumes: `Repository::query_documents(world, doc_type)`, `SceneEcs::set_world_config`/`set_actors`.
- Produces: a `SceneEcs` hydrated with the 3 config singletons + all actors at room creation; the live path keeps them current via `apply_op` (Task 4).

- [ ] **Step 1: Write the failing integration test**

In `src/server/tests/scene_derived.rs`, add (adapt fixtures to the file's existing helpers for spinning up a repo/world/room and subscribing to the `vision` channel):

```rust
#[tokio::test]
async fn vision_frame_includes_lit_mask_after_room_hydration() {
    // GM authors: a world-settings doc (default lighting), a scene, a player token, a bright light.
    // A player subscribing to the `vision` channel receives a masked payload whose `lit` mask
    // contains the lit cell around the token — proving the room hydrated the config-docs and the
    // mask flows end-to-end.
    // ... build repo + world + GM ctx + player member (mirror scene_derived.rs's existing setup) ...
    // ... GM publishes: world-settings (buildWorldSettingsDoc shape), scene, player-owned token,
    //     enabled light at the token cell ...
    // ... player opens a `vision` SceneSubscribe, reads the SceneDerived frame ...
    // assert payload["mode"] == "masked";
    // assert !payload["lit"].as_array().unwrap().is_empty();
    // assert payload["lit"][0]["cells"].as_array().unwrap().len() >= 4;
}
```

Fill the body using the concrete helpers already present in `scene_derived.rs` (the M9 `vision` test in that file is the template — replicate its room/subscribe scaffolding, then add the world-settings + light docs and the `lit` assertions).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p shadowcat-server --test scene_derived vision_frame_includes_lit_mask_after_room_hydration`
Expected: FAIL — `lit` empty (room not hydrating config) or compile error until Step 3 wires hydration.

- [ ] **Step 3: Wire room hydration**

In `RoomRegistry::get_or_create`, after `let docs = repo.query_scene_entities(world_id).await?;` and before constructing the room, fetch config-docs + actors and seed them:

```rust
        let docs = repo.query_scene_entities(world_id).await?;
        let mut scene_ecs = SceneEcs::from_documents(docs, world.seq);
        // M10e-2: hydrate the lighting-aware vision inputs that are NOT scene entities — the three
        // world config singletons + actors — so the mask computation is pure/synchronous under the
        // scene read-lock. Kept live thereafter by `apply_op`.
        let world_settings = repo.query_documents(world_id, "world-settings").await?.into_iter().next();
        let gradation = repo.query_documents(world_id, "light-gradation").await?.into_iter().next();
        let vision_modes = repo.query_documents(world_id, "vision-modes").await?.into_iter().next();
        scene_ecs.set_world_config(world_settings, gradation, vision_modes);
        scene_ecs.set_actors(repo.query_documents(world_id, "actor").await?);
        let room = self
            .rooms
            .entry(world_id)
            .or_insert_with(|| Arc::new(Room::new(world_id, world.seq, scene_ecs, self.broadcast_capacity)))
            .clone();
        Ok(Some(room))
```

(Replace the existing `let scene_ecs = SceneEcs::from_documents(docs, world.seq);` line; `scene_ecs` is now `mut`.)

- [ ] **Step 4: Run the test + full gate**

Run: `cargo test -p shadowcat-server --test scene_derived vision_frame_includes_lit_mask_after_room_hydration`
Expected: PASS.
Then the full local gate:
Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test -p shadowcat-server`
Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/ws/room.rs src/server/tests/scene_derived.rs
git commit -m "feat(m10e-2): hydrate config-docs + actors into the room; e2e lit-mask frame test"
```

---

## Post-execution gates (mandatory before checkpoint completion)

1. **Whole-branch buddy-check** (see directives below).
2. **Skill-update gate:** update `shadowcat-codebase-scene-rendering` (the new `lighting.rs` seam, the `SceneEcs` config/actor side-tables + resolvers, the additive `lit` payload) and `shadowcat-codebase-realtime-sync` if the derived-egress description changed; dispatch `shadowcat-spec-reviewer` on the skill diff. State explicitly if a skill needs no change.
3. **Docs sync:** `docs/PLAN.md` (M10e-2 status), `docs/POST_WORK_FINDINGS.md` (any mid-run anomalies). No conjecture.
4. **Merge** `--no-ff` to LOCAL main; do NOT push (push gate = full M10).

## Buddy-check directives

This checkpoint is **security-sensitive** (the lit mask is the forthcoming per-player secrecy gate) and reworks the engine vision core — it qualifies for a whole-branch buddy-check. After Task 10, dispatch the two-reviewer pair (`shadowcat-spec-reviewer` + `shadowcat-code-reviewer`, on Opus) over the full branch diff with this focus:

- **Fail-closed correctness:** does every resolver/accessor under-reveal on missing/partial/garbled input (no config doc, malformed gradation, unknown vision mode, missing radii, non-positive cell size)? Is there ANY input that makes the mask reveal MORE than pure LOS would?
- **Secrecy of egress:** does the `lit` payload ever carry a cell outside the player's vision sources? Is illumination outside the mask ever serialized? Is the GM path unchanged?
- **Additivity:** confirm `polygons` + `explored` behavior is byte-for-byte unchanged (no regression to the M9 client consuming polygons).
- **Determinism + bounds:** are all cell scans bounded by `MAX_CELLS_PER_POLYGON` and iterated in a fixed order? No `HashMap` iteration leaking into wire order.
- **Side-table coherence:** does `apply_op` keep the config/actor tables consistent across Create/Update/Delete, including a wholesale `/system` Update (mirroring the `token_move` post-image hazard)?

Record the converged outcome (PASS / fixes-applied) in this section before merge.

**Buddy-check outcome (2026-06-24, both reviewers on Opus):** CONVERGED PASS. Both the
`shadowcat-spec-reviewer` and `shadowcat-code-reviewer` independently APPROVED the whole-branch
implementation as fail-closed on every resolver/accessor/mask path, secrecy-safe (the `lit` egress
carries only the recipient's in-LOS cells; GM unchanged), additive (`polygons`/`explored`
unmodified, no M9 regression), deterministic (BTreeMap-ordered output; HashMaps point-lookup only),
and faithful to the spec + client parity (`resolveSceneSettings`/`resolveGradation`/
`resolveVisionModes`/`resolveTokenActor`). The only deviation is the constraint-forced flat-ambient
environment light, logged to `docs/TODO.md`. The spec reviewer's CHANGES-REQUESTED was solely the
remaining post-execution gates (skill-update, docs-sync, plan crate-name, this record) — all now
completed before merge. Per-task review also caught and fixed: a Critical (`all_bright` left players
blind), a `resolveTokenActor` precedence inversion, and a cell-span i64-overflow DoS.
