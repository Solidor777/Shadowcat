// Test support, imported only by test files: a deterministic work counter over `Math.hypot`,
// shared by the polyline-thinning and shared-ruler sender suites so both measure work by one
// definition. It patches the global `Math.hypot` for the duration of one call.

/** Failure text of the positive control on the `Math.hypot` work counter. */
export const WORK_COUNTER_INERT =
  "the Math.hypot work counter reads too low: the complexity pin must be re-pointed at the primitive the algorithm now evaluates";

/** The outcome of a measured run. */
export interface WorkCount<T> {
  /** What the measured function returned. */
  result: T;
  /** The number of `Math.hypot` calls the run made. */
  calls: number;
}

/** Runs `fn` with `Math.hypot` wrapped by a call counter and returns the result with the number
 * of calls. The count is a deterministic measure of the distance evaluations made through
 * `Math.hypot`: in the thinning code `isCollinear` calls it twice per vertex it checks and
 * `chordDeviation` once per deviation it computes, and code that computes distances without
 * `Math.hypot` reads as zero work. The restore is in a
 * `finally`; callers assert outside the patched window.
 * @param fn The work to measure.
 * @returns The result of `fn` and the number of `Math.hypot` calls it made.
 * @example
 * ```ts
 * const { calls } = countHypot(() => Math.hypot(3, 4));
 * calls; // 1
 * ```
 */
export function countHypot<T>(fn: () => T): WorkCount<T> {
  const realHypot = Math.hypot;
  let calls = 0;
  Math.hypot = (...values: number[]): number => {
    calls++;
    return realHypot(...values);
  };
  try {
    return { result: fn(), calls };
  } finally {
    Math.hypot = realHypot;
  }
}
