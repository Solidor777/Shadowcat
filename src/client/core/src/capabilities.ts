// Client-side mirror of the server's capability resolution (resolve_access_world
// + required_cap_for_path + declarative requirements). ADVISORY ONLY: used to
// gate module UI/actions for UX. The server remains authoritative — a bypass is
// rejected at apply_intent.
//
// Mirrors the server's Update-path gate (canWritePath). The server additionally
// gates Create against requirements present in the new body
// (declared_caps_for_document); a client-side canCreateDoc mirror is not yet
// provided, so the UI cannot pre-gate a create the server will reject. Advisory
// only — the create is still enforced server-side.
// TODO: Add a canCreateDoc advisory mirror of the server's Create-path check.
import type { WorldRole } from "@shadowcat/types";
import type { WireDocument, WireCapabilityRequirement } from "./wire";

type Grants = { by_role: Record<string, string[]>; by_user: Record<string, string[]> };
type Perms = WireDocument["permissions"];

/** The built-in capability floor for a `DocRole`, mirroring the server's `role_floor`
 * (`data/permission.rs`): `owner` → read + write_fields, `observer` → read, anything
 * else (including `"none"`) → no capabilities — fail-closed by construction, since an
 * unrecognized/unknown role string falls through to the empty set rather than a grant.
 * Not exported — folded into `resolveCaps`'s public surface.
 * @param role A `DocRole` string (`"owner"` | `"observer"` | `"none"`).
 * @returns The floor capability list for `role`.
 * @example
 * ```
 * // internal helper; not part of the public API (see resolveCaps for the public entry point)
 * roleFloor("owner"); // ["core:read", "core:write_fields"]
 * ```
 */
function roleFloor(role: string): string[] {
  switch (role) {
    case "owner":
      return ["core:read", "core:write_fields"];
    case "observer":
      return ["core:read"];
    default:
      return [];
  }
}

/**
 * Resolve a user's effective (non-GM) capability set on a document, mirroring
 * the server's `resolve_access_world`: the DocRole floor widened by the
 * document's additive grants and the world-default grants. This function does
 * not itself branch on GM/admin status — the sole production caller
 * (`worldSession.canEdit`, `worldSession.svelte.ts:153`) returns early for
 * `role === "gm"` before calling in, so `resolveCaps` only ever computes the
 * non-GM floor in practice.
 * @param perms The document's `permissions` block.
 * @param userId The resolving user's id.
 * @param _role The user's world role. Unused here — `resolveCaps` never branches
 * on it directly; the parameter exists for call-site symmetry with the server's
 * `resolve_access_world`. The sole production caller (`worldSession.canEdit`)
 * already returns early for a GM before reaching this function.
 * @param worldGrants The world's default per-role/per-user capability grants.
 * @param isEffectiveOwner Whether `userId` is the effective owner of a TOKEN document
 * (see `effectiveOwner` in `./actor`); floors the resolved role at `"owner"`. Defaults
 * to `false` — pass `true` only for `doc_type === "token"`, mirroring the server's
 * `owner_floor` gate (`data/permission.rs`), which applies this floor to no other
 * `doc_type`.
 * @returns The resolved capability set (e.g. `"core:read"`, `"core:write_fields"`).
 * @example
 * ```ts
 * import { resolveCaps } from "@shadowcat/core";
 * import type { WireDocument } from "@shadowcat/core";
 *
 * declare const perms: WireDocument["permissions"];
 * resolveCaps(perms, "00000000-0000-0000-0000-000000000001", "player", { by_role: {}, by_user: {} });
 * ```
 */
export function resolveCaps(
  perms: Perms,
  userId: string,
  _role: WorldRole,
  worldGrants: Grants,
  isEffectiveOwner = false,
): Set<string> {
  // Mirrors the server's `effective_role` owner floor: effective ownership of a
  // TOKEN floors the user at `owner` (read + write_fields). Callers pass
  // `isEffectiveOwner` only for a token (see `worldSession.canEdit`) — on every
  // other doc_type `owner` grants no capability. `DocRole` is ordered
  // owner < observer < none, so the floor only strengthens.
  const stored = perms.users[userId] ?? perms.default;
  const docRole = isEffectiveOwner ? "owner" : stored;
  const caps = new Set<string>(roleFloor(docRole));
  for (const c of perms.capabilities.by_role[docRole] ?? []) caps.add(c);
  for (const c of perms.capabilities.by_user[userId] ?? []) caps.add(c);
  for (const c of worldGrants.by_role[docRole] ?? []) caps.add(c);
  for (const c of worldGrants.by_user[userId] ?? []) caps.add(c);
  return caps;
}

/** The structural base capability for a field path (mirrors the server's
 * `required_cap_for_path`, `data/permission.rs`). `/name` is a leaf (a display string,
 * not a container): `/name/...` does NOT match — there is no sub-path to write.
 * Not exported — folded into `canWritePath`'s public surface.
 * @param path A JSON pointer into the document (e.g. `/system/hp`).
 * @returns The required base capability, or `null` if `path` maps to no known gate
 * (advisory fail-closed: `canWritePath` treats `null` as denied, never as unrestricted).
 * @example
 * ```
 * // internal helper; not part of the public API (see canWritePath for the public entry point)
 * baseCapForPath("/system/hp"); // "core:write_fields"
 * ```
 */
function baseCapForPath(path: string): string | null {
  if (
    path === "/system" ||
    path.startsWith("/system/") ||
    path === "/engine" ||
    path.startsWith("/engine/") ||
    path === "/name" ||
    path === "/base" ||
    path.startsWith("/base/")
  ) {
    return "core:write_fields";
  }
  if (path === "/embedded" || path.startsWith("/embedded/")) return "core:manage_embedded";
  if (path === "/permissions" || path.startsWith("/permissions/")) return "core:edit_permissions";
  // `/owner` is the ownership override the effective-owner rule reads: writing it
  // re-targets who may write the document, so it is gated like a permission edit
  // and NOT reachable from the owner floor. A leaf — `/owner/...` has no sub-path.
  if (path === "/owner") return "core:edit_permissions";
  return null;
}

/** Whether `a` and `b` overlap as JSON-pointer subtrees (either contains the other).
 * Not exported — folded into `canWritePath`'s public surface.
 * @param a A JSON pointer.
 * @param b A JSON pointer.
 * @returns `true` if `a === b`, `a` is a descendant of `b`, or `b` is a descendant of `a`.
 * @example
 * ```
 * // internal helper; not part of the public API (see canWritePath for the public entry point)
 * pathsOverlap("/system/hp", "/system"); // true — /system/hp is a descendant of /system
 * ```
 */
function pathsOverlap(a: string, b: string): boolean {
  return a === b || a.startsWith(`${b}/`) || b.startsWith(`${a}/`);
}

/**
 * Whether the user may write `path` on a document, given its resolved caps and
 * the world's declarative requirements. Mirrors the server: the structural base
 * cap must be held, plus every declared cap for any requirement whose prefix
 * overlaps the path (ancestor or descendant). Passing `isGm: true` bypasses
 * every check below. Advisory only — the server enforces the real capability
 * check independently at `apply_intent` (`resolve_access_world` +
 * `required_cap_for_path`, `sqlite.rs:2247-2276`), so nothing here is a security
 * boundary.
 *
 * SCOPE NOTE on `isGm: true`: this function has no `permissions.gm_role` input,
 * so it cannot represent the server's `gm_role: Some(role)` cap — when set,
 * `gm_role` floors even a GM's write caps to an ordinary `DocRole` resolution
 * instead of an unconditional grant (`effective_role`/`resolve_access`,
 * `permission.rs:489-498`,`551-557`). Calling this with `isGm: true` against a
 * `gm_role`-capped document would over-permit. Today that combination has no
 * production caller: `gm_role` is set in exactly one place, chat-message
 * construction (`chat/mod.rs:299-341` — `Public` → `None`, `Whisper` →
 * `Some(None)`, `GmOnly` → `Some(Observer)`), and no chat module calls
 * `canEdit`/`canWritePath` at all — the production callers are the token
 * panels and sheets (`modules/{actors,conditions,sheet-item,sheet-actor,
 * sheet-fallback}`), which never touch a `gm_role`-capped document; and
 * `isGm: true` itself has no production caller today (`worldSession.canEdit`
 * resolves the GM case and always passes `isGm: false`, see `resolveCaps`'s
 * doc above). The live GM-bypass gate is `worldSession.canEdit`'s own
 * `role === "gm"` early return (`worldSession.svelte.ts:153`), which is
 * equally unaware of `gm_role` — out of scope here (`shell` package).
 * @param path A JSON pointer identifying the field being written.
 * @param caps The caller's resolved capability set (from `resolveCaps`).
 * @param isGm Whether to bypass every check below (see the SCOPE NOTE above).
 * @param requirements The world's declarative capability requirements
 * (`WireCapabilityRequirement[]`), each naming a `path_prefix` subtree and the caps
 * it additionally demands.
 * @returns `true` if the write is advisory-permitted.
 * @example
 * ```ts
 * import { canWritePath } from "@shadowcat/core";
 *
 * canWritePath("/system/hp", new Set(["core:write_fields"]), false, []); // true
 * canWritePath("/permissions", new Set(["core:write_fields"]), false, []); // false — needs core:edit_permissions
 * ```
 */
export function canWritePath(
  path: string,
  caps: Set<string>,
  isGm: boolean,
  requirements: WireCapabilityRequirement[],
): boolean {
  if (isGm) return true;
  const base = baseCapForPath(path);
  if (base === null || !caps.has(base)) return false;
  for (const req of requirements) {
    if (pathsOverlap(path, req.path_prefix)) {
      for (const c of req.caps) if (!caps.has(c)) return false;
    }
  }
  return true;
}
