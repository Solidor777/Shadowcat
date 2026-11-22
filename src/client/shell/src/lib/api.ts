import type { TlsStatus, WorldEntry } from "@shadowcat/types";
import { ServerConfigSchema, type WireDocument } from "@shadowcat/core";

/** Local mirror of the server's MeResponse (not ts-rs-exported). */
export interface Me {
  /** The caller's user id. */
  id: string;
  /** The caller's display username. */
  username: string;
  /** Raw server-tier role string. `App.boot()` compares it against `"admin"` when
   * deriving `AppContext.serverRole`; any other value, including absent, falls back
   * to `"user"` (fail-closed). */
  server_role: string;
}

/** Bound on every session/boot fetch. A hung backend request otherwise pins the
 * SPA on its current route forever (the boot chain has no other timeout). */
const FETCH_TIMEOUT_MS = 15_000;

/** Bounded retry for the boot chain (`withRetry`'s only caller is
 * `App`'s `boot()`): a transient backend blip (restart, single 5xx)
 * must not permanently strand the SPA on the login/worlds route with no
 * retry and no error surface. Delays are base values for full jitter, not
 * a policy knob (YAGNI). Rethrows the last error if every attempt fails.
 * @param fn - The operation to retry; invoked again from scratch each
 *   attempt.
 * @param attempts - Maximum number of calls to `fn`.
 * @param delays - Base delay in ms before each retry (full jitter applied,
 *   not the literal wait), indexed by attempt number and clamped to the
 *   last entry once attempts exceed the array length (default: 500ms after
 *   the first failure, 1500ms after the second).
 * @returns The resolved value of the first successful call to `fn`.
 * @example
 * ```
 * await withRetry(() => fetch("/api/me"));
 * ```
 */
export async function withRetry<T>(
  fn: () => Promise<T>,
  attempts = 3,
  delays: number[] = [500, 1500],
): Promise<T> {
  let lastErr: unknown;
  for (let i = 0; i < attempts; i++) {
    try {
      return await fn();
    } catch (e) {
      lastErr = e;
      if (i < attempts - 1) {
        const base = delays[Math.min(i, delays.length - 1)];
        // Full jitter (half..full of `base`), matching WsClient.scheduleReconnect's convention —
        // avoids many concurrently-retrying clients converging on the same lockstep cadence.
        const delay = base * (0.5 + Math.random() * 0.5);
        await new Promise((r) => setTimeout(r, delay));
      }
    }
  }
  throw lastErr;
}

/** Fetches `url` as JSON, aborting after `FETCH_TIMEOUT_MS`. Throws on a
 * non-2xx response.
 * @param url - Request URL.
 * @returns The parsed JSON response body, typed as `T`.
 * @example
 * ```
 * await getJson<Me>("/api/me");
 * ```
 */
async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url, {
    headers: { accept: "application/json" },
    signal: AbortSignal.timeout(FETCH_TIMEOUT_MS),
  });
  if (!res.ok) throw new Error(`${url} → ${res.status}`);
  return (await res.json()) as T;
}

/** POSTs `body` as JSON to `url`, aborting after `FETCH_TIMEOUT_MS`. Returns
 * the raw `Response` — unlike `getJson`, it does not check `res.ok` or parse
 * the body; callers that care about the outcome do so themselves.
 * @param url - Request URL.
 * @param body - Request body, JSON-serialized.
 * @returns The raw fetch `Response`.
 * @example
 * ```
 * await postJson("/api/logout", {});
 * ```
 */
async function postJson(url: string, body: unknown): Promise<Response> {
  return fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(FETCH_TIMEOUT_MS),
  });
}

/** Fetches the caller's identity. Distinguishes "not logged in" from a
 * request failure: a 401 resolves to `null`, any other non-2xx status
 * throws.
 * @returns The caller's `Me` record, or `null` if unauthenticated.
 * @example
 * ```
 * const me = await getMe();
 * ```
 */
export async function getMe(): Promise<Me | null> {
  const res = await fetch("/api/me", {
    headers: { accept: "application/json" },
    signal: AbortSignal.timeout(FETCH_TIMEOUT_MS),
  });
  if (res.status === 401) return null;
  if (!res.ok) throw new Error(`/api/me → ${res.status}`);
  return (await res.json()) as Me;
}

/** Fetches the server's published TLS status (the `tls` block of `GET /api/config`).
 *
 * The entry package reads `GET /api/config` through its own `getConfig` instead of sharing this
 * function, by design: `@shadowcat/module-entry` is a swappable module package and must not
 * depend on shell internals, and the two readers have different lifetimes (the entry view needs a
 * fresh `initialized` flag before login; the shell keeps one status for the session and retries
 * it with backoff, see `TlsStatusLoader`). Both validate against the Zod mirror of the generated
 * `TlsStatus` type (`ServerConfigSchema` here, `TlsStatusSchema` in the entry reader), so the
 * wire shape has one definition.
 * @returns The validated status; throws on a non-2xx response or a body that fails
 * `ServerConfigSchema`.
 * @example
 * ```
 * const tls = await getTlsStatus();
 * ```
 */
export async function getTlsStatus(): Promise<TlsStatus> {
  return ServerConfigSchema.parse(await getJson<unknown>("/api/config")).tls;
}

/** Logs the caller out server-side. Awaits the request but does not inspect
 * its response status (see `postJson`) — a failed logout is not surfaced to
 * the caller.
 * @example
 * ```
 * await logout();
 * ```
 */
export async function logout(): Promise<void> {
  await postJson("/api/logout", {});
}

/** Lists the worlds the caller's account can currently access.
 * @returns Each accessible world, with the caller's effective role in it.
 * @example
 * ```
 * const worlds = await listWorlds();
 * ```
 */
export function listWorlds(): Promise<WorldEntry[]> {
  return getJson<WorldEntry[]>("/api/worlds");
}

/** `GET /api/worlds/{id}/snapshot` response: every document the caller can currently see,
 * per-recipient filtered, plus the room's `current_seq` at read time. Hand-typed (rather than
 * the ts-rs-generated `WorldSnapshot`, whose `seq: bigint` reflects the Rust `i64` field, not
 * the JSON number `fetch` actually parses it into) to match this module's other hand-written
 * response shapes (`Me`). */
export interface WorldSnapshotResponse {
  /** Every visible document, per-recipient filtered. */
  documents: WireDocument[];
  /** The room's `current_seq` this snapshot was read as-of. */
  seq: number;
}

/** Fetches a current-state snapshot of every document the caller can see in `worldId`, plus the
 * seq it was read as-of. `WorldSession.enter` calls this before opening the WS connection so a
 * genuine cold start bootstraps from current state instead of replaying the world's full history.
 * @param worldId The world to snapshot.
 * @returns The visible documents and the snapshot's `seq`.
 * @example
 * ```
 * const snapshot = await getWorldSnapshot("00000000-0000-0000-0000-000000000001");
 * ```
 */
export function getWorldSnapshot(worldId: string): Promise<WorldSnapshotResponse> {
  return getJson<WorldSnapshotResponse>(`/api/worlds/${encodeURIComponent(worldId)}/snapshot`);
}

/** Per-user UI session state and its REST wrappers live in `@shadowcat/core`; re-exported so the
 * shell's own import sites keep one path. */
export { getUiState, putUiState, defaultUiState } from "@shadowcat/core";
export type { UiState, UiStatePatch } from "@shadowcat/core";