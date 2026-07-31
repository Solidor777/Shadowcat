import type { WorldEntry } from "@shadowcat/types";
import type { Route } from "./route.svelte";

/** boot()'s single resolution rule (pure, testable without mounting App.svelte).
 * `enterWorldId` set = the id to enter; `clearLastWorld` = whether the caller
 * must clear the persisted `lastWorld` (a stale/deleted world reference). */
export interface BootResolution {
  enterWorldId: string | null;
  clearLastWorld: boolean;
}

/** Resolves which world (if any) boot() should enter. A world ROUTE
 * (`#/world/<id>`) always wins over `lastWorld` — `lastWorld` is not
 * consulted at all while a world route is present, even if it would resolve
 * to a different, still-valid world. `lastWorld` seeds ONLY a bare/non-world
 * load. A route world absent from `worlds` (deleted/revoked) falls through to
 * the same stale-reference handling as a stale `lastWorld` — clear + let the
 * caller fall back to the worlds list — rather than silently substituting
 * `lastWorld`. */
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
