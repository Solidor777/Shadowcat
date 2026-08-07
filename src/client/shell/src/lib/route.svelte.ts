/** The parsed shape of `location.hash`, as produced by `parseHash` and consumed by
 * `routeToHash`/`currentRoute`/`resolveBootWorld`. */
export type Route =
  | {
      /** Discriminant for the first-run/initial setup route. */
      name: "setup";
    }
  | {
      /** Discriminant for the login route. */
      name: "login";
    }
  | {
      /** Discriminant for the worlds-list route. */
      name: "worlds";
    }
  | {
      /** Discriminant for a specific-world route; carries `id`. */
      name: "world";
      /** The world id from the route's path segment. */
      id: string;
    }
  | {
      /** Discriminant for an unrecognized hash path (including an empty hash);
       * see `parseHash`. */
      name: "unknown";
    };

/** Parses a `location.hash` string into a `Route`. An unrecognized path
 * (including an empty hash) resolves to `{ name: "unknown" }`.
 * @param hash - The raw hash, including the leading `#` if present.
 * @returns The parsed route.
 * @example
 * ```
 * parseHash("#/world/w1"); // => { name: "world", id: "w1" }
 * ```
 */
export function parseHash(hash: string): Route {
  const path = hash.replace(/^#/, "");
  if (path === "/setup") return { name: "setup" };
  if (path === "/login") return { name: "login" };
  if (path === "/worlds") return { name: "worlds" };
  const m = /^\/world\/(.+)$/.exec(path);
  if (m) return { name: "world", id: m[1] };
  return { name: "unknown" };
}

/** Serializes a `Route` back into a `location.hash` string. Not a strict
 * inverse of `parseHash`: an `"unknown"` route serializes to `#/login`, not
 * the original invalid hash — `Route` carries no memory of the string that
 * produced it.
 * @param route - The route to serialize.
 * @returns The hash string, including the leading `#`.
 * @example
 * ```
 * routeToHash({ name: "world", id: "w1" }); // => "#/world/w1"
 * ```
 */
export function routeToHash(route: Route): string {
  switch (route.name) {
    case "world":
      return `#/world/${route.id}`;
    case "unknown":
      return "#/login";
    default:
      return `#/${route.name}`;
  }
}

/** Navigates by setting `location.hash`, which fires the `hashchange`
 * listener that updates `currentRoute()`'s reactive state.
 * @param route - The route to navigate to.
 * @example
 * ```
 * navigate({ name: "worlds" });
 * ```
 */
export function navigate(route: Route): void {
  location.hash = routeToHash(route);
}

let route = $state<Route>(parseHash(location.hash));
if (typeof window !== "undefined") {
  window.addEventListener("hashchange", () => {
    route = parseHash(location.hash);
  });
}

/** The current route, reactive: reading it in a rune context (`$derived`,
 * `$effect`) re-runs when `location.hash` changes, via the module-level
 * `$state` this function reads.
 * @returns The current route.
 * @example
 * ```
 * const route = $derived(currentRoute());
 * ```
 */
export function currentRoute(): Route {
  return route;
}
