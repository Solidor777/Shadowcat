import type { InstalledModuleInfo } from "@shadowcat/types";

// Client-side module-toolchain REST, beside the `asset-rest` module: the installed-module
// discovery + per-world enablement contract with the server. Core stays framework-neutral,
// so no Svelte in its dependency closure — shared by the settings module's GM management UI
// and the world session's external-module load path.

/** Every validly installed module the server discovered under its modules folder.
 * @returns The `GET /api/modules` list (any authenticated caller may read it).
 * @example
 * ```ts
 * import { listInstalledModules } from "@shadowcat/core";
 *
 * const modules = await listInstalledModules();
 * ```
 */
export async function listInstalledModules(): Promise<InstalledModuleInfo[]> {
  const res = await fetch("/api/modules", { headers: { accept: "application/json" } });
  if (!res.ok) throw new Error(`list installed modules failed: ${res.status}`);
  return (await res.json()) as InstalledModuleInfo[];
}

/** A world's enabled installed-module ids. Any world member may read this
 * (needed at join to load the enabled set).
 * @param world The world's id.
 * @returns The world's currently-enabled module ids (folder ids, not manifest ids).
 * @example
 * ```ts
 * import { getEnabledModules } from "@shadowcat/core";
 *
 * const ids = await getEnabledModules("00000000-0000-0000-0000-000000000001");
 * ```
 */
export async function getEnabledModules(world: string): Promise<string[]> {
  const res = await fetch(`/api/worlds/${world}/enabled-modules`, {
    headers: { accept: "application/json" },
  });
  if (!res.ok) throw new Error(`get enabled modules failed: ${res.status}`);
  return (await res.json()) as string[];
}

/** Replace a world's enabled installed-module set. GM/admin only server-side
 * (a non-GM caller gets a 403, surfaced via the thrown error).
 * @param world The world's id.
 * @param ids The new enabled-module set (folder ids).
 * @example
 * ```ts
 * import { setEnabledModules } from "@shadowcat/core";
 *
 * await setEnabledModules("00000000-0000-0000-0000-000000000001", ["example-module"]);
 * ```
 */
export async function setEnabledModules(world: string, ids: string[]): Promise<void> {
  const res = await fetch(`/api/worlds/${world}/enabled-modules`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(ids),
  });
  if (!res.ok) throw new Error(`set enabled modules failed: ${res.status}`);
}
