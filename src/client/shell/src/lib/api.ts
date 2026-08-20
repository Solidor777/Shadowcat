import type { WorldEntry } from "@shadowcat/types";

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

/** Per-user UI session state. The server stores this opaquely (object + size cap);
 * the client owns the structure. */
export interface UiState {
  /** Account-wide settings, independent of any world. */
  global: {
    /** The active i18n locale, applied by `loadSessionState` on load and persisted
     * whenever the `i18n` singleton's locale changes. */
    locale: string;
    /** The most recently entered world id, or `null`. Seeds `App.boot()`'s
     * non-route load only — a world route always wins (see `resolveBootWorld`). */
    lastWorld: string | null;
  };
  /** Per-world settings, keyed by world id. */
  worlds: Record<
    string,
    {
      /** Opaque blob owned by `@shadowcat/module-panels`' persistence codec
       * (encodeLayout/decodeLayout) — the shell never inspects its shape. */
      panelLayout?: unknown;
      /** Opaque blob owned by the chat module's per-channel last-read-marker
       * tracking (unread tab badge) — the shell only stores it. */
      chatRead?: unknown;
    }
  >;
}

/** The `UiState` shape for an account with no persisted blob yet.
 * @returns A fresh `UiState`: locale `"en"`, no `lastWorld`, no per-world
 *   entries.
 * @example
 * ```
 * const empty = defaultUiState();
 * ```
 */
function defaultUiState(): UiState {
  return { global: { locale: "en", lastWorld: null }, worlds: {} };
}

/** Fetches the caller's UI-state blob, filling in defaults for any field the
 * server omits (a brand-new account, or a blob written before a field
 * existed).
 * @returns The caller's `UiState`, with `global` defaults applied field-by-
 *   field and `worlds` defaulted to `{}` if absent.
 * @example
 * ```
 * const ui = await getUiState();
 * ```
 */
export async function getUiState(): Promise<UiState> {
  const raw = await getJson<Partial<UiState>>("/api/me/ui-state");
  const def = defaultUiState();
  return {
    global: { ...def.global, ...(raw.global ?? {}) },
    worlds: raw.worlds ?? {},
  };
}

/** Partial UI-state write, at leaf-key granularity — **the single
 * client-side statement of the merge rule** (mirrors
 * `SqliteRepository::merge_ui_state`'s doc comment server-side). Only the
 * individual fields/keys present are written: each present `global.<field>`
 * (e.g. `locale`, `lastWorld`) replaces just that field server-side; each
 * present `worlds.<id>.<key>` (e.g. `panelLayout`, `chatRead`) replaces just
 * that key within `worlds.<id>` — never the whole slice, and never the whole
 * `worlds.<id>` object unless every key happens to be present. A present
 * value still replaces its leaf wholesale (a `panelLayout`/`chatRead` blob is
 * opaque and is never itself deep-merged). Absent fields/keys are untouched.
 * A `null` value for a whole `worlds.<id>` entry instead REMOVES that entry
 * server-side (`merge_one_level`'s whole-key-removal branch) — distinct from
 * a `null` leaf value nested inside a per-world object, which removes just
 * that one leaf key. Sending only changed leaves is the concurrency control —
 * concurrent sessions of one account (two tabs, or two independent module
 * owners of the same world's slice) contend only on the individual
 * fields/keys both actually write. */
export interface UiStatePatch {
  /** Dirty `global` fields only; absent fields are untouched server-side. */
  global?: Partial<UiState["global"]>;
  /** Dirty per-world keys only, keyed by world id; absent keys/worlds are
   * untouched server-side. A `null` value for a world id removes that
   * world's ENTIRE entry server-side (`merge_one_level`'s whole-key-removal
   * branch) — distinct from a per-leaf-key `null` inside an object value,
   * which removes just that one key. */
  worlds?: Record<string, Partial<UiState["worlds"][string]> | null>;
}

/** Writes a partial UI-state patch (see `UiStatePatch` for the merge rule).
 * Throws on a non-2xx response.
 * @param patch - The leaf fields/keys to write; anything absent is untouched
 *   server-side.
 * @param opts - Write options.
 * @param opts.keepalive - When true, sets `fetch`'s `keepalive` flag so the
 *   request can outlive a page unload — used by `flushOnUnload`.
 * @example
 * ```
 * await putUiState({ global: { locale: "fr" } });
 * ```
 */
export async function putUiState(
  patch: UiStatePatch,
  opts: {
    /** See the `@param opts.keepalive` doc above. */
    keepalive?: boolean;
  } = {},
): Promise<void> {
  // `keepalive` lets the request outlive a page unload (the patch is within
  // the server's 64KB merged cap, under keepalive's body limit).
  const res = await fetch("/api/me/ui-state", {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(patch),
    keepalive: opts.keepalive,
    signal: AbortSignal.timeout(FETCH_TIMEOUT_MS),
  });
  if (!res.ok) throw new Error(`PUT /api/me/ui-state → ${res.status}`);
}
