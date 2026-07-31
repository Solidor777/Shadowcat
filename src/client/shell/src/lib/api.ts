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

/** A world member (visible to every world member). Mirrors the server's MemberEntry. */
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
  // `panelLayout` is an opaque blob owned by @shadowcat/module-panels' persistence
  // codec (encodeLayout/decodeLayout) — the shell never inspects its shape.
  // `chatRead` is likewise opaque, owned by the chat module's per-channel
  // last-read-marker tracking (unread tab badge) — the shell only stores it.
  worlds: Record<string, { panelLayout?: unknown; chatRead?: unknown }>;
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
 * Sending only changed leaves is the concurrency control — concurrent
 * sessions of one account (two tabs, or two independent module owners of the
 * same world's slice) contend only on the individual fields/keys both
 * actually write. */
export interface UiStatePatch {
  global?: Partial<UiState["global"]>;
  worlds?: Record<string, Partial<UiState["worlds"][string]>>;
}

export async function putUiState(
  patch: UiStatePatch,
  opts: { keepalive?: boolean } = {},
): Promise<void> {
  // `keepalive` lets the request outlive a page unload (the patch is within
  // the server's 64KB merged cap, under keepalive's body limit).
  const res = await fetch("/api/me/ui-state", {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(patch),
    keepalive: opts.keepalive,
  });
  if (!res.ok) throw new Error(`PUT /api/me/ui-state → ${res.status}`);
}
