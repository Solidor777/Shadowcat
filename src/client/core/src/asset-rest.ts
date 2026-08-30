import type { Asset, AssetPage, BulkAssetRequest, PatchAssetRequest } from "@shadowcat/types";

// Client-side asset REST, beside AssetResolver: the asset upload/list/replace/
// delete contract with the server. Shared by the assets panel and scene-tools'
// asset picker, so it lives in the framework-neutral core (not a single panel
// module). Plain fetch — core stays framework-neutral, so no Svelte in its dependency closure.

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

/** Filters for `queryAssets`; every field optional. Passing an empty object still hits the
 * query form (an `AssetPage`), never the bare listing. */
export interface AssetQuery {
  /** A folder document id, or `"root"` for unfiled assets; absent = whole world. */
  folder?: string;
  /** With `folder`: include every descendant folder. */
  recursive?: boolean;
  /** Every listed tag must be present (explicit or derived). */
  tags?: string[];
  /** `"image"` (`content_type` starts with `image/`) or `"other"`. */
  kind?: "image" | "other";
  /** Case-insensitive substring of the display name. */
  name?: string;
  /** Rust-syntax regex over the display name (server-capped at 256 bytes). */
  nameRegex?: string;
  /** Sort key (default `"created"`). */
  sort?: "name" | "created" | "size";
  /** Page size, 1..=500 (default 200). */
  limit?: number;
  /** A previous page's `next_cursor`. */
  cursor?: string;
}

/** The shape of the server's failure body (`AppError`'s JSON form). */
interface RestErrorBody {
  /** Player-presentable failure text. */
  error?: unknown;
}

/** The server's `{error}` text from a failed response, else `"HTTP <status>"` — the
 * human-readable half of every REST failure message this module and the chunked-upload
 * client produce.
 * @param res The non-ok response.
 * @returns The server's error text, or `"HTTP <status>"` when the body carries none.
 * @example
 * ```ts
 * import { restErrorText } from "@shadowcat/core";
 *
 * await restErrorText(new Response(null, { status: 413 })); // "HTTP 413"
 * ```
 */
export async function restErrorText(res: Response): Promise<string> {
  try {
    const body: unknown = await res.json();
    if (typeof body === "object" && body !== null && "error" in body) {
      const text = (body as RestErrorBody).error;
      if (typeof text === "string") return text;
    }
  } catch {
    // Not JSON — the status alone is the message.
  }
  return `HTTP ${res.status}`;
}

/** Build the `Error` a failed REST call throws: `"<what> failed: <status> <server text>"`.
 * @param res The non-ok response.
 * @param what The operation name for the message.
 * @returns The error to throw (not thrown here).
 * @example
 * ```ts
 * const err = await restError(new Response(null, { status: 403 }), "patch");
 * err.message; // "patch failed: 403 HTTP 403"
 * ```
 */
async function restError(res: Response, what: string): Promise<Error> {
  return new Error(`${what} failed: ${res.status} ${await restErrorText(res)}`);
}

/** Query a world's assets: folder / tag / kind / name / regex filters, a sort key, and keyset
 * pagination. Membership-gated (`permission_context`, `http::assets::query::list`). Always the
 * page form; use `listAssets` for the bare whole-world array.
 * @param world The world id to query.
 * @param q Filters, sort and page position.
 * @returns One page of assets plus the cursor for the next (or `null`).
 * @example
 * ```ts
 * import { queryAssets } from "@shadowcat/core";
 *
 * const page = await queryAssets("00000000-0000-0000-0000-000000000001", {
 *   folder: "root",
 *   tags: ["map"],
 *   sort: "name",
 *   limit: 50,
 * });
 * ```
 */
export async function queryAssets(world: string, q: AssetQuery): Promise<AssetPage> {
  const params = new URLSearchParams();
  if (q.folder !== undefined) params.set("folder", q.folder);
  if (q.recursive !== undefined) params.set("recursive", String(q.recursive));
  if (q.tags && q.tags.length > 0) params.set("tags", q.tags.join(","));
  if (q.kind !== undefined) params.set("kind", q.kind);
  if (q.name !== undefined && q.name !== "") params.set("name", q.name);
  if (q.nameRegex !== undefined && q.nameRegex !== "") params.set("name_regex", q.nameRegex);
  if (q.sort !== undefined) params.set("sort", q.sort);
  if (q.limit !== undefined) params.set("limit", String(q.limit));
  if (q.cursor !== undefined) params.set("cursor", q.cursor);
  // `limit` is always sent so an otherwise-empty query still selects the page form.
  if (!params.has("limit")) params.set("limit", "200");
  const res = await fetch(`/api/worlds/${world}/assets?${params.toString()}`);
  if (!res.ok) throw await restError(res, "query");
  return (await res.json()) as AssetPage;
}

/** Rename, move and/or retag one asset (GM-only, `http::assets::mutate::patch`). An absent
 * field is left unchanged; `folder_id: null` moves the asset to the world root. Derived tags
 * are recomputed server-side and a `moved` notice reaches every connection in the world.
 * @param uuid The asset's stable id.
 * @param patch The fields to change.
 * @returns The updated asset record.
 * @example
 * ```ts
 * import { patchAsset } from "@shadowcat/core";
 *
 * const asset = await patchAsset("00000000-0000-0000-0000-000000000001", {
 *   name: "crypt.png",
 *   folder_id: null,
 *   tags: ["map", "dungeon"],
 * });
 * ```
 */
export async function patchAsset(uuid: string, patch: PatchAssetRequest): Promise<Asset> {
  const res = await fetch(`/api/assets/${uuid}`, {
    method: "PATCH",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(patch),
  });
  if (!res.ok) throw await restError(res, "patch");
  return (await res.json()) as Asset;
}

/** Move and/or retag several assets in one server transaction (GM-only,
 * `http::assets::mutate::bulk`). Every id must belong to `world` — one foreign id fails the
 * whole batch (404) with nothing applied. One `moved` notice per asset.
 * @param world The world the assets belong to.
 * @param body The ids and the edit to apply to all of them.
 * @returns The updated asset records, in `ids` order.
 * @example
 * ```ts
 * import { bulkPatchAssets } from "@shadowcat/core";
 *
 * const assets = await bulkPatchAssets("00000000-0000-0000-0000-000000000001", {
 *   ids: ["00000000-0000-0000-0000-000000000002"],
 *   folder_id: null,
 *   add_tags: ["map"],
 *   remove_tags: [],
 * });
 * ```
 */
export async function bulkPatchAssets(world: string, body: BulkAssetRequest): Promise<Asset[]> {
  const res = await fetch(`/api/worlds/${world}/assets/bulk`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw await restError(res, "bulk patch");
  return (await res.json()) as Asset[];
}

/** Re-run the conversion pipeline on an asset's retained original (GM-only,
 * `http::assets::mutate::reconvert`); 404 when the original was not retained. Bumps the
 * version and broadcasts `replaced`, exactly like `replaceAsset`.
 * @param uuid The asset's stable id.
 * @returns The updated asset record.
 * @example
 * ```ts
 * import { reconvertAsset } from "@shadowcat/core";
 *
 * const asset = await reconvertAsset("00000000-0000-0000-0000-000000000001");
 * ```
 */
export async function reconvertAsset(uuid: string): Promise<Asset> {
  const res = await fetch(`/api/assets/${uuid}/reconvert`, { method: "POST" });
  if (!res.ok) throw await restError(res, "reconvert");
  return (await res.json()) as Asset;
}

/** The GM-only download URL of an asset's retained original bytes (served as an attachment
 * named after the upload; 404 when `original_retained` is false).
 * @param uuid The asset's stable id.
 * @returns The `/api/assets/{uuid}/original` URL.
 * @example
 * ```ts
 * import { originalUrl } from "@shadowcat/core";
 *
 * const href = originalUrl("00000000-0000-0000-0000-000000000001");
 * ```
 */
export function originalUrl(uuid: string): string {
  return `/api/assets/${uuid}/original`;
}
