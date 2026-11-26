import type { TlsStatus } from "@shadowcat/types";
import {
  httpWorksWhileSocketFails,
  plainHttpUrl,
  shouldHintPlainHttp,
  WS_HINT_AFTER_FAILURES,
  type LocationLike,
} from "@shadowcat/core";

/** What {@link plainHttpHint} needs from its surroundings; injectable so the wiring is testable. */
export interface PlainHttpHintDeps {
  /** The page location. */
  location: LocationLike;
  /** The TLS status already read, or `null` when none is cached. */
  currentTls: () => TlsStatus | null;
  /** Reads the TLS status (with retry); resolves `null` when every attempt failed. */
  loadTls: () => Promise<TlsStatus | null>;
  /** Issues `GET /api/me` for the current page. */
  fetchMe: () => Promise<Pick<Response, "ok">>;
}

/**
 * The plain-HTTP address to suggest after `failures` consecutive first-connect WebSocket failures,
 * or `null` when no hint applies.
 *
 * Discriminator: the hint needs ALL of (1) `failures` is exactly the threshold, (2) the page is
 * `https:` on an explicit port with the server's own self-signed certificate
 * ({@link shouldHintPlainHttp}), and (3) one authenticated `GET /api/me` from the page succeeds
 * ({@link httpWorksWhileSocketFails}): the page and its certificate exception work while the
 * socket does not, which is the `wss:`-refused case. The probe is a single sample by design: a
 * failed probe (server down, expired session) suppresses the hint for that session rather than
 * risk a wrong claim, and a wrongly shown hint only points at the plain address, which the same
 * server always answers.
 * @param failures The consecutive failure count.
 * @param deps The page location, TLS status access and probe.
 * @returns The `http://` URL to offer, or `null`.
 * @example
 * ```ts
 * import { plainHttpHint } from "./plainHttpHint";
 *
 * const url = await plainHttpHint(3, {
 *   location: { protocol: "http:", hostname: "h", host: "h:1", pathname: "/", search: "", hash: "", origin: "http://h:1" },
 *   currentTls: () => null,
 *   loadTls: async () => null,
 *   fetchMe: async () => ({ ok: true }),
 * });
 * // null: not an https page
 * ```
 */
export async function plainHttpHint(failures: number, deps: PlainHttpHintDeps): Promise<string | null> {
  if (failures !== WS_HINT_AFTER_FAILURES || deps.location.protocol !== "https:") return null;
  const status = deps.currentTls() ?? (await deps.loadTls());
  if (!shouldHintPlainHttp(failures, deps.location, status)) return null;
  if (!(await httpWorksWhileSocketFails(deps.fetchMe))) return null;
  return plainHttpUrl(deps.location);
}
