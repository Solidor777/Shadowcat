# M10f-0 — Scene Bounds Primitive — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an explicit per-scene `bounds { width, height }` (grid units) to the scene document — the foundational primitive the M10f navmesh triangulates — resolved fail-closed to a finite default on both client and server, with a GM authoring control.

**Architecture:** The scene `system` body is **client-owned / server-structural** (ARCHITECTURE §2 #6): the client defines the `SceneSystem` shape and resolves it (`scene-docs.ts`); the server parses the opaque JSON structurally (`serde_json::Value::pointer`, fail-closed `unwrap_or`). `bounds` follows that exact pattern — a per-scene field with a **fixed finite default** (not world-inherited, not content-derived), a client resolver + a mirrored server parse, and a whole-object authoring write in `module-game-settings`. No navmesh, no rebuild trigger, no protocol frame — those land in M10f-1.

**Tech Stack:** Rust (server, `serde_json`), TypeScript (`@shadowcat/core`), Svelte 5 Runes + `@testing-library/svelte` (`@shadowcat/module-game-settings`), i18n catalog (`@shadowcat/ui-kit`).

## Global Constraints

- **Client/server default parity (load-bearing):** `DEFAULT_SCENE_BOUNDS = { width: 100, height: 100 }` (TS) and `DEFAULT_SCENE_BOUNDS_UNITS: (f64, f64) = (100.0, 100.0)` (Rust) MUST hold identical values. Units are **grid cells**, not pixels.
- **Fail-closed resolution:** a present-but-malformed `bounds` (non-finite, `≤ 0` on either axis, wrong type) resolves to the default — never a degenerate/zero/negative rectangle (a navmesh would fail to triangulate it). Never throws.
- **`bounds` is per-scene only** — it has NO world-settings default layer (each scene has its own size). The only fallback is the fixed constant.
- **Not content-derived:** the default is a fixed constant, deliberately NOT an AABB of walls/tokens (content-derived bounds were rejected at design time: edge-drag re-mesh churn, ill-defined for open scenes).
- **Scene `system` is client-owned:** there is **no ts-rs struct** for the scene system body and **no Zod drift-guard** for scene-system fields — do not attempt to regenerate ts-rs or edit a Zod scene-system schema for `bounds`. (Verified: the server reads it via `Value::pointer`.)
- **Cross-platform:** pure logic; no filesystem/path/OS-specific code.
- Every client task runs BOTH `test` and `typecheck` (vitest strips types at test time; a type error surfaces only under `typecheck` — see the project lesson on this).

## Model/Effort directives

- **Plan authored mainline** in this session (Opus 4.8, effort high), per the user's tier-switch choice at the writing-plans handoff — the design was fully in-context from the M10f brainstorm.
- **Execution tiering:** default SDD ladder. `sdd-implementer` (Sonnet, effort medium) per task; escalate to `-highthink` / `-opus` on a BLOCKED/DONE_WITH_CONCERNS report. Per-task two-reviewer gate = `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (effort high). Full ladder: `~/.claude/docs/sdd-model-effort-tiers.md`.

## Buddy-check directives

**No buddy-check for M10f-0.** Risk is low: an optional additive scene-doc field with a fail-closed resolver + a GM editor; no secrecy/vision-gate surface, no concurrency, no protocol change, no server authority over game logic. The per-task two-reviewer gate + the SDD whole-branch final review are sufficient. (The high-risk M10f slices — M10f-2's executor refactor and M10f-3's observer-clip leak surface — carry buddy-check directives in *their* plans, not this one.)

---

### Task 1: Client scene-bounds type + resolver

**Files:**
- Modify: `src/client/core/src/scene-docs.ts` (add `SceneDimensions`, `DEFAULT_SCENE_BOUNDS`, `bounds?` on `SceneSystem`, `bounds` on `ResolvedSceneSettings`, resolution in `resolveSceneSettings`, `buildSceneDoc` support; fix the stale "Dimensions deferred" comment)
- Test: `src/client/core/src/scene-docs.test.ts`

**Interfaces:**
- Consumes: `SceneSystem`, `ResolvedSceneSettings`, `resolveSceneSettings(scene, store)`, `buildSceneDoc(worldId, system?, id?)`, `storeWith(...docs)` (existing test helper, `scene-docs.test.ts:11`).
- Produces:
  - `export interface SceneDimensions { width: number; height: number; }`
  - `export const DEFAULT_SCENE_BOUNDS: SceneDimensions` = `{ width: 100, height: 100 }`
  - `SceneSystem.bounds?: SceneDimensions`
  - `ResolvedSceneSettings.bounds: SceneDimensions` (always populated; default when absent/malformed)

- [ ] **Step 1: Write the failing tests**

Add to `src/client/core/src/scene-docs.test.ts` inside the existing `describe("resolveSceneSettings", …)` block. Import `DEFAULT_SCENE_BOUNDS` and (if not already) `SceneSystem`, `SceneDimensions`, `buildSceneDoc` at the top of the file.

```ts
it("absent bounds resolves to DEFAULT_SCENE_BOUNDS", () => {
  const scene = buildSceneDoc("w1", {}, "scene1");
  const r = resolveSceneSettings(scene, storeWith(scene));
  expect(r.bounds).toEqual({ width: 100, height: 100 });
});

it("explicit bounds pass through", () => {
  const scene = buildSceneDoc("w1", { bounds: { width: 40, height: 25 } }, "scene1");
  const r = resolveSceneSettings(scene, storeWith(scene));
  expect(r.bounds).toEqual({ width: 40, height: 25 });
});

it("malformed bounds fail closed to the default", () => {
  const scene = buildSceneDoc("w1", {}, "scene1");
  // Non-positive on either axis is degenerate for a navmesh rectangle → default.
  (scene.system as SceneSystem).bounds = { width: 0, height: -5 } as SceneDimensions;
  const r = resolveSceneSettings(scene, storeWith(scene));
  expect(r.bounds).toEqual(DEFAULT_SCENE_BOUNDS);
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test scene-docs`
Expected: FAIL — `r.bounds` is `undefined` (property does not exist yet); the explicit/malformed cases also fail.

- [ ] **Step 3: Implement the type, default, and resolution**

In `src/client/core/src/scene-docs.ts`:

3a. After the `GridDistance` interface (currently line 17), add:

```ts
/** A scene's authored dimensions in GRID UNITS (width × height cells). The M10f navmesh
 * triangulates this bounded rectangle; grid A* never needs it. Absent ⇒ DEFAULT_SCENE_BOUNDS. */
export interface SceneDimensions { width: number; height: number; }

/** Fail-safe finite default scene size (grid units) when a scene has no authored `bounds`, so
 * navmesh construction never faces an unbounded plane. MUST match DEFAULT_SCENE_BOUNDS_UNITS in
 * the server `scene/mod.rs`. Deliberately a fixed constant — NOT a content AABB (content-derived
 * bounds were rejected: edge-drag re-mesh churn, ill-defined for open scenes). */
export const DEFAULT_SCENE_BOUNDS: SceneDimensions = { width: 100, height: 100 };
```

3b. In `SceneSystem` (currently line 36-42): add the `bounds` field and correct the stale comment.

```ts
/** A scene's engine-owned config (M8d §15, extended M10e-1). `bounds` (M10f-0) = the navmesh's
 * outer rectangle in grid units; absent ⇒ DEFAULT_SCENE_BOUNDS. */
export interface SceneSystem {
  grid: { kind: "square" | "hex"; size: number; distance?: GridDistance };
  background: string | null;
  bounds?: SceneDimensions;
  vision?: SceneVisionOverrides;
  lighting?: SceneLightingOverrides;
}
```

3c. In `ResolvedSceneSettings` (currently line 95-107): add `bounds`.

```ts
  gridDistance: GridDistance;
  bounds: SceneDimensions;
```

3d. Add a fail-closed resolve helper (place it directly above `resolveSceneSettings`):

```ts
/** Fail-closed bounds resolve: a present-but-malformed bounds (non-finite or ≤ 0 on either
 * axis) falls back to the finite default rather than yielding a degenerate navmesh rectangle. */
function resolveBounds(b: SceneDimensions | undefined): SceneDimensions {
  const w = b?.width, h = b?.height;
  if (typeof w === "number" && Number.isFinite(w) && w > 0 &&
      typeof h === "number" && Number.isFinite(h) && h > 0) {
    return { width: w, height: h };
  }
  return DEFAULT_SCENE_BOUNDS;
}
```

3e. In the object returned by `resolveSceneSettings` (currently ends at line 231-232 with `gridDistance`), add:

```ts
    gridDistance: sys?.grid?.distance ?? { perCell: 5, unit: "ft" },
    bounds: resolveBounds(sys?.bounds),
  };
```

3f. In `buildSceneDoc` (currently line 186-194), thread `bounds` through the optional-include spread:

```ts
  const full: SceneSystem = {
    grid: system.grid ?? { kind: "square", size: 100 },
    background: system.background ?? null,
    ...(system.bounds ? { bounds: system.bounds } : {}),
    ...(system.vision ? { vision: system.vision } : {}),
    ...(system.lighting ? { lighting: system.lighting } : {}),
  };
```

- [ ] **Step 4: Run tests + typecheck to verify they pass**

Run: `pnpm --filter @shadowcat/core test scene-docs`
Expected: PASS (all three new tests + the existing `resolveSceneSettings` suite).

Run: `pnpm --filter @shadowcat/core typecheck`
Expected: PASS (no type errors — vitest strips types, so this is the real type gate).

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/scene-docs.ts src/client/core/src/scene-docs.test.ts
git commit -m "feat(m10f-0): client scene bounds type + fail-closed resolver"
```

---

### Task 2: Server scene-bounds resolve

**Files:**
- Modify: `src/server/src/scene/mod.rs` (add `DEFAULT_SCENE_BOUNDS_UNITS` const, `bounds` field on `ResolvedScene`, parse in `resolve_scene`)
- Test: `src/server/src/scene/mod.rs` (unit tests alongside the existing `resolve_scene_*` tests)

**Interfaces:**
- Consumes: `ResolvedScene` (struct at `mod.rs:52`), `resolve_scene(&self, scene: Uuid)` (`mod.rs:379`), the scene-system JSON accessor `let s = scene_sys.as_ref();` and its `serde_json::Value::pointer` reads (`mod.rs:433+`); test helpers `SceneEcs::new()`, `ecs.insert_scene_for_test(scene_id, json!({…}))` (`mod.rs:2619`).
- Produces: `ResolvedScene.bounds: (f64, f64)` — `(width, height)` in grid units, always finite `> 0` (default `(100.0, 100.0)`). Consumed by the M10f-1 navmesh adapter.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)]` module in `src/server/src/scene/mod.rs`, next to the existing `resolve_scene_*` tests:

```rust
#[test]
fn resolve_scene_bounds_defaults_when_absent() {
    let ecs = SceneEcs::new();
    let r = ecs.resolve_scene(Uuid::from_u128(1));
    assert_eq!(r.bounds, (100.0, 100.0));
}

#[test]
fn resolve_scene_bounds_reads_authored_value() {
    use serde_json::json;
    let mut ecs = SceneEcs::new();
    let scene_id = Uuid::from_u128(2);
    ecs.insert_scene_for_test(
        scene_id,
        json!({
            "grid": { "kind": "square", "size": 100 },
            "bounds": { "width": 40.0, "height": 25.0 }
        }),
    );
    let r = ecs.resolve_scene(scene_id);
    assert_eq!(r.bounds, (40.0, 25.0));
}

#[test]
fn resolve_scene_bounds_fail_closed_on_degenerate() {
    use serde_json::json;
    let mut ecs = SceneEcs::new();
    let scene_id = Uuid::from_u128(3);
    // Zero width + negative height are degenerate for a navmesh rectangle → default.
    ecs.insert_scene_for_test(
        scene_id,
        json!({
            "grid": { "kind": "square", "size": 100 },
            "bounds": { "width": 0.0, "height": -5.0 }
        }),
    );
    let r = ecs.resolve_scene(scene_id);
    assert_eq!(r.bounds, (100.0, 100.0));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run (from `src/server/`): `cargo test --lib resolve_scene_bounds`
Expected: FAIL to compile — `ResolvedScene` has no field `bounds`.

- [ ] **Step 3: Implement the const, field, and parse**

3a. Near the top of `src/server/src/scene/mod.rs` (by the other scene constants; place above `ResolvedScene`), add:

```rust
/// Fail-safe finite default scene size (grid units) when a scene has no authored `bounds`.
/// MUST match `DEFAULT_SCENE_BOUNDS` in the client `scene-docs.ts` (client/server parity).
pub const DEFAULT_SCENE_BOUNDS_UNITS: (f64, f64) = (100.0, 100.0);
```

3b. In `ResolvedScene` (struct at `mod.rs:52`), add the field (after `partial_cell_leniency`):

```rust
    pub partial_cell_leniency: bool,
    /// Scene dimensions (width, height) in grid units. Always finite `> 0`
    /// (default `DEFAULT_SCENE_BOUNDS_UNITS`). The M10f navmesh's outer rectangle.
    pub bounds: (f64, f64),
```

3c. In `resolve_scene`, in the scene-override layer (after the `move_str` read at `mod.rs:458-461`, before the `ResolvedScene { … }` construction), add a fail-closed parse. `s` is the `Option<&serde_json::Value>` scene system (`mod.rs:433`).

```rust
    // Scene bounds (M10f-0): per-scene, no world default — a fixed finite fallback. A
    // non-finite or non-positive axis is degenerate for a navmesh rectangle → fail closed.
    let bounds = {
        let w = s.and_then(|s| s.pointer("/bounds/width")).and_then(|v| v.as_f64());
        let h = s.and_then(|s| s.pointer("/bounds/height")).and_then(|v| v.as_f64());
        match (w, h) {
            (Some(w), Some(h)) if w.is_finite() && w > 0.0 && h.is_finite() && h > 0.0 => (w, h),
            _ => DEFAULT_SCENE_BOUNDS_UNITS,
        }
    };
```

3d. Add `bounds` to the `ResolvedScene { … }` literal (after `partial_cell_leniency: d_lenient,` at `mod.rs:476`):

```rust
            partial_cell_leniency: d_lenient,
            bounds,
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run (from `src/server/`): `cargo test --lib resolve_scene_bounds`
Expected: PASS (3 tests).

Run (from `src/server/`): `cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: clean (no fmt diff, no clippy warnings).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/mod.rs
git commit -m "feat(m10f-0): server structural parse of scene bounds (fail-closed)"
```

---

### Task 3: GM bounds authoring control

**Files:**
- Modify: `src/modules/game-settings/src/GameSettingsPanel.svelte` (two number inputs in the scene-override block; whole-object `/system/bounds` write)
- Modify: `src/client/ui-kit/src/locales/en.ts` (three new i18n keys)
- Test: `src/modules/game-settings/src/scene-overrides.test.ts`

**Interfaces:**
- Consumes: `setScene(path, value)` (`GameSettingsPanel.svelte:80` — dispatches `[{ op: "update", doc_id: scene.id, changes: [{ path, old: null, new: value }] }]`); `ssys` (`$derived` selected scene `SceneSystem`, `:76`); `DEFAULT_SCENE_BOUNDS` + `SceneDimensions` from `@shadowcat/core`; test harness `render(GameSettingsPanel, { context: setAppContextForTest({ role, world, documents: gmStoreWith(...), dispatchIntent }) })`, `screen.getByLabelText`, `fireEvent.change` (`scene-overrides.test.ts:2,7,18`).
- Produces: no exported symbols — a UI control writing `/system/bounds` as a whole `{ width, height }` object (whole-object write mirrors the environment editor, since `set_pointer` cannot create a missing parent object from a sub-path).

- [ ] **Step 1: Write the failing tests**

Add to `src/modules/game-settings/src/scene-overrides.test.ts` inside the existing `describe("per-scene overrides", …)`:

```ts
it("setting scene bounds width writes the whole /system/bounds object (height defaults)", async () => {
  const dispatchIntent = vi.fn();
  const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
  const scene = buildSceneDoc("w1", {}, "scene1");
  render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws, scene), dispatchIntent }) });

  const input = screen.getByLabelText("gameSettings.scene.boundsWidth") as HTMLInputElement;
  await fireEvent.change(input, { target: { value: "40" } });

  // No prior bounds → height falls back to DEFAULT_SCENE_BOUNDS.height (100).
  expect(dispatchIntent).toHaveBeenCalledWith([
    { op: "update", doc_id: "scene1", changes: [{ path: "/system/bounds", old: null, new: { width: 40, height: 100 } }] },
  ]);
});

it("setting scene bounds height preserves an existing authored width", async () => {
  const dispatchIntent = vi.fn();
  const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
  const scene = buildSceneDoc("w1", { bounds: { width: 30, height: 30 } }, "scene1");
  render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws, scene), dispatchIntent }) });

  const input = screen.getByLabelText("gameSettings.scene.boundsHeight") as HTMLInputElement;
  await fireEvent.change(input, { target: { value: "50" } });

  expect(dispatchIntent).toHaveBeenCalledWith([
    { op: "update", doc_id: "scene1", changes: [{ path: "/system/bounds", old: null, new: { width: 30, height: 50 } }] },
  ]);
});
```

Ensure the test file imports `buildSceneDoc`, `buildWorldSettingsDoc` (already imported per the existing tests) — add nothing else.

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/module-game-settings test scene-overrides`
Expected: FAIL — `getByLabelText("gameSettings.scene.boundsWidth")` throws (no such control).

- [ ] **Step 3: Add i18n keys**

In `src/client/ui-kit/src/locales/en.ts`, in the `gameSettings.scene.*` block (after `"gameSettings.scene.distanceUnit"` at line 107), add:

```ts
  "gameSettings.scene.bounds": "Scene size (grid units)",
  "gameSettings.scene.boundsWidth": "Scene width (cells)",
  "gameSettings.scene.boundsHeight": "Scene height (cells)",
```

- [ ] **Step 4: Add the authoring control**

4a. In `src/modules/game-settings/src/GameSettingsPanel.svelte`, extend the `@shadowcat/core` import (the one already bringing in `type SceneSystem`, `:8`) to also import the bounds default:

```ts
    type SceneSystem, type WireDocument, DEFAULT_SCENE_BOUNDS,
```

4b. Add a small helper near `setScene` (`:80`) that writes the whole bounds object, preserving the other axis:

```ts
  // Whole-object write: set_pointer cannot create a missing /system/bounds parent from a
  // sub-path, so we always dispatch the full { width, height } (mirrors the environment editor).
  // The unedited axis falls back to the current authored value, else DEFAULT_SCENE_BOUNDS.
  function setBounds(axis: "width" | "height", value: number): void {
    const cur = ssys?.bounds ?? DEFAULT_SCENE_BOUNDS;
    setScene("/system/bounds", { ...cur, [axis]: value });
  }
```

4c. In the scene-override block (the `{#if scene}` section that renders the per-scene controls — after the distance-per-cell inputs), add two number inputs. Follow the existing number-input shape (`gameSettings.scene.distancePerCell`, `:32`):

```svelte
    <label>
      {ctx.t("gameSettings.scene.boundsWidth")}
      <input type="number" min="1" step="1" aria-label="gameSettings.scene.boundsWidth"
        value={ssys?.bounds?.width ?? DEFAULT_SCENE_BOUNDS.width}
        onchange={(e) => setBounds("width", Number((e.currentTarget as HTMLInputElement).value))} />
    </label>
    <label>
      {ctx.t("gameSettings.scene.boundsHeight")}
      <input type="number" min="1" step="1" aria-label="gameSettings.scene.boundsHeight"
        value={ssys?.bounds?.height ?? DEFAULT_SCENE_BOUNDS.height}
        onchange={(e) => setBounds("height", Number((e.currentTarget as HTMLInputElement).value))} />
    </label>
```

(Bounds is per-scene with a fixed default — it is NOT an inherit-from-world tri-state, so unlike the vision/lighting overrides these are plain number inputs, no "" inherit option.)

- [ ] **Step 5: Run tests + typecheck to verify they pass**

Run: `pnpm --filter @shadowcat/module-game-settings test scene-overrides`
Expected: PASS (both new tests + the existing per-scene-override suite).

Run: `pnpm --filter @shadowcat/module-game-settings typecheck`
Expected: PASS.

Run (i18n key-parity guard, if the repo enforces locale completeness): `pnpm --filter @shadowcat/ui-kit typecheck`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/modules/game-settings/src/GameSettingsPanel.svelte src/modules/game-settings/src/scene-overrides.test.ts src/client/ui-kit/src/locales/en.ts
git commit -m "feat(m10f-0): GM scene-bounds authoring control"
```

---

## Final verification (run before declaring the checkpoint done)

- [ ] Full client suite: `pnpm -r test` → all green.
- [ ] Full client typecheck: `pnpm -r typecheck` → all green.
- [ ] Lint: `pnpm lint` → clean.
- [ ] Server (from `src/server/`): `cargo test` → green; `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings` → clean.
- [ ] **Reviewed skill-update gate:** update `shadowcat-codebase-scene-rendering` — add the new `scene.system.bounds` field + `DEFAULT_SCENE_BOUNDS`/`DEFAULT_SCENE_BOUNDS_UNITS` parity constant to the scene-doc/`resolve_scene` notes, and mark scene dimensions as no-longer-deferred (the "Dimensions deferred" gotcha is now false for `bounds`). Confirm the diff via `shadowcat-spec-reviewer`. (M10f-0 touches the scene-settings seam that skill documents, so this is NOT a trivial-no-touch change.)
- [ ] `docs/PLAN.md`: mark M10f-0 done under the M10f entry; note bounds now unblocks the M10e-2 edge-light deviation (implementation still homed to M12).

## Self-review notes (author)

- **Spec coverage:** M10f spec §4.1 (bounds data model), §5.1 (default-bound fallback, fail-closed, not content-derived), §9 (authoring in `module-game-settings`), §12 M10f-0 line item — all covered by Tasks 1-3. Rebuild-trigger + navmesh consumption are correctly OUT (M10f-1), matching the spec's "M10f-0 ships bounds only." Edge-projected light stays homed to M12 (final-verification note only).
- **Type consistency:** `SceneDimensions {width,height}` used identically in `SceneSystem`, `ResolvedSceneSettings`, `DEFAULT_SCENE_BOUNDS`, `setBounds`, and both test suites; server `(f64,f64)` `(width,height)` order matches the client `{width,height}`; default `100×100` grid units identical across `DEFAULT_SCENE_BOUNDS` (TS) and `DEFAULT_SCENE_BOUNDS_UNITS` (Rust) per the Global Constraint.
- **No placeholders:** every step carries real code + exact commands + expected output.
