// The PerformanceController singleton and its Svelte adapter, mirroring `theme.svelte.ts`'s
// shape exactly: a `$state`-backed controller with subscribe/snapshot reactivity, plus a
// `createSubscriber`-backed reactive read for components. The controller never touches
// `Storage` — the shell persists through `onChange` (`writePerformanceMirror`), per-device only:
// never the server `ui_state`. The singleton is named `performanceController`, never
// `performance`, so no importer shadows the ambient `Performance` global (`performance.now()`).
import { createSubscriber } from "svelte/reactivity";
import {
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
   * performanceController.setShowStats(true);
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
   * performanceController.recordStats({ fps: 60, frameMs: 4 });
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
   * performanceController.set({ fpsCap: 30 });
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
   * performanceController.setPreset("mobile");
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
   * const state = performanceController.serialize();
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
   * const unsubscribe = performanceController.subscribe(() => {});
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

/** The app's single performance controller instance (mirrors the `theme` singleton; named
 * `performanceController` so no importer shadows the ambient `Performance` global). */
export const performanceController = new PerformanceController();

const subscribe = createSubscriber((update) => performanceController.subscribe(update));

/** The resolved effective performance settings, read reactively: reading it in a rune context
 * (`$derived`, `$effect`, a component's template) re-runs on any `set`/`setPreset`/`load` change.
 * @returns The current effective `PerformanceSettings`.
 * @example activePerformance().fpsCap; // 60
 */
export function activePerformance(): PerformanceSettings {
  subscribe();
  return performanceController.current;
}
