export type Route =
  | { name: "setup" }
  | { name: "login" }
  | { name: "worlds" }
  | { name: "world"; id: string }
  | { name: "unknown" };

export function parseHash(hash: string): Route {
  const path = hash.replace(/^#/, "");
  if (path === "/setup") return { name: "setup" };
  if (path === "/login") return { name: "login" };
  if (path === "/worlds") return { name: "worlds" };
  const m = /^\/world\/(.+)$/.exec(path);
  if (m) return { name: "world", id: m[1] };
  return { name: "unknown" };
}

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

export function navigate(route: Route): void {
  location.hash = routeToHash(route);
}

let route = $state<Route>(parseHash(location.hash));
if (typeof window !== "undefined") {
  window.addEventListener("hashchange", () => {
    route = parseHash(location.hash);
  });
}

export function currentRoute(): Route {
  return route;
}
