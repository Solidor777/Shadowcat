/** Op carried by an out-of-band AssetChanged frame. */
export type AssetOp = "replaced" | "deleted";

/**
 * Resolves asset UUIDs to serve URLs and reacts to out-of-band AssetChanged
 * notices. The server's ETag handles HTTP caching; a monotonic per-uuid `rev`
 * counter cache-busts the URL on replace so a fresh request (and thus ETag
 * revalidation) happens. Deleted uuids resolve to the placeholder.
 */
export class AssetResolver {
  private revs = new Map<string, number>();
  private deleted = new Set<string>();

  /** A neutral 1×1 transparent placeholder.
   * @returns A `data:` URI for a 1×1 transparent GIF.
   * @example
   * ```ts
   * import { AssetResolver } from "@shadowcat/core";
   *
   * const resolver = new AssetResolver();
   * resolver.placeholder();
   * ```
   */
  placeholder(): string {
    return "data:image/gif;base64,R0lGODlhAQABAAAAACwAAAAAAQABAAA=";
  }

  /** Resolves an asset uuid to a serve URL, cache-busted by the current `rev`
   * so a replace forces a fresh request (and thus ETag revalidation); a
   * deleted uuid resolves to `placeholder()`.
   * @param uuid The asset's stable uuid.
   * @returns The `/api/assets/{uuid}` URL, or the placeholder if deleted.
   * @example
   * ```ts
   * import { AssetResolver } from "@shadowcat/core";
   *
   * const resolver = new AssetResolver();
   * resolver.url("00000000-0000-0000-0000-000000000001");
   * ```
   */
  url(uuid: string): string {
    if (this.deleted.has(uuid)) return this.placeholder();
    const rev = this.revs.get(uuid);
    return rev === undefined ? `/api/assets/${uuid}` : `/api/assets/${uuid}?v=${rev}`;
  }

  /** Invalidate a uuid in response to an AssetChanged frame.
   * @param msg The broadcast frame.
   * @param msg.uuid The affected asset's uuid.
   * @param msg.op `"replaced"` bumps the cache-busting revision; `"deleted"` switches `url()` to the placeholder.
   * @example
   * ```ts
   * import { AssetResolver } from "@shadowcat/core";
   *
   * const resolver = new AssetResolver();
   * resolver.onAssetChanged({ uuid: "00000000-0000-0000-0000-000000000001", op: "replaced" });
   * ```
   */
  onAssetChanged(msg: { uuid: string; op: AssetOp }): void {
    if (msg.op === "deleted") {
      this.deleted.add(msg.uuid);
      this.revs.delete(msg.uuid);
      return;
    }
    // replaced: drop any delete marker and bump the cache-bust revision.
    this.deleted.delete(msg.uuid);
    this.revs.set(msg.uuid, (this.revs.get(msg.uuid) ?? 0) + 1);
  }
}
