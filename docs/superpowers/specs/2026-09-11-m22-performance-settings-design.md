# M22 — Performance settings + render budget — Design Spec

> Master: `2026-09-11-phase3-master-integration-design.md` (§2.1 owns the seam this
> milestone defines; §3 the shared-file conventions; §4 the global constraints; §9 D1).
> Goal: Shadowcat runs acceptably on a phone. The user can cap the frame rate, lower the
> render resolution, and switch off the expensive layers; the client picks a sane preset on
> its own; the stage stops burning a core while nothing changes.

## 1. Problem

`RenderEngine`'s ticker (installed in `start()` through `DisplayBackend.startTicker`) redraws
the FULL canvas every tick whether or not anything changed — the core skill records this as
the reason a GPU-less host pays continuously. `createPixiBackend` fixes `resolution` to
`devicePixelRatio` and `antialias` to `true` at init with no knob; token fx filters, the
photometric lighting overlay and the vision cross-fades run unconditionally. Nothing in the
client reads `prefers-reduced-motion`. On a mid-range Android phone the stage runs the CPU
flat out and the battery down.

## 2. Data — `@shadowcat/core` `src/client/core/src/performance.ts`

```ts
export interface PerformanceSettings { … }           // exactly master §2.1
export type PerformancePreset = "auto" | "mobile" | "balanced" | "quality" | "custom";

/** Device signals `resolveAuto` reads; every field optional so a test can pin any subset. */
export interface DeviceSignals {
  coarsePointer?: boolean;          // matchMedia("(pointer: coarse)")
  compact?: boolean;                // ui-kit `sizeClass() === "compact"`
  hardwareConcurrency?: number;     // navigator.hardwareConcurrency
  deviceMemoryGb?: number;          // navigator.deviceMemory (Chromium only; absent elsewhere)
  reducedMotion?: boolean;          // matchMedia("(prefers-reduced-motion: reduce)")
}

export const PRESETS: Record<Exclude<PerformancePreset, "auto" | "custom">, PerformanceSettings> = {
  mobile:   { fpsCap: 30, renderScale: 0.75, antialias: false, tokenFx: false, lighting: "static",
              vfx: false, dice3d: false, spatialAudio: true, idleSkip: true, reducedMotion: false },
  balanced: { fpsCap: 60, renderScale: 1, antialias: true, tokenFx: true, lighting: "full",
              vfx: true, dice3d: true, spatialAudio: true, idleSkip: true, reducedMotion: false },
  quality:  { fpsCap: "uncapped", renderScale: 1, antialias: true, tokenFx: true, lighting: "full",
              vfx: true, dice3d: true, spatialAudio: true, idleSkip: true, reducedMotion: false },
};

/** "auto" ⇒ mobile when (coarsePointer && compact) || hardwareConcurrency ≤ 4 || deviceMemoryGb ≤ 4;
 *  else balanced. `reducedMotion` is OR-ed in from the signal on top of the chosen preset. */
export function resolveAuto(signals: DeviceSignals): PerformanceSettings;
/** The persisted shape: `{ preset, overrides }` — overrides are the keys the user edited; the
 *  effective settings are `preset === "custom" ? overrides-as-full : PRESETS[preset]` (auto
 *  resolved through `resolveAuto` at load). */
export interface PersistedPerformance { preset: PerformancePreset; overrides: Partial<PerformanceSettings> }
export function parsePersisted(raw: string | null): PersistedPerformance;   // fail-closed to { preset: "auto", overrides: {} }
export function serializePersisted(p: PersistedPerformance): string;
export function effectiveSettings(p: PersistedPerformance, signals: DeviceSignals): PerformanceSettings;
export const PERFORMANCE_STORAGE_KEY = "shadowcat.performance";
```

- Bounds: `renderScale` clamped to `[0.5, 1]`; every unknown key/wrong-typed value in the
  persisted JSON drops that key (fail closed to the preset), never the whole record.
- `effectiveSettings` is the ONE place preset + overrides + signals combine. `PerformanceEditor`
  and the shell both call it; nothing re-derives it.

## 3. Controller — `@shadowcat/ui-kit` `src/client/ui-kit/src/performance.svelte.ts`

Shape mirrors `theme.svelte.ts` EXACTLY, including its division of responsibility: the
controller never touches `Storage`. A rune-backed singleton
`performance = { current: PerformanceSettings ($state), preset, set(patch), setPreset(p),
stats: { fps: number; frameMs: number } ($state), load(parsed: PersistedPerformance |
undefined, signals: DeviceSignals), serialize(): PersistedPerformance, onChange?: (p:
PersistedPerformance) => void }`. `set(patch)` moves `preset` to `"custom"` (overrides = the
full current object); `setPreset(p)` clears overrides. Both run through `effectiveSettings`
and then call `onChange`, which the shell registers to persist. `AppContext.performance` is
this object (typed as master §2.1 plus `stats`).

Persistence lives in the shell beside the theme mirror (`sessionState.svelte.ts`): pure
`readPerformanceMirror(storage: Storage): PersistedPerformance | undefined` /
`writePerformanceMirror(storage, p)` under `PERFORMANCE_STORAGE_KEY`, the
`readThemeMirror`/`writeThemeMirror` shape. The load call site is
`src/client/shell/src/main.ts`, immediately after the pre-mount
`theme.load(readThemeMirror(localStorage))` line:
`performance.load(readPerformanceMirror(localStorage), readDeviceSignals())` — before `App`
mounts, because the settings are per-device and never come from the server (`ui_state` is
per-account, D1). `readDeviceSignals()` lives in `src/client/shell/src/lib/deviceSignals.ts`
(pure; guards every global on existence so jsdom and node environments read `{}`).

## 4. Render engine — `@shadowcat/render`

### 4.1 Options

`RenderEngineOpts.performance?: () => PerformanceSettings` (a getter, like `viewedSceneId`).
Absent ⇒ `PRESETS.quality` with `idleSkip: false` (legacy/test callers keep today's behaviour).
`Stage.svelte` passes `() => ctx.performance.current`.

### 4.2 Frame cap and resolution — `DisplayBackend` additions (append; `MockBackend` records)

```ts
setFrameCap(fps: number): void;              // 0 = uncapped → Pixi `app.ticker.maxFPS`
setRenderScale(scale: number): void;          // renderer.resolution = dpr*scale; then resize()
render(): void;                               // draw one frame NOW (idle-skip drives this)
```

`createPixiBackend(canvas, opts)` already takes `opts.antialias`; it ALSO removes the
`TickerPlugin`'s automatic render listener at init (`app.ticker.remove(app.render, app)` —
`Ticker.remove` matches on fn + context) so the backend owns the render call. Antialias
cannot change after init, and a template `{#key}` around the `<canvas>` would NOT re-run
`Stage.svelte`'s script-level mount `$effect` (its body delegates to an async IIFE and reads
nothing tracked). The mechanism is therefore: the mount `$effect` reads
`ctx.performance.current.antialias` SYNCHRONOUSLY as its first statement (making it a
tracked dependency), passes it to `createBackend`, and its cleanup destroys engine + backend;
a change re-runs the effect, which rebuilds both (the `destroy()` path is exercised, not
leaked). A test pins that toggling antialias calls `createBackend` a second time and
`destroy()` once.

### 4.3 Idle skip (dirty-flag rendering)

`RenderEngine` keeps a `dirty` flag set by: every reconcile pass that pushed a node, every
`setCameraTransform`, `drawOverlay`/`drawMeasure`/`drawPings`/`drawEmotes`, `setLighting`,
`setVisibility(+Blend)`, `resize`, and by the ticker itself while any token tween, light sweep,
vision sweep, ping ring or emote glyph is in flight. The ticker callback:

```
dt → advance tweens/sweeps/overlays (unchanged)
if (!perf.idleSkip || dirty || animationsInFlight) { backend.render(); dirty = false; }
stats sample (fps = 1000/dtMs EMA over 30 ticks; frameMs measured around render())
```

`RenderEngine.stats` is read by the ui-kit controller through a `RenderEngineOpts.onStats?:
(s) => void` hook (host observability pattern, like `onMeasureDrawn`) at most 4×/s.

### 4.4 Layer budgets

- `tokenFx: false` ⇒ `TokenView.toSpec` drops every CONDITION-driven fx entry (the per-token
  `ColorMatrixFilter`s that scale with the token count) and keeps ONLY the selection
  `highlight` entry (bounded by the selection size, typically one token) — the selection
  signifier stays the single mechanism it is today (the fx entry; never a drawn ring).
- `lighting: "static"` ⇒ `advanceLightSweeps` applies the sweep's final frame immediately (no
  per-frame interpolation); `"off"` ⇒ `setLighting` is not forwarded and the lighting layer is
  cleared (vision/fog masks are untouched — secrecy is not a performance knob).
- `reducedMotion: true` ⇒ token position/rotation tweens (`TokenAnimator` in
  `token-animator.ts` — `startAnim`/`tick`'s polyline interpolation and `applySamplesAt`)
  resolve to their end pose on the first tick (`token-animation.ts` is the sprite
  frame-index helper and is untouched); `fog-blend`/`light-sweep` cross-fades apply the `to`
  frame in one step; ping rings and emote glyphs keep their timing (they are signals, not
  motion).
- `vfx`, `dice3d`, `spatialAudio` are read by their owning milestones (master §2.1); M22 only
  ships the keys.

## 5. UI — `PerformanceEditor.svelte` in `src/modules/settings/src/`

`Settings.svelte` is one flat `<section class="panel">` (locale select, theme select, custom
themes, the conditional `<ThemeEditor>`, then `{#if role === "gm"}<ModuleManager />{/if}`);
`<PerformanceEditor />` is inserted after the theme block and before the `ModuleManager`
conditional, wrapped in its own `<fieldset>` with a `t("performance.title")` legend (not
role-gated — every user tunes their own device). Controls: preset radio (`auto`/`mobile`/`balanced`/`quality`, with
`custom` shown when active), then one control per key (fps select, render-scale range 0.5–1
step 0.05, checkboxes, lighting select), a "Show frame stats" toggle that surfaces
`performance.stats` in the statusbar module (`src/modules/statusbar` gains a `PerfStats.svelte`
readout, hidden unless the toggle is on), and a "Reset to auto" button. Every label is `t(…)`
under the `performance.` i18n group; touch-sized targets; works at 400 px width.

The stage element carries `data-fps-cap`, `data-render-scale` and `data-idle-skip` attributes
reflecting the live settings (the e2e hook — `Stage.svelte` already exposes `data-*` signals).

## 6. Tests

- core `performance.test.ts` (`// @vitest-environment node`): `resolveAuto` truth table (each
  signal alone, none, all); `parsePersisted` drops a bad key and keeps the rest, returns auto on
  garbage/`null`; `effectiveSettings` precedence (custom overrides > preset; auto → signals;
  `reducedMotion` OR); `renderScale` clamp.
- ui-kit `performance.svelte.test.ts`: `set` flips preset to custom and fires `onChange`;
  `setPreset` clears overrides; `load(undefined, signals)` resolves auto.
- shell `sessionState.test.ts`: `readPerformanceMirror`/`writePerformanceMirror` round-trip,
  garbage ⇒ `undefined`.
- render: `engine.test.ts` — with `idleSkip` and a mock backend, N idle ticks call `render()` 0
  times; a `setCameraTransform` makes the next tick render exactly once; an in-flight tween
  renders every tick until it settles. `pixi-backend.test.ts` — `setFrameCap` sets
  `ticker.maxFPS`; `setRenderScale` sets `resolution` and calls `resize`. `token-view.test.ts` —
  `tokenFx:false` drops condition fx and keeps the selection highlight.
  `token-animator.test.ts` — reduced motion snaps to the end pose on tick 1.
  `Stage.test.ts` — toggling antialias re-creates the backend once and destroys the old one.
- settings `PerformanceEditor.test.ts`: preset switch, a key edit shows `custom`, reset; a11y
  (every control labelled, `lint:aria-labels`).
- shell `deviceSignals.test.ts` (node env): every global absent ⇒ `{}`.
- e2e `performance.spec.ts` (written here, dispatcher-run): open settings → choose `mobile` →
  stage `data-fps-cap="30"`; toggle stats → statusbar readout visible; choose `quality` →
  `data-fps-cap="0"`.

## 7. Docs + skills

- `docs/site/modules/settings.md` gains the Performance section; a new guide page
  `docs/site/guides/performance.md` (what each knob costs, the mobile preset); sidebar entries.
- New skill `shadowcat-codebase-performance` (master §6) + `client-shell`/`scene-rendering`
  updates; hook map globs: `src/client/core/src/performance\.ts`,
  `src/client/ui-kit/src/performance`, `src/modules/settings/src/PerformanceEditor`,
  `src/modules/statusbar/src/PerfStats`.
- `docs/HISTORY.md` M22 entry; `docs/PLAN.md` untouched until the last Phase-3 merge (master §3).
