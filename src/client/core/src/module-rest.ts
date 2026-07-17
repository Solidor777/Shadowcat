import type { InstalledModuleInfo } from "@shadowcat/types";

// Client-side module-toolchain REST, beside asset-rest.ts: the installed-module
// discovery + per-world enablement contract with the server. Framework-neutral
// (no Svelte in core's closure, invariant #7) — shared by the settings module's
// GM management UI and the world session's external-module load path.

/** Every validly installed module the server discovered under its modules folder. */
export async function listInstalledModules(): Promise<InstalledModuleInfo[]> {
  const res = await fetch("/api/modules", { headers: { accept: "application/json" } });
  if (!res.ok) throw new Error(`list installed modules failed: ${res.status}`);
  return (await res.json()) as InstalledModuleInfo[];
}

/** A world's enabled installed-module ids. Any world member may read this
 * (needed at join to load the enabled set). */
export async function getEnabledModules(world: string): Promise<string[]> {
  const res = await fetch(`/api/worlds/${world}/enabled-modules`, {
    headers: { accept: "application/json" },
  });
  if (!res.ok) throw new Error(`get enabled modules failed: ${res.status}`);
  return (await res.json()) as string[];
}

/** Replace a world's enabled installed-module set. GM/admin only server-side
 * (a non-GM caller gets a 403, surfaced via the thrown error). */
export async function setEnabledModules(world: string, ids: string[]): Promise<void> {
  const res = await fetch(`/api/worlds/${world}/enabled-modules`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(ids),
  });
  if (!res.ok) throw new Error(`set enabled modules failed: ${res.status}`);
}
