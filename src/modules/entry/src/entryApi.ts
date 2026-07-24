import type { ServerConfig, WorldEntry } from "@shadowcat/types";

async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url, { headers: { accept: "application/json" } });
  if (!res.ok) throw new Error(`${url} → ${res.status}`);
  return (await res.json()) as T;
}

async function postJson(url: string, body: unknown): Promise<Response> {
  return fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

export function getConfig(): Promise<ServerConfig> {
  return getJson<ServerConfig>("/api/config");
}

/** Auth probe: the authenticated user's id, or null when unauthenticated (401). */
export async function getMe(): Promise<{ id: string } | null> {
  const res = await fetch("/api/me", { headers: { accept: "application/json" } });
  if (res.status === 401) return null;
  if (!res.ok) throw new Error(`/api/me → ${res.status}`);
  return (await res.json()) as { id: string };
}

export async function login(username: string, password: string): Promise<boolean> {
  const res = await postJson("/api/login", { username, password });
  return res.ok;
}

export async function setup(
  username: string,
  password: string,
  token?: string,
): Promise<{ ok: boolean; status: number }> {
  const body: Record<string, string> = { username, password };
  if (token) body.token = token;
  const res = await postJson("/api/setup", body);
  return { ok: res.ok, status: res.status };
}

export function listWorlds(): Promise<WorldEntry[]> {
  return getJson<WorldEntry[]>("/api/worlds");
}

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
 * failure rather than trying to explain which case it was. */
export async function acceptInvite(code: string): Promise<WorldEntry | null> {
  // The code goes in the BODY: a URL is recorded by browser history, `Referer`,
  // proxy access logs, and the server's request-trace span, none of which a
  // live bearer credential may reach.
  const res = await postJson("/api/invites/accept", { code });
  if (!res.ok) return null;
  return (await res.json()) as WorldEntry;
}
