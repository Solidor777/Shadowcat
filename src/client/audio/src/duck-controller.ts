import type { DuckController, DuckSource } from "@shadowcat/core";

/** Attack time constant, ms — how fast the duck gain drops when demand rises. */
export const DUCK_ATTACK_MS = 50;
/** Release time constant, ms — how fast the duck gain recovers when demand falls. */
export const DUCK_RELEASE_MS = 600;
/** Default duck depth: `gain = 1 - demand * depth` at full demand. */
export const DEFAULT_DUCK_DEPTH = 0.7;

/** Exponential one-pole smoothing toward `target` over `elapsedMs`, with time constant `tauMs`
 * (the SAME `1 - exp(-dt/tau)` shape `AudioParam.setTargetAtTime` uses, so the JS-level demand
 * smoothing and the eventual `AudioParam` write agree in character).
 * @param current The current smoothed value.
 * @param target The value to approach.
 * @param elapsedMs Milliseconds since the previous step.
 * @param tauMs The time constant (attack while rising, release while falling).
 * @returns The advanced smoothed value.
 * @example
 * ```ts
 * import { approach } from "@shadowcat/audio";
 * approach(0, 1, 50, 50); // ~0.632 after one time constant
 * ```
 */
export function approach(current: number, target: number, elapsedMs: number, tauMs: number): number {
  if (tauMs <= 0) return target;
  const alpha = 1 - Math.exp(-elapsedMs / tauMs);
  return current + (target - current) * alpha;
}

/** The mixer's master ducking bus (`AudioEngine.duck`). Demand smoothing is driven by explicit
 * `tick(nowMs)` calls (from `AudioEngine`'s own animation-frame loop in production, or directly
 * from a test) rather than a real-time audio-thread process — this is JS-level gain automation
 * feeding an `AudioParam.setTargetAtTime` write, not sample-accurate DSP. */
export class DuckControllerImpl implements DuckController {
  /** How hard a saturated demand ducks, `0..=1` (the gain floor is `1 - depth`). */
  #depth: number;
  /** Every registered source's current demand, by id. */
  #demands = new Map<string, number>();
  /** The one-pole-smoothed max demand (see `tick`). */
  #smoothed = 0;
  /** The previous `tick`'s timestamp; `null` until the first tick. */
  #lastTickMs: number | null = null;

  /** Construct the controller at `depth`.
   * @param depth Starting duck depth, `0..=1` (clamped); defaults to `DEFAULT_DUCK_DEPTH`.
   * @example
   * ```ts
   * import { DuckControllerImpl } from "@shadowcat/audio";
   * new DuckControllerImpl(0.2).depth; // 0.2
   * ```
   */
  constructor(depth: number = DEFAULT_DUCK_DEPTH) {
    this.#depth = Math.max(0, Math.min(1, depth));
  }

  /** The current effective duck gain, `0..=1` (1 = no ducking).
   * @returns The gain to apply to duckable buses. */
  get gain(): number {
    return 1 - this.#smoothed * this.#depth;
  }

  /** The configured duck depth, `0..=1`.
   * @returns The depth. */
  get depth(): number {
    return this.#depth;
  }

  /** Clamps to `0..=1`; re-targets the smoothed gain on the next `tick` rather than jumping.
   * @param depth The new depth, `0..=1`.
   * @example
   * ```ts
   * import { DuckControllerImpl } from "@shadowcat/audio";
   * const duck = new DuckControllerImpl();
   * duck.setDepth(0.4);
   * duck.depth; // 0.4
   * ```
   */
  setDepth(depth: number): void {
    this.#depth = Math.max(0, Math.min(1, depth));
  }

  /** Register a ducking source; returns its handle. The id IS the dedup key: a second
   * `addSource` with an id already registered replaces that source's demand slot rather than
   * adding a second one.
   * @param id A stable identifier for this source.
   * @returns The handle this source uses to report its demand.
   * @example
   * ```ts
   * import { DuckControllerImpl } from "@shadowcat/audio";
   * const duck = new DuckControllerImpl();
   * duck.addSource("mic").set(1);
   * ```
   */
  addSource(id: string): DuckSource {
    this.#demands.set(id, 0);
    return {
      set: (level: number) => {
        this.#demands.set(id, Math.max(0, Math.min(1, level)));
      },
    };
  }

  /** Unregister a source by the id it was added with.
   * @param id The source's id, as passed to `addSource`.
   * @example
   * ```ts
   * import { DuckControllerImpl } from "@shadowcat/audio";
   * const duck = new DuckControllerImpl();
   * duck.removeSource("mic"); // no-op for a never-registered id
   * ```
   */
  removeSource(id: string): void {
    this.#demands.delete(id);
  }

  /** Advance the smoothed demand toward the current max-of-sources demand by `nowMs -
   * (last tick)`, using the attack time constant while rising and the release constant while
   * falling. The first call after construction (or after a gap) seeds `lastTickMs` and applies
   * no smoothing (elapsed = 0), matching how a fresh mixer starts at rest.
   * @param nowMs Monotonic wall-clock ms (e.g. `performance.now()`).
   * @example
   * ```ts
   * import { DuckControllerImpl } from "@shadowcat/audio";
   * const duck = new DuckControllerImpl();
   * duck.addSource("mic").set(1);
   * duck.tick(1_000);
   * duck.gain < 1; // demand pulled the gain down
   * ```
   */
  tick(nowMs: number): void {
    const target = Math.max(0, ...this.#demands.values());
    const elapsed = this.#lastTickMs === null ? 0 : nowMs - this.#lastTickMs;
    this.#lastTickMs = nowMs;
    const tau = target > this.#smoothed ? DUCK_ATTACK_MS : DUCK_RELEASE_MS;
    this.#smoothed = approach(this.#smoothed, target, elapsed, tau);
  }
}
