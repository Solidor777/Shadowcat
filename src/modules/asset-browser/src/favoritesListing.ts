// Favourites-only listing: fetches just the favourites through the server's `ids` filter, in
// chunks of the per-request id cap, and merges the chunks into the order the server would give.

import type { Asset } from "@shadowcat/types";
import { MAX_QUERY_IDS, queryAssets, type AssetQuery } from "@shadowcat/core";

/** The textual-or-numeric key a listing sorts by, matching the server's `sort_key_of`: the
 * ASCII-lowercased name, the creation time, or the byte size.
 * @param asset - The asset to key.
 * @param sort - The listing's sort key.
 * @returns The comparable key.
 * @example
 * ```ts
 * declare const asset: import("@shadowcat/types").Asset;
 * sortKey(asset, "name");
 * ```
 */
function sortKey(asset: Asset, sort: "name" | "created" | "size"): string | number {
  if (sort === "name") return asset.original_name.replace(/[A-Z]/g, (c) => c.toLowerCase());
  return sort === "size" ? Number(asset.byte_size) : Number(asset.created_at);
}

/** Fetches the assets named by `ids` that match `query`'s other filters, ordered by `query.sort`
 * (then id), the server's order. Each chunk of `MAX_QUERY_IDS` ids is one request and one page,
 * so the result needs no cursor; no id means no request.
 * @param world - The world id.
 * @param query - The filters and sort; its cursor and limit are ignored.
 * @param ids - The asset ids to fetch.
 * @returns The matching assets, merged across chunks.
 * @example
 * ```ts
 * const assets = await queryFavoriteAssets("00000000-0000-0000-0000-000000000001", { sort: "name" }, [
 *   "00000000-0000-0000-0000-000000000002",
 * ]);
 * ```
 */
export async function queryFavoriteAssets(
  world: string,
  query: AssetQuery,
  ids: string[],
): Promise<Asset[]> {
  const chunks: string[][] = [];
  for (let i = 0; i < ids.length; i += MAX_QUERY_IDS) chunks.push(ids.slice(i, i + MAX_QUERY_IDS));
  const pages = await Promise.all(
    chunks.map((chunk) =>
      queryAssets(world, { ...query, cursor: undefined, limit: MAX_QUERY_IDS, ids: chunk }),
    ),
  );
  const sort = query.sort ?? "created";
  return pages
    .flatMap((page) => page.items)
    .sort((a, b) => {
      const ka = sortKey(a, sort);
      const kb = sortKey(b, sort);
      if (ka !== kb) return ka < kb ? -1 : 1;
      return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
    });
}
