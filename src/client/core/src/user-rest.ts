import type { WorldRole } from "@shadowcat/types";

// Account + membership REST, beside module-rest.ts. Framework-neutral (no
// Svelte in core's closure, invariant #7) so the settings module's admin
// user-management and GM member-add surfaces can both consume it.
//
// The server is the sole authority on who may call these: `/api/users` is
// gated on ServerRole::Admin and membership writes on world-GM. Nothing here
// is a permission check — a caller without authority gets a 403 the UI
// surfaces as an error.

/** A server account as the admin surface sees it. Carries no credential
 * material: the server never selects or serializes the password hash. */
export interface ServerUser {
  id: string;
  username: string;
  server_role: "admin" | "user";
}

/** Every account on the server. Admin-only; a non-admin caller gets a 403. */
export async function listUsers(): Promise<ServerUser[]> {
  const res = await fetch("/api/users", { headers: { accept: "application/json" } });
  if (!res.ok) throw new Error(`list users failed: ${res.status}`);
  return (await res.json()) as ServerUser[];
}

/** Create an account. Admin-only. `serverRole` defaults to a plain user
 * server-side; passing "admin" mints another administrator. The plaintext
 * password is sent once and never echoed back. */
export async function createUser(opts: {
  username: string;
  password: string;
  serverRole?: "admin" | "user";
}): Promise<ServerUser> {
  const res = await fetch("/api/users", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      username: opts.username,
      password: opts.password,
      ...(opts.serverRole ? { server_role: opts.serverRole } : {}),
    }),
  });
  if (!res.ok) throw new Error(await restError(res, "create user failed"));
  return (await res.json()) as ServerUser;
}

/** Seat an existing account in a world (or change its world role) by username.
 * GM-only. `role` is a WorldRole — the route accepts no server-tier value, so
 * this can never grant server administration. */
export async function addWorldMemberByUsername(
  world: string,
  username: string,
  role: WorldRole,
): Promise<void> {
  const res = await fetch(`/api/worlds/${world}/members`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ username, role }),
  });
  if (!res.ok) throw new Error(await restError(res, "add member failed"));
}

/** The server's `{ error }` body when present, else the bare status. Handlers
 * return only client-actionable text (5xx detail is logged server-side, never
 * echoed), so surfacing it is safe. */
async function restError(res: Response, fallback: string): Promise<string> {
  try {
    const body = (await res.json()) as { error?: unknown };
    if (typeof body.error === "string" && body.error) return body.error;
  } catch {
    // Non-JSON body — fall through to the status-only message.
  }
  return `${fallback}: ${res.status}`;
}
