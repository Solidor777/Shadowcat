import type { TlsStatus } from "@shadowcat/types";

// The decisions behind every "this server also speaks HTTPS" affordance (entry notice, microphone
// message, WebSocket hint). One module so the entry view and the modules cannot disagree on which
// addresses to offer or when to offer them.

/** The parts of `window.location` the notice reads; injectable so the decision is testable
 * without a browser. */
export interface LocationLike {
  /** Scheme with trailing colon (`"http:"`). */
  protocol: string;
  /** Host without port (`"192.168.1.20"`, `"[::1]"` for IPv6 literals). */
  hostname: string;
  /** Host including any port. */
  host: string;
  /** Path component. */
  pathname: string;
  /** Query string including the leading `?`, or `""`. */
  search: string;
  /** Fragment including the leading `#`, or `""`. */
  hash: string;
  /** Scheme, host and port. */
  origin: string;
}

/** Storage key prefix under which a dismissal is remembered, per page origin. */
export const DISMISS_KEY_PREFIX = "shadowcat.httpsNotice.dismissed:";

/**
 * Whether `hostname` is a loopback name. A loopback origin is already a secure context, so the
 * notice has nothing to offer there.
 * @param hostname A `location.hostname` value.
 * @returns True for `localhost`, `127.0.0.0/8` and the IPv6 loopback in either bracketed form.
 * @example
 * ```ts
 * import { isLoopbackHost } from "./https-access";
 *
 * isLoopbackHost("localhost"); // true
 * isLoopbackHost("192.168.1.20"); // false
 * ```
 */
export function isLoopbackHost(hostname: string): boolean {
  const h = hostname.toLowerCase();
  return h === "localhost" || /^127\.\d+\.\d+\.\d+$/.test(h) || h === "[::1]" || h === "::1";
}

/**
 * Whether to offer the HTTPS notice: the page is plain `http:` on a non-loopback host, the
 * server answers TLS, and there is an address to offer ({@link httpsTargets} is non-empty). The
 * notice never blocks login and never redirects; plain HTTP stays a fully supported way to play.
 * @param tls The server's published TLS status.
 * @param loc The page location.
 * @returns True when the notice applies.
 * @example
 * ```ts
 * import type { TlsStatus } from "@shadowcat/types";
 * import { shouldShowHttpsNotice } from "./https-access";
 *
 * declare const tls: TlsStatus;
 * const lan = {
 *   protocol: "http:", hostname: "192.168.1.20", host: "192.168.1.20:30000",
 *   pathname: "/", search: "", hash: "", origin: "http://192.168.1.20:30000",
 * };
 * shouldShowHttpsNotice(tls, lan);
 * ```
 */
export function shouldShowHttpsNotice(tls: TlsStatus, loc: LocationLike): boolean {
  return (
    tls.enabled &&
    loc.protocol === "http:" &&
    !isLoopbackHost(loc.hostname) &&
    httpsTargets(tls, loc).length > 0
  );
}

/**
 * The HTTPS addresses to offer. When a published origin names the host the player already uses,
 * that origin is the answer (it carries the right port, and a trusted certificate covers the
 * name). With published origins for other names only, those are offered. With none published,
 * the address that just worked is offered with the scheme swapped, but only when it names an
 * explicit port: the listener answers TLS on the port it answers HTTP on, so `host:port` is by
 * construction reachable, whereas a portless `http://host/` is served by something in front of the
 * server (a proxy on port 80) and the implied `https://host/` (port 443) may not exist. An
 * unparseable published origin is ignored.
 * @param tls The server's published TLS status.
 * @param loc The page location.
 * @returns Absolute `https://` URLs; empty when there is nothing trustworthy to offer.
 * @example
 * ```ts
 * import type { TlsStatus } from "@shadowcat/types";
 * import { httpsTargets } from "./https-access";
 *
 * declare const tls: TlsStatus;
 * const loc = {
 *   protocol: "http:", hostname: "192.168.1.20", host: "192.168.1.20:30000",
 *   pathname: "/", search: "", hash: "", origin: "http://192.168.1.20:30000",
 * };
 * const targets = httpsTargets({ ...tls, https_origins: [] }, loc);
 * targets[0]; // "https://192.168.1.20:30000/"
 * ```
 */
export function httpsTargets(tls: TlsStatus, loc: LocationLike): string[] {
  const rest = `${loc.pathname}${loc.search}${loc.hash}`;
  // Normalised (`URL.origin`) and deduplicated: two spellings of one origin would render as
  // duplicate keyed entries in the notice.
  const published = [
    ...new Set(
      tls.https_origins.flatMap((o) => {
        try {
          const url = new URL(o);
          return url.protocol === "https:" ? [url.origin] : [];
        } catch {
          return [];
        }
      }),
    ),
  ];
  const sameHost = published.filter((o) => {
    try {
      return new URL(o).hostname === loc.hostname;
    } catch {
      return false;
    }
  });
  if (sameHost.length > 0) return sameHost.map((o) => `${o}${rest}`);
  if (published.length > 0) return published.map((o) => `${o}/`);
  return /:\d+$/.test(loc.host) ? [`https://${loc.host}${rest}`] : [];
}

/**
 * Whether the notice was dismissed for `origin`. Storage can be unavailable (privacy modes,
 * sandboxed frames), and that must never break the login page: any failure reads as "not
 * dismissed".
 * @param storage A storage object, or `null` when none exists.
 * @param origin The page origin the dismissal is scoped to.
 * @returns True when a dismissal is recorded.
 * @example
 * ```ts
 * import { isDismissed } from "./https-access";
 *
 * isDismissed(null, "http://192.168.1.20:30000"); // false
 * ```
 */
export function isDismissed(storage: Pick<Storage, "getItem"> | null, origin: string): boolean {
  try {
    return storage?.getItem(DISMISS_KEY_PREFIX + origin) === "1";
  } catch {
    return false;
  }
}

/**
 * Remember a dismissal for `origin`; a storage failure is swallowed (the notice then simply
 * reappears next visit).
 * @param storage A storage object, or `null` when none exists.
 * @param origin The page origin the dismissal is scoped to.
 * @example
 * ```ts
 * import { dismiss } from "./https-access";
 *
 * dismiss(null, "http://192.168.1.20:30000"); // no storage: nothing recorded, nothing thrown
 * ```
 */
export function dismiss(storage: Pick<Storage, "setItem"> | null, origin: string): void {
  try {
    storage?.setItem(DISMISS_KEY_PREFIX + origin, "1");
  } catch {
    // Unavailable storage: the dismissal is not persisted.
  }
}

/** Consecutive pre-welcome connection failures after which the plain-HTTP hint appears. */
export const WS_HINT_AFTER_FAILURES = 3;

/**
 * Whether a run of first-connect failures should suggest the `http://` address. Applies only on
 * an `https:` page served with the server's own self-signed certificate: a browser can render such
 * a page after a certificate exception yet still refuse the `wss:` handshake, and the same server
 * answers plain HTTP on the same port. The page address must carry an explicit port: the built-in
 * listener always answers on a stated port, whereas a TLS-terminating proxy normally serves the
 * scheme's default port, where the `http://` twin would be a different (or absent) service, so a
 * portless page is never offered the swap. (A proxy on a non-default port that also drops the
 * WebSocket upgrade is not distinguishable from the listener here and gets the hint; the
 * suggested plain address is then simply unreachable.) Fires exactly once, at the threshold, so a
 * reconnect loop does not repeat the message.
 * @param consecutive Failed connection attempts so far (`WsClientHandlers.onConnectFailure`).
 * @param loc The page location: its scheme and host (with port).
 * @param tls The server's published TLS status, or `null` when it could not be read.
 * @returns True when the hint should be shown now.
 * @example
 * ```ts
 * import { shouldHintPlainHttp } from "./https-access";
 *
 * const loc = { protocol: "https:", host: "192.168.1.20:30000" };
 * shouldHintPlainHttp(3, loc, null); // false: no status, no claim about the certificate
 * ```
 */
export function shouldHintPlainHttp(
  consecutive: number,
  loc: Pick<LocationLike, "protocol" | "host">,
  tls: TlsStatus | null,
): boolean {
  return (
    consecutive === WS_HINT_AFTER_FAILURES &&
    loc.protocol === "https:" &&
    /:\d+$/.test(loc.host) &&
    tls?.source === "self-signed"
  );
}

/**
 * Whether a run of WebSocket failures looks like the browser refusing `wss:` rather than the
 * server being down or the session being invalid. A browser exposes no reason for a failed
 * WebSocket, so the discriminator is a plain request to the same origin: if an authenticated
 * `GET /api/me` still succeeds while the socket keeps failing, the page and the certificate
 * exception work and only the `wss:` handshake is refused, the case the plain-HTTP hint helps. A
 * network error means the server is unreachable, and a non-2xx (a 401 for an expired session, a
 * 5xx) means something else is wrong; neither gets the hint.
 * @param fetchMe Issues `GET /api/me` for the current page and resolves its response.
 * @returns True only when the probe answered 2xx.
 * @example
 * ```ts
 * import { httpWorksWhileSocketFails } from "./https-access";
 *
 * const looksLikeWss = await httpWorksWhileSocketFails(() => fetch("/api/me"));
 * ```
 */
export async function httpWorksWhileSocketFails(
  fetchMe: () => Promise<Pick<Response, "ok">>,
): Promise<boolean> {
  try {
    return (await fetchMe()).ok;
  } catch {
    return false;
  }
}

/**
 * The `http://` twin of the current page: the server answers plain HTTP on the port it answers
 * TLS on.
 * @param loc The page location.
 * @returns The same host, port, path, query and fragment under `http:`.
 * @example
 * ```ts
 * import { plainHttpUrl } from "./https-access";
 *
 * const loc = {
 *   protocol: "https:", hostname: "192.168.1.20", host: "192.168.1.20:30000",
 *   pathname: "/", search: "", hash: "", origin: "https://192.168.1.20:30000",
 * };
 * plainHttpUrl(loc); // "http://192.168.1.20:30000/"
 * ```
 */
export function plainHttpUrl(loc: LocationLike): string {
  return `http://${loc.host}${loc.pathname}${loc.search}${loc.hash}`;
}
