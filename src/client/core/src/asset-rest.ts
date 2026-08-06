import type { Asset } from "@shadowcat/types";

// Client-side asset REST, beside AssetResolver: the asset upload/list/replace/
// delete contract with the server. Shared by the assets panel and scene-tools'
// asset picker, so it lives in the framework-neutral core (not a single panel
// module). Plain fetch — no Svelte in core's closure (invariant #7).

/** Upload an image to a world. GM-only (`require_gm`, `http::assets::upload`);
 * a non-GM member or non-member both get a 403. Streamed multipart to
 * `POST /api/worlds/{world}/assets`; the server validates the leading bytes are
 * a supported image regardless of the file's declared content-type, and
 * enforces a per-user rate limit and a max-byte cap (config-tiered by world
 * role, but `require_gm` means only the GM tier is ever reached from this
 * function), surfacing as a client-actionable 429/413 respectively.
 * @param world The world id to upload into.
 * @param file The image file to upload.
 * @returns The created asset record.
 * @example
 * ```ts
 * import { uploadAsset } from "@shadowcat/core";
 *
 * declare const file: File;
 * const asset = await uploadAsset("00000000-0000-0000-0000-000000000001", file);
 * ```
 */
export async function uploadAsset(world: string, file: File): Promise<Asset> {
  const form = new FormData();
  form.append("file", file);
  const res = await fetch(`/api/worlds/${world}/assets`, { method: "POST", body: form });
  if (!res.ok) throw new Error(`upload failed: ${res.status}`);
  return (await res.json()) as Asset;
}

/** List a world's assets (the grid source). Membership-gated
 * (`permission_context`, `http::assets::list`); a non-member gets a 403. Any
 * failure throws with only the HTTP status in the message — the server's
 * `{error}` body, if any, is not parsed here (unlike the
 * `restError` helper).
 * @param world The world id to list.
 * @returns The world's asset records.
 * @example
 * ```ts
 * import { listAssets } from "@shadowcat/core";
 *
 * const assets = await listAssets("00000000-0000-0000-0000-000000000001");
 * ```
 */
export async function listAssets(world: string): Promise<Asset[]> {
  const res = await fetch(`/api/worlds/${world}/assets`);
  if (!res.ok) throw new Error(`list failed: ${res.status}`);
  return (await res.json()) as Asset[];
}

/** Replace an asset's bytes behind its stable UUID (the id and every existing
 * reference to it survive; only `version`/`content_type`/`byte_size` change).
 * GM-only, scoped to the asset's OWN world (`require_gm`, `http::assets::replace`);
 * an unknown `uuid` is a 404, a known one outside the caller's GM world is a
 * 403. Shares `uploadAsset`'s per-user rate limit and max-byte cap (same
 * GM-only reachability caveat) and broadcasts an out-of-band `AssetChanged` to
 * every connection in the asset's world on success.
 * @param uuid The asset's stable id.
 * @param file The replacement image file.
 * @returns The updated asset record.
 * @example
 * ```ts
 * import { replaceAsset } from "@shadowcat/core";
 *
 * declare const file: File;
 * const asset = await replaceAsset("00000000-0000-0000-0000-000000000001", file);
 * ```
 */
export async function replaceAsset(uuid: string, file: File): Promise<Asset> {
  const form = new FormData();
  form.append("file", file);
  const res = await fetch(`/api/assets/${uuid}/replace`, { method: "POST", body: form });
  if (!res.ok) throw new Error(`replace failed: ${res.status}`);
  return (await res.json()) as Asset;
}

/** Delete an asset: removes its record and its on-disk file, and broadcasts an
 * out-of-band `AssetChanged` to every connection in the asset's world.
 * GM-only, scoped to the asset's own world (`require_gm`,
 * `http::assets::delete`); an unknown `uuid` is a 404. A file already missing
 * from disk is not fatal — the record deletion still succeeds.
 * @param uuid The asset's stable id.
 * @example
 * ```ts
 * import { deleteAsset } from "@shadowcat/core";
 *
 * await deleteAsset("00000000-0000-0000-0000-000000000001");
 * ```
 */
export async function deleteAsset(uuid: string): Promise<void> {
  const res = await fetch(`/api/assets/${uuid}`, { method: "DELETE" });
  if (!res.ok) throw new Error(`delete failed: ${res.status}`);
}
