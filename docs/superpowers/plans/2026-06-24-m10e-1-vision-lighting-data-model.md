# M10e-1 — Vision/Lighting Data Model (V1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the client-side data model + GM-editable config for scene vision, lighting, movement restriction, animation, and actor vision modes — the foundation the M10e-2..6 checkpoints consume. No rendering, no server vision, no pathfinding yet.

**Architecture:** Three new world-scoped config-documents (`world-settings`, `light-gradation`, `vision-modes`) plus new per-scene override fields on `scene.system`, a `blocksLight` wall flag, a `light` doc_type, and `EffectiveActor.visionModes`. All config-docs are **client-owned opaque `system` JSON** (no Rust/ts-rs changes — verified). Core types/builders/resolvers live in `@shadowcat/core` (`scene-docs.ts`, `actor.ts`); editing UI ships as one new GM module `@shadowcat/module-game-settings`; actor vision authoring extends the existing actors module.

**Tech Stack:** TypeScript, Svelte 5 (runes), Vitest + @testing-library/svelte (jsdom). pnpm workspace (`src/modules/*`, `src/client/*`).

## Global Constraints

- **No server/Rust changes** — config-doc `system` bodies are opaque to the server; it stores arbitrary JSON per `doc_type`. Verified: no `ts-rs` types exist for registry shapes.
- **Config = world-scoped documents** — `parent_id: null`, `scope: { kind: "world", world_id }`, built via the existing `envelope()` helper.
- **Idempotent GM seed** — a module creates a config-doc only if `ctx.documents.query("<doc_type>").length === 0`, GM-only, exactly like `FactionsPanel`/`ConditionsPanel`.
- **Single-field updates via JSON-pointer** — `dispatchIntent([{ op:"update", doc_id, changes:[{ path:"/system/...", old:null, new:v }] }])`.
- **Fail-closed resolution** — resolvers tolerate absent/stale docs by falling back to built-in defaults; never throw.
- **i18n** — all user-visible strings go through `ctx.t("ns.key")`; add keys to `src/client/ui-kit/src/locales/en.ts`. Tests assert on the raw key string (no catalog loaded in jsdom), matching `ActorsPanel.test.ts`.
- **Units** — radii/ranges/sizes stored in **grid cells**; `grid.distance.perCell` is labels-only.
- **Svelte 5 runes** — `$state`, `$derived`, `$derived.by`, `$effect`, `$props`; events as `onclick={...}`.

**Spec:** `docs/superpowers/specs/2026-06-24-m10e-vision-lighting-movement-design.md` (§5 is this checkpoint).

---

## File Structure

**Modified (core — `src/client/core/src/`):**
- `scene-docs.ts` — new types + builders + seed constants + `resolveSceneSettings`/`resolveGradation`/`resolveVisionModes`; extend `SceneSystem`, `ActorSystem`, `TokenOverrides`; `buildSceneDoc` defaults.
- `actor.ts` — extend `EffectiveActor` + `project()` with `visionModes`.
- `index.ts` (core barrel) — export the new symbols (mirror how `buildFactionRegistryDoc` etc. are exported).

**Modified (modules):**
- `src/modules/scene-tools/src/controller.svelte.ts` — wall tool adds `blocksLight`.
- `src/modules/actors/src/ActorsPanel.svelte` — actor vision-mode authoring.
- `src/client/ui-kit/src/locales/en.ts` — catalog keys.
- `src/client/shell/src/App.svelte` — import + register the new module.

**Created (new module — `src/modules/game-settings/`):**
- `package.json`, `tsconfig.json` (mirrored from `src/modules/factions/`).
- `src/index.ts` — `Module` manifest; contributes a sidebar panel.
- `src/GameSettingsPanel.svelte` — GM panel: seeds the 3 config-docs; edits world defaults + gradation + vision-modes + per-scene overrides.
- `src/*.test.ts` — panel tests.

---

## Type contract (shared across tasks — defined in Task 1/2/3, repeated here for reference)

```typescript
// scene-docs.ts
export type MovementRestriction = "visible" | "revealed" | "unrestricted";
export type LightMode = "globalIllumination" | "environmentLight";
export type DiagonalRule = "chebyshev" | "alternating" | "euclidean" | "manhattan";
export type EasingMode = "easeInOut" | "linear";
export interface EnvironmentLight { color: string; intensity: number; }
export interface GridDistance { perCell: number; unit: string; }

export interface SceneVisionOverrides { losRestriction?: boolean; fog?: boolean; observerVision?: boolean; movementRestriction?: MovementRestriction; }
export interface SceneLightingOverrides { enabled?: boolean; mode?: LightMode; environment?: EnvironmentLight; }
// SceneSystem gains: vision?, lighting?, grid.distance?

export interface WorldSceneDefaults { losRestriction: boolean; fog: boolean; lightingEnabled: boolean; lightMode: LightMode; environment: EnvironmentLight; observerVision: boolean; movementRestriction: MovementRestriction; partialCellLeniency: boolean; }
export interface WorldSettingsSystem { scene: WorldSceneDefaults; pathfinding: { diagonalRule: DiagonalRule }; animation: { speedCellsPerSec: number; easing: EasingMode }; }
export interface ResolvedSceneSettings { losRestriction: boolean; fog: boolean; observerVision: boolean; movementRestriction: MovementRestriction; lightingEnabled: boolean; lightMode: LightMode; environment: EnvironmentLight; partialCellLeniency: boolean; diagonalRule: DiagonalRule; animation: { speedCellsPerSec: number; easing: EasingMode }; gridDistance: GridDistance; }

export interface GradationBand { name: string; minIllumination: number; }
export interface LightGradationSystem { bands: GradationBand[]; }
export interface VisionMode { id: string; name: string; illuminationFloor: string; defaultRange: number; renderHint?: string; }
export interface VisionModesSystem { modes: Record<string, VisionMode>; }
export interface VisionAssignment { mode: string; range: number; }

export interface LightSystem { x: number; y: number; color: string; intensity: number; brightRadius: number; dimRadius: number; falloff?: { curve: "linear" | "quadratic" | "none" }; enabled: boolean; }

// actor.ts: EffectiveActor gains visionModes: VisionAssignment[]; ActorSystem/TokenOverrides gain vision?: VisionAssignment[]
```

---

### Task 1: Scene settings model + world-settings doc + inheritance resolver

**Files:**
- Modify: `src/client/core/src/scene-docs.ts` (extend `SceneSystem`, `buildSceneDoc`; add settings types/builder/resolver)
- Modify: `src/client/core/src/index.ts` (export new symbols)
- Test: `src/client/core/src/scene-docs.test.ts` (add cases)

**Interfaces:**
- Consumes: existing `envelope(worldId, docType, parentId, system, id?)`, `SceneSystem`, `WireDocument`, `ReadableDocuments`.
- Produces: `MovementRestriction`, `LightMode`, `DiagonalRule`, `EasingMode`, `EnvironmentLight`, `GridDistance`, `SceneVisionOverrides`, `SceneLightingOverrides`, extended `SceneSystem`, `WorldSceneDefaults`, `WorldSettingsSystem`, `DEFAULT_WORLD_SETTINGS`, `buildWorldSettingsDoc(worldId, system?, id?)`, `ResolvedSceneSettings`, `resolveSceneSettings(scene, store)`.

- [ ] **Step 1: Write the failing test**

Add to `src/client/core/src/scene-docs.test.ts`:

```typescript
import {
  buildWorldSettingsDoc, resolveSceneSettings, buildSceneDoc, DEFAULT_WORLD_SETTINGS,
  type WireDocument,
} from "./scene-docs";
import { DocumentStore } from "./store"; // mirror the import the file already uses for DocumentStore

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

describe("resolveSceneSettings", () => {
  it("falls back to built-in defaults when no world-settings doc and no scene overrides", () => {
    const scene = buildSceneDoc("w1", {}, "scene1");
    const r = resolveSceneSettings(scene, storeWith(scene));
    expect(r.losRestriction).toBe(DEFAULT_WORLD_SETTINGS.scene.losRestriction);
    expect(r.movementRestriction).toBe("visible");
    expect(r.diagonalRule).toBe("chebyshev");
    expect(r.gridDistance).toEqual({ perCell: 5, unit: "ft" });
  });

  it("uses world-settings defaults over built-ins", () => {
    const scene = buildSceneDoc("w1", {}, "scene1");
    const ws = buildWorldSettingsDoc("w1", {
      ...DEFAULT_WORLD_SETTINGS,
      scene: { ...DEFAULT_WORLD_SETTINGS.scene, movementRestriction: "unrestricted" },
      pathfinding: { diagonalRule: "alternating" },
    }, "ws1");
    const r = resolveSceneSettings(scene, storeWith(scene, ws));
    expect(r.movementRestriction).toBe("unrestricted");
    expect(r.diagonalRule).toBe("alternating");
  });

  it("scene overrides win over world defaults", () => {
    const scene = buildSceneDoc("w1", {
      vision: { movementRestriction: "revealed", losRestriction: false },
      lighting: { enabled: false },
      grid: { kind: "square", size: 100, distance: { perCell: 1.5, unit: "m" } },
    }, "scene1");
    const ws = buildWorldSettingsDoc("w1", DEFAULT_WORLD_SETTINGS, "ws1");
    const r = resolveSceneSettings(scene, storeWith(scene, ws));
    expect(r.movementRestriction).toBe("revealed");
    expect(r.losRestriction).toBe(false);
    expect(r.lightingEnabled).toBe(false);
    expect(r.gridDistance).toEqual({ perCell: 1.5, unit: "m" });
  });

  it("builds a world-settings doc with world scope and null parent", () => {
    const ws = buildWorldSettingsDoc("w1");
    expect(ws.doc_type).toBe("world-settings");
    expect(ws.parent_id).toBeNull();
    expect((ws.system as { scene: unknown }).scene).toBeTruthy();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/core test -- scene-docs`
Expected: FAIL — `buildWorldSettingsDoc`/`resolveSceneSettings` not exported.

- [ ] **Step 3: Write minimal implementation**

In `src/client/core/src/scene-docs.ts`, add the types and replace `SceneSystem` + `buildSceneDoc`:

```typescript
export type MovementRestriction = "visible" | "revealed" | "unrestricted";
export type LightMode = "globalIllumination" | "environmentLight";
export type DiagonalRule = "chebyshev" | "alternating" | "euclidean" | "manhattan";
export type EasingMode = "easeInOut" | "linear";
export interface EnvironmentLight { color: string; intensity: number; }
export interface GridDistance { perCell: number; unit: string; }

export interface SceneVisionOverrides {
  losRestriction?: boolean;
  fog?: boolean;
  observerVision?: boolean;
  movementRestriction?: MovementRestriction;
}
export interface SceneLightingOverrides {
  enabled?: boolean;
  mode?: LightMode;
  environment?: EnvironmentLight;
}

export interface SceneSystem {
  grid: { kind: "square" | "hex"; size: number; distance?: GridDistance };
  background: string | null;
  vision?: SceneVisionOverrides;
  lighting?: SceneLightingOverrides;
}

export function buildSceneDoc(worldId: string, system: Partial<SceneSystem> = {}, id?: string): WireDocument {
  const full: SceneSystem = {
    grid: system.grid ?? { kind: "square", size: 100 },
    background: system.background ?? null,
    ...(system.vision ? { vision: system.vision } : {}),
    ...(system.lighting ? { lighting: system.lighting } : {}),
  };
  return envelope(worldId, "scene", null, full, id);
}

export interface WorldSceneDefaults {
  losRestriction: boolean;
  fog: boolean;
  lightingEnabled: boolean;
  lightMode: LightMode;
  environment: EnvironmentLight;
  observerVision: boolean;
  movementRestriction: MovementRestriction;
  partialCellLeniency: boolean;
}
export interface WorldSettingsSystem {
  scene: WorldSceneDefaults;
  pathfinding: { diagonalRule: DiagonalRule };
  animation: { speedCellsPerSec: number; easing: EasingMode };
}

export const DEFAULT_WORLD_SETTINGS: WorldSettingsSystem = {
  scene: {
    losRestriction: true,
    fog: true,
    lightingEnabled: true,
    lightMode: "environmentLight",
    environment: { color: "#0a0e1a", intensity: 0.0 },
    observerVision: false,
    movementRestriction: "visible",
    partialCellLeniency: true,
  },
  pathfinding: { diagonalRule: "chebyshev" },
  animation: { speedCellsPerSec: 6, easing: "easeInOut" },
};

export function buildWorldSettingsDoc(worldId: string, system: WorldSettingsSystem = DEFAULT_WORLD_SETTINGS, id?: string): WireDocument {
  return envelope(worldId, "world-settings", null, system, id);
}

export interface ResolvedSceneSettings {
  losRestriction: boolean;
  fog: boolean;
  observerVision: boolean;
  movementRestriction: MovementRestriction;
  lightingEnabled: boolean;
  lightMode: LightMode;
  environment: EnvironmentLight;
  partialCellLeniency: boolean;
  diagonalRule: DiagonalRule;
  animation: { speedCellsPerSec: number; easing: EasingMode };
  gridDistance: GridDistance;
}

// INVARIANT: buildWorldSettingsDoc seeds the FULL default object, so a world-settings
// doc is always complete; single-field edits patch it in place. Hence d = ws ?? default.
export function resolveSceneSettings(scene: WireDocument | undefined, store: ReadableDocuments): ResolvedSceneSettings {
  const ws = store.query("world-settings")[0]?.system as WorldSettingsSystem | undefined;
  const d = ws ?? DEFAULT_WORLD_SETTINGS;
  const sys = scene?.system as SceneSystem | undefined;
  const v = sys?.vision ?? {};
  const l = sys?.lighting ?? {};
  return {
    losRestriction: v.losRestriction ?? d.scene.losRestriction,
    fog: v.fog ?? d.scene.fog,
    observerVision: v.observerVision ?? d.scene.observerVision,
    movementRestriction: v.movementRestriction ?? d.scene.movementRestriction,
    lightingEnabled: l.enabled ?? d.scene.lightingEnabled,
    lightMode: l.mode ?? d.scene.lightMode,
    environment: l.environment ?? d.scene.environment,
    partialCellLeniency: d.scene.partialCellLeniency,
    diagonalRule: d.pathfinding.diagonalRule,
    animation: d.animation,
    gridDistance: sys?.grid?.distance ?? { perCell: 5, unit: "ft" },
  };
}
```

In `src/client/core/src/index.ts`, add to the existing scene-docs re-exports the new names: `MovementRestriction, LightMode, DiagonalRule, EasingMode, EnvironmentLight, GridDistance, SceneVisionOverrides, SceneLightingOverrides, WorldSceneDefaults, WorldSettingsSystem, DEFAULT_WORLD_SETTINGS, buildWorldSettingsDoc, ResolvedSceneSettings, resolveSceneSettings` (match the existing export style — types via `export type`, values via `export`).

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/core test -- scene-docs`
Expected: PASS.

- [ ] **Step 5: Typecheck + commit**

Run: `pnpm --filter @shadowcat/core typecheck`
```bash
git add src/client/core/src/scene-docs.ts src/client/core/src/index.ts src/client/core/src/scene-docs.test.ts
git commit -m "feat(m10e-1): scene settings model + world-settings doc + inheritance resolver"
```

---

### Task 2: Light-gradation + vision-modes registries

**Files:**
- Modify: `src/client/core/src/scene-docs.ts`
- Modify: `src/client/core/src/index.ts`
- Test: `src/client/core/src/scene-docs.test.ts`

**Interfaces:**
- Consumes: `envelope`, `ReadableDocuments`.
- Produces: `GradationBand`, `LightGradationSystem`, `DEFAULT_GRADATION`, `buildLightGradationDoc(worldId, system?, id?)`, `resolveGradation(store)`; `VisionMode`, `VisionModesSystem`, `SEED_VISION_MODES`, `buildVisionModesDoc(worldId, system?, id?)`, `resolveVisionModes(store)`.

- [ ] **Step 1: Write the failing test**

Add to `scene-docs.test.ts`:

```typescript
import { buildLightGradationDoc, resolveGradation, DEFAULT_GRADATION, buildVisionModesDoc, resolveVisionModes, SEED_VISION_MODES } from "./scene-docs";

describe("light-gradation registry", () => {
  it("seeds bright/dim/dark sorted descending by minIllumination", () => {
    const g = resolveGradation(storeWith(buildLightGradationDoc("w1")));
    expect(g.map((b) => b.name)).toEqual(["bright", "dim", "dark"]);
    expect(g[0].minIllumination).toBeGreaterThan(g[1].minIllumination);
  });
  it("falls back to DEFAULT_GRADATION when no doc present", () => {
    expect(resolveGradation(storeWith())).toEqual([...DEFAULT_GRADATION.bands].sort((a, b) => b.minIllumination - a.minIllumination));
  });
});

describe("vision-modes registry", () => {
  it("seeds normal + darkvision with their floors", () => {
    const m = resolveVisionModes(storeWith(buildVisionModesDoc("w1")));
    expect(m.normal.illuminationFloor).toBe("dim");
    expect(m.darkvision.illuminationFloor).toBe("dark");
  });
  it("falls back to SEED_VISION_MODES when no doc present", () => {
    expect(resolveVisionModes(storeWith())).toEqual(SEED_VISION_MODES);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/core test -- scene-docs`
Expected: FAIL — symbols not exported.

- [ ] **Step 3: Write minimal implementation**

Append to `scene-docs.ts`:

```typescript
export interface GradationBand { name: string; minIllumination: number; }
export interface LightGradationSystem { bands: GradationBand[]; }

export const DEFAULT_GRADATION: LightGradationSystem = {
  bands: [
    { name: "bright", minIllumination: 0.67 },
    { name: "dim", minIllumination: 0.34 },
    { name: "dark", minIllumination: 0.0 },
  ],
};

export function buildLightGradationDoc(worldId: string, system: LightGradationSystem = DEFAULT_GRADATION, id?: string): WireDocument {
  return envelope(worldId, "light-gradation", null, system, id);
}

// Returns bands sorted brightest-first so a consumer can pick the first band whose
// minIllumination a cell meets.
export function resolveGradation(store: ReadableDocuments): GradationBand[] {
  const sys = store.query("light-gradation")[0]?.system as LightGradationSystem | undefined;
  const bands = sys?.bands ?? DEFAULT_GRADATION.bands;
  return [...bands].sort((a, b) => b.minIllumination - a.minIllumination);
}

export interface VisionMode { id: string; name: string; illuminationFloor: string; defaultRange: number; renderHint?: string; }
export interface VisionModesSystem { modes: Record<string, VisionMode>; }

export const SEED_VISION_MODES: Record<string, VisionMode> = {
  normal: { id: "normal", name: "Normal", illuminationFloor: "dim", defaultRange: 0 },
  darkvision: { id: "darkvision", name: "Darkvision", illuminationFloor: "dark", defaultRange: 12, renderHint: "desaturate" },
};

export function buildVisionModesDoc(worldId: string, system: VisionModesSystem = { modes: SEED_VISION_MODES }, id?: string): WireDocument {
  return envelope(worldId, "vision-modes", null, system, id);
}

export function resolveVisionModes(store: ReadableDocuments): Record<string, VisionMode> {
  const sys = store.query("vision-modes")[0]?.system as VisionModesSystem | undefined;
  return sys?.modes ?? SEED_VISION_MODES;
}
```

Add exports to `index.ts`: `GradationBand, LightGradationSystem, DEFAULT_GRADATION, buildLightGradationDoc, resolveGradation, VisionMode, VisionModesSystem, SEED_VISION_MODES, buildVisionModesDoc, resolveVisionModes`.

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/core test -- scene-docs`
Expected: PASS.

- [ ] **Step 5: Typecheck + commit**

Run: `pnpm --filter @shadowcat/core typecheck`
```bash
git add src/client/core/src/scene-docs.ts src/client/core/src/index.ts src/client/core/src/scene-docs.test.ts
git commit -m "feat(m10e-1): light-gradation + vision-modes registries"
```

---

### Task 3: Actor/token vision modes in EffectiveActor

**Files:**
- Modify: `src/client/core/src/scene-docs.ts` (`ActorSystem`, `TokenOverrides`)
- Modify: `src/client/core/src/actor.ts` (`EffectiveActor`, `project`)
- Modify: `src/client/core/src/index.ts` (export `VisionAssignment`)
- Test: `src/client/core/src/actor.test.ts`

**Interfaces:**
- Consumes: `VisionMode` (Task 2), existing `resolveTokenActor`, `project`, `ActorSystem`, `TokenOverrides`.
- Produces: `VisionAssignment { mode: string; range: number }`; `ActorSystem.vision?: VisionAssignment[]`; `TokenOverrides.vision?: VisionAssignment[]`; `EffectiveActor.visionModes: VisionAssignment[]`.

- [ ] **Step 1: Write the failing test**

Add to `src/client/core/src/actor.test.ts` (it already builds an `ActorSystem` fixture named `sys` and has `storeWith`):

```typescript
it("resolves actor vision modes onto the effective actor", () => {
  const withVision = { ...sys, vision: [{ mode: "darkvision", range: 12 }] };
  const actor = buildActorDoc("w1", withVision, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
  const eff = resolveTokenActor(token, storeWith(actor));
  expect(eff?.visionModes).toEqual([{ mode: "darkvision", range: 12 }]);
});

it("defaults visionModes to [] when actor has none", () => {
  const actor = buildActorDoc("w1", sys, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
  expect(resolveTokenActor(token, storeWith(actor))?.visionModes).toEqual([]);
});

it("per-token override replaces actor vision modes", () => {
  const withVision = { ...sys, vision: [{ mode: "darkvision", range: 12 }] };
  const actor = buildActorDoc("w1", withVision, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
  (token.system as { overrides?: { vision?: { mode: string; range: number }[] } }).overrides = { vision: [{ mode: "darkvision", range: 6 }] };
  expect(resolveTokenActor(token, storeWith(actor))?.visionModes).toEqual([{ mode: "darkvision", range: 6 }]);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/core test -- actor`
Expected: FAIL — `visionModes` undefined.

- [ ] **Step 3: Write minimal implementation**

In `scene-docs.ts`, add the type and extend `ActorSystem` + `TokenOverrides`:

```typescript
export interface VisionAssignment { mode: string; range: number; }
```
Add `vision?: VisionAssignment[];` to `interface ActorSystem` and to `interface TokenOverrides`.

In `actor.ts`, extend `EffectiveActor` with `visionModes: VisionAssignment[];` (import `VisionAssignment` from `./scene-docs`), and in `project()` add:

```typescript
visionModes: overrides?.vision ?? base.vision ?? [],
```

Export `VisionAssignment` from `index.ts`.

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/core test -- actor`
Expected: PASS.

- [ ] **Step 5: Typecheck + commit**

Run: `pnpm --filter @shadowcat/core typecheck`
```bash
git add src/client/core/src/scene-docs.ts src/client/core/src/actor.ts src/client/core/src/index.ts src/client/core/src/actor.test.ts
git commit -m "feat(m10e-1): actor/token vision modes on EffectiveActor"
```

---

### Task 4: Light doc_type builder + wall `blocksLight`

**Files:**
- Modify: `src/client/core/src/scene-docs.ts` (`LightSystem`, `buildLightDoc`)
- Modify: `src/client/core/src/index.ts`
- Modify: `src/modules/scene-tools/src/controller.svelte.ts` (wall tool)
- Test: `src/client/core/src/scene-docs.test.ts`, `src/modules/scene-tools/src/wall-tool.test.ts`

**Interfaces:**
- Consumes: `envelope`.
- Produces: `LightSystem`, `buildLightDoc(worldId, sceneId, system, id?)`; wall creation now writes `blocksLight: true`.

- [ ] **Step 1: Write the failing tests**

Add to `scene-docs.test.ts`:

```typescript
import { buildLightDoc } from "./scene-docs";

it("builds a light doc parented to its scene", () => {
  const l = buildLightDoc("w1", "scene1", { x: 10, y: 20, color: "#ffd9a0", intensity: 1, brightRadius: 4, dimRadius: 8, enabled: true });
  expect(l.doc_type).toBe("light");
  expect(l.parent_id).toBe("scene1");
  expect((l.system as { brightRadius: number }).brightRadius).toBe(4);
});
```

In `src/modules/scene-tools/src/wall-tool.test.ts`, extend the existing wall-create assertion to require `blocksLight: true` (find the `expect(...).toMatchObject({ ... blocksSight: true, blocksMove: true })` and add `blocksLight: true`).

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test -- scene-docs` (FAIL: `buildLightDoc` missing)
Run: `pnpm --filter @shadowcat/module-scene-tools test -- wall-tool` (FAIL: `blocksLight` absent)

- [ ] **Step 3: Write minimal implementation**

Append to `scene-docs.ts`:

```typescript
export interface LightSystem {
  x: number; y: number;
  color: string; intensity: number;
  brightRadius: number; dimRadius: number;
  falloff?: { curve: "linear" | "quadratic" | "none" };
  enabled: boolean;
}
export function buildLightDoc(worldId: string, sceneId: string, system: LightSystem, id?: string): WireDocument {
  return envelope(worldId, "light", sceneId, system, id);
}
```
Export `LightSystem, buildLightDoc` from `index.ts`.

In `controller.svelte.ts` `makeWallTool`, change the created system to include `blocksLight: true`:

```typescript
doc: buildSceneEntityDoc(ctx.world, scene.id, "wall", {
  seg: { x1: anchor.x, y1: anchor.y, x2: b.x, y2: b.y },
  blocksSight: true,
  blocksMove: true,
  blocksLight: true,
}),
```

- [ ] **Step 4: Run tests to verify they pass**

Run both test commands from Step 2 — Expected: PASS.

- [ ] **Step 5: Typecheck + commit**

Run: `pnpm --filter @shadowcat/core typecheck && pnpm --filter @shadowcat/module-scene-tools typecheck`
```bash
git add src/client/core/src/scene-docs.ts src/client/core/src/index.ts src/client/core/src/scene-docs.test.ts src/modules/scene-tools/src/controller.svelte.ts src/modules/scene-tools/src/wall-tool.test.ts
git commit -m "feat(m10e-1): light doc_type builder + wall blocksLight flag"
```

---

### Task 5: Scaffold `module-game-settings` + idempotent seed of the 3 config-docs

**Files:**
- Create: `src/modules/game-settings/package.json`, `src/modules/game-settings/tsconfig.json` (copy from `src/modules/factions/`, replace `factions`→`game-settings`)
- Create: `src/modules/game-settings/src/index.ts`
- Create: `src/modules/game-settings/src/GameSettingsPanel.svelte`
- Create: `src/modules/game-settings/src/seed.test.ts`
- Modify: `src/client/shell/src/App.svelte` (import + register)

**Interfaces:**
- Consumes: `Module`, `getAppContext`, `buildWorldSettingsDoc`, `buildLightGradationDoc`, `buildVisionModesDoc`.
- Produces: `gameSettings: Module` (manifest id `"game-settings"`, requires `shadowcat.surface:sidebar`), package `@shadowcat/module-game-settings`.

- [ ] **Step 1: Scaffold the package**

Copy `src/modules/factions/package.json` → `src/modules/game-settings/package.json`; set `"name": "@shadowcat/module-game-settings"`. Copy `src/modules/factions/tsconfig.json` → `src/modules/game-settings/tsconfig.json` unchanged. Run `pnpm install` to register the new workspace package.

- [ ] **Step 2: Write the failing seed test**

Create `src/modules/game-settings/src/seed.test.ts`:

```typescript
import { describe, it, expect, vi } from "vitest";
import { render } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

describe("game-settings seed", () => {
  it("GM seeds world-settings, light-gradation, vision-modes once", () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent }),
    });
    const created = dispatchIntent.mock.calls.flatMap((c) => c[0]).map((op: { doc: { doc_type: string } }) => op.doc.doc_type);
    expect(created).toContain("world-settings");
    expect(created).toContain("light-gradation");
    expect(created).toContain("vision-modes");
  });

  it("non-GM seeds nothing", () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, {
      context: setAppContextForTest({ role: "player", world: "w1", documents: new DocumentStore(), dispatchIntent }),
    });
    expect(dispatchIntent).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/module-game-settings test -- seed`
Expected: FAIL — panel/module not found.

- [ ] **Step 4: Write minimal implementation**

Create `src/modules/game-settings/src/index.ts`:

```typescript
import type { Module } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

/** GM game configuration: scene vision/lighting defaults + per-scene overrides,
 * light gradation, vision modes, pathfinding + movement + animation settings.
 * Requires core-ui's sidebar; contributes after the user Settings panel. */
export const gameSettings: Module = {
  manifest: {
    id: "game-settings",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: ["shadowcat.surface:sidebar"],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({ id: "game-settings:sidebar", contract: "shadowcat.surface:sidebar", order: 1, component: GameSettingsPanel });
  },
};
```

Create `src/modules/game-settings/src/GameSettingsPanel.svelte` (seed only for this task; editor UI added in Tasks 6–7):

```svelte
<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildWorldSettingsDoc, buildLightGradationDoc, buildVisionModesDoc } from "@shadowcat/core";

  const ctx = getAppContext();
  let seeded = false;

  $effect(() => {
    if (ctx.role !== "gm" || seeded) return;
    seeded = true;
    const ops = [];
    if (ctx.documents.query("world-settings").length === 0) ops.push({ op: "create" as const, doc: buildWorldSettingsDoc(ctx.world) });
    if (ctx.documents.query("light-gradation").length === 0) ops.push({ op: "create" as const, doc: buildLightGradationDoc(ctx.world) });
    if (ctx.documents.query("vision-modes").length === 0) ops.push({ op: "create" as const, doc: buildVisionModesDoc(ctx.world) });
    if (ops.length > 0) ctx.dispatchIntent(ops);
  });
</script>

<section aria-label={ctx.t("gameSettings.title")}>
  <h2>{ctx.t("gameSettings.title")}</h2>
</section>
```

Add the i18n key to `src/client/ui-kit/src/locales/en.ts`: `"gameSettings.title": "Game settings",`.

In `src/client/shell/src/App.svelte`: add `import { gameSettings } from "@shadowcat/module-game-settings";` alongside the other module imports (after line 16), and add `gameSettings` to the `modules: [...]` array at line 84 (place it after `settings`).

- [ ] **Step 5: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/module-game-settings test -- seed`
Expected: PASS.

- [ ] **Step 6: Typecheck + commit**

Run: `pnpm --filter @shadowcat/module-game-settings typecheck && pnpm --filter @shadowcat/ui typecheck`
```bash
git add src/modules/game-settings package.json pnpm-lock.yaml src/client/ui-kit/src/locales/en.ts src/client/shell/src/App.svelte
git commit -m "feat(m10e-1): module-game-settings scaffold + config-doc seed"
```

---

### Task 6: World-defaults + gradation + vision-modes editors

**Files:**
- Modify: `src/modules/game-settings/src/GameSettingsPanel.svelte`
- Modify: `src/client/ui-kit/src/locales/en.ts`
- Test: `src/modules/game-settings/src/world-defaults.test.ts`

**Interfaces:**
- Consumes: `WorldSettingsSystem`, `LightGradationSystem`, `VisionModesSystem` shapes; `dispatchIntent` JSON-pointer updates against the seeded docs.
- Produces: editable world defaults (movement restriction, light mode, lighting enabled, diagonal rule, animation speed/easing), gradation band thresholds, vision-mode floors/ranges.

- [ ] **Step 1: Write the failing test**

Create `src/modules/game-settings/src/world-defaults.test.ts`:

```typescript
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildWorldSettingsDoc } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

function gmStoreWith(...docs) {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

describe("world defaults editor", () => {
  it("changing movement restriction dispatches a JSON-pointer update", async () => {
    const dispatchIntent = vi.fn();
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.movementRestriction") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "revealed" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "ws1", changes: [{ path: "/system/scene/movementRestriction", old: null, new: "revealed" }] },
    ]);
  });

  it("changing diagonal rule dispatches the pathfinding pointer", async () => {
    const dispatchIntent = vi.fn();
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws), dispatchIntent }) });
    const sel = screen.getByLabelText("gameSettings.diagonalRule") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "alternating" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "ws1", changes: [{ path: "/system/pathfinding/diagonalRule", old: null, new: "alternating" }] },
    ]);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/module-game-settings test -- world-defaults`
Expected: FAIL — controls not present.

- [ ] **Step 3: Write minimal implementation**

Extend `GameSettingsPanel.svelte`. Add (after the seed `$effect`) a derived read of the seeded docs and an `update` helper, then the editor markup. Keep the seed `$effect` from Task 5.

```svelte
<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import {
    buildWorldSettingsDoc, buildLightGradationDoc, buildVisionModesDoc,
    type WorldSettingsSystem, type LightGradationSystem, type VisionModesSystem, type WireDocument,
  } from "@shadowcat/core";

  const ctx = getAppContext();
  let seeded = false;
  $effect(() => {
    if (ctx.role !== "gm" || seeded) return;
    seeded = true;
    const ops = [];
    if (ctx.documents.query("world-settings").length === 0) ops.push({ op: "create" as const, doc: buildWorldSettingsDoc(ctx.world) });
    if (ctx.documents.query("light-gradation").length === 0) ops.push({ op: "create" as const, doc: buildLightGradationDoc(ctx.world) });
    if (ctx.documents.query("vision-modes").length === 0) ops.push({ op: "create" as const, doc: buildVisionModesDoc(ctx.world) });
    if (ops.length > 0) ctx.dispatchIntent(ops);
  });

  const ws = $derived.by((): WireDocument | undefined => ctx.documents.query("world-settings")[0]);
  const wsys = $derived.by((): WorldSettingsSystem | undefined => ws?.system as WorldSettingsSystem | undefined);

  function set(path: string, value: unknown): void {
    if (!ws) return;
    ctx.dispatchIntent([{ op: "update", doc_id: ws.id, changes: [{ path, old: null, new: value }] }]);
  }

  const MOVEMENT = ["visible", "revealed", "unrestricted"] as const;
  const LIGHTMODE = ["environmentLight", "globalIllumination"] as const;
  const DIAGONAL = ["chebyshev", "alternating", "euclidean", "manhattan"] as const;
  const EASING = ["easeInOut", "linear"] as const;
</script>

<section aria-label={ctx.t("gameSettings.title")}>
  <h2>{ctx.t("gameSettings.title")}</h2>

  {#if ctx.role === "gm" && wsys}
    <label>
      {ctx.t("gameSettings.movementRestriction")}
      <select aria-label="gameSettings.movementRestriction" value={wsys.scene.movementRestriction}
        onchange={(e) => set("/system/scene/movementRestriction", (e.currentTarget as HTMLSelectElement).value)}>
        {#each MOVEMENT as m}<option value={m}>{m}</option>{/each}
      </select>
    </label>

    <label>
      {ctx.t("gameSettings.lightingEnabled")}
      <input type="checkbox" aria-label="gameSettings.lightingEnabled" checked={wsys.scene.lightingEnabled}
        onchange={(e) => set("/system/scene/lightingEnabled", (e.currentTarget as HTMLInputElement).checked)} />
    </label>

    <label>
      {ctx.t("gameSettings.lightMode")}
      <select aria-label="gameSettings.lightMode" value={wsys.scene.lightMode}
        onchange={(e) => set("/system/scene/lightMode", (e.currentTarget as HTMLSelectElement).value)}>
        {#each LIGHTMODE as m}<option value={m}>{m}</option>{/each}
      </select>
    </label>

    <label>
      {ctx.t("gameSettings.diagonalRule")}
      <select aria-label="gameSettings.diagonalRule" value={wsys.pathfinding.diagonalRule}
        onchange={(e) => set("/system/pathfinding/diagonalRule", (e.currentTarget as HTMLSelectElement).value)}>
        {#each DIAGONAL as d}<option value={d}>{d}</option>{/each}
      </select>
    </label>

    <label>
      {ctx.t("gameSettings.animSpeed")}
      <input type="number" min="1" step="1" aria-label="gameSettings.animSpeed" value={wsys.animation.speedCellsPerSec}
        onchange={(e) => set("/system/animation/speedCellsPerSec", Number((e.currentTarget as HTMLInputElement).value))} />
    </label>

    <label>
      {ctx.t("gameSettings.animEasing")}
      <select aria-label="gameSettings.animEasing" value={wsys.animation.easing}
        onchange={(e) => set("/system/animation/easing", (e.currentTarget as HTMLSelectElement).value)}>
        {#each EASING as ea}<option value={ea}>{ea}</option>{/each}
      </select>
    </label>
  {/if}
</section>
```

> Gradation + vision-mode field editors follow the same `set()` + JSON-pointer pattern against the `light-gradation` (`/system/bands/<i>/minIllumination`) and `vision-modes` (`/system/modes/<id>/illuminationFloor`, `/system/modes/<id>/defaultRange`) docs. Add one numeric input per seeded gradation band and one row (floor select + range number) per seeded vision mode, each with `aria-label` `gameSettings.gradation.<name>` / `gameSettings.visionMode.<id>` and a matching `set(...)` call. Read them via `$derived.by(() => ctx.documents.query("light-gradation")[0]?.system as LightGradationSystem | undefined)` and `...query("vision-modes")[0]?.system as VisionModesSystem | undefined`.

Add these keys to `en.ts`: `gameSettings.movementRestriction`, `gameSettings.lightingEnabled`, `gameSettings.lightMode`, `gameSettings.diagonalRule`, `gameSettings.animSpeed`, `gameSettings.animEasing`, `gameSettings.gradation`, `gameSettings.visionModes` (English labels of your choosing, e.g. `"gameSettings.movementRestriction": "Movement restriction"`).

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/module-game-settings test -- world-defaults`
Expected: PASS.

- [ ] **Step 5: Typecheck + commit**

Run: `pnpm --filter @shadowcat/module-game-settings typecheck && pnpm --filter @shadowcat/ui typecheck`
```bash
git add src/modules/game-settings/src src/client/ui-kit/src/locales/en.ts
git commit -m "feat(m10e-1): world defaults + gradation + vision-modes editors"
```

---

### Task 7: Per-scene overrides editor (scene picker + grid.distance)

**Files:**
- Modify: `src/modules/game-settings/src/GameSettingsPanel.svelte`
- Modify: `src/client/ui-kit/src/locales/en.ts`
- Test: `src/modules/game-settings/src/scene-overrides.test.ts`

**Interfaces:**
- Consumes: `SceneSystem` shape; scene docs from `ctx.documents.query("scene")`.
- Produces: per-scene override editing — vision (losRestriction, fog, observerVision, movementRestriction), lighting (enabled, mode, environment color/intensity), `grid.distance.perCell`/`unit` — written to the selected scene doc via JSON-pointer.

- [ ] **Step 1: Write the failing test**

Create `src/modules/game-settings/src/scene-overrides.test.ts`:

```typescript
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildWorldSettingsDoc, buildSceneDoc } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

function gmStoreWith(...docs) {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

describe("per-scene overrides", () => {
  it("setting movement restriction override writes to the selected scene doc", async () => {
    const dispatchIntent = vi.fn();
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    const scene = buildSceneDoc("w1", {}, "scene1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws, scene), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.scene.movementRestriction") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "unrestricted" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "scene1", changes: [{ path: "/system/vision/movementRestriction", old: null, new: "unrestricted" }] },
    ]);
  });

  it("setting grid distance per-cell writes to the scene grid", async () => {
    const dispatchIntent = vi.fn();
    const scene = buildSceneDoc("w1", {}, "scene1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(buildWorldSettingsDoc("w1", undefined, "ws1"), scene), dispatchIntent }) });
    const input = screen.getByLabelText("gameSettings.scene.distancePerCell") as HTMLInputElement;
    await fireEvent.change(input, { target: { value: "1.5" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "scene1", changes: [{ path: "/system/grid/distance", old: null, new: { perCell: 1.5, unit: "ft" } }] },
    ]);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/module-game-settings test -- scene-overrides`
Expected: FAIL.

- [ ] **Step 3: Write minimal implementation**

Add to `GameSettingsPanel.svelte`'s script: scene list + selection + setters.

```svelte
  import { type SceneSystem } from "@shadowcat/core";

  const scenes = $derived.by((): WireDocument[] => ctx.documents.query("scene"));
  let selectedSceneId = $state<string | null>(null);
  const scene = $derived.by((): WireDocument | undefined =>
    scenes.find((s) => s.id === (selectedSceneId ?? scenes[0]?.id)));
  const ssys = $derived.by((): SceneSystem | undefined => scene?.system as SceneSystem | undefined);

  function setScene(path: string, value: unknown): void {
    if (!scene) return;
    ctx.dispatchIntent([{ op: "update", doc_id: scene.id, changes: [{ path, old: null, new: value }] }]);
  }
```

Add markup (inside the GM block, after the world-defaults section):

```svelte
  {#if ctx.role === "gm" && scene && ssys}
    <h3>{ctx.t("gameSettings.scene.title")}</h3>
    {#if scenes.length > 1}
      <select aria-label="gameSettings.scene.pick" value={scene.id}
        onchange={(e) => (selectedSceneId = (e.currentTarget as HTMLSelectElement).value)}>
        {#each scenes as s}<option value={s.id}>{s.id}</option>{/each}
      </select>
    {/if}

    <label>
      {ctx.t("gameSettings.scene.movementRestriction")}
      <select aria-label="gameSettings.scene.movementRestriction" value={ssys.vision?.movementRestriction ?? ""}
        onchange={(e) => setScene("/system/vision/movementRestriction", (e.currentTarget as HTMLSelectElement).value)}>
        <option value="">{ctx.t("gameSettings.inherit")}</option>
        {#each MOVEMENT as m}<option value={m}>{m}</option>{/each}
      </select>
    </label>

    <label>
      {ctx.t("gameSettings.scene.distancePerCell")}
      <input type="number" min="0" step="0.5" aria-label="gameSettings.scene.distancePerCell"
        value={ssys.grid?.distance?.perCell ?? ""}
        onchange={(e) => setScene("/system/grid/distance", { perCell: Number((e.currentTarget as HTMLInputElement).value), unit: ssys.grid?.distance?.unit ?? "ft" })} />
    </label>
  {/if}
```

> Add the remaining per-scene override controls (fog, losRestriction, observerVision checkboxes; lighting.enabled, lighting.mode, environment color/intensity) the same way: checkbox/select/input with `aria-label` `gameSettings.scene.<field>`, writing `/system/vision/<field>`, `/system/lighting/<field>`, or `/system/lighting/environment`. Each scene-override control offers an explicit inherit (empty/indeterminate) state so an unset field stays absent and inherits the world default.

Add `en.ts` keys: `gameSettings.scene.title`, `gameSettings.scene.pick`, `gameSettings.scene.movementRestriction`, `gameSettings.scene.distancePerCell`, `gameSettings.inherit`, plus the remaining field keys.

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/module-game-settings test -- scene-overrides`
Expected: PASS.

- [ ] **Step 5: Typecheck + commit**

Run: `pnpm --filter @shadowcat/module-game-settings typecheck && pnpm --filter @shadowcat/ui typecheck`
```bash
git add src/modules/game-settings/src src/client/ui-kit/src/locales/en.ts
git commit -m "feat(m10e-1): per-scene vision/lighting override editor + grid distance"
```

---

### Task 8: Actor vision-mode authoring in the actors module

**Files:**
- Modify: `src/modules/actors/src/ActorsPanel.svelte`
- Modify: `src/client/ui-kit/src/locales/en.ts`
- Test: `src/modules/actors/src/ActorsPanel.test.ts`

**Interfaces:**
- Consumes: `VisionAssignment`, the actor `system.vision` field; the existing create/edit dispatch in `ActorsPanel`.
- Produces: GM authoring of a darkvision range on an actor — create writes `system.vision: [{ mode: "darkvision", range }]` when range > 0; per-row edit updates `/system/vision`.

- [ ] **Step 1: Write the failing test**

Add to `src/modules/actors/src/ActorsPanel.test.ts`:

```typescript
it("create includes darkvision vision when a range is entered", async () => {
  const dispatchIntent = vi.fn();
  const { listAssets } = await import("@shadowcat/core");
  vi.mocked(listAssets).mockResolvedValue([
    { id: "asset-1", world_id: "w1", original_name: "hero.png", content_type: "image/png" } as never,
  ]);
  render(ActorsPanel, {
    context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, assets: { url: (id: string) => `/assets/${id}` } as never }),
  });
  await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "hero.png" }).length).toBeGreaterThan(0));
  await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Drow" } });
  await fireEvent.click(screen.getByRole("button", { name: "hero.png" }));
  await fireEvent.change(screen.getByLabelText("actors.darkvision"), { target: { value: "12" } });
  await fireEvent.click(screen.getByText("actors.create"));

  const ops = dispatchIntent.mock.calls[0][0];
  expect(ops[0].doc.system).toMatchObject({ vision: [{ mode: "darkvision", range: 12 }] });
});

it("create omits vision when darkvision range is 0", async () => {
  const dispatchIntent = vi.fn();
  const { listAssets } = await import("@shadowcat/core");
  vi.mocked(listAssets).mockResolvedValue([
    { id: "asset-1", world_id: "w1", original_name: "hero.png", content_type: "image/png" } as never,
  ]);
  render(ActorsPanel, {
    context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, assets: { url: (id: string) => `/assets/${id}` } as never }),
  });
  await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "hero.png" }).length).toBeGreaterThan(0));
  await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Human" } });
  await fireEvent.click(screen.getByRole("button", { name: "hero.png" }));
  await fireEvent.click(screen.getByText("actors.create"));
  expect(dispatchIntent.mock.calls[0][0][0].doc.system.vision).toBeUndefined();
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/module-actors test -- ActorsPanel`
Expected: FAIL — `actors.darkvision` control absent.

- [ ] **Step 3: Write minimal implementation**

In `ActorsPanel.svelte`: add a `darkvision` create-form state field and control, and include `vision` in the created actor `system` only when range > 0.

In the create-form script add:
```typescript
let darkvision = $state(0);
```
In the create form markup (near the shape/size controls) add:
```svelte
<label>
  {ctx.t("actors.darkvision")}
  <input type="number" min="0" step="1" aria-label="actors.darkvision" bind:value={darkvision} />
</label>
```
Where the create handler builds the actor `system` object, add the conditional vision field (mirror how `shape`/`size` are placed onto the system):
```typescript
const system = {
  name, displayName, visual, size, shape, faction: null, conditions: [], prototype: true,
  ...(darkvision > 0 ? { vision: [{ mode: "darkvision", range: darkvision }] } : {}),
};
```
(Use the actual local variable names already present in the handler; only add the spread line.)

For per-row edit (GM), mirror the existing per-row shape/size edit: add a darkvision number input that dispatches `{ op:"update", doc_id: actor.id, changes:[{ path:"/system/vision", old:null, new: range>0 ? [{ mode:"darkvision", range }] : [] }] }`.

Add `en.ts` key: `"actors.darkvision": "Darkvision range (cells)",`.

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/module-actors test -- ActorsPanel`
Expected: PASS.

- [ ] **Step 5: Typecheck + commit**

Run: `pnpm --filter @shadowcat/module-actors typecheck && pnpm --filter @shadowcat/ui typecheck`
```bash
git add src/modules/actors/src/ActorsPanel.svelte src/modules/actors/src/ActorsPanel.test.ts src/client/ui-kit/src/locales/en.ts
git commit -m "feat(m10e-1): actor darkvision authoring in actors module"
```

---

### Task 9: Full-client gate (typecheck + lint + all tests + build)

**Files:** none (verification task).

- [ ] **Step 1: Run the full client gate**

```bash
pnpm -r typecheck
pnpm -r test
pnpm -r lint
pnpm --filter @shadowcat/ui build
```
Expected: all green. The build proves the new `@shadowcat/module-game-settings` package resolves and `App.svelte` wiring compiles (required because `embed.rs` validates `dist/` at server compile time).

- [ ] **Step 2: Manual smoke note (record, do not skip)**

Confirm in the running client (GM): the Game settings panel appears in the sidebar; changing movement restriction / diagonal rule / a per-scene override / an actor's darkvision range dispatches and round-trips (the doc updates). Lights/walls `blocksLight` carry data but do not render yet (V3).

- [ ] **Step 3: Commit any gate fixes**

```bash
git add -A
git commit -m "chore(m10e-1): full-client gate green"
```

---

## Self-Review

**1. Spec coverage (§5):**
- §5.1 world config-docs — Task 1 (`world-settings`), Task 2 (`light-gradation`, `vision-modes`); seeded in Task 5; edited in Task 6. ✓
- §5.2 scene overrides + `grid.distance` — Task 1 (model) + Task 7 (UI). ✓
- §5.3 wall `blocksLight` — Task 4. ✓
- §5.4 `light` doc_type — Task 4 (builder; place tool + render are V3, per spec §10/§7 — correctly deferred). ✓
- §5.5 actor/token vision modes → `EffectiveActor.visionModes` — Task 3 (resolution) + Task 8 (authoring). ✓
- §5.6 observer tier — **no code in V1**: reuses existing token permissions; consumed in V2. The plan adds no observer field (correct — observer designation is the existing permission tier). Noted, not a task. ✓
- §5.7 units (cells; `grid.distance` labels-only) — encoded in Task 1/7. ✓

**2. Placeholder scan:** The two prose `>` notes (Task 6 gradation/vision-mode editors, Task 7 remaining scene controls) describe additional controls that repeat an already-fully-shown pattern (`set()`/`setScene()` + JSON-pointer + `aria-label`); the representative control and its test are fully specified. These are pattern-repetition instructions, not logic gaps. Acceptable.

**3. Type consistency:** `WorldSettingsSystem`/`ResolvedSceneSettings`/`VisionAssignment`/`LightSystem`/`GradationBand`/`VisionMode` names are consistent across Tasks 1–8; `resolveSceneSettings`/`resolveGradation`/`resolveVisionModes` and `buildWorldSettingsDoc`/`buildLightGradationDoc`/`buildVisionModesDoc`/`buildLightDoc` are referenced with identical signatures where consumed. JSON-pointer paths match the seeded doc shapes.

## Buddy-check directives

Per the M10 execution cadence (`next-session-m10-resume`), after all tasks: run a **whole-branch buddy-check** — dispatch `shadowcat-spec-reviewer` (does the branch implement spec §5 with nothing skipped/downgraded?) + `shadowcat-code-reviewer` (bugs, convention adherence, fail-closed resolution, no server changes leaked in) as the two-reviewer pair; reconcile to convergence; fix findings; then merge `--no-ff` to local main. Reviewer focus areas: (a) `resolveSceneSettings` inheritance correctness + the "world-settings is always complete" invariant; (b) idempotent seed runs once and only for GM; (c) JSON-pointer update shapes round-trip; (d) `EffectiveActor.visionModes` override precedence (token over actor); (e) confirm zero Rust/ts-rs changes.
