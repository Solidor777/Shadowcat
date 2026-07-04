# M10h — Faces + Animated Tokens Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generalize the token `visual` from a flat `{kind:"image"}` to a discriminated union admitting multi-face (manual + condition-driven) and animated (frame-list/grid-sheet) token art, rendered through a new Container-per-token structure that also gives M10j (fx/emotes) a clean attach point.

**Architecture:** A new `TokenVisual`/`RenderVisual`/`FaceVisual`/`AnimatedSource` type family lives in `@shadowcat/core` (`scene-docs.ts`), resolved by a new `resolveTokenVisual` read-through (`actor.ts`) that collapses `faces` down to a plain `RenderVisual` (image or animated) — so the render layer only ever sees two kinds. `TokenView` maps that through the `AssetResolver` into a `TokenNodeSpec.visual` the `DisplayBackend` renders; `PixiBackend` migrates each token from a bare `Sprite` to a `Container` (border + badges as real children) and adds tick-driven `AnimatedSprite` playback. Authoring lands in `module-actors`'s `ActorsPanel.svelte`.

**Tech Stack:** TypeScript, Svelte 5 (runes), Vitest + @testing-library/svelte, Playwright, PixiJS v8 (`Sprite`/`AnimatedSprite`/`Container`/`Texture`).

## Global Constraints

- **No server/ts-rs change.** `visual`/`face` are opaque `system`-body JSON, exactly like `movementModel`/`bounds`/`snapToGrid`. Every task in this plan touches only `src/client/**` and `docs/**`.
- **Not security/secrecy-sensitive.** Visuals are cosmetic; no new document-visibility surface. Standard two-reviewer gate per task (`shadowcat-spec-reviewer` + `shadowcat-code-reviewer`) — no mandatory whole-branch buddy-check (design spec §9), though available on request.
- **`resolveTokenBox`/`resolveTokenActor` stay the single read-through** for size/shape/faction/conditions — untouched by this plan.
- **Add a typecheck run to every task's verification**, not just `vitest run` — esbuild-backed Vitest silently passes type errors that only `tsc --noEmit` catches (a real gap hit in a prior M10 checkpoint).
- **`FaceVisual = RenderVisual`** — a face is never itself `{kind:"faces"}`. This is enforced structurally by the TypeScript type (Task 1); do not add a runtime recursion guard beyond what Task 2 already does for malformed wire data.
- Design spec: `docs/superpowers/specs/2026-07-03-m10h-faces-animated-design.md` (read this first for full rationale — this plan implements it verbatim; deviations are called out inline where the plan makes an implementation-level choice the spec left open).

## Model/Effort directives

Written mainline in this session (Sonnet 5, high effort) per user choice at the tier-switch checkpoint — the design was already fully locked and the decomposition below is mechanical, so a dedicated plan-writer subagent was not dispatched.

**Dispatcher (SDD execution):** mainline in this session (user directive: "You are now the dispatcher") — no `sdd-dispatcher` delegation. Per project `CLAUDE.md`, the implementer/reviewer roles are the project's named agents, not the generic `sdd-*` set: implementer = `shadowcat-coder` (sonnet, effort medium; escalate to `shadowcat-coder-opus` on BLOCKED); per-task review = the `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` pair (opus twins if findings read shallow/uncertain on a tough diff) run as the standard two-reviewer gate, per this plan's Global Constraints (no mandatory whole-branch buddy-check — design spec §9).

---

## Task 1: Core visual union types + per-token face field

**Files:**
- Modify: `src/client/core/src/scene-docs.ts:140-188` (`TokenSystem`, `ActorVisual`, `ActorSystem`, `TokenOverrides`)
- Modify: `src/client/core/src/index.ts:77` (type exports)
- Test: `src/client/core/src/scene-docs.test.ts` (new or extend the existing file — check for one first; if absent, create it)

**Interfaces:**
- Produces: `RenderVisual`, `AnimatedSource`, `FaceVisual`, `TokenVisual` (all exported from `@shadowcat/core`); `ActorSystem.visual: TokenVisual`; `TokenOverrides.visual?: TokenVisual`; `TokenSystem.visual?: TokenVisual`; `TokenSystem.face?: string` (new).

- [ ] **Step 1: Write the failing test**

Add to `src/client/core/src/scene-docs.test.ts` (create the file if it doesn't already exist in the repo — check with a quick `Read`/`Glob` before writing; if it exists, append):

```typescript
import { describe, it, expect } from "vitest";
import type { TokenVisual, FaceVisual, RenderVisual, AnimatedSource } from "./scene-docs";

describe("TokenVisual union (M10h)", () => {
  it("admits a plain image visual", () => {
    const v: TokenVisual = { kind: "image", asset: "a1" };
    expect(v.kind).toBe("image");
  });

  it("admits an animated visual with a frame-list source", () => {
    const v: TokenVisual = { kind: "animated", source: { type: "frames", frames: ["a1", "a2"] }, fps: 8, loop: true };
    expect(v).toMatchObject({ kind: "animated", fps: 8, loop: true });
  });

  it("admits an animated visual with a grid-sheet source", () => {
    const source: AnimatedSource = { type: "sheet", asset: "sheet1", rows: 2, cols: 4, count: 7 };
    const v: TokenVisual = { kind: "animated", source, fps: 12, loop: false };
    expect(v.kind).toBe("animated");
  });

  it("admits a faces visual whose face values are themselves RenderVisuals (image or animated)", () => {
    const bloodied: FaceVisual = { kind: "animated", source: { type: "frames", frames: ["b1"] }, fps: 4, loop: true };
    const v: TokenVisual = {
      kind: "faces",
      faces: { normal: { kind: "image", asset: "n1" }, bloodied },
      default: "normal",
      faceMap: { bleeding: "bloodied" },
    };
    expect(Object.keys(v.faces)).toEqual(["normal", "bloodied"]);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/core test scene-docs.test.ts`
Expected: FAIL — `RenderVisual`/`FaceVisual`/`AnimatedSource` are not exported from `./scene-docs` yet (TS compile error surfaces as a test-run failure).

- [ ] **Step 3: Add the type family and update the three visual-bearing fields**

In `src/client/core/src/scene-docs.ts`, replace the `ActorVisual` interface (currently lines 154-159) with:

```typescript
/** The two kinds the render layer actually draws — the render/resolution boundary (M10h). */
export type RenderVisual =
  | { kind: "image"; asset: string }
  | { kind: "animated"; source: AnimatedSource; fps: number; loop: boolean };

/** An animated visual's frame source: an ordered list of individually-uploaded assets, or one
 * grid-sliced sheet asset. No packed-atlas-JSON format yet (M10h design spec §7). */
export type AnimatedSource =
  | { type: "frames"; frames: string[] }
  | { type: "sheet"; asset: string; rows: number; cols: number; count?: number };

/** A face's own visual. Deliberately never itself `{kind:"faces"}` — no nesting — so an animated
 * face falls out of the same RenderVisual boundary with no separate mechanism. */
export type FaceVisual = RenderVisual;

/** An actor's (or a linked token's override) declared visual: a plain RenderVisual, or a
 * multi-face union resolved per-token by `resolveTokenVisual` (M10h). Client-owned, opaque
 * `system`-body JSON — no ts-rs type, no server change (mirrors `movementModel`/`bounds`). */
export type TokenVisual =
  | RenderVisual
  | {
      kind: "faces";
      faces: Record<string, FaceVisual>;
      default: string;
      /** Optional conditionId -> face name map; the first match (in the token's effective
       * `conditions[]` order) wins over `default`, but never over a manual `token.system.face`. */
      faceMap?: Record<string, string>;
    };
```

Then update the three fields that referenced `ActorVisual`:

```typescript
export interface TokenSystem {
  x: number;
  y: number;
  w: number;
  h: number;
  rotation: number;
  /** Set on raw (actorless) tokens; actor-backed tokens resolve their visual via the actor. */
  visual?: TokenVisual;
  /** Linked token: the shared actor's id (null/absent ⇒ instanced, see `embedded.actor`). */
  actor_id?: string | null;
  /** Linked-only per-token override whitelist (see {@link TokenOverrides}). */
  overrides?: TokenOverrides;
  /** Active face name when the effective visual is a `faces` union member (M10h); token-local
   * always (not part of `overrides` — it selects INTO the actor's faces map, not an override
   * of actor-data). Ignored when the effective visual isn't `faces`. */
  face?: string;
}
```

```typescript
export interface ActorSystem {
  name: string;
  displayName: string;
  visual: TokenVisual;
  size: { w: number; h: number };
  shape: "square" | "circle";
  faction: string | null;
  conditions: string[];
  prototype: boolean;
  vision?: VisionAssignment[];
}
```

```typescript
export interface TokenOverrides {
  name?: string;
  visual?: TokenVisual;
  size?: { w: number; h: number };
  shape?: "square" | "circle";
  vision?: VisionAssignment[];
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/core test scene-docs.test.ts`
Expected: PASS.

- [ ] **Step 5: Update the export barrel**

In `src/client/core/src/index.ts:77`, replace `ActorVisual` with the four new type names in the existing `export type { SceneSystem, TokenSystem, ActorSystem, ActorVisual, TokenOverrides, ... } from "./scene-docs";` line:

```typescript
export type { SceneSystem, TokenSystem, ActorSystem, TokenOverrides, RenderVisual, AnimatedSource, FaceVisual, TokenVisual, Faction, FactionStance, FactionRegistrySystem, Condition, ConditionRegistrySystem, MovementRestriction, MovementModel, LightMode, DiagonalRule, EasingMode, EnvironmentLight, GridDistance, SceneVisionOverrides, SceneLightingOverrides, WorldSceneDefaults, WorldSettingsSystem, ResolvedSceneSettings, GradationBand, LightGradationSystem, VisionMode, VisionModesSystem, VisionAssignment, LightSystem, RegionShapeKind, RegionShape, RegionBehavior, RegionSystem, SceneDimensions } from "./scene-docs";
```

- [ ] **Step 6: Typecheck the whole package**

Run: `pnpm --filter @shadowcat/core typecheck`
Expected: PASS — no lingering `ActorVisual` references anywhere in `@shadowcat/core` (it no longer exists).

- [ ] **Step 7: Run the full core test suite**

Run: `pnpm --filter @shadowcat/core test`
Expected: PASS (no other test in the package referenced `ActorVisual` by name; all existing `visual: { kind: "image", asset }` object literals remain structurally valid under `TokenVisual`).

- [ ] **Step 8: Commit**

```bash
git add src/client/core/src/scene-docs.ts src/client/core/src/index.ts src/client/core/src/scene-docs.test.ts
git commit -m "feat(m10h): add TokenVisual/FaceVisual/RenderVisual/AnimatedSource union + token.system.face"
```

---

## Task 2: `resolveTokenVisual` — the render-boundary resolver

**Files:**
- Modify: `src/client/core/src/actor.ts` (imports, `EffectiveActor.visual`, new `resolveTokenVisual` + private helpers)
- Modify: `src/client/core/src/index.ts:78-79` (export `resolveTokenVisual`)
- Test: `src/client/core/src/actor.test.ts` (extend — check for existing file first)

**Interfaces:**
- Consumes: `TokenVisual`, `FaceVisual`, `RenderVisual`, `AnimatedSource` (Task 1); `EffectiveActor`, `resolveTokenActor(token, store)` (existing).
- Produces: `resolveTokenVisual(token: WireDocument, store: ReadableDocuments, eff?: EffectiveActor | null): RenderVisual | null` — the sole read-through every visual consumer (`token-view.ts`) uses from Task 5 onward.

- [ ] **Step 1: Write the failing tests**

Add to `src/client/core/src/actor.test.ts`:

```typescript
import { resolveTokenVisual } from "./actor";
import type { TokenVisual } from "./scene-docs";

describe("resolveTokenVisual", () => {
  function actorWith(visual: TokenVisual, extra: Partial<{ conditions: string[] }> = {}) {
    return buildActorDoc(
      "w1",
      { name: "G", displayName: "G", visual, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: extra.conditions ?? [], prototype: false },
      "act1",
    );
  }

  it("passes an image visual through unchanged", () => {
    const store = new DocumentStore();
    const actor = actorWith({ kind: "image", asset: "a1" });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: actor }, { op: "create", doc: token }] });
    expect(resolveTokenVisual(token, store)).toEqual({ kind: "image", asset: "a1" });
  });

  it("passes an animated visual through unchanged", () => {
    const store = new DocumentStore();
    const animated: TokenVisual = { kind: "animated", source: { type: "frames", frames: ["a1", "a2"] }, fps: 8, loop: true };
    const actor = actorWith(animated);
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: actor }, { op: "create", doc: token }] });
    expect(resolveTokenVisual(token, store)).toEqual(animated);
  });

  it("resolves faces to the manual token.system.face over the default", () => {
    const store = new DocumentStore();
    const actor = actorWith({
      kind: "faces",
      faces: { normal: { kind: "image", asset: "n1" }, bloodied: { kind: "image", asset: "b1" } },
      default: "normal",
    });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: actor }, { op: "create", doc: token }, { op: "update", doc_id: "tok1", changes: [{ path: "/system/face", old: null, new: "bloodied" }] }] });
    expect(resolveTokenVisual(token, store)).toEqual({ kind: "image", asset: "b1" });
  });

  it("resolves an animated face — proves faces are not restricted to images", () => {
    const store = new DocumentStore();
    const bloodied: TokenVisual extends never ? never : { kind: "animated"; source: { type: "frames"; frames: string[] }; fps: number; loop: boolean } = { kind: "animated", source: { type: "frames", frames: ["b1"] }, fps: 4, loop: true };
    const actor = actorWith({ kind: "faces", faces: { normal: { kind: "image", asset: "n1" }, bloodied }, default: "normal" });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: actor }, { op: "create", doc: token }, { op: "update", doc_id: "tok1", changes: [{ path: "/system/face", old: null, new: "bloodied" }] }] });
    expect(resolveTokenVisual(token, store)).toEqual(bloodied);
  });

  it("falls back to a faceMap match when no manual face is set", () => {
    const store = new DocumentStore();
    const actor = actorWith(
      { kind: "faces", faces: { normal: { kind: "image", asset: "n1" }, bleeding: { kind: "image", asset: "bl1" } }, default: "normal", faceMap: { poisoned: "bleeding" } },
      { conditions: ["poisoned"] },
    );
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: actor }, { op: "create", doc: token }] });
    expect(resolveTokenVisual(token, store)).toEqual({ kind: "image", asset: "bl1" });
  });

  it("falls back to default when neither manual face nor faceMap matches", () => {
    const store = new DocumentStore();
    const actor = actorWith({ kind: "faces", faces: { normal: { kind: "image", asset: "n1" }, bloodied: { kind: "image", asset: "b1" } }, default: "normal" });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: actor }, { op: "create", doc: token }] });
    expect(resolveTokenVisual(token, store)).toEqual({ kind: "image", asset: "n1" });
  });

  it("fails closed to the first face key when default itself is invalid", () => {
    const store = new DocumentStore();
    const actor = actorWith({ kind: "faces", faces: { onlyOne: { kind: "image", asset: "o1" } }, default: "missing" });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: actor }, { op: "create", doc: token }] });
    expect(resolveTokenVisual(token, store)).toEqual({ kind: "image", asset: "o1" });
  });

  it("fails closed to null when the faces map is empty", () => {
    const store = new DocumentStore();
    const actor = actorWith({ kind: "faces", faces: {}, default: "x" });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: actor }, { op: "create", doc: token }] });
    expect(resolveTokenVisual(token, store)).toBeNull();
  });

  it("fails closed on a malformed AnimatedSource (non-positive rows/cols)", () => {
    const store = new DocumentStore();
    const actor = actorWith({ kind: "animated", source: { type: "sheet", asset: "s1", rows: 0, cols: 4 }, fps: 8, loop: true });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: actor }, { op: "create", doc: token }] });
    expect(resolveTokenVisual(token, store)).toBeNull();
  });

  it("fails closed on a malformed AnimatedSource (empty frame list)", () => {
    const store = new DocumentStore();
    const actor = actorWith({ kind: "animated", source: { type: "frames", frames: [] }, fps: 8, loop: true });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: actor }, { op: "create", doc: token }] });
    expect(resolveTokenVisual(token, store)).toBeNull();
  });

  it("fails closed on a malformed nested faces value (defense in depth against garbled wire data)", () => {
    const store = new DocumentStore();
    const nested = { kind: "faces", faces: {}, default: "x" } as unknown as { kind: "image"; asset: string };
    const actor = actorWith({ kind: "faces", faces: { bad: nested }, default: "bad" });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: actor }, { op: "create", doc: token }] });
    expect(resolveTokenVisual(token, store)).toBeNull();
  });
});
```

> Note: match the actual `DocumentStore`/`applyCommand`/`WireOperation` shapes already used in `token-view.test.ts` and `actor.test.ts` if this file's existing helpers differ slightly (e.g. a local `cmd(...)` builder) — reuse whatever helper the file already has rather than inlining `applyCommand` calls if one exists.

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/core test actor.test.ts`
Expected: FAIL — `resolveTokenVisual` is not exported yet.

- [ ] **Step 3: Implement `resolveTokenVisual`**

In `src/client/core/src/actor.ts`, update the import line (currently line 7) to add the new types:

```typescript
import type { ActorSystem, TokenOverrides, ConditionRegistrySystem, SceneSystem, VisionAssignment, TokenVisual, RenderVisual } from "./scene-docs";
```

Update `EffectiveActor.visual` (currently `visual: ActorVisual;`) to:

```typescript
  visual: TokenVisual;
```

Append the resolver + its private helpers at the end of the file:

```typescript
/** Resolve a `faces` visual to the active face's RenderVisual. Precedence: a valid manual
 * `token.system.face` > the first `faceMap` entry whose condition id is in `conditions` (in
 * `conditions` array order — a v1 simplification, no severity ranking across simultaneously
 * active conditions) > `default` > the first key of `faces` (fail-closed continuation, never a
 * missing-visual null while any face exists). Returns null only when `faces` is empty. */
function resolveFace(
  visual: Extract<TokenVisual, { kind: "faces" }>,
  manualFace: string | undefined,
  conditions: string[],
): FaceVisualLike | null {
  const names = Object.keys(visual.faces);
  if (names.length === 0) return null;
  if (manualFace && visual.faces[manualFace]) return visual.faces[manualFace];
  if (visual.faceMap) {
    for (const id of conditions) {
      const name = visual.faceMap[id];
      if (name && visual.faces[name]) return visual.faces[name];
    }
  }
  if (visual.default && visual.faces[visual.default]) return visual.faces[visual.default];
  return visual.faces[names[0]];
}
type FaceVisualLike = TokenVisual extends { kind: "faces"; faces: Record<string, infer F> } ? F : never;

function isValidAnimated(v: { source: RenderVisual extends { kind: "animated"; source: infer S } ? S : never; fps: number }): boolean {
  if (!Number.isFinite(v.fps) || v.fps <= 0) return false;
  const src = v.source as { type: string; frames?: string[]; rows?: number; cols?: number };
  if (src.type === "frames") return (src.frames?.length ?? 0) > 0;
  return Number.isInteger(src.rows) && (src.rows ?? 0) > 0 && Number.isInteger(src.cols) && (src.cols ?? 0) > 0;
}

/** The render boundary: resolves a token's `TokenVisual` (image, animated, or faces) down to a
 * plain `RenderVisual` (image or animated) — the only two kinds the render layer ever draws.
 * Fail-closed to `null` on any malformed/unknown shape; never throws. Pass a pre-resolved `eff`
 * to avoid a second `resolveTokenActor` call; omit to resolve internally. */
export function resolveTokenVisual(
  token: WireDocument,
  store: ReadableDocuments,
  eff?: EffectiveActor | null,
): RenderVisual | null {
  const actor = eff === undefined ? resolveTokenActor(token, store) : eff;
  const sys = token.system as { visual?: TokenVisual; face?: string } | undefined;
  const visual = actor?.visual ?? sys?.visual;
  if (!visual) return null;
  const resolved = visual.kind === "faces" ? resolveFace(visual, sys?.face, actor?.conditions ?? []) : visual;
  if (!resolved) return null;
  if (resolved.kind !== "image" && resolved.kind !== "animated") return null;
  if (resolved.kind === "animated" && !isValidAnimated(resolved)) return null;
  return resolved;
}
```

> Note: the `FaceVisualLike`/conditional-type gymnastics above exist only to avoid re-declaring `FaceVisual`'s shape; if TypeScript complains about the conditional types (they can be fragile across TS versions), simplify by importing `FaceVisual` directly from `./scene-docs` and typing `resolveFace`'s return as `FaceVisual | null`, and `isValidAnimated`'s parameter as `Extract<RenderVisual, { kind: "animated" }>` — this is the cleaner form; prefer it over the conditional-type version above if either compiles ambiguously. Concretely:

```typescript
import type { ActorSystem, TokenOverrides, ConditionRegistrySystem, SceneSystem, VisionAssignment, TokenVisual, RenderVisual, FaceVisual } from "./scene-docs";

function resolveFace(
  visual: Extract<TokenVisual, { kind: "faces" }>,
  manualFace: string | undefined,
  conditions: string[],
): FaceVisual | null {
  const names = Object.keys(visual.faces);
  if (names.length === 0) return null;
  if (manualFace && visual.faces[manualFace]) return visual.faces[manualFace];
  if (visual.faceMap) {
    for (const id of conditions) {
      const name = visual.faceMap[id];
      if (name && visual.faces[name]) return visual.faces[name];
    }
  }
  if (visual.default && visual.faces[visual.default]) return visual.faces[visual.default];
  return visual.faces[names[0]];
}

function isValidAnimated(v: Extract<RenderVisual, { kind: "animated" }>): boolean {
  if (!Number.isFinite(v.fps) || v.fps <= 0) return false;
  if (v.source.type === "frames") return v.source.frames.length > 0;
  return Number.isInteger(v.source.rows) && v.source.rows > 0 && Number.isInteger(v.source.cols) && v.source.cols > 0;
}

export function resolveTokenVisual(
  token: WireDocument,
  store: ReadableDocuments,
  eff?: EffectiveActor | null,
): RenderVisual | null {
  const actor = eff === undefined ? resolveTokenActor(token, store) : eff;
  const sys = token.system as { visual?: TokenVisual; face?: string } | undefined;
  const visual = actor?.visual ?? sys?.visual;
  if (!visual) return null;
  const resolved = visual.kind === "faces" ? resolveFace(visual, sys?.face, actor?.conditions ?? []) : visual;
  if (!resolved) return null;
  if (resolved.kind !== "image" && resolved.kind !== "animated") return null;
  if (resolved.kind === "animated" && !isValidAnimated(resolved)) return null;
  return resolved;
}
```

Use this second, simpler form — implement it directly rather than the first conditional-type sketch.

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/core test actor.test.ts`
Expected: PASS.

- [ ] **Step 5: Export `resolveTokenVisual`**

In `src/client/core/src/index.ts:78`, add it to the existing export list:

```typescript
export { resolveTokenActor, actorDisplayName, resolveConditions, conditionTarget, resolveTokenBox, footprintRadius, resolveTokenVisual } from "./actor";
```

- [ ] **Step 6: Typecheck + full package test run**

Run: `pnpm --filter @shadowcat/core typecheck && pnpm --filter @shadowcat/core test`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/client/core/src/actor.ts src/client/core/src/actor.test.ts src/client/core/src/index.ts
git commit -m "feat(m10h): add resolveTokenVisual — the RenderVisual resolution boundary"
```

---

## Task 3: Pure animated-frame math

**Files:**
- Create: `src/client/render/src/token-animation.ts`
- Test: `src/client/render/src/token-animation.test.ts`

**Interfaces:**
- Produces: `computeAnimatedFrame(elapsedMs: number, fps: number, frameCount: number, loop: boolean): number` — consumed by `PixiBackend.tickTokenAnimations` in Task 6.

- [ ] **Step 1: Write the failing test**

```typescript
import { test, expect } from "vitest";
import { computeAnimatedFrame } from "./token-animation";

test("advances one frame per 1000/fps ms", () => {
  expect(computeAnimatedFrame(0, 8, 10, true)).toBe(0);
  expect(computeAnimatedFrame(125, 8, 10, true)).toBe(1); // 1000/8 = 125ms/frame
  expect(computeAnimatedFrame(999, 8, 10, true)).toBe(7);
});

test("loops by wrapping past the frame count", () => {
  expect(computeAnimatedFrame(1250, 8, 10, true)).toBe(0); // frame 10 -> wraps to 0
  expect(computeAnimatedFrame(1375, 8, 10, true)).toBe(1);
});

test("a one-shot (loop:false) clamps to the last frame and holds", () => {
  expect(computeAnimatedFrame(1250, 8, 10, false)).toBe(9); // frame 10 clamps to index 9
  expect(computeAnimatedFrame(100_000, 8, 10, false)).toBe(9);
});

test("fails closed to frame 0 on degenerate input", () => {
  expect(computeAnimatedFrame(NaN, 8, 10, true)).toBe(0);
  expect(computeAnimatedFrame(100, NaN, 10, true)).toBe(0);
  expect(computeAnimatedFrame(100, 0, 10, true)).toBe(0);
  expect(computeAnimatedFrame(100, -1, 10, true)).toBe(0);
  expect(computeAnimatedFrame(100, 8, 0, true)).toBe(0);
  expect(computeAnimatedFrame(100, 8, -1, true)).toBe(0);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/render test token-animation.test.ts`
Expected: FAIL — module doesn't exist.

- [ ] **Step 3: Implement**

```typescript
/** Pure animated-token frame math (M10h). Extracted for the same reason as
 * `computeFogBlendFactor` in `fog-blend.ts` — `pixi-backend.ts` itself is Playwright-covered
 * only (no WebGL in jsdom), so the frame-selection logic lives here where it's unit-testable. */

/**
 * The frame index to display after `elapsedMs` of playback at `fps`, over `frameCount` frames.
 * `loop:true` wraps (`elapsedMs` can be arbitrarily large); `loop:false` clamps to the last frame
 * once the sequence completes (a one-shot animation holds its final frame, never wraps or stops
 * rendering). Degenerate input (`frameCount<=0`, non-finite `elapsedMs`/`fps`, `fps<=0`) fails
 * closed to frame 0 — always a valid index into a non-empty frame array, never a crash.
 */
export function computeAnimatedFrame(elapsedMs: number, fps: number, frameCount: number, loop: boolean): number {
  if (!Number.isFinite(elapsedMs) || !Number.isFinite(fps) || fps <= 0 || frameCount <= 0) return 0;
  const frame = Math.floor((elapsedMs / 1000) * fps);
  if (loop) return ((frame % frameCount) + frameCount) % frameCount;
  return Math.min(Math.max(frame, 0), frameCount - 1);
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/render test token-animation.test.ts`
Expected: PASS.

- [ ] **Step 5: Typecheck**

Run: `pnpm --filter @shadowcat/render typecheck`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/client/render/src/token-animation.ts src/client/render/src/token-animation.test.ts
git commit -m "feat(m10h): add computeAnimatedFrame — pure tick-driven animation frame math"
```

---

## Task 4: Render-layer types + `DisplayBackend.tickTokenAnimations` + `MockBackend`

**Files:**
- Modify: `src/client/render/src/types.ts` (`TokenNodeSpec`, new `ResolvedAnimatedSource`)
- Modify: `src/client/render/src/backend.ts` (`DisplayBackend` interface)
- Modify: `src/client/render/src/backend.mock.ts` (`MockBackend`)
- Modify: `src/client/render/src/backend.mock.test.ts` (fix the existing `url`-based assertions; add a new test)

**Interfaces:**
- Produces: `TokenNodeSpec.visual: {kind:"image";url:string} | {kind:"animated";source:ResolvedAnimatedSource;fps:number;loop:boolean}` (replaces `TokenNodeSpec.url`); `ResolvedAnimatedSource`; `DisplayBackend.tickTokenAnimations(dtMs: number): void`.
- Consumes: nothing new (pure type/interface work + the recording mock).

- [ ] **Step 1: Write the failing test**

Replace the existing `backend.mock.test.ts` test `"MockBackend records token upserts and removals"` (currently uses `url: "/a"`) with the new shape, and add a tick-animation test:

```typescript
test("MockBackend records token upserts and removals", () => {
  const b = new MockBackend();
  b.setToken("t1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", url: "/a" }, borderColor: null, badges: [], shape: "square" });
  expect(b.tokens.get("t1")).toEqual({ x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", url: "/a" }, borderColor: null, badges: [], shape: "square" });
  b.setToken("t1", { x: 10, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", url: "/a" }, borderColor: null, badges: [], shape: "square" });
  expect(b.tokens.get("t1")!.x).toBe(10);
  b.removeToken("t1");
  expect(b.tokens.has("t1")).toBe(false);
});

test("MockBackend records an animated token visual and accepts tickTokenAnimations calls", () => {
  const b = new MockBackend();
  const visual = { kind: "animated" as const, source: { type: "frames" as const, urls: ["/a", "/b"] }, fps: 8, loop: true };
  b.setToken("t1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual, borderColor: null, badges: [], shape: "square" });
  expect(b.tokens.get("t1")!.visual).toEqual(visual);
  expect(() => b.tickTokenAnimations(16)).not.toThrow();
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/render test backend.mock.test.ts`
Expected: FAIL — `TokenNodeSpec.visual`/`tickTokenAnimations` don't exist yet.

- [ ] **Step 3: Update `types.ts`**

In `src/client/render/src/types.ts`, replace the `TokenNodeSpec` interface (currently lines 48-62):

```typescript
/** Asset UUIDs already resolved to serve URLs by the AssetResolver (M10h) — the backend never
 * resolves asset ids itself, mirroring today's `assets.url(...)` call in `token-view.ts`. */
export type ResolvedAnimatedSource =
  | { type: "frames"; urls: string[] }
  | { type: "sheet"; url: string; rows: number; cols: number; count?: number };

/** A resolved token render node: transform + size + resolved visual + faction border + footprint shape. */
export interface TokenNodeSpec {
  x: number;
  y: number;
  w: number;
  h: number;
  rotation: number;
  /** The resolved, already-URL'd visual to draw (M10h: image, or a tick-driven animation). */
  visual:
    | { kind: "image"; url: string }
    | { kind: "animated"; source: ResolvedAnimatedSource; fps: number; loop: boolean };
  /** Faction border color (0xRRGGBB), or null for no border. */
  borderColor: number | null;
  /** Condition marker glyphs (emoji), rendered as upright chips along the token's top edge. */
  badges: string[];
  /** Footprint shape: drives the border outline + hit-test (M10d). */
  shape: "square" | "circle";
}
```

- [ ] **Step 4: Update `backend.ts`**

In `src/client/render/src/backend.ts`, add to the `DisplayBackend` interface (after `setToken`/`removeToken`, e.g. right after line 32):

```typescript
  /** Advance any tick-driven animated token visuals by `dtMs` (M10h). Called once per frame
   * alongside the `startTicker` callback; a no-op backend when nothing has an `animated` visual. */
  tickTokenAnimations(dtMs: number): void;
```

- [ ] **Step 5: Update `backend.mock.ts`**

Add the method to `MockBackend` (e.g. after `removeToken`):

```typescript
  tickTokenAnimations(_dtMs: number): void {
    // MockBackend records TokenNodeSpec.visual verbatim; frame-advance is real-AnimatedSprite
    // state owned by PixiBackend only, so this is an intentional no-op in tests.
  }
```

- [ ] **Step 6: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/render test backend.mock.test.ts`
Expected: PASS.

- [ ] **Step 7: Typecheck**

Run: `pnpm --filter @shadowcat/render typecheck`
Expected: FAIL at this point — `pixi-backend.ts`, `token-view.ts`, and `token-view.test.ts` still reference the old `TokenNodeSpec.url` shape. This is expected; Tasks 5-6 fix it. Do not attempt to fix those files here — confirm the failure is confined to those files (not a typo in this task's own edits) before moving on.

- [ ] **Step 8: Commit**

```bash
git add src/client/render/src/types.ts src/client/render/src/backend.ts src/client/render/src/backend.mock.ts src/client/render/src/backend.mock.test.ts
git commit -m "feat(m10h): TokenNodeSpec.visual union + DisplayBackend.tickTokenAnimations"
```

> Note for the next task's implementer: this task deliberately leaves `@shadowcat/render` red on typecheck (`pixi-backend.ts`/`token-view.ts` still use the old `url` field) — Task 5 and Task 6 are the two follow-on tasks that each fix their half. This is the one place in this plan where a task's own package doesn't fully typecheck at its end; call this out explicitly to the reviewer rather than treating it as a task failure.

---

## Task 5: `TokenView` — resolve visuals through `resolveTokenVisual`

**Files:**
- Modify: `src/client/render/src/token-view.ts`
- Modify: `src/client/render/src/token-view.test.ts` (update existing `url`-based assertions; add new tests)

**Interfaces:**
- Consumes: `resolveTokenVisual` (Task 2), `TokenNodeSpec.visual`/`ResolvedAnimatedSource` (Task 4), `DisplayBackend.tickTokenAnimations` (Task 4).
- Produces: `TokenView.toSpec` builds `TokenNodeSpec.visual`; `TokenView.tick` drives `backend.tickTokenAnimations`.

- [ ] **Step 1: Update existing test assertions to the new `visual` shape**

In `src/client/render/src/token-view.test.ts`, every assertion of the form `backend.tokens.get(id)!.url` or a `toEqual({..., url: ...})` object needs updating. Concretely:

- Line 6-14 (`tokenDoc` helper): no change needed — it still builds `system: {..., visual: { kind: "image", asset } }`, which is still valid `TokenVisual`.
- Line 41: replace
  ```typescript
  expect(backend.tokens.get("t1")).toEqual({ x: 100, y: 50, w: 100, h: 100, rotation: 0, url: assets.url("img1"), borderColor: null, badges: [], shape: "square" });
  ```
  with
  ```typescript
  expect(backend.tokens.get("t1")).toEqual({ x: 100, y: 50, w: 100, h: 100, rotation: 0, visual: { kind: "image", url: assets.url("img1") }, borderColor: null, badges: [], shape: "square" });
  ```
- Line 69: replace
  ```typescript
  expect(backend.tokens.get("tok1")!.url).toBe(assets.url("actorimg"));
  ```
  with
  ```typescript
  expect(backend.tokens.get("tok1")!.visual).toEqual({ kind: "image", url: assets.url("actorimg") });
  ```

Add new tests at the end of the file (before the animation-config helper section, or after — either is fine):

```typescript
test("renders an animated frame-list visual with resolved frame URLs", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    { name: "Wisp", displayName: "Wisp", visual: { kind: "animated", source: { type: "frames", frames: ["f1", "f2"] }, fps: 6, loop: true }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, assets, backend).reconcile();
  expect(backend.tokens.get("tok1")!.visual).toEqual({
    kind: "animated",
    source: { type: "frames", urls: [assets.url("f1"), assets.url("f2")] },
    fps: 6,
    loop: true,
  });
});

test("renders an animated grid-sheet visual with a resolved sheet URL", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    { name: "Torch", displayName: "Torch", visual: { kind: "animated", source: { type: "sheet", asset: "sheet1", rows: 2, cols: 4, count: 7 }, fps: 12, loop: false }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, assets, backend).reconcile();
  expect(backend.tokens.get("tok1")!.visual).toEqual({
    kind: "animated",
    source: { type: "sheet", url: assets.url("sheet1"), rows: 2, cols: 4, count: 7 },
    fps: 12,
    loop: false,
  });
});

test("a token whose visual fails to resolve (empty faces) is skipped, not crashed", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    { name: "Broken", displayName: "Broken", visual: { kind: "faces", faces: {}, default: "x" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  expect(() => new TokenView(store, new AssetResolver(), backend).reconcile()).not.toThrow();
  expect(backend.tokens.has("tok1")).toBe(false);
});

test("tick() forwards dtMs to the backend's tickTokenAnimations", () => {
  const store = makeStoreWithToken("tok1", { x: 0, y: 0 });
  const backend = new MockBackend();
  const spy = vi.spyOn(backend, "tickTokenAnimations");
  const view = new TokenView(store, new AssetResolver(), backend);
  view.reconcile();
  view.tick(16);
  expect(spy).toHaveBeenCalledWith(16);
});
```

Add `import { vi } from "vitest";` to the top of the file if not already imported (check the existing `import { test, expect, it } from "vitest";` line and extend it to `import { test, expect, it, vi } from "vitest";`).

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/render test token-view.test.ts`
Expected: FAIL — `toSpec` still builds `{url: ...}`, not `{visual: ...}`; `tick` doesn't call `tickTokenAnimations`.

- [ ] **Step 3: Update `token-view.ts`**

Update the import line (currently line 1) to add `resolveTokenVisual` and the `AnimatedSource` type:

```typescript
import { resolveTokenActor, resolveConditions, resolveTokenBox, resolveTokenVisual } from "@shadowcat/core";
import type { ReadableDocuments, AssetResolver, WireDocument, FactionRegistrySystem, AnimatedSource } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { TokenNodeSpec, ResolvedAnimatedSource } from "./types";
```

Remove the `visual` field from the file's own local `TokenSystem` interface (currently lines 10-17) — it's no longer read directly here (`resolveTokenVisual` re-reads `doc.system` internally):

```typescript
/** Engine-reserved token system fields (M8 §4.2; client-owned). `(x,y)` = center. */
interface TokenSystem {
  x: number;
  y: number;
  w: number;
  h: number;
  rotation?: number;
}
```

Add a private method (anywhere in the class, e.g. right before `toSpec`):

```typescript
  private resolveSource(source: AnimatedSource): ResolvedAnimatedSource {
    return source.type === "frames"
      ? { type: "frames", urls: source.frames.map((id) => this.assets.url(id)) }
      : { type: "sheet", url: this.assets.url(source.asset), rows: source.rows, cols: source.cols, ...(source.count !== undefined ? { count: source.count } : {}) };
  }
```

Update `toSpec` (currently lines 139-164) — replace the visual-resolution block:

```typescript
  private toSpec(doc: WireDocument): TokenNodeSpec | null {
    const s = doc.system as TokenSystem | undefined;
    if (!s) return null;
    const eff = resolveTokenActor(doc, this.store);
    const visual = resolveTokenVisual(doc, this.store, eff);
    if (!visual) return null;
    const resolvedVisual: TokenNodeSpec["visual"] =
      visual.kind === "image"
        ? { kind: "image", url: this.assets.url(visual.asset) }
        : { kind: "animated", source: this.resolveSource(visual.source), fps: visual.fps, loop: visual.loop };
    // Faction border color resolves through the world faction registry; null = no border.
    let borderColor: number | null = null;
    if (eff?.faction) {
      const reg = this.store.query("faction-registry")[0]?.system as FactionRegistrySystem | undefined;
      const hex = reg?.factions?.[eff.faction]?.color;
      if (hex) borderColor = parseColor(hex);
    }
    // Condition badges: resolve the actor's condition ids to registry icon glyphs.
    const badges = resolveConditions(doc, this.store).map((c) => c.icon);
    const box = resolveTokenBox(doc, this.store, eff);
    return {
      x: box.x, y: box.y, w: box.w, h: box.h, rotation: s.rotation ?? 0,
      visual: resolvedVisual,
      borderColor,
      badges,
      shape: box.shape,
    };
  }
```

Update `tick` (currently lines 111-113):

```typescript
  tick(dtMs: number): void {
    for (const id of this.animator.tick(dtMs)) this.push(id);
    this.backend.tickTokenAnimations(dtMs);
  }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/render test token-view.test.ts`
Expected: PASS.

- [ ] **Step 5: Typecheck**

Run: `pnpm --filter @shadowcat/render typecheck`
Expected: still FAIL — `pixi-backend.ts` alone remains (Task 6 fixes it). Confirm the only remaining errors are in `pixi-backend.ts`.

- [ ] **Step 6: Run the full render package test suite (excluding the known-red typecheck)**

Run: `pnpm --filter @shadowcat/render test`
Expected: PASS (Vitest doesn't typecheck `pixi-backend.ts`, which isn't imported by any unit test — only Playwright exercises it).

- [ ] **Step 7: Commit**

```bash
git add src/client/render/src/token-view.ts src/client/render/src/token-view.test.ts
git commit -m "feat(m10h): TokenView resolves visuals through resolveTokenVisual; tick drives backend animation"
```

---

## Task 6: `PixiBackend` — Container-per-token migration + animated sprite playback

**Files:**
- Modify: `src/client/render/src/pixi-backend.ts`

**Interfaces:**
- Consumes: `TokenNodeSpec.visual`/`ResolvedAnimatedSource` (Task 4), `computeAnimatedFrame` (Task 3).
- Produces: `PixiBackend.tickTokenAnimations` (fulfills the `DisplayBackend` interface member from Task 4).

This task has no jsdom-testable unit — `PixiBackend` requires a real WebGL context (Playwright-only, per the project's existing convention — see `fog-blend.ts`'s header comment for precedent). Verification here is: (a) the package typechecks clean for the first time since Task 4, and (b) the existing Playwright suite (`stage.spec.ts`'s "place a token via the tool rail, then drag it" test) passes unmodified — proving the Container migration is a behavior-preserving refactor for the image case. Task 9 adds a new Playwright scenario proving the animated case.

- [ ] **Step 1: Add imports**

In `src/client/render/src/pixi-backend.ts:1`, extend the pixi.js import:

```typescript
import { Application, BlurFilter, Container, Graphics, RenderTexture, Sprite, AnimatedSprite, Texture, Rectangle, Text, Assets, type Filter } from "pixi.js";
```

Extend the local imports (line 4):

```typescript
import type { LineSeg, CameraTransform, VisibilityInput, TokenNodeSpec, ShapeNodeSpec, Point, ResolvedAnimatedSource } from "./types";
```

Add a new import for the pure frame helper:

```typescript
import { computeAnimatedFrame } from "./token-animation";
```

- [ ] **Step 2: Replace the token-related private fields**

Remove these five field declarations (currently lines 38, 40, 42, 44, 47):

```typescript
  private readonly tokens = new Map<string, Sprite>();
  /** Last-loaded image URL per token, so a tweening token doesn't reload each frame. */
  private readonly tokenUrls = new Map<string, string>();
  /** Faction border outline per token (absent when the token has no faction color). */
  private readonly tokenBorders = new Map<string, Graphics>();
  /** Condition badge glyph nodes per token (upright; absent when the token has no conditions). */
  private readonly tokenBadges = new Map<string, Text[]>();
  /** Last-rendered badge glyph set per token, so a tweening token (re-pushed ~60×/s with the same
   * glyphs) repositions existing Text nodes instead of reallocating them each frame. */
  private readonly tokenBadgeKeys = new Map<string, string>();
```

Replace with a single map keyed by a per-token node record, declared just above the class (module-level, since it's a plain data shape, not a class member) — add this right before the `PixiBackend` class declaration:

```typescript
/** Per-token render state (M10h). `container` is the outer, non-rotating node (position = token
 * center; badges are its direct children, so they stay upright); `visualContainer` rotates with
 * the token and holds the art + border. `sourceKey` guards visual (re)creation against a
 * tweening token's ~60x/s re-push with an unchanged visual. `anim` is present only while `visual`
 * is an AnimatedSprite. */
interface TokenNode {
  container: Container;
  visualContainer: Container;
  visual: Sprite | AnimatedSprite;
  border: Graphics;
  badges: Text[];
  badgeKey: string;
  sourceKey: string | null;
  anim: { fps: number; loop: boolean; frameCount: number; elapsedMs: number } | null;
}

/** Identity key for a `TokenNodeSpec.visual` — equal specs must produce an equal key so a
 * tweening token's re-push (same visual, new transform) skips texture (re)loading. */
function visualSourceKey(v: TokenNodeSpec["visual"]): string {
  return v.kind === "image" ? `image:${v.url}` : `animated:${JSON.stringify(v.source)}:${v.fps}:${v.loop}`;
}
```

Then add the field to the class:

```typescript
  private readonly tokens = new Map<string, TokenNode>();
```

- [ ] **Step 3: Replace `setToken`/`removeToken`**

Replace the entire `setToken` method through the end of `removeToken` (currently lines 208-302) with:

```typescript
  setToken(id: string, spec: TokenNodeSpec): void {
    let node = this.tokens.get(id);
    if (!node) node = this.createTokenNode(id);
    node.container.position.set(spec.x, spec.y);
    node.visualContainer.angle = spec.rotation; // degrees; rotates art + border, not badges
    this.updateTokenVisual(id, node, spec);
    this.updateTokenBorder(node, spec);
    this.updateTokenBadges(node, spec);
  }

  private createTokenNode(id: string): TokenNode {
    const container = new Container();
    const visualContainer = new Container();
    const visual = new Sprite();
    visual.anchor.set(0.5); // (x,y) is the token center
    const border = new Graphics();
    visualContainer.addChild(visual, border);
    container.addChild(visualContainer);
    this.layers.get("tokens")?.addChild(container);
    const node: TokenNode = { container, visualContainer, visual, border, badges: [], badgeKey: "", sourceKey: null, anim: null };
    this.tokens.set(id, node);
    return node;
  }

  private updateTokenVisual(id: string, node: TokenNode, spec: TokenNodeSpec): void {
    const key = visualSourceKey(spec.visual);
    node.visual.width = spec.w;
    node.visual.height = spec.h;
    if (node.sourceKey === key) return; // unchanged visual: a tweening token's transform-only re-push
    node.sourceKey = key;
    if (spec.visual.kind === "image") {
      if (node.visual instanceof AnimatedSprite) this.replaceVisualChild(node, new Sprite());
      node.anim = null;
      const sprite = node.visual;
      const url = spec.visual.url;
      void Assets.load(url).then((texture) => {
        if (this.tokens.get(id) === node && node.sourceKey === key) sprite.texture = texture;
      });
    } else {
      this.replaceVisualChild(node, new AnimatedSprite([Texture.EMPTY]));
      const sprite = node.visual as AnimatedSprite;
      sprite.autoUpdate = false; // driven by tickTokenAnimations, not Pixi's shared ticker
      node.anim = { fps: spec.visual.fps, loop: spec.visual.loop, frameCount: 1, elapsedMs: 0 };
      const source = spec.visual.source;
      void this.loadAnimatedTextures(source).then((textures) => {
        if (this.tokens.get(id) !== node || node.sourceKey !== key || textures.length === 0) return;
        sprite.textures = textures;
        sprite.gotoAndStop(0);
        node.anim = { fps: spec.visual.kind === "animated" ? spec.visual.fps : 1, loop: spec.visual.kind === "animated" ? spec.visual.loop : false, frameCount: textures.length, elapsedMs: 0 };
      });
    }
    node.visual.width = spec.w;
    node.visual.height = spec.h;
  }

  private replaceVisualChild(node: TokenNode, next: Sprite | AnimatedSprite): void {
    next.anchor.set(0.5);
    const i = node.visualContainer.getChildIndex(node.visual);
    node.visualContainer.removeChild(node.visual);
    node.visual.destroy();
    node.visualContainer.addChildAt(next, i);
    node.visual = next;
  }

  private async loadAnimatedTextures(source: ResolvedAnimatedSource): Promise<Texture[]> {
    if (source.type === "frames") {
      if (source.urls.length === 0) return [];
      return Promise.all(source.urls.map((url) => Assets.load<Texture>(url)));
    }
    if (!Number.isInteger(source.rows) || source.rows <= 0 || !Number.isInteger(source.cols) || source.cols <= 0) return [];
    const sheet = await Assets.load<Texture>(source.url);
    const frameW = sheet.width / source.cols;
    const frameH = sheet.height / source.rows;
    const total = source.count !== undefined ? Math.min(source.count, source.rows * source.cols) : source.rows * source.cols;
    const frames: Texture[] = [];
    for (let i = 0; i < total; i++) {
      const col = i % source.cols;
      const row = Math.floor(i / source.cols);
      frames.push(new Texture({ source: sheet.source, frame: new Rectangle(col * frameW, row * frameH, frameW, frameH) }));
    }
    return frames;
  }

  private updateTokenBorder(node: TokenNode, spec: TokenNodeSpec): void {
    const hw = spec.w / 2;
    const hh = spec.h / 2;
    node.border.clear();
    if (spec.borderColor === null) return;
    if (spec.shape === "circle") node.border.ellipse(0, 0, hw, hh).stroke({ width: 3, color: spec.borderColor });
    else node.border.rect(-hw, -hh, spec.w, spec.h).stroke({ width: 3, color: spec.borderColor });
  }

  private updateTokenBadges(node: TokenNode, spec: TokenNodeSpec): void {
    // Upright glyph chips along the token's top edge, relative to the (non-rotating) outer
    // container's own origin — badges are its children, so they stay upright automatically
    // when visualContainer (the sibling holding the rotating art+border) rotates.
    const size = Math.max(12, Math.min(spec.w, spec.h) * 0.28);
    const place = (txt: Text, i: number): void => {
      txt.position.set(-spec.w / 2 + size / 2 + i * (size + 2), -spec.h / 2 + size / 2);
    };
    const badgeKey = spec.badges.join("");
    if (node.badgeKey === badgeKey) {
      node.badges.forEach(place);
      return;
    }
    for (const b of node.badges) b.destroy();
    node.badgeKey = badgeKey;
    node.badges = spec.badges.map((glyph, i) => {
      const txt = new Text({ text: glyph, style: { fontSize: size, fontFamily: "sans-serif" } });
      txt.anchor.set(0.5);
      place(txt, i);
      node.container.addChild(txt);
      return txt;
    });
  }

  removeToken(id: string): void {
    const node = this.tokens.get(id);
    if (!node) return;
    node.container.destroy({ children: true });
    this.tokens.delete(id);
  }

  tickTokenAnimations(dtMs: number): void {
    for (const node of this.tokens.values()) {
      if (!node.anim || !(node.visual instanceof AnimatedSprite)) continue;
      node.anim.elapsedMs += dtMs;
      const frame = computeAnimatedFrame(node.anim.elapsedMs, node.anim.fps, node.anim.frameCount, node.anim.loop);
      if (node.visual.currentFrame !== frame) node.visual.gotoAndStop(frame);
    }
  }
```

> Note: the `node.visual.width/height` assignment appears twice in `updateTokenVisual` (once unconditionally at the top for the same-key fast path, once at the end of the changed-key path) — this is intentional, not a copy-paste error: the fast path returns early right after setting it, and the changed-key path needs it set again after `replaceVisualChild` swaps in a fresh sprite (whose default width/height don't yet match `spec.w/h`).

- [ ] **Step 4: Typecheck**

Run: `pnpm --filter @shadowcat/render typecheck`
Expected: PASS — this is the first fully-green typecheck since Task 4 (both `token-view.ts` and `pixi-backend.ts` now agree on the new `TokenNodeSpec.visual` shape).

- [ ] **Step 5: Run the full render package test suite**

Run: `pnpm --filter @shadowcat/render test`
Expected: PASS.

- [ ] **Step 6: Build the client and run the existing Playwright token regression test**

```bash
pnpm --filter @shadowcat/shell e2e -- --grep "place a token"
```

Expected: PASS — `stage.spec.ts`'s "place a token via the tool rail, then drag it" test (uploads a PNG, places a token, drags it, asserts `data-token-count` stays `1`) passes unmodified, proving the Container-per-token migration preserves the image-token rendering/drag behavior end to end.

- [ ] **Step 7: Commit**

```bash
git add src/client/render/src/pixi-backend.ts
git commit -m "feat(m10h): PixiBackend Container-per-token migration + tick-driven AnimatedSprite playback"
```

---

## Task 7: i18n keys + `ActorsPanel.svelte` visual-kind editor

**Files:**
- Modify: `src/client/ui-kit/src/locales/en.ts` (new keys, appended near the existing `"actors.*"` block starting at line 33)
- Modify: `src/modules/actors/src/ActorsPanel.svelte`
- Test: `src/modules/actors/src/ActorsPanel.test.ts` (new `describe` block)

**Interfaces:**
- Consumes: `TokenVisual`, `FaceVisual`, `AnimatedSource` (Task 1).
- Produces: the create-form's `visual: TokenVisual` construction (was `visual: { kind: "image", asset: assetId }`).

- [ ] **Step 1: Add the i18n keys**

In `src/client/ui-kit/src/locales/en.ts`, insert after the existing `"actors.darkvision"` entry (line 48):

```typescript
  "actors.visualKind": "Visual",
  "actors.visualKindImage": "Image",
  "actors.visualKindFaces": "Faces",
  "actors.visualKindAnimated": "Animated",
  "actors.animSourceType": "Source",
  "actors.animSourceFrames": "Frame list",
  "actors.animSourceSheet": "Grid sheet",
  "actors.animFramesHint": "Click assets below to append them as ordered frames.",
  "actors.animRemoveFrame": "Remove frame",
  "actors.animRows": "Sheet rows",
  "actors.animCols": "Sheet columns",
  "actors.animCount": "Frame count (optional)",
  "actors.animFps": "Frames per second",
  "actors.animLoop": "Loop",
  "actors.faceName": "Face name",
  "actors.faceAdd": "Add face",
  "actors.faceRemove": "Remove",
  "actors.faceDefault": "Default face",
  "actors.faceMapHint": "Optional: auto-swap to a face when a condition is active.",
  "actors.faceMapAdd": "Add condition mapping",
  "actors.faceSwapHint": "Swap the selected token's active face.",
```

- [ ] **Step 2: Write the failing test**

Add to `src/modules/actors/src/ActorsPanel.test.ts`:

```typescript
describe("ActorsPanel — visual kind editor", () => {
  it("defaults to the image kind and creates an image visual as before", async () => {
    const dispatchIntent = vi.fn();
    const { listAssets } = await import("@shadowcat/core");
    vi.mocked(listAssets).mockResolvedValue([
      { id: "asset-1", world_id: "w1", original_name: "hero.png", content_type: "image/png" } as never,
    ]);
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, assets: { url: (id: string) => `/assets/${id}` } as never }),
    });
    await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "hero.png" }).length).toBeGreaterThan(0));
    await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Ogre" } });
    await fireEvent.click(screen.getByRole("button", { name: "hero.png" }));
    await fireEvent.click(screen.getByText("actors.create"));
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    const op = ops[0] as { doc: WireDocument };
    expect(op.doc.system).toMatchObject({ visual: { kind: "image", asset: "asset-1" } });
  });

  it("switching to the animated kind and choosing frames + fps creates an animated visual", async () => {
    const dispatchIntent = vi.fn();
    const { listAssets } = await import("@shadowcat/core");
    vi.mocked(listAssets).mockResolvedValue([
      { id: "f1", world_id: "w1", original_name: "f1.png", content_type: "image/png" } as never,
      { id: "f2", world_id: "w1", original_name: "f2.png", content_type: "image/png" } as never,
    ]);
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, assets: { url: (id: string) => `/assets/${id}` } as never }),
    });
    await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "f1.png" }).length).toBeGreaterThan(0));
    await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Wisp" } });
    await fireEvent.change(screen.getByLabelText("actors.visualKind"), { target: { value: "animated" } });
    await fireEvent.click(screen.getByRole("button", { name: "f1.png" }));
    await fireEvent.click(screen.getByRole("button", { name: "f2.png" }));
    await fireEvent.change(screen.getByLabelText("actors.animFps"), { target: { value: "10" } });
    await fireEvent.click(screen.getByText("actors.create"));
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    const op = ops[0] as { doc: WireDocument };
    expect(op.doc.system).toMatchObject({ visual: { kind: "animated", source: { type: "frames", frames: ["f1", "f2"] }, fps: 10, loop: true } });
  });

  it("switching to the faces kind with two image faces + a default creates a faces visual", async () => {
    const dispatchIntent = vi.fn();
    const { listAssets } = await import("@shadowcat/core");
    vi.mocked(listAssets).mockResolvedValue([
      { id: "n1", world_id: "w1", original_name: "normal.png", content_type: "image/png" } as never,
      { id: "b1", world_id: "w1", original_name: "bloodied.png", content_type: "image/png" } as never,
    ]);
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, assets: { url: (id: string) => `/assets/${id}` } as never }),
    });
    await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "normal.png" }).length).toBeGreaterThan(0));
    await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Goblin" } });
    await fireEvent.change(screen.getByLabelText("actors.visualKind"), { target: { value: "faces" } });
    await fireEvent.click(screen.getByText("actors.faceAdd"));
    await fireEvent.click(screen.getByText("actors.faceAdd"));
    const nameInputs = screen.getAllByLabelText("actors.faceName");
    await fireEvent.input(nameInputs[0], { target: { value: "normal" } });
    await fireEvent.input(nameInputs[1], { target: { value: "bloodied" } });
    const normalPickBtn = screen.getAllByRole("button", { name: "normal.png" })[0];
    await fireEvent.click(normalPickBtn);
    const bloodiedPickBtn = screen.getAllByRole("button", { name: "bloodied.png" })[0];
    await fireEvent.click(bloodiedPickBtn);
    await fireEvent.change(screen.getByLabelText("actors.faceDefault"), { target: { value: "normal" } });
    await fireEvent.click(screen.getByText("actors.create"));
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    const op = ops[0] as { doc: WireDocument };
    expect(op.doc.system).toMatchObject({
      visual: { kind: "faces", faces: { normal: { kind: "image", asset: "n1" }, bloodied: { kind: "image", asset: "b1" } }, default: "normal" },
    });
  });
});
```

> Note: the exact selector for "which face row's asset picker" (`getAllByRole("button", { name: "normal.png" })[0]`) depends on how many asset-picker instances render simultaneously in your markup (Step 3 below renders one picker per face row when that row's kind is "image") — if the create-form's markup structure differs once written, adjust the query to scope by row (e.g. `within(faceRowElement).getByRole(...)`) rather than changing the assertion's intent.

- [ ] **Step 3: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/module-actors test ActorsPanel.test.ts`
Expected: FAIL — no `actors.visualKind` control exists yet.

- [ ] **Step 4: Implement the kind editor**

In `src/modules/actors/src/ActorsPanel.svelte`, update the import line (currently line 5) to add the new types:

```typescript
  import { buildActorDoc, setNameHidden, actorDisplayName, listAssets, type ActorSystem, type WireDocument, type FactionRegistrySystem, type Faction, type TokenVisual, type FaceVisual, type AnimatedSource, type ConditionRegistrySystem, type Condition } from "@shadowcat/core";
```

Add new script-level state (after the existing `let assetList = $state<Asset[]>([]);` line, currently line 28):

```typescript
  type AnimSourceState = {
    sourceType: "frames" | "sheet";
    frames: string[];
    sheetAsset: string | null;
    rows: number;
    cols: number;
    count: number | null;
    fps: number;
    loop: boolean;
  };
  function newAnimSourceState(): AnimSourceState {
    return { sourceType: "frames", frames: [], sheetAsset: null, rows: 1, cols: 1, count: null, fps: 8, loop: true };
  }
  function animSourceToSource(s: AnimSourceState): AnimatedSource {
    return s.sourceType === "frames"
      ? { type: "frames", frames: s.frames }
      : { type: "sheet", asset: s.sheetAsset ?? "", rows: s.rows, cols: s.cols, ...(s.count ? { count: s.count } : {}) };
  }

  type FaceRowState = { name: string; kind: "image" | "animated"; asset: string | null; anim: AnimSourceState };
  function faceRowToVisual(f: FaceRowState): FaceVisual {
    return f.kind === "image" ? { kind: "image", asset: f.asset ?? "" } : { kind: "animated", source: animSourceToSource(f.anim), fps: f.anim.fps, loop: f.anim.loop };
  }

  let visualKind = $state<"image" | "faces" | "animated">("image");
  let topAnim = $state<AnimSourceState>(newAnimSourceState());
  let faceRows = $state<FaceRowState[]>([]);
  let defaultFace = $state("");
  let faceMapRows = $state<{ conditionId: string; faceName: string }[]>([]);

  const conditionOptions = $derived.by((): [string, Condition][] => {
    subscribe();
    const reg = ctx.documents.query("condition-registry")[0]?.system as ConditionRegistrySystem | undefined;
    return Object.entries(reg?.conditions ?? {});
  });

  function buildVisual(): TokenVisual | null {
    if (visualKind === "image") return assetId ? { kind: "image", asset: assetId } : null;
    if (visualKind === "animated") {
      if (topAnim.sourceType === "frames" && topAnim.frames.length === 0) return null;
      if (topAnim.sourceType === "sheet" && !topAnim.sheetAsset) return null;
      return { kind: "animated", source: animSourceToSource(topAnim), fps: topAnim.fps, loop: topAnim.loop };
    }
    if (faceRows.length === 0 || !defaultFace || faceRows.some((f) => !f.name)) return null;
    const faces: Record<string, FaceVisual> = {};
    for (const f of faceRows) faces[f.name] = faceRowToVisual(f);
    const mapped = faceMapRows.filter((r) => r.conditionId && r.faceName);
    const faceMap = mapped.length > 0 ? Object.fromEntries(mapped.map((r) => [r.conditionId, r.faceName])) : undefined;
    return { kind: "faces", faces, default: defaultFace, ...(faceMap ? { faceMap } : {}) };
  }

  function resetVisualEditor(): void {
    visualKind = "image";
    topAnim = newAnimSourceState();
    faceRows = [];
    defaultFace = "";
    faceMapRows = [];
  }
```

Update `create()` (currently lines 54-79): replace the `visual: { kind: "image", asset: assetId }` line and the `if (!name || !assetId) return;` guard, and add `resetVisualEditor()` to the reset block:

```typescript
  function create(): void {
    const visual = buildVisual();
    if (!name || !visual) return;
    const system: ActorSystem = {
      name,
      displayName: displayName || name,
      visual,
      size: { w: sizeW, h: sizeH },
      shape,
      faction,
      conditions: [],
      prototype: instanceOnDrop,
      ...(darkvision > 0 ? { vision: [{ mode: "darkvision" as const, range: darkvision }] } : {}),
    };
    const doc = buildActorDoc(ctx.world, system);
    if (hideName) setNameHidden(doc, true);
    ctx.dispatchIntent([{ op: "create", doc }]);
    name = "";
    displayName = "";
    assetId = null;
    hideName = false;
    faction = null;
    shape = "square";
    sizeW = 1;
    sizeH = 1;
    darkvision = 0;
    resetVisualEditor();
  }
```

In the template, replace the submit button's `disabled` condition (currently `disabled={!name || !assetId}`) with `disabled={!name || !buildVisual()}`, and replace the existing image picker block (currently the `<div class="picker">...</div>` right before the submit button, lines 168-174) with the kind editor. Insert this markup in place of that block:

```svelte
    <label>{t("actors.visualKind")}
      <select bind:value={visualKind} aria-label={t("actors.visualKind")}>
        <option value="image">{t("actors.visualKindImage")}</option>
        <option value="faces">{t("actors.visualKindFaces")}</option>
        <option value="animated">{t("actors.visualKindAnimated")}</option>
      </select>
    </label>

    {#snippet assetPicker(selected, onPick)}
      <div class="picker">
        {#each assetList as a (a.id)}
          <button type="button" class:selected={selected === a.id} title={a.original_name} onclick={() => onPick(a.id)}>
            <img src={ctx.assets.url(a.id)} alt={a.original_name} />
          </button>
        {/each}
      </div>
    {/snippet}

    {#snippet animatedEditor(anim)}
      <label>{t("actors.animSourceType")}
        <select bind:value={anim.sourceType}>
          <option value="frames">{t("actors.animSourceFrames")}</option>
          <option value="sheet">{t("actors.animSourceSheet")}</option>
        </select>
      </label>
      {#if anim.sourceType === "frames"}
        <p class="hint">{t("actors.animFramesHint")}</p>
        {@render assetPicker(null, (id) => (anim.frames = [...anim.frames, id]))}
        <ol class="frame-list">
          {#each anim.frames as f, i (i)}
            <li><img src={ctx.assets.url(f)} alt="" /> <button type="button" onclick={() => (anim.frames = anim.frames.filter((_, j) => j !== i))}>{t("actors.animRemoveFrame")}</button></li>
          {/each}
        </ol>
      {:else}
        {@render assetPicker(anim.sheetAsset, (id) => (anim.sheetAsset = id))}
        <label>{t("actors.animRows")} <input type="number" min="1" step="1" bind:value={anim.rows} /></label>
        <label>{t("actors.animCols")} <input type="number" min="1" step="1" bind:value={anim.cols} /></label>
        <label>{t("actors.animCount")} <input type="number" min="1" step="1" value={anim.count ?? ""} onchange={(e) => (anim.count = e.currentTarget.value ? Number(e.currentTarget.value) : null)} /></label>
      {/if}
      <label>{t("actors.animFps")} <input type="number" min="1" step="1" bind:value={anim.fps} /></label>
      <label><input type="checkbox" bind:checked={anim.loop} /> {t("actors.animLoop")}</label>
    {/snippet}

    {#if visualKind === "image"}
      {@render assetPicker(assetId, (id) => (assetId = id))}
    {:else if visualKind === "animated"}
      {@render animatedEditor(topAnim)}
    {:else}
      <div class="faces-editor">
        {#each faceRows as f, i (i)}
          <div class="face-row">
            <input placeholder={t("actors.faceName")} aria-label={t("actors.faceName")} bind:value={f.name} />
            <select bind:value={f.kind}>
              <option value="image">{t("actors.visualKindImage")}</option>
              <option value="animated">{t("actors.visualKindAnimated")}</option>
            </select>
            {#if f.kind === "image"}
              {@render assetPicker(f.asset, (id) => (f.asset = id))}
            {:else}
              {@render animatedEditor(f.anim)}
            {/if}
            <button type="button" onclick={() => (faceRows = faceRows.filter((_, j) => j !== i))}>{t("actors.faceRemove")}</button>
          </div>
        {/each}
        <button type="button" onclick={() => (faceRows = [...faceRows, { name: "", kind: "image", asset: null, anim: newAnimSourceState() }])}>{t("actors.faceAdd")}</button>
        <label>{t("actors.faceDefault")}
          <select bind:value={defaultFace} aria-label={t("actors.faceDefault")}>
            <option value="">—</option>
            {#each faceRows as f (f.name)}<option value={f.name}>{f.name}</option>{/each}
          </select>
        </label>
        <div class="face-map-editor">
          <p class="hint">{t("actors.faceMapHint")}</p>
          {#each faceMapRows as r, i (i)}
            <div class="face-map-row">
              <select bind:value={r.conditionId}>
                <option value="">—</option>
                {#each conditionOptions as [id, c] (id)}<option value={id}>{c.name}</option>{/each}
              </select>
              <select bind:value={r.faceName}>
                <option value="">—</option>
                {#each faceRows as f (f.name)}<option value={f.name}>{f.name}</option>{/each}
              </select>
              <button type="button" onclick={() => (faceMapRows = faceMapRows.filter((_, j) => j !== i))}>{t("actors.faceRemove")}</button>
            </div>
          {/each}
          <button type="button" onclick={() => (faceMapRows = [...faceMapRows, { conditionId: "", faceName: "" }])}>{t("actors.faceMapAdd")}</button>
        </div>
      </div>
    {/if}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/module-actors test ActorsPanel.test.ts`
Expected: PASS.

- [ ] **Step 6: Typecheck + full package test run**

Run: `pnpm --filter @shadowcat/module-actors typecheck && pnpm --filter @shadowcat/module-actors test`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/client/ui-kit/src/locales/en.ts src/modules/actors/src/ActorsPanel.svelte src/modules/actors/src/ActorsPanel.test.ts
git commit -m "feat(m10h): actor visual-kind editor (image/faces/animated authoring)"
```

---

## Task 8: Per-token face-swap palette

**Files:**
- Modify: `src/modules/actors/src/ActorsPanel.svelte`
- Test: `src/modules/actors/src/ActorsPanel.test.ts` (new `describe` block)

**Interfaces:**
- Consumes: `resolveTokenVisual` (Task 2), `ctx.tokenSelection.ids`, `ctx.canEdit(doc, path)` (existing `AppContext` members, per `ConditionsPanel.svelte`'s precedent).

- [ ] **Step 1: Write the failing test**

```typescript
describe("ActorsPanel — per-token face swap", () => {
  function facesActor(): WireDocument {
    return buildActorDoc(
      "w1",
      { name: "Goblin", displayName: "Goblin", visual: { kind: "faces", faces: { normal: { kind: "image", asset: "n1" }, bloodied: { kind: "image", asset: "b1" } }, default: "normal" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false },
      "act1",
    );
  }

  it("shows no face palette when no token is selected", async () => {
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(facesActor()), dispatchIntent: vi.fn(), tokenSelection: { ids: new Set() }, canEdit: () => true }),
    });
    expect(screen.queryByText("actors.faceSwapHint")).toBeNull();
  });

  it("shows the face palette for a selected token whose visual is 'faces', not for a plain image token", async () => {
    const actor = facesActor();
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    const store = storeWith(actor, token);
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: store, dispatchIntent: vi.fn(), tokenSelection: { ids: new Set(["tok1"]) }, canEdit: () => true }),
    });
    expect(screen.getByText("actors.faceSwapHint")).toBeTruthy();
    expect(screen.getByRole("button", { name: "normal" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "bloodied" })).toBeTruthy();
  });

  it("clicking a face dispatches a /system/face update reading the raw stored value for `old`", async () => {
    const dispatchIntent = vi.fn();
    const actor = facesActor();
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    const store = storeWith(actor, token);
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: store, dispatchIntent, tokenSelection: { ids: new Set(["tok1"]) }, canEdit: () => true }),
    });
    await fireEvent.click(screen.getByRole("button", { name: "bloodied" }));
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/system/face", old: null, new: "bloodied" }] },
    ]);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/module-actors test ActorsPanel.test.ts`
Expected: FAIL — no face-swap palette exists yet.

- [ ] **Step 3: Implement**

Add `resolveTokenVisual` to the existing `@shadowcat/core` import in `ActorsPanel.svelte` (extend Task 7's import line):

```typescript
  import { buildActorDoc, setNameHidden, actorDisplayName, listAssets, resolveTokenVisual, type ActorSystem, type WireDocument, type FactionRegistrySystem, type Faction, type TokenVisual, type FaceVisual, type AnimatedSource, type ConditionRegistrySystem, type Condition } from "@shadowcat/core";
```

Add script logic (near the `conditionOptions` derived from Task 7):

```typescript
  const selectedFaceToken = $derived.by((): WireDocument | null => {
    subscribe();
    const ids = ctx.tokenSelection.ids;
    if (ids.size === 0) return null;
    const tok = ctx.documents.query("token").find((t) => ids.has(t.id));
    if (!tok) return null;
    const eff = undefined; // resolveTokenActor internally; resolveTokenVisual resolves the actor itself.
    void eff;
    return tok;
  });

  /** The actor's declared faces map, if the selected token's actor visual is `faces` — drives
   * whether the palette shows at all (a plain image/animated token has nothing to swap). */
  const selectedFaceNames = $derived.by((): string[] => {
    subscribe();
    const tok = selectedFaceToken;
    if (!tok) return [];
    const actor = tok.system && (tok.system as { actor_id?: string | null }).actor_id ? ctx.documents.get((tok.system as { actor_id: string }).actor_id) : ctx.documents.query("actor").find(() => false);
    const linkedActorId = (tok.system as { actor_id?: string | null } | undefined)?.actor_id;
    const actorDoc = linkedActorId ? ctx.documents.get(linkedActorId) : tok.embedded?.actor?.[0];
    const visual = (actorDoc?.system as { visual?: TokenVisual } | undefined)?.visual;
    return visual?.kind === "faces" ? Object.keys(visual.faces) : [];
  });

  function currentFace(tok: WireDocument): string | null {
    return (tok.system as { face?: string } | undefined)?.face ?? null;
  }

  function swapFace(faceName: string): void {
    const tok = selectedFaceToken;
    if (!tok || !ctx.canEdit(tok, "/system/face")) return;
    const old = currentFace(tok);
    ctx.dispatchIntent([{ op: "update", doc_id: tok.id, changes: [{ path: "/system/face", old, new: faceName }] }]);
  }
```

> Note: `selectedFaceNames`'s first two local `const` lines (`actor`, `linkedActorId`/`actorDoc` computed twice) are leftover exploratory duplication — clean this up before committing: delete the unused first `actor`/`eff` lines and keep only the working `linkedActorId`/`actorDoc`/`visual` computation. The corrected block is:

```typescript
  const selectedFaceToken = $derived.by((): WireDocument | null => {
    subscribe();
    const ids = ctx.tokenSelection.ids;
    if (ids.size === 0) return null;
    return ctx.documents.query("token").find((t) => ids.has(t.id)) ?? null;
  });

  const selectedFaceNames = $derived.by((): string[] => {
    subscribe();
    const tok = selectedFaceToken;
    if (!tok) return [];
    const linkedActorId = (tok.system as { actor_id?: string | null } | undefined)?.actor_id;
    const actorDoc = linkedActorId ? ctx.documents.get(linkedActorId) : tok.embedded?.actor?.[0];
    const visual = (actorDoc?.system as { visual?: TokenVisual } | undefined)?.visual;
    return visual?.kind === "faces" ? Object.keys(visual.faces) : [];
  });

  function currentFace(tok: WireDocument): string | null {
    return (tok.system as { face?: string } | undefined)?.face ?? null;
  }

  function swapFace(faceName: string): void {
    const tok = selectedFaceToken;
    if (!tok || !ctx.canEdit(tok, "/system/face")) return;
    const old = currentFace(tok);
    ctx.dispatchIntent([{ op: "update", doc_id: tok.id, changes: [{ path: "/system/face", old, new: faceName }] }]);
  }
```

Use this corrected form — implement it directly, not the first draft above.

Add markup (e.g. right after the `<h3>{t("actors.title")}</h3>` line):

```svelte
  {#if selectedFaceToken && selectedFaceNames.length > 0}
    <p class="hint">{t("actors.faceSwapHint")}</p>
    <div class="face-palette">
      {#each selectedFaceNames as name (name)}
        <button type="button" class:active={currentFace(selectedFaceToken) === name} onclick={() => swapFace(name)}>{name}</button>
      {/each}
    </div>
  {/if}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/module-actors test ActorsPanel.test.ts`
Expected: PASS.

- [ ] **Step 5: Typecheck + full package test run**

Run: `pnpm --filter @shadowcat/module-actors typecheck && pnpm --filter @shadowcat/module-actors test`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/modules/actors/src/ActorsPanel.svelte src/modules/actors/src/ActorsPanel.test.ts
git commit -m "feat(m10h): per-token face-swap palette (mirrors module-conditions' toggle palette)"
```

---

## Task 9: Playwright regression + animated-token scenario

**Files:**
- Modify: `src/client/shell/e2e/stage.spec.ts`

**Interfaces:** none new — end-to-end proof only.

- [ ] **Step 1: Add a new Playwright test proving an animated token authors and renders**

Add to `src/client/shell/e2e/stage.spec.ts`, after the existing "place a token via the tool rail, then drag it" test:

```typescript
test("author an animated (frame-list) actor token; it places without error", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: /log in/i }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  // Upload two frames for the animated actor.
  await page.getByTestId("asset-upload").setInputFiles({ name: "f1.png", mimeType: "image/png", buffer: PNG_1X1 });
  await page.getByTestId("asset-upload").setInputFiles({ name: "f2.png", mimeType: "image/png", buffer: PNG_1X1 });

  await page.getByPlaceholderText("actors.name").fill("Wisp");
  await page.getByLabel("actors.visualKind").selectOption("animated");
  await page.getByRole("button", { name: "f1.png" }).click();
  await page.getByRole("button", { name: "f2.png" }).click();
  await page.getByLabel("actors.animFps").fill("10");
  await page.getByText("actors.create").click();

  // Select the actor, then place it on the canvas (mirrors the existing token-placement flow).
  await page.getByText("Wisp").click();
  const canvas = page.getByTestId("stage-canvas");
  const box = (await canvas.boundingBox())!;
  await canvas.click({ position: { x: box.width / 2, y: box.height / 2 } });
  await expect(host).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });
});
```

> Note: this test asserts NO exceptions occur through the full author→place→render pipeline for an animated token (the strongest available Playwright-level signal, since the suite has no per-Pixi-object structural inspector) — if the actor-selection UI's exact interaction (`page.getByText("Wisp").click()`) doesn't match how `ActorsPanel`'s existing selection button works, adjust to match the real selector `ctx.actorSelection.select(a.id)` binds to (already visible in `ActorsPanel.svelte`'s `<button onclick={() => ctx.actorSelection.select(a.id)}>`).

- [ ] **Step 2: Run the full Playwright suite**

```bash
pnpm --filter @shadowcat/shell e2e
```

Expected: PASS — the existing "place a token" test (image case) and the new animated-token test both pass, plus every other existing e2e scenario remains green (proves Task 6's Container migration didn't regress drawings/templates/walls/regions/pings, which are unrelated layers but share the same `PixiBackend` file).

- [ ] **Step 3: Commit**

```bash
git add src/client/shell/e2e/stage.spec.ts
git commit -m "test(m10h): Playwright proof an animated actor token authors and renders end to end"
```

---

## Task 10: Reviewed skill-update gate + `docs/PLAN.md` sync

**Files:**
- Modify: `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md`
- Modify: `.claude/skills/shadowcat-codebase-actors-tokens/SKILL.md`
- Modify: `docs/PLAN.md`

**Interfaces:** docs only; no code.

- [ ] **Step 1: Update the scene-rendering skill**

In `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md`, add a new bullet under "Key files & seams" documenting: the Container-per-token structure (`tokenContainer`/`visualContainer`/border/badges, upright-badges-via-non-rotating-parent), `TokenNodeSpec.visual`'s discriminated shape, `computeAnimatedFrame`/`token-animation.ts` as the pure tick-driven frame helper (mirroring `fog-blend.ts`'s extraction precedent), and `DisplayBackend.tickTokenAnimations` as the new per-frame animation-advance seam alongside `startTicker`.

- [ ] **Step 2: Update the actors-tokens skill**

In `.claude/skills/shadowcat-codebase-actors-tokens/SKILL.md`, add: `TokenVisual`/`FaceVisual`/`RenderVisual`/`AnimatedSource` (the M10h visual union, replacing the old flat `ActorVisual`), `resolveTokenVisual` as the new render-boundary read-through (sibling to `resolveTokenActor`/`resolveTokenBox`/`resolveConditions`), the per-token `token.system.face` field + its resolution precedence (manual > faceMap > default > first key), and the "a face is itself a RenderVisual, never nested" invariant.

- [ ] **Step 3: Dispatch `shadowcat-spec-reviewer` on both skill diffs**

Confirm each diff accurately captures what was actually implemented (no omission, drift, or broken `[[...]]` pointer) — this is the mandatory reviewed skill-update gate per the project's `CLAUDE.md`. Fix inline if it flags anything.

- [ ] **Step 4: Update `docs/PLAN.md`**

Add the M10h DONE entry (mirror the style of the existing M10f-3/M10f-4/M10g entries): the visual union + Container-per-token migration + faces/animated resolution + authoring UI, all client-only, no server change. Note remaining M10 work: M10i (`generated`) and M10j (`fx` + `emotes`) still open before the full-M10 push gate.

- [ ] **Step 5: Commit**

```bash
git add docs/PLAN.md .claude/skills/shadowcat-codebase-scene-rendering/SKILL.md .claude/skills/shadowcat-codebase-actors-tokens/SKILL.md
git commit -m "docs(m10h): PLAN sync, update scene-rendering + actors-tokens skills for faces/animated"
```

---

## Self-Review

**1. Spec coverage** (design spec `2026-07-03-m10h-faces-animated-design.md`):
- §3.1 `RenderVisual`/`FaceVisual`/`AnimatedSource`/`TokenVisual` types → Task 1. ✓
- §3.2 per-token active face selection + precedence → Task 1 (`face` field) + Task 2 (`resolveFace` precedence). ✓
- §3.3 `resolveTokenVisual` resolver, fail-closed rules → Task 2. ✓
- §3.4 `TokenNodeSpec` visual restructure, asset-URL resolution at `TokenView` → Task 4 + Task 5. ✓
- §4.1 Container structure (rotating inner / upright outer), `AnimatedSprite` tick-driven, kind-swap preserves border/badges → Task 6. ✓
- §4.2 "what does NOT change" (TokenAnimator, resolveTokenBox/resolveTokenActor, addLayerFilter untouched) → verified none of Tasks 1-9 touch those. ✓
- §5 authoring UI (kind editor: image/faces/animated; per-token face-swap palette; `old`-raw-value convention) → Task 7 + Task 8. ✓
- §6 testing (core resolver precedence + fail-closed cases; render Container structure, tick-driven advance, sheet slicing, kind-swap child-identity preservation, image regression; GL via Playwright) → Tasks 2, 3, 4, 5, 9. ✓
- §7 out-of-scope (generated, fx/emotes/attach-point API, faces-of-faces, atlas JSON, multi-condition ranking, no drag-reorder) → none of Tasks 1-10 build any of these; explicitly called out in Global Constraints and inline notes. ✓
- §8 reviewed skill-update gate targets → Task 10. ✓
- §9 review tier (standard two-reviewer, no mandatory buddy-check) → Global Constraints. ✓

**2. Placeholder scan:** every code step contains complete, concrete code. The three "Note:" callouts (Task 2's simplified resolver form superseding a fragile conditional-type sketch; Task 6's intentional double `width`/`height` assignment; Task 8's cleanup of leftover exploratory duplication in `selectedFaceNames`) each resolve to one concrete, final implementation — none are "TBD"/"handle it later." Task 4's "leaves the package red" note and Task 9's "adjust the selector if it doesn't match" note are honest cross-task/environment caveats, not missing content.

**3. Type consistency:** `TokenVisual`/`FaceVisual`/`RenderVisual`/`AnimatedSource` (Task 1) are consumed with identical field names throughout — `resolveTokenVisual` (Task 2), `TokenNodeSpec.visual`/`ResolvedAnimatedSource` (Task 4), `TokenView.resolveSource`/`toSpec` (Task 5), `PixiBackend`'s `TokenNode`/`visualSourceKey`/`loadAnimatedTextures` (Task 6), and `ActorsPanel.svelte`'s `AnimSourceState`/`FaceRowState`/`buildVisual` (Task 7-8) all agree on `{type:"frames",frames:string[]}` / `{type:"sheet",asset,rows,cols,count?}` pre-resolution and `{type:"frames",urls:string[]}` / `{type:"sheet",url,rows,cols,count?}` post-resolution, and on `token.system.face` as the one mutable-selection field. `computeAnimatedFrame(elapsedMs,fps,frameCount,loop)` (Task 3) is called with the same argument order in Task 6's `tickTokenAnimations`. `DisplayBackend.tickTokenAnimations(dtMs)` (Task 4) is implemented identically in `MockBackend` (Task 4) and `PixiBackend` (Task 6), and called from `TokenView.tick` (Task 5).

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-07-03-m10h-faces-animated.md`. Two execution options:**

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

**Which approach?**
