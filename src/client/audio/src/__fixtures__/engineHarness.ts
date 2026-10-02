import type { AudioTimer, VisibilitySource } from "../engine";

/** `timerHarness`'s return shape: the injectable timer plus the controls a test drives it by. */
export interface TimerHarness extends AudioTimer {
  /** Run the scheduled callback once, as one elapsed period; a no-op while none is scheduled.
   * @returns Nothing. */
  fire(): void;
  /** The period the engine scheduled, ms, or `null` while nothing is scheduled.
   * @returns The period, or `null`. */
  periodMs(): number | null;
  /** How many `clearInterval` calls the engine made.
   * @returns The count. */
  cleared(): number;
}

/** A manually fired `AudioTimer`: nothing runs until `fire()`.
 * @returns The harness.
 * @example
 * ```ts
 * import { timerHarness } from "../__fixtures__/engineHarness";
 * timerHarness().periodMs(); // null
 * ```
 */
export function timerHarness(): TimerHarness {
  let callback: (() => void) | null = null;
  let period: number | null = null;
  let clears = 0;
  return {
    setInterval(cb: () => void, ms: number): number {
      callback = cb;
      period = ms;
      return 7;
    },
    clearInterval(): void {
      callback = null;
      period = null;
      clears++;
    },
    fire(): void {
      callback?.();
    },
    periodMs: () => period,
    cleared: () => clears,
  };
}

/** `visibilityHarness`'s return shape: the injectable source plus the controls a test drives it by. */
export interface VisibilityHarness extends VisibilitySource {
  /** Set the page's visibility and notify every observer.
   * @param visible The new visibility. */
  set(visible: boolean): void;
  /** How many observers are currently subscribed.
   * @returns The count. */
  observers(): number;
}

/** A scripted `VisibilitySource`, visible until `set(false)`.
 * @returns The harness.
 * @example
 * ```ts
 * import { visibilityHarness } from "../__fixtures__/engineHarness";
 * visibilityHarness().isVisible(); // true
 * ```
 */
export function visibilityHarness(): VisibilityHarness {
  let visible = true;
  const observers = new Set<() => void>();
  return {
    isVisible: () => visible,
    subscribe(cb: () => void): () => void {
      observers.add(cb);
      return () => observers.delete(cb);
    },
    set(next: boolean): void {
      visible = next;
      for (const cb of [...observers]) cb();
    },
    observers: () => observers.size,
  };
}
