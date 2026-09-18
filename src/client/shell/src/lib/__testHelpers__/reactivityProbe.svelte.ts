import { flushSync } from "svelte";

/** The value `reactiveRead`'s tracked `$effect` observed before and after `mutate()`. */
export interface ReactiveReadResult<T> {
  /** The value `read()` produced on the effect's first (pre-mutation) run. */
  before: T;
  /** The value `read()` produced after `mutate()` + a synchronous flush. */
  after: T;
}

/** Test-only helper: observes `read()` through a real `$effect`, mutates via `mutate()`, and
 * reports the value BEFORE and AFTER — the only way to catch a `$state` wrapping a plain `Map`
 * (or `Set`), whose in-place mutations Svelte's proxy does not track (`$state` only deep-proxies
 * plain objects/arrays; a `Map`/`Set` needs `svelte/reactivity`'s `SvelteMap`/`SvelteSet`). A
 * direct getter call in an ordinary (non-rune) test always sees the post-mutation value —
 * getters re-evaluate on every call regardless of reactivity — so it cannot distinguish a
 * correctly-reactive source from a silently-stale one; only a real `$effect` re-run can.
 * @param read Reads the value under test; called inside a tracked `$effect`.
 * @param mutate Performs the mutation whose reactivity is under test.
 * @returns The effect's captured value before and after `mutate()` + a synchronous flush.
 * @example
 * ```
 * declare const read: () => string | null;
 * declare const mutate: () => void;
 * // private test helper; not part of the public API
 * reactiveRead(read, mutate);
 * ```
 */
export function reactiveRead<T>(read: () => T, mutate: () => void): ReactiveReadResult<T> {
  let last!: T;
  const dispose = $effect.root(() => {
    $effect(() => {
      last = read();
    });
  });
  flushSync();
  const before = last;
  mutate();
  flushSync();
  const after = last;
  dispose();
  return { before, after };
}
