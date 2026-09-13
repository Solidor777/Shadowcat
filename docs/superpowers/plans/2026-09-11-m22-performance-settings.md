# M22 · Performance settings + render budget — Implementation Plan

> **For agentic workers:** Execute task-by-task in order; each task's steps use checkbox
> (`- [ ]`) syntax. Written for a sonnet-class implementer with no conversation context —
> every path, symbol and test name below is exact; read the cited code before editing it.

**Goal:** Shadowcat runs acceptably on a phone. The user can cap the frame rate, lower the
render resolution, and switch off the expensive layers; the client picks a sane preset on its
own; the stage stops burning a core while nothing changes.

**Architecture:** `PerformanceSettings`/`PersistedPerformance`/`resolveAuto`/`effectiveSettings`
live once in `@shadowcat/core` (master §2.1, the seam every other Phase-3 milestone consumes);
`@shadowcat/ui-kit`'s `PerformanceController` mirrors `ThemeController`'s shape exactly
(`$state`-backed, `subscribe`/`load`/`serialize`, a module singleton plus a `createSubscriber`
reactive read) and is exposed as `AppContext.performance`; the shell persists it to
`localStorage` beside the theme mirror (per-device only — decision D1, never the server
`ui_state`); `@shadowcat/render`'s `RenderEngine` reads it through a `RenderEngineOpts.performance`
getter (the `viewedSceneId` pattern) and drives frame cap, render scale, dirty-flag idle-skip,
token-fx/lighting/reduced-motion budgets, pushing `onStats` back out; `Stage.svelte` wires the
getter in and re-creates the backend only when `antialias` changes (the one setting Pixi cannot
change post-init); `PerformanceEditor.svelte` (a built-in `Settings.svelte` section, not a
module) and `PerfStats.svelte` (a `statusbar` addition) are the two UI surfaces.

**Tech stack:** TypeScript, Svelte 5 (runes), Vitest (`@vitest-environment node` for every
non-DOM unit test), PixiJS v8 (`node_modules/pixi.js` — vendored source read for
`Ticker.remove`/`maxFPS`/`Application.render`), Playwright (written here, run by the dispatcher).

**Spec:** `docs/superpowers/specs/2026-09-11-m22-performance-settings-design.md` — read it
first; also read `docs/superpowers/specs/2026-09-11-phase3-master-integration-design.md` §0
(campaign directives), §2.1 (the seam this milestone owns), §3 (shared-file conventions), §4
(global constraints + gate battery), §5 (integration order), §6 (skills).

**Worktree:** `C:/Dev/Shadowcat-m22`, branch `m22-performance`. M22 is first in the merge order
(master §5) and consumes no foreign seam — every task before the last runs immediately, in order
(each depends on the previous task's new files); the last task is the (short) merge-forward.

## Execution directives

**Every dispatched agent's first prompt MUST contain this paragraph verbatim:**

> The iron rule is no deferrals of existing work, or new work as it comes up - we fix this now
> unless I give my EXPRESS authorization. The only exception is if a bug or to-do has a genuine
> blocker that is already logged in a milestone in PLAN.md that has not been started yet. Another
> iron clad is rule is that when faced with a design fork, determine the best long term shape in
> keeping with our plans and goals, and implement accordingly. You only need to ask me if the
> question "what is the best long term shape in keeping with our plans and goals?" is not able to
> answer the question. Churn is not a concern. This paragraph must be copied verbatim to any
> agents dispatched in this campaign.

**Reporting rule:** a subagent delivers its report as the Agent tool result, via `SendMessage`
to the dispatcher, or by writing a named file; the dispatching prompt states which. An agent
given a `name` never returns a result — omit `name` for every dispatch whose report is needed.

**Opus is banned** for every dispatch in this campaign — sonnet only.

## Model/Effort directives

- Implementation dispatches: `shadowcat-codebase:shadowcat-coder`, model sonnet, effort medium.
- Review dispatches: `shadowcat-codebase:shadowcat-spec-reviewer` +
  `shadowcat-codebase:shadowcat-code-reviewer`, both sonnet, effort high, as a pair.
- Escalation (a coder reports BLOCKED, or a reviewer's findings read shallow/uncertain): the
  `-fable` twins of the same two agents — never the `-opus` twins, never bare `opus`.
- Every dispatch states its effort explicitly; an unspecified effort silently inherits the
  session's, which defeats this tiering.

## Buddy-check directives

Before the PR (last task's final step), the dispatcher pre-generates the WHOLE branch diff
(`git diff main...HEAD` from the worktree, saved to a file — reviewers have no Bash tool and
cannot generate it themselves) and dispatches the spec-reviewer + code-reviewer pair against
that diff, blind to each other, per the master's §10 verification convention. A finding either
reviewer raises that the dispatcher disagrees with is a design question for the user, not a
silent override. Only after both reviewers pass (or their findings are applied and re-verified)
does the dispatcher run the full gate battery and `pnpm gate:push` before the PR.

## Global constraints

(Copied verbatim from the master spec §4 — every milestone plan inherits these.)

- No lint suppressions of any kind (`#[allow]`, `#[expect]`, `eslint-disable`, `@ts-ignore`);
  `pnpm lint:allowances` is a gate. Fix the code.
- File-size: 5,000-line soft limit needs the owner's allowlist signature, 10,000 hard; Rust
  test bodies in sibling files (`pnpm lint:file-size`, `pnpm lint:inline-tests`).
- Comments cite symbols, never files/lines; no milestone ids, dates, sweep markers or history
  narration in `.ts`/`.rs`/`.svelte` (`pnpm lint:comments`).
- Every new `.ts` unit test that never touches the DOM opens with `// @vitest-environment node`.
- Deletion only through `trash`; never `rm`/`Remove-Item`/`git rm` as the sole step.
- Commits name their paths: `git commit -m "..." -- <paths>`; never `git add -A`.
- Long commands (`cargo test --all`, `pnpm -r test`, `pnpm build:all`) run in the background
  with output to a log file; read the log before claiming green.
- **Cross-platform:** `std::path` only; `#[cfg]`-gated OS code has an implementation for every
  target the matrix builds (Linux, macOS, Windows); responsive + touch-sized UI.
- **Licenses:** MIT / Apache-2.0 / BSD / zlib / MPL-2.0 only; media codecs royalty-free. Every
  new dependency lands with a `Cargo.toml`/`package.json` comment naming its license. (M22 adds
  no new dependency.)
- **Binary size:** `pnpm lint:binary-size` guards the 60 MiB release binary.
- **Server by default:** computation runs on the server; client-side work needs a reason
  (presentation, input capture, optimistic prediction). `docs/design/ARCHITECTURE.md` §2
  invariants 1, 6 and 11 govern every design fork. (Performance settings are a legitimate
  client-side/per-device exception — decision D1 — since the server has no notion of "this
  device's GPU.")
- **UX outranks data secrecy** (invariant 11): send-then-hide is acceptable; PII and
  remote-device security are the two ironclad exceptions.
- `pnpm build` precedes any cargo build (rust-embed validates `dist/` at compile time).
- The Playwright suite is DISPATCHER-run on port 31999 (one suite at a time on the machine);
  this milestone WRITES its spec and runs unit/integration tests itself.

The full gate battery this milestone must show green before its PR: `cargo test --all`,
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo clippy -- -D
missing-docs -D clippy::missing-docs-in-private-items`, `git diff --exit-code
src/types/generated` after regen (M22 touches no Rust type, so this is a no-op check), `pnpm -r
typecheck`, `pnpm -r test`, `pnpm build`, `pnpm lint`, `lint:docs`, `lint:props`,
`lint:comments`, `lint:allowances`, `lint:file-size`, `lint:inline-tests`, `lint:aria-labels`,
`lint:gate-manifest`, `lint:settings-privacy`, `lint:binary-size` (release build), `pnpm
docs:check-examples`, `pnpm docs:check-rust-examples`, `pnpm run test:scripts`, `pnpm run
check:svelte-runtime`, `pnpm --filter "shadowcat-example-*" build`, `pnpm --filter
@shadowcat/core test:e2e`, and `pnpm gate:push` (tree-keyed receipt) immediately before `git
push`.

## A note on a naming hazard this milestone introduces

The ui-kit singleton is exported as `performanceController` — NOT `performance` — so that no
module importing it shadows the ambient `Performance` global (`performance.now()`). The
spec-fixed `AppContext.performance` member (master §2.1) keeps its name; `Table.svelte` binds
`performance: performanceController`. The ONE remaining shadow is a component that
destructures `const { performance } = getAppContext()` (Tasks 9–10): inside such a file the
real Performance API is `globalThis.performance`, never the bare identifier. No file this plan
touches calls `performance.now()` (verified: `rg "performance\.now\(\)|performance\.memory"
src` returns nothing). State both facts in the new `shadowcat-codebase-performance` skill's
Gotchas section (Task 12).

---

## Task 1: `@shadowcat/core` — `performance.ts`

**Files:**
- Create: `src/client/core/src/performance.ts`, `src/client/core/src/performance.test.ts`
  (`// @vitest-environment node`).
- Modify: `src/client/core/src/index.ts` (export, appended at the end after the `templates`
  exports).

**Interfaces (Produces):**
```ts
export interface PerformanceSettings {
  fpsCap: 30 | 60 | 120 | "uncapped";
  renderScale: number;
  antialias: boolean;
  tokenFx: boolean;
  lighting: "full" | "static" | "off";
  vfx: boolean;
  dice3d: boolean;
  spatialAudio: boolean;
  idleSkip: boolean;
  reducedMotion: boolean;
}
export type PerformancePreset = "auto" | "mobile" | "balanced" | "quality" | "custom";
export interface DeviceSignals {
  coarsePointer?: boolean;
  compact?: boolean;
  hardwareConcurrency?: number;
  deviceMemoryGb?: number;
  reducedMotion?: boolean;
}
export const PRESETS: Record<Exclude<PerformancePreset, "auto" | "custom">, PerformanceSettings>;
export function resolveAuto(signals: DeviceSignals): PerformanceSettings;
export interface PersistedPerformance {
  preset: PerformancePreset;
  overrides: Partial<PerformanceSettings>;
}
export function parsePersisted(raw: string | null): PersistedPerformance;
export function serializePersisted(p: PersistedPerformance): string;
export function effectiveSettings(p: PersistedPerformance, signals: DeviceSignals): PerformanceSettings;
export const PERFORMANCE_STORAGE_KEY = "shadowcat.performance";
```

**Step 1 — failing test, then implement.** Write `performance.test.ts`:
```ts
// @vitest-environment node
import { describe, it, expect } from "vitest";
import {
  PRESETS,
  resolveAuto,
  parsePersisted,
  serializePersisted,
  effectiveSettings,
  PERFORMANCE_STORAGE_KEY,
  type DeviceSignals,
  type PersistedPerformance,
} from "./performance";

describe("resolveAuto", () => {
  it("resolves balanced with no signals", () => {
    expect(resolveAuto({})).toEqual(PRESETS.balanced);
  });
  it("resolves mobile when coarsePointer AND compact", () => {
    expect(resolveAuto({ coarsePointer: true, compact: true })).toMatchObject({ fpsCap: 30, renderScale: 0.75 });
  });
  it("does not resolve mobile from coarsePointer alone (compact absent)", () => {
    expect(resolveAuto({ coarsePointer: true })).toEqual(PRESETS.balanced);
  });
  it("resolves mobile when hardwareConcurrency <= 4", () => {
    expect(resolveAuto({ hardwareConcurrency: 4 })).toMatchObject({ fpsCap: 30 });
  });
  it("does not resolve mobile when hardwareConcurrency > 4", () => {
    expect(resolveAuto({ hardwareConcurrency: 8 })).toEqual(PRESETS.balanced);
  });
  it("resolves mobile when deviceMemoryGb <= 4", () => {
    expect(resolveAuto({ deviceMemoryGb: 4 })).toMatchObject({ fpsCap: 30 });
  });
  it("ORs reducedMotion onto the resolved preset", () => {
    expect(resolveAuto({ reducedMotion: true })).toEqual({ ...PRESETS.balanced, reducedMotion: true });
  });
});

describe("parsePersisted", () => {
  it("returns the auto default for null", () => {
    expect(parsePersisted(null)).toEqual({ preset: "auto", overrides: {} });
  });
  it("returns the auto default for unparsable JSON", () => {
    expect(parsePersisted("not json")).toEqual({ preset: "auto", overrides: {} });
  });
  it("returns the auto default for a non-object payload", () => {
    expect(parsePersisted("42")).toEqual({ preset: "auto", overrides: {} });
  });
  it("falls back preset to auto when unresolvable, keeps valid overrides", () => {
    expect(parsePersisted(JSON.stringify({ preset: "bogus", overrides: { fpsCap: 60 } })))
      .toEqual({ preset: "auto", overrides: { fpsCap: 60 } });
  });
  it("drops a single bad override key and keeps the rest", () => {
    const raw = JSON.stringify({
      preset: "custom",
      overrides: { fpsCap: 999, renderScale: 0.8, antialias: "yes" },
    });
    expect(parsePersisted(raw)).toEqual({ preset: "custom", overrides: { renderScale: 0.8 } });
  });
  it("clamps renderScale on parse", () => {
    const raw = JSON.stringify({ preset: "custom", overrides: { renderScale: 5 } });
    expect(parsePersisted(raw).overrides.renderScale).toBe(1);
  });
});

describe("effectiveSettings", () => {
  const signals: DeviceSignals = {};
  it("resolves auto through resolveAuto", () => {
    expect(effectiveSettings({ preset: "auto", overrides: {} }, signals)).toEqual(resolveAuto(signals));
  });
  it("resolves a named preset verbatim", () => {
    expect(effectiveSettings({ preset: "quality", overrides: {} }, signals)).toEqual(PRESETS.quality);
  });
  it("custom overrides win over the balanced fallback base", () => {
    const p: PersistedPerformance = { preset: "custom", overrides: { fpsCap: 120 } };
    expect(effectiveSettings(p, signals)).toEqual({ ...PRESETS.balanced, fpsCap: 120 });
  });
  it("ORs the live reducedMotion signal onto every preset", () => {
    expect(effectiveSettings({ preset: "quality", overrides: {} }, { reducedMotion: true }).reducedMotion).toBe(true);
  });
  it("clamps renderScale at read time regardless of source", () => {
    const p: PersistedPerformance = { preset: "custom", overrides: { renderScale: 0.1 } };
    expect(effectiveSettings(p, signals).renderScale).toBe(0.5);
  });
});

it("serializePersisted round-trips through parsePersisted", () => {
  const p: PersistedPerformance = { preset: "custom", overrides: { fpsCap: 30, idleSkip: false } };
  expect(parsePersisted(serializePersisted(p))).toEqual(p);
});

it("PERFORMANCE_STORAGE_KEY is the expected literal", () => {
  expect(PERFORMANCE_STORAGE_KEY).toBe("shadowcat.performance");
});
```

Implement `performance.ts`:
```ts
/** Per-device render-budget settings — the seam every render/audio consumer (`RenderEngine`,
 * `VfxView`, the dice overlay, the audio mixer) reads. Persisted in `localStorage` only, never
 * the server `ui_state`: a phone and a desktop want different budgets, and `ui_state` is
 * per-account. */
export interface PerformanceSettings {
  /** Frame-rate cap for the stage ticker; `"uncapped"` maps to Pixi's `Ticker.maxFPS = 0`. */
  fpsCap: 30 | 60 | 120 | "uncapped";
  /** Renderer resolution as a fraction of devicePixelRatio, clamped to `[0.5, 1]` at every read. */
  renderScale: number;
  /** Multisampling — fixed at Pixi init; changing it re-creates the render backend (`Stage.svelte`). */
  antialias: boolean;
  /** Per-token ColorMatrix fx (tint/desaturate/highlight) — off ⇒ condition-driven filters never
   * build; the selection highlight is exempt (it is the one signifier, bounded by selection size). */
  tokenFx: boolean;
  /** Lighting overlay quality: `"full"` (per-frame photometric sweeps), `"static"` (a carried-light
   * sweep snaps straight to its committed end frame, no per-tick interpolation), `"off"` (the
   * cosmetic overlay never paints — fog/vision secrecy is untouched; this never affects what a
   * player can or cannot see, only the darkening/tint cosmetic on top of it). */
  lighting: "full" | "static" | "off";
  /** VFX layer — off ⇒ no `VfxView` reconcile, no one-shot playback. */
  vfx: boolean;
  /** 3D dice overlay — off ⇒ chat card only. */
  dice3d: boolean;
  /** Spatial audio — off ⇒ every emitter mixes flat at channel gain. */
  spatialAudio: boolean;
  /** Redraw only when something changed (dirty-flag rendering) — see `RenderEngine`'s ticker. */
  idleSkip: boolean;
  /** Honour `prefers-reduced-motion`: token tweens and sample playback snap to their end pose
   * immediately; light/vision sweep cross-fades apply the incoming sample in one step. */
  reducedMotion: boolean;
}

/** A named budget, or `"auto"` (resolved from `DeviceSignals` via `resolveAuto`) or `"custom"`
 * (the user edited at least one field away from a named preset — see `PersistedPerformance`). */
export type PerformancePreset = "auto" | "mobile" | "balanced" | "quality" | "custom";

/** Device signals `resolveAuto` reads. Every field optional so a test can pin any subset, and so
 * an environment missing a probe (no `matchMedia`, no `navigator.deviceMemory` — Chromium-only)
 * degrades to "unknown" rather than a synthesized false value. */
export interface DeviceSignals {
  /** `matchMedia("(pointer: coarse)").matches` — a touch-primary pointing device. */
  coarsePointer?: boolean;
  /** `@shadowcat/ui-kit`'s `sizeClass() === "compact"` — a narrow viewport. */
  compact?: boolean;
  /** `navigator.hardwareConcurrency`. */
  hardwareConcurrency?: number;
  /** `navigator.deviceMemory`, in GiB — Chromium-only; absent on Firefox/Safari. */
  deviceMemoryGb?: number;
  /** `matchMedia("(prefers-reduced-motion: reduce)").matches`. */
  reducedMotion?: boolean;
}

/** The three named budgets. `"auto"`/`"custom"` are resolved dynamically (`resolveAuto`,
 * `effectiveSettings`) and have no static entry here. */
export const PRESETS: Record<Exclude<PerformancePreset, "auto" | "custom">, PerformanceSettings> = {
  mobile: {
    fpsCap: 30, renderScale: 0.75, antialias: false, tokenFx: false, lighting: "static",
    vfx: false, dice3d: false, spatialAudio: true, idleSkip: true, reducedMotion: false,
  },
  balanced: {
    fpsCap: 60, renderScale: 1, antialias: true, tokenFx: true, lighting: "full",
    vfx: true, dice3d: true, spatialAudio: true, idleSkip: true, reducedMotion: false,
  },
  quality: {
    fpsCap: "uncapped", renderScale: 1, antialias: true, tokenFx: true, lighting: "full",
    vfx: true, dice3d: true, spatialAudio: true, idleSkip: true, reducedMotion: false,
  },
};

/** Clamps a render-scale value to the supported range — applied at every read site
 * (`resolveAuto`, `effectiveSettings`, `parsePersisted`'s override sanitizer) so a garbage or
 * out-of-range persisted value can never reach the backend.
 * @param scale The candidate render scale.
 * @returns `scale` clamped to `[0.5, 1]`.
 * @example
 * ```ts
 * clampRenderScale(5); // 1
 * ```
 */
function clampRenderScale(scale: number): number {
  return Math.min(1, Math.max(0.5, scale));
}

/** Resolves `"auto"` against live device signals: `"mobile"` when `(coarsePointer && compact) ||
 * hardwareConcurrency <= 4 || deviceMemoryGb <= 4`, else `"balanced"`. `reducedMotion` is always
 * OR-ed onto the resolved preset's own value (`false` in every `PRESETS` entry) — an accessibility
 * signal is never gated behind switching to `"custom"`.
 * @param signals The device signals to resolve against.
 * @returns The resolved effective settings.
 * @example
 * ```ts
 * resolveAuto({ hardwareConcurrency: 4 }).fpsCap; // 30
 * ```
 */
export function resolveAuto(signals: DeviceSignals): PerformanceSettings {
  const mobile =
    (signals.coarsePointer === true && signals.compact === true) ||
    (signals.hardwareConcurrency !== undefined && signals.hardwareConcurrency <= 4) ||
    (signals.deviceMemoryGb !== undefined && signals.deviceMemoryGb <= 4);
  const base = mobile ? PRESETS.mobile : PRESETS.balanced;
  return { ...base, reducedMotion: base.reducedMotion || (signals.reducedMotion ?? false) };
}

/** The persisted shape: the active preset, plus the override fields the user edited away from
 * it. For `"custom"`, `overrides` is written as the FULL resulting settings object by
 * `PerformanceController.set` (never a bare single-field patch) — see that method's doc — but
 * `parsePersisted` still validates it field-by-field, since a hand-edited or older-format
 * `localStorage` blob may carry a genuinely partial map. */
export interface PersistedPerformance {
  /** The active preset selector. */
  preset: PerformancePreset;
  /** The user's edited fields; empty for a named (non-`"custom"`) preset. */
  overrides: Partial<PerformanceSettings>;
}

const PRESET_IDS: ReadonlySet<PerformancePreset> = new Set(["auto", "mobile", "balanced", "quality", "custom"]);
const FPS_CAPS: ReadonlySet<PerformanceSettings["fpsCap"]> = new Set([30, 60, 120, "uncapped"]);
const LIGHTING_MODES: ReadonlySet<PerformanceSettings["lighting"]> = new Set(["full", "static", "off"]);

/** Validates one persisted override map field-by-field, dropping any unknown key or
 * wrong-typed/unresolvable value while keeping every other key — a single bad field never
 * invalidates the whole record (fail-closed per key, not per record).
 * @param value The candidate `overrides` value from a parsed JSON payload.
 * @returns The validated subset.
 * @example
 * ```
 * // internal helper; not part of the public API
 * sanitizeOverrides({ fpsCap: 999, renderScale: 0.8 }); // { renderScale: 0.8 }
 * ```
 */
function sanitizeOverrides(value: unknown): Partial<PerformanceSettings> {
  if (typeof value !== "object" || value === null) return {};
  const src = value as Record<string, unknown>;
  const out: Partial<PerformanceSettings> = {};
  if (FPS_CAPS.has(src.fpsCap as PerformanceSettings["fpsCap"])) out.fpsCap = src.fpsCap as PerformanceSettings["fpsCap"];
  if (typeof src.renderScale === "number" && Number.isFinite(src.renderScale)) out.renderScale = clampRenderScale(src.renderScale);
  if (typeof src.antialias === "boolean") out.antialias = src.antialias;
  if (typeof src.tokenFx === "boolean") out.tokenFx = src.tokenFx;
  if (LIGHTING_MODES.has(src.lighting as PerformanceSettings["lighting"])) out.lighting = src.lighting as PerformanceSettings["lighting"];
  if (typeof src.vfx === "boolean") out.vfx = src.vfx;
  if (typeof src.dice3d === "boolean") out.dice3d = src.dice3d;
  if (typeof src.spatialAudio === "boolean") out.spatialAudio = src.spatialAudio;
  if (typeof src.idleSkip === "boolean") out.idleSkip = src.idleSkip;
  if (typeof src.reducedMotion === "boolean") out.reducedMotion = src.reducedMotion;
  return out;
}

/** Parses + validates a persisted `PersistedPerformance` blob. Fails closed to `{ preset: "auto",
 * overrides: {} }` for `null`, unparsable JSON, or a non-object payload; an unresolvable `preset`
 * string falls back to `"auto"` while any still-valid `overrides` entries survive; each override
 * field is validated independently (`sanitizeOverrides`).
 * @param raw The raw persisted string (from `localStorage.getItem`), or `null` if absent.
 * @returns The validated persisted state — never throws.
 * @example
 * ```ts
 * parsePersisted(null); // { preset: "auto", overrides: {} }
 * ```
 */
export function parsePersisted(raw: string | null): PersistedPerformance {
  if (raw === null) return { preset: "auto", overrides: {} };
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return { preset: "auto", overrides: {} };
  }
  if (typeof value !== "object" || value === null) return { preset: "auto", overrides: {} };
  const src = value as Record<string, unknown>;
  const preset = PRESET_IDS.has(src.preset as PerformancePreset) ? (src.preset as PerformancePreset) : "auto";
  return { preset, overrides: sanitizeOverrides(src.overrides) };
}

/** Serializes a `PersistedPerformance` for storage. The exact inverse of `parsePersisted` for
 * any value `parsePersisted` itself could have produced.
 * @param p The persisted state to serialize.
 * @returns The JSON string to store.
 * @example
 * ```ts
 * serializePersisted({ preset: "auto", overrides: {} }); // '{"preset":"auto","overrides":{}}'
 * ```
 */
export function serializePersisted(p: PersistedPerformance): string {
  return JSON.stringify(p);
}

/** The ONE place preset + overrides + device signals combine into effective settings:
 * `"auto"` resolves through `resolveAuto`; a named preset returns `PRESETS[preset]` verbatim;
 * `"custom"` layers `overrides` onto the `balanced` preset as a fallback base (covering a
 * hand-edited or partial persisted `overrides` map — `PerformanceController.set` itself always
 * writes a full object, so this fallback is a defensive floor, not the normal path). The live
 * `reducedMotion` signal is OR-ed onto whichever base was chosen, and `renderScale` is
 * re-clamped regardless of source. `PerformanceEditor` and the shell's `main.ts` load call both
 * go through this — nothing re-derives it.
 * @param p The persisted preset + overrides.
 * @param signals The device signals `"auto"`/`reducedMotion` resolve against.
 * @returns The resolved effective settings.
 * @example
 * ```ts
 * effectiveSettings({ preset: "mobile", overrides: {} }, {}).fpsCap; // 30
 * ```
 */
export function effectiveSettings(p: PersistedPerformance, signals: DeviceSignals): PerformanceSettings {
  const base: PerformanceSettings =
    p.preset === "auto" ? resolveAuto(signals)
    : p.preset === "custom" ? ({ ...PRESETS.balanced, ...p.overrides } as PerformanceSettings)
    : PRESETS[p.preset];
  return {
    ...base,
    renderScale: clampRenderScale(base.renderScale),
    reducedMotion: base.reducedMotion || (signals.reducedMotion ?? false),
  };
}

/** The `localStorage` key the shell's `readPerformanceMirror`/`writePerformanceMirror`
 * (`src/client/shell/src/lib/sessionState.svelte.ts`) read/write under. */
export const PERFORMANCE_STORAGE_KEY = "shadowcat.performance";
```

- [ ] **Step 1:** write the test file above (it fails on the missing module); implement
  `performance.ts`; `pnpm --filter @shadowcat/core test`, `pnpm -r typecheck`, `pnpm lint`,
  `pnpm lint:docs`, `pnpm lint:comments`, `pnpm docs:check-examples` PASS.
- [ ] **Step 2:** export from `src/client/core/src/index.ts` — append after the `templates`
  export block:
  ```ts
  export { PRESETS, resolveAuto, parsePersisted, serializePersisted, effectiveSettings, PERFORMANCE_STORAGE_KEY } from "./performance";
  export type { PerformanceSettings, PerformancePreset, DeviceSignals, PersistedPerformance } from "./performance";
  ```
  `pnpm -r typecheck` PASS.
- [ ] **Step 3:** `git commit -m "feat(core): PerformanceSettings, presets, and auto-resolution" -- src/client/core/src/performance.ts src/client/core/src/performance.test.ts src/client/core/src/index.ts`

## Task 2: `@shadowcat/ui-kit` — `performance.svelte.ts`

**Files:**
- Create: `src/client/ui-kit/src/performance.svelte.ts`, `src/client/ui-kit/src/performance.svelte.test.ts`.
- Modify: `src/client/ui-kit/src/index.ts` (export, appended after the `theme.svelte` export
  block per that file's existing grouping).

**Interfaces (Consumes):** `PerformanceSettings`, `PerformancePreset`, `DeviceSignals`,
`PersistedPerformance`, `effectiveSettings` from `@shadowcat/core` (Task 1).

**Interfaces (Produces):**
```ts
export interface PerformanceStats { fps: number; frameMs: number }
export type PerformanceListener = () => void;
export class PerformanceController {
  get preset(): PerformancePreset;
  get current(): PerformanceSettings;
  get stats(): PerformanceStats;
  get showStats(): boolean;
  onChange?: (p: PersistedPerformance) => void;
  setShowStats(on: boolean): void;
  recordStats(stats: PerformanceStats): void;
  set(patch: Partial<PerformanceSettings>): void;
  setPreset(p: PerformancePreset): void;
  load(parsed: PersistedPerformance | undefined, signals: DeviceSignals): void;
  serialize(): PersistedPerformance;
  subscribe(listener: PerformanceListener): () => void;
}
export const performance: PerformanceController;
export function activePerformance(): PerformanceSettings;
```

Read `src/client/ui-kit/src/theme.svelte.ts` in full before writing this file — the shape below
mirrors its `$state`/getter/`#changed()`/singleton/`createSubscriber` pattern exactly.

**Step 1 — failing test, then implement.** Write `performance.svelte.test.ts`:
```ts
import { describe, it, expect, vi } from "vitest";
import { PerformanceController } from "./performance.svelte";
import { PRESETS } from "@shadowcat/core";

describe("PerformanceController", () => {
  it("defaults to auto/balanced with no signals", () => {
    const c = new PerformanceController();
    expect(c.preset).toBe("auto");
    expect(c.current).toEqual(PRESETS.balanced);
  });

  it("load(undefined, signals) resolves auto against the given signals", () => {
    const c = new PerformanceController();
    c.load(undefined, { hardwareConcurrency: 2 });
    expect(c.preset).toBe("auto");
    expect(c.current).toMatchObject({ fpsCap: 30 });
  });

  it("load(parsed, signals) restores a named preset", () => {
    const c = new PerformanceController();
    c.load({ preset: "quality", overrides: {} }, {});
    expect(c.preset).toBe("quality");
    expect(c.current).toEqual(PRESETS.quality);
  });

  it("setPreset switches preset and clears overrides", () => {
    const c = new PerformanceController();
    c.set({ fpsCap: 30 });
    expect(c.preset).toBe("custom");
    c.setPreset("mobile");
    expect(c.preset).toBe("mobile");
    expect(c.current).toEqual(PRESETS.mobile);
  });

  it("set moves preset to custom, carrying the full current object forward", () => {
    const c = new PerformanceController();
    c.setPreset("quality");
    c.set({ fpsCap: 30 });
    expect(c.preset).toBe("custom");
    expect(c.current).toEqual({ ...PRESETS.quality, fpsCap: 30 });
  });

  it("set fires onChange with the serialized state", () => {
    const c = new PerformanceController();
    const onChange = vi.fn();
    c.onChange = onChange;
    c.set({ idleSkip: false });
    expect(onChange).toHaveBeenCalledWith({ preset: "custom", overrides: { ...PRESETS.balanced, idleSkip: false } });
  });

  it("setShowStats does not fire onChange (not part of PersistedPerformance)", () => {
    const c = new PerformanceController();
    const onChange = vi.fn();
    c.onChange = onChange;
    c.setShowStats(true);
    expect(c.showStats).toBe(true);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("recordStats updates stats without firing onChange or subscribers", () => {
    const c = new PerformanceController();
    const onChange = vi.fn();
    const listener = vi.fn();
    c.onChange = onChange;
    c.subscribe(listener);
    c.recordStats({ fps: 60, frameMs: 4 });
    expect(c.stats).toEqual({ fps: 60, frameMs: 4 });
    expect(onChange).not.toHaveBeenCalled();
    expect(listener).not.toHaveBeenCalled();
  });

  it("subscribe notifies on set/setPreset/load", () => {
    const c = new PerformanceController();
    const listener = vi.fn();
    const unsubscribe = c.subscribe(listener);
    c.setPreset("mobile");
    expect(listener).toHaveBeenCalledTimes(1);
    unsubscribe();
    c.setPreset("quality");
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("serialize round-trips through load", () => {
    const c = new PerformanceController();
    c.set({ fpsCap: 30 });
    const snap = c.serialize();
    const c2 = new PerformanceController();
    c2.load(snap, {});
    expect(c2.current).toEqual(c.current);
  });
});
```

Implement `performance.svelte.ts`:
```ts
// The PerformanceController singleton and its Svelte adapter, mirroring `theme.svelte.ts`'s
// shape exactly: a `$state`-backed controller with subscribe/snapshot reactivity, plus a
// `createSubscriber`-backed reactive read for components. The controller never touches
// `Storage` — the shell persists through `onChange` (`writePerformanceMirror`), per-device only:
// never the server `ui_state`.
import { createSubscriber } from "svelte/reactivity";
import {
  PRESETS,
  effectiveSettings,
  type DeviceSignals,
  type PerformancePreset,
  type PerformanceSettings,
  type PersistedPerformance,
} from "@shadowcat/core";

/** Live frame-rate/frame-time readout, pushed by `RenderEngine`'s `onStats` hook at most 4×/s.
 * `{ fps: 0, frameMs: 0 }` before the stage renders its first sample. */
export interface PerformanceStats {
  /** Ticker-rate frames per second, EMA-smoothed over ~30 ticks. */
  fps: number;
  /** Milliseconds the last actual `DisplayBackend.render()` call took. */
  frameMs: number;
}

/** A no-argument callback invoked after any performance state change (excluding `stats`, which
 * is high-frequency and read reactively instead — see `recordStats`). */
export type PerformanceListener = () => void;

/** Owns the effective performance settings, the active preset, live frame stats, and the
 * statusbar stats-readout toggle. Framework-neutral consumers use `subscribe`; Svelte consumers
 * read `current`/`preset`/`stats`/`showStats` directly (each backed by `$state`) or through the
 * module-level `activePerformance()` adapter. */
export class PerformanceController {
  /** Backing store for {@link PerformanceController.preset}. */
  #preset = $state<PerformancePreset>("auto");
  /** Backing store for the persisted overrides half of `PersistedPerformance`. */
  #overrides = $state<Partial<PerformanceSettings>>({});
  /** Device signals last passed to `load` — re-read by every `current`/`set`/`setPreset`
   * resolution so `"auto"` always resolves against the CURRENT device, not a stale snapshot from
   * whenever `load` last ran. Plain field, not `$state`: it changes only via `load`, which
   * already triggers a full re-render through `#preset`/`#overrides`. */
  #signals: DeviceSignals = {};
  /** Backing store for {@link PerformanceController.stats}. */
  #stats = $state<PerformanceStats>({ fps: 0, frameMs: 0 });
  /** Backing store for {@link PerformanceController.showStats}. */
  #showStats = $state(false);
  /** Subscribers notified after a persisted-state change (never after `recordStats`). */
  #listeners = new Set<PerformanceListener>();

  /** Called after every `set`/`setPreset`/`load` change with the new serialized state — the
   * shell's persistence hook (`sessionState.svelte.ts` writes it to the `localStorage` mirror).
   * Never fired by `setShowStats`/`recordStats` — neither is part of `PersistedPerformance`. */
  onChange?: (p: PersistedPerformance) => void;

  /** The active preset, or `"custom"` once `set` has edited any field.
   * @returns The active preset selector. */
  get preset(): PerformancePreset {
    return this.#preset;
  }

  /** The resolved effective settings — see `effectiveSettings`, the one place preset +
   * overrides + device signals combine. Re-derived on every read, never cached, so a signal
   * change alone (without a `set`/`setPreset`/`load` call) is not currently observable through
   * this getter — device signals are read once at `load` time in this milestone.
   * @returns The current effective `PerformanceSettings`. */
  get current(): PerformanceSettings {
    return effectiveSettings({ preset: this.#preset, overrides: this.#overrides }, this.#signals);
  }

  /** The live frame-rate/frame-time readout.
   * @returns The last `recordStats` sample. */
  get stats(): PerformanceStats {
    return this.#stats;
  }

  /** Whether the statusbar's frame-stats readout (`PerfStats.svelte`) is visible. Deliberately
   * NOT part of `PersistedPerformance` (never persisted; resets to `false` each session) — a
   * transient display preference, not a render-budget setting.
   * @returns Whether the readout is visible. */
  get showStats(): boolean {
    return this.#showStats;
  }

  /** Toggles the statusbar frame-stats readout. Notifies subscribers; never `onChange` (not
   * persisted — see the field doc on {@link PerformanceController.showStats}).
   * @param on Whether the readout should be visible.
   * @example
   * ```ts
   * import { performanceController } from "@shadowcat/ui-kit";
   *
   * performance.setShowStats(true);
   * ```
   */
  setShowStats(on: boolean): void {
    this.#showStats = on;
    this.#notify();
  }

  /** Records the latest stats sample. Deliberately does NOT notify subscribers or fire
   * `onChange` — a component reading `stats` directly observes the `$state` write natively
   * (Svelte's own reactivity), and firing the persistence hook up to 4×/s for a value that is
   * never persisted would be pure waste.
   * @param stats The latest sample, from `RenderEngine`'s `onStats` hook.
   * @example
   * ```ts
   * import { performanceController } from "@shadowcat/ui-kit";
   *
   * performance.recordStats({ fps: 60, frameMs: 4 });
   * ```
   */
  recordStats(stats: PerformanceStats): void {
    this.#stats = stats;
  }

  /** Applies `patch` on top of the CURRENT effective settings and moves `preset` to `"custom"`
   * — `overrides` becomes the FULL resulting settings object, never a bare patch, so a later
   * `effectiveSettings` resolution for this device never needs to fall back to a base preset for
   * an untouched field.
   * @param patch The fields to change.
   * @example
   * ```ts
   * import { performanceController } from "@shadowcat/ui-kit";
   *
   * performance.set({ fpsCap: 30 });
   * ```
   */
  set(patch: Partial<PerformanceSettings>): void {
    this.#overrides = { ...this.current, ...patch };
    this.#preset = "custom";
    this.#changed();
  }

  /** Switches to `preset`, clearing any custom overrides.
   * @param p The preset to activate.
   * @example
   * ```ts
   * import { performanceController } from "@shadowcat/ui-kit";
   *
   * performance.setPreset("mobile");
   * ```
   */
  setPreset(p: PerformancePreset): void {
    this.#preset = p;
    this.#overrides = {};
    this.#changed();
  }

  /** Replaces the whole persisted state from a mirror read and records the device signals
   * `"auto"`/`reducedMotion` resolve against from now on (until the next `load`).
   * @param parsed The persisted state, or `undefined` when no mirror was saved.
   * @param signals The device signals to resolve `"auto"`/`reducedMotion` against.
   * @example
   * ```ts
   * import { performanceController } from "@shadowcat/ui-kit";
   *
   * performanceController.load(undefined, {});
   * ```
   */
  load(parsed: PersistedPerformance | undefined, signals: DeviceSignals): void {
    this.#signals = signals;
    this.#preset = parsed?.preset ?? "auto";
    this.#overrides = parsed?.overrides ?? {};
    this.#changed();
  }

  /** The persisted shape: the active preset plus its overrides.
   * @returns A snapshot suitable for persistence.
   * @example
   * ```ts
   * import { performanceController } from "@shadowcat/ui-kit";
   *
   * const state = performance.serialize();
   * ```
   */
  serialize(): PersistedPerformance {
    return { preset: this.#preset, overrides: { ...this.#overrides } };
  }

  /** Notifies `listener` after every persisted-state change (never after `recordStats`).
   * @param listener Called with no arguments after a change.
   * @returns An unsubscribe function.
   * @example
   * ```ts
   * import { performanceController } from "@shadowcat/ui-kit";
   *
   * const unsubscribe = performance.subscribe(() => {});
   * unsubscribe();
   * ```
   */
  subscribe(listener: PerformanceListener): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  /** Notifies every `subscribe`r, with no `onChange` call.
   * @example
   * ```
   * // internal helper; not part of the public API
   * this.#notify();
   * ```
   */
  #notify(): void {
    for (const fn of this.#listeners) fn();
  }

  /** Fires `onChange` (the persistence hook) with the serialized state, then notifies every
   * `subscribe`r.
   * @example
   * ```
   * // internal helper; not part of the public API
   * this.#changed();
   * ```
   */
  #changed(): void {
    this.onChange?.(this.serialize());
    this.#notify();
  }
}

/** The app's single performance controller instance (mirrors the `theme` singleton). */
export const performanceController = new PerformanceController();

const subscribe = createSubscriber((update) => performance.subscribe(update));

/** The resolved effective performance settings, read reactively: reading it in a rune context
 * (`$derived`, `$effect`, a component's template) re-runs on any `set`/`setPreset`/`load` change.
 * @returns The current effective `PerformanceSettings`.
 * @example activePerformance().fpsCap; // 60
 */
export function activePerformance(): PerformanceSettings {
  subscribe();
  return performance.current;
}
```

- [ ] **Step 1:** write the test file above; implement `performance.svelte.ts`; `pnpm --filter
  @shadowcat/ui-kit test`, `pnpm -r typecheck`, `pnpm lint`, `pnpm lint:docs`,
  `pnpm lint:comments`, `pnpm docs:check-examples` PASS.
- [ ] **Step 2:** export from `src/client/ui-kit/src/index.ts` — append after the `theme.svelte`
  export block:
  ```ts
  export { PerformanceController, performanceController, activePerformance } from "./performance.svelte";
  export type { PerformanceStats, PerformanceListener } from "./performance.svelte";
  ```
  `pnpm -r typecheck` PASS.
- [ ] **Step 3:** `git commit -m "feat(ui-kit): PerformanceController mirroring ThemeController" -- src/client/ui-kit/src/performance.svelte.ts src/client/ui-kit/src/performance.svelte.test.ts src/client/ui-kit/src/index.ts`

## Task 3: `AppContext.performance` + fixtures

**Files:**
- Modify: `src/client/ui-kit/src/appContext.ts` (new member `performance`, appended after
  `panels` per master §3's file-ownership convention for this file).
- Modify: `src/client/ui-kit/src/__fixtures__/appContextTest.ts` (default), `src/client/ui-kit/src/__fixtures__/SurfaceHarness.svelte` (default).

**Interfaces (Consumes):** `PerformanceController` from `./performance.svelte` (Task 2).

**Step 1:** in `appContext.ts`, add the import and the member. After the existing import block
(the last line is `import { AssetPickController, type PickAssetOptions, type PickAssetMultiple } from "./assetPickController.svelte";`), add:
```ts
import type { PerformanceController } from "./performance.svelte";
```
Then, inside `AppContext`, immediately after the `panels: PanelsApi & PanelsChipsView;` member
(the interface's last field), add:
```ts
  /** Per-device render-budget controller — effective settings, active preset, live frame
   * stats, and the statusbar readout toggle. Per-device only: never read from or written to
   * the server `ui_state`. */
  performance: PerformanceController;
```

**Step 2:** in `appContextTest.ts`, add the import and the default. After the existing import
`import { AssetPickController, type PickAssetOptions } from "../assetPickController.svelte";`,
add:
```ts
import { PerformanceController } from "../performance.svelte";
```
Then, inside the `ctx: AppContext = { ... }` object literal, after the `templates:` block (the
object's last field), add:
```ts
    performance: over.performance ?? new PerformanceController(),
```
A FRESH `PerformanceController` per fixture call (never the shared `performance` singleton) —
so one test's `set`/`setPreset` call can never leak into another test that also calls
`setAppContextForTest()` without an explicit override.

**Step 3:** in `SurfaceHarness.svelte`, add `PerformanceController` to the existing import list
from `"../appContext"`'s sibling imports (it currently imports `AssetPickController` etc.
directly by relative path — add `import { PerformanceController } from "../performance.svelte";`
alongside them) and append `, performance: new PerformanceController()` to the single-line
`setAppContext({ ... })` object literal, immediately before the closing `}`.

- [ ] **Step 1:** make the three edits above; `pnpm -r typecheck` FAILS until all three are done
  (the interface gains a required field); once all three land, `pnpm --filter @shadowcat/ui-kit
  test`, `pnpm --filter @shadowcat/ui-kit typecheck` PASS.
- [ ] **Step 2:** `rg "setAppContext\(|: AppContext = " src --type ts --type svelte` confirms
  exactly these three sites plus `Table.svelte` (Task 4) and the two `appContext.ts` declaration
  lines themselves — no other site missed.
- [ ] **Step 3:** `pnpm lint:docs`, `pnpm lint:props` PASS.
- [ ] **Step 4:** `git commit -m "feat(ui-kit): AppContext.performance" -- src/client/ui-kit/src/appContext.ts src/client/ui-kit/src/__fixtures__/appContextTest.ts src/client/ui-kit/src/__fixtures__/SurfaceHarness.svelte`

## Task 4: shell — device signals, mirror persistence, wiring

**Files:**
- Create: `src/client/shell/src/lib/deviceSignals.ts`, `src/client/shell/src/lib/deviceSignals.test.ts` (`// @vitest-environment node`).
- Modify: `src/client/shell/src/lib/sessionState.svelte.ts` (add `readPerformanceMirror`/`writePerformanceMirror`), `src/client/shell/src/lib/sessionState.test.ts` (round-trip tests).
- Modify: `src/client/shell/src/main.ts` (load call), `src/client/shell/src/lib/Table.svelte` (`performance: performanceController` in `setAppContext`).

**Interfaces (Consumes):** `DeviceSignals`, `PersistedPerformance`, `PERFORMANCE_STORAGE_KEY`
from `@shadowcat/core`; `performance` singleton from `@shadowcat/ui-kit`.

**Step 1 — `deviceSignals.ts` (TDD).** Write `deviceSignals.test.ts`:
```ts
// @vitest-environment node
import { describe, it, expect } from "vitest";
import { readDeviceSignals } from "./deviceSignals";

describe("readDeviceSignals", () => {
  it("returns {} when every global probe is absent (node env)", () => {
    expect(readDeviceSignals()).toEqual({});
  });
});
```
Implement `deviceSignals.ts`:
```ts
import type { DeviceSignals } from "@shadowcat/core";

/** Reads the live `DeviceSignals` `resolveAuto` resolves `"auto"` against. Every probe is
 * guarded on existence — `matchMedia`, `navigator.hardwareConcurrency`, and the Chromium-only
 * `navigator.deviceMemory` are all absent under jsdom/node — so an unsupported environment
 * degrades to "unknown" (the field omitted) rather than throwing or synthesizing a false value.
 * Called once, pre-mount, in `main.ts` alongside `performance.load`.
 * @returns The signals this device/browser can report right now.
 * @example
 * ```ts
 * readDeviceSignals(); // {} under jsdom/node; populated fields in a real browser
 * ```
 */
export function readDeviceSignals(): DeviceSignals {
  const signals: DeviceSignals = {};
  if (typeof matchMedia === "function") {
    signals.coarsePointer = matchMedia("(pointer: coarse)").matches;
    signals.compact = !matchMedia("(min-width: 48rem)").matches;
    signals.reducedMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;
  }
  if (typeof navigator !== "undefined") {
    if (typeof navigator.hardwareConcurrency === "number") {
      signals.hardwareConcurrency = navigator.hardwareConcurrency;
    }
    const deviceMemory = (navigator as Navigator & { deviceMemory?: number }).deviceMemory;
    if (typeof deviceMemory === "number") signals.deviceMemoryGb = deviceMemory;
  }
  return signals;
}
```
Note: `!matchMedia("(min-width: 48rem)").matches` mirrors `@shadowcat/ui-kit`'s `sizeClass()`
query exactly (`src/client/ui-kit/src/sizeClass.svelte.ts`'s `QUERY` constant) — `compact` is
true when that query does NOT match, matching `SizeClass`'s own definition. This file does not
import `sizeClass()` itself (that helper is reactive/component-bound via `createSubscriber`;
`deviceSignals.ts` needs one plain synchronous read at boot, not a subscription).

- [ ] Run `pnpm --filter @shadowcat/shell test`, `pnpm -r typecheck` PASS.

**Step 2 — mirror persistence in `sessionState.svelte.ts`.** Add the import (alongside the
existing `import { i18n, theme, type PersistedTheme } from "@shadowcat/ui-kit";` line):
```ts
import { i18n, theme, performanceController, type PersistedTheme } from "@shadowcat/ui-kit";
import { PERFORMANCE_STORAGE_KEY, parsePersisted, serializePersisted, type PersistedPerformance } from "@shadowcat/core";
```
Add, immediately after `writeThemeMirror`'s definition:
```ts
/** Reads the performance mirror, garbage-tolerantly: an absent key is `undefined` (which
 * `PerformanceController.load` resolves via `resolveAuto`); a present-but-garbled value still
 * parses through `parsePersisted`'s own fail-closed validation, never `undefined`.
 * @param storage The storage to read (injectable for tests; the app entry passes `localStorage`).
 * @returns The mirrored value, or `undefined` when the key is absent.
 * @example
 * ```ts
 * const mirror = readPerformanceMirror(localStorage);
 * ```
 */
export function readPerformanceMirror(storage: Pick<Storage, "getItem">): PersistedPerformance | undefined {
  const raw = storage.getItem(PERFORMANCE_STORAGE_KEY);
  return raw === null ? undefined : parsePersisted(raw);
}

/** Writes the performance mirror. A throwing storage (quota, privacy mode) is swallowed with a
 * log — the mirror is a per-device convenience and a failed write must never break the setting
 * change that triggered it.
 * @param storage The storage to write (injectable for tests; callers pass `localStorage`).
 * @param value The `PerformanceController.serialize` output to mirror.
 * @example
 * ```ts
 * writePerformanceMirror(localStorage, performance.serialize());
 * ```
 */
export function writePerformanceMirror(storage: Pick<Storage, "setItem">, value: PersistedPerformance): void {
  try {
    storage.setItem(PERFORMANCE_STORAGE_KEY, serializePersisted(value));
  } catch (e) {
    logger.warn("performance mirror write failed", e);
  }
}
```
Note: `performance` is imported above for symmetry with `theme` but this file does not call
`performance.now()` anywhere — if a future edit to this file needs the real Performance API, it
must use `globalThis.performance.now()` (see the naming-hazard note near the top of this plan).

Add tests to `sessionState.test.ts` (read its existing `readThemeMirror`/`writeThemeMirror`
round-trip tests first and mirror their shape exactly):
```ts
describe("readPerformanceMirror / writePerformanceMirror", () => {
  it("round-trips a written value", () => {
    const store = new Map<string, string>();
    const storage: Pick<Storage, "getItem" | "setItem"> = {
      getItem: (k) => store.get(k) ?? null,
      setItem: (k, v) => void store.set(k, v),
    };
    writePerformanceMirror(storage, { preset: "mobile", overrides: {} });
    expect(readPerformanceMirror(storage)).toEqual({ preset: "mobile", overrides: {} });
  });
  it("returns undefined when the key is absent", () => {
    const storage: Pick<Storage, "getItem"> = { getItem: () => null };
    expect(readPerformanceMirror(storage)).toBeUndefined();
  });
  it("falls back to the auto default on garbage", () => {
    const storage: Pick<Storage, "getItem"> = { getItem: () => "not json" };
    expect(readPerformanceMirror(storage)).toEqual({ preset: "auto", overrides: {} });
  });
});
```

- [ ] Run `pnpm --filter @shadowcat/shell test`, `pnpm -r typecheck`, `pnpm lint:docs` PASS.

**Step 3 — wire `main.ts`.** After the existing line
`theme.load(readThemeMirror(localStorage));`, add:
```ts
import { performanceController } from "@shadowcat/ui-kit";
import { readPerformanceMirror, writePerformanceMirror } from "./lib/sessionState.svelte";
import { readDeviceSignals } from "./lib/deviceSignals";
```
(merge `performanceController` into the existing `import { theme } from "@shadowcat/ui-kit";`
line rather than a second import statement), then:
```ts
// Per-device settings, resolved before mount so pre-login screens never flash the wrong
// budget (never the server ui_state — resolved fresh from this device's own signals).
performanceController.load(readPerformanceMirror(localStorage), readDeviceSignals());
performanceController.onChange = (p) => writePerformanceMirror(localStorage, p);
```

**Step 4 — wire `Table.svelte`.** Add `performance: performanceController` (import `performanceController` from `@shadowcat/ui-kit`) to the existing
`import { setAppContext, Surface, ... } from "@shadowcat/ui-kit";` line, and in the
`setAppContext({ ... })` object literal, immediately after the `panels,` line, add:
```ts
    performance: performanceController,
```

- [ ] **Step 1:** `deviceSignals.ts` + test (above) written and passing.
- [ ] **Step 2:** `sessionState.svelte.ts` mirror functions + tests written and passing.
- [ ] **Step 3:** `main.ts`/`Table.svelte` wiring done; `pnpm --filter @shadowcat/shell test`,
  `pnpm -r typecheck`, `pnpm build`, `pnpm lint`, `pnpm lint:docs` PASS.
- [ ] **Step 4:** `git commit -m "feat(shell): load and persist per-device performance settings" -- src/client/shell/src/lib/deviceSignals.ts src/client/shell/src/lib/deviceSignals.test.ts src/client/shell/src/lib/sessionState.svelte.ts src/client/shell/src/lib/sessionState.test.ts src/client/shell/src/main.ts src/client/shell/src/lib/Table.svelte`

## Task 5: `@shadowcat/render` — backend interface + PixiBackend + MockBackend

Read `src/client/render/src/backend.ts`, `backend.mock.ts`, and `pixi-backend.ts` in full
(already read for this plan) before editing. Read the vendored pixi.js source cited below —
already verified: `node_modules/.pnpm/pixi.js@8.19.0/node_modules/pixi.js/lib/app/TickerPlugin.mjs`
(the auto-render listener `ticker.add(this.render, this, UPDATE_PRIORITY.LOW)`, removed via
`ticker.remove(fn, context)` which matches by the `(fn, context)` pair — confirmed in
`lib/ticker/Ticker.mjs`'s `remove` implementation), `lib/ticker/Ticker.mjs`'s `maxFPS` setter
(`fps === 0` ⇒ uncapped), and `lib/app/Application.mjs`'s `render()` (`this.renderer.render({
container: this.stage })`).

**Files:**
- Modify: `src/client/render/src/backend.ts` (`DisplayBackend` gains `setFrameCap`/
  `setRenderScale`/`render`, appended after `resize` and before `destroy`).
- Modify: `src/client/render/src/backend.mock.ts` (implements the three new methods; records
  structurally).
- Create: `src/client/render/src/dirty-backend.ts`, `src/client/render/src/dirty-backend.test.ts`
  (`// @vitest-environment node`).
- Modify: `src/client/render/src/pixi-backend.ts` (implements the three new methods; tracks
  viewport size for `setRenderScale`'s re-resize; `createPixiBackend` removes Pixi's own
  auto-render ticker listener).
- Modify: `src/client/render/src/index.ts` (export `wrapDirtyTracking`).

**Step 1 — `DisplayBackend` interface.** In `backend.ts`, insert after `resize`'s member (before
`destroy(): void;`):
```ts
  /** Cap the render ticker's rate; `0` = uncapped (Pixi's `Ticker.maxFPS = 0` convention).
   * @param fps The new cap in frames per second, or `0` for uncapped. */
  setFrameCap(fps: number): void;
  /** Set the renderer's resolution (device-pixel-ratio × the given scale) and re-apply the last
   * known viewport size at the new resolution.
   * @param scale The render-scale fraction, already clamped by the caller. */
  setRenderScale(scale: number): void;
  /** Draw exactly one frame now — the idle-skip ticker's render call. */
  render(): void;
```

**Step 2 — `MockBackend`.** Add fields (alongside the existing `lighting`/`tick`/`destroyed`
fields):
```ts
  /** Last `setFrameCap` value, recorded verbatim; `null` before the first call. */
  frameCap: number | null = null;
  /** Last `setRenderScale` value, recorded verbatim; `null` before the first call. */
  renderScale: number | null = null;
  /** Count of `render()` calls — the idle-skip assertion surface. */
  renderCount = 0;
```
Add methods (alongside `resize`):
```ts
  /** `DisplayBackend.setFrameCap`: records `fps` verbatim into `this.frameCap`.
   * @param fps The frame-rate cap, or `0` for uncapped.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.setFrameCap(30);
   * ```
   */
  setFrameCap(fps: number): void {
    this.frameCap = fps;
  }
  /** `DisplayBackend.setRenderScale`: records `scale` verbatim into `this.renderScale`.
   * @param scale The render-scale fraction.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.setRenderScale(0.75);
   * ```
   */
  setRenderScale(scale: number): void {
    this.renderScale = scale;
  }
  /** `DisplayBackend.render`: increments `this.renderCount`.
   * @example
   * ```ts
   * import { MockBackend } from "@shadowcat/render";
   *
   * const backend = new MockBackend();
   * backend.render();
   * backend.renderCount; // 1
   * ```
   */
  render(): void {
    this.renderCount++;
  }
```

- [ ] Run `pnpm --filter @shadowcat/render typecheck` — FAILS until `PixiBackend` also
  implements the three new methods (Step 4); expected at this point.

**Step 3 — `dirty-backend.ts` (TDD).** Write `dirty-backend.test.ts`:
```ts
// @vitest-environment node
import { describe, it, expect, vi } from "vitest";
import { MockBackend } from "./backend.mock";
import { wrapDirtyTracking } from "./dirty-backend";

describe("wrapDirtyTracking", () => {
  it("calls onDirty and forwards for a drawing method", () => {
    const real = new MockBackend();
    const onDirty = vi.fn();
    const wrapped = wrapDirtyTracking(real, onDirty);
    wrapped.resize(800, 600);
    expect(onDirty).toHaveBeenCalledOnce();
    expect(real.size).toEqual({ width: 800, height: 600 });
  });

  it("does not call onDirty for tickTokenAnimations (called unconditionally every tick)", () => {
    const real = new MockBackend();
    const onDirty = vi.fn();
    const wrapped = wrapDirtyTracking(real, onDirty);
    wrapped.tickTokenAnimations(16);
    expect(onDirty).not.toHaveBeenCalled();
  });

  it("does not call onDirty for ensureLayers/addLayerFilter/startTicker/destroy", () => {
    const real = new MockBackend();
    const onDirty = vi.fn();
    const wrapped = wrapDirtyTracking(real, onDirty);
    wrapped.ensureLayers(["background"]);
    wrapped.addLayerFilter("background", {});
    wrapped.startTicker(() => {});
    wrapped.destroy();
    expect(onDirty).not.toHaveBeenCalled();
    expect(real.layers).toEqual(["background"]);
    expect(real.destroyed).toBe(true);
  });

  it("forwards setVisibilityBlend and calls onDirty when the real backend defines it", () => {
    const real = new MockBackend();
    const onDirty = vi.fn();
    const wrapped = wrapDirtyTracking(real, onDirty);
    const input = { mode: "all" as const, visible: [], explored: [], perceived: [] };
    wrapped.setVisibilityBlend?.(input, input, 0.5);
    expect(onDirty).toHaveBeenCalledOnce();
    expect(real.visibility).toEqual(input);
  });

  it("does not call onDirty for setFrameCap/setRenderScale (settings, not draws)", () => {
    const real = new MockBackend();
    const onDirty = vi.fn();
    const wrapped = wrapDirtyTracking(real, onDirty);
    wrapped.setFrameCap(30);
    wrapped.setRenderScale(0.75);
    expect(onDirty).not.toHaveBeenCalled();
    expect(real.frameCap).toBe(30);
    expect(real.renderScale).toBe(0.75);
  });

  it("forwards render() without calling onDirty (render CONSUMES the flag, never sets it)", () => {
    const real = new MockBackend();
    const onDirty = vi.fn();
    const wrapped = wrapDirtyTracking(real, onDirty);
    wrapped.render();
    expect(onDirty).not.toHaveBeenCalled();
    expect(real.renderCount).toBe(1);
  });
});
```
Implement `dirty-backend.ts`:
```ts
import type { DisplayBackend } from "./backend";

/** Wraps `backend` so every mutating draw call also invokes `onDirty()` before forwarding to the
 * real backend — the ONE seam `RenderEngine`'s idle-skip dirty flag hooks. Every reconciler/view
 * (`SceneReconciler`, `TokenView`, `DrawingView`, `TemplateView`, `WallView`, `RegionView`,
 * `LightView`) and both `Compositor`/`Lighting` already route every push through the injected
 * `DisplayBackend`, so intercepting at this one boundary needs no change to any of them.
 *
 * Excluded from dirty-tracking: `ensureLayers`/`addLayerFilter` (one-time/opt-in setup, not a
 * per-frame redraw trigger), `startTicker`/`destroy` (lifecycle, not drawing),
 * `tickTokenAnimations` (called UNCONDITIONALLY every tick by `TokenView.tick` regardless of
 * whether an animated sprite exists — wrapping it would mark every tick dirty and defeat
 * idle-skip entirely; `TokenView.hasAnimatedVisual` covers the real animated-sprite-redraw need
 * instead), and `setFrameCap`/`setRenderScale` (budget settings, not draws) and `render` itself
 * (the call that CONSUMES the dirty flag, never sets it).
 * @param backend The real backend to wrap.
 * @param onDirty Called synchronously before forwarding any dirty-tracked method's call.
 * @returns A `DisplayBackend` behaviorally identical to `backend`, reporting every draw.
 * @example
 * ```ts
 * import { MockBackend } from "@shadowcat/render";
 * import { wrapDirtyTracking } from "@shadowcat/render";
 *
 * let dirty = false;
 * const backend = wrapDirtyTracking(new MockBackend(), () => { dirty = true; });
 * backend.resize(800, 600); // dirty === true
 * ```
 */
export function wrapDirtyTracking(backend: DisplayBackend, onDirty: () => void): DisplayBackend {
  return {
    ensureLayers: (orderedIds) => backend.ensureLayers(orderedIds),
    setBackground: (spec) => { onDirty(); backend.setBackground(spec); },
    setClearColor: (color) => { onDirty(); backend.setClearColor(color); },
    drawGrid: (lines, color) => { onDirty(); backend.drawGrid(lines, color); },
    setVisibility: (input) => { onDirty(); backend.setVisibility(input); },
    setVisibilityBlend: backend.setVisibilityBlend
      ? (from, to, factor) => { onDirty(); backend.setVisibilityBlend!(from, to, factor); }
      : undefined,
    setCameraTransform: (t) => { onDirty(); backend.setCameraTransform(t); },
    addLayerFilter: (layerId, filter) => backend.addLayerFilter(layerId, filter),
    setToken: (id, spec) => { onDirty(); backend.setToken(id, spec); },
    removeToken: (id) => { onDirty(); backend.removeToken(id); },
    tickTokenAnimations: (dtMs) => backend.tickTokenAnimations(dtMs),
    setShape: (id, spec) => { onDirty(); backend.setShape(id, spec); },
    removeShape: (id) => { onDirty(); backend.removeShape(id); },
    drawOverlay: (shapes) => { onDirty(); backend.drawOverlay(shapes); },
    clearOverlay: () => { onDirty(); backend.clearOverlay(); },
    drawMeasure: (from, to, label) => { onDirty(); backend.drawMeasure(from, to, label); },
    clearMeasure: () => { onDirty(); backend.clearMeasure(); },
    drawPings: (rings) => { onDirty(); backend.drawPings(rings); },
    drawEmotes: (glyphs) => { onDirty(); backend.drawEmotes(glyphs); },
    setLighting: (frame) => { onDirty(); backend.setLighting(frame); },
    startTicker: (cb) => backend.startTicker(cb),
    resize: (width, height) => { onDirty(); backend.resize(width, height); },
    setFrameCap: (fps) => backend.setFrameCap(fps),
    setRenderScale: (scale) => backend.setRenderScale(scale),
    render: () => backend.render(),
    destroy: () => backend.destroy(),
  };
}
```
- [ ] **Step 6 — `pixi-backend.test.ts` (existing file).** Its `headlessBackend(): PixiBackend`
  fixture builds a real `PixiBackend` over `{ stage: new Container() } as unknown as Application`
  and is called at ~15 sites as `const backend = headlessBackend();` — its return type stays
  `PixiBackend` so none of them change. Give it an optional stub parameter instead:
  ```ts
  /** A stub `Application` for the headless backend; callers pass the parts their test reads. */
  function fakeApp(extra: Record<string, unknown> = {}): Application {
    return { stage: new Container(), ...extra } as unknown as Application;
  }
  function headlessBackend(app: Application = fakeApp()): PixiBackend {
    return new PixiBackend(app);
  }
  ```
  then add:
  ```ts
  describe("frame cap and render scale", () => {
    it("setFrameCap forwards to ticker.maxFPS (0 = uncapped)", () => {
      const app = fakeApp({ ticker: { maxFPS: 0, add: vi.fn(), remove: vi.fn() } });
      const backend = headlessBackend(app);
      backend.setFrameCap(30);
      expect((app as { ticker: { maxFPS: number } }).ticker.maxFPS).toBe(30);
      backend.setFrameCap(0);
      expect((app as { ticker: { maxFPS: number } }).ticker.maxFPS).toBe(0);
    });
    it("setRenderScale sets renderer.resolution to dpr*scale and calls resize", () => {
      const renderer = { resolution: 1, resize: vi.fn(), render: vi.fn() };
      const backend = headlessBackend(fakeApp({ renderer }));
      globalThis.devicePixelRatio = 2;
      backend.setRenderScale(0.5);
      expect(renderer.resolution).toBe(1);
      expect(renderer.resize).toHaveBeenCalledOnce();
    });
    it("render() calls renderer.render once with the stage", () => {
      const renderer = { resolution: 1, resize: vi.fn(), render: vi.fn() };
      const app = fakeApp({ renderer });
      const backend = headlessBackend(app);
      backend.render();
      expect(renderer.render).toHaveBeenCalledWith({ container: app.stage });
    });
  });
  ```
  (`setRenderScale` reads the size it re-applies from the canvas element the backend already
  holds — check `PixiBackend.resize`'s current signature and pass whatever it needs through
  the stub; the ONE fixture stays the one every `PixiBackend` test shares.)
- [ ] `pnpm --filter @shadowcat/render test`, `pnpm --filter @shadowcat/render typecheck` PASS
  (typecheck still fails on `PixiBackend`'s missing methods — expected until Step 4).

**Step 4 — `PixiBackend` + `createPixiBackend`.** Add a private field (alongside the existing
`loadSeq` field):
```ts
  /** CSS-pixel viewport size from the last `resize` call — re-applied by `setRenderScale` after
   * changing `resolution`, since `renderer.resize` takes CSS dimensions, not device pixels. */
  private viewportWidth = 0;
  /** See `viewportWidth`'s doc. */
  private viewportHeight = 0;
```
Modify `resize` to record it:
```ts
  resize(width: number, height: number): void {
    this.viewportWidth = width;
    this.viewportHeight = height;
    this.app.renderer.resize(width, height);
  }
```
Add the three new methods, immediately after `resize`:
```ts
  /** `DisplayBackend.setFrameCap`: sets Pixi's `Ticker.maxFPS` (`0` = uncapped, its own
   * convention — confirmed against the vendored `Ticker` setter).
   * @param fps The frame-rate cap, or `0` for uncapped.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.setFrameCap(30);
   * ```
   */
  setFrameCap(fps: number): void {
    this.app.ticker.maxFPS = fps;
  }

  /** `DisplayBackend.setRenderScale`: sets the renderer's `resolution` to `devicePixelRatio *
   * scale`, then re-applies the last known CSS-pixel viewport size — `renderer.resize` takes
   * CSS dimensions and internally multiplies by `resolution`, so the resolution change alone
   * would leave the backing texture at the OLD device-pixel size until the next external resize.
   * @param scale The render-scale fraction (already clamped by the caller).
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.setRenderScale(0.75);
   * ```
   */
  setRenderScale(scale: number): void {
    this.app.renderer.resolution = (globalThis.devicePixelRatio || 1) * scale;
    this.app.renderer.resize(this.viewportWidth, this.viewportHeight);
  }

  /** `DisplayBackend.render`: draws exactly one frame now — forwards to `Application.render()`,
   * the same call `TickerPlugin`'s own (now-removed, see `createPixiBackend`) auto-render
   * listener used to make every tick.
   * @example
   * ```ts
   * import { PixiBackend } from "@shadowcat/render";
   *
   * declare const backend: PixiBackend;
   * backend.render();
   * ```
   */
  render(): void {
    this.app.render();
  }
```
Modify `createPixiBackend`:
```ts
export async function createPixiBackend(
  canvas: HTMLCanvasElement,
  opts: PixiBackendOptions,
): Promise<PixiBackend> {
  const app = new Application();
  await app.init({
    canvas,
    antialias: opts.antialias ?? true,
    resolution: globalThis.devicePixelRatio || 1,
    autoDensity: true,
    background: opts.background,
    preference: "webgl",
  });
  // RenderEngine's idle-skip ticker calls PixiBackend.render() itself; without removing
  // TickerPlugin's own auto-render listener (installed by `Application.init` via
  // `ticker.add(this.render, this, UPDATE_PRIORITY.LOW)` — confirmed against the vendored
  // `TickerPlugin.init` source), Pixi would additionally redraw every tick regardless of
  // idleSkip. `Ticker.remove(fn, context)` matches by the exact `(fn, context)` pair the add
  // call used (confirmed against the vendored `Ticker.remove`/`Listener.match` source).
  app.ticker.remove(app.render, app);
  return new PixiBackend(app);
}
```

- [ ] **Step 1:** interface + `MockBackend` edits.
- [ ] **Step 2:** `dirty-backend.ts` + test (TDD as shown).
- [ ] **Step 3:** `PixiBackend`/`createPixiBackend` edits.
- [ ] **Step 4:** export `wrapDirtyTracking` from `src/client/render/src/index.ts` — append
  after the `MockBackend` export line: `export { wrapDirtyTracking } from "./dirty-backend";`.
- [ ] **Step 5:** `pnpm --filter @shadowcat/render test`, `pnpm --filter @shadowcat/render
  typecheck`, `pnpm -r typecheck`, `pnpm lint`, `pnpm lint:docs`, `pnpm lint:comments`,
  `pnpm docs:check-examples` PASS.
- [ ] **Step 6:** `git commit -m "feat(render): DisplayBackend frame-cap/render-scale/render, dirty-tracking backend wrapper" -- src/client/render/src/backend.ts src/client/render/src/backend.mock.ts src/client/render/src/dirty-backend.ts src/client/render/src/dirty-backend.test.ts src/client/render/src/pixi-backend.ts src/client/render/src/index.ts`

## Task 6: `RenderEngine` — performance getter, idle-skip ticker, stats, lighting/reducedMotion budgets

Read `src/client/render/src/engine.ts` in full before editing (already read for this plan: the
constructor at line 314, `start()` at line 351, `applyDerived`/`renderVisibility` at 529–561,
`retargetLighting`/`applyCommittedLighting` at 854–879, `tickLightSweep`/`applyLightSweep` at
1328–1397, `sweepBlendInputs`/`applyVisionSweep` at 1484–1535, `setViewport`/`applyCamera` at
1636–1670, `destroy` at 1702).

**Files:**
- Modify: `src/client/render/src/engine.ts`.
- Modify: `src/client/render/src/token-view.ts` (add `hasAnimatedVisual()` — used by this task's
  ticker; the `tokenFx`/`reducedMotion` constructor params land in Task 7, which this task's
  `RenderEngine` constructor already calls with the two new trailing getters, matching Task 7's
  signature exactly, since Task 7 runs immediately after this one in the same worktree).
- Modify: `src/client/render/src/engine.test.ts`.

**Interfaces (Consumes):** `PerformanceSettings`, `PRESETS` from `@shadowcat/core`;
`wrapDirtyTracking` from `./dirty-backend` (Task 5).

**Interfaces (Produces — `RenderEngineOpts` additions):**
```ts
performance?: () => PerformanceSettings;
onStats?: (s: { fps: number; frameMs: number }) => void;
```

**Step 1 — `TokenView.hasAnimatedVisual`.** In `token-view.ts`, add after `specOf`:
```ts
  /** Whether any currently-tracked token's resolved visual is tick-driven (an `"animated"` art
   * kind, top-level or nested inside a `"generated"` frame). Read by `RenderEngine`'s ticker
   * alongside the dirty flag: `tickTokenAnimations` advances an animated sprite's own frame
   * index every tick unconditionally (it is excluded from `wrapDirtyTracking`'s dirty set — see
   * that module's doc — because it fires whether or not anything is actually animated), so this
   * is the real "is a redraw needed for animation" signal idle-skip consults.
   * @returns Whether at least one tracked token has an animated visual.
   * @example
   * ```ts
   * import { TokenView, MockBackend } from "@shadowcat/render";
   * import { AssetResolver, type ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new TokenView(store, new AssetResolver(), new MockBackend());
   * view.hasAnimatedVisual(); // false
   * ```
   */
  hasAnimatedVisual(): boolean {
    for (const spec of this.specs.values()) {
      if (spec.visual.kind === "animated") return true;
      if (spec.visual.kind === "generated" && spec.visual.art.kind === "animated") return true;
    }
    return false;
  }
```

**Step 2 — `engine.ts` imports + `RenderEngineOpts`.** Add to the existing `@shadowcat/core`
type-only import line: `PerformanceSettings`; add a new value import: `import { PRESETS } from
"@shadowcat/core";` (alongside the existing `import { EMPTY_FOOTPRINTS } from "@shadowcat/core";`
line). Add to `import { wrapDirtyTracking } from "./dirty-backend";` (new import line).

In `RenderEngineOpts`, append after `selectedTokens`:
```ts
  /** Live per-device render budget (Stage → `() => ctx.performance.current`). Absent ⇒
   * `PRESETS.quality` with `idleSkip: false` (legacy/test callers keep today's unconditional
   * per-tick render). A getter, like `viewedSceneId` — read fresh every tick, never cached. */
  performance?: () => PerformanceSettings;
  /** Host observability hook: called with the ticker's live fps/frameMs sample, at most 4×/s
   * (like `onMeasureDrawn`). Absent ⇒ stats are still computed internally but never pushed out. */
  onStats?: (s: { fps: number; frameMs: number }) => void;
```

**Step 3 — constructor.** Add new private fields (near `viewedScene`, same style — a lazy
arrow-function field reading `this.opts`, safe under the field-initializer/parameter-property
ordering exactly as `viewedScene` already demonstrates):
```ts
  /** Resolves the live per-device performance budget. Absent `opts.performance` ⇒ `PRESETS.quality`
   * with `idleSkip: false` (legacy/test callers keep today's unconditional per-tick render). */
  private readonly perf = (): PerformanceSettings =>
    this.opts.performance?.() ?? { ...PRESETS.quality, idleSkip: false };
  /** Set by the dirty-tracking backend wrapper on every draw call; consumed and cleared by the
   * idle-skip ticker branch. Starts `true` so the very first tick always renders. */
  private dirty = true;
  /** EMA-smoothed ticker rate, `null` until the first sample (its own doc names the smoothing
   * window: ~30 ticks). */
  private statsFpsEma: number | null = null;
  /** Milliseconds the last actual `backend.render()` call took; `0` until the first render. */
  private statsFrameMs = 0;
  /** Milliseconds accumulated since the last `onStats` push — throttles the hook to at most
   * 4×/s regardless of ticker rate. */
  private statsElapsedSinceEmit = 0;
  /** The frame cap value last pushed to the backend (`"uncapped"` mapped to `0`); `null` before
   * the first tick, so the very first tick always pushes. */
  private lastFrameCap: number | null = null;
  /** The render scale last pushed to the backend; `null` before the first tick. */
  private lastRenderScale: number | null = null;
```
Change the `constructor`'s FIRST statement from `this.grid = new Grid(opts.grid);` to wrap the
backend first:
```ts
  constructor(private readonly opts: RenderEngineOpts) {
    this.backend = wrapDirtyTracking(opts.backend, () => { this.dirty = true; });
    this.grid = new Grid(opts.grid);
    this.gridColor = opts.gridColor ?? 0x3a3a4a;
    this.reconciler = new SceneReconciler(opts.store, opts.assets, this.backend, this.viewedScene);
    this.tokens = new TokenView(opts.store, opts.assets, this.backend, this.viewedScene, () => opts.footprints?.() ?? EMPTY_FOOTPRINTS, () => this.perceived, opts.selectedTokens, () => this.perf().tokenFx, () => this.perf().reducedMotion);
    this.drawings = new DrawingView(opts.store, this.backend, this.viewedScene);
    this.templates = new TemplateView(opts.store, this.backend, this.viewedScene);
    this.walls = new WallView(opts.store, this.backend, this.viewedScene);
    this.regions = new RegionView(opts.store, this.backend, this.viewedScene);
    this.lights = new LightView(opts.store, this.backend, this.viewedScene);
    this.compositor = new Compositor(this.backend);
    this.lighting = new Lighting(this.backend, (frame) => opts.onLightingApplied?.(frame, this.lightSweeps.size > 0));
  }
```
Add a new private readonly field `private readonly backend: DisplayBackend;` in the field-
declaration block, immediately before the `readonly camera = new Camera();` field (its doc:
"The dirty-tracking-wrapped backend every reconciler/view/compositor/lighting object is
constructed against — see `wrapDirtyTracking`. `opts.backend` itself is never read again after
the constructor."). `DisplayBackend` is already imported as a type in this file (via
`RenderEngineOpts.backend: DisplayBackend`) — confirm the import line includes it as a value-or-
type import compatible with a field type annotation (it is currently `import type {
LineSeg, CameraTransform, ... } from "./types";` for OTHER types and `DisplayBackend` comes from
`./backend` — add `import type { DisplayBackend } from "./backend";` if not already present;
check `rg "from \"./backend\"" src/client/render/src/engine.ts` first, since `RenderEngineOpts`
already references `DisplayBackend` as a field type and the import may already exist).

**Step 4 — replace every remaining `this.opts.backend.X` call site with `this.backend.X`.**
`rg "this\.opts\.backend\." src/client/render/src/engine.ts` and convert EVERY hit the command
returns (14 today) — the list below is a guide, the `rg` output is the authority:
`start()`'s `ensureLayers`/`startTicker` calls, the ticker's `drawPings`/`drawEmotes` calls,
`previewOverlay`'s `drawOverlay`, `clearOverlay`, `drawMeasure`, `clearMeasure`, `setThemeColors`'s
`setClearColor`, `setViewport`'s `resize`, `redrawGrid`'s `drawGrid`, `applyCamera`'s
`setCameraTransform`, `destroy`'s `destroy`, and the `addLayerFilter` forwarder (`return
this.opts.backend.addLayerFilter(layerId, filter);`). Every one becomes `this.backend.X`
(drop `.opts`); re-run the `rg` afterwards and expect zero hits.

**Step 5 — the ticker.** Replace `start()`'s `this.backend.startTicker((dt) => { ... });` body
(after Step 4's rename) with:
```ts
    this.backend.startTicker((dt) => {
      this.tokens.tick(dt);
      this.lighting.tick(dt);
      this.tickVisionSweep(dt);
      this.tickLightSweep(dt);
      const rings = this.pings.tick(dt);
      if (rings.length > 0 || this.pingsActive) {
        this.backend.drawPings(rings);
        this.pingsActive = rings.length > 0;
      }
      const glyphs = this.emotes.tick(dt);
      if (glyphs.length > 0 || this.emotesActive) {
        this.backend.drawEmotes(glyphs);
        this.emotesActive = glyphs.length > 0;
      }
      const perf = this.perf();
      const frameCapValue = perf.fpsCap === "uncapped" ? 0 : perf.fpsCap;
      if (frameCapValue !== this.lastFrameCap) {
        this.lastFrameCap = frameCapValue;
        this.backend.setFrameCap(frameCapValue);
      }
      if (perf.renderScale !== this.lastRenderScale) {
        this.lastRenderScale = perf.renderScale;
        this.backend.setRenderScale(perf.renderScale);
      }
      if (dt > 0) {
        const sample = 1000 / dt;
        const alpha = 2 / 31; // EMA over ~30 ticks
        this.statsFpsEma = this.statsFpsEma === null ? sample : alpha * sample + (1 - alpha) * this.statsFpsEma;
      }
      const animationsInFlight = this.tokens.hasAnimatedVisual();
      if (!perf.idleSkip || this.dirty || animationsInFlight) {
        const t0 = globalThis.performance?.now?.() ?? Date.now();
        this.backend.render();
        this.dirty = false;
        this.statsFrameMs = (globalThis.performance?.now?.() ?? Date.now()) - t0;
      }
      this.statsElapsedSinceEmit += dt;
      if (this.statsElapsedSinceEmit >= 250) {
        this.statsElapsedSinceEmit = 0;
        this.opts.onStats?.({ fps: Math.round(this.statsFpsEma ?? 0), frameMs: this.statsFrameMs });
      }
    });
```
(`globalThis.performance` — the ambient Web Performance API, unrelated to and never imported
from `@shadowcat/ui-kit`'s `performance` singleton; `@shadowcat/render` does not depend on
`@shadowcat/ui-kit`, so there is no shadowing risk in this file, but the explicit `globalThis.`
prefix is used here to match the project-wide convention this plan's naming-hazard note sets.)

**Step 6 — lighting `"static"`/`"off"` budgets.** In `applyCommittedLighting`, add the "off"
short-circuit as the FIRST statement:
```ts
  private applyCommittedLighting(): void {
    if (this.perf().lighting === "off") {
      this.lighting.setTarget(null);
      return;
    }
    const li = this.lastLightingInput;
    const before = this.visionSweeps.size > 0 ? this.lightingBeforeSweep : null;
    let next = before && li ? unionLightingInputs(before, li) : li;
    const heldFrom = this.lightSweeps.size > 0 ? this.lightingBeforeLightSweep : null;
    if (next && heldFrom) next = holdLightingCells(heldFrom, next, this.lightSweepEndKeys());
    this.lighting.setTarget(next);
  }
```
In `applyLightSweep`, add BOTH the "off" and "static" short-circuits as the first two statements
(after the existing `if (this.lightSweeps.size === 0) return;` guard):
```ts
  private applyLightSweep(): void {
    if (this.lightSweeps.size === 0) return;
    if (this.perf().lighting === "off") {
      this.lighting.setSweep(null);
      return;
    }
    if (this.perf().lighting === "static") {
      // No per-frame interpolation: jump straight to the sweep's committed end state. The
      // post-move committed frame (`lastLightingInput`) already carries the light at its final
      // position (see `tickLightSweep`'s own doc for why), so clearing every in-flight sweep and
      // re-applying the committed frame IS "the sweep's final frame, immediately."
      this.lightSweeps.clear();
      this.lightingBeforeLightSweep = null;
      this.lighting.setSweep(null);
      this.applyCommittedLighting();
      return;
    }
    const li = this.lastLightingInput;
    if (!li) {
      this.lighting.setSweep(null);
      return;
    }
    const los = this.currentLosPolygons();
    const bandCount = li.bands.length;
    const cellsOf = (s: MoveLightSample): LitDrawCell[] => lightSampleCells(s, this.grid, los, bandCount);
    if (this.lightSweeps.size === 1) {
      const [sweep] = this.lightSweeps.values();
      const cur = chooseVisionSample(sweep.samples, sweep.elapsed);
      let next: MoveLightSample | null = null;
      for (const s of sweep.samples) {
        if (s.tMs > cur.tMs && (next === null || s.tMs < next.tMs)) next = s;
      }
      if (next) {
        const factor = this.perf().reducedMotion ? 1 : computeFogBlendFactor(sweep.elapsed, cur.tMs, next.tMs);
        this.lighting.setSweep(blendLightCells(cellsOf(cur), cellsOf(next), factor));
        return;
      }
    }
    const cells: LitDrawCell[] = [];
    for (const sweep of this.lightSweeps.values()) cells.push(...cellsOf(chooseVisionSample(sweep.samples, sweep.elapsed)));
    this.lighting.setSweep(cells);
  }
```

**Step 7 — vision-sweep reducedMotion.** In `sweepBlendInputs`, change the `factor` line:
```ts
      factor: this.perf().reducedMotion ? 1 : computeFogBlendFactor(sweep.elapsed, cur.tMs, next.tMs),
```

- [ ] **Step 1:** `TokenView.hasAnimatedVisual` added.
- [ ] **Step 2:** failing tests first — add to `engine.test.ts` (see below), THEN implement
  Steps 2–7 above.

Add to `engine.test.ts` (new `describe` block; construct `RenderEngine` with `performance: () =>
({ ...PRESETS.quality, idleSkip: true, ... })` overrides per case — import `PRESETS` from
`@shadowcat/core`):
```ts
import { PRESETS } from "@shadowcat/core";
// ... alongside the existing imports

describe("idle-skip", () => {
  function makeIdleEngine(overrides: Partial<import("@shadowcat/core").PerformanceSettings> = {}) {
    const store = new DocumentStore();
    const assets = new AssetResolver();
    const backend = new MockBackend();
    const engine = new RenderEngine({
      store, assets, backend, grid: { kind: "square", size: 100 },
      performance: () => ({ ...PRESETS.quality, idleSkip: true, ...overrides }),
    });
    engine.start();
    backend.renderCount = 0; // discard start()'s own initial-reconcile render
    return { engine, backend, store };
  }

  it("N idle ticks call render() 0 times", () => {
    const { backend } = makeIdleEngine();
    backend.runTicker(16);
    backend.runTicker(16);
    backend.runTicker(16);
    expect(backend.renderCount).toBe(0);
  });

  it("a setCameraTransform makes the next tick render exactly once", () => {
    const { engine, backend } = makeIdleEngine();
    backend.runTicker(16);
    expect(backend.renderCount).toBe(0);
    engine.applyCamera();
    backend.runTicker(16);
    expect(backend.renderCount).toBe(1);
    backend.runTicker(16);
    expect(backend.renderCount).toBe(1); // consumed; back to idle
  });

  it("an in-flight tween renders every tick until it settles", () => {
    const { engine, backend, store } = makeIdleEngine();
    backend.runTicker(16);
    expect(backend.renderCount).toBe(0);
    // A confirmed position change starts a TokenAnimator tween: every tick pushes a
    // `setToken`, which `wrapDirtyTracking` marks dirty. Same shape as this file's existing
    // move tests: `tokenCmd` seeds the token, an update command moves it.
    store.applyCommand(tokenCmd(1, "t1", 0));
    backend.runTicker(16); // the create's own reconcile render
    store.applyCommand({ seq: 2, world_id: "w1", author: "a", ts: 0, ops: [{ op: "update", doc_id: "t1", changes: [{ path: "/engine/x", old: 0, new: 400 }] }] });
    const before = backend.renderCount;
    backend.runTicker(16);
    backend.runTicker(16);
    expect(backend.renderCount).toBe(before + 2);
    backend.runTicker(1000); // past the tween's duration: it settles
    const settled = backend.renderCount;
    backend.runTicker(16);
    expect(backend.renderCount).toBe(settled); // idle again
  });

  it("!idleSkip renders every tick regardless of dirty state", () => {
    const { backend } = makeIdleEngine({ idleSkip: false });
    backend.runTicker(16);
    backend.runTicker(16);
    expect(backend.renderCount).toBe(2);
  });

  it("pushes setFrameCap/setRenderScale on the first tick and only again on change", () => {
    const { backend } = makeIdleEngine({ fpsCap: 30, renderScale: 0.75 });
    backend.runTicker(16);
    expect(backend.frameCap).toBe(30);
    expect(backend.renderScale).toBe(0.75);
  });

  it("maps fpsCap uncapped to a 0 frame cap", () => {
    const { backend } = makeIdleEngine({ fpsCap: "uncapped" });
    backend.runTicker(16);
    expect(backend.frameCap).toBe(0);
  });
});

describe("lighting budget", () => {
  it("off clears the lighting overlay and never forwards a committed frame", () => {
    const store = new DocumentStore();
    const backend = new MockBackend();
    const engine = new RenderEngine({
      store, assets: new AssetResolver(), backend, grid: { kind: "square", size: 100 },
      performance: () => ({ ...PRESETS.quality, lighting: "off" }),
    });
    engine.start();
    expect(backend.lighting).toEqual({ cell: 0, cells: [], darkness: [] });
  });
});
```

- [ ] **Step 3:** implement Steps 2–7 above; `pnpm --filter @shadowcat/render test`,
  `pnpm --filter @shadowcat/render typecheck` (EXPECTED TO FAIL with an excess-argument error on
  the `new TokenView(...)` line until Task 7 widens the constructor — the same interim state Task 5
  documents; note it and proceed, Task 7's checklist requires it green), `pnpm lint`, `pnpm lint:docs`,
  `pnpm lint:comments`, `pnpm docs:check-examples` PASS.
- [ ] **Step 4:** `git commit -m "feat(render): RenderEngine idle-skip, frame-cap/render-scale push, stats, lighting/reducedMotion budgets" -- src/client/render/src/engine.ts src/client/render/src/engine.test.ts src/client/render/src/token-view.ts`

## Task 7: token-fx budget + reduced-motion snap

**Files:**
- Modify: `src/client/render/src/token-view.ts` (constructor gains `tokenFx`/`reducedMotion`
  getters — already called with these two trailing args by Task 6's `RenderEngine` constructor
  edit; this task adds the PARAMETERS themselves and the `toSpec` gate).
- Modify: `src/client/render/src/token-animator.ts` (constructor gains `reducedMotion`; snaps in
  `startAnim` and `animateSamples`).
- Modify: `src/client/render/src/token-view.test.ts`, `src/client/render/src/token-animator.test.ts`.

**Step 1 — failing tests.** Add to `token-animator.test.ts`:
```ts
describe("reducedMotion", () => {
  it("startAnim (setTarget on an existing token) snaps to the end pose immediately", () => {
    const a = new TokenAnimator(() => true);
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    a.setConfig({ speedCellsPerSec: 6, easing: "linear", worldUnitsPerCell: 100 });
    a.setTarget("t1", { x: 100, y: 0, rotation: 0 });
    expect(a.get("t1")).toEqual({ x: 100, y: 0, rotation: 0 });
    expect(a.tick(16)).toEqual([]); // nothing left to animate
  });

  it("animateSamples snaps straight to the last sample", () => {
    const a = new TokenAnimator(() => true);
    a.animateSamples("t1", [{ tMs: 0, pos: [0, 0] }, { tMs: 500, pos: [100, 0] }], 500, 0);
    expect(a.get("t1")).toEqual({ x: 100, y: 0, rotation: 0 });
  });
});
```
Add to `token-view.test.ts`:
```ts
describe("tokenFx budget", () => {
  it("tokenFx: false drops condition fx and keeps only the selection highlight", () => {
    // Arrange a document/store fixture with a condition that carries fx (mirror an existing
    // "condition fx" test in this file for the exact document shape), plus the token selected.
    const view = new TokenView(store, assets, backend, undefined, undefined, undefined, () => new Set([tokenId]), () => false);
    view.reconcile();
    const spec = view.specOf(tokenId)!;
    expect(spec.fx).toEqual([{ kind: "highlight", color: expect.any(Number), strength: expect.any(Number) }]);
  });
});
```
(Read an EXISTING condition-fx test in `token-view.test.ts` first — mirror its exact store/doc
setup for `store`/`assets`/`backend`/`tokenId` rather than inventing new fixture shapes.)

**Step 2 — implement `token-animator.ts`.** Add a constructor:
```ts
  /** @param reducedMotion Resolves the live `PerformanceSettings.reducedMotion` flag, read fresh
   * per `startAnim`/`animateSamples` call. Defaults to always-`false` (legacy/test callers keep
   * today's tweened behavior). */
  constructor(private readonly reducedMotion: () => boolean = () => false) {}
```
Modify `startAnim` — insert as the FIRST statement of its body (before `const segLen: number[] =
[];`):
```ts
    if (this.reducedMotion()) {
      const last = poly[poly.length - 1];
      if (Number.isFinite(last[0]) && Number.isFinite(last[1])) {
        this.cur.set(id, { x: last[0], y: last[1], rotation: finalRot });
      }
      this.anim.delete(id);
      return;
    }
```
Modify `animateSamples` — insert as the FIRST statement after the existing `if (samples.length
=== 0) return;` guard:
```ts
    if (this.reducedMotion()) {
      const last = samples[samples.length - 1];
      this.cur.set(id, { x: last.pos[0], y: last.pos[1], rotation: this.cur.get(id)?.rotation ?? 0 });
      this.anim.delete(id);
      this.samplesAnim.delete(id);
      this.hidden.delete(id);
      return;
    }
```
Update both methods' doc comments to state the reducedMotion short-circuit (cite `PerformanceSettings.reducedMotion`).

**Step 3 — implement `token-view.ts`.** Change the field declaration:
```ts
  /** Drives every tracked token's tween/sample-playback transform. Constructed in the
   * constructor body (not a field initializer) so it can take the `reducedMotion` getter as a
   * constructor parameter — see the constructor. */
  private readonly animator: TokenAnimator;
```
Change the constructor signature and body:
```ts
  constructor(
    private readonly store: ReadableDocuments,
    private readonly assets: AssetResolver,
    private readonly backend: DisplayBackend,
    private readonly viewedSceneId: () => string | null = () => null,
    private readonly footprints: () => FootprintLookup = () => EMPTY_FOOTPRINTS,
    private readonly perceived: () => ReadonlySet<string> = () => NO_PERCEIVED,
    private readonly selectedTokens: () => ReadonlySet<string> = () => EMPTY_TOKEN_SELECTION,
    private readonly tokenFx: () => boolean = () => true,
    reducedMotion: () => boolean = () => false,
  ) {
    this.animator = new TokenAnimator(reducedMotion);
  }
```
Also add doc lines for the two new parameters (mirroring the existing `@param` style above this
constructor): `@param tokenFx Resolves the live \`PerformanceSettings.tokenFx\` flag; \`false\`
drops every condition-driven fx entry in \`toSpec\`, keeping only the selection highlight.
Defaults to always-\`true\`.` and `@param reducedMotion Resolves the live
\`PerformanceSettings.reducedMotion\` flag, forwarded to the internal \`TokenAnimator\`. Defaults
to always-\`false\`.`

Modify `toSpec`'s fx line:
```ts
    const fx: TokenFx[] = this.tokenFx()
      ? conditions.flatMap((c) => (c.fx ? conditionFxEntries(c.fx) : []))
      : [];
```

- [ ] **Step 1:** write the failing tests above (reading an existing condition-fx fixture in
  `token-view.test.ts` first to match its exact shape).
- [ ] **Step 2:** implement `token-animator.ts` + `token-view.ts` per Steps 2–3 above.
- [ ] **Step 3:** `pnpm --filter @shadowcat/render test`, `pnpm --filter @shadowcat/render
  typecheck`, `pnpm -r typecheck`, `pnpm lint`, `pnpm lint:docs`, `pnpm lint:comments` PASS.
- [ ] **Step 4:** `git commit -m "feat(render): tokenFx and reducedMotion budgets on TokenView/TokenAnimator" -- src/client/render/src/token-view.ts src/client/render/src/token-view.test.ts src/client/render/src/token-animator.ts src/client/render/src/token-animator.test.ts`

## Task 8: `Stage.svelte` — antialias re-init, data-* attributes

Read `src/modules/stage/src/Stage.svelte` in full before editing (already read for this plan).

**Files:**
- Modify: `src/modules/stage/src/Stage.svelte`, `src/modules/stage/src/Stage.test.ts`.

**Step 1 — failing test.** Add to `Stage.test.ts` (mirror an existing `$effect`-teardown test's
setup — e.g. the one asserting `createBackend` is called once — for the exact `render(Stage,
{...})` context/props shape):
```ts
it("toggling antialias re-creates the backend once and destroys the old one", async () => {
  const backend1 = fakeBackend();
  const backend2 = fakeBackend();
  const createBackend = vi.fn(async () => (createBackend.mock.calls.length === 1 ? backend1 : backend2));
  const controller = new PerformanceController();
  controller.setPreset("quality"); // antialias: true
  const { rerender } = render(Stage, {
    props: { createBackend },
    context: setAppContextForTest({ performance: controller }),
  });
  await vi.waitFor(() => expect(createBackend).toHaveBeenCalledOnce());
  expect(backend1.destroyed).toBe(false);
  controller.set({ antialias: false });
  await vi.waitFor(() => expect(createBackend).toHaveBeenCalledTimes(2));
  expect(backend1.destroyed).toBe(true);
});
```
(Import `PerformanceController` from `@shadowcat/ui-kit`. The file's existing `fakeBackend()`
helper implements `destroy() { this.destroyed = true; }` as a plain method, which is why the
snippet asserts the boolean `destroyed` flag like every existing test in the file — never
`toHaveBeenCalled*` on a non-mock method.)

**Step 2 — implement.** Widen the `createBackend` prop's type and default, and read the tracked
antialias value as literally the FIRST statement of the mount `$effect`:
```svelte
  let {
    createBackend = (canvas: HTMLCanvasElement, opts: { antialias: boolean }): Promise<DisplayBackend> =>
      createPixiBackend(canvas, {
        background: readColor("--surface-base", 0x101014),
        antialias: import.meta.env.VITE_SC_ANTIALIAS !== "0" && opts.antialias,
      }),
    logger,
  }: {
    createBackend?: (canvas: HTMLCanvasElement, opts: { antialias: boolean }) => Promise<DisplayBackend>;
    logger?: Logger;
  } = $props();
```
In the mount `$effect`, insert as the literal first statement (before `let engine: RenderEngine
| null = null;`):
```ts
    // Tracked FIRST: antialias cannot change after Pixi init (PixiBackendOptions.antialias's
    // doc), so a change re-runs this whole effect, tearing down and rebuilding both the engine
    // and the backend (the destroy() path below is exercised, never leaked).
    const antialias = ctx.performance.current.antialias;
```
Change the `createBackend(canvas)` call site inside the async IIFE to
`createBackend(canvas, { antialias })`. Add `performance: () => ctx.performance.current,` and
`onStats: (s) => ctx.performance.recordStats(s),` to the `RenderEngine({...})` options object
(alongside the existing `selectedTokens: () => ctx.tokenSelection.ids,` line).

Add the three data-* attributes to `onDocs()` (alongside the existing
`host.dataset.tokenCount = ...` line):
```ts
        const perf = ctx.performance.current;
        host.dataset.fpsCap = perf.fpsCap === "uncapped" ? "0" : String(perf.fpsCap);
        host.dataset.renderScale = String(perf.renderScale);
        host.dataset.idleSkip = perf.idleSkip ? "1" : "0";
```

- [ ] **Step 1:** write the failing test above.
- [ ] **Step 2:** implement the edits above; `pnpm --filter @shadowcat/module-stage test`,
  `pnpm -r typecheck`, `pnpm lint`, `pnpm lint:docs`, `pnpm lint:comments` PASS.
- [ ] **Step 3:** `git commit -m "feat(stage): performance-driven backend init, frame stats, data-* signals" -- src/modules/stage/src/Stage.svelte src/modules/stage/src/Stage.test.ts`

## Task 9: `PerformanceEditor.svelte` (settings) + locale keys

Read `src/modules/settings/src/Settings.svelte` and `ThemeEditor.svelte` in full before editing
(already read for this plan).

**Files:**
- Create: `src/modules/settings/src/PerformanceEditor.svelte`, `src/modules/settings/src/PerformanceEditor.test.ts`.
- Modify: `src/modules/settings/src/Settings.svelte` (insert `<PerformanceEditor />` between the
  theme block and the `{#if role === "gm"}<ModuleManager />{/if}` conditional).
- Modify: `src/client/ui-kit/src/locales/en.ts` (new `performance.` group, appended after the
  last existing group per that file's flat-key convention).

**Step 1 — locale keys.** Append to `en.ts` (flat dotted keys, matching the existing
`"settings.theme.*"` style seen at the top of that file):
```ts
  "performance.title": "Performance",
  "performance.preset": "Preset",
  "performance.preset.auto": "Auto",
  "performance.preset.mobile": "Mobile",
  "performance.preset.balanced": "Balanced",
  "performance.preset.quality": "Quality",
  "performance.preset.custom": "Custom",
  "performance.fpsCap": "Frame-rate cap",
  "performance.uncapped": "Uncapped",
  "performance.renderScale": "Render scale",
  "performance.lighting": "Lighting quality",
  "performance.lighting.full": "Full (per-frame)",
  "performance.lighting.static": "Static (no sweeps)",
  "performance.lighting.off": "Off",
  "performance.antialias": "Antialiasing",
  "performance.tokenFx": "Token effects",
  "performance.vfx": "Visual effects",
  "performance.dice3d": "3D dice",
  "performance.spatialAudio": "Spatial audio",
  "performance.idleSkip": "Skip redraws when idle",
  "performance.reducedMotion": "Reduce motion",
  "performance.showStats": "Show frame stats",
  "performance.reset": "Reset to auto",
```

**Step 2 — TDD `PerformanceEditor.test.ts`.**
```ts
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { PerformanceController } from "@shadowcat/ui-kit";
import PerformanceEditor from "./PerformanceEditor.svelte";

describe("PerformanceEditor", () => {
  function renderEditor(controller = new PerformanceController()) {
    render(PerformanceEditor, { context: setAppContextForTest({ performance: controller }) });
    return controller;
  }

  it("selecting a preset radio calls setPreset", async () => {
    const controller = renderEditor();
    await fireEvent.click(screen.getByTestId("perf-preset-mobile"));
    expect(controller.preset).toBe("mobile");
    expect(controller.current).toMatchObject({ fpsCap: 30 });
  });

  it("editing a single field shows custom", async () => {
    const controller = renderEditor();
    await fireEvent.change(screen.getByTestId("perf-fps-cap"), { target: { value: "30" } });
    expect(controller.preset).toBe("custom");
    expect(screen.getByText("performance.preset.custom")).toBeInTheDocument();
  });

  it("reset returns to auto", async () => {
    const controller = renderEditor();
    await fireEvent.change(screen.getByTestId("perf-fps-cap"), { target: { value: "30" } });
    await fireEvent.click(screen.getByTestId("perf-reset"));
    expect(controller.preset).toBe("auto");
  });

  it("every control is labelled", () => {
    renderEditor();
    for (const id of ["perf-fps-cap", "perf-render-scale", "perf-lighting", "perf-antialias", "perf-token-fx", "perf-vfx", "perf-dice3d", "perf-spatial-audio", "perf-idle-skip", "perf-reduced-motion", "perf-show-stats"]) {
      expect(screen.getByTestId(id).closest("label")).not.toBeNull();
    }
  });
});
```

**Step 3 — implement `PerformanceEditor.svelte`** (per this plan's earlier design; controls: a
preset radio group, an fps-cap select, a render-scale range, a lighting select, seven boolean
checkboxes, a show-stats checkbox, a reset button — every label via `t(...)` under the
`performance.` group, every control `data-testid`'d for the e2e spec):
```svelte
<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { PerformancePreset, PerformanceSettings } from "@shadowcat/core";

  const { t, performance } = getAppContext();

  const PRESET_IDS: PerformancePreset[] = ["auto", "mobile", "balanced", "quality"];
  const FPS_CAPS: PerformanceSettings["fpsCap"][] = [30, 60, 120, "uncapped"];
  const LIGHTING_MODES: PerformanceSettings["lighting"][] = ["full", "static", "off"];

  type BooleanKey = "antialias" | "tokenFx" | "vfx" | "dice3d" | "spatialAudio" | "idleSkip" | "reducedMotion";

  function setFpsCap(e: Event): void {
    const raw = (e.currentTarget as HTMLSelectElement).value;
    performance.set({ fpsCap: raw === "uncapped" ? "uncapped" : (Number(raw) as 30 | 60 | 120) });
  }
  function setRenderScale(e: Event): void {
    performance.set({ renderScale: Number((e.currentTarget as HTMLInputElement).value) });
  }
  function setLighting(e: Event): void {
    performance.set({ lighting: (e.currentTarget as HTMLSelectElement).value as PerformanceSettings["lighting"] });
  }
  function toggle(key: BooleanKey) {
    return (e: Event) => performance.set({ [key]: (e.currentTarget as HTMLInputElement).checked });
  }
</script>

<fieldset class="perf-editor">
  <legend>{t("performance.title")}</legend>
  <div class="preset-row" role="radiogroup" aria-label={t("performance.preset")}>
    {#each PRESET_IDS as p (p)}
      <label>
        <input
          type="radio"
          name="perf-preset"
          data-testid={`perf-preset-${p}`}
          checked={performance.preset === p}
          onchange={() => performance.setPreset(p)}
        />
        {t(`performance.preset.${p}`)}
      </label>
    {/each}
    {#if performance.preset === "custom"}
      <span class="custom-badge">{t("performance.preset.custom")}</span>
    {/if}
  </div>
  <label>{t("performance.fpsCap")}
    <select data-testid="perf-fps-cap" value={String(performance.current.fpsCap)} onchange={setFpsCap}>
      {#each FPS_CAPS as cap (cap)}
        <option value={String(cap)}>{cap === "uncapped" ? t("performance.uncapped") : cap}</option>
      {/each}
    </select>
  </label>
  <label>{t("performance.renderScale")}
    <input
      type="range"
      data-testid="perf-render-scale"
      min="0.5" max="1" step="0.05"
      value={performance.current.renderScale}
      oninput={setRenderScale}
    />
    <span>{performance.current.renderScale.toFixed(2)}</span>
  </label>
  <label>{t("performance.lighting")}
    <select data-testid="perf-lighting" value={performance.current.lighting} onchange={setLighting}>
      {#each LIGHTING_MODES as mode (mode)}
        <option value={mode}>{t(`performance.lighting.${mode}`)}</option>
      {/each}
    </select>
  </label>
  <label><input type="checkbox" data-testid="perf-antialias" checked={performance.current.antialias} onchange={toggle("antialias")} /> {t("performance.antialias")}</label>
  <label><input type="checkbox" data-testid="perf-token-fx" checked={performance.current.tokenFx} onchange={toggle("tokenFx")} /> {t("performance.tokenFx")}</label>
  <label><input type="checkbox" data-testid="perf-vfx" checked={performance.current.vfx} onchange={toggle("vfx")} /> {t("performance.vfx")}</label>
  <label><input type="checkbox" data-testid="perf-dice3d" checked={performance.current.dice3d} onchange={toggle("dice3d")} /> {t("performance.dice3d")}</label>
  <label><input type="checkbox" data-testid="perf-spatial-audio" checked={performance.current.spatialAudio} onchange={toggle("spatialAudio")} /> {t("performance.spatialAudio")}</label>
  <label><input type="checkbox" data-testid="perf-idle-skip" checked={performance.current.idleSkip} onchange={toggle("idleSkip")} /> {t("performance.idleSkip")}</label>
  <label><input type="checkbox" data-testid="perf-reduced-motion" checked={performance.current.reducedMotion} onchange={toggle("reducedMotion")} /> {t("performance.reducedMotion")}</label>
  <label><input type="checkbox" data-testid="perf-show-stats" checked={performance.showStats} onchange={(e) => performance.setShowStats((e.currentTarget as HTMLInputElement).checked)} /> {t("performance.showStats")}</label>
  <button type="button" data-testid="perf-reset" onclick={() => performance.setPreset("auto")}>{t("performance.reset")}</button>
</fieldset>

<style lang="scss">
  .perf-editor {
    display: grid;
    gap: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    padding: var(--space-3);
  }
  .preset-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  @media (pointer: coarse) {
    input[type="checkbox"],
    input[type="radio"] {
      min-width: var(--input-height-coarse);
      min-height: var(--input-height-coarse);
    }
  }
  @media (max-width: 400px) {
    .perf-editor {
      padding: var(--space-2);
    }
  }
</style>
```

**Step 4 — wire `Settings.svelte`.** Add `import PerformanceEditor from "./PerformanceEditor.svelte";`
alongside the other imports, and insert `<PerformanceEditor />` immediately after the `{/if}`
that closes the theme-editor block and before `{#if role === "gm"}<ModuleManager />{/if}`.

- [ ] **Step 1:** locale keys added.
- [ ] **Step 2:** failing test written.
- [ ] **Step 3:** `PerformanceEditor.svelte` implemented; `Settings.svelte` wired.
- [ ] **Step 4:** `pnpm --filter @shadowcat/module-settings test`, `pnpm -r typecheck`, `pnpm
  lint`, `pnpm lint:docs`, `pnpm lint:props`, `pnpm lint:aria-labels`, `pnpm docs:check-examples`
  PASS.
- [ ] **Step 5:** `git commit -m "feat(settings): PerformanceEditor built-in section" -- src/modules/settings/src/PerformanceEditor.svelte src/modules/settings/src/PerformanceEditor.test.ts src/modules/settings/src/Settings.svelte src/client/ui-kit/src/locales/en.ts`

## Task 10: `PerfStats.svelte` (statusbar)

Read `src/modules/statusbar/src/StatusBar.svelte` in full before editing (already read for this
plan).

**Files:**
- Create: `src/modules/statusbar/src/PerfStats.svelte`, `src/modules/statusbar/src/PerfStats.test.ts`.
- Modify: `src/modules/statusbar/src/StatusBar.svelte`.

**Step 1 — failing test.**
```ts
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { PerformanceController } from "@shadowcat/ui-kit";
import PerfStats from "./PerfStats.svelte";

describe("PerfStats", () => {
  it("renders nothing when showStats is off", () => {
    const controller = new PerformanceController();
    render(PerfStats, { context: setAppContextForTest({ performance: controller }) });
    expect(screen.queryByTestId("perf-stats")).toBeNull();
  });

  it("shows fps/frameMs when showStats is on", () => {
    const controller = new PerformanceController();
    controller.setShowStats(true);
    controller.recordStats({ fps: 60, frameMs: 4 });
    render(PerfStats, { context: setAppContextForTest({ performance: controller }) });
    expect(screen.getByTestId("perf-stats").textContent).toContain("60");
  });
});
```

**Step 2 — implement `PerfStats.svelte`.**
```svelte
<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  const { performance } = getAppContext();
</script>

{#if performance.showStats}
  <span class="perf-stats" data-testid="perf-stats">
    {performance.stats.fps} fps · {performance.stats.frameMs.toFixed(1)} ms
  </span>
{/if}

<style lang="scss">
  .perf-stats {
    font-size: var(--font-size-caption);
    color: var(--text-muted);
    white-space: nowrap;
  }
</style>
```

**Step 3 — wire `StatusBar.svelte`.**
```svelte
<script lang="ts">
  import { getAppContext, Surface } from "@shadowcat/ui-kit";
  import PerfStats from "./PerfStats.svelte";
  const { role } = getAppContext();
</script>

<footer class="statusbar">
  <span>{role}</span>
  <PerfStats />
  <div class="dock"><Surface contract="shadowcat.surface:panel-dock" /></div>
</footer>
```
(The `<style>` block is unchanged.)

- [ ] **Step 1:** failing test written.
- [ ] **Step 2:** `PerfStats.svelte` implemented; `StatusBar.svelte` wired.
- [ ] **Step 3:** `pnpm --filter @shadowcat/module-statusbar test`, `pnpm -r typecheck`, `pnpm
  lint`, `pnpm lint:docs`, `pnpm lint:aria-labels` PASS.
- [ ] **Step 4:** `git commit -m "feat(statusbar): PerfStats readout" -- src/modules/statusbar/src/PerfStats.svelte src/modules/statusbar/src/PerfStats.test.ts src/modules/statusbar/src/StatusBar.svelte`

## Task 11: Playwright spec (written here, run by the dispatcher)

Read `src/client/shell/e2e/fixtures.ts` and `src/client/shell/e2e/combat-settings.spec.ts` in
full before writing (already read for this plan — the `login`/`account`/`createAccount` fixture
shapes and the `stageHost`/launcher/settings-panel-open sequence).

**Files:**
- Create: `src/client/shell/e2e/performance.spec.ts`.

```ts
import { test, expect, login } from "./fixtures";
import type { Page } from "@playwright/test";

function stageHost(page: Page) {
  return page.locator(".stage-host");
}

async function openSettings(page: Page): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-settings:panel").click();
}

test("the performance editor drives the stage's frame-cap/render-scale/idle-skip signals and the statusbar readout", async ({ page, account }) => {
  const worldName = `Performance World ${Date.now().toString(36)}`;
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(worldName);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(page)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await openSettings(page);
  await page.getByTestId("perf-preset-mobile").click();
  await expect(stageHost(page)).toHaveAttribute("data-fps-cap", "30");
  await expect(stageHost(page)).toHaveAttribute("data-idle-skip", "1");

  // Toggle stats: the statusbar readout appears.
  await page.getByTestId("perf-show-stats").check();
  await expect(page.getByTestId("perf-stats")).toBeVisible();
  await page.getByTestId("perf-show-stats").uncheck();
  await expect(page.getByTestId("perf-stats")).toBeHidden();

  await page.getByTestId("perf-preset-quality").click();
  await expect(stageHost(page)).toHaveAttribute("data-fps-cap", "0");
});
```

- [ ] **Step 1:** write the spec above (behavior verified against `PerformanceEditor.svelte`'s
  and `Stage.svelte`'s actual test ids/attributes from Tasks 8–9 before this task starts).
- [ ] **Step 2:** `pnpm --filter @shadowcat/shell typecheck`, `pnpm lint` PASS. Do NOT run the
  suite — the dispatcher runs it.
- [ ] **Step 3:** `git commit -m "test(e2e): performance settings drive the stage and statusbar (written; dispatcher runs)" -- src/client/shell/e2e/performance.spec.ts`

## Task 12: docs, HISTORY.md, skills

**Files:**
- Modify: `docs/site/modules/settings.md` (Performance section: a new row in the Contributions
  table is not needed — `PerformanceEditor` is a built-in section, not a contribution — instead
  add a `## Performance` subsection under `## Components` naming `PerformanceEditor.svelte` and
  its ten controls, and cross-link the new guide page).
- Modify: `docs/site/modules/statusbar.md` (add `PerfStats.svelte` to `## Components`).
- Create: `docs/site/guides/performance.md` (what each of the ten `PerformanceSettings` fields
  costs and what turning it off buys; the `mobile`/`balanced`/`quality` presets table verbatim
  from `PRESETS`; a short "why auto?" section citing `resolveAuto`'s three signals).
- Modify: `docs/site/.vitepress/config.mts` (one sidebar entry: `{ text: "Performance", link:
  "/guides/performance" }`, appended to the `/guides/` group's `items` array after "Creating a
  system").
- Modify: `docs/HISTORY.md` (append the M22 entry under "## Phase 3 — Atmosphere", creating that
  heading if absent — master §3's convention: only the LAST Phase-3 milestone to merge flips
  `docs/PLAN.md`'s heading, so `docs/PLAN.md` is untouched by this task).
- Skills (plugin checkout `C:/Users/emper/.claude/skills/shadowcat-codebase/`, edited WITHOUT
  committing there):
  - Create `skills/shadowcat-codebase-performance/SKILL.md` (fixed shape — Purpose / Key files &
    seams / Hard invariants / Gotchas / Pointers): Purpose states the render-budget goal;
    Key files & seams lists `src/client/core/src/performance.ts` (the owned seam type),
    `src/client/ui-kit/src/performance.svelte.ts`, `src/modules/settings/src/PerformanceEditor.svelte`,
    `src/modules/statusbar/src/PerfStats.svelte`, and the render-side seams
    `RenderEngineOpts.performance`/`onStats` and `DisplayBackend.setFrameCap`/`setRenderScale`/
    `render` in `src/client/render/src/{engine,pixi-backend,backend,backend.mock,dirty-backend}.ts`;
    Hard invariants: `effectiveSettings` is the ONE combination point, `wrapDirtyTracking` is the
    ONE dirty-flag source, antialias needs a backend re-create (never a live toggle), `lighting:
    "off"` never touches fog/vision secrecy; Gotchas: the ui-kit singleton is named `performanceController` so it never shadows
    `globalThis.performance`; a component that destructures `const { performance } =
    getAppContext()` must reach the real Performance API as `globalThis.performance`, `tickTokenAnimations` is
    deliberately excluded from dirty-tracking; Pointers: this plan's spec file, master §2.1.
  - Modify `hooks/codebase-skill-reminder.py`'s `SUBSYSTEMS` map: add a `performance` entry with
    globs `src/client/core/src/performance\.ts`, `src/client/ui-kit/src/performance\.svelte\.ts`,
    `src/modules/settings/src/PerformanceEditor`, `src/modules/statusbar/src/PerfStats`.
  - Modify `hooks/test-codebase-skill-reminder.sh`: add one `check` line per new glob with an
    ABSOLUTE Windows-style path, e.g. `C:/Dev/Shadowcat/src/client/core/src/performance.ts`; run
    the script with `bash` from the plugin checkout root and paste its output into the final
    report.
  - Update `skills/shadowcat-codebase-client-shell/SKILL.md`: add the performance mirror
    (`readPerformanceMirror`/`writePerformanceMirror`, `main.ts`'s load call) to its `sessionState.svelte.ts`
    coverage.
  - Update `skills/shadowcat-codebase-scene-rendering/SKILL.md`: add the idle-skip dirty-tracking
    seam (`wrapDirtyTracking`), the frame-cap/render-scale/antialias budget, and the
    lighting/reducedMotion budgets to its `RenderEngine`/`DisplayBackend` coverage.

- [ ] **Step 1:** doc pages + sidebar + HISTORY.md edits; `pnpm docs:check-examples` PASS (no
  Rust examples added, so `pnpm docs:check-rust-examples` is a no-op check here, still run it).
- [ ] **Step 2:** skill edits (create + two updates) in the plugin checkout, per the shape above;
  `node scripts/check-skill-symbol-refs-cli.mjs`, `node scripts/check-skill-api-refs-cli.mjs`
  (needs `pnpm build:all`'s `dist-docs`), `pnpm run test:scripts` from the worktree — zero broken
  citations introduced.
- [ ] **Step 3:** dispatch `shadowcat-codebase:shadowcat-spec-reviewer` (sonnet, effort high) on
  the skill diff alone (new file + two SKILL.md updates + the two hook-map edits); apply any
  finding the dispatcher agrees with, else it is a design question for the user.
- [ ] **Step 4:** commit + push the skill-checkout edit from INSIDE
  `C:/Users/emper/.claude/skills/shadowcat-codebase/` (its own git remote, a separate repository
  from this one) — never as part of a commit made in `C:/Dev/Shadowcat-m22`.
- [ ] **Step 5:** `git commit -m "docs: performance settings guide, module pages, HISTORY entry" -- docs/site/ docs/HISTORY.md`

## Task 13: merge-forward + integration (final task)

Per master §5, M22 is FIRST in the merge order and consumes no other milestone's seam, so this
task is short — no foreign seam to wire up, only the standard merge-forward protocol (master §5).

**Files:** none owned by this task beyond what the merge itself touches (conflict resolution, if
any, per master §3's conventions — M22 touches no shared-file row in that table except
`appContext.ts`/`appContextTest.ts`/`en.ts`, each an ADDITIVE append per that table's own
convention, so a genuine conflict is unlikely against a still-empty `main`).

- [ ] **Step 1:** `git fetch origin && git merge origin/main` in the worktree (a merge commit,
  never a rebase). Since M22 is the FIRST Phase-3 milestone to merge, `origin/main` at this point
  is Phase-2's own `main` (no other Phase-3 seam exists yet to conflict with) — resolve anything
  unexpected per master §3's conventions and log it in the final report.
- [ ] **Step 2:** dispatcher pre-generates `git diff main...HEAD` to a file (reviewers have no
  Bash tool) and dispatches `shadowcat-codebase:shadowcat-spec-reviewer` +
  `shadowcat-codebase:shadowcat-code-reviewer` (both sonnet, effort high) against it, blind to
  each other, per this plan's Buddy-check directives. Apply agreed findings; escalate disputed
  ones to the user.
- [ ] **Step 3:** run the FULL gate battery from this plan's Global constraints section
  (background the long-running ones, read the log before claiming green); paste every result
  line in the final report.
- [ ] **Step 4:** `pnpm gate:push` (tree-keyed receipt) immediately before `git push`.
- [ ] **Step 5:** `git commit` the merge (append the trailer to its auto-generated message; a
  merge commit carries its own message, do not `--no-edit` past a real conflict resolution) --
  the merge's own changed paths.
- [ ] **Step 6:** open the PR into `main` (branch-protected, `--auto` off — master §5). Report:
  STATUS, every commit hash + subject, every gate result line, the browser suite NOT RUN (the
  dispatcher runs `performance.spec.ts`), the plugin-repo skill diff stat + its commit hash, and
  any deviation from this plan.
