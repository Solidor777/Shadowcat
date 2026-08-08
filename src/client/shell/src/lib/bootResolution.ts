import type { WorldEntry } from "@shadowcat/types";
import type { Route } from "./route.svelte";

/** `boot()`'s single resolution rule (pure, testable without mounting `App`).
 * `enterWorldId` set = the id to enter; `clearLastWorld` = whether the caller
 * must clear the persisted `lastWorld` (a stale/deleted world reference). */
export interface BootResolution {
  /** The world id `boot()` should enter, or `null` to stay on the entry/worlds route. */
  enterWorldId: string | null;
  /** Whether the caller must clear the persisted `lastWorld` (a stale/deleted world
   * reference) before proceeding. */
  clearLastWorld: boolean;
}

/** Resolves which world (if any) boot() should enter. A world ROUTE
 * (`#/world/<id>`) always wins over `lastWorld` — `lastWorld` is not
 * consulted at all while a world route is present, even if it would resolve
 * to a different, still-valid world. `lastWorld` seeds ONLY a bare/non-world
 * load. A route world absent from `worlds` (deleted/revoked) falls back to
 * the worlds list, clearing `lastWorld` only when it is ALSO stale — a dead
 * deep link must never wipe an otherwise-valid `lastWorld` reference.
 * @param route - The current hash route.
 * @param lastWorld - The persisted `ui_state.global.lastWorld`, or `null`.
 * @param worlds - The worlds the caller's account can currently access (from
 *   `listWorlds()`).
 * @returns Which world id to enter, if any, and whether `lastWorld` must be
 *   cleared.
 * @example
 * ```
 * resolveBootWorld({ name: "world", id: "w1" }, null, [{ id: "w1", name: "W", role: "gm" }]);
 * // => { enterWorldId: "w1", clearLastWorld: false }
 * ```
 */
export function resolveBootWorld(
  route: Route,
  lastWorld: string | null,
  worlds: WorldEntry[],
): BootResolution {
  if (route.name === "world") {
    if (worlds.some((w) => w.id === route.id)) {
      return { enterWorldId: route.id, clearLastWorld: false };
    }
    // The route's world is gone, but `lastWorld` may still be a perfectly
    // valid reference to a DIFFERENT world — clear it only when it is
    // ALSO stale, never as a side effect of a dead deep link.
    const lastWorldStale = lastWorld !== null && !worlds.some((w) => w.id === lastWorld);
    return { enterWorldId: null, clearLastWorld: lastWorldStale };
  }
  if (lastWorld && worlds.some((w) => w.id === lastWorld)) {
    return { enterWorldId: lastWorld, clearLastWorld: false };
  }
  return { enterWorldId: null, clearLastWorld: lastWorld !== null };
}
