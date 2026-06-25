# M10e-3 — Client Lighting Render Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the server's per-(user,scene) lighting-aware mask on the client — a new engine-owned lighting layer that darkens/tints visible cells by gradation band, fades day/night transitions smoothly, and desaturates darkvision-only cells via a faithful per-cell render hint.

**Architecture:** The server `vision` frame (hand-shaped JSON, parsed as `unknown` — NOT a ts-rs type) is extended additively with a per-cell `renderHint` reference plus a top-level `renderHints` table. The client parses the frame into a `LightingInput`, a new engine-owned `Lighting` class resolves band→darkening + hint→desaturate and interpolates day/night transitions, and a new `lighting` core layer (between `templates` and `mask`) renders the result with a blur for smooth edges. Fog is untouched and remains the secrecy gate; lighting is purely cosmetic.

**Tech Stack:** Rust (server scene ECS, `serde_json`), TypeScript (`@shadowcat/render`, Vitest), PixiJS v8 (`BlurFilter`), SCSS unaffected.

## Global Constraints

- **Server crate is `shadowcat`** (NOT `shadowcat-server`). Server commands run from `src/server/`.
- **`dist/` must be built before any `cargo` build** (`rust-embed` compile-time validation): `pnpm --filter @shadowcat/ui build` first. Client-only tasks don't need a server build.
- **Determinism:** any derived/wire output uses sorted/`BTreeMap` containers, never `HashMap`-into-wire. The `renderHints` table order must be deterministic.
- **Fail-closed secrecy (fog), cosmetic lighting:** fog (`mode:"masked"`, empty `visible` ⇒ hide-all) remains the SOLE secrecy gate. Lighting renders only cosmetic darkening/tint over cells the server already placed in the mask; garbled/missing lighting data ⇒ render NO lighting overlay (never over- or under-reveal). The per-cell hint is cosmetic metadata on already-visible cells — it never widens visibility (`server-mirrors-client-resolver-semantics`, `fog-is-the-secrecy-gate-fail-closed`).
- **Server resolvers mirror the client source exactly** — the server `renderHint` seed/parse must equal `scene-docs.ts` `SEED_VISION_MODES` / `VisionMode` (`server-mirrors-client-resolver-semantics`).
- **Cross-platform:** no OS-specific paths; pure-Rust server; client renders desktop + mobile.
- Client: `pnpm --filter @shadowcat/render test` (Vitest), `pnpm -r typecheck`, `pnpm lint`. Server: `cargo test`, `cargo fmt`, `cargo clippy` from `src/server/`.

## Design decisions (folded from brainstorming, 2026-06-25)

Governing spec: `docs/superpowers/specs/2026-06-24-m10e-vision-lighting-movement-design.md` §7. The brainstormed checkpoint decisions:

1. **New engine-owned `lighting` core layer** (between `templates` and `mask`). Lighting = per-cell band darkening + tint over visible cells; fog (memory) composites above; tokens (below) darken in shadow.
2. **Faithful darkvision (server + client).** The `vision` frame gains a per-cell `renderHint` reference + a top-level `renderHints` table. The hint = the `renderHint` of the admitting vision mode with the **highest illumination floor** (a normal-lit perception suppresses the hint; a cell admitted only by a darkvision-class floor carries that mode's hint). Secrecy-neutral.
3. **Smooth edges via a Pixi `BlurFilter`** on the lighting layer. (Per-cell radial gradients → `POST_WORK_FINDINGS` follow-up.)
4. **Smooth day/night transitions** interpolate per-cell darkening/tint over a short fade in the `Lighting` class (cells present in both prev+target lerp; added/removed cells snap). Logic is GL-free and unit-tested.

**Deferred to `POST_WORK_FINDINGS.md` (logged in Task 10):** (a) per-cell radial-gradient soft edges (vs the blur approximation); (b) true masked color-matrix desaturation of the underlying scene for darkvision (V1 renders a desaturating overlay approximation — the *payload* is already faithful, so the refinement is client-render-only with no future server change).

---

### Task 1: Server — `VisionMode.render_hint` parse + seed

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`VisionMode` struct ~36-42; `resolved_vision_modes` ~417-462)

**Interfaces:**
- Produces: `VisionMode { illumination_floor: String, default_range: f64, render_hint: Option<String> }`. Seed: `normal` ⇒ `render_hint: None`; `darkvision` ⇒ `render_hint: Some("desaturate")` (mirrors `scene-docs.ts` `SEED_VISION_MODES`). Present-doc branch reads optional `renderHint` string; absent ⇒ `None`.

- [ ] **Step 1: Write the failing test** (append to the `#[cfg(test)] mod tests` in `mod.rs`):

```rust
#[test]
fn vision_modes_carry_render_hint() {
    use serde_json::json;
    // Absent doc → built-in seed mirrors scene-docs.ts: darkvision desaturates, normal does not.
    let seeded = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
    let m = seeded.resolved_vision_modes();
    assert_eq!(m["normal"].render_hint, None);
    assert_eq!(m["darkvision"].render_hint.as_deref(), Some("desaturate"));

    // Present doc → renderHint parsed; absent field → None.
    let mut vm = entity_doc(30, 10, "vision-modes", json!({}));
    vm.doc_type = "vision-modes".into();
    vm.parent_id = None;
    vm.system = json!({ "modes": {
        "truesight": { "illuminationFloor": "dark", "defaultRange": 8, "renderHint": "outline" },
        "plain":     { "illuminationFloor": "dim",  "defaultRange": 0 }
    }});
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), vm], 0);
    let m = ecs.resolved_vision_modes();
    assert_eq!(m["truesight"].render_hint.as_deref(), Some("outline"));
    assert_eq!(m["plain"].render_hint, None);
}
```

(If `vision-modes` config docs hydrate via a side-table rather than `from_documents`, mirror the construction used by the existing `resolved_vision_modes` test near line 1463 — read that test first and match its setup exactly.)

- [ ] **Step 2: Run to verify it fails**

Run (from `src/server/`): `cargo test vision_modes_carry_render_hint`
Expected: FAIL — `render_hint` field does not exist.

- [ ] **Step 3: Implement**

Add the field to the struct:

```rust
#[derive(Clone, Debug)]
pub struct VisionMode {
    pub illumination_floor: String,
    pub default_range: f64,
    pub render_hint: Option<String>,
}
```

In `resolved_vision_modes`, present-doc branch — parse the optional hint:

```rust
out.insert(
    id.clone(),
    VisionMode {
        illumination_floor: floor.to_string(),
        default_range: range,
        render_hint: m
            .get("renderHint")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    },
);
```

None (seed) branch:

```rust
out.insert("normal".into(), VisionMode {
    illumination_floor: "dim".into(), default_range: 0.0, render_hint: None,
});
out.insert("darkvision".into(), VisionMode {
    illumination_floor: "dark".into(), default_range: 12.0,
    render_hint: Some("desaturate".into()),
});
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test vision_modes_carry_render_hint` → PASS. Then `cargo fmt && cargo clippy`.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "feat(m10e-3): vision modes carry an optional renderHint (mirrors client seed)"
```

---

### Task 2: Server — `token_vision_floors` carries the per-mode hint

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`token_vision_floors` ~685-747; its only caller `player_lit_mask` ~816)

**Interfaces:**
- Consumes: `VisionMode.render_hint` (Task 1).
- Produces: `token_vision_floors(&self, token: &Document) -> Vec<(f64, f64, Option<String>)>` — `(floor_min_value, range_cells, render_hint)` per resolved mode. The `out.is_empty()` normal fallback yields `(normal_floor, 0.0, None)`.

- [ ] **Step 1: Write the failing test** (append to tests):

```rust
#[test]
fn token_vision_floors_include_render_hint() {
    use serde_json::json;
    // Instanced token with embedded actor granting normal + darkvision.
    let mut tok = entity_doc(11, 10, "token", json!({ "x": 0, "y": 0 }));
    tok.embedded.insert("actor".into(), vec![{
        let mut a = doc(99, None, "actor");
        a.system = json!({ "vision": [
            { "mode": "normal", "range": 0 },
            { "mode": "darkvision", "range": 6 }
        ]});
        a
    }]);
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok.clone()], 0);
    let floors = ecs.token_vision_floors(&tok);
    // darkvision entry carries the desaturate hint; normal carries none.
    assert!(floors.iter().any(|(_, _, h)| h.as_deref() == Some("desaturate")));
    assert!(floors.iter().any(|(_, _, h)| h.is_none()));
}
```

(Match the embedded-actor construction to the existing instanced-token tests near line 1504 — read them first.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test token_vision_floors_include_render_hint`
Expected: FAIL — return type is a 2-tuple; `.2` does not exist.

- [ ] **Step 3: Implement**

Change the signature and both push sites:

```rust
pub fn token_vision_floors(&self, token: &Document) -> Vec<(f64, f64, Option<String>)> {
    // ... unchanged resolution of `assignments` ...
    let mut out: Vec<(f64, f64, Option<String>)> = Vec::new();
    if let Some(arr) = assignments.and_then(|v| v.as_array()) {
        for a in arr {
            let Some(mode_id) = a.get("mode").and_then(|v| v.as_str()) else { continue; };
            let Some(vm) = modes.get(mode_id) else { continue; };
            let range = a.get("range").and_then(|v| v.as_f64()).unwrap_or(vm.default_range);
            out.push((
                crate::scene::lighting::floor_min(&bands, &vm.illumination_floor),
                range,
                vm.render_hint.clone(),
            ));
        }
    }
    if out.is_empty() {
        let normal_floor = modes.get("normal")
            .map(|m| m.illumination_floor.clone())
            .unwrap_or_else(|| "dim".into());
        out.push((crate::scene::lighting::floor_min(&bands, &normal_floor), 0.0, None));
    }
    out
}
```

In `player_lit_mask`, the `Src.floors` field type becomes `Vec<(f64, f64, Option<String>)>` and the floor loop destructures the third element (fully wired in Task 3; for now adjust the destructure to `for &(fmin, range, ref _hint) in &src.floors` so it compiles).

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test token_vision_floors_include_render_hint` and `cargo test scene::` → PASS. `cargo fmt && cargo clippy`.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "feat(m10e-3): token_vision_floors returns the per-mode renderHint"
```

---

### Task 3: Server — `player_lit_mask` resolves the per-cell hint

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`LitScene` ~81-87; `player_lit_mask` cell accumulation ~824-972)

**Interfaces:**
- Consumes: `token_vision_floors` 3-tuples (Task 2).
- Produces: `LitScene { scene: Uuid, cell: f64, cells: Vec<(i32, i32, usize, u32, Option<String>)> }` — `(i, j, band_index, tint, render_hint)`. Hint rule: among admitting modes (range covers + `level >= floor`) across all of the cell's sources, the hint of the one with the **highest floor**; on a floor tie, `None` wins over `Some`. Initialized so a normal (dim-floor, `None`) perception suppresses any darkvision hint on a lit cell.

- [ ] **Step 1: Write the failing test** (append to tests; mirror the existing lit-mask fixtures near line 1654):

```rust
#[test]
fn lit_mask_tags_darkvision_only_cells_with_hint() {
    use serde_json::json;
    let player = Uuid::from_u128(7);
    // Dark scene (no lights, environmentLight, lighting on) → only darkvision admits cells.
    let mut tok = entity_doc(11, 10, "token", json!({ "x": 50, "y": 50 }));
    tok.owner = Some(player);
    tok.embedded.insert("actor".into(), vec![{
        let mut a = doc(99, None, "actor");
        a.system = json!({ "vision": [{ "mode": "darkvision", "range": 6 }] });
        a
    }]);
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok], 0);
    let mask = ecs.player_lit_mask(player);
    assert_eq!(mask.len(), 1);
    assert!(mask[0].cells.iter().all(|(_, _, _, _, h)| h.as_deref() == Some("desaturate")),
        "dark cells perceived only via darkvision carry the desaturate hint");

    // Bright cell under a light, seen by normal vision → no hint (normal floor suppresses it).
    let player2 = Uuid::from_u128(8);
    let mut tok2 = entity_doc(12, 10, "token", json!({ "x": 50, "y": 50 }));
    tok2.owner = Some(player2); // no embedded vision → normal fallback
    let light = entity_doc(20, 10, "light", json!({
        "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
        "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true }));
    let lit = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok2, light], 0);
    let mask2 = lit.player_lit_mask(player2);
    assert!(mask2[0].cells.iter().any(|(_, _, _, _, h)| h.is_none()),
        "a normally-lit cell seen by normal vision carries no hint");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test lit_mask_tags_darkvision_only_cells_with_hint`
Expected: FAIL — `LitScene.cells` is a 4-tuple.

- [ ] **Step 3: Implement**

Update the struct:

```rust
/// One scene's visible cells for a player: `cells` are `(i, j, band_index, tint 0xRRGGBB, render_hint)`.
#[derive(Debug)]
pub struct LitScene {
    pub scene: Uuid,
    pub cell: f64,
    pub cells: Vec<(i32, i32, usize, u32, Option<String>)>,
}
```

In `player_lit_mask`, widen the per-cell accumulator to carry `(best_level, band, tint, hint_floor, hint)` and resolve the hint per the rule. Replace the `CellEntry` type and the inner accumulation block:

```rust
// (i, j) -> (best_level, band_index, tint, hint_floor, hint). hint_floor seeds NEG_INFINITY so the
// first admitting mode always sets it; brightness (level/band/tint) and hint reduce independently.
type CellEntry = BTreeMap<(i32, i32), (f64, usize, u32, f64, Option<String>)>;
```

Inside the `point_in_poly` block, after computing `cl`, replace the floor/admit logic:

```rust
let dist_cells = (((cx - src.vp.0).powi(2) + (cy - src.vp.1).powi(2)).sqrt()) / cell;
// Lowest applicable floor decides visibility; highest applicable floor decides the hint.
let mut visible_floor = f64::INFINITY;          // min admitting floor → does the cell show at all
let mut admit_floor = f64::NEG_INFINITY;        // max admitting floor → which mode's hint wins
let mut admit_hint: Option<String> = None;
for (fmin, range, hint) in &src.floors {
    let in_range = *range == 0.0 || dist_cells <= *range;
    if !in_range { continue; }
    visible_floor = visible_floor.min(*fmin);
    if cl.level >= *fmin {
        // Highest admitting floor wins; on a tie, None (a normal-equivalent perception) wins.
        let take = *fmin > admit_floor || (*fmin == admit_floor && admit_hint.is_some() && hint.is_none());
        if take {
            admit_floor = *fmin;
            admit_hint = hint.clone();
        }
    }
}
if visible_floor.is_finite() && cl.level >= visible_floor {
    let band = crate::scene::lighting::band_index(&bands, cl.level);
    let slot = entry.1.entry((i, j)).or_insert((cl.level, band, cl.tint, admit_floor, admit_hint.clone()));
    if cl.level > slot.0 {
        slot.0 = cl.level; slot.1 = band; slot.2 = cl.tint; // brightest source wins band/tint
    }
    // Hint reduces across sources by the same highest-floor/None-wins rule.
    if admit_floor > slot.3 || (admit_floor == slot.3 && slot.4.is_some() && admit_hint.is_none()) {
        slot.3 = admit_floor;
        slot.4 = admit_hint;
    }
}
```

Update the final `map` that builds `LitScene`:

```rust
cells: cells
    .into_iter()
    .map(|((i, j), (_lvl, band, tint, _hf, hint))| (i, j, band, tint, hint))
    .collect(),
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test lit_mask_tags_darkvision_only_cells_with_hint` and `cargo test scene::` → PASS. `cargo fmt && cargo clippy`.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "feat(m10e-3): player_lit_mask resolves the per-cell darkvision render hint"
```

---

### Task 4: Server — emit `renderHints` table + 5-int cells in the vision frame

**Files:**
- Modify: `src/server/src/scene/mod.rs` (`compute_derived` "vision" arm ~1068-1108; existing test `vision_payload_carries_lit_mask_for_players_not_gm` ~1358-1407)

**Interfaces:**
- Produces (player `vision` payload, additive): `{ "mode":"masked", "polygons":[…], "bands":[{name,min}…], "renderHints":[string…], "lit":[{ "scene":uuid, "cell":f64, "cells":[i,j,band,tint,hint_idx,…] }] }`. `hint_idx` = index into `renderHints`, or `-1` for none. GM stays `{mode:"all"}`.

- [ ] **Step 1: Update the existing test** (it asserts the old 4-int packing — tests yield to correct code):

```rust
let cells = lit[0]["cells"].as_array().unwrap();
assert!(!cells.is_empty());
assert_eq!(cells.len() % 5, 0, "cells packed 5 ints/cell (i,j,band,tint,hint_idx)");
assert!(!pv["bands"].as_array().unwrap().is_empty());
assert!(pv["renderHints"].is_array(), "renderHints table present at top level");
```

And add a focused assertion (new test) that a darkvision-only cell's `hint_idx` resolves to `"desaturate"`:

```rust
#[test]
fn vision_payload_resolves_render_hint_index() {
    use crate::data::document::WorldRole;
    use serde_json::json;
    let player = Uuid::from_u128(7);
    let mut tok = entity_doc(11, 10, "token", json!({ "x": 50, "y": 50 }));
    tok.owner = Some(player);
    tok.embedded.insert("actor".into(), vec![{
        let mut a = doc(99, None, "actor");
        a.system = json!({ "vision": [{ "mode": "darkvision", "range": 6 }] });
        a
    }]);
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok], 0);
    let pl = PermissionContext { user_id: player, world_role: WorldRole::Player };
    let pv = compute_derived("vision", &ecs, &pl).unwrap();
    let hints = pv["renderHints"].as_array().unwrap();
    assert!(hints.iter().any(|h| h == "desaturate"));
    let cells = pv["lit"][0]["cells"].as_array().unwrap();
    let hint_idx = cells[4].as_i64().unwrap(); // 5th int of the first cell
    assert_eq!(pv["renderHints"][hint_idx as usize], json!("desaturate"));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test vision_payload_` → FAIL (4-int packing / no `renderHints`).

- [ ] **Step 3: Implement**

In the `"vision"` masked branch, build a deterministic hint table from the mask and emit 5-int cells:

```rust
let mask = ecs.player_lit_mask(ctx.user_id);
// Deterministic renderHints table: distinct hints in first-seen order over the (sorted) mask.
let mut hints: Vec<String> = Vec::new();
let hint_idx = |h: &Option<String>, table: &mut Vec<String>| -> i64 {
    match h {
        None => -1,
        Some(s) => match table.iter().position(|x| x == s) {
            Some(i) => i as i64,
            None => { table.push(s.clone()); (table.len() - 1) as i64 }
        },
    }
};
let lit: Vec<serde_json::Value> = mask
    .into_iter()
    .map(|s| {
        let flat: Vec<i64> = s.cells.into_iter()
            .flat_map(|(i, j, band, tint, hint)| {
                let hi = hint_idx(&hint, &mut hints);
                [i as i64, j as i64, band as i64, tint as i64, hi]
            })
            .collect();
        serde_json::json!({ "scene": s.scene, "cell": s.cell, "cells": flat })
    })
    .collect();
let bands_json: Vec<serde_json::Value> = ecs.resolved_bands().into_iter()
    .map(|b| serde_json::json!({ "name": b.name, "min": b.min_illumination }))
    .collect();
Some(serde_json::json!({
    "mode": "masked", "polygons": polygons,
    "bands": bands_json, "renderHints": hints, "lit": lit,
}))
```

(Note the closure borrows `hints` mutably inside the `flat_map`; if the borrow checker objects, inline the table-building as a plain `for` loop over the mask before the `json!`.)

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test vision_payload_` and `cargo test --lib` → PASS. `cargo fmt && cargo clippy`. Then full `cargo test` (lib + integration) green.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "feat(m10e-3): vision frame carries renderHints table + per-cell hint index"
```

---

### Task 5: Client — `LightingInput` parse from the vision frame

**Files:**
- Modify: `src/client/render/src/types.ts` (add lighting value types)
- Modify: `src/client/render/src/engine.ts` (add `toLighting`; ~219-254 sits beside `toVisibility`)
- Test: `src/client/render/src/engine.test.ts`

**Interfaces:**
- Produces (in `types.ts`):

```ts
/** One visible cell's lighting: grid coords + gradation band index + packed tint + hint ref. */
export interface LitCell { i: number; j: number; band: number; tint: number; hint: number } // hint = renderHints index, -1 = none
/** Parsed lighting for the active scene (engine-internal, pre-resolution). `null` ⇒ no overlay
 * (GM `mode:"all"`, or garbled/missing data — lighting is cosmetic, fog is the secrecy gate). */
export interface LightingInput {
  cell: number;                          // active scene cell size (px)
  bands: { name: string; min: number }[]; // brightest-first
  hints: string[];                        // renderHints table
  cells: LitCell[];                       // active-scene visible cells
}
```

- Produces (in `engine.ts`): `private toLighting(payload: unknown): LightingInput | null` — active-scene-filtered, fail-safe (`null` on any non-masked / missing / malformed input).

- [ ] **Step 1: Write the failing test** (append to `engine.test.ts`). Mirror the active-scene setup the existing vision tests use; assert parse + active-scene filtering + fail-safe:

```ts
test("toLighting parses lit cells for the active scene and fails safe", () => {
  const { store, engine } = makeEngine();
  engine.start();
  // Seed an active scene "s1" (mirror the scene-create command in the fog tests).
  store.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id: "s1", scope: { kind: "world", world_id: "w1" }, doc_type: "scene", schema_version: 1,
      source: null, owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, parent_id: null, system: { grid: { kind: "square", size: 100 } }, created_at: 0, updated_at: 0,
    } }],
  });
  const li = engine.toLightingForTest({
    mode: "masked", bands: [{ name: "bright", min: 0.67 }, { name: "dim", min: 0.34 }, { name: "dark", min: 0 }],
    renderHints: ["desaturate"],
    lit: [
      { scene: "s1", cell: 100, cells: [0, 0, 2, 0, 0] },      // active: dark band, hint "desaturate"
      { scene: "other", cell: 100, cells: [9, 9, 0, 0, -1] },  // other scene: dropped
    ],
  });
  expect(li).not.toBeNull();
  expect(li!.cell).toBe(100);
  expect(li!.cells).toEqual([{ i: 0, j: 0, band: 2, tint: 0, hint: 0 }]);
  expect(li!.hints).toEqual(["desaturate"]);
  // GM / garbled → null (cosmetic, no overlay).
  expect(engine.toLightingForTest({ mode: "all" })).toBeNull();
  expect(engine.toLightingForTest({ mode: "masked", lit: "garbage" })).toBeNull();
  expect(engine.toLightingForTest(null)).toBeNull();
});
```

Add a thin test accessor on `RenderEngine`: `toLightingForTest(p: unknown) { return this.toLighting(p); }` (or make `toLighting` package-visible — match how `toVisibility` is tested; if it isn't directly tested, the accessor is the minimal seam).

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/render test -- engine` → FAIL (`toLightingForTest` undefined).

- [ ] **Step 3: Implement** `toLighting` in `engine.ts` (beside `toVisibility`):

```ts
/** Parse the `vision` payload's lighting dimension into a LightingInput for the ACTIVE scene, or
 * null. Lighting is COSMETIC — fog (toVisibility) is the secrecy gate — so any non-masked, missing,
 * or malformed input yields null (no overlay), never an over/under-reveal. Mirrors toVisibility's
 * active-scene filter so a lit set for a token in another scene cannot tint this scene. */
private toLighting(payload: unknown): LightingInput | null {
  const p = payload as {
    mode?: string;
    bands?: { name?: string; min?: number }[];
    renderHints?: string[];
    lit?: { scene?: string; cell?: number; cells?: number[] }[];
  } | null | undefined;
  if (p?.mode !== "masked" || !Array.isArray(p.lit)) return null;
  const activeScene = this.opts.store.query("scene")[0]?.id;
  const group = p.lit.find(
    (g): g is { scene?: string; cell: number; cells: number[] } =>
      !!g && g.scene === activeScene && typeof g.cell === "number" && g.cell > 0 && Array.isArray(g.cells),
  );
  if (!group) return null;
  const cells: LitCell[] = [];
  for (let k = 0; k + 4 < group.cells.length; k += 5) {
    cells.push({
      i: group.cells[k], j: group.cells[k + 1], band: group.cells[k + 2],
      tint: group.cells[k + 3], hint: group.cells[k + 4],
    });
  }
  const bands = Array.isArray(p.bands)
    ? p.bands.filter((b): b is { name: string; min: number } => !!b && typeof b.min === "number").map((b) => ({ name: String(b.name), min: b.min }))
    : [];
  const hints = Array.isArray(p.renderHints) ? p.renderHints.map(String) : [];
  return { cell: group.cell, bands, hints, cells };
}

/** Test seam (cosmetic parse has no secrecy implication). */
toLightingForTest(p: unknown): LightingInput | null { return this.toLighting(p); }
```

Import `LightingInput`, `LitCell` from `./types`.

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --filter @shadowcat/render test -- engine` → PASS. `pnpm -r typecheck`.

- [ ] **Step 5: Commit**

```bash
git add src/client/render/src/types.ts src/client/render/src/engine.ts src/client/render/src/engine.test.ts
git commit -m "feat(m10e-3): client parses the vision frame's lighting dimension"
```

---

### Task 6: Client — `Lighting` class (band→darkening resolve + day/night interpolation)

**Files:**
- Create: `src/client/render/src/lighting.ts`
- Create: `src/client/render/src/lighting.test.ts`
- Modify: `src/client/render/src/backend.ts` (add `setLighting`)
- Modify: `src/client/render/src/backend.mock.ts` (record it)
- Modify: `src/client/render/src/index.ts` (export `Lighting`, `LightingFrame`)

**Interfaces:**
- Consumes: `LightingInput | null` (Task 5).
- Produces:

```ts
/** A resolved + interpolated cell ready to draw: alpha = band darkening, tint = packed color,
 * tintAlpha (0 ⇒ no tint), desaturate from the render hint. */
export interface LitDrawCell { i: number; j: number; alpha: number; tint: number; tintAlpha: number; desaturate: boolean }
export interface LightingFrame { cell: number; cells: LitDrawCell[] }
```

`class Lighting`: `constructor(backend)`, `setTarget(input: LightingInput | null): void`, `tick(dtMs: number): void`, `current(): LightingFrame` (re-apply on resize). `DisplayBackend.setLighting(frame: LightingFrame): void`. Constants `LIGHTING_FADE_MS = 250`, `MAX_DARK_ALPHA = 0.6`.

- Resolve: `alpha = (band / max(1, bandCount - 1)) * MAX_DARK_ALPHA`; `tintAlpha = tint === 0 ? 0 : 0.25`; `desaturate = hint >= 0 && hints[hint] === "desaturate"`.
- Interpolate (day/night): on `setTarget`, keep `prev` (last drawn) + `target`. Over `LIGHTING_FADE_MS`, for cells present in BOTH (keyed `i,j`) lerp `alpha`, `tint` channels, `tintAlpha`; cells only in target or only in prev SNAP (appear/disappear immediately — visibility changes are not day/night fades). `desaturate` snaps (boolean). When a side's `tintAlpha === 0`, hold the other side's `tint` color and lerp only `tintAlpha` (avoid lerping toward black). `null` target ⇒ snap to empty (no overlay).

- [ ] **Step 1: Write the failing test** (`lighting.test.ts`):

```ts
import { test, expect } from "vitest";
import { Lighting, MockBackend } from "./index";

const bands = [{ name: "bright", min: 0.67 }, { name: "dim", min: 0.34 }, { name: "dark", min: 0 }];

test("resolves band index to darkening alpha and the desaturate hint", () => {
  const backend = new MockBackend();
  const l = new Lighting(backend);
  l.setTarget({ cell: 100, bands, hints: ["desaturate"], cells: [
    { i: 0, j: 0, band: 0, tint: 0, hint: -1 },        // bright → no darkening
    { i: 1, j: 0, band: 2, tint: 0, hint: 0 },         // dark + desaturate
  ] });
  l.tick(1000); // run any fade to completion
  const f = backend.lighting!;
  expect(f.cell).toBe(100);
  expect(f.cells.find((c) => c.i === 0)!.alpha).toBeCloseTo(0);
  expect(f.cells.find((c) => c.i === 1)!.alpha).toBeCloseTo(0.6);
  expect(f.cells.find((c) => c.i === 1)!.desaturate).toBe(true);
});

test("interpolates darkening for cells present before and after (day/night fade)", () => {
  const backend = new MockBackend();
  const l = new Lighting(backend);
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0, hint: -1 }] }); // bright
  l.tick(1000);
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 2, tint: 0, hint: -1 }] }); // → dark
  l.tick(125); // half of 250ms
  const mid = backend.lighting!.cells[0].alpha;
  expect(mid).toBeGreaterThan(0.2);
  expect(mid).toBeLessThan(0.5); // partway between 0 and 0.6
  l.tick(125);
  expect(backend.lighting!.cells[0].alpha).toBeCloseTo(0.6);
});

test("null target clears the overlay", () => {
  const backend = new MockBackend();
  const l = new Lighting(backend);
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 2, tint: 0, hint: -1 }] });
  l.tick(1000);
  l.setTarget(null);
  l.tick(0);
  expect(backend.lighting!.cells).toEqual([]);
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/render test -- lighting` → FAIL (`Lighting` not exported).

- [ ] **Step 3: Implement** `lighting.ts`:

```ts
import type { DisplayBackend } from "./backend";
import type { LightingInput } from "./types";

export interface LitDrawCell { i: number; j: number; alpha: number; tint: number; tintAlpha: number; desaturate: boolean }
export interface LightingFrame { cell: number; cells: LitDrawCell[] }

const LIGHTING_FADE_MS = 250;
const MAX_DARK_ALPHA = 0.6;
const TINT_ALPHA = 0.25;

const key = (c: { i: number; j: number }): string => `${c.i},${c.j}`;
const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;
function lerpRgb(a: number, b: number, t: number): number {
  const ar = (a >> 16) & 0xff, ag = (a >> 8) & 0xff, ab = a & 0xff;
  const br = (b >> 16) & 0xff, bg = (b >> 8) & 0xff, bb = b & 0xff;
  return (Math.round(lerp(ar, br, t)) << 16) | (Math.round(lerp(ag, bg, t)) << 8) | Math.round(lerp(ab, bb, t));
}

/** Owns the lighting layer's data. Resolves the parsed LightingInput to drawable cells (band→alpha,
 * hint→desaturate) and interpolates day/night transitions; the backend just paints LightingFrames.
 * Cosmetic only — fog is the secrecy gate. */
export class Lighting {
  private prev: LightingFrame = { cell: 0, cells: [] };
  private target: LightingFrame = { cell: 0, cells: [] };
  private elapsed = LIGHTING_FADE_MS; // start settled

  constructor(private readonly backend: DisplayBackend) {}

  setTarget(input: LightingInput | null): void {
    this.prev = this.currentInterpolated(); // fade starts from whatever is on screen now
    this.target = input ? resolve(input) : { cell: this.target.cell, cells: [] };
    this.elapsed = 0;
    this.apply();
  }

  tick(dtMs: number): void {
    if (this.elapsed >= LIGHTING_FADE_MS) return;
    this.elapsed = Math.min(LIGHTING_FADE_MS, this.elapsed + dtMs);
    this.apply();
  }

  current(): LightingFrame { return this.currentInterpolated(); }

  private apply(): void { this.backend.setLighting(this.currentInterpolated()); }

  private currentInterpolated(): LightingFrame {
    const t = LIGHTING_FADE_MS === 0 ? 1 : this.elapsed / LIGHTING_FADE_MS;
    if (t >= 1) return this.target;
    const prevByKey = new Map(this.prev.cells.map((c) => [key(c), c]));
    const cells: LitDrawCell[] = this.target.cells.map((tc) => {
      const pc = prevByKey.get(key(tc));
      if (!pc) return tc; // appeared: snap
      const tintAlpha = lerp(pc.tintAlpha, tc.tintAlpha, t);
      // Hold the present side's color when the other has no tint (avoid lerping toward black).
      const tint = pc.tintAlpha === 0 ? tc.tint : tc.tintAlpha === 0 ? pc.tint : lerpRgb(pc.tint, tc.tint, t);
      return { i: tc.i, j: tc.j, alpha: lerp(pc.alpha, tc.alpha, t), tint, tintAlpha, desaturate: tc.desaturate };
    });
    return { cell: this.target.cell, cells };
  }
}

function resolve(input: LightingInput): LightingFrame {
  const n = Math.max(1, input.bands.length - 1);
  const cells: LitDrawCell[] = input.cells.map((c) => ({
    i: c.i, j: c.j,
    alpha: (c.band / n) * MAX_DARK_ALPHA,
    tint: c.tint,
    tintAlpha: c.tint === 0 ? 0 : TINT_ALPHA,
    desaturate: c.hint >= 0 && input.hints[c.hint] === "desaturate",
  }));
  return { cell: input.cell, cells };
}
```

Add to `backend.ts` `DisplayBackend`:

```ts
/** Paint the lighting overlay (the `lighting` layer): per-cell darkening + tint + desaturate hint. */
setLighting(frame: import("./lighting").LightingFrame): void;
```

Add to `backend.mock.ts`:

```ts
lighting: import("./lighting").LightingFrame | null = null;
setLighting(frame: import("./lighting").LightingFrame): void { this.lighting = frame; }
```

Export from `index.ts`:

```ts
export { Lighting, type LightingFrame, type LitDrawCell } from "./lighting";
export type { LightingInput, LitCell } from "./types";
```

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --filter @shadowcat/render test -- lighting` → PASS. `pnpm -r typecheck`.

- [ ] **Step 5: Commit**

```bash
git add src/client/render/src/lighting.ts src/client/render/src/lighting.test.ts src/client/render/src/backend.ts src/client/render/src/backend.mock.ts src/client/render/src/index.ts
git commit -m "feat(m10e-3): Lighting class resolves bands + interpolates day/night fades"
```

---

### Task 7: Client — wire `Lighting` into the engine (layer + frame + ticker)

**Files:**
- Modify: `src/client/render/src/layers.ts` (`CoreLayerId` + `CORE_LAYERS`: add `lighting` between `templates` and `mask`)
- Modify: `src/client/render/src/engine.ts` (construct `Lighting`; thread it through `pendingDerived`/`applyDerived`; tick it)
- Test: `src/client/render/src/engine.test.ts`

**Interfaces:**
- Consumes: `Lighting` (Task 6), `toLighting` (Task 5).
- Layer order becomes: `background, grid, tiles, drawings, walls, tokens, templates, lighting, mask, overlays`. CORE_LAYERS is the engine-owned z-order contract; the `lighting` index sits at 7 (mask→8, overlays→9).

- [ ] **Step 1: Write the failing test** (append to `engine.test.ts`):

```ts
test("the lighting layer is in the core z-order between templates and mask", () => {
  const { backend, engine } = makeEngine();
  engine.start();
  const li = backend.layers.indexOf("lighting");
  expect(li).toBeGreaterThan(backend.layers.indexOf("templates"));
  expect(li).toBeLessThan(backend.layers.indexOf("mask"));
});

test("applying a derived frame drives the lighting overlay; GM clears it", () => {
  const { store, backend, engine } = makeEngine();
  engine.start();
  store.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: {
      id: "s1", scope: { kind: "world", world_id: "w1" }, doc_type: "scene", schema_version: 1,
      source: null, owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
      embedded: {}, parent_id: null, system: { grid: { kind: "square", size: 100 } }, created_at: 0, updated_at: 0,
    } }],
  });
  engine.onSceneFrameForTest({ payload: {
    mode: "masked", polygons: [], bands: [{ name: "bright", min: 0.67 }, { name: "dim", min: 0.34 }, { name: "dark", min: 0 }],
    renderHints: ["desaturate"], lit: [{ scene: "s1", cell: 100, cells: [0, 0, 2, 0, 0] }],
  }, computedAtSeq: 1 });
  backend.tick?.(1000); // settle the fade
  expect(backend.lighting!.cells.length).toBe(1);
  expect(backend.lighting!.cells[0].desaturate).toBe(true);

  engine.onSceneFrameForTest({ payload: { mode: "all" }, computedAtSeq: 2 });
  backend.tick?.(1000);
  expect(backend.lighting!.cells).toEqual([]); // GM → no overlay
});
```

Add a test seam `onSceneFrameForTest(f) { this.onSceneFrame(f); }` if `onSceneFrame` isn't already reachable from tests (mirror however the existing fog frames are driven in `engine.test.ts` — read those tests; if they call a public method, reuse it instead of adding a seam).

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm --filter @shadowcat/render test -- engine` → FAIL (`lighting` not in layers; no lighting forwarded).

- [ ] **Step 3: Implement**

`layers.ts`:

```ts
export type CoreLayerId =
  | "background" | "grid" | "tiles" | "drawings" | "walls"
  | "tokens" | "templates" | "lighting" | "mask" | "overlays";

export const CORE_LAYERS: readonly CoreLayerId[] = [
  "background", "grid", "tiles", "drawings", "walls",
  "tokens", "templates", "lighting", "mask", "overlays",
] as const;
```

`engine.ts`:
- Construct in the ctor: `this.lighting = new Lighting(opts.backend);` (add `private readonly lighting: Lighting;` field; import `Lighting`).
- Change the derived pipeline to carry lighting alongside visibility. Update `pendingDerived` to `{ input: VisibilityInput; lighting: LightingInput | null; seq: number }` and `applyDerived(input, lighting, seq)`. In `onSceneFrame`, parse both: `const input = this.toVisibility(frame.payload); const lighting = this.toLighting(frame.payload);` and thread `lighting` through the watermark/defer branches and `flushPendingDerived`.
- In `applyDerived`, after `this.renderVisibility()`, call `this.lighting.setTarget(lighting);`.
- In `start()`'s `startTicker` callback, add `this.lighting.tick(dt);`.
- Add the test seam `onSceneFrameForTest`.

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --filter @shadowcat/render test` (full render suite) → PASS. `pnpm -r typecheck && pnpm lint`.

- [ ] **Step 5: Commit**

```bash
git add src/client/render/src/layers.ts src/client/render/src/engine.ts src/client/render/src/engine.test.ts
git commit -m "feat(m10e-3): engine drives the lighting layer from the derived frame + ticker"
```

---

### Task 8: Client — PixiBackend renders the lighting layer (darkening + tint + desaturate + blur)

**Files:**
- Modify: `src/client/render/src/pixi-backend.ts` (`ensureLayers` parenting; `setLighting`)

**Interfaces:**
- Consumes: `LightingFrame` (Task 6). No unit test (GL module — Playwright-covered); gate on `typecheck` + the existing render smoke. Render technique: per cell, fill rect `(i*cell, j*cell, cell, cell)` black @ `alpha`, then the tint color @ `tintAlpha`; desaturate cells additionally get a neutral-gray wash (V1 approximation — true color-matrix desaturation is a logged follow-up). A `BlurFilter` on the lighting container softens band/edge boundaries.

- [ ] **Step 1: Implement** (no failing-test step — GL is Playwright-tier; keep the change minimal + typed).

In `pixi-backend.ts`, add a graphics + filter for the layer:

```ts
import { /* …existing… */ BlurFilter } from "pixi.js";
// fields:
private readonly lightingGraphics = new Graphics();
```

In `ensureLayers`, parent it under the `lighting` layer with a blur:

```ts
if (id === "lighting") {
  c.addChild(this.lightingGraphics);
  c.filters = [new BlurFilter({ strength: 8 })]; // smooth band/edge boundaries (POST_WORK: radial gradients)
}
```

Implement `setLighting`:

```ts
setLighting(frame: LightingFrame): void {
  this.lightingGraphics.clear();
  const s = frame.cell;
  for (const c of frame.cells) {
    const x = c.i * s, y = c.j * s;
    if (c.alpha > 0) this.lightingGraphics.rect(x, y, s, s).fill({ color: 0x000000, alpha: c.alpha });
    if (c.tintAlpha > 0) this.lightingGraphics.rect(x, y, s, s).fill({ color: c.tint, alpha: c.tintAlpha });
    // V1 desaturate approximation: a low-alpha neutral wash mutes color in darkvision-only cells.
    // POST_WORK: replace with a masked ColorMatrixFilter over the scene layers for true desaturation.
    if (c.desaturate) this.lightingGraphics.rect(x, y, s, s).fill({ color: 0x808080, alpha: 0.18 });
  }
}
```

Import `LightingFrame` type at the top (`import type { … } from "./lighting";`).

- [ ] **Step 2: Verify build + typecheck**

Run: `pnpm -r typecheck` → PASS. `pnpm --filter @shadowcat/ui build` (the embed-ordering build) → succeeds.

- [ ] **Step 3: Visual smoke (manual or existing Playwright)**

If a Playwright scene smoke exists (`grep -rl "lighting\|fog\|mask" tests/ e2e/ 2>/dev/null`), extend it to assert the `lighting` layer exists; otherwise note manual verification in the PR/commit body. Do NOT add a new Playwright harness in this task.

- [ ] **Step 4: Commit**

```bash
git add src/client/render/src/pixi-backend.ts
git commit -m "feat(m10e-3): PixiBackend paints the lighting layer (darkening + tint + blur)"
```

---

### Task 9: Full-suite green + fmt/clippy/lint

**Files:** none (verification task).

- [ ] **Step 1:** Server — from `src/server/`: `cargo test` (lib + integration) → all green; `cargo fmt --check`; `cargo clippy -- -D warnings` clean.
- [ ] **Step 2:** Client — `pnpm -r test` → green; `pnpm -r typecheck`; `pnpm lint`.
- [ ] **Step 3:** Build ordering — `pnpm --filter @shadowcat/ui build` then (from `src/server/`) `cargo build` → compiles (embed validation).
- [ ] **Step 4:** If anything fails, fix-forward (treat as an implementation failure, not a spec issue) and re-run before proceeding. No commit unless a fix was needed.

---

### Task 10: Docs + codebase-skill sync + deferral logging (doc-sync gate)

**Files:**
- Modify: `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md` (note the new engine-owned `lighting` core layer + that the client now consumes `lit`/`bands`/`renderHints`; the faithful per-cell `renderHint` path)
- Modify: `docs/PLAN.md` (M10e-3 → done, pointer to this plan)
- Modify: `docs/POST_WORK_FINDINGS.md` (log the two deferrals)
- Modify: `docs/TODO.md` only if a concrete deferral surfaced during execution

- [ ] **Step 1:** Update the scene-rendering skill: under "Key files & seams" add `src/client/render/src/lighting.ts` (Lighting class: band→darkening + hint→desaturate + day/night interpolation) and the new `lighting` core layer in the z-order; under "Hard invariants" note that lighting is cosmetic (fog stays the secrecy gate) and the per-cell hint never widens visibility. Keep it orientation+index — point into this plan, don't restate it.

- [ ] **Step 2:** Append to `docs/POST_WORK_FINDINGS.md`:

```markdown
- Title: M10e-3 lighting soft edges via blur, not gradients. Summary: the lighting layer softens
  band/edge boundaries with a single Pixi BlurFilter; per-cell radial gradients (crisper falloff)
  were deferred. Status: Revisit (cosmetic; client-render-only).
- Title: M10e-3 darkvision render is an overlay approximation. Summary: darkvision-only cells get a
  low-alpha neutral wash; true desaturation needs a masked ColorMatrixFilter over the scene layers.
  The wire payload already carries the faithful per-cell renderHint, so the refinement is
  client-render-only (no server change). Status: Revisit.
```

- [ ] **Step 3:** Update `docs/PLAN.md` M10e-3 status + plan pointer.

- [ ] **Step 4: Commit**

```bash
git add .claude/skills/shadowcat-codebase-scene-rendering/SKILL.md docs/PLAN.md docs/POST_WORK_FINDINGS.md
git commit -m "docs(m10e-3): skill + PLAN/POST_WORK sync for client lighting render"
```

---

## Self-Review (completed)

**Spec coverage (§7):** §7.1 lighting layer → Tasks 7 (layer) + 8 (render) + 6 (band→darkening); smooth edges → Task 8 blur. §7.2 smooth transitions → Task 6 interpolation + Task 7 ticker. §7.3 darkvision renderHint → Tasks 1-4 (server faithful hint) + 6/8 (client desaturate); fog integration → unchanged fog (Compositor) composites above lighting (Task 7 z-order). Server payload extension (brainstorm decision 2) → Tasks 1-4.

**Type consistency:** `render_hint`/`renderHint` (server `Option<String>` ↔ wire `renderHints[]` + `hint_idx`); `LightingInput`/`LitCell` (parsed) → `LightingFrame`/`LitDrawCell` (resolved); `setLighting` consistent across `DisplayBackend`/`MockBackend`/`PixiBackend`/`Lighting`; `CORE_LAYERS` `lighting` index consistent in `layers.ts` + the engine z-order assertions.

**Placeholders:** none — every code/test step carries concrete content.

## Buddy-check directives

This checkpoint extends the M10e-2 lighting-aware secrecy gate (server mask computation + the additive `vision` egress) and adds an engine z-order layer. High-risk signals: touches the per-player secrecy-adjacent mask; mirrors a client resolver (the `renderHint` seed must equal `scene-docs.ts`); a wire-format change (4→5 ints/cell) with a determinism constraint. Per the established M10 cadence, after Task 10 run a **whole-branch buddy-check** (`buddy-checking`): dispatch `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` as the two-reviewer pair, reconcile to convergence. Specific things for reviewers to probe: (1) the hint never widens visibility (cosmetic-only) — verify the mask's `visible_floor` admission is unchanged from M10e-2; (2) the server seed/parse equals the client `SEED_VISION_MODES` exactly; (3) `renderHints` table order is deterministic; (4) lighting fails to *no overlay* (not over-darken) on garbled input while fog still fails closed; (5) the `lighting` core-layer index shift (mask 7→8) doesn't break any existing module-layer fractional-order assumption.
