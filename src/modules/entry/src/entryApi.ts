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
