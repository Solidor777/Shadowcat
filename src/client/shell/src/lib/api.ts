import type { WorldEntry } from "@shadowcat/types";

/** Local mirror of the server's MeResponse (not ts-rs-exported). */
export interface Me {
  id: string;
  username: string;
  server_role: string;
}

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

export async function getMe(): Promise<Me | null> {
  const res = await fetch("/api/me", { headers: { accept: "application/json" } });
  if (res.status === 401) return null;
  if (!res.ok) throw new Error(`/api/me → ${res.status}`);
  return (await res.json()) as Me;
}

export async function logout(): Promise<void> {
  await postJson("/api/logout", {});
}

export function listWorlds(): Promise<WorldEntry[]> {
  return getJson<WorldEntry[]>("/api/worlds");
}

/** A world member (GM-only endpoint). Mirrors the server's MemberEntry. */
export interface WorldMember {
  user: string;
  username: string;
  role: "gm" | "player" | "spectator";
}

export function listWorldMembers(world: string): Promise<WorldMember[]> {
  return getJson<WorldMember[]>(`/api/worlds/${world}/members`);
}

/** Per-user UI session state. The server stores this opaquely (object + size cap);
 * the client owns the structure. */
export interface UiState {
  global: { locale: string; lastWorld: string | null };
  worlds: Record<string, { activeTab?: string }>;
}

function defaultUiState(): UiState {
  return { global: { locale: "en", lastWorld: null }, worlds: {} };
}

export async function getUiState(): Promise<UiState> {
  const raw = await getJson<Partial<UiState>>("/api/me/ui-state");
  const def = defaultUiState();
  return {
    global: { ...def.global, ...(raw.global ?? {}) },
    worlds: raw.worlds ?? {},
  };
}

export async function putUiState(
  state: UiState,
  opts: { keepalive?: boolean } = {},
): Promise<void> {
  // `keepalive` lets the request outlive a page unload (the blob is within the
  // server's 64KB cap, under keepalive's body limit).
  const res = await fetch("/api/me/ui-state", {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(state),
    keepalive: opts.keepalive,
  });
  if (!res.ok) throw new Error(`PUT /api/me/ui-state → ${res.status}`);
}
