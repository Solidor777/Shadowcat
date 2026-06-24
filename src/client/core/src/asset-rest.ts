import type { Asset } from "@shadowcat/types";

// Client-side asset REST, beside AssetResolver: the asset upload/list/replace/
// delete contract with the server. Shared by the assets panel and scene-tools'
// asset picker, so it lives in the framework-neutral core (not a single panel
// module). Plain fetch — no Svelte in core's closure (invariant #7).

/** Upload an image to a world; returns the created asset record. */
export async function uploadAsset(world: string, file: File): Promise<Asset> {
  const form = new FormData();
  form.append("file", file);
  const res = await fetch(`/api/worlds/${world}/assets`, { method: "POST", body: form });
  if (!res.ok) throw new Error(`upload failed: ${res.status}`);
  return (await res.json()) as Asset;
}

/** List a world's assets (the grid source). */
export async function listAssets(world: string): Promise<Asset[]> {
  const res = await fetch(`/api/worlds/${world}/assets`);
  if (!res.ok) throw new Error(`list failed: ${res.status}`);
  return (await res.json()) as Asset[];
}

/** Replace an asset's bytes behind its stable UUID; returns the updated record. */
export async function replaceAsset(uuid: string, file: File): Promise<Asset> {
  const form = new FormData();
  form.append("file", file);
  const res = await fetch(`/api/assets/${uuid}/replace`, { method: "POST", body: form });
  if (!res.ok) throw new Error(`replace failed: ${res.status}`);
  return (await res.json()) as Asset;
}

/** Delete an asset (file + record). */
export async function deleteAsset(uuid: string): Promise<void> {
  const res = await fetch(`/api/assets/${uuid}`, { method: "DELETE" });
  if (!res.ok) throw new Error(`delete failed: ${res.status}`);
}
