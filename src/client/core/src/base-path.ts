/**
 * The global the server's served `index.html` defines before any module script runs.
 */
interface BaseGlobal {
  /** The normalized route prefix; absent when the page was not served by the server. */
  __SHADOWCAT_BASE__?: string;
}

/**
 * The server-configured route prefix: `""` at the default root mount, `"/ttrpg"` when the server
 * runs with `route_prefix = "/ttrpg"` behind a subpath reverse proxy. The value is the
 * normalized form `Config::load` produces (leading slash, no trailing slash), written into the
 * served page as `window.__SHADOWCAT_BASE__` by the server's `index.html` substitution, so the
 * client never has to ask the server for it.
 *
 * Every same-origin request path the client builds (REST, asset, module and WebSocket URLs) must
 * go through `apiPath` or `wsUrl` rather than a bare `/api/...` literal, so the prefix is applied
 * in exactly one place.
 *
 * @returns The prefix, or `""` when the page was served from the origin root.
 *
 * @example
 * ```ts
 * import { basePath } from "./base-path";
 *
 * // No injected global: the root-mount default.
 * basePath(); // ""
 * ```
 */
export function basePath(): string {
  return (globalThis as BaseGlobal).__SHADOWCAT_BASE__ ?? "";
}

/**
 * Prefixes a same-origin absolute path with the configured route prefix.
 *
 * @param path An absolute path beginning with `/`, optionally with a query string.
 * @returns `path` unchanged at the root mount, `<prefix><path>` under a subpath mount.
 *
 * @example
 * ```ts
 * import { apiPath } from "./base-path";
 *
 * apiPath("/api/config"); // "/api/config" at the root mount
 * ```
 */
export function apiPath(path: string): string {
  return `${basePath()}${path}`;
}

/**
 * The origin parts `wsUrl` reads: the page's `location.protocol` and `location.host`.
 */
export interface WsLocation {
  /** `"https:"` selects `wss:`; any other value selects `ws:`. */
  protocol: string;
  /** Host and optional port, as `location.host`. */
  host: string;
}

/**
 * Builds the WebSocket endpoint URL for the page's own origin, under the configured prefix.
 *
 * @param loc The origin to build against; defaults to the page's `location`.
 * @returns `ws://` or `wss://` followed by the host and `apiPath("/ws")`.
 *
 * @example
 * ```ts
 * import { wsUrl } from "./base-path";
 *
 * wsUrl({ protocol: "https:", host: "play.example" }); // "wss://play.example/ws" at the root mount
 * ```
 */
export function wsUrl(loc: WsLocation = globalThis.location): string {
  const scheme = loc.protocol === "https:" ? "wss:" : "ws:";
  return `${scheme}//${loc.host}${apiPath("/ws")}`;
}
