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
