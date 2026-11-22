import type { WorldRole } from "@shadowcat/types";

// Account + membership REST, beside the `module-rest` module. Core stays framework-neutral,
// so no Svelte in its dependency closure — the settings module's admin user-management and
// GM member-add surfaces can both consume it.
//
// The server is the sole authority on who may call these: `/api/users` is
// gated on ServerRole::Admin and membership writes on world-GM. Nothing here
// is a permission check — a caller without authority gets a 403 the UI
// surfaces as an error.

/** A server account as the admin surface sees it. Carries no credential
 * material: the server never selects or serializes the password hash. */
export interface ServerUser {
  /** The account's server-assigned id. */
  id: string;
  /** The account's login name, unique case-insensitively across the server. */
  username: string;
  /** Server-wide tier: `"admin"` may call the admin-only user-management routes; `"user"` may not. */
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

/** Bound on `listWorldMembers`'s fetch. A hung backend request otherwise leaves
 * the caller's roster read pending forever. */
const LIST_WORLD_MEMBERS_TIMEOUT_MS = 15_000;

/** A world member. Visible to every member of that world. */
export interface WorldMember {
  /** The member's account id (`ServerUser.id`). */
  user: string;
  /** The member's account username, as of this roster read. */
  username: string;
  /** The member's role within this world (distinct from `ServerUser.server_role`). */
  role: WorldRole;
  /** The member's colour accent as `#rrggbb`, or `null`/absent when unset. */
  colour?: string | null;
  /** The member's pronouns display text, or `null`/absent when unset. */
  pronouns?: string | null;
  /** Whether the member has an avatar; the bytes are fetched from `avatarUrl`, never inline. */
  has_avatar?: boolean;
}

/** A world's roster, straight from the server. Distinct from `AppContext`'s
 * `members` map, which is a session-start snapshot: a surface that must reflect
 * a membership change it just caused re-reads through this. Any world member may
 * call it (chat resolves user ids to names for every viewer); a non-member gets a
 * 403, never a 404 — the world id is caller-supplied, so a distinguishable
 * unknown-world response would confirm existence to a non-member
 * (`http::routes::list_members`).
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
    signal: AbortSignal.timeout(LIST_WORLD_MEMBERS_TIMEOUT_MS),
  });
  if (!res.ok) throw new Error(await restError(res, "list members failed"));
  return (await res.json()) as WorldMember[];
}

/** The URL of a world member's avatar image, or `null` when the member has none. The route is
 * scoped under the world so the roster's own membership boundary applies
 * (`http::routes::get_member_avatar`).
 * @param world The world id.
 * @param user The member's account id.
 * @param hasAvatar The roster's `has_avatar` flag for that member.
 * @returns The avatar URL, or `null` when `hasAvatar` is false.
 * @example
 * ```ts
 * import { avatarUrl } from "@shadowcat/core";
 *
 * const url = avatarUrl("00000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-000000000002", true);
 * ```
 */
export function avatarUrl(world: string, user: string, hasAvatar: boolean | undefined): string | null {
  if (!hasAvatar) return null;
  return `/api/worlds/${encodeURIComponent(world)}/members/${encodeURIComponent(user)}/avatar`;
}

/** The values `updateProfile` stores. */
export interface ProfileUpdate {
  /** Colour accent as `#rrggbb`, or `null` to clear it. */
  colour: string | null;
  /** Pronouns text (at most 32 printable characters), or `null` to clear it. */
  pronouns: string | null;
}

/** Replace the signed-in account's own colour accent and pronouns. Both are replaced together;
 * `null` clears one. The server validates the colour (`#rrggbb`) and the pronouns (at most 32
 * printable characters) and answers 422 with a client-actionable message.
 * @param profile The new colour and pronouns; its fields are described on `ProfileUpdate`.
 * @example
 * ```ts
 * import { updateProfile } from "@shadowcat/core";
 *
 * await updateProfile({ colour: "#336699", pronouns: "they/them" });
 * ```
 */
export async function updateProfile(profile: ProfileUpdate): Promise<void> {
  const res = await fetch("/api/account/profile", {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ colour: profile.colour, pronouns: profile.pronouns }),
  });
  if (!res.ok) throw new Error(await restError(res, "update profile failed"));
}

/** Set the signed-in account's own avatar from an image file. The body is the raw file bytes;
 * the server sniffs the real type (PNG, JPEG or WebP), caps the size at 128 KiB and decodes
 * under a bound, answering 413 or 422 with a client-actionable message.
 * @param file The image to upload.
 * @example
 * ```ts
 * import { uploadAvatar } from "@shadowcat/core";
 *
 * await uploadAvatar(new File([new Uint8Array(8)], "me.png", { type: "image/png" }));
 * ```
 */
export async function uploadAvatar(file: Blob): Promise<void> {
  const res = await fetch("/api/account/avatar", { method: "PUT", body: file });
  if (!res.ok) throw new Error(await restError(res, "upload avatar failed"));
}

/** Clear the signed-in account's own avatar. Clearing an absent avatar also succeeds.
 * @example
 * ```ts
 * import { deleteAvatar } from "@shadowcat/core";
 *
 * await deleteAvatar();
 * ```
 */
export async function deleteAvatar(): Promise<void> {
  const res = await fetch("/api/account/avatar", { method: "DELETE" });
  if (!res.ok) throw new Error(await restError(res, "delete avatar failed"));
}

/** Fields for `createUser`. */
export interface CreateUserOptions {
  /** The new account's username (server validates length/charset/uniqueness). */
  username: string;
  /** The new account's plaintext password, sent once. */
  password: string;
  /** The new account's server tier; omitted means a plain user. */
  serverRole?: "admin" | "user";
}

/** Create an account. Admin-only. `serverRole` defaults to a plain user
 * server-side; passing "admin" mints another administrator. The plaintext
 * password is sent once and never echoed back. A case-insensitive username
 * collision surfaces as a client-actionable 409 ("username already taken"),
 * never a raw constraint-violation 500 (`data::sqlite::SqliteRepository::create_user_unique`).
 * @param opts The new account's fields.
 * @returns The created account.
 * @example
 * ```ts
 * import { createUser } from "@shadowcat/core";
 *
 * const user = await createUser({ username: "example-user", password: "correct-horse-battery-staple" });
 * ```
 */
export async function createUser(opts: CreateUserOptions): Promise<ServerUser> {
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
 * (`DataError::NotFound`, `SqliteRepository::delete_user`). The account's
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

/** The failure `changeOwnPassword` throws. Carries the HTTP status so a caller can tell a
 * refused credential (401) from a refused new password (422) without parsing the message. */
export class ChangePasswordError extends Error {
  /** The HTTP status of the refused request. */
  readonly status: number;

  /** Builds the error from the server's client-actionable message and the response status.
   * @param message The server's error text, or the bare-status fallback.
   * @param status The HTTP status of the refused request.
   * @example
   * ```ts
   * import { ChangePasswordError } from "@shadowcat/core";
   *
   * const err = new ChangePasswordError("unauthorized", 401);
   * ```
   */
  constructor(message: string, status: number) {
    super(message);
    this.name = "ChangePasswordError";
    this.status = status;
  }
}

/** Change the signed-in account's own password. The server re-verifies `current` (a wrong one
 * is a 401, indistinguishable from any other credential refusal) and checks `next` against its
 * password policy (422). On success the server revokes EVERY session for the account, this
 * one included, so the caller must end its own session state instead of carrying on.
 * @param current The account's current plaintext password.
 * @param next The new plaintext password.
 * @throws ChangePasswordError On any non-204 response, carrying the response status.
 * @example
 * ```ts
 * import { changeOwnPassword } from "@shadowcat/core";
 *
 * await changeOwnPassword("current-password", "a-new-password");
 * ```
 */
export async function changeOwnPassword(current: string, next: string): Promise<void> {
  const res = await fetch("/api/account/password", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ current_password: current, new_password: next }),
  });
  if (!res.ok) {
    throw new ChangePasswordError(await restError(res, "change password failed"), res.status);
  }
}

/** A minted invite. `code` is a bearer credential the server keeps only as a
 * hash — it is returned once, at mint, and is unrecoverable afterwards. */
export interface MintedInvite {
  /** The invite's id, used to list/revoke it (never used to redeem it — that needs `code`). */
  id: string;
  /** The bearer redemption credential, returned only here — the server keeps just its hash. */
  code: string;
  /** The `WorldRole` a redeemer is seated at; cannot express a server tier. */
  role: WorldRole;
  /** Unix-ms timestamp after which redemption fails. */
  expires_at: number;
}

/** An invite in a GM's listing. Carries no credential material. */
export interface InviteEntry {
  /** The invite's id, used to revoke it (not a redemption credential — see `MintedInvite.code`). */
  id: string;
  /** The `WorldRole` a redeemer will be seated at. */
  role: WorldRole;
  /** Unix-ms timestamp the invite was minted at. */
  created_at: number;
  /** Unix-ms timestamp after which redemption fails, regardless of `revoked_at`/`consumed_at`. */
  expires_at: number;
  /** Unix-ms timestamp the GM revoked it at, or `null` if never revoked. */
  revoked_at: number | null;
  /** Unix-ms timestamp it was redeemed at, or `null` if unredeemed (single-use: a non-null value
   * means the code can no longer be redeemed). */
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
 * caller's own world (`http::routes::revoke_invite`).
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

/** Bound on every UI-state fetch. A hung backend request otherwise pins the boot chain and the
 * unload flush forever. */
const UI_STATE_TIMEOUT_MS = 15_000;

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
    /** The account's theme preference, in the ui-kit `PersistedTheme` shape
     * (`ThemeController.serialize`'s output). Typed `unknown` here because core cannot depend on
     * ui-kit; the server stores it opaquely and `ThemeController.load` validates it. Optional:
     * blobs written before the field existed simply omit it. */
    theme?: unknown;
    /** The account's keybinding overrides, keyed `"<moduleId>:<bindingId>"`; `null` marks an
     * explicitly unbound binding. Optional: blobs written before the field existed omit it. The
     * server stores it opaquely — `KeybindingsController.loadOverrides` canonicalizes it. */
    keybindings?: Record<string, string | null>;
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
      /** The composer's recalled sent-text history, oldest first (`pushChatHistory` in
       * `@shadowcat/core` bounds it by entry count and serialized size). Per account, never
       * transmitted to other users. */
      chatHistory?: string[];
      /** GM's last-chosen level id per scene, keyed by scene id. Absent scene key = follow the
       * default (first/lowest level); never set for a player, who instead follows their primary
       * token's level. */
      viewedLevel?: Record<string, string>;
    }
  >;
  /** Per-module account settings, keyed by module id: one opaque object per module, owned by that
   * module and stored across worlds (`ModuleContext.settings.getUser`/`setUser`). */
  modules: Record<string, Record<string, unknown>>;
}

/** The `UiState` shape for an account with no persisted blob yet.
 * @returns A fresh `UiState`: locale `"en"`, no `lastWorld`, no per-world or per-module
 *   entries.
 * @example
 * ```ts
 * import { defaultUiState } from "@shadowcat/core";
 *
 * const empty = defaultUiState();
 * ```
 */
export function defaultUiState(): UiState {
  return { global: { locale: "en", lastWorld: null }, worlds: {}, modules: {} };
}

/** Fetches the caller's UI-state blob, filling in defaults for any field the
 * server omits (a brand-new account, or a blob written before a field
 * existed).
 * @returns The caller's `UiState`, with `global` defaults applied field-by-
 *   field and `worlds` and `modules` defaulted to `{}` if absent.
 * @example
 * ```ts
 * import { getUiState } from "@shadowcat/core";
 *
 * const ui = await getUiState();
 * ```
 */
export async function getUiState(): Promise<UiState> {
  const url = "/api/me/ui-state";
  const res = await fetch(url, {
    headers: { accept: "application/json" },
    signal: AbortSignal.timeout(UI_STATE_TIMEOUT_MS),
  });
  if (!res.ok) throw new Error(`${url} → ${res.status}`);
  const raw = (await res.json()) as Partial<UiState>;
  const def = defaultUiState();
  return {
    global: { ...def.global, ...(raw.global ?? {}) },
    worlds: raw.worlds ?? {},
    modules: raw.modules ?? {},
  };
}

/** Partial UI-state write, at leaf-key granularity — **the single
 * client-side statement of the merge rule** (mirrors
 * `SqliteRepository::merge_ui_state`'s doc comment server-side). Only the
 * individual fields/keys present are written: each present `global.<field>`
 * (e.g. `locale`, `lastWorld`) replaces just that field server-side; each
 * present `worlds.<id>.<key>` (e.g. `panelLayout`, `chatRead`) replaces just
 * that key within `worlds.<id>`, and each present `modules.<id>.<key>` replaces just that key
 * within `modules.<id>` — never the whole slice, and never the whole
 * per-id object unless every key happens to be present. A present
 * value still replaces its leaf wholesale (a `panelLayout`/`chatRead` blob is
 * opaque and is never itself deep-merged). Absent fields/keys are untouched.
 * A `null` value for a whole `worlds.<id>` or `modules.<id>` entry instead REMOVES that entry
 * server-side (`merge_one_level`'s whole-key-removal branch) — distinct from
 * a `null` leaf value nested inside a per-id object, which removes just
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
  /** Dirty per-module keys only, keyed by module id, with the same `null` rules as `worlds`: a
   * `null` module id removes that module's whole slice, a `null` leaf removes that one key. */
  modules?: Record<string, Record<string, unknown> | null>;
}

/** Writes a partial UI-state patch (see `UiStatePatch` for the merge rule).
 * Throws on a non-2xx response.
 * @param patch - The leaf fields/keys to write; anything absent is untouched
 *   server-side.
 * @param opts - Write options.
 * @param opts.keepalive - When true, sets `fetch`'s `keepalive` flag so the
 *   request can outlive a page unload — used by `flushOnUnload`.
 * @example
 * ```ts
 * import { putUiState } from "@shadowcat/core";
 *
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
    signal: AbortSignal.timeout(UI_STATE_TIMEOUT_MS),
  });
  if (!res.ok) throw new Error(`PUT /api/me/ui-state → ${res.status}`);
}

/** The server's `{ error }` body when present, else the bare status. Handlers
 * return only client-actionable text (5xx detail is logged server-side, never
 * echoed), so surfacing it is safe. Exported to sibling REST modules
 * (`trash-rest`) only, not from the package barrel: the shared error-message
 * helper every request wrapper calls.
 * @param res The failed `Response` (already known non-ok by the caller).
 * @param fallback The message to use when the body has no usable `error` string.
 * @returns The server's error message, or `"<fallback>: <status>"`.
 * @example
 * ```
 * // internal helper; not part of the public API
 * declare const res: Response;
 * await restError(res, "list users failed");
 * ```
 */
export async function restError(res: Response, fallback: string): Promise<string> {
  try {
    const body = (await res.json()) as { /** The server's client-actionable error message, if the body is JSON and carries one. */ error?: unknown };
    if (typeof body.error === "string" && body.error) return body.error;
  } catch {
    // Non-JSON body — fall through to the status-only message.
  }
  return `${fallback}: ${res.status}`;
}
