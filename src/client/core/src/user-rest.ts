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

/** Every account on the server. Admin-only (`AdminUser` extractor gates on
 * `ServerRole::Admin`, never on world role); a non-admin caller gets a 403.
 * @returns Every server account.
 * @example
 * ```ts
 * import { listUsers } from "@shadowcat/core";
 *
 * const users = await listUsers();
 * ```
 */
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
 * a membership change it just caused re-reads through this. Any world member may
 * call it (chat resolves user ids to names for every viewer); a non-member gets a
 * 403, never a 404 — the world id is caller-supplied, so a distinguishable
 * unknown-world response would confirm existence to a non-member (`list_members`,
 * `http/routes.rs`).
 * @param world The world id.
 * @returns The world's member roster.
 * @example
 * ```ts
 * import { listWorldMembers } from "@shadowcat/core";
 *
 * const members = await listWorldMembers("00000000-0000-0000-0000-000000000001");
 * ```
 */
export async function listWorldMembers(world: string): Promise<WorldMember[]> {
  const res = await fetch(`/api/worlds/${encodeURIComponent(world)}/members`, {
    headers: { accept: "application/json" },
  });
  if (!res.ok) throw new Error(await restError(res, "list members failed"));
  return (await res.json()) as WorldMember[];
}

/** Create an account. Admin-only. `serverRole` defaults to a plain user
 * server-side; passing "admin" mints another administrator. The plaintext
 * password is sent once and never echoed back. A case-insensitive username
 * collision surfaces as a client-actionable 409 ("username already taken"),
 * never a raw constraint-violation 500 (`create_user_unique`, `data/sqlite.rs`).
 * @param opts The new account's fields.
 * @param opts.username The new account's username (server validates length/charset/uniqueness).
 * @param opts.password The new account's plaintext password, sent once.
 * @param opts.serverRole The new account's server tier; omitted means a plain user.
 * @returns The created account.
 * @example
 * ```ts
 * import { createUser } from "@shadowcat/core";
 *
 * const user = await createUser({ username: "example-user", password: "correct-horse-battery-staple" });
 * ```
 */
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
 * client-actionable — surface it verbatim. An unknown `id` gets a plain 404
 * (`DataError::NotFound`, `data/sqlite.rs::delete_user`). The account's
 * sessions are revoked inside the delete transaction, so a reconnect fails
 * authentication; after that commit, the deleted account's live connections
 * are separately evicted from every room.
 * @param id The account id to delete.
 * @example
 * ```ts
 * import { deleteUser } from "@shadowcat/core";
 *
 * await deleteUser("00000000-0000-0000-0000-000000000001");
 * ```
 */
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

/** Mint a single-use invite for a world. GM of that world only (`require_gm`: a
 * non-member OR a member who isn't that world's GM both get a uniform 403 — the
 * two cases are not distinguished). `role` is a WorldRole — an invite cannot
 * express, let alone confer, a server tier. Minting past the per-world active-
 * invite cap (`MAX_ACTIVE_INVITES_PER_WORLD`) is a 409 asking the caller to
 * revoke one first.
 *
 * The GM never names the invited account: naming one would make the membership
 * route a username-existence oracle. The invited user redeems the code from
 * their own session instead.
 * @param world The world id to mint an invite for.
 * @param role The `WorldRole` the invite will seat the redeemer at.
 * @returns The minted invite, including its one-time-visible `code`.
 * @example
 * ```ts
 * import { createWorldInvite } from "@shadowcat/core";
 *
 * const invite = await createWorldInvite("00000000-0000-0000-0000-000000000001", "player");
 * ```
 */
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

/** A world's invites. GM of that world only (`require_gm`, same uniform-403 gate as
 * `createWorldInvite`); codes are not recoverable here — only the bearer `code`
 * returned once at mint time can redeem an invite.
 * @param world The world id.
 * @returns The world's invite listing (no bearer codes).
 * @example
 * ```ts
 * import { listWorldInvites } from "@shadowcat/core";
 *
 * const invites = await listWorldInvites("00000000-0000-0000-0000-000000000001");
 * ```
 */
export async function listWorldInvites(world: string): Promise<InviteEntry[]> {
  const res = await fetch(`/api/worlds/${encodeURIComponent(world)}/invites`, {
    headers: { accept: "application/json" },
  });
  if (!res.ok) throw new Error(await restError(res, "list invites failed"));
  return (await res.json()) as InviteEntry[];
}

/** Revoke an invite, effective immediately (redemption re-checks revocation in its
 * consume statement). GM of that world only, and the revoke is scoped to `world` in
 * SQL — `codeId` belonging to a DIFFERENT world 404s identically to an unknown
 * `codeId`, so this route never confirms an invite id's existence outside the
 * caller's own world (`revoke_invite`, `http/routes.rs`).
 * @param world The world id.
 * @param codeId The invite id to revoke.
 * @example
 * ```ts
 * import { revokeWorldInvite } from "@shadowcat/core";
 *
 * await revokeWorldInvite("00000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-000000000002");
 * ```
 */
export async function revokeWorldInvite(world: string, codeId: string): Promise<void> {
  const res = await fetch(
    `/api/worlds/${encodeURIComponent(world)}/invites/${encodeURIComponent(codeId)}`,
    { method: "DELETE" },
  );
  if (!res.ok) throw new Error(await restError(res, "revoke invite failed"));
}

/** The server's `{ error }` body when present, else the bare status. Handlers
 * return only client-actionable text (5xx detail is logged server-side, never
 * echoed), so surfacing it is safe. Not exported — the shared error-message
 * helper every wrapper in this file calls.
 * @param res The failed `Response` (already known non-ok by the caller).
 * @param fallback The message to use when the body has no usable `error` string.
 * @returns The server's error message, or `"<fallback>: <status>"`.
 * @example
 * ```
 * // internal helper; not part of the public API
 * await restError(res, "list users failed");
 * ```
 */
async function restError(res: Response, fallback: string): Promise<string> {
  try {
    const body = (await res.json()) as { error?: unknown };
    if (typeof body.error === "string" && body.error) return body.error;
  } catch {
    // Non-JSON body — fall through to the status-only message.
  }
  return `${fallback}: ${res.status}`;
}
