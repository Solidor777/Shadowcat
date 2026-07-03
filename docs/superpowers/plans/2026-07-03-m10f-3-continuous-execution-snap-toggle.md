# M10f-3 — Continuous Execution + Scene-Level Snap Toggle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the client-side refusal that is the ONLY thing preventing continuous
(any-angle, gridless) scenes from executing server-authoritative gated movement, and add a new
independent scene-level `snapToGrid` authoring axis enforced at one render-engine chokepoint.
Server-side, this checkpoint adds tests only (proving what M10f-2 already built is engine-
agnostic) — zero new production Rust code.

**Architecture:**
1. **Continuous execution:** `controller.svelte.ts`'s `commitRoute` has an early-return that
   refuses to send a `moveRequest` when the active scene's `movementModel` is `"continuous"`.
   Deleting it lets `commitRoute` proceed identically for both engines — the server's
   `handle_move_request` → `Room::execute_move` → `move_exec::execute_move` (via `gate_walk`) →
   `move_stream::sample_path` → per-recipient `MoveStream` clip path has had no `movementModel`
   branch since M10f-2, so it already gates/executes/streams any polyline (grid cell-centers or
   any-angle vertices) correctly. This plan proves that with new server tests, it does not change
   server production code.
2. **Snap toggle:** a new `SceneSystem.snapToGrid?: boolean` (opaque JSON, no ts-rs type) resolved
   in `resolveSceneSettings` with a derived default (`false` for a continuous scene, `true`
   otherwise, unless explicitly overridden in either direction). Enforced at a single chokepoint —
   `RenderEngine.snap` — gated by a new `setSnapEnabled` seam on the `SceneToolHost` interface,
   forwarded through `SceneInteractionBridge`, pushed from `Stage.svelte`'s existing scene-settings
   effect, and authored via a new GM toggle button in `ToolRail.svelte`.

**Tech Stack:** TypeScript (Svelte 5 runes, `@shadowcat/core`/`render`/`ui-kit`/
`module-stage`/`module-scene-tools`), Rust (`shadowcat` server crate, tests only). No new
dependencies.

## Global Constraints

- **No new server production code.** `move_exec.rs`, `move_stream.rs`, `room.rs`'s
  `Room::execute_move`, and `conn.rs`'s `clip_move_stream`/`egress_loop` are untouched except for
  new `#[cfg(test)]` test functions. If any server test in this plan requires production-code
  changes to pass, STOP and report a spec deviation — do not silently patch production code to
  make a test pass; the design's central claim (server is already engine-agnostic) would be false
  and needs human review, not a workaround.
- **No new dependencies, no ts-rs types.** `snapToGrid` and `movementModel` are both opaque
  `system`-body JSON, client-owned/server-structural — never add a Rust struct field or a ts-rs
  derive for either.
- **`snapToGrid` is genuinely scene-level, not per-client.** It rides the scene document (shared
  state via the normal document sync path), never a local/ephemeral UI toggle.
- **`RenderEngine.snap` gating must not affect grid rendering.** A snap-off scene may still
  display its reference grid; `setSnapEnabled` governs the `snap()` call chain only, never
  `redrawGrid`/grid line drawing.
- Verification commands throughout:
  - TypeScript: `pnpm --filter <package> test -- <file>`, escalating to
    `pnpm -r typecheck && pnpm lint && pnpm -r test` whenever a task changes a SHARED interface
    (`ResolvedSceneSettings`, `SceneToolHost`) that other packages/tests construct or implement.
  - Rust: `cargo test -p shadowcat --lib <module path>`, `cargo fmt`,
    `cargo clippy -p shadowcat --lib -- -D warnings`.
- No cargo-bloat check this checkpoint (no new Rust dependencies).

---

## File Structure

- **`src/client/core/src/scene-docs.ts`** (Task 1) — `SceneSystem.snapToGrid?: boolean`,
  `ResolvedSceneSettings.snapToGrid: boolean`, `buildSceneDoc` conditional include,
  `resolveSceneSettings` derived-default resolution.
- **`src/client/render/src/types.ts`** (Task 2) — `SceneToolHost.setSnapEnabled(enabled: boolean): void`.
- **`src/client/render/src/engine.ts`** (Task 2) — `RenderEngine`'s `#snapEnabled`-gated `snap()`
  (private field named `snapEnabled`, matching this class's existing `private` field convention)
  + `setSnapEnabled` implementation.
- **`src/client/ui-kit/src/sceneInteraction.ts`** (Task 3) — `SceneInteractionBridge.setSnapEnabled`
  forward, no-op when detached.
- **`src/client/ui-kit/src/__fixtures__/fakeSceneHost.ts`** (Task 3) — default no-op for the new
  seam (shared test fixture; non-breaking for existing callers).
- **`src/modules/stage/src/Stage.svelte`** (Task 4) — resolves + pushes `snapToGrid` in the
  existing `onDocs` reactive callback, alongside grid size + `diagonalRule` + animation.
- **`src/modules/scene-tools/src/ToolRail.svelte`** (Task 5) — GM-only persistent snap-toggle
  button, reflecting the resolved field, dispatching a `/system/snapToGrid` update on click.
- **`src/client/ui-kit/src/locales/en.ts`** (Task 5) — new `tools.snap` string.
- **`src/modules/scene-tools/src/controller.svelte.ts`** (Task 6) — removes the `commitRoute`
  continuous-scene refusal + its now-unused `resolveSceneSettings` import.
- **`src/modules/scene-tools/src/measure-tool.test.ts`** (Task 6) — rewrites the test that
  asserted the removed refusal to assert the move now fires.
- **`src/server/src/ws/room.rs`** (Task 7) — new `#[cfg(test)]` helper + two new
  `Room::execute_move` continuous-scene tests (any-angle success, cell-gate rejection).
- **`src/server/src/scene/move_stream.rs`** (Task 8) — new `#[cfg(test)]` any-angle
  `sample_path` unit test.
- **`src/server/src/ws/conn.rs`** (Task 9) — new `#[cfg(test)]` any-angle `clip_move_stream`
  no-leak test.

---

## Model/Effort directives

- **Plan-writer:** this plan was written by a dispatched plan-writing subagent (Sonnet 5, effort
  high), per the dispatching session's explicit instruction.
- **Dispatcher tier:** the calling (mainline) session acts as its own SDD dispatcher at
  Sonnet/medium, per explicit user directive — the loop is NOT delegated to the `sdd-dispatcher`
  subagent.
- **Execution tier:** `shadowcat-coder` (sonnet, effort medium) per task, escalating to
  `shadowcat-coder-opus` (opus, effort high, identical body) on any BLOCKED/DONE_WITH_CONCERNS
  report — per this project's CLAUDE.md `## Codebase Skills & Agents` rule (never the generic
  `sdd-implementer`).
- **Reviewers:** `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (both effort high) as the
  per-task two-reviewer pair, escalating to their `-opus` twins if a base-tier review reads
  shallow/uncertain or the base coder reports BLOCKED.
- **Final whole-branch review:** a two-reviewer buddy-check pass on Opus (two independent blind
  reviewers, debate to convergence) — NOT `sdd-final-reviewer`. See Buddy-check directives below.

## Buddy-check directives

This checkpoint touches the server-authoritative movement/secrecy-gate class that every prior
M1/M2/M3/M10f checkpoint has required heavy buddy-check on (PLAN.md's M10f-1/M10f-2 entries record
buddy-check repeatedly catching real Critical/Important defects in this exact code class). Even
though this checkpoint's server change is test-only, the CLIENT change (removing the `commitRoute`
refusal) is the one thing that newly EXPOSES continuous execution to real users, so it is treated
with the same rigor.

- **Standard single-reviewer gate:** Tasks 1–5 (client data-model/chokepoint/UI scaffolding —
  the server remains the sole enforcement point for all of these; a defect here is a UX bug, not
  a secrecy leak).
- **Pre-authorized for per-task buddy-check** (two independent blind reviewers, debate to
  convergence):
  - **Task 6** (removes the `commitRoute` refusal) — even though the server enforces regardless,
    verify there is no OTHER client-side assumption anywhere in the changed file that silently
    relied on continuous scenes never reaching `moveRequest`.
  - **Tasks 7, 8, 9** (the continuous-scene server test coverage) — per explicit dispatch
    instruction, verify these tests actually PROVE the cell-gate/no-leak invariants (reject into
    genuinely unseen space, clip an genuinely occluded any-angle sample), not merely that the code
    runs without panicking. A test with a wrong hand-derived geometry literal that happens to pass
    is a known failure mode in this exact code class (M10f-2 Task 6 fixture-derivation history).
- **Whole-branch buddy-check (mandatory):** after all 9 tasks are committed, before merge, run a
  whole-branch buddy-check (two independent blind Opus reviewers, debate to convergence) across
  the assembled diff. Specifically direct the reviewers to check:
  1. **Does removing the `commitRoute` guard introduce any way for a continuous move to bypass
     the cell-sampled gate** — trace the full path from the client's `moveRequest` call through
     `handle_move_request` → `Room::execute_move` → `move_exec::execute_move` and confirm no
     branch anywhere admits an ungated polyline for a continuous scene.
  2. **Does the snap-toggle's derived-default logic ever silently produce an inconsistent
     resolved state** — e.g. does `resolveSceneSettings` ever return a `snapToGrid` value computed
     from a STALE `movementModel` (a partial-update race), or a value that disagrees between two
     call sites resolving the same scene at the "same" logical moment.
  3. **Is the `snapToGrid` field genuinely scene-level/shared (not per-client) end to end** —
     confirm the ToolRail toggle writes the scene DOCUMENT (not local component state), that
     `Stage.svelte` re-resolves from the document store (not a cached value), and that a second
     client observing the same scene sees the same resolved value after the doc syncs.
- **Reviewed skill-update gate:** update `shadowcat-codebase-scene-rendering` — continuous
  execution now wired end-to-end (the `commitRoute` refusal removed), the new `snapToGrid` scene
  axis + its derived default, the `RenderEngine.snap` chokepoint + `SceneToolHost.setSnapEnabled`
  seam — and confirm accurate via `shadowcat-spec-reviewer` before merge. This is the final step
  before merge (see Completion checklist).

---

## Task 1: `snapToGrid` data model + resolver default (`@shadowcat/core`)

**Files:**
- Modify: `src/client/core/src/scene-docs.ts`
- Test: `src/client/core/src/scene-docs.test.ts`

**Interfaces:**
- Produces: `SceneSystem.snapToGrid?: boolean`, `ResolvedSceneSettings.snapToGrid: boolean`,
  resolved by `resolveSceneSettings`.
- Consumed by: Task 4 (`Stage.svelte`), Task 5 (`ToolRail.svelte`).

- [ ] **Step 1: Write the failing tests**

In `src/client/core/src/scene-docs.test.ts`, add inside the `describe("resolveSceneSettings", ...)`
block, immediately after the existing `it("movementModel: null scene override inherits world", ...)`
test (right before the block's closing `});`):

```ts
  it("snapToGrid defaults to true for a grid-stepped scene", () => {
    const scene = buildSceneDoc("w1", {}, "scene1");
    const r = resolveSceneSettings(scene, storeWith(scene));
    expect(r.movementModel).toBe("grid-stepped");
    expect(r.snapToGrid).toBe(true);
  });

  it("snapToGrid defaults to false for a continuous scene (derived default, spec §4.1)", () => {
    const scene = buildSceneDoc("w1", { vision: { movementModel: "continuous" } }, "scene1");
    const r = resolveSceneSettings(scene, storeWith(scene));
    expect(r.movementModel).toBe("continuous");
    expect(r.snapToGrid).toBe(false);
  });

  it("snapToGrid: an explicit true overrides the continuous default", () => {
    const scene = buildSceneDoc(
      "w1",
      { vision: { movementModel: "continuous" }, snapToGrid: true },
      "scene1",
    );
    const r = resolveSceneSettings(scene, storeWith(scene));
    expect(r.snapToGrid).toBe(true);
  });

  it("snapToGrid: an explicit false overrides the grid-stepped default", () => {
    const scene = buildSceneDoc("w1", { snapToGrid: false }, "scene1");
    const r = resolveSceneSettings(scene, storeWith(scene));
    expect(r.movementModel).toBe("grid-stepped");
    expect(r.snapToGrid).toBe(false);
  });
```

Also add, near the other `buildSceneDoc` tests (immediately after
`test("buildSceneDoc honors a partial system override and an explicit id", ...)`):

```ts
test("buildSceneDoc persists an explicit snapToGrid:false (not omitted as falsy)", () => {
  const doc = buildSceneDoc("w1", { snapToGrid: false });
  expect((doc.system as SceneSystem).snapToGrid).toBe(false);
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- scene-docs`
Expected: FAIL — `r.snapToGrid` and `(doc.system as SceneSystem).snapToGrid` are `undefined`,
not the asserted booleans (the field doesn't exist yet).

- [ ] **Step 3: Implement the data model + resolver**

In `SceneSystem` (add `snapToGrid?: boolean;` after `bounds?: SceneDimensions;`):

```ts
export interface SceneSystem {
  grid: { kind: "square" | "hex"; size: number; distance?: GridDistance };
  background: string | null;
  bounds?: SceneDimensions;
  /** Scene-level snap-to-grid toggle (M10f-3 §4.1), independent of `movementModel`. Absent ⇒
   * derived default resolved in `resolveSceneSettings` (false for a continuous scene, true
   * otherwise) — reading this field alone is NOT the effective value. */
  snapToGrid?: boolean;
  vision?: SceneVisionOverrides;
  lighting?: SceneLightingOverrides;
}
```

In `ResolvedSceneSettings` (add `snapToGrid: boolean;` after `movementModel: MovementModel;`):

```ts
export interface ResolvedSceneSettings {
  losRestriction: boolean;
  fog: boolean;
  observerVision: boolean;
  movementRestriction: MovementRestriction;
  movementModel: MovementModel;
  /** Effective snap-to-grid axis (M10f-3 §4.1): an explicit scene value overrides in either
   * direction (including `false`); absent falls back to a derived default keyed off the
   * RESOLVED `movementModel` (false for continuous, true otherwise). */
  snapToGrid: boolean;
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

Replace `buildSceneDoc`:

```ts
export function buildSceneDoc(worldId: string, system: Partial<SceneSystem> = {}, id?: string): WireDocument {
  const full: SceneSystem = {
    grid: system.grid ?? { kind: "square", size: 100 },
    background: system.background ?? null,
    ...(system.bounds ? { bounds: system.bounds } : {}),
    ...(system.vision ? { vision: system.vision } : {}),
    ...(system.lighting ? { lighting: system.lighting } : {}),
    // Explicit undefined-check (not a truthy check) — false is a meaningful, persistable
    // value (M10f-3 §4.1); `system.snapToGrid ? ... : {}` would silently drop an explicit false.
    ...(system.snapToGrid !== undefined ? { snapToGrid: system.snapToGrid } : {}),
  };
  return envelope(worldId, "scene", null, full, id);
}
```

Replace `resolveSceneSettings`:

```ts
export function resolveSceneSettings(scene: WireDocument | undefined, store: ReadableDocuments): ResolvedSceneSettings {
  const ws = store.query("world-settings")[0]?.system as WorldSettingsSystem | undefined;
  // Structural guard: a partial doc (missing scene/pathfinding/animation) falls back to
  // built-in defaults rather than throwing at d.scene.* access below.
  const d = (ws?.scene && ws?.pathfinding && ws?.animation) ? ws : DEFAULT_WORLD_SETTINGS;
  const sys = scene?.system as SceneSystem | undefined;
  const v = sys?.vision ?? {};
  const l = sys?.lighting ?? {};
  const movementModel = v.movementModel ?? d.scene.movementModel;
  return {
    losRestriction: v.losRestriction ?? d.scene.losRestriction,
    fog: v.fog ?? d.scene.fog,
    observerVision: v.observerVision ?? d.scene.observerVision,
    movementRestriction: v.movementRestriction ?? d.scene.movementRestriction,
    movementModel,
    // Derived default keyed off the RESOLVED movementModel (M10f-3 §4.1) — `??` only falls
    // through on null/undefined, never on `false`, so an explicit stored boolean (including
    // false) always overrides the derived default in either direction.
    snapToGrid: sys?.snapToGrid ?? (movementModel === "continuous" ? false : true),
    lightingEnabled: l.enabled ?? d.scene.lightingEnabled,
    lightMode: l.mode ?? d.scene.lightMode,
    environment: l.environment ?? d.scene.environment,
    partialCellLeniency: d.scene.partialCellLeniency,
    diagonalRule: d.pathfinding.diagonalRule,
    animation: d.animation,
    gridDistance: sys?.grid?.distance ?? { perCell: 5, unit: "ft" },
    bounds: resolveBounds(sys?.bounds),
  };
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/core test -- scene-docs`
Expected: PASS.

- [ ] **Step 5: Run the full client gate (a required field was added to a shared interface)**

Run: `pnpm -r typecheck && pnpm lint && pnpm -r test`
Expected: all green (confirms no other `ResolvedSceneSettings` consumer breaks).

- [ ] **Step 6: Commit**

```bash
git add src/client/core/src/scene-docs.ts src/client/core/src/scene-docs.test.ts
git commit -m "feat(m10f-3): add snapToGrid scene axis with a movementModel-derived default"
```

---

## Task 2: `RenderEngine.snap` chokepoint (`@shadowcat/render`)

**Files:**
- Modify: `src/client/render/src/types.ts`
- Modify: `src/client/render/src/engine.ts`
- Test: `src/client/render/src/engine.test.ts`

**Interfaces:**
- Produces: `SceneToolHost.setSnapEnabled(enabled: boolean): void`; `RenderEngine.setSnapEnabled`
  + gated `snap()`.
- Consumed by: Task 3 (`SceneInteractionBridge` forward), Task 4 (`Stage.svelte` push).

- [ ] **Step 1: Write the failing test**

In `src/client/render/src/engine.test.ts`, add immediately after the existing
`test("snap delegates to the active grid; setGrid changes it", ...)` test:

```ts
test("setSnapEnabled(false) makes snap identity; true restores the active grid's snap", () => {
  const { engine } = makeEngine(); // square / 100
  expect(engine.snap({ x: 140, y: 160 })).toEqual({ x: 150, y: 150 }); // default: enabled
  engine.setSnapEnabled(false);
  expect(engine.snap({ x: 140, y: 160 })).toEqual({ x: 140, y: 160 }); // identity
  engine.setSnapEnabled(true);
  expect(engine.snap({ x: 140, y: 160 })).toEqual({ x: 150, y: 150 }); // restored
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/render test -- engine`
Expected: FAIL to typecheck/run — `engine.setSnapEnabled is not a function`.

- [ ] **Step 3: Implement the interface + engine gating**

In `src/client/render/src/types.ts`, in the `SceneToolHost` interface, add immediately after
`snap(p: Point): Point;`:

```ts
  /** Toggle the scene-level snap-to-grid axis (M10f-3 §4.2): disabled makes `snap` identity
   * (free-form float placement/movement for a snap-off scene); grid RENDERING is unaffected —
   * a snap-off scene may still display its reference grid. Every tool that calls `snap` via
   * the AppContext `scene` bridge inherits this automatically. */
  setSnapEnabled(enabled: boolean): void;
```

In `src/client/render/src/engine.ts`, add a new private field immediately after the existing
`private grid: Grid;` field declaration:

```ts
  /** Scene-level snap-to-grid toggle (M10f-3 §4.2); default enabled. Disabled makes `snap`
   * identity — grid RENDERING is unaffected, only the snap call chain every tool inherits
   * via `ctx.scene.snap`. */
  private snapEnabled = true;
```

Replace the existing `snap` method:

```ts
  snap(p: Point): Point {
    return this.grid.snap(p);
  }
```

with:

```ts
  snap(p: Point): Point {
    return this.snapEnabled ? this.grid.snap(p) : p;
  }

  setSnapEnabled(enabled: boolean): void {
    this.snapEnabled = enabled;
  }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/render test -- engine`
Expected: PASS.

- [ ] **Step 5: Run the full client gate (a required method was added to a shared interface)**

Run: `pnpm -r typecheck && pnpm lint && pnpm -r test`
Expected: all green (confirms every other `SceneToolHost` implementer/fixture still compiles —
this will fail at Task 3, where `SceneInteractionBridge` and `fakeSceneHost` gain the method).

- [ ] **Step 6: Commit**

```bash
git add src/client/render/src/types.ts src/client/render/src/engine.ts src/client/render/src/engine.test.ts
git commit -m "feat(m10f-3): gate RenderEngine.snap on a scene-level snapEnabled toggle"
```

---

## Task 3: Bridge forward + shared test fixture (`@shadowcat/ui-kit`)

**Files:**
- Modify: `src/client/ui-kit/src/sceneInteraction.ts`
- Modify: `src/client/ui-kit/src/__fixtures__/fakeSceneHost.ts`
- Test: `src/client/ui-kit/src/sceneInteraction.test.ts`

**Interfaces:**
- Consumes: `SceneToolHost.setSnapEnabled` (Task 2).
- Produces: `SceneInteractionBridge.setSnapEnabled(enabled: boolean): void` (no-op when
  detached, mirroring every other bridge method).
- Consumed by: Task 4 (`Stage.svelte` may call either `e.setSnapEnabled` directly or, in future,
  through `ctx.scene`; this task completes the interface contract either way).

- [ ] **Step 1: Write the failing test**

In `src/client/ui-kit/src/sceneInteraction.test.ts`, add immediately after the existing
`test("animateSamples forwards moverVision to the host (M2 §T6 seam)", ...)` test:

```ts
test("setSnapEnabled forwards to the host (no-op when detached)", () => {
  const bridge = new SceneInteractionBridge();
  expect(() => bridge.setSnapEnabled(false)).not.toThrow(); // detached: no-op
  const calls: boolean[] = [];
  bridge.attach(fakeSceneHost({ setSnapEnabled: (enabled) => calls.push(enabled) }));
  bridge.setSnapEnabled(false);
  bridge.setSnapEnabled(true);
  expect(calls).toEqual([false, true]);
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/ui-kit test -- sceneInteraction`
Expected: FAIL — `bridge.setSnapEnabled is not a function` (and the file fails to typecheck,
since `SceneInteractionBridge implements SceneInteraction extends SceneToolHost` is now missing
a required member).

- [ ] **Step 3: Implement the bridge forward + fixture default**

In `src/client/ui-kit/src/sceneInteraction.ts`, add to `SceneInteractionBridge`, immediately
after the existing `snap` method:

```ts
  setSnapEnabled(enabled: boolean): void {
    this.#host?.setSnapEnabled(enabled);
  }
```

In `src/client/ui-kit/src/__fixtures__/fakeSceneHost.ts`, add a default, immediately after
`snap: (p: Point) => p,`:

```ts
    setSnapEnabled: () => {},
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/ui-kit test -- sceneInteraction`
Expected: PASS.

- [ ] **Step 5: Run the full client gate (completes the shared interface contract)**

Run: `pnpm -r typecheck && pnpm lint && pnpm -r test`
Expected: all green (confirms every `SceneToolHost` implementer — `RenderEngine`,
`SceneInteractionBridge`, `fakeSceneHost`, and any test-local fake hosts — now satisfies the
interface; a test-local fake host built via an object literal typed as `SceneToolHost` rather
than `fakeSceneHost(...)` would surface here).

- [ ] **Step 6: Commit**

```bash
git add src/client/ui-kit/src/sceneInteraction.ts src/client/ui-kit/src/__fixtures__/fakeSceneHost.ts src/client/ui-kit/src/sceneInteraction.test.ts
git commit -m "feat(m10f-3): forward setSnapEnabled through SceneInteractionBridge"
```

---

## Task 4: `Stage.svelte` wiring (`@shadowcat/module-stage`)

**Files:**
- Modify: `src/modules/stage/src/Stage.svelte`
- Test: `src/modules/stage/src/Stage.test.ts`

**Interfaces:**
- Consumes: `resolveSceneSettings(...).snapToGrid` (Task 1), `RenderEngine.setSnapEnabled`
  (Task 2).

- [ ] **Step 1: Write the failing tests**

In `src/modules/stage/src/Stage.test.ts`, add the `RenderEngine` import to the existing import
list:

```ts
import { test, expect, vi } from "vitest";
import { render } from "@testing-library/svelte";
import Stage from "./Stage.svelte";
import type { DisplayBackend } from "@shadowcat/render";
import { RenderEngine } from "@shadowcat/render";
import type { ReadableDocuments } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
```

Add a fixture function immediately after the existing `tokenDocs()` function:

```ts
/** A documents view exposing a single scene doc with the given `system` body (M10f-3 snap
 * wiring: `resolveSceneSettings` reads it via `documents.query("scene")[0]`). */
function sceneDocs(system: Record<string, unknown>): ReadableDocuments {
  return {
    query: (t: string) => (t === "scene" ? [{ id: "s1", doc_type: "scene", system }] : []),
    get: () => undefined,
    subscribe: () => () => {},
    snapshot: () => [],
    appliedSeq: 0,
  } as unknown as ReadableDocuments;
}
```

Add two new tests at the end of the file:

```ts
test("pushes the resolved snapToGrid to the engine (grid-stepped scene: default true)", async () => {
  const createBackend = vi.fn(async () => fakeBackend());
  const spy = vi.spyOn(RenderEngine.prototype, "setSnapEnabled");
  render(Stage, {
    props: { createBackend },
    context: setAppContextForTest({
      documents: sceneDocs({ grid: { kind: "square", size: 100 } }),
      subscribeScene: () => ({ unsubscribe() {} }),
    }),
  });
  await vi.waitFor(() => expect(spy).toHaveBeenCalledWith(true));
  spy.mockRestore();
});

test("pushes the resolved snapToGrid to the engine (continuous scene: default false)", async () => {
  const createBackend = vi.fn(async () => fakeBackend());
  const spy = vi.spyOn(RenderEngine.prototype, "setSnapEnabled");
  render(Stage, {
    props: { createBackend },
    context: setAppContextForTest({
      documents: sceneDocs({ grid: { kind: "square", size: 100 }, vision: { movementModel: "continuous" } }),
      subscribeScene: () => ({ unsubscribe() {} }),
    }),
  });
  await vi.waitFor(() => expect(spy).toHaveBeenCalledWith(false));
  spy.mockRestore();
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/module-stage test -- Stage`
Expected: FAIL — `spy` is never called with `true`/`false` (`setSnapEnabled` exists as a no-op
inherited default from `SceneToolHost`'s implementation on `RenderEngine`, but nothing in
`Stage.svelte` calls it yet).

- [ ] **Step 3: Implement the Stage wiring**

In `src/modules/stage/src/Stage.svelte`, inside the `onDocs` function, replace:

```ts
        const key = `${spec.kind}:${spec.size}:${diagonalRule}`;
        if (key !== lastGridKey) {
          lastGridKey = key;
          e.setGrid(spec);
        }
```

with:

```ts
        const key = `${spec.kind}:${spec.size}:${diagonalRule}`;
        if (key !== lastGridKey) {
          lastGridKey = key;
          e.setGrid(spec);
        }
        // Snap-to-grid is per-scene (M10f-3 §4.2-4.3). Pushed unconditionally each pass — a
        // cheap flag assignment (unlike setGrid's Grid rebuild or setAnimation's config
        // object), so no change-detection gate is needed here.
        e.setSnapEnabled(settings.snapToGrid);
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/module-stage test -- Stage`
Expected: PASS.

- [ ] **Step 5: Run the package gate**

Run: `pnpm --filter @shadowcat/module-stage typecheck && pnpm --filter @shadowcat/module-stage test`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add src/modules/stage/src/Stage.svelte src/modules/stage/src/Stage.test.ts
git commit -m "feat(m10f-3): push the resolved snapToGrid to the render engine from Stage"
```

---

## Task 5: GM tool-rail snap toggle (`@shadowcat/module-scene-tools` + `@shadowcat/ui-kit` locale)

**Files:**
- Modify: `src/modules/scene-tools/src/ToolRail.svelte`
- Modify: `src/client/ui-kit/src/locales/en.ts`
- Test: `src/modules/scene-tools/src/ToolRail.test.ts`

**Interfaces:**
- Consumes: `resolveSceneSettings(...).snapToGrid` (Task 1); `ctx.dispatchIntent` (existing
  `AppContext` seam).
- Produces: a `data-testid="snap-toggle"` button; a scene-doc `/system/snapToGrid` update on
  click.

- [ ] **Step 1: Write the failing tests**

In `src/client/ui-kit/src/locales/en.ts`, add immediately after `"tools.color": "Color",`:

```ts
  "tools.snap": "Snap to grid",
```

In `src/modules/scene-tools/src/ToolRail.test.ts`, add to the import list:

```ts
import { render, screen, fireEvent } from "@testing-library/svelte";
import { test, expect } from "vitest";
import type { SceneTool } from "@shadowcat/render";
import { SceneInteractionBridge } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildSceneDoc, type WireOperation } from "@shadowcat/core";
import ToolRail from "./ToolRail.svelte";
```

Add a fixture + three new tests at the end of the file:

```ts
/** A DocumentStore seeded with one scene doc carrying `system`. */
function sceneStore(system: Record<string, unknown> = {}): DocumentStore {
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: buildSceneDoc("w1", system, "s1") }],
  });
  return docs;
}

test("the snap toggle reflects the resolved snapToGrid (grid-stepped default: pressed) and dispatches an update on click", async () => {
  const dispatched: WireOperation[][] = [];
  render(ToolRail, {
    context: setAppContextForTest({
      role: "gm",
      documents: sceneStore(),
      dispatchIntent: (ops) => dispatched.push(ops),
    }),
  });
  const toggle = screen.getByTestId("snap-toggle");
  expect(toggle.getAttribute("aria-pressed")).toBe("true"); // grid-stepped default
  await fireEvent.click(toggle);
  expect(dispatched.at(-1)).toEqual([
    { op: "update", doc_id: "s1", changes: [{ path: "/system/snapToGrid", old: null, new: false }] },
  ]);
});

test("the snap toggle reflects a continuous scene's false default", () => {
  render(ToolRail, {
    context: setAppContextForTest({
      role: "gm",
      documents: sceneStore({ vision: { movementModel: "continuous" } }),
    }),
  });
  expect(screen.getByTestId("snap-toggle").getAttribute("aria-pressed")).toBe("false");
});

test("no active scene: the snap toggle does not render", () => {
  render(ToolRail, { context: setAppContextForTest({ role: "gm", documents: new DocumentStore() }) });
  expect(screen.queryByTestId("snap-toggle")).toBeNull();
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/module-scene-tools test -- ToolRail`
Expected: FAIL — `screen.getByTestId("snap-toggle")` throws (no such element yet).

- [ ] **Step 3: Implement the toggle button**

In `src/modules/scene-tools/src/ToolRail.svelte`, replace the script block's imports and setup:

```svelte
<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { ToolController, type ToolId, type DrawMode, type TemplateMode, type RegionShapeMode, type RegionBehaviorMode } from "./controller.svelte";
  import AssetPicker from "./AssetPicker.svelte";

  const ctx = getAppContext();
```

with:

```svelte
<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { resolveSceneSettings, type WireDocument } from "@shadowcat/core";
  import { ToolController, type ToolId, type DrawMode, type TemplateMode, type RegionShapeMode, type RegionBehaviorMode } from "./controller.svelte";
  import AssetPicker from "./AssetPicker.svelte";

  const ctx = getAppContext();
```

Then, immediately after the existing `const isGm = ctx.role === "gm";` line, add:

```ts
  // Reactive subscription mirrors GameSettingsPanel's registry-seed pattern: calling
  // subscribe() inside each $derived.by registers a reactive dependency on the document
  // store so the snap toggle re-resolves as the active scene's doc changes (M10f-3 §4.4).
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const activeScene = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("scene")[0];
  });
  const snapToGrid = $derived.by((): boolean => {
    subscribe();
    return resolveSceneSettings(activeScene, ctx.documents).snapToGrid;
  });

  /** GM-authored scene-level snap toggle (M10f-3 §4.4): writes the opaque
   * `/system/snapToGrid` field on the active scene document (shared, not local UI state).
   * No-op with no active scene. */
  function toggleSnap(): void {
    const scene = activeScene;
    if (!scene) return;
    ctx.dispatchIntent([
      { op: "update", doc_id: scene.id, changes: [{ path: "/system/snapToGrid", old: null, new: !snapToGrid }] },
    ]);
  }
```

Then, in the markup, replace:

```svelte
    {#each tools as tool (tool.id)}
      <button
        type="button"
        class="tool"
        class:active={controller.active === tool.id}
        aria-pressed={controller.active === tool.id}
        data-testid="tool-{tool.id}"
        title={tool.label}
        onclick={() => controller.toggle(tool.id)}
      >
        {tool.label}
      </button>
    {/each}

    {#if controller.active === "place"}
```

with:

```svelte
    {#each tools as tool (tool.id)}
      <button
        type="button"
        class="tool"
        class:active={controller.active === tool.id}
        aria-pressed={controller.active === tool.id}
        data-testid="tool-{tool.id}"
        title={tool.label}
        onclick={() => controller.toggle(tool.id)}
      >
        {tool.label}
      </button>
    {/each}

    {#if activeScene}
      <button
        type="button"
        class="tool"
        aria-pressed={snapToGrid}
        data-testid="snap-toggle"
        title={t("tools.snap")}
        onclick={toggleSnap}
      >
        {t("tools.snap")}
      </button>
    {/if}

    {#if controller.active === "place"}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/module-scene-tools test -- ToolRail`
Expected: PASS.

- [ ] **Step 5: Run the package gate**

Run: `pnpm --filter @shadowcat/module-scene-tools typecheck && pnpm --filter @shadowcat/module-scene-tools test && pnpm --filter @shadowcat/ui-kit test`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add src/modules/scene-tools/src/ToolRail.svelte src/modules/scene-tools/src/ToolRail.test.ts src/client/ui-kit/src/locales/en.ts
git commit -m "feat(m10f-3): add a GM tool-rail toggle authoring the scene snapToGrid axis"
```

---

## Task 6: Remove the `commitRoute` continuous-scene refusal (`@shadowcat/module-scene-tools`)

**Files:**
- Modify: `src/modules/scene-tools/src/controller.svelte.ts`
- Modify: `src/modules/scene-tools/src/measure-tool.test.ts`

**Interfaces:**
- Consumes: nothing new — this task only removes code and an unused import.

- [ ] **Step 1: Rewrite the test to the new (post-fix) expectation**

In `src/modules/scene-tools/src/measure-tool.test.ts`, replace the existing test:

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

with:

```ts
test("commitRoute fires via moveRequest in a continuous-movement-model scene (M10f-3: execution now wired end-to-end)", async () => {
  // [[tests-yield-to-correct-code]]: this test used to assert commitRoute's continuous-scene
  // refusal (the M10f-1 preview-only guard). M10f-2 shipped the engine-agnostic unified
  // executor and M10f-3 removes the guard, so committing a route now proceeds identically to
  // a grid-stepped scene — this test now asserts the move FIRES, not that it's suppressed.
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
  expect(moves).toEqual([{ tokenId: "tok1", path: [[0, 0], [100, 0], [100, 100]] }]);
});
```

- [ ] **Step 2: Run tests to verify the rewritten test fails against the current (unfixed) code**

Run: `pnpm --filter @shadowcat/module-scene-tools test -- measure-tool`
Expected: FAIL — `moves` is `[]` (the refusal guard is still in place, so `moveRequest` never
fires); this is the red state for this task's fix.

- [ ] **Step 3: Remove the refusal guard + its now-unused import**

In `src/modules/scene-tools/src/controller.svelte.ts`, replace the import line:

```ts
import { buildTokenDoc, buildTokenFromActor, buildSceneEntityDoc, resolveTokenBox, resolveTokenActor, footprintRadius, buildRegionDoc, setRegionVisibility, resolveSceneSettings, type ReadableDocuments, type AssetResolver, type WireOperation, type PathResult, type MoveStream } from "@shadowcat/core";
```

with:

```ts
import { buildTokenDoc, buildTokenFromActor, buildSceneEntityDoc, resolveTokenBox, resolveTokenActor, footprintRadius, buildRegionDoc, setRegionVisibility, type ReadableDocuments, type AssetResolver, type WireOperation, type PathResult, type MoveStream } from "@shadowcat/core";
```

Replace the start of `commitRoute`:

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
```

with:

```ts
  function commitRoute(goal: Point): void {
    if (!ctx.pathfind || !ctx.moveRequest || !ctx.tokenSelection || ctx.tokenSelection.ids.size !== 1) return;
    // Continuous-scene execution is wired end-to-end (M10f-3): the server move-execution path
    // is engine-agnostic since M10f-2 (no movementModel branch anywhere), so committing a route
    // proceeds identically for grid-stepped and continuous scenes.
    const scene = activeScene(ctx);
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/module-scene-tools test -- measure-tool`
Expected: PASS (all `measure-tool.test.ts` tests, including the rewritten one).

- [ ] **Step 5: Run the package gate**

Run: `pnpm --filter @shadowcat/module-scene-tools typecheck && pnpm --filter @shadowcat/module-scene-tools test`
Expected: all green (confirms the removed import doesn't leave an unused-import lint error and
no other test in this file relied on the removed guard).

- [ ] **Step 6: Commit**

```bash
git add src/modules/scene-tools/src/controller.svelte.ts src/modules/scene-tools/src/measure-tool.test.ts
git commit -m "feat(m10f-3): remove the commitRoute continuous-scene refusal (execution now wired)"
```

---

## Task 7: `Room::execute_move` continuous-scene tests (server, test-only)

**Files:**
- Modify: `src/server/src/ws/room.rs` (new `#[cfg(test)]` helper + two new tests, inside the
  existing `mod tests` block)

**Interfaces:**
- Consumes: `Room::execute_move` (unchanged, already engine-agnostic since M10f-2), the existing
  `movement_scene` helper's shape (mirrored, not modified — this file's established convention is
  one dedicated helper per test scenario; `movement_scene_two_lit_pockets`,
  `movement_scene_partial_cell`, and `movement_scene_with_wall` are all near-duplicates of
  `movement_scene`, not compositions of it).
- Produces: nothing consumed by later tasks — this is a leaf verification task.

- [ ] **Step 1: Add the helper + two tests**

Add, immediately before the closing `}` of `mod tests` (i.e. directly after the existing
`execute_move_revealed_union_allows_explored_cell` test):

```rust

    /// Identical to `movement_scene`, but the scene doc's `system.vision.movementModel` is
    /// explicitly `"continuous"` (M10f-3 §6): proves `execute_move` gates an any-angle route
    /// from a scene genuinely marked continuous, not just incidentally sent a diagonal path.
    /// Functionally inert on the server today — `execute_move` has no `movementModel` branch
    /// (engine-agnostic since M10f-2); this mirrors `movement_scene`'s body (this file's
    /// established per-scenario-helper convention) with one added JSON key.
    async fn movement_scene_continuous(restriction: &str, with_light: bool) -> MovementHandle {
        use serde_json::json;

        let (repo, world_id, gm) = repo_with_world().await;
        let p = repo
            .create_user("player_continuous", None, crate::auth::role::ServerRole::User, 0)
            .await
            .unwrap();
        repo.add_member(world_id, p, WorldRole::Player)
            .await
            .unwrap();
        let player = PermissionContext {
            user_id: p,
            world_role: WorldRole::Player,
        };

        let reg = RoomRegistry::new();
        let room = reg.get_or_create(&repo, world_id).await.unwrap().unwrap();
        let wdoc = crate::data::document::tests::world_scoped_doc;
        let (scene_id, token_id, ws_id, light_id) = (
            Uuid::from_u128(0xC047_0000),
            Uuid::from_u128(0xC047_0001),
            Uuid::from_u128(0xC047_0002),
            Uuid::from_u128(0xC047_0003),
        );

        let mut ws = wdoc(world_id, ws_id, "world-settings");
        ws.owner = Some(gm.user_id);
        ws.system = json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#000000", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": restriction,
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        room.publish(&repo, &gm, vec![Operation::Create { doc: ws }], 0)
            .await
            .unwrap();

        // Only structural difference from `movement_scene`: declares `vision.movementModel` on
        // the scene doc. Inert server-side today — execute_move has no movementModel branch.
        let mut scene = wdoc(world_id, scene_id, "scene");
        scene.owner = Some(gm.user_id);
        scene.system = json!({
            "grid": { "kind": "square", "size": 100 },
            "vision": { "movementModel": "continuous" }
        });
        room.publish(&repo, &gm, vec![Operation::Create { doc: scene }], 0)
            .await
            .unwrap();

        let mut token = wdoc(world_id, token_id, "token");
        token.parent_id = Some(scene_id);
        token.owner = Some(p);
        token.system = json!({ "x": 50.0, "y": 50.0 });
        room.publish(&repo, &gm, vec![Operation::Create { doc: token }], 0)
            .await
            .unwrap();

        if with_light {
            // Bright boundary = 1.5 * 100 = 150 world units; dim boundary = 3.0 * 100 = 300.
            let mut light = wdoc(world_id, light_id, "light");
            light.parent_id = Some(scene_id);
            light.owner = Some(gm.user_id);
            light.system = json!({
                "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
                "brightRadius": 1.5, "dimRadius": 3.0, "enabled": true
            });
            room.publish(&repo, &gm, vec![Operation::Create { doc: light }], 0)
                .await
                .unwrap();
        }

        MovementHandle {
            room,
            repo,
            gm,
            player,
            world_id,
            scene_id,
            token_id,
            start: (50.0, 50.0),
            lit_goal: (50.0, 150.0),
            adj: (150.0, 50.0),
            adj2: (250.0, 50.0),
        }
    }

    #[tokio::test]
    async fn execute_move_continuous_any_angle_route_commits_atomically() {
        // Proves the M10f-2 unified sampled executor gates a genuinely any-angle
        // (non-grid-aligned) polyline exactly like a grid path — no movementModel branch
        // anywhere on this path (M10f-3 §3.2). Goal (110,130) is a 3-4-5 triangle scaled ×20
        // from start (50,50): distance = sqrt(60²+80²) = 100 wu, safely inside the light's
        // 150 wu bright radius (50 wu margin) and not a grid cell-center (cell centers sit
        // at 50 + 100k on each axis).
        let h = movement_scene_continuous("visible", /*with_light=*/ true).await;
        let goal = (110.0, 130.0);
        let res = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                h.scene_id,
                h.token_id,
                vec![h.start, goal],
                now_millis(),
            )
            .await
            .unwrap();
        assert_eq!(res.stop, goal, "any-angle move commits at the exact goal");
        assert_eq!(h.committed_pos(h.token_id).await, res.stop);
    }

    #[tokio::test]
    async fn execute_move_continuous_rejects_move_into_unseen_space() {
        // A continuous route whose subdivided cells leave the visible set is cell-gate
        // rejected (Forbidden, fail-closed) — proves the cell-sampled gate applies to
        // any-angle paths, not just grid ones. Goal (650,850) is a 3-4-5 triangle scaled
        // ×200 from start (50,50): distance = sqrt(600²+800²) = 1000 wu, far beyond the
        // light's 300 wu dim radius (dark).
        let h = movement_scene_continuous("visible", /*with_light=*/ true).await;
        let goal = (650.0, 850.0);
        let blocked = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                h.scene_id,
                h.token_id,
                vec![h.start, goal],
                now_millis(),
            )
            .await;
        assert!(
            matches!(blocked, Err(DataError::Forbidden)),
            "any-angle move into unseen space must be cell-gate rejected"
        );
    }
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p shadowcat --lib ws::room::tests::execute_move_continuous -- --nocapture`
Expected: both tests PASS. (These prove production behavior that already exists — there is no
"red" phase distinct from "test doesn't exist yet"; if either test fails, per Global Constraints
this is a signal to re-derive the test's geometry, NOT to add a `movementModel` branch to
`execute_move`.)

- [ ] **Step 3: Format and lint**

Run: `cargo fmt && cargo clippy -p shadowcat --lib -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/server/src/ws/room.rs
git commit -m "test(m10f-3): prove Room::execute_move gates a continuous-scene any-angle route"
```

---

## Task 8: `sample_path` any-angle unit test (server, test-only)

**Files:**
- Modify: `src/server/src/scene/move_stream.rs` (new test in the existing `mod tests` block)

**Interfaces:**
- Consumes: `sample_path` (unchanged, already polyline-shape-agnostic).

- [ ] **Step 1: Add the test**

Add, immediately after the existing `straight_two_cell_path_samples_endpoints_and_interior` test
(before `cap_bounds_samples`):

```rust

    /// Any-angle diagonal path (non-grid-aligned vertices, no 45°/90° structure): endpoints
    /// exact, `t_ms` strictly increasing (arc-length monotonic) — proves `sample_path` is
    /// engine-agnostic geometry, not shaped around grid king-steps (M10f-3 §6).
    #[test]
    fn diagonal_any_angle_path_samples_endpoints_with_monotonic_time() {
        let path = vec![(0.0_f64, 0.0_f64), (137.5, 84.2), (310.0, 10.0)];
        let cell = 100.0_f64;
        let duration_ms = 1500.0_f64;
        let samples = sample_path(&path, cell, duration_ms);

        let first = &samples[0];
        let last = samples.last().unwrap();

        assert!((first.t_ms - 0.0).abs() < 1e-9, "first t_ms {}", first.t_ms);
        assert_eq!(first.pos, (0.0, 0.0), "first pos exact");
        assert!(
            (last.t_ms - duration_ms).abs() < 1e-6,
            "last t_ms {}",
            last.t_ms
        );
        assert_eq!(last.pos, (310.0, 10.0), "last pos exact");

        for w in samples.windows(2) {
            assert!(
                w[1].t_ms > w[0].t_ms,
                "t_ms not strictly increasing: {} then {}",
                w[0].t_ms,
                w[1].t_ms
            );
        }
    }
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p shadowcat --lib scene::move_stream::tests::diagonal -- --nocapture`
Expected: PASS.

- [ ] **Step 3: Format and lint**

Run: `cargo fmt && cargo clippy -p shadowcat --lib -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/server/src/scene/move_stream.rs
git commit -m "test(m10f-3): prove sample_path handles a genuinely any-angle path"
```

---

## Task 9: `clip_move_stream` any-angle no-leak test (server, test-only)

**Files:**
- Modify: `src/server/src/ws/conn.rs` (new test in the existing `mod tests` block)

**Interfaces:**
- Consumes: `clip_move_stream` + `setup_clip_room` (both unchanged).

- [ ] **Step 1: Add the test**

Add, immediately after the existing `clip_observer_sees_near_side_prefix` test (before
`clip_gm_only_wall_suppresses_observer`):

```rust

    /// Same near-side/occluded clip boundary as `clip_observer_sees_near_side_prefix`, but over
    /// a genuinely any-angle (non-axis-aligned) path — proves the M2 per-recipient egress clip
    /// is engine-agnostic geometry, unaffected by whether the sampled polyline is grid-stepped or
    /// continuous (M10f-3 §6). Wall at x=100 (unchanged); observer at (50,50) sees anything with
    /// x<100 regardless of y, so the diagonal y-offsets below don't change the visibility split.
    #[tokio::test]
    async fn clip_observer_sees_near_side_prefix_any_angle_diagonal_path() {
        use crate::ws::protocol::PosSample;

        let wall_sys = json!({
            "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 },
            "blocksSight": true
        });
        let (room, _, obs_ctx, scene_id) =
            setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), false).await;

        let mover_id = Uuid::from_u128(0xAABB);
        let frame = ServerMsg::MoveStream {
            request_id: Uuid::from_u128(1),
            token_id: Uuid::from_u128(2),
            mover: mover_id,
            scene: scene_id,
            start_server_ms: 1000.0,
            duration_ms: 1500.0,
            stop: [310.0, 10.0],
            samples: vec![
                PosSample {
                    t_ms: 0.0,
                    pos: [50.0, 60.0], // near side, diagonal offset — visible
                },
                PosSample {
                    t_ms: 750.0,
                    pos: [140.0, 95.0], // behind wall, diagonal — occluded
                },
                PosSample {
                    t_ms: 1500.0,
                    pos: [310.0, 10.0], // further behind wall, diagonal — occluded
                },
            ],
            mover_vision: None,
            cost: Some(3.0),
        };

        let result = clip_move_stream(&frame, &obs_ctx, &room).await;

        assert!(
            result.is_some(),
            "partial-visibility observer must receive a clipped frame"
        );
        match result.unwrap() {
            ServerMsg::MoveStream {
                samples: s,
                mover_vision: mv,
                stop: out_stop,
                cost,
                ..
            } => {
                assert_eq!(
                    s.len(),
                    1,
                    "only the near-side diagonal sample is visible; got {} samples: {s:?}",
                    s.len()
                );
                assert_eq!(s[0].pos, [50.0_f64, 60.0_f64]);
                assert_eq!(mv, None, "mover_vision must be None for observers");
                assert_eq!(
                    out_stop,
                    [50.0_f64, 60.0_f64],
                    "stop clips to the last visible sample, not the true diagonal goal"
                );
                assert_eq!(cost, None, "cost must be nulled for a clipped observer");
            }
            other => panic!("expected MoveStream, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p shadowcat --lib ws::conn::tests::clip_observer_sees_near_side_prefix_any_angle -- --nocapture`
Expected: PASS.

- [ ] **Step 3: Format and lint**

Run: `cargo fmt && cargo clippy -p shadowcat --lib -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/server/src/ws/conn.rs
git commit -m "test(m10f-3): prove clip_move_stream stays leak-free over an any-angle path"
```

---

## Completion checklist (before merge)

- [ ] All 9 tasks committed, per-task reviewed (Tasks 6, 7, 8, 9 buddy-checked; Tasks 1–5
      standard single-reviewer).
- [ ] Full monorepo gate green: `pnpm -r typecheck && pnpm lint && pnpm -r test`.
- [ ] Full server gate green: `cargo fmt --all -- --check && cargo clippy -p shadowcat --all-targets -- -D warnings && cargo test -p shadowcat --all`.
- [ ] Mandatory whole-branch buddy-check (two independent blind Opus reviewers, debate to
      convergence) converged PASS across all 9 tasks' assembled diff, covering the three items
      listed in "Buddy-check directives" above.
- [ ] `shadowcat-codebase-scene-rendering` updated (continuous execution wired end-to-end, the
      `snapToGrid` scene axis + derived default, the `RenderEngine.snap`/`SceneToolHost`
      chokepoint) and confirmed accurate by `shadowcat-spec-reviewer`.
- [ ] `docs/PLAN.md`'s M10f entry updated: mark M10f-3 DONE, matching the M10f-0/M10f-1/M10f-2
      entry style (branch name, commit range, what shipped, buddy-check summary, note the §9
      parent-spec supersession per the design doc §5, "Next = M10f-4").
- [ ] Merge `m10f-3-continuous-execution-snap-toggle` --no-ff to LOCAL `main` (merge gate = full
      M10f, per the standing M10f convention) — do not push unless the user directs otherwise.
