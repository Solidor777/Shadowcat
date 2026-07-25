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
  if (!res.ok) throw new Error(await restError(res, "list users failed"));
  return (await res.json()) as ServerUser[];
}

/** A world member. Visible to every member of that world. */
export interface WorldMember {
  user: string;
  username: string;
  role: WorldRole;
}

/** A world's roster, straight from the server. Distinct from `AppContext`'s
 * `members` map, which is a session-start snapshot: a surface that must reflect
 * a membership change it just caused re-reads through this. */
export async function listWorldMembers(world: string): Promise<WorldMember[]> {
  const res = await fetch(`/api/worlds/${encodeURIComponent(world)}/members`, {
    headers: { accept: "application/json" },
  });
  if (!res.ok) throw new Error(await restError(res, "list members failed"));
  return (await res.json()) as WorldMember[];
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

/** Delete a user account (server-admin only). The server refuses self-
 * deletion and deleting the last administrator with a 409 whose message is
 * client-actionable — surface it verbatim. */
export async function deleteUser(id: string): Promise<void> {
  const res = await fetch(`/api/users/${encodeURIComponent(id)}`, { method: "DELETE" });
  if (!res.ok) throw new Error(await restError(res, "delete user failed"));
}

/** A minted invite. `code` is a bearer credential the server keeps only as a
 * hash — it is returned once, at mint, and is unrecoverable afterwards. */
export interface MintedInvite {
  id: string;
  code: string;
  role: WorldRole;
  expires_at: number;
}

/** An invite in a GM's listing. Carries no credential material. */
export interface InviteEntry {
  id: string;
  role: WorldRole;
  created_at: number;
  expires_at: number;
  revoked_at: number | null;
  consumed_at: number | null;
}

/** Mint a single-use invite for a world. GM of that world only. `role` is a
 * WorldRole — an invite cannot express, let alone confer, a server tier.
 *
 * The GM never names the invited account: naming one would make the membership
 * route a username-existence oracle. The invited user redeems the code from
 * their own session instead. */
export async function createWorldInvite(
  world: string,
  role: WorldRole,
): Promise<MintedInvite> {
  const res = await fetch(`/api/worlds/${encodeURIComponent(world)}/invites`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ role }),
  });
  if (!res.ok) throw new Error(await restError(res, "create invite failed"));
  return (await res.json()) as MintedInvite;
}

/** A world's invites. GM of that world only; codes are not recoverable here. */
export async function listWorldInvites(world: string): Promise<InviteEntry[]> {
  const res = await fetch(`/api/worlds/${encodeURIComponent(world)}/invites`, {
    headers: { accept: "application/json" },
  });
  if (!res.ok) throw new Error(await restError(res, "list invites failed"));
  return (await res.json()) as InviteEntry[];
}

/** Revoke an invite, effective immediately. GM of that world only. */
export async function revokeWorldInvite(world: string, codeId: string): Promise<void> {
  const res = await fetch(
    `/api/worlds/${encodeURIComponent(world)}/invites/${encodeURIComponent(codeId)}`,
    { method: "DELETE" },
  );
  if (!res.ok) throw new Error(await restError(res, "revoke invite failed"));
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
