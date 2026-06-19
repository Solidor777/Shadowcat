// Client-side mirror of the server's capability resolution (resolve_access_world
// + required_cap_for_path + declarative requirements). ADVISORY ONLY: used to
// gate module UI/actions for UX. The server remains authoritative — a bypass is
// rejected at apply_intent.
import type { WorldRole } from "@shadowcat/types";
import type { WireDocument, WireCapabilityRequirement } from "./wire";

type Grants = { by_role: Record<string, string[]>; by_user: Record<string, string[]> };
type Perms = WireDocument["permissions"];

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
 * Resolve an actor's effective (non-GM) capability set on a document, mirroring
 * the server's `resolve_access_world`: the DocRole floor widened by the
 * document's additive grants and the world-default grants. GM/admin holds
 * everything — callers pass `role === "gm"` to `canWritePath`, which
 * short-circuits; this returns the concrete non-GM set.
 */
export function resolveCaps(
  perms: Perms,
  userId: string,
  _role: WorldRole,
  worldGrants: Grants,
): Set<string> {
  const docRole = perms.users[userId] ?? perms.default;
  const caps = new Set<string>(roleFloor(docRole));
  for (const c of perms.capabilities.by_role[docRole] ?? []) caps.add(c);
  for (const c of perms.capabilities.by_user[userId] ?? []) caps.add(c);
  for (const c of worldGrants.by_role[docRole] ?? []) caps.add(c);
  for (const c of worldGrants.by_user[userId] ?? []) caps.add(c);
  return caps;
}

/** The structural base capability for a field path (mirrors the server). */
function baseCapForPath(path: string): string | null {
  if (path === "/system" || path.startsWith("/system/")) return "core:write_fields";
  if (path === "/embedded" || path.startsWith("/embedded/")) return "core:manage_embedded";
  if (path === "/permissions" || path.startsWith("/permissions/")) return "core:edit_permissions";
  return null;
}

/** Whether `a` and `b` overlap as JSON-pointer subtrees (either contains the other). */
function pathsOverlap(a: string, b: string): boolean {
  return a === b || a.startsWith(`${b}/`) || b.startsWith(`${a}/`);
}

/**
 * Whether the actor may write `path` on a document, given its resolved caps and
 * the world's declarative requirements. Mirrors the server: the structural base
 * cap must be held, plus every declared cap for any requirement whose prefix
 * overlaps the path (ancestor or descendant). GM bypasses all checks. Advisory.
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
