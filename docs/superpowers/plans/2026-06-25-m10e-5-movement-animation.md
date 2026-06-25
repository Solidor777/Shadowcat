# M10e-5 Movement Animation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the fixed exponential-smoothing token tween with a configurable duration+easing, path-aware, interruptible animation engine, and add a route-commit gesture so a player can make a selected token walk an A* route around obstacles.

**Architecture:** Client-only. The render-package `TokenAnimator` is rewritten to model each token's motion as an eased traversal of a polyline whose duration = `pathDistanceCells ÷ speedCellsPerSec`. Config (`speedCellsPerSec`, `easing`, `cellSize`) flows from `world-settings.animation` via `resolveSceneSettings` through `Stage.svelte` → `RenderEngine.setAnimation`/`setGrid` → `TokenView` → `TokenAnimator`, mirroring the existing diagonal-rule grid wiring. A new `animateAlongPath` seam (added to `SceneToolHost`/`SceneInteraction`) lets the measure tool's route-commit drive a smooth local walk along the route's waypoints, decoupled from the prompt authoritative document commit. The commit decomposes the route into maximal collinear straight runs and dispatches one position-update intent per run **as separate publishes**, because the server move-gate (`Room::publish` → `token_move`) reads the pre-image from the *committed* doc and runs before `apply_intent`; only serialized separate publishes chain (each run gates against the prior committed cell), so a single `start→goal` op would be `Forbidden` the moment a route bends around a wall.

**Tech Stack:** TypeScript, Svelte 5 (runes), Vitest. Rust (server) only for one integration test that locks the gate-chaining invariant the commit relies on — no server code change.

## Global Constraints

- **Server crate is `shadowcat`** (NOT `shadowcat-server`). The server is touched only by ONE added integration test; no server source/protocol change.
- **No protocol change.** Route-following animation is local to the committing client; the wire carries only the existing position-update intents and the existing `Pathfind`/`PathResult` frames.
- **`dist/` must be built before any server `cargo` build** (rust-embed validates `../../dist/` at compile time). For the server integration test: `pnpm --filter @shadowcat/ui build` first, then `cargo test`.
- **Cross-platform from day one** (macOS/Linux/Windows server; desktop + mobile/touch browsers). The route-commit gesture must be touch-compatible (double-tap, not keyboard-only).
- **TDD**: every behavioral change is a failing test first. Client tests run with `pnpm --filter @shadowcat/render test` (render pkg) and `pnpm --filter @shadowcat/module-scene-tools test` (scene-tools). Typecheck each touched package (`pnpm -r typecheck`) — esbuild strips types so a vitest-green change can still be a type error (see memory `vitest-skips-typecheck-in-sdd`).
- **Animation config defaults** (from `DEFAULT_WORLD_SETTINGS.animation`): `speedCellsPerSec: 6`, `easing: "easeInOut"`. `EasingMode = "easeInOut" | "linear"` (exported from `@shadowcat/core`).
- **Debug hygiene**: no `console.log`/`dbg!`; client logging through the project logger if any is needed (none expected).

---

## File Structure

- **Create** `src/client/render/src/easing.ts` — pure easing functions (`linear`, `easeInOut`) + `applyEasing(mode, t)`.
- **Create** `src/client/render/src/easing.test.ts`.
- **Rewrite** `src/client/render/src/token-animator.ts` — duration/easing/path-aware/interruptible animator + config.
- **Rewrite** `src/client/render/src/token-animator.test.ts`.
- **Modify** `src/client/render/src/token-view.ts` — animation-config + cellSize passthrough; `animateAlongPath`.
- **Modify** `src/client/render/src/token-view.test.ts`.
- **Modify** `src/client/render/src/types.ts` — `AnimationConfig` type; `animateAlongPath` on `SceneToolHost`.
- **Modify** `src/client/render/src/engine.ts` — `setAnimation`; `animateAlongPath`; thread `cellSize` from grid.
- **Modify** `src/client/render/src/index.ts` — export `AnimationConfig` (and `EasingMode` re-export if not already available).
- **Modify** `src/client/ui-kit/src/sceneInteraction.ts` — forward `animateAlongPath`.
- **Modify** `src/client/ui-kit/src/sceneInteraction.test.ts`.
- **Create** `src/modules/scene-tools/src/path-runs.ts` — pure `collinearRuns(path)` route → maximal straight-run vertices.
- **Create** `src/modules/scene-tools/src/path-runs.test.ts`.
- **Modify** `src/modules/scene-tools/src/controller.svelte.ts` — route-commit gesture (double-click) in the measure tool: simplify → dispatch run-intents → `animateAlongPath`.
- **Modify** `src/modules/scene-tools/src/measure-tool.test.ts`.
- **Modify** `src/modules/stage/src/Stage.svelte` — wire `resolveSceneSettings(...).animation` → `engine.setAnimation`; thread `cellSize`.
- **Create** `src/server/tests/movement_route_commit.rs` (or add to an existing integration file) — locks gate-chaining: an around-wall route committed as chained per-run intents all succeed; the same net move as one straight op is `Forbidden`.

---

### Task 1: Easing module

**Files:**
- Create: `src/client/render/src/easing.ts`
- Test: `src/client/render/src/easing.test.ts`

**Interfaces:**
- Produces: `export type EasingMode = "easeInOut" | "linear"` (structurally identical to the core type; the render pkg keeps its own to avoid a value import) and `export function applyEasing(mode: EasingMode, t: number): number`.

- [ ] **Step 1: Write the failing test**

```ts
// src/client/render/src/easing.test.ts
import { describe, it, expect } from "vitest";
import { applyEasing } from "./easing";

describe("applyEasing", () => {
  it("linear is the identity on [0,1]", () => {
    expect(applyEasing("linear", 0)).toBe(0);
    expect(applyEasing("linear", 0.25)).toBeCloseTo(0.25);
    expect(applyEasing("linear", 1)).toBe(1);
  });

  it("easeInOut pins endpoints and is symmetric about the midpoint", () => {
    expect(applyEasing("easeInOut", 0)).toBe(0);
    expect(applyEasing("easeInOut", 1)).toBe(1);
    expect(applyEasing("easeInOut", 0.5)).toBeCloseTo(0.5);
    // Symmetry: f(t) + f(1-t) === 1.
    expect(applyEasing("easeInOut", 0.3) + applyEasing("easeInOut", 0.7)).toBeCloseTo(1);
  });

  it("easeInOut starts slower than linear (ease-in) below the midpoint", () => {
    expect(applyEasing("easeInOut", 0.25)).toBeLessThan(0.25);
  });

  it("clamps out-of-range input", () => {
    expect(applyEasing("linear", -1)).toBe(0);
    expect(applyEasing("easeInOut", 5)).toBe(1);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/render test -- easing`
Expected: FAIL — `applyEasing` is not defined.

- [ ] **Step 3: Write minimal implementation**

```ts
// src/client/render/src/easing.ts
/** Token-motion easing curves. Pure, GL-free, unit-tested. */
export type EasingMode = "easeInOut" | "linear";

/** Standard quadratic ease-in-out (smooth accel/decel). Source: standard easing
 * formula (Penner). Chosen over cubic for a gentle, predictable VTT feel. */
function easeInOutQuad(t: number): number {
  return t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
}

/** Map a normalized progress `t` through `mode`. Input is clamped to [0,1]. */
export function applyEasing(mode: EasingMode, t: number): number {
  const c = t <= 0 ? 0 : t >= 1 ? 1 : t;
  return mode === "linear" ? c : easeInOutQuad(c);
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @shadowcat/render test -- easing`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/render/src/easing.ts src/client/render/src/easing.test.ts
git commit -m "feat(m10e-5): easing curves (linear, easeInOut) for token animation"
```

---

### Task 2: Path-aware, duration-based, interruptible TokenAnimator

**Files:**
- Rewrite: `src/client/render/src/token-animator.ts`
- Rewrite: `src/client/render/src/token-animator.test.ts`

**Interfaces:**
- Consumes: `applyEasing`, `EasingMode` from `./easing`; `TokenTransform` from `./types`.
- Produces:
  - `export interface AnimationConfig { speedCellsPerSec: number; easing: EasingMode; cellSize: number }`
  - `class TokenAnimator` with:
    - `setConfig(cfg: AnimationConfig): void`
    - `has(id: string): boolean`
    - `get(id: string): TokenTransform | undefined`
    - `setTarget(id: string, t: TokenTransform): void` — single-segment retarget from current; **ignored** when a path animation is active and `t` matches a path vertex at-or-ahead of current progress (optimistic route steps don't clobber the smooth walk); a brand-new id snaps.
    - `animateAlongPath(id: string, path: [number, number][], rotation: number): void` — eased traversal of the polyline `[current, ...path[1..]]`, ending at `path.last()` with `rotation`.
    - `remove(id: string): void`
    - `tick(dtMs: number): string[]` — advance all animations; return ids whose transform changed.

**Design notes (read before implementing):**
- Each token holds `{ cur, anim }`. `anim` (when present) = `{ poly: TokenTransform-less points [number,number][], segLen: number[], total: number, elapsed: number, duration: number, startRot: number, finalRot: number, pathDriven: boolean, segIndex: number }`. `poly[0]` is the live start (current position at (re)target time).
- Duration: `cells = total / cellSize; durationMs = cells / speedCellsPerSec * 1000`. If `total < EPSILON` or `cellSize <= 0` or `speedCellsPerSec <= 0` → snap immediately (no anim).
- `tick`: `elapsed += dt; t = min(1, elapsed/duration); e = applyEasing(easing, t); dist = e*total`. Walk `poly`/`segLen` to find the point at `dist`. `rotation = lerp(startRot, finalRot, e)`. At `t>=1` settle to the final vertex and clear `anim`.
- `setTarget` interrupt rule (path active): find the lowest path-vertex index `>= segIndex` whose point ≈ `t.{x,y}` (EPSILON). If found → **ignore** (expected optimistic progress) but record it as `segIndex` floor so a later backward target is detected. If not found → **interrupt**: clear `pathDriven`, start a fresh single-segment anim `[cur, t]`.
- `animateAlongPath`: dedupe consecutive ≈-equal points; if `<2` distinct points or zero total → `setTarget(last)`. Else build a `pathDriven` anim with `poly = [cur, ...rest]`, `segIndex = 0`.
- `EPSILON = 0.01`. `lerp(a,b,t) = a + (b-a)*t`.

- [ ] **Step 1: Write the failing tests**

```ts
// src/client/render/src/token-animator.test.ts
import { describe, it, expect } from "vitest";
import { TokenAnimator } from "./token-animator";

const cfg = { speedCellsPerSec: 6, easing: "linear" as const, cellSize: 100 };

function fresh(): TokenAnimator {
  const a = new TokenAnimator();
  a.setConfig(cfg);
  return a;
}

describe("TokenAnimator duration model", () => {
  it("a brand-new id snaps to its target", () => {
    const a = fresh();
    a.setTarget("t1", { x: 100, y: 50, rotation: 0 });
    expect(a.get("t1")).toEqual({ x: 100, y: 50, rotation: 0 });
    expect(a.tick(16)).toEqual([]); // already there → nothing moves
  });

  it("duration = distanceCells / speed; reaches target exactly at duration (linear)", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 }); // snap
    a.setTarget("t1", { x: 600, y: 0, rotation: 0 }); // 6 cells @ 6 c/s → 1000ms
    a.tick(500); // half time, linear → halfway
    expect(a.get("t1")!.x).toBeCloseTo(300, 0);
    a.tick(500); // remaining → settle exactly
    expect(a.get("t1")).toEqual({ x: 600, y: 0, rotation: 0 });
    expect(a.tick(16)).toEqual([]); // settled
  });

  it("easeInOut is slower than linear at the first quarter of the duration", () => {
    const a = new TokenAnimator();
    a.setConfig({ ...cfg, easing: "easeInOut" });
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    a.setTarget("t1", { x: 600, y: 0, rotation: 0 }); // 1000ms
    a.tick(250); // quarter time
    expect(a.get("t1")!.x).toBeLessThan(150); // < linear's 150
  });

  it("animateAlongPath traverses waypoints (L-route bends at the corner)", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 }); // snap to start
    // start (0,0) → (300,0) → (300,300): total 6 cells → 1000ms.
    a.animateAlongPath("t1", [[0, 0], [300, 0], [300, 300]], 0);
    a.tick(500); // half distance (3 cells) → end of first leg, at the corner
    expect(a.get("t1")!.x).toBeCloseTo(300, 0);
    expect(a.get("t1")!.y).toBeCloseTo(0, 0);
    a.tick(500);
    expect(a.get("t1")).toEqual({ x: 300, y: 300, rotation: 0 });
  });

  it("optimistic route-vertex setTarget does NOT clobber an active path walk", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    a.animateAlongPath("t1", [[0, 0], [300, 0], [300, 300]], 0);
    a.tick(100); // partway along leg 1
    const mid = a.get("t1")!.x;
    // Authoritative store steps to the corner vertex (a run endpoint).
    a.setTarget("t1", { x: 300, y: 0, rotation: 0 });
    a.tick(0);
    // Still on the smooth walk near `mid`, NOT jumped to the corner.
    expect(a.get("t1")!.x).toBeCloseTo(mid, 0);
  });

  it("a foreign authoritative position interrupts the path walk and retargets", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    a.animateAlongPath("t1", [[0, 0], [300, 0]], 0);
    a.tick(100);
    a.setTarget("t1", { x: -600, y: 0, rotation: 0 }); // off-path (another mover)
    a.tick(10_000); // settle
    expect(a.get("t1")).toEqual({ x: -600, y: 0, rotation: 0 });
  });

  it("interrupting a tween retargets from the CURRENT position (no stacking)", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    a.setTarget("t1", { x: 1200, y: 0, rotation: 0 }); // 2000ms
    a.tick(500); // ~quarter → x≈300
    const here = a.get("t1")!.x;
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 }); // reverse from `here`
    a.tick(10_000);
    expect(a.get("t1")).toEqual({ x: 0, y: 0, rotation: 0 });
    expect(here).toBeGreaterThan(0);
  });

  it("zero-distance / degenerate config snaps", () => {
    const a = new TokenAnimator();
    a.setConfig({ speedCellsPerSec: 0, easing: "linear", cellSize: 100 });
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    a.setTarget("t1", { x: 500, y: 0, rotation: 0 });
    expect(a.get("t1")!.x).toBe(500); // speed 0 → snap, never freeze
  });

  it("remove drops all state", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    a.remove("t1");
    expect(a.has("t1")).toBe(false);
    expect(a.get("t1")).toBeUndefined();
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/render test -- token-animator`
Expected: FAIL — new API (`setConfig`, `animateAlongPath`) not present.

- [ ] **Step 3: Write the implementation**

```ts
// src/client/render/src/token-animator.ts
import type { TokenTransform } from "./types";
import { applyEasing, type EasingMode } from "./easing";

/** Below this distance (px) a component is treated as coincident. */
const EPSILON = 0.01;

/** Animation tuning resolved from `world-settings.animation` + the active grid. */
export interface AnimationConfig {
  speedCellsPerSec: number;
  easing: EasingMode;
  /** Pixels per grid cell (grid.size); converts pixel distance to cells for duration. */
  cellSize: number;
}

const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;

interface Anim {
  /** Polyline in scene px; `poly[0]` is the start captured at (re)target time. */
  poly: [number, number][];
  segLen: number[];
  total: number;
  elapsed: number;
  duration: number;
  startRot: number;
  finalRot: number;
  easing: EasingMode;
  /** True for an explicit route walk; gates the optimistic-vertex ignore rule. */
  pathDriven: boolean;
  /** Lowest vertex index the walk is still heading toward (monotonic). */
  segIndex: number;
}

/** Holds each token's rendered transform and advances it toward the document-authoritative
 * target along an eased polyline. Duration = pathDistanceCells / speedCellsPerSec.
 * New tokens snap; moves tween; a newer authoritative position retargets in place. */
export class TokenAnimator {
  private cur = new Map<string, TokenTransform>();
  private anim = new Map<string, Anim>();
  private cfg: AnimationConfig = { speedCellsPerSec: 6, easing: "easeInOut", cellSize: 100 };

  setConfig(cfg: AnimationConfig): void {
    this.cfg = cfg;
  }
  has(id: string): boolean {
    return this.cur.has(id);
  }
  get(id: string): TokenTransform | undefined {
    return this.cur.get(id);
  }
  remove(id: string): void {
    this.cur.delete(id);
    this.anim.delete(id);
  }

  setTarget(id: string, t: TokenTransform): void {
    const c = this.cur.get(id);
    if (!c) {
      this.cur.set(id, { ...t }); // brand-new → snap
      return;
    }
    const active = this.anim.get(id);
    if (active?.pathDriven) {
      // Expected optimistic progress: a target matching a path vertex at-or-ahead of the
      // current segment is the authoritative store catching up to where we already walk —
      // keep the smooth walk. Anything else (foreign mover / backward rollback) interrupts.
      for (let i = active.segIndex; i < active.poly.length; i++) {
        const v = active.poly[i];
        if (Math.abs(v[0] - t.x) < EPSILON && Math.abs(v[1] - t.y) < EPSILON) {
          active.finalRot = t.rotation; // adopt authoritative rotation as the settle value
          return;
        }
      }
    }
    this.startAnim(id, c, [[c.x, c.y], [t.x, t.y]], t.rotation, false);
  }

  animateAlongPath(id: string, path: [number, number][], rotation: number): void {
    const c = this.cur.get(id);
    if (!c) {
      const last = path[path.length - 1] ?? [0, 0];
      this.cur.set(id, { x: last[0], y: last[1], rotation }); // no prior render → snap
      return;
    }
    // Dedupe consecutive coincident points; anchor the walk at the live current position.
    const pts: [number, number][] = [[c.x, c.y]];
    for (const p of path) {
      const prev = pts[pts.length - 1];
      if (Math.abs(prev[0] - p[0]) >= EPSILON || Math.abs(prev[1] - p[1]) >= EPSILON) pts.push([p[0], p[1]]);
    }
    if (pts.length < 2) {
      this.setTarget(id, { x: pts[0][0], y: pts[0][1], rotation });
      return;
    }
    this.startAnim(id, c, pts, rotation, true);
  }

  private startAnim(id: string, c: TokenTransform, poly: [number, number][], finalRot: number, pathDriven: boolean): void {
    const segLen: number[] = [];
    let total = 0;
    for (let i = 1; i < poly.length; i++) {
      const dx = poly[i][0] - poly[i - 1][0];
      const dy = poly[i][1] - poly[i - 1][1];
      const len = Math.hypot(dx, dy);
      segLen.push(len);
      total += len;
    }
    const last = poly[poly.length - 1];
    if (total < EPSILON || this.cfg.cellSize <= 0 || this.cfg.speedCellsPerSec <= 0) {
      this.cur.set(id, { x: last[0], y: last[1], rotation: finalRot }); // degenerate → snap
      this.anim.delete(id);
      return;
    }
    const cells = total / this.cfg.cellSize;
    this.anim.set(id, {
      poly, segLen, total, elapsed: 0,
      duration: (cells / this.cfg.speedCellsPerSec) * 1000,
      startRot: c.rotation, finalRot, easing: this.cfg.easing,
      pathDriven, segIndex: 0,
    });
  }

  tick(dtMs: number): string[] {
    const moved: string[] = [];
    for (const [id, a] of this.anim) {
      a.elapsed += dtMs;
      const tRaw = Math.min(1, a.elapsed / a.duration);
      const e = applyEasing(a.easing, tRaw);
      const target = e * a.total;
      // Walk segments to the eased distance.
      let acc = 0;
      let pos = a.poly[a.poly.length - 1];
      let idx = a.poly.length - 1;
      for (let i = 0; i < a.segLen.length; i++) {
        if (target <= acc + a.segLen[i] || i === a.segLen.length - 1) {
          const f = a.segLen[i] > 0 ? Math.min(1, (target - acc) / a.segLen[i]) : 1;
          pos = [lerp(a.poly[i][0], a.poly[i + 1][0], f), lerp(a.poly[i][1], a.poly[i + 1][1], f)];
          idx = f >= 1 ? i + 1 : i;
          break;
        }
        acc += a.segLen[i];
      }
      a.segIndex = idx; // monotonic-ish progress marker for the ignore rule
      const cur = this.cur.get(id)!;
      cur.x = pos[0];
      cur.y = pos[1];
      cur.rotation = lerp(a.startRot, a.finalRot, e);
      moved.push(id);
      if (tRaw >= 1) {
        const last = a.poly[a.poly.length - 1];
        cur.x = last[0];
        cur.y = last[1];
        cur.rotation = a.finalRot;
        this.anim.delete(id);
      }
    }
    return moved;
  }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/render test -- token-animator`
Expected: PASS.

- [ ] **Step 5: Typecheck the render package**

Run: `pnpm --filter @shadowcat/render typecheck`
Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add src/client/render/src/token-animator.ts src/client/render/src/token-animator.test.ts
git commit -m "feat(m10e-5): duration+easing, path-aware, interruptible TokenAnimator"
```

---

### Task 3: TokenView — config + cellSize passthrough + animateAlongPath

**Files:**
- Modify: `src/client/render/src/token-view.ts`
- Modify: `src/client/render/src/token-view.test.ts`

**Interfaces:**
- Consumes: `TokenAnimator` (Task 2) — `setConfig`, `animateAlongPath`.
- Produces (new `TokenView` methods): `setAnimationConfig(cfg: { speedCellsPerSec: number; easing: EasingMode }): void`, `setCellSize(px: number): void`, `animateAlongPath(id: string, path: [number, number][]): void`.

**Design notes:** `TokenView` keeps the live `cellSize` and the `{speedCellsPerSec, easing}` pair and re-pushes a merged `AnimationConfig` to the animator whenever either changes. `animateAlongPath` resolves the token's current rotation from its spec (rotation does not change during a route move) and forwards.

- [ ] **Step 1: Write the failing test** (append to `token-view.test.ts`)

```ts
// Animation config reaches the animator: a slow speed makes a move take longer.
it("setAnimationConfig + setCellSize drive tween duration", () => {
  const store = makeStoreWithToken("tok1", { x: 0, y: 0 }); // existing helper in this file
  const backend = new RecordingBackend();
  const view = new TokenView(store, new AssetResolver(), backend);
  view.setCellSize(100);
  view.setAnimationConfig({ speedCellsPerSec: 1, easing: "linear" }); // 1 cell/s
  view.reconcile(); // snap at (0,0)
  moveToken(store, "tok1", { x: 100, y: 0 }); // 1 cell → 1000ms
  view.reconcile();
  view.tick(500); // half → ~x=50
  expect(backend.lastTokenX("tok1")).toBeCloseTo(50, 0);
  view.tick(500);
  expect(backend.lastTokenX("tok1")).toBeCloseTo(100, 0);
});

it("animateAlongPath walks the route polyline", () => {
  const store = makeStoreWithToken("tok1", { x: 0, y: 0 });
  const backend = new RecordingBackend();
  const view = new TokenView(store, new AssetResolver(), backend);
  view.setCellSize(100);
  view.setAnimationConfig({ speedCellsPerSec: 6, easing: "linear" });
  view.reconcile();
  view.animateAlongPath("tok1", [[0, 0], [300, 0], [300, 300]]); // 6 cells → 1000ms
  view.tick(500);
  expect(backend.lastTokenX("tok1")).toBeCloseTo(300, 0); // at the corner
  expect(backend.lastTokenY("tok1")).toBeCloseTo(0, 0);
});
```

> If `token-view.test.ts` lacks `makeStoreWithToken`/`moveToken`/`RecordingBackend.lastTokenX`, add minimal local helpers in this test file mirroring the existing fixtures (read the file first; reuse what's there — the file already builds tokens and a backend for the settle test at line ~53).

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/render test -- token-view`
Expected: FAIL — `setAnimationConfig`/`setCellSize`/`animateAlongPath` not defined.

- [ ] **Step 3: Implement** — edit `token-view.ts`:

Add fields + methods to `TokenView` (the animator already defaults its config):

```ts
  private cellSize = 100;
  private animSpeed = 6;
  private animEasing: EasingMode = "easeInOut";

  setCellSize(px: number): void {
    this.cellSize = px;
    this.pushAnimConfig();
  }
  setAnimationConfig(cfg: { speedCellsPerSec: number; easing: EasingMode }): void {
    this.animSpeed = cfg.speedCellsPerSec;
    this.animEasing = cfg.easing;
    this.pushAnimConfig();
  }
  private pushAnimConfig(): void {
    this.animator.setConfig({ speedCellsPerSec: this.animSpeed, easing: this.animEasing, cellSize: this.cellSize });
  }

  /** Drive a smooth local walk along a route's scene-coord waypoints. Rotation is held
   * (a route move does not rotate the token). The prompt authoritative commit catches up
   * via reconcile()'s setTarget, which the animator recognizes as expected progress. */
  animateAlongPath(id: string, path: [number, number][]): void {
    const rotation = this.specs.get(id)?.rotation ?? 0;
    this.animator.animateAlongPath(id, path, rotation);
    this.push(id);
  }
```

Add the import: `import { TokenAnimator } from "./token-animator"; import type { EasingMode } from "./easing";` (extend the existing animator import line).

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/render test -- token-view`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/render/src/token-view.ts src/client/render/src/token-view.test.ts
git commit -m "feat(m10e-5): TokenView animation-config + cellSize + animateAlongPath passthrough"
```

---

### Task 4: Engine seam — setAnimation, animateAlongPath, cellSize from grid

**Files:**
- Modify: `src/client/render/src/types.ts` (add `animateAlongPath` to `SceneToolHost`)
- Modify: `src/client/render/src/engine.ts`
- Modify: `src/client/render/src/index.ts` (export `AnimationConfig`)
- Modify: `src/client/render/src/engine.test.ts` (one test)

**Interfaces:**
- Consumes: `TokenView` (Task 3).
- Produces:
  - `RenderEngine.setAnimation(cfg: { speedCellsPerSec: number; easing: EasingMode }): void`
  - `SceneToolHost.animateAlongPath(id: string, path: [number, number][]): void` (implemented by `RenderEngine`).
  - `RenderEngine.setGrid` additionally calls `tokens.setCellSize(spec.size)`.

**Design notes:** `setGrid` currently reassigns `this.grid = new Grid(spec)`. `GridSpec` has `size` (px/cell). After building the grid, call `this.tokens.setCellSize(spec.size)`. In the constructor, seed `this.tokens.setCellSize(opts.grid.size)` so the first reconcile has a real cell size. `setAnimation` forwards to `this.tokens.setAnimationConfig`.

- [ ] **Step 1: Write the failing test** (append to `engine.test.ts`)

```ts
it("animateAlongPath forwards to the token view (SceneToolHost seam)", () => {
  const { engine, backend } = makeEngineWithToken("tok1", { x: 0, y: 0 }); // mirror existing engine test setup
  engine.setGrid({ kind: "square", size: 100 });
  engine.setAnimation({ speedCellsPerSec: 6, easing: "linear" });
  engine.start();
  engine.animateAlongPath("tok1", [[0, 0], [300, 0]]);
  backend.runTicker(500); // advance the injected ticker by 500ms
  expect(backend.lastTokenX("tok1")).toBeCloseTo(300, 0);
});
```

> Read `engine.test.ts` first; reuse its existing engine/backend fixture and ticker-driving helper. If no `runTicker` helper exists, drive `tick` via the backend's captured ticker callback as the other engine tests do.

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/render test -- engine`
Expected: FAIL — `setAnimation`/`animateAlongPath` not defined.

- [ ] **Step 3: Implement**

In `types.ts`, add to `SceneToolHost` (after `addPing`):

```ts
  /** Drive a smooth local walk of a token along a route's scene-coord waypoints. */
  animateAlongPath(id: string, path: [number, number][]): void;
```

In `engine.ts`:
- Constructor: after `this.tokens = new TokenView(...)`, add `this.tokens.setCellSize(opts.grid.size);`
- `setGrid(spec)`: after the grid is rebuilt, add `this.tokens.setCellSize(spec.size);`
- Add methods:

```ts
  setAnimation(cfg: { speedCellsPerSec: number; easing: EasingMode }): void {
    this.tokens.setAnimationConfig(cfg);
  }

  animateAlongPath(id: string, path: [number, number][]): void {
    this.tokens.animateAlongPath(id, path);
  }
```
Add `import type { EasingMode } from "./easing";` to engine.ts.

In `index.ts`, export the config type: `export { TokenAnimator, type AnimationConfig } from "./token-animator";` (extend the existing `TokenAnimator` export line) and ensure `EasingMode` is exported (`export type { EasingMode } from "./easing";`).

- [ ] **Step 4: Run tests + typecheck**

Run: `pnpm --filter @shadowcat/render test && pnpm --filter @shadowcat/render typecheck`
Expected: PASS, no type errors. (All existing `SceneToolHost` implementers — only `RenderEngine` and the test fakes — now need `animateAlongPath`; update render-package fakes in this task; ui-kit fakes are Task 5.)

- [ ] **Step 5: Commit**

```bash
git add src/client/render/src/types.ts src/client/render/src/engine.ts src/client/render/src/index.ts src/client/render/src/engine.test.ts
git commit -m "feat(m10e-5): RenderEngine setAnimation + animateAlongPath + cellSize from grid"
```

---

### Task 5: SceneInteraction bridge — forward animateAlongPath

**Files:**
- Modify: `src/client/ui-kit/src/sceneInteraction.ts`
- Modify: `src/client/ui-kit/src/sceneInteraction.test.ts`
- Modify: `src/client/ui-kit/src/__fixtures__/fakeSceneHost.ts` (add the new method to the fake)

**Interfaces:**
- Produces: `SceneInteraction.animateAlongPath(id, path)` forwarding to the attached host; a detached bridge no-ops.

- [ ] **Step 1: Write the failing test** (append to `sceneInteraction.test.ts`)

```ts
test("animateAlongPath forwards to the host (no-op when detached)", () => {
  const bridge = new SceneInteractionBridge();
  expect(() => bridge.animateAlongPath("t1", [[0, 0], [1, 1]])).not.toThrow(); // detached: no-op
  const calls: Array<{ id: string; path: [number, number][] }> = [];
  bridge.attach(fakeSceneHost({ animateAlongPath: (id, path) => calls.push({ id, path }) }));
  bridge.animateAlongPath("t1", [[0, 0], [1, 1]]);
  expect(calls).toEqual([{ id: "t1", path: [[0, 0], [1, 1]] }]);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @shadowcat/ui-kit test -- sceneInteraction`
Expected: FAIL — `animateAlongPath` not on the bridge / fake.

- [ ] **Step 3: Implement**

In `sceneInteraction.ts`, add to `SceneInteractionBridge` (the interface extends `SceneToolHost`, which now declares it):

```ts
  animateAlongPath(id: string, path: [number, number][]): void {
    this.#host?.animateAlongPath(id, path);
  }
```

In `__fixtures__/fakeSceneHost.ts`, add a default `animateAlongPath: () => {}` to the fake's defaults so existing callers stay valid.

- [ ] **Step 4: Run tests + typecheck**

Run: `pnpm --filter @shadowcat/ui-kit test && pnpm --filter @shadowcat/ui-kit typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/client/ui-kit/src/sceneInteraction.ts src/client/ui-kit/src/sceneInteraction.test.ts src/client/ui-kit/src/__fixtures__/fakeSceneHost.ts
git commit -m "feat(m10e-5): SceneInteraction bridge forwards animateAlongPath"
```

---

### Task 6: collinearRuns — route → maximal straight-run vertices

**Files:**
- Create: `src/modules/scene-tools/src/path-runs.ts`
- Create: `src/modules/scene-tools/src/path-runs.test.ts`

**Interfaces:**
- Produces: `export function collinearRuns(path: [number, number][]): [number, number][]` — returns the route's turn-point vertices: `path[0]`, each point where direction changes, and `path.last()`. Consecutive collinear segments collapse into one run. A run is wall-clear by transitivity (each unit step is clear and they share a line), so each `runs[i]→runs[i+1]` is a gate-valid straight move.

**Design notes:** Direction comparison uses the normalized integer step where possible; for floating cell-center coords, compare the cross-product of consecutive segment vectors against an epsilon (`|ax*by - ay*bx| < 1e-6 * maxlen`) AND require same sign of dot product (no reversal). Always keep first and last. Degenerate (`<2` points) returns the input unchanged.

- [ ] **Step 1: Write the failing tests**

```ts
// src/modules/scene-tools/src/path-runs.test.ts
import { describe, it, expect } from "vitest";
import { collinearRuns } from "./path-runs";

describe("collinearRuns", () => {
  it("collapses a straight horizontal run to its endpoints", () => {
    expect(collinearRuns([[0, 0], [100, 0], [200, 0], [300, 0]])).toEqual([[0, 0], [300, 0]]);
  });

  it("keeps the corner of an L-route", () => {
    expect(collinearRuns([[0, 0], [100, 0], [200, 0], [200, 100], [200, 200]])).toEqual([
      [0, 0], [200, 0], [200, 200],
    ]);
  });

  it("keeps a diagonal run as one segment and its turn", () => {
    expect(collinearRuns([[0, 0], [100, 100], [200, 200], [300, 200]])).toEqual([
      [0, 0], [200, 200], [300, 200],
    ]);
  });

  it("passes through trivial paths unchanged", () => {
    expect(collinearRuns([[5, 5]])).toEqual([[5, 5]]);
    expect(collinearRuns([[0, 0], [50, 0]])).toEqual([[0, 0], [50, 0]]);
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/module-scene-tools test -- path-runs`
Expected: FAIL — `collinearRuns` not defined.

- [ ] **Step 3: Implement**

```ts
// src/modules/scene-tools/src/path-runs.ts
/** Collapse a per-cell A* route into its turn-point vertices: the start, every direction
 * change, and the goal. Each returned segment `runs[i]→runs[i+1]` is a single straight run
 * of collinear clear unit steps, so it crosses no `blocksMove` wall (transitivity along a
 * line) and is gate-valid as one position-update intent. */
export function collinearRuns(path: [number, number][]): [number, number][] {
  if (path.length < 3) return path.map((p) => [p[0], p[1]]);
  const out: [number, number][] = [[path[0][0], path[0][1]]];
  for (let i = 1; i < path.length - 1; i++) {
    const a: [number, number] = [path[i][0] - path[i - 1][0], path[i][1] - path[i - 1][1]];
    const b: [number, number] = [path[i + 1][0] - path[i][0], path[i + 1][1] - path[i][1]];
    const cross = a[0] * b[1] - a[1] * b[0];
    const dot = a[0] * b[0] + a[1] * b[1];
    const scale = Math.max(Math.hypot(...a) * Math.hypot(...b), 1e-9);
    // A turn = non-collinear (cross != 0) or a reversal (dot < 0). Keep this vertex.
    if (Math.abs(cross) > 1e-6 * scale || dot < 0) out.push([path[i][0], path[i][1]]);
  }
  out.push([path[path.length - 1][0], path[path.length - 1][1]]);
  return out;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @shadowcat/module-scene-tools test -- path-runs`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/modules/scene-tools/src/path-runs.ts src/modules/scene-tools/src/path-runs.test.ts
git commit -m "feat(m10e-5): collinearRuns route simplification for gate-valid commit segments"
```

---

### Task 7: Route-commit gesture in the measure tool

**Files:**
- Modify: `src/modules/scene-tools/src/controller.svelte.ts` (`makeMeasureTool`)
- Modify: `src/modules/scene-tools/src/measure-tool.test.ts`

**Interfaces:**
- Consumes: `collinearRuns` (Task 6); `ctx.scene.animateAlongPath` (Task 5); `ctx.pathfind`, `ctx.dispatchIntent`, `ctx.documents`, `ctx.tokenSelection`.
- Produces: in route mode, a **double-click** (two `onPointerDown` within `DOUBLE_CLICK_MS` at ~the same point) commits a route from the selected token's center to the double-clicked (snapped) goal: it pathfinds, then on resolve (a) `animateAlongPath(tokenId, fullPath)` for the local smooth walk, and (b) dispatches one `op:"update"` position intent per `collinearRuns` segment endpoint **as separate `dispatchIntent` calls** (each gates against the prior committed cell). Single clicks keep the existing preview behavior.

**Design notes:**
- Add module constant `const DOUBLE_CLICK_MS = 350;` and `const COMMIT_RADIUS = 12;` (px, in scene coords post-snap a double-click lands on the same cell, so this is generous).
- Track `lastDownAt` and `lastDownPt` across `onPointerDown`. On a down: if `inRouteMode()` and `now - lastDownAt < DOUBLE_CLICK_MS` and within `COMMIT_RADIUS` of `lastDownPt` → `commitRoute(goal)` and reset `lastDownAt`. Else record `lastDownAt/lastDownPt` and run the existing waypoint-push/preview path.
- `commitRoute(goal)`: requires a single selected token (`tokenCenter()`), an active scene, and `ctx.pathfind`. Issue `ctx.pathfind(scene.id, start, [...waypoints, [goal.x, goal.y]], fp)`. On resolve with `result.path.length >= 2`:
  - `const tokenId = [...ctx.tokenSelection!.ids][0];`
  - `ctx.scene.animateAlongPath(tokenId, result.path);`
  - `const runs = collinearRuns(result.path);` then for `i` in `1..runs.length`: read the token's current committed `system.x/y` as the `old` of the FIRST op only; for each run dispatch a **separate** intent:
    ```ts
    for (let i = 1; i < runs.length; i++) {
      const [nx, ny] = runs[i];
      const sys = ctx.documents.get(tokenId)?.system as { x?: number; y?: number } | undefined;
      ctx.dispatchIntent([{ op: "update", doc_id: tokenId, changes: [
        { path: "/system/x", old: sys?.x ?? null, new: nx },
        { path: "/system/y", old: sys?.y ?? null, new: ny },
      ] }]);
    }
    ```
    > Each iteration re-reads `sys` AFTER the prior `dispatchIntent` so the optimistic store has advanced, giving each op the correct `old`. (The optimistic client applies each intent synchronously before the next read.)
  - Then `clearRoute()` (clears overlays + waypoints).
- `now` source: reuse `ctx.now ?? (() => Date.now())` as the select-move tool does.
- Guard: if `result.path.length < 2` or the promise rejects → `clearRoute()` only (no move).

- [ ] **Step 1: Write the failing test** (append to `measure-tool.test.ts`; reuse its `seedRouteCtx` fixture)

```ts
test("double-click in route mode commits: animates the path and dispatches one intent per collinear run", async () => {
  const sent: WireOperation[][] = [];
  const animated: Array<{ id: string; path: [number, number][] }> = [];
  // L-route 0,0 → 200,0 → 200,200 (per-cell points); collinearRuns → 2 runs.
  const pathfind: ToolContext["pathfind"] = async () => ({
    path: [[0, 0], [100, 0], [200, 0], [200, 100], [200, 200]] as [number, number][],
    cost: 4,
  });
  const { ctx, now } = seedRouteCtx({
    pathfind,
    dispatchIntent: (ops) => sent.push(ops),
    animateAlongPath: (id, path) => animated.push({ id, path }),
    tokenAt: { id: "tok1", x: 0, y: 0 },
  });
  const tool = makeMeasureTool(ctx);
  tool.onPointerDown({ x: 200, y: 200 }, ev()); // first click → records time
  now.advance(100);
  tool.onPointerDown({ x: 200, y: 200 }, ev()); // double-click → commit
  await drain(); // resolve the pathfind promise

  expect(animated).toEqual([{ id: "tok1", path: [[0, 0], [100, 0], [200, 0], [200, 100], [200, 200]] }]);
  // Two collinear runs → two SEPARATE dispatchIntent calls (chaining through the gate).
  expect(sent.length).toBe(2);
  expect(sent[0][0]).toMatchObject({ op: "update", doc_id: "tok1" });
  // First run goal = the corner (200,0); second = (200,200).
  const xy = (ops: WireOperation[]) => {
    const ch = (ops[0] as { changes: { path: string; new: unknown }[] }).changes;
    return [ch.find((c) => c.path === "/system/x")!.new, ch.find((c) => c.path === "/system/y")!.new];
  };
  expect(xy(sent[0])).toEqual([200, 0]);
  expect(xy(sent[1])).toEqual([200, 200]);
});

test("a single click in route mode does NOT commit", async () => {
  const sent: WireOperation[][] = [];
  const { ctx } = seedRouteCtx({ pathfind: async () => ({ path: [[0, 0], [100, 0]] as [number, number][], cost: 1 }), dispatchIntent: (o) => sent.push(o), tokenAt: { id: "tok1", x: 0, y: 0 } });
  const tool = makeMeasureTool(ctx);
  tool.onPointerDown({ x: 100, y: 0 }, ev());
  await drain();
  expect(sent.length).toBe(0);
});
```

> Read `measure-tool.test.ts` first. It already has a `pathfind` stub pattern, a `drain()` microtask helper (line ~10), and a single-token-selected ctx builder (line ~41). Extend that builder into `seedRouteCtx` accepting `animateAlongPath`, `dispatchIntent`, an injected `now` with `advance()`, and a `tokenAt` seed; add an `ev()` PointerEvent stub. Keep the new helpers local to the test file.

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm --filter @shadowcat/module-scene-tools test -- measure-tool`
Expected: FAIL — no commit path; `animateAlongPath` not called; `sent` empty.

- [ ] **Step 3: Implement** in `controller.svelte.ts`

- Add imports: `import { collinearRuns } from "./path-runs";` and ensure `WireOperation` is already imported (it is, line 6).
- Add module constants near `ROUTE_COLOR`: `const DOUBLE_CLICK_MS = 350; const COMMIT_RADIUS = 12;`
- In `makeMeasureTool`, add state: `const now = ctx.now ?? ((): number => Date.now()); let lastDownAt = -Infinity; let lastDownPt: Point = { x: 0, y: 0 };`
- Add `commitRoute`:

```ts
  /** Commit a route from the selected token's center to `goal`: smooth local walk +
   * one position intent per collinear run (separate publishes so each gates against the
   * prior committed cell — a single straight start→goal op would be Forbidden around walls). */
  function commitRoute(goal: Point): void {
    if (!ctx.pathfind || !ctx.tokenSelection || ctx.tokenSelection.ids.size !== 1) return;
    const scene = activeScene(ctx);
    const start = tokenCenter();
    if (!scene || !start) return;
    const tokenId = [...ctx.tokenSelection.ids][0];
    const fp = resolveFootprint();
    const seq = ++pendingSeq;
    ctx.pathfind(scene.id, start, [...waypoints, [goal.x, goal.y]], fp).then(
      (result) => {
        if (seq !== pendingSeq) return;
        if (result.path.length < 2) { clearRoute(); return; }
        ctx.scene.animateAlongPath(tokenId, result.path);
        const runs = collinearRuns(result.path);
        for (let i = 1; i < runs.length; i++) {
          const [nx, ny] = runs[i];
          const sys = ctx.documents.get(tokenId)?.system as { x?: number; y?: number } | undefined;
          ctx.dispatchIntent([{ op: "update", doc_id: tokenId, changes: [
            { path: "/system/x", old: sys?.x ?? null, new: nx },
            { path: "/system/y", old: sys?.y ?? null, new: ny },
          ] }]);
        }
        clearRoute();
      },
      () => { if (seq === pendingSeq) clearRoute(); },
    );
  }
```

- In `onPointerDown`, at the very top of the `inRouteMode()` branch, add the double-click check BEFORE the waypoint push:

```ts
    onPointerDown(p: Point): boolean {
      if (inRouteMode()) {
        const scene = activeScene(ctx);
        if (scene) {
          const snapped = ctx.scene.snap(p);
          const t = now();
          const isDouble = t - lastDownAt < DOUBLE_CLICK_MS &&
            Math.hypot(snapped.x - lastDownPt.x, snapped.y - lastDownPt.y) < COMMIT_RADIUS;
          if (isDouble) {
            lastDownAt = -Infinity; // consume the gesture
            commitRoute(snapped);
            return true;
          }
          lastDownAt = t;
          lastDownPt = snapped;
          waypoints.push([snapped.x, snapped.y]);
          return true;
        }
      }
      anchor = p;
      return true;
    },
```

(Leave `onPointerMove`/`onPointerUp`/`onDeactivate` as-is; `clearRoute` already resets `waypoints` + overlays.)

- [ ] **Step 4: Run tests + typecheck**

Run: `pnpm --filter @shadowcat/module-scene-tools test && pnpm --filter @shadowcat/module-scene-tools typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/modules/scene-tools/src/controller.svelte.ts src/modules/scene-tools/src/measure-tool.test.ts
git commit -m "feat(m10e-5): route-commit double-click — walk the A* route via chained per-run intents"
```

---

### Task 8: Stage wiring — animation config from world-settings

**Files:**
- Modify: `src/modules/stage/src/Stage.svelte`

**Interfaces:**
- Consumes: `RenderEngine.setAnimation` (Task 4); `resolveSceneSettings(...).animation` (already exported from `@shadowcat/core`).

**Design notes:** The `onDocs` reactive block already resolves `resolveSceneSettings(scene, documents)` for `diagonalRule`. Extend it to also push `animation` to the engine, keyed so it only fires on change (mirror `lastGridKey`). `cellSize` is already pushed via `setGrid(spec)` (Task 4 wires `spec.size` → animator), so no extra cellSize call is needed here.

- [ ] **Step 1: Implement** (no new unit test — covered by the render/scene-tools tests + the existing Stage Playwright smoke; this is wiring of already-tested seams). Edit the `onDocs` block:

```ts
      let lastGridKey = "";
      let lastAnimKey = "";
      const onDocs = (): void => {
        const scene = documents.query("scene")[0];
        const settings = resolveSceneSettings(scene, documents);
        const g = (scene?.system as { grid?: { kind: "square" | "hex"; size: number } } | undefined)?.grid;
        const diagonalRule = settings.diagonalRule;
        const spec = { ...(g ?? { kind: "square" as const, size: 100 }), diagonalRule };
        const key = `${spec.kind}:${spec.size}:${diagonalRule}`;
        if (key !== lastGridKey) {
          lastGridKey = key;
          e.setGrid(spec);
        }
        const anim = settings.animation;
        const animKey = `${anim.speedCellsPerSec}:${anim.easing}`;
        if (animKey !== lastAnimKey) {
          lastAnimKey = animKey;
          e.setAnimation({ speedCellsPerSec: anim.speedCellsPerSec, easing: anim.easing });
        }
        // ...rest of onDocs unchanged (tokenCount, shapeCount, playerOptions, gmView)...
      };
```

(Replace the existing `const diagonalRule = resolveSceneSettings(...)` line with the `settings` hoist above so it is resolved once.)

- [ ] **Step 2: Typecheck + build the client**

Run: `pnpm --filter @shadowcat/module-stage typecheck && pnpm --filter @shadowcat/ui build`
Expected: no type errors; client build succeeds (needed for the Task 9 server test and to confirm Stage compiles).

- [ ] **Step 3: Commit**

```bash
git add src/modules/stage/src/Stage.svelte
git commit -m "feat(m10e-5): Stage drives token animation config from world-settings.animation"
```

---

### Task 9: Server integration test — gate-chaining invariant

**Files:**
- Create: `src/server/tests/movement_route_commit.rs` (or add a `#[tokio::test]` to an existing `ws`/room integration test module if the repo prefers in-crate tests — read `src/server/src/ws/room.rs` tests + `src/server/tests/` first and follow the established harness).

**Why:** The route-commit's correctness depends on a server invariant: separate serialized publishes chain (each `token_move` reads the pre-image from the *committed* doc, so publish N gates against publish N-1's committed cell). This test LOCKS that invariant so a future refactor can't silently break route-commit. No server source change — test only.

**Interfaces:**
- Consumes: the existing room/publish test harness (`movement_scene(...)`, `Room::publish`, `DataError::Forbidden`) in `room.rs` tests (lines ~960–1350 show the established pattern).

**Design notes:** Build a scene with `movementRestriction:"unrestricted"` (isolate the wall gate from the mask gate) and a `blocksMove` wall positioned so a straight `start→goal` crosses it but an L-route around it does not. Assert:
1. A single publish of `start→goal` (straight, crossing the wall) → `Err(Forbidden)`.
2. Two sequential publishes `start→corner` then `corner→goal` (the two collinear runs, neither crossing the wall) → both `Ok`, and the committed token ends at `goal`.

- [ ] **Step 1: Write the test** (adapt to the existing harness; sketch):

```rust
#[tokio::test]
async fn route_commits_as_chained_runs_around_a_wall() {
    // Scene grid 100, unrestricted; a blocksMove wall segment between the start row and the
    // goal so the straight diagonal crosses it but an L around the corner does not.
    let h = movement_scene_with_wall(/* wall crossing the straight path */).await;

    // 1. One straight op start->goal crosses the wall → Forbidden.
    let straight = h.publish_move(h.token, h.start, h.goal_straight).await;
    assert!(matches!(straight, Err(DataError::Forbidden)));

    // 2. Two chained runs around the corner → both Ok; committed at goal.
    assert!(h.publish_move(h.token, h.start, h.corner).await.is_ok());
    assert!(h.publish_move(h.token, h.corner, h.goal).await.is_ok());
    assert_eq!(h.committed_pos(h.token).await, h.goal);
}
```

> `publish_move(token, from, to)` issues one `Operation::Update` with `/system/x`+`/system/y` changes from the currently committed position. Model it on the existing `movement_scene` helpers (the `MovementHandle` already wires a `Room`, world-settings, scene, and token). Place the wall using the same wall-doc shape the M9a `movement_blocked_for_player_crossing_wall` test uses.

- [ ] **Step 2: Build client (rust-embed) then run the test**

Run: `pnpm --filter @shadowcat/ui build && cargo test -p shadowcat route_commits_as_chained_runs`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/server/tests/movement_route_commit.rs
git commit -m "test(m10e-5): lock gate-chaining — around-wall route commits as chained per-run publishes"
```

---

## Verification (end of plan, before review)

- [ ] `pnpm -r test` (all client packages green).
- [ ] `pnpm -r typecheck` (no type errors — esbuild masks these in per-task vitest).
- [ ] `pnpm --filter @shadowcat/ui build && cargo test -p shadowcat` (server suite green incl. Task 9).
- [ ] `cargo clippy -p shadowcat --all-targets -- -D warnings && cargo fmt --check` (if any Rust test file added).
- [ ] Manual reasoning pass: a route around a wall (a) animates smoothly along the bend locally, (b) commits as ≥2 chained intents, (c) ends at the goal authoritatively; an interrupting move (another player) retargets the in-flight walk.

---

## Self-Review (completed during authoring)

- **Spec §9 coverage:** duration = distance ÷ speed (Task 2), easeInOut default + linear (Task 1/2), config from `world-settings.animation` (Task 8), drives drag commits (Task 2/3 setTarget) AND routed moves along waypoints (Task 7 `animateAlongPath`), interruptible retarget (Task 2). Route-commit (user-chosen addition) = Tasks 6/7 + gate-chaining lock (Task 9). No protocol change (local animation + existing intents).
- **Type consistency:** `AnimationConfig` (Task 2) used by `setConfig` (Task 2/3/4); `animateAlongPath(id, path: [number,number][])` identical across animator → view → engine/SceneToolHost → bridge → tool; `EasingMode` from `./easing` in render, from `@shadowcat/core` at the Stage/config boundary (structurally identical string-union).
- **Placeholders:** none — every code step is concrete. Task 9's harness is sketched against the existing `movement_scene` pattern (the implementer reads the established helpers first, per the note).

---

## Buddy-check directives

**High-risk signal:** the route-commit (Tasks 6–7) introduces new code that issues **authoritative position writes** and depends on a subtle, security-adjacent server invariant (the M10e-4/M9 move-gate chaining). A run-decomposition bug, or a wrong assumption about publish serialization, could either reject valid moves or — worst case — be read as a way to slip a move past the gate (it cannot, since the server re-gates every op, but the interaction deserves adversarial eyes). The pure-animation tasks (1–5, 8) are client-cosmetic and low-risk.

**Directive:** at the whole-branch review checkpoint, run a **buddy-check** (two reviewers — `shadowcat-spec-reviewer` spec-lens + `shadowcat-code-reviewer` code-lens, blind round 1 → debate to convergence) focused on Tasks 6, 7, 9: collinear-run wall-clearness (transitivity claim), the per-op `old`-rechaining against the optimistic store, the `setTarget` ignore-vs-interrupt rule (no clobber of the walk, correct interrupt on foreign/rollback), and the gate-chaining test actually exercising the committed-pre-image path. Tasks 1–5 and 8 may take a single final review.
