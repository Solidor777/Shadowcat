import { createSubscriber } from "svelte/reactivity";

/** Single source of truth for the compact/expanded breakpoint; replaces ad-hoc
 * `40rem`-style media queries as call sites are touched. "compact" = narrow
 * viewport (query does not match); "expanded" = matches. */
export type SizeClass = "compact" | "expanded";

const QUERY = "(min-width: 48rem)";

// jsdom (the default test environment) does not implement `matchMedia`; treat
// its absence as always-expanded rather than throwing during component init.
const mql: MediaQueryList | null =
  typeof matchMedia === "function" ? matchMedia(QUERY) : null;

const subscribe = mql
  ? createSubscriber((update) => {
      const listener = () => update();
      mql.addEventListener("change", listener);
      return () => mql.removeEventListener("change", listener);
    })
  : null;

/** Reactive current size class: reading it in a rune context re-runs when the
 * `(min-width: 48rem)` media query flips. Under jsdom (no `matchMedia`), always
 * returns `"expanded"` — treated as an always-matching query, never throwing.
 * @returns `"expanded"` when the query matches (or `matchMedia` is unavailable),
 * else `"compact"`.
 * @example sizeClass(); // "expanded"
 */
export function sizeClass(): SizeClass {
  subscribe?.();
  return (mql?.matches ?? true) ? "expanded" : "compact";
}
