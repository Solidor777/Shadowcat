import type { ServerConfig, WorldEntry } from "@shadowcat/types";

/**
 * Fetch JSON from `url`, throwing on any non-2xx response.
 * @param url The request URL.
 * @returns The parsed JSON body, typed as `T`.
 * @example
 * ```
 * // module-private; not part of the public API
 * const cfg = await getJson<ServerConfig>("/api/config");
 * ```
 */
async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url, { headers: { accept: "application/json" } });
  if (!res.ok) throw new Error(`${url} → ${res.status}`);
  return (await res.json()) as T;
}

/**
 * POST `body` as JSON to `url`, returning the raw response without checking
 * status. Each caller decides its own error shape: `login` returns a bare
 * boolean, `setup` returns `{ ok, status }`, `createWorld` throws on any
 * non-2xx, and `acceptInvite` collapses every failure to `null`.
 * @param url The request URL.
 * @param body The request body, serialized with `JSON.stringify`.
 * @returns The unchecked `Response`.
 * @example
 * ```
 * // module-private; not part of the public API
 * const res = await postJson("/api/login", { username, password });
 * ```
 */
async function postJson(url: string, body: unknown): Promise<Response> {
  return fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

/**
 * Fetch the server's public configuration.
 * @returns The server config. Field semantics are defined by the generated
 * `ServerConfig` type (from the Rust
 * source) — not restated here.
 * @example
 * ```
 * // module-private; not part of the public API
 * const cfg = await getConfig();
 * ```
 */
export function getConfig(): Promise<ServerConfig> {
  return getJson<ServerConfig>("/api/config");
}

/**
 * Auth probe: the authenticated user's id, or null when unauthenticated (401).
 * @returns The authenticated user's id, or `null` for a 401. Throws on any
 * other non-2xx status.
 * @example
 * ```
 * // module-private; not part of the public API
 * const me = await getMe();
 * ```
 */
export async function getMe(): Promise<{
  /** The authenticated caller's user id. */
  id: string;
} | null> {
  const res = await fetch("/api/me", { headers: { accept: "application/json" } });
  if (res.status === 401) return null;
  if (!res.ok) throw new Error(`/api/me → ${res.status}`);
  return (await res.json()) as {
    /** The authenticated caller's user id. */
    id: string;
  };
}

/**
 * Log in with a username and password.
 * @param username The account username.
 * @param password The account password.
 * @returns `true` on a 2xx response, `false` otherwise — the status code
 * itself is discarded.
 * @example
 * ```
 * // module-private; not part of the public API
 * const ok = await login("alice", "hunter2");
 * ```
 */
export async function login(username: string, password: string): Promise<boolean> {
  const res = await postJson("/api/login", { username, password });
  return res.ok;
}

/**
 * Bootstrap the very first admin account. Only reachable while the server is
 * uninitialized; `token` is required only when the server's setup-token
 * policy demands one, and is omitted from the request body entirely when not
 * supplied. Under the DEFAULT `auto` policy a loopback-only bind needs no
 * token, but `setup_token: "required"` demands one on every bind including
 * loopback (`Config::setup_token_policy`).
 * @param username The new admin account's username.
 * @param password The new admin account's password.
 * @param token The setup token, when the server requires one.
 * @returns `{ ok, status }` — `status` is returned specifically so the caller
 * can distinguish a token mismatch (403, see `Setup`'s
 * `errorToken` branch) from any other failure, which is surfaced generically
 * using the status code.
 * @example
 * ```
 * // module-private; not part of the public API
 * const { ok, status } = await setup("MOCK_ADMIN", "MOCK_PASSWORD");
 * ```
 */
export async function setup(
  username: string,
  password: string,
  token?: string,
): Promise<{
  /** `true` on a 2xx response. */
  ok: boolean;
  /** The raw HTTP status code, kept so a 403 (bad/missing setup token) is distinguishable
   * from any other failure — see the `@returns` note above. */
  status: number;
}> {
  const body: Record<string, string> = { username, password };
  if (token) body.token = token;
  const res = await postJson("/api/setup", body);
  return { ok: res.ok, status: res.status };
}

/**
 * List worlds the caller is a member of.
 * @returns The caller's worlds. Field semantics are defined by the generated
 * `WorldEntry` type (from the Rust
 * source) — not restated here.
 * @example
 * ```
 * // module-private; not part of the public API
 * const worlds = await listWorlds();
 * ```
 */
export function listWorlds(): Promise<WorldEntry[]> {
  return getJson<WorldEntry[]>("/api/worlds");
}

/**
 * Permanently delete a world. This function performs NO confirmation of its
 * own — the type-the-exact-name gate that guards destructive calls lives one
 * layer up, in `WorldSelect` (`armDelete`/`confirmDelete`). Do
 * not call this from a control that skips that gate.
 * @param id The world id to delete.
 * @returns Resolves on success. Throws on any non-2xx.
 * @example
 * ```
 * // module-private; not part of the public API
 * await deleteWorld(worldId);
 * ```
 */
export async function deleteWorld(id: string): Promise<void> {
  const res = await fetch(`/api/worlds/${encodeURIComponent(id)}`, { method: "DELETE" });
  if (!res.ok) throw new Error(`/api/worlds/${id} → ${res.status}`);
}

/**
 * Create a new world; the caller is seated as its GM.
 * @param name The new world's display name.
 * @returns The created world. Field semantics are defined by the generated
 * `WorldEntry` type. Throws on any
 * non-2xx.
 * @example
 * ```
 * // module-private; not part of the public API
 * const world = await createWorld("My Campaign");
 * ```
 */
export async function createWorld(name: string): Promise<WorldEntry> {
  const res = await postJson("/api/worlds", { name });
  if (!res.ok) throw new Error(`/api/worlds → ${res.status}`);
  return (await res.json()) as WorldEntry;
}

/** Redeem a world invite, seating the caller in the invite's world. Any
 * authenticated user; the code is the only authorization.
 *
 * Every rejection — unknown, malformed, expired, revoked, already used — is
 * one indistinguishable 404 by design, so the caller learns nothing about a
 * world they hold no valid code for. Callers must surface a single generic
 * failure rather than trying to explain which case it was.
 *
 * Server proof: both rejection paths in `accept_invite` return `AppError::NotFound`,
 * which maps to 404
 * in `AppError`'s `IntoResponse` impl. The first of those collapses "no such
 * invite" and "wrong secret" into ONE branch (`record.filter(|_| verified)`),
 * so the two are indistinguishable by construction, not by discipline.
 * @param code The invite's bearer code.
 * @returns The redeemed world on success, or `null` on any rejection
 * (collapsed at this layer — see the description above).
 * @example
 * ```
 * // module-private; not part of the public API
 * const world = await acceptInvite(code);
 * ```
 */
export async function acceptInvite(code: string): Promise<WorldEntry | null> {
  // The code goes in the BODY: a URL is recorded by browser history, `Referer`,
  // proxy access logs, and the server's request-trace span, none of which a
  // live bearer credential may reach.
  const res = await postJson("/api/invites/accept", { code });
  if (!res.ok) return null;
  return (await res.json()) as WorldEntry;
}
